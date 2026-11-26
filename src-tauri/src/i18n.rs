//! Messages visibles par l'utilisateur, dans la langue de l'interface (réglage `lang`, anglais par défaut).
//! Le journal sur disque reste toujours en français : `log` émet les deux versions.

use crate::api::SLOTS;
use crate::settings::ModeKind;
use serde::Deserialize;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En,
    Fr,
    De,
    Es,
}

/// Langue courante : lue par les messages rendus hors du thread du worker (erreurs renvoyées au front).
static CURRENT: AtomicU8 = AtomicU8::new(Lang::En as u8);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Fr,
        2 => Lang::De,
        3 => Lang::Es,
        _ => Lang::En,
    }
}

/// Sections lues à chaque sondage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Part {
    Battery,
    System,
    Device,
    Mode,
    Solar,
    Meter,
    Wifi,
    Bluetooth,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// Texte technique identique dans toutes les langues (erreur système, nom de plan).
    Raw(String),
    /// « étiquette : détail ».
    Detail(Box<Msg>, Box<Msg>),
    Section(Part),
    Mode(ModeKind),
    // Erreurs de l'API et de la saisie.
    Io(String),
    BadReply(String),
    NoReply,
    PortClosed(u16),
    DeviceRefused { code: i64, message: String },
    TooManySlots(usize),
    SlotsNeeded(usize),
    BadRange(String),
    RangeTooWide(usize),
    NoInterface,
    BadPort(String),
    ScanTooBig { hosts: usize, ports: usize, n: usize, max: usize },
    BadTime(String),
    BadBlock { end: String, start: String },
    BadDay(u8),
    Overlap(usize),
    // Étapes du balayage.
    ScanStage1 { ports: String, wait: String },
    ScanManual { hosts: usize, ports: String, wait: String },
    ScanStage { n: u8, hosts: usize, ports: String, wait: String },
    // Erreurs du worker et des commandes.
    ThreadStopped,
    NoBattery,
    TestRunning,
    TestAlready,
    TestRefused,
    TestNearMidnight,
    SlotsMissing(String),
    BadIp(String),
    CsvWrite(String),
    Accepted,
    Refused,
    SlotsRetry(String),
    PowerLowered { from: u16, cap: u16 },
    // Libellés d'écriture et d'étiquettes de journal.
    WriteMode,
    WriteSlots,
    WritePlan,
    WriteDod,
    LabelConnection,
    LabelNotification,
    LabelHistory,
    LabelBatterySearch,
    LabelPortSearch,
    LabelAddrNotSaved,
    LabelPortNotSaved,
    LabelPlanNotSaved,
    LabelTestNotSaved,
    LabelPlan(String),
    // Journal.
    Connecting(String),
    NoReplyAside,
    Found { new: String, old: String },
    PortSearch(String),
    PortFound(u16),
    PortNotFound,
    PollTuned(u64),
    Reapplied(String),
    TestNoResult,
    // Notifications.
    TitleLowBattery,
    TitleUnreachable,
    TitleReconnected,
    TitleWriteFailed,
    TitleModeChanged,
    AutoWriteRefused,
    OfflinePortClosed(u16),
    OfflineNoWifi,
    OfflineSilent,
    Back { mins: u64, check: bool },
    // Pilotage.
    RuleLow,
    RuleHigh,
    RuleApplied { high: bool, mode: ModeKind },
    Block(ModeKind),
    SocLimit,
    SocLimitHold,
    AppDriven(Box<Msg>),
    NoActivePlan,
    ReadOnlyPlan(Box<Msg>),
    PlanTakesOver(String),
    Charge(u16),
    Discharge(u16),
    Passive(Box<Msg>),
    // Synchro.
    DownPortClosed(u16),
    DownSilentSecs(u64),
    DownSilentMins(u64),
    DownSilent,
    NoPlanSent,
    SrcOutsideTest,
    SrcRule,
    SrcManual,
    PowerUnknown,
    ReadOnlyNotFollowed,
    HoldCap,
    OutsideSlot,
    ModeNotRead,
    ModeMismatch { expected: String, read: String },
    OutsideUnknown,
    BatPowerUnread,
    OppositeDir { target: f64, measured: f64 },
    BatStopped,
    Gap(f64),
    // Secours cloud.
    LabelCloud,
    CloudUnreadable,
    CloudLoginRefused,
    CloudUnexpected,
    CloudTimeout,
    CloudHttp(u16),
    CloudUnreachable,
    CloudBadEmail,
    CloudBadPassword,
    CloudStore,
    CloudExpired,
    CloudWait,
    CloudEnableUnsupported,
    CloudMqtt,
    CloudReenableSent,
    CloudBadCtMac,
    CloudCtReselectSent,
    // Zone de notification.
    TrayOpen,
    TrayStop,
    TrayQuit,
    TrayOffline,
}

macro_rules! t {
    ($lang:expr; $en:expr, $fr:expr, $de:expr, $es:expr) => {
        match $lang {
            Lang::En => $en,
            Lang::Fr => $fr,
            Lang::De => $de,
            Lang::Es => $es,
        }
    };
}

const DAYS: [[&str; 7]; 4] = [
    ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"],
    ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"],
    ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"],
];

impl Msg {
    pub fn raw(s: impl Into<String>) -> Self {
        Self::Raw(s.into())
    }

    /// `self : detail`.
    pub fn detail(self, detail: Msg) -> Self {
        Self::Detail(Box::new(self), Box::new(detail))
    }

