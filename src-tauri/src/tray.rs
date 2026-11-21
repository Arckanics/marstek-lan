//! Icône de zone de notification : pile dessinée à l'exécution, infobulle et menu.

use crate::api::{CHARGE_SIGN, Direction, ModeReq};
use crate::worker::{Worker, Write};
use std::thread;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

const ID: &str = "main";
const SIZE: u32 = 32;
/// En dessous, la batterie est considérée au repos (bruit de mesure).
const IDLE_BELOW: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flow {
    Charge,
    Discharge,
    Idle,
    Offline,
}

/// Sens du flux d'après la puissance batterie, signe de `CHARGE_SIGN` (charge = négatif tant que non vérifié).
pub fn flow(online: bool, power: Option<f64>) -> Flow {
    match power {
        _ if !online => Flow::Offline,
        Some(p) if p * f64::from(CHARGE_SIGN) > IDLE_BELOW => Flow::Charge,
        Some(p) if p * f64::from(CHARGE_SIGN) < -IDLE_BELOW => Flow::Discharge,
        _ => Flow::Idle,
    }
}

/// Segments allumés de la pile, 0 à 5.
pub fn level(soc: Option<f64>) -> u8 {
    soc.map_or(0, |s| (s / 20.0).round().clamp(0.0, 5.0) as u8)
}

pub fn tooltip(online: bool, soc: Option<f64>, power: Option<f64>, mode: Option<&str>) -> String {
    if !online {
        return "Marstek LAN : batterie injoignable".into();
    }
    let parts: Vec<String> = [soc.map(|s| format!("SOC {s:.0} %")), power.map(|p| format!("{p:+.0} W")), mode.map(str::to_owned)]
        .into_iter()
        .flatten()
        .collect();
    if parts.is_empty() { "Marstek LAN".into() } else { parts.join(" · ") }
}

fn rect(px: &mut [u8], x: std::ops::Range<u32>, y: std::ops::Range<u32>, color: [u8; 4]) {
    for y in y {
        for x in x.clone() {
            let i = ((y * SIZE + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&color);
        }
    }
}

/// Tampon RGBA 32×32 : pile verticale à 5 segments, vert en charge, orange en décharge, gris hors ligne.
pub fn icon_rgba(level: u8, flow: Flow) -> Vec<u8> {
    let [r, g, b] = match flow {
        Flow::Charge => [60, 190, 90],
        Flow::Discharge => [240, 150, 40],
        Flow::Idle => [215, 220, 225],
        Flow::Offline => [130, 130, 130],
    };
    let outline = [150, 150, 155, 255];
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    rect(&mut px, 13..19, 2..5, outline);
    rect(&mut px, 8..24, 5..30, outline);
    rect(&mut px, 10..22, 7..28, [0; 4]);
    if flow != Flow::Offline {
        for i in 0..u32::from(level.min(5)) {
            let bottom = 28 - i * 4;
            rect(&mut px, 11..21, bottom - 3..bottom, [r, g, b, 255]);
        }
    }
    px
}

pub fn update(app: &AppHandle, level: u8, flow: Flow, tip: &str) {
    if let Some(tray) = app.tray_by_id(ID) {
        let _ = tray.set_icon(Some(Image::new_owned(icon_rgba(level, flow), SIZE, SIZE)));
        let _ = tray.set_tooltip(Some(tip));
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Le menu tourne sur le thread principal : l'écriture (bloquante) part sur son propre thread.
fn set_mode(app: &AppHandle, mode: ModeReq) {
    let app = app.clone();
    thread::spawn(move || {
        let _ = app.state::<Worker>().write_one(Write::Mode(mode));
    });
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Ouvrir", true, None::<&str>)?;
    let auto = MenuItem::with_id(app, "auto", "Auto", true, None::<&str>)?;
    let ai = MenuItem::with_id(app, "ai", "IA", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Arrêt", true, None::<&str>)?;
    let suspend = CheckMenuItem::with_id(app, "suspend", "Suspendre les règles", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &sep, &auto, &ai, &stop, &suspend, &sep, &quit])?;

    TrayIconBuilder::with_id(ID)
        .icon(Image::new_owned(icon_rgba(0, Flow::Offline), SIZE, SIZE))
        .tooltip("Marstek LAN")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "auto" => set_mode(app, ModeReq::Auto),
            "ai" => set_mode(app, ModeReq::Ai),
            "stop" => set_mode(app, ModeReq::Passive { direction: Direction::Charge, power: 0, seconds: 3600 }),
            "suspend" => {
                let _ = app.state::<Worker>().suspend(suspend.is_checked().unwrap_or(false));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(px: &[u8]) -> usize {
        px.chunks(4).filter(|p| p[3] == 255).count()
    }

    fn pixel(px: &[u8], x: u32, y: u32) -> &[u8] {
        let i = ((y * SIZE + x) * 4) as usize;
        &px[i..i + 4]
    }

    #[test]
    fn icon_is_a_transparent_32_square_buffer() {
        let px = icon_rgba(3, Flow::Idle);
        assert_eq!(px.len(), 32 * 32 * 4);
        assert_eq!(pixel(&px, 0, 0), [0, 0, 0, 0]);
        assert_eq!(pixel(&px, 31, 31), [0, 0, 0, 0]);
    }

    #[test]
    fn more_charge_lights_more_segments() {
        let counts: Vec<_> = (0..=5).map(|l| opaque(&icon_rgba(l, Flow::Charge))).collect();
        assert!(counts.windows(2).all(|w| w[0] < w[1]), "{counts:?}");
        assert_eq!(opaque(&icon_rgba(9, Flow::Charge)), counts[5]);
    }

    #[test]
    fn color_follows_the_flow() {
        assert_eq!(pixel(&icon_rgba(1, Flow::Charge), 16, 26), [60, 190, 90, 255]);
        assert_eq!(pixel(&icon_rgba(1, Flow::Discharge), 16, 26), [240, 150, 40, 255]);
        assert_eq!(pixel(&icon_rgba(1, Flow::Idle), 16, 26), [215, 220, 225, 255]);
    }

    #[test]
    fn offline_is_grey_without_fill() {
        let px = icon_rgba(5, Flow::Offline);
        assert_eq!(pixel(&px, 16, 26), [0, 0, 0, 0]);
        assert!(px.chunks(4).filter(|p| p[3] == 255).all(|p| p[..3] == [150, 150, 155]));
    }

    #[test]
    fn level_rounds_to_five_steps() {
        assert_eq!([level(None), level(Some(0.0)), level(Some(9.0)), level(Some(76.0)), level(Some(100.0))], [0, 0, 0, 4, 5]);
    }

    #[test]
    fn flow_uses_the_charge_sign_with_a_dead_band() {
        let charge = f64::from(CHARGE_SIGN) * 400.0;
        assert_eq!(flow(true, Some(charge)), Flow::Charge);
        assert_eq!(flow(true, Some(-charge)), Flow::Discharge);
        assert_eq!(flow(true, Some(5.0)), Flow::Idle);
        assert_eq!(flow(true, None), Flow::Idle);
        assert_eq!(flow(false, Some(charge)), Flow::Offline);
    }

    #[test]
    fn tooltip_lists_what_is_known() {
        assert_eq!(tooltip(true, Some(76.0), Some(420.0), Some("Auto")), "SOC 76 % · +420 W · Auto");
        assert_eq!(tooltip(true, Some(76.0), None, None), "SOC 76 %");
        assert_eq!(tooltip(true, None, None, None), "Marstek LAN");
        assert_eq!(tooltip(false, Some(76.0), None, None), "Marstek LAN : batterie injoignable");
    }
}
