//! Per-field source selection across layers of decreasing priority (spec §5.2).

use std::collections::{BTreeMap, BTreeSet};

/// For each field, the index of the highest-priority layer supporting it.
/// `supported[i]` = fields layer `i` supports; lower index = higher priority.
pub fn assign<F: Ord + Copy>(supported: &[BTreeSet<F>]) -> BTreeMap<F, usize> {
    let mut owners = BTreeMap::new();
    for (layer, fields) in supported.iter().enumerate() {
        for &field in fields {
            // Layers are visited in priority order, so the first owner stays.
            owners.entry(field).or_insert(layer);
        }
    }
    owners
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(fields: &[&'static str]) -> BTreeSet<&'static str> {
        fields.iter().copied().collect()
    }

    #[test]
    fn highest_priority_layer_wins() {
        let owners = assign(&[set(&["temp", "clock"]), set(&["temp", "fan"])]);
        assert_eq!(owners["temp"], 0);
        assert_eq!(owners["clock"], 0);
    }

    #[test]
    fn field_only_in_a_low_priority_layer_is_still_assigned() {
        let owners = assign(&[set(&["temp"]), set(&[]), set(&["temp", "load"])]);
        assert_eq!(owners["load"], 2);
        assert_eq!(owners.len(), 2);
    }

    #[test]
    fn empty_input_gives_an_empty_map() {
        assert!(assign::<u8>(&[]).is_empty());
        assert!(assign::<u8>(&[BTreeSet::new(), BTreeSet::new()]).is_empty());
    }

    #[test]
    fn every_field_has_exactly_the_first_layer_that_supports_it() {
        let layers = [
            set(&["a", "b"]),
            set(&["b", "c"]),
            set(&["a", "c", "d"]),
            set(&["d", "e"]),
        ];
        let owners = assign(&layers);
        let all: BTreeSet<_> = layers.iter().flatten().copied().collect();
        assert_eq!(owners.keys().copied().collect::<BTreeSet<_>>(), all);
        for (field, &owner) in &owners {
            let first = layers.iter().position(|l| l.contains(field)).unwrap();
            assert_eq!(owner, first, "field {field}");
        }
        assert_eq!(
            owners.into_iter().collect::<Vec<_>>(),
            vec![("a", 0), ("b", 0), ("c", 1), ("d", 2), ("e", 3)]
        );
    }
}
