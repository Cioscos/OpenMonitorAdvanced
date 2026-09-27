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
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, paths, [], 'en', 40, theme);
  expect(strokes).toHaveLength(16);
  for (let i = 0; i < 8; i++) {
    expect(ctx.stroke.mock.calls[i * 2][0]).toBe(paths[i].stroke);
    expect(ctx.stroke.mock.calls[i * 2 + 1][0]).toBe(paths[i].stroke);
    expect(strokes.slice(i * 2, i * 2 + 2).map((s) => s.color)).toEqual([paths[i].color, paths[i].color]);
    expect(strokes[i * 2].alpha).toBeLessThan(1);
    expect(strokes[i * 2 + 1].alpha).toBe(1);
  }
  expect(ctx.rect).toHaveBeenCalledWith(10, 20, 440, 200);
});

test('preserves each path gap clip and draws nothing for empty paths', () => {
  const { plot, ctx, strokes } = canvasFixture([[0, 1], [20, null], [30, 40]]);
  const gap = new Path2D();
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, [
    { stroke: new Path2D(), gapsClip: gap, color: '#f0f' },
    { stroke: null, gapsClip: null, color: '#0ff' },
  ], [], 'en', 0, theme);
  expect(ctx.clip.mock.calls).toEqual([[], [gap]]);
  expect(strokes).toHaveLength(2);
  expect(ctx.save).toHaveBeenCalledTimes(ctx.restore.mock.calls.length);
});

test('keeps grid, tick marks and labels crisp and draws a future tick in right overscan', () => {
  const { plot, ctx, strokes, fills } = canvasFixture([[0, 1], [20, 40]]);
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, [], [0, 2, 4.2], 'it', 30, theme);
  expect(ctx.moveTo).toHaveBeenCalledWith(430, 20);
  expect(ctx.fillText).toHaveBeenCalledWith(new Date(4200).toLocaleTimeString('it', { hour: '2-digit', minute: '2-digit' }), 430, expect.any(Number));
  expect(strokes.length).toBeGreaterThan(0);
  expect(strokes.every((s) => s.blur === 0 && s.color === theme.gridColor)).toBe(true);
  expect(fills).toHaveLength(0);
  expect(ctx.shadowBlur).toBe(0);
  expect(ctx.fillStyle).toBe('');
});

test('scales widths and font with DPR while keeping canvas pixel positions', () => {
  uPlot.pxRatio = 2;
  const { plot, ctx, strokes, textDraws } = canvasFixture([[0, 1], [20, 40]]);
  const path = new Path2D();
  drawChartCanvas(ctx as unknown as CanvasRenderingContext2D, plot, [{ stroke: path, gapsClip: null, color: '#f0f' }], [2], 'en', 40, theme);
  expect(ctx.rect).toHaveBeenCalledWith(10, 20, 440, 200);
  expect(ctx.moveTo).toHaveBeenCalledWith(210, 20);
  expect(textDraws[0]).toEqual({ font: '24px sans-serif', color: theme.textColor, blur: 0 });
  expect(strokes.slice(1, 3).map((s) => s.width)).toEqual([12, 3]);
  expect(ctx.save).toHaveBeenCalledTimes(ctx.restore.mock.calls.length);
});
