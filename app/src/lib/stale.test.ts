import { isStale, staleAfterMs } from './stale';

test('the threshold is five intervals, never under five seconds', () => {
  expect(staleAfterMs(500)).toBe(5000);
  expect(staleAfterMs(1000)).toBe(5000);
  expect(staleAfterMs(2000)).toBe(10_000);
  expect(staleAfterMs(5000)).toBe(25_000);
});

test('data turns stale only after the threshold has passed', () => {
  expect(isStale(10_000, 15_000, 1000)).toBe(false);
  expect(isStale(10_000, 15_001, 1000)).toBe(true);
  expect(isStale(10_000, 19_000, 2000)).toBe(false);
  expect(isStale(10_000, 20_001, 2000)).toBe(true);
});

test('an unknown last snapshot is not stale', () => {
  expect(isStale(null, 1e12, 1000)).toBe(false);
});
