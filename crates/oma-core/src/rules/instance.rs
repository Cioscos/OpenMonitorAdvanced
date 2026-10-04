//! Rule instances: a rule expanded over the sensors it matches, each with its
//! own level state machine (spec §3.4).

use serde::{Deserialize, Serialize};

use super::{Condition, LevelSpec, Rule, Target, Threshold};
use crate::engine::Quality;
use crate::model::{Device, Schema, Sensor, Source, Unit};

/// Alert level of one instance; ordered by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Crit,
}

/// Thresholds of an instance after reading the device properties. `None` for
/// a level the rule lacks, and for flag levels, which have no threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    pub warn: Option<f64>,
    pub crit: Option<f64>,
}

/// Where a resolved threshold came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThresholdSource {
    /// A fixed number in the rule.
    Fixed,
    /// The device property, plus the offset.
    Property,
    /// The fallback, because the property is missing or not a number.
    Fallback,
}

/// The source of each level of [`Resolved`], `None` where it has no value.
/// Kept apart from it so that [`same_semantics`] compares values only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sources {
    pub warn: Option<ThresholdSource>,
    pub crit: Option<ThresholdSource>,
}

/// Identity of an instance: one rule on one sensor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstanceKey {
    pub rule_id: String,
    pub sensor_id: String,
}

/// Why an instance is kept but never evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstanceProblem {
    /// The resolved thresholds are in the wrong order for the condition.
    Order,
    /// The explicitly targeted sensor is not in the rule's unit.
    UnitMismatch,
}

/// Outcome of one evaluation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Stay,
    /// A more severe level was entered.
    Entered(Level),
    /// A less severe level was reached.
    Left {
        from: Level,
        to: Level,
    },
}

/// Timers and latch of one level.
#[derive(Debug, Clone, Copy, Default)]
struct LevelState {
    /// The entry timer matured and the exit timer has not.
    latched: bool,
    /// Valid time the entry condition has held, while it runs.
    entry_ms: Option<u64>,
    /// Valid time the exit condition has held, while it runs.
    exit_ms: Option<u64>,
}

/// A rule applied to one sensor.
#[derive(Debug, Clone)]
pub struct Instance {
    pub key: InstanceKey,
    /// Position of the sensor in `schema.sensors`.
    pub sensor_index: usize,
    pub resolved: Resolved,
    pub sources: Sources,
    pub problem: Option<InstanceProblem>,
    source: Source,
    sensor_unit: Unit,
    warn: LevelState,
    crit: LevelState,
    /// Time of the last Fresh valid tick, kept across held values and
    /// cleared by absent ones; a Fresh valid tick adds the time since it.
    anchor_ms: Option<u64>,
}

/// The `<name>` segment of `sensor.id`, read from the right because the
/// device id may itself contain `/`; `None` when the id does not carry the
/// sensor's own device and kind.
fn name_segment(sensor: &Sensor) -> Option<&str> {
    let mut parts = sensor.id.rsplitn(3, '/');
    let (name, kind, device) = (parts.next()?, parts.next()?, parts.next()?);
    (kind == sensor.kind.as_str() && device == sensor.device_id && !name.is_empty()).then_some(name)
}

/// Whether `name` is selected by `names` (R8): empty means every name, an
/// entry ending with `*` is a prefix, any other entry must match exactly. An
/// empty entry, or one with a `*` elsewhere, matches nothing.
fn name_matches(names: &[String], name: &str) -> bool {
    if names.is_empty() {
        return true;
    }
    names.iter().any(|entry| {
        if entry.is_empty() {
            return false;
        }
        match entry.strip_suffix('*') {
            Some(prefix) => !prefix.contains('*') && name.starts_with(prefix),
            None => !entry.contains('*') && name == entry,
        }
    })
}

/// A threshold for `device`, with its source: a property is read as a number
/// plus `offset`, with `fallback` when it is missing, not a number or not
/// finite.
fn resolve(threshold: &Threshold, device: Option<&Device>) -> (f64, ThresholdSource) {
    match threshold {
        Threshold::Fixed { fixed } => (*fixed, ThresholdSource::Fixed),
        Threshold::Property {
            property,
            offset,
            fallback,
        } => device
            .and_then(|d| d.properties.get(property))
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .map(|value| value + offset)
            .filter(|sum| sum.is_finite())
            .map_or((*fallback, ThresholdSource::Fallback), |sum| {
                (sum, ThresholdSource::Property)
            }),
    }
}

fn instance(rule: &Rule, schema: &Schema, sensor_index: usize) -> Instance {
    let sensor = &schema.sensors[sensor_index];
    let device = schema.devices.iter().find(|d| d.id == sensor.device_id);
    let threshold = |level: &Option<LevelSpec>| {
        let threshold = level.as_ref()?.threshold.as_ref()?;
        Some(resolve(threshold, device))
    };
    let (warn, crit) = (threshold(&rule.warn), threshold(&rule.crit));
    let resolved = Resolved {
        warn: warn.map(|(value, _)| value),
        crit: crit.map(|(value, _)| value),
    };
    let sources = Sources {
        warn: warn.map(|(_, source)| source),
        crit: crit.map(|(_, source)| source),
    };
    let misordered = match (rule.condition, resolved.warn, resolved.crit) {
        (Condition::Above, Some(warn), Some(crit)) => crit < warn,
        (Condition::Below, Some(warn), Some(crit)) => crit > warn,
        _ => false,
    };
    let problem = if sensor.unit != rule.unit {
        Some(InstanceProblem::UnitMismatch)
    } else if misordered {
        Some(InstanceProblem::Order)
    } else {
        None
    };
    Instance {
        key: InstanceKey {
            rule_id: rule.id.clone(),
            sensor_id: sensor.id.clone(),
        },
        sensor_index,
        resolved,
        sources,
        problem,
        source: sensor.source,
        sensor_unit: sensor.unit,
        warn: LevelState::default(),
        crit: LevelState::default(),
        anchor_ms: None,
    }
}

