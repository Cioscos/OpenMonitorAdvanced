//! Validation of rules and of the rules section of the settings.

use std::collections::HashSet;

use super::{
    default_rules, Condition, Hysteresis, LevelSpec, Rule, RuleOverride, RulesSettings, Target,
    Threshold, MAX_CUSTOM_RULES, MAX_DURATION_S,
};
use crate::model::{SensorKind, Unit};

/// Why a rule is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    /// Path of the offending field, relative to the rule (`warn.durationS`).
    pub field: String,
    /// i18n key of the message (`rules.error.order`).
    pub key: &'static str,
}

fn err(field: &str, key: &'static str) -> RuleError {
    RuleError {
        field: field.to_owned(),
        key,
    }
}

/// `<device>/<kind>/<name>`, where the device id may itself contain `/`.
fn is_sensor_id(id: &str) -> bool {
    let mut parts = id.rsplitn(3, '/');
    let (Some(name), Some(kind), Some(device)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let known_kind = serde_json::from_value::<SensorKind>(kind.into()).is_ok();
    known_kind && !name.is_empty() && device.split('/').all(|segment| !segment.is_empty())
}

fn validate_target(target: &Target) -> Result<(), RuleError> {
    let valid = match target {
        Target::Sensor { sensor } => is_sensor_id(sensor),
        // A `*` is allowed only at the end, as a prefix match (R8).
        Target::Selector { names, .. } => names.iter().all(|n| {
            let body = n.strip_suffix('*').unwrap_or(n);
            !n.is_empty() && !n.contains('/') && !body.contains('*')
        }),
    };
    if valid {
        Ok(())
    } else {
        Err(err("target", "rules.error.target"))
    }
}

/// Checks one level on its own; `name` is `warn` or `crit`.
fn validate_level(name: &str, level: &LevelSpec, condition: Condition) -> Result<(), RuleError> {
    let threshold_field = format!("{name}.threshold");
    match (&level.threshold, condition) {
        (None, Condition::FlagActive) => {}
        (Some(_), Condition::FlagActive) | (None, _) => {
            return Err(err(&threshold_field, "rules.error.threshold"));
        }
        (Some(Threshold::Fixed { fixed }), _) => {
            if !fixed.is_finite() {
                return Err(err(&threshold_field, "rules.error.nonFinite"));
            }
        }
        (
            Some(Threshold::Property {
                property,
                offset,
                fallback,
            }),
            _,
        ) => {
            if property.is_empty() {
                return Err(err(&threshold_field, "rules.error.threshold"));
            }
            if !offset.is_finite() || !fallback.is_finite() {
                return Err(err(&threshold_field, "rules.error.nonFinite"));
            }
        }
    }
    if level.duration_s > MAX_DURATION_S {
        return Err(err(&format!("{name}.durationS"), "rules.error.duration"));
    }
    Ok(())
}

fn validate_hysteresis(hysteresis: &Hysteresis) -> Result<(), RuleError> {
    if !hysteresis.amount.is_finite() {
        return Err(err("hysteresis.amount", "rules.error.nonFinite"));
    }
    if hysteresis.amount < 0.0 {
        return Err(err("hysteresis.amount", "rules.error.hysteresis"));
    }
    if hysteresis.duration_s > MAX_DURATION_S {
        return Err(err("hysteresis.durationS", "rules.error.duration"));
    }
    Ok(())
}

/// Checks everything that can be checked on one rule alone. The order of
/// thresholds is only checked when both are fixed; with properties it depends
/// on the sensor's device.
pub fn validate_rule(rule: &Rule) -> Result<(), RuleError> {
    validate_target(&rule.target)?;
    let flag = rule.condition == Condition::FlagActive;
    if flag != (rule.unit == Unit::Boolean) {
        return Err(err("condition", "rules.error.condition"));
    }
    if rule.warn.is_none() && rule.crit.is_none() {
        return Err(err("levels", "rules.error.noLevel"));
    }
    if let Some(warn) = &rule.warn {
        validate_level("warn", warn, rule.condition)?;
    }
    if let Some(crit) = &rule.crit {
        validate_level("crit", crit, rule.condition)?;
    }
    validate_hysteresis(&rule.hysteresis)?;

    let fixed_of = |level: &Option<LevelSpec>| match level.as_ref()?.threshold {
        Some(Threshold::Fixed { fixed }) => Some(fixed),
        _ => None,
    };
    if let (Some(warn), Some(crit)) = (fixed_of(&rule.warn), fixed_of(&rule.crit)) {
        let misordered = match rule.condition {
            Condition::Above => crit < warn,
            Condition::Below => crit > warn,
            Condition::FlagActive => false,
        };
        if misordered {
            return Err(err("crit", "rules.error.order"));
        }
    }
    Ok(())
}

/// `custom-` followed by 36 lowercase hexadecimal digits and dashes.
fn is_custom_id(id: &str) -> bool {
    id.strip_prefix("custom-").is_some_and(|uuid| {
        uuid.len() == 36
            && uuid
                .bytes()
                .all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Admits the custom rules of a section one at a time, in order, with the
/// checks that span the list: id shape, validity, unique id and the count.
/// Invalid rules do not claim their id, so a later valid rule with the same
/// id is still admitted.
#[derive(Default)]
pub(crate) struct CustomRules {
    seen: HashSet<String>,
}

pub(crate) const TOO_MANY: &str = "rules.error.tooMany";

impl CustomRules {
    /// Checks `rule` as the next custom rule; on success it counts towards
    /// the limit and its id is taken. `field` is relative to the rule; it is
    /// empty for [`TOO_MANY`].
    pub(crate) fn admit(&mut self, rule: &Rule) -> Result<(), RuleError> {
        if !is_custom_id(&rule.id) {
            return Err(err("id", "rules.error.customId"));
        }
        validate_rule(rule)?;
        if self.seen.contains(&rule.id) {
            return Err(err("id", "rules.error.duplicateId"));
        }
        if self.seen.len() >= MAX_CUSTOM_RULES {
            return Err(err("", TOO_MANY));
        }
        self.seen.insert(rule.id.clone());
        Ok(())
    }
}

/// Checks the override of the built-in rule `id` on the rule it produces.
/// `field` is relative to the override; it is empty for an unknown id.
pub(crate) fn validate_override(id: &str, over: &RuleOverride) -> Result<(), RuleError> {
    let Some(default) = default_rules().into_iter().find(|r| r.id == id) else {
        return Err(err("", "rules.error.unknownRule"));
    };
    validate_rule(&default.with_override(over))
}

/// `path.field`, or `path` alone for an empty field.
pub(crate) fn nested(path: &str, field: &str) -> String {
    if field.is_empty() {
        path.to_owned()
    } else {
        format!("{path}.{field}")
    }
}

/// Checks the whole rules section; on failure returns the settings path of the
/// first problem (`rules.custom.3.warn.durationS`) and its i18n key.
pub fn validate_rules(settings: &RulesSettings) -> Result<(), (String, &'static str)> {
    for (id, over) in &settings.overrides {
        validate_override(id, over)
            .map_err(|e| (nested(&format!("rules.overrides.{id}"), &e.field), e.key))?;
    }

    let mut custom = CustomRules::default();
    for (i, rule) in settings.custom.iter().enumerate() {
        custom.admit(rule).map_err(|e| {
            if e.key == TOO_MANY {
                ("rules.custom".to_owned(), e.key)
            } else {
                (nested(&format!("rules.custom.{i}"), &e.field), e.key)
            }
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    fn fixed_level(value: f64, duration_s: u32) -> Option<LevelSpec> {
        Some(LevelSpec {
            threshold: Some(Threshold::Fixed { fixed: value }),
            duration_s,
        })
    }

    fn base() -> Rule {
        Rule {
            id: "custom-00000000-0000-4000-8000-000000000001".into(),
            target: Target::Sensor {
                sensor: "cpu/0/temperature/package".into(),
            },
            unit: Unit::Celsius,
            condition: Condition::Above,
            warn: fixed_level(80.0, 10),
            crit: fixed_level(90.0, 5),
            hysteresis: Hysteresis::default(),
            enabled: true,
            notify: Notify::default(),
        }
    }

    fn error_of(rule: &Rule) -> (String, &'static str) {
        let e = validate_rule(rule).expect_err("rule should be invalid");
        (e.field, e.key)
    }

    fn custom_with_id(n: usize) -> Rule {
        Rule {
            id: format!("custom-00000000-0000-4000-8000-{n:012x}"),
            ..base()
        }
    }

    #[test]
    fn a_plain_rule_is_valid() {
        assert_eq!(validate_rule(&base()), Ok(()));
    }

    #[test]
    fn validation_rejects() {
        let no_level = Rule {
            warn: None,
            crit: None,
            ..base()
        };
        assert_eq!(error_of(&no_level).1, "rules.error.noLevel");

        let above_order = Rule {
            warn: fixed_level(90.0, 0),
            crit: fixed_level(80.0, 0),
            ..base()
        };
        assert_eq!(
            error_of(&above_order),
            ("crit".to_owned(), "rules.error.order")
        );
        let below_order = Rule {
            condition: Condition::Below,
            unit: Unit::Percent,
            warn: fixed_level(15.0, 0),
            crit: fixed_level(20.0, 0),
            ..base()
        };
        assert_eq!(error_of(&below_order).1, "rules.error.order");
        // Equal thresholds are fine, and so is the right order for below.
        let equal = Rule {
            warn: fixed_level(80.0, 0),
            crit: fixed_level(80.0, 0),
            ..base()
        };
        assert_eq!(validate_rule(&equal), Ok(()));
        let below_ok = Rule {
            warn: fixed_level(20.0, 0),
            crit: fixed_level(15.0, 0),
            ..below_order.clone()
        };
        assert_eq!(validate_rule(&below_ok), Ok(()));
        // With a property the order is left to the per-instance check.
        let with_property = Rule {
            warn: Some(LevelSpec {
                threshold: Some(Threshold::Property {
                    property: "tjMaxC".into(),
                    offset: 0.0,
                    fallback: 95.0,
                }),
                duration_s: 0,
            }),
            crit: fixed_level(10.0, 0),
            ..base()
        };
        assert_eq!(validate_rule(&with_property), Ok(()));

        let long = Rule {
            warn: fixed_level(80.0, 601),
            ..base()
        };
        assert_eq!(
            error_of(&long),
            ("warn.durationS".to_owned(), "rules.error.duration")
        );
        let at_limit = Rule {
            warn: fixed_level(80.0, 600),
            ..base()
        };
        assert_eq!(validate_rule(&at_limit), Ok(()));
        let long_exit = Rule {
            hysteresis: Hysteresis {
                amount: 3.0,
                duration_s: 601,
            },
            ..base()
        };
        assert_eq!(
            error_of(&long_exit),
            ("hysteresis.durationS".to_owned(), "rules.error.duration")
        );

        let negative = Rule {
            hysteresis: Hysteresis {
                amount: -1.0,
                duration_s: 0,
            },
            ..base()
        };
        assert_eq!(
            error_of(&negative),
            ("hysteresis.amount".to_owned(), "rules.error.hysteresis")
        );

        let nan = Rule {
            warn: fixed_level(f64::NAN, 0),
            ..base()
        };
        assert_eq!(
            error_of(&nan),
            ("warn.threshold".to_owned(), "rules.error.nonFinite")
        );
        let infinite_hysteresis = Rule {
            hysteresis: Hysteresis {
                amount: f64::INFINITY,
                duration_s: 0,
            },
            ..base()
        };
        assert_eq!(error_of(&infinite_hysteresis).1, "rules.error.nonFinite");
        let infinite_offset = Rule {
            warn: Some(LevelSpec {
                threshold: Some(Threshold::Property {
                    property: "tjMaxC".into(),
                    offset: f64::INFINITY,
                    fallback: 85.0,
                }),
                duration_s: 0,
            }),
            ..base()
        };
        assert_eq!(error_of(&infinite_offset).1, "rules.error.nonFinite");

        let flag_on_celsius = Rule {
            condition: Condition::FlagActive,
            warn: Some(LevelSpec {
                threshold: None,
                duration_s: 0,
            }),
            crit: None,
            ..base()
        };
        assert_eq!(
            error_of(&flag_on_celsius),
            ("condition".to_owned(), "rules.error.condition")
        );
        let above_on_boolean = Rule {
            unit: Unit::Boolean,
            ..base()
        };
        assert_eq!(error_of(&above_on_boolean).1, "rules.error.condition");

        let missing_threshold = Rule {
            warn: Some(LevelSpec {
                threshold: None,
                duration_s: 0,
            }),
            ..base()
        };
        assert_eq!(
            error_of(&missing_threshold),
            ("warn.threshold".to_owned(), "rules.error.threshold")
        );
        let flag_with_threshold = Rule {
            condition: Condition::FlagActive,
            unit: Unit::Boolean,
            warn: fixed_level(1.0, 0),
            crit: None,
            ..base()
        };
        assert_eq!(error_of(&flag_with_threshold).1, "rules.error.threshold");

        for bad in [
            "cpu",
            "cpu/0/temperature",
            "cpu/0/bogus/package",
            "/temperature/x",
            "cpu/0/temperature/",
        ] {
            let rule = Rule {
                target: Target::Sensor {
                    sensor: bad.to_owned(),
                },
                ..base()
            };
            assert_eq!(
                error_of(&rule),
                ("target".to_owned(), "rules.error.target"),
                "{bad}"
            );
        }
        let device_with_slashes = Rule {
            target: Target::Sensor {
                sensor: "gpu/pci-0000:01:00.0/temperature/core".into(),
            },
            ..base()
        };
        assert_eq!(validate_rule(&device_with_slashes), Ok(()));
        for names in [
            vec![String::new()],
            vec!["a/b".to_owned()],
            vec!["a*b".to_owned()],
            vec!["**".to_owned()],
        ] {
            let rule = Rule {
                target: Target::Selector {
                    device_kind: DeviceKind::Gpu,
                    sensor_kind: SensorKind::Temperature,
                    names,
                },
                ..base()
            };
            assert_eq!(error_of(&rule).1, "rules.error.target");
        }
    }

    #[test]
    fn rules_section_validation_rejects() {
        let mut settings = RulesSettings {
            custom: (0..=MAX_CUSTOM_RULES).map(custom_with_id).collect(),
            ..RulesSettings::default()
        };
        assert_eq!(
            validate_rules(&settings),
            Err(("rules.custom".to_owned(), "rules.error.tooMany"))
        );
        settings.custom.truncate(MAX_CUSTOM_RULES);
        assert_eq!(validate_rules(&settings), Ok(()));

        let bad_id = RulesSettings {
            custom: vec![Rule {
                id: "custom-x".into(),
                ..base()
            }],
            ..RulesSettings::default()
        };
        assert_eq!(
            validate_rules(&bad_id),
            Err(("rules.custom.0.id".to_owned(), "rules.error.customId"))
        );
        let taking_a_default_id = RulesSettings {
            custom: vec![Rule {
                id: "gpu-temp".into(),
                ..base()
            }],
            ..RulesSettings::default()
        };
        assert_eq!(
            validate_rules(&taking_a_default_id).unwrap_err().1,
            "rules.error.customId"
        );

        let duplicate = RulesSettings {
            custom: vec![custom_with_id(1), custom_with_id(2), custom_with_id(1)],
            ..RulesSettings::default()
        };
        assert_eq!(
            validate_rules(&duplicate),
            Err(("rules.custom.2.id".to_owned(), "rules.error.duplicateId"))
        );

        let bad_level = RulesSettings {
            custom: vec![
                custom_with_id(1),
                custom_with_id(2),
                custom_with_id(3),
                Rule {
                    warn: fixed_level(80.0, 601),
                    ..custom_with_id(4)
                },
            ],
            ..RulesSettings::default()
        };
        assert_eq!(
            validate_rules(&bad_level),
            Err((
                "rules.custom.3.warn.durationS".to_owned(),
                "rules.error.duration"
            ))
        );

        let mut unknown = RulesSettings::default();
        unknown
            .overrides
            .insert("no-such-rule".into(), RuleOverride::default());
        assert_eq!(
            validate_rules(&unknown),
            Err((
                "rules.overrides.no-such-rule".to_owned(),
                "rules.error.unknownRule"
            ))
        );

        // An override is validated on the rule it produces.
        let mut broken_override = RulesSettings::default();
        broken_override.overrides.insert(
            "gpu-temp".into(),
            RuleOverride {
                warn: Some(fixed_level(95.0, 0)),
                ..RuleOverride::default()
            },
        );
        assert_eq!(
            validate_rules(&broken_override),
            Err((
                "rules.overrides.gpu-temp.crit".to_owned(),
                "rules.error.order"
            ))
        );
        let mut no_levels = RulesSettings::default();
        no_levels.overrides.insert(
            "gpu-throttle".into(),
            RuleOverride {
                warn: Some(None),
                ..RuleOverride::default()
            },
        );
        assert_eq!(
            validate_rules(&no_levels).unwrap_err().1,
            "rules.error.noLevel"
        );
    }
}
