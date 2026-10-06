// Adapted from OpenDCDiag (https://github.com/opendcdiag/opendcdiag),
// commit 9957c45b899e2ff7deb7bad94229246e2281d667: framework/sandstone.h and
// tests/examples/vector_add.c (golden value in init, recompute and compare
// in the loop, reproducible seed).
// Original work: Copyright 2022 Intel Corporation, licensed under the
// Apache License, Version 2.0 (see THIRD_PARTY_LICENSES.txt).
// Modified for OpenMonitor Advanced: rewritten in Rust, reference computed
// on three cores that must agree, 64-bit digests instead of memcmp.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.

//! The contract between the phase engine and the kernels (K1–K10, A9–A15).
//!
//! For each phase the engine asks the [`KernelFactory`] of the phase kernel for:
//! - the reference, [`KernelFactory::reference`], run on up to three processors of
//!   different cores that must agree (DA7); `None` for the kernels that check themselves
//!   (K9, K10);
//! - one [`Kernel`] per worker thread, [`KernelFactory::worker`], built on the worker's own
//!   pinned thread (so its memory is first touched there), before the reference.
//!
//! Every [`Kernel::iterate`] returns a [`Check`]: a digest the engine compares with the
//! reference, or the kernel's own verdict.
//!
//! **Memory (DA10).** `worker` sizes its data from `ctx.budget.ram_per_thread`. When the
//! allocation fails it returns [`KernelError::Memory`] with the size per thread to try next
//! (half, not below 256 MiB), or [`KernelError::Insufficient`] when it is already at the
//! floor. The engine then rebuilds every worker of the phase with the new size (and
//! `Notice { code: "ram_reduced" }`), so the workers and the reference always share one
//! `ctx`; for K3 and K4 it retries `Insufficient` with one thread per physical core, then
//! skips the phase with `Notice { code: "ram_insufficient" }`.

use std::any::Any;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};

use oma_ipc::load::{DataSize, Isa, KernelId, LogicalCpu, RamPattern, Topology};

/// The result of one iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// A digest of the output, compared with the reference digest.
    Digest(u64),
    /// A self-checking kernel found its output right.
    Ok,
    /// A self-checking kernel found a wrong value.
    Mismatch { expected: u64, actual: u64 },
}

/// One worker's instance of a kernel, with its data.
pub trait Kernel: Send {
    /// Runs one iteration, at most 2 s on a slow CPU, bumping `beat` (any change counts)
    /// at least every 250 ms of work.
    fn iterate(&mut self, beat: &AtomicU64) -> Check;
}

pub trait KernelFactory: Sync {
    /// The reference digest for `ctx`, computed with the same code as the workers; `None`
    /// for a self-checking kernel. `Err` is a defect of the implementation found by the
    /// reference's own checks (`reference_invalid`), never an error of a core.
    fn reference(&self, ctx: &WorkerCtx) -> Option<Result<u64, String>>;

    /// A kernel for worker `ctx.worker`; see the module documentation for the errors.
    fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelError {
    /// The requested instruction set (or the size) is not available here.
    Unsupported,
    /// The allocation failed: try again with this many bytes per thread.
    Memory(u64),
    /// Not enough memory even at the smallest size.
    Insufficient,
}

/// Data sizes per thread of a phase (DA9), in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ThreadBudget {
    /// L1 data cache of one core.
    pub l1d: u64,
    /// L2 divided by the phase threads sharing it.
    pub l2_thread: u64,
    /// L3 of the domain divided by the phase threads on it; 0 without an L3.
    pub l3_share: u64,
    /// The plan's RAM share divided by the phase threads.
    pub ram_per_thread: u64,
}

impl ThreadBudget {
    /// The budget of the phase threads running on `cpus`.
    pub fn for_workers(topology: &Topology, cpus: &[LogicalCpu], ram_bytes: u64) -> Self {
        // ponytail: the topology has one L2 instance; an L2 shared by more threads than a
        // core has (a cluster of E cores) counts as shared by that many cores.
        let per_core = most_per_key(topology.logical.iter().map(|l| l.core));
        let cores_per_l2 = (u64::from(topology.caches.l2_shared_by) / per_core).max(1);
        let on_core = most_per_key(cpus.iter().map(|l| l.core));
        let on_llc = most_per_key(cpus.iter().map(|l| l.llc));
        let c = &topology.caches;
        Self {
            l1d: c.l1d_bytes,
            l2_thread: c.l2_bytes / (on_core * cores_per_l2),
            l3_share: c.l3_bytes / on_llc,
            ram_per_thread: ram_bytes / (cpus.len().max(1) as u64),
        }
    }
}

