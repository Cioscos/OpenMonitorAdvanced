//! Alert rules: the rule model, the built-in rules and their validation.
//!
//! Only the model lives here. Instances (a rule expanded over the sensors it
//! matches) and the evaluation state machine build on these types.

mod validate;

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{DeviceKind, SensorKind, Unit};

pub use validate::{validate_rule, validate_rules, RuleError};

/// Most custom rules a settings file may hold.
pub const MAX_CUSTOM_RULES: usize = 256;
/// Longest entry or exit duration, in seconds.
pub const MAX_DURATION_S: u32 = 600;

/// How a sensor value is compared with a threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Condition {
    /// Fires when the value is at or above the threshold.
    Above,
    /// Fires when the value is at or below the threshold.
    Below,
    /// Fires when a flag sensor is not zero; has no threshold.
    FlagActive,
}

/// Which sensors a rule applies to. Untagged: `{"sensor": …}` or
/// `{"deviceKind": …, "sensorKind": …, "names": […]}`; an object with fields
/// of both shapes, or unknown fields, is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Target {
    /// One sensor, by its full id.
    Sensor { sensor: String },
    /// Every sensor of a device kind and sensor kind whose `<name>` segment is
    /// in `names` (empty means all).
    Selector {
        device_kind: DeviceKind,
        sensor_kind: SensorKind,
        names: Vec<String>,
    },
}

/// A threshold value. Untagged: `{"fixed": 83}` or
/// `{"property": "tempWarningC", "offset": 0, "fallback": 70}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Threshold {
    Fixed {
        fixed: f64,
    },
    /// Read from a property of the sensor's device, plus `offset`; `fallback`
    /// when the property is missing or not a number.
    Property {
        property: String,
        offset: f64,
        fallback: f64,
    },
}

/// One alert level of a rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelSpec {
    /// `None` only with [`Condition::FlagActive`].
    pub threshold: Option<Threshold>,
    /// Seconds the condition must hold before the level is entered.
    pub duration_s: u32,
}

/// How far and for how long a value must recede before a level is left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hysteresis {
    /// In the sensor's base unit; never negative.
    pub amount: f64,
    pub duration_s: u32,
}

impl Default for Hysteresis {
    fn default() -> Self {
        Self {
            amount: 3.0,
            duration_s: 10,
        }
    }
}

/// Which levels raise a toast on entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notify {
    pub warn: bool,
    pub crit: bool,
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            warn: false,
            crit: true,
        }
    }
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// Stable id: `gpu-temp` for built-in rules, `custom-<uuid>` otherwise.
    pub id: String,
    pub target: Target,
    /// Base unit of the targeted sensors.
    pub unit: Unit,
    pub condition: Condition,
    pub warn: Option<LevelSpec>,
    pub crit: Option<LevelSpec>,
    #[serde(default)]
    pub hysteresis: Hysteresis,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub notify: Notify,
}

/// Reads a field that is either missing, `null` or a value: `Some(None)` is
/// `null`, which the default `Option<Option<T>>` deserializer cannot tell
/// from a missing field.
fn null_or_value<'de, D>(deserializer: D) -> Result<Option<Option<LevelSpec>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<LevelSpec>::deserialize(deserializer).map(Some)
}

/// The fields of a built-in rule changed by the user. An absent field leaves
/// the default untouched; for `warn` and `crit` a `null` switches the level
/// off.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(
        default,
        deserialize_with = "null_or_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub warn: Option<Option<LevelSpec>>,
    #[serde(
        default,
        deserialize_with = "null_or_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub crit: Option<Option<LevelSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hysteresis: Option<Hysteresis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify: Option<Notify>,
}

/// What the settings file stores about rules.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesSettings {
    /// Per built-in rule id, the fields the user changed.
    #[serde(default)]
    pub overrides: BTreeMap<String, RuleOverride>,
    /// Complete user-defined rules.
    #[serde(default)]
    pub custom: Vec<Rule>,
}

