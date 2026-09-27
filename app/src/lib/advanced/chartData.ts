import type uPlot from 'uplot';
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
