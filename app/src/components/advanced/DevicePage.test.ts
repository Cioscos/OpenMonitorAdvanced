import { cleanup, render, screen } from '@testing-library/svelte';
import type { SidebarEntry } from '../../lib/advanced/nav';
import { SECTION_KEY } from '../../lib/advanced/persist';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { formatValue } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { GpuProcess, SensorStats } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { FakeUplot } from '../../test/uplot-stub';
import AdvancedView from './AdvancedView.svelte';
import DevicePage from './DevicePage.svelte';

const plots = FakeUplot.instances;
const GPU = 'gpu/pci-0000:01:00.0';
const GPU_ENTRY: SidebarEntry = { id: GPU, kind: 'gpu', deviceIds: [GPU], labelKey: 'advanced.section.gpu', labelArg: 'Mock GeForce RTX 4080' };
const CPU_ENTRY: SidebarEntry = { id: 'cpu/0', kind: 'cpu', deviceIds: ['cpu/0'], labelKey: 'advanced.section.cpu', labelArg: 'Mock Ryzen 7 7800X3D' };
const GAME: GpuProcess = { pid: 4242, name: 'game.exe', loadPercent: 87, engine: '3D', dedicatedBytes: 1024 ** 3, sharedBytes: 0 };
const STATS: SensorStats = { min: 1, max: 2, avg: 1.5, count: 2 };
const idsOf = (deviceId: string) => MOCK_SCHEMA.sensors.filter((s) => s.deviceId === deviceId).map((s) => s.id);

beforeEach(() => {
  plots.length = 0;
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(cleanup);

function setup() {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.stats = Object.fromEntries(MOCK_SCHEMA.sensors.map((s) => [s.id, STATS]));
  backend.gpuProcesses = [GAME];
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 2000, values: mockValues(1) });
  return { backend, store };
}

const kpiLabels = () => [...document.querySelectorAll('.kpi .label')].map((e) => e.textContent);
const kpiValues = () => [...document.querySelectorAll('.kpi .value')].map((e) => e.textContent);

test('gpu page: kpis, chart, sensor table, properties and processes', async () => {
  const { backend, store } = setup();
  render(DevicePage, { entry: GPU_ENTRY, store, backend });

  expect(kpiLabels()).toEqual(['load', 'temperature', 'power', 'vram'].map((id) => t(`advanced.kpi.${id}`)));
  const load = mockValues(1)[MOCK_SCHEMA.sensors.findIndex((s) => s.id === `${GPU}/load/core`)];
  expect(kpiValues()[0]).toBe(formatValue(load, 'percent', 'en', t));
  expect(screen.getByText(t('advanced.kpi.vramOf', { total: '16.0 GB' }))).toBeTruthy();

  await vi.waitFor(() => expect(backend.statsCalls[0]).toEqual(idsOf(GPU)));
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(backend.historyCalls[0]).toEqual({ ids: [`${GPU}/load/core`, `${GPU}/temperature/core`], seconds: 300, maxPoints: undefined });
  expect(screen.getByRole('button', { name: t('advanced.table.reset') })).toBeTruthy();

  expect(screen.getByText(t('advanced.info.title'))).toBeTruthy();
  expect(screen.getByText('0000:01:00.0')).toBeTruthy();
  expect(backend.gpuProcessCalls[0]).toBe(GPU);
  await vi.waitFor(() => expect(screen.getByText('game.exe')).toBeTruthy());
  expect(screen.queryByRole('heading', { level: 2 })).toBeNull();
});

test('cpu page: peak load from the core statistics, no process list, no empty info box', async () => {
  const { backend, store } = setup();
  render(DevicePage, { entry: CPU_ENTRY, store, backend });

  expect(kpiLabels()).toEqual(['load', 'clock', 'busiestThread', 'peakLoad'].map((id) => t(`advanced.kpi.${id}`)));
  await vi.waitFor(() => expect(kpiValues()[3]).toBe(formatValue(STATS.max, 'percent', 'en', t)));
  expect(backend.statsCalls[0]).toEqual(idsOf('cpu/0'));
  expect(backend.gpuProcessCalls).toEqual([]);
  expect(screen.queryByText(t('advanced.processes.title'))).toBeNull();
  expect(screen.queryByText(t('advanced.info.title'))).toBeNull();
});

test('network page: traffic in bits per second, like the Simple view', async () => {
  const { backend, store } = setup();
  const NIC = 'network/mock-eth';
  const entry: SidebarEntry = { id: NIC, kind: 'network', deviceIds: [NIC], labelKey: 'advanced.section.network', labelArg: 'Ethernet' };
  render(DevicePage, { entry, store, backend });

  const down = mockValues(1)[MOCK_SCHEMA.sensors.findIndex((s) => s.id === `${NIC}/throughput/down`)];
  expect(kpiValues()[0]).toBe(formatValue(down, 'bytes_per_second', 'en', t, { rate: 'bits' }));
  expect(kpiValues()[0]).toMatch(/bit\/s$/);
  await vi.waitFor(() => expect(kpiValues()[3]).toBe(formatValue(STATS.max, 'bytes_per_second', 'en', t, { rate: 'bits' })));
  // The label also appears in the KPI row and in the series picker: take the table row.
  const name = [...document.querySelectorAll('.sensors th .name')].find((n) => n.textContent === t('sensor.network.down'))!;
  const cells = () => [...name.closest('tr')!.querySelectorAll('td')].map((td) => td.textContent!);
  await vi.waitFor(() => expect(cells().every((c) => c.endsWith('bit/s'))).toBe(true));
});

test('leaving the page stops the statistics polling and destroys the chart', async () => {
  vi.useFakeTimers();
  try {
    const { backend, store } = setup();
    const { unmount } = render(DevicePage, { entry: CPU_ENTRY, store, backend });
    await vi.advanceTimersByTimeAsync(2000);
    const calls = backend.statsCalls.length;
    expect(calls).toBeGreaterThanOrEqual(2);
    unmount();
    await vi.advanceTimersByTimeAsync(5000);
    expect(backend.statsCalls).toHaveLength(calls);
    expect(plots.every((p) => p.destroyed)).toBe(true);
  } finally {
    vi.useRealTimers();
  }
});

test('the advanced view mounts the full page under its heading', async () => {
  localStorage.setItem(SECTION_KEY, GPU);
  const { backend, store } = setup();
  render(AdvancedView, { store, backend });

  expect(screen.getByRole('heading', { level: 2 }).textContent).toBe(t('advanced.section.gpu'));
  expect(kpiLabels()).toHaveLength(4);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  await vi.waitFor(() => expect(screen.getByText('game.exe')).toBeTruthy());
});
