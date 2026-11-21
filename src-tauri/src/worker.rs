//! Thread unique qui possède le client UDP : une requête à la fois, jamais en parallèle.
//! Il porte aussi le moteur de pilotage (plan, règles SOC, action manuelle) et les alertes.

use crate::api::{
    self, ApiError, BatStatus, BleStatus, Client, DeviceInfo, Direction, EmStatus, EsMode, EsStatus, Link, ModeConfig,
    ModeReq, PvStatus, Request, Slot, WifiStatus, manual_slots,
};
use crate::clock::{self, MINUTE};
use crate::cloud::{self, CloudDevice, Reading};
use crate::control::{
    self, Action, Control, Down, DownKind, Drive, Engine, Hybrid, HybridAction, OFFLINE_AFTER, Point, Reapply, Side, SlotTest, Source, Sync,
    SyncInput, Verdict,
};
use crate::history::{Counters, History, Sample};
use crate::i18n::{self, Lang, Msg, Part};
use crate::plan::{Plan, compile_plan};
use crate::settings::{self, ModeKind, Notify, Rules};
use crate::tray::{self, Flow};
use serde::Serialize;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// La batterie décroche, voire redémarre avec l'Open API coupée, quand on la sollicite trop (observé : plantages à ~180
/// requêtes/h ; stable vers 240 s de sondage) : lectures étalées sur l'intervalle, au moins `MIN_GAP` entre une lecture et
/// la requête précédente quelle qu'elle soit, `MIN_GAP_ANY` entre une écriture et la requête précédente, `SLOT_GAP` entre
/// deux créneaux d'un même envoi de plan (écritures demandées par l'utilisateur).
const MIN_GAP: Duration = Duration::from_secs(60);
const MIN_GAP_ANY: Duration = Duration::from_secs(5);
const SLOT_GAP: Duration = Duration::from_secs(3);
/// Sondage choisi dans Réglages : bornes et valeur par défaut, en secondes.
const POLL_MIN: u64 = 30;
const POLL_CEIL: u64 = 600;
const POLL_DEFAULT: u64 = 240;
/// Calibration auto de la cadence : on fait descendre la période de cycle par pas de 5 s pour des données plus fraîches,
/// jusqu'à ce que la batterie décroche (API auto-coupée), puis +5 s et on verrouille. Plancher `TUNE_BASE_FLOOR` (gap de
/// 15 s entre lectures, minimum tenable d'après la communauté). Le gap réel découle de la période (= période / 3, borné).
const TUNE_BASE_FLOOR: u64 = 45;
const GAP_FLOOR: Duration = Duration::from_secs(15);
const TUNE_STEP: Duration = Duration::from_secs(5);
/// Cycles sains d'affilée à une cadence avant de tenter 5 s plus vite.
const TUNE_STABLE_CYCLES: u32 = 10;
/// Hors ligne, le sondage ne dépasse pas le plus grand de cette durée et du sondage de base.
const BACKOFF_CEIL: u64 = 60;
/// Une lecture sans réponse 3 fois de suite n'est plus relue que toutes les 5 min.
const ASIDE_AFTER: u32 = 3;
const ASIDE_FOR: Duration = Duration::from_secs(300);
/// Sondages d'affilée sur un port fermé avant de chercher le nouveau port, et délai minimal entre deux recherches.
const CLOSED_AFTER: u32 = 3;
const RECOVERY_EVERY: Duration = Duration::from_secs(600);
/// Le mode lu est jugé jusqu'à cet âge (ms) : il n'est relu que toutes les 10 min.
const MODE_MAX_AGE: u64 = 300_000;
/// Réactivation auto de l'API locale : 1re tentative immédiate, puis attente croissante entre deux (15, 30, 60 min, plafond
/// 60), pour ne pas marteler le cloud/broker. Remis à zéro dès que le local répond.
const REENABLE_BASE: Duration = Duration::from_secs(900);
/// Après un envoi de réactivation, délai avant de re-sonder le port local, le temps que l'appareil rouvre l'API.
const REENABLE_HOLD: Duration = Duration::from_secs(30);

/// Intervalle de sondage en bonne santé : celui des Réglages (borné), sinon `POLL_DEFAULT`.
fn base_poll(chosen: Option<u64>) -> Duration {
    Duration::from_secs(chosen.unwrap_or(POLL_DEFAULT).clamp(POLL_MIN, POLL_CEIL))
}

/// Durée réelle d'un cycle de trois lectures : jamais sous 3 x le gap entre lectures, même si le sondage est plus court.
fn cycle_period(base: Duration, gap: Duration) -> Duration {
    base.max(gap * 3)
}

/// Silence avant l'alerte « injoignable » : jamais avant deux cycles complets sans réponse.
fn offline_after(base: Duration, gap: Duration) -> Duration {
    OFFLINE_AFTER.max(cycle_period(base, gap) * 2 + gap)
}
/// Hors ligne, le sondage double à chaque échec (sans dépasser `BACKOFF_CEIL` ni le sondage de base s'il est plus long) ;
/// un succès le ramène au sondage de base.
fn poll_interval(base: Duration, failures: u32) -> Duration {
    (base * (1 << failures.min(3))).min(base.max(Duration::from_secs(BACKOFF_CEIL)))
}

/// Lectures secondaires : un cycle de sondage lit toujours Bat.GetStatus puis ES.GetStatus, et au plus une de celles-ci.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Extra {
    Device,
    Mode,
    Pv,
    Em,
    Wifi,
    Ble,
}

impl Extra {
    fn key(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Mode => "mode",
            Self::Pv => "pv",
            Self::Em => "em",
            Self::Wifi => "wifi",
            Self::Ble => "ble",
        }
    }
}

/// Chaque lecture secondaire et sa période, par ordre de priorité à retard égal.
const EXTRAS: [(Extra, Duration); 6] = [
    (Extra::Device, Duration::from_secs(1800)),
    (Extra::Mode, Duration::from_secs(600)),
    (Extra::Pv, Duration::from_secs(600)),
    (Extra::Em, Duration::from_secs(1800)),
    (Extra::Wifi, Duration::from_secs(1800)),
    (Extra::Ble, Duration::from_secs(1800)),
];

/// La lecture secondaire de ce sondage : la plus en retard sur sa période (jamais lue : la première de la liste), pas
/// de lecture si aucune n'est due. Tant que le modèle est inconnu, `Marstek.GetDevice` est toujours due.
fn next_extra(last: &HashMap<Extra, Instant>, aside: &Aside, device_known: bool, now: Instant) -> Option<Extra> {
    EXTRAS
        .iter()
        .filter(|(e, _)| aside.due(e.key(), now))
        .filter_map(|&(e, every)| {
            let every = if e == Extra::Device && !device_known { Duration::ZERO } else { every };
            let late = last.get(&e).map_or(Some(Duration::MAX), |t| now.duration_since(*t).checked_sub(every))?;
            Some((e, late))
        })
        .reduce(|a, b| if b.1 > a.1 { b } else { a })
        .map(|(e, _)| e)
}

/// Une lecture du cycle de sondage, envoyée à chaque tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Read {
    Bat,
    Es,
    Extra(Extra),
}

impl Read {
    fn key(self) -> &'static str {
        match self {
            Self::Bat => "bat",
            Self::Es => "es",
            Self::Extra(e) => e.key(),
        }
    }
}

/// Les lectures d'un cycle : Bat.GetStatus, ES.GetStatus et la secondaire éventuelle.
fn plan(extra: Option<Extra>) -> VecDeque<Read> {
    [Read::Bat, Read::Es].into_iter().chain(extra.map(Read::Extra)).collect()
}

/// Écart entre deux lectures d'un cycle de `n` lectures étalé sur l'intervalle de sondage.
fn tick_gap(interval: Duration, n: usize, gap: Duration) -> Duration {
    (interval / n.max(1) as u32).max(gap)
}

/// État d'un cycle de sondage. Un échec de Bat.GetStatus ou d'ES.GetStatus seul ne coupe pas la liaison.
#[derive(Default)]
struct Cycle {
    /// Nombre de lectures prévues, et début (ms).
    n: usize,
    start: u64,
    bat_ok: bool,
    es_ok: bool,
    bat_lost: bool,
    es_lost: bool,
}

impl Cycle {
    /// Hors ligne seulement si ni Bat.GetStatus ni ES.GetStatus n'ont répondu pendant le cycle.
    fn online(&self) -> bool {
        self.bat_ok || self.es_ok
    }
}

/// Famille de requêtes pour l'espacement : une lecture, une requête quelconque (écriture, confirmation), ou le créneau
/// suivant d'un même envoi de plan.
#[derive(Clone, Copy)]
enum Kind {
    Read,
    Any,
    Slot,
}

/// Fin de la dernière requête : le délai à attendre avant la prochaine.
struct Pace {
    any: Option<Instant>,
    read_gap: Duration,
    any_gap: Duration,
    slot_gap: Duration,
}

impl Pace {
    fn new() -> Self {
        Self { any: None, read_gap: MIN_GAP, any_gap: MIN_GAP_ANY, slot_gap: SLOT_GAP }
    }

    fn delay(&self, kind: Kind, now: Instant) -> Duration {
        let left = |t: Option<Instant>, gap: Duration| t.map_or(Duration::ZERO, |t| (t + gap).saturating_duration_since(now));
        match kind {
            Kind::Read => left(self.any, self.read_gap),
            Kind::Any => left(self.any, self.any_gap),
            Kind::Slot => left(self.any, self.slot_gap),
        }
    }

    fn wait(&self, kind: Kind) {
        thread::sleep(self.delay(kind, Instant::now()));
    }

    fn done(&mut self, at: Instant) {
        self.any = Some(at);
    }
}

/// Une commande en attente passe toujours avant une lecture échue.
fn next<T>(rx: &Receiver<T>, wait: Duration) -> Result<T, RecvTimeoutError> {
    match rx.try_recv() {
        Ok(c) => Ok(c),
        Err(TryRecvError::Disconnected) => Err(RecvTimeoutError::Disconnected),
        Err(TryRecvError::Empty) => rx.recv_timeout(wait),
    }
}

/// Les lectures secondaires ne sont envoyées que si l'une des deux lectures de base a répondu.
fn extras_allowed(bat_lost: bool, es_lost: bool) -> bool {
    !(bat_lost && es_lost)
}

/// La recherche du port est due après `CLOSED_AFTER` sondages sur port fermé, puis au plus une fois par `RECOVERY_EVERY`.
fn recovery_due(closed: u32, last: Option<Instant>, now: Instant) -> bool {
    closed >= CLOSED_AFTER && last.is_none_or(|t| now.duration_since(t) >= RECOVERY_EVERY)
}

