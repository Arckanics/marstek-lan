//! Client pour l'Open API locale de Marstek (JSON-RPC sur UDP).
//!
//! Doc officielle : « Marstek Device Local API » (Rev 3.1.2).
//! La doc contient des incohérences (unités, noms de champs) : tous les champs
//! sont donc optionnels et à vérifier sur ta Venus D.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::fmt;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
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
const SCAN_WAIT: Duration = Duration::from_millis(1500);

#[derive(Debug)]
pub enum ApiError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Timeout,
    Device { code: i64, message: String },
    Invalid(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "erreur réseau : {e}"),
            Self::Json(e) => write!(f, "réponse illisible : {e}"),
            Self::Timeout => write!(f, "pas de réponse (Open API activée ? bonne IP/port ?)"),
            Self::Device { code, message } => write!(f, "la batterie a refusé ({code}) : {message}"),
            Self::Invalid(msg) => write!(f, "{msg}"),
        }
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
        return Err(ApiError::Invalid(format!(
            "{} créneaux demandés, la batterie n'en accepte que {SLOTS}",
            slots.len()
        )));
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
    next_id: u64,
    pub(crate) waits: [Duration; 3],
    /// Une entrée par requête : le RTT de la tentative réussie, `None` si elle a échoué.
    recent: VecDeque<Option<Duration>>,
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
        let next_id = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| u64::from(d.subsec_nanos()));
        Ok(Self { sock, addr, next_id, waits: WAITS, recent: VecDeque::new() })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn link(&self) -> Link {
        Link::of(self.recent.iter())
    }

    /// Envoie une requête et attend la réponse portant le même `id`, en la renvoyant (même `id`) si elle tarde :
    /// la réponse à une tentative précédente reste acceptée.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ApiError> {
        self.next_id += 1;
        let req = serde_json::to_vec(&json!({ "id": self.next_id, "method": method, "params": params }))?;
        let sent = self.attempts(self.next_id, &req);
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(sent.as_ref().ok().map(|(_, rtt)| *rtt));
        sent?.0
    }

    /// Une réponse de la batterie, même un refus, prouve que la liaison marche ; son RTT est celui de la tentative réussie.
    fn attempts(&self, id: u64, req: &[u8]) -> Result<(Result<Value, ApiError>, Duration), ApiError> {
        for wait in self.waits {
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
                // Sous Windows un ICMP « port fermé » (paquet précédent) fait échouer recv_from.
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
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
        Ok(serde_json::from_value(self.call(method, json!({ "id": 0 }))?)?)
    }

    fn set(&mut self, method: &str, params: Value) -> Result<bool, ApiError> {
        let r = self.call(method, params)?;
        Ok(r.get("set_result").and_then(Value::as_bool).unwrap_or(false))
    }

    pub fn device_info(&mut self) -> Result<DeviceInfo, ApiError> {
        Ok(serde_json::from_value(self.call("Marstek.GetDevice", json!({ "ble_mac": "0" }))?)?)
    }

    pub fn wifi_status(&mut self) -> Result<WifiStatus, ApiError> {
        self.get("Wifi.GetStatus")
    }

    pub fn ble_status(&mut self) -> Result<BleStatus, ApiError> {
        self.get("BLE.GetStatus")
    }

    pub fn bat_status(&mut self) -> Result<BatStatus, ApiError> {
        self.get("Bat.GetStatus")
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

/// Plage `192.168.1.1-254` ou `192.168.1.0/24` (préfixes 24 à 30, hors adresses réseau et broadcast).
pub fn parse_range(s: &str) -> Result<Vec<Ipv4Addr>, ApiError> {
    let bad = || ApiError::Invalid(format!("plage IP invalide : « {s} » (ex. 192.168.1.1-254 ou 192.168.1.0/24)"));
    let s = s.trim();
    if let Some((ip, prefix)) = s.split_once('/') {
        let ip: Ipv4Addr = ip.parse().map_err(|_| bad())?;
        let prefix: u32 = prefix.parse().map_err(|_| bad())?;
        if !(24..=30).contains(&prefix) {
            return Err(bad());
        }
        let size = 1u32 << (32 - prefix);
        let base = u32::from(ip) & !(size - 1);
        Ok((base + 1..base + size - 1).map(Ipv4Addr::from).collect())
    } else if let Some((ip, last)) = s.split_once('-') {
        let ip: Ipv4Addr = ip.parse().map_err(|_| bad())?;
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

/// Le /24 de l'interface principale. `connect` sur une UDP n'envoie aucun paquet : il fixe seulement la route.
fn default_range() -> Result<Vec<Ipv4Addr>, ApiError> {
    // shortcut: une seule interface (route par défaut) et un masque /24 supposé ; saisir la plage à la main sinon.
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    probe.connect("8.8.8.8:80")?;
    let IpAddr::V4(ip) = probe.local_addr()?.ip() else {
        return Err(ApiError::Invalid("aucune adresse IPv4 locale".into()));
    };
    let [a, b, c, _] = ip.octets();
    Ok((1..=254).map(|d| Ipv4Addr::new(a, b, c, d)).collect())
}

/// Un seul socket : broadcast puis envoi direct à chaque IP de la plage, réponses dédoublonnées par IP.
pub fn discover(range: Option<&str>, port: u16) -> Result<Vec<Device>, ApiError> {
    let targets = match range.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => parse_range(r)?,
        None => default_range()?,
    };
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(100)))?;
    let req = serde_json::to_vec(&json!({ "id": 1, "method": "Marstek.GetDevice", "params": { "ble_mac": "0" } }))?;
    let _ = sock.send_to(&req, (Ipv4Addr::BROADCAST, port));
    for ip in targets {
        let _ = sock.send_to(&req, (ip, port));
    }

    let mut found: Vec<Device> = Vec::new();
    let deadline = Instant::now() + SCAN_WAIT;
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        // Sous Windows un ICMP « port fermé » fait échouer recv_from : on ignore et on continue.
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let Ok(resp) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
        let Some(result) = resp.get("result") else { continue };
        if found.iter().any(|d| d.ip == from.ip().to_string()) {
            continue;
        }
        let info = serde_json::from_value(result.clone()).unwrap_or_default();
        found.push(Device::new(from, info));
    }
    Ok(found)
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
    fn call_gives_up_after_three_attempts() {
        let (addr, server) = flaky_server(usize::MAX);
        let mut client = quick(addr);
        assert!(matches!(client.call("Test", json!({})), Err(ApiError::Timeout)));
        assert_eq!(server.join().unwrap().len(), 3);
        assert_eq!((client.link().ok, client.link().total, client.link().rtt_ms), (0, 1, None));
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
        assert!(parse_range("10.0.0.0/16").is_err());
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
