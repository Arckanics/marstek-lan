//! Historique SQLite : moyenne par minute des puissances et dernière valeur horaire des compteurs.

use crate::clock::{DAY, HOUR, MINUTE, format_local, month_start};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS samples (
        ts INTEGER PRIMARY KEY, soc REAL, pv REAL, bat REAL, grid REAL, load REAL
    );
    CREATE TABLE IF NOT EXISTS counters (
        ts INTEGER PRIMARY KEY, pv REAL, grid_in REAL, grid_out REAL, load REAL
    );";

/// Puissances instantanées d'une lecture, en W (SOC en %).
#[derive(Debug, Default, Clone, Copy)]
pub struct Sample {
    pub soc: Option<f64>,
    pub pv: Option<f64>,
    pub bat: Option<f64>,
    pub grid: Option<f64>,
    pub load: Option<f64>,
}

/// Compteurs cumulés bruts, dans les unités de l'API.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counters {
    pub pv: Option<f64>,
    pub grid_in: Option<f64>,
    pub grid_out: Option<f64>,
    pub load: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub struct PowerSeries {
    pub ts: Vec<i64>,
    pub soc: Vec<Option<f64>>,
    pub pv: Vec<Option<f64>>,
    pub bat: Vec<Option<f64>>,
    pub grid: Vec<Option<f64>>,
    pub load: Vec<Option<f64>>,
}

/// Énergie d'une période, en Wh.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnergyRow {
    pub ts: i64,
    pub pv: Option<f64>,
    pub grid_in: Option<f64>,
    pub grid_out: Option<f64>,
    pub load: Option<f64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    Hour,
    Day,
    Month,
}

/// Début (UTC, ms) de la période locale qui contient `ts`.
// shortcut: décalage horaire figé à celui d'aujourd'hui ; un changement d'heure dans la plage décale d'1 h les jours concernés.
fn bucket_start(ts: i64, bucket: Bucket, offset: i64) -> i64 {
    let floor = |unit: i64| (ts + offset).div_euclid(unit) * unit - offset;
    match bucket {
        Bucket::Hour => floor(HOUR),
        Bucket::Day => floor(DAY),
        Bucket::Month => month_start(ts, offset),
    }
}

type Values = [Option<f64>; 4];

fn delta(last: Option<f64>, prev: Option<f64>) -> Option<f64> {
    // Un compteur qui recule a été remis à zéro : la période est inconnue.
    last.zip(prev).filter(|(l, p)| l >= p).map(|(l, p)| l - p)
}

fn energy(ts: i64, last: &Values, prev: Option<&Values>) -> EnergyRow {
    let d = |i: usize| prev.and_then(|p| delta(last[i], p[i]));
    EnergyRow { ts, pv: d(0), grid_in: d(1), grid_out: d(2), load: d(3) }
}

/// Moyenne en cours de la minute courante : somme et nombre de mesures par grandeur.
struct Minute {
    ts: i64,
    acc: [(f64, u32); 5],
}

