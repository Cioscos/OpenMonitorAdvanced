import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { WINDOW_KEY, seriesKey } from '../../lib/advanced/persist';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { HistorySeed, Sensor } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { FakeUplot } from '../../test/uplot-stub';
import { canvasFixture, RecordingPath } from '../../test/uplot-canvas';
import { drawChartCanvas, type ChartHeldSegment } from '../../lib/advanced/chartCanvas';
import { formatTimeTick, TIME_LABEL_GAP_PX } from '../../lib/advanced/chartData';
import HistoryChart from './HistoryChart.svelte';

// Calls through to the real painter; tests read the held segments it was handed.
vi.mock('../../lib/advanced/chartCanvas', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../lib/advanced/chartCanvas')>();
  return { ...actual, drawChartCanvas: vi.fn(actual.drawChartCanvas) };
});
const paints = () => vi.mocked(drawChartCanvas).mock.calls;
/** Held segments of the latest canvas paint, in canvas pixels. */
const lastHeld = (): ReadonlyArray<ChartHeldSegment> => paints().at(-1)?.[8] ?? [];

const plots = FakeUplot.instances;
const GPU = 'gpu/pci-0000:01:00.0';
const LOAD = `${GPU}/load/core`;
const TEMP = `${GPU}/temperature/core`;
const HOTSPOT = `${GPU}/temperature/hotspot`;
const VRAM = `${GPU}/data/memory-dedicated-used`;
const gpuSensors = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU);
const cpuSensors = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === 'cpu/0');
const byId = (id: string) => MOCK_SCHEMA.sensors.find((s) => s.id === id)!;
const index = (id: string) => MOCK_SCHEMA.sensors.findIndex((s) => s.id === id);
const labelOf = (s: Sensor) => t(`sensor.${s.label.key}`, s.label.arg === undefined ? {} : { arg: s.label.arg });
const checkbox = (label: string) => screen.getByLabelText(label) as HTMLInputElement;

let visibility: DocumentVisibilityState = 'visible';
let monotonicMs = 0;
let reducedMotion = false;
let nextFrameId = 1;
const frames = new Map<number, FrameRequestCallback>();
const motionListeners = new Set<() => void>();
function frame(at: number) {
  monotonicMs = at;
  const [id, callback] = [...frames][0] ?? [];
  if (id === undefined || !callback) throw new Error('No animation frame pending');
  frames.delete(id);
  callback(at);
}
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
const setVisibility = (state: DocumentVisibilityState) => {
  visibility = state;
  document.dispatchEvent(new Event('visibilitychange'));
};

beforeEach(() => {
  vi.stubGlobal('Path2D', RecordingPath);
  const contexts = new WeakMap<HTMLCanvasElement, CanvasRenderingContext2D>();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    if (!contexts.has(this)) contexts.set(this, {
      ...canvasFixture([[0], [1]]).ctx, clearRect: vi.fn(), translate: vi.fn(),
    } as unknown as CanvasRenderingContext2D);
    return contexts.get(this)!;
  });
  plots.length = 0;
  vi.mocked(drawChartCanvas).mockClear();
  localStorage.clear();
  i18n.locale = 'en';
  visibility = 'visible';
  monotonicMs = 0;
  reducedMotion = false;
  nextFrameId = 1;
  frames.clear();
  motionListeners.clear();
  vi.spyOn(performance, 'now').mockImplementation(() => monotonicMs);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = nextFrameId++;
    frames.set(id, callback);
    return id;
  });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
  vi.stubGlobal('matchMedia', (query: string) => ({
    get matches() { return reducedMotion; },
    media: query,
    addEventListener: (_type: string, listener: () => void) => { if (query.includes('reduced-motion')) motionListeners.add(listener); },
    removeEventListener: (_type: string, listener: () => void) => motionListeners.delete(listener),
  }));
});
afterEach(cleanup);
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

/** FakeBackend with two history samples (1 s and 2 s): column i holds [10 + i, 20 + i]. */
function fakeBackend(): FakeBackend {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.history = { timestampsMs: [1000, 2000], series: Array.from({ length: 8 }, (_, i) => [10 + i, 20 + i]) };
  return backend;
}

function renderChart(backend: FakeBackend, store = new LiveStore(), sensors = gpuSensors, defaults = [LOAD, TEMP], sectionId = GPU) {
  if (!store.schema) store.applySchema(MOCK_SCHEMA);
  return render(HistoryChart, { sectionId, sensors, defaults, schema: MOCK_SCHEMA, store, backend });
}

