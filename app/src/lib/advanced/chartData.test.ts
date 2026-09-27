import { readFileSync } from 'node:fs';
import { MOCK_SCHEMA } from '../backend/mock';
import {
  ChartBuffer,
  MAX_SERIES,
  PALETTE_TOKENS,
  WINDOWS,
  canAdd,
  fitSelection,
  formatTimeTick,
  initialSeries,
  maxPointsFor,
  scaleLayout,
  scaleOptions,
  seriesPalette,
  unitsOf,
} from './chartData';
import { STORED_WINDOWS } from './persist';

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

test('theme.css defines every palette token', () => {
  const theme = readFileSync('src/styles/theme.css', 'utf8'); // vitest runs from app/
  for (const token of PALETTE_TOKENS) expect(theme).toMatch(new RegExp(`${token}\\s*:`));
});
