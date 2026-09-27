import { heldLengthPx, scrollOffsetPx, timeTicks } from './chartCompositor';

const base = 10_000;

test('scrolls a cached plot by the elapsed viewport time', () => {
  expect(scrollOffsetPx(base, base + 250, 60_000, 600)).toBe(2.5);
});

test('does not scroll backward and ignores invalid plot dimensions', () => {
  expect(scrollOffsetPx(base, base - 250, 60_000, 600)).toBe(0);
  expect(scrollOffsetPx(base, base + 250, 0, 600)).toBe(0);
  expect(scrollOffsetPx(base, base + 250, 60_000, Infinity)).toBe(0);
  expect(scrollOffsetPx(base, Infinity, 60_000, 600)).toBe(0);
});

test('extends a valid last sample visually to the viewport edge', () => {
  expect(heldLengthPx(base, base + 250, 60_000, 600)).toBe(2.5);
});

test('omits a held segment when its endpoint or geometry is invalid', () => {
  expect(heldLengthPx(null, base + 250, 60_000, 600)).toBeNull();
  expect(heldLengthPx(base - 60_001, base, 60_000, 600)).toBeNull();
  expect(heldLengthPx(base + 1, base, 60_000, 600)).toBeNull();
  expect(heldLengthPx(Infinity, base + 250, 60_000, 600)).toBeNull();
  expect(heldLengthPx(base, Infinity, 60_000, 600)).toBeNull();
  expect(heldLengthPx(base, base + 250, NaN, 600)).toBeNull();
  expect(heldLengthPx(base, base + 250, 60_000, 0)).toBeNull();
});

test('keeps a held segment finite when valid dimensions are very large', () => {
  expect(heldLengthPx(0, 1e308, 1e308, 1e308)).toBe(1e308);
});

test('includes one time tick on either side of the visible range', () => {
  expect(timeTicks(60, 120, 30)).toEqual([30, 60, 90, 120, 150]);
});

test('aligns overscan ticks to increments when bounds fall between ticks', () => {
  expect(timeTicks(65, 115, 30)).toEqual([60, 90, 120]);
});

test('does not generate ticks for invalid ranges or increments', () => {
  expect(timeTicks(120, 60, 30)).toEqual([]);
  expect(timeTicks(60, 120, 0)).toEqual([]);
  expect(timeTicks(60, 120, Infinity)).toEqual([]);
});

test('rejects a tick range too large to allocate', () => {
  expect(timeTicks(0, 100_000, 1)).toEqual([]);
});

test('rejects tick indices that cannot advance safely', () => {
  expect(timeTicks(1e16, 1e16, 1)).toEqual([]);
});
