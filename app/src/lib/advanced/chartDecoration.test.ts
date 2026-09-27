import uPlot from 'uplot';
import { canvasFixture, RecordingPath } from '../../test/uplot-canvas';
import { drawChartSeriesDecoration } from './chartDecoration';

beforeEach(() => vi.stubGlobal('Path2D', RecordingPath));
afterEach(() => { vi.unstubAllGlobals(); uPlot.pxRatio = 1; });

test('clips a subtle coloured stroke pass and an opaque white endpoint, restoring canvas state', () => {
  const { plot, ctx, strokes, fills } = canvasFixture([[1, 2], [20, 80]]);
  const stroke = new Path2D();
  const gaps = new Path2D();
  drawChartSeriesDecoration(plot, 1, stroke, gaps, '#ff4fd8');
  expect(ctx.rect).toHaveBeenCalledWith(10, 20, 400, 200);
  expect(ctx.clip.mock.calls).toEqual([[], [gaps]]);
  expect(ctx.stroke).toHaveBeenCalledWith(stroke);
  expect(strokes).toHaveLength(1);
  expect(strokes[0].color).toBe('#ff4fd8');
  expect(strokes[0].alpha).toBeGreaterThan(0);
  expect(strokes[0].alpha).toBeLessThan(0.3);
  expect(fills).toEqual([{ alpha: 1, color: '#fff', blur: 0 }]);
  expect(ctx.arc).toHaveBeenCalledWith(210, 60, 3, 0, Math.PI * 2);
  expect(ctx.save).toHaveBeenCalledTimes(1);
  expect(ctx.restore).toHaveBeenCalledTimes(1);
  expect(ctx.globalAlpha).toBe(1);
  expect(ctx.shadowBlur).toBe(0);
  expect(ctx.strokeStyle).toBe('');
});

test.each([null, NaN, Infinity])('does not backtrack to an earlier endpoint when the final value is %s', (last) => {
  const { plot, ctx } = canvasFixture([[1, 2], [40, last]]);
  drawChartSeriesDecoration(plot, 1, new Path2D(), null, '#f0f');
  expect(ctx.arc).not.toHaveBeenCalled();
  expect(ctx.stroke).toHaveBeenCalledTimes(1);
});

test('a singleton draws its point without inventing a line and respects the series Y scale and DPR', () => {
  uPlot.pxRatio = 2;
  const { plot, ctx } = canvasFixture([[2], [100]], [{ scale: 'x' }, { scale: 'celsius' }]);
  drawChartSeriesDecoration(plot, 1, null, null, '#f0f');
  expect(ctx.stroke).not.toHaveBeenCalled();
  expect(ctx.arc).toHaveBeenCalledWith(210, 120, 6, 0, Math.PI * 2);
});

test.each([[-1, 40], [5, 40], [2, -1], [2, 101]])('does not draw an endpoint outside the plot (%s, %s)', (x, y) => {
  const { plot, ctx } = canvasFixture([[x], [y]]);
  drawChartSeriesDecoration(plot, 1, null, null, '#f0f');
  expect(ctx.arc).not.toHaveBeenCalled();
});

test('empty series and hidden series produce no marker', () => {
  const empty = canvasFixture([[], []]);
  drawChartSeriesDecoration(empty.plot, 1, null, null, '#f0f');
  expect(empty.ctx.arc).not.toHaveBeenCalled();
  const hidden = canvasFixture([[2], [40]], [{ scale: 'x' }, { scale: 'percent', show: false }]);
  drawChartSeriesDecoration(hidden.plot, 1, new Path2D(), null, '#f0f');
  expect(hidden.ctx.stroke).not.toHaveBeenCalled();
  expect(hidden.ctx.arc).not.toHaveBeenCalled();
});

// Characterizes the pinned uPlot 1.6.32 path factory, not a duplicate spline implementation.
test.each([[0, 100, 0], [42, 42, 42], [0, 1, 100]])('uPlot spline remains within adjacent sample bounds for %j', (...ys) => {
  const { plot } = canvasFixture([[0, 1, 4], ys]);
  const paths = uPlot.paths.spline!()(plot, 1, 0, 2)!;
  const commands = (paths.stroke as unknown as RecordingPath).commands;
  expect(commands.filter((c) => c.kind === 'cubic')).toHaveLength(2);
  let prev = commands[0].args;
  for (const command of commands.slice(1)) {
    const [x1, y1, x2, y2, x3, y3] = command.args;
    const lo = Math.min(prev[1], y3), hi = Math.max(prev[1], y3);
    for (let i = 0; i <= 100; i++) {
      const t = i / 100, s = 1 - t;
      const y = s ** 3 * prev[1] + 3 * s ** 2 * t * y1 + 3 * s * t ** 2 * y2 + t ** 3 * y3;
      expect(y).toBeGreaterThanOrEqual(lo - 1e-9);
      expect(y).toBeLessThanOrEqual(hi + 1e-9);
    }
    expect([x1, x2, x3].every(Number.isFinite)).toBe(true);
    prev = [x3, y3];
  }
});