test('seeds the default series and draws them on two unit scales', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  expect(backend.historyCalls).toEqual([{ ids: [LOAD, TEMP], seconds: 300, maxPoints: undefined }]);
  const [plot] = plots;
  expect(plot.opts.series.slice(1).map((s) => s.label)).toEqual([labelOf(byId(LOAD)), labelOf(byId(TEMP))]);
  expect(plot.opts.series.slice(1).map((s) => s.scale)).toEqual(['percent', 'celsius']);
  expect(plot.opts.axes?.map((a) => [a.scale, a.side])).toEqual([[undefined, undefined], ['percent', 3], ['celsius', 1]]);
  expect(plot.data).toEqual([[1, 2], [10, 20], [11, 21]]);
  expect(screen.getByRole('button', { name: t('advanced.chart.window.300') }).getAttribute('aria-pressed')).toBe('true');

  expect(plot.opts.axes?.[0].values).toEqual([]);
  expect(plot.opts.axes?.[0].grid?.show).toBe(false);
  expect(plot.opts.axes?.[0].ticks?.show).toBe(false);
});

test('long windows ask for decimated history and the choice persists', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.3600') }));
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(backend.historyCalls.at(-1)).toEqual({ ids: [LOAD, TEMP], seconds: 3600, maxPoints: 900 });
  expect(localStorage.getItem(WINDOW_KEY)).toBe('3600');
  expect(plots[0].destroyed).toBe(true);

  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.60') }));
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(backend.historyCalls.at(-1)).toEqual({ ids: [LOAD, TEMP], seconds: 60, maxPoints: undefined });
});

test('the saved window is used on mount', async () => {
  localStorage.setItem(WINDOW_KEY, '1800');
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(backend.historyCalls).toEqual([{ ids: [LOAD, TEMP], seconds: 1800, maxPoints: 900 }]);
});

test('live snapshots extend the chart and retain one point outside the left edge', async () => {
  localStorage.setItem(WINDOW_KEY, '60');
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 62_000, values: mockValues(1) });
  flushSync();
  const plot = plots[0];
  expect(plot.setDataCalls).toBe(1);
  // 1000 ms precedes the visible edge; uPlot needs it to draw through that edge.
  expect(plot.data).toEqual([
    [1, 2, 62],
    [10, 20, mockValues(1)[index(LOAD)]],
    [11, 21, mockValues(1)[index(TEMP)]],
  ]);
});

test('a snapshot that arrives while history loads is not lost', async () => {
  const backend = fakeBackend();
  let resolve!: (h: HistorySeed) => void;
  backend.getHistory = () => new Promise((done) => (resolve = done));
  const store = new LiveStore();
  renderChart(backend, store);
  await vi.waitFor(() => expect(resolve).toBeDefined());

  store.applySnapshot({ revision: 1, seq: 5, timestampMs: 3000, values: mockValues(5) });
  flushSync();
  resolve({ revision: 1, seq: 4, timestampsMs: [1000, 2000], series: [[1, 2], [3, 4]] });
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(plots[0].data[0]).toEqual([1, 2, 3]);
});

test('the picker allows at most 8 series and saves the choice per section', async () => {
  const backend = fakeBackend();
  renderChart(backend, new LiveStore(), cpuSensors, ['cpu/0/load/total', 'cpu/0/clock/effective'], 'cpu/0');
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  const thread = (i: number) => checkbox(t('sensor.cpu.load.thread', { arg: i }));
  for (let i = 0; i < 6; i++) await fireEvent.click(thread(i));
  expect(thread(6).disabled).toBe(true);
  expect(screen.getByText(`${t('advanced.chart.series')} · 8/8`)).toBeTruthy();
  await vi.waitFor(() => expect(backend.historyCalls.at(-1)?.ids).toHaveLength(8));
  expect(JSON.parse(localStorage.getItem(seriesKey('cpu/0'))!)).toHaveLength(8);

  await fireEvent.click(thread(0));
  expect(thread(6).disabled).toBe(false);
});

test('the picker refuses a third unit', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  expect(checkbox(labelOf(byId(VRAM))).disabled).toBe(true);
  expect(checkbox(labelOf(byId(HOTSPOT))).disabled).toBe(false);
  await fireEvent.click(checkbox(labelOf(byId(TEMP))));
  expect(checkbox(labelOf(byId(VRAM))).disabled).toBe(false);
});

test('the saved selection of the section wins over the defaults', async () => {
  localStorage.setItem(seriesKey(GPU), JSON.stringify([VRAM, 'gone']));
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(backend.historyCalls[0].ids).toEqual([VRAM]);
});

test('an empty selection shows a hint and fetches nothing', async () => {
  localStorage.setItem(seriesKey(GPU), '[]');
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(screen.getByText(t('advanced.chart.empty'))).toBeTruthy());
  expect(backend.historyCalls).toEqual([]);
  expect(plots).toHaveLength(0);
});

test('rendering pauses while hidden and history is reloaded when visible', async () => {
  const backend = fakeBackend();
  const store = new LiveStore();
  renderChart(backend, store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  setVisibility('hidden');
  flushSync();
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].setDataCalls).toBe(0);
  expect(backend.historyCalls).toHaveLength(1);

  setVisibility('visible');
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(backend.historyCalls).toHaveLength(2);
});

test('series colours come from the theme tokens', async () => {
  document.documentElement.style.setProperty('--accent', '#ff4fd8');
  document.documentElement.style.setProperty('--accent-2', '#4cc9f0');
  try {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    expect(plots[0].opts.series.slice(1).map((s) => s.stroke)).toEqual(['#ff4fd8', '#4cc9f0']);
  } finally {
    document.documentElement.removeAttribute('style');
  }
});

