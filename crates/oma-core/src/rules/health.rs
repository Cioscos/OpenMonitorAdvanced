//! The rule engine and its health report (spec §3.4–3.5): coverage, lost
//! instances (R2), level entries (R3), suspend gaps (R4) and the display key
//! that decides when the report changes (R7).

use std::collections::BTreeMap;

use std::mem;

use serde::{Deserialize, Serialize};

use super::{
    expand, is_builtin, same_semantics, Condition, Instance, InstanceKey, InstanceProblem, Level,
    Rule, Step, ThresholdSource,
};
use crate::engine::Quality;
use crate::model::{Label, Schema, Unit};

/// Overall level of the report; `Neutral` without alerts and valid data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverallLevel {
    Neutral,
    Ok,
    Warn,
    Crit,
}

/// How many of the observed targets have a value in the latest tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Coverage {
    Complete,
    Partial,
    Unavailable,
}

/// An instance in `warn` or `crit`, current or retained after its sensor
/// was lost (R2). The presentation fields are kept from the last schema that
/// had the sensor, so formatters never need it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    pub rule_id: String,
    pub sensor_id: String,
    pub device_id: String,
    pub unit: Unit,
    pub sensor_label: Label,
    pub level: Level,
    /// Raw value of the last Fresh valid tick, never rounded.
    pub value: Option<f64>,
    /// Threshold of `level`; `None` for flags.
    pub threshold: Option<f64>,
    /// System time the instance entered `level`; for display only.
    pub since_ms: u64,
    /// Whether the latest tick had a value for the sensor.
    pub valid: bool,
    /// System time of the last Fresh valid value; for display only.
    pub last_valid_ms: Option<u64>,
    pub message_key: String,
    pub params: BTreeMap<String, String>,
}

/// A rule/sensor pair.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetRef {
    pub rule_id: String,
    pub sensor_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub level: OverallLevel,
    /// System time of the tick in which `level` last changed.
    pub since_ms: u64,
    /// Goes up by one on every change of the report.
    pub revision: u64,
    pub coverage: Coverage,
    /// Observed targets without a value, sorted by key.
    pub unavailable_targets: Vec<TargetRef>,
    /// Crit first, then the oldest level entry, then rule and sensor id.
    pub alerts: Vec<Alert>,
}

impl Default for HealthReport {
    fn default() -> Self {
        Self {
            level: OverallLevel::Neutral,
            since_ms: 0,
            revision: 0,
            coverage: Coverage::Complete,
            unavailable_targets: Vec::new(),
            alerts: Vec::new(),
        }
    }
}

/// An instance entered a more severe level (R3); `notify` is the rule's
/// setting for that level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelEntry {
    pub rule_id: String,
    pub sensor_id: String,
    pub device_id: String,
    pub level: Level,
    pub notify: bool,
}

/// Result of one [`RuleEngine::evaluate`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Evaluation {
    /// The report, only when it changed.
    pub report: Option<HealthReport>,
    pub entries: Vec<LevelEntry>,
}

/// How long the overall level has lasted, from the monotonic clock, for the
/// report with `revision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthClock {
    pub revision: u64,
    pub level_elapsed_ms: u64,
}

/// The instances of one rule, for the rules settings (R9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleStatus {
    pub rule_id: String,
    pub instances: Vec<InstanceStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceStatus {
    pub sensor_id: String,
    pub level: Level,
    /// Resolved thresholds.
    pub warn: Option<f64>,
    pub crit: Option<f64>,
    /// Where each resolved threshold came from, kept while the instance is
    /// lost (R2); `None` for a level without a threshold.
    pub warn_source: Option<ThresholdSource>,
    pub crit_source: Option<ThresholdSource>,
    pub valid: bool,
    /// `"order"` or `"unitMismatch"`.
    pub problem: Option<String>,
}

/// Smallest gap between two ticks that counts as a suspend (R4).
const SUSPEND_MIN_MS: u64 = 5000;

/// What an alert shows about its sensor, kept after the sensor is lost.
#[derive(Debug, Clone)]
struct Presentation {
    device_id: String,
    device_name: String,
    unit: Unit,
    label: Label,
    message_key: String,
}

/// An instance with what the report needs about it.
#[derive(Debug, Clone)]
struct Slot {
    instance: Instance,
    /// Index of its rule in `RuleEngine::rules`; stale while lost.
    rule: usize,
    present: Presentation,
    /// Raw value and system time of the last Fresh valid tick.
    value: Option<f64>,
    last_valid_ms: Option<u64>,
    /// The latest tick had a value and the instance has no problem.
    available: bool,
    /// When the instance entered its level: system time for display,
    /// monotonic time for ordering.
    since_ms: u64,
    since_mono: u64,
}

/// An instance whose sensor left the schema (R2), with the rule it was
/// expanded from.
#[derive(Debug, Clone)]
struct Lost {
    slot: Slot,
    rule: Rule,
}

/// A current or a lost slot, by index.
#[derive(Debug, Clone, Copy)]
enum Src {
    Current(usize),
    Lost(usize),
}

fn slot<'a>(slots: &'a [Slot], lost: &'a [Lost], src: Src) -> &'a Slot {
    match src {
        Src::Current(i) => &slots[i],
        Src::Lost(i) => &lost[i].slot,
    }
}

/// Everything but `notify` and `enabled`: what decides whether a lost
/// instance still belongs to the rule.
fn same_rule_semantics(a: &Rule, b: &Rule) -> bool {
    a.target == b.target
        && a.unit == b.unit
        && a.condition == b.condition
        && a.warn == b.warn
        && a.crit == b.crit
        && a.hysteresis == b.hysteresis
}

fn message_key(rule: &Rule) -> String {
    if is_builtin(&rule.id) {
        return format!("rule.{}.message", rule.id);
    }
    match rule.condition {
        Condition::Above => "rule.custom.above",
        Condition::Below => "rule.custom.below",
        Condition::FlagActive => "rule.custom.flag",
    }
    .to_owned()
}

fn presentation(schema: &Schema, instance: &Instance, message_key: &str) -> Presentation {
    let sensor = &schema.sensors[instance.sensor_index];
    let device_name = schema
        .devices
        .iter()
        .find(|d| d.id == sensor.device_id)
        .map_or_else(|| sensor.device_id.clone(), |d| d.name.clone());
    Presentation {
        device_id: sensor.device_id.clone(),
        device_name,
        unit: sensor.unit,
        label: sensor.label.clone(),
        message_key: message_key.to_owned(),
    }
}

impl Slot {
    fn new(instance: Instance, rule: usize, present: Presentation, mono: u64, wall: u64) -> Self {
        Self {
            instance,
            rule,
            present,
            value: None,
            last_valid_ms: None,
            available: false,
            since_ms: wall,
            since_mono: mono,
        }
    }

    fn threshold(&self) -> Option<f64> {
        match self.instance.level() {
            Level::Ok => None,
            Level::Warn => self.instance.resolved.warn,
            Level::Crit => self.instance.resolved.crit,
        }
    }

    fn display_key(&self) -> Option<[i64; 4]> {
        self.value.map(|v| display_key(v, self.present.unit))
    }

    /// Whether `alert` shows this slot as it is now, up to the value's
    /// display key and the timestamps.
    fn shown_by(&self, alert: &Alert) -> bool {
        let key = &self.instance.key;
        let present = &self.present;
        alert.rule_id == key.rule_id
            && alert.sensor_id == key.sensor_id
            && alert.level == self.instance.level()
            && alert.valid == self.available
            && alert.threshold == self.threshold()
            && alert.device_id == present.device_id
            && alert.unit == present.unit
            && alert.sensor_label == present.label
            && alert.message_key == present.message_key
            && alert.params.len() == 1
            && alert.params.get("device") == Some(&present.device_name)
    }

    fn alert(&self) -> Alert {
        Alert {
            rule_id: self.instance.key.rule_id.clone(),
            sensor_id: self.instance.key.sensor_id.clone(),
            device_id: self.present.device_id.clone(),
            unit: self.present.unit,
            sensor_label: self.present.label.clone(),
            level: self.instance.level(),
            value: self.value,
            threshold: self.threshold(),
            since_ms: self.since_ms,
            valid: self.available,
            last_valid_ms: self.last_valid_ms,
            message_key: self.present.message_key.clone(),
            params: BTreeMap::from([("device".to_owned(), self.present.device_name.clone())]),
        }
    }

