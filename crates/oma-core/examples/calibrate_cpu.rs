//! `cargo run -p oma-core --example calibrate_cpu -- <score.json>`: rewrites
//! `src/scores/cpu-1-baseline.json` with the rates of a score file (4 significant digits)
//! and `provisional: false`.

use std::collections::BTreeMap;

use oma_core::scores::{parse_score, Baseline, BenchKernel, SCORE_VERSION};

fn round4(x: f64) -> f64 {
    format!("{x:.3e}").parse().unwrap_or(x)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: calibrate_cpu <score.json>")?;
    let score = parse_score(&std::fs::read(path)?)?;
    let mut single: BTreeMap<BenchKernel, f64> = BTreeMap::new();
    let mut multi: BTreeMap<BenchKernel, f64> = BTreeMap::new();
    for k in &score.kernels {
        if let Some(v) = k.single {
            single.insert(k.id, round4(v));
        }
        if let Some(v) = k.multi {
            multi.insert(k.id, round4(v));
        }
    }
    if single.len() != 6 || multi.len() != 6 {
        return Err("the score file needs all six kernels in both modes".into());
    }
    let baseline = Baseline {
        version: SCORE_VERSION.into(),
        provisional: false,
        single,
        multi,
    };
    let out = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/scores/cpu-1-baseline.json"
    );
    std::fs::write(out, serde_json::to_string_pretty(&baseline)? + "\n")?;
    println!("wrote {out}");
    Ok(())
}