test('a history reply arriving while hidden cannot create a plot', async () => {
  const backend = fakeBackend();
  let resolve!: (h: HistorySeed) => void;
  backend.getHistory = () => new Promise((done) => (resolve = done));
  renderChart(backend);
  await vi.waitFor(() => expect(resolve).toBeDefined());
  setVisibility('hidden');
  flushSync();
  resolve({ revision: 1, seq: 0, timestampsMs: [1000], series: [[1], [2]] });
  await new Promise((done) => setTimeout(done, 0));
  expect(plots).toHaveLength(0);
});

test('history from another schema revision is never plotted', async () => {
  const backend = fakeBackend();
  backend.getHistory = async () => ({ revision: 2, seq: 0, timestampsMs: [1000], series: [[1], [2]] });
  renderChart(backend);
  await new Promise((done) => setTimeout(done, 0));
  expect(plots).toHaveLength(0);
});

test('a newer snapshot after a clock rollback starts a new chart segment', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 10_000, values: mockValues(1) });
  flushSync();
  store.applySnapshot({ revision: 1, seq: 2, timestampMs: 5000, values: mockValues(2) });
  flushSync();
  expect(plots[0].data[0]).toEqual([5]);
  expect(plots[0].data[1]).toEqual([mockValues(2)[index(LOAD)]]);
});

test('unmounting destroys the plot', async () => {
  const { unmount } = renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  unmount();
  expect(plots[0].destroyed).toBe(true);
});

test('frames translate the shared series and X canvas without rebuilding uPlot or replacing data', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const plot = plots[0];
  const scaleCalls = plot.scales.length;
  const paintCalls = vi.mocked(HTMLCanvasElement.prototype.getContext).mock.calls.length;
  expect(plot.setDataCalls).toBe(0);
  frame(0);
  frame(250);
  expect(plot.setDataCalls).toBe(0);
  expect(plot.scales).toHaveLength(scaleCalls);
  expect(vi.mocked(HTMLCanvasElement.prototype.getContext)).toHaveBeenCalledTimes(paintCalls);
  const canvas = document.querySelector<HTMLCanvasElement>('.chart-canvas')!;
  expect(canvas).not.toBeNull();
  expect(canvas.style.transform).toBe(`translateX(${-0.25 * plot.bbox.width / 300}px)`);

  monotonicMs = 500;
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plot.setDataCalls).toBe(1);
  frame(750);
  expect(plot.setDataCalls).toBe(1);
  expect(plot.scales.at(-1)).toEqual({ key: 'x', range: { min: -297, max: 3 } });
  expect(canvas.style.transform).toBe(`translateX(${-0.25 * plot.bbox.width / 300}px)`);
});

test('hiding and unmounting stop scale changes', async () => {
  const { unmount } = renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const plot = plots[0];
  frame(0);
  const count = plot.scales.length;
  setVisibility('hidden');
  expect(frames.size).toBe(0);
  expect(plot.scales).toHaveLength(count);
  setVisibility('visible');
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const resumed = plots[1];
  const resumedCount = resumed.scales.length;
  unmount();
  expect(frames.size).toBe(0);
  expect(resumed.scales).toHaveLength(resumedCount);
});

test('resuming visibility does not catch up hidden time in one frame', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(0);
  setVisibility('hidden');
  monotonicMs = 20_000;
  setVisibility('visible');
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const plot = plots[1];
  frame(20_000);
  expect(plot.scales.at(-1)?.range.max).toBeCloseTo(2, 3);
});

test('reduced motion shows new snapshots without animation frames', async () => {
  reducedMotion = true;
  for (const listener of motionListeners) listener();
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(frames.size).toBe(0);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].data[0]).toEqual([1, 2, 3]);
  expect(plots[0].scales.at(-1)).toEqual({ key: 'x', range: { min: -297, max: 3 } });
});

test('reduced-motion pause keeps snapshots visible and resumes without replaying elapsed time', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(250);
  reducedMotion = true;
  for (const listener of motionListeners) listener();
  expect(frames.size).toBe(0);

  monotonicMs = 600_000;
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].scales.at(-1)?.range.max).toBe(3);

  monotonicMs = 1_200_000;
  reducedMotion = false;
  for (const listener of motionListeners) listener();
  frame(1_200_000);
  expect(plots[0].scales.at(-1)?.range.max).toBe(3);
});

