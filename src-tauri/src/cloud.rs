//! Secours cloud Marstek (API non officielle, lecture seule) quand l'Open API locale est coupée.
//!
//! Le mot de passe et son MD5 ne vivent que dans `login` : jamais écrits, journalisés, ni repris dans une erreur
//! (les messages sont fixes, ni le texte du serveur ni celui d'`ureq`, qui peut citer l'URL, n'est recopié).
//! Seuls le jeton, l'e-mail et la région sont gardés, dans le Gestionnaire d'identifiants.
//! Une seule écriture, sur action explicite : `enable_local_api` publie la commande de réouverture de l'Open API
//! locale sur le topic MQTT cloud de l'appareil. Le reste (connexion, `getDeviceList`) est en lecture seule.

use crate::i18n::Msg;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const SERVICE: &str = "marstek-lan";
const ENTRY: &str = "cloud";
/// Codes de réponse qui signifient « jeton refusé ».
const REFUSED: [i64; 4] = [-1, 401, 403, 8];
const IGNORED_TYPE: &str = "HME-3";
/// Écart minimal entre deux lectures de `getDeviceList`, quel que soit l'appelant (secours ou liste des appareils).
pub const GAP: Duration = Duration::from_secs(300);

static LAST: Mutex<Option<Instant>> = Mutex::new(None);
/// Le serveur a refusé le jeton : plus aucune requête avant une nouvelle connexion (le mot de passe n'est pas gardé).
static EXPIRED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    Eu,
    Us,
}

impl Region {
    /// Hôte choisi dans une liste fermée : aucune saisie ne décide de la destination.
    fn host(self) -> &'static str {
        match self {
            Self::Eu => "eu.hamedata.com",
            Self::Us => "us.hamedata.com",
        }
    }
}

/// Pas de `Debug` : le jeton ne doit jamais finir dans un journal par accident.
#[derive(Serialize, Deserialize)]
struct Session {
    email: String,
    region: Region,
    token: String,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub email: String,
    pub region: Region,
    pub expired: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudDevice {
    pub devid: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub sn: Option<String>,
    /// MAC et salt servent à dériver le topic cloud pour `enable_local_api` ; internes, non transmis au front.
    #[serde(skip)]
    pub mac: Option<String>,
    #[serde(skip)]
    pub salt: Option<String>,
    pub soc: Option<f64>,
    pub charge: Option<f64>,
    pub discharge: Option<f64>,
    pub load: Option<f64>,
    pub pv: Option<f64>,
    /// Secondes depuis l'epoch ; absent : la mesure n'est pas datée.
    pub report_time: Option<u64>,
}

/// Dernière mesure du cloud, telle que l'interface l'affiche.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub soc: Option<f64>,
    pub charge_w: Option<f64>,
    pub discharge_w: Option<f64>,
    pub load_w: Option<f64>,
    pub pv_w: Option<f64>,
    pub report_time: u64,
    pub age_secs: u64,
}

impl Reading {
    /// Une mesure sans `report_time` est ignorée : le serveur renvoie alors des zéros (SOC 0 % au lieu de 91 %).
    pub fn new(d: &CloudDevice, now_secs: u64) -> Option<Self> {
        let report_time = d.report_time?;
        Some(Self {
            soc: d.soc,
            charge_w: d.charge,
            discharge_w: d.discharge,
            load_w: d.load,
            pv_w: d.pv,
            report_time,
            age_secs: now_secs.saturating_sub(report_time),
        })
    }
}

fn md5_hex(s: &str) -> String {
    Md5::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .https_only(true)
            // Une redirection recopierait le mot de passe ou le jeton (dans l'URL) vers un autre hôte.
            .max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(20)))
            .build(),
    )
}

/// Message fixe par famille d'erreur : on ne formate jamais l'erreur `ureq`, qui peut contenir l'URL.
fn network_error(e: &ureq::Error) -> Msg {
    match e {
        ureq::Error::Timeout(_) => Msg::CloudTimeout,
        ureq::Error::StatusCode(code) => Msg::CloudHttp(*code),
        _ => Msg::CloudUnreachable,
    }
}

