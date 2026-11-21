//! Planning hebdomadaire : blocs → créneaux Manuel, et tarif heures creuses → blocs.

use crate::api::{ApiError, Direction, SLOTS, Slot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DAY: u32 = 24 * 60;
const DAY_NAMES: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];

#[derive(Debug, Deserialize)]
pub struct Range {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tariff {
    pub offpeak: Vec<Range>,
    pub charge_power: u16,
    pub discharge_power: u16,
    pub days: u8,
}

/// Plage de la grille hebdomadaire ; `end` peut valoir "24:00" (fin de journée).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// 0 = lundi.
    pub day: u8,
    pub start: String,
    pub end: String,
    pub direction: Direction,
    pub power: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub name: String,
    pub blocks: Vec<Block>,
}

/// Minutes depuis minuit ; "24:00" n'est accepté que pour une fin de bloc.
pub fn minutes(s: &str, allow_24: bool) -> Result<u32, ApiError> {
    if allow_24 && s == "24:00" {
        return Ok(DAY);
    }
    let parsed = s.split_once(':').and_then(|(h, m)| Some((h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)));
    match parsed {
        Some((h, m)) if h < 24 && m < 60 => Ok(h * 60 + m),
        _ => Err(ApiError::Invalid(format!("heure invalide : « {s} » (attendu hh:mm)"))),
    }
}