test('captures real spline gap clips for the composite and suppresses native strokes', async () => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [0, 1000, 2000, 3000, 4000], series: [[10, 20, null, 30, 40], [40, 80, null, 100, 120]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const configured = plots[0];
  const fixture = canvasFixture(configured.data, configured.opts.series);
  const contexts = vi.mocked(HTMLCanvasElement.prototype.getContext).mock.results.map((r) => r.value);
  const ctx = contexts.at(-1)! as CanvasRenderingContext2D;
  vi.mocked(ctx.stroke).mockClear();
  vi.mocked(ctx.clip).mockClear();
  for (let i = 1; i <= 2; i++) {
    expect(configured.opts.series[i].paths!(fixture.plot, i, 0, 4)).toBeNull();
  }
  configured.opts.hooks!.draw![0]!(fixture.plot);
  const strokes = vi.mocked(ctx.stroke).mock.calls.map((c) => c[0]).filter(Boolean) as unknown as RecordingPath[];
  expect(strokes).toHaveLength(4);
  expect(strokes.every((p) => p.commands.some((c) => c.kind === 'cubic'))).toBe(true);
  expect(vi.mocked(ctx.clip).mock.calls.filter((c) => c[0])).toHaveLength(2);
});

test('keeps fixed held segments and white points for valid final values only', async () => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [1000, 2000], series: [[10, 20], [11, null]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(document.querySelector('.chart-marker')).toBeNull();
  const plot = plots[0];
  // The held segment starts at the last real sample and is only handed to the painter.
  expect(lastHeld()).toEqual([{ x: plot.bbox.left + plot.bbox.width, y: plot.valToPos(20, 'percent', true), color: plot.opts.series[1].stroke }]);
  expect(plot.data).toEqual([[1, 2], [10, 20], [11, null]]);
  const dots = [...document.querySelectorAll<HTMLElement>('.chart-dot')];
  expect(dots.map((d) => d.hidden)).toEqual([false, true]);
  const painted = paints().length;
  frame(1000);
  expect(paints()).toHaveLength(painted);
  expect(dots.map((d) => d.hidden)).toEqual([false, true]);
  frame(301_000);
  expect(lastHeld()).toEqual([]);
  expect(dots.every((d) => d.hidden)).toBe(true);
});

test('drops held segments and dots with a repaint once the last sample leaves the window', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const plot = plots[0];
  frame(5000);
  monotonicMs = 5000;
  // A delayed snapshot rebases at the displayed edge, 4 s after its own timestamp.
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(lastHeld()).toHaveLength(2);
  const scales = plot.scales.length;
  const dots = [...document.querySelectorAll<HTMLElement>('.chart-dot')];
  frame(300_900);
  // Still in the window, and short of overscan exhaustion: the canvas only translated.
  expect(plot.scales).toHaveLength(scales);
  expect(dots.map((d) => d.hidden)).toEqual([false, false]);
  const scrolled = 295_900 * plot.bbox.width / 300_000;
  const canvas = document.querySelector<HTMLCanvasElement>('.chart-canvas')!;
  expect(canvas.style.transform).toBe(`translateX(${-scrolled}px)`);
  // The held segment ends at the right overscan edge, so it still covers the plot's right edge.
  expect(plot.bbox.left + 2 * plot.bbox.width - scrolled).toBeGreaterThanOrEqual(plot.bbox.left + plot.bbox.width);
  frame(301_100);
  expect(plot.scales).toHaveLength(scales + 1);
  expect(lastHeld()).toEqual([]);
  expect(dots.every((d) => d.hidden)).toBe(true);
  expect(plot.setDataCalls).toBe(1);
});

test('expiry under a deferred uPlot commit repaints once instead of rebasing again', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const plot = plots[0];
  frame(5000);
  monotonicMs = 5000;
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(lastHeld()).toHaveLength(2);
  const scales = plot.scales.length;
  FakeUplot.deferDraw = true;
  try {
    frame(301_100);
    expect(plot.scales).toHaveLength(scales + 1);
    await Promise.resolve();
    expect(lastHeld()).toEqual([]);
    frame(301_120);
    expect(plot.scales).toHaveLength(scales + 1);
  } finally { FakeUplot.deferDraw = false; }
});

test('rebases delayed snapshots at the displayed edge without adding synthetic samples', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(5000);
  monotonicMs = 5000;
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].scales.at(-1)?.range.max).toBe(7);
  expect(plots[0].data[0]).toEqual([1, 2, 3]);
  expect(document.querySelector<HTMLCanvasElement>('.chart-canvas')!.style.transform).toBe('translateX(0px)');
  // The sample sits 4 s (8 px) inside the displayed edge; its held segment starts there.
  const right = plots[0].bbox.left + plots[0].bbox.width;
  expect(lastHeld().map((h) => h.x)).toEqual([right - 8, right - 8]);
});