/// Every instance of the enabled `rules` on `schema`, in rule order and then
/// schema order. A selector skips sensors in another unit than the rule's; an
/// explicit target keeps its sensor as [`InstanceProblem::UnitMismatch`].
pub fn expand(rules: &[Rule], schema: &Schema) -> Vec<Instance> {
    let mut instances = Vec::new();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        match &rule.target {
            Target::Sensor { sensor } => {
                let index = schema
                    .sensors
                    .iter()
                    .position(|s| s.id == *sensor && name_segment(s).is_some());
                if let Some(index) = index {
                    instances.push(instance(rule, schema, index));
                }
            }
            Target::Selector {
                device_kind,
                sensor_kind,
                names,
            } => {
                for (index, sensor) in schema.sensors.iter().enumerate() {
                    if sensor.kind != *sensor_kind || sensor.unit != rule.unit {
                        continue;
                    }
                    let Some(name) = name_segment(sensor) else {
                        continue;
                    };
                    let device_matches = schema
                        .devices
                        .iter()
                        .any(|d| d.id == sensor.device_id && d.kind == *device_kind);
                    if device_matches && name_matches(names, name) {
                        instances.push(instance(rule, schema, index));
                    }
                }
            }
        }
    }
    instances
}

/// Whether `b` may keep the state of `a` (spec §3.4): same rule and sensor,
/// source, unit, condition, resolved thresholds, durations and hysteresis.
/// `notify` does not count.
pub fn same_semantics(a: &Instance, b: &Instance, rules_a: &Rule, rules_b: &Rule) -> bool {
    let duration = |level: &Option<LevelSpec>| level.as_ref().map(|l| l.duration_s);
    a.key == b.key
        && a.source == b.source
        && a.sensor_unit == b.sensor_unit
        && a.problem == b.problem
        && a.resolved == b.resolved
        && rules_a.unit == rules_b.unit
        && rules_a.condition == rules_b.condition
        && duration(&rules_a.warn) == duration(&rules_b.warn)
        && duration(&rules_a.crit) == duration(&rules_b.crit)
        && rules_a.hysteresis == rules_b.hysteresis
}

impl LevelState {
    /// Advances this level with a Fresh valid value. `enter` and `exit` are
    /// the entry and strict exit conditions for the value; `delta_ms` is the
    /// time since the previous Fresh valid tick. A timer that starts counts
    /// from 0, so the condition must hold for the whole duration.
    fn advance(
        &mut self,
        enter: bool,
        exit: bool,
        delta_ms: u64,
        entry_after: u64,
        exit_after: u64,
    ) {
        let accumulate = |timer: Option<u64>| Some(timer.map_or(0, |t| t.saturating_add(delta_ms)));
        if self.latched {
            self.entry_ms = None;
            self.exit_ms = if exit { accumulate(self.exit_ms) } else { None };
            if self.exit_ms.is_some_and(|t| t >= exit_after) {
                self.latched = false;
                self.exit_ms = None;
            }
        } else {
            self.exit_ms = None;
            self.entry_ms = if enter {
                accumulate(self.entry_ms)
            } else {
                None
            };
            if self.entry_ms.is_some_and(|t| t >= entry_after) {
                self.latched = true;
                self.entry_ms = None;
            }
        }
    }

    fn reset_timers(&mut self) {
        self.entry_ms = None;
        self.exit_ms = None;
    }
}

impl Instance {
    /// The most severe latched level.
    pub fn level(&self) -> Level {
        if self.crit.latched {
            Level::Crit
        } else if self.warn.latched {
            Level::Warn
        } else {
            Level::Ok
        }
    }

    /// Advances the state machine with the tick at `now_ms` (monotonic).
    /// `rule` is the rule this instance was expanded from.
    ///
    /// Both levels keep independent entry and exit timers, also while the
    /// other one is latched. An absent value resets the running timers and
    /// the anchor. A held value (R1) neither enters, exits, matures nor
    /// resets anything, and leaves the anchor at the last Fresh valid tick:
    /// the next Fresh valid value counts the time since that tick and may
    /// then mature. Held stretches are bounded upstream (a timed-out or
    /// stale source turns absent) and suspend gaps reset the timers. Levels
    /// change only on Fresh valid values. A suspended sensor (the source
    /// intentionally does not measure) keeps the timers and levels but
    /// clears the anchor: the suspended time is not counted either way.
    /// Does not allocate.
    pub fn step(&mut self, rule: &Rule, value: Option<f64>, quality: Quality, now_ms: u64) -> Step {
        if self.problem.is_some() {
            return Step::Stay;
        }
        if quality == Quality::Suspended {
            // The source intentionally does not measure (a disk in standby):
            // keep the timers and levels, but drop the anchor so the first
            // fresh tick afterwards does not count the suspended time.
            self.anchor_ms = None;
            return Step::Stay;
        }
        let Some(value) = value.filter(|v| v.is_finite()) else {
            self.reset_timers();
            return Step::Stay;
        };
        if quality == Quality::Held {
            return Step::Stay;
        }
        let delta_ms = self
            .anchor_ms
            .map_or(0, |anchor| now_ms.saturating_sub(anchor));
        self.anchor_ms = Some(now_ms);

        let before = self.level();
        let amount = rule.hysteresis.amount;
        let exit_after = u64::from(rule.hysteresis.duration_s) * 1000;
        let levels = [
            (&mut self.warn, &rule.warn, self.resolved.warn),
            (&mut self.crit, &rule.crit, self.resolved.crit),
        ];
        for (state, spec, threshold) in levels {
            let Some(spec) = spec else { continue };
            let (enter, exit) = match (rule.condition, threshold) {
                (Condition::Above, Some(t)) => (value >= t, value < t - amount),
                (Condition::Below, Some(t)) => (value <= t, value > t + amount),
                (Condition::FlagActive, _) => (value != 0.0, value == 0.0),
                // A threshold level without its threshold never fires.
                (_, None) => continue,
            };
            let entry_after = u64::from(spec.duration_s) * 1000;
            state.advance(enter, exit, delta_ms, entry_after, exit_after);
        }

        let after = self.level();
        match after.cmp(&before) {
            std::cmp::Ordering::Greater => Step::Entered(after),
            std::cmp::Ordering::Less => Step::Left {
                from: before,
                to: after,
            },
            std::cmp::Ordering::Equal => Step::Stay,
        }
    }

