//! Minimal safe wrapper over the Performance Data Helper (PDH) API.

use std::fmt;

use oma_core::provider::ProviderError;
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhGetRawCounterArrayW, PdhOpenQueryW, PDH_CSTATUS_NEW_DATA,
    PDH_CSTATUS_VALID_DATA, PDH_FMT, PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PDH_RAW_COUNTER_ITEM_W,
};

/// `PDH_FMT_DOUBLE | PDH_FMT_NOCAP100`: windows-rs 0.62 does not export
/// `PDH_FMT_NOCAP100` (0x8000).
const FMT_DOUBLE_NOCAP: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000);
/// Returned while a rate counter has fewer than two samples.
pub(crate) const PDH_INVALID_DATA: u32 = 0xC000_0BBA;
/// `ERROR_INVALID_DATA`: PDH reported more items than its buffer can hold.
const ERROR_INVALID_DATA: u32 = 13;
/// Returned when a wildcard counter currently has no instances.
const PDH_NO_DATA: u32 = 0x8000_07D5;
/// A rate counter's base went backwards between the two samples: a 32-bit
/// base that wrapped, or `_Total` raw data that is inconsistent for one
/// interval. The next interval is computed from new samples.
pub(crate) const PDH_CALC_NEGATIVE_DENOMINATOR: u32 = 0x8000_07D6;
/// Instances can appear between the size query and the read; retry a few times.
const MAX_ARRAY_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdhError {
    pub call: &'static str,
    pub status: u32,
}

impl fmt::Display for PdhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} failed with PDH status {:#010x}",
            self.call, self.status
        )
    }
}

impl std::error::Error for PdhError {}

impl From<PdhError> for ProviderError {
    fn from(e: PdhError) -> Self {
        ProviderError::Failed(e.to_string())
    }
}

fn check(call: &'static str, status: u32) -> Result<(), PdhError> {
    if status == 0 {
        Ok(())
    } else {
        Err(PdhError { call, status })
    }
}

fn valid_status(status: u32) -> bool {
    matches!(status, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA)
}

/// A formatted value, or the PDH status that made it unavailable: the call's
/// own status when it failed, otherwise the value's `CStatus`.
fn formatted(status: u32, value_status: u32, value: f64) -> Result<f64, u32> {
    if status != 0 {
        Err(status)
    } else if valid_status(value_status) {
        Ok(value)
    } else {
        Err(value_status)
    }
}

/// True if `count` items of `item_size` bytes fit in `buffer_bytes`, without
/// overflowing the multiplication.
fn items_fit(count: u32, item_size: usize, buffer_bytes: usize) -> bool {
    (count as usize)
        .checked_mul(item_size)
        .is_some_and(|bytes| bytes <= buffer_bytes)
}

/// Name of a PDH item; empty if the pointer is null.
fn item_name(name: PWSTR) -> String {
    if name.is_null() {
        return String::new();
    }
    // SAFETY: non-null, and PDH points `szName` at a NUL-terminated string
    // inside the buffer it filled, which outlives this call.
    unsafe { name.to_string() }.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_new_and_unchanged_data_only() {
        assert!(valid_status(PDH_CSTATUS_VALID_DATA));
        assert!(valid_status(PDH_CSTATUS_NEW_DATA));
        assert!(!valid_status(PDH_INVALID_DATA));
    }

    #[test]
    fn formatted_value_keeps_the_status_that_made_it_unavailable() {
        assert_eq!(formatted(0, PDH_CSTATUS_VALID_DATA, 104.5), Ok(104.5));
        assert_eq!(formatted(0, PDH_CSTATUS_NEW_DATA, 3.0), Ok(3.0));
        // The call itself fails: its status wins over the (unset) value status.
        assert_eq!(
            formatted(
                PDH_CALC_NEGATIVE_DENOMINATOR,
                PDH_CALC_NEGATIVE_DENOMINATOR,
                0.0
            ),
            Err(PDH_CALC_NEGATIVE_DENOMINATOR)
        );
        assert_eq!(formatted(PDH_INVALID_DATA, 0, 0.0), Err(PDH_INVALID_DATA));
        // The call succeeds but PDH marks the value itself invalid.
        assert_eq!(formatted(0, PDH_INVALID_DATA, 0.0), Err(PDH_INVALID_DATA));
    }

    #[test]
    fn items_fit_rejects_counts_beyond_the_buffer() {
        assert!(items_fit(2, 24, 48));
        assert!(!items_fit(3, 24, 48));
        assert!(!items_fit(u32::MAX, usize::MAX / 2, 1024));
    }

    #[test]
    fn a_null_item_name_reads_as_empty() {
        assert_eq!(item_name(PWSTR::null()), "");
    }
}

pub struct Query {
    handle: PDH_HQUERY,
}

// SAFETY: a PDH query handle is not bound to the creating thread, and a Query
// is only used through `&mut`/`&` by the provider that owns it.
unsafe impl Send for Query {}

#[derive(Debug, Clone, Copy)]
pub struct Counter(PDH_HCOUNTER);

// SAFETY: counter handles are plain identifiers owned by their Query.
unsafe impl Send for Counter {}

