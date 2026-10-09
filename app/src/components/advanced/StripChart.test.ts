import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import { loadSeries } from '../../lib/advanced/persist';
import type { HistorySeed, Sensor } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { spyChartFrames } from '../../test/chart-frames';
import { connectSettings, disconnectSettings } from '../../test/settings';
import { FakeUplot } from '../../test/uplot-stub';
import { canvasFixture, RecordingPath } from '../../test/uplot-canvas';
import { drawChartCanvas, type ChartHeldSegment } from '../../lib/advanced/chartCanvas';
import { ChartBuffer, formatTimeTick, scaleOptions, TIME_LABEL_GAP_PX } from '../../lib/advanced/chartData';
import { formatValue } from '../../lib/format';
import type uPlot from 'uplot';
import StripChart from './StripChart.svelte';
import legendSource from './StripLegend.svelte?raw';
import drawerSource from './SeriesDrawer.svelte?raw';

// Calls through to the real painter; tests read the held segments it was handed.
vi.mock('../../lib/advanced/chartCanvas', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../lib/advanced/chartCanvas')>();
  return { ...actual, drawChartCanvas: vi.fn(actual.drawChartCanvas) };
});
const paints = () => vi.mocked(drawChartCanvas).mock.calls;
/** The composited canvas of every strip on screen, in strip order. */
const canvases = () => [...document.querySelectorAll<HTMLCanvasElement>('.chart-canvas')];
/** Paints of one strip, told apart by the context of its canvas. */
const paintsOf = (canvas: HTMLCanvasElement) => paints().filter((call) => call[0] === canvas.getContext('2d'));
/**
 * Held segments of the latest paint of every strip on screen, in canvas pixels and strip
 * order: the series of all strips, as one plot used to hand them over.
 */
const lastHeld = (): ReadonlyArray<ChartHeldSegment> => canvases().flatMap((canvas) => paintsOf(canvas).at(-1)?.[8] ?? []);

const plots = FakeUplot.instances;
/** Plots of the strips on screen, in strip order. */
const live = () => plots.filter((p) => !p.destroyed);
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
const seriesButton = () => screen.getByRole('button', { name: new RegExp(`^${t('advanced.chart.series')} · \\d+$`) });
const drawer = () => screen.queryByRole('dialog', { name: t('advanced.chart.series') });
/** Opens the «Series» drawer when it is closed; its checkboxes exist only while it is open. */
function openDrawer() {
  if (drawer()) return;
  void fireEvent.click(seriesButton());
  flushSync();
}
const checkbox = (label: string) => {
  openDrawer();
  return screen.getByLabelText(label) as HTMLInputElement;
};
/** The legend row of a series, found by its name button. */
const legendRow = (s: Sensor) => screen.getByRole('button', { name: labelOf(s) }).closest<HTMLElement>('.row')!;
const legendCell = (s: Sensor, cell: 'now' | 'min' | 'max') => legendRow(s).querySelector(`.${cell}`)!.textContent;
/** Title of a unit, «Name · symbol», or the name alone for units whose values pick their own prefix. */
const legendTitle = (unit: string, symbol: string | null = null) => {
  const name = t(`advanced.chart.unit.${unit}`);
  return symbol === null ? name : `${name} · ${symbol}`;
};
/** Matches a legend title by its whole text (the symbol has its own span). */
const titled = (text: string) => (_: string, el: Element | null) => !!el?.matches('.legend .title') && el.textContent === text;
/** Matches the legend title of a unit whatever its symbol. */
const named = (unit: string) => (_: string, el: Element | null) =>
  !!el?.matches('.legend .title') && new RegExp(`^${t(`advanced.chart.unit.${unit}`)}( · |$)`).test(el.textContent ?? '');
const removeButton = (s: Sensor) => screen.getByRole('button', { name: t('advanced.chart.remove', { name: labelOf(s) }) });

/** Runs a strip's cursor hook as uPlot does with the pointer at sample `idx` (null: it left). */
function hover(plot: FakeUplot, idx: number | null) {
  const u = { cursor: { left: idx === null ? -10 : 100, top: 30, idxs: plot.data.map(() => idx) }, legend: {}, over: plot.over };
  for (const hook of plot.opts.hooks?.setCursor ?? []) hook?.(u as unknown as uPlot);
  flushSync();
}

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

beforeEach(async () => {
  vi.stubGlobal('Path2D', RecordingPath);
  const contexts = new WeakMap<HTMLCanvasElement, CanvasRenderingContext2D>();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    if (!contexts.has(this)) contexts.set(this, {
      ...canvasFixture([[0], [1]]).ctx, clearRect: vi.fn(), translate: vi.fn(), setTransform: vi.fn(),
    } as unknown as CanvasRenderingContext2D);
    return contexts.get(this)!;
  });
  plots.length = 0;
  FakeUplot.autoRangeY = false;
  vi.mocked(drawChartCanvas).mockClear();
  await connectSettings();
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
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); disconnectSettings(); });

/** FakeBackend with two history samples (1 s and 2 s): column i holds [10 + i, 20 + i]. */
function fakeBackend(): FakeBackend {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.history = { timestampsMs: [1000, 2000], series: Array.from({ length: 8 }, (_, i) => [10 + i, 20 + i]) };
  return backend;
}

function renderChart(backend: FakeBackend, store = new LiveStore(), sensors = gpuSensors, defaults = [LOAD, TEMP], sectionId = GPU) {
  if (!store.schema) store.applySchema(MOCK_SCHEMA);
  return render(StripChart, { sectionId, sensors, defaults, schema: MOCK_SCHEMA, store, backend });
}

test('one_plot_per_unit_with_its_own_y_axis', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  // One history request for the series of both strips.
  expect(backend.historyCalls).toEqual([{ ids: [LOAD, TEMP], seconds: 300, maxPoints: undefined }]);
  const [percent, celsius] = plots;
  expect(percent.opts.series.slice(1).map((s) => s.label)).toEqual([labelOf(byId(LOAD))]);
  expect(celsius.opts.series.slice(1).map((s) => s.label)).toEqual([labelOf(byId(TEMP))]);
  expect(percent.opts.series.slice(1).map((s) => s.scale)).toEqual(['percent']);
  expect(celsius.opts.series.slice(1).map((s) => s.scale)).toEqual(['celsius']);
  expect(percent.opts.axes?.map((a) => [a.scale, a.side])).toEqual([[undefined, undefined], ['percent', 3]]);
  expect(celsius.opts.axes?.map((a) => [a.scale, a.side])).toEqual([[undefined, undefined], ['celsius', 3]]);
  expect(percent.data).toEqual([[1, 2], [10, 20]]);
  expect(celsius.data).toEqual([[1, 2], [11, 21]]);
  expect(screen.getByRole('button', { name: t('advanced.chart.window.300') }).getAttribute('aria-pressed')).toBe('true');

  for (const plot of plots) {
    expect(plot.opts.axes?.[0].values).toEqual([]);
    expect(plot.opts.axes?.[0].grid?.show).toBe(false);
    expect(plot.opts.axes?.[0].ticks?.show).toBe(false);
    // Each strip's own Y axis, on the left, follows its grid and keeps its ticks.
    expect(plot.opts.axes?.[1].ticks?.show).toBe(true);
    expect(plot.opts.axes?.[1].grid?.show).toBe(true);
    expect(plot.opts.axes?.[1].size).toBe(72);
    // Our legend replaces uPlot's.
    expect(plot.opts.legend?.show).toBe(false);
  }
  expect(screen.getByText(titled(legendTitle('percent', '%')))).toBeTruthy();
  expect(screen.getByText(titled(legendTitle('celsius', '°C')))).toBeTruthy();
});

test('long windows ask for decimated history and the choice persists', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.3600') }));
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  expect(backend.historyCalls.at(-1)).toEqual({ ids: [LOAD, TEMP], seconds: 3600, maxPoints: 900 });
  await vi.waitFor(() => expect(settings.state?.settings.advanced.window).toBe(3600));
  expect(plots[0].destroyed).toBe(true);
  expect(plots[1].destroyed).toBe(true);

  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.60') }));
  await vi.waitFor(() => expect(plots).toHaveLength(6));
  expect(backend.historyCalls.at(-1)).toEqual({ ids: [LOAD, TEMP], seconds: 60, maxPoints: undefined });
});

test('the saved window is used on mount', async () => {
  await settings.update({ advanced: { window: 1800 } });
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(backend.historyCalls).toEqual([{ ids: [LOAD, TEMP], seconds: 1800, maxPoints: 900 }]);
});

test('live snapshots extend the chart and retain one point outside the left edge', async () => {
  await settings.update({ advanced: { window: 60 } });
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 62_000, values: mockValues(1) });
  flushSync();
  const [percent, celsius] = plots;
  expect(percent.setDataCalls).toBe(1);
  expect(celsius.setDataCalls).toBe(1);
  // 1000 ms precedes the visible edge; uPlot needs it to draw through that edge.
  expect(percent.data).toEqual([[1, 2, 62], [10, 20, mockValues(1)[index(LOAD)]]]);
  expect(celsius.data).toEqual([[1, 2, 62], [11, 21, mockValues(1)[index(TEMP)]]]);
});

test('a suspended reading leaves a gap in the plotted data', async () => {
  await settings.update({ advanced: { window: 60 } });
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  const quality = (code: number) => MOCK_SCHEMA.sensors.map((_, i) => (i === index(TEMP) ? code : 0));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3_000, values: mockValues(1), quality: quality(2) });
  flushSync();
  store.applySnapshot({ revision: 1, seq: 2, timestampMs: 4_000, values: mockValues(2), quality: quality(0) });
  flushSync();
  expect(plots[0].data).toEqual([[1, 2, 3, 4], [10, 20, mockValues(1)[index(LOAD)], mockValues(2)[index(LOAD)]]]);
  expect(plots[1].data).toEqual([[1, 2, 3, 4], [11, 21, null, mockValues(2)[index(TEMP)]]]);
});

test('a fully suspended window keeps a y range and says why it is empty', async () => {
  await settings.update({ advanced: { window: 60 } });
  FakeUplot.autoRangeY = true;
  const backend = fakeBackend();
  backend.history = { timestampsMs: [], series: [] };
  const store = new LiveStore();
  renderChart(backend, store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(screen.queryByText(t('chart.suspended'))).toBeNull();

  const quality = MOCK_SCHEMA.sensors.map(() => 2);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 100_000, values: mockValues(1), quality });
  flushSync();
  expect(plots[0].data.slice(1)).toEqual([[null]]);
  expect(plots[1].data.slice(1)).toEqual([[null]]);
  for (const [plot, scale] of [[plots[0], 'percent'], [plots[1], 'celsius']] as const) {
    const range = plot.yRanges.get(scale)!;
    expect(Number.isFinite(range.min)).toBe(true);
    expect(Number.isFinite(range.max)).toBe(true);
  }
  // Each strip says so over its own lines.
  expect(screen.getAllByText(t('chart.suspended'))).toHaveLength(2);

  // The first fresh reading brings the plot back.
  const fresh = MOCK_SCHEMA.sensors.map(() => 0);
  store.applySnapshot({ revision: 1, seq: 2, timestampMs: 101_000, values: mockValues(2), quality: fresh });
  flushSync();
  expect(screen.queryByText(t('chart.suspended'))).toBeNull();
});

