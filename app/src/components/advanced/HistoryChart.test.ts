import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { WINDOW_KEY, seriesKey } from '../../lib/advanced/persist';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { HistorySeed, Sensor } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { FakeUplot } from '../../test/uplot-stub';
import HistoryChart from './HistoryChart.svelte';

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
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
const setVisibility = (state: DocumentVisibilityState) => {
  visibility = state;
  document.dispatchEvent(new Event('visibilitychange'));
};

beforeEach(() => {
  plots.length = 0;
  localStorage.clear();
  i18n.locale = 'en';
  visibility = 'visible';
});
afterEach(cleanup);

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

test('live snapshots extend the chart and old points leave the window', async () => {
  localStorage.setItem(WINDOW_KEY, '60');
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 62_000, values: mockValues(1) });
  flushSync();
  const plot = plots[0];
  expect(plot.setDataCalls).toBe(1);
  // 1000 ms is older than 62 s - 60 s and leaves; 2000 ms is exactly on the edge and stays.
  expect(plot.data).toEqual([
    [2, 62],
    [20, mockValues(1)[index(LOAD)]],
    [21, mockValues(1)[index(TEMP)]],
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
