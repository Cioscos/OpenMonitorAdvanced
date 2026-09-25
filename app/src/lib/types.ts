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
  | 'percent';

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
  | 'boolean';

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
  | 'mock';

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
