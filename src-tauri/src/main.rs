#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod clock;
mod cloud;
mod control;
mod history;
mod i18n;
mod journal;
mod mqtt;
mod plan;
mod settings;
mod topic_id;
mod tray;
mod worker;

use api::{ModeReq, Scan, Slot};
use i18n::Msg;
use history::{AverageDay, Bucket, Column, EnergyRow, History, PowerSeries, Price, Savings, SocStat};
use plan::{Block, Plan, Tariff};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use worker::{Worker, Write};

type Res<T> = Result<T, String>;

struct ScanCancel(AtomicBool);

// `async` : sans lui, la commande bloquerait le thread de l'interface.
#[tauri::command(async)]
fn discover(range: Option<String>, ports: String, auto: bool, cancel: State<ScanCancel>, app: tauri::AppHandle) -> Res<Scan> {
    cancel.0.store(false, Ordering::Relaxed);
    let scan = api::discover(range.as_deref(), &ports, auto, &cancel.0, &mut |p| {
        let _ = app.emit("scan", p);
    })
    .map_err(|e| e.to_string())?;
    if let Some(saved) = &scan.saved_ports {
        settings::save_ports(&app, saved)?;
    }
    Ok(scan)
}

#[tauri::command]
fn cancel_scan(cancel: State<ScanCancel>) {
    cancel.0.store(true, Ordering::Relaxed);
}

#[tauri::command]
fn connect(ip: String, port: u16, worker: State<Worker>) -> Res<()> {
    let ip: Ipv4Addr = ip.trim().parse().map_err(|_| Msg::BadIp(ip).to_string())?;
    worker.connect(SocketAddr::new(ip.into(), port))
}

#[tauri::command]
fn refresh(worker: State<Worker>) -> Res<()> {
    worker.refresh()
}

#[tauri::command(async)]
fn set_mode(mode: ModeReq, worker: State<Worker>) -> Res<bool> {
    worker.write_one(Write::Mode(mode))
}

#[tauri::command]
fn compile_plan(plan: Plan) -> Res<Vec<Slot>> {
    plan::compile_plan(&plan.blocks).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn apply_plan(plan: Plan, worker: State<Worker>) -> Res<Vec<bool>> {
    let slots = plan::compile_plan(&plan.blocks).map_err(|e| e.to_string())?;
    worker.write(Write::Plan(plan, slots))
}

#[tauri::command]
fn tariff_plan(tariff: Tariff) -> Res<Vec<Block>> {
    plan::tariff_plan(&tariff).map_err(|e| e.to_string())
}

#[tauri::command]
fn config_changed(worker: State<Worker>) -> Res<()> {
    worker.reload()
}

#[tauri::command(async)]
fn set_dod(value: u8, worker: State<Worker>) -> Res<bool> {
    worker.write_one(Write::Dod(value))
}

#[tauri::command(async)]
fn set_led(on: bool, worker: State<Worker>) -> Res<bool> {
    worker.write_one(Write::Led(on))
}

#[tauri::command(async)]
fn set_ble(on: bool, worker: State<Worker>) -> Res<bool> {
    worker.write_one(Write::Ble(on))
}

#[tauri::command(async)]
fn run_slot_test(worker: State<Worker>) -> Res<()> {
    worker.slot_test()
}

// `async` : ces commandes attendent le réseau (jusqu'à 20 s) et ne doivent pas bloquer l'interface.
#[tauri::command(async)]
fn cloud_login(email: String, password: String, region: cloud::Region) -> Res<()> {
    cloud::login(&email, &password, region).map_err(|m| m.to_string())
}

#[tauri::command(async)]
fn cloud_devices() -> Res<Vec<cloud::CloudDevice>> {
    cloud::devices().map_err(|m| m.to_string())
}

#[tauri::command(async)]
fn cloud_logout() -> Res<()> {
    cloud::logout().map_err(|m| m.to_string())
}

#[tauri::command(async)]
fn cloud_status() -> Res<Option<cloud::Status>> {
    cloud::status().map_err(|m| m.to_string())
}

// `async` : attend le réseau (HTTPS puis une connexion MQTT) et ne doit pas bloquer l'interface.
#[tauri::command(async)]
fn cloud_enable_local_api(devid: String) -> Res<()> {
    cloud::enable_local_api(&devid).map_err(|m| m.to_string())
}

#[tauri::command(async)]
fn cloud_reselect_ct(devid: String, meter: u8, mac: Option<String>) -> Res<String> {
    cloud::reselect_ct(&devid, meter, mac.as_deref()).map_err(|m| m.to_string())
}

#[tauri::command(async)]
fn history_power(from: i64, to: i64, history: State<History>) -> Res<PowerSeries> {
    history.power(from, to).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn history_energy(from: i64, to: i64, bucket: Bucket, history: State<History>) -> Res<Vec<EnergyRow>> {
    history.energy(from, to, bucket, clock::local_offset_ms()).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn history_soc(from: i64, to: i64, profile: bool, history: State<History>) -> Res<Vec<SocStat>> {
    let offset = clock::local_offset_ms();
    (if profile { history.soc_profile(from, to, offset) } else { history.soc_daily(from, to, offset) }).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn history_average(column: Column, days: i64, weekend: Option<bool>, history: State<History>) -> Res<AverageDay> {
    history.average_day(column, days, weekend, 15, clock::now_ms(), clock::local_offset_ms()).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn history_savings(from: i64, to: i64, price: Price, history: State<History>) -> Res<Savings> {
    history.savings(from, to, &price, clock::local_offset_ms()).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn export_csv(from: i64, to: i64, history: State<History>, app: tauri::AppHandle) -> Res<String> {
    let offset = clock::local_offset_ms();
    let csv = history::csv(&history.power(from, to).map_err(|e| e.to_string())?, offset);
    let day = |ms| clock::format_local(ms, offset)[..10].to_owned();
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let path = dir.join(format!("marstek-{}_{}.csv", day(from), day(to)));
    std::fs::write(&path, csv).map_err(|e| Msg::CsvWrite(e.to_string()).to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .on_window_event(|window, event| {
            // Fermer la fenêtre la cache : « Quitter » n'existe que dans le menu de l'icône.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let history = History::open_or_recover(&dir.join("history.db"));
            app.manage(history);
            // Avant le worker et l'icône : leurs premiers textes sont déjà dans la langue choisie.
            i18n::set(settings::load(app.handle()).lang);
            journal::setup(app.handle(), &dir);
            app.manage(ScanCancel(AtomicBool::new(false)));
            app.manage(Worker::spawn(app.handle().clone()));
            tray::setup(app.handle())?;
            // La fenêtre est créée cachée (tauri.conf.json) : seul un démarrage sans `--hidden` l'affiche.
            if !std::env::args().any(|a| a == "--hidden")
                && let Some(window) = app.get_webview_window("main")
            {
                window.show()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            discover,
            cancel_scan,
            connect,
            refresh,
            set_mode,
            compile_plan,
            apply_plan,
            tariff_plan,
            config_changed,
            set_dod,
            set_led,
            set_ble,
            run_slot_test,
            cloud_login,
            cloud_devices,
            cloud_logout,
            cloud_status,
            cloud_enable_local_api,
            cloud_reselect_ct,
            history_power,
            history_energy,
            history_soc,
            history_average,
            history_savings,
            export_csv
        ])
        .build(tauri::generate_context!())
        .expect("erreur au lancement de l'application")
        .run(|_, _| {});
}