/// Décision de calibration après un cycle : nouvelle (période, plancher sûr, cycles stables). Sur un décrochage, +5 s et ce
/// niveau devient le plancher (verrouille) ; stable assez longtemps et au-dessus du plancher, -5 s pour tenter plus vite.
fn tune_step(base: Duration, floor: Duration, stable: u32, crashed: bool) -> (Duration, Duration, u32) {
    if crashed {
        let b = (base + TUNE_STEP).min(Duration::from_secs(POLL_DEFAULT));
        (b, b, 0)
    } else if stable + 1 >= TUNE_STABLE_CYCLES && base > floor {
        ((base - TUNE_STEP).max(Duration::from_secs(TUNE_BASE_FLOOR)), floor, 0)
    } else {
        (base, floor, stable + 1)
    }
}

/// Attente avant la prochaine tentative de réactivation, selon le nombre de tentatives déjà faites dans cette coupure :
/// 0 → immédiate, 1 → 15 min, 2 → 30 min, 3 et plus → 60 min.
fn reenable_wait(tries: u32) -> Duration {
    if tries == 0 {
        Duration::ZERO
    } else {
        REENABLE_BASE * (1 << (tries - 1).min(2))
    }
}

#[derive(Debug)]
pub enum Write {
    Mode(ModeReq),
    Slots(Vec<Slot>),
    /// Envoi d'un plan : ses créneaux, et le plan devient le plan actif.
    Plan(Plan, Vec<Slot>),
    Dod(u8),
    Led(bool),
    Ble(bool),
}

impl Write {
    fn label(&self) -> Msg {
        match self {
            Self::Mode(_) => Msg::WriteMode,
            Self::Slots(_) => Msg::WriteSlots,
            Self::Plan(..) => Msg::WritePlan,
            Self::Dod(_) => Msg::WriteDod,
            Self::Led(_) => Msg::raw("LED"),
            Self::Ble(_) => Msg::raw("Bluetooth"),
        }
    }
}

/// Ce qu'il faut retenir d'une écriture réussie.
enum After {
    Nothing,
    /// Libellé du mode, et durée d'un mode passif.
    Manual(Msg, Option<u32>),
    Plan(Plan),
}

enum Cmd {
    Connect(SocketAddr),
    Refresh,
    Reload,
    SlotTest(Sender<Result<(), Msg>>),
    Write(Write, Sender<Result<Vec<bool>, Msg>>),
}

pub struct Worker(Sender<Cmd>);

impl Worker {
    pub fn spawn(app: AppHandle) -> Self {
        let (tx, rx) = channel();
        thread::spawn(move || {
            let mut state = State::new(app);
            state.reload();
            loop {
                match next(&rx, state.until_tick()) {
                    Ok(Cmd::Connect(addr)) => state.connect(addr),
                    Ok(Cmd::Refresh) => state.refresh(),
                    Ok(Cmd::Reload) => {
                        state.reload();
                        state.tick();
                    }
                    Ok(Cmd::SlotTest(reply)) => {
                        let _ = reply.send(state.start_test());
                    }
                    Ok(Cmd::Write(w, reply)) => {
                        let r = state.write(w);
                        let _ = reply.send(r);
                    }
                    Err(RecvTimeoutError::Timeout) => state.read_step(),
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self(tx)
    }

    fn send(&self, cmd: Cmd) -> Result<(), String> {
        self.0.send(cmd).map_err(|_| Msg::ThreadStopped.to_string())
    }

    pub fn connect(&self, addr: SocketAddr) -> Result<(), String> {
        self.send(Cmd::Connect(addr))
    }

    pub fn refresh(&self) -> Result<(), String> {
        self.send(Cmd::Refresh)
    }

    /// Relit `settings.json` (règles, notifications, plan actif).
    pub fn reload(&self) -> Result<(), String> {
        self.send(Cmd::Reload)
    }

    /// Bloque jusqu'à l'écriture du créneau de test ; la suite se suit par l'événement `slot_test`.
    pub fn slot_test(&self) -> Result<(), String> {
        let (tx, rx) = channel();
        self.send(Cmd::SlotTest(tx))?;
        rx.recv().map_err(|_| Msg::ThreadStopped.to_string())?.map_err(|m| m.to_string())
    }

    /// Bloque jusqu'à la fin de l'écriture (à appeler hors du thread principal).
    pub fn write(&self, w: Write) -> Result<Vec<bool>, String> {
        let (tx, rx) = channel();
        self.send(Cmd::Write(w, tx))?;
        rx.recv().map_err(|_| Msg::ThreadStopped.to_string())?.map_err(|m| m.to_string())
    }

    pub fn write_one(&self, w: Write) -> Result<bool, String> {
        self.write(w).map(|r| r[0])
    }
}

macro_rules! snapshot {
    ($($field:ident: $ty:ty),*) => {
        /// Dernier état connu, émis tel quel au front.
        #[derive(Default, Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Snapshot {
            /// Date de la dernière lecture, ms depuis l'epoch.
            ts: u64,
            /// Début du dernier cycle de sondage terminé : une section lue avant lui n'est plus fraîche.
            #[serde(skip)]
            since: u64,
            /// Numéro de la dernière mise à jour des requêtes : une réémission sans requête nouvelle garde le même.
            seq: u64,
            /// Date de la dernière lecture réussie de chaque section : une lecture ratée garde l'ancienne valeur.
            updated: HashMap<&'static str, u64>,
            link: Link,
            /// Requêtes de la dernière lecture ou écriture.
            requests: Vec<Request>,
            /// Pourquoi la batterie ne répond pas ; absent tant qu'elle répond.
            #[serde(skip_serializing_if = "Option::is_none")]
            down: Option<Down>,
            /// Période réelle d'un cycle de sondage, en secondes.
            poll_secs: u64,
            /// Plan envoyé comparé à ce que fait la batterie.
            sync: Sync,
            /// Dernières valeurs du cloud, seulement tant que la batterie ne répond pas en local.
            #[serde(skip_serializing_if = "Option::is_none")]
            cloud: Option<Reading>,
            $(
                #[serde(skip_serializing_if = "Option::is_none")]
                $field: Option<$ty>,
            )*
        }
    };
}

snapshot!(
    device: DeviceInfo, wifi: WifiStatus, ble: BleStatus, bat: BatStatus,
    es: EsStatus, mode: EsMode, em: EmStatus, pv: PvStatus
);

impl Snapshot {
    /// La section si elle a été lue pendant le dernier cycle de sondage, sinon `None`.
    fn fresh<'a, T>(&self, section: &'a Option<T>, key: &str) -> Option<&'a T> {
        section.as_ref().filter(|_| self.updated.get(key).is_some_and(|&t| t >= self.since))
    }

    /// La section si elle a été lue au plus `max_age` ms avant la dernière lecture (les lectures lentes ne sont pas de tous les cycles).
    fn recent<'a, T>(&self, section: &'a Option<T>, key: &str, max_age: u64) -> Option<&'a T> {
        section.as_ref().filter(|_| self.updated.get(key).is_some_and(|t| t + max_age >= self.ts))
    }
}

/// Test hors créneaux en cours : on relève la puissance à chaque sondage jusqu'à `ends`.
struct Test {
    /// Fin du créneau de test (ms).
    slot_end: i64,
    ends: i64,
    samples: Vec<Point>,
}

struct State {
    app: AppHandle,
    client: Option<Client>,
    snap: Snapshot,
    rules: Rules,
    notify: Notify,
    plan: Option<Plan>,
    /// Résultat du test hors créneaux ; `Follows` fait des blocs auto, IA et UPS de simples trous.
    outside: Option<Verdict>,
    engine: Engine,
    manual_label: Msg,
    /// Langue de l'interface (réglage `lang`) ; le menu de la zone de notification est refait quand elle change.
    lang: Lang,
    control: Option<Control>,
    test: Option<Test>,
    /// Le dernier cycle a répondu (ou Bat/ES vient de répondre dans le cycle en cours).
    online: bool,
    last_ok: Instant,
    /// Cycles de sondage ratés d'affilée.
    fails: u32,
    last_search: Option<Instant>,
    /// Sondages d'affilée où l'IP répond mais où le port est fermé.
    closed: u32,
    last_recovery: Option<Instant>,
    low_notified: bool,
    offline_notified: bool,
    /// Dernière lecture réussie avant la coupure alertée : sert au message de retour.
    down_since: Option<Instant>,
    aside: Aside,
    pace: Pace,
    /// Lectures restant à envoyer dans le cycle en cours, une par tick, et date du prochain tick.
    queue: VecDeque<Read>,
    cycle: Cycle,
    next_tick: Instant,
    seq: u64,
    /// Dernière tentative de chaque lecture secondaire depuis la connexion.
    extras: HashMap<Extra, Instant>,
    engine_failed: bool,
    tray: Option<(u8, Flow, String)>,
    /// Bloc courant (fin dans la semaine, en minutes) et date où il est devenu actif (ms).
    block_since: Option<(u32, i64)>,
    max_power: Option<f64>,
    /// Réglage `reapplyPlan` ; ne s'applique qu'avec le pilotage automatique.
    reapply_plan: bool,
    reapply: Reapply,
    poll_seconds: Option<u64>,
    /// Appareil du compte cloud suivi par le secours (réglage `cloudDevice`) ; absent : pas de secours.
    cloud_device: Option<String>,
    /// Dernière mesure datée reçue du cloud ; effacée dès que la batterie répond en local.
    cloud: Option<CloudDevice>,
    /// `report_time` de la dernière mesure cloud écrite dans l'historique (pour ne pas l'écrire deux fois).
    cloud_recorded: Option<u64>,
    /// Réglage `autoReenable` : rouvrir seul l'API locale via le cloud quand le port est détecté fermé.
    auto_reenable: bool,
    /// Dernière tentative de réactivation et nombre de tentatives de la coupure en cours (pour le backoff).
    reenable_at: Option<Instant>,
    reenable_tries: u32,
    /// Après un envoi, on ne re-sonde pas le port local avant cette date (laisse l'appareil rouvrir l'API).
    reenable_hold: Option<Instant>,
    /// Réglage `ctReselectMeter`/`ctReselectMac` : renvoyer la sélection du CT au cloud s'il est perdu après une coupure.
    ct_reselect: Option<(u8, Option<String>)>,
    /// Dernier envoi de re-sélection du CT et nombre d'essais, pour espacer les retries tant que le CT reste perdu.
    ct_reselect_at: Option<Instant>,
    ct_reselect_tries: u32,
    /// Réglage `autoTune` : chercher seule la période de cycle la plus rapide que la batterie supporte.
    auto_tune: bool,
    /// Période de cycle en cours quand la calibration est active (remplace le sondage des Réglages).
    tuned_base: Duration,
    /// Période la plus rapide connue comme sûre : la calibration ne descend jamais en dessous.
    tune_floor: Duration,
    /// Cycles sains d'affilée à la cadence courante (pour tenter de descendre).
    tune_stable: u32,
}

fn mode_label(m: &ModeReq) -> Msg {
    match m {
        ModeReq::Auto => Msg::Mode(ModeKind::Auto),
        ModeReq::Ai => Msg::Mode(ModeKind::Ai),
        ModeReq::Ups => Msg::Mode(ModeKind::Ups),
        ModeReq::Passive { direction, power, .. } => {
            let d = if *direction == Direction::Charge { Msg::Charge(*power) } else { Msg::Discharge(*power) };
            Msg::Passive(Box::new(d))
        }
    }
}