fn read_body(mut resp: ureq::http::Response<ureq::Body>) -> Result<String, Msg> {
    resp.body_mut().read_to_string().map_err(|_| Msg::CloudUnreadable)
}

fn parse_login(body: &str) -> Result<String, Msg> {
    let v: Value = serde_json::from_str(body).map_err(|_| Msg::CloudUnreadable)?;
    match v.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        Some(token) => Ok(token.to_owned()),
        None => Err(Msg::CloudLoginRefused),
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Le serveur mélange nombres et chaînes numériques.
fn number(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// `report_time` en secondes ; au-delà de 10^11 le serveur donne des millisecondes. Zéro ou négatif : non daté.
// shortcut: format de date texte non géré (ignoré), à ajouter si le serveur en envoie.
fn report_time(v: &Value) -> Option<u64> {
    let t = number(v, "report_time").filter(|t| *t >= 1.0)?;
    Some(if t > 1e11 { t / 1000.0 } else { t } as u64)
}

fn parse_devices(body: &str) -> Result<Vec<CloudDevice>, Msg> {
    let v: Value = serde_json::from_str(body).map_err(|_| Msg::CloudUnreadable)?;
    if number(&v, "code").is_some_and(|c| REFUSED.contains(&(c as i64))) {
        return Err(Msg::CloudExpired);
    }
    let list = v.get("data").and_then(Value::as_array).ok_or(Msg::CloudUnexpected)?;
    Ok(list
        .iter()
        .filter(|d| text(d, "type").as_deref() != Some(IGNORED_TYPE))
        .filter_map(|d| {
            Some(CloudDevice {
                devid: text(d, "devid")?,
                name: text(d, "name"),
                kind: text(d, "type"),
                sn: text(d, "sn"),
                mac: text(d, "mac"),
                salt: text(d, "salt"),
                soc: number(d, "soc"),
                charge: number(d, "charge"),
                discharge: number(d, "discharge"),
                load: number(d, "load"),
                pv: number(d, "pv"),
                report_time: report_time(d),
            })
        })
        .collect())
}

fn check(email: &str, password: &str) -> Result<(), Msg> {
    if email.is_empty() || email.len() > 254 || email.chars().any(char::is_control) {
        return Err(Msg::CloudBadEmail);
    }
    if password.is_empty() || password.len() > 256 {
        return Err(Msg::CloudBadPassword);
    }
    Ok(())
}

fn entry() -> Result<keyring::Entry, Msg> {
    keyring::Entry::new(SERVICE, ENTRY).map_err(|_| Msg::CloudStore)
}

fn session() -> Result<Option<Session>, Msg> {
    match entry()?.get_password() {
        Ok(json) => Ok(serde_json::from_str(&json).ok()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(Msg::CloudStore),
    }
}

fn last() -> std::sync::MutexGuard<'static, Option<Instant>> {
    LAST.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn login(email: &str, password: &str, region: Region) -> Result<(), Msg> {
    let email = email.trim();
    check(email, password)?;
    let resp = agent()
        .post(format!("https://{}/app/Solar/v2_get_device.php", region.host()))
        .query("mailbox", email)
        // Imposé par l'API ; le hachage n'existe que dans cet appel.
        .query("pwd", md5_hex(password))
        .send_empty()
        .map_err(|e| network_error(&e))?;
    let token = parse_login(&read_body(resp)?)?;
    let json = serde_json::to_string(&Session { email: email.to_owned(), region, token }).map_err(|_| Msg::CloudStore)?;
    entry()?.set_password(&json).map_err(|_| Msg::CloudStore)?;
    // Nouveau jeton : la lecture de validation qui suit est permise tout de suite.
    EXPIRED.store(false, Ordering::Relaxed);
    *last() = None;
    Ok(())
}

/// Le secours est dû : batterie injoignable en local et dernière lecture cloud vieille d'au moins `GAP`.
fn poll_due(down: bool, last: Option<Instant>, now: Instant) -> bool {
    down && last.is_none_or(|t| now.duration_since(t) >= GAP)
}

/// Le secours est dû maintenant : batterie injoignable en local, délai écoulé, jeton non refusé.
pub fn ready(down: bool) -> bool {
    !EXPIRED.load(Ordering::Relaxed) && poll_due(down, *last(), Instant::now())
}

/// Lit les appareils du compte ; au plus une fois par `GAP`, et plus du tout après un jeton refusé.
pub fn devices() -> Result<Vec<CloudDevice>, Msg> {
    if EXPIRED.load(Ordering::Relaxed) {
        return Err(Msg::CloudExpired);
    }
    let s = session()?.ok_or(Msg::CloudExpired)?;
    {
        let mut last = last();
        if !poll_due(true, *last, Instant::now()) {
            return Err(Msg::CloudWait);
        }
        *last = Some(Instant::now());
    }
    let expired = || {
        EXPIRED.store(true, Ordering::Relaxed);
        Msg::CloudExpired
    };
    let resp = agent()
        .get(format!("https://{}/ems/api/v1/getDeviceList", s.region.host()))
        .query("token", &s.token)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401 | 403) => expired(),
            e => network_error(&e),
        })?;
    parse_devices(&read_body(resp)?).inspect_err(|e| {
        if *e == Msg::CloudExpired {
            expired();
        }
    })
}