test('replenishes X ticks after a full window of silence without per-frame redraws', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const plot = plots[0];
  const count = plot.scales.length;
  const ctx = document.querySelector<HTMLCanvasElement>('.chart-canvas')!.getContext('2d')!;
  const label = new Date(300_000).toLocaleTimeString('en', { hour: '2-digit', minute: '2-digit' });
  const before = vi.mocked(ctx.fillText).mock.calls.find(([text]) => text === label)![1];
  frame(299_000);
  expect(plot.scales).toHaveLength(count);
  vi.mocked(ctx.fillText).mockClear();
  frame(301_000);
  expect(plot.scales).toHaveLength(count + 1);
  expect(plot.yAutoDecisions.at(-1)).toEqual([false, false]);
  expect(plot.scales.at(-1)?.range.max).toBe(303);
  const after = vi.mocked(ctx.fillText).mock.calls.find(([text]) => text === label)![1];
  expect(after).toBeCloseTo(before - 301 * plot.bbox.width / 300);
  expect(document.querySelector<HTMLCanvasElement>('.chart-canvas')!.style.transform).toBe('translateX(0px)');
  frame(302_000);
  expect(plot.scales).toHaveLength(count + 1);
  expect(document.querySelector<HTMLCanvasElement>('.chart-canvas')!.style.transform).toBe('translateX(-2px)');
  expect(plot.setDataCalls).toBe(0);
});

test('paints seconds on the time axis when uPlot picks a sub-minute split', async () => {
  const labels = () => {
    const ctx = document.querySelector<HTMLCanvasElement>('.chart-canvas')!.getContext('2d')!;
    return vi.mocked(ctx.fillText).mock.calls.map(([text]) => text);
  };
  FakeUplot.xIncrement = 5;
  try {
    localStorage.setItem(WINDOW_KEY, '60');
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    const withSeconds = { hour: '2-digit', minute: '2-digit', second: '2-digit' } as const;
    expect(labels()).toContain(new Date(0).toLocaleTimeString('en', withSeconds));
    expect(labels()).toContain(new Date(5000).toLocaleTimeString('en', withSeconds));
    expect(new Set(labels()).size).toBe(labels().length);
  } finally { FakeUplot.xIncrement = 60; }
});

test('selection and range reseeds keep the already displayed edge', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(5000);
  await fireEvent.click(checkbox(labelOf(byId(LOAD))));
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(plots[1].scales.at(-1)?.range.max).toBe(7);
  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.60') }));
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(plots[2].scales.at(-1)?.range).toEqual({ min: -53, max: 7 });
});

test.each([null, NaN, Infinity])('omits the marker for a nonfinite final value %s', async (value) => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [1000, 2000], series: [[10, value], [11, 21]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(lastHeld().map((h) => h.color)).toEqual([plots[0].opts.series[2].stroke]);
  expect(document.querySelector<HTMLElement>('.chart-dot')!.hidden).toBe(true);
});

test('locale and theme rebuild cached geometry without refetching or moving the visible edge', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(5000);
  i18n.locale = 'it';
  flushSync();
  expect(plots).toHaveLength(2);
  expect(plots[1].scales.at(-1)?.range.max).toBe(7);
  document.documentElement.style.setProperty('--accent', '#123456');
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(plots[2].opts.series[1].stroke).toBe('#123456');
  expect(plots[2].scales.at(-1)?.range.max).toBe(7);
  expect(backend.historyCalls).toHaveLength(1);
});

test('hides the composited path and marker when the legend hides a series', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const configured = plots[0];
  configured.opts.series[1].show = false;
  const fixture = canvasFixture(configured.data, configured.opts.series);
  const ctx = document.querySelector<HTMLCanvasElement>('.chart-canvas')!.getContext('2d')!;
  vi.mocked(ctx.stroke).mockClear();
  configured.opts.hooks!.draw![0]!(fixture.plot);
  expect(vi.mocked(ctx.stroke).mock.calls.filter((c) => c[0])).toHaveLength(2);
  expect(lastHeld().map((h) => h.color)).toEqual([configured.opts.series[2].stroke]);
  expect([...document.querySelectorAll<HTMLElement>('.chart-dot')].map((d) => d.hidden)).toEqual([true, false]);
});

test('a singleton uses its real unit scale and DPR for the fixed endpoint', async () => {
  FakeUplot.pxRatio = 2;
  try {
    const backend = fakeBackend();
    backend.history = { timestampsMs: [2000], series: [[100]] };
    renderChart(backend, new LiveStore(), gpuSensors, [TEMP]);
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    // Canvas pixels at DPR 2: the plot's right edge, half-way up the 0-200 °C scale.
    expect(lastHeld()).toEqual([{ x: 672, y: 110, color: plots[0].opts.series[1].stroke }]);
    const dot = document.querySelector<HTMLElement>('.chart-dot')!;
    expect(dot.hidden).toBe(false);
    expect(dot.style.top).toBe('53px');
    const painted = paints().length;
    frame(1000);
    expect(paints()).toHaveLength(painted);
    expect(dot.style.top).toBe('53px');
    expect(plots[0].data).toEqual([[2], [100]]);
  } finally { FakeUplot.pxRatio = 1; }
});

test('resize rebuilds layers at the current edge and disconnects on teardown', async () => {
  let resize!: ResizeObserverCallback;
  const disconnect = vi.fn();
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: ResizeObserverCallback) { resize = callback; }
    observe() {}
    disconnect = disconnect;
  });
  const backend = fakeBackend();
  const { unmount } = renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(5000);
  resize([], {} as ResizeObserver);
  expect(plots).toHaveLength(2);
  expect(plots[0].destroyed).toBe(true);
  expect(plots[1].scales.at(-1)?.range.max).toBe(7);
  expect(document.querySelectorAll('.chart-canvas')).toHaveLength(1);
  expect(backend.historyCalls).toHaveLength(1);
  unmount();
  expect(disconnect).toHaveBeenCalledOnce();
});

