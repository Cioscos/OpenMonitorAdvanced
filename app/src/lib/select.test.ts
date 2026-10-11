import { MOCK_BATTERY_SCHEMA, MOCK_SCHEMA, mockBatteryValues, mockValues } from './backend/mock';
import {
  batterySummary,
  cpuSummary,
  gpuSummaries,
  memorySummary,
  networkSummary,
  diskRows,
  simpleViewGpus,
  storageSummary,
  sumSeries,
  type GpuSummary,
} from './select';
import type { Schema, Sensor } from './types';

const values = mockValues(1);
const valueOf = (id: string) => {
  const i = MOCK_SCHEMA.sensors.findIndex((s) => s.id === id);
  return i < 0 ? null : values[i];
};

test('cpu summary', () => {
  const cpu = cpuSummary(MOCK_SCHEMA, valueOf)!;
  expect(cpu.name).toBe('Mock Ryzen 7 7800X3D');
  expect(cpu.loadId).toBe('cpu/0/load/total');
  expect(cpu.load).toBe(values[0]);
  expect(cpu.clockMhz).toBe(valueOf('cpu/0/clock/effective'));
});

test('gpu summaries read the core sensors of each GPU', () => {
  const [gpu] = gpuSummaries(MOCK_SCHEMA, valueOf);
  expect(gpu).toEqual({
    deviceId: 'gpu/pci-0000:01:00.0',
    name: 'Mock GeForce RTX 4080',
    integrated: false,
    load: valueOf('gpu/pci-0000:01:00.0/load/core'),
    loadId: 'gpu/pci-0000:01:00.0/load/core',
    temperatureC: valueOf('gpu/pci-0000:01:00.0/temperature/core'),
    clockMhz: valueOf('gpu/pci-0000:01:00.0/clock/core'),
    powerW: valueOf('gpu/pci-0000:01:00.0/power/board'),
    memUsedBytes: valueOf('gpu/pci-0000:01:00.0/data/memory-dedicated-used'),
    memTotalBytes: 16 * 1024 ** 3,
    hotspotC: valueOf('gpu/pci-0000:01:00.0/temperature/hotspot'),
    powerLimitW: null,
    fanPct: null,
    fanRpm: null,
    throttle: null,
  });
});

test('gpu summaries report missing sensors as null', () => {
  const schema: Schema = {
    revision: 1,
    devices: [{ id: 'gpu/pci-0000:11:00.0', kind: 'gpu', name: 'iGPU', properties: { integrated: 'true' } }],
    sensors: [],
  };
  expect(gpuSummaries(schema, valueOf)).toEqual([
    {
      deviceId: 'gpu/pci-0000:11:00.0',
      name: 'iGPU',
      integrated: true,
      load: null,
      loadId: null,
      temperatureC: null,
      clockMhz: null,
      powerW: null,
      memUsedBytes: null,
      memTotalBytes: null,
      hotspotC: null,
      powerLimitW: null,
      fanPct: null,
      fanRpm: null,
      throttle: null,
    },
  ]);
});

const gpu = (deviceId: string, integrated: boolean): GpuSummary => ({
  deviceId,
  name: deviceId,
  integrated,
  load: null,
  loadId: null,
  temperatureC: null,
  clockMhz: null,
  powerW: null,
  memUsedBytes: null,
  memTotalBytes: null,
  hotspotC: null,
  powerLimitW: null,
  fanPct: null,
  fanRpm: null,
  throttle: null,
});

test('simple view keeps discrete GPUs and hides the integrated one', () => {
  const list = [gpu('nvidia', false), gpu('igpu', true), gpu('second', false)];
  expect(simpleViewGpus(list).map((g) => g.deviceId)).toEqual(['nvidia', 'second']);
});

test('simple view falls back to integrated GPUs when there is no discrete one', () => {
  expect(simpleViewGpus([gpu('igpu', true)]).map((g) => g.deviceId)).toEqual(['igpu']);
  expect(simpleViewGpus([])).toEqual([]);
});