/// `getDeviceList` avec `mailbox`+`token`, sans la limite des 5 min : pour une action explicite de l'utilisateur.
/// (La variante throttlée `devices()` reste la voie du sondage de secours.)
fn fetch_devices_body(s: &Session) -> Result<String, Msg> {
    let resp = agent()
        .get(format!("https://{}/ems/api/v1/getDeviceList", s.region.host()))
        .query("mailbox", &s.email)
        .query("token", &s.token)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401 | 403) => {
                EXPIRED.store(true, Ordering::Relaxed);
                Msg::CloudExpired
            }
            e => network_error(&e),
        })?;
    read_body(resp)
}

fn get_device_list(s: &Session) -> Result<Vec<CloudDevice>, Msg> {
    parse_devices(&fetch_devices_body(s)?).inspect_err(|e| {
        if *e == Msg::CloudExpired {
            EXPIRED.store(true, Ordering::Relaxed);
        }
    })
}

/// Noms et types de toutes les entrées de la liste cloud (sans le filtre des batteries), pour voir ce qui est listé — dont le CT.
fn device_names(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return Vec::new() };
    v.get("data")
        .and_then(Value::as_array)
        .map(|list| list.iter().map(|d| format!("{}({})", text(d, "name").as_deref().unwrap_or("?"), text(d, "type").as_deref().unwrap_or("?"))).collect())
        .unwrap_or_default()
}

/// Les topics cloud de `dev` : commande (`App` → appareil) et réponse (appareil → `App`, segment `device`), ou `None`
/// si l'appareil n'a pas de quoi les dériver (MAC/salt absents, ou salt non confirmé : cf. `topic_id::resolve_topic_id`).
fn device_topics(dev: &CloudDevice) -> Option<(String, String)> {
    let (mac, salt, kind) = (dev.mac.as_deref()?, dev.salt.as_deref()?, dev.kind.as_deref()?);
    // Le type vient de la réponse cloud : on ne l'interpole dans le topic que s'il a la forme attendue (ex. « VNSD-0 »),
    // pour qu'une réponse falsifiée ne puisse pas y glisser un séparateur ou un joker MQTT (`/`, `+`, `#`).
    if mac.is_empty() || kind.is_empty() || !kind.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    let id = crate::topic_id::resolve_topic_id(salt, mac, kind)?;
    Some((format!("marstek_energy/{kind}/App/{id}/ctrl"), format!("marstek_energy/{kind}/device/{id}/ctrl")))
}

fn device_topic(dev: &CloudDevice) -> Option<String> {
    device_topics(dev).map(|(cmd, _)| cmd)
}

/// Identifiant de client MQTT unique (préfixe `mst_` imposé par le broker + 24 hexa), jamais réutilisé.
fn client_id() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("mst_{:016x}{:08x}", nanos as u64, (nanos >> 64) as u32 ^ std::process::id())
}

