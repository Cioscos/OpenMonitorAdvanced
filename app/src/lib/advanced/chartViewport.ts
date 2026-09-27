/** Monotonic clock anchored to the most recent sensor timestamp. uPlot x values are Unix seconds. */
export function createChartViewport(windowSeconds: number): {
  sample(timestampMs: number, receivedMonoMs: number): void;
  range(nowMonoMs: number): { min: number; max: number } | null;
  suspend(nowMonoMs: number): void;
  resume(nowMonoMs: number): void;
  reset(): void;
} {
  let anchorUnixMs: number | null = null;
  let anchorMonoMs = 0;
  let suspended = false;

  return {
    sample(timestampMs, receivedMonoMs) {
      // Delayed snapshots may lag behind the already displayed edge. Correct only the
      // time anchor; the plotted sensor sample keeps its original timestamp and value.
      const displayedMs = anchorUnixMs === null ? -Infinity : anchorUnixMs + (suspended ? 0 : Math.max(0, receivedMonoMs - anchorMonoMs));
      anchorUnixMs = Math.max(timestampMs, displayedMs);
      anchorMonoMs = receivedMonoMs;
    },
    range(nowMonoMs) {
      if (anchorUnixMs === null) return null;
      const max = (anchorUnixMs + (suspended ? 0 : Math.max(0, nowMonoMs - anchorMonoMs))) / 1000;
      return { min: max - windowSeconds, max };
    },
    suspend(nowMonoMs) {
      if (suspended) return;
      if (anchorUnixMs !== null) anchorUnixMs += Math.max(0, nowMonoMs - anchorMonoMs);
      anchorMonoMs = nowMonoMs;
      suspended = true;
    },
    resume(nowMonoMs) {
      if (!suspended) return;
      anchorMonoMs = nowMonoMs;
      suspended = false;
    },
    reset() {
      anchorUnixMs = null;
      anchorMonoMs = 0;
    },
  };
}
