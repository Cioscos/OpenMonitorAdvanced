import { readFileSync } from 'node:fs';
import { MOCK_SCHEMA } from '../backend/mock';
import { formatValue } from '../format';
import { translate } from '../i18n/index.svelte';
import {
  ChartBuffer,
  MAX_SERIES,
  PALETTE_TOKENS,
  WINDOWS,
  canAdd,
  fitSelection,
  formatTimeTick,
  initialSeries,
  labelSafeIncrs,
  maxPointsFor,
  NUMERIC_INCRS,
  scaleLayout,
  scaleOptions,
  seriesPalette,
  timeAxisSpace,
  TIME_AXIS_MIN_SPACE,
  TIME_LABEL_GAP_PX,
  unitsOf,
} from './chartData';
import { STORED_WINDOWS } from './persist';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
/** Calls a `uPlot.Axis.Incrs` function form with the arguments the axis only needs for this table. */
const incrsFor = (unit: Parameters<typeof labelSafeIncrs>[0], min: number, max: number) => {
  const incrs = labelSafeIncrs(unit, 'en', tEn);
  if (typeof incrs !== 'function') throw new Error('expected a function');
  return incrs(undefined as never, 1, min, max, 200, 30);
};
/** uPlot's own split algorithm (numAxisSplits): incrRoundUp(min, incr), then +incr to max. */
const splitsFor = (min: number, max: number, incr: number): number[] => {
  const splits: number[] = [];
  for (let v = Math.ceil(min / incr) * incr; v <= max + 1e-9; v += incr) splits.push(v);
  return splits;
};

const GPU = 'gpu/pci-0000:01:00.0';
const LOAD = `${GPU}/load/core`;
const TEMP = `${GPU}/temperature/core`;
const HOTSPOT = `${GPU}/temperature/hotspot`;
const VRAM = `${GPU}/data/memory-dedicated-used`;
const CPU_THREADS = Array.from({ length: 8 }, (_, i) => `cpu/0/load/thread-0-${i}`);
const ALL = MOCK_SCHEMA.sensors.map((s) => s.id);

const seed = (timestampsMs: number[], series: (number | null)[][]) => ({ revision: 1, seq: 7, timestampsMs, series });

test('the windows are the persisted ones and decimation starts at 30 minutes', () => {
  expect([...WINDOWS]).toEqual([...STORED_WINDOWS]);
  expect(maxPointsFor(60)).toBeUndefined();
  expect(maxPointsFor(300)).toBeUndefined();
  expect(maxPointsFor(1800)).toBe(900);
  expect(maxPointsFor(3600)).toBe(900);
});

test('seed keeps the history in id order and converts to seconds for uPlot', () => {
  const buffer = new ChartBuffer(['a', 'b'], 60);
  buffer.seed(seed([1000, 2000], [[1, 2], [null, 4]]));
  expect(buffer.data()).toEqual([[1, 2], [1, 2], [null, 4]]);
  expect(buffer.lastTimestampMs).toBe(2000);
});

test('seed pads short or missing columns with gaps', () => {
  const buffer = new ChartBuffer(['a', 'b'], 60);
  buffer.seed(seed([1000, 2000], [[1]]));
  expect(buffer.data()).toEqual([[1, 2], [1, null], [null, null]]);
});

test('append ignores samples that are not newer than the last one', () => {
  const buffer = new ChartBuffer(['a'], 60);
  buffer.seed(seed([1000, 2000], [[1, 2]]));
  buffer.append(2000, [99]);
  buffer.append(1500, [98]);
  buffer.append(3000, [3]);
  expect(buffer.data()).toEqual([[1, 2, 3], [1, 2, 3]]);
});

test('append turns missing and non-finite values into gaps', () => {
  const buffer = new ChartBuffer(['a', 'b'], 60);
  buffer.append(1000, [Number.NaN, Number.POSITIVE_INFINITY]);
  buffer.append(2000, [5]);
  expect(buffer.data()).toEqual([[1, 2], [null, 5], [null, null]]);
});

