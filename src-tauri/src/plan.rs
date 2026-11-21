//! Planning hebdomadaire : blocs → créneaux Manuel, et tarif heures creuses → blocs.

use crate::api::{ApiError, Direction, SLOTS, Slot};
use crate::settings::ModeKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DAY: u32 = 24 * 60;
const DAY_NAMES: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];

#[derive(Debug, Deserialize)]
pub struct Range {
    pub start: String,
    pub end: String,
}

/// Ce que la batterie fait en heures pleines.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Peak {
    #[default]
    Auto,
    Discharge,
}

fn default_charge_soc() -> u8 {
    90
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tariff {
    pub offpeak: Vec<Range>,
    pub charge_power: u16,
    pub discharge_power: u16,
    pub days: u8,
    /// SOC visé en heures creuses ; 100 = pas de limite.
    #[serde(default = "default_charge_soc")]
    pub charge_soc: u8,
    #[serde(default)]
    pub peak: Peak,
}

/// Charge et décharge s'écrivent en créneaux Manuel ; les autres types ne se pilotent que depuis l'application.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Charge,
    Discharge,
    Auto,
    Ai,
    Ups,
}

impl Kind {
    pub fn direction(self) -> Option<Direction> {
        match self {
            Self::Charge => Some(Direction::Charge),
            Self::Discharge => Some(Direction::Discharge),
            _ => None,
        }
    }

    pub fn mode(self) -> Option<ModeKind> {
        match self {
            Self::Auto => Some(ModeKind::Auto),
            Self::Ai => Some(ModeKind::Ai),
            Self::Ups => Some(ModeKind::Ups),
            _ => None,
        }
    }
}

/// Plage de la grille hebdomadaire ; `end` peut valoir "24:00" (fin de journée).
/// Un ancien bloc `{direction, power}` se lit comme charge ou décharge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// 0 = lundi.
    pub day: u8,
    pub start: String,
    pub end: String,
    #[serde(alias = "direction")]
    pub kind: Kind,
    /// Plafond natif d'un créneau de charge ou décharge ; sans objet pour les autres types.
    #[serde(default)]
    pub power: u16,
    /// Cible de SOC (charge) ou plancher (décharge), tenu par l'application et non par la batterie.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soc_limit: Option<u8>,
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

type Segment = (u32, u32, Kind, u16);

