/** M3 decision D12: no snapshot for max(5 s, 5 intervals) means the core stopped sampling. */
export function staleAfterMs(intervalMs: number): number {
  return Math.max(5000, 5 * intervalMs);
}

/** True when the last snapshot is older than the stale threshold; false when unknown (null). */
export function isStale(lastSnapshotAtMs: number | null, nowMs: number, intervalMs: number): boolean {
  return lastSnapshotAtMs !== null && nowMs - lastSnapshotAtMs > staleAfterMs(intervalMs);
}
