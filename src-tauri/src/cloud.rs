//! Compte Marstek (API non officielle, lecture seule).
//!
//! Le mot de passe et son MD5 ne vivent que dans `login` : jamais écrits, journalisés, ni repris dans une erreur
//! (les messages sont fixes, ni le texte du serveur ni celui d'`ureq`, qui peut citer l'URL, n'est recopié).
//! Seuls le jeton, l'e-mail et la région sont gardés, dans le Gestionnaire d'identifiants.

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const SERVICE: &str = "marstek-lan";
const ENTRY: &str = "cloud";
pub const EXPIRED: &str = "session expirée";
const STORE_ERROR: &str = "Gestionnaire d'identifiants indisponible";
/// Codes de réponse qui signifient « jeton refusé ».
const REFUSED: [i64; 4] = [-1, 401, 403, 8];
const IGNORED_TYPE: &str = "HME-3";

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
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudDevice {
    pub devid: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub version: Option<String>,
    pub sn: Option<String>,
    pub soc: Option<f64>,
    pub charge: Option<f64>,
    pub discharge: Option<f64>,
    pub load: Option<f64>,
    pub profit: Option<f64>,
    pub capacity_kwh: Option<f64>,
    pub status: Option<String>,
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
fn network_error(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "le serveur Marstek ne répond pas (délai dépassé)".into(),
        ureq::Error::StatusCode(code) => format!("le serveur Marstek a répondu avec l'erreur HTTP {code}"),
        _ => "connexion au serveur Marstek impossible".into(),
    }
}

fn read_body(mut resp: ureq::http::Response<ureq::Body>) -> Result<String, String> {
    resp.body_mut().read_to_string().map_err(|_| "réponse illisible du serveur Marstek".to_owned())
}

fn parse_login(body: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "réponse illisible du serveur Marstek".to_owned())?;
    match v.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        Some(token) => Ok(token.to_owned()),
        None => Err("connexion refusée : e-mail ou mot de passe incorrect".into()),
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

fn parse_devices(body: &str) -> Result<Vec<CloudDevice>, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "réponse illisible du serveur Marstek".to_owned())?;
    if number(&v, "code").is_some_and(|c| REFUSED.contains(&(c as i64))) {
        return Err(EXPIRED.into());
    }
    let list = v.get("data").and_then(Value::as_array).ok_or("réponse inattendue du serveur Marstek")?;
    Ok(list
        .iter()
        .filter(|d| text(d, "type").as_deref() != Some(IGNORED_TYPE))
        .filter_map(|d| {
            Some(CloudDevice {
                devid: text(d, "devid")?,
                name: text(d, "name"),
                kind: text(d, "type"),
                version: text(d, "version"),
                sn: text(d, "sn"),
                soc: number(d, "soc"),
                charge: number(d, "charge"),
                discharge: number(d, "discharge"),
                load: number(d, "load"),
                profit: number(d, "profit"),
                capacity_kwh: number(d, "capacity_kwh"),
                status: text(d, "status"),
            })
        })
        .collect())
}

fn check(email: &str, password: &str) -> Result<(), String> {
    if email.is_empty() || email.len() > 254 || email.chars().any(char::is_control) {
        return Err("adresse e-mail invalide".into());
    }
    if password.is_empty() || password.len() > 256 {
        return Err("mot de passe invalide".into());
    }
    Ok(())
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ENTRY).map_err(|_| STORE_ERROR.to_owned())
}

fn session() -> Result<Option<Session>, String> {
    match entry()?.get_password() {
        Ok(json) => Ok(serde_json::from_str(&json).ok()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(STORE_ERROR.into()),
    }
}

pub fn login(email: &str, password: &str, region: Region) -> Result<(), String> {
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
    let json = serde_json::to_string(&Session { email: email.to_owned(), region, token })
        .map_err(|_| STORE_ERROR.to_owned())?;
    entry()?.set_password(&json).map_err(|_| STORE_ERROR.to_owned())
}

pub fn devices() -> Result<Vec<CloudDevice>, String> {
    let s = session()?.ok_or(EXPIRED)?;
    let resp = agent()
        .get(format!("https://{}/ems/api/v1/getDeviceList", s.region.host()))
        .query("token", &s.token)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401 | 403) => EXPIRED.to_owned(),
            e => network_error(&e),
        })?;
    parse_devices(&read_body(resp)?)
}

pub fn logout() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(STORE_ERROR.into()),
    }
}

pub fn status() -> Result<Option<Status>, String> {
    Ok(session()?.map(|s| Status { email: s.email, region: s.region }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_is_lowercase_hex() {
        assert_eq!(md5_hex("password"), "5f4dcc3b5aa765d61d8327deb882cf99");
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn login_reads_the_token_and_never_echoes_the_server() {
        assert_eq!(parse_login(r#"{"code":1,"token":"abc123"}"#).unwrap(), "abc123");
        let err = parse_login(r#"{"code":0,"msg":"secret-from-server"}"#).unwrap_err();
        assert!(err.contains("refusée") && !err.contains("secret-from-server"));
        assert!(parse_login(r#"{"token":""}"#).is_err());
        assert!(parse_login("<html>").unwrap_err().contains("illisible"));
    }

    const DEVICES: &str = r#"{"code":1,"data":[
        {"devid":"d1","name":"Venus","type":"HME-4","version":"153","sn":"S1","soc":"76","charge":120.5,
         "discharge":0,"load":"310","profit":1.2,"capacity_kwh":"5.12","status":1},
        {"devid":"d2","type":"HME-3","name":"Compteur"},
        {"name":"sans identifiant"}
    ]}"#;

    #[test]
    fn devices_skip_ignored_types_and_accept_numbers_as_strings() {
        let list = parse_devices(DEVICES).unwrap();
        assert_eq!(list.len(), 1);
        let d = &list[0];
        assert_eq!((d.devid.as_str(), d.kind.as_deref(), d.status.as_deref()), ("d1", Some("HME-4"), Some("1")));
        assert_eq!((d.soc, d.charge, d.load, d.capacity_kwh), (Some(76.0), Some(120.5), Some(310.0), Some(5.12)));
        assert_eq!(d.discharge, Some(0.0));
    }

    #[test]
    fn refused_token_codes_mean_expired_session() {
        for code in ["-1", "401", "403", "8", "\"8\""] {
            let body = format!(r#"{{"code":{code},"data":[]}}"#);
            assert_eq!(parse_devices(&body).unwrap_err(), EXPIRED, "{code}");
        }
        assert!(parse_devices(r#"{"code":1}"#).unwrap_err().contains("inattendue"));
        assert!(parse_devices("nope").is_err());
    }

    #[test]
    fn serialized_device_uses_the_ipc_names() {
        let json = serde_json::to_value(&parse_devices(DEVICES).unwrap()[0]).unwrap();
        assert_eq!(json["type"], "HME-4");
        assert_eq!(json["capacityKwh"], 5.12);
        assert!(json.get("kind").is_none());
    }

    #[test]
    fn inputs_are_validated() {
        assert!(check("a@b.fr", "pw").is_ok());
        assert!(check("", "pw").is_err());
        assert!(check("a@b.fr\n", "pw").is_err());
        assert!(check("a@b.fr", "").is_err());
        assert!(check(&"x".repeat(255), "pw").is_err());
    }

    #[test]
    fn region_is_a_closed_list_of_https_hosts() {
        assert_eq!(serde_json::from_str::<Region>("\"eu\"").unwrap().host(), "eu.hamedata.com");
        assert!(serde_json::from_str::<Region>("\"evil.example\"").is_err());
    }
}
