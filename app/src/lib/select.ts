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

/** The value of the first of `keys` whose sensor exists and has a value. */
function firstValue(schema: Schema, deviceId: string, valueOf: ValueOf, ...keys: string[]): number | null {
  for (const key of keys) {
    const v = read(valueOf, sensorsWith(schema, [deviceId], key)[0]);
    if (v !== null) return v;
  }
  return null;
}

export interface CpuSummary {
  name: string;
  load: number | null;
  loadId: string | null;
  clockMhz: number | null;
  temperatureC: number | null;
  powerW: number | null;
  busiestThread: number | null;
  threads: Array<{ id: string; value: number | null }>;
}

export function cpuSummary(schema: Schema, valueOf: ValueOf): CpuSummary | null {
  const cpu = devicesOf(schema, 'cpu')[0];
  if (!cpu) return null;
  const load = sensorsWith(schema, [cpu.id], 'cpu.load.total')[0];
  const clock = sensorsWith(schema, [cpu.id], 'cpu.clock.effective')[0];
  const threads = sensorsWith(schema, [cpu.id], 'cpu.load.thread').map((s) => ({ id: s.id, value: valueOf(s.id) }));
  const loads = threads.flatMap((th) => (th.value === null ? [] : [th.value]));
  return {
    name: cpu.name,
    load: read(valueOf, load),
    loadId: load?.id ?? null,
    clockMhz: read(valueOf, clock),
    temperatureC: firstValue(schema, cpu.id, valueOf, 'cpu.temperature.package', 'cpu.temperature.tctl'),
    powerW: firstValue(schema, cpu.id, valueOf, 'cpu.power.package'),
    busiestThread: loads.length ? Math.max(...loads) : null,
    threads,
  };
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
  hotspotC: number | null;
  powerLimitW: number | null;
  fanPct: number | null;
  fanRpm: number | null;
  throttle: 'power' | 'thermal' | null;
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
      hotspotC: read(valueOf, find('gpu.temperature.hotspot')),
      powerLimitW: read(valueOf, find('gpu.power.limit')),
      fanPct: read(valueOf, find('gpu.fan.percent')),
      fanRpm: read(valueOf, find('gpu.fan.rpm')),
      throttle: read(valueOf, find('gpu.throttle.power')) === 1 ? 'power' : read(valueOf, find('gpu.throttle.thermal')) === 1 ? 'thermal' : null,
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
  availableBytes: number | null;
  dimmTempsC: number[];
}

export function memorySummary(schema: Schema, valueOf: ValueOf): MemorySummary | null {
  const mem = devicesOf(schema, 'memory')[0];
  if (!mem) return null;
  const usedBytes = read(valueOf, sensorsWith(schema, [mem.id], 'memory.used')[0]);
  const totalBytes = read(valueOf, sensorsWith(schema, [mem.id], 'memory.total')[0]);
  return {
    usedBytes,
    totalBytes,
    availableBytes: usedBytes !== null && totalBytes !== null ? Math.max(0, totalBytes - usedBytes) : null,
    dimmTempsC: sensorsWith(schema, [mem.id], 'memory.temperature.dimm').flatMap((s) => {
      const v = valueOf(s.id);
      return v === null ? [] : [v];
    }),
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
  adapterName: string | null;
  linkBps: number | null;
}

export interface DiskRow {
  deviceId: string;
  name: string;
  letter: string | null;
  usedPct: number | null;
  activePct: number | null;
  readBps: number | null;
  writeBps: number | null;
  temperatureC: number | null;
  standby: boolean;
}

/** One row per disk, in schema order (the order of the Advanced navigation). */
export function diskRows(schema: Schema, valueOf: ValueOf, powerOf: (deviceId: string) => string | undefined): DiskRow[] {
  return devicesOf(schema, 'storage').map((d) => {
    const find = (key: string) => sensorsWith(schema, [d.id], key)[0];
    const volume = find('storage.volumeUsed');
    return {
      deviceId: d.id,
      name: d.name,
      letter: volume?.label.arg ?? null,
      usedPct: read(valueOf, volume),
      activePct: read(valueOf, find('storage.active')),
      readBps: read(valueOf, find('storage.read')),
      writeBps: read(valueOf, find('storage.write')),
      temperatureC: read(valueOf, find('storage.temperature')),
      standby: powerOf(d.id) === 'standby',
    };
  });
}

export function networkSummary(schema: Schema, valueOf: ValueOf): NetworkSummary | null {
  const nics = devicesOf(schema, 'network');
  const ids = nics.map((d) => d.id);
  if (ids.length === 0) return null;
  const down = sensorsWith(schema, ids, 'network.down');
  return {
    downBps: sum(down.map((s) => valueOf(s.id))),
    upBps: sum(sensorsWith(schema, ids, 'network.up').map((s) => valueOf(s.id))),
    downIds: down.map((s) => s.id),
    adapterName: nics[0].name,
    linkBps: read(valueOf, sensorsWith(schema, [nics[0].id], 'network.linkSpeed')[0]),
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