test('a partly suspended window shows no suspended notice', async () => {
  await settings.update({ advanced: { window: 60 } });
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  const quality = MOCK_SCHEMA.sensors.map((_, i) => (i === index(TEMP) ? 2 : 0));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3_000, values: mockValues(1), quality });
  flushSync();
  expect(screen.queryByText(t('chart.suspended'))).toBeNull();

  // Every series suspended, but older readings still sit in the visible window.
  const all = MOCK_SCHEMA.sensors.map(() => 2);
  store.applySnapshot({ revision: 1, seq: 2, timestampMs: 4_000, values: mockValues(2), quality: all });
  flushSync();
  expect(screen.queryByText(t('chart.suspended'))).toBeNull();
});

test('a suspended snapshot that arrives while history loads is plotted as a gap', async () => {
  const backend = fakeBackend();
  let resolve!: (h: HistorySeed) => void;
  backend.getHistory = () => new Promise((done) => (resolve = done));
  const store = new LiveStore();
  renderChart(backend, store);
  await vi.waitFor(() => expect(resolve).toBeDefined());

  const quality = MOCK_SCHEMA.sensors.map((_, i) => (i === index(TEMP) ? 2 : 0));
  store.applySnapshot({ revision: 1, seq: 5, timestampMs: 3000, values: mockValues(5), quality });
  flushSync();
  resolve({ revision: 1, seq: 4, timestampsMs: [1000, 2000], series: [[1, 2], [3, 4]] });
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(plots[0].data).toEqual([[1, 2, 3], [1, 2, mockValues(5)[index(LOAD)]]]);
  expect(plots[1].data).toEqual([[1, 2, 3], [3, 4, null]]);
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(plots[0].data[0]).toEqual([1, 2, 3]);
  expect(plots[1].data[0]).toEqual([1, 2, 3]);
});

test('the_drawer_has_no_series_limit_and_saves_per_section', async () => {
  const backend = fakeBackend();
  renderChart(backend, new LiveStore(), [...cpuSensors, ...gpuSensors], ['cpu/0/load/total', 'cpu/0/clock/effective'], 'cpu/0');
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  const thread = (i: number) => checkbox(t('sensor.cpu.load.thread', { arg: i }));
  for (let i = 0; i < 8; i++) await fireEvent.click(thread(i));
  await fireEvent.click(checkbox(labelOf(byId(TEMP))));
  await fireEvent.click(checkbox(labelOf(byId(VRAM))));
  expect(screen.getAllByRole<HTMLInputElement>('checkbox').every((box) => !box.disabled)).toBe(true);
  expect(screen.getByText(`${t('advanced.chart.series')} · 12`)).toBeTruthy();
  expect(screen.queryByText(t('advanced.chart.maxSeries'))).toBeNull();
  await vi.waitFor(() => expect(backend.historyCalls.at(-1)?.ids).toHaveLength(12));
  await vi.waitFor(() => expect(settings.state?.settings.advanced.series['cpu/0']).toHaveLength(12));
  // Four units, four strips; the nine load series share one.
  await vi.waitFor(() => expect(live().map((p) => p.opts.series[1].scale)).toEqual(['percent', 'megahertz', 'celsius', 'bytes']));
  expect(live()[0].opts.series).toHaveLength(10);
});

test('a_third_unit_adds_a_third_strip', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  expect(checkbox(labelOf(byId(VRAM))).disabled).toBe(false);
  await fireEvent.click(checkbox(labelOf(byId(VRAM))));
  await vi.waitFor(() => expect(plots).toHaveLength(5));
  expect(live().map((p) => p.opts.series[1].scale)).toEqual(['percent', 'celsius', 'bytes']);
  expect(document.querySelectorAll('.strip')).toHaveLength(3);
  expect(within(document.querySelectorAll<HTMLElement>('.strip')[2]).getByText(titled(legendTitle('bytes')))).toBeTruthy();
  expect(checkbox(labelOf(byId(VRAM))).checked).toBe(true);
});

test('the saved selection of the section wins over the defaults', async () => {
  await settings.update({ advanced: { series: { [GPU]: [VRAM, 'gone'] } } });
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(backend.historyCalls[0].ids).toEqual([VRAM]);
});

test('a saved sensor that appears after mount joins the chart, without saving over the selection', async () => {
  await settings.update({ advanced: { series: { [GPU]: [LOAD, VRAM] } } });
  const backend = fakeBackend();
  const early = gpuSensors.filter((s) => s.id !== VRAM);
  const { rerender } = renderChart(backend, new LiveStore(), early);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(backend.historyCalls[0].ids).toEqual([LOAD]);
  await rerender({ sectionId: GPU, sensors: gpuSensors, defaults: [LOAD, TEMP], schema: MOCK_SCHEMA, store: new LiveStore(), backend });
  await vi.waitFor(() => expect(backend.historyCalls.at(-1)?.ids).toEqual([LOAD, VRAM]));
  expect(loadSeries(GPU)).toEqual([LOAD, VRAM]);
});

test('an empty selection shows a hint and fetches nothing', async () => {
  await settings.update({ advanced: { series: { [GPU]: [] } } });
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(screen.getByText(t('advanced.chart.empty'))).toBeTruthy());
  expect(backend.historyCalls).toEqual([]);
  expect(plots).toHaveLength(0);
});

test('asks for no frames without a plot: empty selection or history still loading', async () => {
  await settings.update({ advanced: { series: { [GPU]: [] } } });
  const backend = fakeBackend();
  let resolve!: (h: HistorySeed) => void;
  backend.getHistory = () => new Promise((done) => (resolve = done));
  renderChart(backend);
  await vi.waitFor(() => expect(screen.getByText(t('advanced.chart.empty'))).toBeTruthy());
  expect(frames.size).toBe(0);
  await fireEvent.click(checkbox(labelOf(byId(LOAD))));
  await vi.waitFor(() => expect(resolve).toBeDefined());
  expect(frames.size).toBe(0);
  resolve({ revision: 1, seq: 0, timestampsMs: [1000, 2000], series: [[10, 20]] });
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(frames.size).toBe(1);
  frame(1000);
  expect(frames.size).toBe(1);
  await fireEvent.click(checkbox(labelOf(byId(LOAD))));
  await vi.waitFor(() => expect(plots[0].destroyed).toBe(true));
  expect(frames.size).toBe(0);
});

test('rendering pauses while hidden and history is reloaded when visible', async () => {
  const backend = fakeBackend();
  const store = new LiveStore();
  renderChart(backend, store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  setVisibility('hidden');
  flushSync();
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].setDataCalls).toBe(0);
  expect(plots[1].setDataCalls).toBe(0);
  expect(backend.historyCalls).toHaveLength(1);

  setVisibility('visible');
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  expect(backend.historyCalls).toHaveLength(2);
});

test('series colours come from the theme tokens', async () => {
  document.documentElement.style.setProperty('--accent', '#ff4fd8');
  document.documentElement.style.setProperty('--accent-2', '#4cc9f0');
  try {
    // The palette restarts in every strip: two series of one unit take the first two colours.
    renderChart(fakeBackend(), new LiveStore(), gpuSensors, [TEMP, HOTSPOT]);
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 10_000, values: mockValues(1) });
  flushSync();
  store.applySnapshot({ revision: 1, seq: 2, timestampMs: 5000, values: mockValues(2) });
  flushSync();
  expect(plots[0].data[0]).toEqual([5]);
  expect(plots[0].data[1]).toEqual([mockValues(2)[index(LOAD)]]);
  expect(plots[1].data).toEqual([[5], [mockValues(2)[index(TEMP)]]]);
});

test('unmounting destroys the plot', async () => {
  const { unmount } = renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  unmount();
  expect(plots[0].destroyed).toBe(true);
  expect(plots[1].destroyed).toBe(true);
});

test('frames translate the shared series and X canvas without rebuilding uPlot or replacing data', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const plot = plots[0];
  frame(0);
  const count = plot.scales.length;
  setVisibility('hidden');
  expect(frames.size).toBe(0);
  expect(plot.scales).toHaveLength(count);
  setVisibility('visible');
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  const resumed = plots[2];
  const resumedCount = resumed.scales.length;
  unmount();
  expect(frames.size).toBe(0);
  expect(resumed.scales).toHaveLength(resumedCount);
});

test('resuming visibility does not catch up hidden time in one frame', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(0);
  setVisibility('hidden');
  monotonicMs = 20_000;
  setVisibility('visible');
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  const plot = plots[2];
  frame(20_000);
  expect(plot.scales.at(-1)?.range.max).toBeCloseTo(2, 3);
  expect(plots[3].scales.at(-1)?.range.max).toBeCloseTo(2, 3);
});

test('reduced motion shows new snapshots without animation frames', async () => {
  reducedMotion = true;
  for (const listener of motionListeners) listener();
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(frames.size).toBe(0);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].data[0]).toEqual([1, 2, 3]);
  expect(plots[0].scales.at(-1)).toEqual({ key: 'x', range: { min: -297, max: 3 } });
  expect(plots[1].scales.at(-1)).toEqual({ key: 'x', range: { min: -297, max: 3 } });
});

test('reduced-motion pause keeps snapshots visible and resumes without replaying elapsed time', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const strokes: RecordingPath[] = [];
  let clips = 0;
  // One series per strip: each strip's own canvas strokes its own captured path.
  for (const [configured, canvas] of plots.map((p, i) => [p, canvases()[i]] as const)) {
    const fixture = canvasFixture(configured.data, configured.opts.series);
    const ctx = canvas.getContext('2d')!;
    vi.mocked(ctx.stroke).mockClear();
    vi.mocked(ctx.clip).mockClear();
    expect(configured.opts.series[1].paths!(fixture.plot, 1, 0, 4)).toBeNull();
    configured.opts.hooks!.draw![0]!(fixture.plot);
    strokes.push(...(vi.mocked(ctx.stroke).mock.calls.map((c) => c[0]).filter(Boolean) as unknown as RecordingPath[]));
    clips += vi.mocked(ctx.clip).mock.calls.filter((c) => c[0]).length;
  }
  expect(strokes).toHaveLength(4);
  expect(strokes.every((p) => p.commands.some((c) => c.kind === 'cubic'))).toBe(true);
  expect(clips).toBe(2);
});

test('keeps fixed held segments and white points for valid final values only', async () => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [1000, 2000], series: [[10, 20], [11, null]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(document.querySelector('.chart-marker')).toBeNull();
  const plot = plots[0];
  // The held segment starts at the last real sample and is only handed to the painter.
  expect(lastHeld()).toEqual([{ x: plot.bbox.left + plot.bbox.width, y: plot.valToPos(20, 'percent', true), color: plot.opts.series[1].stroke }]);
  expect(plot.data).toEqual([[1, 2], [10, 20]]);
  expect(plots[1].data).toEqual([[1, 2], [11, null]]);
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
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
  expect(plots[1].setDataCalls).toBe(1);
});

test('expiry under a deferred uPlot commit repaints once instead of rebasing again', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(5000);
  monotonicMs = 5000;
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].scales.at(-1)?.range.max).toBe(7);
  expect(plots[1].scales.at(-1)?.range.max).toBe(7);
  expect(plots[0].data[0]).toEqual([1, 2, 3]);
  expect(canvases().map((c) => c.style.transform)).toEqual(['translateX(0px)', 'translateX(0px)']);
  // The sample sits 4 s (8 px) inside the displayed edge; its held segment starts there.
  const right = plots[0].bbox.left + plots[0].bbox.width;
  expect(lastHeld().map((h) => h.x)).toEqual([right - 8, right - 8]);
});

