import { sparklinePath } from './sparkline';

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
