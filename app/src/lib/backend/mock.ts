import type { Label, Schema, Sensor, SensorKind, Snapshot, Unit } from '../types';
import type { Backend } from './backend';

const THREADS = 8;
const GIB = 1024 ** 3;

const sensor = (id: string, deviceId: string, kind: SensorKind, unit: Unit, label: Label): Sensor => ({
  id,
  deviceId,
  kind,
  unit,
  label,
  source: 'mock',
  category: kind,
});

/** Same ids and label keys the Rust providers produce (crates/oma-win). */
export const MOCK_SCHEMA: Schema = {
  revision: 1,
  devices: [
    { id: 'cpu/0', kind: 'cpu', name: 'Mock Ryzen 7 7800X3D' },
    { id: 'memory/0', kind: 'memory', name: 'RAM' },
    { id: 'storage/device-mock-ssd', kind: 'storage', name: 'Disk 0 (C:)' },
    { id: 'network/mock-eth', kind: 'network', name: 'Ethernet' },
  ],
  sensors: [
    sensor('cpu/0/load/total', 'cpu/0', 'load', 'percent', { key: 'cpu.load.total' }),
    ...Array.from({ length: THREADS }, (_, i) =>
      sensor(`cpu/0/load/thread-0-${i}`, 'cpu/0', 'load', 'percent', { key: 'cpu.load.thread', arg: String(i) }),
    ),
    sensor('cpu/0/clock/effective', 'cpu/0', 'clock', 'megahertz', { key: 'cpu.clock.effective' }),
    sensor('memory/0/load/used', 'memory/0', 'load', 'percent', { key: 'memory.load' }),
    sensor('memory/0/data/used', 'memory/0', 'data', 'bytes', { key: 'memory.used' }),
    sensor('memory/0/data/total', 'memory/0', 'data', 'bytes', { key: 'memory.total' }),
    sensor('storage/device-mock-ssd/throughput/read', 'storage/device-mock-ssd', 'throughput', 'bytes_per_second', { key: 'storage.read' }),
    sensor('storage/device-mock-ssd/throughput/write', 'storage/device-mock-ssd', 'throughput', 'bytes_per_second', { key: 'storage.write' }),
    sensor('storage/device-mock-ssd/load/active', 'storage/device-mock-ssd', 'load', 'percent', { key: 'storage.active' }),
    sensor('storage/device-mock-ssd/percent/volume-mock-guid', 'storage/device-mock-ssd', 'percent', 'percent', { key: 'storage.volumeUsed', arg: 'C:' }),
    sensor('storage/device-mock-ssd/data/volume-mock-guid-free', 'storage/device-mock-ssd', 'data', 'bytes', { key: 'storage.volumeFree', arg: 'C:' }),
    sensor('network/mock-eth/throughput/down', 'network/mock-eth', 'throughput', 'bytes_per_second', { key: 'network.down' }),
    sensor('network/mock-eth/throughput/up', 'network/mock-eth', 'throughput', 'bytes_per_second', { key: 'network.up' }),
    sensor('network/mock-eth/throughput/link-speed', 'network/mock-eth', 'throughput', 'bits_per_second', { key: 'network.linkSpeed' }),
  ],
};

/** Deterministic plausible values for tick `t`, in MOCK_SCHEMA sensor order. */
export function mockValues(t: number): (number | null)[] {
  const wave = (period: number, phase = 0) => (Math.sin((t + phase) / period) + 1) / 2;
  const total = 20 + 50 * wave(9);
  const threads = Array.from({ length: THREADS }, (_, i) => Math.min(100, total * (0.6 + 0.1 * i)));
  const memTotal = 32 * GIB;
  const memUsed = memTotal * (0.5 + 0.1 * wave(30));
  return [
    total,
    ...threads,
    4200 + 400 * wave(7),
    (memUsed / memTotal) * 100,
    memUsed,
    memTotal,
    120e6 * wave(5),
    30e6 * wave(6, 2),
    60 * wave(5),
    65,
    700 * GIB,
    6e6 * wave(4),
    4e5 * wave(4, 1),
    1e9,
  ];
}

/** Browser-only backend used by `pnpm dev` and component tests. */
export function createMockBackend(intervalMs = 1000): Backend {
  let seq = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  const listeners = new Set<(s: Snapshot) => void>();
  const emit = () => {
    seq++;
    const snapshot: Snapshot = { revision: MOCK_SCHEMA.revision, seq, timestampMs: Date.now(), values: mockValues(seq) };
    listeners.forEach((cb) => cb(snapshot));
  };
  return {
    getSchema: async () => MOCK_SCHEMA,
    getHistory: async (ids, seconds) => {
      const n = Math.max(0, Math.min(seconds, 300));
      const now = Date.now();
      const ticks = Array.from({ length: n }, (_, i) => seq - n + 1 + i);
      const indices = ids.map((id) => MOCK_SCHEMA.sensors.findIndex((s) => s.id === id));
      return {
        revision: MOCK_SCHEMA.revision,
        seq,
        timestampsMs: ticks.map((_, i) => now - (n - 1 - i) * intervalMs),
        series: indices.map((k) => ticks.map((tick) => (k < 0 ? null : mockValues(tick)[k]))),
      };
    },
    onSchema: async () => () => {},
    onSnapshot: async (cb) => {
      listeners.add(cb);
      timer ??= setInterval(emit, intervalMs);
      return () => {
        listeners.delete(cb);
        if (listeners.size === 0 && timer !== undefined) {
          clearInterval(timer);
          timer = undefined;
        }
      };
    },
  };
}
