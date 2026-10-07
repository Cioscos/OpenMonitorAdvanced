import { angleFor, fullScale, gpuFullScale, smooth, ticks } from './gauge';

test('angle_bounds', () => {
  expect(angleFor(0, 2000)).toBe(135);
  expect(angleFor(2000, 2000)).toBe(405);
  expect(angleFor(9000, 2000)).toBe(405);
  expect(angleFor(-5, 2000)).toBe(135);
  expect(angleFor(1000, 2000)).toBe(270);
  expect(angleFor(10, 0)).toBe(135);
});

test('ticks_count_and_kinds', () => {
  const all = ticks(2000);
  expect(all).toHaveLength(51);
  const major = all.filter((t) => t.kind === 'major');
  expect(major).toHaveLength(6);
  expect(major.every((t) => t.label !== undefined)).toBe(true);
  expect(major.map((t) => t.label)).toEqual(['0', '400', '800', '1200', '1600', '2000']);
  expect(all.filter((t) => t.kind === 'mid')).toHaveLength(5);
  expect(all.filter((t) => t.label !== undefined)).toHaveLength(6);
  expect(all[0].angle).toBe(135);
  expect(all[50].angle).toBe(405);
  expect(ticks(100_000).at(-1)?.label).toBe('100k');
});

test('full_scale_matches_the_rust_series', () => {
  // The same cases as `full_scale_series` in crates/oma-core/src/scores/gauge.rs.
  expect(fullScale([1500])).toBe(2000);
  expect(fullScale([1900])).toBe(2500);
  expect(fullScale([4500])).toBe(5000);
  expect(fullScale([4600])).toBe(10_000);
  expect(fullScale([120])).toBe(2000);
  expect(fullScale([])).toBe(2000);
  expect(fullScale([50_000])).toBe(100_000);
  expect(fullScale([120, 1900, 3])).toBe(2500);
  expect(fullScale([Number.NaN, Number.POSITIVE_INFINITY])).toBe(2000);
});

test('smooth_converges_without_overshoot', () => {
  let v = 0;
  let previous = 0;
  for (let i = 0; i < 120; i++) {
    v = smooth(v, 1000, 16);
    expect(v).toBeGreaterThanOrEqual(previous);
    expect(v).toBeLessThanOrEqual(1000);
    previous = v;
  }
  expect(v).toBeGreaterThan(999);
  // One time constant covers 1 - 1/e of the gap.
  expect(smooth(0, 1000, 150)).toBeCloseTo(1000 * (1 - Math.exp(-1)), 6);
  expect(smooth(500, 0, 10_000)).toBeCloseTo(0, 6);
  expect(smooth(42, 1000, 0)).toBe(42);
});

test('gpu_full_scale_without_record_uses_the_estimate', () => {
  expect(gpuFullScale([], false)).toBe(2000);
  expect(gpuFullScale([], true)).toBe(25);
  expect(gpuFullScale([0, Number.NaN], true)).toBe(25);
});

test('gpu_full_scale_follows_the_record', () => {
  expect(gpuFullScale([150], false)).toBe(200);
  expect(gpuFullScale([5.2], true)).toBe(10);
  expect(gpuFullScale([1500], false)).toBe(2000);
});
