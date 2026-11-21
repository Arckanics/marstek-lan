//! Thread unique qui possède le client UDP : une requête à la fois, jamais en parallèle.
//! Il porte aussi le moteur de pilotage (plan, règles SOC, action manuelle) et les alertes.

use crate::api::{
    self, ApiError, BatStatus, BleStatus, Client, DeviceInfo, Direction, EmStatus, EsMode, EsStatus, Link, ModeConfig,
    ModeReq, PvStatus, Slot, WifiStatus, manual_slots,
};
use crate::clock::{self, MINUTE};
use crate::control::{self, Action, Control, Engine, OFFLINE_AFTER, Point, Side, SlotTest, Source};
use crate::history::{Counters, History, Sample};
use crate::plan::{Plan, compile_plan};
use crate::settings::{self, Notify, Rules};
use crate::tray::{self, Flow};
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// La batterie supporte mal les requêtes trop rapprochées.
const PAUSE: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_secs(10);
const POLL_MAX: Duration = Duration::from_secs(60);

/// Hors ligne, le sondage s'espace (10, 20, 40, 60 s) pour ne pas enchaîner les timeouts ; un succès le ramène à 10 s.
fn poll_interval(failures: u32) -> Duration {
    (POLL * (1 << failures.min(3))).min(POLL_MAX)
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
    fn label(&self) -> &'static str {
        match self {
            Self::Mode(_) => "Changement de mode",
            Self::Slots(_) => "Écriture des créneaux",
            Self::Plan(..) => "Envoi du planning",
            Self::Dod(_) => "Profondeur de décharge",
            Self::Led(_) => "LED",
            Self::Ble(_) => "Bluetooth",
        }
    }
}

/// Ce qu'il faut retenir d'une écriture réussie.
enum After {
    Nothing,
    /// Libellé du mode, et durée d'un mode passif.
    Manual(String, Option<u32>),
    Plan(Plan),
}

enum Cmd {
    Connect(SocketAddr),
    Refresh,
    Reload,
    Suspend(bool),
    SlotTest(Sender<Result<(), String>>),
    Write(Write, Sender<Result<Vec<bool>, String>>),
}

pub struct Worker(Sender<Cmd>);

