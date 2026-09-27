import { createChartViewport } from './chartViewport';

test('advances the Unix-second window by monotonic elapsed time', () => {
  const viewport = createChartViewport(60);
  viewport.sample(10_000, 1000);
  expect(viewport.range(1250)).toEqual({ min: -49.75, max: 10.25 });
});

test('a delayed sample never moves the viewport backward', () => {
  const viewport = createChartViewport(60);
  viewport.sample(10_000, 1000);
  viewport.sample(10_100, 1250);
  expect(viewport.range(1250)).toEqual({ min: -49.75, max: 10.25 });
});

test('reset allows a lower timestamp to start a new epoch', () => {
  const viewport = createChartViewport(60);
  viewport.sample(10_000, 1000);
  viewport.reset();
  viewport.sample(5_000, 1250);
  expect(viewport.range(1250)).toEqual({ min: -55, max: 5 });
});

test('has no range before its first sample', () => {
  expect(createChartViewport(60).range(1000)).toBeNull();
});

test('suspension freezes elapsed time while snapshots advance the visible edge', () => {
  const viewport = createChartViewport(60);
  viewport.sample(2000, 0);
  viewport.suspend(250);
  expect(viewport.range(600_000)?.max).toBe(2.25);
  viewport.sample(3000, 600_000);
  expect(viewport.range(1_200_000)?.max).toBe(3);
  viewport.resume(1_200_000);
  expect(viewport.range(1_200_000)?.max).toBe(3);
  expect(viewport.range(1_200_250)?.max).toBe(3.25);
});
