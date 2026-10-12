//! The variable contract shared with the UI (`app/src/themes/variables.json`).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

const VARIABLES_JSON: &str = include_str!("../../../../app/src/themes/variables.json");

/// What a variable accepts (spec M10 §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VarKind {
    /// `light` or `dark`.
    Scheme,
    Color,
    /// A number of `px` within `min..=max`.
    Length,
    FontList,
}

/// One variable of the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VarSpec {
    pub name: String,
    pub group: String,
    pub kind: VarKind,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// The variables of the contract, in the order of the guide.
pub fn variables() -> &'static [VarSpec] {
    static VARS: OnceLock<Vec<VarSpec>> = OnceLock::new();
    VARS.get_or_init(|| {
        serde_json::from_str(VARIABLES_JSON).expect("variables.json is valid (tested)")
    })
}

/// The contract entry for `name` (case-sensitive, like custom properties).
pub fn variable(name: &str) -> Option<&'static VarSpec> {
    variables().iter().find(|v| v.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_has_the_22_variables_in_order() {
        let vars = variables();
        assert_eq!(vars.len(), 22);
        assert_eq!(vars[0].name, "--color-scheme");
        assert_eq!(vars[0].kind, VarKind::Scheme);
        let radius = variable("--radius").unwrap();
        assert_eq!(radius.kind, VarKind::Length);
        assert_eq!((radius.min, radius.max), (Some(0.0), Some(24.0)));
        assert_eq!(variable("--font").unwrap().kind, VarKind::FontList);
        assert_eq!(variable("--font").unwrap().group, "font");
        assert!(variable("--topbar-h").is_none());
    }
}
