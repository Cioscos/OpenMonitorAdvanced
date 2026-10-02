import { cleanup, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import type { SidebarEntry } from '../../lib/advanced/nav';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { formatValue } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import type { GpuProcess, SensorStats, ServiceStatus } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
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

beforeEach(async () => {
  plots.length = 0;
  await connectSettings();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

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

test('a disk whose only property is the SMART switch has no info box', async () => {
  const { backend, store } = setup();
  const disk = MOCK_SCHEMA.devices.find((d) => d.kind === 'storage')!;
  store.applySchema({
    ...MOCK_SCHEMA,
    revision: MOCK_SCHEMA.revision + 1,
    devices: MOCK_SCHEMA.devices.map((d) => (d.id === disk.id ? { ...d, properties: { smartSelectable: 'true' } } : d)),
  });
  render(DevicePage, { entry: { id: disk.id, kind: 'storage', labelKey: 'advanced.section.storage', deviceIds: [disk.id], labelArg: disk.name }, store, backend });
  expect(screen.queryByText(t('advanced.info.title'))).toBeNull();
});

const SSD = 'storage/device-mock-ssd';
const SSD_ENTRY: SidebarEntry = { id: SSD, kind: 'storage', deviceIds: [SSD], labelKey: 'advanced.section.storage', labelArg: 'Disk 0 (C:)' };
const stateLabels = () => [...document.querySelectorAll('.disk-state')].map((e) => e.textContent);

test('a disk in standby shows its state', () => {
  const { backend, store } = setup();
  store.setDiskStates([{ deviceId: SSD, power: 'standby' }]);
  render(DevicePage, { entry: SSD_ENTRY, store, backend });
  expect(stateLabels()).toEqual([t('storage.power.standby')]);
  expect(document.querySelector('.disk-state')!.classList.contains('tag')).toBe(true);
});

test('an idle disk shows "Inattivo"', () => {
  i18n.locale = 'it';
  const { backend, store } = setup();
  store.setDiskStates([{ deviceId: SSD, power: 'idle' }]);
  render(DevicePage, { entry: SSD_ENTRY, store, backend });
  expect(stateLabels()).toEqual(['Inattivo']);
});

test('an active disk shows no state label', () => {
  const { backend, store } = setup();
  store.setDiskStates([{ deviceId: SSD, power: 'active' }]);
  render(DevicePage, { entry: SSD_ENTRY, store, backend });
  expect(stateLabels()).toEqual([]);
});

test('an unknown or removed disk keeps no state label', () => {
  const { backend, store } = setup();
  store.setDiskStates([{ deviceId: SSD, power: 'standby' }]);
  render(DevicePage, { entry: SSD_ENTRY, store, backend });
  expect(stateLabels()).toHaveLength(1);
  // The state is read live: it follows the store, not a discovery property.
  store.setDiskStates([{ deviceId: SSD, power: 'unknown' }]);
  flushSync();
  expect(stateLabels()).toEqual([]);
  store.setDiskStates([{ deviceId: SSD, power: 'idle' }]);
  flushSync();
  expect(stateLabels()).toHaveLength(1);
  store.setDiskStates([]);
  flushSync();
  expect(stateLabels()).toEqual([]);
});

test('only a disk page shows a power state', () => {
  const { backend, store } = setup();
  store.setDiskStates([{ deviceId: 'cpu/0', power: 'standby' }]);
  render(DevicePage, { entry: CPU_ENTRY, store, backend });
  expect(stateLabels()).toEqual([]);
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
  const cells = () => [...name.closest('tr')!.querySelectorAll('td.num')].map((td) => td.textContent!);
  await vi.waitFor(() => expect(cells().every((c) => c.endsWith('bit/s'))).toBe(true));
});

test('network pages follow the throughput setting and disks stay in bytes', async () => {
  await connectSettings({ general: { throughputUnit: 'bytes' } });
  const { backend, store } = setup();
  const NIC = 'network/mock-eth';
  const DISK = 'storage/device-mock-ssd';
  const NIC_ENTRY: SidebarEntry = { id: NIC, kind: 'network', deviceIds: [NIC], labelKey: 'advanced.section.network', labelArg: 'Ethernet' };
  const DISK_ENTRY: SidebarEntry = { id: DISK, kind: 'storage', deviceIds: [DISK], labelKey: 'advanced.section.storage', labelArg: 'Disk 0 (C:)' };
  const tableCells = (label: string) => {
    const name = [...document.querySelectorAll('.sensors th .name')].find((n) => n.textContent === label)!;
    return [...name.closest('tr')!.querySelectorAll('td.num')].map((td) => td.textContent!);
  };

  const nic = render(DevicePage, { entry: NIC_ENTRY, store, backend });
  await vi.waitFor(() => expect(kpiValues()[3]).toBe(formatValue(STATS.max, 'bytes_per_second', 'en', t, { rate: 'bytes' })));
  expect(kpiValues()[0]).not.toMatch(/bit\/s$/);
  expect(tableCells(t('sensor.network.down')).every((c) => /[A-Z]B\/s$|^\d+ B\/s$/.test(c))).toBe(true);
  // The setting changes the open page at once.
  await settings.update({ general: { throughputUnit: 'bits' } });
  await vi.waitFor(() => expect(kpiValues()[0]).toMatch(/bit\/s$/));
  nic.unmount();

  const disk = render(DevicePage, { entry: DISK_ENTRY, store, backend });
  const readRow = () => tableCells(t('sensor.storage.read'));
  // Value, min, max and average: wait for the statistics too.
  await vi.waitFor(() => expect(readRow().every((c) => c !== '—')).toBe(true));
  expect(readRow().every((c) => !c.includes('bit/s') && c.endsWith('/s'))).toBe(true);
  disk.unmount();
});

test('a network page hands the chart the throughput setting, a disk page always bytes', async () => {
  const { backend, store } = setup();
  const NIC = 'network/mock-eth';
  const DISK = 'storage/device-mock-ssd';
  const nic = render(DevicePage, { entry: { id: NIC, kind: 'network', deviceIds: [NIC], labelKey: 'advanced.section.network' }, store, backend });
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const axis = plots[0].opts.axes!.find((a) => a.scale === 'bytes_per_second')!;
  const label = (axis.values as (u: unknown, splits: number[]) => string[])(plots[0], [8e6])[0];
  expect(label).toBe('8.0 Mbit/s');
  nic.unmount();

  plots.length = 0;
  render(DevicePage, { entry: { id: DISK, kind: 'storage', deviceIds: [DISK], labelKey: 'advanced.section.storage' }, store, backend });
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  const diskAxis = plots[0].opts.axes!.find((a) => a.scale === 'bytes_per_second')!;
  expect((diskAxis.values as (u: unknown, splits: number[]) => string[])(plots[0], [8 * 1024 ** 2])[0]).toBe('8.0 MB/s');
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

test('the generic notice appears on CPU, memory and disk pages only without the service', async () => {
  const { backend, store } = setup();
  const NOTICE = t('service.pageNotice');
  const MEMORY = 'memory/0';
  const DISK = 'storage/device-mock-ssd';
  const NIC = 'network/mock-eth';
  const MEMORY_ENTRY: SidebarEntry = { id: MEMORY, kind: 'memory', deviceIds: [MEMORY], labelKey: 'advanced.section.memory' };
  const DISK_ENTRY: SidebarEntry = { id: DISK, kind: 'storage', deviceIds: [DISK], labelKey: 'advanced.section.storage', labelArg: 'Disk 0 (C:)' };
  const NIC_ENTRY: SidebarEntry = { id: NIC, kind: 'network', deviceIds: [NIC], labelKey: 'advanced.section.network', labelArg: 'Ethernet' };
  const notConnected: ServiceStatus = { state: 'unreachable', detail: null, pawnIo: null, sources: null };
  const connected: ServiceStatus = { state: 'connected', detail: null, pawnIo: null, sources: null };

  const { unmount: u1 } = render(DevicePage, { entry: CPU_ENTRY, store, backend, service: notConnected });
  expect(screen.getByText(NOTICE)).toBeTruthy();
  u1();

  const { unmount: u1b } = render(DevicePage, { entry: MEMORY_ENTRY, store, backend, service: notConnected });
  expect(screen.getByText(NOTICE)).toBeTruthy();
  u1b();

  const { unmount: u2 } = render(DevicePage, { entry: DISK_ENTRY, store, backend, service: notConnected });
  expect(screen.getByText(NOTICE)).toBeTruthy();
  u2();

  const { unmount: u3 } = render(DevicePage, { entry: GPU_ENTRY, store, backend, service: notConnected });
  expect(screen.queryByText(NOTICE)).toBeNull();
  u3();

  const { unmount: u4 } = render(DevicePage, { entry: NIC_ENTRY, store, backend, service: notConnected });
  expect(screen.queryByText(NOTICE)).toBeNull();
  u4();

  render(DevicePage, { entry: CPU_ENTRY, store, backend, service: connected });
  expect(screen.queryByText(NOTICE)).toBeNull();
});

test('the advanced view mounts the full page under its heading', async () => {
  await settings.update({ advanced: { section: GPU } });
  const { backend, store } = setup();
  render(AdvancedView, { store, backend });

  expect(screen.getByRole('heading', { level: 2 }).textContent).toBe(t('advanced.section.gpu'));
  expect(kpiLabels()).toHaveLength(4);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  await vi.waitFor(() => expect(screen.getByText('game.exe')).toBeTruthy());
});