pub fn clock(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// La batterie n'accepte pas "24:00" : la fin de journée s'écrit 23:59.
fn hhmm(m: u32) -> String {
    clock(m.min(DAY - 1))
}

type Segment = (u32, u32, Direction, u16);

/// Fusionne les blocs contigus de même réglage par jour, puis regroupe les créneaux identiques
/// de plusieurs jours en un seul (`week_set`). Plus de 10 créneaux : erreur avec le nombre.
pub fn compile_plan(blocks: &[Block]) -> Result<Vec<Slot>, ApiError> {
    let mut days: [Vec<Segment>; 7] = Default::default();
    for b in blocks {
        let (start, end) = (minutes(&b.start, false)?, minutes(&b.end, true)?);
        if end <= start {
            return Err(ApiError::Invalid(format!("bloc invalide : {} doit finir après {}", b.end, b.start)));
        }
        days.get_mut(usize::from(b.day))
            .ok_or_else(|| ApiError::Invalid(format!("jour invalide : {}", b.day)))?
            .push((start, end, b.direction, b.power));
    }

    let mut groups: BTreeMap<(u32, u32, bool, u16), u8> = BTreeMap::new();
    for (day, list) in days.iter_mut().enumerate() {
        list.sort_unstable_by_key(|b| (b.0, b.1));
        let mut merged: Vec<Segment> = Vec::new();
        for &b in list.iter() {
            match merged.last_mut() {
                Some(last) if b.0 < last.1 => {
                    return Err(ApiError::Invalid(format!("des blocs se chevauchent le {}", DAY_NAMES[day])));
                }
                Some(last) if b.0 == last.1 && b.2 == last.2 && b.3 == last.3 => last.1 = b.1,
                _ => merged.push(b),
            }
        }
        for (start, end, direction, power) in merged {
            *groups.entry((start, end, direction == Direction::Discharge, power)).or_default() |= 1 << day;
        }
    }
    if groups.len() > SLOTS {
        return Err(ApiError::Invalid(format!(
            "{} créneaux nécessaires, la batterie n'en accepte que {SLOTS} : simplifie le planning",
            groups.len()
        )));
    }
    Ok(groups
        .into_iter()
        .map(|((start, end, discharge, power), days)| Slot {
            start: hhmm(start),
            end: hhmm(end),
            days,
            direction: if discharge { Direction::Discharge } else { Direction::Charge },
            power,
            enabled: true,
        })
        .collect())
}

/// Charge sur chaque plage creuse, décharge sur le reste de la journée.
fn tariff_segments(t: &Tariff) -> Result<Vec<Segment>, ApiError> {
    let mut charge: Vec<(u32, u32)> = Vec::new();
    for r in &t.offpeak {
        let start = minutes(&r.start, false)?;
        // Une fin à 23:59 vaut minuit, sinon le complément laisserait une minute de décharge.
        let end = match minutes(&r.end, false)? {
            m if m == DAY - 1 => DAY,
            m => m,
        };
        if end > start {
            charge.push((start, end));
        } else {
            charge.push((start, DAY));
            if end > 0 {
                charge.push((0, end));
            }
        }
    }
    charge.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (s, e) in charge {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }

    let mut peak = Vec::new();
    let mut cursor = 0;
    for &(s, e) in &merged {
        if s > cursor {
            peak.push((cursor, s));
        }
        cursor = e;
    }
    if cursor < DAY {
        peak.push((cursor, DAY));
    }

    let mut segments: Vec<Segment> = merged
        .into_iter()
        .map(|(s, e)| (s, e, Direction::Charge, t.charge_power))
        .chain(peak.into_iter().map(|(s, e)| (s, e, Direction::Discharge, t.discharge_power)))
        .collect();
    segments.sort_unstable_by_key(|s| s.0);
    Ok(segments)
}

/// Remplit la grille : les mêmes segments sur chaque jour coché. La limite de 10 créneaux se vérifie à la compilation.
pub fn tariff_plan(t: &Tariff) -> Result<Vec<Block>, ApiError> {
    let segments = tariff_segments(t)?;
    Ok((0..7u8)
        .filter(|d| t.days >> d & 1 == 1)
        .flat_map(|day| {
            segments.iter().map(move |&(s, e, direction, power)| Block {
                day,
                start: clock(s),
                end: clock(e),
                direction,
                power,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tariff(ranges: &[(&str, &str)]) -> Tariff {
        Tariff {
            offpeak: ranges.iter().map(|(s, e)| Range { start: (*s).into(), end: (*e).into() }).collect(),
            charge_power: 800,
            discharge_power: 300,
            days: 127,
        }
    }

    fn block(day: u8, start: &str, end: &str, direction: Direction, power: u16) -> Block {
        Block { day, start: start.into(), end: end.into(), direction, power }
    }

    fn monday(blocks: &[Block]) -> Vec<(&str, &str, Direction, u16)> {
        blocks.iter().filter(|b| b.day == 0).map(|b| (b.start.as_str(), b.end.as_str(), b.direction, b.power)).collect()
    }

    #[test]
    fn splits_at_midnight_and_fills_peak_hours() {
        let blocks = tariff_plan(&tariff(&[("22:00", "06:00")])).unwrap();
        assert_eq!(
            monday(&blocks),
            [
                ("00:00", "06:00", Direction::Charge, 800),
                ("06:00", "22:00", Direction::Discharge, 300),
                ("22:00", "24:00", Direction::Charge, 800),
            ]
        );
        assert_eq!(blocks.len(), 21);
    }

    #[test]
    fn two_ranges_without_midnight() {
        let blocks = tariff_plan(&tariff(&[("01:00", "07:00"), ("12:30", "14:30")])).unwrap();
        assert_eq!(
            monday(&blocks),
            [
                ("00:00", "01:00", Direction::Discharge, 300),
                ("01:00", "07:00", Direction::Charge, 800),
                ("07:00", "12:30", Direction::Discharge, 300),
                ("12:30", "14:30", Direction::Charge, 800),
                ("14:30", "24:00", Direction::Discharge, 300),
            ]
        );
    }

    #[test]
    fn overlapping_ranges_are_merged() {
        let blocks = tariff_plan(&tariff(&[("01:00", "05:00"), ("04:00", "07:00")])).unwrap();
        let charge: Vec<_> = blocks.iter().filter(|b| b.day == 0 && b.direction == Direction::Charge).collect();
        assert_eq!((charge.len(), charge[0].end.as_str()), (1, "07:00"));
    }

    #[test]
    fn tariff_only_fills_selected_days() {
        let mut t = tariff(&[("22:00", "06:00")]);
        t.days = 0b100_0001; // lundi et dimanche
        let days: Vec<u8> = tariff_plan(&t).unwrap().iter().map(|b| b.day).collect();
        assert!(days.iter().all(|d| *d == 0 || *d == 6) && days.contains(&6));
    }

    #[test]
    fn invalid_time_is_an_error() {
        assert!(tariff_plan(&tariff(&[("25:00", "06:00")])).is_err());
        assert!(tariff_plan(&tariff(&[("22:00", "6h")])).is_err());
    }

    #[test]
    fn contiguous_blocks_merge_per_day() {
        let blocks = [
            block(0, "00:00", "03:00", Direction::Charge, 800),
            block(0, "03:00", "06:00", Direction::Charge, 800),
            block(0, "06:00", "08:00", Direction::Charge, 400),
        ];
        let slots = compile_plan(&blocks).unwrap();
        let view: Vec<_> = slots.iter().map(|s| (s.start.as_str(), s.end.as_str(), s.power)).collect();
        assert_eq!(view, [("00:00", "06:00", 800), ("06:00", "08:00", 400)]);
    }

    #[test]
    fn identical_slots_share_a_week_set() {
        let blocks = [
            block(0, "22:00", "24:00", Direction::Charge, 800),
            block(2, "22:00", "24:00", Direction::Charge, 800),
            block(1, "22:00", "24:00", Direction::Charge, 600),
        ];
        let slots = compile_plan(&blocks).unwrap();
        assert_eq!(slots.len(), 2);
        let strong = slots.iter().find(|s| s.power == 800).unwrap();
        assert_eq!((strong.days, strong.end.as_str()), (0b101, "23:59"));
        assert_eq!(slots.iter().find(|s| s.power == 600).unwrap().days, 0b10);
    }

    #[test]
    fn more_than_ten_slots_is_an_error_with_the_count() {
        let blocks: Vec<_> =
            (0..11).map(|i| block(0, &format!("{i:02}:00"), &format!("{i:02}:30"), Direction::Charge, 500)).collect();
        let err = compile_plan(&blocks).unwrap_err().to_string();
        assert!(err.contains("11 créneaux"), "{err}");
        assert_eq!(compile_plan(&blocks[..10]).unwrap().len(), 10);
    }

    #[test]
    fn bad_blocks_are_rejected() {
        let one = |s, e| [block(0, s, e, Direction::Charge, 500)];
        assert!(compile_plan(&one("10:00", "10:00")).is_err());
        assert!(compile_plan(&one("24:00", "24:00")).is_err());
        assert!(compile_plan(&[block(7, "01:00", "02:00", Direction::Charge, 500)]).is_err());
        let overlap = [
            block(3, "01:00", "05:00", Direction::Charge, 500),
            block(3, "04:00", "06:00", Direction::Discharge, 500),
        ];
        assert!(compile_plan(&overlap).unwrap_err().to_string().contains("jeudi"));
    }

    #[test]
    fn tariff_plan_compiles_to_three_daily_slots() {
        let slots = compile_plan(&tariff_plan(&tariff(&[("22:00", "06:00")])).unwrap()).unwrap();
        assert_eq!(slots.len(), 3);
        assert!(slots.iter().all(|s| s.days == 127 && s.enabled));
        assert_eq!(slots[2].end, "23:59");
    }
}