    fn status(&self) -> InstanceStatus {
        InstanceStatus {
            sensor_id: self.instance.key.sensor_id.clone(),
            level: self.instance.level(),
            warn: self.instance.resolved.warn,
            crit: self.instance.resolved.crit,
            warn_source: self.instance.sources.warn,
            crit_source: self.instance.sources.crit,
            valid: self.available,
            problem: self.instance.problem.map(|problem| {
                match problem {
                    InstanceProblem::Order => "order",
                    InstanceProblem::UnitMismatch => "unitMismatch",
                }
                .to_owned()
            }),
        }
    }
}

/// Evaluates the rules on every tick and keeps the health report.
///
/// Instances are expanded again only when the schema or the rules change;
/// a steady tick reuses every buffer and does not allocate.
#[derive(Debug)]
pub struct RuleEngine {
    rules: Vec<Rule>,
    /// Rules set since the last evaluation, applied by the next one.
    pending: Option<Vec<Rule>>,
    interval_ms: u64,
    slots: Vec<Slot>,
    /// Sorted by key.
    lost: Vec<Lost>,
    /// Schema revision of the expansion; `None` before the first one.
    schema_revision: Option<u64>,
    /// Monotonic and system time of the previous tick.
    previous: Option<(u64, u64)>,
    /// The latest published report, with values updated in place.
    report: HealthReport,
    /// Display keys of the published alerts, in the same order.
    published_keys: Vec<Option<[i64; 4]>>,
    /// Monotonic time of the latest change of the overall level.
    level_mono: u64,
    // Scratch buffers of every evaluation: the alerts and the unavailable
    // targets of the tick, in report order.
    alerts: Vec<Src>,
    unavailable: Vec<Src>,
}

impl Default for RuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleEngine {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            pending: None,
            interval_ms: 1000,
            slots: Vec::new(),
            lost: Vec::new(),
            schema_revision: None,
            previous: None,
            report: HealthReport::default(),
            published_keys: Vec::new(),
            level_mono: 0,
            alerts: Vec::new(),
            unavailable: Vec::new(),
        }
    }

    /// Takes effect on the next [`evaluate`](Self::evaluate). Instances
    /// whose semantics do not change keep their state, lost ones included.
    pub fn set_rules(&mut self, rules: Vec<Rule>) {
        self.pending = Some(rules);
    }

    /// The sampling interval, for the suspend bound (R4).
    pub fn set_interval_ms(&mut self, interval_ms: u64) {
        self.interval_ms = interval_ms;
    }

    /// Evaluates one tick. `values` and `quality` are aligned with
    /// `schema.sensors`; `monotonic_ms` drives the timers and durations,
    /// `timestamp_ms` (system time) is only shown. The first evaluation
    /// always returns a report.
    pub fn evaluate(
        &mut self,
        schema: &Schema,
        schema_changed: bool,
        values: &[Option<f64>],
        quality: &[Quality],
        monotonic_ms: u64,
        timestamp_ms: u64,
    ) -> Evaluation {
        if schema_changed || self.pending.is_some() || self.schema_revision != Some(schema.revision)
        {
            self.expand(schema, monotonic_ms, timestamp_ms);
        }

        // R4: time not observed does not prove a condition held.
        let bound = self.interval_ms.saturating_mul(3).max(SUSPEND_MIN_MS);
        if let Some((mono, wall)) = self.previous {
            if monotonic_ms.saturating_sub(mono) > bound || timestamp_ms.abs_diff(wall) > bound {
                for slot in &mut self.slots {
                    slot.instance.reset_timers();
                }
            }
        }
        self.previous = Some((monotonic_ms, timestamp_ms));

        let mut entries = Vec::new();
        for slot in &mut self.slots {
            let rule = &self.rules[slot.rule];
            let index = slot.instance.sensor_index;
            let value = values
                .get(index)
                .copied()
                .flatten()
                .filter(|v| v.is_finite());
            let quality = quality.get(index).copied().unwrap_or(Quality::Fresh);
            let step = slot.instance.step(rule, value, quality, monotonic_ms);
            if step != Step::Stay {
                slot.since_ms = timestamp_ms;
                slot.since_mono = monotonic_ms;
            }
            // R3: only a more severe level is an entry.
            if let Step::Entered(level) = step {
                entries.push(LevelEntry {
                    rule_id: slot.instance.key.rule_id.clone(),
                    sensor_id: slot.instance.key.sensor_id.clone(),
                    device_id: slot.present.device_id.clone(),
                    level,
                    notify: match level {
                        Level::Ok => false,
                        Level::Warn => rule.notify.warn,
                        Level::Crit => rule.notify.crit,
                    },
                });
            }
            slot.available = slot.instance.problem.is_none() && value.is_some();
            if value.is_some() && quality == Quality::Fresh {
                slot.value = value;
                slot.last_valid_ms = Some(timestamp_ms);
            }
        }

        self.collect();
        let (level, coverage) = self.verdict();
        let report = if self.is_published(level, coverage) {
            self.update_in_place();
            None
        } else {
            Some(self.publish(level, coverage, monotonic_ms, timestamp_ms))
        };
        Evaluation { report, entries }
    }

    /// The latest report: the published one, with the raw values and
    /// `last_valid_ms` of its alerts brought up to the latest tick.
    pub fn report(&self) -> &HealthReport {
        &self.report
    }

    /// How long the overall level has lasted at `monotonic_ms`.
    pub fn clock(&self, monotonic_ms: u64) -> HealthClock {
        HealthClock {
            revision: self.report.revision,
            level_elapsed_ms: monotonic_ms.saturating_sub(self.level_mono),
        }
    }

    /// Every rule, in order, with its current instances in expansion order
    /// and then its lost ones by sensor id (R9).
    pub fn status(&self) -> Vec<RuleStatus> {
        let rules = self.pending.as_ref().unwrap_or(&self.rules);
        rules
            .iter()
            .map(|rule| {
                let current = self
                    .slots
                    .iter()
                    .filter(|s| s.instance.key.rule_id == rule.id);
                let lost = self
                    .lost
                    .iter()
                    .map(|l| &l.slot)
                    .filter(|s| s.instance.key.rule_id == rule.id);
                RuleStatus {
                    rule_id: rule.id.clone(),
                    instances: current.chain(lost).map(Slot::status).collect(),
                }
            })
            .collect()
    }

    /// Expands the rules on `schema`, keeping the state of the instances
    /// with the same semantics, and moves the instances whose sensor left
    /// the schema to the lost ones (R2). A lost instance resumes with its
    /// level and fresh timers when its sensor returns with the same
    /// semantics; it is dropped when its rule is disabled or changes.
    fn expand(&mut self, schema: &Schema, mono: u64, wall: u64) {
        let old_rules = mem::take(&mut self.rules);
        let rules = self.pending.take().unwrap_or_else(|| old_rules.clone());
        let mut old_slots = mem::take(&mut self.slots);
        let mut old_lost = mem::take(&mut self.lost);

        let mut slots = Vec::new();
        for (rule_index, rule) in rules.iter().enumerate() {
            let message_key = message_key(rule);
            for instance in expand(std::slice::from_ref(rule), schema) {
                let present = presentation(schema, &instance, &message_key);
                let kept = if let Some(pos) = old_slots
                    .iter()
                    .position(|s| s.instance.key == instance.key)
                {
                    let old = old_slots.swap_remove(pos);
                    same_semantics(&old.instance, &instance, &old_rules[old.rule], rule)
                        .then_some(old)
                } else if let Some(pos) = old_lost
                    .iter()
                    .position(|l| l.slot.instance.key == instance.key)
                {
                    let lost = old_lost.swap_remove(pos);
                    same_semantics(&lost.slot.instance, &instance, &lost.rule, rule).then(|| {
                        let mut slot = lost.slot;
                        slot.instance.reset_timers();
                        slot
                    })
                } else {
                    None
                };
                slots.push(match kept {
                    Some(mut slot) => {
                        slot.instance.sensor_index = instance.sensor_index;
                        // Same numbers, maybe from another source now.
                        slot.instance.sources = instance.sources;
                        slot.rule = rule_index;
                        slot.present = present;
                        slot
                    }
                    None => Slot::new(instance, rule_index, present, mono, wall),
                });
            }
        }

        // What did not match: lost if its sensor is gone and its rule is
        // still enabled with the same semantics, dropped otherwise.
        let in_schema = |sensor_id: &str| schema.sensors.iter().any(|s| s.id == sensor_id);
        let keeps = |key: &InstanceKey, old: &Rule| {
            rules
                .iter()
                .find(|r| r.id == key.rule_id && r.enabled && same_rule_semantics(old, r))
                .cloned()
        };
        let mut lost = Vec::new();
        for mut slot in old_slots {
            if in_schema(&slot.instance.key.sensor_id) {
                continue;
            }
            if let Some(rule) = keeps(&slot.instance.key, &old_rules[slot.rule]) {
                slot.available = false;
                lost.push(Lost { slot, rule });
            }
        }
        for mut entry in old_lost {
            if in_schema(&entry.slot.instance.key.sensor_id) {
                continue;
            }
            if let Some(rule) = keeps(&entry.slot.instance.key, &entry.rule) {
                entry.rule = rule;
                lost.push(entry);
            }
        }
        lost.sort_unstable_by(|a, b| a.slot.instance.key.cmp(&b.slot.instance.key));

        self.rules = rules;
        self.slots = slots;
        self.lost = lost;
        self.schema_revision = Some(schema.revision);
    }

    /// Fills the scratch lists of alerts and unavailable targets, in report
    /// order. Sorts in place: no allocation once the buffers have grown.
    fn collect(&mut self) {
        self.alerts.clear();
        self.unavailable.clear();
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.instance.level() > Level::Ok {
                self.alerts.push(Src::Current(i));
            }
            if !slot.available {
                self.unavailable.push(Src::Current(i));
            }
        }
        for (i, lost) in self.lost.iter().enumerate() {
            if lost.slot.instance.level() > Level::Ok {
                self.alerts.push(Src::Lost(i));
            }
            self.unavailable.push(Src::Lost(i));
        }

        let (slots, lost) = (&self.slots, &self.lost);
        // Keys are unique, so the unstable sorts are deterministic.
        self.alerts.sort_unstable_by(|&a, &b| {
            let (a, b) = (slot(slots, lost, a), slot(slots, lost, b));
            b.instance
                .level()
                .cmp(&a.instance.level())
                .then(a.since_mono.cmp(&b.since_mono))
                .then_with(|| a.instance.key.cmp(&b.instance.key))
        });
        let key = |src: Src| &slot(slots, lost, src).instance.key;
        self.unavailable
            .sort_unstable_by(|&a, &b| key(a).cmp(key(b)));
        self.unavailable.dedup_by(|a, b| key(*a) == key(*b));
    }

    /// Overall level and coverage of the collected tick.
    fn verdict(&self) -> (OverallLevel, Coverage) {
        let available = self.slots.iter().filter(|s| s.available).count();
        let worst = self
            .alerts
            .first()
            .map(|&src| slot(&self.slots, &self.lost, src).instance.level());
        let level = match worst {
            Some(Level::Crit) => OverallLevel::Crit,
            Some(Level::Warn) => OverallLevel::Warn,
            _ if available > 0 => OverallLevel::Ok,
            _ => OverallLevel::Neutral,
        };
        let coverage = if self.unavailable.is_empty() {
            Coverage::Complete
        } else if available == 0 {
            Coverage::Unavailable
        } else {
            Coverage::Partial
        };
        (level, coverage)
    }

    /// Whether the published report already shows the collected tick, with
    /// the published display keys (R7). Does not allocate.
    fn is_published(&self, level: OverallLevel, coverage: Coverage) -> bool {
        let report = &self.report;
        if report.revision == 0
            || report.level != level
            || report.coverage != coverage
            || report.unavailable_targets.len() != self.unavailable.len()
            || report.alerts.len() != self.alerts.len()
        {
            return false;
        }
        let (slots, lost) = (&self.slots, &self.lost);
        let targets = report
            .unavailable_targets
            .iter()
            .zip(&self.unavailable)
            .all(|(target, &src)| {
                let key = &slot(slots, lost, src).instance.key;
                target.rule_id == key.rule_id && target.sensor_id == key.sensor_id
            });
        targets
            && report
                .alerts
                .iter()
                .zip(&self.published_keys)
                .zip(&self.alerts)
                .all(|((alert, published), &src)| {
                    let slot = slot(slots, lost, src);
                    slot.shown_by(alert) && *published == slot.display_key()
                })
    }

    fn update_in_place(&mut self) {
        let (slots, lost) = (&self.slots, &self.lost);
        for (alert, &src) in self.report.alerts.iter_mut().zip(&self.alerts) {
            let slot = slot(slots, lost, src);
            alert.value = slot.value;
            alert.last_valid_ms = slot.last_valid_ms;
        }
    }

    fn publish(
        &mut self,
        level: OverallLevel,
        coverage: Coverage,
        mono: u64,
        wall: u64,
    ) -> HealthReport {
        let (slots, lost) = (&self.slots, &self.lost);
        let unavailable_targets = self
            .unavailable
            .iter()
            .map(|&src| {
                let key = &slot(slots, lost, src).instance.key;
                TargetRef {
                    rule_id: key.rule_id.clone(),
                    sensor_id: key.sensor_id.clone(),
                }
            })
            .collect();
        let alerts = self
            .alerts
            .iter()
            .map(|&src| slot(slots, lost, src).alert())
            .collect();
        self.published_keys = self
            .alerts
            .iter()
            .map(|&src| slot(slots, lost, src).display_key())
            .collect();

        let report = &mut self.report;
        if report.revision == 0 || report.level != level {
            report.since_ms = wall;
            self.level_mono = mono;
        }
        report.level = level;
        report.coverage = coverage;
        report.revision += 1;
        report.unavailable_targets = unavailable_targets;
        report.alerts = alerts;
        report.clone()
    }
}

