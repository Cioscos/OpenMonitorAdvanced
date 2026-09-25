import { MOCK_SCHEMA, mockValues } from './backend/mock';
import {
  cpuSummary,
  gpuSummaries,
  memorySummary,
  networkSummary,
  simpleViewGpus,
  storageSummary,
  sumSeries,
  type GpuSummary,
} from './select';
import type { Schema } from './types';

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