impl Query {
    pub fn open() -> Result<Self, PdhError> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: valid out-pointer; a null data source means live data.
        check("PdhOpenQueryW", unsafe {
            PdhOpenQueryW(PCWSTR::null(), 0, &mut handle)
        })?;
        Ok(Self { handle })
    }

    /// Adds a counter by its English path, so it resolves on every Windows
    /// display language (localized names differ, e.g. on Italian Windows).
    pub fn add_english(&mut self, path: &str) -> Result<Counter, PdhError> {
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: the query handle is open and the out-pointer is valid.
        let status =
            unsafe { PdhAddEnglishCounterW(self.handle, &HSTRING::from(path), 0, &mut counter) };
        check("PdhAddEnglishCounterW", status)?;
        Ok(Counter(counter))
    }

    pub fn collect(&mut self) -> Result<(), PdhError> {
        // SAFETY: the query handle is open.
        check("PdhCollectQueryData", unsafe {
            PdhCollectQueryData(self.handle)
        })
    }

    /// Formatted value of a single-instance counter, or the PDH status that
    /// made it unavailable: until two samples exist, or when PDH cannot
    /// compute the rate of this interval.
    pub fn value(&self, counter: Counter) -> Result<f64, u32> {
        let mut value = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: the counter belongs to this query; the out-pointer is valid.
        let status =
            unsafe { PdhGetFormattedCounterValue(counter.0, FMT_DOUBLE_NOCAP, None, &mut value) };
        // SAFETY: PDH_FMT_DOUBLE fills the `doubleValue` union member, and any
        // bit pattern is a valid f64; `formatted` keeps it only when valid.
        formatted(status, value.CStatus, unsafe {
            value.Anonymous.doubleValue
        })
    }

    /// Formatted values of a wildcard counter as `(instance, value)`. Empty
    /// while no data is available yet; invalid instance values are NaN.
    pub fn array(&self, counter: Counter) -> Result<Vec<(String, f64)>, PdhError> {
        const CALL: &str = "PdhGetFormattedCounterArrayW";
        for _ in 0..MAX_ARRAY_ATTEMPTS {
            let (mut size, mut count) = (0u32, 0u32);
            // SAFETY: a size query with a null buffer.
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    counter.0,
                    FMT_DOUBLE_NOCAP,
                    &mut size,
                    &mut count,
                    None,
                )
            };
            match status {
                PDH_MORE_DATA => {}
                PDH_NO_DATA | PDH_INVALID_DATA => return Ok(Vec::new()),
                other => {
                    return Err(PdhError {
                        call: CALL,
                        status: other,
                    })
                }
            }
            // u64 storage keeps the items 8-byte aligned.
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            // SAFETY: `buffer` holds at least `size` bytes.
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    counter.0,
                    FMT_DOUBLE_NOCAP,
                    &mut size,
                    &mut count,
                    Some(items),
                )
            };
            match status {
                0 => {}
                PDH_MORE_DATA => continue,
                PDH_NO_DATA | PDH_INVALID_DATA => return Ok(Vec::new()),
                other => {
                    return Err(PdhError {
                        call: CALL,
                        status: other,
                    })
                }
            }
            if !items_fit(
                count,
                std::mem::size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                buffer.len() * 8,
            ) {
                return Err(PdhError {
                    call: CALL,
                    status: ERROR_INVALID_DATA,
                });
            }
            // SAFETY: `count` items were just checked to fit in `buffer`, which is
            // 8-byte aligned and outlives the slice; PDH wrote them.
            let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
            return Ok(items
                .iter()
                .map(|item| {
                    let name = item_name(item.szName);
                    let value = if valid_status(item.FmtValue.CStatus) {
                        // SAFETY: PDH_FMT_DOUBLE fills the `doubleValue` union member.
                        unsafe { item.FmtValue.Anonymous.doubleValue }
                    } else {
                        f64::NAN
                    };
                    (name, value)
                })
                .collect());
        }
        Err(PdhError {
            call: CALL,
            status: PDH_MORE_DATA,
        })
    }

    /// Instance names of a wildcard counter. Unlike `array`, this works right
    /// after the first `collect`, so discovery does not have to wait.
    pub fn instances(&self, counter: Counter) -> Result<Vec<String>, PdhError> {
        const CALL: &str = "PdhGetRawCounterArrayW";
        for _ in 0..MAX_ARRAY_ATTEMPTS {
            let (mut size, mut count) = (0u32, 0u32);
            // SAFETY: a size query with a null buffer.
            let status = unsafe { PdhGetRawCounterArrayW(counter.0, &mut size, &mut count, None) };
            match status {
                PDH_MORE_DATA => {}
                PDH_NO_DATA => return Ok(Vec::new()),
                other => {
                    return Err(PdhError {
                        call: CALL,
                        status: other,
                    })
                }
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let items = buffer.as_mut_ptr().cast::<PDH_RAW_COUNTER_ITEM_W>();
            // SAFETY: `buffer` holds at least `size` bytes.
            let status =
                unsafe { PdhGetRawCounterArrayW(counter.0, &mut size, &mut count, Some(items)) };
            match status {
                0 => {}
                PDH_MORE_DATA => continue,
                PDH_NO_DATA => return Ok(Vec::new()),
                other => {
                    return Err(PdhError {
                        call: CALL,
                        status: other,
                    })
                }
            }
            if !items_fit(
                count,
                std::mem::size_of::<PDH_RAW_COUNTER_ITEM_W>(),
                buffer.len() * 8,
            ) {
                return Err(PdhError {
                    call: CALL,
                    status: ERROR_INVALID_DATA,
                });
            }
            // SAFETY: `count` items were just checked to fit in `buffer`, which is
            // 8-byte aligned and outlives the slice; PDH wrote them.
            let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
            return Ok(items.iter().map(|item| item_name(item.szName)).collect());
        }
        Err(PdhError {
            call: CALL,
            status: PDH_MORE_DATA,
        })
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: the handle is open and not used after this point.
        unsafe {
            let _ = PdhCloseQuery(self.handle);
        }
    }
}
