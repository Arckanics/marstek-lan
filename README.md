# marstek-lan

Windows desktop app that controls a Marstek home battery (written for the Venus D) over the local network, through the Marstek Device Local API (JSON-RPC over UDP, port 30000). Everything runs on the LAN; the optional Marstek account panel is the only feature that goes online.

> Alpha. The charge/discharge power sign and the battery's behavior outside Manual slots are not documented by Marstek. Run the built-in checks described below before relying on a plan.

## Features

- Finds batteries by broadcast and by sweeping the PC's subnet or a custom IP range
- Live status: power-flow diagram (solar, battery, grid, house), SOC gauge, last-hour chart
- Modes: Auto, AI, UPS, Passive, with editable presets (charge or discharge X W for Y min, back to Auto)
- Weekly planner drawn with the mouse, compiled into the battery's 10 Manual slots so the battery runs it on its own with the PC off; named templates, day copy, off-peak tariff generator
- SOC rules (switch mode below or above a threshold, 5-point hysteresis)
- Runs in the notification area with a SOC icon, optional launch at Windows startup, Windows notifications
- Local history in SQLite: daily power curves, weekly and monthly energy bars, CSV export
- Device tab: Wi-Fi, CT meter, PV inputs, energy counters, DOD, LED and Bluetooth settings
- Optional read-only Marstek account panel (firmware version, serial number); only the session token is stored, in the Windows Credential Manager

## Before you rely on it

1. Enable Open API on the battery in the Marstek app.
2. Modes tab: run the "Charge 100 W for 1 min" preset and check that the battery charges. If it discharges, the sign constant `CHARGE_SIGN` in `src-tauri/src/api.rs` must be flipped.
3. Planning tab: run the "outside slots" test (about 10 minutes) to learn what your firmware does between slots.

## Install

Download the installer from the [releases](https://github.com/Arckanics/marstek-lan/releases). It is not code-signed, so Windows SmartScreen shows "Unknown publisher".

## Build

Requires Rust (edition 2024), Node 24 and the Tauri CLI (`cargo install tauri-cli`).

```sh
npm ci
cargo tauri dev     # development
cargo tauri build   # NSIS installer in target/release/bundle/nsis
```

## Notes

- The official API documentation has inconsistencies in units and field names, so every field the app reads is optional.
- The Marstek account uses an unofficial API reverse-engineered by the community. Logging in may disconnect the mobile app.

## License

MIT or Apache-2.0, at your option.

---

## En français

Application Windows qui pilote une batterie domestique Marstek (écrite pour la Venus D) sur le réseau local, via l'API locale Marstek (JSON-RPC sur UDP, port 30000). Tout reste sur le réseau local ; seul le panneau facultatif « Compte Marstek » passe par internet.

Version alpha : le signe de puissance charge/décharge et le comportement hors créneaux ne sont pas documentés par Marstek. Avant de vous fier à un plan, lancez le préréglage « Charger 100 W pendant 1 min » (onglet Modes) et le test « hors créneaux » (onglet Planning).

Fonctions : découverte par broadcast et balayage d'IP, état en direct (diagramme de flux, jauge SOC, courbe de la dernière heure), modes Auto, IA, UPS et Passif avec préréglages, planning hebdomadaire envoyé à la batterie (elle l'exécute seule, PC éteint), règles selon le SOC, icône dans la zone de notification, démarrage avec Windows, notifications, historique local (jour, semaine, mois, export CSV), réglages de l'appareil (DOD, LED, Bluetooth), compte Marstek en lecture seule.

Installation : téléchargez l'installateur dans les [releases](https://github.com/Arckanics/marstek-lan/releases). Il n'est pas signé : Windows SmartScreen affiche « Éditeur inconnu ».
