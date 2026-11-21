//! Thread réseau : l'interface ne bloque jamais pendant les échanges UDP.

use crate::api::{self, BatStatus, Client, DeviceInfo, EsMode, EsStatus, ModeConfig};
use std::net::SocketAddr;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

/// La batterie supporte mal les requêtes trop rapprochées.
const PAUSE: Duration = Duration::from_millis(250);

pub enum Command {
    Discover(u16),
    Refresh(SocketAddr),
    SetMode(SocketAddr, ModeConfig),
    SetDod(SocketAddr, u8),
}

#[derive(Default)]
pub struct Snapshot {
    pub device: Option<DeviceInfo>,
    pub bat: Option<BatStatus>,
    pub es: Option<EsStatus>,
    pub mode: Option<EsMode>,
}

pub enum Event {
    Discovered(Vec<(SocketAddr, DeviceInfo)>),
    Status(Box<Snapshot>),
    Log(String),
}

pub fn spawn(ctx: eframe::egui::Context) -> (Sender<Command>, Receiver<Event>) {
    let (cmd_tx, cmd_rx) = channel::<Command>();
    let (evt_tx, evt_rx) = channel::<Event>();

    thread::spawn(move || {
        let send = |e: Event| {
            let _ = evt_tx.send(e);
            ctx.request_repaint();
        };
        let log = |msg: String| send(Event::Log(msg));

        for cmd in cmd_rx {
            match cmd {
                Command::Discover(port) => match api::discover(port) {
                    Ok(list) => {
                        log(format!("{} batterie(s) trouvée(s)", list.len()));
                        send(Event::Discovered(list));
                    }
                    Err(e) => log(format!("Recherche : {e}")),
                },
                Command::Refresh(addr) => match Client::new(addr) {
                    Ok(mut c) => send(Event::Status(Box::new(read_all(&mut c, &log)))),
                    Err(e) => log(format!("Connexion : {e}")),
                },
                Command::SetMode(addr, cfg) => {
                    let r = Client::new(addr).and_then(|mut c| c.set_mode(&cfg));
                    log(match r {
                        Ok(true) => format!("Mode appliqué : {cfg:?}"),
                        Ok(false) => "La batterie a refusé le mode".into(),
                        Err(e) => format!("Changement de mode : {e}"),
                    });
                }
                Command::SetDod(addr, v) => {
                    let r = Client::new(addr).and_then(|mut c| c.set_dod(v));
                    log(match r {
                        Ok(true) => format!("Profondeur de décharge réglée à {v} %"),
                        Ok(false) => "La batterie a refusé le réglage DOD".into(),
                        Err(e) => format!("DOD : {e}"),
                    });
                }
            }
        }
    });

    (cmd_tx, evt_rx)
}

/// Lit chaque bloc séparément : un appel en échec n'empêche pas les autres.
fn read_all(c: &mut Client, log: &impl Fn(String)) -> Snapshot {
    fn step<T>(name: &str, r: Result<T, api::ApiError>, log: &impl Fn(String)) -> Option<T> {
        thread::sleep(PAUSE);
        r.map_err(|e| log(format!("{name} : {e}"))).ok()
    }
    Snapshot {
        device: step("Appareil", c.device_info(), log),
        bat: step("Batterie", c.bat_status(), log),
        es: step("Système", c.es_status(), log),
        mode: step("Mode", c.es_mode(), log),
    }
}
