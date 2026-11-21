//! Historique SQLite : moyenne par minute des puissances et dernière valeur horaire des compteurs.

use crate::clock::{DAY, HOUR, MINUTE, format_local, month_start};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS samples (
        ts INTEGER PRIMARY KEY, soc REAL, pv REAL, bat REAL, grid REAL, load REAL, src INTEGER NOT NULL DEFAULT 0
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
    /// Pour chaque point, vrai si la mesure vient du cloud (coupure locale) : courbe estompée, exclu des économies.
    pub cloud: Vec<bool>,
}

/// Un créneau tarifaire : prix de l'énergie pendant `[start, end)` en heure locale (peut passer minuit).
#[derive(Debug, Clone, Deserialize)]
pub struct PriceSlot {
    pub start: String,
    pub end: String,
    /// Prix par kWh dans la monnaie de l'utilisateur.
    pub price: f64,
}

/// Tarif par créneaux pour le calcul d'économies.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub slots: Vec<PriceSlot>,
    /// Prix hors de tout créneau, par kWh.
    pub default: f64,
    /// Prix de revente (export), par kWh ; absent : l'export n'est pas valorisé (cas « sans revente »).
    pub feed_in: Option<f64>,
}

fn hhmm(s: &str) -> Option<i64> {
    let (h, m) = s.split_once(':')?;
    Some(h.trim().parse::<i64>().ok()? * 60 + m.trim().parse::<i64>().ok()?)
}

impl Price {
    /// Prix au créneau qui contient `minute` (minute de la journée locale), sinon le prix par défaut.
    fn at(&self, minute: i64) -> f64 {
        for s in &self.slots {
            if let (Some(a), Some(b)) = (hhmm(&s.start), hhmm(&s.end)) {
                let inside = if a <= b { minute >= a && minute < b } else { minute >= a || minute < b };
                if inside {
                    return s.price;
                }
            }
        }
        self.default
    }
}

/// Économies sur une période : coût sans système, coût réel, différence, et énergies (kWh).
#[derive(Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Savings {
    /// Coût sans solaire ni batterie : toute la consommation importée au prix du créneau.
    pub baseline: f64,
    /// Coût réel : import payé moins revente de l'export.
    pub cost: f64,
    /// Économie = `baseline - cost`.
    pub saved: f64,
    pub imported: f64,
    pub exported: f64,
    /// Consommation couverte par le solaire et la batterie plutôt que par le réseau.
    pub self_used: f64,
}

/// Un trou plus long que cela entre deux mesures n'est pas intégré (pas de données pour l'estimer).
const SAVINGS_MAX_GAP: i64 = 5 * MINUTE;

/// Intègre les économies sur des mesures `(ts, réseau W, maison W)` triées (réseau > 0 = soutirage).
fn compute_savings(rows: &[(i64, Option<f64>, Option<f64>)], price: &Price, offset: i64) -> Savings {
    let mut s = Savings::default();
    for w in rows.windows(2) {
        let (t0, grid, load) = w[0];
        let dt = (w[1].0 - t0).min(SAVINGS_MAX_GAP);
        if dt <= 0 {
            continue;
        }
        let hours = dt as f64 / 3_600_000.0;
        let p = price.at((t0 + offset).rem_euclid(DAY) / MINUTE);
        let load_kwh = load.unwrap_or(0.0).max(0.0) / 1000.0 * hours;
        let grid_w = grid.unwrap_or(0.0);
        let import_kwh = grid_w.max(0.0) / 1000.0 * hours;
        let export_kwh = (-grid_w).max(0.0) / 1000.0 * hours;
        s.baseline += load_kwh * p;
        s.cost += import_kwh * p - price.feed_in.unwrap_or(0.0) * export_kwh;
        s.imported += import_kwh;
        s.exported += export_kwh;
        s.self_used += (load_kwh - import_kwh).max(0.0);
    }
    s.saved = s.baseline - s.cost;
    s
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

/// SOC agrégé : min, max et moyenne des échantillons d'une période (`ts` : début du jour local) ou d'une heure du jour.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SocStat {
    pub ts: i64,
    pub hour: i64,
    pub weekend: bool,
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    pub n: i64,
}

