//! Client pour l'Open API locale de Marstek (JSON-RPC sur UDP).
//!
//! Doc officielle : « Marstek Device Open API » (Rev 2.0).
//! La doc contient des incohérences (unités, noms de champs) : tous les champs
//! sont donc optionnels et à vérifier sur ta Venus D.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::fmt;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 30000;
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub enum ApiError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Timeout,
    Device { code: i64, message: String },
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "erreur réseau : {e}"),
            Self::Json(e) => write!(f, "réponse illisible : {e}"),
            Self::Timeout => write!(f, "pas de réponse (Open API activée ? bonne IP/port ?)"),
            Self::Device { code, message } => write!(f, "la batterie a refusé ({code}) : {message}"),
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceInfo {
    pub device: Option<String>,
    pub ver: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BatStatus {
    pub soc: Option<f64>,
    pub bat_temp: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EsStatus {
    pub bat_soc: Option<f64>,
    pub pv_power: Option<f64>,
    pub ongrid_power: Option<f64>,
    pub bat_power: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct EsMode {
    pub mode: Option<String>,
    pub ct_state: Option<i64>,
    pub total_power: Option<f64>,
}

// ---------- Modes ----------

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
    /// Puissance en W (signe charge/décharge à vérifier sur ta batterie).
    pub power: i32,
    pub enable: bool,
}

#[derive(Debug, Clone)]
pub enum ModeConfig {
    Auto,
    Ai,
    Manual(ManualSlot),
    /// Puissance en W et durée (compte à rebours) en secondes.
    Passive { power: i32, cd_time: u32 },
}

impl ModeConfig {
    fn to_json(&self) -> Value {
        match self {
            Self::Auto => json!({ "mode": "Auto", "auto_cfg": { "enable": 1 } }),
            Self::Ai => json!({ "mode": "AI", "ai_cfg": { "enable": 1 } }),
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
}

impl Client {
    pub fn new(addr: SocketAddr) -> Result<Self, ApiError> {
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        sock.set_read_timeout(Some(TIMEOUT))?;
        Ok(Self { sock, addr, next_id: 0 })
    }

    /// Envoie une requête et attend la réponse portant le même `id`.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ApiError> {
        self.next_id += 1;
        let id = self.next_id;
        let req = json!({ "id": id, "method": method, "params": params });
        self.sock.send_to(&serde_json::to_vec(&req)?, self.addr)?;

        let deadline = Instant::now() + TIMEOUT;
        let mut buf = [0u8; 4096];
        while Instant::now() < deadline {
            let (n, from) = match self.sock.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => break,
                Err(e) => return Err(e.into()),
            };
            if from.ip() != self.addr.ip() {
                continue;
            }
            let resp: Value = serde_json::from_slice(&buf[..n])?;
            if resp.get("id").and_then(Value::as_u64) != Some(id) {
                continue; // réponse à une ancienne requête
            }
            if let Some(err) = resp.get("error") {
                return Err(ApiError::Device {
                    code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: err.get("message").and_then(Value::as_str).unwrap_or("?").to_owned(),
                });
            }
            return Ok(resp.get("result").cloned().unwrap_or(Value::Null));
        }
        Err(ApiError::Timeout)
    }

    fn get<T: DeserializeOwned>(&mut self, method: &str) -> Result<T, ApiError> {
        Ok(serde_json::from_value(self.call(method, json!({ "id": 0 }))?)?)
    }

    pub fn device_info(&mut self) -> Result<DeviceInfo, ApiError> {
        Ok(serde_json::from_value(self.call("Marstek.GetDevice", json!({ "ble_mac": "0" }))?)?)
    }

    pub fn bat_status(&mut self) -> Result<BatStatus, ApiError> {
        self.get("Bat.GetStatus")
    }

    pub fn es_status(&mut self) -> Result<EsStatus, ApiError> {
        self.get("ES.GetStatus")
    }

    pub fn es_mode(&mut self) -> Result<EsMode, ApiError> {
        self.get("ES.GetMode")
    }

    pub fn set_mode(&mut self, cfg: &ModeConfig) -> Result<bool, ApiError> {
        let r = self.call("ES.SetMode", json!({ "id": 0, "config": cfg.to_json() }))?;
        Ok(r.get("set_result").and_then(Value::as_bool).unwrap_or(false))
    }

    /// Profondeur de décharge, 30 à 88 % (firmware ≥ 150).
    pub fn set_dod(&mut self, value: u8) -> Result<bool, ApiError> {
        let r = self.call("DOD.SET", json!({ "value": value.clamp(30, 88) }))?;
        Ok(r.get("set_result").and_then(Value::as_bool).unwrap_or(false))
    }
}

/// Cherche les batteries sur le réseau local (broadcast de `Marstek.GetDevice`).
pub fn discover(port: u16) -> Result<Vec<(SocketAddr, DeviceInfo)>, ApiError> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(300)))?;
    let req = json!({ "id": 1, "method": "Marstek.GetDevice", "params": { "ble_mac": "0" } });
    sock.send_to(&serde_json::to_vec(&req)?, (Ipv4Addr::BROADCAST, port))?;

    let mut found: Vec<(SocketAddr, DeviceInfo)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let Ok(resp) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
        let Some(result) = resp.get("result") else { continue };
        let info = serde_json::from_value(result.clone()).unwrap_or_default();
        if !found.iter().any(|(a, _)| *a == from) {
            found.push((from, info));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