    /// Clears the running entry and exit timers; the levels stay.
    pub fn reset_timers(&mut self) {
        self.warn.reset_timers();
        self.crit.reset_timers();
        self.anchor_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeviceKind, Label, SensorKind};
    use crate::rules::{default_rules, Hysteresis, LevelSpec, Notify};
    use Quality::{Fresh, Held, Suspended};

    const GPU_CORE: &str = "gpu/0/temperature/core";
    const GPU_FLAG: &str = "gpu/0/flag/throttle-thermal";

    fn device(id: &str, kind: DeviceKind, properties: &[(&str, &str)]) -> Device {
        Device {
            id: id.into(),
            kind,
            name: id.into(),
            vendor: None,
            properties: properties
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    fn sensor(device_id: &str, kind: SensorKind, name: &str, unit: Unit) -> Sensor {
        Sensor::new(device_id, kind, name, unit, Label::new("x"), Source::Mock)
    }

    fn schema(devices: Vec<Device>, sensors: Vec<Sensor>) -> Schema {
        Schema {
            revision: 1,
            devices,
            sensors,
        }
    }

    fn builtin(id: &str) -> Rule {
        default_rules()
            .into_iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no rule {id}"))
    }

    fn keys(instances: &[Instance]) -> Vec<(String, String)> {
        instances
            .iter()
            .map(|i| (i.key.rule_id.clone(), i.key.sensor_id.clone()))
            .collect()
    }

    fn fixed(value: f64, duration_s: u32) -> LevelSpec {
        LevelSpec {
            threshold: Some(Threshold::Fixed { fixed: value }),
            duration_s,
        }
    }

    fn flag(duration_s: u32) -> LevelSpec {
        LevelSpec {
            threshold: None,
            duration_s,
        }
    }

    fn custom(
        sensor: &str,
        unit: Unit,
        condition: Condition,
        warn: Option<LevelSpec>,
        crit: Option<LevelSpec>,
        (amount, duration_s): (f64, u32),
    ) -> Rule {
        Rule {
            id: "custom-00000000-0000-4000-8000-000000000001".into(),
            target: Target::Sensor {
                sensor: sensor.into(),
            },
            unit,
            condition,
            warn,
            crit,
            hysteresis: Hysteresis { amount, duration_s },
            enabled: true,
            notify: Notify::default(),
        }
    }

    /// `above` on the GPU core temperature.
    fn above(warn: Option<(f64, u32)>, crit: Option<(f64, u32)>, hysteresis: (f64, u32)) -> Rule {
        custom(
            GPU_CORE,
            Unit::Celsius,
            Condition::Above,
            warn.map(|(v, d)| fixed(v, d)),
            crit.map(|(v, d)| fixed(v, d)),
            hysteresis,
        )
    }

    fn gpu_schema() -> Schema {
        schema(
            vec![device("gpu/0", DeviceKind::Gpu, &[])],
            vec![
                sensor("gpu/0", SensorKind::Temperature, "core", Unit::Celsius),
                sensor("gpu/0", SensorKind::Flag, "throttle-thermal", Unit::Boolean),
            ],
        )
    }

    fn only_instance(rule: &Rule, schema: &Schema) -> Instance {
        let mut all = expand(std::slice::from_ref(rule), schema);
        assert_eq!(all.len(), 1, "{:?}", keys(&all));
        all.remove(0)
    }

    /// Feeds one tick per second in `[from_s, to_s]` and returns the
    /// non-`Stay` steps with their time in milliseconds.
    fn feed(
        instance: &mut Instance,
        rule: &Rule,
        from_s: u64,
        to_s: u64,
        tick: impl Fn(u64) -> (Option<f64>, Quality),
    ) -> Vec<(u64, Step)> {
        let mut steps = Vec::new();
        for s in from_s..=to_s {
            let (value, quality) = tick(s);
            let now_ms = s * 1000;
            match instance.step(rule, value, quality, now_ms) {
                Step::Stay => {}
                step => steps.push((now_ms, step)),
            }
        }
        steps
    }

    fn fresh(value: f64) -> impl Fn(u64) -> (Option<f64>, Quality) {
        move |_| (Some(value), Fresh)
    }

    use Level::{Crit, Ok, Warn};

    fn left(from: Level, to: Level) -> Step {
        Step::Left { from, to }
    }

    // --- expansion -------------------------------------------------------

    #[test]
    fn selector_expands_every_matching_sensor() {
        let schema = schema(
            vec![
                device("cpu/0", DeviceKind::Cpu, &[]),
                device("gpu/0", DeviceKind::Gpu, &[]),
                device("gpu/1", DeviceKind::Gpu, &[]),
            ],
            vec![
                // A CPU sensor named like a GPU one: wrong device kind.
                sensor("cpu/0", SensorKind::Temperature, "core", Unit::Celsius),
                sensor("gpu/0", SensorKind::Temperature, "core", Unit::Celsius),
                sensor("gpu/0", SensorKind::Temperature, "hotspot", Unit::Celsius),
                sensor("gpu/0", SensorKind::Load, "core", Unit::Percent),
                sensor("gpu/1", SensorKind::Temperature, "core", Unit::Celsius),
            ],
        );
        let instances = expand(&[builtin("gpu-temp")], &schema);
        assert_eq!(
            keys(&instances),
            [
                ("gpu-temp".to_owned(), "gpu/0/temperature/core".to_owned()),
                ("gpu-temp".to_owned(), "gpu/1/temperature/core".to_owned()),
            ]
        );
        assert_eq!(instances[0].sensor_index, 1);
        assert_eq!(instances[1].sensor_index, 4);
        assert_eq!(
            instances[0].resolved,
            Resolved {
                warn: Some(83.0),
                crit: Some(90.0)
            }
        );
        assert!(instances.iter().all(|i| i.problem.is_none()));
        assert!(instances.iter().all(|i| i.level() == Ok));

        // Rule order first, then schema order; disabled rules expand to nothing.
        let mut disabled = builtin("gpu-mem-temp");
        disabled.enabled = false;
        let instances = expand(
            &[builtin("gpu-hotspot"), disabled, builtin("gpu-temp")],
            &schema,
        );
        assert_eq!(
            keys(&instances),
            [
                (
                    "gpu-hotspot".to_owned(),
                    "gpu/0/temperature/hotspot".to_owned()
                ),
                ("gpu-temp".to_owned(), "gpu/0/temperature/core".to_owned()),
                ("gpu-temp".to_owned(), "gpu/1/temperature/core".to_owned()),
            ]
        );

        // An empty `names` matches every sensor of the kinds.
        let mut all = builtin("gpu-temp");
        all.target = Target::Selector {
            device_kind: DeviceKind::Gpu,
            sensor_kind: SensorKind::Temperature,
            names: vec![],
        };
        assert_eq!(expand(&[all], &schema).len(), 3);
    }

    #[test]
    fn prefix_name_matches_volumes() {
        let schema = schema(
            vec![device("storage/0", DeviceKind::Storage, &[])],
            vec![
                sensor("storage/0", SensorKind::Percent, "wear", Unit::Percent),
                sensor(
                    "storage/0",
                    SensorKind::Percent,
                    "volume-{a1}",
                    Unit::Percent,
                ),
                sensor(
                    "storage/0",
                    SensorKind::Data,
                    "volume-{a1}-free",
                    Unit::Bytes,
                ),
                sensor(
                    "storage/0",
                    SensorKind::Percent,
                    "volume-{b2}",
                    Unit::Percent,
                ),
            ],
        );
        let instances = expand(&[builtin("volume-used")], &schema);
        assert_eq!(
            keys(&instances),
            [
                (
                    "volume-used".to_owned(),
                    "storage/0/percent/volume-{a1}".to_owned()
                ),
                (
                    "volume-used".to_owned(),
                    "storage/0/percent/volume-{b2}".to_owned()
                ),
            ]
        );

        // Without the trailing `*` the name must match exactly; a `*`
        // anywhere else or an empty entry matches nothing.
        for names in [
            vec!["volume-".to_owned()],
            vec!["vol*ume-{a1}".to_owned()],
            vec![String::new()],
        ] {
            let mut rule = builtin("volume-used");
            rule.target = Target::Selector {
                device_kind: DeviceKind::Storage,
                sensor_kind: SensorKind::Percent,
                names,
            };
            assert!(expand(&[rule], &schema).is_empty());
        }
        let mut exact = builtin("volume-used");
        exact.target = Target::Selector {
            device_kind: DeviceKind::Storage,
            sensor_kind: SensorKind::Percent,
            names: vec!["volume-{b2}".into()],
        };
        assert_eq!(
            expand(&[exact], &schema)[0].key.sensor_id,
            "storage/0/percent/volume-{b2}"
        );
    }

    #[test]
    fn sensor_target_with_wrong_unit_is_kept_but_idle() {
        let schema = gpu_schema();
        let rule = custom(
            GPU_CORE,
            Unit::Percent,
            Condition::Above,
            Some(fixed(50.0, 0)),
            None,
            (3.0, 0),
        );
        let mut instance = only_instance(&rule, &schema);
        assert_eq!(instance.key.sensor_id, GPU_CORE);
        assert_eq!(instance.sensor_index, 0);
        assert_eq!(instance.problem, Some(InstanceProblem::UnitMismatch));
        assert!(feed(&mut instance, &rule, 0, 10, fresh(99.0)).is_empty());
        assert_eq!(instance.level(), Ok);

        // A selector just skips sensors in another unit.
        let mut selector = rule.clone();
        selector.target = Target::Selector {
            device_kind: DeviceKind::Gpu,
            sensor_kind: SensorKind::Temperature,
            names: vec!["core".into()],
        };
        assert!(expand(&[selector], &schema).is_empty());

        // A target that is not in the schema has no instance.
        let missing = custom(
            "gpu/1/temperature/core",
            Unit::Celsius,
            Condition::Above,
            Some(fixed(50.0, 0)),
            None,
            (3.0, 0),
        );
        assert!(expand(&[missing], &schema).is_empty());
    }

    #[test]
    fn sensor_target_accepts_device_ids_with_slashes() {
        let schema = schema(
            vec![device("gpu/pci-0/x", DeviceKind::Gpu, &[])],
            vec![sensor(
                "gpu/pci-0/x",
                SensorKind::Temperature,
                "core",
                Unit::Celsius,
            )],
        );
        let rule = custom(
            "gpu/pci-0/x/temperature/core",
            Unit::Celsius,
            Condition::Above,
            Some(fixed(50.0, 0)),
            None,
            (3.0, 0),
        );
        let instance = only_instance(&rule, &schema);
        assert_eq!(instance.problem, None);
        assert_eq!(only_instance(&builtin("gpu-temp"), &schema).sensor_index, 0);
    }

    fn cpu_schema(properties: &[(&str, &str)]) -> Schema {
        schema(
            vec![device("cpu/0", DeviceKind::Cpu, properties)],
            vec![sensor(
                "cpu/0",
                SensorKind::Temperature,
                "tctl",
                Unit::Celsius,
            )],
        )
    }

    #[test]
    fn property_threshold_uses_offset_and_fallback() {
        let rule = builtin("cpu-temp");
        let instance = only_instance(&rule, &cpu_schema(&[("tjMaxC", "89")]));
        assert_eq!(
            instance.resolved,
            Resolved {
                warn: Some(79.0),
                crit: Some(89.0)
            }
        );
        assert_eq!(instance.problem, None);

        let fallback = Resolved {
            warn: Some(85.0),
            crit: Some(95.0),
        };
        assert_eq!(only_instance(&rule, &cpu_schema(&[])).resolved, fallback);
        assert_eq!(
            only_instance(&rule, &cpu_schema(&[("tjMaxC", "abc")])).resolved,
            fallback
        );
        assert_eq!(
            only_instance(&rule, &cpu_schema(&[("tjMaxC", "")])).resolved,
            fallback
        );
    }

    #[test]
    fn resolved_thresholds_record_their_source() {
        use ThresholdSource::{Fallback, Fixed, Property};
        let rule = builtin("cpu-temp");
        let sources = |warn, crit| Sources { warn, crit };
        assert_eq!(
            only_instance(&rule, &cpu_schema(&[("tjMaxC", "89")])).sources,
            sources(Some(Property), Some(Property))
        );
        for properties in [&[][..], &[("tjMaxC", "abc")], &[("tjMaxC", "inf")]] {
            assert_eq!(
                only_instance(&rule, &cpu_schema(properties)).sources,
                sources(Some(Fallback), Some(Fallback)),
                "{properties:?}"
            );
        }
        // A fixed level, and a flag or a missing level without a source.
        let rule = above(None, Some((90.0, 0)), (3.0, 0));
        assert_eq!(
            only_instance(&rule, &gpu_schema()).sources,
            sources(None, Some(Fixed))
        );
        let rule = custom(
            GPU_FLAG,
            Unit::Boolean,
            Condition::FlagActive,
            Some(flag(0)),
            None,
            (0.0, 0),
        );
        assert_eq!(
            only_instance(&rule, &gpu_schema()).sources,
            sources(None, None)
        );
    }

    #[test]
    fn non_finite_property_uses_fallback() {
        let rule = builtin("cpu-temp");
        let fallback = Resolved {
            warn: Some(85.0),
            crit: Some(95.0),
        };
        for value in ["NaN", "inf", "-inf", "infinity", "1e400"] {
            assert_eq!(
                only_instance(&rule, &cpu_schema(&[("tjMaxC", value)])).resolved,
                fallback,
                "{value}"
            );
        }
        // A finite property whose sum with the offset overflows.
        let overflow = custom(
            "cpu/0/temperature/tctl",
            Unit::Celsius,
            Condition::Above,
            Some(LevelSpec {
                threshold: Some(Threshold::Property {
                    property: "tjMaxC".into(),
                    offset: 1e308,
                    fallback: 70.0,
                }),
                duration_s: 0,
            }),
            None,
            (3.0, 0),
        );
        let instance = only_instance(&overflow, &cpu_schema(&[("tjMaxC", "1e308")]));
        assert_eq!(instance.resolved.warn, Some(70.0));
    }

    fn disk_schema(properties: &[(&str, &str)]) -> Schema {
        schema(
            vec![device("storage/0", DeviceKind::Storage, properties)],
            vec![sensor(
                "storage/0",
                SensorKind::Temperature,
                "drive",
                Unit::Celsius,
            )],
        )
    }

    #[test]
    fn order_is_checked_after_resolution() {
        let rule = builtin("disk-temp");
        let mut instance = only_instance(
            &rule,
            &disk_schema(&[("tempWarningC", "90"), ("tempCriticalC", "80")]),
        );
        assert_eq!(instance.problem, Some(InstanceProblem::Order));
        assert_eq!(
            instance.resolved,
            Resolved {
                warn: Some(90.0),
                crit: Some(80.0)
            }
        );
        assert!(feed(&mut instance, &rule, 0, 100, fresh(200.0)).is_empty());
        assert_eq!(instance.level(), Ok);

        let fine = only_instance(
            &rule,
            &disk_schema(&[("tempWarningC", "70"), ("tempCriticalC", "80")]),
        );
        assert_eq!(fine.problem, None);
        // Equal thresholds are in order.
        let equal = only_instance(
            &rule,
            &disk_schema(&[("tempWarningC", "80"), ("tempCriticalC", "80")]),
        );
        assert_eq!(equal.problem, None);

        // `below` wants crit ≤ warn.
        let below = custom(
            GPU_CORE,
            Unit::Celsius,
            Condition::Below,
            Some(fixed(10.0, 0)),
            Some(fixed(20.0, 0)),
            (3.0, 0),
        );
        assert_eq!(
            only_instance(&below, &gpu_schema()).problem,
            Some(InstanceProblem::Order)
        );
    }

    // --- state machine ---------------------------------------------------

    #[test]
    fn warn_after_its_duration_then_crit_after_its_own() {
        let rule = above(Some((83.0, 10)), Some((90.0, 60)), (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 90, fresh(95.0)),
            [(10_000, Step::Entered(Warn)), (60_000, Step::Entered(Crit))]
        );
        assert_eq!(instance.level(), Crit);
    }

    #[test]
    fn zero_duration_enters_on_the_first_tick() {
        let rule = above(Some((83.0, 0)), Some((90.0, 0)), (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 5, 8, fresh(95.0)),
            [(5_000, Step::Entered(Crit))]
        );
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 5, 8, fresh(85.0)),
            [(5_000, Step::Entered(Warn))]
        );
    }