/// Grandeur d'une moyenne journalière : liste fermée, seul `name` finit dans le SQL.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    Soc,
    Pv,
    Bat,
    Grid,
    Load,
}

impl Column {
    fn name(self) -> &'static str {
        match self {
            Self::Soc => "soc",
            Self::Pv => "pv",
            Self::Bat => "bat",
            Self::Grid => "grid",
            Self::Load => "load",
        }
    }
}

/// Journée moyenne : valeur moyenne par tranche de l'heure locale (`None` sans mesure) et nombre de jours distincts couverts.
#[derive(Debug, PartialEq, Serialize)]
pub struct AverageDay {
    pub days: i64,
    pub avg: Vec<Option<f64>>,
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

    /// Ouvre la base sur disque, ou récupère : si le fichier existant est illisible (corrompu), il est écarté en
    /// `.corrupt` et on repart sur un fichier neuf **persistant**. La mémoire (historique perdu à la fermeture) n'est
    /// retenue qu'en dernier recours, si le disque est inécrivable — jamais un simple fichier illisible.
    pub fn open_or_recover(path: &Path) -> Self {
        if let Ok(h) = Self::open(path) {
            return h;
        }
        if path.exists() {
            let _ = std::fs::rename(path, path.with_extension("corrupt"));
        }
        Self::open(path).or_else(|_| Self::memory()).expect("base SQLite en mémoire")
    }

