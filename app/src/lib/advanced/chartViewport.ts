/** Monotonic clock anchored to the most recent sensor timestamp. uPlot x values are Unix seconds. */
export function createChartViewport(windowSeconds: number): {
  sample(timestampMs: number, receivedMonoMs: number): void;
  range(nowMonoMs: number): { min: number; max: number } | null;
  reset(): void;
} {
  let anchorUnixMs: number | null = null;
  let anchorMonoMs = 0;

  return {
    sample(timestampMs, receivedMonoMs) {
      // Delayed snapshots may lag behind the already displayed edge. Correct only the
      // time anchor; the plotted sensor sample keeps its original timestamp and value.
      const displayedMs = anchorUnixMs === null ? -Infinity : anchorUnixMs + Math.max(0, receivedMonoMs - anchorMonoMs);
      anchorUnixMs = Math.max(timestampMs, displayedMs);
      anchorMonoMs = receivedMonoMs;
    },
    range(nowMonoMs) {
      if (anchorUnixMs === null) return null;
      const max = (anchorUnixMs + Math.max(0, nowMonoMs - anchorMonoMs)) / 1000;
      return { min: max - windowSeconds, max };
    },
    reset() {
      anchorUnixMs = null;
      anchorMonoMs = 0;
    },
  };
}