/// Publie `cd=30,api=1,port=30000` sur le topic cloud de `dev` pour rouvrir son Open API locale. L'authentification MQTT
/// est le certificat client (pas le jeton), donc aucun appel HTTPS ici : réutilisable avec un appareil déjà en main.
/// L'ordre `api` avant `port` est imposé par l'appareil.
pub fn enable_from(dev: &CloudDevice) -> Result<(), Msg> {
    let topic = device_topic(dev).ok_or(Msg::CloudEnableUnsupported)?;
    crate::mqtt::publish(&client_id(), &topic, b"cd=30,api=1,port=30000")
}

fn valid_ct_mac(s: &str) -> bool {
    s.len() == 12 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Commande de re-sélection du CT : `cd=18,meter=<code>` puis `,mac=<mac>` (l'appareil impose cet ordre). Valide les entrées
/// avant de construire la commande.
fn ct_payload(meter: u8, mac: Option<&str>) -> Result<String, Msg> {
    if meter > 12 {
        return Err(Msg::CloudEnableUnsupported);
    }
    match mac {
        None => Ok(format!("cd=18,meter={meter}")),
        Some(m) if valid_ct_mac(m) => Ok(format!("cd=18,meter={meter},mac={m}")),
        Some(_) => Err(Msg::CloudBadCtMac),
    }
}

/// Attente d'une réponse de l'appareil sur le topic cloud (la bascule `cd=30` répond en ~5 s dans nos essais).
const CT_REPLY_WAIT: Duration = Duration::from_secs(8);

/// Valeur d'un champ `clé=valeur` dans une réponse `cd=…,k1=v1,k2=v2` de l'appareil.
fn field<'a>(reply: &'a str, key: &str) -> Option<&'a str> {
    reply.split(',').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

/// Publie une commande sur le topic de `dev` et attend la réponse de l'appareil sur son topic `device`.
fn ct_request(dev: &CloudDevice, client: &str, payload: &[u8]) -> Result<Option<String>, Msg> {
    let (cmd, reply) = device_topics(dev).ok_or(Msg::CloudEnableUnsupported)?;
    crate::mqtt::request(client, &reply, &cmd, payload, CT_REPLY_WAIT)
}

/// Résumé technique : config CT lue en direct (`cd=1`), action (commande + réponse, ou rien si le CT est déjà joint),
/// et la liste cloud brute (pour y voir le vrai nom du CT).
fn ct_summary(status: Option<&str>, power: Option<&str>, action: &str, names: &[String]) -> String {
    let read = status.map_or_else(
        || "?".to_owned(),
        |s| format!("ct_t={} gct_s={} prc_c={}", field(s, "ct_t").unwrap_or("?"), field(s, "gct_s").unwrap_or("?"), field(s, "prc_c").unwrap_or("?")),
    );
    format!("read[{read}] power[{}] {action} devices[{}]", power.unwrap_or("\u{2014}"), names.join(" "))
}

/// Re-sélectionne le CT de `devid`. Lit d'abord son état réel en direct (`cd=1`) : s'il est déjà joint (`gct_s=1`) on ne
/// renvoie rien ; sinon on repose `cd=18,meter=<ct_t réel>` (le `meter` fourni n'est qu'un repli si `ct_t` est illisible).
/// Rend un résumé avec la réponse de l'appareil et la liste cloud brute. `cd=18` fixe le type (et efface l'erreur ≥ fw149)
/// mais n'est pas un ré-appairage documenté : le résumé sert à voir si la batterie accepte et si `gct_s` repasse à 1.
pub fn reselect_ct(devid: &str, meter: u8, mac: Option<&str>) -> Result<String, Msg> {
    ct_payload(meter, mac)?; // valide le repli avant tout réseau
    if EXPIRED.load(Ordering::Relaxed) {
        return Err(Msg::CloudExpired);
    }
    let s = session()?.ok_or(Msg::CloudExpired)?;
    let body = fetch_devices_body(&s)?;
    let names = device_names(&body);
    let dev = parse_devices(&body)?.into_iter().find(|d| d.devid == devid).ok_or(Msg::CloudUnexpected)?;
    let client = client_id();
    let status = ct_request(&dev, &client, b"cd=1")?;
    let power = ct_request(&dev, &client, b"cd=19")?; // puissance CT en direct via le cloud (cd=19,get_power=…)
    // Décision sur la valeur COURANTE lue à l'instant, pas sur la dernière mesure locale (compteur sondé ~30 min).
    let action = if status.as_deref().and_then(|r| field(r, "gct_s")) == Some("1") {
        "skip[CT déjà joint]".to_owned()
    } else {
        let target = status.as_deref().and_then(|r| field(r, "ct_t")).and_then(|v| v.parse::<u8>().ok()).filter(|&m| m <= 12).unwrap_or(meter);
        let payload = ct_payload(target, mac)?;
        let reply = ct_request(&dev, &client, payload.as_bytes())?;
        format!("sent[{payload}] reply[{}]", reply.as_deref().unwrap_or("\u{2014}"))
    };
    Ok(ct_summary(status.as_deref(), power.as_deref(), &action, &names))
}

/// Rouvre l'Open API locale de l'appareil `devid` via le cloud. Récupère d'abord l'appareil (salt courant) puis publie.
/// Unique écriture déclenchée par le bouton explicite ; la réactivation automatique réutilise plutôt `enable_from`.
pub fn enable_local_api(devid: &str) -> Result<(), Msg> {
    if EXPIRED.load(Ordering::Relaxed) {
        return Err(Msg::CloudExpired);
    }
    let s = session()?.ok_or(Msg::CloudExpired)?;
    let dev = get_device_list(&s)?.into_iter().find(|d| d.devid == devid).ok_or(Msg::CloudUnexpected)?;
    enable_from(&dev)
}

pub fn logout() -> Result<(), Msg> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(Msg::CloudStore),
    }
}