test('replenishes X ticks after a full window of silence without per-frame redraws', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  // The last strip carries the time labels.
  const plot = plots[1];
  const canvas = canvases()[1];
  const count = plot.scales.length;
  const ctx = canvas.getContext('2d')!;
  const label = new Date(300_000).toLocaleTimeString('en', { hour: '2-digit', minute: '2-digit' });
  const before = vi.mocked(ctx.fillText).mock.calls.find(([text]) => text === label)![1];
  frame(299_000);
  expect(plot.scales).toHaveLength(count);
  vi.mocked(ctx.fillText).mockClear();
  frame(301_000);
  expect(plot.scales).toHaveLength(count + 1);
  expect(plot.yAutoDecisions.at(-1)).toEqual([false]);
  expect(plot.scales.at(-1)?.range.max).toBe(303);
  const after = vi.mocked(ctx.fillText).mock.calls.find(([text]) => text === label)![1];
  expect(after).toBeCloseTo(before - 301 * plot.bbox.width / 300);
  expect(canvas.style.transform).toBe('translateX(0px)');
  frame(302_000);
  expect(plot.scales).toHaveLength(count + 1);
  expect(canvas.style.transform).toBe('translateX(-2px)');
  expect(plot.setDataCalls).toBe(0);
});

test('paints seconds on the time axis when uPlot picks a sub-minute split', async () => {
  const labels = () => {
    const ctx = canvases().at(-1)!.getContext('2d')!;
    return vi.mocked(ctx.fillText).mock.calls.map(([text]) => text);
  };
  FakeUplot.xIncrement = 5;
  try {
    await settings.update({ advanced: { window: 60 } });
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    const withSeconds = { hour: '2-digit', minute: '2-digit', second: '2-digit' } as const;
    expect(labels()).toContain(new Date(0).toLocaleTimeString('en', withSeconds));
    expect(labels()).toContain(new Date(5000).toLocaleTimeString('en', withSeconds));
    expect(new Set(labels()).size).toBe(labels().length);
  } finally { FakeUplot.xIncrement = 60; }
});

test('selection and range reseeds keep the already displayed edge', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(5000);
  await fireEvent.click(checkbox(labelOf(byId(LOAD))));
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(plots[2].scales.at(-1)?.range.max).toBe(7);
  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.60') }));
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  expect(plots[3].scales.at(-1)?.range).toEqual({ min: -53, max: 7 });
});

test.each([null, NaN, Infinity])('omits the marker for a nonfinite final value %s', async (value) => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [1000, 2000], series: [[10, value], [11, 21]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(lastHeld().map((h) => h.color)).toEqual([plots[1].opts.series[1].stroke]);
  expect(document.querySelector<HTMLElement>('.chart-dot')!.hidden).toBe(true);
});

test('locale and theme rebuild cached geometry without refetching or moving the visible edge', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(5000);
  i18n.locale = 'it';
  flushSync();
  expect(plots).toHaveLength(4);
  expect(plots[2].scales.at(-1)?.range.max).toBe(7);
  expect(plots[3].scales.at(-1)?.range.max).toBe(7);
  document.documentElement.style.setProperty('--accent', '#123456');
  await vi.waitFor(() => expect(plots).toHaveLength(6));
  expect(plots[4].opts.series[1].stroke).toBe('#123456');
  expect(plots[5].opts.series[1].stroke).toBe('#123456');
  expect(plots[4].scales.at(-1)?.range.max).toBe(7);
  expect(backend.historyCalls).toHaveLength(1);
});

test('hides the composited path and marker when the legend hides a series', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const name = screen.getByRole('button', { name: labelOf(byId(LOAD)) });
  expect(name.getAttribute('aria-pressed')).toBe('true');
  await fireEvent.click(name);
  expect(name.getAttribute('aria-pressed')).toBe('false');
  const configured = plots[0];
  expect(configured.series[1].show).toBe(false);
  const fixture = canvasFixture(configured.data, configured.opts.series);
  const ctx = canvases()[0].getContext('2d')!;
  vi.mocked(ctx.stroke).mockClear();
  configured.opts.hooks!.draw![0]!(fixture.plot);
  // The load strip strokes no path; the temperature strip keeps its own.
  expect(vi.mocked(ctx.stroke).mock.calls.filter((c) => c[0])).toHaveLength(0);
  expect(paintsOf(canvases()[1]).at(-1)![2].filter((p) => p.stroke)).toHaveLength(1);
  expect(lastHeld().map((h) => h.color)).toEqual([plots[1].opts.series[1].stroke]);
  expect([...document.querySelectorAll<HTMLElement>('.chart-dot')].map((d) => d.hidden)).toEqual([true, false]);
  // The row stays, dimmed, with its values.
  expect(legendRow(byId(LOAD)).classList.contains('off')).toBe(true);
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

/**
 * Replaces ResizeObserver; every strip observes its own `.plot`, and the returned function
 * reports a new content width for all of them, as the chart's one column of lines resizes.
 */
function stubResizeObserver() {
  const callbacks: ResizeObserverCallback[] = [];
  const disconnect = vi.fn();
  vi.stubGlobal('ResizeObserver', class {
    constructor(cb: ResizeObserverCallback) { callbacks.push(cb); }
    observe() {}
    disconnect = disconnect;
  });
  const resize = (width: number) => {
    for (const callback of callbacks) callback([{ contentRect: { width } } as ResizeObserverEntry], {} as ResizeObserver);
  };
  return { resize, disconnect };
}

test('resize sizes the same plot in place on the current time base and disconnects on teardown', async () => {
  const { resize, disconnect } = stubResizeObserver();
  const backend = fakeBackend();
  const { unmount } = renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const plot = plots[0];
  frame(5000);
  const scales = plot.scales.length;
  const painted = paints().length;
  resize(1000);
  expect(plots).toHaveLength(2);
  expect(plot.destroyed).toBe(false);
  // Stacked strips: 130 px of lines, and the last one 40 px more for the time labels.
  expect(plot.sizes).toEqual([{ width: 1000, height: 130 }]);
  expect(plots[1].sizes).toEqual([{ width: 1000, height: 170 }]);
  // The draw hook repainted the layers at the new width, without committing a new X scale.
  expect(paints().length).toBeGreaterThan(painted);
  expect(plot.scales).toHaveLength(scales);
  expect(plot.bbox.width).toBe(800);
  const canvas = document.querySelector<HTMLCanvasElement>('.chart-canvas')!;
  expect(canvas.style.width).toBe(`${800 * 2 + 72}px`);
  // 5 s after the painted edge, now at 800 px per 300 s: no jump in time.
  expect(canvas.style.transform).toBe(`translateX(${-5 * 800 / 300}px)`);
  frame(6000);
  expect(canvas.style.transform).toBe(`translateX(${-6 * 800 / 300}px)`);
  expect(canvases()[1].style.transform).toBe(`translateX(${-6 * 800 / 300}px)`);
  expect(document.querySelectorAll('.chart-canvas')).toHaveLength(2);
  expect(backend.historyCalls).toHaveLength(1);
  unmount();
  // One observer per strip.
  expect(disconnect).toHaveBeenCalledTimes(2);
});

test('repaints keep the canvas backing store while its size is unchanged, and clear it instead', async () => {
  const { resize } = stubResizeObserver();
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  // The last strip, whose canvas also holds the time labels under the lines.
  const canvas = canvases()[1];
  const ctx = canvas.getContext('2d')!;
  expect([canvas.width, canvas.height]).toEqual([600 * 2 + 72, 200 + 40]);
  const setWidth = vi.spyOn(HTMLCanvasElement.prototype, 'width', 'set');
  const setHeight = vi.spyOn(HTMLCanvasElement.prototype, 'height', 'set');
  vi.mocked(ctx.clearRect).mockClear();
  const painted = paints().length;
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(paints().length).toBeGreaterThan(painted);
  expect(setWidth).not.toHaveBeenCalled();
  expect(setHeight).not.toHaveBeenCalled();
  expect(ctx.setTransform).toHaveBeenCalledWith(1, 0, 0, 1, 0, 0);
  expect(ctx.clearRect).toHaveBeenCalledWith(0, 0, canvas.width, canvas.height);
  // A new plot width does resize it.
  resize(1000);
  expect(setWidth).toHaveBeenCalledWith(800 * 2 + 72);
  expect(setHeight).not.toHaveBeenCalled();
});

test('a series hidden in the legend stays hidden after a resize', async () => {
  const { resize } = stubResizeObserver();
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const plot = plots[0];
  await fireEvent.click(screen.getByRole('button', { name: labelOf(byId(LOAD)) }));
  resize(1000);
  expect(plots).toHaveLength(2);
  expect(plot.series[1].show).toBe(false);
  expect(lastHeld().map((h) => h.color)).toEqual([plots[1].opts.series[1].stroke]);
  expect([...document.querySelectorAll<HTMLElement>('.chart-dot')].map((d) => d.hidden)).toEqual([true, false]);
});

test('a height-only change of the plot box, as the legend wraps or a reseed refills it, neither resizes nor rebuilds', async () => {
  const { resize } = stubResizeObserver();
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  // jsdom lays nothing out, so the plot was built 800 px wide.
  const painted = paints().length;
  resize(800);
  expect(plots).toHaveLength(2);
  expect(plots[0].sizes).toEqual([]);
  expect(plots[1].sizes).toEqual([]);
  expect(paints()).toHaveLength(painted);
  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.60') }));
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  resize(800);
  expect(plots).toHaveLength(4);
  expect(plots[2].sizes).toEqual([]);
  expect(plots[3].sizes).toEqual([]);
});

test('series hidden in the legend stay hidden through density, locale, theme and window rebuilds', async () => {
  const density = new Set<() => void>();
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: false, media: query,
    addEventListener: (_type: string, listener: () => void) => { if (query.includes('resolution')) density.add(listener); },
    removeEventListener: (_type: string, listener: () => void) => density.delete(listener),
  }) as unknown as MediaQueryList);
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  // The legend's name button is a toggle; its accessible name follows the language.
  const nameButton = () => screen.getByRole('button', { name: labelOf(byId(LOAD)) });
  await fireEvent.click(nameButton());
  const expectHidden = ([percent, celsius]: FakeUplot[]) => {
    expect(percent.series.slice(1).map((s) => s.show !== false)).toEqual([false]);
    expect(celsius.series.slice(1).map((s) => s.show !== false)).toEqual([true]);
    expect(lastHeld().map((h) => h.color)).toEqual([celsius.opts.series[1].stroke]);
    expect([...document.querySelectorAll<HTMLElement>('.chart-dot')].map((d) => d.hidden)).toEqual([true, false]);
    expect(nameButton().getAttribute('aria-pressed')).toBe('false');
  };
  try {
    for (const listener of [...density]) listener();
    expect(plots).toHaveLength(4);
    expectHidden(plots.slice(2, 4));
    // The language comes from the settings, which re-apply it with every state they receive.
    await settings.update({ general: { language: 'it' } });
    flushSync();
    expect(plots).toHaveLength(6);
    expectHidden(plots.slice(4, 6));
    // A value no earlier test left on the root, so the style attribute really changes.
    document.documentElement.style.setProperty('--accent', '#654321');
    await vi.waitFor(() => expect(plots).toHaveLength(8));
    expectHidden(plots.slice(6, 8));
    await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.60') }));
    await vi.waitFor(() => expect(plots).toHaveLength(10));
    expectHidden(plots.slice(8, 10));
    // Showing it again in the legend is remembered too.
    await fireEvent.click(nameButton());
    await settings.update({ general: { language: 'en' } });
    flushSync();
    expect(plots).toHaveLength(12);
    expect(plots.slice(10).flatMap((p) => p.series.slice(1).map((s) => s.show !== false))).toEqual([true, true]);
  } finally {
    document.documentElement.removeAttribute('style');
  }
});

