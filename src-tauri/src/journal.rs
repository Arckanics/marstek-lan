//! Journal sur disque : chaque message du journal et un résumé par sondage, pour diagnostiquer après coup.

use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Listener};

use crate::clock;

const MAX_BYTES: u64 = 5 * 1024 * 1024;

struct Sink {
    path: PathBuf,
    file: Option<File>,
    /// `seq` du dernier état journalisé.
    seen: Option<u64>,
}

impl Sink {
    fn write(&mut self, line: &str) {
        // Au-delà de 5 Mo, l'ancien journal devient `journal.1.log` (un seul gardé).
        if fs::metadata(&self.path).is_ok_and(|m| m.len() > MAX_BYTES) {
            self.file = None;
            let _ = fs::rename(&self.path, self.path.with_extension("1.log"));
        }
        if self.file.is_none() {
            self.file = OpenOptions::new().create(true).append(true).open(&self.path).ok();
        }
        if let Some(f) = self.file.as_mut() {
            let _ = writeln!(f, "{} {line}", stamp(clock::now_ms()));
        }
    }
}

/// `AAAA-MM-JJ hh:mm:ss` en heure locale.
fn stamp(ms: i64) -> String {
    let local = (ms + clock::local_offset_ms()).div_euclid(1000);
    let (days, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    // Jour civil depuis 1970-01-01 (algorithme de H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// Une ligne par mise à jour des requêtes : un état réémis sans requête nouvelle garde son `seq` et n'est pas journalisé.
fn is_new(seen: &mut Option<u64>, status: &Value) -> bool {
    let seq = status["seq"].as_u64();
    seq != std::mem::replace(seen, seq)
}

/// Une ligne par lecture : requêtes (méthode:résultat:latence), synchro, panne.
fn poll_line(status: &Value) -> String {
    let requests = status["requests"].as_array().map_or(String::new(), |rs| {
        rs.iter()
            .map(|r| {
                let rtt = r["rttMs"].as_u64().map_or("-".into(), |v| format!("{v}ms"));
                format!("{}:{}:{rtt}", r["method"].as_str().unwrap_or("?"), r["outcome"].as_str().unwrap_or("?"))
            })
            .collect::<Vec<_>>()
            .join(" ")
    });
    let sync = &status["sync"];
    let mut line = format!(
        "POLL [{requests}] sync={} attendu={} mesuré={}",
        sync["state"].as_str().unwrap_or("?"),
        sync["expectedW"],
        sync["measuredW"]
    );
    // Le journal reste en français : `journalReason` double `reason` quand l'interface parle une autre langue.
    if let Some(reason) = sync["journalReason"].as_str().or(sync["reason"].as_str()) {
        line += &format!(" raison=« {reason} »");
    }
    if !status["down"].is_null() {
        line += &format!(" panne={}", status["down"]);
    }
    line
}

pub fn setup(app: &AppHandle, dir: &Path) {
    let sink = Arc::new(Mutex::new(Sink { path: dir.join("journal.log"), file: None, seen: None }));
    let s = Arc::clone(&sink);
    // `journal` : le même message que `log`, toujours en français (voir `i18n::log`).
    app.listen_any("journal", move |e| {
        let msg = serde_json::from_str::<String>(e.payload()).unwrap_or_else(|_| e.payload().to_owned());
        if let Ok(mut s) = s.lock() {
            s.write(&format!("LOG {msg}"));
        }
    });
    app.listen_any("status", move |e| {
        let Ok(v) = serde_json::from_str::<Value>(e.payload()) else { return };
        if let Ok(mut s) = sink.lock()
            && is_new(&mut s.seen, &v)
        {
            s.write(&poll_line(&v));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_status_without_new_requests_is_not_journaled_twice() {
        let (mut seen, status) = (None, |seq| json!({ "seq": seq }));
        assert!(is_new(&mut seen, &status(1)));
        assert!(!is_new(&mut seen, &status(1)));
        assert!(is_new(&mut seen, &status(2)));
    }

    #[test]
    fn the_journal_reason_is_french_even_when_the_interface_is_not() {
        let v = json!({ "sync": { "state": "bad", "reason": "mode expected Manual, read Auto", "journalReason": "mode attendu Manual, lu Auto" } });
        assert!(poll_line(&v).ends_with("raison=« mode attendu Manual, lu Auto »"));
    }

    #[test]
    fn poll_line_lists_each_request_and_the_sync_state() {
        let v = json!({
            "requests": [{ "method": "Bat.GetStatus", "rttMs": 120, "outcome": "ok" }, { "method": "EM.GetStatus", "outcome": "timeout" }],
            "sync": { "state": "warn", "expectedW": -800.0, "measuredW": -650.0, "reason": "écart" },
        });
        assert_eq!(
            poll_line(&v),
            "POLL [Bat.GetStatus:ok:120ms EM.GetStatus:timeout:-] sync=warn attendu=-800.0 mesuré=-650.0 raison=« écart »"
        );
    }
}
