/** Columns, and per-tile column spans of the Simple view grid for a given tile count. */
export function simpleGrid(count: number): { columns: number; spans: number[] } {
  const ones = (n: number) => Array<number>(n).fill(1);
  switch (count) {
    case 0:
    case 1:
      return { columns: 1, spans: ones(Math.max(count, 0)) };
    case 2:
      return { columns: 2, spans: [1, 1] };
    case 3:
      return { columns: 2, spans: [1, 1, 2] };
    case 4:
      return { columns: 2, spans: ones(4) };
    case 5:
      return { columns: 6, spans: [2, 2, 2, 3, 3] };
    case 6:
      return { columns: 3, spans: ones(6) };
    default:
      return { columns: 3, spans: ones(count) };
  }
}
