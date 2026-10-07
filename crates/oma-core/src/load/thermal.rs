//! Thermal stop threshold and guard (plan DA5, spec §2.6).

/// `cpuStopC` if set, else Tjmax - 5, else 95.
pub fn cpu_stop_threshold(setting: Option<u32>, tjmax_c: Option<f64>) -> f64 {
    match (setting, tjmax_c) {
        (Some(s), _) => f64::from(s),
        (None, Some(t)) => t - 5.0,
        (None, None) => 95.0,
    }
}

/// `gpuStopC` (DG12): the setting itself, already within 60-110.
pub fn gpu_stop_threshold(setting: u32) -> f64 {
    f64::from(setting)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThermalEvent {
    None,
    /// Two consecutive samples above the threshold; carries the last one.
    Trip(f64),
    /// No value for more than 10 s; reported once until a value returns.
    Missing,
}

const MISSING_AFTER_MS: u64 = 10_000;

#[derive(Debug, Clone)]
pub struct ThermalGuard {
    threshold_c: f64,
    over: u32,
    /// Last time a value arrived, or the first `None` seen since then.
    since_ms: Option<u64>,
    warned: bool,
}

impl ThermalGuard {
    pub fn new(threshold_c: f64) -> Self {
        Self {
            threshold_c,
            over: 0,
            since_ms: None,
            warned: false,
        }
    }

    pub fn observe(&mut self, temp_c: Option<f64>, mono_ms: u64) -> ThermalEvent {
        match temp_c.filter(|t| t.is_finite()) {
            Some(t) => {
                self.since_ms = Some(mono_ms);
                self.warned = false;
                if t > self.threshold_c {
                    self.over += 1;
                    if self.over >= 2 {
                        return ThermalEvent::Trip(t);
                    }
                } else {
                    self.over = 0;
                }
                ThermalEvent::None
            }
            None => {
                self.over = 0;
                let since = *self.since_ms.get_or_insert(mono_ms);
                if !self.warned && mono_ms.saturating_sub(since) > MISSING_AFTER_MS {
                    self.warned = true;
                    return ThermalEvent::Missing;
                }
                ThermalEvent::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_uses_setting_then_tjmax_then_95() {
        assert_eq!(cpu_stop_threshold(None, Some(89.0)), 84.0);
        assert_eq!(cpu_stop_threshold(Some(70), Some(89.0)), 70.0);
        assert_eq!(cpu_stop_threshold(None, None), 95.0);
    }

    #[test]
    fn two_consecutive_samples_trip() {
        let mut g = ThermalGuard::new(85.0);
        let evs: Vec<_> = [90.0, 84.0, 90.0, 90.0]
            .iter()
            .enumerate()
            .map(|(i, t)| g.observe(Some(*t), i as u64 * 1000))
            .collect();
        assert_eq!(
            evs,
            [
                ThermalEvent::None,
                ThermalEvent::None,
                ThermalEvent::None,
                ThermalEvent::Trip(90.0)
            ]
        );
    }

    #[test]
    fn missing_for_ten_seconds_warns_once() {
        let mut g = ThermalGuard::new(85.0);
        assert_eq!(g.observe(Some(60.0), 0), ThermalEvent::None);
        assert_eq!(g.observe(None, 5_000), ThermalEvent::None);
        assert_eq!(g.observe(None, 10_000), ThermalEvent::None);
        assert_eq!(g.observe(None, 10_001), ThermalEvent::Missing);
        assert_eq!(g.observe(None, 20_000), ThermalEvent::None);
        // Re-arms once the value returns.
        assert_eq!(g.observe(Some(60.0), 21_000), ThermalEvent::None);
        assert_eq!(g.observe(None, 32_000), ThermalEvent::Missing);
    }
}