/// Fusionne les blocs contigus de même réglage par jour, puis regroupe les créneaux identiques
/// de plusieurs jours en un seul (`week_set`). Les blocs auto, IA et UPS ne donnent pas de créneau.
/// Plus de 10 créneaux : erreur avec le nombre.
pub fn compile_plan(blocks: &[Block]) -> Result<Vec<Slot>, ApiError> {
    let mut days: [Vec<Segment>; 7] = Default::default();
    for b in blocks {
        let (start, end) = (minutes(&b.start, false)?, minutes(&b.end, true)?);
        if end <= start {
            return Err(ApiError::Invalid(format!("bloc invalide : {} doit finir après {}", b.end, b.start)));
        }
        days.get_mut(usize::from(b.day))
            .ok_or_else(|| ApiError::Invalid(format!("jour invalide : {}", b.day)))?
            .push((start, end, b.kind, b.power));
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
        for (start, end, kind, power) in merged {
            if let Some(direction) = kind.direction() {
                *groups.entry((start, end, direction == Direction::Discharge, power)).or_default() |= 1 << day;
            }
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

/// Charge sur chaque plage creuse, le mode des heures pleines sur le reste de la journée.
fn tariff_segments(t: &Tariff) -> Result<Vec<(u32, u32, Kind)>, ApiError> {
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

    let peak_kind = match t.peak {
        Peak::Auto => Kind::Auto,
        Peak::Discharge => Kind::Discharge,
    };
    let mut segments: Vec<_> = merged
        .into_iter()
        .map(|(s, e)| (s, e, Kind::Charge))
        .chain(peak.into_iter().map(|(s, e)| (s, e, peak_kind)))
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
            segments.iter().map(move |&(s, e, kind)| Block {
                day,
                start: clock(s),
                end: clock(e),
                kind,
                power: match kind {
                    Kind::Charge => t.charge_power,
                    Kind::Discharge => t.discharge_power,
                    _ => 0,
                },
                soc_limit: (kind == Kind::Charge && t.charge_soc < 100).then_some(t.charge_soc),
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
            charge_soc: 100,
            peak: Peak::Discharge,
        }
    }

    fn block(day: u8, start: &str, end: &str, kind: Kind, power: u16) -> Block {
        Block { day, start: start.into(), end: end.into(), kind, power, soc_limit: None }
    }

    fn monday(blocks: &[Block]) -> Vec<(&str, &str, Kind, u16)> {
        blocks.iter().filter(|b| b.day == 0).map(|b| (b.start.as_str(), b.end.as_str(), b.kind, b.power)).collect()
    }

    #[test]
    fn splits_at_midnight_and_fills_peak_hours() {
        let blocks = tariff_plan(&tariff(&[("22:00", "06:00")])).unwrap();
        assert_eq!(
            monday(&blocks),
            [
                ("00:00", "06:00", Kind::Charge, 800),
                ("06:00", "22:00", Kind::Discharge, 300),
                ("22:00", "24:00", Kind::Charge, 800),
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
                ("00:00", "01:00", Kind::Discharge, 300),
                ("01:00", "07:00", Kind::Charge, 800),
                ("07:00", "12:30", Kind::Discharge, 300),
                ("12:30", "14:30", Kind::Charge, 800),
                ("14:30", "24:00", Kind::Discharge, 300),
            ]
        );
    }

    #[test]
    fn overlapping_ranges_are_merged() {
        let blocks = tariff_plan(&tariff(&[("01:00", "05:00"), ("04:00", "07:00")])).unwrap();
        let charge: Vec<_> = blocks.iter().filter(|b| b.day == 0 && b.kind == Kind::Charge).collect();
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
            block(0, "00:00", "03:00", Kind::Charge, 800),
            block(0, "03:00", "06:00", Kind::Charge, 800),
            block(0, "06:00", "08:00", Kind::Charge, 400),
        ];
        let slots = compile_plan(&blocks).unwrap();
        let view: Vec<_> = slots.iter().map(|s| (s.start.as_str(), s.end.as_str(), s.power)).collect();
        assert_eq!(view, [("00:00", "06:00", 800), ("06:00", "08:00", 400)]);
    }

    #[test]
    fn identical_slots_share_a_week_set() {
        let blocks = [
            block(0, "22:00", "24:00", Kind::Charge, 800),
            block(2, "22:00", "24:00", Kind::Charge, 800),
            block(1, "22:00", "24:00", Kind::Charge, 600),
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
            (0..11).map(|i| block(0, &format!("{i:02}:00"), &format!("{i:02}:30"), Kind::Charge, 500)).collect();
        let err = compile_plan(&blocks).unwrap_err().to_string();
        assert!(err.contains("11 créneaux"), "{err}");
        assert_eq!(compile_plan(&blocks[..10]).unwrap().len(), 10);
    }

    #[test]
    fn bad_blocks_are_rejected() {
        let one = |s, e| [block(0, s, e, Kind::Charge, 500)];
        assert!(compile_plan(&one("10:00", "10:00")).is_err());
        assert!(compile_plan(&one("24:00", "24:00")).is_err());
        assert!(compile_plan(&[block(7, "01:00", "02:00", Kind::Charge, 500)]).is_err());
        let overlap = [
            block(3, "01:00", "05:00", Kind::Charge, 500),
            block(3, "04:00", "06:00", Kind::Discharge, 500),
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

    #[test]
    fn legacy_block_reads_as_charge_or_discharge() {
        let old = r#"{"day":2,"start":"01:00","end":"02:00","direction":"discharge","power":400}"#;
        let b: Block = serde_json::from_str(old).unwrap();
        assert_eq!((b.kind, b.power, b.soc_limit), (Kind::Discharge, 400, None));
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!((v["kind"].as_str(), v.get("socLimit")), (Some("discharge"), None));
    }

    #[test]
    fn hybrid_blocks_do_not_become_slots() {
        let blocks = [
            block(0, "00:00", "06:00", Kind::Charge, 800),
            block(0, "06:00", "22:00", Kind::Auto, 0),
            block(0, "22:00", "24:00", Kind::Charge, 800),
            block(1, "08:00", "09:00", Kind::Ups, 0),
        ];
        let slots = compile_plan(&blocks).unwrap();
        assert_eq!(slots.len(), 2);
        assert!(compile_plan(&blocks[1..2]).unwrap().is_empty());
        let overlap = [block(0, "01:00", "05:00", Kind::Auto, 0), block(0, "04:00", "06:00", Kind::Ai, 0)];
        assert!(compile_plan(&overlap).is_err());
    }

    #[test]
    fn default_tariff_charges_to_90_then_leaves_peak_hours_to_auto() {
        let t: Tariff = serde_json::from_str(
            r#"{"offpeak":[{"start":"22:00","end":"06:00"}],"chargePower":800,"dischargePower":300,"days":1}"#,
        )
        .unwrap();
        let day = tariff_plan(&t).unwrap();
        let view: Vec<_> = day.iter().map(|b| (b.kind, b.power, b.soc_limit)).collect();
        assert_eq!(view, [(Kind::Charge, 800, Some(90)), (Kind::Auto, 0, None), (Kind::Charge, 800, Some(90))]);
        assert_eq!(compile_plan(&day).unwrap().len(), 2);
    }

    #[test]
    fn tariff_can_discharge_at_peak_and_drop_the_charge_limit() {
        let mut t = tariff(&[("22:00", "06:00")]);
        t.charge_soc = 80;
        assert_eq!(tariff_plan(&t).unwrap()[0].soc_limit, Some(80));
        t.charge_soc = 100;
        let blocks = tariff_plan(&t).unwrap();
        assert!(blocks.iter().all(|b| b.soc_limit.is_none()));
        assert!(blocks.iter().any(|b| b.kind == Kind::Discharge && b.power == 300));
    }
}
