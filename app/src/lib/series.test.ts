import { SeriesBuffer } from './series';

test('keeps the most recent values up to capacity', () => {
  const b = new SeriesBuffer(3);
  [1, 2, 3, 4].forEach((v) => b.push(v));
  expect(b.toArray()).toEqual([2, 3, 4]);
  expect(b.length).toBe(3);
});

test('null is stored as NaN', () => {
  const b = new SeriesBuffer(2);
  b.push(null);
  expect(Number.isNaN(b.toArray()[0])).toBe(true);
});

test('clear empties the buffer', () => {
  const b = new SeriesBuffer(2);
  b.push(1);
  b.clear();
  expect(b.toArray()).toEqual([]);
});
