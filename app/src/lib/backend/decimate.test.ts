import type { HistoryWindow } from '../types';
import { decimateWindow } from './decimate';

const window = (n: number, series: ((i: number) => number | null)[]): HistoryWindow => ({
  timestampsMs: Array.from({ length: n }, (_, i) => 1000 * (i + 1)),
  series: series.map((f) => Array.from({ length: n }, (_, i) => f(i))),
});

test('windows that already fit are returned unchanged', () => {
  const w = window(10, [(i) => i]);
  expect(decimateWindow(w, 10)).toBe(w);
  expect(decimateWindow(w, 900)).toBe(w);
});

test('max points below two disable decimation', () => {
  const w = window(10, [(i) => i]);
  expect(decimateWindow(w, 1)).toBe(w);
  expect(decimateWindow(w, 0)).toBe(w);
});

test('each bucket emits its first timestamp with the min and its last with the max', () => {
  // 10 samples, 4 points -> 2 buckets of 5.
  const w = window(10, [(i) => [5, 1, 9, 3, 4, 7, 2, 8, 6, 0][i]]);
  expect(decimateWindow(w, 4)).toEqual({
    timestampsMs: [1000, 5000, 6000, 10000],
    series: [[1, 9, 0, 8]],
  });
});

test('balanced buckets distribute the remainder and odd max points round down', () => {
  // 11 samples, 5 points -> 2 buckets: 0..4 and 5..10.
  const w = window(11, [(i) => i]);
  const d = decimateWindow(w, 5);
  expect(d.timestampsMs).toEqual([1000, 5000, 6000, 11000]);
  expect(d.series).toEqual([[0, 4, 5, 10]]);
});

test('missing or non-finite samples make their bucket a gap, other series keep theirs', () => {
  // 6 samples, 4 points -> 2 buckets of 3; NaN counts as missing.
  const w = window(6, [(i) => (i < 3 ? null : i), (i) => (i === 1 ? Number.NaN : 10 + i)]);
  expect(decimateWindow(w, 4)).toEqual({
    timestampsMs: [1000, 3000, 4000, 6000],
    series: [
      [null, null, 3, 5],
      [null, null, 13, 15],
    ],
  });
});

test('a single missing sample preserves the gap conservatively', () => {
  const w = window(6, [(i) => (i === 1 ? null : i + 1)]);
  expect(decimateWindow(w, 4).series).toEqual([[null, null, 4, 6]]);
});

test('near one hour has no oversized final bucket', () => {
  const d = decimateWindow(window(3599, [(i) => i]), 900);
  expect(d.timestampsMs).toHaveLength(900);
  expect(d.timestampsMs[0]).toBe(1000);
  expect(d.timestampsMs.at(-1)).toBe(3_599_000);
  for (let i = 0; i < d.timestampsMs.length; i += 2) {
    const span = d.timestampsMs[i + 1] - d.timestampsMs[i];
    expect(span).toBeGreaterThanOrEqual(6000);
    expect(span).toBeLessThanOrEqual(7000);
  }
});

test('one hour at 1 s becomes 900 rows', () => {
  const d = decimateWindow(window(3600, [(i) => Math.sin(i)]), 900);
  expect(d.timestampsMs).toHaveLength(900);
  expect(d.series[0]).toHaveLength(900);
  expect(d.timestampsMs[0]).toBe(1000);
  expect(d.timestampsMs.at(-1)).toBe(3_600_000);
});
