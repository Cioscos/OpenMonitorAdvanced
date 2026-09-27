import type uPlot from 'uplot';
import { formatValue } from '../format';
import type { Translate } from '../i18n/index.svelte';
import type { HistorySeed, Schema, Unit } from '../types';

/** Chart windows in seconds: 1m, 5m, 30m, 1h (spec §7.3). Same values as persist.ts `StoredWindow`. */
export const WINDOWS = [60, 300, 1800, 3600] as const;
export type WindowSeconds = (typeof WINDOWS)[number];
/** Window used until the user picks one. */
export const DEFAULT_WINDOW: WindowSeconds = 300;
/** At most 8 series and 2 units per chart (decision D4). */
export const MAX_SERIES = 8;
export const MAX_UNITS = 2;
/** Windows of at least this many seconds are requested decimated (decision D3). */
export const DECIMATE_FROM = 1800;
export const MAX_POINTS = 900;

/** `maxPoints` argument of `Backend.getHistory`: raw below 30 min, decimated from 30 min. */
export function maxPointsFor(windowSeconds: number): number | undefined {
  return windowSeconds >= DECIMATE_FROM ? MAX_POINTS : undefined;
}

const clean = (v: number | null | undefined): number | null =>
  v === null || v === undefined || !Number.isFinite(v) ? null : v;

/**
 * Time-aligned chart data for a fixed list of sensor ids: seeded from the core history,
 * extended by live snapshots, trimmed to the window. uPlot needs `null` for gaps, which a
 * Float64Array cannot hold, so the columns are plain arrays.
 */
export class ChartBuffer {
  readonly ids: string[];
  readonly windowSeconds: number;
  #timestampsMs: number[] = [];
  #series: (number | null)[][];

  constructor(ids: string[], windowSeconds: number) {
    this.ids = [...ids];
    this.windowSeconds = windowSeconds;
    this.#series = this.ids.map(() => []);
  }

  get length(): number {
    return this.#timestampsMs.length;
  }

  get lastTimestampMs(): number | null {
    return this.#timestampsMs.at(-1) ?? null;
  }

