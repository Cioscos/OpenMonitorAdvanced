// Adapted from OpenDCDiag (https://github.com/opendcdiag/opendcdiag),
// commit 9957c45b899e2ff7deb7bad94229246e2281d667: framework/sandstone.h and
// tests/examples/vector_add.c (golden value in init, recompute and compare
// in the loop, reproducible seed).
// Original work: Copyright 2022 Intel Corporation, licensed under the
// Apache License, Version 2.0 (see THIRD_PARTY_LICENSES.txt).
// Modified for OpenMonitor Advanced: rewritten in Rust, reference computed
// on three cores that must agree, 64-bit digests instead of memcmp.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.

//! Verification (§4.2, DA7): 64-bit digests of a kernel's output and the reference, the
//! digests every iteration must match bit for bit.

use oma_ipc::load::LogicalCpu;

/// At most this many processors compute the reference.
pub const REFERENCE_CPUS: usize = 3;

const DIGEST_START: u64 = 0xCBF2_9CE4_8422_2325;

fn mix(h: u64, w: u64) -> u64 {
    (h ^ w).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31)
}

/// Digest of 64-bit words.
pub fn digest_words(words: &[u64]) -> u64 {
    words.iter().fold(DIGEST_START, |h, &w| mix(h, w))
}

/// Digest of the bit patterns of `values`: no tolerance, `-0.0` differs from `0.0`.
pub fn digest_f64(values: &[f64]) -> u64 {
    values.iter().fold(DIGEST_START, |h, v| mix(h, v.to_bits()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefError<E> {
    /// The processors computed different vectors, in the order of the processors.
    Disagree(Vec<Vec<u64>>),
    /// The computation failed.
    Failed(E),
    /// The computation panicked.
    Panicked,
}

/// Runs `f` on each of the first [`REFERENCE_CPUS`] processors of `cpus`, one after the
/// other on a thread pinned to it, and returns the vector they agree on. With no processor
/// it runs once on the calling thread. The first failure ends it.
pub fn reference_on<E: Send>(
    cpus: &[LogicalCpu],
    f: &(dyn Fn() -> Result<Vec<u64>, E> + Sync),
) -> Result<Vec<u64>, RefError<E>> {
    let mut values = Vec::with_capacity(REFERENCE_CPUS);
    if cpus.is_empty() {
        values.push(f().map_err(RefError::Failed)?);
    }
    for cpu in cpus.iter().take(REFERENCE_CPUS) {
        let value = std::thread::scope(|s| {
            s.spawn(|| {
                if let Err(e) = crate::sys::pin(cpu) {
                    tracing::warn!(logical = cpu.index, error = %e, "reference not pinned");
                }
                f()
            })
            .join()
        })
        .map_err(|_| RefError::Panicked)?;
        values.push(value.map_err(RefError::Failed)?);
    }
    if values.iter().all(|v| *v == values[0]) {
        Ok(values.swap_remove(0))
    } else {
        Err(RefError::Disagree(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    type Digests = Result<Vec<u64>, String>;

    fn cpu(index: u32) -> LogicalCpu {
        LogicalCpu {
            index,
            group: 0,
            number: index as u8,
            core: index,
            core_index: index,
            efficiency_class: 0,
            llc: 0,
            parked: false,
            apic_id: None,
        }
    }

    #[test]
    fn digest_detects_a_single_bit() {
        let words: Vec<u64> = (0..64).map(|i| i * 0x0101_0101).collect();
        let d = digest_words(&words);
        assert_eq!(d, digest_words(&words));
        for i in [0, 31, 63] {
            for bit in [0, 17, 63] {
                let mut w = words.clone();
                w[i] ^= 1 << bit;
                assert_ne!(digest_words(&w), d, "word {i} bit {bit}");
            }
        }
        assert_ne!(digest_words(&[]), digest_words(&[0]));
        let f = [1.5f64, -2.25, 0.0];
        assert_eq!(
            digest_f64(&f),
            digest_words(&f.map(f64::to_bits)),
            "the f64 digest is over the bits"
        );
        assert_ne!(digest_f64(&[0.0]), digest_f64(&[-0.0]));
    }

    #[test]
    fn reference_agrees_or_reports_disagreement() {
        let cpus = [cpu(0), cpu(1), cpu(1), cpu(0)];
        let calls = AtomicU32::new(0);
        let same = || -> Digests {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(vec![9, 4])
        };
        assert_eq!(reference_on(&cpus, &same), Ok(vec![9, 4]));
        assert_eq!(calls.load(Ordering::Relaxed), 3, "at most three processors");

        // The whole vector must agree: only the second entry of the third differs.
        let n = AtomicU32::new(0);
        let third_differs = || -> Digests {
            let last = if n.fetch_add(1, Ordering::Relaxed) == 2 {
                8
            } else {
                9
            };
            Ok(vec![1, last])
        };
        assert_eq!(
            reference_on(&cpus, &third_differs),
            Err(RefError::Disagree(vec![vec![1, 9], vec![1, 9], vec![1, 8]]))
        );
        let invalid = || -> Digests { Err("round trip off by 1e-3".into()) };
        assert_eq!(
            reference_on(&cpus, &invalid),
            Err(RefError::Failed("round trip off by 1e-3".into()))
        );
        let panics = || -> Digests { panic!("kernel bug") };
        assert_eq!(reference_on(&cpus, &panics), Err(RefError::Panicked));
    }

    #[test]
    fn reference_with_one_core_uses_it_alone() {
        let calls = AtomicU32::new(0);
        let f = || -> Digests { Ok(vec![u64::from(calls.fetch_add(1, Ordering::Relaxed)) + 5]) };
        assert_eq!(reference_on(&[cpu(0)], &f), Ok(vec![5]));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            reference_on(&[], &f),
            Ok(vec![6]),
            "no processor: the calling thread"
        );
    }
}
