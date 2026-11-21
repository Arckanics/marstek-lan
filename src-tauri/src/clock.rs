//! Heure locale sans dépendance : Windows donne l'heure locale, le reste est du calcul de calendrier.

use std::time::{SystemTime, UNIX_EPOCH};

pub const MINUTE: i64 = 60_000;
pub const HOUR: i64 = 60 * MINUTE;
pub const DAY: i64 = 24 * HOUR;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Jours depuis 1970-01-01 pour une date du calendrier grégorien.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (année, mois, jour) pour un nombre de jours depuis 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Décalage UTC → heure locale, en ms (heure d'été comprise, au moment de l'appel).
#[cfg(windows)]
pub fn local_offset_ms() -> i64 {
    #[repr(C)]
    #[derive(Default)]
    struct LocalTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(t: *mut LocalTime);
    }
    let mut t = LocalTime::default();
    // SAFETY: pointeur valide vers une structure de même disposition que SYSTEMTIME.
    unsafe { GetLocalTime(&mut t) };
    let local = days_from_civil(i64::from(t.year), i64::from(t.month), i64::from(t.day)) * DAY
        + i64::from(t.hour) * HOUR
        + i64::from(t.minute) * MINUTE
        + i64::from(t.second) * 1000;
    let utc = now_ms() / 1000 * 1000;
    // Arrondi à la minute : les deux lectures d'horloge diffèrent de quelques ms.
    ((local - utc) as f64 / MINUTE as f64).round() as i64 * MINUTE
}

// shortcut: hors Windows l'heure locale est supposée UTC ; l'application ne cible que Windows.
#[cfg(not(windows))]
pub fn local_offset_ms() -> i64 {
    0
}

/// Minute de la semaine locale, 0 = lundi 00:00 (le 1er janvier 1970 était un jeudi).
pub fn week_minute(utc_ms: i64, offset_ms: i64) -> u32 {
    let local = utc_ms + offset_ms;
    let day = (local.div_euclid(DAY) + 3).rem_euclid(7);
    (day * 1440 + local.rem_euclid(DAY) / MINUTE) as u32
}

/// Début (UTC, ms) du mois local qui contient `utc_ms`.
pub fn month_start(utc_ms: i64, offset_ms: i64) -> i64 {
    let (y, m, _) = civil_from_days((utc_ms + offset_ms).div_euclid(DAY));
    days_from_civil(y, m, 1) * DAY - offset_ms
}

/// "AAAA-MM-JJ hh:mm" en heure locale.
pub fn format_local(utc_ms: i64, offset_ms: i64) -> String {
    let local = utc_ms + offset_ms;
    let (y, m, d) = civil_from_days(local.div_euclid(DAY));
    let min = local.rem_euclid(DAY) / MINUTE;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", min / 60, min % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for days in [-1, 0, 59, 11_016, 19_782, 20_400] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn week_minute_starts_on_monday() {
        let monday = days_from_civil(2026, 11, 16) * DAY; // lundi 00:00 UTC
        assert_eq!(week_minute(monday, 0), 0);
        assert_eq!(week_minute(monday + 90 * MINUTE, 0), 90);
        assert_eq!(week_minute(monday + 6 * DAY + 23 * HOUR, 0), 6 * 1440 + 23 * 60);
        // 23:30 UTC dimanche avec +1 h : lundi 00:30 locale.
        assert_eq!(week_minute(monday - 30 * MINUTE, HOUR), 30);
    }

    #[test]
    fn month_start_and_format_follow_the_offset() {
        let t = days_from_civil(2026, 11, 30) * DAY + 23 * HOUR + 30 * MINUTE; // 30 nov. 23:30 UTC
        assert_eq!(format_local(t, HOUR), "2026-12-01 00:30");
        assert_eq!(month_start(t, HOUR), days_from_civil(2026, 12, 1) * DAY - HOUR);
        assert_eq!(month_start(t, 0), days_from_civil(2026, 11, 1) * DAY);
    }

    #[test]
    fn local_offset_is_a_quarter_hour_multiple() {
        let off = local_offset_ms();
        assert!(off.abs() <= 14 * HOUR && off % (15 * MINUTE) == 0, "{off}");
    }
}