  /** Replaces the content with a history window whose series follow `ids` order. */
  seed(h: HistorySeed): void {
    this.#timestampsMs = [...h.timestampsMs];
    this.#series = this.ids.map((_, i) => {
      const column = h.series[i] ?? [];
      return this.#timestampsMs.map((_, k) => clean(column[k]));
    });
  }

  /** Adds one sample (values in `ids` order); a sample not newer than the last one is ignored. */
  append(timestampMs: number, values: (number | null)[]): void {
    const last = this.lastTimestampMs;
    if (last !== null && timestampMs <= last) return;
    this.#timestampsMs.push(timestampMs);
    this.#series.forEach((column, i) => column.push(clean(values[i])));
  }

  /** Keeps one sample before the window so lines cross the left clip edge continuously. */
  trim(nowMs: number): void {
    const since = nowMs - this.windowSeconds * 1000;
    let drop = 0;
    while (drop < this.#timestampsMs.length && this.#timestampsMs[drop] < since) drop++;
    if (drop > 0) drop--;
    if (drop === 0) return;
    this.#timestampsMs.splice(0, drop);
    for (const column of this.#series) column.splice(0, drop);
  }

  /** uPlot layout, copied: x in seconds, then one column per id. */
  data(): uPlot.AlignedData {
    return [this.#timestampsMs.map((ms) => ms / 1000), ...this.#series.map((column) => [...column])];
  }
}

/** Distinct units of `ids`, in order of first appearance; unknown ids are skipped. */
export function unitsOf(ids: string[], schema: Schema): Unit[] {
  const units: Unit[] = [];
  for (const id of ids) {
    const unit = schema.sensors.find((s) => s.id === id)?.unit;
    if (unit && !units.includes(unit)) units.push(unit);
  }
  return units;
}

/** True when `candidate` fits: fewer than 8 series and at most a second unit. */
export function canAdd(selected: string[], candidate: string, schema: Schema): boolean {
  if (selected.includes(candidate) || selected.length >= MAX_SERIES) return false;
  const sensor = schema.sensors.find((s) => s.id === candidate);
  if (!sensor) return false;
  const units = unitsOf(selected, schema);
  return units.includes(sensor.unit) || units.length < MAX_UNITS;
}

/** The ids present in `candidates`, in order, as long as they respect the series and unit limits. */
export function fitSelection(ids: string[], candidates: string[], schema: Schema): string[] {
  const out: string[] = [];
  for (const id of ids) {
    if (candidates.includes(id) && canAdd(out, id, schema)) out.push(id);
  }
  return out;
}

/**
 * Series to chart on mount. `saved` is persist.ts `loadSeries` (null = never saved): it is
 * cleaned against the current sensors, and the defaults apply when nothing of it survives.
 * An explicitly saved empty list stays empty.
 */
export function initialSeries(saved: string[] | null, candidates: string[], defaults: string[], schema: Schema): string[] {
  const fallback = fitSelection(defaults, candidates, schema);
  if (saved === null) return fallback;
  if (saved.length === 0) return [];
  const kept = fitSelection(saved, candidates, schema);
  return kept.length > 0 ? kept : fallback;
}

/** Series colours: accent tokens first, then the four chart-only tokens of theme.css. */
export const PALETTE_TOKENS = ['--accent', '--accent-2', '--ok', '--warn', '--series-5', '--series-6', '--series-7', '--series-8'] as const;

/** Resolves the palette through `read`, e.g. `getComputedStyle(root).getPropertyValue`. */
export function seriesPalette(read: (token: string) => string): string[] {
  return PALETTE_TOKENS.map((token) => read(token).trim());
}

/** Scale key per series (its unit): the first unit is drawn on the left axis, the second on the right. */
export function scaleLayout(ids: string[], schema: Schema): { scales: Unit[]; seriesScale: Unit[] } {
  const scales = unitsOf(ids, schema);
  const seriesScale = ids.map((id) => schema.sensors.find((s) => s.id === id)?.unit ?? scales[0]);
  return { scales, seriesScale };
}

/**
 * Scale options per unit: percent shows at least 0..100 and flags 0..1, but the range still
 * grows past the bound when the data does (the GPU power ratio can exceed 100 %).
 */
export function scaleOptions(unit: Unit): uPlot.Scale {
  const bounds = unit === 'percent' ? [0, 100] : unit === 'boolean' ? [0, 1] : null;
  if (!bounds) return {};
  return { range: { min: { soft: bounds[0], mode: 1, pad: 0 }, max: { soft: bounds[1], mode: 1, pad: 0 } } };
}

/**
 * Locale time-of-day for an x-axis split (uPlot gives seconds), 24-hour in `it`, matching the
 * legend. Splits closer than a minute show seconds, or every label of a minute would repeat.
 */
export function formatTimeTick(seconds: number, locale: string, incrementSeconds: number): string {
  const options: Intl.DateTimeFormatOptions = incrementSeconds < 60
    ? { hour: '2-digit', minute: '2-digit', second: '2-digit' }
    : { hour: '2-digit', minute: '2-digit' };
  return new Date(seconds * 1000).toLocaleTimeString(locale, options);
}

/**
 * uPlot's own default numeric split table (`numIncrs`, uPlot 1.6.32): 1, 2, 2.5, 5 times every
 * power of ten from 1e-9 to 1e20, ascending. Not exported by the package, so mirrored here to
 * filter it (see `labelSafeIncrs`).
 */
const NUMERIC_INCRS: number[] = (() => {
  const mults = [1, 2, 2.5, 5];
  const incrs: number[] = [];
  for (let exp = -9; exp < 21; exp++) {
    const mag = 10 ** exp;
    for (const mult of mults) incrs.push(mult * mag);
  }
  return incrs;
})();

/** Bounds a probe of split labels so a huge range paired with a tiny increment cannot hang. */
const MAX_PROBE_SPLITS = 200;

/**
 * True when every split uPlot would draw between `min` and `max` at this increment (uPlot's own
 * `numAxisSplits`: `incrRoundUp(min, incr)`, then `+incr` up to `max`) gets a distinct label.
 */
function hasDistinctLabels(min: number, max: number, incr: number, label: (v: number) => string): boolean {
  const seen = new Set<string>();
  let count = 0;
  for (let v = Math.ceil(min / incr) * incr; v <= max + incr * 1e-6; v += incr) {
    if (count++ >= MAX_PROBE_SPLITS) break;
    const text = label(v);
    if (seen.has(text)) return false;
    seen.add(text);
  }
  return true;
}

/**
 * uPlot `axis.incrs` for a Y axis: the default numeric table, filtered to increments no finer
 * than the precision `formatValue` actually displays for `unit`, so consecutive split labels
 * never repeat (e.g. a 42.6-44.3 °C range must not offer a 0.2 or 0.5 step, which whole-degree
 * rounding turns into duplicate labels). The step is derived by probing the formatter itself
 * over the visible range, not hardcoded per unit, so it also protects units with more decimals
 * (volt) or none (celsius, percent) alike.
 */
export function labelSafeIncrs(unit: Unit, locale: string, t: Translate): uPlot.Axis.Incrs {
  return (_u, _axisIdx, min, max) => {
    if (!Number.isFinite(min) || !Number.isFinite(max) || !(max > min)) return NUMERIC_INCRS;
    const label = (v: number) => formatValue(v, unit, locale, t);
    for (const incr of NUMERIC_INCRS) {
      if (hasDistinctLabels(min, max, incr, label)) return NUMERIC_INCRS.filter((candidate) => candidate >= incr);
    }
    return NUMERIC_INCRS;
  };
}

/** uPlot's default minimum distance between X splits, in CSS px. */
export const TIME_AXIS_MIN_SPACE = 50;
/** Clear space kept between two neighbouring X labels, in CSS px. */
export const TIME_LABEL_GAP_PX = 12;

/**
 * Minimum split spacing to hand uPlot's time axis (`axis.space`) so the increment it then
 * picks from `incrs` (ascending, uPlot's own table) leaves room for that increment's labels:
 * sub-minute splits carry seconds and are wider than HH:MM. uPlot still chooses the split;
 * this only raises the spacing it requires. `labelWidthPx` is the widest label for an
 * increment, in CSS px.
 */
export function timeAxisSpace(
  minSeconds: number,
  maxSeconds: number,
  plotWidthPx: number,
  incrs: readonly number[],
  labelWidthPx: (incrementSeconds: number) => number,
): number {
  const span = maxSeconds - minSeconds;
  if (!(span > 0) || !(plotWidthPx > 0) || !Number.isFinite(span) || !Number.isFinite(plotWidthPx)) {
    return TIME_AXIS_MIN_SPACE;
  }
  let previous = 0;
  for (const incr of incrs) {
    // The same spacing uPlot's findIncr compares with the minimum space.
    const spacing = plotWidthPx * incr / span;
    const required = Math.max(TIME_AXIS_MIN_SPACE, labelWidthPx(incr) + TIME_LABEL_GAP_PX);
    // Also exceed every smaller increment's spacing, so uPlot cannot stop at one whose
    // (wider) labels did not fit.
    if (spacing >= required) return Math.max(required, previous + 1e-6);
    previous = spacing;
  }
  return TIME_AXIS_MIN_SPACE;
}
