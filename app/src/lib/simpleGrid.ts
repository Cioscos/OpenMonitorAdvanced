/** Columns, per-tile column spans and rows of the Simple view grid for a given tile count. */
export function simpleGrid(count: number): { columns: number; spans: number[]; rows: number } {
  const ones = (n: number) => Array<number>(n).fill(1);
  switch (count) {
    case 0:
    case 1:
      return { columns: 1, spans: ones(Math.max(count, 0)), rows: 1 };
    case 2:
      return { columns: 2, spans: [1, 1], rows: 1 };
    case 3:
      return { columns: 2, spans: [1, 1, 2], rows: 2 };
    case 4:
      return { columns: 2, spans: ones(4), rows: 2 };
    case 5:
      return { columns: 6, spans: [2, 2, 2, 3, 3], rows: 2 };
    case 6:
      return { columns: 3, spans: ones(6), rows: 2 };
    default:
      return { columns: 3, spans: ones(count), rows: Math.ceil(count / 3) };
  }
}
