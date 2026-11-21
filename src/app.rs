use crate::api::{DEFAULT_PORT, DeviceInfo, ManualSlot, ModeConfig};
use crate::worker::{self, Command, Event, Snapshot};
use eframe::egui::{self, RichText};
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

const AUTO_REFRESH: Duration = Duration::from_secs(10);
const DAYS: [&str; 7] = ["Lu", "Ma", "Me", "Je", "Ve", "Sa", "Di"];

#[derive(PartialEq, Clone, Copy)]
enum ModeChoice {
    Auto,
    Ai,
    Manual,
    Passive,
}

pub struct MarstekApp {
    tx: Sender<Command>,
    rx: Receiver<Event>,

    ip: String,
    port: u16,
    discovered: Vec<(SocketAddr, DeviceInfo)>,
    snapshot: Snapshot,
    log: Vec<String>,

    auto_refresh: bool,
    last_refresh: Option<Instant>,

    mode: ModeChoice,
    passive_power: i32,
    passive_secs: u32,
    slot: ManualSlot,
    dod: u8,
}

impl MarstekApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = worker::spawn(cc.egui_ctx.clone());
        Self {
            tx,
            rx,
            ip: String::new(),
            port: DEFAULT_PORT,
            discovered: Vec::new(),
            snapshot: Snapshot::default(),
            log: Vec::new(),
            auto_refresh: false,
            last_refresh: None,
            mode: ModeChoice::Auto,
            passive_power: 0,
            passive_secs: 300,
            slot: ManualSlot {
                time_num: 0,
                start: "22:00".into(),
                end: "06:00".into(),
                week_set: 127,
                power: 0,
                enable: true,
            },
            dod: 88,
        }
    }

    fn target(&mut self) -> Option<SocketAddr> {
        match self.ip.trim().parse::<IpAddr>() {
            Ok(ip) => Some(SocketAddr::new(ip, self.port)),
            Err(_) => {
                self.log.push("Adresse IP invalide".into());
                None
            }
        }
    }

    fn send(&mut self, make: impl FnOnce(SocketAddr) -> Command) {
        if let Some(addr) = self.target() {
            let _ = self.tx.send(make(addr));
        }
    }

    fn refresh(&mut self) {
        self.last_refresh = Some(Instant::now());
        self.send(Command::Refresh);
    }

    fn handle_events(&mut self) {
        while let Ok(evt) = self.rx.try_recv() {
            match evt {
                Event::Discovered(list) => {
                    if let [(addr, _)] = list.as_slice() {
                        self.ip = addr.ip().to_string();
                    }
                    self.discovered = list;
                }
                Event::Status(s) => self.snapshot = *s,
                Event::Log(msg) => self.log.push(msg),
            }
        }
    }

    fn connection_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("IP :");
            ui.add(egui::TextEdit::singleline(&mut self.ip).desired_width(130.0).hint_text("192.168.1.x"));
            ui.label("Port :");
            ui.add(egui::DragValue::new(&mut self.port));
            if ui.button("🔍 Rechercher").clicked() {
                let _ = self.tx.send(Command::Discover(self.port));
            }
            if ui.button("🔄 Actualiser").clicked() {
                self.refresh();
            }
            ui.checkbox(&mut self.auto_refresh, "Auto (10 s)");
        });
        let mut picked = None;
        for (addr, info) in &self.discovered {
            let name = info.device.as_deref().unwrap_or("?");
            if ui.small_button(format!("{name} — {addr}")).clicked() {
                picked = Some(addr.ip().to_string());
            }
        }
        if let Some(ip) = picked {
            self.ip = ip;
        }
    }

    fn status_ui(&self, ui: &mut egui::Ui) {
        let s = &self.snapshot;
        let w = |v: Option<f64>| v.map_or("—".into(), |v| format!("{v:.0} W"));
        let pct = |v: Option<f64>| v.map_or("—".into(), |v| format!("{v:.0} %"));

        egui::Grid::new("status").num_columns(2).striped(true).show(ui, |ui| {
            let mut row = |k: &str, v: String| {
                ui.label(k);
                ui.label(RichText::new(v).strong());
                ui.end_row();
            };
            let dev = s.device.as_ref();
            row("Appareil", dev.and_then(|d| d.device.clone()).unwrap_or("—".into()));
            row("Firmware", dev.and_then(|d| d.ver.as_ref()).map_or("—".into(), |v| v.to_string()));
            row("Charge (SOC)", pct(s.es.as_ref().and_then(|e| e.bat_soc).or(s.bat.as_ref().and_then(|b| b.soc))));
            row("Batterie", w(s.es.as_ref().and_then(|e| e.bat_power)));
            row("Solaire", w(s.es.as_ref().and_then(|e| e.pv_power)));
            row("Réseau (sortie onduleur)", w(s.es.as_ref().and_then(|e| e.ongrid_power)));
            row("Compteur CT", w(s.mode.as_ref().and_then(|m| m.total_power)));
            row(
                "CT connecté",
                s.mode.as_ref().and_then(|m| m.ct_state).map_or("—".into(), |c| if c == 1 { "oui" } else { "non" }.into()),
            );
            row("Température", s.bat.as_ref().and_then(|b| b.bat_temp).map_or("—".into(), |t| format!("{t:.1} °C")));
            row("Mode actuel", s.mode.as_ref().and_then(|m| m.mode.clone()).unwrap_or("—".into()));
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode, ModeChoice::Auto, "Auto");
            ui.selectable_value(&mut self.mode, ModeChoice::Ai, "IA");
            ui.selectable_value(&mut self.mode, ModeChoice::Manual, "Manuel");
            ui.selectable_value(&mut self.mode, ModeChoice::Passive, "Passif");
        });

        match self.mode {
            ModeChoice::Auto => {
                ui.label("Autoconsommation pilotée par le CT002.");
            }
            ModeChoice::Ai => {
                ui.label("Mode IA de Marstek (prévisions météo / habitudes).");
            }
            ModeChoice::Passive => {
                ui.horizontal(|ui| {
                    ui.label("Puissance (W) :");
                    ui.add(egui::DragValue::new(&mut self.passive_power).range(-2500..=2500));
                    ui.label("Durée (s) :");
                    ui.add(egui::DragValue::new(&mut self.passive_secs).range(0..=86_400));
                });
                ui.small("Le signe (charge / décharge) est à vérifier sur ta batterie avec une petite puissance.");
            }
            ModeChoice::Manual => {
                let s = &mut self.slot;
                ui.horizontal(|ui| {
                    ui.label("Créneau :");
                    ui.add(egui::DragValue::new(&mut s.time_num).range(0..=9));
                    ui.label("de");
                    ui.add(egui::TextEdit::singleline(&mut s.start).desired_width(50.0));
                    ui.label("à");
                    ui.add(egui::TextEdit::singleline(&mut s.end).desired_width(50.0));
                    ui.label("Puissance (W) :");
                    ui.add(egui::DragValue::new(&mut s.power).range(-2500..=2500));
                    ui.checkbox(&mut s.enable, "Actif");
                });
                ui.horizontal(|ui| {
                    for (i, day) in DAYS.iter().enumerate() {
                        let bit = 1u8 << i;
                        let mut on = s.week_set & bit != 0;
                        if ui.checkbox(&mut on, *day).changed() {
                            s.week_set ^= bit;
                        }
                    }
                });
            }
        }

        if ui.button("Appliquer le mode").clicked() {
            let cfg = match self.mode {
                ModeChoice::Auto => ModeConfig::Auto,
                ModeChoice::Ai => ModeConfig::Ai,
                ModeChoice::Manual => ModeConfig::Manual(self.slot.clone()),
                ModeChoice::Passive => ModeConfig::Passive { power: self.passive_power, cd_time: self.passive_secs },
            };
            self.send(|a| Command::SetMode(a, cfg));
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Profondeur de décharge (DOD) :");
            ui.add(egui::Slider::new(&mut self.dod, 30..=88).suffix(" %"));
            if ui.button("Appliquer").clicked() {
                let v = self.dod;
                self.send(|a| Command::SetDod(a, v));
            }
        });
        ui.small("88 % = valeur par défaut. Plus bas = plus de réserve gardée dans la batterie.");
    }
}

impl eframe::App for MarstekApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_events();

        if self.auto_refresh {
            let due = self.last_refresh.is_none_or(|t| t.elapsed() >= AUTO_REFRESH);
            if due {
                self.refresh();
            }
            ui.ctx().request_repaint_after(AUTO_REFRESH);
        }

        egui::Panel::top("connexion").show(ui, |ui| {
            ui.add_space(4.0);
            self.connection_ui(ui);
            ui.add_space(4.0);
        });

        egui::Panel::bottom("journal").resizable(true).default_size(120.0).show(ui, |ui| {
            ui.label(RichText::new("Journal").strong());
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                for line in &self.log {
                    ui.monospace(line);
                }
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("État");
                self.status_ui(ui);
                ui.add_space(12.0);
                ui.heading("Réglages");
                self.settings_ui(ui);
            });
        });
    }
}