fn fixed_level(value: f64, duration_s: u32) -> Option<LevelSpec> {
    Some(LevelSpec {
        threshold: Some(Threshold::Fixed { fixed: value }),
        duration_s,
    })
}

fn property_level(
    property: &str,
    offset: f64,
    fallback: f64,
    duration_s: u32,
) -> Option<LevelSpec> {
    Some(LevelSpec {
        threshold: Some(Threshold::Property {
            property: property.to_owned(),
            offset,
            fallback,
        }),
        duration_s,
    })
}

fn flag_level(duration_s: u32) -> Option<LevelSpec> {
    Some(LevelSpec {
        threshold: None,
        duration_s,
    })
}

/// A built-in rule with the default hysteresis, notifications and state.
fn builtin(
    id: &str,
    (device_kind, sensor_kind, names): (DeviceKind, SensorKind, &[&str]),
    unit: Unit,
    condition: Condition,
    warn: Option<LevelSpec>,
    crit: Option<LevelSpec>,
) -> Rule {
    Rule {
        id: id.to_owned(),
        target: Target::Selector {
            device_kind,
            sensor_kind,
            names: names.iter().map(|n| (*n).to_owned()).collect(),
        },
        unit,
        condition,
        warn,
        crit,
        hysteresis: Hysteresis::default(),
        enabled: true,
        notify: Notify::default(),
    }
}

/// The built-in rules, in display order.
pub fn default_rules() -> Vec<Rule> {
    use Condition::{Above, Below, FlagActive};
    use DeviceKind::{Battery, Cpu, Gpu, Memory, Storage};
    use SensorKind::{Flag, Load, Percent, Temperature};

    vec![
        builtin(
            "cpu-temp",
            (Cpu, Temperature, &["tctl", "tdie", "package"]),
            Unit::Celsius,
            Above,
            property_level("tjMaxC", -10.0, 85.0, 30),
            property_level("tjMaxC", 0.0, 95.0, 10),
        ),
        builtin(
            "cpu-throttle",
            (Cpu, Flag, &["throttle-thermal"]),
            Unit::Boolean,
            FlagActive,
            None,
            flag_level(10),
        ),
        builtin(
            "gpu-temp",
            (Gpu, Temperature, &["core"]),
            Unit::Celsius,
            Above,
            fixed_level(83.0, 30),
            fixed_level(90.0, 10),
        ),
        builtin(
            "gpu-hotspot",
            (Gpu, Temperature, &["hotspot"]),
            Unit::Celsius,
            Above,
            fixed_level(95.0, 30),
            fixed_level(105.0, 10),
        ),
        builtin(
            "gpu-mem-temp",
            (Gpu, Temperature, &["memory"]),
            Unit::Celsius,
            Above,
            fixed_level(100.0, 30),
            fixed_level(105.0, 10),
        ),
        builtin(
            "gpu-throttle",
            (Gpu, Flag, &["throttle-thermal"]),
            Unit::Boolean,
            FlagActive,
            flag_level(10),
            None,
        ),
        builtin(
            "disk-temp",
            (Storage, Temperature, &["drive"]),
            Unit::Celsius,
            Above,
            property_level("tempWarningC", 0.0, 70.0, 30),
            property_level("tempCriticalC", 0.0, 80.0, 30),
        ),
        builtin(
            "disk-wear",
            (Storage, Percent, &["wear"]),
            Unit::Percent,
            Above,
            fixed_level(90.0, 0),
            None,
        ),
        builtin(
            "disk-critical",
            (Storage, Flag, &["critical-warning"]),
            Unit::Boolean,
            FlagActive,
            None,
            flag_level(0),
        ),
        builtin(
            "volume-used",
            (Storage, Percent, &["volume-*"]),
            Unit::Percent,
            Above,
            fixed_level(90.0, 0),
            fixed_level(97.0, 0),
        ),
        builtin(
            "ram-used",
            (Memory, Load, &["used"]),
            Unit::Percent,
            Above,
            fixed_level(90.0, 60),
            fixed_level(97.0, 30),
        ),
        builtin(
            "battery-low",
            (Battery, Percent, &["charge"]),
            Unit::Percent,
            Below,
            fixed_level(15.0, 0),
            fixed_level(5.0, 0),
        ),
    ]
}

