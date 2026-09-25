import type { Label, Schema, Sensor, SensorKind, Snapshot, StartupStatus, Unit } from '../types';
import type { Backend } from './backend';

const THREADS = 8;
const GIB = 1024 ** 3;
const GPU = 'gpu/pci-0000:01:00.0';

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
    {
      id: GPU,
      kind: 'gpu',
      name: 'Mock GeForce RTX 4080',
      vendor: 'NVIDIA',
      properties: { pciAddress: '0000:01:00.0', integrated: 'false' },
    },
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
    sensor(`${GPU}/load/core`, GPU, 'load', 'percent', { key: 'gpu.load.core' }),
    sensor(`${GPU}/data/memory-dedicated-used`, GPU, 'data', 'bytes', { key: 'gpu.memory.dedicatedUsed' }),
    sensor(`${GPU}/data/memory-dedicated-total`, GPU, 'data', 'bytes', { key: 'gpu.memory.dedicatedTotal' }),
    sensor(`${GPU}/temperature/core`, GPU, 'temperature', 'celsius', { key: 'gpu.temperature.core' }),
    { ...sensor(`${GPU}/temperature/hotspot`, GPU, 'temperature', 'celsius', { key: 'gpu.temperature.hotspot' }), experimental: true },
    sensor(`${GPU}/clock/core`, GPU, 'clock', 'megahertz', { key: 'gpu.clock.core' }),
    sensor(`${GPU}/power/board`, GPU, 'power', 'watt', { key: 'gpu.power.board' }),
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
  const gpuLoad = 10 + 80 * wave(11, 3);
  return [
    total,
    ...threads,
    4200 + 400 * wave(7),
    gpuLoad,
    (2 + 6 * wave(40)) * GIB,
    16 * GIB,
    40 + gpuLoad * 0.3,
    52 + gpuLoad * 0.35,
    1500 + 12 * gpuLoad,
    25 + 2.9 * gpuLoad,
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

/** The mock never starts in GPU safe mode. */
export const MOCK_STARTUP: StartupStatus = { safeMode: false, reason: null, crashModule: null };

/** Browser-only backend used by `pnpm dev` and component tests. */
export function createMockBackend(intervalMs = 1000): Backend {
  let seq = 0;
  let startup = MOCK_STARTUP;
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
    getStartupStatus: async () => startup,
    enableVendorLibraries: async () => {
      startup = { ...startup, safeMode: false };
      return startup;
    },
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
