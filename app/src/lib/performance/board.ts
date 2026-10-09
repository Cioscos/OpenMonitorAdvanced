import type { Board, BoardRow, BoardTable, ScoreSummary } from '../types';

/** The leaderboard categories, in display order. */
export const BOARDS: Board[] = ['cpu-single', 'cpu-multi', 'gpu-compute', 'gpu-graphics', 'disk'];
/** Rows shown on the page: the table rows and the user's own together (spec 7.2: 8-12, 10 is the centre). */
export const NEIGHBOURS = 10;
/** Below this many table rows a percentile says nothing. */
export const PERCENTILE_MIN_ROWS = 10;

/** One of the user's own best scores, shown beside the table rows (never merged into the table). */
export interface OwnRow {
  model: string;
  value: number;
  scoreId: string;
}

export type ViewRow = { kind: 'table'; row: BoardRow } | { kind: 'own'; own: OwnRow };

/** The score a saved summary contributes to `board`, or null when it does not belong there. */
function valueOn(s: ScoreSummary, board: Board): number | null {
  switch (board) {
    case 'cpu-single':
      return s.category === 'cpu' ? s.single : null;
    case 'cpu-multi':
      return s.category === 'cpu' ? s.multi : null;
    case 'gpu-compute':
      return s.category === 'gpu' ? s.compute : null;
    case 'gpu-graphics':
      return s.category === 'gpu' ? s.graphics : null;
    case 'disk':
      // Only the B1 test is comparable with the table's points.
      return s.category === 'disk' && s.diskProfile !== 'b2' ? s.points : null;
  }
}

/** The best valid, non-provisional score of the current version for each model, highest first. */
export function ownRows(scores: ScoreSummary[], board: Board, version: string): OwnRow[] {
  const best = new Map<string, OwnRow>();
  for (const s of scores) {
    const value = valueOn(s, board);
    if (value === null || !s.valid || s.provisional || s.scoreVersion !== version || !s.model) continue;
    if ((best.get(s.model)?.value ?? -Infinity) < value) best.set(s.model, { model: s.model, value, scoreId: s.id });
  }
  return [...best.values()].sort((a, b) => b.value - a.value);
}

/** The score version of the table that holds `board`. */
export const versionOf = (table: BoardTable, board: Board): string => table.versions[board === 'disk' ? 'disk' : board.startsWith('gpu') ? 'gpu' : 'cpu'];

/**
 * The rows to show for `board`: ten, centred on the user's best row (the top ten without one),
 * the percentile of that best score among the table rows, and the number of table models.
 */
export function boardView(table: BoardTable, own: OwnRow[], board: Board): { rows: ViewRow[]; percentile: number | null; models: number } {
  const version = versionOf(table, board);
  const rows = table.rows.filter((r) => r.board === board && r.scoreVersion === version);
  const merged: ViewRow[] = [
    ...rows.map((row): ViewRow => ({ kind: 'table', row })),
    ...own.map((o): ViewRow => ({ kind: 'own', own: o })),
  ];
  const valueOf = (r: ViewRow) => (r.kind === 'table' ? r.row.value : r.own.value);
  merged.sort((a, b) => valueOf(b) - valueOf(a));
  const mine = merged.findIndex((r) => r.kind === 'own');
  const start = mine < 0 ? 0 : Math.min(Math.max(mine - NEIGHBOURS / 2, 0), Math.max(merged.length - NEIGHBOURS, 0));
  const top = own.reduce((m, o) => Math.max(m, o.value), -Infinity);
  const percentile =
    own.length > 0 && rows.length >= PERCENTILE_MIN_ROWS ? Math.floor((100 * rows.filter((r) => r.value < top).length) / rows.length) : null;
  return { rows: merged.slice(start, start + NEIGHBOURS), percentile, models: rows.length };
}