fn passive_seconds(m: &ModeReq) -> Option<u32> {
    match m {
        ModeReq::Passive { seconds, .. } => Some(*seconds),
        _ => None,
    }
}

/// Échantillon et compteurs d'historique d'une lecture, dans les mêmes conventions que l'onglet État.
fn history_rows(es: Option<&EsStatus>, em: Option<&EmStatus>, soc: Option<f64>) -> (Sample, Counters) {
    let sample = Sample {
        soc,
        pv: es.and_then(|e| e.pv_power),
        bat: es.and_then(|e| e.bat_power),
        grid: em.filter(|e| e.ct_state == Some(1)).and_then(|e| e.total_power),
        // shortcut: « maison » = sortie hors-réseau ; la déduire (PV + réseau - batterie) dépend du signe de bat_power.
        load: es.and_then(|e| e.offgrid_power),
    };
    let counters = Counters {
        // total_pv_energy est en 0,01 kWh, les autres compteurs en Wh.
        pv: es.and_then(|e| e.total_pv_energy).map(|v| v * 10.0),
        grid_in: es.and_then(|e| e.total_grid_input_energy),
        grid_out: es.and_then(|e| e.total_grid_output_energy),
        load: es.and_then(|e| e.total_load_energy),
    };
    (sample, counters)
}

fn all_accepted(r: &Result<Vec<bool>, Msg>) -> bool {
    matches!(r, Ok(v) if v.iter().all(|&ok| ok))
}

impl State {
    fn new(app: AppHandle) -> Self {
        Self {
            app,
            client: None,
            snap: Snapshot::default(),
            rules: Rules::default(),
            notify: Notify::default(),
            plan: None,
            outside: None,
            // Lecture seule tant que `reload` n'a pas lu le réglage `autoControl` (absent : désactivé).
            engine: Engine { suspended: true, ..Engine::default() },
            manual_label: Msg::raw(""),
            lang: i18n::lang(),
            control: None,
            test: None,
            online: false,
            last_ok: Instant::now(),
            fails: 0,
            last_search: None,
            closed: 0,
            last_recovery: None,
            low_notified: false,
            offline_notified: false,
            down_since: None,
            aside: Aside::default(),
            pace: Pace::new(),
            queue: VecDeque::new(),
            cycle: Cycle::default(),
            next_tick: Instant::now(),
            seq: 0,
            extras: HashMap::new(),
            engine_failed: false,
            tray: None,
            block_since: None,
            max_power: None,
            reapply_plan: false,
            reapply: Reapply::default(),
            poll_seconds: None,
            cloud_device: None,
            cloud: None,
            cloud_recorded: None,
            auto_reenable: false,
            reenable_at: None,
            reenable_tries: 0,
            reenable_hold: None,
            ct_reselect: None,
            ct_reselect_at: None,
            ct_reselect_tries: 0,
            auto_tune: false,
            tuned_base: Duration::from_secs(POLL_DEFAULT),
            tune_floor: Duration::from_secs(TUNE_BASE_FLOOR),
            tune_stable: 0,
        }
    }

    /// Période de cycle en bonne santé : la cadence calibrée si la calibration est active, sinon le réglage (borné).
    fn base_poll(&self) -> Duration {
        if self.auto_tune { self.tuned_base } else { base_poll(self.poll_seconds) }
    }

    /// Espacement minimal entre deux requêtes. Sans calibration : toujours `MIN_GAP` (60 s), quel que soit le sondage réglé.
    /// Avec : découle de la période calibrée (période / 3), plafonné à 60 s et plancher à 15 s.
    fn gap(&self) -> Duration {
        if self.auto_tune {
            (self.tuned_base / 3).clamp(GAP_FLOOR, MIN_GAP)
        } else {
            MIN_GAP
        }
    }

    fn interval(&self) -> Duration {
        poll_interval(self.base_poll(), self.fails)
    }

    /// Applique la décision de calibration de ce cycle. Met à jour l'espacement effectif, persiste la cadence apprise
    /// (au mieux) et la journalise quand elle change. Sans effet si la calibration est coupée.
    fn tune(&mut self, crashed: bool) {
        if !self.auto_tune {
            return;
        }
        let (base, floor, stable) = tune_step(self.tuned_base, self.tune_floor, self.tune_stable, crashed);
        let changed = base != self.tuned_base;
        (self.tuned_base, self.tune_floor, self.tune_stable) = (base, floor, stable);
        if changed {
            self.pace.read_gap = self.gap();
            let _ = settings::save_tuned_poll(&self.app, base.as_secs());
            self.log(Msg::PollTuned(base.as_secs()));
        }
    }

    fn until_tick(&self) -> Duration {
        self.next_tick.saturating_duration_since(Instant::now())
    }

    fn log(&self, msg: Msg) {
        i18n::log(&self.app, &msg);
    }

    fn notify_user(&self, title: Msg, body: &Msg) {
        let lang = i18n::lang();
        if let Err(e) = self.app.notification().builder().title(title.text(lang)).body(body.text(lang)).show() {
            self.log(Msg::LabelNotification.detail(Msg::raw(e.to_string())));
        }
    }

    /// Le plan actif est le dernier plan envoyé à la batterie (`sentPlan` du store).
    fn reload(&mut self) {
        let s = settings::load(&self.app);
        self.plan = s.sent_plan;
        self.outside = s.outside_slots.map(|o| o.verdict);
        self.rules = s.rules;
        self.notify = s.notify;
        self.max_power = s.max_power;
        self.engine.suspended = !s.auto_control;
        self.reapply_plan = s.reapply_plan;
        self.poll_seconds = s.poll_seconds;
        self.cloud_device = s.cloud_device.map(|c| c.id);
        self.auto_reenable = s.auto_reenable;
        self.ct_reselect = s.ct_reselect_meter.map(|m| (m, s.ct_reselect_mac));
        let was_tuning = self.auto_tune;
        self.auto_tune = s.auto_tune;
        if s.auto_tune && !was_tuning {
            // Activation : reprend la cadence apprise (verrouillée, pas de re-recherche) ou démarre depuis le défaut.
            match s.tuned_poll {
                Some(secs) => {
                    self.tuned_base = Duration::from_secs(secs.clamp(TUNE_BASE_FLOOR, POLL_CEIL));
                    self.tune_floor = self.tuned_base;
                }
                None => {
                    self.tuned_base = Duration::from_secs(POLL_DEFAULT);
                    self.tune_floor = Duration::from_secs(TUNE_BASE_FLOOR);
                }
            }
            self.tune_stable = 0;
        }
        self.pace.read_gap = self.gap();
        if s.lang != self.lang {
            self.lang = s.lang;
            i18n::set(s.lang);
            tray::set_lang(&self.app, s.lang);
        }
        self.emit_control();
    }

    fn power_cap(&self) -> u16 {
        settings::power_cap(self.max_power, self.snap.device.as_ref().and_then(|d| d.device.as_deref()))
    }

    /// SOC local du dernier sondage : seule source des décisions (règles, hybride, alertes).
    fn soc(&self) -> Option<f64> {
        let s = &self.snap;
        s.fresh(&s.es, "es").and_then(|e| e.bat_soc).or_else(|| s.fresh(&s.bat, "bat").and_then(|b| b.soc))
    }

    /// Moteur à l'état de connexion : rien n'est renvoyé à la batterie.
    fn reset_engine(&mut self) {
        self.engine = Engine { suspended: self.engine.suspended, ..Engine::default() };
    }

    fn connect(&mut self, addr: SocketAddr) {
        // Déjà connecté et en ligne (le front redemande au démarrage ou après un rechargement) : pas de seconde connexion,
        // l'état courant est seulement réémis.
        if self.online && self.client.as_ref().is_some_and(|c| c.addr() == addr) {
            let _ = self.app.emit("status", &self.snap);
            return self.emit_control();
        }
        match Client::new(addr) {
            Ok(c) => {
                self.client = Some(c);
                self.reapply.reset();
                self.snap = Snapshot::default();
                self.reset_engine();
                self.test = None;
                self.last_ok = Instant::now();
                self.fails = 0;
                self.last_search = None;
                self.closed = 0;
                self.offline_notified = false;
                self.aside = Aside::default();
                self.extras.clear();
                self.log(Msg::Connecting(addr.to_string()));
                self.refresh();
            }
            Err(e) => self.log(Msg::LabelConnection.detail((&e).into())),
        }
    }

    /// Repart d'un cycle neuf tout de suite et réémet l'état du pilotage.
    fn refresh(&mut self) {
        self.queue.clear();
        self.next_tick = Instant::now();
        self.control = None;
        self.emit_control();
    }

    /// Un tick : une lecture du cycle (Bat.GetStatus, ES.GetStatus, puis au plus une secondaire, `next_extra`), étalée sur
    /// l'intervalle. Une lecture en échec n'empêche pas les suivantes et garde la dernière valeur connue ; la dernière du
    /// cycle le termine (historique, décisions).
    fn read_step(&mut self) {
        let now = Instant::now();
        if self.client.is_none() {
            self.next_tick = now + self.interval();
            return;
        }
        let wait = self.pace.delay(Kind::Read, now);
        if !wait.is_zero() {
            self.next_tick = now + wait;
            return;
        }
        if self.queue.is_empty() {
            let extra = next_extra(&self.extras, &self.aside, self.snap.device.is_some(), now);
            self.queue = plan(extra);
            self.cycle = Cycle { n: self.queue.len(), start: clock::now_ms() as u64, ..Cycle::default() };
        }
        let gap = tick_gap(self.interval(), self.cycle.n, self.gap());
        self.next_tick = now + gap;
        // Une lecture mise de côté n'est pas envoyée.
        let read = loop {
            match self.queue.pop_front() {
                Some(r) if !self.aside.due(r.key(), now) => continue,
                r => break r,
            }
        };
        if let Some(r) = read {
            self.read(r, gap);
        }
        let ended = self.queue.is_empty();
        if ended {
            self.end_cycle();
        }
        if read.is_some() {
            self.emit_status();
        }
        if ended && self.cycle.online() {
            self.record_history();
        }
        self.update_tray();
        if ended {
            self.tick();
            self.relocate();
            self.cloud_step();
            // Avant `recover_port` : une réactivation réussie pose un délai que la re-sonde du port respecte.
            self.auto_reenable_step();
            self.ct_reselect_step();
            self.recover_port();
        }
    }