test('DPR change rebuilds at the current edge and keeps canvas coordinates in device pixels', async () => {
  const listeners = new Set<() => void>();
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: false, media: query,
    addEventListener: (_type: string, listener: () => void) => { if (query.includes('resolution')) listeners.add(listener); },
    removeEventListener: (_type: string, listener: () => void) => listeners.delete(listener),
  }) as unknown as MediaQueryList);
  const backend = fakeBackend();
  const { unmount } = renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(5000);
  FakeUplot.pxRatio = 2;
  try {
    for (const listener of [...listeners]) listener();
    expect(plots).toHaveLength(2);
    expect(plots[1].scales.at(-1)?.range.max).toBe(7);
    const canvas = document.querySelector<HTMLCanvasElement>('.chart-canvas')!;
    expect(canvas.width).toBe(1344);
    expect(canvas.style.width).toBe('672px');
    expect(backend.historyCalls).toHaveLength(1);
    unmount();
    expect(listeners.size).toBe(0);
  } finally { FakeUplot.pxRatio = 1; }
});

test('a new schema reanchors to its own history instead of preserving the old edge', async () => {
  const backend = fakeBackend();
  const store = new LiveStore();
  const { rerender } = renderChart(backend, store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(5000);
  const schema = { ...MOCK_SCHEMA, revision: 2 };
  store.applySchema(schema);
  backend.getHistory = async () => ({ revision: 2, seq: 0, timestampsMs: [1000], series: [[10], [20]] });
  await rerender({ sectionId: GPU, sensors: gpuSensors, defaults: [LOAD, TEMP], schema, store, backend });
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(plots[1].scales.at(-1)?.range.max).toBe(1);
});

/**
 * A real uPlot 1.6.32 built from the options the component handed to the stub, so the
 * component's cursor callbacks run against the library's own cursor and legend code.
 * The draw hook is left out: it would repaint the component's layers from this plot.
 */
async function realCursorPlot(configured: FakeUplot) {
  const { default: RealUplot } = await vi.importActual<{ default: typeof import('uplot') }>('uplot');
  const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) }, {
    get: (target, key) => key in target ? target[key as keyof typeof target] : () => {},
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
  const target = document.createElement('div');
  document.body.append(target);
  const { draw: _draw, ...hooks } = configured.opts.hooks ?? {};
  const actual = new RealUplot({ ...configured.opts, hooks }, configured.data, target);
  // The component's Y autoscale gate is already closed once its own plot has painted.
  actual.batch(() => {
    actual.setScale('x', configured.scales.at(-1)!.range);
    actual.setScale('percent', { min: 0, max: 100 });
    actual.setScale('celsius', { min: 0, max: 200 });
  });
  await Promise.resolve();
  return { actual, dispose: () => { actual.destroy(); target.remove(); } };
}

test('real uPlot cursor selects the visible sample after scrolling and keeps the crosshair under the pointer', async () => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [1000, 2000, 3000], series: [[10, 20, 30], [40, 50, 60]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(20_000);
  const configured = plots[0];
  const offset = 40;
  expect(document.querySelector<HTMLCanvasElement>('.chart-canvas')!.style.transform).toBe('translateX(-40px)');
  const { actual, dispose } = await realCursorPlot(configured);
  try {
    // The curve of the cached scale is drawn `offset` px to the left of uPlot's X mapping.
    const pointerX = actual.valToPos(2, 'x') - offset;
    actual.setCursor({ left: pointerX, top: 30 });
    expect(actual.legend.idx).toBe(1);
    expect(actual.cursor.idx).toBe(1);
    expect(actual.cursor.idxs).toEqual([1, 1, 1]);
    expect(actual.legend.values?.[0]).toEqual({ _: new Date(2000).toLocaleTimeString('en') });
    expect(actual.legend.values?.[1]).toEqual({ _: '20%' });
    expect(actual.cursor.left).toBe(pointerX);
    expect(actual.over.querySelector<HTMLElement>('.u-cursor-x')!.style.transform).toBe(`translate(${Math.round(pointerX)}px,0px)`);
    // uPlot places the point at the cached X; the shared CSS offset moves it onto the drawn curve.
    const points = [...actual.over.querySelectorAll<HTMLElement>('.u-cursor-pt')];
    expect(points[0].style.transform.startsWith(`translate(${Math.ceil(actual.valToPos(2, 'x'))}px,`)).toBe(true);
    // Written on the hovered plot's overlay, where the points live; the scrolling frames
    // before the hover never wrote it.
    expect(actual.over.style.getPropertyValue('--chart-cursor-offset')).toBe(`${-offset}px`);
    expect(configured.over.style.getPropertyValue('--chart-cursor-offset')).toBe('');
    expect(configured.setDataCalls).toBe(0);
    expect(configured.scales).toHaveLength(1);
  } finally {
    dispose();
  }
});

