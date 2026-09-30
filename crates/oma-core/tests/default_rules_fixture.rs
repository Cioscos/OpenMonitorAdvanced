//! The default rules against the schema recorded on the development machine
//! (`this-machine-schema.json`, written by the `records_this_machine_schema`
//! hardware test in `oma-win`): every rule that can apply here must find its
//! sensors, so a renamed sensor id breaks this test instead of silently
//! switching an alarm off.

use std::path::PathBuf;

use oma_core::model::Schema;
use oma_core::rules::{default_rules, expand};

/// Rules that need hardware this machine does not have (or a state it is not
/// in), so they legitimately have no instance here.
const ONLY_IF_PRESENT: [&str; 3] = ["battery-low", "cpu-throttle", "gpu-hotspot"];

fn recorded_schema() -> Schema {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("this-machine-schema.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the fixture is a valid schema")
}

fn sensors_of(schema: &Schema, rule_id: &str) -> Vec<String> {
    expand(&default_rules(), schema)
        .into_iter()
        .filter(|i| i.key.rule_id == rule_id)
        .map(|i| i.key.sensor_id)
        .collect()
}

#[test]
fn default_rules_find_their_sensors() {
    let schema = recorded_schema();
    let instances = expand(&default_rules(), &schema);
    for rule in default_rules() {
        if ONLY_IF_PRESENT.contains(&rule.id.as_str()) {
            continue;
        }
        assert!(
            instances.iter().any(|i| i.key.rule_id == rule.id),
            "rule {} finds no sensor on the recorded schema",
            rule.id
        );
    }

    assert_eq!(
        sensors_of(&schema, "cpu-temp"),
        ["cpu/0/temperature/tctl"],
        "cpu-temp"
    );
    assert_eq!(sensors_of(&schema, "disk-critical").len(), 2, "two NVMe");
    assert_eq!(sensors_of(&schema, "disk-wear").len(), 2, "two NVMe");
    assert_eq!(sensors_of(&schema, "disk-temp").len(), 4, "four disks");
}