test('a series removed from the selection forgets that it was hidden', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  await fireEvent.click(screen.getByRole('button', { name: labelOf(byId(LOAD)) }));
  await fireEvent.click(checkbox(labelOf(byId(LOAD))));
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  await fireEvent.click(checkbox(labelOf(byId(LOAD))));
  await vi.waitFor(() => expect(plots).toHaveLength(5));
  // Ticked again, it comes back last: temperature first, then load, both shown.
  expect(live().map((p) => p.opts.series[1].scale)).toEqual(['celsius', 'percent']);
  expect(live().flatMap((p) => p.opts.series.slice(1).map((s) => s.show !== false))).toEqual([true, true]);
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(5000);
  FakeUplot.pxRatio = 2;
  try {
    for (const listener of [...listeners]) listener();
    expect(plots).toHaveLength(4);
    expect(plots[2].scales.at(-1)?.range.max).toBe(7);
    expect(plots[3].scales.at(-1)?.range.max).toBe(7);
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(5000);
  const schema = { ...MOCK_SCHEMA, revision: 2 };
  store.applySchema(schema);
  backend.getHistory = async () => ({ revision: 2, seq: 0, timestampsMs: [1000], series: [[10], [20]] });
  await rerender({ sectionId: GPU, sensors: gpuSensors, defaults: [LOAD, TEMP], schema, store, backend });
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  expect(plots[2].scales.at(-1)?.range.max).toBe(1);
  expect(plots[3].scales.at(-1)?.range.max).toBe(1);
});

/**
 * A real uPlot 1.6.32 built from the options the component handed to the stub, so the
 * component's cursor callbacks run against the library's own cursor code.
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
  const unit = configured.opts.series[1].scale!;
  actual.batch(() => {
    actual.setScale('x', configured.scales.at(-1)!.range);
    actual.setScale(unit, unit === 'celsius' ? { min: 0, max: 200 } : { min: 0, max: 100 });
  });
  await Promise.resolve();
  return { actual, dispose: () => { actual.destroy(); target.remove(); } };
}

test('real uPlot cursor selects the visible sample after scrolling and keeps the crosshair under the pointer', async () => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [1000, 2000, 3000], series: [[10, 20, 30], [40, 50, 60]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(20_000);
  const configured = plots[0];
  const offset = 40;
  expect(document.querySelector<HTMLCanvasElement>('.chart-canvas')!.style.transform).toBe('translateX(-40px)');
  const { actual, dispose } = await realCursorPlot(configured);
  try {
    // The curve of the cached scale is drawn `offset` px to the left of uPlot's X mapping.
    const pointerX = actual.valToPos(2, 'x') - offset;
    actual.setCursor({ left: pointerX, top: 30 });
    flushSync();
    expect(actual.legend.idx).toBe(1);
    expect(actual.cursor.idx).toBe(1);
    expect(actual.cursor.idxs).toEqual([1, 1]);
    // Our legend shows the values of that sample, in every strip.
    expect(legendCell(byId(LOAD), 'now')).toBe('20%');
    expect(legendCell(byId(TEMP), 'now')).toBe('50 °C');
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
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
    flushSync();
    expect(actual.over.style.getPropertyValue('--chart-cursor-offset')).toBe('0px');
    expect(actual.legend.idx).toBe(2);
    expect(legendCell(byId(LOAD), 'now')).toBe('30%');
    expect(actual.over.querySelector<HTMLElement>('.u-cursor-x')!.style.transform).toBe(`translate(${Math.round(pointerX)}px,0px)`);
  } finally {
    dispose();
  }
});

test('a stationary hovered pointer is hit-tested again while the canvas scrolls, never on rebase', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(1000);
  expect(document.querySelector('.chart-marker-clip')).toBeNull();
  const lineClips = [...document.querySelectorAll<HTMLElement>('.chart-canvas-clip')];
  const dotClips = [...document.querySelectorAll<HTMLElement>('.chart-dot-clip')];
  expect(lineClips).toHaveLength(2);
  expect(dotClips).toHaveLength(2);
  const px = (v: string) => Number.parseFloat(v);
  const plot = plots[0].bbox;
  // Held lines are canvas strokes, clipped to the plot area by the canvas layer and painter.
  for (const lineClip of lineClips) {
    expect(px(lineClip.style.left)).toBe(plot.left);
    expect(px(lineClip.style.top)).toBe(plot.top);
    expect(px(lineClip.style.width)).toBe(plot.width);
    expect(lineClip.querySelector('.chart-dot')).toBeNull();
  }
  expect(lastHeld().map((h) => h.y)).toEqual([plot.top, plot.top + plot.height]);
  for (const dotClip of dotClips) {
    expect(px(dotClip.style.left)).toBe(plot.left - 3);
    expect(px(dotClip.style.top)).toBe(plot.top - 3);
    expect(px(dotClip.style.width)).toBe(plot.width + 6);
    expect(px(dotClip.style.height)).toBe(plot.height + 6);
  }
  const dots = dotClips.flatMap((clip) => [...clip.querySelectorAll<HTMLElement>('.chart-dot')]);
  expect(dots).toHaveLength(2);
  expect(dots.map((d) => d.hidden)).toEqual([false, false]);
  expect(dots.map((d) => px(d.style.top))).toEqual([3, plot.height + 3]);
  for (const dot of dots) {
    const centerY = px(dot.style.top);
    expect(centerY - 3).toBeGreaterThanOrEqual(0);
    expect(centerY + 3).toBeLessThanOrEqual(px(dot.parentElement!.style.height));
  }
});

test('frames between samples only change the canvas transform', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
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
  await vi.waitFor(() => expect(plots).toHaveLength(2));
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
  // The Y axis is 72 px wide and the right side has no padding; the rest of the width is the plot.
  const actual = new RealUplot({ ...configured.opts, width: plotWidthPx + 72, hooks }, configured.data, target);
  const end = Date.UTC(2026, 8, 27, 21, 9, 3) / 1000;
  const unit = configured.opts.series[1].scale!;
  actual.batch(() => {
    actual.setScale('x', { min: end - windowSeconds, max: end });
    actual.setScale(unit, unit === 'celsius' ? { min: 0, max: 200 } : { min: 0, max: 100 });
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
    await settings.update({ advanced: { window: seconds } });
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    for (const width of [320, 651, 1200]) {
      // The last strip carries the time labels.
      const axis = await realTimeAxis(plots[1], width, seconds);
      expect(axis.plotWidth).toBe(width);
      expect(axis.incr).toBeGreaterThan(0);
      const labels = axis.splits.map((split) => formatTimeTick(split, locale, axis.incr));
      expect(axis.spacing).toBeGreaterThanOrEqual(Math.max(...labels.map((label) => arialWidth(label))) + TIME_LABEL_GAP_PX);
      expect(new Set(labels).size).toBe(labels.length);
    }
  });
});

/**
 * The Y axis of a real uPlot 1.6.32 built from a strip's options at the strip's own height,
 * with a fixed scale range, so uPlot's own increment table and label filter choose the
 * splits and render the labels via `axis.values`.
 */
async function realYAxis(configured: FakeUplot, range: [number, number]) {
  const { default: RealUplot } = await vi.importActual<{ default: typeof import('uplot') }>('uplot');
  const ctx = new Proxy({
    font: '12px sans-serif',
    measureText(text: string) { return { width: arialWidth(text, Number.parseFloat(this.font)) }; },
  }, { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
  const target = document.createElement('div');
  document.body.append(target);
  const { draw: _draw, ...hooks } = configured.opts.hooks ?? {};
  const actual = new RealUplot({ ...configured.opts, hooks }, configured.data, target);
  actual.batch(() => {
    actual.setScale('x', configured.scales.at(-1)!.range);
    actual.setScale(configured.opts.series[1].scale!, { min: range[0], max: range[1] });
  });
  await Promise.resolve();
  await Promise.resolve();
  const axis = actual.axes[1] as unknown as { _values: string[] };
  const result = { labels: [...axis._values] };
  actual.destroy();
  target.remove();
  return result;
}

test('the temperature strip axis never repeats a label on a narrow real temperature range', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const axis = await realYAxis(plots[1], [42.6, 44.3]);
  expect(axis.labels.length).toBeGreaterThan(1);
  expect(new Set(axis.labels).size).toBe(axis.labels.length);
});

test('a 1 min window at 651 px shows seconds on splits wide enough for them in en and it', async () => {
  for (const [locale, sample] of [['en', '12:58:05 AM'], ['it', '21:09:05']] as const) {
    cleanup();
    plots.length = 0;
    i18n.locale = locale;
    await settings.update({ advanced: { window: 60 } });
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    const axis = await realTimeAxis(plots[1], 651, 60);
    // uPlot's default 50 px would pick 5 s splits, about 54 px apart.
    expect(axis.incr).toBeLessThan(60);
    expect(axis.spacing).toBeGreaterThanOrEqual(arialWidth(sample) + TIME_LABEL_GAP_PX);
    expect(formatTimeTick(axis.splits[0], locale, axis.incr)).toMatch(/:\d{2}:\d{2}/);
  }
});

type YRange = { min: number; max: number };
/** The range uPlot's default numeric autoscale gives the celsius scale for this data. */
const celsiusRange = (min: number, max: number): YRange => {
  const [lo, hi] = FakeUplot.rangeNum(min, max, 0.1, true);
  return { min: lo!, max: hi! };
};
/** The transition's documented ease-out cubic between two displayed ranges. */
const eased = (from: YRange, to: YRange, progress: number): YRange => {
  const k = 1 - (1 - progress) ** 3;
  return { min: from.min + (to.min - from.min) * k, max: from.max + (to.max - from.max) * k };
};
const expectRange = (actual: YRange | undefined, expected: YRange) => {
  expect(actual?.min).toBeCloseTo(expected.min, 9);
  expect(actual?.max).toBeCloseTo(expected.max, 9);
};
/** Applies a snapshot whose GPU temperature is `temperature`, at the current monotonic time. */
function snapshotTemperature(store: LiveStore, seq: number, timestampMs: number, temperature: number) {
  const values = mockValues(seq);
  values[index(TEMP)] = temperature;
  store.applySnapshot({ revision: 1, seq, timestampMs, values });
  flushSync();
}
const yScaleCalls = (plot: FakeUplot, key: string) => plot.scales.filter((s) => s.key === key);

/** A chart whose stub autoscales Y like uPlot; the history puts the celsius scale on 11-21. */
async function transitionChart() {
  FakeUplot.autoRangeY = true;
  const store = new LiveStore();
  const view = renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  frame(0);
  // The plot of the temperature strip; the load strip is plots[0].
  return { store, view, plot: plots[1], from: celsiusRange(11, 21) };
}

test('a changed Y range moves the uPlot scale, paths, held segments and dots together for 180 ms', async () => {
  const { store, plot, from } = await transitionChart();
  expectRange(plot.yRanges.get('celsius'), from);
  const percentPlot = plots[0];
  const percent = percentPlot.yRanges.get('percent');
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 90);
  const to = celsiusRange(11, 90);
  // The new sample is first drawn on the displayed scale, not on the new target.
  expectRange(plot.yRanges.get('celsius'), from);
  const temperatureY = (value: number, range: YRange) => plot.bbox.top + plot.bbox.height * (1 - (value - range.min) / (range.max - range.min));
  const dot = document.querySelectorAll<HTMLElement>('.chart-dot')[1];
  /** The temperature path passes through the 21 °C history sample at this range's height. */
  const pathThrough21 = (range: YRange) => {
    const path = paintsOf(plot.root.querySelector<HTMLCanvasElement>('.chart-canvas')!).at(-1)![2][0].stroke as unknown as RecordingPath;
    return path.commands.some((c) => Math.abs(c.args.at(-1)! - temperatureY(21, range)) < 1e-9);
  };
  const expectDrawnOn = (range: YRange) => {
    expect(pathThrough21(range)).toBe(true);
    expect(lastHeld()[1].y).toBeCloseTo(temperatureY(90, range), 9);
    expect(dot.hidden).toBe(false);
    expect(Number.parseFloat(dot.style.top)).toBeCloseTo(temperatureY(90, range) - plot.bbox.top + 3, 9);
  };
  expect(pathThrough21(from)).toBe(true);
  // 90 °C lies above the displayed range but inside the target: held segment and dot stay,
  // clamped to the plot's top edge until the scale reaches them.
  expect(lastHeld()).toHaveLength(2);
  expect(lastHeld()[1].y).toBe(plot.bbox.top);
  expect(dot.hidden).toBe(false);
  expect(dot.style.top).toBe('3px');

  frame(1150);
  const mid = eased(from, to, 150 / 180);
  // uPlot draws its Y ticks and labels from this scale, in the same commit as the canvas.
  expectRange(yScaleCalls(plot, 'celsius').at(-1)?.range, mid);
  expectRange(plot.yRanges.get('celsius'), mid);
  expectDrawnOn(mid);
  // The X translation keeps going on the same frames.
  const transform = document.querySelector<HTMLCanvasElement>('.chart-canvas')!.style.transform;
  expect(Number.parseFloat(/translateX\((.+)px\)/.exec(transform)![1])).toBeCloseTo(-0.15 * plot.bbox.width / 300, 9);

  frame(1180);
  expect(plot.yRanges.get('celsius')).toEqual(to);
  expectDrawnOn(to);
  // The unchanged percent scale is never set explicitly.
  expect(yScaleCalls(percentPlot, 'percent')).toEqual([]);
  expect(percentPlot.yRanges.get('percent')).toEqual(percent);
  expect(plot.setDataCalls).toBe(1);
});