test('real uPlot cursor uses the plain X mapping at offset zero after a snapshot rebase', async () => {
  const store = new LiveStore();
  const backend = fakeBackend();
  backend.history = { timestampsMs: [1000, 2000, 3000], series: [[10, 20, 30], [40, 50, 60]] };
  renderChart(backend, store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(20_000);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 4000, values: mockValues(1) });
  flushSync();
  const configured = plots[0];
  expect(document.querySelector<HTMLCanvasElement>('.chart-canvas')!.style.transform).toBe('translateX(0px)');
  expect(configured.over.style.getPropertyValue('--chart-cursor-offset')).toBe('');
  const { actual, dispose } = await realCursorPlot(configured);
  try {
    const pointerX = actual.valToPos(3, 'x');
    actual.setCursor({ left: pointerX, top: 30 });
    expect(actual.over.style.getPropertyValue('--chart-cursor-offset')).toBe('0px');
    expect(actual.legend.idx).toBe(2);
    expect(actual.legend.values?.[1]).toEqual({ _: '30%' });
    expect(actual.over.querySelector<HTMLElement>('.u-cursor-x')!.style.transform).toBe(`translate(${Math.round(pointerX)}px,0px)`);
  } finally {
    dispose();
  }
});

test('a stationary hovered pointer is hit-tested again while the canvas scrolls, never on rebase', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const plot = plots[0];
  frame(0);
  expect(plot.setCursorCalls).toEqual([]);
  plot.cursor.left = 100;
  plot.cursor.top = 30;
  frame(1000);
  expect(plot.setCursorCalls).toEqual([{ left: 100, top: 30 }]);
  frame(1000);
  expect(plot.setCursorCalls).toHaveLength(1);
  plot.cursor.left = -10;
  frame(2000);
  expect(plot.setCursorCalls).toHaveLength(1);
  // uPlot re-runs the cursor itself when a rebase commits the new X scale.
  plot.cursor.left = 100;
  const scales = plot.scales.length;
  frame(301_000);
  expect(plot.scales).toHaveLength(scales + 1);
  expect(plot.setCursorCalls).toHaveLength(1);
  expect(plot.setDataCalls).toBe(0);
});

test('fixed dots have their full radius at the right edge and both Y extrema while held lines remain clipped', async () => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [2000], series: [[100], [0]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(1000);
  expect(document.querySelector('.chart-marker-clip')).toBeNull();
  const lineClip = document.querySelector<HTMLElement>('.chart-canvas-clip')!;
  const dotClip = document.querySelector<HTMLElement>('.chart-dot-clip');
  expect(dotClip).not.toBeNull();
  const px = (v: string) => Number.parseFloat(v);
  const plot = plots[0].bbox;
  // Held lines are canvas strokes, clipped to the plot area by the canvas layer and painter.
  expect(px(lineClip.style.left)).toBe(plot.left);
  expect(px(lineClip.style.top)).toBe(plot.top);
  expect(px(lineClip.style.width)).toBe(plot.width);
  expect(lastHeld().map((h) => h.y)).toEqual([plot.top, plot.top + plot.height]);
  expect(px(dotClip!.style.left)).toBe(plot.left - 3);
  expect(px(dotClip!.style.top)).toBe(plot.top - 3);
  expect(px(dotClip!.style.width)).toBe(plot.width + 6);
  expect(px(dotClip!.style.height)).toBe(plot.height + 6);
  const dots = [...dotClip!.querySelectorAll<HTMLElement>('.chart-dot')];
  expect(dots).toHaveLength(2);
  expect(dots.map((d) => d.hidden)).toEqual([false, false]);
  expect(dots.map((d) => px(d.style.top))).toEqual([3, plot.height + 3]);
  expect(lineClip.querySelector('.chart-dot')).toBeNull();
  for (const dot of dots) {
    const centerY = px(dot.style.top);
    expect(centerY - 3).toBeGreaterThanOrEqual(0);
    expect(centerY + 3).toBeLessThanOrEqual(px(dotClip!.style.height));
  }
});

test('frames between samples only change the canvas transform', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  frame(0);
  const canvas = document.querySelector<HTMLCanvasElement>('.chart-canvas')!;
  const records: MutationRecord[] = [];
  const observer = new MutationObserver((batch) => records.push(...batch));
  observer.observe(plots[0].root, { attributes: true, subtree: true, childList: true, characterData: true });
  const before = canvas.style.cssText;
  const painted = paints().length;
  for (const at of [16, 33, 50, 1000, 2000]) frame(at);
  await Promise.resolve();
  records.push(...observer.takeRecords());
  observer.disconnect();
  expect(records.length).toBeGreaterThan(0);
  for (const record of records) {
    expect(record.type).toBe('attributes');
    expect(record.target).toBe(canvas);
    expect(record.attributeName).toBe('style');
  }
  const strip = (css: string) => css.replace(/transform:[^;]*;?/, '').trim();
  expect(strip(canvas.style.cssText)).toBe(strip(before));
  expect(paints()).toHaveLength(painted);
});