    /// Secours : tant que la batterie ne répond pas en local, lit le cloud (au plus toutes les 5 min, voir `cloud::ready`).
    /// La mesure alimente l'affichage et est aussi écrite dans l'historique (marquée source cloud) pour combler le trou ;
    /// les décisions restent fondées sur le SOC local.
    // shortcut: l'appel réseau (20 s au plus) bloque ce thread, les commandes attendent ; à déporter si ça gêne.
    fn cloud_step(&mut self) {
        let Some(id) = self.cloud_device.clone() else { return };
        if !cloud::ready(!self.online && self.fails > 0) {
            return;
        }
        match cloud::devices() {
            Ok(list) => {
                let now = (clock::now_ms() / 1000) as u64;
                // Une mesure non datée n'écrase pas la précédente.
                if let Some(d) = list.into_iter().find(|d| d.devid == id).filter(|d| Reading::new(d, now).is_some()) {
                    self.record_cloud(&d);
                    self.cloud = Some(d);
                }
            }
            Err(e) => self.log(Msg::LabelCloud.detail(e)),
        }
        self.emit_status();
    }

    /// Écrit une mesure cloud dans l'historique (marquée source cloud), une seule fois par mesure datée. Le signe de `bat`
    /// suit la convention appareil (charge négative), le réseau et les compteurs manquent (le cloud ne les donne pas).
    fn record_cloud(&mut self, d: &CloudDevice) {
        let Some(rt) = d.report_time else { return };
        if self.cloud_recorded == Some(rt) {
            return;
        }
        let bat = match (d.charge, d.discharge) {
            (None, None) => None,
            (c, dis) => Some(dis.unwrap_or(0.0) - c.unwrap_or(0.0)),
        };
        let sample = Sample { soc: d.soc, pv: d.pv, bat, grid: None, load: d.load };
        if let Err(e) = self.app.state::<History>().record((rt as i64) * 1000, sample, Counters::default(), true) {
            self.log(Msg::LabelHistory.detail(Msg::raw(e.to_string())));
        } else {
            self.cloud_recorded = Some(rt);
        }
    }

    /// Rouvre seule l'API locale via le cloud quand le port local est détecté fermé (API coupée après un redémarrage).
    /// 1re tentative immédiate puis backoff (`reenable_wait`), une connexion à la fois, remis à zéro dès que le local répond
    /// (voir `end_cycle`). Sans effet si le réglage est coupé, sans appareil cloud, ou pour un simple silence : seul un port
    /// fermé déclenche (rouvrir n'aiderait pas un Wi-Fi coupé).
    // shortcut: l'appel réseau (HTTPS + MQTT, ~30 s) bloque ce thread comme `cloud_step` ; à déporter si ça gêne.
    fn auto_reenable_step(&mut self) {
        let Some(id) = self.cloud_device.clone() else { return };
        if !self.auto_reenable || self.online || self.closed < CLOSED_AFTER {
            return;
        }
        let now = Instant::now();
        if self.reenable_at.is_some_and(|t| now.duration_since(t) < reenable_wait(self.reenable_tries)) {
            return;
        }
        self.reenable_at = Some(now);
        self.reenable_tries = self.reenable_tries.saturating_add(1);
        // `cloud_step` vient de remplir `self.cloud` (mac+salt) : on le réutilise pour publier sans re-interroger le cloud.
        // Sinon (pas de mesure datée ce tour) on retombe sur le chemin complet, qui refait le `getDeviceList`.
        let result = match self.cloud.as_ref().filter(|d| d.devid == id) {
            Some(dev) => cloud::enable_from(dev),
            None => cloud::enable_local_api(&id),
        };
        match result {
            Ok(()) => {
                self.reenable_hold = Some(now + REENABLE_HOLD);
                self.log(Msg::CloudReenableSent);
            }
            Err(e) => self.log(Msg::LabelCloud.detail(e)),
        }
        self.emit_status();
    }

    /// Renvoie la sélection du CT au cloud quand le compteur est perdu (`ct_state == 0`), que l'Open API soit tombée ou non.
    /// Ré-essaie avec le backoff de la réactivation (`reenable_wait`) tant que le CT reste perdu ; l'état retombe quand le CT revient ou que le local décroche.
    fn ct_reselect_step(&mut self) {
        let ct = self.snap.em.as_ref().and_then(|e| e.ct_state);
        if !self.online || ct == Some(1) {
            self.ct_reselect_at = None;
            self.ct_reselect_tries = 0;
            return;
        }
        let (Some((meter, mac)), Some(id)) = (self.ct_reselect.clone(), self.cloud_device.clone()) else { return };
        if ct != Some(0) {
            return;
        }
        let now = Instant::now();
        if self.ct_reselect_at.is_some_and(|t| now.duration_since(t) < reenable_wait(self.ct_reselect_tries)) {
            return;
        }
        self.ct_reselect_at = Some(now);
        self.ct_reselect_tries = self.ct_reselect_tries.saturating_add(1);
        // En ligne, `self.cloud` est vidé (emit_status) : on refait le `getDeviceList` comme le repli de `enable_local_api`.
        match cloud::reselect_ct(&id, meter, mac.as_deref()) {
            Ok(_) => self.log(Msg::CloudCtReselectSent),
            Err(e) => self.log(Msg::LabelCloud.detail(e)),
        }
    }

    fn read(&mut self, r: Read, gap: Duration) {
        let Some(c) = self.client.as_mut() else { return };
        c.begin_poll();
        let (app, s, aside) = (&self.app, &mut self.snap, &mut self.aside);
        let now = clock::now_ms() as u64;
        macro_rules! read {
            ($key:ident, $name:expr, $call:expr) => {{
                let res = $call;
                if aside.record(stringify!($key), matches!(res, Err(ApiError::Timeout)), Instant::now()) {
                    i18n::log(app, &Msg::Section($name).detail(Msg::NoReplyAside));
                }
                step(app, $name, stringify!($key), res, &mut s.$key, &mut s.updated, now)
            }};
        }
        let mut closed = false;
        let lost = match r {
            Read::Bat => {
                let bat = c.bat_status();
                closed = matches!(bat, Err(ApiError::PortClosed(_)));
                step(app, Part::Battery, "bat", bat, &mut s.bat, &mut s.updated, now)
            }
            Read::Es => read!(es, Part::System, c.es_status()),
            Read::Extra(e) => {
                self.extras.insert(e, Instant::now());
                match e {
                    Extra::Device => read!(device, Part::Device, c.device_info()),
                    Extra::Mode => read!(mode, Part::Mode, c.es_mode()),
                    Extra::Pv => read!(pv, Part::Solar, c.pv_status()),
                    Extra::Em => read!(em, Part::Meter, c.em_status()),
                    Extra::Wifi => read!(wifi, Part::Wifi, c.wifi_status()),
                    Extra::Ble => read!(ble, Part::Bluetooth, c.ble_status()),
                }
            }
        };
        self.pace.done(Instant::now());
        s.ts = now;
        s.link = Link { rssi: s.wifi.as_ref().and_then(|w| w.rssi), ..c.link() };
        s.requests = c.requests().to_vec();
        let ok = s.updated.get(r.key()) == Some(&now);
        // Port fermé : rien n'écoute, les lectures suivantes coûteraient leur attente pour rien. Sinon, même sans réponse de
        // Bat.GetStatus, ES.GetStatus est tenté au tick suivant : une réponse des deux suffit à garder la batterie en ligne.
        let rest = match r {
            Read::Bat => {
                (self.cycle.bat_ok, self.cycle.bat_lost) = (ok, lost);
                self.closed = if closed { self.closed + 1 } else { 0 };
                closed
            }
            Read::Es => {
                (self.cycle.es_ok, self.cycle.es_lost) = (ok, lost);
                // Le modèle fixe le plafond de puissance : relu tant qu'il manque.
                !extras_allowed(self.cycle.bat_lost, self.cycle.es_lost)
            }
            Read::Extra(_) => false,
        };
        if ok && matches!(r, Read::Bat | Read::Es) {
            self.online = true;
            self.last_ok = Instant::now();
        }
        if rest {
            // Les lectures abandonnées gardent leur place dans le calendrier du cycle.
            self.next_tick += gap * self.queue.len() as u32;
            self.queue.clear();
        }
    }

    /// Fin de cycle : la batterie est hors ligne si ni Bat.GetStatus ni ES.GetStatus n'ont répondu.
    fn end_cycle(&mut self) {
        self.snap.since = self.cycle.start;
        if self.cycle.online() {
            if self.fails > 0 {
                self.reapply.back(clock::now_ms());
            }
            self.fails = 0;
            self.last_recovery = None;
            // Le local répond : la prochaine coupure repart au délai de base.
            (self.reenable_tries, self.reenable_at, self.reenable_hold) = (0, None, None);
            // Cycle sain : vers la cadence la plus rapide (après assez de cycles stables).
            self.tune(false);
        } else {
            self.online = false;
            // Première défaillance de cette coupure : la batterie a peut-être décroché d'avoir été trop sollicitée → +5 s.
            if self.fails == 0 {
                self.tune(true);
            }
            self.fails = self.fails.saturating_add(1);
        }
    }

    fn emit_status(&mut self) {
        self.snap.poll_secs = cycle_period(self.base_poll(), self.gap()).as_secs();
        self.snap.sync = self.sync(clock::now_ms());
        if self.online {
            self.cloud = None;
        }
        self.snap.cloud = self.cloud.as_ref().and_then(|d| Reading::new(d, (clock::now_ms() / 1000) as u64));
        self.seq += 1;
        self.snap.seq = self.seq;
        let _ = self.app.emit("status", &self.snap);
    }

    fn sync(&mut self, now: i64) -> Sync {
        let week = clock::week_minute(now, clock::local_offset_ms());
        let end = self.plan.as_ref().and_then(|p| control::current_block(p, week)).map(|(_, left)| week + left);
        match (end, self.block_since) {
            (None, _) => self.block_since = None,
            (Some(e), Some((k, _))) if e == k => {}
            (Some(e), _) => self.block_since = Some((e, now)),
        }
        let closed_port = (self.closed > 0).then(|| self.client.as_ref().map_or(0, |c| c.addr().port()));
        let down = Down::new(closed_port, self.last_ok.elapsed());
        self.snap.down = (!self.online).then_some(down);
        let s = &self.snap;
        control::sync(&SyncInput {
            lang: self.lang,
            plan: self.plan.as_ref(),
            week_minute: week,
            engine: self.engine,
            testing: self.test.is_some(),
            outside: self.outside,
            mode: s.recent(&s.mode, "mode", MODE_MAX_AGE).and_then(|m| m.mode.as_deref()),
            mode_seen: s.mode.is_some(),
            bat_power: s.fresh(&s.es, "es").and_then(|e| e.bat_power),
            active_secs: self.block_since.map_or(0, |(_, t)| ((now - t) / 1000).max(0) as u64),
            online: self.online,
            down: Some(down),
            power_cap: self.power_cap(),
        })
    }

    /// Un historique de valeurs fraîches seulement : une valeur gardée après une lecture ratée serait réécrite à tort.
    fn record_history(&self) {
        let s = &self.snap;
        let (es, em) = (s.fresh(&s.es, "es"), s.fresh(&s.em, "em"));
        let (sample, counters) = history_rows(es, em, self.soc());
        if let Err(e) = self.app.state::<History>().record(s.ts as i64, sample, counters, false) {
            self.log(Msg::LabelHistory.detail(Msg::raw(e.to_string())));
        }
    }