test('an expanding Y transition never hides the dot or held segment of the value that caused it', async () => {
  const { store, plot } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 190);
  const dot = document.querySelectorAll<HTMLElement>('.chart-dot')[1];
  const { top, height } = plot.bbox;
  for (const at of [1000, 1016, 1033, 1050, 1100, 1180]) {
    if (at > 1000) frame(at);
    expect(dot.hidden).toBe(false);
    expect(lastHeld()).toHaveLength(2);
    const y = lastHeld()[1].y;
    expect(y).toBeGreaterThanOrEqual(top);
    expect(y).toBeLessThanOrEqual(top + height);
    expect(Number.parseFloat(dot.style.top)).toBeCloseTo(y - top + 3, 9);
  }
  expect(lastHeld()[1].y).toBeCloseTo(plot.valToPos(190, 'celsius', true), 9);
});

test('a final value outside a Y scale that is not moving still hides its dot and held segment', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const values = mockValues(1);
  // The stub keeps the percent scale at 0-100, so 150% is outside and no transition runs.
  values[index(LOAD)] = 150;
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values });
  flushSync();
  expect(lastHeld().map((h) => h.color)).toEqual([plots[1].opts.series[1].stroke]);
  expect(document.querySelectorAll<HTMLElement>('.chart-dot')[0].hidden).toBe(true);
});

test('the Y transition repaints only for 180 ms, then frames are transform-only again', async () => {
  const { store, plot } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 90);
  const painted = paints().length;
  for (const at of [1016, 1033, 1100, 1180]) frame(at);
  expect(paints().length).toBeGreaterThanOrEqual(painted + 4);
  const canvas = plot.root.querySelector<HTMLCanvasElement>('.chart-canvas')!;
  const records: MutationRecord[] = [];
  const observer = new MutationObserver((batch) => records.push(...batch));
  observer.observe(plot.root, { attributes: true, subtree: true, childList: true, characterData: true });
  const repainted = paints().length;
  const scales = plot.scales.length;
  for (const at of [1196, 1213, 1300, 2000]) frame(at);
  await Promise.resolve();
  records.push(...observer.takeRecords());
  observer.disconnect();
  expect(records.length).toBeGreaterThan(0);
  for (const record of records) {
    expect(record.target).toBe(canvas);
    expect(record.attributeName).toBe('style');
  }
  expect(paints()).toHaveLength(repainted);
  expect(plot.scales).toHaveLength(scales);
});

test('a snapshot that keeps the Y range starts no transition and no extra redraw', async () => {
  const { store, plot, from } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 15);
  expectRange(plot.yRanges.get('celsius'), from);
  const painted = paints().length;
  const scales = plot.scales.length;
  for (const at of [1016, 1090, 1180, 1300]) frame(at);
  expect(paints()).toHaveLength(painted);
  expect(plot.scales).toHaveLength(scales);
});

test('a second snapshot during a transition restarts it from the displayed Y range', async () => {
  const { store, plot, from } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 90);
  frame(1090);
  const shown = eased(from, celsiusRange(11, 90), 0.5);
  expectRange(plot.yRanges.get('celsius'), shown);
  monotonicMs = 1090;
  snapshotTemperature(store, 2, 4000, 190);
  // The new autoscale starts where the scale is, not at the old target or at the new one.
  expectRange(plot.yRanges.get('celsius'), shown);
  const to = celsiusRange(11, 190);
  frame(1180);
  expectRange(plot.yRanges.get('celsius'), eased(shown, to, 0.5));
  frame(1270);
  expect(plot.yRanges.get('celsius')).toEqual(to);
});

test('reduced motion applies a changed Y range at once, without transition frames', async () => {
  reducedMotion = true;
  FakeUplot.autoRangeY = true;
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  snapshotTemperature(store, 1, 3000, 90);
  expect(plots[1].yRanges.get('celsius')).toEqual(celsiusRange(11, 90));
  expect(lastHeld()[1].y).toBeCloseTo(plots[1].valToPos(90, 'celsius', true), 9);
  expect(frames.size).toBe(0);
});

test('switching to reduced motion during a transition jumps to the final Y range', async () => {
  const { store, plot } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 90);
  frame(1050);
  reducedMotion = true;
  for (const listener of motionListeners) listener();
  expect(frames.size).toBe(0);
  expect(plot.yRanges.get('celsius')).toEqual(celsiusRange(11, 90));
  expect(lastHeld()[1].y).toBeCloseTo(plot.valToPos(90, 'celsius', true), 9);
});

test('hiding during a transition cancels it and the visible rebuild shows the final range at once', async () => {
  const { store, plot } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 90);
  frame(1090);
  setVisibility('hidden');
  expect(frames.size).toBe(0);
  const scales = plot.scales.length;
  monotonicMs = 5000;
  setVisibility('visible');
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  const rebuilt = plots[3];
  expect(rebuilt.yRanges.get('celsius')).toEqual(celsiusRange(11, 90));
  for (const at of [5016, 5100, 5200]) frame(at);
  expect(yScaleCalls(rebuilt, 'celsius')).toEqual([]);
  expect(plot.scales).toHaveLength(scales);
});

test('unmounting during a transition cancels its frames', async () => {
  const { store, view, plot } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 90);
  frame(1090);
  const scales = plot.scales.length;
  view.unmount();
  expect(frames.size).toBe(0);
  expect(plot.scales).toHaveLength(scales);
});

test('a resize during a transition keeps it running on the same plot', async () => {
  const { resize } = stubResizeObserver();
  const { store, plot, from } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 90);
  const to = celsiusRange(11, 90);
  frame(1090);
  resize(1000);
  expect(plots).toHaveLength(2);
  expectRange(plot.yRanges.get('celsius'), eased(from, to, 0.5));
  frame(1180);
  expect(plot.yRanges.get('celsius')).toEqual(to);
  expect(lastHeld()[1].y).toBeCloseTo(plot.valToPos(90, 'celsius', true), 9);
});

test.each(['density', 'locale', 'theme'] as const)('a %s rebuild during a transition shows the final range on fresh layers', async (kind) => {
  const { disconnect } = stubResizeObserver();
  const density = new Set<() => void>();
  vi.stubGlobal('matchMedia', (query: string) => ({
    get matches() { return reducedMotion; },
    media: query,
    addEventListener: (_type: string, listener: () => void) => {
      if (query.includes('resolution')) density.add(listener);
      else if (query.includes('reduced-motion')) motionListeners.add(listener);
    },
    removeEventListener: (_type: string, listener: () => void) => { density.delete(listener); motionListeners.delete(listener); },
  }));
  const { store, view, plot } = await transitionChart();
  monotonicMs = 1000;
  snapshotTemperature(store, 1, 3000, 90);
  frame(1090);
  try {
    if (kind === 'density') for (const listener of [...density]) listener();
    else if (kind === 'locale') { i18n.locale = 'it'; flushSync(); }
    else document.documentElement.style.setProperty('--accent', '#123456');
    await vi.waitFor(() => expect(plots).toHaveLength(4));
    const rebuilt = plots[3];
    expect(rebuilt.yRanges.get('celsius')).toEqual(celsiusRange(11, 90));
    // One canvas and one dot layer per strip, none left over from the old plots.
    expect(document.querySelectorAll('.chart-canvas')).toHaveLength(2);
    expect(document.querySelectorAll('.chart-dot-clip')).toHaveLength(2);
    const dot = document.querySelectorAll<HTMLElement>('.chart-dot')[1];
    expect(Number.parseFloat(dot.style.top)).toBeCloseTo(rebuilt.valToPos(90, 'celsius', true) - rebuilt.bbox.top + 3, 9);
    expect(density.size).toBe(1);
    const scales = plot.scales.length;
    for (const at of [1100, 1180, 1300]) frame(at);
    expect(yScaleCalls(rebuilt, 'celsius')).toEqual([]);
    expect(plot.scales).toHaveLength(scales);
    view.unmount();
    expect(density.size).toBe(0);
    // One resize observer per strip.
    expect(disconnect).toHaveBeenCalledTimes(2);
  } finally {
    document.documentElement.removeAttribute('style');
  }
});