test('trim retains one point before the viewport so the left edge does not jump on sample removal', () => {
  const buffer = new ChartBuffer(['a'], 60);
  for (let t = 0; t <= 120; t += 10) buffer.append(t * 1000, [t]);
  buffer.trim(120_000);
  expect(buffer.data()[1]).toEqual([50, 60, 70, 80, 90, 100, 110, 120]);
  buffer.append(121_000, [121]);
  buffer.trim(121_000);
  expect(buffer.data()[1]).toEqual([60, 70, 80, 90, 100, 110, 120, 121]);
});

test('data returns copies, so uPlot never sees later appends', () => {
  const buffer = new ChartBuffer(['a'], 60);
  buffer.append(1000, [1]);
  const before = buffer.data();
  buffer.append(2000, [2]);
  expect(before).toEqual([[1], [1]]);
});

test('unitsOf lists distinct units in order and skips unknown ids', () => {
  expect(unitsOf([LOAD, TEMP, 'nope', HOTSPOT], MOCK_SCHEMA)).toEqual(['percent', 'celsius']);
  expect(unitsOf([], MOCK_SCHEMA)).toEqual([]);
});

test('canAdd allows a second unit but not a third', () => {
  expect(canAdd([LOAD], TEMP, MOCK_SCHEMA)).toBe(true);
  expect(canAdd([LOAD, TEMP], HOTSPOT, MOCK_SCHEMA)).toBe(true);
  expect(canAdd([LOAD, TEMP], VRAM, MOCK_SCHEMA)).toBe(false);
});

test('canAdd refuses the ninth series, duplicates and unknown ids', () => {
  const eight = ['cpu/0/load/total', ...CPU_THREADS.slice(0, 7)];
  expect(eight).toHaveLength(MAX_SERIES);
  expect(canAdd(eight.slice(0, 7), eight[7], MOCK_SCHEMA)).toBe(true);
  expect(canAdd(eight, CPU_THREADS[7], MOCK_SCHEMA)).toBe(false);
  expect(canAdd([LOAD], LOAD, MOCK_SCHEMA)).toBe(false);
  expect(canAdd([], 'nope', MOCK_SCHEMA)).toBe(false);
});

test('fitSelection drops unknown ids and whatever breaks the limits', () => {
  expect(fitSelection([LOAD, 'gone', TEMP, VRAM], ALL, MOCK_SCHEMA)).toEqual([LOAD, TEMP]);
  const ten = ['cpu/0/load/total', ...CPU_THREADS, 'cpu/0/clock/effective'];
  expect(fitSelection(ten, ALL, MOCK_SCHEMA)).toHaveLength(MAX_SERIES);
  expect(fitSelection([LOAD], ['other'], MOCK_SCHEMA)).toEqual([]);
});

test('initial series: saved ones cleaned, else the defaults; an empty choice stays empty', () => {
  const gpu = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU).map((s) => s.id);
  expect(initialSeries(null, gpu, [LOAD, TEMP], MOCK_SCHEMA)).toEqual([LOAD, TEMP]);
  expect(initialSeries([VRAM, 'gone'], gpu, [LOAD, TEMP], MOCK_SCHEMA)).toEqual([VRAM]);
  expect(initialSeries(['gone'], gpu, [LOAD, TEMP], MOCK_SCHEMA)).toEqual([LOAD, TEMP]);
  expect(initialSeries([], gpu, [LOAD, TEMP], MOCK_SCHEMA)).toEqual([]);
  expect(initialSeries(null, gpu, [LOAD, 'cpu/0/load/total'], MOCK_SCHEMA)).toEqual([LOAD]);
});

test('scale layout puts the first unit left and the second right', () => {
  expect(scaleLayout([TEMP, LOAD, HOTSPOT], MOCK_SCHEMA)).toEqual({
    scales: ['celsius', 'percent'],
    seriesScale: ['celsius', 'percent', 'celsius'],
  });
});

test('percent and flag scales include their natural bounds, others auto-range', () => {
  expect(scaleOptions('percent')).toEqual({ range: { min: { soft: 0, mode: 1, pad: 0 }, max: { soft: 100, mode: 1, pad: 0 } } });
  expect(scaleOptions('boolean')).toEqual({ range: { min: { soft: 0, mode: 1, pad: 0 }, max: { soft: 1, mode: 1, pad: 0 } } });
  expect(scaleOptions('celsius')).toEqual({});
});