    pub fn text(&self, l: Lang) -> String {
        use Msg::*;
        match self {
            Raw(s) => s.clone(),
            Detail(a, b) => format!("{}{}{}", a.text(l), t!(l; ": ", " : ", ": ", ": "), b.text(l)),
            Section(p) => {
                let n = match p {
                    Part::Battery => t!(l; "Battery", "Batterie", "Batterie", "Batería"),
                    Part::System => t!(l; "System", "Système", "System", "Sistema"),
                    Part::Device => t!(l; "Device", "Appareil", "Gerät", "Dispositivo"),
                    Part::Mode => t!(l; "Mode", "Mode", "Modus", "Modo"),
                    Part::Solar => t!(l; "Solar", "Solaire", "Solar", "Solar"),
                    Part::Meter => t!(l; "Meter", "Compteur", "Zähler", "Contador"),
                    Part::Wifi => "Wi-Fi",
                    Part::Bluetooth => "Bluetooth",
                };
                n.into()
            }
            Mode(m) => match m {
                ModeKind::Auto => "Auto".into(),
                ModeKind::Ai => t!(l; "AI", "IA", "AI", "AI").into(),
                ModeKind::Ups => "UPS".into(),
            },
            Io(e) => t!(l; format!("network error: {e}"), format!("erreur réseau : {e}"), format!("Netzwerkfehler: {e}"), format!("error de red: {e}")),
            BadReply(e) => t!(l; format!("unreadable response: {e}"), format!("réponse illisible : {e}"), format!("Antwort nicht lesbar: {e}"), format!("respuesta ilegible: {e}")),
            NoReply => t!(l;
                "no response (is Open API enabled? correct IP/port?)".into(),
                "pas de réponse (Open API activée ? bonne IP/port ?)".into(),
                "keine Antwort (Open API aktiviert? Richtige IP/Port?)".into(),
                "sin respuesta (¿Open API activada? ¿IP/puerto correctos?)".into()),
            PortClosed(p) => t!(l;
                format!("port {p} closed on the battery: Open API disabled or port changed in the Marstek app"),
                format!("port {p} fermé sur la batterie : Open API désactivée ou port changé dans l'app Marstek"),
                format!("Port {p} an der Batterie geschlossen: Open API deaktiviert oder Port in der Marstek-App geändert"),
                format!("puerto {p} cerrado en la batería: Open API desactivada o puerto cambiado en la app Marstek")),
            DeviceRefused { code, message } => t!(l;
                format!("the battery refused ({code}): {message}"),
                format!("la batterie a refusé ({code}) : {message}"),
                format!("die Batterie hat abgelehnt ({code}): {message}"),
                format!("la batería lo rechazó ({code}): {message}")),
            TooManySlots(n) => t!(l;
                format!("{n} slots requested, the battery accepts only {SLOTS}"),
                format!("{n} créneaux demandés, la batterie n'en accepte que {SLOTS}"),
                format!("{n} Zeitfenster angefordert, die Batterie akzeptiert nur {SLOTS}"),
                format!("{n} franjas solicitadas, la batería solo acepta {SLOTS}")),
            SlotsNeeded(n) => t!(l;
                format!("{n} slots needed, the battery accepts only {SLOTS}: simplify the schedule"),
                format!("{n} créneaux nécessaires, la batterie n'en accepte que {SLOTS} : simplifie le planning"),
                format!("{n} Zeitfenster nötig, die Batterie akzeptiert nur {SLOTS}: vereinfache den Zeitplan"),
                format!("{n} franjas necesarias, la batería solo acepta {SLOTS}: simplifica la programación")),
            BadRange(s) => t!(l;
                format!("invalid IP range: \"{s}\" (e.g. 192.168.1.1-254 or 192.168.1.0/24, prefixes /16 to /30)"),
                format!("plage IP invalide : « {s} » (ex. 192.168.1.1-254 ou 192.168.1.0/24, préfixes /16 à /30)"),
                format!("ungültiger IP-Bereich: „{s}“ (z. B. 192.168.1.1-254 oder 192.168.1.0/24, Präfixe /16 bis /30)"),
                format!("rango IP no válido: «{s}» (p. ej. 192.168.1.1-254 o 192.168.1.0/24, prefijos /16 a /30)")),
            RangeTooWide(max) => t!(l;
                format!("range too large: more than {max} addresses"),
                format!("plage trop vaste : plus de {max} adresses"),
                format!("Bereich zu groß: mehr als {max} Adressen"),
                format!("rango demasiado amplio: más de {max} direcciones")),
            NoInterface => t!(l;
                "no active IPv4 interface".into(),
                "aucune interface IPv4 active".into(),
                "keine aktive IPv4-Schnittstelle".into(),
                "ninguna interfaz IPv4 activa".into()),
            BadPort(p) => t!(l;
                format!("invalid port: \"{p}\" (e.g. 30000, 49152-65535; integers from 1 to 65535, start before end)"),
                format!("port invalide : « {p} » (ex. 30000, 49152-65535 ; entiers de 1 à 65535, début avant fin)"),
                format!("ungültiger Port: „{p}“ (z. B. 30000, 49152-65535; ganze Zahlen von 1 bis 65535, Anfang vor Ende)"),
                format!("puerto no válido: «{p}» (p. ej. 30000, 49152-65535; enteros de 1 a 65535, inicio antes del fin)")),
            ScanTooBig { hosts, ports, n, max } => t!(l;
                format!("scan too large: {hosts} addresses × {ports} ports = {n} probes (at most {max}). Reduce the address range or the port range."),
                format!("balayage trop vaste : {hosts} adresses × {ports} ports = {n} sondes (au plus {max}). Réduisez la plage d'adresses ou la plage de ports."),
                format!("Scan zu groß: {hosts} Adressen × {ports} Ports = {n} Sonden (höchstens {max}). Verkleinere den Adressbereich oder den Portbereich."),
                format!("escaneo demasiado grande: {hosts} direcciones × {ports} puertos = {n} sondas (como máximo {max}). Reduce el rango de direcciones o el de puertos.")),
            BadTime(s) => t!(l;
                format!("invalid time: \"{s}\" (expected hh:mm)"),
                format!("heure invalide : « {s} » (attendu hh:mm)"),
                format!("ungültige Uhrzeit: „{s}“ (erwartet hh:mm)"),
                format!("hora no válida: «{s}» (se espera hh:mm)")),
            BadBlock { end, start } => t!(l;
                format!("invalid block: {end} must end after {start}"),
                format!("bloc invalide : {end} doit finir après {start}"),
                format!("ungültiger Block: {end} muss nach {start} enden"),
                format!("bloque no válido: {end} debe terminar después de {start}")),
            BadDay(d) => t!(l; format!("invalid day: {d}"), format!("jour invalide : {d}"), format!("ungültiger Tag: {d}"), format!("día no válido: {d}")),
            Overlap(day) => {
                let name = DAYS[l as usize][*day % 7];
                t!(l;
                    format!("blocks overlap on {name}"),
                    format!("des blocs se chevauchent le {name}"),
                    format!("Blöcke überlappen sich am {name}"),
                    format!("hay bloques que se solapan el {name}"))
            }
            ScanStage1 { ports, wait } => t!(l;
                format!("Step 1/3: this PC's subnets, port(s) {ports}, about {wait}"),
                format!("Étape 1/3 : sous-réseaux du PC, port(s) {ports}, environ {wait}"),
                format!("Schritt 1/3: Subnetze dieses PCs, Port(s) {ports}, etwa {wait}"),
                format!("Paso 1/3: subredes del PC, puerto(s) {ports}, aprox. {wait}")),
            ScanManual { hosts, ports, wait } => t!(l;
                format!("Scanning {hosts} addresses, port(s) {ports}, about {wait}"),
                format!("Balayage de {hosts} adresses, port(s) {ports}, environ {wait}"),
                format!("Scan von {hosts} Adressen, Port(s) {ports}, etwa {wait}"),
                format!("Escaneo de {hosts} direcciones, puerto(s) {ports}, aprox. {wait}")),
            ScanStage { n, hosts, ports, wait } => t!(l;
                format!("Step {n}/3: {hosts} live hosts, port(s) {ports}, about {wait}"),
                format!("Étape {n}/3 : {hosts} hôtes vivants, port(s) {ports}, environ {wait}"),
                format!("Schritt {n}/3: {hosts} aktive Hosts, Port(s) {ports}, etwa {wait}"),
                format!("Paso {n}/3: {hosts} hosts activos, puerto(s) {ports}, aprox. {wait}")),
            ThreadStopped => t!(l;
                "the network thread has stopped".into(),
                "le thread réseau s'est arrêté".into(),
                "der Netzwerk-Thread wurde beendet".into(),
                "el hilo de red se ha detenido".into()),
            NoBattery => t!(l; "no battery connected".into(), "aucune batterie connectée".into(), "keine Batterie verbunden".into(), "ninguna batería conectada".into()),
            TestRunning => t!(l;
                "out-of-slot test in progress: wait for it to finish".into(),
                "test hors créneaux en cours : attends sa fin".into(),
                "Test außerhalb der Zeitfenster läuft: warte, bis er fertig ist".into(),
                "prueba fuera de franjas en curso: espera a que termine".into()),
            TestAlready => t!(l; "a test is already running".into(), "un test est déjà en cours".into(), "ein Test läuft bereits".into(), "ya hay una prueba en curso".into()),
            TestRefused => t!(l;
                "test slot refused by the battery".into(),
                "créneau de test refusé par la batterie".into(),
                "Testfenster von der Batterie abgelehnt".into(),
                "franja de prueba rechazada por la batería".into()),
            TestNearMidnight => t!(l;
                "too close to midnight: run the test again in a few minutes".into(),
                "trop près de minuit : relance le test dans quelques minutes".into(),
                "zu nah an Mitternacht: starte den Test in einigen Minuten erneut".into(),
                "demasiado cerca de la medianoche: repite la prueba en unos minutos".into()),
            SlotsMissing(list) => t!(l;
                format!("slots not written: {list}"),
                format!("créneaux non écrits : {list}"),
                format!("Zeitfenster nicht geschrieben: {list}"),
                format!("franjas no escritas: {list}")),
            BadIp(ip) => t!(l; format!("invalid IP address: {ip}"), format!("adresse IP invalide : {ip}"), format!("ungültige IP-Adresse: {ip}"), format!("dirección IP no válida: {ip}")),
            CsvWrite(e) => t!(l; format!("cannot write: {e}"), format!("écriture impossible : {e}"), format!("Schreiben nicht möglich: {e}"), format!("no se puede escribir: {e}")),
            Accepted => t!(l; "accepted".into(), "accepté".into(), "akzeptiert".into(), "aceptado".into()),
            Refused => t!(l; "refused by the battery".into(), "refusé par la batterie".into(), "von der Batterie abgelehnt".into(), "rechazado por la batería".into()),
            SlotsRetry(list) => t!(l;
                format!("Slots {list} did not respond: trying again"),
                format!("Créneaux {list} sans réponse : nouvelle tentative"),
                format!("Zeitfenster {list} ohne Antwort: neuer Versuch"),
                format!("Franjas {list} sin respuesta: nuevo intento")),
            PowerLowered { from, cap } => t!(l;
                format!("power lowered from {from} to {cap} W"),
                format!("puissance ramenée de {from} à {cap} W"),
                format!("Leistung von {from} auf {cap} W reduziert"),
                format!("potencia reducida de {from} a {cap} W")),
            WriteMode => t!(l; "Mode change".into(), "Changement de mode".into(), "Moduswechsel".into(), "Cambio de modo".into()),
            WriteSlots => t!(l; "Writing slots".into(), "Écriture des créneaux".into(), "Zeitfenster schreiben".into(), "Escritura de franjas".into()),
            WritePlan => t!(l; "Sending the schedule".into(), "Envoi du planning".into(), "Zeitplan senden".into(), "Envío de la programación".into()),
            WriteDod => t!(l; "Depth of discharge".into(), "Profondeur de décharge".into(), "Entladetiefe".into(), "Profundidad de descarga".into()),
            LabelConnection => t!(l; "Connection".into(), "Connexion".into(), "Verbindung".into(), "Conexión".into()),
            LabelNotification => t!(l; "Notification".into(), "Notification".into(), "Benachrichtigung".into(), "Notificación".into()),
            LabelHistory => t!(l; "History".into(), "Historique".into(), "Verlauf".into(), "Historial".into()),
            LabelBatterySearch => t!(l; "Battery search".into(), "Recherche de la batterie".into(), "Batteriesuche".into(), "Búsqueda de la batería".into()),
            LabelPortSearch => t!(l; "Port search".into(), "Recherche du port".into(), "Portsuche".into(), "Búsqueda del puerto".into()),
            LabelAddrNotSaved => t!(l; "New address not saved".into(), "Nouvelle adresse non enregistrée".into(), "Neue Adresse nicht gespeichert".into(), "Nueva dirección no guardada".into()),
            LabelPortNotSaved => t!(l; "Port not saved".into(), "Port non enregistré".into(), "Port nicht gespeichert".into(), "Puerto no guardado".into()),
            LabelPlanNotSaved => t!(l; "Sent plan not saved".into(), "Plan envoyé non enregistré".into(), "Gesendeter Plan nicht gespeichert".into(), "Plan enviado no guardado".into()),
            LabelTestNotSaved => t!(l; "Test result not saved".into(), "Résultat du test non enregistré".into(), "Testergebnis nicht gespeichert".into(), "Resultado de la prueba no guardado".into()),
            LabelPlan(name) => t!(l; format!("Plan \"{name}\""), format!("Plan « {name} »"), format!("Plan „{name}“"), format!("Plan «{name}»")),
            Connecting(addr) => t!(l; format!("Connecting to {addr}"), format!("Connexion à {addr}"), format!("Verbinde mit {addr}"), format!("Conectando a {addr}")),
            NoReplyAside => t!(l;
                "no response from the battery, retrying every 5 min".into(),
                "pas de réponse de la batterie, relu toutes les 5 min".into(),
                "keine Antwort der Batterie, erneuter Versuch alle 5 Min.".into(),
                "sin respuesta de la batería, se reintenta cada 5 min".into()),
            Found { new, old } => t!(l;
                format!("Battery found at {new} (it was at {old})"),
                format!("Batterie retrouvée à {new} (elle était à {old})"),
                format!("Batterie unter {new} gefunden (zuvor {old})"),
                format!("Batería encontrada en {new} (estaba en {old})")),
            PortSearch(ip) => t!(l;
                format!("Looking for the API port on {ip}"),
                format!("Recherche du port de l'API sur {ip}"),
                format!("Suche nach dem API-Port auf {ip}"),
                format!("Buscando el puerto de la API en {ip}")),
            PortFound(p) => t!(l; format!("API found on port {p}"), format!("API retrouvée sur le port {p}"), format!("API auf Port {p} gefunden"), format!("API encontrada en el puerto {p}")),
            PortNotFound => t!(l;
                "Port not found, searching again in 10 min".into(),
                "Port introuvable, nouvelle recherche dans 10 min".into(),
                "Port nicht gefunden, neue Suche in 10 Min.".into(),
                "Puerto no encontrado, nueva búsqueda en 10 min".into()),
            PollTuned(s) => t!(l;
                format!("Poll cadence tuned to {s} s"),
                format!("Cadence de sondage calibrée à {s} s"),
                format!("Abfragetakt auf {s} s kalibriert"),
                format!("Cadencia de sondeo calibrada a {s} s")),
            Reapplied(read) => t!(l;
                format!("Plan reapplied: mode read {read} instead of Manual"),
                format!("Plan réappliqué : mode lu {read} au lieu de Manual"),
                format!("Plan erneut angewendet: gelesener Modus {read} statt Manual"),
                format!("Plan reaplicado: modo leído {read} en lugar de Manual")),
            TestNoResult => t!(l;
                "Out-of-slot test: no measurement after the end, result discarded".into(),
                "Test hors créneaux : aucune mesure après la fin, résultat non retenu".into(),
                "Test außerhalb der Zeitfenster: keine Messung nach dem Ende, Ergebnis verworfen".into(),
                "Prueba fuera de franjas: ninguna medida tras el final, resultado descartado".into()),
            TitleLowBattery => t!(l; "Low battery".into(), "Batterie faible".into(), "Batterie schwach".into(), "Batería baja".into()),
            TitleUnreachable => t!(l; "Battery unreachable".into(), "Batterie injoignable".into(), "Batterie nicht erreichbar".into(), "Batería inaccesible".into()),
            TitleReconnected => t!(l; "Battery reconnected".into(), "Batterie reconnectée".into(), "Batterie wieder verbunden".into(), "Batería reconectada".into()),
            TitleWriteFailed => t!(l; "Write failed".into(), "Écriture échouée".into(), "Schreiben fehlgeschlagen".into(), "Escritura fallida".into()),
            TitleModeChanged => t!(l; "Mode changed".into(), "Mode changé".into(), "Modus geändert".into(), "Modo cambiado".into()),
            AutoWriteRefused => t!(l;
                "The battery did not accept the automatic control command.".into(),
                "La batterie n'a pas pris la commande du pilotage automatique.".into(),
                "Die Batterie hat den Befehl der automatischen Steuerung nicht angenommen.".into(),
                "La batería no aceptó la orden del control automático.".into()),
            OfflinePortClosed(p) => t!(l;
                format!("Open API disabled on the battery (port {p} closed). Re-enable it in the Marstek phone app: the connection will resume on its own."),
                format!("Open API désactivée sur la batterie (port {p} fermé). Réactive-la dans l'app Marstek du téléphone : la connexion reprendra seule."),
                format!("Open API an der Batterie deaktiviert (Port {p} geschlossen). Aktiviere sie in der Marstek-App auf dem Handy wieder: die Verbindung wird von selbst wiederhergestellt."),
                format!("Open API desactivada en la batería (puerto {p} cerrado). Reactívala en la app Marstek del teléfono: la conexión se retomará sola.")),
            OfflineNoWifi => t!(l;
                "Battery missing from the Wi-Fi network (no ping response). It may be restarting; check its Wi-Fi in the Marstek app.".into(),
                "Batterie absente du réseau Wi-Fi (pas de réponse au ping). Elle redémarre peut-être ; vérifie son Wi-Fi dans l'app Marstek.".into(),
                "Batterie nicht im WLAN (keine Antwort auf Ping). Vielleicht startet sie neu; prüfe ihr WLAN in der Marstek-App.".into(),
                "Batería ausente de la red Wi-Fi (sin respuesta al ping). Quizá se está reiniciando; revisa su Wi-Fi en la app Marstek.".into()),
            OfflineSilent => t!(l;
                "Battery is on the network but Open API no longer responds. If this persists, re-enable Open API in the Marstek app.".into(),
                "Batterie sur le réseau mais l'Open API ne répond plus. Si ça dure, réactive l'Open API dans l'app Marstek.".into(),
                "Batterie ist im Netzwerk, aber die Open API antwortet nicht mehr. Falls das anhält, aktiviere die Open API in der Marstek-App neu.".into(),
                "La batería está en la red pero la Open API ya no responde. Si dura, reactiva la Open API en la app Marstek.".into()),
            Back { mins, check } => {
                let tail = match (*check, l) {
                    (false, _) => "",
                    (_, Lang::En) => " Check the CT and the schedule.",
                    (_, Lang::Fr) => " Vérifie la CT et le planning.",
                    (_, Lang::De) => " Prüfe CT und Zeitplan.",
                    (_, Lang::Es) => " Revisa el CT y la programación.",
                };
                t!(l;
                    format!("Battery reachable again (outage of {mins} min).{tail}"),
                    format!("Batterie de nouveau joignable (coupure de {mins} min).{tail}"),
                    format!("Batterie wieder erreichbar (Ausfall von {mins} Min.).{tail}"),
                    format!("Batería de nuevo accesible (corte de {mins} min).{tail}"))
            }
            RuleLow => t!(l; "Low SOC".into(), "SOC bas".into(), "SOC niedrig".into(), "SOC bajo".into()),
            RuleHigh => t!(l; "High SOC".into(), "SOC haut".into(), "SOC hoch".into(), "SOC alto".into()),
            RuleApplied { high, mode } => {
                let name = (if *high { RuleHigh } else { RuleLow }).text(l);
                let mode = Mode(*mode).text(l);
                t!(l; format!("Rule {name}: mode {mode}"), format!("Règle {name} : mode {mode}"), format!("Regel {name}: Modus {mode}"), format!("Regla {name}: modo {mode}"))
            }
            Block(m) => {
                let m = Mode(*m).text(l);
                t!(l; format!("block {m}"), format!("bloc {m}"), format!("Block {m}"), format!("bloque {m}"))
            }
            SocLimit => t!(l; "SOC limit reached".into(), "limite de SOC atteinte".into(), "SOC-Grenze erreicht".into(), "límite de SOC alcanzado".into()),
            SocLimitHold => t!(l; "SOC limit reached, holding".into(), "limite de SOC atteinte, maintien".into(), "SOC-Grenze erreicht, Halten".into(), "límite de SOC alcanzado, manteniendo".into()),
            AppDriven(what) => {
                let w = what.text(l);
                t!(l; format!("Controlled by the app: {w}"), format!("Pilotage par l'application : {w}"), format!("Steuerung durch die App: {w}"), format!("Control por la aplicación: {w}"))
            }
            NoActivePlan => t!(l; "no active plan".into(), "aucun plan actif".into(), "kein aktiver Plan".into(), "ningún plan activo".into()),
            ReadOnlyPlan(plan) => {
                let p = plan.text(l);
                t!(l;
                    format!("automatic control off, read-only: {p}"),
                    format!("pilotage automatique désactivé, lecture seule : {p}"),
                    format!("automatische Steuerung aus, nur Lesen: {p}"),
                    format!("control automático desactivado, solo lectura: {p}"))
            }
            PlanTakesOver(name) => t!(l;
                format!("Plan \"{name}\" takes over again"),
                format!("Le plan « {name} » reprend la main"),
                format!("Plan „{name}“ übernimmt wieder"),
                format!("El plan «{name}» retoma el control")),
            Charge(w) => t!(l; format!("charge {w} W"), format!("charge {w} W"), format!("Laden {w} W"), format!("carga {w} W")),
            Discharge(w) => t!(l; format!("discharge {w} W"), format!("décharge {w} W"), format!("Entladen {w} W"), format!("descarga {w} W")),
            Passive(d) => {
                let d = d.text(l);
                t!(l; format!("Passive {d}"), format!("Passif {d}"), format!("Passiv {d}"), format!("Pasivo {d}"))
            }
            DownPortClosed(p) => t!(l; format!("API closed on port {p}"), format!("API fermée sur le port {p}"), format!("API auf Port {p} geschlossen"), format!("API cerrada en el puerto {p}")),
            DownSilentSecs(s) => t!(l;
                format!("battery unresponsive for {s} s"),
                format!("batterie sans réponse depuis {s} s"),
                format!("Batterie antwortet seit {s} s nicht"),
                format!("batería sin respuesta desde hace {s} s")),
            DownSilentMins(m) => t!(l;
                format!("battery unresponsive for {m} min"),
                format!("batterie sans réponse depuis {m} min"),
                format!("Batterie antwortet seit {m} Min. nicht"),
                format!("batería sin respuesta desde hace {m} min")),
            DownSilent => t!(l; "battery unresponsive".into(), "batterie sans réponse".into(), "Batterie antwortet nicht".into(), "batería sin respuesta".into()),
            NoPlanSent => t!(l; "no plan sent".into(), "aucun plan envoyé".into(), "kein Plan gesendet".into(), "ningún plan enviado".into()),
            SrcOutsideTest => t!(l; "Out-of-slot test".into(), "Test hors créneaux".into(), "Test außerhalb der Zeitfenster".into(), "Prueba fuera de franjas".into()),
            SrcRule => t!(l; "SOC rule".into(), "Règle SOC".into(), "SOC-Regel".into(), "Regla SOC".into()),
            SrcManual => t!(l; "Manual action".into(), "Action manuelle".into(), "Manuelle Aktion".into(), "Acción manual".into()),
            PowerUnknown => t!(l; "power unknown".into(), "puissance inconnue".into(), "Leistung unbekannt".into(), "potencia desconocida".into()),
            ReadOnlyNotFollowed => t!(l;
                "read-only: plan not followed".into(),
                "lecture seule : plan non suivi".into(),
                "nur Lesen: Plan wird nicht befolgt".into(),
                "solo lectura: plan no seguido".into()),
            HoldCap => t!(l;
                "SOC cap hold, 0 W".into(),
                "maintien de plafond SOC, 0 W".into(),
                "Halten an der SOC-Grenze, 0 W".into(),
                "mantenimiento del límite de SOC, 0 W".into()),
            OutsideSlot => t!(l; "outside slot".into(), "hors créneau".into(), "außerhalb eines Zeitfensters".into(), "fuera de franja".into()),
            ModeNotRead => t!(l; "mode not read".into(), "mode non lu".into(), "Modus nicht gelesen".into(), "modo no leído".into()),
            ModeMismatch { expected, read } => t!(l;
                format!("expected mode {expected}, read {read}"),
                format!("mode attendu {expected}, lu {read}"),
                format!("erwarteter Modus {expected}, gelesen {read}"),
                format!("modo esperado {expected}, leído {read}")),
            OutsideUnknown => t!(l;
                "behavior outside slots unknown".into(),
                "comportement hors créneaux inconnu".into(),
                "Verhalten außerhalb der Zeitfenster unbekannt".into(),
                "comportamiento fuera de franjas desconocido".into()),
            BatPowerUnread => t!(l;
                "battery power not read".into(),
                "puissance batterie non lue".into(),
                "Batterieleistung nicht gelesen".into(),
                "potencia de la batería no leída".into()),
            OppositeDir { target, measured } => t!(l;
                format!("opposite direction: expected {target:+.0} W, measured {measured:+.0} W"),
                format!("sens opposé : attendu {target:+.0} W, mesuré {measured:+.0} W"),
                format!("entgegengesetzte Richtung: erwartet {target:+.0} W, gemessen {measured:+.0} W"),
                format!("sentido opuesto: esperado {target:+.0} W, medido {measured:+.0} W")),
            BatStopped => t!(l;
                "battery stopped (full, empty or DOD reached)".into(),
                "batterie à l'arrêt (pleine, vide ou DOD atteint)".into(),
                "Batterie steht still (voll, leer oder DOD erreicht)".into(),
                "batería detenida (llena, vacía o DOD alcanzado)".into()),
            Gap(w) => t!(l;
                format!("{w:.0} W away from the expected power"),
                format!("écart de {w:.0} W avec la puissance attendue"),
                format!("{w:.0} W Abweichung von der erwarteten Leistung"),
                format!("desviación de {w:.0} W respecto a la potencia esperada")),
            LabelCloud => "Cloud".into(),
            CloudUnreadable => t!(l;
                "unreadable response from the Marstek server".into(),
                "réponse illisible du serveur Marstek".into(),
                "Antwort des Marstek-Servers nicht lesbar".into(),
                "respuesta ilegible del servidor Marstek".into()),
            CloudLoginRefused => t!(l;
                "sign-in refused: wrong e-mail or password".into(),
                "connexion refusée : e-mail ou mot de passe incorrect".into(),
                "Anmeldung abgelehnt: E-Mail oder Passwort falsch".into(),
                "inicio de sesión rechazado: correo o contraseña incorrectos".into()),
            CloudUnexpected => t!(l;
                "unexpected response from the Marstek server".into(),
                "réponse inattendue du serveur Marstek".into(),
                "unerwartete Antwort des Marstek-Servers".into(),
                "respuesta inesperada del servidor Marstek".into()),
            CloudTimeout => t!(l;
                "the Marstek server is not responding (timed out)".into(),
                "le serveur Marstek ne répond pas (délai dépassé)".into(),
                "der Marstek-Server antwortet nicht (Zeitüberschreitung)".into(),
                "el servidor Marstek no responde (tiempo agotado)".into()),
            CloudHttp(c) => t!(l;
                format!("the Marstek server answered with HTTP error {c}"),
                format!("le serveur Marstek a répondu avec l'erreur HTTP {c}"),
                format!("der Marstek-Server antwortete mit HTTP-Fehler {c}"),
                format!("el servidor Marstek respondió con el error HTTP {c}")),
            CloudUnreachable => t!(l;
                "cannot reach the Marstek server".into(),
                "connexion au serveur Marstek impossible".into(),
                "Marstek-Server nicht erreichbar".into(),
                "no se puede conectar con el servidor Marstek".into()),
            CloudBadEmail => t!(l;
                "invalid e-mail address".into(),
                "adresse e-mail invalide".into(),
                "ungültige E-Mail-Adresse".into(),
                "dirección de correo no válida".into()),
            CloudBadPassword => t!(l;
                "invalid password".into(),
                "mot de passe invalide".into(),
                "ungültiges Passwort".into(),
                "contraseña no válida".into()),
            CloudStore => t!(l;
                "Windows Credential Manager unavailable".into(),
                "Gestionnaire d'identifiants indisponible".into(),
                "Windows-Anmeldeinformationsverwaltung nicht verfügbar".into(),
                "Administrador de credenciales de Windows no disponible".into()),
            CloudExpired => t!(l;
                "session expired: sign in again".into(),
                "session expirée : reconnectez-vous".into(),
                "Sitzung abgelaufen: bitte erneut anmelden".into(),
                "sesión caducada: vuelve a iniciar sesión".into()),
            CloudWait => t!(l;
                "wait a few minutes: the cloud is queried at most every 5 minutes".into(),
                "patientez quelques minutes : le cloud n'est interrogé qu'une fois toutes les 5 minutes".into(),
                "bitte einige Minuten warten: die Cloud wird höchstens alle 5 Minuten abgefragt".into(),
                "espera unos minutos: la nube se consulta como máximo cada 5 minutos".into()),
            CloudEnableUnsupported => t!(l;
                "this battery can't be re-enabled remotely (its cloud setup isn't supported)".into(),
                "cette batterie ne peut pas être réactivée à distance (sa configuration cloud n'est pas prise en charge)".into(),
                "diese Batterie kann nicht aus der Ferne wieder aktiviert werden (ihre Cloud-Konfiguration wird nicht unterstützt)".into(),
                "esta batería no puede reactivarse a distancia (su configuración en la nube no es compatible)".into()),
            CloudBadCtMac => t!(l;
                "the CT's MAC address must be 12 hexadecimal characters".into(),
                "l'adresse MAC du CT doit comporter 12 caractères hexadécimaux".into(),
                "die MAC-Adresse des CT muss aus 12 Hexadezimalzeichen bestehen".into(),
                "la dirección MAC del CT debe tener 12 caracteres hexadecimales".into()),
            CloudCtReselectSent => t!(l;
                "CT lost after the outage: re-selection command sent through the cloud".into(),
                "CT perdu après la coupure : commande de re-sélection envoyée via le cloud".into(),
                "CT nach dem Ausfall verloren: Befehl zur erneuten Auswahl über die Cloud gesendet".into(),
                "CT perdido tras el corte: orden de reselección enviada por la nube".into()),
            CloudMqtt => t!(l;
                "couldn't send the re-enable command through the cloud; re-enable Open API in the Marstek app".into(),
                "impossible d'envoyer la commande de réactivation via le cloud ; réactive l'Open API dans l'app Marstek".into(),
                "Reaktivierungsbefehl konnte nicht über die Cloud gesendet werden; aktiviere die Open API in der Marstek-App".into(),
                "no se pudo enviar la orden de reactivación por la nube; reactiva la Open API en la app Marstek".into()),
            CloudReenableSent => t!(l;
                "local API re-enable command sent through the cloud".into(),
                "commande de réactivation de l'API locale envoyée via le cloud".into(),
                "Befehl zur Reaktivierung der lokalen API über die Cloud gesendet".into(),
                "orden de reactivación de la API local enviada por la nube".into()),
            TrayOpen => t!(l; "Open".into(), "Ouvrir".into(), "Öffnen".into(), "Abrir".into()),
            TrayStop => t!(l; "Stop".into(), "Arrêt".into(), "Stopp".into(), "Parar".into()),
            TrayQuit => t!(l; "Quit".into(), "Quitter".into(), "Beenden".into(), "Salir".into()),
            TrayOffline => t!(l;
                "Marstek LAN: battery unreachable".into(),
                "Marstek LAN : batterie injoignable".into(),
                "Marstek LAN: Batterie nicht erreichbar".into(),
                "Marstek LAN: batería inaccesible".into()),
        }
    }
}