/// Rounds half away from zero like the UI's `Intl.NumberFormat`; the cast
/// saturates.
fn round(value: f64) -> i64 {
    value.round() as i64
}

/// Step and rounded digits of `value` shown in steps of `base` up to
/// `last_step`, with `digits(step, scaled)` decimals, as
/// `[step * 4 + digits, rounded]`: the text depends on all three.
fn stepped(mut value: f64, base: f64, last_step: usize, digits: fn(usize, f64) -> i32) -> [i64; 2] {
    let mut step = 0;
    while value.abs() >= base && step < last_step {
        value /= base;
        step += 1;
    }
    let digits = digits(step, value);
    [
        step as i64 * 4 + i64::from(digits),
        round(value * 10f64.powi(digits)),
    ]
}

/// `formatBytes`: binary steps, B to TB, one decimal below 100 past bytes.
fn bytes_key(value: f64) -> [i64; 2] {
    stepped(value, 1024.0, 4, |step, v| {
        i32::from(step != 0 && v < 100.0)
    })
}

/// `formatRate` in bits: decimal steps, bit/s to Tbit/s, one decimal below 10.
fn bits_key(bytes_per_second: f64) -> [i64; 2] {
    stepped(bytes_per_second * 8.0, 1000.0, 4, |_, v| {
        i32::from(v < 10.0)
    })
}

