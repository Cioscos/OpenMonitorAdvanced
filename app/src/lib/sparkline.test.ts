import { sparklineGeometry, sparklinePath } from './sparkline';
import { sumSeries } from './select';

test('draws a line scaled to min/max', () => {
  expect(sparklinePath([0, 50, 100], 100, 10, 0, 100)).toBe('M0 10L50 5L100 0');
});

test('gaps split the line', () => {
  expect(sparklinePath([0, NaN, 100], 100, 10, 0, 100)).toBe('M0 10M100 0');
});

test('short series are right-aligned to the capacity', () => {
  expect(sparklinePath([100], 100, 10, 0, 100, 3)).toBe('M100 0');
});

test('empty or all-missing series draw nothing', () => {
  expect(sparklinePath([], 100, 10)).toBe('');
  expect(sparklinePath([NaN, NaN], 100, 10)).toBe('');
});

test('flat series sit on the baseline', () => {
  expect(sparklinePath([0, 0], 100, 10)).toBe('M0 10L100 10');
});

test('the same samples move left as the right edge advances', () => {
  const before = sparklineGeometry([0, 100], [0, 1_000], 1_000, 1_000, 100, 20, 0, 100);
  const after = sparklineGeometry([0, 100], [0, 1_000], 1_100, 1_000, 100, 20, 0, 100);
  expect(before.endpoint).toEqual({ x: 100, y: 0 });
  expect(after.endpoint).toEqual({ x: 90, y: 0 });
  expect(after.path).not.toContain('NaN');
});

test('irregular timestamps determine x for each sample', () => {
  const geometry = sparklineGeometry([0, 50, 100], [0, 250, 1_000], 1_000, 1_000, 100, 20, 0, 100);
  expect(geometry.path).toContain('C');
  expect(geometry.path).toContain('25 10');
  expect(geometry.endpoint).toEqual({ x: 100, y: 0 });
});

test('missing values split the path and hide a missing final endpoint', () => {
  const middle = sparklineGeometry([10, NaN, 20], [0, 500, 1_000], 1_000, 1_000, 100, 20, 0, 100);
  expect(middle.path.match(/M/g)).toHaveLength(2);
  expect(middle.endpoint).toEqual({ x: 100, y: 16 });
  const tail = sparklineGeometry([10, 20, NaN], [0, 500, 1_000], 1_000, 1_000, 100, 20, 0, 100);
  expect(tail.endpoint).toBeNull();
});

test('flat, single, and coincident samples stay finite', () => {
  for (const [values, times] of [
    [[5, 5, 5], [0, 500, 1_000]],
    [[5], [1_000]],
    [[5, 10, 20], [0, 0, 1_000]],
  ]) {
    const geometry = sparklineGeometry(values, times, 1_000, 1_000, 100, 20, 0);
    expect(geometry.path).not.toMatch(/NaN|Infinity/);
    expect(geometry.endpoint).not.toBeNull();
  }
});

test('cubic control y coordinates stay within each pair of sample endpoints', () => {
  const geometry = sparklineGeometry([0, 100, 10], [0, 500, 1_000], 1_000, 1_000, 100, 20, 0, 100);
  const cubics = [...geometry.path.matchAll(/C([\d.-]+) ([\d.-]+) ([\d.-]+) ([\d.-]+) ([\d.-]+) ([\d.-]+)/g)];
  expect(cubics).toHaveLength(2);
  for (const [index, match] of cubics.entries()) {
    const [low, high] = index === 0 ? [0, 20] : [0, 18];
    expect(Number(match[2])).toBeGreaterThanOrEqual(low);
    expect(Number(match[2])).toBeLessThanOrEqual(high);
    expect(Number(match[4])).toBeGreaterThanOrEqual(low);
    expect(Number(match[4])).toBeLessThanOrEqual(high);
  }
});

test('aggregated network and disk values retain the shared time axis', () => {
  const times = [0, 200, 1_000];
  for (const values of [sumSeries([[1, 2, 3], [4, 5, 6]]), sumSeries([[1, NaN, 3], [4, NaN, 6]])]) {
    const geometry = sparklineGeometry(values, times, 1_000, 1_000, 100, 20, 0, 10);
    expect(geometry.path).toContain('M0 ');
    expect(geometry.endpoint?.x).toBe(100);
    if (Number.isNaN(values[1])) expect(geometry.path.match(/M/g)).toHaveLength(2);
    else expect(geometry.path).toContain('20 ');
  }
});

test('segments crossing the five minute boundary are clipped to the viewport', () => {
  const geometry = sparklineGeometry([0, 100], [0, 200], 150, 100, 100, 20, 0, 100);
  expect(geometry.path).toMatch(/^M0 15C/);
  expect(geometry.path).toContain('100 5');
  expect(geometry.path).not.toContain('-50');
  expect(geometry.endpoint).toBeNull();
});
