import type uPlot from 'uplot';

/**
 * Stand-in for uPlot in jsdom, which has no canvas: test-setup.ts mocks 'uplot' with
 * this class, and tests read `instances` to check what a component handed to uPlot.
 */
export class FakeUplot {
  static instances: FakeUplot[] = [];
  opts: uPlot.Options;
  data: uPlot.AlignedData;
  target: HTMLElement | undefined;
  destroyed = false;
  setDataCalls = 0;
  scales: Array<{ key: string; range: { min: number; max: number } }> = [];
  sizes: { width: number; height: number }[] = [];

  constructor(opts: uPlot.Options, data: uPlot.AlignedData, target?: HTMLElement) {
    this.opts = opts;
    this.data = data;
    this.target = target;
    FakeUplot.instances.push(this);
  }

  setData(data: uPlot.AlignedData): void {
    this.data = data;
    this.setDataCalls++;
  }

  setSize(size: { width: number; height: number }): void {
    this.sizes.push(size);
  }

  setScale(key: string, range: { min: number; max: number }): void {
    this.scales.push({ key, range });
  }

  destroy(): void {
    this.destroyed = true;
  }
}