pub fn status() -> Result<Option<Status>, Msg> {
    Ok(session()?.map(|s| Status { email: s.email, region: s.region, expired: EXPIRED.load(Ordering::Relaxed) }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    const LANGS: [Lang; 4] = [Lang::En, Lang::Fr, Lang::De, Lang::Es];

    #[test]
    fn md5_is_lowercase_hex() {
        assert_eq!(md5_hex("password"), "5f4dcc3b5aa765d61d8327deb882cf99");
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn login_reads_the_token_and_never_echoes_the_server() {
        assert_eq!(parse_login(r#"{"code":1,"token":"abc123"}"#).unwrap(), "abc123");
        assert_eq!(parse_login(r#"{"code":0,"msg":"secret-from-server"}"#).unwrap_err(), Msg::CloudLoginRefused);
        assert!(parse_login(r#"{"token":""}"#).is_err());
        assert_eq!(parse_login("<html>").unwrap_err(), Msg::CloudUnreadable);
    }

    const DEVICES: &str = r#"{"code":1,"data":[
        {"devid":"d1","name":"Venus","type":"HME-4","sn":"S1","mac":"bc2a3314cc36","salt":"84905ed2bc7ab1e5,84905ed2bc7ab1e5","soc":"76","charge":120.5,
         "discharge":0,"load":"310","pv":"450","report_time":"1700000000"},
        {"devid":"d2","type":"HME-3","name":"Compteur"},
        {"name":"sans identifiant"},
        {"devid":"d3","soc":0,"charge":0,"discharge":0,"load":0,"pv":0}
    ]}"#;

    #[test]
    fn devices_skip_ignored_types_and_accept_numbers_as_strings() {
        let list = parse_devices(DEVICES).unwrap();
        assert_eq!(list.len(), 2);
        let d = &list[0];
        assert_eq!((d.devid.as_str(), d.kind.as_deref(), d.report_time), ("d1", Some("HME-4"), Some(1_700_000_000)));
        assert_eq!((d.soc, d.charge, d.discharge, d.load, d.pv), (Some(76.0), Some(120.5), Some(0.0), Some(310.0), Some(450.0)));
    }

    #[test]
    fn device_topic_is_derived_from_the_cloud_fields() {
        let d1 = &parse_devices(DEVICES).unwrap()[0];
        assert_eq!((d1.mac.as_deref(), d1.salt.as_deref()), (Some("bc2a3314cc36"), Some("84905ed2bc7ab1e5,84905ed2bc7ab1e5")));
        // Appareil réel VNSD-0 : le topic cloud effectivement utilisé avec succès contre la Venus D (fw 147).
        let real = r#"{"code":1,"data":[{"devid":"x","type":"VNSD-0","mac":"bc2a3314cc36","salt":"84905ed2bc7ab1e5,84905ed2bc7ab1e5"}]}"#;
        assert_eq!(
            device_topic(&parse_devices(real).unwrap()[0]).as_deref(),
            Some("marstek_energy/VNSD-0/App/XVC0fVI0TV3VJVm05Vk020i0/ctrl"),
        );
        // Salt seulement émis (pas encore pris par l'appareil) : pas de topic.
        let pending = r#"{"code":1,"data":[{"devid":"x","type":"VNSD-0","mac":"bc2a3314cc36","salt":"0,84905ed2bc7ab1e5"}]}"#;
        assert_eq!(device_topic(&parse_devices(pending).unwrap()[0]), None);
        // MAC ou salt absent : pas de topic.
        let bare = r#"{"code":1,"data":[{"devid":"x","type":"VNSD-0"}]}"#;
        assert_eq!(device_topic(&parse_devices(bare).unwrap()[0]), None);
        // Type falsifié avec un joker MQTT : refusé (pas d'injection dans le topic).
        let evil = r#"{"code":1,"data":[{"devid":"x","type":"VNSD-0/#","mac":"bc2a3314cc36","salt":"84905ed2bc7ab1e5,84905ed2bc7ab1e5"}]}"#;
        assert_eq!(device_topic(&parse_devices(evil).unwrap()[0]), None);
    }

    #[test]
    fn ct_payload_is_validated_and_ordered() {
        assert_eq!(ct_payload(0, None).as_deref(), Ok("cd=18,meter=0"));
        assert_eq!(ct_payload(4, Some("aabbccddeeff")).as_deref(), Ok("cd=18,meter=4,mac=aabbccddeeff"));
        assert_eq!(ct_payload(13, None), Err(Msg::CloudEnableUnsupported));
        assert_eq!(ct_payload(4, Some("aabbccddeef")), Err(Msg::CloudBadCtMac));
        assert!(valid_ct_mac("AABBCCDDEEFF"));
        assert!(!valid_ct_mac("aabbccddeeff0") && !valid_ct_mac("aabbccddeefg") && !valid_ct_mac(""));
    }

    #[test]
    fn report_time_accepts_seconds_or_milliseconds_and_rejects_zero() {
        let rt = |s: &str| report_time(&serde_json::from_str(s).unwrap());
        assert_eq!(rt(r#"{"report_time":1700000000}"#), Some(1_700_000_000));
        assert_eq!(rt(r#"{"report_time":1700000000123}"#), Some(1_700_000_000));
        assert_eq!(rt(r#"{"report_time":0}"#), None);
        assert_eq!(rt(r#"{"report_time":"soon"}"#), None);
        assert_eq!(rt("{}"), None);
    }

    #[test]
    fn a_reading_without_report_time_is_ignored_even_with_a_zero_soc() {
        let list = parse_devices(DEVICES).unwrap();
        assert_eq!(list[1].soc, Some(0.0));
        assert_eq!(Reading::new(&list[1], 1_700_000_100), None);
        let r = Reading::new(&list[0], 1_700_000_090).unwrap();
        assert_eq!((r.soc, r.charge_w, r.pv_w, r.age_secs), (Some(76.0), Some(120.5), Some(450.0), 90));
        // Horloge en retard sur le serveur : jamais d'âge négatif.
        assert_eq!(Reading::new(&list[0], 1_699_999_000).unwrap().age_secs, 0);
    }

    #[test]
    fn serialized_reading_uses_the_ipc_names() {
        let json = serde_json::to_value(Reading::new(&parse_devices(DEVICES).unwrap()[0], 1_700_000_060).unwrap()).unwrap();
        for k in ["soc", "chargeW", "dischargeW", "loadW", "pvW", "reportTime", "ageSecs"] {
            assert!(json.get(k).is_some(), "{k}");
        }
        assert_eq!(json["ageSecs"], 60);
    }

    #[test]
    fn refused_token_codes_mean_expired_session() {
        for code in ["-1", "401", "403", "8", "\"8\""] {
            let body = format!(r#"{{"code":{code},"data":[]}}"#);
            assert_eq!(parse_devices(&body).unwrap_err(), Msg::CloudExpired, "{code}");
        }
        assert_eq!(parse_devices(r#"{"code":1}"#).unwrap_err(), Msg::CloudUnexpected);
        assert!(parse_devices("nope").is_err());
    }

    #[test]
    fn the_fallback_asks_the_cloud_only_while_down_and_at_most_every_five_minutes() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // Locale en ligne : jamais de requête, même sans lecture précédente.
        assert!(!poll_due(false, None, t0));
        assert!(!poll_due(false, Some(at(0)), at(10_000)));
        // En panne : une première lecture, puis plus rien avant 5 min.
        assert!(poll_due(true, None, t0));
        assert!(!poll_due(true, Some(at(0)), at(299)));
        assert!(poll_due(true, Some(at(0)), at(300)));
        // Une heure de panne, un passage par seconde : 12 requêtes au plus.
        let (mut last, mut n) = (None, 0);
        for s in 0..3600 {
            if poll_due(true, last, at(s)) {
                last = Some(at(s));
                n += 1;
            }
        }
        assert_eq!(n, 12);
    }

    #[test]
    fn an_expired_token_stops_requests_until_the_next_login() {
        EXPIRED.store(true, Ordering::Relaxed);
        assert!(!ready(true));
        assert_eq!(devices().unwrap_err(), Msg::CloudExpired);
        EXPIRED.store(false, Ordering::Relaxed);
    }

    #[test]
    fn inputs_are_validated() {
        assert!(check("a@b.fr", "pw").is_ok());
        assert_eq!(check("", "pw").unwrap_err(), Msg::CloudBadEmail);
        assert!(check("a@b.fr\n", "pw").is_err());
        assert_eq!(check("a@b.fr", "").unwrap_err(), Msg::CloudBadPassword);
        assert!(check(&"x".repeat(255), "pw").is_err());
    }

    #[test]
    fn region_is_a_closed_list_of_https_hosts() {
        assert_eq!(serde_json::from_str::<Region>("\"eu\"").unwrap().host(), "eu.hamedata.com");
        assert!(serde_json::from_str::<Region>("\"evil.example\"").is_err());
    }

    /// Tout ce que le secours peut écrire dans le journal ou renvoyer à l'interface : messages fixes, sans secret.
    #[test]
    fn messages_never_carry_a_secret() {
        let (token, email, password) = ("tok3n-SECRET", "me@secret.example", "pw-SECRET");
        let hash = md5_hex(password);
        let bodies = [
            format!(r#"{{"code":0,"token":"","msg":"{token} {email} {hash}"}}"#),
            format!(r#"{{"code":8,"data":[],"msg":"{token} {email}"}}"#),
            format!(r#"{{"code":1,"data":"{token} {email}"}}"#),
            format!("<html>{token} {email} {hash}</html>"),
        ];
        let errors = [ureq::Error::Timeout(ureq::Timeout::Global), ureq::Error::StatusCode(500), ureq::Error::HostNotFound];
        let mut msgs: Vec<Msg> = bodies.iter().flat_map(|b| [parse_login(b).err(), parse_devices(b).err()]).flatten().collect();
        msgs.extend(errors.iter().map(network_error));
        msgs.extend([check(email, "").unwrap_err(), check("", password).unwrap_err(), Msg::CloudStore, Msg::CloudWait]);
        assert!(msgs.len() >= 10);
        for m in msgs {
            for l in LANGS {
                let s = m.text(l);
                assert!(!s.is_empty());
                for secret in [token, email, password, hash.as_str(), "token=", "mailbox=", "pwd=", "https://"] {
                    assert!(!s.contains(secret), "{m:?} en {l:?} contient {secret}");
                }
            }
        }
    }
}
