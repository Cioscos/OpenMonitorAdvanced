//! The UI's copy of the built-in rules (`app/src/test/fixtures/default-rules.json`,
//! read by the browser mock and the test backend in place of
//! `get_default_rules`) against `default_rules()`, so the two never drift.
//!
//! With `OMA_WRITE_FIXTURES=1` set (single-threaded), the test rewrites the
//! fixture from `default_rules()` instead of comparing.

use std::path::PathBuf;

use oma_core::rules::default_rules;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("app")
        .join("src")
        .join("test")
        .join("fixtures")
        .join("default-rules.json")
}

#[test]
fn ui_fixture_matches_the_default_rules() {
    let expected = serde_json::to_value(default_rules()).expect("the rules encode");
    let path = fixture_path();

    if std::env::var("OMA_WRITE_FIXTURES").as_deref() == Ok("1") {
        let mut text = serde_json::to_string_pretty(&expected).expect("the rules encode");
        text.push('\n');
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        return;
    }

    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let on_disk: serde_json::Value = serde_json::from_str(&text).expect("the fixture is JSON");
    assert_eq!(
        on_disk, expected,
        "app/src/test/fixtures/default-rules.json is out of date: rerun this test with OMA_WRITE_FIXTURES=1"
    );
}
