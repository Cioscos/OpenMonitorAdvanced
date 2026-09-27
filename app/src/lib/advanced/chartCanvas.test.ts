import uPlot from 'uplot';
import { canvasFixture, RecordingPath } from '../../test/uplot-canvas';
import { drawChartCanvas } from './chartCanvas';

const theme = { gridColor: '#334455', textColor: '#ccddee' };

beforeEach(() => vi.stubGlobal('Path2D', RecordingPath));
afterEach(() => { vi.unstubAllGlobals(); uPlot.pxRatio = 1; });

test('paints eight real spline paths on two Y scales with one glow and one line each', () => {
  const series: uPlot.Series[] = [{ scale: 'x' }, ...Array.from({ length: 8 }, (_, i) => ({ scale: i % 2 ? 'celsius' : 'percent' }))];
  const { plot, ctx, strokes } = canvasFixture([[0, 1, 4], ...Array.from({ length: 8 }, (_, i) => [i, i + 5, i + 10])], series);
  const paths = series.slice(1).map((_, i) => {
    const result = uPlot.paths.spline!()(plot, i + 1, 0, 2)!;
    return { stroke: result.stroke as Path2D, gapsClip: result.clip ?? null, color: `#00000${i}` };
  });
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, paths, [], 60, 'en', 40, theme);
  expect(strokes).toHaveLength(16);
  for (let i = 0; i < 8; i++) {
    expect(ctx.stroke.mock.calls[i * 2][0]).toBe(paths[i].stroke);
    expect(ctx.stroke.mock.calls[i * 2 + 1][0]).toBe(paths[i].stroke);
    expect(strokes.slice(i * 2, i * 2 + 2).map((s) => s.color)).toEqual([paths[i].color, paths[i].color]);
    expect(strokes[i * 2].alpha).toBeLessThan(1);
    expect(strokes[i * 2 + 1].alpha).toBe(1);
  }
  expect(ctx.rect).toHaveBeenCalledWith(-30, 20, 480, 200);
});

test('preserves each path gap clip and draws nothing for empty paths', () => {
  const { plot, ctx, strokes } = canvasFixture([[0, 1], [20, null], [30, 40]]);
  const gap = new Path2D();
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, [
    { stroke: new Path2D(), gapsClip: gap, color: '#f0f' },
    { stroke: null, gapsClip: null, color: '#0ff' },
  ], [], 60, 'en', 0, theme);
  expect(ctx.clip.mock.calls).toEqual([[], [gap]]);
  expect(strokes).toHaveLength(2);
  expect(ctx.save).toHaveBeenCalledTimes(ctx.restore.mock.calls.length);
});

test('keeps grid, tick marks and labels crisp and draws a future tick in right overscan', () => {
  const { plot, ctx, strokes, fills } = canvasFixture([[0, 1], [20, 40]]);
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, [], [0, 2, 4.2], 60, 'it', 30, theme);
  expect(ctx.moveTo).toHaveBeenCalledWith(430, 20);
  expect(ctx.fillText).toHaveBeenCalledWith(new Date(4200).toLocaleTimeString('it', { hour: '2-digit', minute: '2-digit' }), 430, expect.any(Number));
  expect(strokes.length).toBeGreaterThan(0);
  expect(strokes.every((s) => s.blur === 0 && s.color === theme.gridColor)).toBe(true);
  expect(fills).toHaveLength(0);
  expect(ctx.shadowBlur).toBe(0);
  expect(ctx.fillStyle).toBe('');
});

test('labels sub-minute ticks with seconds using the increment it paints', () => {
  const { plot, ctx } = canvasFixture([[0, 1], [20, 40]]);
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, [], [0, 1, 2, 3], 1, 'it', 0, theme);
  const withSeconds = { hour: '2-digit', minute: '2-digit', second: '2-digit' } as const;
  expect(ctx.fillText.mock.calls.map(([text]) => text)).toEqual(
    [0, 1, 2, 3].map((s) => new Date(s * 1000).toLocaleTimeString('it', withSeconds)),
  );
});

test('scales widths and font with DPR while keeping canvas pixel positions', () => {
  uPlot.pxRatio = 2;
  const { plot, ctx, strokes, textDraws } = canvasFixture([[0, 1], [20, 40]]);
  const path = new Path2D();
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, [{ stroke: path, gapsClip: null, color: '#f0f' }], [2], 60, 'en', 40, theme);
  expect(ctx.rect).toHaveBeenCalledWith(-30, 20, 480, 200);
  expect(ctx.moveTo).toHaveBeenCalledWith(210, 20);
  expect(textDraws[0]).toEqual({ font: '24px sans-serif', color: theme.textColor, blur: 0 });
  expect(strokes.slice(1, 3).map((s) => s.width)).toEqual([12, 3]);
  expect(ctx.save).toHaveBeenCalledTimes(ctx.restore.mock.calls.length);
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
