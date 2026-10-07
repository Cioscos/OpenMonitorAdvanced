//! `cargo run -p oma-core --example calibrate_gpu -- <score.json>`: rewrites
//! `src/scores/gpu-1-baseline.json` with the medians of a GPU score file (4 significant
//! digits) and `provisional: false`. The run must be valid, without flags and of the
//! current GPU score version; check the printed device and medians before committing the
//! result (DH1: the author's RTX 4080 at factory settings).

use oma_core::scores::{gpu_calibration_from, parse_score};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: calibrate_gpu <score.json>")?;
    let score = parse_score(&std::fs::read(path)?)?;
    let baseline = gpu_calibration_from(&score)?;
    let d = &score.device;
    println!(
        "device: {} ({}), VRAM {} bytes",
        d.model,
        d.device_id.as_deref().unwrap_or("-"),
        d.dedicated_bytes.map_or("-".into(), |b| b.to_string())
    );
    for (group, table) in [
        ("compute", &baseline.compute),
        ("graphics", &baseline.graphics),
    ] {
        for (load, v) in table {
            println!("{group:>8} {load:?}: {v}");
        }
    }
    let out = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/scores/gpu-1-baseline.json"
    );
    std::fs::write(out, serde_json::to_string_pretty(&baseline)? + "\n")?;
    println!("wrote {out}");
    Ok(())
}
