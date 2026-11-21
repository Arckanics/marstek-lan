#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod clock;
mod cloud;
mod control;
mod history;
mod plan;
mod settings;
mod tray;
mod worker;

use api::{DEFAULT_PORT, Device, ModeReq, Slot};
use history::{Bucket, EnergyRow, History, PowerSeries};
use plan::{Block, Plan, Tariff};
use std::net::{Ipv4Addr, SocketAddr};
use tauri::{Manager, State, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;
use worker::{Worker, Write};

type Res<T> = Result<T, String>;

// `async` : sans lui, la commande bloquerait le thread de l'interface.
#[tauri::command(async)]
fn discover(range: Option<String>) -> Res<Vec<Device>> {
    api::discover(range.as_deref(), DEFAULT_PORT).map_err(|e| e.to_string())
}

#[tauri::command]
fn connect(ip: String, worker: State<Worker>) -> Res<()> {
    let ip: Ipv4Addr = ip.trim().parse().map_err(|_| format!("adresse IP invalide : {ip}"))?;
    worker.connect(SocketAddr::new(ip.into(), DEFAULT_PORT))
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

#[tauri::command(async)]
fn history_power(from: i64, to: i64, history: State<History>) -> Res<PowerSeries> {
    history.power(from, to).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn history_energy(from: i64, to: i64, bucket: Bucket, history: State<History>) -> Res<Vec<EnergyRow>> {
    history.energy(from, to, bucket, clock::local_offset_ms()).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn export_csv(from: i64, to: i64, history: State<History>, app: tauri::AppHandle) -> Res<String> {
    let offset = clock::local_offset_ms();
    let csv = history::csv(&history.power(from, to).map_err(|e| e.to_string())?, offset);
    let day = |ms| clock::format_local(ms, offset)[..10].to_owned();
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let path = dir.join(format!("marstek-{}_{}.csv", day(from), day(to)));
    std::fs::write(&path, csv).map_err(|e| format!("écriture impossible : {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command(async)]
fn cloud_login(email: String, password: String, region: cloud::Region) -> Res<()> {
    cloud::login(&email, &password, region)
}

#[tauri::command(async)]
fn cloud_devices() -> Res<Vec<cloud::CloudDevice>> {
    cloud::devices()
}

#[tauri::command(async)]
fn cloud_logout() -> Res<()> {
    cloud::logout()
}

#[tauri::command(async)]
fn cloud_status() -> Res<Option<cloud::Status>> {
    cloud::status()
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
            // shortcut: base illisible = historique en mémoire pour la session (perdu à la fermeture), sans message.
            let history = History::open(&dir.join("history.db")).or_else(|_| History::memory())?;
            app.manage(history);
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
            history_power,
            history_energy,
            export_csv,
            cloud_login,
            cloud_devices,
            cloud_logout,
            cloud_status
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application");
}
