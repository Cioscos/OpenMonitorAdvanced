// Pure geometry and maths of the CPU (M8a2) and GPU (M8b2) benchmark gauges. The dial sweeps 270 degrees
// clockwise from 135 (bottom left) to 405 (bottom right), in SVG angles (0 = +x, y down).

export const START_ANGLE = 135;
export const SWEEP = 270;
/** The score of the calibration CPU; the full scale never drops below 1.1 times this. */
export const SCALE_POINTS = 1500;
/** Time constant of the needle's exponential smoothing, in milliseconds. */
export const SMOOTH_TAU_MS = 150;

export type Tick = { angle: number; kind: 'major' | 'mid' | 'minor'; label?: string };

/** Needle angle for `value` on a dial with full scale `max`, clamped to [0, max]. */
export function angleFor(value: number, max: number): number {
  if (!(max > 0) || !Number.isFinite(value)) return START_ANGLE;
  return START_ANGLE + SWEEP * Math.min(1, Math.max(0, value / max));
}

function tickLabel(value: number): string {
  return value >= 10_000 ? `${Math.round(value / 100) / 10}k` : String(Math.round(value));
}

/** 51 ticks (0 to 50): major every 10 with a label, mid every 5, minor otherwise. */
export function ticks(max: number): Tick[] {
  return Array.from({ length: 51 }, (_, i) => {
    const angle = START_ANGLE + (SWEEP * i) / 50;
    if (i % 10 === 0) return { angle, kind: 'major', label: tickLabel((max * i) / 50) };
    return { angle, kind: i % 5 === 0 ? 'mid' : 'minor' };
  });
}

/**
 * Full scale of the dial: the first number of the 1-2-2.5-5 x 10^n series at or above
 * 1.1 x max(values, 1500). Mirrors `oma-core::scores::gauge::full_scale` (DB6); non-finite
 * values are ignored, as `f64::max` ignores NaN there.
 */
export function fullScale(values: number[]): number {
  return series(1.1 * Math.max(SCALE_POINTS, ...values.filter(Number.isFinite)));
}

/** The GPU score estimate without a record or a reference (M8b2 DH7). */
const GPU_ESTIMATE = { dedicated: 1500, integrated: 20 };

/**
 * Full scale of a GPU dial (DH7): the series at or above 1.1 x max(values); without a finite
 * positive value (no record, no reference, no needle) the base is the estimate, so a 5-point
 * integrated GPU does not sit at the bottom of a 2000-point dial.
 */
export function gpuFullScale(values: number[], integrated: boolean): number {
  const present = values.filter((v) => Number.isFinite(v) && v > 0);
  const base = present.length ? Math.max(...present) : integrated ? GPU_ESTIMATE.integrated : GPU_ESTIMATE.dedicated;
  return series(1.1 * base);
}

/** The first number of the 1-2-2.5-5 x 10^n series at or above `target` (from 1). */
export function series(target: number): number {
  for (let decade = 1; ; decade *= 10) {
    for (const step of [1, 2, 2.5, 5]) {
      if (step * decade >= target) return step * decade;
    }
  }
}

/** One step of an exponential moving average towards `target` over `dtMs`; never overshoots. */
export function smooth(current: number, target: number, dtMs: number): number {
  if (!(dtMs > 0)) return current;
  return target + (current - target) * Math.exp(-dtMs / SMOOTH_TAU_MS);
}
