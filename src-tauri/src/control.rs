//! Décisions de pilotage, en fonctions pures : règles SOC, reprise du plan, alertes, test hors créneaux.

use crate::api::{CHARGE_SIGN, Direction, Slot};
use crate::clock::MINUTE;
use crate::i18n::{Lang, Msg};
use crate::plan::{Block, Kind, Plan, clock, minutes};
use crate::settings::{ModeKind, Notify, Rules};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Écart entre déclenchement et relâchement d'une règle SOC, en points.
pub const HYSTERESIS: f64 = 5.0;
pub const OFFLINE_AFTER: Duration = Duration::from_secs(600);
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
    /// Pilotage automatique désactivé (réglage par défaut) : l'application lit sans rien écrire.
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
    // Suspendu : rien n'est décidé ni écrit, la batterie garde son mode ; seul l'état du moteur est mis à jour.
    if e.suspended {
        e.latch = None;
        e.hybrid = None;
        return (e, Action::None);
    }
    let latch = next_latch(rules, soc, e.latch);
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
pub fn offline_alert(enabled: bool, silent: Duration, after: Duration, notified: bool) -> (bool, bool) {
    if silent > after { (true, enabled && !notified) } else { (false, false) }
}

/// Texte de l'alerte « injoignable » selon la cause. `ping_ok` : l'IP a répondu au ping (ignoré si le port est fermé).
pub fn offline_message(kind: DownKind, port: Option<u16>, ping_ok: bool) -> Msg {
    match (kind, port, ping_ok) {
        (DownKind::PortClosed, Some(port), _) => Msg::OfflinePortClosed(port),
        (_, _, false) => Msg::OfflineNoWifi,
        _ => Msg::OfflineSilent,
    }
}

/// Retour de la connexion : rend la durée de la coupure, une seule fois par incident (`since` = début de la coupure alertée).
pub fn back_alert(since: &mut Option<Instant>, online: bool) -> Option<Duration> {
    if online { since.take().map(|t| t.elapsed()) } else { None }
}

