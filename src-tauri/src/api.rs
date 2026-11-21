//! Client pour l'Open API locale de Marstek (JSON-RPC sur UDP).
//!
//! Doc officielle : « Marstek Device Local API » (Rev 3.1.2).
//! La doc contient des incohérences (unités, noms de champs) : tous les champs
//! sont donc optionnels et à vérifier sur ta Venus D.

use crate::i18n::Msg;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fmt;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_PORT: u16 = 30000;
/// Nombre de créneaux Manuel (`time_num` 0 à 9).
pub const SLOTS: usize = 10;
/// Signe de la puissance en mode Manuel et Passif : non documenté, charge = négatif
/// (à vérifier sur la batterie ; c'est le seul endroit à changer).
pub const CHARGE_SIGN: i32 = -1;
/// Attente de la réponse à chaque tentative : un Wi-Fi faible perd des paquets, on renvoie plutôt que d'attendre.
const WAITS: [Duration; 3] = [Duration::from_millis(1500), Duration::from_secs(2), Duration::from_secs(3)];
/// Requêtes retenues pour juger la qualité de la liaison.
const WINDOW: usize = 20;
const SCAN_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum ApiError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Timeout,
    /// L'IP répond (ICMP « port fermé ») mais rien n'écoute sur ce port.
    PortClosed(u16),
    Device { code: i64, message: String },
    Invalid(Msg),
}

impl From<&ApiError> for Msg {
    fn from(e: &ApiError) -> Self {
        match e {
            ApiError::Io(e) => Msg::Io(e.to_string()),
            ApiError::Json(e) => Msg::BadReply(e.to_string()),
            ApiError::Timeout => Msg::NoReply,
            ApiError::PortClosed(port) => Msg::PortClosed(*port),
            ApiError::Device { code, message } => Msg::DeviceRefused { code: *code, message: message.clone() },
            ApiError::Invalid(msg) => msg.clone(),
        }
    }
}

/// Dans la langue courante de l'interface (voir `i18n::lang`).
impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Msg::from(self).fmt(f)
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

// ---------- Réponses ----------

/// Lit l'API en snake_case et sert le front en camelCase ; les champs absents ne sont pas émis.
macro_rules! readings {
    ($($name:ident { $($field:ident: $ty:ty),* $(,)? })*) => {$(
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        #[serde(rename_all(serialize = "camelCase", deserialize = "snake_case"))]
        pub struct $name {
            $(
                #[serde(skip_serializing_if = "Option::is_none")]
                pub $field: Option<$ty>,
            )*
        }
    )*};
}

readings! {
    DeviceInfo { device: String, ver: Value, ble_mac: String, wifi_mac: String, wifi_name: String, ip: String }
    WifiStatus { ssid: String, rssi: f64, sta_ip: String, wifi_mac: String }
    BleStatus { state: String, ble_mac: String }
    BatStatus {
        soc: f64, bat_temp: f64, bat_capacity: f64, rated_capacity: f64,
        charg_flag: bool, dischrg_flag: bool,
    }
    PvStatus {
        pv1_power: f64, pv1_voltage: f64, pv1_current: f64,
        pv2_power: f64, pv2_voltage: f64, pv2_current: f64,
        pv3_power: f64, pv3_voltage: f64, pv3_current: f64,
        pv4_power: f64, pv4_voltage: f64, pv4_current: f64,
    }
    EsStatus {
        bat_soc: f64, bat_cap: f64, pv_power: f64, ongrid_power: f64, offgrid_power: f64,
        bat_power: f64, total_pv_energy: f64, total_grid_output_energy: f64,
        total_grid_input_energy: f64, total_load_energy: f64,
    }
    EsMode { mode: String, ongrid_power: f64, offgrid_power: f64, bat_soc: f64 }
    EmStatus {
        ct_state: i64, a_power: f64, b_power: f64, c_power: f64, total_power: f64,
        input_energy: f64, output_energy: f64,
    }
}

/// Batterie trouvée par le balayage.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub ip: String,
    pub port: u16,
    pub name: Option<String>,
    pub version: Option<String>,
    pub wifi_mac: Option<String>,
}

impl Device {
    fn new(from: SocketAddr, info: DeviceInfo) -> Self {
        Self {
            ip: from.ip().to_string(),
            port: from.port(),
            name: info.device,
            version: info.ver.map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)),
            wifi_mac: info.wifi_mac,
        }
    }
}

// ---------- Modes ----------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Charge,
    Discharge,
}

impl Direction {
    fn signed(self, power: u16) -> i32 {
        let charge = CHARGE_SIGN * i32::from(power);
        match self {
            Self::Charge => charge,
            Self::Discharge => -charge,
        }
    }
}

/// Demande de mode venue de l'interface.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ModeReq {
    Auto,
    Ai,
    Ups,
    Passive { direction: Direction, power: u16, seconds: u32 },
}

impl ModeReq {
    pub fn config(&self) -> ModeConfig {
        match *self {
            Self::Auto => ModeConfig::Auto,
            Self::Ai => ModeConfig::Ai,
            Self::Ups => ModeConfig::Ups,
            Self::Passive { direction, power, seconds } => {
                ModeConfig::Passive { power: direction.signed(power), cd_time: seconds }
            }
        }
    }
}

/// Créneau du mode Manuel tel que l'interface le manipule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub start: String,
    pub end: String,
    /// Masque des jours, bit 0 = lundi.
    pub days: u8,
    pub direction: Direction,
    pub power: u16,
    pub enabled: bool,
}

/// Complète à 10 créneaux : l'API ne relit pas les créneaux, chaque envoi doit tout réécrire.
pub fn manual_slots(slots: &[Slot]) -> Result<Vec<ManualSlot>, ApiError> {
    if slots.len() > SLOTS {
        return Err(ApiError::Invalid(Msg::TooManySlots(slots.len())));
    }
    Ok((0..SLOTS as u8)
        .map(|n| match slots.get(usize::from(n)) {
            Some(s) => ManualSlot {
                time_num: n,
                start: s.start.clone(),
                end: s.end.clone(),
                week_set: s.days,
                power: s.direction.signed(s.power),
                enable: s.enabled,
            },
            None => ManualSlot {
                time_num: n,
                start: "00:00".into(),
                end: "00:00".into(),
                week_set: 127,
                power: 0,
                enable: false,
            },
        })
        .collect())
}

#[derive(Debug, Clone)]
pub struct ManualSlot {
    /// Créneau 0 à 9.
    pub time_num: u8,
    /// "hh:mm"
    pub start: String,
    /// "hh:mm"
    pub end: String,
    /// Masque des jours, bit 0 = lundi (127 = tous les jours).
    pub week_set: u8,
    /// Puissance en W, signée (voir `CHARGE_SIGN`).
    pub power: i32,
    pub enable: bool,
}

#[derive(Debug, Clone)]
pub enum ModeConfig {
    Auto,
    Ai,
    Ups,
    Manual(ManualSlot),
    /// Puissance en W et durée (compte à rebours) en secondes.
    Passive { power: i32, cd_time: u32 },
}

