//! The local UTC offset for an instant, as the CSV log needs it.

use std::io;

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx};

/// Milliseconds between 1601-01-01 (the FILETIME epoch) and the Unix epoch.
const UNIX_EPOCH_AS_FILETIME_MS: u64 = 11_644_473_600_000;

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since the Unix epoch of a `SYSTEMTIME` read as a plain calendar time.
fn seconds_of(t: &SYSTEMTIME) -> i64 {
    days_from_civil(t.wYear.into(), t.wMonth.into(), t.wDay.into()) * 86_400
        + i64::from(t.wHour) * 3600
        + i64::from(t.wMinute) * 60
        + i64::from(t.wSecond)
}

/// `local - utc` in whole minutes, across day, month and year boundaries.
fn offset_between(utc: &SYSTEMTIME, local: &SYSTEMTIME) -> i32 {
    let secs = seconds_of(local) - seconds_of(utc);
    (secs as f64 / 60.0).round() as i32
}

/// The local time zone's offset from UTC, in minutes, at `unix_ms`
/// (daylight saving included).
pub fn utc_offset_minutes(unix_ms: u64) -> io::Result<i32> {
    let ticks = unix_ms
        .checked_add(UNIX_EPOCH_AS_FILETIME_MS)
        .and_then(|ms| ms.checked_mul(10_000))
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: `ft` and `utc` are valid for the call.
    unsafe { FileTimeToSystemTime(&ft, &mut utc) }
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    // SAFETY: a null time zone selects the current one; `utc` and `local`
    // are valid for the call.
    unsafe { SystemTimeToTzSpecificLocalTimeEx(None, &utc, &mut local) }
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    Ok(offset_between(&utc, &local))
}

type Source = Box<dyn FnMut(u64) -> io::Result<i32> + Send>;

/// Caches the offset for the minute it was computed in.
pub struct OffsetCache {
    minute: Option<u64>,
    offset: i32,
    warned: bool,
    source: Source,
}

impl Default for OffsetCache {
    fn default() -> Self {
        Self::new()
    }
}

impl OffsetCache {
    pub fn new() -> Self {
        Self::with_source(utc_offset_minutes)
    }

    /// A cache fed by `source` instead of the system time zone (tests).
    pub fn with_source(source: impl FnMut(u64) -> io::Result<i32> + Send + 'static) -> Self {
        Self {
            minute: None,
            offset: 0,
            warned: false,
            source: Box::new(source),
        }
    }

    /// Recomputes when `unix_ms / 60 000` changes; on error keeps the last
    /// offset (0 at first) and logs once.
    pub fn offset(&mut self, unix_ms: u64) -> i32 {
        let minute = unix_ms / 60_000;
        if self.minute != Some(minute) {
            self.minute = Some(minute);
            match (self.source)(unix_ms) {
                Ok(offset) => self.offset = offset,
                Err(err) => {
                    if !self.warned {
                        self.warned = true;
                        tracing::warn!(%err, "cannot read the local UTC offset; keeping the last one");
                    }
                }
            }
        }
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    fn st(year: u16, month: u16, day: u16, hour: u16, minute: u16) -> SYSTEMTIME {
        SYSTEMTIME {
            wYear: year,
            wMonth: month,
            wDay: day,
            wHour: hour,
            wMinute: minute,
            ..Default::default()
        }
    }

    #[test]
    fn offset_between_handles_day_and_month_rollover() {
        assert_eq!(
            offset_between(&st(2026, 12, 31, 23, 30), &st(2027, 1, 1, 1, 30)),
            120
        );
        assert_eq!(
            offset_between(&st(2026, 3, 1, 0, 15), &st(2026, 2, 28, 19, 15)),
            -300
        );
    }

    #[test]
    fn offset_cache_recomputes_each_minute() {
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();
        let mut cache = OffsetCache::with_source(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(60)
        });
        assert_eq!(cache.offset(120_000), 60);
        assert_eq!(cache.offset(179_999), 60);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(cache.offset(180_000), 60);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn offset_cache_keeps_the_last_value_on_error() {
        let mut n = 0;
        let mut cache = OffsetCache::with_source(move |_| {
            n += 1;
            if n == 1 {
                Ok(-300)
            } else {
                Err(io::Error::other("boom"))
            }
        });
        assert_eq!(cache.offset(0), -300);
        assert_eq!(cache.offset(60_000), -300);
        assert_eq!(cache.offset(120_000), -300);
        let mut failing = OffsetCache::with_source(|_| Err(io::Error::other("boom")));
        assert_eq!(failing.offset(0), 0);
    }

    #[test]
    fn utc_offset_is_plausible_here() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let offset = utc_offset_minutes(now).expect("offset");
        assert!((-840..=840).contains(&offset), "{offset}");
        assert_eq!(offset % 15, 0, "{offset}");
    }
}
