//! Réglages écrits par le front dans `settings.json` (plugin-store) ; le backend les relit ici.

use crate::api::ModeReq;
use crate::control::Verdict;
use crate::i18n::Lang;
use crate::plan::Plan;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::net::SocketAddr;
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

/// Résultat du test hors créneaux ; la date ne sert qu'à l'interface.
#[derive(Debug, Clone, Deserialize)]
pub struct OutsideSlots {
    pub verdict: Verdict,
}

/// Appareil du compte cloud suivi par le secours ; le front y garde aussi son nom, inutile ici.
#[derive(Debug, Clone, Deserialize)]
pub struct CloudPick {
    pub id: String,
}

#[derive(Debug, Default)]
pub struct Settings {
    pub sent_plan: Option<Plan>,
    pub rules: Rules,
    pub notify: Notify,
    pub scan_range: Option<String>,
    /// « 30000, 49152-65535 » ; les anciens réglages stockaient un tableau de nombres.
    pub ports: String,
    pub outside_slots: Option<OutsideSlots>,
    /// Plafond de puissance voulu par l'utilisateur, en W.
    pub max_power: Option<f64>,
    /// Pilotage automatique (règles SOC, hybride, reprise du plan) ; absent : désactivé, l'application ne fait que lire.
    pub auto_control: bool,
    /// Réécrit le plan quand la batterie a quitté le mode Manual ; sans effet si `auto_control` est faux.
    pub reapply_plan: bool,
    /// Secondes entre deux sondages, choisies dans Réglages ; absent : valeur automatique (voir `worker::base_poll`).
    pub poll_seconds: Option<u64>,
    /// Langue de l'interface, des notifications et de l'icône de la zone de notification ; absent : anglais.
    pub lang: Lang,
    /// Secours cloud actif seulement si un appareil est choisi (réglage `cloudDevice`, compte connecté dans Réglages).
    pub cloud_device: Option<CloudPick>,
    /// Rouvre seule l'API locale via le cloud quand elle a été coupée (port fermé) ; activé par défaut, coupable dans Réglages.
    pub auto_reenable: bool,
    /// Cherche seule la période de sondage la plus rapide que la batterie supporte (calibration).
    pub auto_tune: bool,
    /// Cadence apprise par la calibration, en secondes ; absent : la calibration repart de zéro.
    pub tuned_poll: Option<u64>,
    /// Code compteur (0-12) à renvoyer au cloud quand le CT est perdu après une coupure ; absent : désactivé.
    pub ct_reselect_meter: Option<u8>,
    /// MAC (12 hexa) du compteur, pour les modèles qui en ont besoin.
    pub ct_reselect_mac: Option<String>,
}

/// Puissance maximale du modèle d'après `device` (Marstek.GetDevice) ; modèle inconnu : borne absolue.
pub fn model_max(device: Option<&str>) -> u16 {
    let name = device.unwrap_or_default().to_lowercase().replace(' ', "");
    if name.contains("mini") {
        800
    } else if name.contains("venusd") {
        2200
    } else if name.contains("venusa") {
        1200
    } else {
        2500
    }
}

/// Plafond effectif : le plus bas du réglage utilisateur et du maximum du modèle.
pub fn power_cap(max_power: Option<f64>, device: Option<&str>) -> u16 {
    let model = model_max(device);
    max_power.filter(|p| *p >= 0.0).map_or(model, |p| model.min(p.round() as u16))
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
        ports: match store.get("ports") {
            Some(serde_json::Value::Array(list)) => list.iter().filter_map(serde_json::Value::as_u64).map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
            other => other.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
        },
        outside_slots: read(&store, "outsideSlots"),
        max_power: read(&store, "maxPower"),
        auto_control: read(&store, "autoControl"),
        reapply_plan: read(&store, "reapplyPlan"),
        poll_seconds: read(&store, "pollSeconds"),
        lang: read(&store, "lang"),
        cloud_device: read(&store, "cloudDevice"),
        // Activé par défaut : l'absence de réglage vaut « oui » (le défaut `bool` serait « non »).
        auto_reenable: store.get("autoReenable").and_then(|v| serde_json::from_value(v).ok()).unwrap_or(true),
        auto_tune: read(&store, "autoTune"),
        tuned_poll: read(&store, "tunedPoll"),
        ct_reselect_meter: read(&store, "ctReselectMeter"),
        ct_reselect_mac: read(&store, "ctReselectMac"),
    }
}

/// Persiste la cadence apprise par la calibration (écriture côté backend, relue au prochain démarrage).
pub fn save_tuned_poll(app: &AppHandle, secs: u64) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set("tunedPoll", secs);
    store.save().map_err(|e| e.to_string())
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

pub fn save_ports(app: &AppHandle, ports: &str) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set("ports", ports);
    store.save().map_err(|e| e.to_string())
}

/// La batterie a changé d'adresse : le front la retrouve au prochain démarrage.
pub fn save_last_addr(app: &AppHandle, addr: SocketAddr) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set("lastIp", addr.ip().to_string());
    store.set("lastPort", addr.port());
    store.save().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_max_follows_the_device_name() {
        let max = |d| model_max(Some(d));
        assert_eq!((max("VenusD"), max("Venus A"), max("VenusE"), max("Venus Mini")), (2200, 1200, 2500, 800));
        assert_eq!((model_max(None), max("Inconnu")), (2500, 2500));
    }

    #[test]
    fn the_language_is_english_by_default() {
        assert_eq!(Settings::default().lang, Lang::En);
    }

    #[test]
    fn automatic_control_is_off_by_default() {
        assert!(!Settings::default().auto_control);
        assert!(!Settings::default().reapply_plan);
    }

    #[test]
    fn cap_is_the_lowest_of_setting_and_model() {
        assert_eq!(power_cap(None, Some("VenusA")), 1200);
        assert_eq!(power_cap(Some(900.0), Some("VenusA")), 900);
        assert_eq!(power_cap(Some(5000.0), Some("VenusA")), 1200);
        assert_eq!(power_cap(Some(-1.0), None), 2500);
        assert_eq!(power_cap(Some(f64::NAN), None), 2500);
    }
}