test('memory summary', () => {
  const mem = memorySummary(MOCK_SCHEMA, valueOf)!;
  expect(mem.totalBytes).toBe(32 * 1024 ** 3);
  expect(mem.usedPct).toBe(valueOf('memory/0/load/used'));
});

test('storage summary prefers the C: volume', () => {
  const disk = storageSummary(MOCK_SCHEMA, valueOf)!;
  expect(disk.volume).toEqual({ letter: 'C:', usedPct: 65 });
  expect(disk.readBps).toBe(valueOf('storage/device-mock-ssd/throughput/read'));
});

test('network summary sums every adapter', () => {
  const schema: Schema = {
    ...MOCK_SCHEMA,
    devices: [...MOCK_SCHEMA.devices, { id: 'network/wifi', kind: 'network', name: 'Wi-Fi' }],
    sensors: [
      ...MOCK_SCHEMA.sensors,
      { id: 'network/wifi/throughput/down', deviceId: 'network/wifi', kind: 'throughput', unit: 'bytes_per_second', label: { key: 'network.down' }, source: 'mock', category: 'throughput' },
    ],
  };
  const extra = (id: string) => (id === 'network/wifi/throughput/down' ? 100 : valueOf(id));
  const net = networkSummary(schema, extra)!;
  expect(net.downBps).toBe((valueOf('network/mock-eth/throughput/down') ?? 0) + 100);
  expect(net.downIds).toEqual(['network/mock-eth/throughput/down', 'network/wifi/throughput/down']);
});

test('summaries are null when the device kind is missing', () => {
  const empty: Schema = { revision: 1, devices: [], sensors: [] };
  expect(cpuSummary(empty, valueOf)).toBeNull();
  expect(memorySummary(empty, valueOf)).toBeNull();
  expect(storageSummary(empty, valueOf)).toBeNull();
  expect(networkSummary(empty, valueOf)).toBeNull();
  expect(gpuSummaries(empty, valueOf)).toEqual([]);
});

test('sumSeries right-aligns and ignores gaps', () => {
  expect(sumSeries([[1, 2, 3], [10, NaN, 30]])).toEqual([11, 2, 33]);
  expect(sumSeries([[1, 2], [5, 6, 7]])).toEqual([5, 7, 9]);
  expect(Number.isNaN(sumSeries([[NaN]])[0])).toBe(true);
  expect(sumSeries([])).toEqual([]);
});

const sensorOf = (deviceId: string, key: string, arg?: string): Sensor => ({
  id: `${deviceId}/x/${key}${arg ?? ''}`,
  deviceId,
  kind: 'load',
  unit: 'percent',
  label: { key, arg },
  source: 'mock',
  category: 'load',
});
const withSensors = (devices: Schema['devices'], sensors: Schema['sensors']): Schema => ({ revision: 1, devices, sensors });

test('cpu_summary_has_temperature_power_and_threads', () => {
  const cpuDev = { id: 'cpu/0', kind: 'cpu' as const, name: 'CPU' };
  const schema = withSensors([cpuDev], [
    sensorOf('cpu/0', 'cpu.temperature.tctl'),
    sensorOf('cpu/0', 'cpu.power.package'),
    sensorOf('cpu/0', 'cpu.load.thread', '0'),
    sensorOf('cpu/0', 'cpu.load.thread', '1'),
  ]);
  const v: Record<string, number> = { 'cpu/0/x/cpu.temperature.tctl': 61, 'cpu/0/x/cpu.power.package': 88, 'cpu/0/x/cpu.load.thread0': 20, 'cpu/0/x/cpu.load.thread1': 90 };
  const cpu = cpuSummary(schema, (id) => v[id] ?? null)!;
  expect(cpu.temperatureC).toBe(61);
  expect(cpu.powerW).toBe(88);
  expect(cpu.busiestThread).toBe(90);
  expect(cpu.threads.map((th) => th.value)).toEqual([20, 90]);
  const bare = cpuSummary(withSensors([cpuDev], []), () => null)!;
  expect([bare.temperatureC, bare.powerW, bare.busiestThread, bare.threads]).toEqual([null, null, null, []]);
});