    #[test]
    fn ok_to_crit_directly() {
        let rule = above(Some((83.0, 30)), Some((90.0, 10)), (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 60, fresh(95.0)),
            [(10_000, Step::Entered(Crit))]
        );
        assert_eq!(instance.level(), Crit);
    }

    #[test]
    fn exit_needs_strict_margin_for_hysteresis_duration() {
        // above: exit needs value < 80 − 3 for 5 s.
        let rule = above(Some((80.0, 0)), None, (3.0, 5));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 0, fresh(85.0)),
            [(0, Step::Entered(Warn))]
        );
        assert!(feed(&mut instance, &rule, 1, 30, fresh(77.0)).is_empty());
        assert_eq!(
            feed(&mut instance, &rule, 31, 60, fresh(76.9)),
            [(36_000, left(Warn, Ok))]
        );

        // above with no hysteresis: equality keeps the level.
        let rule = above(Some((80.0, 0)), None, (0.0, 0));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 30, fresh(80.0)),
            [(0, Step::Entered(Warn))]
        );
        assert_eq!(
            feed(&mut instance, &rule, 31, 32, fresh(79.99)),
            [(31_000, left(Warn, Ok))]
        );

        // below: exit needs value > 15 + 3 for 5 s.
        let rule = custom(
            GPU_CORE,
            Unit::Celsius,
            Condition::Below,
            Some(fixed(15.0, 0)),
            None,
            (3.0, 5),
        );
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 0, fresh(15.0)),
            [(0, Step::Entered(Warn))]
        );
        assert!(feed(&mut instance, &rule, 1, 30, fresh(18.0)).is_empty());
        assert_eq!(
            feed(&mut instance, &rule, 31, 60, fresh(18.1)),
            [(36_000, left(Warn, Ok))]
        );

        // flagActive: exit needs the flag at 0 for 5 s; a 1 restarts it.
        let rule = custom(
            GPU_FLAG,
            Unit::Boolean,
            Condition::FlagActive,
            Some(flag(0)),
            None,
            (0.0, 5),
        );
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(instance.sensor_index, 1);
        assert_eq!(
            feed(&mut instance, &rule, 0, 0, fresh(1.0)),
            [(0, Step::Entered(Warn))]
        );
        assert!(feed(&mut instance, &rule, 1, 4, fresh(0.0)).is_empty());
        assert!(feed(&mut instance, &rule, 5, 5, fresh(1.0)).is_empty());
        assert_eq!(
            feed(&mut instance, &rule, 6, 20, fresh(0.0)),
            [(11_000, left(Warn, Ok))]
        );
    }

    #[test]
    fn crit_falls_to_warn_only_if_warn_still_holds() {
        let rule = above(Some((83.0, 30)), Some((90.0, 10)), (3.0, 10));

        // Warn matured while in crit: crit recedes to warn.
        let mut instance = only_instance(&rule, &gpu_schema());
        let steps = feed(&mut instance, &rule, 0, 90, |s| {
            (Some(if s <= 40 { 95.0 } else { 85.0 }), Fresh)
        });
        assert_eq!(
            steps,
            [(10_000, Step::Entered(Crit)), (51_000, left(Crit, Warn))]
        );

        // Warn not matured yet: crit falls to ok, then warn matures on its
        // own timer, which kept running during crit.
        let mut instance = only_instance(&rule, &gpu_schema());
        let steps = feed(&mut instance, &rule, 0, 60, |s| {
            (Some(if s <= 15 { 95.0 } else { 85.0 }), Fresh)
        });
        assert_eq!(
            steps,
            [
                (10_000, Step::Entered(Crit)),
                (26_000, left(Crit, Ok)),
                (30_000, Step::Entered(Warn)),
            ]
        );

        // Both recede together: straight to ok.
        let mut instance = only_instance(&rule, &gpu_schema());
        let steps = feed(&mut instance, &rule, 0, 90, |s| {
            (Some(if s <= 40 { 95.0 } else { 70.0 }), Fresh)
        });
        assert_eq!(
            steps,
            [(10_000, Step::Entered(Crit)), (51_000, left(Crit, Ok))]
        );
    }

    #[test]
    fn most_severe_matured_level_wins() {
        // Warn stays latched while crit matures on its own timer.
        let rule = above(Some((83.0, 0)), Some((90.0, 5)), (3.0, 0));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 10, fresh(95.0)),
            [(0, Step::Entered(Warn)), (5_000, Step::Entered(Crit))]
        );
    }

    #[test]
    fn warn_latch_survives_inside_hysteresis_band() {
        let rule = above(Some((83.0, 0)), Some((90.0, 0)), (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 0, fresh(95.0)),
            [(0, Step::Entered(Crit))]
        );
        // 81 is below the warn threshold but inside its band (≥ 80).
        assert_eq!(
            feed(&mut instance, &rule, 1, 120, fresh(81.0)),
            [(11_000, left(Crit, Warn))]
        );
        assert_eq!(instance.level(), Warn);
    }

    #[test]
    fn oscillation_at_threshold_does_not_flap() {
        let rule = above(Some((83.0, 30)), Some((90.0, 10)), (3.0, 10));
        let oscillate = |s: u64| (Some(if s % 2 == 0 { 83.1 } else { 82.9 }), Fresh);

        let mut instance = only_instance(&rule, &gpu_schema());
        assert!(feed(&mut instance, &rule, 0, 120, oscillate).is_empty());
        assert_eq!(instance.level(), Ok);

        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 30, fresh(85.0)),
            [(30_000, Step::Entered(Warn))]
        );
        assert!(feed(&mut instance, &rule, 31, 151, oscillate).is_empty());
        assert_eq!(instance.level(), Warn);
    }

    #[test]
    fn held_values_neither_mature_nor_reset() {
        let rule = above(Some((83.0, 10)), None, (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        // 5 s of Fresh, then 30 s of Held: the duration passes on a Held
        // tick, which does not enter, and the Held values below the band do
        // not reset the timer. The next Fresh value enters.
        let steps = feed(&mut instance, &rule, 0, 60, |s| match s {
            0..=5 => (Some(95.0), Fresh),
            6..=30 => (Some(95.0), Held),
            31..=35 => (Some(70.0), Held),
            _ => (Some(95.0), Fresh),
        });
        assert_eq!(steps, [(36_000, Step::Entered(Warn))]);

        // Same for the exit timer.
        let steps = feed(&mut instance, &rule, 100, 160, |s| match s {
            100..=104 => (Some(70.0), Fresh),
            105..=130 => (Some(70.0), Held),
            131..=135 => (Some(95.0), Held),
            _ => (Some(70.0), Fresh),
        });
        assert_eq!(steps, [(136_000, left(Warn, Ok))]);
    }

    /// Steps at the given `(ms, value, quality)` ticks; returns every step.
    fn steps_at(
        instance: &mut Instance,
        rule: &Rule,
        ticks: &[(u64, f64, Quality)],
    ) -> Vec<(u64, Step)> {
        ticks
            .iter()
            .map(|&(ms, value, quality)| (ms, instance.step(rule, Some(value), quality, ms)))
            .collect()
    }

    #[test]
    fn next_fresh_after_held_counts_since_the_last_fresh() {
        let rule = above(Some((83.0, 3)), None, (3.0, 3));

        // The Held tick keeps the anchor at 1000; the Fresh at 3000 counts
        // the 2 s since then and enters.
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            steps_at(
                &mut instance,
                &rule,
                &[
                    (0, 95.0, Fresh),
                    (1000, 95.0, Fresh),
                    (2000, 95.0, Held),
                    (3000, 95.0, Fresh),
                ]
            ),
            [
                (0, Step::Stay),
                (1000, Step::Stay),
                (2000, Step::Stay),
                (3000, Step::Entered(Warn)),
            ]
        );
        assert_eq!(
            steps_at(
                &mut instance,
                &rule,
                &[
                    (10_000, 70.0, Fresh),
                    (11_000, 70.0, Fresh),
                    (12_000, 70.0, Held),
                    (13_000, 70.0, Fresh),
                ]
            ),
            [
                (10_000, Step::Stay),
                (11_000, Step::Stay),
                (12_000, Step::Stay),
                (13_000, left(Warn, Ok)),
            ]
        );

        // The duration is reached on a Held tick: nothing matures until the
        // next Fresh value.
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            steps_at(
                &mut instance,
                &rule,
                &[(0, 95.0, Fresh), (3000, 95.0, Held), (3500, 95.0, Fresh)]
            ),
            [
                (0, Step::Stay),
                (3000, Step::Stay),
                (3500, Step::Entered(Warn)),
            ]
        );
        assert_eq!(
            steps_at(
                &mut instance,
                &rule,
                &[
                    (20_000, 70.0, Fresh),
                    (23_000, 70.0, Held),
                    (23_500, 70.0, Fresh)
                ]
            ),
            [
                (20_000, Step::Stay),
                (23_000, Step::Stay),
                (23_500, left(Warn, Ok)),
            ]
        );
    }

    #[test]
    fn alternating_fresh_and_held_still_accrues() {
        let rule = above(Some((83.0, 10)), None, (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        let quality = |s: u64| if s % 2 == 0 { Fresh } else { Held };
        assert_eq!(
            feed(&mut instance, &rule, 0, 30, |s| (Some(95.0), quality(s))),
            [(10_000, Step::Entered(Warn))]
        );
        assert_eq!(
            feed(&mut instance, &rule, 40, 70, |s| (Some(70.0), quality(s))),
            [(50_000, left(Warn, Ok))]
        );
    }

    #[test]
    fn suspended_without_a_value_neither_resets_nor_matures() {
        let rule = above(Some((83.0, 30)), None, (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        // 20 s above the threshold, then a minute of suspension (no step),
        // then fresh ticks: the first only sets a new anchor, so the
        // suspended minute is not counted, and 10 measured seconds later the
        // 30 s are reached.
        let steps = feed(&mut instance, &rule, 0, 91, |s| match s {
            0..=20 => (Some(95.0), Fresh),
            21..=80 => (None, Suspended),
            _ => (Some(95.0), Fresh),
        });
        assert_eq!(steps, [(91_000, Step::Entered(Warn))]);
    }

    #[test]
    fn suspended_keeps_the_level_and_the_exit_timer() {
        let rule = above(Some((83.0, 0)), None, (3.0, 30));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 0, fresh(95.0)),
            [(0, Step::Entered(Warn))]
        );
        // 20 s below the band, then suspended: the level stays and the
        // accumulated exit time is kept, so 10 more measured seconds leave.
        assert!(feed(&mut instance, &rule, 10, 30, fresh(70.0)).is_empty());
        assert!(feed(&mut instance, &rule, 31, 200, |_| (None, Suspended)).is_empty());
        assert_eq!(instance.level(), Warn);
        assert_eq!(
            feed(&mut instance, &rule, 201, 211, fresh(70.0)),
            [(211_000, left(Warn, Ok))]
        );
    }

    #[test]
    fn held_without_a_value_still_resets_timers() {
        // Transport loss is not an intentional suspension: the same shape as
        // the suspended test needs the full 30 s again.
        let rule = above(Some((83.0, 30)), None, (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        let steps = feed(&mut instance, &rule, 0, 120, |s| match s {
            0..=20 => (Some(95.0), Fresh),
            21..=80 => (None, Held),
            _ => (Some(95.0), Fresh),
        });
        assert_eq!(steps, [(111_000, Step::Entered(Warn))]);
    }

    #[test]
    fn slow_fresh_measurements_separated_by_held_ticks_still_mature() {
        // M5 R1: a real fresh measurement every 10 s with Held ticks in
        // between still accrues the 30 s.
        let rule = above(Some((83.0, 30)), None, (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        let steps = feed(&mut instance, &rule, 0, 40, |s| {
            (Some(95.0), if s % 10 == 0 { Fresh } else { Held })
        });
        assert_eq!(steps, [(30_000, Step::Entered(Warn))]);
    }

    #[test]
    fn absent_value_resets_running_timers_and_keeps_the_level() {
        let rule = above(Some((83.0, 10)), None, (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        let steps = feed(&mut instance, &rule, 0, 30, |s| match s {
            9 => (None, Fresh),
            _ => (Some(95.0), Fresh),
        });
        assert_eq!(steps, [(20_000, Step::Entered(Warn))]);

        // A long gap neither leaves nor enters.
        assert!(feed(&mut instance, &rule, 31, 90, |_| (None, Fresh)).is_empty());
        assert_eq!(instance.level(), Warn);

        // A gap restarts the exit timer too.
        let steps = feed(&mut instance, &rule, 91, 130, |s| match s {
            96 => (None, Fresh),
            _ => (Some(70.0), Fresh),
        });
        assert_eq!(steps, [(107_000, left(Warn, Ok))]);
    }

    #[test]
    fn reset_timers_keeps_the_level() {
        let rule = above(Some((83.0, 10)), Some((90.0, 10)), (3.0, 10));
        let mut instance = only_instance(&rule, &gpu_schema());
        assert_eq!(
            feed(&mut instance, &rule, 0, 15, |s| (
                Some(if s <= 10 { 85.0 } else { 95.0 }),
                Fresh
            )),
            [(10_000, Step::Entered(Warn))]
        );
        // Crit timer at 4 s of 10: a reset restarts it from the next tick.
        instance.reset_timers();
        assert_eq!(instance.level(), Warn);
        assert_eq!(
            feed(&mut instance, &rule, 16, 40, fresh(95.0)),
            [(26_000, Step::Entered(Crit))]
        );
    }

    #[test]
    fn notify_change_keeps_state() {
        let schema = gpu_schema();
        let rule = builtin("gpu-temp");
        let a = only_instance(&rule, &schema);
        let mut notify = rule.clone();
        notify.notify = Notify {
            warn: true,
            crit: false,
        };
        let b = only_instance(&notify, &schema);
        assert!(same_semantics(&a, &b, &rule, &notify));
        assert!(same_semantics(&a, &a, &rule, &rule));

        // A property that resolves to the same number is the same threshold.
        let mut property = rule.clone();
        property.warn = Some(LevelSpec {
            threshold: Some(Threshold::Property {
                property: "missing".into(),
                offset: 0.0,
                fallback: 83.0,
            }),
            duration_s: 30,
        });
        let c = only_instance(&property, &schema);
        assert!(same_semantics(&a, &c, &rule, &property));
    }

    #[test]
    fn threshold_change_resets_state() {
        let schema = gpu_schema();
        let rule = builtin("gpu-temp");
        let a = only_instance(&rule, &schema);

        let mut threshold = rule.clone();
        threshold.warn = Some(fixed(84.0, 30));
        let mut duration = rule.clone();
        duration.crit = Some(fixed(90.0, 11));
        let mut hysteresis = rule.clone();
        hysteresis.hysteresis.amount = 2.0;
        let mut level_off = rule.clone();
        level_off.warn = None;
        for changed in [threshold, duration, hysteresis, level_off] {
            let b = only_instance(&changed, &schema);
            assert!(!same_semantics(&a, &b, &rule, &changed), "{changed:?}");
        }

        // Another source for the same sensor id.
        let mut moved = schema.clone();
        moved.sensors[0].source = Source::Lhm;
        let b = only_instance(&rule, &moved);
        assert!(!same_semantics(&a, &b, &rule, &rule));

        // Another sensor.
        let two = schema_with_two_gpus();
        let all = expand(std::slice::from_ref(&rule), &two);
        assert!(!same_semantics(&all[0], &all[1], &rule, &rule));
    }

    fn schema_with_two_gpus() -> Schema {
        schema(
            vec![
                device("gpu/0", DeviceKind::Gpu, &[]),
                device("gpu/1", DeviceKind::Gpu, &[]),
            ],
            vec![
                sensor("gpu/0", SensorKind::Temperature, "core", Unit::Celsius),
                sensor("gpu/1", SensorKind::Temperature, "core", Unit::Celsius),
            ],
        )
    }

    #[test]
    fn level_is_ordered_and_serialized_lowercase() {
        assert!(Ok < Warn && Warn < Crit);
        assert_eq!(
            serde_json::to_value(Crit).unwrap(),
            serde_json::json!("crit")
        );
        assert_eq!(
            serde_json::to_value(InstanceProblem::UnitMismatch).unwrap(),
            serde_json::json!("unitMismatch")
        );
    }
}