pub struct History {
    db: Mutex<(Connection, Minute)>,
}

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::new(Connection::open(path)?)
    }

    pub fn memory() -> rusqlite::Result<Self> {
        Self::new(Connection::open_in_memory()?)
    }

    pub fn new(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { db: Mutex::new((conn, Minute { ts: i64::MIN, acc: [(0.0, 0); 5] })) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Connection, Minute)> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Cumule la lecture dans la moyenne de sa minute (réécrite à chaque lecture, rien n'est perdu à la fermeture)
    /// et garde la dernière valeur des compteurs de l'heure.
    pub fn record(&self, ts: i64, s: Sample, c: Counters) -> rusqlite::Result<()> {
        let mut guard = self.lock();
        let (conn, minute) = &mut *guard;
        let m = ts.div_euclid(MINUTE) * MINUTE;
        if minute.ts != m {
            *minute = Minute { ts: m, acc: [(0.0, 0); 5] };
        }
        for (slot, v) in minute.acc.iter_mut().zip([s.soc, s.pv, s.bat, s.grid, s.load]) {
            if let Some(v) = v {
                *slot = (slot.0 + v, slot.1 + 1);
            }
        }
        let avg = minute.acc.map(|(sum, n)| (n > 0).then(|| sum / f64::from(n)));
        conn.execute(
            "INSERT OR REPLACE INTO samples VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![m, avg[0], avg[1], avg[2], avg[3], avg[4]],
        )?;
        conn.execute(
            "INSERT INTO counters (ts, pv, grid_in, grid_out, load) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(ts) DO UPDATE SET
                pv = COALESCE(excluded.pv, pv), grid_in = COALESCE(excluded.grid_in, grid_in),
                grid_out = COALESCE(excluded.grid_out, grid_out), load = COALESCE(excluded.load, load)",
            params![ts.div_euclid(HOUR) * HOUR, c.pv, c.grid_in, c.grid_out, c.load],
        )?;
        Ok(())
    }

    // shortcut: aucune réduction de points ; un mois fait environ 43 000 minutes, à décimer si le graphe rame.
    pub fn power(&self, from: i64, to: i64) -> rusqlite::Result<PowerSeries> {
        let guard = self.lock();
        let mut stmt =
            guard.0.prepare("SELECT ts, soc, pv, bat, grid, load FROM samples WHERE ts BETWEEN ?1 AND ?2 ORDER BY ts")?;
        let mut out = PowerSeries::default();
        for row in stmt.query_map(params![from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })? {
            let (ts, soc, pv, bat, grid, load) = row?;
            out.ts.push(ts);
            out.soc.push(soc);
            out.pv.push(pv);
            out.bat.push(bat);
            out.grid.push(grid);
            out.load.push(load);
        }
        Ok(out)
    }

    /// Énergie par période = dernier compteur de la période moins dernier compteur de la précédente :
    /// un trou (PC éteint) se rattrape dans la première période suivante.
    pub fn energy(&self, from: i64, to: i64, bucket: Bucket, offset: i64) -> rusqlite::Result<Vec<EnergyRow>> {
        let guard = self.lock();
        let first = bucket_start(from, bucket, offset);
        let read = |r: &rusqlite::Row| Ok::<_, rusqlite::Error>((r.get::<_, i64>(0)?, [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?]));
        let mut prev: Option<Values> = guard
            .0
            .query_row(
                "SELECT ts, pv, grid_in, grid_out, load FROM counters WHERE ts < ?1 ORDER BY ts DESC LIMIT 1",
                [first],
                read,
            )
            .optional()?
            .map(|(_, v)| v);

        let mut stmt = guard
            .0
            .prepare("SELECT ts, pv, grid_in, grid_out, load FROM counters WHERE ts >= ?1 AND ts < ?2 ORDER BY ts")?;
        let mut out = Vec::new();
        let mut current: Option<(i64, Values)> = None;
        for row in stmt.query_map(params![first, to], read)? {
            let (ts, values) = row?;
            let b = bucket_start(ts, bucket, offset);
            match current {
                Some((cb, last)) if cb != b => {
                    out.push(energy(cb, &last, prev.as_ref()));
                    prev = Some(last);
                }
                _ => {}
            }
            current = Some((b, values));
        }
        if let Some((cb, last)) = current {
            out.push(energy(cb, &last, prev.as_ref()));
        }
        Ok(out)
    }
}

/// CSV pour un tableur français : BOM UTF-8, séparateur « ; », virgule décimale.
pub fn csv(series: &PowerSeries, offset: i64) -> String {
    let num = |v: Option<f64>| v.map_or_else(String::new, |v| format!("{v:.1}").replace('.', ","));
    let mut out = String::from("\u{feff}date;soc (%);solaire (W);batterie (W);réseau (W);maison (W)\n");
    for i in 0..series.ts.len() {
        out.push_str(&format!(
            "{};{};{};{};{};{}\n",
            format_local(series.ts[i], offset),
            num(series.soc[i]),
            num(series.pv[i]),
            num(series.bat[i]),
            num(series.grid[i]),
            num(series.load[i])
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::days_from_civil;

    fn db() -> History {
        History::memory().unwrap()
    }

    fn at(day: i64, hour: i64, min: i64) -> i64 {
        days_from_civil(2026, 11, day) * DAY + hour * HOUR + min * MINUTE
    }

    fn counters(pv: f64) -> Counters {
        Counters { pv: Some(pv), grid_in: Some(pv * 2.0), ..Counters::default() }
    }

    #[test]
    fn samples_are_averaged_per_minute() {
        let h = db();
        let s = |soc, bat| Sample { soc: Some(soc), bat: Some(bat), ..Sample::default() };
        h.record(at(10, 8, 0) + 5_000, s(50.0, 100.0), Counters::default()).unwrap();
        h.record(at(10, 8, 0) + 35_000, s(52.0, 300.0), Counters::default()).unwrap();
        h.record(at(10, 8, 1) + 5_000, s(60.0, 0.0), Counters::default()).unwrap();
        let p = h.power(at(10, 0, 0), at(11, 0, 0)).unwrap();
        assert_eq!(p.ts, [at(10, 8, 0), at(10, 8, 1)]);
        assert_eq!(p.soc, [Some(51.0), Some(60.0)]);
        assert_eq!(p.bat, [Some(200.0), Some(0.0)]);
        assert_eq!(p.pv, [None, None]);
        assert!(h.power(at(12, 0, 0), at(13, 0, 0)).unwrap().ts.is_empty());
    }

    #[test]
    fn hourly_energy_is_a_counter_difference_across_gaps() {
        let h = db();
        for (hour, pv) in [(8, 1000.0), (9, 1400.0), (12, 2000.0)] {
            h.record(at(10, hour, 30), Sample::default(), counters(pv)).unwrap();
            h.record(at(10, hour, 50), Sample::default(), counters(pv + 50.0)).unwrap();
        }
        let rows = h.energy(at(10, 8, 0), at(10, 13, 0), Bucket::Hour, 0).unwrap();
        let pv: Vec<_> = rows.iter().map(|r| (r.ts, r.pv)).collect();
        // 8 h : pas de relevé précédent ; 9 h : +400 ; 12 h : le trou de 10 h et 11 h se rattrape ici.
        assert_eq!(pv, [(at(10, 8, 0), None), (at(10, 9, 0), Some(400.0)), (at(10, 12, 0), Some(600.0))]);
        assert_eq!(rows[1].grid_in, Some(800.0));
    }

    #[test]
    fn first_bucket_uses_the_reading_before_the_range() {
        let h = db();
        h.record(at(9, 23, 30), Sample::default(), counters(1000.0)).unwrap();
        h.record(at(10, 6, 0), Sample::default(), counters(1300.0)).unwrap();
        h.record(at(10, 22, 0), Sample::default(), counters(1900.0)).unwrap();
        let rows = h.energy(at(10, 0, 0), at(11, 0, 0), Bucket::Day, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].ts, rows[0].pv), (at(10, 0, 0), Some(900.0)));
    }

    #[test]
    fn day_buckets_follow_the_local_offset_and_months_group_days() {
        let h = db();
        h.record(at(30, 22, 30), Sample::default(), counters(100.0)).unwrap();
        h.record(at(30, 23, 30), Sample::default(), counters(150.0)).unwrap(); // 1er déc. 00:30 locale (UTC+1)
        h.record(at(30, 23, 50), Sample::default(), counters(170.0)).unwrap();
        let rows = h.energy(at(30, 0, 0), at(31, 0, 0), Bucket::Day, HOUR).unwrap();
        let starts: Vec<_> = rows.iter().map(|r| r.ts).collect();
        assert_eq!(starts, [at(29, 23, 0), at(30, 23, 0)]);
        let months = h.energy(at(1, 0, 0), at(30, 23, 59), Bucket::Month, 0).unwrap();
        assert_eq!((months.len(), months[0].ts), (1, at(1, 0, 0)));
    }

    #[test]
    fn counter_reset_gives_unknown_energy() {
        let h = db();
        h.record(at(10, 8, 0), Sample::default(), counters(5000.0)).unwrap();
        h.record(at(10, 9, 0), Sample::default(), counters(10.0)).unwrap();
        let rows = h.energy(at(10, 8, 0), at(10, 10, 0), Bucket::Hour, 0).unwrap();
        assert_eq!(rows[1].pv, None);
    }

    #[test]
    fn database_with_legacy_meter_columns_still_records() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE counters (
                ts INTEGER PRIMARY KEY, pv REAL, grid_in REAL, grid_out REAL, load REAL, em_in REAL, em_out REAL
            );",
        )
        .unwrap();
        let h = History::new(conn).unwrap();
        h.record(at(10, 8, 0), Sample::default(), counters(100.0)).unwrap();
        h.record(at(10, 9, 0), Sample::default(), counters(150.0)).unwrap();
        let rows = h.energy(at(10, 8, 0), at(10, 10, 0), Bucket::Hour, 0).unwrap();
        assert_eq!(rows[1].pv, Some(50.0));
    }

    #[test]
    fn missing_counters_do_not_erase_the_stored_value() {
        let h = db();
        h.record(at(10, 8, 0), Sample::default(), counters(100.0)).unwrap();
        h.record(at(10, 8, 30), Sample::default(), Counters { grid_in: Some(7.0), ..Counters::default() }).unwrap();
        h.record(at(10, 9, 0), Sample::default(), counters(150.0)).unwrap();
        let rows = h.energy(at(10, 8, 0), at(10, 10, 0), Bucket::Hour, 0).unwrap();
        assert_eq!(rows[1].pv, Some(50.0));
    }

    #[test]
    fn csv_uses_semicolons_decimal_commas_and_local_time() {
        let series = PowerSeries {
            ts: vec![at(10, 8, 0)],
            soc: vec![Some(76.0)],
            pv: vec![None],
            bat: vec![Some(-420.5)],
            grid: vec![Some(0.0)],
            load: vec![None],
        };
        let out = csv(&series, HOUR);
        assert!(out.starts_with("\u{feff}date;soc"));
        assert!(out.ends_with("2026-11-10 09:00;76,0;;-420,5;0,0;\n"), "{out}");
    }
}
