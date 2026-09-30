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

/** State of the PawnIO driver as the service reports it (spec M5 §2.8); only known while connected. */
export type PawnIoStatus = 'ok' | 'missing' | 'unavailable' | 'unknown' | 'rebootPending';

/** Whether the service has taken the sources this app asked for. */
export type Reconfiguration = 'applied' | 'pending' | 'failed';

/**
 * The service's effective sources, shared by all its clients: what runs may differ from what this
 * app asked. Mirrors `oma-ipc::ServiceSources`; disks are core device ids.
 */
export interface ServiceSources {
  /** Modules that are on (`cpu`, `motherboard`, `memory`, `storage`, `controller`, `psu`). */
  activeModules: string[];
  /**
   * The modules this app asked off in the request `reconfiguration` refers to: a module that is
   * among them and still active is kept on by another client.
   */
  requestedDisabledModules: string[];
  /** Core ids of the disks whose SMART is off. */
  smartDisabledDrives: string[];
  reconfiguration: Reconfiguration;
  /**
   * Core ids of the disks that keep SMART closed for all disks. An entry that is not a device of
   * the schema is a disk the app cannot identify ("unknown disk"); the list can be empty while the
   * gate is closed.
   */
  smartBlockedBy: string[];
}

export interface ServiceStatus {
  state: ServiceState;
  detail: ServiceDetail | null;
  /** null unless connected. */
  pawnIo: PawnIoStatus | null;
  /** null unless connected (and until the service has described itself). */
  sources: ServiceSources | null;
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

// --- Settings (mirrors app/src-tauri/src/settings and crates/oma-core/src/settings; spec M5 §2) ---

export type Language = 'system' | 'en' | 'it';
export type TemperatureUnit = 'c' | 'f';
export type ThroughputUnit = 'bits' | 'bytes';
export type ChartFps = 60 | 30 | 15;
/** The view opened at startup; `last` reopens the view shown when the app was closed. */
export type DefaultView = 'simple' | 'advanced' | 'last';
/** A view that can be remembered or requested from the tray. */
export type ViewKind = 'simple' | 'advanced';

export interface ServiceModules {
  cpu: boolean;
  motherboard: boolean;
  memory: boolean;
  storage: boolean;
  controller: boolean;
  psu: boolean;
}

/**
 * The settings file as the core encodes it (camelCase). `advanced.section`, `advanced.window` and
 * `view.last` are absent (not null) while never set; `advanced.series` holds only the sections set.
 */
export interface Settings {
  version: number;
  general: {
    language: Language;
    temperatureUnit: TemperatureUnit;
    throughputUnit: ThroughputUnit;
    intervalMs: number;
    chartFps: ChartFps;
    defaultView: DefaultView;
  };
  tray: {
    closeToTray: boolean;
    autostart: boolean;
    /** Sensor id, or null for automatic. */
    iconSensor: string | null;
  };
  sources: {
    vendorLibraries: { nvml: boolean; nvapi: boolean; adl: boolean; igcl: boolean };
    antiCheat: boolean;
    serviceModules: ServiceModules;
    smartDisabledDrives: string[];
  };
  advanced: {
    section?: string;
    window?: number;
    series: Record<string, string[]>;
  };
  view: { last?: ViewKind };
  rules: RulesSettings;
  /** Opaque until the log milestone. */
  log: Record<string, unknown>;
  migrations: { serviceV1: boolean; webviewV1: boolean };
}

/** `{ fixed }`, or a property of the sensor's device plus `offset`, with `fallback` when it is missing. */
export type Threshold = { fixed: number } | { property: string; offset: number; fallback: number };

/** One alert level; `threshold` is null only with `flagActive`. */
export interface LevelSpec {
  threshold: Threshold | null;
  durationS: number;
}

/** One sensor by id, or every sensor of a device kind and sensor kind whose name is in `names` (empty: all). */
export type RuleTarget = { sensor: string } | { deviceKind: DeviceKind; sensorKind: SensorKind; names: string[] };

export type RuleCondition = 'above' | 'below' | 'flagActive';

/** Mirrors `oma_core::rules::Rule`; thresholds and hysteresis are in the sensor's base unit. */
export interface Rule {
  id: string;
  target: RuleTarget;
  unit: Unit;
  condition: RuleCondition;
  warn: LevelSpec | null;
  crit: LevelSpec | null;
  hysteresis: { amount: number; durationS: number };
  enabled: boolean;
  notify: { warn: boolean; crit: boolean };
}

/** The changed fields of a built-in rule; `warn: null` / `crit: null` switch the level off. */
export interface RuleOverride {
  enabled?: boolean;
  warn?: LevelSpec | null;
  crit?: LevelSpec | null;
  hysteresis?: Rule['hysteresis'];
  notify?: Rule['notify'];
}

/** What the settings file keeps about rules: overrides per built-in rule id, then the custom rules. */
export interface RulesSettings {
  overrides: Record<string, RuleOverride>;
  custom: Rule[];
}

/**
 * What opening the settings file had to fix; `invalidRule` marks a custom rule (`rules.custom.2`) or an
 * override field (`rules.overrides.gpu-temp.warn`) left out, with the i18n key of the reason.
 */
export type SettingsDiagnostic = { path: string } & (
  | { kind: 'wrongType' | 'unknownVariant' | 'missingVersion' }
  | { kind: 'corrected'; from: string; to: string }
  | { kind: 'invalidRule'; key: string }
);

/** Where the settings stand on disk. `recovered` carries the path the corrupt file was kept at. */
export type Persistence =
  | { kind: 'ok' }
  | { kind: 'pending' }
  | { kind: 'recovered'; path: string }
  | { kind: 'readOnly'; reason: string }
  | { kind: 'error'; reason: string };

/** State of an effect outside the settings file (service, autostart, vendor libraries). */
export type EffectStatus = { kind: 'idle' | 'pending' | 'applied' } | { kind: 'failed'; reason: string };

/** What `get_settings`, `update_settings` and the `oma:settings` event carry; `seq` only ever grows. */
export interface SettingsState {
  settings: Settings;
  revision: number;
  persistedRevision: number;
  seq: number;
  persistence: Persistence;
  applyStatus: { service: EffectStatus; autostart: EffectStatus; vendorLibraries: EffectStatus };
  /** What opening the file fixed or left out, for the whole session. */
  diagnostics: SettingsDiagnostic[];
}

/** A rejected patch: the camelCase path of the failing field and an i18n key. */
export interface PatchError {
  field: string;
  key: string;
}

export type DeepPartial<T> = T extends readonly unknown[] ? T : T extends object ? { [K in keyof T]?: DeepPartial<T[K]> } : T;

/**
 * Objects merge, arrays and scalars replace; `null` only on the nullable fields. The fields of a rule
 * override (`warn`, `crit`, `hysteresis`, `notify`) replace whole, and `rules.custom` is an array.
 */
export type SettingsPatch = DeepPartial<Omit<Settings, 'version' | 'migrations' | 'rules' | 'log'>> & {
  rules?: { overrides?: Record<string, RuleOverride>; custom?: Rule[] };
};

/** What the web view kept in `localStorage` before the settings file existed. */
export interface LegacyWebviewState {
  section?: string;
  window?: number;
  series: Record<string, string[]>;
  view?: ViewKind;
}

/** Whether Windows will start the app's start-up entry. */
export type AutostartEffective = 'notConfigured' | 'enabled' | 'disabledByWindows' | 'unknown';

export interface AutostartStatus {
  configured: boolean;
  effective: AutostartEffective;
  error: string | null;
}

/** What the About page shows; mirrors `commands::AppInfo`. The paths are folders. */
export interface AppInfo {
  version: string;
  /** From the last service `Hello`; null until a service has answered. */
  serviceVersion: string | null;
  protocolVersion: number;
  settingsPath: string | null;
  logsPath: string | null;
}

/** The only places `openKnownPath` opens (never a path the UI chooses). */
export type KnownPath = 'settingsFolder' | 'logsFolder' | 'thirdPartyNotices' | 'startupAppsSettings';