test('real uPlot autoscales through the transition range exactly as through its own unit ranges', async () => {
  const backend = fakeBackend();
  backend.history = { timestampsMs: [1000, 2000], series: [[10, 120], [11, 21]] };
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const { default: RealUplot } = await vi.importActual<{ default: typeof import('uplot') }>('uplot');
  const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) }, {
    get: (target, key) => key in target ? target[key as keyof typeof target] : () => {},
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
  /** The Y range a real uPlot gives each strip's own scale with `scales`. */
  const ranges = async (configured: FakeUplot, scales: uPlot.Options['scales']) => {
    const { draw: _draw, ...hooks } = configured.opts.hooks ?? {};
    const unit = configured.opts.series[1].scale!;
    const target = document.createElement('div');
    document.body.append(target);
    const actual = new RealUplot({ ...configured.opts, scales, hooks }, configured.data, target);
    await Promise.resolve();
    await Promise.resolve();
    const result = [actual.scales[unit].min, actual.scales[unit].max];
    actual.destroy();
    target.remove();
    return result;
  };
  const expected: Array<Array<number | undefined>> = [];
  for (const configured of plots) {
    const unit = configured.opts.series[1].scale! as 'percent' | 'celsius';
    const own = { x: { time: true }, [unit]: scaleOptions(unit) };
    expected.push(await ranges(configured, own));
    // The component's autoscale gate is closed once it has painted; open it as a snapshot does.
    const gateOpen = Object.fromEntries(Object.entries(configured.opts.scales!).map(([key, scale]) => [key, key === 'x' ? scale : { ...scale, auto: true }]));
    expect(await ranges(configured, gateOpen)).toEqual(expected.at(-1));
  }
  expect(expected[0]).toEqual([0, 120]);
});

const NIC = 'network/mock-eth';
const nicSensors = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === NIC);
const NIC_DEFAULTS = [`${NIC}/throughput/down`, `${NIC}/throughput/up`];
/** Axis and legend text of the series on `scale`, for the value `v` in the units the plot holds. */
function axisAndLegend(plot: FakeUplot, scale: string, v: number): [string, string] {
  const axisIndex = plot.opts.axes!.findIndex((a) => a.scale === scale);
  const axis = plot.opts.axes![axisIndex];
  const series = plot.opts.series.find((s) => s.scale === scale)!;
  const label = (axis.values as (u: uPlot, splits: number[]) => string[])(plot as unknown as uPlot, [v])[0];
  const legend = (series.value as (u: uPlot, v: number | null) => string)(plot as unknown as uPlot, v);
  return [label, legend];
}

test('history chart converts temperature series and axis to °F', async () => {
  await connectSettings({ general: { temperatureUnit: 'f' } });
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));

  // Column 1 holds the temperature in degrees Celsius: 11 and 21.
  expect(plots[0].data).toEqual([[1, 2], [10, 20]]);
  expect(plots[1].data).toEqual([[1, 2], [51.8, 69.8]]);
  expect(plots[1].opts.series[1].scale).toBe('celsius');
  expect(axisAndLegend(plots[1], 'celsius', 212)).toEqual(['212 °F', '212 °F']);
  // Other units stay as they are.
  expect(axisAndLegend(plots[0], 'percent', 50)).toEqual(['50%', '50%']);

  // A live sample is converted too.
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  cleanup();
  plots.length = 0;
  renderChart(backend, store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: MOCK_SCHEMA.sensors.map((s) => (s.id === TEMP ? 100 : 0)) });
  await vi.waitFor(() => expect(plots[1].data[1].at(-1)).toBe(212));
  // The legend shows the latest reading in the chosen unit too.
  expect(legendCell(byId(TEMP), 'now')).toBe('212 °F');
});

test('a temperature unit change redraws the chart from the buffer without reloading the history', async () => {
  await connectSettings({ general: { temperatureUnit: 'f' } });
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(backend.historyCalls).toHaveLength(1);

  await settings.update({ general: { temperatureUnit: 'c' } });
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  expect(backend.historyCalls).toHaveLength(1);
  expect(plots[2].data).toEqual([[1, 2], [10, 20]]);
  expect(plots[3].data).toEqual([[1, 2], [11, 21]]);
  expect(axisAndLegend(plots[3], 'celsius', 100)).toEqual(['100 °C', '100 °C']);
});

test('network chart axis is in bit/s when bits are chosen', async () => {
  const backend = fakeBackend();
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  render(StripChart, { sectionId: NIC, sensors: nicSensors, defaults: NIC_DEFAULTS, schema: MOCK_SCHEMA, store, backend, rate: 'bits' });
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  // Bytes per second in the history, bits per second on the chart.
  expect(plots[0].data).toEqual([[1, 2], [80, 160], [88, 168]]);
  const [axis, legend] = axisAndLegend(plots[0], 'bytes_per_second', 8e6);
  expect(axis).toBe('8.0 Mbit/s');
  expect(legend).toBe('8.0 Mbit/s');

  cleanup();
  plots.length = 0;
  render(StripChart, { sectionId: NIC, sensors: nicSensors, defaults: NIC_DEFAULTS, schema: MOCK_SCHEMA, store, backend, rate: 'bytes' });
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(plots[0].data).toEqual([[1, 2], [10, 20], [11, 21]]);
  expect(axisAndLegend(plots[0], 'bytes_per_second', 8 * 1024 ** 2)[0]).toBe('8.0 MB/s');
});

test('chart and sparkline resubscribe when fps changes: chart', async () => {
  await connectSettings({ general: { chartFps: 60 } });
  const subscriptions = spyChartFrames();
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  // One subscription drives every strip of the chart.
  const live = () => subscriptions.filter((s) => !s.stopped);
  expect(live().map((s) => s.fps)).toEqual([60]);

  await settings.update({ general: { chartFps: 30 } });
  await vi.waitFor(() => expect(live().map((s) => s.fps)).toEqual([30]));
  expect(subscriptions.map((s) => [s.fps, s.stopped])).toEqual([[60, true], [30, false]]);

  cleanup();
  expect(live()).toEqual([]);
});

/** Replaces IntersectionObserver; the returned function tells a strip it left or entered the screen. */
function stubIntersectionObserver() {
  const observers: Array<{ callback: IntersectionObserverCallback; targets: Element[] }> = [];
  vi.stubGlobal('IntersectionObserver', class {
    entry: { callback: IntersectionObserverCallback; targets: Element[] };
    constructor(callback: IntersectionObserverCallback) { observers.push((this.entry = { callback, targets: [] })); }
    observe(target: Element) { this.entry.targets.push(target); }
    unobserve() {}
    disconnect() {}
  });
  /** `plot.target` is the strip's own `.plot`, which its observer watches. */
  const intersect = (plot: FakeUplot, isIntersecting: boolean) => {
    const observer = observers.find((o) => o.targets.includes(plot.target!))!;
    observer.callback([{ isIntersecting, target: plot.target! } as unknown as IntersectionObserverEntry], {} as IntersectionObserver);
  };
  return { intersect };
}

test('only_the_last_strip_draws_time_labels', async () => {
  renderChart(fakeBackend(), new LiveStore(), gpuSensors, [LOAD, TEMP, VRAM]);
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  const [first, second, last] = canvases();
  for (const canvas of [first, second]) {
    expect(paintsOf(canvas).length).toBeGreaterThan(0);
    expect(paintsOf(canvas).every((call) => call[9]?.timeLabels === false)).toBe(true);
    expect(vi.mocked(canvas.getContext('2d')!.fillText)).not.toHaveBeenCalled();
  }
  expect(paintsOf(last).length).toBeGreaterThan(0);
  expect(paintsOf(last).every((call) => call[9]?.timeLabels === true)).toBe(true);
  expect(vi.mocked(last.getContext('2d')!.fillText)).toHaveBeenCalled();
  // The vertical grid lines fall on the same instants in every strip.
  const ticks = paintsOf(first).at(-1)![3];
  expect(ticks.length).toBeGreaterThan(0);
  expect(paintsOf(second).at(-1)![3]).toEqual(ticks);
  expect(paintsOf(last).at(-1)![3]).toEqual(ticks);
});

test('strip_heights_are_260_alone_and_130_plus_axis_when_stacked', async () => {
  renderChart(fakeBackend(), new LiveStore(), gpuSensors, [TEMP]);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(plots[0].opts.height).toBe(260);
  expect(plots[0].opts.axes![0].size).toBe(40);
  expect(canvases()[0].height).toBe(200 + 40);

  cleanup();
  plots.length = 0;
  renderChart(fakeBackend(), new LiveStore(), gpuSensors, [LOAD, TEMP, VRAM]);
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(plots.map((p) => p.opts.height)).toEqual([130, 130, 170]);
  // Only the last strip keeps room for the time labels under its lines.
  expect(plots.map((p) => p.opts.axes![0].size)).toEqual([0, 0, 40]);
  expect(canvases().map((c) => c.height)).toEqual([200, 200, 240]);
  // Every Y axis has the same width, so the lines of all strips line up.
  expect(plots.map((p) => p.opts.axes![1].size)).toEqual([72, 72, 72]);
});

test('frames_only_translate_every_strip', async () => {
  renderChart(fakeBackend(), new LiveStore(), gpuSensors, [LOAD, TEMP, VRAM]);
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  frame(0);
  const chart = document.querySelector<HTMLElement>('section.chart')!;
  const strips = canvases();
  expect(strips).toHaveLength(3);
  const records: MutationRecord[] = [];
  const observer = new MutationObserver((batch) => records.push(...batch));
  // The whole chart, legends included.
  observer.observe(chart, { attributes: true, subtree: true, childList: true, characterData: true });
  const before = strips.map((c) => c.style.cssText);
  const painted = paints().length;
  const scales = plots.map((p) => p.scales.length);
  for (const at of [16, 33, 50, 1000, 2000]) frame(at);
  await Promise.resolve();
  records.push(...observer.takeRecords());
  observer.disconnect();
  expect(new Set(records.map((r) => r.target))).toEqual(new Set(strips));
  for (const record of records) {
    expect(record.type).toBe('attributes');
    expect(record.attributeName).toBe('style');
  }
  const strip = (css: string) => css.replace(/transform:[^;]*;?/, '').trim();
  expect(strips.map((c) => strip(c.style.cssText))).toEqual(before.map(strip));
  // 2 s of a 300 s window over 600 px: every strip scrolled together.
  expect(strips.map((c) => c.style.transform)).toEqual(Array(3).fill('translateX(-4px)'));
  expect(paints()).toHaveLength(painted);
  expect(plots.map((p) => p.scales.length)).toEqual(scales);
  expect(plots.map((p) => p.setDataCalls)).toEqual([0, 0, 0]);
});

test('offscreen_strips_skip_set_data_until_visible', async () => {
  const { intersect } = stubIntersectionObserver();
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  const [percent, celsius] = plots;
  frame(0);
  intersect(celsius, false);
  const hiddenCanvas = canvases()[1];
  const transform = hiddenCanvas.style.transform;
  const painted = paintsOf(hiddenCanvas).length;

  monotonicMs = 500;
  const columns = vi.spyOn(ChartBuffer.prototype, 'columns');
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(percent.setDataCalls).toBe(1);
  expect(celsius.setDataCalls).toBe(0);
  // Only the strip on screen gets its columns built.
  expect(columns).toHaveBeenCalledTimes(1);
  columns.mockRestore();
  frame(1000);
  frame(2000);
  // No paths, no paint, no scroll for the strip off screen.
  expect(hiddenCanvas.style.transform).toBe(transform);
  expect(paintsOf(hiddenCanvas)).toHaveLength(painted);
  expect(celsius.data).toEqual([[1, 2], [11, 21]]);
  // Its legend still follows the data.
  expect(legendCell(byId(TEMP), 'now')).toBe(formatValue(mockValues(1)[index(TEMP)], 'celsius', 'en', t));

  monotonicMs = 2000;
  intersect(celsius, true);
  expect(celsius.setDataCalls).toBe(1);
  expect(celsius.data).toEqual([[1, 2, 3], [11, 21, mockValues(1)[index(TEMP)]]]);
  // Rebased on the shared viewport, 1.5 s after the snapshot rebased the strip on screen.
  expect(celsius.scales.at(-1)?.range.max).toBeCloseTo(percent.scales.at(-1)!.range.max + 1.5, 9);
  expect(paintsOf(hiddenCanvas).length).toBeGreaterThan(painted);
});