impl Worker {
    pub fn spawn(app: AppHandle) -> Self {
        let (tx, rx) = channel();
        thread::spawn(move || {
            let mut state = State::new(app);
            state.reload();
            loop {
                match rx.recv_timeout(poll_interval(state.fails)) {
                    Ok(Cmd::Connect(addr)) => state.connect(addr),
                    Ok(Cmd::Refresh) => state.poll(true),
                    Ok(Cmd::Reload) => {
                        state.reload();
                        state.tick();
                    }
                    Ok(Cmd::Suspend(on)) => {
                        state.engine.suspended = on;
                        state.tick();
                    }
                    Ok(Cmd::SlotTest(reply)) => {
                        let _ = reply.send(state.start_test());
                    }
                    Ok(Cmd::Write(w, reply)) => {
                        let r = state.write(w);
                        let _ = reply.send(r);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        state.poll(false);
                        state.tick();
                        state.relocate();
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self(tx)
    }

    fn send(&self, cmd: Cmd) -> Result<(), String> {
        self.0.send(cmd).map_err(|_| "le thread réseau s'est arrêté".to_owned())
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

    pub fn suspend(&self, on: bool) -> Result<(), String> {
        self.send(Cmd::Suspend(on))
    }

    /// Bloque jusqu'à l'écriture du créneau de test ; la suite se suit par l'événement `slot_test`.
    pub fn slot_test(&self) -> Result<(), String> {
        let (tx, rx) = channel();
        self.send(Cmd::SlotTest(tx))?;
        rx.recv().map_err(|_| "le thread réseau s'est arrêté".to_owned())?
    }

    /// Bloque jusqu'à la fin de l'écriture (à appeler hors du thread principal).
    pub fn write(&self, w: Write) -> Result<Vec<bool>, String> {
        let (tx, rx) = channel();
        self.send(Cmd::Write(w, tx))?;
        rx.recv().map_err(|_| "le thread réseau s'est arrêté".to_owned())?
    }

    pub fn write_one(&self, w: Write) -> Result<bool, String> {
        self.write(w).map(|r| r[0])
    }
}

macro_rules! snapshot {
    ($($field:ident: $ty:ty),*) => {
        /// Dernier état connu, émis tel quel au front.
        #[derive(Default, Serialize)]
        struct Snapshot {
            /// Date du sondage, ms depuis l'epoch.
            ts: u64,
            /// Date de la dernière lecture réussie de chaque section : une lecture ratée garde l'ancienne valeur.
            updated: HashMap<&'static str, u64>,
            link: Link,
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
    /// La section si elle a été lue pendant le dernier sondage, sinon `None`.
    fn fresh<'a, T>(&self, section: &'a Option<T>, key: &str) -> Option<&'a T> {
        section.as_ref().filter(|_| self.updated.get(key) == Some(&self.ts))
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
    engine: Engine,
    manual_label: String,
    control: Option<Control>,
    test: Option<Test>,
    /// Le dernier sondage a répondu.
    online: bool,
    last_ok: Instant,
    /// Sondages ratés d'affilée.
    fails: u32,
    last_search: Option<Instant>,
    low_notified: bool,
    offline_notified: bool,
    engine_failed: bool,
    tray: Option<(u8, Flow, String)>,
}

fn mode_label(m: &ModeReq) -> String {
    match m {
        ModeReq::Auto => "Auto".into(),
        ModeReq::Ai => "IA".into(),
        ModeReq::Ups => "UPS".into(),
        ModeReq::Passive { direction, power, .. } => {
            let d = if *direction == Direction::Charge { "charge" } else { "décharge" };
            format!("Passif {d} {power} W")
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

fn all_accepted(r: &Result<Vec<bool>, String>) -> bool {
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
            engine: Engine::default(),
            manual_label: String::new(),
            control: None,
            test: None,
            online: false,
            last_ok: Instant::now(),
            fails: 0,
            last_search: None,
            low_notified: false,
            offline_notified: false,
            engine_failed: false,
            tray: None,
        }
    }

    fn log(&self, msg: String) {
        let _ = self.app.emit("log", msg);
    }

    fn notify_user(&self, title: &str, body: String) {
        if let Err(e) = self.app.notification().builder().title(title).body(body).show() {
            self.log(format!("Notification : {e}"));
        }
    }

    /// Le plan actif est le dernier plan envoyé à la batterie (`sentPlan` du store).
    fn reload(&mut self) {
        let s = settings::load(&self.app);
        self.plan = s.sent_plan;
        self.rules = s.rules;
        self.notify = s.notify;
        self.emit_control();
    }

    fn soc(&self) -> Option<f64> {
        let s = &self.snap;
        s.fresh(&s.es, "es").and_then(|e| e.bat_soc).or_else(|| s.fresh(&s.bat, "bat").and_then(|b| b.soc))
    }

    /// Moteur à l'état de connexion : le plan est renvoyé au premier sondage.
    fn reset_engine(&mut self) {
        self.engine = Engine { suspended: self.engine.suspended, manual_until: Some(i64::MIN), ..Engine::default() };
    }

    fn connect(&mut self, addr: SocketAddr) {
        match Client::new(addr) {
            Ok(c) => {
                self.client = Some(c);
                self.snap = Snapshot::default();
                self.reset_engine();
                self.test = None;
                self.last_ok = Instant::now();
                self.fails = 0;
                self.last_search = None;
                self.offline_notified = false;
                self.log(format!("Connexion à {addr}"));
                self.poll(true);
            }
            Err(e) => self.log(format!("Connexion : {e}")),
        }
    }

    /// Lit chaque bloc séparément : un appel en échec n'empêche pas les autres et garde la dernière valeur connue.
    /// `full` relit aussi les infos statiques (appareil, Wi-Fi, BLE).
    fn poll(&mut self, full: bool) {
        let Some(c) = self.client.as_mut() else { return };
        let (app, s) = (&self.app, &mut self.snap);
        let now = clock::now_ms() as u64;
        macro_rules! read {
            ($key:ident, $name:expr, $call:expr) => {
                step(app, $name, stringify!($key), $call, &mut s.$key, &mut s.updated, now)
            };
        }
        // Sans réponse de la batterie, chaque lecture suivante coûterait ses trois tentatives pour rien.
        if !read!(bat, "Batterie", c.bat_status()) {
            if full {
                read!(device, "Appareil", c.device_info());
                read!(wifi, "Wi-Fi", c.wifi_status());
                read!(ble, "Bluetooth", c.ble_status());
            }
            read!(es, "Système", c.es_status());
            read!(mode, "Mode", c.es_mode());
            read!(em, "Compteur", c.em_status());
            read!(pv, "Solaire", c.pv_status());
        }
        s.ts = now;
        s.link = Link { rssi: s.wifi.as_ref().and_then(|w| w.rssi), ..c.link() };
        let _ = app.emit("status", &*s);
        self.online = s.updated.values().any(|&t| t == now);
        if self.online {
            self.last_ok = Instant::now();
            self.fails = 0;
            self.record_history();
        } else {
            self.fails = self.fails.saturating_add(1);
        }
        self.update_tray();
        if full {
            self.control = None;
            self.emit_control();
        }
    }

    /// Un historique de valeurs fraîches seulement : une valeur gardée après une lecture ratée serait réécrite à tort.
    fn record_history(&self) {
        let s = &self.snap;
        let (es, em) = (s.fresh(&s.es, "es"), s.fresh(&s.em, "em"));
        let (sample, counters) = history_rows(es, em, self.soc());
        if let Err(e) = self.app.state::<History>().record(s.ts as i64, sample, counters) {
            self.log(format!("Historique : {e}"));
        }
    }

    /// Hors ligne depuis `OFFLINE_AFTER` : un bail DHCP a pu changer l'IP. Cherche la batterie par son adresse MAC.
    fn relocate(&mut self) {
        let Some(addr) = self.client.as_ref().map(Client::addr) else { return };
        let due = self.last_search.is_none_or(|t| t.elapsed() >= OFFLINE_AFTER);
        if self.online || self.last_ok.elapsed() < OFFLINE_AFTER || !due {
            return;
        }
        let s = &self.snap;
        let mac = s.device.as_ref().and_then(|d| d.wifi_mac.clone()).or_else(|| s.wifi.as_ref().and_then(|w| w.wifi_mac.clone()));
        let Some(mac) = mac else { return };
        self.last_search = Some(Instant::now());
        let scan = settings::load(&self.app).scan_range.filter(|r| !r.trim().is_empty());
        let mut found = Vec::new();
        for range in std::iter::once(None).chain(scan.as_deref().map(Some)) {
            match api::discover(range, addr.port()) {
                Ok(devices) => found.extend(devices),
                Err(e) => self.log(format!("Recherche de la batterie : {e}")),
            }
        }
        let Some(new) = api::find_moved(&found, &mac, addr.ip()).and_then(|d| Some(SocketAddr::new(d.ip.parse().ok()?, d.port))) else {
            return;
        };
        self.log(format!("Batterie retrouvée à {} (elle était à {})", new.ip(), addr.ip()));
        self.connect(new);
        if let Err(e) = settings::save_last_ip(&self.app, &new.ip().to_string()) {
            self.log(format!("Nouvelle IP non enregistrée : {e}"));
        }
        let _ = self.app.emit("connected", new.ip().to_string());
    }

    fn update_tray(&mut self) {
        let power = self.snap.es.as_ref().and_then(|e| e.bat_power);
        let soc = self.soc();
        let mode = self.snap.mode.as_ref().and_then(|m| m.mode.as_deref());
        let key = (
            tray::level(soc),
            tray::flow(self.online, power),
            tray::tooltip(self.online, soc, power, mode),
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
                    Side::Low => ("SOC bas", &self.rules.low),
                    Side::High => ("SOC haut", &self.rules.high),
                };
                Control { source: Source::Rule, detail: format!("{name} : {}", rule.mode.label()) }
            }
            (None, Some(_)) => Control { source: Source::Manual, detail: self.manual_label.clone() },
            (None, None) => Control {
                source: Source::Plan,
                detail: self.plan.as_ref().map_or_else(|| "aucun plan actif".to_owned(), |p| p.name.clone()),
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
        let (next, action) = control::decide(&self.rules, self.engine, self.soc(), now);
        let done = match action {
            Action::None => true,
            Action::Rule(side) => self.apply_rule(side),
            Action::HandBack => self.hand_back(),
        };
        // Un échec garde l'ancien état : la même décision est retentée au prochain sondage.
        if done {
            self.engine = next;
        }
        self.emit_control();
    }

    fn alerts(&mut self) {
        let soc = self.soc();
        let (state, send) = control::low_soc_alert(&self.notify, soc, self.low_notified);
        self.low_notified = state;
        if send {
            self.notify_user("Batterie faible", format!("SOC {:.0} %", soc.unwrap_or_default()));
        }
        let (state, send) = control::offline_alert(self.notify.offline, self.last_ok.elapsed(), self.offline_notified);
        self.offline_notified = state;
        if send {
            self.notify_user("Batterie injoignable", "Aucune réponse depuis plus de 2 minutes.".into());
        }
    }

    /// Notification d'échec d'une écriture du moteur, une seule par série d'échecs.
    fn settle(&mut self, r: &Result<Vec<bool>, String>) -> bool {
        let ok = all_accepted(r);
        if !ok && !self.engine_failed && self.notify.write_failed {
            self.notify_user("Écriture échouée", "La batterie n'a pas pris la commande du pilotage automatique.".into());
        }
        self.engine_failed = !ok;
        ok
    }

    fn mode_changed(&self, detail: String) {
        if self.notify.mode_changed {
            self.notify_user("Mode changé", detail);
        }
    }

    fn apply_rule(&mut self, side: Side) -> bool {
        let (name, mode) = match side {
            Side::Low => ("SOC bas", self.rules.low.mode),
            Side::High => ("SOC haut", self.rules.high.mode),
        };
        let r = self.exec(Write::Mode(mode.req()));
        let ok = self.settle(&r);
        if ok {
            self.mode_changed(format!("Règle {name} : mode {}", mode.label()));
        }
        ok
    }

    /// Renvoie les 10 créneaux du plan actif : la batterie repasse en Manual et le suit seule.
    fn hand_back(&mut self) -> bool {
        let Some(plan) = self.plan.clone() else { return true };
        let slots = match compile_plan(&plan.blocks) {
            Ok(slots) => slots,
            Err(e) => {
                self.log(format!("Plan « {} » : {e}", plan.name));
                return true;
            }
        };
        let r = self.exec(Write::Slots(slots));
        let ok = self.settle(&r);
        if ok {
            self.mode_changed(format!("Le plan « {} » reprend la main", plan.name));
        }
        ok
    }

    /// Écriture demandée par l'utilisateur : retient le plan ou l'action manuelle si la batterie accepte.
    fn write(&mut self, w: Write) -> Result<Vec<bool>, String> {
        if self.test.is_some() && matches!(w, Write::Mode(_) | Write::Slots(_) | Write::Plan(..)) {
            return Err("test hors créneaux en cours : attends sa fin".into());
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
                    self.manual_label = label;
                }
                After::Plan(plan) => {
                    if let Err(e) = settings::save_sent_plan(&self.app, &plan) {
                        self.log(format!("Plan envoyé non enregistré : {e}"));
                    }
                    self.plan = Some(plan);
                    self.engine.manual_until = None;
                    self.engine.latch = None;
                }
                After::Nothing => {}
            }
            self.emit_control();
        } else if self.notify.write_failed {
            let detail = r.as_ref().err().map_or("refusé par la batterie", String::as_str);
            self.notify_user("Écriture échouée", format!("{label} : {detail}"));
        }
        r
    }

    /// Le sondage est suspendu pendant l'écriture (même thread), puis l'état est relu.
    fn exec(&mut self, w: Write) -> Result<Vec<bool>, String> {
        let Some(c) = self.client.as_mut() else { return Err("aucune batterie connectée".into()) };
        let _ = self.app.emit("busy", true);
        let label = w.label();
        let app = &self.app;
        let r = run(c, w, &|m| {
            let _ = app.emit("log", m);
        });
        let _ = app.emit("busy", false);
        self.log(match &r {
            Ok(v) if v.iter().all(|&ok| ok) => format!("{label} : accepté"),
            Ok(_) => format!("{label} : refusé par la batterie"),
            Err(e) => format!("{label} : {e}"),
        });
        self.poll(false);
        r
    }

    fn emit_test(&self, samples: &[Point], verdict: Option<control::Verdict>) {
        let _ = self.app.emit("slot_test", SlotTest { samples: samples.to_vec(), verdict });
    }

    /// Écrit le créneau de test, puis laisse les sondages suivants relever la puissance.
    fn start_test(&mut self) -> Result<(), String> {
        if self.client.is_none() {
            return Err("aucune batterie connectée".into());
        }
        if self.test.is_some() {
            return Err("un test est déjà en cours".into());
        }
        let now = clock::now_ms();
        let slot = control::test_slot(clock::week_minute(now, clock::local_offset_ms()))?;
        let r = self.exec(Write::Slots(vec![slot]));
        if !all_accepted(&r) {
            return Err(r.err().unwrap_or_else(|| "créneau de test refusé par la batterie".into()));
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
                if let Err(e) = settings::save_outside_slots(&self.app, json!({ "verdict": v, "ts": now })) {
                    self.log(format!("Résultat du test non enregistré : {e}"));
                }
            }
            // shortcut: sans mesure après la fin (batterie coupée), le test se termine sans verdict ni résultat retenu.
            None => self.log("Test hors créneaux : aucune mesure après la fin, résultat non retenu".into()),
        }
        self.emit_test(&t.samples, verdict);
        self.reset_engine();
        self.emit_control();
    }
}

/// Garde la valeur lue et sa date ; rend `true` si la liaison est perdue (pas de réponse), pas si la batterie refuse.
fn step<T>(
    app: &AppHandle,
    name: &str,
    key: &'static str,
    r: Result<T, ApiError>,
    slot: &mut Option<T>,
    updated: &mut HashMap<&'static str, u64>,
    now: u64,
) -> bool {
    thread::sleep(PAUSE);
    match r {
        Ok(v) => {
            *slot = Some(v);
            updated.insert(key, now);
            false
        }
        Err(e) => {
            let _ = app.emit("log", format!("{name} : {e}"));
            matches!(e, ApiError::Timeout | ApiError::Io(_))
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
fn slots_outcome(results: Vec<Result<bool, ApiError>>) -> Result<Vec<bool>, String> {
    let missing: Vec<usize> = (0..results.len()).filter(|&i| results[i].is_err()).collect();
    if !missing.is_empty() {
        return Err(format!("créneaux non écrits : {}", slot_numbers(&missing)));
    }
    Ok(results.into_iter().map(|r| r.unwrap_or(false)).collect())
}

fn run(c: &mut Client, w: Write, progress: &dyn Fn(String)) -> Result<Vec<bool>, String> {
    let r = match w {
        Write::Mode(m) => {
            let cfg = m.config();
            let r = c.set_mode(&cfg).or_else(|e| {
                // Réponse perdue, ordre peut-être passé : la batterie dit dans quel mode elle est.
                // shortcut: le mode passif n'est pas confirmé (sa puissance n'est pas comparée, un passif déjà actif validerait à tort) ; à affiner si ça gêne.
                if !matches!(e, ApiError::Timeout) || matches!(cfg, ModeConfig::Passive { .. }) {
                    return Err(e);
                }
                match c.es_mode().ok().and_then(|m| m.mode) {
                    Some(mode) if mode == cfg.name() => Ok(true),
                    _ => Err(e),
                }
            });
            r.map(|ok| vec![ok]).map_err(|e| e.to_string())
        }
        Write::Slots(slots) | Write::Plan(_, slots) => {
            let slots = manual_slots(&slots).map_err(|e| e.to_string())?;
            let mut send = |i: usize| {
                let r = c.set_mode(&ModeConfig::Manual(slots[i].clone()));
                thread::sleep(PAUSE);
                r
            };
            let first = (0..slots.len()).map(&mut send).collect();
            let announce = |failed: &[usize]| progress(format!("Créneaux {} sans réponse : nouvelle tentative", slot_numbers(failed)));
            slots_outcome(second_pass(first, announce, send))
        }
        Write::Dod(v) => c.set_dod(v).map(|ok| vec![ok]).map_err(|e| e.to_string()),
        Write::Led(on) => c.set_led(on).map(|ok| vec![ok]).map_err(|e| e.to_string()),
        Write::Ble(on) => c.set_ble(on).map(|ok| vec![ok]).map_err(|e| e.to_string()),
    };
    thread::sleep(PAUSE);
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
    fn passive_seconds_only_for_passive_mode() {
        assert_eq!(passive_seconds(&ModeReq::Auto), None);
        assert_eq!(passive_seconds(&ModeReq::Passive { direction: Direction::Charge, power: 100, seconds: 90 }), Some(90));
    }

    #[test]
    fn poll_interval_backs_off_to_a_minute() {
        let secs: Vec<u64> = (0..6).map(|n| poll_interval(n).as_secs()).collect();
        assert_eq!(secs, [10, 20, 40, 60, 60, 60]);
        assert_eq!(poll_interval(u32::MAX), POLL_MAX);
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
        assert_eq!(slots_outcome(r.into_iter().collect()).unwrap_err(), "créneaux non écrits : 4");
    }

    #[test]
    fn second_pass_skips_replay_when_every_slot_failed() {
        let r = second_pass(vec![lost(), lost()], |_| panic!("liaison coupée"), |_| panic!("liaison coupée"));
        assert_eq!(slots_outcome(r).unwrap_err(), "créneaux non écrits : 1, 2");
    }

    #[test]
    fn second_pass_stays_quiet_when_everything_was_answered() {
        let r = second_pass(vec![Ok(true), Ok(false)], |_| panic!("rien à rejouer"), |_| panic!("rien à rejouer"));
        assert_eq!(slots_outcome(r).unwrap(), [true, false]);
    }

    #[test]
    fn unconfirmed_slots_are_all_named() {
        let err = slots_outcome(vec![lost(), Ok(true), lost()]).unwrap_err();
        assert_eq!(err, "créneaux non écrits : 1, 3");
    }

    #[test]
    fn fresh_ignores_sections_kept_from_an_earlier_poll() {
        let mut s = Snapshot { ts: 20, ..Default::default() };
        s.es = Some(EsStatus::default());
        s.bat = Some(BatStatus::default());
        s.updated.insert("es", 20);
        s.updated.insert("bat", 10);
        assert!(s.fresh(&s.es, "es").is_some());
        assert!(s.fresh(&s.bat, "bat").is_none());
        assert!(s.fresh(&s.em, "em").is_none());
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
        assert_eq!(run(&mut client, Write::Mode(ModeReq::Ups), &|_| ()), Ok(vec![true]));
        assert!(run(&mut client, Write::Mode(ModeReq::Auto), &|_| ()).is_err());
        t.join().unwrap();
    }
}
