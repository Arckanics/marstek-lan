# marstek-lan

Windows desktop app that controls a Marstek home battery (written for the Venus D) over the local network, through the Marstek Device Local API (JSON-RPC over UDP, port 30000). Everything runs on the LAN; the optional cloud backup is the only feature that goes online.

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
- Optional cloud backup: when the local Open API stops answering (a firmware fault), SOC and power are read from the Marstek cloud at most every 5 minutes, through a secondary account the battery is shared with from the Marstek app (a main account would sign out the phone app); only the session token is stored, in Windows Credential Manager. One opt-in button re-enables the local Open API remotely when the firmware has switched it off after a reboot, by publishing a single command to the device over the Marstek cloud (the only write the cloud side makes)
- Interface in English (default), French, German and Spanish, switchable in Settings without a restart

## Before you rely on it

1. Enable Open API on the battery in the Marstek app.
2. Modes tab: run the "Charge 100 W for 1 min" preset and check that the battery charges. If it discharges, the sign constant `CHARGE_SIGN` in `src-tauri/src/api.rs` must be flipped.
3. Planning tab: run the "outside slots" test (about 10 minutes) to learn what your firmware does between slots.

## Install

Download the installer from the [releases](https://github.com/Arckanics/marstek-lan/releases). It is not code-signed, so Windows SmartScreen shows "Unknown publisher".

## Build

Requires Rust (edition 2024), Node 24, the Tauri CLI (`cargo install tauri-cli`) and PowerShell 7 (`pwsh`).

The remote re-enable feature embeds the hame relay's mTLS client certificate, which is not redistributed here (it is gitignored, as upstream keeps it out of its own repo). Fetch it once from the public `tomquist/hame-relay` image into `src-tauri/certs/` before building; `build.rs` checks it by sha256:

```sh
pwsh vendor-certs.ps1
npm ci
cargo tauri dev     # development
cargo tauri build   # NSIS installer in target/release/bundle/nsis
```

## Notes

- The official API documentation has inconsistencies in units and field names, so every field the app reads is optional.

## License

MIT or Apache-2.0, at your option.

---

## En français

Application Windows qui pilote une batterie domestique Marstek (écrite pour la Venus D) sur le réseau local, via l'API locale Marstek (JSON-RPC sur UDP, port 30000). Tout reste sur le réseau local, sauf le secours cloud facultatif (compte secondaire partagé) : lecture seule, plus un bouton qui rouvre l'API locale à distance quand le firmware l'a coupée après un redémarrage.

Version alpha : le signe de puissance charge/décharge et le comportement hors créneaux ne sont pas documentés par Marstek. Avant de vous fier à un plan, lancez le préréglage « Charger 100 W pendant 1 min » (onglet Modes) et le test « hors créneaux » (onglet Planning).

Fonctions : découverte par broadcast et balayage d'IP, état en direct (diagramme de flux, jauge SOC, courbe de la dernière heure), modes Auto, IA, UPS et Passif avec préréglages, planning hebdomadaire envoyé à la batterie (elle l'exécute seule, PC éteint), règles selon le SOC, icône dans la zone de notification, démarrage avec Windows, notifications, historique local (jour, semaine, mois, export CSV), réglages de l'appareil (DOD, LED, Bluetooth).

Installation : téléchargez l'installateur dans les [releases](https://github.com/Arckanics/marstek-lan/releases). Il n'est pas signé : Windows SmartScreen affiche « Éditeur inconnu ».