    /// Hors ligne depuis `OFFLINE_AFTER` : un bail DHCP a pu changer l'IP. Cherche la batterie par son adresse MAC.
    fn relocate(&mut self) {
        let Some(addr) = self.client.as_ref().map(Client::addr) else { return };
        let due = self.last_search.is_none_or(|t| t.elapsed() >= RECOVERY_EVERY);
        // Port fermé : l'IP répond, la relocalisation par MAC ne servirait à rien (voir `recover_port`).
        if self.online || self.closed > 0 || self.last_ok.elapsed() < offline_after(self.base_poll(), self.gap()) || !due {
            return;
        }
        let s = &self.snap;
        let mac = s.device.as_ref().and_then(|d| d.wifi_mac.clone()).or_else(|| s.wifi.as_ref().and_then(|w| w.wifi_mac.clone()));
        let Some(mac) = mac else { return };
        self.last_search = Some(Instant::now());
        let settings = settings::load(&self.app);
        let scan = settings.scan_range.filter(|r| !r.trim().is_empty());
        let mut found = Vec::new();
        // Recherche automatique : ports unitaires seulement (30000 et ceux des réglages), jamais les plages.
        let ports = api::unit_ports(&settings.ports).iter().map(u16::to_string).collect::<Vec<_>>().join(",");
        for range in std::iter::once(None).chain(scan.as_deref().map(Some)) {
            match api::discover(range, &ports, false, &AtomicBool::new(false), &mut |_| {}) {
                Ok(scan) => found.extend(scan.devices),
                Err(e) => self.log(Msg::LabelBatterySearch.detail((&e).into())),
            }
        }
        let Some(new) = api::find_moved(&found, &mac, addr.ip()).and_then(|d| Some(SocketAddr::new(d.ip.parse().ok()?, d.port))) else {
            return;
        };
        self.log(Msg::Found { new: new.ip().to_string(), old: addr.ip().to_string() });
        self.moved_to(new);
    }

    /// Reconnecte à la nouvelle adresse, la retient pour le prochain démarrage et prévient le front.
    fn moved_to(&mut self, new: SocketAddr) {
        self.connect(new);
        if let Err(e) = settings::save_last_addr(&self.app, new) {
            self.log(Msg::LabelAddrNotSaved.detail(Msg::raw(e)));
        }
        let _ = self.app.emit("connected", json!({ "ip": new.ip(), "port": new.port() }));
    }

    /// L'IP répond mais le port est fermé (port changé ou Open API coupée dans l'app Marstek) : cherche le nouveau port
    /// sur cette seule IP, doucement, au plus une fois par `RECOVERY_EVERY`.
    fn recover_port(&mut self) {
        let now = Instant::now();
        // Une réactivation vient d'être envoyée : on laisse l'appareil rouvrir l'API avant de re-sonder.
        if self.reenable_hold.is_some_and(|t| now < t) {
            return;
        }
        if self.online || !recovery_due(self.closed, self.last_recovery, now) {
            return;
        }
        let Some(addr) = self.client.as_ref().map(Client::addr) else { return };
        let IpAddr::V4(ip) = addr.ip() else { return };
        if self.last_recovery.is_none() {
            self.down_since.get_or_insert(self.last_ok);
            self.alert_down(control::offline_message(DownKind::PortClosed, Some(addr.port()), false));
        }
        self.last_recovery = Some(now);
        let setting = settings::load(&self.app).ports;
        self.log(Msg::PortSearch(ip.to_string()));
        match api::find_port(ip, &setting, &AtomicBool::new(false)) {
            Ok(Some(d)) => {
                self.log(Msg::PortFound(d.port));
                if let Some(ports) = api::with_found_ports(&setting, std::slice::from_ref(&d))
                    && let Err(e) = settings::save_ports(&self.app, &ports)
                {
                    self.log(Msg::LabelPortNotSaved.detail(Msg::raw(e)));
                }
                self.moved_to(SocketAddr::new(ip.into(), d.port));
            }
            Ok(None) => self.log(Msg::PortNotFound),
            Err(e) => self.log(Msg::LabelPortSearch.detail((&e).into())),
        }
    }

    fn update_tray(&mut self) {
        let power = self.snap.es.as_ref().and_then(|e| e.bat_power);
        let soc = self.soc();
        let mode = self.snap.mode.as_ref().and_then(|m| m.mode.as_deref());
        let key = (
            tray::level(soc),
            tray::flow(self.online, power),
            tray::tooltip(self.online, soc, power, mode, self.lang),
        );
        if self.tray.as_ref() != Some(&key) {
            tray::update(&self.app, key.0, key.1, &key.2);
            self.tray = Some(key);
        }
    }

    fn emit_control(&mut self) {
        let c = match (self.engine.latch, self.engine.manual_until) {
            (Some(side), _) => {
                let (name, rule) = match side {
                    Side::Low => (Msg::RuleLow, &self.rules.low),
                    Side::High => (Msg::RuleHigh, &self.rules.high),
                };
                Control { source: Source::Rule, detail: name.detail(Msg::Mode(rule.mode)).text(self.lang) }
            }
            (None, Some(_)) => Control { source: Source::Manual, detail: self.manual_label.text(self.lang) },
            (None, None) => match self.engine.hybrid {
                Some(Hybrid { drive: Drive::Mode(mode), .. }) => {
                    Control { source: Source::Hybrid, detail: Msg::Block(mode).text(self.lang) }
                }
                Some(Hybrid { drive: Drive::Hold, .. }) => {
                    Control { source: Source::Hybrid, detail: Msg::SocLimitHold.text(self.lang) }
                }
                None => {
                    let plan = self.plan.as_ref().map_or(Msg::NoActivePlan, |p| Msg::raw(p.name.clone()));
                    let detail = if self.engine.suspended { Msg::ReadOnlyPlan(Box::new(plan)) } else { plan };
                    Control { source: Source::Plan, detail: detail.text(self.lang) }
                }
            },
        };
        if self.control.as_ref() != Some(&c) {
            let _ = self.app.emit("control", &c);
            self.control = Some(c);
        }
    }

    /// Une seule passe de décision après chaque sondage. Rien ne se décide hors ligne.
    fn tick(&mut self) {
        if self.client.is_none() {
            return;
        }
        self.alerts();
        if !self.online {
            return;
        }
        let now = clock::now_ms();
        // shortcut: règles SOC et reprise du plan suspendues pendant le test (10 min) ; à fusionner si ça gêne.
        if self.test.is_some() {
            return self.test_step(now);
        }
        let (mut next, action) = control::decide(&self.rules, self.engine, self.soc(), now);
        let done = match action {
            Action::None => true,
            Action::Rule(side) => self.apply_rule(side),
            Action::HandBack => self.hand_back(),
        };
        // Un échec garde l'ancien état : la même décision est retentée au prochain sondage.
        if done {
            if action != Action::None {
                // La règle ou le plan vient de réécrire la batterie : le pilotage hybride est à recalculer.
                next.hybrid = None;
            }
            self.engine = next;
            // Règles SOC et action manuelle passent avant le pilotage hybride.
            if action == Action::None && !next.suspended && next.latch.is_none() && next.manual_until.is_none() {
                self.hybrid_tick(now);
            }
            if action == Action::None {
                self.reapply_tick(now);
            }
        }
        self.emit_control();
    }

    /// Réécrit le plan si la batterie l'a perdu (voir `Reapply`) ; seulement avec le pilotage automatique et un plan envoyé,
    /// jamais pendant une action manuelle, une règle SOC ou un bloc piloté par l'application.
    fn reapply_tick(&mut self, now: i64) {
        let e = self.engine;
        let idle = self.reapply_plan && !e.suspended && self.plan.is_some() && e.latch.is_none() && e.manual_until.is_none() && e.hybrid.is_none();
        let s = &self.snap;
        let read = s.recent(&s.mode, "mode", MODE_MAX_AGE).and_then(|m| m.mode.clone());
        let at = s.updated.get("mode").map_or(0, |&t| t as i64);
        if self.reapply.step(read.as_deref().map(|r| (r, at)), idle, now) {
            self.log(Msg::Reapplied(read.unwrap_or_default()));
            self.hand_back();
        }
    }

    fn hybrid_tick(&mut self, now: i64) {
        let week = clock::week_minute(now, clock::local_offset_ms());
        let holding = matches!(self.engine.hybrid, Some(Hybrid { drive: Drive::Hold, .. }));
        let wanted = control::hybrid_wanted(self.plan.as_ref(), self.outside, self.soc(), now, week, holding);
        let (done, next) = match control::hybrid_step(self.engine.hybrid, wanted) {
            HybridAction::None => return,
            HybridAction::Apply(h) => (self.apply_hybrid(h, now), Some(h)),
            HybridAction::HandBack => (self.hand_back(), None),
        };
        if done {
            self.engine.hybrid = next;
        }
    }

    fn apply_hybrid(&mut self, h: Hybrid, now: i64) -> bool {
        let (req, what) = match h.drive {
            Drive::Mode(mode) => (mode.req(), Msg::Block(mode)),
            Drive::Hold => {
                let seconds = control::hold_seconds(h.until, now);
                (ModeReq::Passive { direction: Direction::Charge, power: 0, seconds }, Msg::SocLimit)
            }
        };
        let r = self.exec(Write::Mode(req));
        let ok = self.settle(&r);
        if ok {
            self.mode_changed(&Msg::AppDriven(Box::new(what)));
        }
        ok
    }

    fn alerts(&mut self) {
        let soc = self.soc();
        let (state, send) = control::low_soc_alert(&self.notify, soc, self.low_notified);
        self.low_notified = state;
        if send {
            self.notify_user(Msg::TitleLowBattery, &Msg::raw(format!("SOC {:.0} %", soc.unwrap_or_default())));
        }
        // `enabled` toujours vrai : le journal garde chaque incident, la notification dépend du réglage.
        let (state, first) = control::offline_alert(true, self.last_ok.elapsed(), offline_after(self.base_poll(), self.gap()), self.offline_notified);
        self.offline_notified = state;
        // Port fermé : `recover_port` a déjà prévenu.
        if first && self.closed == 0 {
            self.down_since.get_or_insert(self.last_ok);
            // Un seul ping par incident, jamais de requête vers le port de la batterie.
            let ping_ok = self.client.as_ref().is_some_and(|c| api::ping(c.addr().ip()));
            self.alert_down(control::offline_message(DownKind::Silent, None, ping_ok));
        }
        if let Some(outage) = control::back_alert(&mut self.down_since, self.online) {
            let msg = control::back_message(outage, !self.engine.suspended || self.plan.is_some());
            self.log(msg.clone());
            if self.notify.offline {
                self.notify_user(Msg::TitleReconnected, &msg);
            }
        }
    }

