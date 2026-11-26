use sha2::{Digest, Sha256};

/// Certs embarqués par `mqtt.rs` (hors dépôt, cf. `.gitignore`). Posés par `vendor-certs.ps1`, épinglés par sha256
/// pour détecter une absence ou une altération avant la compilation.
const CERTS: [(&str, &str); 3] = [
    ("certs/ca.crt", "2c43952ee9e000ff2acc4e2ed0897c0a72ad5fa72c3d934e81741cbd54f05bd1"),
    ("certs/hame-2025.crt", "2502b161f5f533defa924840b2c2764440eea27b12b910c5b23aeb214715984e"),
    ("certs/hame-2025.key", "6ceeedbacda597f98bc4605518c7e750101635dbf11f327d06ab215eb44886e4"),
];

fn main() {
    for (path, want) in CERTS {
        println!("cargo::rerun-if-changed={path}");
        let Ok(bytes) = std::fs::read(path) else {
            println!("cargo::error=certificat manquant : {path}. Lance `pwsh vendor-certs.ps1` pour le récupérer (voir README).");
            std::process::exit(1);
        };
        let got: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if got != want {
            println!("cargo::error=certificat {path} inattendu (sha256 {got}). Relance `pwsh vendor-certs.ps1`.");
            std::process::exit(1);
        }
    }
    tauri_build::build()
}
