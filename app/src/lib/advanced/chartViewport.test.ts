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