/// How many times the most frequent key occurs; at least 1.
fn most_per_key(keys: impl Iterator<Item = u32>) -> u64 {
    let mut keys: Vec<u32> = keys.collect();
    keys.sort_unstable();
    keys.chunk_by(|a, b| a == b)
        .map(|c| c.len() as u64)
        .max()
        .unwrap_or(1)
}

/// State shared by the workers of one phase (one slice of a `core_cycle` phase).
#[derive(Default)]
pub struct PhaseShared {
    /// Raised when the workers must return: a kernel that waits for another worker (K9)
    /// checks it in its waits.
    pub quit: AtomicBool,
    /// Free slot for the kernel's own shared structures, set once by the first worker.
    pub slot: OnceLock<Box<dyn Any + Send + Sync>>,
}

/// What a kernel needs to build its data.
#[derive(Clone)]
pub struct WorkerCtx {
    pub isa: Isa,
    pub size: DataSize,
    pub budget: ThreadBudget,
    pub seed: u64,
    /// This worker, from 0; the reference runs as worker 0.
    pub worker: u32,
    pub workers: u32,
    pub patterns: Vec<RamPattern>,
    pub shared: Arc<PhaseShared>,
}

/// The factory of kernel `id`, or `None` while it does not exist yet.
pub fn factory(id: KernelId) -> Option<&'static dyn KernelFactory> {
    match id {
        // The kernels arrive with A9–A15.
        KernelId::K1
        | KernelId::K2
        | KernelId::K3
        | KernelId::K4
        | KernelId::K5
        | KernelId::K7
        | KernelId::K8
        | KernelId::K9
        | KernelId::K10 => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::load::CacheSizes;

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;

    fn cpu(index: u32, core: u32, llc: u32) -> LogicalCpu {
        LogicalCpu {
            index,
            group: 0,
            number: index as u8,
            core,
            core_index: core,
            efficiency_class: 0,
            llc,
            parked: false,
            apic_id: None,
        }
    }

    #[test]
    fn budget_matches_the_plan_table() {
        // Two cores with SMT on one L3; L2 1 MiB shared by the 2 threads of a core.
        let topology = Topology {
            logical: vec![cpu(0, 0, 0), cpu(1, 0, 0), cpu(2, 1, 0), cpu(3, 1, 0)],
            caches: CacheSizes {
                l1d_bytes: 32 * KIB,
                l2_bytes: MIB,
                l2_shared_by: 2,
                l3_bytes: 32 * MIB,
                l3_total_bytes: 32 * MIB,
            },
            hypervisor: false,
            vendor: String::new(),
            brand: String::new(),
        };
        let all = ThreadBudget::for_workers(&topology, &topology.logical, 4096 * MIB);
        assert_eq!(
            all,
            ThreadBudget {
                l1d: 32 * KIB,
                l2_thread: 512 * KIB,
                l3_share: 8 * MIB,
                ram_per_thread: 1024 * MIB,
            }
        );
        let one_per_core = [topology.logical[0].clone(), topology.logical[2].clone()];
        let b = ThreadBudget::for_workers(&topology, &one_per_core, 4096 * MIB);
        assert_eq!(
            (b.l2_thread, b.l3_share, b.ram_per_thread),
            (MIB, 16 * MIB, 2048 * MIB)
        );

        // An L2 shared by a cluster of four single-thread cores (E cores).
        let mut cluster = topology.clone();
        cluster.logical = (0..4).map(|i| cpu(i, i, 0)).collect();
        cluster.caches.l2_bytes = 2 * MIB;
        cluster.caches.l2_shared_by = 4;
        let b = ThreadBudget::for_workers(&cluster, &cluster.logical, 0);
        assert_eq!(b.l2_thread, 512 * KIB);

        // No L3, no workers: zeros, no panic.
        let mut no_l3 = topology;
        no_l3.caches.l3_bytes = 0;
        let b = ThreadBudget::for_workers(&no_l3, &[], 1);
        assert_eq!(b.l3_share, 0);
        assert_eq!(b.ram_per_thread, 1);
    }
}