    fn alert_down(&self, msg: Msg) {
        self.log(msg.clone());
        if self.notify.offline {
            self.notify_user(Msg::TitleUnreachable, &msg);
        }
    }

    /// Notification d'échec d'une écriture du moteur, une seule par série d'échecs.
    fn settle(&mut self, r: &Result<Vec<bool>, Msg>) -> bool {
        let ok = all_accepted(r);
        if !ok && !self.engine_failed && self.notify.write_failed {
            self.notify_user(Msg::TitleWriteFailed, &Msg::AutoWriteRefused);
        }
        self.engine_failed = !ok;
        ok
    }

    fn mode_changed(&self, detail: &Msg) {
        if self.notify.mode_changed {
            self.notify_user(Msg::TitleModeChanged, detail);
        }
    }

    fn apply_rule(&mut self, side: Side) -> bool {
        let (high, mode) = match side {
            Side::Low => (false, self.rules.low.mode),
            Side::High => (true, self.rules.high.mode),
        };
        let r = self.exec(Write::Mode(mode.req()));
        let ok = self.settle(&r);
        if ok {
            self.mode_changed(&Msg::RuleApplied { high, mode });
        }
        ok
    }

    /// Renvoie les 10 créneaux du plan actif : la batterie repasse en Manual et le suit seule.
    fn hand_back(&mut self) -> bool {
        let Some(plan) = self.plan.clone() else { return true };
        let slots = match compile_plan(&plan.blocks) {
            Ok(slots) => slots,
            Err(e) => {
                self.log(Msg::LabelPlan(plan.name.clone()).detail((&e).into()));
                return true;
            }
        };
        let r = self.exec(Write::Slots(slots));
        let ok = self.settle(&r);
        if ok {
            self.mode_changed(&Msg::PlanTakesOver(plan.name.clone()));
        }
        ok
    }

    /// Écriture demandée par l'utilisateur : retient le plan ou l'action manuelle si la batterie accepte.
    fn write(&mut self, w: Write) -> Result<Vec<bool>, Msg> {
        if self.test.is_some() && matches!(w, Write::Mode(_) | Write::Slots(_) | Write::Plan(..)) {
            return Err(Msg::TestRunning);
        }
        let after = match &w {
            Write::Mode(m) => After::Manual(mode_label(m), passive_seconds(m)),
            Write::Plan(p, _) => After::Plan(p.clone()),
            _ => After::Nothing,
        };
        let label = w.label();
        let r = self.exec(w);
        if all_accepted(&r) {
            match after {
                After::Manual(label, seconds) => {
                    let now = clock::now_ms();
                    let week = clock::week_minute(now, clock::local_offset_ms());
                    self.engine.manual_until = Some(match seconds {
                        Some(s) => control::passive_until(self.plan.as_ref(), now, week, s),
                        None => control::manual_until(self.plan.as_ref(), now, week),
                    });
                    self.engine.latch = None;
                    self.engine.hybrid = None;
                    self.manual_label = label;
                }
                After::Plan(plan) => {
                    if let Err(e) = settings::save_sent_plan(&self.app, &plan) {
                        self.log(Msg::LabelPlanNotSaved.detail(Msg::raw(e)));
                    }
                    self.plan = Some(plan);
                    self.engine.manual_until = None;
                    self.engine.latch = None;
                    self.engine.hybrid = None;
                }
                After::Nothing => {}
            }
            self.emit_control();
        } else if self.notify.write_failed {
            let detail = r.as_ref().err().cloned().unwrap_or(Msg::Refused);
            self.notify_user(Msg::TitleWriteFailed, &label.detail(detail));
        }
        r
    }

    /// Le sondage est suspendu pendant l'écriture (même thread), puis le mode est relu une fois.
    fn exec(&mut self, w: Write) -> Result<Vec<bool>, Msg> {
        let cap = self.power_cap();
        let (w, lowered) = cap_write(w, cap);
        if let Some(from) = lowered {
            self.log(Msg::PowerLowered { from, cap });
        }
        let Some(c) = self.client.as_mut() else { return Err(Msg::NoBattery) };
        let _ = self.app.emit("busy", true);
        let label = w.label();
        let app = &self.app;
        c.begin_poll();
        let r = run(c, w, &|m| i18n::log(app, &m), &mut self.pace);
        let _ = app.emit("busy", false);
        self.log(label.detail(match &r {
            Ok(v) if v.iter().all(|&ok| ok) => Msg::Accepted,
            Ok(_) => Msg::Refused,
            Err(e) => e.clone(),
        }));
        self.confirm();
        r
    }

    /// Après une écriture : une seule relecture, ES.GetMode, au moins `MIN_GAP_ANY` après la dernière requête.
    fn confirm(&mut self) {
        let Some(c) = self.client.as_mut() else { return };
        self.pace.wait(Kind::Any);
        let now = clock::now_ms() as u64;
        let r = c.es_mode();
        self.pace.done(Instant::now());
        let s = &mut self.snap;
        step(&self.app, Part::Mode, "mode", r, &mut s.mode, &mut s.updated, now);
        s.requests = c.requests().to_vec();
        s.link = Link { rssi: s.wifi.as_ref().and_then(|w| w.rssi), ..c.link() };
        self.emit_status();
        self.update_tray();
    }

    fn emit_test(&self, samples: &[Point], verdict: Option<control::Verdict>) {
        let _ = self.app.emit("slot_test", SlotTest { samples: samples.to_vec(), verdict });
    }

    /// Écrit le créneau de test, puis laisse les sondages suivants relever la puissance.
    fn start_test(&mut self) -> Result<(), Msg> {
        if self.client.is_none() {
            return Err(Msg::NoBattery);
        }
        if self.test.is_some() {
            return Err(Msg::TestAlready);
        }
        let now = clock::now_ms();
        let slot = control::test_slot(clock::week_minute(now, clock::local_offset_ms()))?;
        let r = self.exec(Write::Slots(vec![slot]));
        if !all_accepted(&r) {
            return Err(r.err().unwrap_or(Msg::TestRefused));
        }
        let slot_end = now - now.rem_euclid(MINUTE) + i64::from(control::TEST_MINUTES) * MINUTE;
        self.test = Some(Test { slot_end, ends: slot_end + i64::from(control::TEST_MINUTES) * MINUTE, samples: Vec::new() });
        self.emit_test(&[], None);
        Ok(())
    }

    fn test_step(&mut self, now: i64) {
        let power = self.snap.es.as_ref().and_then(|e| e.bat_power);
        let Some(t) = self.test.as_mut() else { return };
        t.samples.push(Point { ts: now, bat_power: power });
        if now < t.ends {
            let samples = t.samples.clone();
            return self.emit_test(&samples, None);
        }
        let Some(t) = self.test.take() else { return };
        let verdict = control::verdict(&t.samples, t.slot_end);
        // Rend la batterie au plan actif, ou désactive tout (10 créneaux éteints) sans plan.
        let slots = self.plan.as_ref().and_then(|p| compile_plan(&p.blocks).ok()).unwrap_or_default();
        let r = self.exec(Write::Slots(slots));
        self.settle(&r);
        match verdict {
            Some(v) => {
                self.outside = Some(v);
                if let Err(e) = settings::save_outside_slots(&self.app, json!({ "verdict": v, "ts": now })) {
                    self.log(Msg::LabelTestNotSaved.detail(Msg::raw(e)));
                }
            }
            // shortcut: sans mesure après la fin (batterie coupée), le test se termine sans verdict ni résultat retenu.
            None => self.log(Msg::TestNoResult),
        }
        self.emit_test(&t.samples, verdict);
        self.reset_engine();
        self.emit_control();
    }
}

