export type DeviceKind =
  | 'cpu'
  | 'gpu'
  | 'memory'
  | 'storage'
  | 'network'
  | 'motherboard'
  | 'battery'
  | 'fan_controller'
  | 'psu';

export type SensorKind =
  | 'temperature'
  | 'load'
  | 'clock'
  | 'power'
  | 'voltage'
  | 'current'
  | 'fan'
  | 'data'
  | 'throughput'
  | 'energy'
  | 'flag'
  | 'percent'
  | 'link'
  | 'counter';

export type Unit =
  | 'celsius'
  | 'percent'
  | 'megahertz'
  | 'watt'
  | 'volt'
  | 'ampere'
  | 'rpm'
  | 'bytes'
  | 'bytes_per_second'
  | 'bits_per_second'
  | 'joule'
  | 'boolean'
  | 'pcie_generation'
  | 'lanes'
  | 'hours'
  | 'count';

export type Source =
  | 'pdh'
  | 'win32'
  | 'ip_helper'
  | 'dxgi'
  | 'd3dkmt'
  | 'nvml'
  | 'nvapi'
  | 'adl'
  | 'igcl'
  | 'pnp'
  | 'mock'
  | 'lhm';

/** Translation key (looked up as `sensor.<key>`) plus optional `{arg}`. */
export interface Label {
  key: string;
  arg?: string;
}

export interface Device {
  id: string;
  kind: DeviceKind;
  name: string;
  vendor?: string;
  properties?: Record<string, string>;
}

export interface Sensor {
  id: string;
  deviceId: string;
  kind: SensorKind;
  unit: Unit;
  label: Label;
  source: Source;
  category: string;
  /** Present (true) only for readings from undocumented vendor calls (spec §5.2). */
  experimental?: boolean;
}

export interface Schema {
  revision: number;
  devices: Device[];
  sensors: Sensor[];
}

/** `values[i]` belongs to `schema.sensors[i]` of the schema with the same revision. */
export interface Snapshot {
  revision: number;
  seq: number;
  timestampMs: number;
  values: (number | null)[];
}

export interface HistoryWindow {
  timestampsMs: number[];
  series: (number | null)[][];
}

/** Atomic watermark attached by the backend while holding the engine lock. */
export interface HistorySeed extends HistoryWindow {
  revision: number;
  seq: number;
}

/** GPU safe mode (spec §8): vendor libraries off after `--safe` or a crash. */
export interface StartupStatus {
  safeMode: boolean;
  reason: 'flag' | 'crash' | null;
  /** File name of the module that crashed the previous run, e.g. "nvml.dll". */
  crashModule: string | null;
}

/** Running statistics of one sensor since the app started or its last reset (spec §4.2). */
export interface SensorStats {
  min: number;
  max: number;
  avg: number;
  count: number;
}

/** `stats[i]` belongs to the i-th requested id; null = unknown id or no sample yet. */
export interface StatsReply {
  revision: number;
  stats: (SensorStats | null)[];
}

/** Sampling session of the core process (it outlives the window). */
export interface Session {
  /** Timestamp of the engine's first tick; null before it. */
  startedAtMs: number | null;
  intervalMs: number;
}

/** Sensor service status shown by the shell (spec §6); mirrors `oma-ipc::ServiceStatus`. */
export type ServiceState = 'notInstalled' | 'antiCheat' | 'starting' | 'connected' | 'unreachable' | 'incompatible';

/** Why the service is in its current state, when there is more to say. */
export type ServiceDetail = 'accessDenied' | 'startFailed' | 'stopping' | 'stopFailed' | 'pidMismatch' | 'disconnected';

export interface ServiceStatus {
  state: ServiceState;
  detail: ServiceDetail | null;
}

/** One process using a GPU (not a sensor: no id, no history). */
export interface GpuProcess {
  pid: number;
  name: string;
  /** Busiest engine of the process, 0..100; null on the first tick after (re)attach. */
  loadPercent: number | null;
  /** Type of that engine (e.g. "3D"), only while the load is above zero. */
  engine: string | null;
  dedicatedBytes: number | null;
  sharedBytes: number | null;
}