impl ModeConfig {
    /// Valeur de `mode` dans `ES.SetMode` et `ES.GetMode`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Ai => "AI",
            Self::Ups => "UPS",
            Self::Manual(_) => "Manual",
            Self::Passive { .. } => "Passive",
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Auto => json!({ "mode": "Auto", "auto_cfg": { "enable": 1 } }),
            Self::Ai => json!({ "mode": "AI", "ai_cfg": { "enable": 1 } }),
            Self::Ups => json!({ "mode": "UPS", "ups_cfg": { "enable": 1 } }),
            Self::Manual(s) => json!({
                "mode": "Manual",
                "manual_cfg": {
                    "time_num": s.time_num,
                    "start_time": s.start,
                    "end_time": s.end,
                    "week_set": s.week_set,
                    "power": s.power,
                    "enable": u8::from(s.enable),
                }
            }),
            Self::Passive { power, cd_time } => json!({
                "mode": "Passive",
                "passive_cfg": { "power": power, "cd_time": cd_time }
            }),
        }
    }
}

// ---------- Client ----------

pub struct Client {
    sock: UdpSocket,
    addr: SocketAddr,
    next_id: u16,
    pub(crate) waits: [Duration; 3],
    /// Une entrée par requête : le RTT de la tentative réussie, `None` si elle a échoué.
    recent: VecDeque<Option<Duration>>,
    /// Les requêtes depuis le dernier `begin_poll`.
    requests: Vec<Request>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Ok,
    /// La batterie a répondu par une erreur.
    Refused,
    /// Aucune réponse (ou envoi impossible).
    Timeout,
    PortClosed,
}

/// Une requête envoyée à la batterie, pour le détail du dernier sondage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub method: String,
    /// RTT de la tentative qui a obtenu une réponse, refus compris.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    pub outcome: Outcome,
}

type Sent = Result<(Result<Value, ApiError>, Duration), ApiError>;

impl Request {
    fn of(method: &str, sent: &Sent) -> Self {
        let (rtt, outcome) = match sent {
            Ok((reply, rtt)) => (Some(rtt.as_millis() as u32), if reply.is_ok() { Outcome::Ok } else { Outcome::Refused }),
            Err(ApiError::PortClosed(_)) => (None, Outcome::PortClosed),
            Err(_) => (None, Outcome::Timeout),
        };
        Self { method: method.to_owned(), rtt_ms: rtt, outcome }
    }
}

/// Qualité de la liaison, d'après les dernières requêtes.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub ok: u32,
    pub total: u32,
    /// Médiane des RTT des requêtes réussies, en ms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rssi: Option<f64>,
}

impl Link {
    fn of<'a>(recent: impl Iterator<Item = &'a Option<Duration>>) -> Self {
        let (mut ok, mut total, mut rtts) = (0, 0, Vec::new());
        for r in recent {
            total += 1;
            if let Some(rtt) = r {
                ok += 1;
                rtts.push(rtt.as_millis() as u32);
            }
        }
        rtts.sort_unstable();
        Self { ok, total, rtt_ms: rtts.get(rtts.len() / 2).copied(), rssi: None }
    }
}

impl Client {
    pub fn new(addr: SocketAddr) -> Result<Self, ApiError> {
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        // Départ aléatoire : une réponse tardive d'une session précédente ne porte pas l'id attendu.
        // La batterie ne renvoie que les 16 bits bas de l'id : au-delà, sa réponse ne correspondrait jamais.
        let next_id = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as u16);
        Ok(Self { sock, addr, next_id, waits: WAITS, recent: VecDeque::new(), requests: Vec::new() })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn link(&self) -> Link {
        Link::of(self.recent.iter())
    }

    pub fn begin_poll(&mut self) {
        self.requests.clear();
    }

    pub fn requests(&self) -> &[Request] {
        &self.requests
    }

    /// Envoie une requête et attend la réponse portant le même `id`, en la renvoyant (même `id`) si elle tarde :
    /// la réponse à une tentative précédente reste acceptée.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ApiError> {
        self.call_with(method, params, self.waits)
    }

    /// Une lecture n'est pas renvoyée (une batterie surchargée décroche) : elle sera refaite au cycle suivant.
    /// Attente longue (6 s) : ES.GetStatus répond parfois après 2 s, une réponse tardive vaut mieux qu'un renvoi.
    fn read(&mut self, method: &str, params: Value) -> Result<Value, ApiError> {
        self.call_with(method, params, [self.waits[2] * 2])
    }

    fn call_with(&mut self, method: &str, params: Value, waits: impl IntoIterator<Item = Duration>) -> Result<Value, ApiError> {
        self.next_id = self.next_id.wrapping_add(1);
        let req = serde_json::to_vec(&json!({ "id": self.next_id, "method": method, "params": params }))?;
        let sent = self.attempts(u64::from(self.next_id), &req, waits);
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(sent.as_ref().ok().map(|(_, rtt)| *rtt));
        self.requests.push(Request::of(method, &sent));
        sent?.0
    }

    /// Une réponse de la batterie, même un refus, prouve que la liaison marche ; son RTT est celui de la tentative réussie.
    fn attempts(
        &self,
        id: u64,
        req: &[u8],
        waits: impl IntoIterator<Item = Duration>,
    ) -> Sent {
        for wait in waits {
            let sent = Instant::now();
            self.sock.send_to(req, self.addr)?;
            if let Some(reply) = self.await_reply(id, sent + wait)? {
                return Ok((reply, sent.elapsed()));
            }
        }
        Err(ApiError::Timeout)
    }

    /// `Ok(None)` : rien avant l'échéance.
    fn await_reply(&self, id: u64, deadline: Instant) -> Result<Option<Result<Value, ApiError>>, ApiError> {
        let mut buf = [0u8; 4096];
        while let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
            self.sock.set_read_timeout(Some(left))?;
            let (n, from) = match self.sock.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
                // Sous Windows l'ICMP « port fermé » qui suit l'envoi fait échouer recv_from : l'IP répond, le port est fermé.
                Err(e) if e.kind() == ErrorKind::ConnectionReset => return Err(ApiError::PortClosed(self.addr.port())),
                Err(e) => return Err(e.into()),
            };
            if from.ip() != self.addr.ip() {
                continue;
            }
            let Ok(resp) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
            if resp.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // réponse à une ancienne requête
            }
            return Ok(Some(match resp.get("error") {
                Some(err) => Err(ApiError::Device {
                    code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: err.get("message").and_then(Value::as_str).unwrap_or("?").to_owned(),
                }),
                None => Ok(resp.get("result").cloned().unwrap_or(Value::Null)),
            }));
        }
        Ok(None)
    }

    fn get<T: DeserializeOwned>(&mut self, method: &str) -> Result<T, ApiError> {
        Ok(serde_json::from_value(self.read(method, json!({ "id": 0 }))?)?)
    }

    fn set(&mut self, method: &str, params: Value) -> Result<bool, ApiError> {
        let r = self.call(method, params)?;
        Ok(r.get("set_result").and_then(Value::as_bool).unwrap_or(false))
    }

    pub fn device_info(&mut self) -> Result<DeviceInfo, ApiError> {
        Ok(serde_json::from_value(self.read("Marstek.GetDevice", json!({ "ble_mac": "0" }))?)?)
    }

    pub fn wifi_status(&mut self) -> Result<WifiStatus, ApiError> {
        self.get("Wifi.GetStatus")
    }

    pub fn ble_status(&mut self) -> Result<BleStatus, ApiError> {
        self.get("BLE.GetStatus")
    }

    /// Lecture sentinelle du sondage : renvoyée comme une écriture, une seule perte ne doit pas faire croire la batterie hors ligne.
    pub fn bat_status(&mut self) -> Result<BatStatus, ApiError> {
        Ok(serde_json::from_value(self.call("Bat.GetStatus", json!({ "id": 0 }))?)?)
    }

    pub fn pv_status(&mut self) -> Result<PvStatus, ApiError> {
        self.get("PV.GetStatus")
    }

    pub fn es_status(&mut self) -> Result<EsStatus, ApiError> {
        self.get("ES.GetStatus")
    }

    pub fn es_mode(&mut self) -> Result<EsMode, ApiError> {
        self.get("ES.GetMode")
    }

    pub fn em_status(&mut self) -> Result<EmStatus, ApiError> {
        self.get("EM.GetStatus")
    }

    pub fn set_mode(&mut self, cfg: &ModeConfig) -> Result<bool, ApiError> {
        self.set("ES.SetMode", json!({ "id": 0, "config": cfg.to_json() }))
    }

    /// Profondeur de décharge, 30 à 88 % (firmware ≥ 150).
    pub fn set_dod(&mut self, value: u8) -> Result<bool, ApiError> {
        self.set("DOD.SET", json!({ "value": value.clamp(30, 88) }))
    }

    pub fn set_led(&mut self, on: bool) -> Result<bool, ApiError> {
        self.set("Led.Ctrl", json!({ "state": u8::from(on) }))
    }

    /// `Ble.Adv` : `enable` 0 = publicité activée, 1 = désactivée.
    pub fn set_ble(&mut self, on: bool) -> Result<bool, ApiError> {
        self.set("Ble.Adv", json!({ "enable": u8::from(!on) }))
    }
}

