//! Décisions de pilotage, en fonctions pures : règles SOC, reprise du plan, alertes, test hors créneaux.

use crate::api::{Direction, Slot};
use crate::clock::MINUTE;
use crate::plan::{Block, Kind, Plan, clock, minutes};
use crate::settings::{ModeKind, Notify, Rules};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Écart entre déclenchement et relâchement d'une règle SOC, en points.
pub const HYSTERESIS: f64 = 5.0;
pub const OFFLINE_AFTER: Duration = Duration::from_secs(120);
/// Fin d'une action manuelle qui n'a pas de plan à qui rendre la main.
pub const FOREVER: i64 = i64::MAX;
const WEEK: u32 = 7 * 1440;
pub const TEST_MINUTES: u32 = 5;
const TEST_POWER: u16 = 200;
const IDLE_BELOW: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Low,
    High,
}

/// Qui pilote la batterie en ce moment.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Plan,
    Rule,
    Manual,
    /// Blocs auto, IA, UPS et limites de SOC du plan, tenus par l'application.
    Hybrid,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Control {
    pub source: Source,
    pub detail: String,
}

/// Ce que l'application impose à la batterie à la place du plan Manual.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Drive {
    /// Bloc auto, IA ou UPS.
    Mode(ModeKind),
    /// Limite de SOC atteinte : Passive 0 W jusqu'à la fin du bloc.
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hybrid {
    pub drive: Drive,
    /// Fin du bloc (ms epoch).
    pub until: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Engine {
    pub latch: Option<Side>,
    /// Fin de l'action manuelle (ms epoch), `FOREVER` sans plan à qui rendre la main.
    pub manual_until: Option<i64>,
    pub suspended: bool,
    pub hybrid: Option<Hybrid>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    None,
    /// Une règle vient de se déclencher : écrire son mode.
    Rule(Side),
    /// Renvoyer les 10 créneaux du plan actif.
    HandBack,
}

/// Règle verrouillée : déclenchée à `soc <= X` (bas) ou `soc >= Y` (haut), relâchée avec 5 points d'écart.
/// Sans mesure, l'état ne bouge pas.
pub fn next_latch(rules: &Rules, soc: Option<f64>, latch: Option<Side>) -> Option<Side> {
    let Some(soc) = soc else { return latch };
    let (low, high) = (&rules.low, &rules.high);
    match latch {
        Some(Side::Low) if low.enabled && soc < low.soc + HYSTERESIS => latch,
        Some(Side::High) if high.enabled && soc > high.soc - HYSTERESIS => latch,
        _ if low.enabled && soc <= low.soc => Some(Side::Low),
        _ if high.enabled && soc >= high.soc => Some(Side::High),
        _ => None,
    }
}

pub fn decide(rules: &Rules, mut e: Engine, soc: Option<f64>, now_ms: i64) -> (Engine, Action) {
    let expired = e.manual_until.is_some_and(|t| now_ms >= t);
    if expired {
        e.manual_until = None;
    }
    let latch = if e.suspended { None } else { next_latch(rules, soc, e.latch) };
    let action = match (latch, e.latch) {
        (Some(side), old) if old != Some(side) => Action::Rule(side),
        (None, Some(_)) if e.manual_until.is_none() => Action::HandBack,
        (None, None) if expired => Action::HandBack,
        _ => Action::None,
    };
    e.latch = latch;
    (e, action)
}

/// Minutes jusqu'à la prochaine borne de bloc (début ou fin), strictement dans le futur.
pub fn minutes_to_boundary(plan: &Plan, week_minute: u32) -> Option<u32> {
    plan.blocks
        .iter()
        .filter_map(|b| {
            let day = u32::from(b.day) * 1440;
            Some([day + minutes(&b.start, false).ok()?, day + minutes(&b.end, true).ok()?])
        })
        .flatten()
        .map(|t| match (t + WEEK - week_minute % WEEK) % WEEK {
            0 => WEEK,
            d => d,
        })
        .min()
}

/// Fin d'une action manuelle lancée à `now_ms` : la prochaine borne du plan, sinon jamais.
pub fn manual_until(plan: Option<&Plan>, now_ms: i64, week_minute: u32) -> i64 {
    plan.and_then(|p| minutes_to_boundary(p, week_minute)).map_or(FOREVER, |d| {
        now_ms - now_ms.rem_euclid(MINUTE) + i64::from(d) * MINUTE
    })
}

/// Fin d'un mode passif de `seconds` secondes : la durée demandée, ou la prochaine borne du plan si elle vient avant.
pub fn passive_until(plan: Option<&Plan>, now_ms: i64, week_minute: u32, seconds: u32) -> i64 {
    manual_until(plan, now_ms, week_minute).min(now_ms + i64::from(seconds) * 1000)
}

/// Bloc qui couvre la minute de la semaine, et les minutes qui lui restent.
pub fn current_block(plan: &Plan, week_minute: u32) -> Option<(&Block, u32)> {
    let (day, minute) = (week_minute / 1440, week_minute % 1440);
    plan.blocks.iter().find_map(|b| {
        let (start, end) = (minutes(&b.start, false).ok()?, minutes(&b.end, true).ok()?);
        (u32::from(b.day) == day && (start..end).contains(&minute)).then(|| (b, end - minute))
    })
}

/// Pilotage que le bloc courant demande à l'application.
/// Un bloc auto, IA ou UPS n'a rien à piloter si la batterie suit déjà la consommation hors créneaux.
/// Une limite de SOC garde l'hystérésis des règles tant que le maintien est en cours (`holding`).
pub fn hybrid_wanted(
    plan: Option<&Plan>,
    verdict: Option<Verdict>,
    soc: Option<f64>,
    now_ms: i64,
    week_minute: u32,
    holding: bool,
) -> Option<Hybrid> {
    let (block, left) = current_block(plan?, week_minute)?;
    let drive = match block.kind.mode() {
        Some(mode) => (verdict != Some(Verdict::Follows)).then_some(Drive::Mode(mode))?,
        None => {
            let limit = f64::from(block.soc_limit?);
            let margin = if holding { HYSTERESIS } else { 0.0 };
            let reached = soc.map_or(holding, |s| {
                if block.kind == Kind::Charge { s >= limit - margin } else { s <= limit + margin }
            });
            reached.then_some(Drive::Hold)?
        }
    };
    Some(Hybrid { drive, until: now_ms - now_ms.rem_euclid(MINUTE) + i64::from(left) * MINUTE })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HybridAction {
    None,
    Apply(Hybrid),
    /// Le pilotage est fini : renvoyer les 10 créneaux du plan.
    HandBack,
}

pub fn hybrid_step(active: Option<Hybrid>, wanted: Option<Hybrid>) -> HybridAction {
    match (active, wanted) {
        // Un mode n'expire pas : deux blocs contigus du même mode (minuit) ne demandent pas de nouvelle écriture.
        (Some(a), Some(w)) if a.drive == w.drive && matches!(w.drive, Drive::Mode(_)) => HybridAction::None,
        (a, Some(w)) if a != Some(w) => HybridAction::Apply(w),
        (Some(_), None) => HybridAction::HandBack,
        _ => HybridAction::None,
    }
}

/// Durée du maintien passif : les secondes qui restent au bloc.
pub fn hold_seconds(until_ms: i64, now_ms: i64) -> u32 {
    ((until_ms - now_ms) / 1000).max(1) as u32
}

/// L'application a-t-elle mis la batterie hors du plan Manual ?
pub fn off_plan(e: &Engine, testing: bool) -> bool {
    e.latch.is_some() || e.manual_until.is_some() || e.hybrid.is_some() || testing
}

/// Alerte « SOC bas » : une fois par descente, ré-armée avec la même hystérésis. Rend (état, envoyer).
pub fn low_soc_alert(n: &Notify, soc: Option<f64>, notified: bool) -> (bool, bool) {
    let Some(soc) = soc else { return (notified, false) };
    if !n.low_soc.enabled {
        return (false, false);
    }
    if notified {
        (soc < n.low_soc.soc + HYSTERESIS, false)
    } else {
        let low = soc <= n.low_soc.soc;
        (low, low)
    }
}

/// Alerte « injoignable » : une fois par incident, ré-armée dès qu'une lecture réussit. Rend (état, envoyer).
pub fn offline_alert(enabled: bool, silent: Duration, notified: bool) -> (bool, bool) {
    if silent > OFFLINE_AFTER { (true, enabled && !notified) } else { (false, false) }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Idle,
    Follows,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub ts: i64,
    pub bat_power: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlotTest {
    pub samples: Vec<Point>,
    /// `None` tant que le test tourne.
    pub verdict: Option<Verdict>,
}

/// Moyenne de |puissance| après la fin du créneau : sous 30 W la batterie reste au repos hors créneau.
pub fn verdict(samples: &[Point], slot_end_ms: i64) -> Option<Verdict> {
    let after: Vec<f64> = samples.iter().filter(|p| p.ts >= slot_end_ms).filter_map(|p| p.bat_power).collect();
    if after.is_empty() {
        return None;
    }
    let mean = after.iter().map(|p| p.abs()).sum::<f64>() / after.len() as f64;
    Some(if mean < IDLE_BELOW { Verdict::Idle } else { Verdict::Follows })
}

/// Charge 200 W de la minute courante à +5 min, aujourd'hui seulement.
// shortcut: refus dans les 5 dernières minutes de la journée au lieu de couper le créneau à minuit.
pub fn test_slot(week_minute: u32) -> Result<Slot, String> {
    let start = week_minute % 1440;
    if start + TEST_MINUTES >= 1440 {
        return Err("trop près de minuit : relance le test dans quelques minutes".into());
    }
    Ok(Slot {
        start: clock(start),
        end: clock(start + TEST_MINUTES),
        days: 1 << (week_minute / 1440),
        direction: Direction::Charge,
        power: TEST_POWER,
        enabled: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{ModeKind, Rule};

    fn rules(low: Option<f64>, high: Option<f64>) -> Rules {
        let rule = |soc: Option<f64>| Rule { enabled: soc.is_some(), soc: soc.unwrap_or(0.0), mode: ModeKind::Ups };
        Rules { low: rule(low), high: rule(high) }
    }

    fn plan(blocks: &[(u8, &str, &str)]) -> Plan {
        Plan {
            name: "p".into(),
            blocks: blocks
                .iter()
                .map(|&(day, s, e)| Block { day, start: s.into(), end: e.into(), kind: Kind::Charge, power: 500, soc_limit: None })
                .collect(),
        }
    }

    #[test]
    fn low_rule_triggers_and_releases_with_hysteresis() {
        let r = rules(Some(20.0), None);
        let mut latch = None;
        let mut seen = Vec::new();
        for soc in [30.0, 20.0, 22.0, 24.9, 25.0, 40.0] {
            latch = next_latch(&r, Some(soc), latch);
            seen.push(latch);
        }
        assert_eq!(seen, [None, Some(Side::Low), Some(Side::Low), Some(Side::Low), None, None]);
    }

    #[test]
    fn high_rule_triggers_and_releases_with_hysteresis() {
        let r = rules(None, Some(90.0));
        let mut latch = None;
        let mut seen = Vec::new();
        for soc in [80.0, 90.0, 86.0, 85.1, 85.0] {
            latch = next_latch(&r, Some(soc), latch);
            seen.push(latch);
        }
        assert_eq!(seen, [None, Some(Side::High), Some(Side::High), Some(Side::High), None]);
    }

    #[test]
    fn disabled_rule_never_triggers_and_missing_soc_keeps_state() {
        assert_eq!(next_latch(&rules(None, None), Some(1.0), None), None);
        assert_eq!(next_latch(&rules(Some(20.0), None), None, Some(Side::Low)), Some(Side::Low));
    }

    #[test]
    fn decide_applies_a_rule_once_then_hands_back() {
        let r = rules(Some(20.0), None);
        let (e, a) = decide(&r, Engine::default(), Some(18.0), 0);
        assert_eq!(a, Action::Rule(Side::Low));
        let (e, a) = decide(&r, e, Some(19.0), 10_000);
        assert_eq!(a, Action::None);
        let (e, a) = decide(&r, e, Some(26.0), 20_000);
        assert_eq!((a, e.latch), (Action::HandBack, None));
    }

    #[test]
    fn manual_action_holds_until_expiry_then_hands_back() {
        let r = rules(None, None);
        let e = Engine { manual_until: Some(1000), ..Engine::default() };
        assert_eq!(decide(&r, e, Some(50.0), 999).1, Action::None);
        let (e, a) = decide(&r, e, Some(50.0), 1000);
        assert_eq!((a, e.manual_until), (Action::HandBack, None));
        assert_eq!(decide(&r, e, Some(50.0), 2000).1, Action::None);
    }

    #[test]
    fn rule_release_during_manual_action_does_not_hand_back() {
        let r = rules(Some(20.0), None);
        let e = Engine { latch: Some(Side::Low), manual_until: Some(10_000), ..Engine::default() };
        let (e, a) = decide(&r, e, Some(30.0), 5000);
        assert_eq!((a, e.latch), (Action::None, None));
    }

    #[test]
    fn suspending_releases_the_rule() {
        let r = rules(Some(20.0), None);
        let e = Engine { latch: Some(Side::Low), suspended: true, ..Engine::default() };
        assert_eq!(decide(&r, e, Some(10.0), 0).1, Action::HandBack);
    }

    #[test]
    fn rule_beats_pending_manual_action() {
        let r = rules(Some(20.0), None);
        let e = Engine { manual_until: Some(FOREVER), ..Engine::default() };
        assert_eq!(decide(&r, e, Some(10.0), 0).1, Action::Rule(Side::Low));
    }

    #[test]
    fn boundary_is_the_next_block_edge_and_wraps_the_week() {
        let p = plan(&[(0, "08:00", "12:00"), (6, "22:00", "24:00")]);
        assert_eq!(minutes_to_boundary(&p, 9 * 60), Some(3 * 60));
        assert_eq!(minutes_to_boundary(&p, 8 * 60), Some(4 * 60)); // une borne à l'instant même ne compte pas
        // dimanche 23:00 : la fin de bloc 24:00 tombe lundi 00:00.
        assert_eq!(minutes_to_boundary(&p, 6 * 1440 + 23 * 60), Some(60));
        assert_eq!(minutes_to_boundary(&plan(&[]), 0), None);
    }

    #[test]
    fn manual_until_rounds_to_the_minute_and_defaults_to_forever() {
        let p = plan(&[(0, "08:00", "12:00")]);
        assert_eq!(manual_until(Some(&p), 90_500, 9 * 60), 60_000 + 180 * 60_000);
        assert_eq!(manual_until(None, 0, 0), FOREVER);
    }

    #[test]
    fn passive_ends_at_the_earlier_of_duration_and_boundary() {
        let p = plan(&[(0, "08:00", "12:00")]);
        assert_eq!(passive_until(Some(&p), 90_500, 9 * 60, 60), 90_500 + 60_000);
        assert_eq!(passive_until(Some(&p), 90_500, 9 * 60, 86_400), manual_until(Some(&p), 90_500, 9 * 60));
        assert_eq!(passive_until(None, 1000, 0, 30), 31_000);
    }

    #[test]
    fn fresh_connection_hands_the_plan_back_on_first_tick() {
        let e = Engine { manual_until: Some(i64::MIN), ..Engine::default() };
        assert_eq!(decide(&rules(None, None), e, Some(50.0), 0).1, Action::HandBack);
    }

    #[test]
    fn low_soc_alert_fires_once_per_descent() {
        let n = Notify::default(); // seuil 15
        let (s, send) = low_soc_alert(&n, Some(14.0), false);
        assert!(s && send);
        assert_eq!(low_soc_alert(&n, Some(12.0), true), (true, false));
        assert_eq!(low_soc_alert(&n, Some(19.0), true), (true, false));
        assert_eq!(low_soc_alert(&n, Some(20.0), true), (false, false));
        assert_eq!(low_soc_alert(&n, Some(60.0), false), (false, false));
    }

    #[test]
    fn offline_alert_fires_once_after_two_minutes() {
        let s = Duration::from_secs;
        assert_eq!(offline_alert(true, s(120), false), (false, false));
        assert_eq!(offline_alert(true, s(121), false), (true, true));
        assert_eq!(offline_alert(true, s(300), true), (true, false));
        assert_eq!(offline_alert(true, s(5), true), (false, false));
        assert_eq!(offline_alert(false, s(300), false), (true, false));
    }

    #[test]
    fn verdict_averages_power_after_the_slot() {
        let p = |ts, w| Point { ts, bat_power: w };
        assert_eq!(verdict(&[p(0, Some(-200.0)), p(100, Some(5.0)), p(110, Some(-10.0))], 100), Some(Verdict::Idle));
        assert_eq!(verdict(&[p(100, Some(-180.0)), p(110, Some(-190.0))], 100), Some(Verdict::Follows));
        assert_eq!(verdict(&[p(0, Some(-200.0)), p(100, None)], 100), None);
    }

    #[test]
    fn test_slot_covers_five_minutes_of_today() {
        let s = test_slot(2 * 1440 + 14 * 60 + 58).unwrap();
        assert_eq!((s.start.as_str(), s.end.as_str(), s.days, s.power), ("14:58", "15:03", 0b100, 200));
        assert!(test_slot(1435).is_err());
        assert!(test_slot(1434).is_ok());
    }

    fn hybrid_plan(blocks: &[(Kind, u32, u32, Option<u8>)]) -> Plan {
        let clock_at = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        Plan {
            name: "h".into(),
            blocks: blocks
                .iter()
                .map(|&(kind, s, e, soc_limit)| Block { day: 0, start: clock_at(s), end: clock_at(e), kind, power: 500, soc_limit })
                .collect(),
        }
    }

    const NOW: i64 = 5 * MINUTE;

    #[test]
    fn current_block_gives_the_block_and_the_minutes_left() {
        let p = hybrid_plan(&[(Kind::Auto, 8 * 60, 10 * 60, None)]);
        assert_eq!(current_block(&p, 9 * 60).map(|(b, left)| (b.kind, left)), Some((Kind::Auto, 60)));
        assert!(current_block(&p, 8 * 60).is_some());
        assert!(current_block(&p, 10 * 60).is_none());
        assert!(current_block(&p, 1440 + 9 * 60).is_none());
    }

    #[test]
    fn auto_blocks_are_piloted_unless_the_battery_already_follows_the_load() {
        let p = hybrid_plan(&[(Kind::Ups, 0, 120, None)]);
        let wanted = |v| hybrid_wanted(Some(&p), v, Some(50.0), NOW, 5, false);
        let on = Some(Hybrid { drive: Drive::Mode(ModeKind::Ups), until: 120 * MINUTE });
        assert_eq!(wanted(None), on);
        assert_eq!(wanted(Some(Verdict::Idle)), on);
        assert_eq!(wanted(Some(Verdict::Follows)), None);
        assert_eq!(hybrid_wanted(None, None, None, NOW, 5, false), None);
    }

    #[test]
    fn charge_limit_holds_once_the_target_is_reached() {
        let p = hybrid_plan(&[(Kind::Charge, 0, 360, Some(90))]);
        let wanted = |soc, holding| hybrid_wanted(Some(&p), Some(Verdict::Follows), soc, NOW, 5, holding).map(|h| h.drive);
        assert_eq!(wanted(Some(89.0), false), None);
        assert_eq!(wanted(Some(90.0), false), Some(Drive::Hold));
        // le maintien ne se relâche pas sur un rebond de moins de 5 points
        assert_eq!(wanted(Some(86.0), true), Some(Drive::Hold));
        assert_eq!(wanted(Some(85.0), true), Some(Drive::Hold));
        assert_eq!(wanted(Some(84.9), true), None);
        assert_eq!(wanted(None, true), Some(Drive::Hold));
        assert_eq!(wanted(None, false), None);
    }

    #[test]
    fn discharge_limit_is_a_floor_and_a_block_without_limit_is_left_to_the_plan() {
        let p = hybrid_plan(&[(Kind::Discharge, 0, 60, Some(20)), (Kind::Charge, 60, 120, None)]);
        let wanted = |soc, minute| hybrid_wanted(Some(&p), None, Some(soc), NOW, minute, false);
        assert_eq!(wanted(21.0, 5), None);
        assert_eq!(wanted(20.0, 5).map(|h| (h.drive, h.until)), Some((Drive::Hold, 60 * MINUTE)));
        assert_eq!(wanted(100.0, 70), None);
        assert_eq!(wanted(0.0, 70), None);
    }

    #[test]
    fn hybrid_step_applies_changes_and_hands_back_when_it_ends() {
        let h = |until| Hybrid { drive: Drive::Hold, until };
        assert_eq!(hybrid_step(None, Some(h(1))), HybridAction::Apply(h(1)));
        assert_eq!(hybrid_step(Some(h(1)), Some(h(1))), HybridAction::None);
        assert_eq!(hybrid_step(Some(h(1)), Some(h(2))), HybridAction::Apply(h(2)));
        assert_eq!(hybrid_step(Some(h(1)), None), HybridAction::HandBack);
        assert_eq!(hybrid_step(None, None), HybridAction::None);
        let auto = |until| Hybrid { drive: Drive::Mode(ModeKind::Auto), until };
        assert_eq!(hybrid_step(Some(auto(1)), Some(auto(2))), HybridAction::None);
        let ups = Hybrid { drive: Drive::Mode(ModeKind::Ups), until: 2 };
        assert_eq!(hybrid_step(Some(auto(1)), Some(ups)), HybridAction::Apply(ups));
    }

    #[test]
    fn hold_lasts_until_the_end_of_the_block() {
        assert_eq!(hold_seconds(10 * MINUTE, 4 * MINUTE + 30_000), 330);
        assert_eq!(hold_seconds(1000, 5000), 1);
    }

    #[test]
    fn app_control_is_off_plan() {
        assert!(!off_plan(&Engine::default(), false));
        assert!(off_plan(&Engine::default(), true));
        assert!(off_plan(&Engine { latch: Some(Side::Low), ..Engine::default() }, false));
        assert!(off_plan(&Engine { manual_until: Some(FOREVER), ..Engine::default() }, false));
        let hybrid = Some(Hybrid { drive: Drive::Hold, until: 1 });
        assert!(off_plan(&Engine { hybrid, ..Engine::default() }, false));
    }
}
