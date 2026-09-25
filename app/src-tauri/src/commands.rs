//! Tauri commands called by the UI (see app/src/lib/backend/tauri.ts).

use std::sync::PoisonError;

use oma_core::history::HistoryWindow;
use oma_core::model::Schema;
use oma_core::sampler::unix_ms;
use tauri::State;

use crate::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySeed {
    revision: u64,
    seq: u64,
    #[serde(flatten)]
    history: HistoryWindow,
}

/// Longest history the UI may request: the whole buffer (1 h).
const MAX_HISTORY_SECONDS: u64 = 3_600;

pub(crate) fn history_since(now_ms: u64, seconds: u64) -> u64 {
    now_ms.saturating_sub(seconds.min(MAX_HISTORY_SECONDS) * 1_000)
}

// Run off the main thread: the sampler holds the engine lock for up to
// ~200 ms per tick, and a sync command would block window/tray event handling
// for that long.
#[tauri::command(async)]
pub fn get_schema(state: State<'_, AppState>) -> Schema {
    state
        .engine
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .schema()
        .clone()
}

#[tauri::command(async)]
pub fn get_history(state: State<'_, AppState>, ids: Vec<String>, seconds: u64) -> HistorySeed {
    let since = history_since(unix_ms(), seconds);
    let engine = state.engine.lock().unwrap_or_else(PoisonError::into_inner);
    HistorySeed {
        revision: engine.schema().revision,
        seq: engine.sequence(),
        history: engine.history().window(&ids, since),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_window_is_capped_at_one_hour() {
        assert_eq!(history_since(10_000_000, 300), 10_000_000 - 300_000);
        assert_eq!(history_since(10_000_000, 999_999), 10_000_000 - 3_600_000);
    }

    #[test]
    fn history_window_never_underflows() {
        assert_eq!(history_since(1_000, 300), 0);
    }

    #[test]
    fn history_seed_serializes_with_the_ts_contract_keys() {
        let seed = HistorySeed {
            revision: 1,
            seq: 2,
            history: HistoryWindow {
                timestamps_ms: vec![1_000],
                series: vec![vec![Some(3.0)]],
            },
        };
        let value = serde_json::to_value(&seed).expect("serialize");
        let object = value.as_object().expect("object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["revision", "seq", "series", "timestampsMs"]);
    }
}
