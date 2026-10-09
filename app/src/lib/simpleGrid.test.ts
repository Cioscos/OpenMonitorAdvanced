import { simpleGrid } from './simpleGrid';

test('grid_for_1_to_7_tiles', () => {
  expect(simpleGrid(1)).toEqual({ columns: 1, spans: [1] });
  expect(simpleGrid(2)).toEqual({ columns: 2, spans: [1, 1] });
  expect(simpleGrid(3)).toEqual({ columns: 2, spans: [1, 1, 2] });
  expect(simpleGrid(4)).toEqual({ columns: 2, spans: [1, 1, 1, 1] });
  expect(simpleGrid(5)).toEqual({ columns: 6, spans: [2, 2, 2, 3, 3] });
  expect(simpleGrid(6)).toEqual({ columns: 3, spans: [1, 1, 1, 1, 1, 1] });
  expect(simpleGrid(7)).toEqual({ columns: 3, spans: Array(7).fill(1) });
  expect(simpleGrid(10).columns).toBe(3);
});
