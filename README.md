# marstek-lan

Desktop app that controls a Marstek home battery (written for the Venus D) over the local network, through the Marstek Device Open API (JSON-RPC over UDP, port 30000). No cloud account involved.

## Features

- Find batteries on the LAN by UDP broadcast
- Read device info, battery status and energy-system status, with auto-refresh every 10 s
- Switch the operating mode: Auto, AI, Manual (time slots per weekday) or Passive (fixed power for a set duration)
- Set the depth of discharge

## Requirements

- Open API enabled on the battery (Marstek app, device settings)
- The PC and the battery on the same network
- Rust 1.85 or later (edition 2024)

## Build

```sh
cargo run --release
```

## Notes

The official API documentation (Rev 2.0) has inconsistencies in units and field names, so every field the app reads is optional. Check values against your own device.

## License

MIT or Apache-2.0, at your option.

---

## En français

Application de bureau pour piloter une batterie domestique Marstek (écrite pour la Venus D) sur le réseau local, via l'Open API Marstek (JSON-RPC sur UDP, port 30000). Aucun compte cloud.

Elle découvre les batteries du réseau, affiche leur état (rafraîchi toutes les 10 s), change le mode de fonctionnement (Auto, IA, Manuel par créneaux horaires, Passif à puissance fixe) et règle la profondeur de décharge.

Prérequis : Open API activée dans l'app Marstek, PC et batterie sur le même réseau. Lancement : `cargo run --release`.