test('strips_share_one_time_column_per_snapshot', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].data[0]).toEqual([1, 2, 3]);
  expect(plots[1].data[0]).toBe(plots[0].data[0]);
});

test('legend_shows_latest_values_without_cursor_and_cursor_values_on_hover', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  const latestLoad = formatValue(mockValues(1)[index(LOAD)], 'percent', 'en', t);
  const latestTemp = formatValue(mockValues(1)[index(TEMP)], 'celsius', 'en', t);
  expect(legendCell(byId(LOAD), 'now')).toBe(latestLoad);
  expect(legendCell(byId(TEMP), 'now')).toBe(latestTemp);

  // The pointer on one strip: every legend shows that instant.
  hover(plots[1], 0);
  expect(legendCell(byId(LOAD), 'now')).toBe('10%');
  expect(legendCell(byId(TEMP), 'now')).toBe('11 °C');
  hover(plots[0], 1);
  expect(legendCell(byId(LOAD), 'now')).toBe('20%');
  expect(legendCell(byId(TEMP), 'now')).toBe('21 °C');
  // Min and max stay those of the window.
  expect(legendCell(byId(LOAD), 'min')).toBe('10%');

  hover(plots[0], null);
  expect(legendCell(byId(LOAD), 'now')).toBe(latestLoad);
  expect(legendCell(byId(TEMP), 'now')).toBe(latestTemp);
});

test('legend_min_max_follow_the_visible_window', async () => {
  await settings.update({ advanced: { window: 60 } });
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(legendCell(byId(LOAD), 'min')).toBe('10%');
  expect(legendCell(byId(LOAD), 'max')).toBe('20%');

  const values = mockValues(1);
  values[index(LOAD)] = 50;
  const quality = MOCK_SCHEMA.sensors.map((_, i) => (i === index(TEMP) ? 2 : 0));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 62_000, values, quality });
  flushSync();
  // The sample at 1 s is kept for the left edge but lies outside the 60 s window.
  expect(plots[0].data[1]).toEqual([10, 20, 50]);
  expect(legendCell(byId(LOAD), 'min')).toBe('20%');
  expect(legendCell(byId(LOAD), 'max')).toBe('50%');
  // A gap does not count.
  expect(legendCell(byId(TEMP), 'min')).toBe('21 °C');
  expect(legendCell(byId(TEMP), 'max')).toBe('21 °C');

  const next = mockValues(2);
  next[index(LOAD)] = 5;
  store.applySnapshot({ revision: 1, seq: 2, timestampMs: 63_000, values: next, quality });
  flushSync();
  expect(legendCell(byId(LOAD), 'min')).toBe('5%');
  expect(legendCell(byId(LOAD), 'max')).toBe('50%');

  // No value in the window: a dash.
  store.applySnapshot({ revision: 1, seq: 3, timestampMs: 200_000, values: next, quality });
  flushSync();
  expect(legendCell(byId(TEMP), 'min')).toBe('—');
  expect(legendCell(byId(TEMP), 'max')).toBe('—');
});

test('remove_button_drops_the_series_and_saves', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  await fireEvent.click(removeButton(byId(LOAD)));
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(backend.historyCalls.at(-1)?.ids).toEqual([TEMP]);
  expect(live().map((p) => p.opts.series[1].scale)).toEqual(['celsius']);
  await vi.waitFor(() => expect(settings.state?.settings.advanced.series[GPU]).toEqual([TEMP]));
  expect(checkbox(labelOf(byId(LOAD))).checked).toBe(false);
  expect(screen.queryByRole('button', { name: labelOf(byId(LOAD)) })).toBeNull();
});

test('removing_the_last_series_of_a_unit_drops_its_strip', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  // The pointer is on the temperature strip when its only series goes.
  hover(plots[1], 0);
  expect(legendCell(byId(LOAD), 'now')).toBe('10%');
  await fireEvent.click(removeButton(byId(TEMP)));
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(live().map((p) => p.opts.series[1].scale)).toEqual(['percent']);
  expect(document.querySelectorAll('.strip')).toHaveLength(1);
  expect(screen.queryByText(named('celsius'))).toBeNull();
  // The cursor index is gone with it: the legend shows the latest reading again.
  expect(legendCell(byId(LOAD), 'now')).toBe(formatValue(mockValues(1)[index(LOAD)], 'percent', 'en', t));
  // The one strip left carries the time axis, at the single-strip height.
  expect(live()[0].opts.height).toBe(260);
  expect(live()[0].opts.axes![0].size).toBe(40);
});

test('temperature_unit_change_keeps_one_strip', async () => {
  const backend = fakeBackend();
  renderChart(backend, new LiveStore(), gpuSensors, [LOAD, TEMP, HOTSPOT]);
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(plots[1].opts.series.slice(1).map((s) => s.scale)).toEqual(['celsius', 'celsius']);

  await settings.update({ general: { temperatureUnit: 'f' } });
  await vi.waitFor(() => expect(plots).toHaveLength(4));
  // °F stays on the one temperature strip, rebuilt from the buffer.
  expect(live().map((p) => p.opts.series.slice(1).map((s) => s.scale))).toEqual([['percent'], ['celsius', 'celsius']]);
  expect(live()[1].data).toEqual([[1, 2], [51.8, 69.8], [53.6, 71.6]]);
  expect(axisAndLegend(live()[1], 'celsius', 212)).toEqual(['212 °F', '212 °F']);
  expect(document.querySelectorAll('.strip')).toHaveLength(2);
  // One temperature legend, its title in the unit now drawn.
  expect(screen.getAllByText(named('celsius'))).toHaveLength(1);
  expect(screen.getByText(titled(legendTitle('celsius', '°F')))).toBeTruthy();
  expect(legendCell(byId(TEMP), 'max')).toBe('70 °F');
  expect(backend.historyCalls).toHaveLength(1);
});

test('schema_revision_regroups_strips', async () => {
  const backend = fakeBackend();
  const store = new LiveStore();
  const { rerender } = renderChart(backend, store, gpuSensors, [LOAD, TEMP, VRAM]);
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  // The new revision has no temperature sensors any more.
  const schema = { ...MOCK_SCHEMA, revision: 2, sensors: MOCK_SCHEMA.sensors.filter((s) => s.id !== TEMP && s.id !== HOTSPOT) };
  store.applySchema(schema);
  const asked: string[][] = [];
  backend.getHistory = async (ids) => {
    asked.push(ids);
    return { revision: 2, seq: 0, timestampsMs: [1000], series: ids.map(() => [5]) };
  };
  await rerender({ sectionId: GPU, sensors: schema.sensors.filter((s) => s.deviceId === GPU), defaults: [LOAD, TEMP, VRAM], schema, store, backend });
  await vi.waitFor(() => expect(plots).toHaveLength(5));
  expect(asked.at(-1)).toEqual([LOAD, VRAM]);
  expect(live().map((p) => p.opts.series[1].scale)).toEqual(['percent', 'bytes']);
  // No empty strip is left behind, and the bytes strip is now the last, with the time axis.
  expect(document.querySelectorAll('.strip')).toHaveLength(2);
  expect(live().map((p) => p.opts.axes![0].size)).toEqual([0, 40]);
  expect(screen.queryByText(named('celsius'))).toBeNull();
  expect(screen.queryByRole('button', { name: labelOf(byId(TEMP)) })).toBeNull();
});

test('cursor_sync_key_is_shared_by_the_strips_of_a_chart', async () => {
  renderChart(fakeBackend(), new LiveStore(), gpuSensors, [LOAD, TEMP, VRAM]);
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(plots.map((p) => (p.opts.cursor?.sync as { key: string } | undefined)?.key)).toEqual(Array(3).fill(`chart-${GPU}`));
});

test('a_strip_with_16_series_keeps_its_height_and_its_legend_scrolls', async () => {
  // Sixteen load series on one strip, plus a temperature strip under it.
  const extra = Array.from({ length: 8 }, (_, i) => ({
    ...byId(`cpu/0/load/thread-0-${i}`),
    id: `cpu/0/load/thread-1-${i}`,
    label: { key: 'cpu.load.thread', arg: String(8 + i) },
  }));
  const schema = { ...MOCK_SCHEMA, sensors: [...MOCK_SCHEMA.sensors, ...extra] };
  const loads = [...cpuSensors.filter((s) => s.unit === 'percent').slice(1), ...extra];
  expect(loads).toHaveLength(16);
  const store = new LiveStore();
  store.applySchema(schema);
  const backend = new FakeBackend(schema);
  backend.history = { timestampsMs: [1000, 2000], series: Array.from({ length: 17 }, (_, i) => [10 + i, 20 + i]) };
  const ids = [...loads.map((s) => s.id), TEMP];
  render(StripChart, { sectionId: 'cpu/0', sensors: [...loads, byId(TEMP)], defaults: ids, schema, store, backend });
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(plots[0].opts.series).toHaveLength(17);
  expect(plots.map((p) => p.opts.height)).toEqual([130, 170]);
  // Each legend is bounded to its strip and scrolls; the strip height stays the spec's.
  const legends = [...document.querySelectorAll<HTMLElement>('.strip .legend')];
  expect(legends.map((l) => l.style.height)).toEqual(['130px', '170px']);
  expect(legends.map((l) => l.style.overflowY)).toEqual(['auto', 'auto']);
  expect(legends[0].querySelectorAll('.row')).toHaveLength(16);
});

test('legend_titles_carry_the_display_unit_symbol', async () => {
  await connectSettings({ general: { temperatureUnit: 'f' } });
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(screen.getByText(titled(legendTitle('celsius', '°F')))).toBeTruthy();
  await settings.update({ general: { temperatureUnit: 'c' } });
  await vi.waitFor(() => expect(screen.getByText(titled(legendTitle('celsius', '°C')))).toBeTruthy());
  expect(screen.getByText(titled(legendTitle('percent', '%')))).toBeTruthy();

  cleanup();
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  render(StripChart, { sectionId: NIC, sensors: nicSensors, defaults: NIC_DEFAULTS, schema: MOCK_SCHEMA, store, backend: fakeBackend(), rate: 'bits' });
  await vi.waitFor(() => expect(screen.getByText(titled(legendTitle('bytes_per_second')))).toBeTruthy());
});

test('title_symbols_keep_their_case_in_their_own_span', async () => {
  await connectSettings({ general: { temperatureUnit: 'f' } });
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect([...document.querySelectorAll('.legend .title .symbol')].map((el) => el.textContent)).toEqual(['%', '°F']);
  expect(legendSource).toMatch(/\.symbol\s*\{[^}]*text-transform: none;/);
  expect(drawerSource).toMatch(/\.symbol\s*\{[^}]*text-transform: none;/);
  openDrawer();
  expect(drawer()!.querySelectorAll('.title .symbol').length).toBeGreaterThan(0);
});