/// Display key of `value` in `unit` (R7): values with the same key format to
/// the same text in every display unit (°C and °F, bytes and bits), so the
/// report changes only when a formatted value would. Mirrors `formatValue`
/// in `app/src/lib/format.ts`, the most precise formatter; the tray
/// (`app/src-tauri/src/tray_icon.rs`) shows whole numbers only. Floating
/// point ties may round differently from `Intl.NumberFormat`.
fn display_key(value: f64, unit: Unit) -> [i64; 4] {
    let pair = |[a, b]: [i64; 2], [c, d]: [i64; 2]| [a, b, c, d];
    match unit {
        Unit::Celsius => [round(value), round(value * 9.0 / 5.0 + 32.0), 0, 0],
        Unit::Percent | Unit::Watt | Unit::Rpm | Unit::Hours | Unit::Count => {
            [round(value), 0, 0, 0]
        }
        Unit::Megahertz if value >= 1000.0 => [1, round(value / 1000.0 * 100.0), 0, 0],
        Unit::Megahertz => [0, round(value), 0, 0],
        Unit::Volt => [round(value * 1000.0), 0, 0, 0],
        Unit::Ampere => [round(value * 10.0), 0, 0, 0],
        Unit::Bytes => pair(bytes_key(value), [0, 0]),
        // Throughput is shown in bytes or in bits, as the settings say.
        Unit::BytesPerSecond => pair(bytes_key(value), bits_key(value)),
        Unit::BitsPerSecond => pair(bytes_key(value / 8.0), bits_key(value / 8.0)),
        // `formatEnergy`: decimal steps, J to GJ, decimals like bytes.
        Unit::Joule => pair(
            stepped(value, 1000.0, 3, |step, v| {
                i32::from(step != 0 && v < 100.0)
            }),
            [0, 0],
        ),
        Unit::Boolean => [i64::from(value >= 0.5), 0, 0, 0],
        // `Math.round`: half up.
        Unit::PcieGeneration | Unit::Lanes => [(value + 0.5).floor() as i64, 0, 0, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, DeviceKind, Sensor, SensorKind, Source};
    use crate::rules::{
        default_rules, Condition, Hysteresis, LevelSpec, Notify, Target, Threshold,
    };
    use serde_json::json;
    use Quality::{Fresh, Held};

    const WALL0: u64 = 1_700_000_000_000;
    const GPU0: &str = "gpu/0/temperature/core";
    const GPU1: &str = "gpu/1/temperature/core";
    const GPU2: &str = "gpu/2/temperature/core";
    const HOTSPOT0: &str = "gpu/0/temperature/hotspot";
    const CUSTOM: &str = "custom-00000000-0000-4000-8000-000000000001";

    fn device(id: &str, kind: DeviceKind, name: &str, properties: &[(&str, &str)]) -> Device {
        Device {
            id: id.into(),
            kind,
            name: name.into(),
            vendor: None,
            properties: properties
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    fn gpu(index: usize) -> Device {
        device(
            &format!("gpu/{index}"),
            DeviceKind::Gpu,
            &format!("GPU {index}"),
            &[],
        )
    }

    fn core(index: usize) -> Sensor {
        Sensor::new(
            &format!("gpu/{index}"),
            SensorKind::Temperature,
            "core",
            Unit::Celsius,
            Label::new("gpu.core"),
            Source::Mock,
        )
    }

    fn schema(devices: Vec<Device>, sensors: Vec<Sensor>) -> Schema {
        Schema {
            revision: 1,
            devices,
            sensors,
        }
    }

    /// `n` GPUs with a core temperature each.
    fn gpus(n: usize) -> Schema {
        schema((0..n).map(gpu).collect(), (0..n).map(core).collect())
    }

    fn builtin(id: &str) -> Rule {
        default_rules()
            .into_iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no rule {id}"))
    }

    fn fixed(value: f64, duration_s: u32) -> Option<LevelSpec> {
        Some(LevelSpec {
            threshold: Some(Threshold::Fixed { fixed: value }),
            duration_s,
        })
    }

    /// `gpu-temp` (83 / 90) with the given durations and hysteresis.
    fn gpu_temp(warn_s: u32, crit_s: u32, (amount, duration_s): (f64, u32)) -> Rule {
        let mut rule = builtin("gpu-temp");
        rule.warn = fixed(83.0, warn_s);
        rule.crit = fixed(90.0, crit_s);
        rule.hysteresis = Hysteresis { amount, duration_s };
        rule
    }

    /// `gpu-temp` that enters and leaves on the first tick.
    fn instant() -> Rule {
        gpu_temp(0, 0, (3.0, 0))
    }

    fn custom(id: &str, sensor: &str, unit: Unit, condition: Condition) -> Rule {
        Rule {
            id: id.into(),
            target: Target::Sensor {
                sensor: sensor.into(),
            },
            unit,
            condition,
            warn: match condition {
                Condition::FlagActive => Some(LevelSpec {
                    threshold: None,
                    duration_s: 0,
                }),
                _ => fixed(50.0, 0),
            },
            crit: None,
            hysteresis: Hysteresis::default(),
            enabled: true,
            notify: Notify::default(),
        }
    }

    fn target(rule_id: &str, sensor_id: &str) -> TargetRef {
        TargetRef {
            rule_id: rule_id.into(),
            sensor_id: sensor_id.into(),
        }
    }

    fn keys(report: &HealthReport) -> Vec<(String, String, Level)> {
        report
            .alerts
            .iter()
            .map(|a| (a.rule_id.clone(), a.sensor_id.clone(), a.level))
            .collect()
    }

    fn key(rule_id: &str, sensor_id: &str, level: Level) -> (String, String, Level) {
        (rule_id.into(), sensor_id.into(), level)
    }

    /// An engine fed one tick per second: monotonic `ms`, wall `WALL0 + ms`.
    struct Rig {
        engine: RuleEngine,
        schema: Schema,
        changed: bool,
        ms: u64,
    }

    impl Rig {
        fn new(rules: Vec<Rule>, schema: Schema) -> Self {
            let mut engine = RuleEngine::new();
            engine.set_interval_ms(1000);
            engine.set_rules(rules);
            Self {
                engine,
                schema,
                changed: true,
                ms: 0,
            }
        }

        fn tick(&mut self, values: &[Option<f64>]) -> Evaluation {
            let quality = vec![Fresh; values.len()];
            self.tick_with(values, &quality)
        }

        fn tick_with(&mut self, values: &[Option<f64>], quality: &[Quality]) -> Evaluation {
            let out = self.engine.evaluate(
                &self.schema,
                self.changed,
                values,
                quality,
                self.ms,
                WALL0 + self.ms,
            );
            self.changed = false;
            self.ms += 1000;
            out
        }

        fn set_schema(&mut self, mut schema: Schema) {
            schema.revision = self.schema.revision + 1;
            self.schema = schema;
            self.changed = true;
        }

        fn report(&self) -> &HealthReport {
            self.engine.report()
        }
    }

    // --- level and coverage ----------------------------------------------

    #[test]
    fn no_rules_and_no_data_is_neutral() {
        let mut rig = Rig::new(vec![], gpus(1));
        let report = rig.tick(&[Some(50.0)]).report.expect("first tick reports");
        assert_eq!(report.level, OverallLevel::Neutral);
        assert_eq!(report.coverage, Coverage::Complete);
        assert!(report.alerts.is_empty());
        assert!(report.unavailable_targets.is_empty());
        assert_eq!(report.revision, 1);
        assert_eq!(report.since_ms, WALL0);
        assert_eq!(rig.report(), &report);
        assert_eq!(rig.tick(&[Some(50.0)]).report, None);

        // Rules whose sensors never appeared.
        let mut rig = Rig::new(default_rules(), schema(vec![], vec![]));
        let report = rig.tick(&[]).report.expect("first tick reports");
        assert_eq!(
            (report.level, report.coverage),
            (OverallLevel::Neutral, Coverage::Complete)
        );
    }

    #[test]
    fn all_clear_with_full_coverage_is_ok() {
        let mut rig = Rig::new(vec![builtin("gpu-temp")], gpus(2));
        let report = rig.tick(&[Some(50.0), Some(60.0)]).report.unwrap();
        assert_eq!(
            (report.level, report.coverage),
            (OverallLevel::Ok, Coverage::Complete)
        );
        assert!(report.alerts.is_empty());
        assert!(report.unavailable_targets.is_empty());
    }

    #[test]
    fn worst_alert_sets_the_level_and_alerts_are_sorted() {
        let hotspot = Sensor::new(
            "gpu/0",
            SensorKind::Temperature,
            "hotspot",
            Unit::Celsius,
            Label::new("gpu.hotspot"),
            Source::Mock,
        );
        let schema = schema(vec![gpu(0), gpu(1)], vec![core(0), hotspot, core(1)]);
        let mut hotspot_rule = builtin("gpu-hotspot");
        hotspot_rule.warn = fixed(95.0, 0);
        hotspot_rule.crit = fixed(105.0, 0);
        let mut rig = Rig::new(vec![instant(), hotspot_rule], schema);

        let first = rig.tick(&[Some(50.0), Some(60.0), Some(85.0)]);
        assert_eq!(first.report.unwrap().level, OverallLevel::Warn);
        rig.tick(&[Some(85.0), Some(60.0), Some(85.0)]);
        let report = rig
            .tick(&[Some(85.0), Some(110.0), Some(85.0)])
            .report
            .unwrap();
        assert_eq!(report.level, OverallLevel::Crit);
        assert_eq!(report.coverage, Coverage::Complete);
        assert_eq!(
            keys(&report),
            [
                key("gpu-hotspot", HOTSPOT0, Level::Crit),
                key("gpu-temp", GPU1, Level::Warn),
                key("gpu-temp", GPU0, Level::Warn),
            ]
        );
        assert_eq!(
            report.alerts[1],
            Alert {
                rule_id: "gpu-temp".into(),
                sensor_id: GPU1.into(),
                device_id: "gpu/1".into(),
                unit: Unit::Celsius,
                sensor_label: Label::new("gpu.core"),
                level: Level::Warn,
                value: Some(85.0),
                threshold: Some(83.0),
                since_ms: WALL0,
                valid: true,
                last_valid_ms: Some(WALL0 + 2000),
                message_key: "rule.gpu-temp.message".into(),
                params: BTreeMap::from([("device".to_owned(), "GPU 1".to_owned())]),
            }
        );
        assert_eq!(report.alerts[0].threshold, Some(105.0));
        assert_eq!(report.alerts[0].message_key, "rule.gpu-hotspot.message");
        assert_eq!(report.alerts[0].since_ms, WALL0 + 2000);
    }

    #[test]
    fn custom_rules_use_generic_messages() {
        let flag = Sensor::new(
            "gpu/0",
            SensorKind::Flag,
            "throttle-thermal",
            Unit::Boolean,
            Label::new("gpu.throttle"),
            Source::Mock,
        );
        let flag_id = flag.id.clone();
        let schema = schema(vec![gpu(0)], vec![core(0), flag]);
        let above = custom(CUSTOM, GPU0, Unit::Celsius, Condition::Above);
        let mut below = custom(
            "custom-00000000-0000-4000-8000-000000000002",
            GPU0,
            Unit::Celsius,
            Condition::Below,
        );
        below.warn = fixed(60.0, 0);
        let flagged = custom(
            "custom-00000000-0000-4000-8000-000000000003",
            &flag_id,
            Unit::Boolean,
            Condition::FlagActive,
        );
        let mut rig = Rig::new(vec![above, below, flagged], schema);
        let report = rig.tick(&[Some(55.0), Some(1.0)]).report.unwrap();
        let messages: Vec<(&str, &str)> = report
            .alerts
            .iter()
            .map(|a| (a.rule_id.as_str(), a.message_key.as_str()))
            .collect();
        assert_eq!(
            messages,
            [
                (CUSTOM, "rule.custom.above"),
                (
                    "custom-00000000-0000-4000-8000-000000000002",
                    "rule.custom.below"
                ),
                (
                    "custom-00000000-0000-4000-8000-000000000003",
                    "rule.custom.flag"
                ),
            ]
        );
        assert_eq!(report.alerts[2].threshold, None);
    }

    #[test]
    fn absent_values_make_coverage_partial_not_ok() {
        let mut rig = Rig::new(vec![builtin("gpu-temp")], gpus(2));
        let report = rig.tick(&[Some(50.0), None]).report.unwrap();
        assert_eq!(
            (report.level, report.coverage),
            (OverallLevel::Ok, Coverage::Partial)
        );
        assert_eq!(report.unavailable_targets, [target("gpu-temp", GPU1)]);
        // A non-finite value is absent too.
        assert_eq!(rig.tick(&[Some(50.0), Some(f64::NAN)]).report, None);
    }

    #[test]
    fn all_observed_targets_unavailable_is_unavailable() {
        let mut rig = Rig::new(vec![builtin("gpu-temp")], gpus(2));
        let report = rig.tick(&[None, None]).report.unwrap();
        assert_eq!(
            (report.level, report.coverage),
            (OverallLevel::Neutral, Coverage::Unavailable)
        );
        assert_eq!(
            report.unavailable_targets,
            [target("gpu-temp", GPU0), target("gpu-temp", GPU1)]
        );

        // An explicit target in another unit.
        let mismatch = custom(CUSTOM, GPU0, Unit::Percent, Condition::Above);
        let mut rig = Rig::new(vec![mismatch], gpus(1));
        let report = rig.tick(&[Some(99.0)]).report.unwrap();
        assert_eq!(
            (report.level, report.coverage),
            (OverallLevel::Neutral, Coverage::Unavailable)
        );
        assert_eq!(report.unavailable_targets, [target(CUSTOM, GPU0)]);

        // Thresholds in the wrong order after resolution.
        let disk = schema(
            vec![device(
                "storage/0",
                DeviceKind::Storage,
                "SSD",
                &[("tempWarningC", "90"), ("tempCriticalC", "80")],
            )],
            vec![Sensor::new(
                "storage/0",
                SensorKind::Temperature,
                "drive",
                Unit::Celsius,
                Label::new("storage.temperature"),
                Source::Mock,
            )],
        );
        let mut rig = Rig::new(vec![builtin("disk-temp")], disk);
        let report = rig.tick(&[Some(99.0)]).report.unwrap();
        assert_eq!(
            (report.level, report.coverage),
            (OverallLevel::Neutral, Coverage::Unavailable)
        );
        assert_eq!(
            report.unavailable_targets,
            [target("disk-temp", "storage/0/temperature/drive")]
        );
    }

    #[test]
    fn unavailable_target_replacement_emits_report() {
        let mut rig = Rig::new(vec![builtin("gpu-temp")], gpus(3));
        let first = rig.tick(&[None, Some(50.0), Some(50.0)]).report.unwrap();
        assert_eq!(first.coverage, Coverage::Partial);
        assert_eq!(first.unavailable_targets, [target("gpu-temp", GPU0)]);
        let second = rig.tick(&[Some(50.0), None, Some(50.0)]).report.unwrap();
        assert_eq!(second.coverage, Coverage::Partial);
        assert_eq!(second.unavailable_targets, [target("gpu-temp", GPU1)]);
        assert_eq!(second.revision, first.revision + 1);
        assert_eq!(rig.tick(&[Some(50.0), None, Some(50.0)]).report, None);
        let third = rig.tick(&[Some(50.0), None, None]).report.unwrap();
        assert_eq!(
            third.unavailable_targets,
            [target("gpu-temp", GPU1), target("gpu-temp", GPU2)]
        );
    }

    #[test]
    fn never_seen_optional_targets_do_not_degrade_coverage() {
        let mut rules = default_rules();
        rules.push(custom(
            CUSTOM,
            "gpu/9/temperature/core",
            Unit::Celsius,
            Condition::Above,
        ));
        let mut rig = Rig::new(rules, gpus(1));
        let report = rig.tick(&[Some(50.0)]).report.unwrap();
        assert_eq!(
            (report.level, report.coverage),
            (OverallLevel::Ok, Coverage::Complete)
        );
        assert!(report.unavailable_targets.is_empty());
    }

    // --- lost instances (R2) ---------------------------------------------

    /// `gpus(2)` without the sensor of GPU 0; with `keep_device` its device
    /// stays.
    fn without_gpu0(keep_device: bool) -> Schema {
        let devices = if keep_device {
            vec![gpu(0), gpu(1)]
        } else {
            vec![gpu(1)]
        };
        schema(devices, vec![core(1)])
    }

    #[test]
    fn lost_sensor_keeps_alarm_as_retained() {
        let mut rig = Rig::new(vec![instant()], gpus(2));
        let crit = rig.tick(&[Some(95.0), Some(50.0)]);
        assert_eq!(crit.report.unwrap().level, OverallLevel::Crit);

        rig.set_schema(without_gpu0(true));
        let out = rig.tick(&[Some(50.0)]);
        assert!(out.entries.is_empty());
        let report = out.report.expect("the loss is reported");
        assert_eq!(report.level, OverallLevel::Crit);
        assert_eq!(report.coverage, Coverage::Partial);
        assert_eq!(report.unavailable_targets, [target("gpu-temp", GPU0)]);
        assert_eq!(keys(&report), [key("gpu-temp", GPU0, Level::Crit)]);
        let alert = &report.alerts[0];
        assert!(!alert.valid);
        assert_eq!(alert.value, Some(95.0));
        assert_eq!(alert.last_valid_ms, Some(WALL0));
        assert_eq!(alert.since_ms, WALL0);
        assert_eq!(rig.tick(&[Some(50.0)]).report, None);

        // The only instance: nothing is available any more.
        let mut rig = Rig::new(vec![instant()], gpus(1));
        rig.tick(&[Some(95.0)]);
        rig.set_schema(schema(vec![gpu(0)], vec![]));
        let report = rig.tick(&[]).report.unwrap();
        assert_eq!(report.level, OverallLevel::Crit);
        assert_eq!(report.coverage, Coverage::Unavailable);
        assert_eq!(report.unavailable_targets, [target("gpu-temp", GPU0)]);
        assert!(!report.alerts[0].valid);
    }

    /// `cpu/0` with `properties` and, with `sensor`, its Tctl temperature.
    fn cpu_schema(properties: &[(&str, &str)], sensor: bool) -> Schema {
        let tctl = Sensor::new(
            "cpu/0",
            SensorKind::Temperature,
            "tctl",
            Unit::Celsius,
            Label::new("cpu.tctl"),
            Source::Mock,
        );
        schema(
            vec![device("cpu/0", DeviceKind::Cpu, "CPU", properties)],
            if sensor { vec![tctl] } else { vec![] },
        )
    }

    #[test]
    fn retained_instance_keeps_threshold_sources() {
        use ThresholdSource::{Fallback, Property};
        let sources = |engine: &RuleEngine| {
            let status = engine.status();
            let i = &status[0].instances[0];
            (i.warn, i.crit, i.warn_source, i.crit_source, i.valid)
        };
        // The service stops: the sensor and the property go, the thresholds
        // resolved from the property stay with their source.
        let mut rig = Rig::new(
            vec![builtin("cpu-temp")],
            cpu_schema(&[("tjMaxC", "89")], true),
        );
        rig.tick(&[Some(50.0)]);
        let resolved = (Some(79.0), Some(89.0), Some(Property), Some(Property));
        assert_eq!(
            sources(&rig.engine),
            (resolved.0, resolved.1, resolved.2, resolved.3, true)
        );
        rig.set_schema(cpu_schema(&[], false));
        rig.tick(&[]);
        assert_eq!(
            sources(&rig.engine),
            (resolved.0, resolved.1, resolved.2, resolved.3, false)
        );

        // Same numbers from the fallback keep the state, but not the source.
        let mut rig = Rig::new(
            vec![builtin("cpu-temp")],
            cpu_schema(&[("tjMaxC", "95")], true),
        );
        rig.tick(&[Some(50.0)]);
        rig.set_schema(cpu_schema(&[], true));
        rig.tick(&[Some(50.0)]);
        assert_eq!(
            sources(&rig.engine),
            (Some(85.0), Some(95.0), Some(Fallback), Some(Fallback), true)
        );
    }

    #[test]
    fn lost_ok_instance_is_only_unavailable() {
        let mut rig = Rig::new(vec![instant()], gpus(2));
        rig.tick(&[Some(50.0), Some(50.0)]);
        rig.set_schema(without_gpu0(true));
        let report = rig.tick(&[Some(50.0)]).report.unwrap();
        assert_eq!(
            (report.level, report.coverage),
            (OverallLevel::Ok, Coverage::Partial)
        );
        assert!(report.alerts.is_empty());
        assert_eq!(report.unavailable_targets, [target("gpu-temp", GPU0)]);

        // Back: complete again.
        rig.set_schema(gpus(2));
        let report = rig.tick(&[Some(50.0), Some(50.0)]).report.unwrap();
        assert_eq!(report.coverage, Coverage::Complete);
        assert!(report.unavailable_targets.is_empty());
    }

    #[test]
    fn returning_sensor_resumes_level_without_entry() {
        // Exit needs < 87 for 10 s.
        let mut rig = Rig::new(vec![gpu_temp(0, 0, (3.0, 10))], gpus(2));
        let entered = rig.tick(&[Some(95.0), Some(50.0)]);
        assert_eq!(entered.entries.len(), 1);
        // The exit timer runs for 7 s (1 s .. 8 s).
        for _ in 1..=8 {
            assert!(rig.tick(&[Some(70.0), Some(50.0)]).entries.is_empty());
        }
        assert_eq!(rig.report().level, OverallLevel::Crit);

        rig.set_schema(without_gpu0(true));
        assert!(rig.tick(&[Some(50.0)]).entries.is_empty());

        // Back at 10 s with the same semantics: crit again, valid, no entry.
        rig.set_schema(gpus(2));
        let out = rig.tick(&[Some(70.0), Some(50.0)]);
        assert!(out.entries.is_empty());
        let report = out.report.expect("validity and coverage changed");
        assert_eq!(report.coverage, Coverage::Complete);
        assert_eq!(keys(&report), [key("gpu-temp", GPU0, Level::Crit)]);
        assert!(report.alerts[0].valid);
        assert_eq!(report.alerts[0].since_ms, WALL0);
        assert_eq!(report.alerts[0].value, Some(70.0));

        // The timers restarted at 10 s: the exit matures at 20 s, not 11 s.
        for _ in 11..=19 {
            let out = rig.tick(&[Some(70.0), Some(50.0)]);
            assert!(out.entries.is_empty());
            assert_eq!(rig.report().level, OverallLevel::Crit, "at {} ms", rig.ms);
        }
        let out = rig.tick(&[Some(70.0), Some(50.0)]);
        assert!(out.entries.is_empty());
        assert_eq!(out.report.unwrap().level, OverallLevel::Ok);
    }

    #[test]
    fn returning_sensor_with_other_semantics_starts_over() {
        let mut rig = Rig::new(vec![instant()], gpus(2));
        rig.tick(&[Some(95.0), Some(50.0)]);
        rig.set_schema(without_gpu0(true));
        rig.tick(&[Some(50.0)]);
        // Back from another source: a new instance, level ok.
        let mut moved = gpus(2);
        moved.sensors[0].source = Source::Lhm;
        rig.set_schema(moved);
        let report = rig.tick(&[Some(50.0), Some(50.0)]).report.unwrap();
        assert_eq!(report.level, OverallLevel::Ok);
        assert!(report.alerts.is_empty());
        assert_eq!(report.coverage, Coverage::Complete);
    }

    #[test]
    fn retained_alert_keeps_unit_and_label() {
        let mut rig = Rig::new(vec![instant()], gpus(2));
        rig.tick(&[Some(95.0), Some(50.0)]);
        // The device goes too.
        rig.set_schema(without_gpu0(false));
        let report = rig.tick(&[Some(50.0)]).report.unwrap();
        let alert = &report.alerts[0];
        assert_eq!(alert.device_id, "gpu/0");
        assert_eq!(alert.unit, Unit::Celsius);
        assert_eq!(alert.sensor_label, Label::new("gpu.core"));
        assert_eq!(
            alert.params.get("device").map(String::as_str),
            Some("GPU 0")
        );
        assert_eq!(alert.message_key, "rule.gpu-temp.message");
        assert_eq!(alert.threshold, Some(90.0));
    }

    #[test]
    fn disabling_the_rule_clears_lost_targets() {
        let mut rig = Rig::new(vec![instant()], gpus(2));
        rig.tick(&[Some(95.0), Some(50.0)]);
        rig.set_schema(without_gpu0(true));
        let lost = rig.tick(&[Some(50.0)]).report.unwrap();
        assert_eq!(lost.coverage, Coverage::Partial);

        // Only `notify` changes: the lost alert and the report survive.
        let mut notify = instant();
        notify.notify.warn = true;
        rig.engine.set_rules(vec![notify.clone()]);
        let out = rig.tick(&[Some(50.0)]);
        assert_eq!(out.report, None);
        assert!(out.entries.is_empty());
        assert_eq!(rig.report().unavailable_targets, [target("gpu-temp", GPU0)]);

        // Disabled: both the alert and the unavailable target go.
        notify.enabled = false;
        rig.engine.set_rules(vec![notify]);
        let report = rig.tick(&[Some(50.0)]).report.unwrap();
        assert!(report.alerts.is_empty());
        assert!(report.unavailable_targets.is_empty());
        assert_eq!(report.coverage, Coverage::Complete);
        assert_eq!(report.level, OverallLevel::Neutral);
    }

    // --- report changes (R7) -----------------------------------------------

    #[test]
    fn report_changes_only_when_it_should() {
        let mut rig = Rig::new(vec![instant()], gpus(1));
        let revision = |out: Evaluation| out.report.map(|r| r.revision);
        assert_eq!(revision(rig.tick(&[Some(50.0)])), Some(1));
        // No alerts: values are not in the report.
        assert_eq!(revision(rig.tick(&[Some(60.0)])), None);
        assert_eq!(revision(rig.tick(&[Some(85.0)])), Some(2));
        assert_eq!(revision(rig.tick(&[Some(85.0)])), None);
        // Validity of the alert.
        assert_eq!(revision(rig.tick(&[None])), Some(3));
        assert!(!rig.report().alerts[0].valid);
        assert_eq!(revision(rig.tick(&[None])), None);
        assert_eq!(revision(rig.tick(&[Some(85.0)])), Some(4));
        // Level.
        assert_eq!(revision(rig.tick(&[Some(95.0)])), Some(5));
        // A held value stays valid.
        assert_eq!(revision(rig.tick_with(&[Some(95.0)], &[Held])), None);
        // A notify-only rule change.
        let mut notify = instant();
        notify.notify.warn = true;
        rig.engine.set_rules(vec![notify]);
        let out = rig.tick(&[Some(95.0)]);
        assert_eq!(out, Evaluation::default());
        // A schema change that keeps everything.
        rig.set_schema(gpus(1));
        assert_eq!(revision(rig.tick(&[Some(95.0)])), None);
        // A new label is part of the presentation.
        let mut relabeled = gpus(1);
        relabeled.sensors[0].label = Label::with_arg("gpu.core", "0");
        rig.set_schema(relabeled);
        assert_eq!(revision(rig.tick(&[Some(95.0)])), Some(6));
    }

    #[test]
    fn report_ignores_sub_precision_changes() {
        let mut rig = Rig::new(vec![instant()], gpus(1));
        assert!(rig.tick(&[Some(91.6)]).report.is_some());
        // 92 °C and 197 °F both times.
        assert_eq!(rig.tick(&[Some(91.8)]).report, None);
        assert_eq!(rig.tick(&[Some(91.6)]).report, None);
        let report = rig.tick(&[Some(92.6)]).report.expect("93 °C");
        assert_eq!(report.alerts[0].value, Some(92.6));
    }

    #[test]
    fn fahrenheit_display_boundary_emits_report() {
        let mut rig = Rig::new(vec![instant()], gpus(1));
        assert!(rig.tick(&[Some(91.9)]).report.is_some());
        // 92 °C both times, 197 °F then 198 °F.
        let report = rig.tick(&[Some(91.95)]).report.expect("198 °F");
        assert_eq!(report.alerts[0].value, Some(91.95));
    }

    #[test]
    fn unchanged_report_keeps_current_raw_value_and_last_valid_time() {
        let mut rig = Rig::new(vec![instant()], gpus(1));
        rig.tick(&[Some(91.6)]);
        assert_eq!(rig.tick(&[Some(91.8)]).report, None);
        let alert = &rig.report().alerts[0];
        assert_eq!(alert.value, Some(91.8));
        assert_eq!(alert.last_valid_ms, Some(WALL0 + 1000));
        assert_eq!(rig.report().revision, 1);

        // A held value is not a new measurement.
        assert_eq!(rig.tick_with(&[Some(91.7)], &[Held]).report, None);
        let alert = &rig.report().alerts[0];
        assert_eq!(alert.value, Some(91.8));
        assert_eq!(alert.last_valid_ms, Some(WALL0 + 1000));
        assert!(alert.valid);

        // An absent value changes validity, not the last valid value.
        let report = rig.tick(&[None]).report.unwrap();
        assert_eq!(report.alerts[0].value, Some(91.8));
        assert_eq!(report.alerts[0].last_valid_ms, Some(WALL0 + 1000));
    }

    #[test]
    fn display_key_matches_formatter_precision() {
        let same = |a: f64, b: f64, unit: Unit| display_key(a, unit) == display_key(b, unit);
        // Whole percent, watt, rpm.
        assert!(same(50.4, 49.6, Unit::Percent));
        assert!(!same(50.4, 50.6, Unit::Percent));
        assert!(same(120.2, 119.8, Unit::Watt));
        // Temperatures in both °C and °F.
        assert!(same(91.6, 91.8, Unit::Celsius));
        assert!(!same(91.9, 91.95, Unit::Celsius));
        // Volts with three decimals, amperes with one.
        assert!(!same(1.2004, 1.2006, Unit::Volt));
        assert!(same(1.2004, 1.2001, Unit::Volt));
        assert!(!same(1.04, 1.06, Unit::Ampere));
        // Throughput in bytes (binary steps) and bits (decimal steps):
        // 1234 B/s is 1.2 KB/s and 9.9 kbit/s; 1240 B/s is 1.2 KB/s and
        // 9.9 kbit/s; 1300 B/s is 1.3 KB/s and 10 kbit/s.
        assert!(same(1234.0, 1240.0, Unit::BytesPerSecond));
        assert!(!same(1240.0, 1300.0, Unit::BytesPerSecond));
        // Same text in bits, not in bytes: 96 kbit/s, 11.7 vs 11.8 KB/s.
        assert!(!same(12_000.0, 12_050.0, Unit::BytesPerSecond));
        // Same text in bytes, not in bits: 11.7 KB/s, 95 vs 96 kbit/s.
        assert!(!same(11_930.0, 11_950.0, Unit::BytesPerSecond));
        // 15.0 KB and 150 KB share digits but not text.
        assert!(!same(15.0 * 1024.0, 150.0 * 1024.0, Unit::Bytes));
        assert!(!same(15.0 * 1024.0, 150.0 * 1024.0, Unit::BytesPerSecond));
        assert!(same(12_000.0, 12_400.0, Unit::BitsPerSecond));
        // Clocks: MHz below 1000, GHz with two decimals above.
        assert!(same(3601.0, 3604.0, Unit::Megahertz));
        assert!(!same(3601.0, 3606.0, Unit::Megahertz));
        // Flags: on or off.
        assert!(same(1.0, 0.7, Unit::Boolean));
        assert!(!same(1.0, 0.0, Unit::Boolean));
    }

    // --- clocks, ordering, suspend and entries ---------------------------

    #[test]
    fn health_duration_ignores_wall_clock_jumps() {
        let mut engine = RuleEngine::new();
        engine.set_interval_ms(1000);
        engine.set_rules(vec![instant()]);
        let schema = gpus(1);
        let mut eval = |value: f64, mono: u64, wall: u64| {
            engine
                .evaluate(&schema, mono == 0, &[Some(value)], &[Fresh], mono, wall)
                .report
                .map(|r| (r.revision, r.since_ms))
        };
        assert_eq!(eval(50.0, 0, WALL0), Some((1, WALL0)));
        assert_eq!(eval(85.0, 1000, WALL0 + 1000), Some((2, WALL0 + 1000)));
        // Wall clock jumps alone emit nothing.
        assert_eq!(eval(85.0, 2000, WALL0 - 3_600_000), None);
        assert_eq!(eval(85.0, 3000, WALL0 + 7_200_000), None);
        assert_eq!(
            engine.clock(5000),
            HealthClock {
                revision: 2,
                level_elapsed_ms: 4000
            }
        );
        assert_eq!(engine.report().since_ms, WALL0 + 1000);
        assert_eq!(
            serde_json::to_value(engine.clock(1000)).unwrap(),
            json!({ "revision": 2, "levelElapsedMs": 0 })
        );
    }

    #[test]
    fn alert_order_is_stable_for_ties() {
        // Sensors and rules listed against key order.
        let schema = schema(vec![gpu(1), gpu(0)], vec![core(1), core(0)]);
        let mut late = custom(CUSTOM, GPU0, Unit::Celsius, Condition::Above);
        late.crit = fixed(90.0, 0);
        let mut rig = Rig::new(vec![instant(), late], schema);
        let report = rig.tick(&[Some(95.0), Some(95.0)]).report.unwrap();
        let expected = [
            key(CUSTOM, GPU0, Level::Crit),
            key("gpu-temp", GPU0, Level::Crit),
            key("gpu-temp", GPU1, Level::Crit),
        ];
        assert_eq!(keys(&report), expected);
        for _ in 0..20 {
            assert_eq!(rig.tick(&[Some(95.0), Some(95.0)]).report, None);
            assert_eq!(keys(rig.report()), expected);
        }
    }

    /// Feeds `value` at each `(monotonic, wall)` pair and returns the
    /// monotonic times of the level entries.
    fn entries_at(engine: &mut RuleEngine, schema: &Schema, ticks: &[(u64, u64)]) -> Vec<u64> {
        let mut at = Vec::new();
        for &(mono, wall) in ticks {
            let out = engine.evaluate(schema, mono == 0, &[Some(85.0)], &[Fresh], mono, wall);
            if !out.entries.is_empty() {
                at.push(mono);
            }
        }
        at
    }

    fn warn_engine(interval_ms: u64) -> RuleEngine {
        let mut engine = RuleEngine::new();
        engine.set_interval_ms(interval_ms);
        engine.set_rules(vec![builtin("gpu-temp")]);
        engine
    }

    #[test]
    fn suspend_gap_resets_timers() {
        let schema = gpus(1);
        let seconds = |from: u64, to: u64, wall_offset: u64| -> Vec<(u64, u64)> {
            (from..=to)
                .map(|s| (s * 1000, WALL0 + wall_offset + s * 1000))
                .collect()
        };

        // Warn timer at 25 s of 30, then 60 s of suspend: no entry on the
        // next tick; the timer restarts there.
        let mut engine = warn_engine(1000);
        assert!(entries_at(&mut engine, &schema, &seconds(0, 25, 0)).is_empty());
        assert_eq!(
            entries_at(&mut engine, &schema, &seconds(85, 130, 0)),
            [115_000]
        );
        assert_eq!(engine.report().level, OverallLevel::Warn);

        // A wall clock jump counts as a gap too.
        let mut engine = warn_engine(1000);
        assert!(entries_at(&mut engine, &schema, &seconds(0, 29, 0)).is_empty());
        assert_eq!(
            entries_at(&mut engine, &schema, &seconds(30, 70, 60_000)),
            [60_000]
        );

        // Without a gap the timer matures at 30 s.
        let mut engine = warn_engine(1000);
        assert_eq!(
            entries_at(&mut engine, &schema, &seconds(0, 40, 0)),
            [30_000]
        );

        // A gap of exactly max(3 × interval, 5 s) is not a suspend.
        let mut engine = warn_engine(1000);
        assert!(entries_at(&mut engine, &schema, &seconds(0, 25, 0)).is_empty());
        assert_eq!(
            entries_at(&mut engine, &schema, &seconds(30, 31, 0)),
            [30_000]
        );
        let mut engine = warn_engine(3000);
        let ticks: Vec<(u64, u64)> = (0..=7).map(|i| (i * 3000, WALL0 + i * 3000)).collect();
        assert!(entries_at(&mut engine, &schema, &ticks).is_empty());
        // 21 s → 30 s: 9 s = 3 × 3 s.
        assert_eq!(
            entries_at(&mut engine, &schema, &[(30_000, WALL0 + 30_000)]),
            [30_000]
        );
    }

    #[test]
    fn entries_only_on_escalation() {
        let mut rig = Rig::new(vec![instant()], gpus(1));
        let entry = |level: Level, notify: bool| LevelEntry {
            rule_id: "gpu-temp".into(),
            sensor_id: GPU0.into(),
            device_id: "gpu/0".into(),
            level,
            notify,
        };
        assert_eq!(rig.tick(&[Some(85.0)]).entries, [entry(Level::Warn, false)]);
        assert_eq!(rig.tick(&[Some(95.0)]).entries, [entry(Level::Crit, true)]);
        // crit → warn is not an entry.
        assert!(rig.tick(&[Some(85.0)]).entries.is_empty());
        assert_eq!(rig.report().level, OverallLevel::Warn);
        assert_eq!(rig.tick(&[Some(95.0)]).entries, [entry(Level::Crit, true)]);
        assert!(rig.tick(&[Some(50.0)]).entries.is_empty());
        assert_eq!(rig.report().level, OverallLevel::Ok);
        // ok → crit directly.
        assert_eq!(rig.tick(&[Some(95.0)]).entries, [entry(Level::Crit, true)]);

        // `notify` is read from the rule for the level entered.
        let mut loud = instant();
        loud.notify = Notify {
            warn: true,
            crit: false,
        };
        let mut rig = Rig::new(vec![loud], gpus(1));
        assert_eq!(rig.tick(&[Some(85.0)]).entries, [entry(Level::Warn, true)]);
        assert_eq!(rig.tick(&[Some(95.0)]).entries, [entry(Level::Crit, false)]);
    }

    // --- status and serialization ------------------------------------------

    #[test]
    fn rule_status_reports_thresholds_and_problems() {
        let drive = Sensor::new(
            "storage/0",
            SensorKind::Temperature,
            "drive",
            Unit::Celsius,
            Label::new("storage.temperature"),
            Source::Mock,
        );
        let schema = schema(
            vec![
                gpu(0),
                gpu(1),
                device(
                    "storage/0",
                    DeviceKind::Storage,
                    "SSD",
                    &[("tempWarningC", "90"), ("tempCriticalC", "80")],
                ),
            ],
            vec![core(0), core(1), drive],
        );
        let mismatch = custom(CUSTOM, GPU0, Unit::Percent, Condition::Above);
        let rules = vec![
            instant(),
            builtin("gpu-hotspot"),
            builtin("disk-temp"),
            mismatch,
        ];
        let mut rig = Rig::new(rules, schema);
        rig.tick(&[Some(95.0), Some(50.0), Some(40.0)]);
        // GPU 0 is lost while in crit.
        let mut lost = rig.schema.clone();
        lost.sensors.remove(0);
        rig.set_schema(lost);
        rig.tick(&[Some(50.0), Some(40.0)]);

        let source = |value: Option<f64>, source| value.map(|_| source);
        let status = |sensor: &str,
                      level,
                      warn: Option<f64>,
                      crit: Option<f64>,
                      valid,
                      problem: Option<&str>| {
            // Every threshold here is fixed but the disk's, which come from its properties.
            let from = if sensor.starts_with("storage/") {
                ThresholdSource::Property
            } else {
                ThresholdSource::Fixed
            };
            InstanceStatus {
                sensor_id: sensor.into(),
                level,
                warn,
                crit,
                warn_source: source(warn, from),
                crit_source: source(crit, from),
                valid,
                problem: problem.map(str::to_owned),
            }
        };
        assert_eq!(
            rig.engine.status(),
            [
                RuleStatus {
                    rule_id: "gpu-temp".into(),
                    instances: vec![
                        status(GPU1, Level::Ok, Some(83.0), Some(90.0), true, None),
                        status(GPU0, Level::Crit, Some(83.0), Some(90.0), false, None),
                    ],
                },
                RuleStatus {
                    rule_id: "gpu-hotspot".into(),
                    instances: vec![],
                },
                RuleStatus {
                    rule_id: "disk-temp".into(),
                    instances: vec![status(
                        "storage/0/temperature/drive",
                        Level::Ok,
                        Some(90.0),
                        Some(80.0),
                        false,
                        Some("order"),
                    )],
                },
                // Lost too, even without an alert.
                RuleStatus {
                    rule_id: CUSTOM.into(),
                    instances: vec![status(
                        GPU0,
                        Level::Ok,
                        Some(50.0),
                        None,
                        false,
                        Some("unitMismatch"),
                    )],
                },
            ]
        );
        assert_eq!(
            serde_json::to_value(status(
                GPU0,
                Level::Warn,
                None,
                None,
                true,
                Some("unitMismatch")
            ))
            .unwrap(),
            json!({
                "sensorId": GPU0, "level": "warn", "warn": null, "crit": null,
                "warnSource": null, "critSource": null,
                "valid": true, "problem": "unitMismatch"
            })
        );

        // An instance with a problem is not valid even with a value.
        let mut rig = Rig::new(
            vec![custom(CUSTOM, GPU0, Unit::Percent, Condition::Above)],
            gpus(1),
        );
        rig.tick(&[Some(1.0)]);
        assert_eq!(
            rig.engine.status()[0].instances,
            [status(
                GPU0,
                Level::Ok,
                Some(50.0),
                None,
                false,
                Some("unitMismatch")
            )]
        );
    }

    #[test]
    fn health_report_serializes_camel_case() {
        let mut rig = Rig::new(vec![instant()], gpus(2));
        rig.tick(&[Some(95.0), Some(50.0)]);
        rig.set_schema(without_gpu0(true));
        let report = rig.tick(&[Some(50.0)]).report.unwrap();
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "level": "crit",
                "sinceMs": WALL0,
                "revision": 2,
                "coverage": "partial",
                "unavailableTargets": [{ "ruleId": "gpu-temp", "sensorId": GPU0 }],
                "alerts": [{
                    "ruleId": "gpu-temp",
                    "sensorId": GPU0,
                    "deviceId": "gpu/0",
                    "unit": "celsius",
                    "sensorLabel": { "key": "gpu.core" },
                    "level": "crit",
                    "value": 95.0,
                    "threshold": 90.0,
                    "sinceMs": WALL0,
                    "valid": false,
                    "lastValidMs": WALL0,
                    "messageKey": "rule.gpu-temp.message",
                    "params": { "device": "GPU 0" }
                }]
            })
        );
        assert_eq!(
            serde_json::to_value(OverallLevel::Neutral).unwrap(),
            json!("neutral")
        );
        assert_eq!(
            serde_json::to_value(Coverage::Unavailable).unwrap(),
            json!("unavailable")
        );
    }
}