/// Lectures mises de côté : par méthode, timeouts d'affilée et date de la prochaine tentative.
#[derive(Default)]
struct Aside(HashMap<&'static str, (u32, Option<Instant>)>);

impl Aside {
    fn due(&self, key: &str, now: Instant) -> bool {
        self.0.get(key).and_then(|e| e.1).is_none_or(|t| now >= t)
    }

    /// Note le résultat d'une lecture ; `true` quand elle vient d'être mise de côté (à journaliser une fois).
    fn record(&mut self, key: &'static str, timed_out: bool, now: Instant) -> bool {
        if !timed_out {
            self.0.remove(key);
            return false;
        }
        let e = self.0.entry(key).or_default();
        e.0 += 1;
        if e.0 >= ASIDE_AFTER {
            e.1 = Some(now + ASIDE_FOR);
        }
        e.0 == ASIDE_AFTER
    }
}

/// Garde la valeur lue et sa date ; rend `true` si la liaison est perdue (pas de réponse), pas si la batterie refuse.
fn step<T>(
    app: &AppHandle,
    part: Part,
    key: &'static str,
    r: Result<T, ApiError>,
    slot: &mut Option<T>,
    updated: &mut HashMap<&'static str, u64>,
    now: u64,
) -> bool {
    match r {
        Ok(v) => {
            *slot = Some(v);
            updated.insert(key, now);
            false
        }
        Err(e) => {
            // Port fermé : dit une fois par `recover_port`, pas à chaque sondage.
            if !matches!(e, ApiError::PortClosed(_)) {
                i18n::log(app, &Msg::Section(part).detail((&e).into()));
            }
            matches!(e, ApiError::Timeout | ApiError::Io(_) | ApiError::PortClosed(_))
        }
    }
}

/// Rejoue une fois les créneaux dont l'envoi a échoué : une seule perte de paquet ne doit pas laisser un plan à moitié écrit.
fn second_pass(
    mut results: Vec<Result<bool, ApiError>>,
    announce: impl FnOnce(&[usize]),
    mut send: impl FnMut(usize) -> Result<bool, ApiError>,
) -> Vec<Result<bool, ApiError>> {
    let failed: Vec<usize> = (0..results.len()).filter(|&i| results[i].is_err()).collect();
    // Tout a échoué : la liaison est coupée, rejouer doublerait l'attente (3 tentatives par créneau) pour rien.
    if !failed.is_empty() && failed.len() < results.len() {
        announce(&failed);
        for i in failed {
            results[i] = send(i);
        }
    }
    results
}

fn slot_numbers(indexes: &[usize]) -> String {
    indexes.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(", ")
}

/// Résultat par créneau ; l'erreur nomme ceux que la batterie n'a jamais confirmés.
fn slots_outcome(results: Vec<Result<bool, ApiError>>) -> Result<Vec<bool>, Msg> {
    let missing: Vec<usize> = (0..results.len()).filter(|&i| results[i].is_err()).collect();
    if !missing.is_empty() {
        return Err(Msg::SlotsMissing(slot_numbers(&missing)));
    }
    Ok(results.into_iter().map(|r| r.unwrap_or(false)).collect())
}

/// Ramène toute puissance de l'écriture au plafond ; rend aussi la plus haute valeur réduite.
fn cap_write(w: Write, cap: u16) -> (Write, Option<u16>) {
    let mut top = None;
    let mut lower = |p: &mut u16| {
        if *p > cap {
            top = top.max(Some(*p));
            *p = cap;
        }
    };
    let w = match w {
        Write::Mode(ModeReq::Passive { direction, mut power, seconds }) => {
            lower(&mut power);
            Write::Mode(ModeReq::Passive { direction, power, seconds })
        }
        Write::Slots(mut slots) => {
            slots.iter_mut().for_each(|s| lower(&mut s.power));
            Write::Slots(slots)
        }
        Write::Plan(plan, mut slots) => {
            slots.iter_mut().for_each(|s| lower(&mut s.power));
            Write::Plan(plan, slots)
        }
        w => w,
    };
    (w, top)
}

/// Une écriture, `MIN_GAP_ANY` après la dernière requête ; ses créneaux sont espacés de `SLOT_GAP`.
fn run(c: &mut Client, w: Write, progress: &dyn Fn(Msg), pace: &mut Pace) -> Result<Vec<bool>, Msg> {
    pace.wait(Kind::Any);
    let r = match w {
        Write::Mode(m) => {
            let cfg = m.config();
            let sent = c.set_mode(&cfg);
            pace.done(Instant::now());
            let r = sent.or_else(|e| {
                // Réponse perdue, ordre peut-être passé : la batterie dit dans quel mode elle est.
                // shortcut: le mode passif n'est pas confirmé (sa puissance n'est pas comparée, un passif déjà actif validerait à tort) ; à affiner si ça gêne.
                if !matches!(e, ApiError::Timeout) || matches!(cfg, ModeConfig::Passive { .. }) {
                    return Err(e);
                }
                pace.wait(Kind::Any);
                let mode = c.es_mode();
                pace.done(Instant::now());
                match mode.ok().and_then(|m| m.mode) {
                    Some(mode) if mode == cfg.name() => Ok(true),
                    _ => Err(e),
                }
            });
            r.map(|ok| vec![ok]).map_err(|e| Msg::from(&e))
        }
        Write::Slots(slots) | Write::Plan(_, slots) => {
            let slots = manual_slots(&slots).map_err(|e| Msg::from(&e))?;
            let mut send = |i: usize| {
                pace.wait(Kind::Slot);
                let r = c.set_mode(&ModeConfig::Manual(slots[i].clone()));
                pace.done(Instant::now());
                r
            };
            let first = (0..slots.len()).map(&mut send).collect();
            let announce = |failed: &[usize]| progress(Msg::SlotsRetry(slot_numbers(failed)));
            slots_outcome(second_pass(first, announce, send))
        }
        Write::Dod(v) => c.set_dod(v).map(|ok| vec![ok]).map_err(|e| Msg::from(&e)),
        Write::Led(on) => c.set_led(on).map(|ok| vec![ok]).map_err(|e| Msg::from(&e)),
        Write::Ble(on) => c.set_ble(on).map(|ok| vec![ok]).map_err(|e| Msg::from(&e)),
    };
    pace.done(Instant::now());
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_converts_pv_counter_and_gates_grid_on_the_meter() {
        let es = EsStatus { total_pv_energy: Some(123.0), total_load_energy: Some(500.0), ongrid_power: Some(9.0), ..Default::default() };
        let em = |ct| EmStatus { ct_state: ct, total_power: Some(250.0), ..Default::default() };
        let (s, c) = history_rows(Some(&es), Some(&em(Some(1))), Some(50.0));
        assert_eq!((s.grid, c.pv, c.load), (Some(250.0), Some(1230.0), Some(500.0)));
        assert_eq!(history_rows(Some(&es), Some(&em(Some(0))), None).0.grid, None);
        assert_eq!(history_rows(Some(&es), None, None).0.grid, None);
    }

    #[test]
    fn calibration_speeds_up_when_stable_backs_off_and_locks_on_a_crash() {
        let secs = Duration::from_secs;
        // Assez de cycles sains d'affilée : -5 s pour tenter plus vite.
        assert_eq!(tune_step(secs(120), secs(45), TUNE_STABLE_CYCLES - 1, false), (secs(115), secs(45), 0));
        // Pas encore : on continue de compter, la cadence ne bouge pas.
        assert_eq!(tune_step(secs(120), secs(45), 0, false), (secs(120), secs(45), 1));
        // Décrochage : +5 s et ce niveau devient le plancher (verrouille, plus de descente).
        assert_eq!(tune_step(secs(90), secs(45), 3, true), (secs(95), secs(95), 0));
        // Au plancher : on ne descend pas plus bas.
        assert_eq!(tune_step(secs(45), secs(45), TUNE_STABLE_CYCLES, false), (secs(45), secs(45), TUNE_STABLE_CYCLES + 1));
        // Décrochage plafonné au sondage par défaut (jamais au-dessus).
        assert_eq!(tune_step(secs(240), secs(60), 0, true).0, secs(240));
    }

    #[test]
    fn reenable_backs_off_from_immediate_to_an_hour() {
        let secs = |t| reenable_wait(t).as_secs();
        // 1re tentative immédiate, puis 15, 30, 60 min, plafonné à 60.
        assert_eq!([secs(0), secs(1), secs(2), secs(3), secs(4)], [0, 900, 1800, 3600, 3600]);
    }

    #[test]
    fn cap_write_lowers_passive_and_slots_and_reports_the_highest() {
        let slot = |power| Slot { start: "08:00".into(), end: "09:00".into(), days: 127, direction: Direction::Charge, power, enabled: true };
        let (w, lowered) = cap_write(Write::Mode(ModeReq::Passive { direction: Direction::Charge, power: 3000, seconds: 60 }), 1200);
        assert!(matches!(w, Write::Mode(ModeReq::Passive { power: 1200, .. })) && lowered == Some(3000));
        let (w, lowered) = cap_write(Write::Slots(vec![slot(500), slot(2000), slot(1500)]), 1200);
        let Write::Slots(s) = w else { panic!() };
        assert_eq!((s.iter().map(|s| s.power).collect::<Vec<_>>(), lowered), (vec![500, 1200, 1200], Some(2000)));
        assert_eq!(cap_write(Write::Slots(vec![slot(500)]), 1200).1, None);
    }

    #[test]
    fn passive_seconds_only_for_passive_mode() {
        assert_eq!(passive_seconds(&ModeReq::Auto), None);
        assert_eq!(passive_seconds(&ModeReq::Passive { direction: Direction::Charge, power: 100, seconds: 90 }), Some(90));
    }

    #[test]
    fn poll_interval_backs_off_from_the_base_to_the_larger_of_a_minute_and_the_base() {
        let secs = |base, n| poll_interval(Duration::from_secs(base), n).as_secs();
        assert_eq!((0..6).map(|n| secs(30, n)).collect::<Vec<_>>(), [30, 60, 60, 60, 60, 60]);
        assert_eq!((0..4).map(|n| secs(60, n)).collect::<Vec<_>>(), [60, 60, 60, 60]);
        assert_eq!((0..4).map(|n| secs(120, n)).collect::<Vec<_>>(), [120, 120, 120, 120], "base au-dessus d'une minute : pas de plafond plus bas");
        assert_eq!(secs(600, u32::MAX), 600);
    }

    #[test]
    fn base_poll_defaults_to_240_s_and_the_choice_is_bounded() {
        let secs = |chosen| base_poll(chosen).as_secs();
        assert_eq!(secs(None), 240);
        assert_eq!((secs(Some(45)), secs(Some(30))), (45, 30), "le choix de l'utilisateur prime");
        assert_eq!((secs(Some(1)), secs(Some(9999))), (30, 600), "bornes 30 à 600 s");
    }

    #[test]
    fn a_cycle_lasts_three_minutes_at_least_and_the_offline_alert_waits_two_cycles() {
        let secs = Duration::from_secs;
        assert_eq!((cycle_period(secs(30), MIN_GAP), cycle_period(secs(240), MIN_GAP)), (secs(180), secs(240)));
        assert_eq!(offline_after(secs(240), MIN_GAP), secs(600));
        assert_eq!(offline_after(secs(600), MIN_GAP), secs(1260));
        // Cadence calibrée rapide : le gap (période / 3) descend, et le cycle le suit vraiment.
        assert_eq!(cycle_period(secs(45), secs(15)), secs(45));
    }

    /// `polls` sondages espacés de `every` secondes : la lecture secondaire que choisit `next_extra` à chacun.
    fn schedule(every: u64, polls: u64) -> Vec<Option<Extra>> {
        let (t0, aside, mut last) = (Instant::now(), Aside::default(), HashMap::new());
        (0..polls)
            .map(|i| {
                let now = t0 + Duration::from_secs(every * i);
                let extra = next_extra(&last, &aside, last.contains_key(&Extra::Device), now);
                if let Some(e) = extra {
                    last.insert(e, now);
                }
                extra
            })
            .collect()
    }

    fn no_pause() -> Pace {
        Pace { read_gap: Duration::ZERO, any_gap: Duration::ZERO, slot_gap: Duration::ZERO, ..Pace::new() }
    }

    #[test]
    fn reads_are_spread_over_the_interval_with_a_sixty_second_floor() {
        let secs = Duration::from_secs;
        assert_eq!(tick_gap(secs(240), plan(None).len(), MIN_GAP), secs(120));
        assert_eq!(tick_gap(secs(240), plan(Some(Extra::Mode)).len(), MIN_GAP), secs(80));
        assert_eq!(tick_gap(secs(30), 3, MIN_GAP), MIN_GAP, "jamais sous le gap, 60 s hors calibration");
        assert_eq!(tick_gap(secs(45), 3, secs(15)), secs(15), "cadence calibrée : le gap descend à 15 s");
        assert_eq!(plan(Some(Extra::Pv)).into_iter().collect::<Vec<_>>(), [Read::Bat, Read::Es, Read::Extra(Extra::Pv)]);
    }

    /// Temps virtuel : lectures au rythme des ticks, une écriture de plan (10 créneaux puis confirmation) à 47 s.
    /// Rend (début, famille) de chaque requête ; chaque requête dure 300 ms.
    fn timeline(interval: u64, until: u64, write_at: u64) -> Vec<(Duration, &'static str)> {
        let (t0, rtt) = (Instant::now(), Duration::from_millis(300));
        let (mut pace, mut log) = (Pace::new(), Vec::new());
        let (mut clock, mut next_read, mut write) = (Duration::ZERO, Duration::ZERO, Some(Duration::from_secs(write_at)));
        let mut send = |pace: &mut Pace, clock: &mut Duration, kind: Kind, name| {
            *clock += pace.delay(kind, t0 + *clock);
            log.push((*clock, name));
            *clock += rtt;
            pace.done(t0 + *clock);
        };
        while clock < Duration::from_secs(until) {
            if write.is_some_and(|w| w <= clock) {
                write = None;
                send(&mut pace, &mut clock, Kind::Any, "slot");
                for _ in 1..10 {
                    send(&mut pace, &mut clock, Kind::Slot, "slot");
                }
                send(&mut pace, &mut clock, Kind::Any, "confirm");
            } else if clock >= next_read {
                let wait = pace.delay(Kind::Read, t0 + clock);
                if wait.is_zero() {
                    send(&mut pace, &mut clock, Kind::Read, "read");
                    next_read = clock - rtt + tick_gap(Duration::from_secs(interval), 3, MIN_GAP);
                } else {
                    next_read = clock + wait;
                }
            } else {
                clock += Duration::from_secs(1);
            }
        }
        log
    }

    #[test]
    fn requests_keep_their_distance_whatever_the_user_does() {
        for interval in [30, 120, 240] {
            let log = timeline(interval, 3600, 647);
            for w in log.windows(2) {
                let min = match (w[0].1, w[1].1) {
                    ("slot", "slot") => SLOT_GAP,
                    (_, "read") => MIN_GAP,
                    _ => MIN_GAP_ANY,
                };
                assert!(w[1].0 - w[0].0 >= min, "{interval} s : {w:?}");
            }
        }
        // Sans écriture, trois lectures étalées sur 240 s : 0, 80 et 160 s.
        let reads: Vec<_> = timeline(240, 250, u64::MAX / 2).iter().map(|r| r.0.as_secs()).collect();
        assert_eq!(reads, [0, 80, 160, 240]);
        // Une heure à 240 s : au plus 45 requêtes, contre ~180 avant.
        assert!(timeline(240, 3600, u64::MAX / 2).len() <= 45);
    }

    #[test]
    fn a_ten_slot_plan_takes_thirty_two_seconds_and_is_confirmed_five_seconds_after_the_last_slot() {
        let (t0, mut pace, mut starts) = (Instant::now(), Pace::new(), Vec::new());
        let mut clock = Duration::ZERO;
        for kind in std::iter::once(Kind::Any).chain([Kind::Slot; 9]).chain([Kind::Any]) {
            clock += pace.delay(kind, t0 + clock);
            starts.push(clock.as_secs());
            pace.done(t0 + clock);
        }
        assert_eq!(starts, [0, 3, 6, 9, 12, 15, 18, 21, 24, 27, 32]);
    }

    #[test]
    fn a_waiting_command_comes_before_a_due_read() {
        let (tx, rx) = channel();
        assert!(matches!(next::<u8>(&rx, Duration::ZERO), Err(RecvTimeoutError::Timeout)));
        tx.send(7).unwrap();
        assert!(matches!(next(&rx, Duration::ZERO), Ok(7)));
        drop(tx);
        assert!(matches!(next(&rx, Duration::ZERO), Err(RecvTimeoutError::Disconnected)));
    }

    #[test]
    fn a_new_connection_reads_one_extra_per_poll_starting_with_the_model() {
        use Extra::*;
        let first: Vec<_> = schedule(240, 6).into_iter().flatten().collect();
        assert_eq!(first, [Device, Mode, Pv, Em, Wifi, Ble]);
    }

    #[test]
    fn extras_follow_their_period_and_none_is_starved() {
        let count = |s: &[Option<Extra>], e| s.iter().filter(|&&x| x == Some(e)).count();
        let s = schedule(240, 31); // deux heures
        // Une lecture secondaire par sondage au plus ; mode et solaire toutes les 10 min au mieux (un sondage sur trois
        // avec 240 s), les autres toutes les 30 min au mieux.
        for e in [Extra::Mode, Extra::Pv] {
            assert!(count(&s, e) >= 7, "{e:?} : {s:?}");
        }
        for e in [Extra::Device, Extra::Em, Extra::Wifi, Extra::Ble] {
            assert!((3..=5).contains(&count(&s, e)), "{e:?} : {}", count(&s, e));
        }
        // Un sondage de 15 min : tout est dû à chaque sondage, une lecture par sondage, aucune n'est perdue.
        let s = schedule(900, 31);
        assert!(s.iter().all(Option::is_some));
        for e in [Extra::Mode, Extra::Pv, Extra::Em, Extra::Wifi, Extra::Ble] {
            assert!(count(&s, e) >= 3, "{e:?}");
        }
    }

    #[test]
    fn nothing_is_read_while_no_extra_is_due() {
        let (t0, aside) = (Instant::now(), Aside::default());
        let last: HashMap<_, _> = EXTRAS.iter().map(|&(e, _)| (e, t0)).collect();
        assert_eq!(next_extra(&last, &aside, true, t0 + Duration::from_secs(599)), None);
        assert_eq!(next_extra(&last, &aside, true, t0 + Duration::from_secs(600)), Some(Extra::Mode));
    }

    #[test]
    fn the_model_is_read_at_every_poll_until_it_is_known() {
        let (t0, aside) = (Instant::now(), Aside::default());
        let last: HashMap<_, _> = EXTRAS.iter().map(|&(e, _)| (e, t0)).collect();
        assert_eq!(next_extra(&last, &aside, false, t0), Some(Extra::Device));
        assert_eq!(next_extra(&last, &aside, true, t0), None);
    }

    #[test]
    fn a_read_set_aside_is_not_scheduled() {
        let (t0, mut aside) = (Instant::now(), Aside::default());
        for _ in 0..ASIDE_AFTER {
            aside.record("device", true, t0);
        }
        assert_eq!(next_extra(&HashMap::new(), &aside, false, t0), Some(Extra::Mode));
    }

    #[test]
    fn port_search_starts_after_three_closed_polls_and_at_most_every_ten_minutes() {
        let t0 = Instant::now();
        assert!(!recovery_due(2, None, t0), "deux sondages ne suffisent pas");
        assert!(recovery_due(3, None, t0));
        assert!(!recovery_due(0, None, t0), "port ouvert : rien à chercher");
        let later = |s| t0 + Duration::from_secs(s);
        assert!(!recovery_due(9, Some(t0), later(599)));
        assert!(recovery_due(9, Some(t0), later(600)));
        assert!(!recovery_due(2, Some(t0), later(9999)));
    }

    #[test]
    fn read_is_set_aside_after_three_timeouts_then_retried_after_five_minutes() {
        let (mut a, t0) = (Aside::default(), Instant::now());
        assert!(!a.record("em", true, t0) && !a.record("em", true, t0));
        assert!(a.due("em", t0));
        assert!(a.record("em", true, t0), "journalisé à la mise de côté");
        assert!(!a.due("em", t0 + ASIDE_FOR - Duration::from_secs(1)));
        assert!(a.due("em", t0 + ASIDE_FOR));
        assert!(!a.record("em", true, t0 + ASIDE_FOR), "pas de second message");
        assert!(!a.due("em", t0 + ASIDE_FOR + Duration::from_secs(1)));
        assert!(!a.record("em", false, t0 + ASIDE_FOR));
        assert!(a.due("em", t0 + ASIDE_FOR + Duration::from_secs(1)), "une réponse remet à zéro");
        assert!(!a.record("em", true, t0) && !a.record("em", true, t0), "compteur reparti de zéro");
    }

    fn lost() -> Result<bool, ApiError> {
        Err(ApiError::Timeout)
    }

    #[test]
    fn second_pass_only_replays_failed_slots() {
        let first = vec![Ok(true), lost(), Ok(false), lost()];
        let (mut sent, mut announced) = (Vec::new(), Vec::new());
        let r = second_pass(first, |f| announced = f.to_vec(), |i| {
            sent.push(i);
            if i == 1 { Ok(true) } else { lost() }
        });
        assert_eq!((sent, announced), (vec![1, 3], vec![1, 3]));
        assert_eq!(slots_outcome(r.into_iter().collect()).unwrap_err(), Msg::SlotsMissing("4".into()));
    }

    #[test]
    fn second_pass_skips_replay_when_every_slot_failed() {
        let r = second_pass(vec![lost(), lost()], |_| panic!("liaison coupée"), |_| panic!("liaison coupée"));
        assert_eq!(slots_outcome(r).unwrap_err(), Msg::SlotsMissing("1, 2".into()));
    }

    #[test]
    fn second_pass_stays_quiet_when_everything_was_answered() {
        let r = second_pass(vec![Ok(true), Ok(false)], |_| panic!("rien à rejouer"), |_| panic!("rien à rejouer"));
        assert_eq!(slots_outcome(r).unwrap(), [true, false]);
    }

    #[test]
    fn unconfirmed_slots_are_all_named() {
        let err = slots_outcome(vec![lost(), Ok(true), lost()]).unwrap_err();
        assert_eq!(err, Msg::SlotsMissing("1, 3".into()));
    }

    #[test]
    fn fresh_ignores_sections_kept_from_an_earlier_poll() {
        let mut s = Snapshot { ts: 20, since: 20, ..Default::default() };
        s.es = Some(EsStatus::default());
        s.bat = Some(BatStatus::default());
        s.updated.insert("es", 20);
        s.updated.insert("bat", 10);
        assert!(s.fresh(&s.es, "es").is_some());
        assert!(s.fresh(&s.bat, "bat").is_none());
        assert!(s.fresh(&s.em, "em").is_none());
        s.mode = Some(EsMode::default());
        s.updated.insert("mode", 15);
        assert!(s.fresh(&s.mode, "mode").is_none() && s.recent(&s.mode, "mode", 5).is_some() && s.recent(&s.mode, "mode", 4).is_none());
    }

    #[test]
    fn lost_set_result_counts_when_the_mode_is_applied() {
        use std::net::UdpSocket;
        let server = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        server.set_read_timeout(Some(Duration::from_millis(400))).unwrap();
        let mut client = Client::new(server.local_addr().unwrap()).unwrap();
        client.waits = [Duration::from_millis(40); 3];
        let t = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok((n, from)) = server.recv_from(&mut buf) {
                let req: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
                // La commande est ignorée (réponse perdue), la lecture du mode répond.
                if req["method"] == "ES.GetMode" {
                    let reply = json!({ "id": req["id"], "result": { "mode": "UPS" } });
                    server.send_to(&serde_json::to_vec(&reply).unwrap(), from).unwrap();
                }
            }
        });
        assert_eq!(run(&mut client, Write::Mode(ModeReq::Ups), &|_| (), &mut no_pause()), Ok(vec![true]));
        assert!(run(&mut client, Write::Mode(ModeReq::Auto), &|_| (), &mut no_pause()).is_err());
        t.join().unwrap();
    }

    #[test]
    fn the_battery_is_offline_only_when_both_base_reads_failed_in_the_cycle() {
        let cycle = |bat_ok, es_ok| Cycle { bat_ok, es_ok, ..Cycle::default() };
        assert!(cycle(false, true).online(), "Bat.GetStatus perdu, ES.GetStatus reçu");
        assert!(cycle(true, false).online(), "l'inverse");
        assert!(!cycle(false, false).online());
    }

    #[test]
    fn extras_are_skipped_only_when_both_base_reads_are_lost() {
        assert!(extras_allowed(true, false));
        assert!(extras_allowed(false, true));
        assert!(extras_allowed(false, false));
        assert!(!extras_allowed(true, true));
    }
}