// ---------- Balayage ----------

/// La batterie connue par son adresse MAC Wi-Fi, retrouvée à une autre IP (bail DHCP changé).
pub fn find_moved<'a>(found: &'a [Device], wifi_mac: &str, current: IpAddr) -> Option<&'a Device> {
    let norm = |m: &str| m.chars().filter(char::is_ascii_hexdigit).collect::<String>().to_ascii_lowercase();
    let mac = norm(wifi_mac);
    found.iter().find(|d| !mac.is_empty() && d.wifi_mac.as_deref().map(norm) == Some(mac.clone()) && d.ip != current.to_string())
}

/// Plafond du balayage manuel, en adresses.
const MAX_MANUAL: usize = 65_534;
/// Plafond par interface en balayage automatique : un /22.
const MAX_AUTO_PREFIX: u32 = 22;
/// Plafond d'une étape : adresses × ports.
const MAX_PROBES: usize = 2_000_000;
/// Au-delà, une seule passe : la batterie perd environ un paquet sur trois, mais deux passes doubleraient la durée.
const TWO_PASSES_MAX: usize = 5_000;
const PASS_WAIT: Duration = Duration::from_secs(1);
/// Cadence globale d'envoi en paquets par seconde : au-delà, le Wi-Fi décroche.
const RATE: f64 = 20_000.0;
/// Cadence par IP cible : au-delà, la pile réseau de la batterie décroche et son API s'arrête.
const HOST_RATE: f64 = 50.0;
/// Recherche automatique du port d'une batterie connue : au plus ce nombre de ports, une requête par `PROBE_GAP`.
const MAX_FIND: usize = 20;
const PROBE_GAP: Duration = Duration::from_secs(1);
/// Sondes envoyées entre deux pauses de cadence, tests d'annulation et rapports.
const BATCH: u64 = 100;
const REPORT_EVERY: Duration = Duration::from_millis(200);

/// Une plage `192.168.1.1-254` ou `192.168.1.0/24` (préfixes 16 à 30, hors adresses réseau et broadcast).
fn parse_one(s: &str) -> Result<Vec<Ipv4Addr>, ApiError> {
    let bad = || ApiError::Invalid(Msg::BadRange(s.to_owned()));
    if let Some((ip, prefix)) = s.split_once('/') {
        let ip: Ipv4Addr = ip.trim().parse().map_err(|_| bad())?;
        let prefix: u32 = prefix.trim().parse().map_err(|_| bad())?;
        if !(16..=30).contains(&prefix) {
            return Err(bad());
        }
        Ok(subnet(ip, prefix, 16).0)
    } else if let Some((ip, last)) = s.split_once('-') {
        let ip: Ipv4Addr = ip.trim().parse().map_err(|_| bad())?;
        let last: u8 = last.trim().parse().map_err(|_| bad())?;
        let [a, b, c, first] = ip.octets();
        if last < first {
            return Err(bad());
        }
        Ok((first..=last).map(|d| Ipv4Addr::new(a, b, c, d)).collect())
    } else {
        Err(bad())
    }
}

/// Plusieurs plages séparées par des virgules, dédoublonnées, au plus `MAX_MANUAL` adresses.
pub fn parse_range(s: &str) -> Result<Vec<Ipv4Addr>, ApiError> {
    let mut all = BTreeSet::new();
    for part in s.split(',') {
        all.extend(parse_one(part.trim())?);
        if all.len() > MAX_MANUAL {
            return Err(ApiError::Invalid(Msg::RangeTooWide(MAX_MANUAL)));
        }
    }
    Ok(all.into_iter().collect())
}

/// Hôtes et adresse de broadcast du sous-réseau de `ip` ; un masque plus large que `/widest` est ramené à `/widest` autour de `ip`.
fn subnet(ip: Ipv4Addr, prefix: u32, widest: u32) -> (Vec<Ipv4Addr>, Ipv4Addr) {
    let size = 1u32 << (32 - prefix.clamp(widest, 30));
    let base = u32::from(ip) & !(size - 1);
    ((base + 1..base + size - 1).map(Ipv4Addr::from).collect(), Ipv4Addr::from(base + size - 1))
}

/// Ce qui sera balayé : IP sondées, broadcasts dirigés, et un libellé par plage pour le journal.
struct Targets {
    hosts: BTreeSet<Ipv4Addr>,
    broadcasts: Vec<Ipv4Addr>,
    sources: Vec<String>,
}

/// Le sous-réseau de chaque interface IPv4 active (Wi-Fi, Ethernet, VPN…), hors loopback et lien local.
fn auto_targets() -> Result<Targets, ApiError> {
    let mut t = Targets { hosts: BTreeSet::new(), broadcasts: Vec::new(), sources: Vec::new() };
    for itf in if_addrs::get_if_addrs()? {
        let if_addrs::IfAddr::V4(v4) = &itf.addr else { continue };
        if itf.is_loopback() || v4.ip.is_link_local() {
            continue;
        }
        let prefix = u32::from(v4.netmask).count_ones().max(MAX_AUTO_PREFIX);
        let (hosts, broadcast) = subnet(v4.ip, prefix, MAX_AUTO_PREFIX);
        t.sources.push(format!("{} ({}/{prefix})", itf.name, v4.ip));
        t.hosts.extend(hosts);
        t.broadcasts.push(broadcast);
    }
    if t.hosts.is_empty() {
        return Err(ApiError::Invalid(Msg::NoInterface));
    }
    Ok(t)
}

