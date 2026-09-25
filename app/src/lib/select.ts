import type { DeviceKind, Schema, Sensor } from './types';

export type ValueOf = (id: string) => number | null;

const devicesOf = (schema: Schema, kind: DeviceKind) => schema.devices.filter((d) => d.kind === kind);

const sensorsWith = (schema: Schema, deviceIds: string[], key: string): Sensor[] =>
  schema.sensors.filter((s) => deviceIds.includes(s.deviceId) && s.label.key === key);

function sum(values: (number | null)[]): number | null {
  const present = values.filter((v): v is number => v !== null);
  return present.length ? present.reduce((a, b) => a + b, 0) : null;
}

const read = (valueOf: ValueOf, sensor: Sensor | undefined) => (sensor ? valueOf(sensor.id) : null);

export interface CpuSummary {
  name: string;
  load: number | null;
  loadId: string | null;
  clockMhz: number | null;
}

export function cpuSummary(schema: Schema, valueOf: ValueOf): CpuSummary | null {
  const cpu = devicesOf(schema, 'cpu')[0];
  if (!cpu) return null;
  const load = sensorsWith(schema, [cpu.id], 'cpu.load.total')[0];
  const clock = sensorsWith(schema, [cpu.id], 'cpu.clock.effective')[0];
  return { name: cpu.name, load: read(valueOf, load), loadId: load?.id ?? null, clockMhz: read(valueOf, clock) };
}

export interface GpuSummary {
  deviceId: string;
  name: string;
  integrated: boolean;
  load: number | null;
  loadId: string | null;
  temperatureC: number | null;
  clockMhz: number | null;
  powerW: number | null;
  memUsedBytes: number | null;
  memTotalBytes: number | null;
}

/** One summary per GPU, in schema order (the Rust enumeration order). */
export function gpuSummaries(schema: Schema, valueOf: ValueOf): GpuSummary[] {
  return devicesOf(schema, 'gpu').map((gpu) => {
    const find = (key: string) => sensorsWith(schema, [gpu.id], key)[0];
    const load = find('gpu.load.core');
    return {
      deviceId: gpu.id,
      name: gpu.name,
      integrated: gpu.properties?.integrated === 'true',
      load: read(valueOf, load),
      loadId: load?.id ?? null,
      temperatureC: read(valueOf, find('gpu.temperature.core')),
      clockMhz: read(valueOf, find('gpu.clock.core')),
      powerW: read(valueOf, find('gpu.power.board')),
      memUsedBytes: read(valueOf, find('gpu.memory.dedicatedUsed')),
      memTotalBytes: read(valueOf, find('gpu.memory.dedicatedTotal')),
    };
  });
}

/** Spec §7.2: one tile per discrete GPU; integrated GPUs only when there is no discrete one. */
export function simpleViewGpus(list: GpuSummary[]): GpuSummary[] {
  const discrete = list.filter((gpu) => !gpu.integrated);
  return discrete.length > 0 ? discrete : list;
}

export interface MemorySummary {
  usedBytes: number | null;
  totalBytes: number | null;
  usedPct: number | null;
  loadId: string | null;
}

export function memorySummary(schema: Schema, valueOf: ValueOf): MemorySummary | null {
  const mem = devicesOf(schema, 'memory')[0];
  if (!mem) return null;
  return {
    usedBytes: read(valueOf, sensorsWith(schema, [mem.id], 'memory.used')[0]),
    totalBytes: read(valueOf, sensorsWith(schema, [mem.id], 'memory.total')[0]),
    usedPct: read(valueOf, sensorsWith(schema, [mem.id], 'memory.load')[0]),
    loadId: sensorsWith(schema, [mem.id], 'memory.load')[0]?.id ?? null,
  };
}

export interface StorageSummary {
  readBps: number | null;
  writeBps: number | null;
  volume: { letter: string; usedPct: number | null } | null;
  readIds: string[];
}

export function storageSummary(schema: Schema, valueOf: ValueOf): StorageSummary | null {
  const ids = devicesOf(schema, 'storage').map((d) => d.id);
  if (ids.length === 0) return null;
  const volumes = sensorsWith(schema, ids, 'storage.volumeUsed');
  const system = volumes.find((s) => s.label.arg === 'C:') ?? volumes[0];
  return {
    readIds: sensorsWith(schema, ids, 'storage.read').map((s) => s.id),
    readBps: sum(sensorsWith(schema, ids, 'storage.read').map((s) => valueOf(s.id))),
    writeBps: sum(sensorsWith(schema, ids, 'storage.write').map((s) => valueOf(s.id))),
    volume: system ? { letter: system.label.arg ?? '', usedPct: valueOf(system.id) } : null,
  };
}

export interface NetworkSummary {
  downBps: number | null;
  upBps: number | null;
  downIds: string[];
}

export function networkSummary(schema: Schema, valueOf: ValueOf): NetworkSummary | null {
  const ids = devicesOf(schema, 'network').map((d) => d.id);
  if (ids.length === 0) return null;
  const down = sensorsWith(schema, ids, 'network.down');
  return {
    downBps: sum(down.map((s) => valueOf(s.id))),
    upBps: sum(sensorsWith(schema, ids, 'network.up').map((s) => valueOf(s.id))),
    downIds: down.map((s) => s.id),
  };
}

/** Element-wise sum of right-aligned series; NaN where no series has a value. */
export function sumSeries(series: number[][]): number[] {
  const length = Math.max(0, ...series.map((s) => s.length));
  return Array.from({ length }, (_, i) => {
    let total = 0;
    let any = false;
    for (const s of series) {
      const v = s[s.length - length + i];
      if (v !== undefined && Number.isFinite(v)) {
        total += v;
        any = true;
      }
    }
    return any ? total : NaN;
  });
}
