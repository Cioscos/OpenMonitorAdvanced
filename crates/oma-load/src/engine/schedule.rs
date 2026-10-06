//! Which logical processors a phase runs on (DA4, DA7). `order` is the core order of
//! `oma_core::load::core_order`: the best cores first. Parked processors are placed like the
//! others: `Parked` is only their idle state, and the hard affinity wakes them.

use oma_ipc::load::{LogicalCpu, Topology};

/// The logical processors of `core`, in topology order.
fn logical_of(topology: &Topology, core: u32) -> impl Iterator<Item = &LogicalCpu> {
    topology.logical.iter().filter(move |l| l.core == core)
}

/// Up to `per_core` logical processors of every core of `order`: `usize::MAX` for
/// `all_logical`, 1 for `one_per_core`.
pub(crate) fn phase_cpus(topology: &Topology, order: &[u32], per_core: usize) -> Vec<LogicalCpu> {
    order
        .iter()
        .flat_map(|&c| logical_of(topology, c).take(per_core).cloned())
        .collect()
}

/// The first logical processor of `core`, and its sibling with `both_smt`.
pub(crate) fn core_cpus(topology: &Topology, core: u32, both_smt: bool) -> Vec<LogicalCpu> {
    logical_of(topology, core)
        .take(if both_smt { 2 } else { 1 })
        .cloned()
        .collect()
}

/// The first processor of each core among `cpus`.
pub(crate) fn one_per_core(cpus: &[LogicalCpu]) -> Vec<LogicalCpu> {
    let mut out: Vec<LogicalCpu> = Vec::new();
    for cpu in cpus {
        if !out.iter().any(|c| c.core == cpu.core) {
            out.push(cpu.clone());
        }
    }
    out
}

/// The reference processors (DA7): the first logical processor of the first, middle and
/// last core of `order`, fewer when there are fewer cores.
pub(crate) fn reference_cpus(topology: &Topology, order: &[u32]) -> Vec<LogicalCpu> {
    if order.is_empty() {
        return Vec::new();
    }
    let mut picks = vec![0, order.len() / 2, order.len() - 1];
    picks.dedup();
    picks
        .into_iter()
        .filter_map(|i| logical_of(topology, order[i]).next().cloned())
        .collect()
}

/// The index of the next core of `cores` from `from` (wrapping around) that has not
/// `failed`; `None` when every core failed.
pub(crate) fn next_core(cores: &[u32], from: usize, failed: impl Fn(u32) -> bool) -> Option<usize> {
    (0..cores.len())
        .map(|k| (from + k) % cores.len())
        .find(|&i| !failed(cores[i]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oma_ipc::load::CacheSizes;

    fn topology() -> Topology {
        // Cores 0..4 with SMT; core 3 is parked.
        let logical = (0..8)
            .map(|i| LogicalCpu {
                index: i,
                group: 0,
                number: i as u8,
                core: i / 2,
                core_index: i / 2,
                efficiency_class: 0,
                llc: 0,
                parked: i / 2 == 3,
                apic_id: None,
            })
            .collect();
        Topology {
            logical,
            caches: CacheSizes {
                l1d_bytes: 0,
                l2_bytes: 0,
                l2_shared_by: 2,
                l3_bytes: 0,
                l3_total_bytes: 0,
            },
            hypervisor: false,
            vendor: String::new(),
            brand: String::new(),
        }
    }

    fn indexes(cpus: &[LogicalCpu]) -> Vec<u32> {
        cpus.iter().map(|c| c.index).collect()
    }

    #[test]
    fn placements_pick_the_right_processors() {
        let t = topology();
        let order = oma_core::load::core_order(&t);
        assert_eq!(order, [0, 1, 2, 3], "the parked core is tested too");
        assert_eq!(
            indexes(&phase_cpus(&t, &order, usize::MAX)),
            [0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(indexes(&phase_cpus(&t, &order, 1)), [0, 2, 4, 6]);
        assert_eq!(indexes(&core_cpus(&t, 1, true)), [2, 3]);
        assert_eq!(indexes(&core_cpus(&t, 1, false)), [2]);
        assert_eq!(indexes(&core_cpus(&t, 3, true)), [6, 7]);
        assert_eq!(
            indexes(&one_per_core(&phase_cpus(&t, &order, 9))),
            [0, 2, 4, 6]
        );
        assert_eq!(indexes(&reference_cpus(&t, &order)), [0, 4, 6]);
        assert_eq!(indexes(&reference_cpus(&t, &order[..2])), [0, 2]);
        assert_eq!(indexes(&reference_cpus(&t, &order[..1])), [0]);
        assert!(reference_cpus(&t, &[]).is_empty());
    }

    #[test]
    fn a_parked_smt_sibling_is_still_placed() {
        let mut t = topology();
        t.logical[3].parked = true; // the second thread of core 1
        assert_eq!(indexes(&core_cpus(&t, 1, true)), [2, 3]);
        let order = oma_core::load::core_order(&t);
        assert_eq!(phase_cpus(&t, &order, 2).len(), 8);
    }

    #[test]
    fn next_core_wraps_and_skips_failed() {
        let cores = [0, 1, 2];
        assert_eq!(next_core(&cores, 0, |_| false), Some(0));
        assert_eq!(next_core(&cores, 3, |_| false), Some(0));
        assert_eq!(next_core(&cores, 2, |c| c == 2), Some(0));
        assert_eq!(next_core(&cores, 0, |_| true), None);
        assert_eq!(next_core(&[], 0, |_| false), None);
    }
}
