/** Pixel distance that cached chart geometry moves as the viewport advances. */
export function scrollOffsetPx(
  baseRightMs: number,
  visibleRightMs: number,
  windowMs: number,
  plotWidthPx: number,
): number {
  if (
    !Number.isFinite(baseRightMs) ||
    !Number.isFinite(visibleRightMs) ||
    !Number.isFinite(windowMs) ||
    !Number.isFinite(plotWidthPx) ||
    windowMs <= 0 ||
    plotWidthPx <= 0
  ) return 0;

  const offset = Math.max(0, visibleRightMs - baseRightMs) * plotWidthPx / windowMs;
  return Number.isFinite(offset) ? offset : 0;
}

/** Visual projection only; callers must not add the endpoint to sensor data. */
export function heldLengthPx(
  lastSampleMs: number | null,
  visibleRightMs: number,
  windowMs: number,
  plotWidthPx: number,
): number | null {
  if (
    lastSampleMs === null ||
    !Number.isFinite(lastSampleMs) ||
    !Number.isFinite(visibleRightMs) ||
    !Number.isFinite(windowMs) ||
    !Number.isFinite(plotWidthPx) ||
    windowMs <= 0 ||
    plotWidthPx <= 0
  ) return null;

  const elapsedMs = visibleRightMs - lastSampleMs;
  if (elapsedMs < 0 || elapsedMs > windowMs) return null;
  return elapsedMs / windowMs * plotWidthPx;
}

/** Tick positions in Unix seconds, including one aligned tick beyond each edge. */
export function timeTicks(minSeconds: number, maxSeconds: number, incrementSeconds: number): number[] {
  if (
    !Number.isFinite(minSeconds) ||
    !Number.isFinite(maxSeconds) ||
    !Number.isFinite(incrementSeconds) ||
    incrementSeconds <= 0 ||
    minSeconds > maxSeconds
  ) return [];

  const first = Math.ceil(minSeconds / incrementSeconds) - 1;
  const last = Math.floor(maxSeconds / incrementSeconds) + 1;
  if (
    !Number.isSafeInteger(first) ||
    !Number.isSafeInteger(last) ||
    last - first + 1 > 10_000
  ) return [];

  const ticks: number[] = [];
  for (let index = first; index <= last; index += 1) {
    ticks.push(index * incrementSeconds);
  }
  return ticks;
}