impl Rule {
    /// This rule with `over` applied: the fields it sets replace the rule's.
    fn with_override(mut self, over: &RuleOverride) -> Rule {
        if let Some(enabled) = over.enabled {
            self.enabled = enabled;
        }
        if let Some(warn) = &over.warn {
            self.warn = warn.clone();
        }
        if let Some(crit) = &over.crit {
            self.crit = crit.clone();
        }
        if let Some(hysteresis) = over.hysteresis {
            self.hysteresis = hysteresis;
        }
        if let Some(notify) = over.notify {
            self.notify = notify;
        }
        self
    }
}

/// The built-in rules with the overrides applied (overrides for unknown ids
/// are ignored), followed by the custom rules.
pub fn effective_rules(settings: &RulesSettings) -> Vec<Rule> {
    let mut rules: Vec<Rule> = default_rules()
        .into_iter()
        .map(|rule| match settings.overrides.get(&rule.id) {
            Some(over) => rule.with_override(over),
            None => rule,
        })
        .collect();
    rules.extend(settings.custom.iter().cloned());
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixed(value: f64, duration_s: u32) -> Option<LevelSpec> {
        Some(LevelSpec {
            threshold: Some(Threshold::Fixed { fixed: value }),
            duration_s,
        })
    }

    fn by_id(rules: &[Rule], id: &str) -> Rule {
        rules
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no rule {id}"))
            .clone()
    }

    fn selector(device: DeviceKind, sensor: SensorKind, names: &[&str]) -> Target {
        Target::Selector {
            device_kind: device,
            sensor_kind: sensor,
            names: names.iter().map(|n| (*n).to_owned()).collect(),
        }
    }

    fn prop(property: &str, offset: f64, fallback: f64, duration_s: u32) -> Option<LevelSpec> {
        Some(LevelSpec {
            threshold: Some(Threshold::Property {
                property: property.into(),
                offset,
                fallback,
            }),
            duration_s,
        })
    }

    fn flag(duration_s: u32) -> Option<LevelSpec> {
        Some(LevelSpec {
            threshold: None,
            duration_s,
        })
    }

    #[test]
    fn defaults_match_the_spec_table() {
        use Condition::{Above, Below, FlagActive};
        use DeviceKind::{Battery, Cpu, Gpu, Memory, Storage};
        use SensorKind::{Flag, Load, Percent, Temperature};
        let ids: Vec<String> = default_rules().into_iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            [
                "cpu-temp",
                "cpu-throttle",
                "gpu-temp",
                "gpu-hotspot",
                "gpu-mem-temp",
                "gpu-throttle",
                "disk-temp",
                "disk-wear",
                "disk-critical",
                "volume-used",
                "ram-used",
                "battery-low"
            ]
        );
        // (id, target, unit, condition, warn, crit)
        #[allow(clippy::type_complexity)]
        let table: Vec<(
            &str,
            Target,
            Unit,
            Condition,
            Option<LevelSpec>,
            Option<LevelSpec>,
        )> = vec![
            (
                "cpu-temp",
                selector(Cpu, Temperature, &["tctl", "tdie", "package"]),
                Unit::Celsius,
                Above,
                prop("tjMaxC", -10.0, 85.0, 30),
                prop("tjMaxC", 0.0, 95.0, 10),
            ),
            (
                "cpu-throttle",
                selector(Cpu, Flag, &["throttle-thermal"]),
                Unit::Boolean,
                FlagActive,
                None,
                flag(10),
            ),
            (
                "gpu-temp",
                selector(Gpu, Temperature, &["core"]),
                Unit::Celsius,
                Above,
                fixed(83.0, 30),
                fixed(90.0, 10),
            ),
            (
                "gpu-hotspot",
                selector(Gpu, Temperature, &["hotspot"]),
                Unit::Celsius,
                Above,
                fixed(95.0, 30),
                fixed(105.0, 10),
            ),
            (
                "gpu-mem-temp",
                selector(Gpu, Temperature, &["memory"]),
                Unit::Celsius,
                Above,
                fixed(100.0, 30),
                fixed(105.0, 10),
            ),
            (
                "gpu-throttle",
                selector(Gpu, Flag, &["throttle-thermal"]),
                Unit::Boolean,
                FlagActive,
                flag(10),
                None,
            ),
            (
                "disk-temp",
                selector(Storage, Temperature, &["drive"]),
                Unit::Celsius,
                Above,
                prop("tempWarningC", 0.0, 70.0, 30),
                prop("tempCriticalC", 0.0, 80.0, 30),
            ),
            (
                "disk-wear",
                selector(Storage, Percent, &["wear"]),
                Unit::Percent,
                Above,
                fixed(90.0, 0),
                None,
            ),
            (
                "disk-critical",
                selector(Storage, Flag, &["critical-warning"]),
                Unit::Boolean,
                FlagActive,
                None,
                flag(0),
            ),
            (
                "volume-used",
                selector(Storage, Percent, &["volume-*"]),
                Unit::Percent,
                Above,
                fixed(90.0, 0),
                fixed(97.0, 0),
            ),
            (
                "ram-used",
                selector(Memory, Load, &["used"]),
                Unit::Percent,
                Above,
                fixed(90.0, 60),
                fixed(97.0, 30),
            ),
            (
                "battery-low",
                selector(Battery, Percent, &["charge"]),
                Unit::Percent,
                Below,
                fixed(15.0, 0),
                fixed(5.0, 0),
            ),
        ];
        let rules = default_rules();
        assert_eq!(rules.len(), table.len());
        for (rule, (id, target, unit, condition, warn, crit)) in rules.iter().zip(table) {
            assert_eq!(rule.id, id);
            assert_eq!(rule.target, target, "{id} target");
            assert_eq!(rule.unit, unit, "{id} unit");
            assert_eq!(rule.condition, condition, "{id} condition");
            assert_eq!(rule.warn, warn, "{id} warn");
            assert_eq!(rule.crit, crit, "{id} crit");
            assert_eq!(
                rule.hysteresis,
                Hysteresis {
                    amount: 3.0,
                    duration_s: 10
                },
                "{id} hysteresis"
            );
            assert_eq!(
                rule.notify,
                Notify {
                    warn: false,
                    crit: true
                },
                "{id} notify"
            );
            assert!(rule.enabled, "{id} enabled");
        }
    }

    #[test]
    fn every_default_rule_is_valid() {
        for rule in default_rules() {
            assert_eq!(validate_rule(&rule), Ok(()), "{}", rule.id);
        }
        assert_eq!(validate_rules(&RulesSettings::default()), Ok(()));
    }

    #[test]
    fn override_replaces_fields_and_null_disables_a_level() {
        let mut settings = RulesSettings::default();
        settings.overrides.insert(
            "gpu-temp".into(),
            RuleOverride {
                enabled: Some(false),
                warn: Some(fixed(80.0, 5)),
                crit: Some(None),
                hysteresis: Some(Hysteresis {
                    amount: 1.0,
                    duration_s: 2,
                }),
                notify: Some(Notify {
                    warn: true,
                    crit: false,
                }),
            },
        );
        let rules = effective_rules(&settings);
        let gpu = by_id(&rules, "gpu-temp");
        assert!(!gpu.enabled);
        assert_eq!(gpu.warn, fixed(80.0, 5));
        assert_eq!(gpu.crit, None);
        assert_eq!(
            gpu.hysteresis,
            Hysteresis {
                amount: 1.0,
                duration_s: 2
            }
        );
        assert_eq!(
            gpu.notify,
            Notify {
                warn: true,
                crit: false
            }
        );
        // The untouched fields and the other rules stay as they were.
        let default = by_id(&default_rules(), "gpu-temp");
        assert_eq!(gpu.target, default.target);
        assert_eq!(gpu.unit, default.unit);
        assert_eq!(
            by_id(&rules, "gpu-hotspot"),
            by_id(&default_rules(), "gpu-hotspot")
        );
    }

    #[test]
    fn absent_override_field_leaves_the_default_level() {
        let mut settings = RulesSettings::default();
        settings.overrides.insert(
            "gpu-temp".into(),
            RuleOverride {
                enabled: Some(false),
                ..RuleOverride::default()
            },
        );
        let gpu = by_id(&effective_rules(&settings), "gpu-temp");
        let default = by_id(&default_rules(), "gpu-temp");
        assert_eq!(gpu.warn, default.warn);
        assert_eq!(gpu.crit, default.crit);
    }

    #[test]
    fn override_for_an_unknown_rule_is_ignored() {
        let mut settings = RulesSettings::default();
        settings.overrides.insert(
            "no-such-rule".into(),
            RuleOverride {
                enabled: Some(false),
                ..RuleOverride::default()
            },
        );
        assert_eq!(effective_rules(&settings), default_rules());
    }

    #[test]
    fn custom_rules_follow_the_defaults() {
        let custom = Rule {
            id: "custom-00000000-0000-4000-8000-000000000001".into(),
            target: Target::Sensor {
                sensor: "cpu/0/temperature/package".into(),
            },
            unit: Unit::Celsius,
            condition: Condition::Above,
            warn: fixed(70.0, 0),
            crit: None,
            hysteresis: Hysteresis::default(),
            enabled: true,
            notify: Notify::default(),
        };
        let settings = RulesSettings {
            overrides: BTreeMap::new(),
            custom: vec![custom.clone()],
        };
        let rules = effective_rules(&settings);
        let defaults = default_rules();
        assert_eq!(rules.len(), defaults.len() + 1);
        assert_eq!(rules[..defaults.len()], defaults[..]);
        assert_eq!(rules.last(), Some(&custom));
    }

    #[test]
    fn rule_json_round_trips() {
        let gpu = by_id(&default_rules(), "gpu-temp");
        let gpu_json = json!({
            "id": "gpu-temp",
            "target": { "deviceKind": "gpu", "sensorKind": "temperature", "names": ["core"] },
            "unit": "celsius",
            "condition": "above",
            "warn": { "threshold": { "fixed": 83.0 }, "durationS": 30 },
            "crit": { "threshold": { "fixed": 90.0 }, "durationS": 10 },
            "hysteresis": { "amount": 3.0, "durationS": 10 },
            "enabled": true,
            "notify": { "warn": false, "crit": true }
        });
        assert_eq!(serde_json::to_value(&gpu).unwrap(), gpu_json);
        assert_eq!(serde_json::from_value::<Rule>(gpu_json).unwrap(), gpu);

        let disk = Rule {
            id: "custom-00000000-0000-4000-8000-000000000002".into(),
            target: Target::Sensor {
                sensor: "storage/0/temperature/drive".into(),
            },
            unit: Unit::Celsius,
            condition: Condition::Above,
            warn: prop("tempWarningC", -5.0, 70.0, 30),
            crit: None,
            hysteresis: Hysteresis::default(),
            enabled: false,
            notify: Notify {
                warn: true,
                crit: true,
            },
        };
        let disk_json = json!({
            "id": "custom-00000000-0000-4000-8000-000000000002",
            "target": { "sensor": "storage/0/temperature/drive" },
            "unit": "celsius",
            "condition": "above",
            "warn": {
                "threshold": { "property": "tempWarningC", "offset": -5.0, "fallback": 70.0 },
                "durationS": 30
            },
            "crit": null,
            "hysteresis": { "amount": 3.0, "durationS": 10 },
            "enabled": false,
            "notify": { "warn": true, "crit": true }
        });
        assert_eq!(serde_json::to_value(&disk).unwrap(), disk_json);
        assert_eq!(serde_json::from_value::<Rule>(disk_json).unwrap(), disk);

        // A flag level carries no threshold; the model enums keep snake_case.
        let throttle = by_id(&default_rules(), "gpu-throttle");
        let value = serde_json::to_value(&throttle).unwrap();
        assert_eq!(value["unit"], json!("boolean"));
        assert_eq!(value["condition"], json!("flagActive"));
        assert_eq!(value["warn"], json!({ "threshold": null, "durationS": 10 }));
        assert_eq!(serde_json::from_value::<Rule>(value).unwrap(), throttle);
        assert_eq!(
            serde_json::to_value(Unit::BytesPerSecond).unwrap(),
            json!("bytes_per_second")
        );
    }

    #[test]
    fn override_json_distinguishes_missing_null_and_value() {
        let empty: RuleOverride = serde_json::from_value(json!({})).unwrap();
        assert_eq!(empty, RuleOverride::default());
        assert_eq!(serde_json::to_value(&empty).unwrap(), json!({}));

        let off: RuleOverride = serde_json::from_value(json!({ "crit": null })).unwrap();
        assert_eq!(off.crit, Some(None));
        assert_eq!(off.warn, None);
        assert_eq!(serde_json::to_value(&off).unwrap(), json!({ "crit": null }));

        let value = json!({ "crit": { "threshold": { "fixed": 88.0 }, "durationS": 4 } });
        let set: RuleOverride = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(set.crit, Some(fixed(88.0, 4)));
        assert_eq!(serde_json::to_value(&set).unwrap(), value);

        // The same holds inside the settings, through encode.
        let mut settings = RulesSettings::default();
        settings.overrides.insert("gpu-temp".into(), off);
        settings.overrides.insert("gpu-hotspot".into(), set);
        settings
            .overrides
            .insert("ram-used".into(), RuleOverride::default());
        let encoded = crate::settings::encode(&crate::settings::Settings {
            rules: serde_json::to_value(&settings).unwrap(),
            ..crate::settings::Settings::default()
        });
        let back: RulesSettings = serde_json::from_value(encoded["rules"].clone()).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn ambiguous_target_and_threshold_are_rejected() {
        let both = json!({
            "sensor": "cpu/0/temperature/package",
            "deviceKind": "cpu", "sensorKind": "temperature", "names": []
        });
        assert!(serde_json::from_value::<Target>(both).is_err());
        let unknown = json!({ "sensor": "cpu/0/temperature/package", "extra": 1 });
        assert!(serde_json::from_value::<Target>(unknown).is_err());
        assert!(serde_json::from_value::<Target>(json!({})).is_err());

        let both = json!({ "fixed": 1.0, "property": "p", "offset": 0.0, "fallback": 1.0 });
        assert!(serde_json::from_value::<Threshold>(both).is_err());
        let unknown = json!({ "fixed": 1.0, "extra": 1 });
        assert!(serde_json::from_value::<Threshold>(unknown).is_err());
        // Each shape alone still parses.
        assert_eq!(
            serde_json::from_value::<Threshold>(json!({ "fixed": 1.0 })).unwrap(),
            Threshold::Fixed { fixed: 1.0 }
        );
        assert_eq!(
            serde_json::from_value::<Target>(json!({ "sensor": "a/temperature/b" })).unwrap(),
            Target::Sensor {
                sensor: "a/temperature/b".into()
            }
        );
    }
}