    pub fn new(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        // Migration des bases d'avant le marqueur de source : ajoute `src` (erreur « colonne déjà là » ignorée).
        let _ = conn.execute("ALTER TABLE samples ADD COLUMN src INTEGER NOT NULL DEFAULT 0", []);
        Ok(Self { db: Mutex::new((conn, Minute { ts: i64::MIN, acc: [(0.0, 0); 5] })) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Connection, Minute)> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Cumule la lecture dans la moyenne de sa minute (réécrite à chaque lecture, rien n'est perdu à la fermeture)
    /// et garde la dernière valeur des compteurs de l'heure.
    pub fn record(&self, ts: i64, s: Sample, c: Counters, cloud: bool) -> rusqlite::Result<()> {
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
            "INSERT OR REPLACE INTO samples (ts, soc, pv, bat, grid, load, src) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![m, avg[0], avg[1], avg[2], avg[3], avg[4], i64::from(cloud)],
        )?;
        // Le cloud n'a pas de compteurs : on ne touche pas la table `counters` (pas de ligne d'heure vide).
        if !cloud {
            conn.execute(
                "INSERT INTO counters (ts, pv, grid_in, grid_out, load) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(ts) DO UPDATE SET
                    pv = COALESCE(excluded.pv, pv), grid_in = COALESCE(excluded.grid_in, grid_in),
                    grid_out = COALESCE(excluded.grid_out, grid_out), load = COALESCE(excluded.load, load)",
                params![ts.div_euclid(HOUR) * HOUR, c.pv, c.grid_in, c.grid_out, c.load],
            )?;
        }
        Ok(())
    }

    // shortcut: aucune réduction de points ; un mois fait environ 43 000 minutes, à décimer si le graphe rame.
    pub fn power(&self, from: i64, to: i64) -> rusqlite::Result<PowerSeries> {
        let guard = self.lock();
        let mut stmt =
            guard.0.prepare("SELECT ts, soc, pv, bat, grid, load, src FROM samples WHERE ts BETWEEN ?1 AND ?2 ORDER BY ts")?;
        let mut out = PowerSeries::default();
        for row in stmt.query_map(params![from, to], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get::<_, i64>(6)?))
        })? {
            let (ts, soc, pv, bat, grid, load, src) = row?;
            out.ts.push(ts);
            out.soc.push(soc);
            out.pv.push(pv);
            out.bat.push(bat);
            out.grid.push(grid);
            out.load.push(load);
            out.cloud.push(src != 0);
        }
        Ok(out)
    }

    /// Économies sur `[from, to]` d'après le tarif par créneaux. N'utilise que les mesures locales (les points cloud,
    /// grossiers, sont exclus) ; `offset` place les créneaux à l'heure locale.
    pub fn savings(&self, from: i64, to: i64, price: &Price, offset: i64) -> rusqlite::Result<Savings> {
        let guard = self.lock();
        let mut stmt = guard.0.prepare("SELECT ts, grid, load FROM samples WHERE ts BETWEEN ?1 AND ?2 AND src = 0 ORDER BY ts")?;
        let rows: Vec<(i64, Option<f64>, Option<f64>)> =
            stmt.query_map(params![from, to], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(compute_savings(&rows, price, offset))
    }

    /// SOC par jour local (`hour` et `weekend` sans objet) ; agrégé en SQL : un an fait environ 525 000 minutes.
    // shortcut: décalage horaire figé, comme `bucket_start`.
    pub fn soc_daily(&self, from: i64, to: i64, offset: i64) -> rusqlite::Result<Vec<SocStat>> {
        self.soc_stats(
            "(ts + ?3) / ?4 * ?4 - ?3, 0, 0",
            "(ts + ?3) / ?4",
            from,
            to,
            offset,
        )
    }

    /// SOC par heure locale du jour, jours ouvrés et week-ends séparés (`ts` sans objet).
    pub fn soc_profile(&self, from: i64, to: i64, offset: i64) -> rusqlite::Result<Vec<SocStat>> {
        // 1er janvier 1970 = jeudi : (jour + 3) % 7 vaut 5 ou 6 le samedi et le dimanche.
        self.soc_stats(
            "0, (ts + ?3) / ?5 % 24, ((ts + ?3) / ?4 + 3) % 7 >= 5",
            "(ts + ?3) / ?5 % 24, ((ts + ?3) / ?4 + 3) % 7 >= 5",
            from,
            to,
            offset,
        )
    }

    fn soc_stats(&self, cols: &str, group: &str, from: i64, to: i64, offset: i64) -> rusqlite::Result<Vec<SocStat>> {
        let guard = self.lock();
        let sql = format!(
            "SELECT {cols}, MIN(soc), MAX(soc), AVG(soc), COUNT(soc) FROM samples
             WHERE ts BETWEEN ?1 AND ?2 AND soc IS NOT NULL GROUP BY {group} ORDER BY 1, 2, 3"
        )
        .replace("?5", &HOUR.to_string());
        let mut stmt = guard.0.prepare(&sql)?;
        let rows = stmt.query_map(params![from, to, offset, DAY], |r| {
            Ok(SocStat { ts: r.get(0)?, hour: r.get(1)?, weekend: r.get(2)?, min: r.get(3)?, max: r.get(4)?, avg: r.get(5)?, n: r.get(6)? })
        })?;
        rows.collect()
    }

    /// Moyenne par tranche de `step_min` minutes de l'heure locale sur les `days` jours qui précèdent aujourd'hui
    /// (aujourd'hui exclu) ; `days` = 0 : aujourd'hui jusqu'à `now`. `weekend` filtre les jours de semaine ou de week-end.
    // shortcut: décalage horaire figé, comme `bucket_start`.
    pub fn average_day(&self, col: Column, days: i64, weekend: Option<bool>, step_min: i64, now: i64, offset: i64) -> rusqlite::Result<AverageDay> {
        let today = bucket_start(now, Bucket::Day, offset);
        let (from, to) = if days == 0 { (today, now + 1) } else { (today - days * DAY, today) };
        let (step, wk) = (step_min * MINUTE, weekend.map_or(-1, i64::from));
        let c = col.name();
        // 1er janvier 1970 = jeudi : (jour + 3) % 7 vaut 5 ou 6 le samedi et le dimanche.
        let filter = format!(
            "FROM samples WHERE ts >= ?1 AND ts < ?2 AND {c} IS NOT NULL AND (?6 < 0 OR (((ts + ?3) / ?4 + 3) % 7 >= 5) = ?6)"
        );
        let guard = self.lock();
        let mut out = vec![None; usize::try_from(DAY / step).unwrap_or(0)];
        let mut stmt = guard.0.prepare(&format!("SELECT (ts + ?3) % ?4 / ?5, AVG({c}) {filter} GROUP BY 1"))?;
        for row in stmt.query_map(params![from, to, offset, DAY, step, wk], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))? {
            let (slot, v) = row?;
            out[slot as usize] = Some(v);
        }
        let days = guard.0.query_row(
            &format!("SELECT COUNT(DISTINCT (ts + ?3) / ?4) {filter}"),
            params![from, to, offset, DAY, step, wk],
            |r| r.get(0),
        )?;
        Ok(AverageDay { days, avg: out })
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
    fn open_or_recover_salvages_a_corrupt_file_and_persists_to_disk() {
        let path = std::env::temp_dir().join(format!("marstek-recover-{}.db", std::process::id()));
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(path.with_extension("corrupt")).ok();
        std::fs::write(&path, b"garbage, not a sqlite database").unwrap();
        let ts = at(10, 8, 0);
        {
            let h = History::open_or_recover(&path);
            h.record(ts, Sample { soc: Some(50.0), ..Sample::default() }, Counters::default(), false).unwrap();
        }
        assert!(path.with_extension("corrupt").exists(), "le fichier illisible est écarté, pas gardé");
        // Rouvrir le même chemin : l'écriture a atterri sur disque (sinon ce serait une base mémoire, perdue).
        let again = History::open(&path).unwrap();
        assert_eq!(again.power(ts, ts + HOUR).unwrap().soc, vec![Some(50.0)]);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(path.with_extension("corrupt")).ok();
    }

    #[test]
    fn samples_are_averaged_per_minute() {
        let h = db();
        let s = |soc, bat| Sample { soc: Some(soc), bat: Some(bat), ..Sample::default() };
        h.record(at(10, 8, 0) + 5_000, s(50.0, 100.0), Counters::default(), false).unwrap();
        h.record(at(10, 8, 0) + 35_000, s(52.0, 300.0), Counters::default(), false).unwrap();
        h.record(at(10, 8, 1) + 5_000, s(60.0, 0.0), Counters::default(), false).unwrap();
        let p = h.power(at(10, 0, 0), at(11, 0, 0)).unwrap();
        assert_eq!(p.ts, [at(10, 8, 0), at(10, 8, 1)]);
        assert_eq!(p.soc, [Some(51.0), Some(60.0)]);
        assert_eq!(p.bat, [Some(200.0), Some(0.0)]);
        assert_eq!(p.pv, [None, None]);
        assert!(h.power(at(12, 0, 0), at(13, 0, 0)).unwrap().ts.is_empty());
    }

    #[test]
    fn cloud_points_are_marked_in_the_power_series() {
        let h = db();
        let s = |soc| Sample { soc: Some(soc), ..Sample::default() };
        h.record(at(10, 8, 0) + 5_000, s(50.0), Counters::default(), false).unwrap();
        h.record(at(10, 8, 2) + 5_000, s(60.0), Counters::default(), true).unwrap();
        assert_eq!(h.power(at(10, 8, 0), at(10, 9, 0)).unwrap().cloud, [false, true]);
    }

    #[test]
    fn savings_price_by_slot_and_avoided_import_plus_export() {
        let price = Price {
            slots: vec![PriceSlot { start: "08:00".into(), end: "20:00".into(), price: 0.30 }],
            default: 0.10,
            feed_in: Some(0.15),
        };
        assert_eq!((price.at(9 * 60), price.at(2 * 60)), (0.30, 0.10)); // dans le créneau jour / hors créneau
        // Deux mesures à 1 min (sous le plafond anti-trou) à 10h : maison 1 kW couverte par la batterie, et export 0,5 kW.
        let t = at(10, 10, 0);
        let rows = vec![(t, Some(-500.0), Some(1000.0)), (t + MINUTE, Some(0.0), Some(0.0))];
        let s = compute_savings(&rows, &price, 0);
        let h = 1.0 / 60.0; // 1 min en heures
        assert!((s.baseline - 1.0 * h * 0.30).abs() < 1e-9, "{}", s.baseline); // conso 1 kWh·h évitée au prix jour
        assert!((s.cost + 0.15 * 0.5 * h).abs() < 1e-9, "{}", s.cost); // import 0, revente 0,5 kW
        assert!((s.saved - (1.0 * h * 0.30 + 0.15 * 0.5 * h)).abs() < 1e-9, "{}", s.saved);
        assert!((s.exported - 0.5 * h).abs() < 1e-9 && s.imported == 0.0);
        // Un trou plus long que 5 min n'est pas intégré au-delà du plafond.
        let gap = compute_savings(&[(t, Some(1000.0), Some(1000.0)), (t + HOUR, Some(0.0), Some(0.0))], &price, 0);
        assert!((gap.imported - 1.0 * (5.0 / 60.0)).abs() < 1e-9, "{}", gap.imported);
    }

    #[test]
    fn hourly_energy_is_a_counter_difference_across_gaps() {
        let h = db();
        for (hour, pv) in [(8, 1000.0), (9, 1400.0), (12, 2000.0)] {
            h.record(at(10, hour, 30), Sample::default(), counters(pv), false).unwrap();
            h.record(at(10, hour, 50), Sample::default(), counters(pv + 50.0), false).unwrap();
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
        h.record(at(9, 23, 30), Sample::default(), counters(1000.0), false).unwrap();
        h.record(at(10, 6, 0), Sample::default(), counters(1300.0), false).unwrap();
        h.record(at(10, 22, 0), Sample::default(), counters(1900.0), false).unwrap();
        let rows = h.energy(at(10, 0, 0), at(11, 0, 0), Bucket::Day, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].ts, rows[0].pv), (at(10, 0, 0), Some(900.0)));
    }

    #[test]
    fn day_buckets_follow_the_local_offset_and_months_group_days() {
        let h = db();
        h.record(at(30, 22, 30), Sample::default(), counters(100.0), false).unwrap();
        h.record(at(30, 23, 30), Sample::default(), counters(150.0), false).unwrap(); // 1er déc. 00:30 locale (UTC+1)
        h.record(at(30, 23, 50), Sample::default(), counters(170.0), false).unwrap();
        let rows = h.energy(at(30, 0, 0), at(31, 0, 0), Bucket::Day, HOUR).unwrap();
        let starts: Vec<_> = rows.iter().map(|r| r.ts).collect();
        assert_eq!(starts, [at(29, 23, 0), at(30, 23, 0)]);
        let months = h.energy(at(1, 0, 0), at(30, 23, 59), Bucket::Month, 0).unwrap();
        assert_eq!((months.len(), months[0].ts), (1, at(1, 0, 0)));
    }

    #[test]
    fn soc_is_aggregated_per_local_day_and_per_hour() {
        let h = db();
        let soc = |v| Sample { soc: Some(v), ..Sample::default() };
        // 2026-11-14 est un samedi, le 16 un lundi.
        for (day, hour, min, v) in [(14, 8, 0, 40.0), (14, 8, 1, 60.0), (14, 9, 0, 80.0), (16, 8, 0, 20.0)] {
            h.record(at(day, hour, min), soc(v), Counters::default(), false).unwrap();
        }
        let days = h.soc_daily(at(14, 0, 0), at(17, 0, 0), 0).unwrap();
        let d: Vec<_> = days.iter().map(|r| (r.ts, r.min, r.max, r.avg, r.n)).collect();
        assert_eq!(d, [(at(14, 0, 0), 40.0, 80.0, 60.0, 3), (at(16, 0, 0), 20.0, 20.0, 20.0, 1)]);
        let p: Vec<_> = h.soc_profile(at(14, 0, 0), at(17, 0, 0), 0).unwrap().iter().map(|r| (r.hour, r.weekend, r.avg)).collect();
        assert_eq!(p, [(8, false, 20.0), (8, true, 50.0), (9, true, 80.0)]);
        // UTC+1 : 23:30 UTC le 14 est le 15 à 00:30 local.
        h.record(at(14, 23, 30), soc(10.0), Counters::default(), false).unwrap();
        let local = h.soc_daily(at(14, 22, 0), at(15, 2, 0), HOUR).unwrap();
        assert_eq!(local.iter().map(|r| r.ts).collect::<Vec<_>>(), [at(14, 23, 0)]);
    }

    #[test]
    fn average_day_follows_local_time_and_excludes_today() {
        let h = db();
        let pv = |v| Sample { pv: Some(v), soc: Some(50.0), ..Sample::default() };
        // Lundi 16 et mardi 17 : 8 h UTC = 9 h locale (UTC+1) ; samedi 14 : week-end ; mercredi 18 : jour courant.
        for (day, hour, v) in [(16, 8, 100.0), (17, 8, 300.0), (14, 8, 700.0)] {
            h.record(at(day, hour, 10), pv(v), Counters::default(), false).unwrap();
        }
        h.record(at(18, 8, 10), pv(9999.0), Counters::default(), false).unwrap();
        let now = at(18, 12, 0);
        let a = h.average_day(Column::Pv, 7, None, 60, now, HOUR).unwrap();
        assert_eq!((a.days, a.avg.len(), a.avg[8], a.avg[10]), (3, 24, None, None));
        assert!((a.avg[9].unwrap() - 1100.0 / 3.0).abs() < 1e-9);
        let week = h.average_day(Column::Pv, 7, Some(false), 60, now, HOUR).unwrap();
        assert_eq!((week.days, week.avg[9]), (2, Some(200.0)));
        let end = h.average_day(Column::Pv, 7, Some(true), 15, now, HOUR).unwrap();
        assert_eq!((end.days, end.avg.len(), end.avg[36]), (1, 96, Some(700.0)));
        let today = h.average_day(Column::Pv, 0, None, 60, now, HOUR).unwrap();
        assert_eq!((today.days, today.avg[9]), (1, Some(9999.0)));
        assert_eq!(h.average_day(Column::Bat, 7, None, 60, now, HOUR).unwrap().days, 0);
    }

    #[test]
    fn average_column_must_be_in_the_closed_list() {
        assert!(matches!(serde_json::from_str("\"grid\""), Ok(Column::Grid)));
        assert!(serde_json::from_str::<Column>("\"ts; DROP TABLE samples\"").is_err());
    }

    #[test]
    fn counter_reset_gives_unknown_energy() {
        let h = db();
        h.record(at(10, 8, 0), Sample::default(), counters(5000.0), false).unwrap();
        h.record(at(10, 9, 0), Sample::default(), counters(10.0), false).unwrap();
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
        h.record(at(10, 8, 0), Sample::default(), counters(100.0), false).unwrap();
        h.record(at(10, 9, 0), Sample::default(), counters(150.0), false).unwrap();
        let rows = h.energy(at(10, 8, 0), at(10, 10, 0), Bucket::Hour, 0).unwrap();
        assert_eq!(rows[1].pv, Some(50.0));
    }

    #[test]
    fn missing_counters_do_not_erase_the_stored_value() {
        let h = db();
        h.record(at(10, 8, 0), Sample::default(), counters(100.0), false).unwrap();
        h.record(at(10, 8, 30), Sample::default(), Counters { grid_in: Some(7.0), ..Counters::default() }, false).unwrap();
        h.record(at(10, 9, 0), Sample::default(), counters(150.0), false).unwrap();
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
            cloud: vec![false],
        };
        let out = csv(&series, HOUR);
        assert!(out.starts_with("\u{feff}date;soc"));
        assert!(out.ends_with("2026-11-10 09:00;76,0;;-420,5;0,0;\n"), "{out}");
    }
}
