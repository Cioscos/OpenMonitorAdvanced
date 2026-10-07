//! `cargo run -p oma-core --example calibrate_cpu -- <score.json>`: rewrites
//! `src/scores/cpu-1-baseline.json` with the medians of a score file (4 significant digits)
//! and `provisional: false`. The run must be valid, without flags and of the current score
//! version; check the printed device and medians before committing the result (DB1: the
//! author's Ryzen 7 7800X3D at factory settings).

use oma_core::scores::{calibration_from, parse_score};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: calibrate_cpu <score.json>")?;
    let score = parse_score(&std::fs::read(path)?)?;
    let baseline = calibration_from(&score)?;
    let d = &score.device;
    println!(
        "device: {} ({} cores, {} logical), isa {:?}",
        d.model, d.cores, d.logical, score.isa
    );
    for (mode, table) in [("single", &baseline.single), ("multi", &baseline.multi)] {
        for (kernel, v) in table {
            println!("{mode:>6} {kernel:?}: {v}");
        }
    }
    let out = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/scores/cpu-1-baseline.json"
    );
    std::fs::write(out, serde_json::to_string_pretty(&baseline)? + "\n")?;
    println!("wrote {out}");
    Ok(())
}