test('palette reads the eight tokens in order', () => {
  expect(seriesPalette((token) => ` ${token}-value `)).toEqual(PALETTE_TOKENS.map((t) => `${t}-value`));
  expect(PALETTE_TOKENS).toHaveLength(MAX_SERIES);
});

test('formatTimeTick renders minute splits in the app locale', () => {
  const seconds = Date.UTC(2026, 0, 1, 15, 45) / 1000;
  const minutes = { hour: '2-digit', minute: '2-digit' } as const;
  expect(formatTimeTick(seconds, 'it', 60)).toBe(new Date(seconds * 1000).toLocaleTimeString('it', minutes));
  expect(formatTimeTick(seconds, 'it', 60)).toMatch(/^\d{2}:\d{2}$/); // 24-hour, e.g. "15:45" (UTC offset notwithstanding)
  expect(formatTimeTick(seconds, 'en', 60)).toBe(new Date(seconds * 1000).toLocaleTimeString('en', minutes));
  expect(formatTimeTick(seconds, 'it', 300)).toMatch(/^\d{2}:\d{2}$/);
});

test('formatTimeTick shows seconds for sub-minute splits in the app locale', () => {
  const seconds = Date.UTC(2026, 0, 1, 15, 45, 35) / 1000;
  const withSeconds = { hour: '2-digit', minute: '2-digit', second: '2-digit' } as const;
  expect(formatTimeTick(seconds, 'it', 5)).toBe(new Date(seconds * 1000).toLocaleTimeString('it', withSeconds));
  expect(formatTimeTick(seconds, 'it', 5)).toMatch(/^\d{2}:\d{2}:35$/);
  expect(formatTimeTick(seconds, 'en', 30)).toBe(new Date(seconds * 1000).toLocaleTimeString('en', withSeconds));
  expect(formatTimeTick(seconds, 'en', 30)).not.toBe(formatTimeTick(seconds, 'en', 60));
  // One minute of 5 s splits no longer repeats one HH:MM label twelve times.
  const start = Date.UTC(2026, 0, 1, 15, 45) / 1000;
  const labels = Array.from({ length: 12 }, (_, i) => formatTimeTick(start + i * 5, 'it', 5));
  expect(new Set(labels).size).toBe(12);
});

const TIME_INCRS = [1, 5, 10, 15, 30, 60, 300, 600, 900, 1800, 3600];
/** uPlot 1.6.32 findIncr: the smallest increment whose split spacing reaches the minimum space. */
const uplotPick = (min: number, max: number, dim: number, space: number) =>
  TIME_INCRS.find((incr) => dim * incr / (max - min) >= space);

test('timeAxisSpace keeps uPlot on an increment whose labels fit, at the width boundary', () => {
  // Seconds labels 68 px wide ("12:58:05 AM" in Arial 12px), minute labels 40 px.
  const width = (incr: number) => (incr < 60 ? 68 : 40);
  const need = 68 + TIME_LABEL_GAP_PX;
  // 5 s splits exactly as wide as a label and its gap: uPlot may keep them.
  const exact = need * 60 / 5;
  expect(timeAxisSpace(0, 60, exact, TIME_INCRS, width)).toBe(need);
  expect(uplotPick(0, 60, exact, timeAxisSpace(0, 60, exact, TIME_INCRS, width))).toBe(5);
  // One pixel less and uPlot must move to 10 s splits.
  const space = timeAxisSpace(0, 60, exact - 1, TIME_INCRS, width);
  expect(uplotPick(0, 60, exact - 1, space)).toBe(10);
  expect((exact - 1) * 10 / 60).toBeGreaterThanOrEqual(need);
});

test('timeAxisSpace skips sub-minute splits that fit the default space but not their labels', () => {
  const width = (incr: number) => (incr < 60 ? 68 : 40);
  // 30 s splits are 60 px apart: enough for uPlot's 50 px default, too tight for 68 px labels.
  const dim = 120;
  const space = timeAxisSpace(0, 60, dim, TIME_INCRS, width);
  expect(space).toBeGreaterThan(dim * 30 / 60);
  expect(uplotPick(0, 60, dim, space)).toBe(60);
});