/// Ports de `s` : « 30000, 49152-65535 », de 1 à 65535, sans doublon, dans l'ordre saisi. Vide : aucun port.
pub fn parse_ports(s: &str) -> Result<Vec<u16>, ApiError> {
    let bad = |p: &str| ApiError::Invalid(Msg::BadPort(p.to_owned()));
    let num = |p: &str| p.trim().parse::<u16>().ok().filter(|&n| n != 0);
    let mut seen = HashSet::new();
    let mut ports = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (first, last) = match part.split_once('-') {
            Some((a, b)) => (num(a), num(b)),
            None => (num(part), num(part)),
        };
        let (Some(first), Some(last)) = (first, last) else { return Err(bad(part)) };
        if first > last {
            return Err(bad(part));
        }
        ports.extend((first..=last).filter(|&p| seen.insert(p)));
    }
    Ok(ports)
}

fn check_probes(hosts: usize, ports: usize) -> Result<(), ApiError> {
    let n = hosts * ports;
    if n > MAX_PROBES {
        return Err(ApiError::Invalid(Msg::ScanTooBig { hosts, ports, n, max: MAX_PROBES }));
    }
    Ok(())
}

/// Les ports en plages triées, pour le journal et l'étape affichée : « 30000, 49152-65535 ».
fn ranges(ports: &[u16]) -> String {
    let mut sorted = ports.to_vec();
    sorted.sort_unstable();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i;
        while j + 1 < sorted.len() && sorted[j + 1] == sorted[j] + 1 {
            j += 1;
        }
        parts.push(if j == i { sorted[i].to_string() } else { format!("{}-{}", sorted[i], sorted[j]) });
        i = j + 1;
    }
    parts.join(", ")
}

/// Le réglage `ports` complété des ports où un appareil a répondu ; `None` si tous y figuraient déjà.
pub fn with_found_ports(setting: &str, devices: &[Device]) -> Option<String> {
    let known: HashSet<u16> = parse_ports(setting).ok()?.into_iter().collect();
    let mut new: Vec<u16> = devices.iter().map(|d| d.port).filter(|p| !known.contains(p)).collect();
    new.sort_unstable();
    new.dedup();
    if new.is_empty() {
        return None;
    }
    let added = ranges(&new);
    let base = setting.trim().trim_end_matches(',').trim_end();
    Some(if base.is_empty() { added } else { format!("{base}, {added}") })
}

/// Les IPv4 de `ips` qui sont des hôtes d'un sous-réseau balayé (ni broadcast, ni multicast, ni autre réseau).
fn alive_hosts(ips: impl IntoIterator<Item = Ipv4Addr>, hosts: &BTreeSet<Ipv4Addr>) -> Vec<Ipv4Addr> {
    ips.into_iter().filter(|ip| hosts.contains(ip)).collect::<BTreeSet<_>>().into_iter().collect()
}

/// Les IPv4 de la table ARP de Windows (`arp -a`) : les voisins qui ont répondu récemment.
/// shortcut: sortie texte de `arp -a` plutôt que GetIpNetTable2, qui demande windows-sys en dépendance directe.
fn arp_neighbours() -> Vec<Ipv4Addr> {
    let mut cmd = std::process::Command::new("arp");
    cmd.arg("-a");
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
    let Ok(out) = cmd.output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout).split_whitespace().filter_map(|w| w.parse().ok()).collect()
}

/// Un seul ping ICMP : la machine répond-elle sur le réseau ? (hors port UDP de la batterie)
pub fn ping(ip: IpAddr) -> bool {
    let mut cmd = std::process::Command::new("ping");
    cmd.args(if cfg!(windows) { ["-n", "1", "-w", "1000"] } else { ["-c", "1", "-W", "1"] }).arg(ip.to_string());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
    // Le code de sortie seul ne suffit pas : Windows rend 0 pour « hôte inaccessible » venant de la passerelle.
    cmd.output().is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).to_ascii_uppercase().contains("TTL="))
}

/// Avancement d'un balayage, émis environ 5 fois par seconde.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: String,
    pub sent: u64,
    pub total: u64,
    pub found: usize,
}

/// Résultat d'un balayage et ce qui a été sondé (dernière étape jouée).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub devices: Vec<Device>,
    pub addresses: usize,
    pub sources: Vec<String>,
    pub ports: String,
    /// Le réglage `ports` complété, quand un appareil a répondu sur un port qu'il ne couvrait pas.
    pub saved_ports: Option<String>,
}

/// Cadence globale qui tient `HOST_RATE` par IP : chaque broadcast atteint tous les hôtes en plus de leur propre sonde.
fn scan_rate(hosts: usize, broadcasts: usize) -> f64 {
    (HOST_RATE * (hosts + broadcasts) as f64 / (1 + broadcasts) as f64).min(RATE)
}

fn passes(hosts: usize, ports: usize) -> u64 {
    if hosts * ports <= TWO_PASSES_MAX { 2 } else { 1 }
}

/// Pour chaque port, toutes les cibles : une IP reçoit au plus une sonde par tour de ports.
fn probes<'a>(targets: &'a [Ipv4Addr], ports: &'a [u16]) -> impl Iterator<Item = (Ipv4Addr, u16)> + 'a {
    ports.iter().flat_map(move |&p| targets.iter().map(move |&ip| (ip, p)))
}

/// Durée d'une étape, pour le journal : « 55 s » ou « 2 min 43 s ».
fn eta(hosts: usize, broadcasts: usize, ports: usize) -> String {
    let passes = passes(hosts, ports);
    let sending = ((hosts + broadcasts) * ports) as f64 * passes as f64 / scan_rate(hosts, broadcasts);
    let secs = (sending + (passes - 1) as f64 * PASS_WAIT.as_secs_f64() + SCAN_WAIT.as_secs_f64()).round() as u64;
    if secs < 90 { format!("{secs} s") } else { format!("{} min {:02} s", secs / 60, secs % 60) }
}

/// Ports qu'une recherche automatique peut essayer : 30000 et les ports unitaires du réglage (les plages sont ignorées),
/// `MAX_FIND` au plus. Un balayage plus large reste à la main de l'utilisateur.
pub fn unit_ports(setting: &str) -> Vec<u16> {
    let mut seen = HashSet::new();
    let saved = setting.split(',').filter_map(|p| p.trim().parse::<u16>().ok()).filter(|&p| p != 0);
    [DEFAULT_PORT].into_iter().chain(saved).filter(|&p| seen.insert(p)).take(MAX_FIND).collect()
}

fn device_request() -> Result<Vec<u8>, ApiError> {
    Ok(serde_json::to_vec(&json!({ "id": 1, "method": "Marstek.GetDevice", "params": { "ble_mac": "0" } }))?)
}

/// Lit le socket pendant l'envoi : sinon les réponses s'accumulent et le tampon du système déborde.
fn listen(sock: &UdpSocket, stop: &AtomicBool, found: &Mutex<Vec<Device>>) {
    let mut buf = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        // Sous Windows un ICMP « port fermé » fait échouer recv_from : on ignore et on continue.
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let Ok(resp) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
        let Some(result) = resp.get("result") else { continue };
        let mut found = found.lock().unwrap();
        if found.iter().any(|d| d.ip == from.ip().to_string() && d.port == from.port()) {
            continue;
        }
        let info = serde_json::from_value(result.clone()).unwrap_or_default();
        found.push(Device::new(from, info));
    }
}