test('strips_have_no_right_padding_so_the_lines_end_by_the_legend', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  for (const plot of plots) expect(plot.opts.padding?.[1]).toBe(0);
  // In a real uPlot the plot box then ends at the right edge of the chart.
  const { default: RealUplot } = await vi.importActual<{ default: typeof import('uplot') }>('uplot');
  const ctx = new Proxy({ measureText: (text: string) => ({ width: text.length * 7 }) }, {
    get: (target, key) => key in target ? target[key as keyof typeof target] : () => {},
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
  const target = document.createElement('div');
  document.body.append(target);
  const { draw: _draw, ...hooks } = plots[1].opts.hooks ?? {};
  const actual = new RealUplot({ ...plots[1].opts, width: 800, hooks }, plots[1].data, target);
  try {
    expect((actual.bbox.left + actual.bbox.width) / FakeUplot.pxRatio).toBe(800);
  } finally {
    actual.destroy();
    target.remove();
  }
});

describe('series drawer', () => {
  const groupNames = () => [...drawer()!.querySelectorAll('[role="group"]')].map((g) => g.getAttribute('aria-label'));

  test('series_button_opens_the_drawer_over_the_lines_only', async () => {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    expect(drawer()).toBeNull();
    await fireEvent.click(seriesButton());
    const dialog = drawer()!;
    expect(dialog).toBeTruthy();
    expect(dialog.getAttribute('aria-modal')).not.toBe('true');
    // Over the column of lines: from the Y axis (72 px) to the legend (220 px + 12 px gap).
    const strips = dialog.parentElement!;
    expect(strips.classList.contains('strips')).toBe(true);
    expect(strips.querySelectorAll('.strip')).toHaveLength(2);
    expect(dialog.style.left).toBe('72px');
    expect(dialog.style.right).toBe('232px');
    expect(dialog.style.top).toBe('0px');
    // It ends where the last strip's time labels start, so they stay readable.
    expect(dialog.style.bottom).toBe('40px');
    // Axes and legends stay outside it.
    for (const legend of document.querySelectorAll('.legend')) expect(dialog.contains(legend)).toBe(false);
  });

  test('restore defaults charts and saves the page defaults again', async () => {
    await settings.update({ advanced: { series: { [GPU]: [VRAM] } } });
    const backend = fakeBackend();
    renderChart(backend);
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    await fireEvent.click(seriesButton());
    await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.restoreDefaults') }));
    await vi.waitFor(() => expect(backend.historyCalls.at(-1)?.ids).toEqual([LOAD, TEMP]));
    expect(loadSeries(GPU)).toEqual([LOAD, TEMP]);
    expect(checkbox(labelOf(byId(VRAM))).checked).toBe(false);
  });

  test('drawer_closes_on_escape_outside_click_and_button_and_returns_focus', async () => {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));

    await fireEvent.click(seriesButton());
    await fireEvent.keyDown(screen.getByRole('searchbox'), { key: 'Escape' });
    expect(drawer()).toBeNull();
    expect(document.activeElement).toBe(seriesButton());

    await fireEvent.click(seriesButton());
    // A click inside keeps it open.
    await fireEvent.click(drawer()!.querySelector('[role="group"]')!);
    expect(drawer()).toBeTruthy();
    await fireEvent.click(document.body);
    expect(drawer()).toBeNull();
    expect(document.activeElement).toBe(seriesButton());

    await fireEvent.click(seriesButton());
    expect(drawer()).toBeTruthy();
    await fireEvent.click(seriesButton());
    expect(drawer()).toBeNull();
    expect(document.activeElement).toBe(seriesButton());
  });

  test('search_has_focus_on_open', async () => {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    await fireEvent.click(seriesButton());
    const search = screen.getByRole('searchbox');
    expect(document.activeElement).toBe(search);
    expect(search.getAttribute('placeholder')).toBe(t('advanced.chart.search'));
  });

  test('typing_in_the_search_filters_the_rows', async () => {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    openDrawer();
    expect(screen.getAllByRole('checkbox')).toHaveLength(gpuSensors.length);
    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'TEMPERATÚRE' } });
    expect(screen.getAllByRole<HTMLInputElement>('checkbox').map((box) => box.getAttribute('aria-label')))
      .toEqual([labelOf(byId(TEMP)), labelOf(byId(HOTSPOT))]);
    expect(groupNames()).toEqual([legendTitle('celsius', '°C')]);
  });

  test('all_and_none_set_only_the_filtered_rows_and_save', async () => {
    const backend = fakeBackend();
    renderChart(backend);
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    openDrawer();
    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'hotspot' } });
    // The legend of the temperature strip is a group of the same name, outside the drawer.
    const inDrawer = () => within(drawer()!).getByRole('group', { name: legendTitle('celsius', '°C') });
    await fireEvent.click(within(inDrawer()).getByRole('button', { name: t('advanced.chart.all') }));
    await vi.waitFor(() => expect(backend.historyCalls.at(-1)?.ids).toEqual([LOAD, TEMP, HOTSPOT]));
    await vi.waitFor(() => expect(settings.state?.settings.advanced.series[GPU]).toEqual([LOAD, TEMP, HOTSPOT]));
    expect(seriesButton().textContent?.trim()).toBe(`${t('advanced.chart.series')} · 3`);

    await fireEvent.input(screen.getByRole('searchbox'), { target: { value: 'temp' } });
    await fireEvent.click(within(inDrawer()).getByRole('button', { name: t('advanced.chart.none') }));
    await vi.waitFor(() => expect(settings.state?.settings.advanced.series[GPU]).toEqual([LOAD]));
    await vi.waitFor(() => expect(live().map((p) => p.opts.series[1].scale)).toEqual(['percent']));
    expect(drawer()).toBeTruthy();
  });

  test('ticking_a_new_unit_adds_a_strip_while_open', async () => {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    openDrawer();
    expect(groupNames().slice(0, 2)).toEqual([legendTitle('percent', '%'), legendTitle('celsius', '°C')]);
    await fireEvent.click(checkbox(labelOf(byId(VRAM))));
    await vi.waitFor(() => expect(plots).toHaveLength(5));
    expect(live().map((p) => p.opts.series[1].scale)).toEqual(['percent', 'celsius', 'bytes']);
    expect(drawer()).toBeTruthy();
    expect(checkbox(labelOf(byId(VRAM))).checked).toBe(true);
    // The strips' units come first in the drawer, in strip order, named like the legends.
    expect(groupNames().slice(0, 3)).toEqual([legendTitle('percent', '%'), legendTitle('celsius', '°C'), legendTitle('bytes')]);
    expect(screen.getAllByRole('group', { name: legendTitle('bytes') }).filter((g) => !drawer()!.contains(g))).toHaveLength(1);
  });

  test('unticking_in_the_drawer_drops_an_empty_strip', async () => {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    hover(plots[0], 0);
    expect(legendCell(byId(LOAD), 'now')).toBe('10%');
    openDrawer();
    await fireEvent.click(checkbox(labelOf(byId(TEMP))));
    await vi.waitFor(() => expect(plots).toHaveLength(3));
    expect(live().map((p) => p.opts.series[1].scale)).toEqual(['percent']);
    expect(document.querySelectorAll('.strip')).toHaveLength(1);
    expect(document.querySelectorAll('.legend')).toHaveLength(1);
    // The cursor index went back to null: the legend shows the latest reading, none yet.
    expect(legendCell(byId(LOAD), 'now')).toBe('—');
    expect(drawer()).toBeTruthy();
    expect(checkbox(labelOf(byId(TEMP))).checked).toBe(false);
  });

  test('chart_grows_to_360px_while_the_drawer_is_open', async () => {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    const legends = () => [...document.querySelectorAll<HTMLElement>('.strip .legend')].map((l) => l.style.height);
    const [first, last] = live();
    expect(legends()).toEqual(['130px', '170px']);
    openDrawer();
    // 130 + 170 px: the last strip takes the 60 px missing to 360, so its time labels stay under the drawer.
    expect(last.sizes.at(-1)).toEqual({ width: last.width, height: 230 });
    expect(last.height).toBe(230);
    expect(first.sizes).toEqual([]);
    expect(legends()).toEqual(['130px', '230px']);
    expect(document.querySelector<HTMLElement>('.strips')!.style.minHeight).toBe('');
    await fireEvent.click(seriesButton());
    expect(last.height).toBe(170);
    expect(legends()).toEqual(['130px', '170px']);
    expect(live()).toEqual([first, last]);
  });

  test('a_single_strip_grows_to_360px_with_the_drawer', async () => {
    renderChart(fakeBackend(), new LiveStore(), gpuSensors, [TEMP]);
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    openDrawer();
    expect(plots[0].height).toBe(360);
  });

  test('the_drawer_opens_with_no_series_chosen', async () => {
    await settings.update({ advanced: { series: { [GPU]: [] } } });
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(screen.getByText(t('advanced.chart.empty'))).toBeTruthy());
    openDrawer();
    expect(drawer()!.parentElement!.classList.contains('strips')).toBe(true);
    await fireEvent.click(checkbox(labelOf(byId(LOAD))));
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    // The same drawer stays open, focus and all.
    expect(drawer()).toBeTruthy();
  });

  test('button_reports_aria_expanded_and_count', async () => {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    const button = seriesButton();
    expect(button.textContent?.trim()).toBe(`${t('advanced.chart.series')} · 2`);
    expect(button.getAttribute('aria-expanded')).toBe('false');
    // Only an open drawer is in the document to be controlled.
    expect(button.hasAttribute('aria-controls')).toBe(false);
    openDrawer();
    expect(button.getAttribute('aria-expanded')).toBe('true');
    expect(button.getAttribute('aria-controls')).toBe(drawer()!.id);
    await fireEvent.click(checkbox(labelOf(byId(VRAM))));
    expect(button.textContent?.trim()).toBe(`${t('advanced.chart.series')} · 3`);
    await fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(button.hasAttribute('aria-controls')).toBe(false);
  });

  test('rows_show_the_value_now_like_the_legend', async () => {
    await connectSettings({ general: { temperatureUnit: 'f' } });
    const store = new LiveStore();
    renderChart(fakeBackend(), store);
    await vi.waitFor(() => expect(plots).toHaveLength(2));
    const values = MOCK_SCHEMA.sensors.map((s) => (s.id === TEMP ? 100 : s.id === HOTSPOT ? 0 : 42));
    store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values });
    flushSync();
    openDrawer();
    const value = (s: Sensor) => checkbox(labelOf(s)).closest('li')!.querySelector('.value')!.textContent;
    expect(value(byId(LOAD))).toBe(legendCell(byId(LOAD), 'now'));
    expect(value(byId(TEMP))).toBe('212 °F');
    expect(value(byId(TEMP))).toBe(legendCell(byId(TEMP), 'now'));
    // A sensor outside the chart shows its value too, in the display units.
    expect(value(byId(HOTSPOT))).toBe('32 °F');
    store.applySnapshot({ revision: 1, seq: 2, timestampMs: 4000, values: values.map((v) => (v === 42 ? 7 : v)) });
    flushSync();
    expect(value(byId(LOAD))).toBe('7%');
  });
});
