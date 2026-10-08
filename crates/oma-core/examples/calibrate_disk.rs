//! `cargo run -p oma-core --example calibrate_disk -- <score.json>`: rewrites
//! `src/scores/disk-1-baseline.json` with the best rates of a disk score file (4
//! significant digits) and `provisional: false`. The run must be a B1 one, valid, without
//! flags and of the current disk score version; check the printed device and rates before
//! committing the result.

use oma_core::scores::{disk_calibration_from, parse_score};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: calibrate_disk <score.json>")?;
    let score = parse_score(&std::fs::read(path)?)?;
    let baseline = disk_calibration_from(&score)?;
    let d = &score.device;
    println!(
        "device: {} ({}), {}",
        d.model,
        d.device_id.as_deref().unwrap_or("-"),
        d.kind.as_deref().unwrap_or("-")
    );
    for (direction, table) in [("read", &baseline.read), ("write", &baseline.write)] {
        for (test, v) in table {
            println!("{direction:>5} {test:?}: {v} MB/s");
        }
    }
    let out = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/scores/disk-1-baseline.json"
    );
    std::fs::write(out, serde_json::to_string_pretty(&baseline)? + "\n")?;
    println!("wrote {out}");
    Ok(())
}
