import { MOCK_SCHEMA, mockValues } from './backend/mock';
import { cpuSummary, memorySummary, networkSummary, storageSummary, sumSeries } from './select';
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
});

test('sumSeries right-aligns and ignores gaps', () => {
  expect(sumSeries([[1, 2, 3], [10, NaN, 30]])).toEqual([11, 2, 33]);
  expect(sumSeries([[1, 2], [5, 6, 7]])).toEqual([5, 7, 9]);
  expect(Number.isNaN(sumSeries([[NaN]])[0])).toBe(true);
  expect(sumSeries([])).toEqual([]);
});
