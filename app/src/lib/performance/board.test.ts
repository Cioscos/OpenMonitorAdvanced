import { makeScoreFile, scoreSummaryOf } from '../../test/fake-backend';
import type { Board, BoardRow, BoardTable, ScoreSummary } from '../types';
import { boardView, NEIGHBOURS, ownRows } from './board';

const sum = (over: Partial<ScoreSummary> = {}): ScoreSummary => ({ ...scoreSummaryOf(makeScoreFile()), ...over });
const row = (i: number, board: Board = 'cpu-single', over: Partial<BoardRow> = {}): BoardRow => ({
  board,
  scoreVersion: board === 'disk' ? 'disk-1' : board.startsWith('gpu') ? 'gpu-1' : 'cpu-1',
  model: `Model ${i}`,
  key: `model ${i}`,
  value: i * 100,
  n: 5,
  source: 'community',
  ...over,
});
const table = (rows: BoardRow[]): BoardTable => ({
  rows,
  communityAt: null,
  checkedAtMs: null,
  error: null,
  enabled: true,
  versions: { cpu: 'cpu-1', gpu: 'gpu-1', disk: 'disk-1' },
});
/** Values n*100 down to 100. */
const range = (n: number, board: Board = 'cpu-single') => Array.from({ length: n }, (_, i) => row(n - i, board));

test('own_rows_take_the_best_valid_current_score_per_model', () => {
  const scores = [
    sum({ id: 'a', model: 'X', single: 1000 }),
    sum({ id: 'b', model: 'X', single: 1400 }),
    sum({ id: 'c', model: 'X', single: 9000, provisional: true }),
    sum({ id: 'd', model: 'X', single: 9000, valid: false }),
    sum({ id: 'e', model: 'X', single: 9000, scoreVersion: 'cpu-0' }),
    sum({ id: 'f', model: 'Y', single: 800 }),
  ];
  expect(ownRows(scores, 'cpu-single', 'cpu-1')).toEqual([
    { model: 'X', value: 1400, scoreId: 'b' },
    { model: 'Y', value: 800, scoreId: 'f' },
  ]);
});

test('neighbours_center_on_the_best_own_row', () => {
  const own = { model: 'Me', value: 1550, scoreId: 's' };
  const view = boardView(table(range(30)), [own], 'cpu-single');
  expect(view.rows).toHaveLength(NEIGHBOURS);
  expect(view.rows[5]).toEqual({ kind: 'own', own });
});

test('neighbours_clamp_at_the_edges', () => {
  const top = boardView(table(range(30)), [{ model: 'Me', value: 99999, scoreId: 's' }], 'cpu-single');
  expect(top.rows[0].kind).toBe('own');
  const bottom = boardView(table(range(30)), [{ model: 'Me', value: 1, scoreId: 's' }], 'cpu-single');
  expect(bottom.rows).toHaveLength(NEIGHBOURS);
  expect(bottom.rows[NEIGHBOURS - 1].kind).toBe('own');
});

test('without_own_scores_the_top_ten_show', () => {
  const view = boardView(table(range(30)), [], 'cpu-single');
  expect(view.rows.map((r) => (r.kind === 'table' ? r.row.value : 0))).toEqual([3000, 2900, 2800, 2700, 2600, 2500, 2400, 2300, 2200, 2100]);
  expect(view.percentile).toBeNull();
  expect(view.models).toBe(30);
});

test('percentile_needs_ten_rows', () => {
  const own = [{ model: 'Me', value: 750, scoreId: 's' }];
  expect(boardView(table(range(9)), own, 'cpu-single').percentile).toBeNull();
  expect(boardView(table(range(10)), own, 'cpu-single').percentile).toBe(70);
});

test('other_versions_are_not_compared', () => {
  const view = boardView(table([...range(12), row(50, 'cpu-single', { scoreVersion: 'cpu-0' })]), [], 'cpu-single');
  expect(view.models).toBe(12);
  expect(view.rows.every((r) => r.kind === 'table' && r.row.scoreVersion === 'cpu-1')).toBe(true);
});

test('disk_own_rows_use_points_of_b1_only', () => {
  const scores = [
    sum({ id: 'b1', category: 'disk', model: 'D', points: 700, diskProfile: 'b1' }),
    sum({ id: 'b2', category: 'disk', model: 'D', points: 900, diskProfile: 'b2' }),
    sum({ id: 'p', category: 'disk', model: 'E', points: null, diskProfile: 'b1' }),
  ].map((s) => ({ ...s, scoreVersion: 'disk-1' }));
  expect(ownRows(scores, 'disk', 'disk-1')).toEqual([{ model: 'D', value: 700, scoreId: 'b1' }]);
});