test('gpu_summary_reports_throttle_only_when_set', () => {
  const dev = { id: 'gpu/a', kind: 'gpu' as const, name: 'G' };
  const schema = withSensors([dev], [sensorOf('gpu/a', 'gpu.throttle.power'), sensorOf('gpu/a', 'gpu.throttle.thermal')]);
  const at = (power: number, thermal: number) =>
    gpuSummaries(schema, (id) => (id.endsWith('power') ? power : thermal))[0].throttle;
  expect(at(0, 0)).toBeNull();
  expect(at(1, 0)).toBe('power');
  expect(at(0, 1)).toBe('thermal');
});

test('memory_available_never_negative', () => {
  const dev = { id: 'memory/0', kind: 'memory' as const, name: 'RAM' };
  const schema = withSensors([dev], [sensorOf('memory/0', 'memory.used'), sensorOf('memory/0', 'memory.total'), sensorOf('memory/0', 'memory.temperature.dimm', '0')]);
  const mem = (used: number) => memorySummary(schema, (id) => (id.includes('used') ? used : id.includes('total') ? 100 : null))!;
  expect(mem(30).availableBytes).toBe(70);
  expect(mem(130).availableBytes).toBe(0);
  expect(mem(30).dimmTempsC).toEqual([]);
});

test('disk_rows_follow_schema_order_with_letters', () => {
  const devs = [
    { id: 'storage/b', kind: 'storage' as const, name: 'B' },
    { id: 'storage/a', kind: 'storage' as const, name: 'A' },
  ];
  const schema = withSensors(devs, [sensorOf('storage/a', 'storage.volumeUsed', 'D:'), sensorOf('storage/b', 'storage.volumeUsed', 'C:')]);
  const rows = diskRows(schema, () => 5, (id) => (id === 'storage/a' ? 'standby' : 'active'));
  expect(rows.map((r) => [r.name, r.letter, r.standby])).toEqual([['B', 'C:', false], ['A', 'D:', true]]);
  expect(rows[0].temperatureC).toBeNull();
});

test('network_summary_has_adapter_and_link', () => {
  const net = networkSummary(MOCK_SCHEMA, valueOf)!;
  expect(net.adapterName).toBe('Ethernet');
  expect(net.linkBps).toBe(valueOf('network/mock-eth/throughput/link-speed'));
});

const BAT = 'battery/0';
const batteryOf = (overrides: Record<string, number | null> = {}) => {
  const base = mockBatteryValues(1);
  return (id: string) => {
    if (id in overrides) return overrides[id];
    const i = MOCK_BATTERY_SCHEMA.sensors.findIndex((s) => s.id === id);
    return i < 0 ? null : base[i];
  };
};

test('battery_summary_states', () => {
  const summary = (o: Record<string, number | null>) => batterySummary(MOCK_BATTERY_SCHEMA, batteryOf(o))!;
  expect(summary({})).toEqual({ deviceId: BAT, chargePct: 64, chargeId: `${BAT}/percent/charge`, state: 'discharging', secondsLeft: 5400, rateW: -12.5 });
  expect(summary({ [`${BAT}/flag/charging`]: 1, [`${BAT}/flag/ac-online`]: 1 }).state).toBe('charging');
  // Plugged in and full: not charging, but not discharging either.
  const full = summary({ [`${BAT}/flag/charging`]: 0, [`${BAT}/flag/ac-online`]: 1, [`${BAT}/duration/remaining`]: null });
  expect(full.state).toBe('plugged');
  expect(full.secondsLeft).toBeNull();
  // Without the AC flag nothing says the battery is discharging.
  expect(summary({ [`${BAT}/flag/ac-online`]: null, [`${BAT}/flag/charging`]: 0 }).state).toBe('plugged');
});

test('no_battery_no_summary', () => {
  expect(batterySummary(MOCK_SCHEMA, valueOf)).toBeNull();
});
