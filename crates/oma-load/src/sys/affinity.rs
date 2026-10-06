//! Thread placement and priority for the load workers (§2.1).

use std::io;

use oma_ipc::load::LogicalCpu;
use windows::Win32::System::SystemInformation::GROUP_AFFINITY;
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadGroupAffinity, SetThreadInformation, SetThreadPriority,
    ThreadPowerThrottling, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
    THREAD_PRIORITY_BELOW_NORMAL,
};

/// The affinity mask of `cpu` inside its processor group.
pub fn group_affinity(cpu: &LogicalCpu) -> GROUP_AFFINITY {
    GROUP_AFFINITY {
        Mask: 1usize << cpu.number,
        Group: cpu.group,
        Reserved: [0; 3],
    }
}

/// Pins the calling thread to `cpu`.
pub fn pin_current_thread(cpu: &LogicalCpu) -> io::Result<()> {
    let affinity = group_affinity(cpu);
    // SAFETY: the pseudo-handle of the current thread needs no close; `affinity` is a live
    // GROUP_AFFINITY for the call; the previous affinity is not requested.
    let ok = unsafe { SetThreadGroupAffinity(GetCurrentThread(), &affinity, None) };
    if ok.as_bool() {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Below-normal priority and EcoQoS off (execution speed throttling disabled), so the
/// workers never run on the efficiency cores by Windows' choice and never starve the
/// desktop.
pub fn prepare_worker_thread() -> io::Result<()> {
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: the pseudo-handle of the current thread needs no close.
    let thread = unsafe { GetCurrentThread() };
    // SAFETY: `thread` is the current thread.
    unsafe { SetThreadPriority(thread, THREAD_PRIORITY_BELOW_NORMAL) }
        .map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
    // SAFETY: `state` is a live THREAD_POWER_THROTTLING_STATE and the size passed is its size.
    unsafe {
        SetThreadInformation(
            thread,
            ThreadPowerThrottling,
            (&raw const state).cast(),
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .map_err(|e| io::Error::from_raw_os_error(e.code().0 & 0xFFFF))
}

/// Topology of this machine with the APIC id of every logical processor, each read on a
/// short-lived thread pinned to it, one after the other and without load (DA3).
pub fn full_topology() -> io::Result<oma_ipc::load::Topology> {
    let mut topology = oma_win::topology::read()?;
    for cpu in &mut topology.logical {
        let probe = cpu.clone();
        let apic = std::thread::spawn(move || {
            pin_current_thread(&probe).map(|()| super::cpuid::apic_id())
        })
        .join()
        .map_err(|_| io::Error::other("the APIC probe thread panicked"))?;
        // A processor that refuses the affinity keeps `None`: WHEA cannot be mapped to it.
        cpu.apic_id = match apic {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(logical = cpu.index, error = %e, "cannot read the APIC id");
                None
            }
        };
    }
    Ok(topology)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn group_affinity_sets_one_bit() {
        let a = group_affinity(&LogicalCpu {
            group: 1,
            number: 5,
            ..cpu(0)
        });
        assert_eq!((a.Group, a.Mask), (1, 1 << 5));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn pinning_reports_distinct_apic_ids() {
        let topology = oma_win::topology::read().unwrap();
        let usable: Vec<_> = topology
            .logical
            .iter()
            .filter(|l| !l.parked)
            .take(2)
            .collect();
        if usable.len() < 2 {
            eprintln!("skipped: fewer than two logical processors");
            return;
        }
        let ids: Vec<u32> = usable
            .into_iter()
            .map(|l| {
                let l = l.clone();
                std::thread::spawn(move || {
                    pin_current_thread(&l).unwrap();
                    super::super::cpuid::apic_id()
                })
                .join()
                .unwrap()
            })
            .collect();
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn worker_thread_can_be_prepared() {
        std::thread::spawn(|| prepare_worker_thread().unwrap())
            .join()
            .unwrap();
    }
}