/// Dans la langue courante de l'interface.
impl fmt::Display for Msg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text(lang()))
    }
}

/// Texte de l'interface et texte du journal (toujours français).
pub fn pair(msg: &Msg, ui: Lang) -> (String, String) {
    (msg.text(ui), msg.text(Lang::Fr))
}

/// Un message du journal de l'onglet Journal (`log`, langue de l'interface) et du fichier (`journal`, français).
pub fn log(app: &AppHandle, msg: &Msg) {
    let (ui, fr) = pair(msg, lang());
    let _ = app.emit("log", ui);
    let _ = app.emit("journal", fr);
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANGS: [Lang; 4] = [Lang::En, Lang::Fr, Lang::De, Lang::Es];

    /// Une instance de chaque variante : `all` ne compile plus si une variante est ajoutée sans être listée ici.
    fn all() -> Vec<Msg> {
        use Msg::*;
        let s = || "x".to_owned();
        let leaf = Box::new(Raw(s()));
        let msgs = vec![
            Raw(s()), Detail(leaf.clone(), leaf.clone()), Section(Part::Wifi), Mode(ModeKind::Ai),
            Io(s()), BadReply(s()), NoReply, PortClosed(1), DeviceRefused { code: 1, message: s() }, TooManySlots(11), SlotsNeeded(11),
            BadRange(s()), RangeTooWide(2), NoInterface, BadPort(s()), ScanTooBig { hosts: 1, ports: 2, n: 2, max: 3 },
            BadTime(s()), BadBlock { end: s(), start: s() }, BadDay(8), Overlap(2),
            ScanStage1 { ports: s(), wait: s() }, ScanManual { hosts: 1, ports: s(), wait: s() }, ScanStage { n: 2, hosts: 1, ports: s(), wait: s() },
            ThreadStopped, NoBattery, TestRunning, TestAlready, TestRefused, TestNearMidnight, SlotsMissing(s()), BadIp(s()), CsvWrite(s()),
            Accepted, Refused, SlotsRetry(s()), PowerLowered { from: 1, cap: 2 },
            WriteMode, WriteSlots, WritePlan, WriteDod, LabelConnection, LabelNotification, LabelHistory, LabelBatterySearch,
            LabelPortSearch, LabelAddrNotSaved, LabelPortNotSaved, LabelPlanNotSaved, LabelTestNotSaved, LabelPlan(s()),
            Connecting(s()), NoReplyAside, Found { new: s(), old: s() }, PortSearch(s()), PortFound(1), PortNotFound, PollTuned(60), Reapplied(s()), TestNoResult,
            TitleLowBattery, TitleUnreachable, TitleReconnected, TitleWriteFailed, TitleModeChanged, AutoWriteRefused,
            OfflinePortClosed(1), OfflineNoWifi, OfflineSilent, Back { mins: 1, check: true }, Back { mins: 1, check: false },
            RuleLow, RuleHigh, RuleApplied { high: true, mode: ModeKind::Ups }, Block(ModeKind::Auto), SocLimit, SocLimitHold,
            AppDriven(leaf.clone()), NoActivePlan, ReadOnlyPlan(leaf.clone()), PlanTakesOver(s()), Charge(1), Discharge(1), Passive(leaf),
            DownPortClosed(1), DownSilentSecs(1), DownSilentMins(1), DownSilent, NoPlanSent, SrcOutsideTest, SrcRule, SrcManual,
            PowerUnknown, ReadOnlyNotFollowed, HoldCap, OutsideSlot, ModeNotRead, ModeMismatch { expected: s(), read: s() },
            OutsideUnknown, BatPowerUnread, OppositeDir { target: 1.0, measured: -1.0 }, BatStopped, Gap(1.0),
            LabelCloud, CloudUnreadable, CloudLoginRefused, CloudUnexpected, CloudTimeout, CloudHttp(500), CloudUnreachable,
            CloudBadEmail, CloudBadPassword, CloudStore, CloudExpired, CloudWait, CloudEnableUnsupported, CloudMqtt, CloudReenableSent, CloudBadCtMac, CloudCtReselectSent,
            TrayOpen, TrayStop, TrayQuit, TrayOffline,
        ];
        let _exhaustive = |m: &Msg| match m {
            Raw(_) | Detail(..) | Section(_) | Mode(_) | Io(_) | BadReply(_) | NoReply | PortClosed(_) | DeviceRefused { .. }
            | TooManySlots(_) | SlotsNeeded(_) | BadRange(_) | RangeTooWide(_) | NoInterface | BadPort(_) | ScanTooBig { .. }
            | BadTime(_) | BadBlock { .. } | BadDay(_) | Overlap(_) | ScanStage1 { .. } | ScanManual { .. } | ScanStage { .. }
            | ThreadStopped | NoBattery | TestRunning | TestAlready | TestRefused | TestNearMidnight | SlotsMissing(_) | BadIp(_)
            | CsvWrite(_) | Accepted | Refused | SlotsRetry(_) | PowerLowered { .. } | WriteMode | WriteSlots | WritePlan | WriteDod
            | LabelConnection | LabelNotification | LabelHistory | LabelBatterySearch | LabelPortSearch | LabelAddrNotSaved
            | LabelPortNotSaved | LabelPlanNotSaved | LabelTestNotSaved | LabelPlan(_) | Connecting(_) | NoReplyAside | Found { .. }
            | PortSearch(_) | PortFound(_) | PortNotFound | PollTuned(_) | Reapplied(_) | TestNoResult | TitleLowBattery | TitleUnreachable
            | TitleReconnected | TitleWriteFailed | TitleModeChanged | AutoWriteRefused | OfflinePortClosed(_) | OfflineNoWifi
            | OfflineSilent | Back { .. } | RuleLow | RuleHigh | RuleApplied { .. } | Block(_) | SocLimit | SocLimitHold
            | AppDriven(_) | NoActivePlan | ReadOnlyPlan(_) | PlanTakesOver(_) | Charge(_) | Discharge(_) | Passive(_)
            | DownPortClosed(_) | DownSilentSecs(_) | DownSilentMins(_) | DownSilent | NoPlanSent | SrcOutsideTest | SrcRule
            | SrcManual | PowerUnknown | ReadOnlyNotFollowed | HoldCap | OutsideSlot | ModeNotRead | ModeMismatch { .. }
            | OutsideUnknown | BatPowerUnread | OppositeDir { .. } | BatStopped | Gap(_) | LabelCloud | CloudUnreadable
            | CloudLoginRefused | CloudUnexpected | CloudTimeout | CloudHttp(_) | CloudUnreachable | CloudBadEmail | CloudBadPassword
            | CloudStore | CloudExpired | CloudWait | CloudEnableUnsupported | CloudMqtt | CloudReenableSent | CloudBadCtMac | CloudCtReselectSent | TrayOpen | TrayStop
            | TrayQuit | TrayOffline => (),
        };
        msgs
    }

    #[test]
    fn every_message_exists_in_the_four_languages() {
        for msg in all() {
            for lang in LANGS {
                assert!(!msg.text(lang).trim().is_empty(), "{msg:?} vide en {lang:?}");
            }
        }
    }

    #[test]
    fn languages_differ_for_translated_messages() {
        for msg in [Msg::TrayOpen, Msg::NoBattery, Msg::OfflineSilent, Msg::TitleLowBattery] {
            let texts: Vec<_> = LANGS.iter().map(|&l| msg.text(l)).collect();
            assert!((0..4).all(|i| (i + 1..4).all(|j| texts[i] != texts[j])), "{msg:?}: {texts:?}");
        }
    }

    #[test]
    fn french_is_kept_as_it_was() {
        let fr = |m: Msg| m.text(Lang::Fr);
        assert_eq!(fr(Msg::Back { mins: 6, check: true }), "Batterie de nouveau joignable (coupure de 6 min). Vérifie la CT et le planning.");
        assert_eq!(fr(Msg::PortClosed(30000)), "port 30000 fermé sur la batterie : Open API désactivée ou port changé dans l'app Marstek");
        assert_eq!(fr(Msg::SlotsMissing("1, 3".into())), "créneaux non écrits : 1, 3");
        assert_eq!(fr(Msg::Overlap(2)), "des blocs se chevauchent le mercredi");
        assert_eq!(fr(Msg::LabelPlan("Été".into()).detail(Msg::Refused)), "Plan « Été » : refusé par la batterie");
        assert_eq!(fr(Msg::Passive(Box::new(Msg::Discharge(300)))), "Passif décharge 300 W");
        assert_eq!(fr(Msg::Mode(ModeKind::Ai)), "IA");
    }

    #[test]
    fn the_journal_stays_in_french_whatever_the_interface_language() {
        let msg = Msg::NoBattery;
        for ui in LANGS {
            let (shown, journal) = pair(&msg, ui);
            assert_eq!(shown, msg.text(ui));
            assert_eq!(journal, "aucune batterie connectée");
        }
        let detail = Msg::LabelConnection.detail(Msg::NoReply);
        assert_eq!(pair(&detail, Lang::De).1, "Connexion : pas de réponse (Open API activée ? bonne IP/port ?)");
    }

    #[test]
    fn the_language_setting_reads_lowercase_codes_and_defaults_to_english() {
        let read = |v: &str| serde_json::from_str::<Lang>(v).unwrap_or_default();
        assert_eq!([read("\"fr\""), read("\"de\""), read("\"es\""), read("\"en\"")], [Lang::Fr, Lang::De, Lang::Es, Lang::En]);
        assert_eq!((read("\"it\""), Lang::default()), (Lang::En, Lang::En));
    }
}