/// Texte du retour. `check` : le pilotage automatique ou un planning est actif, à revérifier après la coupure.
pub fn back_message(outage: Duration, check: bool) -> Msg {
    Msg::Back { mins: outage.as_secs().div_ceil(60), check }
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
pub fn test_slot(week_minute: u32) -> Result<Slot, Msg> {
    let start = week_minute % 1440;
    if start + TEST_MINUTES >= 1440 {
        return Err(Msg::TestNearMidnight);
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncState {
    Ok,
    Warn,
    Bad,
    #[default]
    Unknown,
}

/// Ce que la batterie devrait faire d'après le plan envoyé, comparé à ce qu'elle fait.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Sync {
    pub state: SyncState,
    pub expected: String,
    pub measured: String,
    /// Puissance batterie attendue (W signée) quand le plan la fixe, et mesurée.
    #[serde(rename = "expectedW")]
    pub expected_w: Option<f64>,
    #[serde(rename = "measuredW")]
    pub measured_w: Option<f64>,
    pub reason: Option<String>,
    /// `reason` en français pour le journal sur disque, quand la langue de l'interface en est une autre.
    #[serde(rename = "journalReason", skip_serializing_if = "Option::is_none")]
    pub journal_reason: Option<String>,
}

#[derive(Clone, Copy)]
pub struct SyncInput<'a> {
    /// Langue des textes rendus (`expected`, `measured`, `reason`).
    pub lang: Lang,
    /// Dernier plan envoyé à la batterie.
    pub plan: Option<&'a Plan>,
    pub week_minute: u32,
    pub engine: Engine,
    pub testing: bool,
    pub outside: Option<Verdict>,
    /// Mode lu (ES.GetMode) et puissance batterie, seulement s'ils datent du dernier sondage.
    pub mode: Option<&'a str>,
    /// ES.GetMode a été lu depuis la connexion (même s'il est trop ancien pour être jugé).
    pub mode_seen: bool,
    pub bat_power: Option<f64>,
    /// Secondes depuis lesquelles le bloc courant est actif.
    pub active_secs: u64,
    pub online: bool,
    /// Pourquoi la batterie ne répond pas.
    pub down: Option<Down>,
    /// Plafond de puissance appliqué aux écritures : la batterie ne reçoit jamais plus.
    pub power_cap: u16,
}

/// Écart toléré entre puissance attendue et mesurée : 30 % ou 100 W, le plus large.
const SYNC_TOLERANCE: f64 = 0.3;
const SYNC_MIN_GAP: f64 = 100.0;
const SYNC_SETTLE_SECS: u64 = 60;
/// En dessous, la batterie est à l'arrêt (bruit de mesure).
const SYNC_ZERO_BELOW: f64 = 10.0;

fn mode_name(m: ModeKind) -> &'static str {
    match m {
        ModeKind::Auto => "Auto",
        ModeKind::Ai => "AI",
        ModeKind::Ups => "UPS",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DownKind {
    /// L'IP répond mais rien n'écoute sur le port.
    PortClosed,
    Silent,
}

/// Pourquoi la batterie ne répond pas.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Down {
    pub kind: DownKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Secondes depuis la dernière lecture réussie.
    pub since_secs: u64,
}

impl Down {
    pub fn new(closed_port: Option<u16>, silent: Duration) -> Self {
        let kind = if closed_port.is_some() { DownKind::PortClosed } else { DownKind::Silent };
        Self { kind, port: closed_port, since_secs: silent.as_secs() }
    }

    /// La raison d'un état inconnu : port fermé ou silence.
    pub fn msg(&self) -> Msg {
        match (self.kind, self.port) {
            (DownKind::PortClosed, Some(port)) => Msg::DownPortClosed(port),
            _ if self.since_secs < 60 => Msg::DownSilentSecs(self.since_secs),
            _ => Msg::DownSilentMins(self.since_secs / 60),
        }
    }
}

/// Le journal reste en français : si l'interface parle une autre langue, la raison française est jointe à part.
pub fn sync(i: &SyncInput) -> Sync {
    let mut s = judge(i);
    if i.lang != Lang::Fr {
        s.journal_reason = judge(&SyncInput { lang: Lang::Fr, ..*i }).reason;
    }
    s
}

fn judge(i: &SyncInput) -> Sync {
    let l = i.lang;
    let judged = |state, expected: &str, measured: &str, reason: Option<String>| Sync {
        state,
        expected: expected.to_owned(),
        measured: measured.to_owned(),
        expected_w: None,
        measured_w: None,
        reason,
        journal_reason: None,
    };
    let unknown = |why: Msg| judged(SyncState::Unknown, "", "", Some(why.text(l)));
    if !i.online {
        return unknown(i.down.map_or(Msg::DownSilent, |d| d.msg()));
    }
    let Some(plan) = i.plan else { return unknown(Msg::NoPlanSent) };
    let source = if i.testing {
        Some(Msg::SrcOutsideTest.text(l))
    } else if i.engine.latch.is_some() {
        Some(Msg::SrcRule.text(l))
    } else if i.engine.manual_until.is_some() {
        Some(Msg::SrcManual.text(l))
    } else {
        None
    };
    if let Some(source) = source {
        return judged(SyncState::Ok, &source, &source, None);
    }
    let power = i.bat_power.map_or_else(|| Msg::PowerUnknown.text(l), |p| format!("{p:+.0} W"));
    if i.engine.suspended {
        // Lecture seule : la batterie peut être dans n'importe quel mode, le plan envoyé n'est plus un attendu.
        let measured = i.mode.map_or(power.clone(), |m| format!("{m} · {power}"));
        let s = judged(SyncState::Unknown, "", &measured, Some(Msg::ReadOnlyNotFollowed.text(l)));
        return Sync { measured_w: i.bat_power, ..s };
    }
    let block = current_block(plan, i.week_minute).map(|(b, _)| b);
    // (mode attendu, puissance attendue en W signée quand elle se juge, libellé)
    let (mode, target, label) = match (i.engine.hybrid.map(|h| h.drive), block) {
        (Some(Drive::Mode(m)), _) => (mode_name(m), None, Msg::Block(m).text(l)),
        (Some(Drive::Hold), _) => ("Passive", Some(0.0), Msg::HoldCap.text(l)),
        (None, Some(b)) if b.kind.direction().is_some() => {
            let w = b.power.min(i.power_cap);
            let (sign, label) = if b.kind == Kind::Charge { (CHARGE_SIGN, Msg::Charge(w)) } else { (-CHARGE_SIGN, Msg::Discharge(w)) };
            ("Manual", Some(f64::from(sign) * f64::from(w)), label.text(l))
        }
        _ => ("Manual", None, Msg::OutsideSlot.text(l)),
    };
    let expected = format!("{mode} · {label}");
    let judged = |state, expected: &str, measured: &str, reason| Sync {
        expected_w: target,
        measured_w: i.bat_power,
        ..judged(state, expected, measured, reason)
    };
    let Some(read) = i.mode else {
        // Tant qu'aucun ES.GetMode n'est lu depuis la connexion, rien à reprocher : inconnu, pas à vérifier.
        let state = if i.mode_seen { SyncState::Warn } else { SyncState::Unknown };
        return judged(state, &expected, &Msg::ModeNotRead.text(l), Some(Msg::ModeNotRead.text(l)));
    };
    let measured = format!("{read} · {power}");
    if !read.eq_ignore_ascii_case(mode) {
        return judged(SyncState::Bad, &expected, &measured, Some(Msg::ModeMismatch { expected: mode.into(), read: read.into() }.text(l)));
    }
    let Some(target) = target else {
        // Un mode imposé par l'application ou un bloc qui ne se juge pas : seul le hors-créneau dépend du verdict.
        let unknown = block.is_none() && i.engine.hybrid.is_none() && i.outside.is_none();
        let reason = unknown.then(|| Msg::OutsideUnknown.text(l));
        return judged(if unknown { SyncState::Warn } else { SyncState::Ok }, &expected, &measured, reason);
    };
    let Some(p) = i.bat_power else {
        return judged(SyncState::Warn, &expected, &measured, Some(Msg::BatPowerUnread.text(l)));
    };
    let (state, reason) = if (p - target).abs() <= (SYNC_TOLERANCE * target.abs()).max(SYNC_MIN_GAP) {
        (SyncState::Ok, None)
    } else if p * target < 0.0 {
        (SyncState::Bad, Some(Msg::OppositeDir { target, measured: p }.text(l)))
    } else if p.abs() < SYNC_ZERO_BELOW && i.active_secs <= SYNC_SETTLE_SECS {
        (SyncState::Ok, None)
    } else if p.abs() < SYNC_ZERO_BELOW {
        (SyncState::Warn, Some(Msg::BatStopped.text(l)))
    } else {
        (SyncState::Warn, Some(Msg::Gap((p - target).abs()).text(l)))
    };
    judged(state, &expected, &measured, reason)
}

/// Lectures de mode d'affilée hors Manual qui déclenchent la réécriture du plan, espacées d'au moins `REAPPLY_GAP`.
const REAPPLY_READS: u32 = 2;
const REAPPLY_GAP: i64 = 2 * MINUTE;
/// Au plus une réécriture automatique du plan par ce délai.
const REAPPLY_EVERY: i64 = 30 * MINUTE;

/// Suivi de la réapplication du plan (réglage `reapplyPlan`) ; dates en ms.
#[derive(Debug, Clone, Copy, Default)]
pub struct Reapply {
    off_reads: u32,
    last_read: Option<i64>,
    applied: Option<i64>,
    /// La batterie est revenue après une panne à cette date : le premier mode lu ensuite compte tout de suite.
    back_at: Option<i64>,
}

impl Reapply {
    /// Nouvelle connexion : les lectures comptées ne valent plus, la limite des 30 min reste.
    pub fn reset(&mut self) {
        *self = Self { applied: self.applied, ..Self::default() };
    }

    /// La batterie répond de nouveau après une panne.
    pub fn back(&mut self, now: i64) {
        self.back_at = Some(now);
    }

    /// `mode` : mode lu (frais) et date de sa lecture. `idle` : réapplication permise (réglages actifs, plan envoyé,
    /// ni test, ni action manuelle, ni règle, ni pilotage hybride). Vrai : il faut réécrire le plan maintenant.
    pub fn step(&mut self, mode: Option<(&str, i64)>, idle: bool, now: i64) -> bool {
        let Some((mode, at)) = mode.filter(|_| idle) else {
            if !idle {
                *self = Self { applied: self.applied, ..Self::default() };
            }
            return false;
        };
        let manual = mode.eq_ignore_ascii_case("Manual");
        let fresh = self.last_read != Some(at);
        if fresh {
            if manual {
                self.off_reads = 0;
            } else if self.last_read.is_none_or(|t| at - t >= REAPPLY_GAP) {
                self.off_reads += 1;
            }
            self.last_read = Some(at);
        }
        let back = !manual && self.back_at.is_some_and(|t| at >= t);
        // Le premier mode lu depuis le retour est jugé : Manual solde la panne, un autre mode la déclenche.
        if self.back_at.is_some_and(|t| at >= t) {
            self.back_at = None;
        }
        let due = self.off_reads >= REAPPLY_READS || back;
        if !due || self.applied.is_some_and(|t| now - t < REAPPLY_EVERY) {
            return false;
        }
        *self = Self { applied: Some(now), ..Self::default() };
        true
    }
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
    fn suspended_engine_writes_nothing_whatever_the_state() {
        let r = rules(Some(20.0), Some(90.0));
        let hybrid = Some(Hybrid { drive: Drive::Hold, until: 1 });
        let engines = [
            Engine::default(),
            Engine { latch: Some(Side::Low), ..Engine::default() },
            Engine { manual_until: Some(1000), ..Engine::default() },
            Engine { manual_until: Some(i64::MIN), ..Engine::default() },
            Engine { hybrid, ..Engine::default() },
        ];
        for e in engines {
            for soc in [None, Some(5.0), Some(50.0), Some(95.0)] {
                let (e, a) = decide(&r, Engine { suspended: true, ..e }, soc, 5000);
                assert_eq!((a, e.latch, e.hybrid), (Action::None, None, None), "{soc:?}");
            }
        }
    }

    #[test]
    fn suspended_engine_still_ends_the_manual_action_at_its_end() {
        let e = Engine { suspended: true, manual_until: Some(1000), ..Engine::default() };
        assert_eq!(decide(&rules(None, None), e, Some(50.0), 999).0.manual_until, Some(1000));
        assert_eq!(decide(&rules(None, None), e, Some(50.0), 1000).0.manual_until, None);
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
    fn fresh_connection_writes_nothing_on_first_tick() {
        for suspended in [true, false] {
            let e = Engine { suspended, ..Engine::default() };
            assert_eq!(decide(&rules(None, None), e, Some(50.0), 0).1, Action::None);
        }
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
    fn offline_alert_fires_once_after_the_given_silence() {
        let s = Duration::from_secs;
        let after = OFFLINE_AFTER;
        assert_eq!(offline_alert(true, after, after, false), (false, false));
        assert_eq!(offline_alert(true, after + s(1), after, false), (true, true));
        assert_eq!(offline_alert(true, s(900), after, true), (true, false));
        assert_eq!(offline_alert(true, s(5), after, true), (false, false));
        assert_eq!(offline_alert(false, s(900), after, false), (true, false));
    }

    #[test]
    fn offline_message_follows_cause() {
        let fr = |m: Msg| m.text(Lang::Fr);
        let closed = fr(offline_message(DownKind::PortClosed, Some(30000), true));
        assert!(closed.starts_with("Open API désactivée") && closed.contains("port 30000 fermé"));
        assert!(fr(offline_message(DownKind::Silent, None, false)).starts_with("Batterie absente du réseau Wi-Fi"));
        assert!(fr(offline_message(DownKind::Silent, None, true)).starts_with("Batterie sur le réseau"));
    }

    #[test]
    fn back_alert_fires_once_per_incident() {
        let mut since = Some(Instant::now());
        assert_eq!(back_alert(&mut since, false), None);
        assert!(since.is_some());
        assert!(back_alert(&mut since, true).is_some());
        assert_eq!(back_alert(&mut since, true), None);
        assert_eq!(back_alert(&mut None, true), None);
    }

    #[test]
    fn back_message_reports_outage_and_check() {
        let m = |s, check| back_message(Duration::from_secs(s), check).text(Lang::Fr);
        assert_eq!(m(300, false), "Batterie de nouveau joignable (coupure de 5 min).");
        assert_eq!(m(301, true), "Batterie de nouveau joignable (coupure de 6 min). Vérifie la CT et le planning.");
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

    fn sync_of(plan: &Plan, engine: Engine, mode: Option<&str>, power: Option<f64>, secs: u64, outside: Option<Verdict>) -> Sync {
        sync(&SyncInput { lang: Lang::Fr, plan: Some(plan), week_minute: 9 * 60, engine, testing: false, outside, mode, mode_seen: true, bat_power: power, active_secs: secs, online: true, power_cap: 2500, down: None })
    }

    fn charge_block() -> Plan {
        hybrid_plan(&[(Kind::Charge, 8 * 60, 10 * 60, None)])
    }

    #[test]
    fn sync_is_unknown_without_a_sent_plan_or_offline() {
        let input = |plan, online| SyncInput { lang: Lang::Fr, plan, week_minute: 0, engine: Engine::default(), testing: false, outside: None, mode: Some("Manual"), mode_seen: true, bat_power: Some(0.0), active_secs: 0, online,power_cap: 2500, down: None };
        let reason = |s: Sync| (s.state, s.reason.unwrap_or_default());
        assert_eq!(reason(sync(&input(None, true))), (SyncState::Unknown, "aucun plan envoyé".to_owned()));
        let plan = charge_block();
        assert_eq!(reason(sync(&input(Some(&plan), false))), (SyncState::Unknown, "batterie sans réponse".to_owned()));
        let closed = SyncInput { down: Some(Down::new(Some(49152), Duration::from_secs(5))), ..input(None, false) };
        assert_eq!(sync(&closed).reason.as_deref(), Some("API fermée sur le port 49152"), "la panne prime sur l'absence de plan");
    }

    #[test]
    fn down_reason_names_a_closed_port_or_the_silence() {
        let secs = Duration::from_secs;
        assert_eq!(Down::new(Some(49152), secs(500)).msg().text(Lang::Fr), "API fermée sur le port 49152");
        assert_eq!(Down::new(None, secs(45)).msg().text(Lang::Fr), "batterie sans réponse depuis 45 s");
        assert_eq!(Down::new(None, secs(180)).msg().text(Lang::Fr), "batterie sans réponse depuis 3 min");
        let d = Down::new(Some(49152), secs(500));
        assert_eq!((d.kind, d.port, d.since_secs), (DownKind::PortClosed, Some(49152), 500));
    }

    #[test]
    fn sync_does_not_judge_the_plan_during_app_control() {
        let p = charge_block();
        let manual = Engine { manual_until: Some(FOREVER), ..Engine::default() };
        let rule = Engine { latch: Some(Side::Low), ..Engine::default() };
        let s = sync_of(&p, manual, Some("Auto"), Some(0.0), 0, None);
        assert_eq!((s.state, s.expected.as_str()), (SyncState::Ok, "Action manuelle"));
        assert_eq!(sync_of(&p, rule, Some("UPS"), None, 0, None).expected, "Règle SOC");
        let testing = SyncInput { lang: Lang::Fr, plan: Some(&p), week_minute: 0, engine: Engine::default(), testing: true, outside: None, mode: None, mode_seen: false, bat_power: None, active_secs: 0, online: true, power_cap: 2500, down: None };
        assert_eq!((sync(&testing).state, sync(&testing).expected), (SyncState::Ok, "Test hors créneaux".to_owned()));
    }

    #[test]
    fn sync_flags_a_wrong_mode() {
        let s = sync_of(&charge_block(), Engine::default(), Some("Auto"), Some(-500.0 * f64::from(CHARGE_SIGN)), 0, None);
        assert_eq!((s.state, s.reason.as_deref()), (SyncState::Bad, Some("mode attendu Manual, lu Auto")));
        assert_eq!(sync_of(&charge_block(), Engine::default(), None, None, 0, None).state, SyncState::Warn);
    }

    #[test]
    fn sync_reason_follows_the_language_and_the_journal_stays_french() {
        let input = |lang| SyncInput { lang, ..SyncInput { lang: Lang::Fr, plan: None, week_minute: 0, engine: Engine::default(), testing: false, outside: None, mode: None, mode_seen: false, bat_power: None, active_secs: 0, online: true, power_cap: 2500, down: None } };
        let de = sync(&input(Lang::De));
        assert_eq!((de.reason.as_deref(), de.journal_reason.as_deref()), (Some("kein Plan gesendet"), Some("aucun plan envoyé")));
        let fr = sync(&input(Lang::Fr));
        assert_eq!((fr.reason.as_deref(), fr.journal_reason), (Some("aucun plan envoyé"), None));
    }

    #[test]
    fn sync_does_not_judge_the_plan_in_read_only_mode() {
        let off = Engine { suspended: true, ..Engine::default() };
        let s = sync_of(&charge_block(), off, Some("Auto"), Some(12.0), 0, None);
        assert_eq!((s.state, s.reason.as_deref()), (SyncState::Unknown, Some("lecture seule : plan non suivi")));
        assert_eq!((s.measured.as_str(), s.measured_w), ("Auto · +12 W", Some(12.0)));
        let on = sync_of(&charge_block(), Engine::default(), Some("Auto"), Some(12.0), 0, None);
        assert_eq!(on.state, SyncState::Bad);
    }

    #[test]
    fn unread_mode_is_unknown_until_one_is_read_then_warns_when_stale() {
        let p = charge_block();
        let input = |mode_seen| SyncInput { lang: Lang::Fr, plan: Some(&p), week_minute: 9 * 60, engine: Engine::default(), testing: false, outside: None, mode: None, mode_seen, bat_power: None, active_secs: 0, online: true, power_cap: 2500, down: None };
        assert_eq!(sync(&input(false)).state, SyncState::Unknown);
        assert_eq!(sync(&input(true)).state, SyncState::Warn);
    }

    const MIN: i64 = MINUTE;

    #[test]
    fn plan_is_reapplied_after_two_non_manual_reads_two_minutes_apart() {
        let mut r = Reapply::default();
        assert!(!r.step(Some(("Manual", 0)), true, 0));
        assert!(!r.step(Some(("Auto", 2 * MIN)), true, 2 * MIN), "une seule lecture");
        assert!(!r.step(Some(("Auto", 2 * MIN)), true, 3 * MIN), "même lecture : pas recomptée");
        assert!(r.step(Some(("Auto", 4 * MIN)), true, 4 * MIN));
        assert!(!r.step(Some(("Auto", 6 * MIN)), true, 6 * MIN));
        assert!(!r.step(Some(("Auto", 8 * MIN)), true, 8 * MIN), "au plus une fois par 30 min");
        assert!(r.step(Some(("Auto", 34 * MIN)), true, 34 * MIN));
    }

    #[test]
    fn a_manual_read_or_a_busy_engine_resets_the_count() {
        let mut r = Reapply::default();
        assert!(!r.step(Some(("Auto", 0)), true, 0));
        assert!(!r.step(Some(("Manual", 2 * MIN)), true, 2 * MIN));
        assert!(!r.step(Some(("Auto", 4 * MIN)), true, 4 * MIN));
        assert!(!r.step(Some(("Auto", 6 * MIN)), false, 6 * MIN), "action manuelle ou test en cours");
        assert!(!r.step(Some(("Auto", 8 * MIN)), true, 8 * MIN), "le compte est reparti de zéro");
        assert!(r.step(Some(("Auto", 10 * MIN)), true, 10 * MIN));
    }

    #[test]
    fn plan_is_reapplied_at_once_when_the_battery_comes_back_in_another_mode() {
        let mut r = Reapply::default();
        r.back(10 * MIN);
        assert!(!r.step(Some(("Auto", 8 * MIN)), true, 11 * MIN), "lecture d'avant la panne");
        assert!(r.step(Some(("Auto", 12 * MIN)), true, 12 * MIN));
        let mut r = Reapply::default();
        r.back(10 * MIN);
        assert!(!r.step(Some(("Manual", 12 * MIN)), true, 12 * MIN));
        assert!(!r.step(Some(("Auto", 14 * MIN)), true, 14 * MIN), "la panne est soldée");
    }

    #[test]
    fn sync_checks_block_power_and_sign() {
        let p = charge_block(); // 500 W
        let charge = |w: f64| w * f64::from(CHARGE_SIGN);
        let at = |w, secs| sync_of(&p, Engine::default(), Some("Manual"), Some(w), secs, None).state;
        assert_eq!(at(charge(500.0), 0), SyncState::Ok);
        assert_eq!(at(charge(360.0), 0), SyncState::Ok); // dans les 30 %
        assert_eq!(at(charge(300.0), 0), SyncState::Warn);
        assert_eq!(at(-charge(300.0), 0), SyncState::Bad);
        assert_eq!(at(0.0, 30), SyncState::Ok);
        assert_eq!(at(0.0, 61), SyncState::Warn);
        let low = hybrid_plan(&[(Kind::Discharge, 8 * 60, 10 * 60, None)]);
        let s = sync_of(&low, Engine::default(), Some("Manual"), Some(charge(500.0)), 0, None);
        assert_eq!(s.state, SyncState::Bad); // décharge attendue, charge mesurée
    }

    #[test]
    fn sync_exposes_the_expected_and_measured_power_in_watts() {
        let sign = f64::from(CHARGE_SIGN);
        let w = |s: Sync| (s.expected_w, s.measured_w);
        let manual = |plan: &Plan, power| w(sync_of(plan, Engine::default(), Some("Manual"), Some(power), 0, None));
        let charge = charge_block();
        assert_eq!(manual(&charge, 480.0 * sign), (Some(500.0 * sign), Some(480.0 * sign)));
        let low = hybrid_plan(&[(Kind::Discharge, 8 * 60, 10 * 60, None)]);
        assert_eq!(manual(&low, -sign * 300.0), (Some(-sign * 500.0), Some(-sign * 300.0)));
        let on = Engine { hybrid: Some(Hybrid { drive: Drive::Mode(ModeKind::Auto), until: 1 }), ..Engine::default() };
        assert_eq!(w(sync_of(&charge_block(), on, Some("Auto"), Some(12.0), 0, None)), (None, Some(12.0)));
        assert_eq!(w(sync_of(&charge_block(), Engine::default(), Some("Manual"), None, 0, None)).1, None);
    }

    #[test]
    fn sync_expects_the_capped_power() {
        let p = charge_block(); // 500 W, plafonné à 200 W à l'écriture
        let input = SyncInput { lang: Lang::Fr, plan: Some(&p), week_minute: 9 * 60, engine: Engine::default(), testing: false, outside: None, mode: Some("Manual"), mode_seen: true, bat_power: Some(200.0 * f64::from(CHARGE_SIGN)), active_secs: 0, online: true, power_cap: 200, down: None };
        let s = sync(&input);
        assert_eq!((s.state, s.expected.as_str()), (SyncState::Ok, "Manual · charge 200 W"));
    }

    #[test]
    fn sync_expects_passive_zero_while_the_soc_hold_is_active() {
        let p = hybrid_plan(&[(Kind::Charge, 8 * 60, 10 * 60, Some(90))]);
        let hold = Engine { hybrid: Some(Hybrid { drive: Drive::Hold, until: 1 }), ..Engine::default() };
        assert_eq!(sync_of(&p, hold, Some("Passive"), Some(0.0), 600, None).state, SyncState::Ok);
        assert_eq!(sync_of(&p, hold, Some("Manual"), Some(0.0), 600, None).state, SyncState::Bad);
    }

    #[test]
    fn sync_expects_the_mode_of_a_piloted_block() {
        let p = hybrid_plan(&[(Kind::Ai, 8 * 60, 10 * 60, None)]);
        let on = Engine { hybrid: Some(Hybrid { drive: Drive::Mode(ModeKind::Ai), until: 1 }), ..Engine::default() };
        assert_eq!(sync_of(&p, on, Some("AI"), Some(0.0), 600, None).state, SyncState::Ok);
        assert_eq!(sync_of(&p, on, Some("Manual"), Some(0.0), 600, None).state, SyncState::Bad);
    }

    #[test]
    fn sync_outside_blocks_depends_on_the_known_verdict() {
        let p = hybrid_plan(&[(Kind::Charge, 12 * 60, 13 * 60, None)]);
        let at = |v| sync_of(&p, Engine::default(), Some("Manual"), Some(0.0), 0, v).state;
        assert_eq!(at(Some(Verdict::Idle)), SyncState::Ok);
        assert_eq!(at(Some(Verdict::Follows)), SyncState::Ok);
        assert_eq!(at(None), SyncState::Warn);
    }
}