/// Un socket, une requête `Marstek.GetDevice`, les appareils trouvés et le suivi d'avancement.
struct Radar<'a> {
    sock: UdpSocket,
    req: Vec<u8>,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(Progress),
    found: Mutex<Vec<Device>>,
    /// Adresses et ports de la dernière étape.
    last: (usize, String),
}

impl<'a> Radar<'a> {
    fn new(cancel: &'a AtomicBool, progress: &'a mut dyn FnMut(Progress)) -> Result<Self, ApiError> {
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        sock.set_broadcast(true)?;
        sock.set_read_timeout(Some(Duration::from_millis(100)))?;
        Ok(Self { sock, req: device_request()?, cancel, progress, found: Mutex::new(Vec::new()), last: (0, String::new()) })
    }

    /// Une étape : pour chaque port, broadcasts puis chaque IP, cadencé à `HOST_RATE` par IP, en une passe (deux si elle
    /// est courte : la batterie perd environ un paquet sur trois). L'id de la réponse n'est pas vérifié : la batterie le tronque à 16 bits.
    fn run(&mut self, stage: &str, hosts: &[Ipv4Addr], broadcasts: &[Ipv4Addr], ports: &[u16]) -> Result<(), ApiError> {
        check_probes(hosts.len(), ports.len())?;
        self.last = (hosts.len(), ranges(ports));
        let passes = passes(hosts.len(), ports.len());
        let rate = scan_rate(hosts.len(), broadcasts.len());
        let targets: Vec<Ipv4Addr> = broadcasts.iter().chain(hosts).copied().collect();
        let total = ((hosts.len() + broadcasts.len()) * ports.len()) as u64 * passes;
        let Self { sock, req, cancel, progress, found, .. } = self;
        let found = &*found;
        let reader = sock.try_clone()?;
        let stop = AtomicBool::new(false);
        let mut last_report = Instant::now();
        let mut sent = 0u64;
        let mut report = |sent: u64, force: bool| {
            if force || last_report.elapsed() >= REPORT_EVERY {
                last_report = Instant::now();
                progress(Progress { stage: stage.into(), sent, total, found: found.lock().unwrap().len() });
            }
        };
        std::thread::scope(|s| {
            s.spawn(|| listen(&reader, &stop, found));
            'run: for pass in 0..passes {
                let started = Instant::now();
                let mut n = 0u64;
                for probe in probes(&targets, ports) {
                    let _ = sock.send_to(req, probe);
                    n += 1;
                    sent += 1;
                    if n.is_multiple_of(BATCH) {
                        if cancel.load(Ordering::Relaxed) {
                            break 'run;
                        }
                        if let Some(rest) = Duration::from_secs_f64(n as f64 / rate).checked_sub(started.elapsed()) {
                            std::thread::sleep(rest);
                        }
                        report(sent, false);
                    }
                }
                let until = Instant::now() + if pass + 1 == passes { SCAN_WAIT } else { PASS_WAIT };
                while Instant::now() < until && !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                    report(sent, false);
                }
            }
            report(sent, true);
            stop.store(true, Ordering::Relaxed);
        });
        Ok(())
    }

    fn found(&self) -> bool {
        !self.found.lock().unwrap().is_empty()
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Balayage manuel (plage et ports saisis ; vides : sous-réseaux et port par défaut) ou, avec `auto`, recherche en
/// étapes qui s'arrêtent dès qu'un appareil répond : 1) sous-réseaux du PC, ports enregistrés et 30000 ;
/// 2) hôtes vivants (table ARP, remplie par l'étape 1) sur 49152-65535 ; 3) ces hôtes sur 1024-49151.
pub fn discover(range: Option<&str>, setting: &str, auto: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(Progress)) -> Result<Scan, ApiError> {
    let mut radar = Radar::new(cancel, progress)?;
    let range = range.map(str::trim).filter(|r| !r.is_empty() && !auto);
    let mut targets = match range {
        Some(r) => Targets {
            hosts: parse_range(r)?.into_iter().collect(),
            broadcasts: Vec::new(),
            sources: r.split(',').map(|p| p.trim().to_string()).collect(),
        },
        None => auto_targets()?,
    };
    targets.broadcasts.push(Ipv4Addr::BROADCAST);
    let hosts: Vec<Ipv4Addr> = targets.hosts.iter().copied().collect();
    let mut ports = if auto {
        let mut first = vec![DEFAULT_PORT];
        first.extend(parse_ports(setting).unwrap_or_default().into_iter().filter(|&p| p != DEFAULT_PORT));
        first.truncate((MAX_PROBES / hosts.len()).max(1));
        first
    } else {
        parse_ports(setting)?
    };
    if ports.is_empty() {
        ports.push(DEFAULT_PORT);
    }
    let wait = eta(hosts.len(), targets.broadcasts.len(), ports.len());
    let stage = if auto {
        Msg::ScanStage1 { ports: ranges(&ports), wait }.to_string()
    } else {
        Msg::ScanManual { hosts: hosts.len(), ports: ranges(&ports), wait }.to_string()
    };
    radar.run(&stage, &hosts, &targets.broadcasts, &ports)?;
    if auto && !radar.found() && !radar.cancelled() {
        // L'étape 1 a fait répondre les hôtes en ligne : la table ARP les liste.
        let alive = alive_hosts(arp_neighbours(), &targets.hosts);
        // Un IP:port déjà sondé à l'étape 1 n'est pas renvoyé.
        let mut done: HashSet<u16> = ports.iter().copied().collect();
        for (n, range) in [(2, 49152..=65535), (3, 1024..=49151)] {
            if alive.is_empty() || radar.found() || radar.cancelled() {
                break;
            }
            let ports: Vec<u16> = range.filter(|&p| done.insert(p)).collect();
            let stage = Msg::ScanStage { n, hosts: alive.len(), ports: ranges(&ports), wait: eta(alive.len(), 0, ports.len()) }.to_string();
            radar.run(&stage, &alive, &[], &ports)?;
        }
    }
    let Radar { found, last, .. } = radar;
    let devices = found.into_inner().unwrap();
    let saved_ports = with_found_ports(setting, &devices);
    Ok(Scan { devices, addresses: last.0, sources: targets.sources, ports: last.1, saved_ports })
}

/// Cherche l'API d'une batterie dont l'IP est connue sur `unit_ports` : une requête par port, une par `PROBE_GAP` ;
/// s'arrête à la première réponse.
pub fn find_port(ip: Ipv4Addr, setting: &str, cancel: &AtomicBool) -> Result<Option<Device>, ApiError> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    let req = device_request()?;
    let mut buf = [0u8; 4096];
    for port in unit_ports(setting) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        sock.send_to(&req, (ip, port))?;
        let until = Instant::now() + PROBE_GAP;
        while let Some(left) = until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
            sock.set_read_timeout(Some(left))?;
            // Sous Windows l'ICMP « port fermé » fait échouer recv_from : on ignore et on attend la fin du délai.
            let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
            if from != SocketAddr::new(ip.into(), port) {
                continue;
            }
            let Some(result) = serde_json::from_slice::<Value>(&buf[..n]).ok().and_then(|r| r.get("result").cloned()) else { continue };
            return Ok(Some(Device::new(from, serde_json::from_value(result).unwrap_or_default())));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(start: &str, end: &str, direction: Direction, power: u16) -> Slot {
        Slot { start: start.into(), end: end.into(), days: 127, direction, power, enabled: true }
    }

    #[test]
    fn call_skips_unreadable_datagram() {
        let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut client = Client::new(server.local_addr().unwrap()).unwrap();
        let t = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let id = serde_json::from_slice::<Value>(&buf[..n]).unwrap()["id"].clone();
            server.send_to(b"pas du json", from).unwrap();
            server.send_to(&serde_json::to_vec(&json!({ "id": id, "result": 7 })).unwrap(), from).unwrap();
        });
        assert_eq!(client.call("Test", json!({})).unwrap(), json!(7));
        t.join().unwrap();
    }

    /// Serveur loopback qui ignore les `skip` premiers paquets puis répond `result: 7` à chacun des suivants.
    fn flaky_server(skip: usize) -> (SocketAddr, std::thread::JoinHandle<Vec<Value>>) {
        let server = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        server.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        let addr = server.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (mut seen, mut buf) = (Vec::new(), [0u8; 4096]);
            while let Ok((n, from)) = server.recv_from(&mut buf) {
                let req: Value = serde_json::from_slice(&buf[..n]).unwrap();
                if seen.len() >= skip {
                    server.send_to(&serde_json::to_vec(&json!({ "id": req["id"], "result": 7 })).unwrap(), from).unwrap();
                }
                seen.push(req);
            }
            seen
        });
        (addr, t)
    }

    fn quick(addr: SocketAddr) -> Client {
        let mut c = Client::new(addr).unwrap();
        c.waits = [Duration::from_millis(60); 3];
        c
    }

    #[test]
    fn call_retries_with_the_same_id_after_a_lost_packet() {
        let (addr, server) = flaky_server(1);
        let mut client = quick(addr);
        assert_eq!(client.call("Test", json!({})).unwrap(), json!(7));
        let seen = server.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["id"], seen[1]["id"]);
        let link = client.link();
        assert_eq!((link.ok, link.total), (1, 1));
        assert!(link.rtt_ms.unwrap() < 60, "RTT mesuré sur la tentative réussie, pas depuis la première : {link:?}");
    }

    #[test]
    fn request_outcome_follows_the_error_kind() {
        let ms = Duration::from_millis(42);
        let device = || ApiError::Device { code: -32601, message: "Method not found".into() };
        let of = |sent: Sent| Request::of("Bat.GetStatus", &sent);
        assert_eq!((of(Ok((Ok(json!(1)), ms))).outcome, of(Ok((Ok(json!(1)), ms))).rtt_ms), (Outcome::Ok, Some(42)));
        assert_eq!((of(Ok((Err(device()), ms))).outcome, of(Ok((Err(device()), ms))).rtt_ms), (Outcome::Refused, Some(42)));
        assert_eq!((of(Err(ApiError::Timeout)).outcome, of(Err(ApiError::Timeout)).rtt_ms), (Outcome::Timeout, None));
        assert_eq!((of(Err(ApiError::PortClosed(1))).outcome, of(Err(ApiError::PortClosed(1))).rtt_ms), (Outcome::PortClosed, None));
        assert_eq!(of(Err(ApiError::Timeout)).method, "Bat.GetStatus");
    }

    #[test]
    fn begin_poll_keeps_only_the_requests_of_the_current_poll() {
        let (addr, _server) = flaky_server(usize::MAX);
        let mut client = quick(addr);
        let _ = client.call("A", json!({}));
        client.begin_poll();
        let _ = client.call("B", json!({}));
        let got: Vec<_> = client.requests().iter().map(|r| (r.method.as_str(), r.outcome)).collect();
        assert_eq!(got, [("B", Outcome::Timeout)]);
    }

    #[test]
    fn call_gives_up_after_three_attempts() {
        let (addr, server) = flaky_server(usize::MAX);
        let mut client = quick(addr);
        assert!(matches!(client.call("Test", json!({})), Err(ApiError::Timeout)));
        assert_eq!(server.join().unwrap().len(), 3);
        assert_eq!((client.link().ok, client.link().total, client.link().rtt_ms), (0, 1, None));
    }

    /// Seul Windows remonte l'ICMP « port fermé » d'un socket UDP non connecté (ConnectionReset).
    #[cfg(windows)]
    #[test]
    fn closed_port_fails_at_once_without_retry() {
        let addr = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap();
        let mut client = Client::new(addr).unwrap();
        let started = Instant::now();
        let err = client.call("Test", json!({})).unwrap_err();
        assert!(matches!(err, ApiError::PortClosed(p) if p == addr.port()), "{err}");
        assert!(started.elapsed() < Duration::from_millis(1000), "pas d'attente du timeout");
        assert!(err.to_string().contains(&format!("port {} closed", addr.port())));
        assert_eq!((client.link().ok, client.link().total), (0, 1));
    }

    #[test]
    fn read_sends_once_and_write_keeps_three_attempts() {
        let (addr, server) = flaky_server(usize::MAX);
        let mut client = quick(addr);
        assert!(matches!(client.get::<Value>("Test"), Err(ApiError::Timeout)));
        assert_eq!(server.join().unwrap().len(), 1);
        assert_eq!((client.link().ok, client.link().total), (0, 1));
    }

    #[test]
    fn link_keeps_the_last_twenty_requests_and_the_median_rtt() {
        let ms = Duration::from_millis;
        let outcomes: Vec<_> = [Some(ms(10)), None, Some(ms(300)), Some(ms(40))].into();
        assert_eq!(Link::of(outcomes.iter()), Link { ok: 3, total: 4, rtt_ms: Some(40), rssi: None });
        assert_eq!(Link::of([].iter()), Link::default());

        let (addr, server) = flaky_server(0);
        let mut client = quick(addr);
        for _ in 0..25 {
            client.call("Test", json!({})).unwrap();
        }
        assert_eq!(client.link().total, 20);
        assert_eq!(server.join().unwrap().len(), 25);
    }

    #[test]
    fn finds_the_same_battery_at_another_ip() {
        let dev = |ip: &str, mac: Option<&str>| Device {
            ip: ip.into(), port: 30000, name: None, version: None, wifi_mac: mac.map(Into::into),
        };
        let found = [dev("192.168.1.7", Some("AA:BB:CC:00:11:22")), dev("192.168.1.9", Some("aabbcc001122")), dev("192.168.1.8", None)];
        let current: IpAddr = "192.168.1.7".parse().unwrap();
        assert_eq!(find_moved(&found, "aabbcc001122", current).map(|d| d.ip.as_str()), Some("192.168.1.9"));
        assert!(find_moved(&found[..1], "aabbcc001122", current).is_none(), "même IP : rien à changer");
        assert!(find_moved(&found, "ffffffffffff", current).is_none());
        assert!(find_moved(&found, "", current).is_none());
    }

    #[test]
    fn passive_json_matches_doc() {
        let v = ModeConfig::Passive { power: 100, cd_time: 300 }.to_json();
        assert_eq!(v, json!({ "mode": "Passive", "passive_cfg": { "power": 100, "cd_time": 300 } }));
    }

    #[test]
    fn parses_es_status_with_missing_fields() {
        let s: EsStatus = serde_json::from_value(json!({ "id": 0, "bat_soc": 98, "pv_power": 0 })).unwrap();
        assert_eq!(s.bat_soc, Some(98.0));
        assert_eq!(s.bat_power, None);
    }

    #[test]
    fn auto_ai_ups_json() {
        assert_eq!(ModeConfig::Auto.to_json(), json!({ "mode": "Auto", "auto_cfg": { "enable": 1 } }));
        assert_eq!(ModeConfig::Ai.to_json(), json!({ "mode": "AI", "ai_cfg": { "enable": 1 } }));
        assert_eq!(ModeConfig::Ups.to_json(), json!({ "mode": "UPS", "ups_cfg": { "enable": 1 } }));
    }

    #[test]
    fn manual_json_matches_doc() {
        let s = ManualSlot {
            time_num: 2,
            start: "22:00".into(),
            end: "23:59".into(),
            week_set: 127,
            power: -300,
            enable: true,
        };
        assert_eq!(
            ModeConfig::Manual(s).to_json(),
            json!({ "mode": "Manual", "manual_cfg": {
                "time_num": 2, "start_time": "22:00", "end_time": "23:59",
                "week_set": 127, "power": -300, "enable": 1 } })
        );
    }

    #[test]
    fn charge_sign_applies_to_passive_and_manual() {
        let passive = |direction| ModeReq::Passive { direction, power: 500, seconds: 60 }.config();
        let ModeConfig::Passive { power, cd_time } = passive(Direction::Charge) else { panic!() };
        assert_eq!((power, cd_time), (500 * CHARGE_SIGN, 60));
        let ModeConfig::Passive { power, .. } = passive(Direction::Discharge) else { panic!() };
        assert_eq!(power, -500 * CHARGE_SIGN);

        let m = manual_slots(&[slot("00:00", "06:00", Direction::Charge, 300)]).unwrap();
        assert_eq!(m[0].power, 300 * CHARGE_SIGN);
    }

    #[test]
    fn mode_req_deserializes_from_front() {
        let m: ModeReq = serde_json::from_value(json!({ "kind": "ups" })).unwrap();
        assert!(matches!(m, ModeReq::Ups));
        let m: ModeReq =
            serde_json::from_value(json!({ "kind": "passive", "direction": "charge", "power": 200, "seconds": 90 })).unwrap();
        assert!(matches!(m, ModeReq::Passive { direction: Direction::Charge, power: 200, seconds: 90 }));
    }

    #[test]
    fn pads_to_ten_slots() {
        let m = manual_slots(&[slot("00:00", "06:00", Direction::Charge, 300)]).unwrap();
        assert_eq!(m.len(), SLOTS);
        assert_eq!(m.iter().map(|s| s.time_num).collect::<Vec<_>>(), (0..10).collect::<Vec<_>>());
        assert!(m[0].enable);
        assert!(m[1..].iter().all(|s| !s.enable && s.power == 0));
    }

    #[test]
    fn rejects_more_than_ten_slots() {
        let eleven = vec![slot("00:00", "01:00", Direction::Charge, 100); 11];
        assert!(manual_slots(&eleven).is_err());
    }

    #[test]
    fn parses_ip_range_dash() {
        let r = parse_range("192.168.1.10-12").unwrap();
        assert_eq!(r, ["192.168.1.10", "192.168.1.11", "192.168.1.12"].map(|s| s.parse::<Ipv4Addr>().unwrap()));
        assert_eq!(parse_range(" 10.0.0.1-254 ").unwrap().len(), 254);
        assert!(parse_range("192.168.1.20-10").is_err());
    }

    #[test]
    fn parses_ip_range_cidr() {
        let r = parse_range("192.168.1.77/24").unwrap();
        assert_eq!(r.len(), 254);
        assert_eq!(r[0], Ipv4Addr::new(192, 168, 1, 1));
        assert_eq!(r[253], Ipv4Addr::new(192, 168, 1, 254));
        assert_eq!(parse_range("10.0.0.0/30").unwrap().len(), 2);
        assert_eq!(parse_range("10.0.0.0/16").unwrap().len(), 65_534);
        assert!(parse_range("10.0.0.0/15").is_err());
        assert!(parse_range("10.0.0.0/31").is_err());
    }

    #[test]
    fn parses_several_ranges_without_duplicates() {
        let r = parse_range("192.168.1.10-12, 192.168.1.11-13 , 10.0.0.0/30").unwrap();
        assert_eq!(r.len(), 6);
        assert!(r.contains(&Ipv4Addr::new(192, 168, 1, 13)) && r.contains(&Ipv4Addr::new(10, 0, 0, 2)));
        assert!(parse_range("192.168.1.1-5,").is_err());
    }

    #[test]
    fn caps_total_addresses() {
        assert!(parse_range("10.0.0.0/16, 10.1.0.0/24").is_err());
        assert_eq!(parse_range("10.0.0.0/16, 10.0.5.0/24").unwrap().len(), 65_534);
    }

    #[test]
    fn caps_wide_interface_masks_to_a_22() {
        let (hosts, broadcast) = subnet(Ipv4Addr::new(10, 1, 2, 77), 8, MAX_AUTO_PREFIX);
        assert_eq!(hosts.len(), 1022);
        assert_eq!((hosts[0], broadcast), (Ipv4Addr::new(10, 1, 0, 1), Ipv4Addr::new(10, 1, 3, 255)));
        let (hosts, broadcast) = subnet(Ipv4Addr::new(192, 168, 1, 20), 24, MAX_AUTO_PREFIX);
        assert_eq!((hosts.len(), broadcast), (254, Ipv4Addr::new(192, 168, 1, 255)));
    }

    #[test]
    fn parses_ports_lists_and_ranges() {
        assert_eq!(parse_ports("49152, 30000-30002,49152 ,").unwrap(), [49152, 30000, 30001, 30002]);
        assert_eq!(parse_ports("1-65535").unwrap().len(), 65_535);
        assert_eq!(parse_ports("65535").unwrap(), [65535]);
        assert!(parse_ports(" ").unwrap().is_empty());
        for s in ["0", "0-10", "1-65536", "65536", "100-50", "abc", "1-", "-5", "1-2-3"] {
            assert!(parse_ports(s).is_err(), "{s}");
        }
    }

    #[test]
    fn scan_rate_keeps_each_target_under_the_per_host_cap() {
        // Un seul hôte et un broadcast : 2 paquets par tour, dont 2 pour l'hôte.
        assert_eq!(scan_rate(1, 1), HOST_RATE);
        assert_eq!(scan_rate(10, 0), 500.0);
        assert_eq!(scan_rate(65_534, 1), RATE);
        for (h, b) in [(1, 0), (1, 1), (10, 0), (66, 0), (254, 2), (1022, 2), (65_534, 1)] {
            let per_host = scan_rate(h, b) * (1 + b) as f64 / (h + b) as f64;
            assert!(per_host <= HOST_RATE + 1e-9, "{h} hôtes, {b} broadcasts : {per_host}");
        }
    }

    #[test]
    fn probes_visit_every_target_for_each_port_before_the_next_port() {
        let ips = [Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2), Ipv4Addr::new(10, 0, 0, 3)];
        let all: Vec<_> = probes(&ips, &[30000, 49152]).collect();
        assert_eq!(all.len(), 6);
        assert_eq!(all.iter().map(|p| p.1).collect::<Vec<_>>(), [30000, 30000, 30000, 49152, 49152, 49152]);
        assert_eq!(all.iter().collect::<HashSet<_>>().len(), 6, "aucun IP:port deux fois");
        assert!(all[..3].iter().map(|p| p.0).eq(ips), "un tour entier avant de revenir au même IP");
    }

    #[test]
    fn eta_follows_the_per_host_cap() {
        assert_eq!(eta(254, 2, 2), "4 s", "étape 1 : deux passes, rien à cadencer");
        assert_eq!(eta(1, 0, 16_384), "5 min 31 s", "un hôte : 16 384 ports à 50 par seconde, plus l'attente");
        assert_eq!(eta(41, 0, 48_128), "16 min 06 s");
        assert_eq!(eta(1_000, 0, 2_000), "1 min 43 s", "au-delà de 400 hôtes, c'est la cadence globale");
        assert_eq!(eta(1, 0, 65_535), "21 min 54 s");
    }

    #[test]
    fn unit_ports_are_30000_then_the_saved_single_ports_without_ranges() {
        assert_eq!(unit_ports("40000, 49152-65535, 40001, 40000, 0, 70000, abc"), [30000, 40000, 40001]);
        assert_eq!(unit_ports(""), [30000]);
        assert_eq!(unit_ports("30000"), [30000]);
        let many = (40000..40100).map(|p| p.to_string()).collect::<Vec<_>>().join(",");
        let p = unit_ports(&many);
        assert_eq!((p.len(), p[..2].to_vec()), (MAX_FIND, vec![30000, 40000]), "20 ports au plus, 30000 d'abord");
    }

    #[test]
    fn find_port_sends_one_request_per_saved_port_and_finds_the_battery() {
        let server = UdpSocket::bind((Ipv4Addr::new(127, 0, 0, 3), 0)).unwrap();
        server.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let port = server.local_addr().unwrap().port();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                let mut seen = 0;
                while !stop.load(Ordering::Relaxed) {
                    if let Ok((_, from)) = server.recv_from(&mut buf) {
                        seen += 1;
                        let _ = server.send_to(br#"{"id":1,"result":{"device":"VenusD","ver":147}}"#, from);
                    }
                }
                seen
            })
        };
        // 127.0.0.3 : l'autre test balaie 127.0.0.2 autour de son port. La plage qui contient le port n'est pas balayée.
        let setting = format!("{}-{}, {port}", port.saturating_sub(30).max(1), port.saturating_add(30));
        let started = Instant::now();
        let found = find_port(Ipv4Addr::new(127, 0, 0, 3), &setting, &AtomicBool::new(false)).unwrap();
        stop.store(true, Ordering::Relaxed);
        assert_eq!(found.map(|d| (d.ip, d.port)), Some(("127.0.0.3".to_owned(), port)));
        assert_eq!(thread.join().unwrap(), 1, "une seule requête reçue");
        // 30000 (fermé) attend sa seconde, puis le port unitaire répond.
        assert!(started.elapsed() >= PROBE_GAP);
    }

    #[test]
    fn caps_probes() {
        assert!(check_probes(122, 16_384).is_ok());
        assert!(check_probes(123, 16_384).unwrap_err().to_string().contains("Reduce"));
    }

    #[test]
    fn keeps_only_neighbours_inside_the_scanned_subnets() {
        let hosts: BTreeSet<_> = subnet(Ipv4Addr::new(192, 168, 1, 20), 24, MAX_AUTO_PREFIX).0.into_iter().collect();
        let ips = ["192.168.1.7", "192.168.1.255", "192.168.1.0", "224.0.0.22", "10.0.0.5", "255.255.255.255", "192.168.1.7", "192.168.1.1"];
        let alive = alive_hosts(ips.map(|s| s.parse::<Ipv4Addr>().unwrap()), &hosts);
        assert_eq!(alive, [Ipv4Addr::new(192, 168, 1, 1), Ipv4Addr::new(192, 168, 1, 7)]);
    }

    #[test]
    fn adds_found_ports_to_the_setting() {
        let dev = |port| Device::new(SocketAddr::from(([192, 168, 1, 5], port)), DeviceInfo::default());
        assert_eq!(with_found_ports("", &[dev(30000)]).as_deref(), Some("30000"));
        assert_eq!(with_found_ports("30000, ", &[dev(50001), dev(50002), dev(30000)]).as_deref(), Some("30000, 50001-50002"));
        assert_eq!(with_found_ports("49152-65535", &[dev(50000)]), None);
    }

    #[test]
    fn finds_a_fake_battery_in_a_port_range() {
        // 127.0.0.2 : les autres tests écoutent sur 127.0.0.1 et ne doivent pas recevoir nos sondes.
        let server = UdpSocket::bind((Ipv4Addr::new(127, 0, 0, 2), 0)).unwrap();
        server.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let port = server.local_addr().unwrap().port();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while !stop.load(Ordering::Relaxed) {
                    if let Ok((_, from)) = server.recv_from(&mut buf) {
                        let _ = server.send_to(br#"{"id":1,"result":{"device":"VenusE","ver":1}}"#, from);
                    }
                }
            })
        };
        let range = format!("{}-{}", port.saturating_sub(30).max(1), port.saturating_add(30));
        let mut steps = 0;
        let scan = discover(Some("127.0.0.2-2"), &range, false, &AtomicBool::new(false), &mut |_| steps += 1).unwrap();
        stop.store(true, Ordering::Relaxed);
        thread.join().unwrap();
        assert_eq!(scan.devices.iter().map(|d| (d.ip.as_str(), d.port)).collect::<Vec<_>>(), [("127.0.0.2", port)]);
        assert_eq!(scan.saved_ports, None);
        assert!(steps > 0);
    }

    #[test]
    fn rejects_bad_ranges() {
        for s in ["", "abc", "192.168.1.1", "192.168.1.1-300", "192.168.1.0/x"] {
            assert!(parse_range(s).is_err(), "{s}");
        }
    }

    #[test]
    fn missing_fields_everywhere() {
        let b: BatStatus = serde_json::from_value(json!({ "soc": 90, "charg_flag": true })).unwrap();
        assert_eq!((b.soc, b.charg_flag, b.bat_temp), (Some(90.0), Some(true), None));
        let p: PvStatus = serde_json::from_value(json!({ "pv1_power": 12 })).unwrap();
        assert_eq!((p.pv1_power, p.pv4_voltage), (Some(12.0), None));
        let d: DeviceInfo = serde_json::from_value(json!({ "device": "VenusD", "ver": 111 })).unwrap();
        assert_eq!(d.ip, None);
        let e: EmStatus = serde_json::from_value(json!({})).unwrap();
        assert_eq!(e.total_power, None);
    }

    #[test]
    fn serializes_camel_case_without_missing_fields() {
        let e = EsStatus { bat_soc: Some(98.0), total_grid_input_energy: Some(5.0), ..Default::default() };
        assert_eq!(serde_json::to_value(e).unwrap(), json!({ "batSoc": 98.0, "totalGridInputEnergy": 5.0 }));
    }

    #[test]
    fn device_version_is_a_string() {
        let info: DeviceInfo = serde_json::from_value(json!({ "device": "VenusD", "ver": 111, "wifi_mac": "aa" })).unwrap();
        let d = Device::new("192.168.1.5:30000".parse().unwrap(), info);
        assert_eq!(
            serde_json::to_value(d).unwrap(),
            json!({ "ip": "192.168.1.5", "port": 30000, "name": "VenusD", "version": "111", "wifiMac": "aa" })
        );
    }
}