test('the cursor offset is written only while hovering and reset once on leave', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const plot = plots[0];
  const set = vi.spyOn(plot.over.style, 'setProperty');
  const remove = vi.spyOn(plot.over.style, 'removeProperty');
  const offsetVar = () => plot.over.style.getPropertyValue('--chart-cursor-offset');
  for (const at of [0, 16, 33, 500]) frame(at);
  expect(set).not.toHaveBeenCalled();
  expect(remove).not.toHaveBeenCalled();
  expect(offsetVar()).toBe('');
  plot.cursor.left = 100;
  plot.cursor.top = 30;
  frame(1000);
  expect(offsetVar()).toBe('-2px');
  frame(1000);
  expect(set).toHaveBeenCalledTimes(1);
  frame(1500);
  expect(offsetVar()).toBe('-3px');
  expect(set).toHaveBeenCalledTimes(2);
  plot.cursor.left = -10;
  frame(2000);
  expect(offsetVar()).toBe('');
  frame(2500);
  frame(3000);
  expect(set).toHaveBeenCalledTimes(2);
  expect(remove).toHaveBeenCalledTimes(1);
});

/** Arial advance widths in em (digits 0.556), so a label measures as it would on Windows at 12px. */
const ARIAL_EM: Record<string, number> = { ':': 0.278, ' ': 0.278, '\u202f': 0.278, A: 0.667, P: 0.667, M: 0.833 };
const arialWidth = (text: string, px = 12) => [...text].reduce((sum, c) => sum + (ARIAL_EM[c] ?? 0.556), 0) * px;

/**
 * The X axis of a real uPlot 1.6.32 built from the component's options at a given plot width,
 * so uPlot's own increment table and findIncr choose the split. Labels measure as Arial.
 */
async function realTimeAxis(configured: FakeUplot, plotWidthPx: number, windowSeconds: number) {
  const { default: RealUplot } = await vi.importActual<{ default: typeof import('uplot') }>('uplot');
  const ctx = new Proxy({
    font: '12px sans-serif',
    measureText(text: string) { return { width: arialWidth(text, Number.parseFloat(this.font)) }; },
  }, { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
  const target = document.createElement('div');
  document.body.append(target);
  const { draw: _draw, ...hooks } = configured.opts.hooks ?? {};
  // Both Y axes are 72 px wide; the rest of the width is the plot.
  const actual = new RealUplot({ ...configured.opts, width: plotWidthPx + 144, hooks }, configured.data, target);
  const end = Date.UTC(2026, 8, 27, 21, 9, 3) / 1000;
  actual.batch(() => {
    actual.setScale('x', { min: end - windowSeconds, max: end });
    actual.setScale('percent', { min: 0, max: 100 });
    actual.setScale('celsius', { min: 0, max: 200 });
  });
  await Promise.resolve();
  await Promise.resolve();
  const axis = actual.axes[0] as unknown as { _found: [number, number]; _splits: number[] };
  const [incr, spacing] = axis._found;
  const result = { plotWidth: actual.bbox.width, incr, spacing, splits: [...axis._splits] };
  actual.destroy();
  target.remove();
  return result;
}

describe.each(['en', 'it'] as const)('X labels in %s', (locale) => {
  test.each([60, 300, 1800, 3600])('never overlap in a %i s window at 320, 651 and 1200 px', async (seconds) => {
    i18n.locale = locale;
    localStorage.setItem(WINDOW_KEY, String(seconds));
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    for (const width of [320, 651, 1200]) {
      const axis = await realTimeAxis(plots[0], width, seconds);
      expect(axis.plotWidth).toBe(width);
      expect(axis.incr).toBeGreaterThan(0);
      const labels = axis.splits.map((split) => formatTimeTick(split, locale, axis.incr));
      expect(axis.spacing).toBeGreaterThanOrEqual(Math.max(...labels.map((label) => arialWidth(label))) + TIME_LABEL_GAP_PX);
      expect(new Set(labels).size).toBe(labels.length);
    }
  });
});

test('a 1 min window at 651 px shows seconds on splits wide enough for them in en and it', async () => {
  for (const [locale, sample] of [['en', '12:58:05 AM'], ['it', '21:09:05']] as const) {
    cleanup();
    plots.length = 0;
    i18n.locale = locale;
    localStorage.setItem(WINDOW_KEY, '60');
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    const axis = await realTimeAxis(plots[0], 651, 60);
    // uPlot's default 50 px would pick 5 s splits, about 54 px apart.
    expect(axis.incr).toBeLessThan(60);
    expect(axis.spacing).toBeGreaterThanOrEqual(arialWidth(sample) + TIME_LABEL_GAP_PX);
    expect(formatTimeTick(axis.splits[0], locale, axis.incr)).toMatch(/:\d{2}:\d{2}/);
  }
});
