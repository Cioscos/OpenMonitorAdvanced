//! Byte-for-byte fixture parity tests against `protocol/fixtures/load/*.msgpack`.
//!
//! With `OMA_WRITE_FIXTURES=1` set, `load_fixtures_match_the_encoder_byte_for_byte`
//! (over)writes the fixture files from the encoder instead of comparing against them.
//! The files hold the MessagePack payload without the 4-byte length prefix.

use std::fs;
use std::path::PathBuf;

use oma_ipc::load::*;
use oma_ipc::{decode_payload_of, encode_frame_of};

const NAMES: &[&str] = &[
    "hello",
    "run",
    "run_gpu",
    "run_gpu_bench",
    "stop",
    "topology",
    "progress",
    "error",
    "notice",
    "phase_done",
    "finished",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fixtures/load")
}

fn reference(name: &str) -> LoadMessage {
    match name {
        "hello" => LoadMessage::Hello(LoadHello {
            protocol_version: LOAD_PROTOCOL_VERSION,
            version: "0.6.0".to_owned(),
            shader_digest: Some("0123456789abcdef".to_owned()),
            isa: vec![Isa::Avx512, Isa::Avx2, Isa::Sse2],
        }),
        "run" => LoadMessage::Run(RunRequest {
            plan: Plan {
                seed: 0x1234_5678_9ABC_DEF0,
                ram_bytes: 4 << 30,
                phases: vec![
                    Phase {
                        kernel: KernelId::K3,
                        alt_kernel: Some(KernelId::K4),
                        isa: Isa::Avx2,
                        size: DataSize::Auto,
                        mode: LoadMode::Variable,
                        placement: Placement::CoreCycle,
                        duration_s: 120,
                        per_core_s: Some(10),
                        both_smt: true,
                        cores: Some(vec![0, 2, 4]),
                        patterns: vec![],
                        stop_on_error: false,
                        iterations: None,
                        pause_before_ms: 0,
                        windows: None,
                    },
                    Phase {
                        kernel: KernelId::K10,
                        alt_kernel: None,
                        isa: Isa::Sse2,
                        size: DataSize::Ram,
                        mode: LoadMode::Steady,
                        placement: Placement::AllLogical,
                        duration_s: 300,
                        per_core_s: None,
                        both_smt: false,
                        cores: None,
                        patterns: vec![RamPattern::MovingInversions, RamPattern::CrcCopy],
                        stop_on_error: true,
                        iterations: None,
                        pause_before_ms: 0,
                        windows: None,
                    },
                ],
                gpu: None,
            },
        }),
        "run_gpu" => {
            let mut p = Phase {
                kernel: KernelId::S5,
                alt_kernel: Some(KernelId::S1),
                isa: Isa::Sse2,
                size: DataSize::Auto,
                mode: LoadMode::Steady,
                placement: Placement::AllLogical,
                duration_s: 600,
                per_core_s: None,
                both_smt: false,
                cores: None,
                patterns: vec![],
                stop_on_error: true,
                iterations: None,
                pause_before_ms: 0,
                windows: None,
            };
            let first = p.clone();
            p.kernel = KernelId::S1;
            p.alt_kernel = None;
            p.mode = LoadMode::Ramp;
            LoadMessage::Run(RunRequest {
                plan: Plan {
                    seed: 0x1234_5678_9ABC_DEF0,
                    ram_bytes: 0,
                    phases: vec![first, p],
                    gpu: Some(GpuTarget {
                        luid: 0x17e99,
                        integrated: false,
                    }),
                },
            })
        }
        "run_gpu_bench" => {
            let phase = |kernel| Phase {
                kernel,
                alt_kernel: None,
                isa: Isa::Sse2,
                size: DataSize::Auto,
                mode: LoadMode::Steady,
                placement: Placement::AllLogical,
                duration_s: 30,
                per_core_s: None,
                both_smt: false,
                cores: None,
                patterns: vec![],
                stop_on_error: true,
                iterations: None,
                pause_before_ms: 0,
                windows: Some(5),
            };
            LoadMessage::Run(RunRequest {
                plan: Plan {
                    seed: 0x1234_5678_9ABC_DEF0,
                    ram_bytes: 0,
                    phases: [
                        KernelId::S1,
                        KernelId::S2,
                        KernelId::S3,
                        KernelId::Fill,
                        KernelId::Texture,
                        KernelId::Overdraw,
                    ]
                    .into_iter()
                    .map(phase)
                    .collect(),
                    gpu: Some(GpuTarget {
                        luid: 0x17e99,
                        integrated: false,
                    }),
                },
            })
        }
        "stop" => LoadMessage::Stop(StopRequest {}),
        "topology" => LoadMessage::Topology(Topology {
            logical: vec![
                LogicalCpu {
                    index: 0,
                    group: 0,
                    number: 0,
                    core: 0,
                    core_index: 0,
                    efficiency_class: 1,
                    llc: 0,
                    parked: false,
                    apic_id: Some(0),
                },
                LogicalCpu {
                    index: 1,
                    group: 0,
                    number: 1,
                    core: 0,
                    core_index: 0,
                    efficiency_class: 1,
                    llc: 0,
                    parked: true,
                    apic_id: None,
                },
            ],
            caches: CacheSizes {
                l1d_bytes: 32 * 1024,
                l2_bytes: 1 << 20,
                l2_shared_by: 2,
                l3_bytes: 32 << 20,
                l3_total_bytes: 64 << 20,
            },
            hypervisor: false,
            vendor: "AuthenticAMD".to_owned(),
            brand: "AMD Ryzen 9 7950X3D 16-Core Processor".to_owned(),
        }),
        "progress" => LoadMessage::Progress(Progress {
            phase: 1,
            phase_elapsed_ms: 12_500,
            elapsed_ms: 132_500,
            checks: 4_000_000,
            errors: 0,
            current_core: Some(2),
            cores: vec![
                CoreProgress {
                    core: 0,
                    state: CoreState::Passed,
                },
                CoreProgress {
                    core: 1,
                    state: CoreState::Testing,
                },
                CoreProgress {
                    core: 2,
                    state: CoreState::Untested,
                },
                CoreProgress {
                    core: 3,
                    state: CoreState::Failed,
                },
            ],
            memory_bytes: 1 << 30,
            rate: Some(2.5e9),
            load_percent: Some(60),
        }),
        "error" => LoadMessage::Error(ComputeError {
            phase: 0,
            kernel: KernelId::K1,
            isa: Isa::Avx2,
            kind: ErrorKind::Mismatch,
            logical: Some(5),
            core: Some(2),
            iteration: 123_456_789,
            expected: 0xDEAD_BEEF,
            actual: 0xDEAD_BEEE,
            seed: 42,
            load_percent: None,
        }),
        "notice" => LoadMessage::Notice(Notice {
            phase: 0,
            code: "ram_reduced".to_owned(),
            value: None,
        }),
        "phase_done" => LoadMessage::PhaseDone(PhaseDone {
            phase: 2,
            checks: 777,
            errors: 1,
            duration_ms: 60_000,
            skipped: Some("isa_unavailable".to_owned()),
            work_ms: Some(59_000),
            workers: vec![WorkerDone {
                logical: 4,
                iterations: 50,
                work_ms: 58_500,
            }],
            rates: vec![1.5e12, 1.25e12],
        }),
        "finished" => LoadMessage::Finished(Finished {
            reason: FinishReason::Completed,
            checks: 9_000_000,
            errors: 0,
        }),
        other => panic!("unknown fixture name: {other}"),
    }
}

fn payload(msg: &LoadMessage) -> Vec<u8> {
    // The frame is a 4-byte little-endian length followed by the payload.
    encode_frame_of(msg).expect("encode reference message")[4..].to_vec()
}

#[test]
fn load_fixtures_match_the_encoder_byte_for_byte() {
    let write_mode = std::env::var("OMA_WRITE_FIXTURES").as_deref() == Ok("1");
    let dir = fixtures_dir();

    for name in NAMES {
        let encoded = payload(&reference(name));
        let path = dir.join(format!("{name}.msgpack"));

        if write_mode {
            fs::write(&path, &encoded).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
        } else {
            let on_disk = fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
            assert_eq!(
                encoded, on_disk,
                "{name}: encoder output does not match protocol/fixtures/load/{name}.msgpack"
            );
        }
    }
}

#[test]
fn load_fixtures_decode_to_the_reference_messages() {
    let dir = fixtures_dir();

    for name in NAMES {
        let path = dir.join(format!("{name}.msgpack"));
        let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let decoded: LoadMessage =
            decode_payload_of(&bytes).unwrap_or_else(|e| panic!("decode {name}: {e}"));
        assert_eq!(decoded, reference(name), "{name}: decoded message mismatch");
        decoded.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
