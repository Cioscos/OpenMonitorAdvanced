import type { HistoryWindow } from '../types';

/**
 * Min/max envelope, the same rule as `History::window_decimated` in oma-core:
 * with more samples than `maxPoints` (and `maxPoints >= 2`) the samples are split into
 * `floor(maxPoints / 2)` balanced buckets whose sizes differ by at most one;
 * each bucket emits (first timestamp, per-series min) then
 * (last timestamp, per-series max). These are envelope bounds, not extremum times.
 * Any missing or non-finite value makes that series emit null twice for the bucket.
 */
export function decimateWindow(window: HistoryWindow, maxPoints: number): HistoryWindow {
  const n = window.timestampsMs.length;
  if (n <= maxPoints || maxPoints < 2) return window;
  const buckets = Math.floor(maxPoints / 2);
  const timestampsMs: number[] = [];
  const series: (number | null)[][] = window.series.map(() => []);
  for (let b = 0; b < buckets; b++) {
    const start = Math.floor(b * n / buckets);
    const end = Math.floor((b + 1) * n / buckets);
    timestampsMs.push(window.timestampsMs[start], window.timestampsMs[end - 1]);
    window.series.forEach((values, k) => {
      if (values.slice(start, end).some((v) => v == null || !Number.isFinite(v))) {
        series[k].push(null, null);
        return;
      }
      let min = Infinity;
      let max = -Infinity;
      for (let i = start; i < end; i++) {
        const v = values[i];
        if (v === null || v === undefined || !Number.isFinite(v)) continue;
        if (v < min) min = v;
        if (v > max) max = v;
      }
      const found = min <= max;
      series[k].push(found ? min : null, found ? max : null);
    });
  }
  return { timestampsMs, series };
}
