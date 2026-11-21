//! Réglages écrits par le front dans `settings.json` (plugin-store) ; le backend les relit ici.

use crate::api::ModeReq;
use crate::plan::Plan;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tauri::AppHandle;
use tauri_plugin_store::{Store, StoreExt};

const FILE: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeKind {
    Auto,
    Ai,
    Ups,
}

impl ModeKind {
    pub fn req(self) -> ModeReq {
        match self {
            Self::Auto => ModeReq::Auto,
            Self::Ai => ModeReq::Ai,
            Self::Ups => ModeReq::Ups,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Ai => "IA",
            Self::Ups => "UPS",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub enabled: bool,
    pub soc: f64,
    pub mode: ModeKind,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Rules {
    pub low: Rule,
    pub high: Rule,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            low: Rule { enabled: false, soc: 10.0, mode: ModeKind::Auto },
            high: Rule { enabled: false, soc: 95.0, mode: ModeKind::Auto },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LowSoc {
    pub enabled: bool,
    pub soc: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Notify {
    pub low_soc: LowSoc,
    pub offline: bool,
    pub mode_changed: bool,
    pub write_failed: bool,
}

impl Default for Notify {
    fn default() -> Self {
        Self { low_soc: LowSoc { enabled: true, soc: 15.0 }, offline: true, mode_changed: false, write_failed: true }
    }
}

#[derive(Debug, Default)]
pub struct Settings {
    pub sent_plan: Option<Plan>,
    pub rules: Rules,
    pub notify: Notify,
    pub scan_range: Option<String>,
}

fn read<T: DeserializeOwned + Default>(store: &Store<tauri::Wry>, key: &str) -> T {
    store.get(key).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

/// Valeur absente ou illisible : réglage par défaut, la boucle de pilotage ne s'arrête pas pour autant.
pub fn load(app: &AppHandle) -> Settings {
    let Ok(store) = app.store(FILE) else { return Settings::default() };
    Settings {
        sent_plan: read(&store, "sentPlan"),
        rules: read(&store, "rules"),
        notify: read(&store, "notify"),
        scan_range: read(&store, "scanRange"),
    }
}

pub fn save_outside_slots(app: &AppHandle, value: serde_json::Value) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set("outsideSlots", value);
    store.save().map_err(|e| e.to_string())
}

/// Le plan réellement écrit dans la batterie : il reste le plan actif tant qu'un autre n'est pas envoyé.
pub fn save_sent_plan(app: &AppHandle, plan: &Plan) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set("sentPlan", serde_json::to_value(plan).map_err(|e| e.to_string())?);
    store.save().map_err(|e| e.to_string())
}

/// La batterie a changé d'IP : le front la retrouve au prochain démarrage.
pub fn save_last_ip(app: &AppHandle, ip: &str) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set("lastIp", ip);
    store.save().map_err(|e| e.to_string())
}
