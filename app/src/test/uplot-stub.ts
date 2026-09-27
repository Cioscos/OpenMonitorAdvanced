import type uPlot from 'uplot';
import { canvasFixture } from './uplot-canvas';

/**
 * Stand-in for uPlot in jsdom, which has no canvas: test-setup.ts mocks 'uplot' with
 * this class, and tests read `instances` to check what a component handed to uPlot.
 */
export class FakeUplot {
  static instances: FakeUplot[] = [];
  static paths: uPlot.Series.PathBuilderFactories;
  static pxRatio = 1;
  /** X split increment in seconds that the stub reports to the component's `splits`. */
  static xIncrement = 60;
  /** Like the real uPlot, commit scale changes (and the draw hooks) in a microtask. */
  static deferDraw = false;
  opts: uPlot.Options;
  data: uPlot.AlignedData;
  target: HTMLElement | undefined;
  destroyed = false;
  bbox = { left: 72, top: 10, width: 600, height: 200 };
  over = document.createElement('div');
  root = document.createElement('div');
  cursor = { left: -10, top: -10 };
  setCursorCalls: Array<{ left: number; top: number }> = [];
  private range = { min: 0, max: 4 };
  get series() { return this.opts.series; }
  setDataCalls = 0;
  scales: Array<{ key: string; range: { min: number; max: number } }> = [];
  sizes: { width: number; height: number }[] = [];
  yAutoDecisions: boolean[][] = [];

  constructor(opts: uPlot.Options, data: uPlot.AlignedData, target?: HTMLElement) {
    this.opts = opts;
    this.data = data;
    this.target = target;
    this.root.append(this.over);
    target?.append(this.root);
    FakeUplot.instances.push(this);
  }

  setData(data: uPlot.AlignedData): void {
    this.data = data;
    this.setDataCalls++;
  }

  setSize(size: { width: number; height: number }): void {
    this.sizes.push(size);
    this.draw();
  }

  setScale(key: string, range: { min: number; max: number }): void {
    this.scales.push({ key, range });
    if (key === 'x') this.range = range;
    if (FakeUplot.deferDraw) queueMicrotask(() => { if (!this.destroyed) this.draw(); });
    else this.draw();
  }

  batch(fn: () => void): void { fn(); }

  setCursor(opts: { left: number; top: number }): void {
    this.setCursorCalls.push(opts);
  }

  valToPos(value: number, scale: string, canvasPixels = false): number {
    const valuePx = scale === 'x'
      ? this.bbox.left + (value - this.range.min) / (this.range.max - this.range.min) * this.bbox.width
      : this.bbox.top + this.bbox.height * (1 - value / (scale === 'celsius' ? 200 : 100));
    return canvasPixels ? valuePx : valuePx / FakeUplot.pxRatio;
  }

  private draw(): void {
    const { plot } = canvasFixture(this.data, this.opts.series);
    Object.assign(plot.bbox, this.bbox);
    Object.assign(plot.scales.x, this.range);
    for (const series of this.opts.series.slice(1)) {
      if (!plot.scales[series.scale!]) plot.scales[series.scale!] = { ...plot.scales.percent };
    }
    plot.valToPos = this.valToPos.bind(this);
    this.yAutoDecisions.push(Object.entries(this.opts.scales ?? {}).filter(([key]) => key !== 'x').map(([, scale]) =>
      typeof scale.auto === 'function' ? scale.auto(plot, false) : scale.auto !== false));
    const splits = this.opts.axes?.[0].splits;
    if (typeof splits === 'function') splits(plot, 0, this.range.min, this.range.max, FakeUplot.xIncrement, 100);
    if (typeof Path2D !== 'undefined') {
      for (let i = 1; i < this.opts.series.length; i++) {
        this.opts.series[i].paths?.(plot, i, 0, this.data[0].length - 1);
      }
    }
    for (const hook of this.opts.hooks?.draw ?? []) hook?.(plot);
  }

  destroy(): void {
    this.destroyed = true;
    this.root.remove();
  }
}
