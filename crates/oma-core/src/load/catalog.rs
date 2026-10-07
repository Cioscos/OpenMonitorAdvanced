//! The catalogue shared with the UI (`testdata/performance/catalog.json`).

use serde_json::{json, Value};

use super::plan::{presets, Component, Objective};

fn ids<T: serde::Serialize>(all: &[T]) -> Value {
    Value::Array(all.iter().map(|v| json!(v)).collect())
}

/// Kernel ids, instruction sets, load modes, RAM patterns, components, objectives and
/// the preset durations; the Vitest glossary test reads the fixture written from this.
pub fn catalog_json() -> Value {
    use oma_ipc::load::{Isa, KernelId::*, RamPattern};
    let mut presets_json = serde_json::Map::new();
    for (c, cname) in [
        (Component::Cpu, "cpu"),
        (Component::Ram, "ram"),
        (Component::Gpu, "gpu"),
    ] {
        for (o, oname) in [
            (Objective::Normal, "normal"),
            (Objective::Overclock, "overclock"),
        ] {
            let seconds: serde_json::Map<String, Value> = presets(c, o)
                .iter()
                .map(|(p, s)| (json!(p).as_str().unwrap_or_default().to_owned(), json!(s)))
                .collect();
            presets_json.insert(format!("{cname}.{oname}"), Value::Object(seconds));
        }
    }
    json!({
        "kernels": ids(&[K1, K2, K3, K4, K5, K7, K8, K9, K10]),
        "gpuKernels": ids(&[S1, S2, S4, S5, S6]),
        "isa": ids(&[Isa::Avx512, Isa::Avx2, Isa::Sse2]),
        "modes": ["steady", "variable", "light", "coreCycle", "allCore", "ramp", "alternate", "pauseResume"],
        "patterns": ids(&[
            RamPattern::MovingInversions,
            RamPattern::Modulo20,
            RamPattern::Random,
            RamPattern::Address,
            RamPattern::CrcCopy,
        ]),
        "components": ["cpu", "ram", "gpu"],
        "objectives": ["normal", "overclock"],
        "presets": presets_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_fixture_matches() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/performance/catalog.json"
        );
        let want = catalog_json();
        if std::env::var_os("OMA_WRITE_FIXTURES").is_some() {
            let text = serde_json::to_string_pretty(&want).unwrap() + "\n";
            std::fs::write(path, text).unwrap();
        }
        let got: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn catalog_lists_gpu_entries() {
        let c = catalog_json();
        assert!(c["components"].as_array().unwrap().contains(&json!("gpu")));
        assert_eq!(c["gpuKernels"].as_array().unwrap().len(), 5);
        assert_eq!(c["presets"]["gpu.normal"]["standard"], 900);
        assert_eq!(c["presets"]["gpu.overclock"]["night"], 7200);
        assert!(c["modes"]
            .as_array()
            .unwrap()
            .contains(&json!("pauseResume")));
    }
}