test('timeAxisSpace never goes below the uPlot default space and tolerates empty geometry', () => {
  const narrow = () => 10;
  expect(timeAxisSpace(0, 300, 651, TIME_INCRS, narrow)).toBe(TIME_AXIS_MIN_SPACE);
  expect(timeAxisSpace(0, 0, 651, TIME_INCRS, narrow)).toBe(TIME_AXIS_MIN_SPACE);
  expect(timeAxisSpace(0, 60, 0, TIME_INCRS, narrow)).toBe(TIME_AXIS_MIN_SPACE);
  expect(timeAxisSpace(0, 60, 651, [], narrow)).toBe(TIME_AXIS_MIN_SPACE);
});

test('theme.css defines every palette token', () => {
  const theme = readFileSync('src/styles/theme.css', 'utf8'); // vitest runs from app/
  for (const token of PALETTE_TOKENS) expect(theme).toMatch(new RegExp(`${token}\\s*:`));
});

describe('labelSafeIncrs', () => {
  test('drops celsius increments finer than a whole degree, for a narrow range', () => {
    const incrs = incrsFor('celsius', 42.6, 44.3);
    expect(Math.min(...incrs)).toBeGreaterThanOrEqual(1);
    // uPlot's default table offers 0.2 and 0.5 here; both round to duplicate whole degrees.
    expect(incrs).not.toContain(0.2);
    expect(incrs).not.toContain(0.5);
  });

  test('every celsius increment left in the table yields distinct split labels on that range', () => {
    const [min, max] = [42.6, 44.3];
    for (const incr of incrsFor('celsius', min, max)) {
      const labels = splitsFor(min, max, incr).map((v) => formatValue(v, 'celsius', 'en', tEn));
      expect(new Set(labels).size).toBe(labels.length);
    }
  });

  test('a wide percent range still offers the usual small increments (digits already coarse enough)', () => {
    const incrs = incrsFor('percent', 0, 100);
    expect(incrs).toContain(1);
    expect(incrs).toContain(2);
    expect(incrs).toContain(2.5);
    expect(incrs).toContain(5);
  });

  test('volt keeps its three decimals: a 0.001 step is safe, unlike a coarser unit', () => {
    const incrs = incrsFor('volt', 1.198, 1.212);
    expect(Math.min(...incrs)).toBeLessThanOrEqual(0.005);
  });

  test('NUMERIC_INCRS matches a real uPlot instance default numeric axis table bit-for-bit', async () => {
    const { default: RealUplot } = await vi.importActual<{ default: typeof import('uplot') }>('uplot');
    const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) }, {
      get: (target, key) => (key in target ? target[key as keyof typeof target] : () => {}),
    });
    const originalGetContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = (() => ctx) as unknown as typeof HTMLCanvasElement.prototype.getContext;
    const target = document.createElement('div');
    document.body.append(target);
    try {
      // A y axis bound to a scale with no data series: uPlot assigns its own default numeric
      // incrs table to it (no custom `incrs`), and nothing needs to draw a path (no Path2D).
      const actual = new RealUplot(
        { width: 400, height: 300, scales: { x: { time: false }, y: {} }, series: [{}], axes: [{}, { scale: 'y' }] },
        [[0, 1]],
        target,
      );
      const defaultIncrs = actual.axes[1].incrs as unknown as (...args: unknown[]) => number[];
      const real = defaultIncrs(actual, 1, 0, 100, 300, 30);
      expect(NUMERIC_INCRS).toEqual(real);
      actual.destroy();
    } finally {
      HTMLCanvasElement.prototype.getContext = originalGetContext;
      target.remove();
    }
  });

  test('rejects an increment outright once its splits would exceed the probe cap, on a wide bytes_per_second range', () => {
    const [min, max] = [900, 1_500_000];
    const incrs = incrsFor('bytes_per_second', min, max);
    const smallest = Math.min(...incrs);
    // Every remaining increment must fit within the probe cap over the whole range...
    const splitCount = splitsFor(min, max, smallest).length;
    expect(splitCount).toBeLessThanOrEqual(200);
    // ...and genuinely have distinct labels end-to-end, not merely for an early sample of them.
    const labels = splitsFor(min, max, smallest).map((v) => formatValue(v, 'bytes_per_second', 'en', tEn));
    expect(new Set(labels).size).toBe(labels.length);
  });
});
