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
  /** One code per value: 0 fresh, 1 held, 2 suspended. Absent or malformed reads as all fresh. */
  quality?: number[];
}

/** What a disk is doing, as far as the core can tell without touching it. */
export type DiskPower = 'active' | 'idle' | 'standby' | 'unknown';

export interface DiskStateEntry {
  deviceId: string;
  power: DiskPower;
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
  /** Every physical drive the service enumerates, in `physicalDrive` order. */
  drives: SourceDrive[];
}

/** What the service reports about a drive: `active`, `standby`, `idle`, `unknown`, `smartOff` or `noMedia`. */
export type SourceDriveState = 'active' | 'standby' | 'idle' | 'unknown' | 'smartOff' | 'noMedia';

/** One physical drive of the service's list; mirrors `oma-ipc::SourceDrive`. */
export interface SourceDrive {
  physicalDrive: number;
  /** Core device id when the drive matches a disk of the schema. */
  deviceId: string | null;
  model: string | null;
  state: SourceDriveState;
  /** Whether this drive keeps the SMART gate closed for all drives. */
  blocksSmart: boolean;
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
/**
 * Where `oma:navigate` and `take_pending_view` send the window: a tray item names only the view, a
 * clicked toast the Advanced view and the device whose page it opens.
 */
export interface NavigationTarget {
  view: ViewKind;
  deviceId?: string;
  /** The settings section to open, for the update toast. */
  settingsSection?: 'about';
}

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
export type LogEveryTicks = 1 | 2 | 5 | 10 | 30 | 60;

/** The `log` section: hotkeys are stored in canonical form (`Ctrl+Alt+Shift+R`). */
export interface LogSettings {
  /** Absolute folder, or null for `Documents\OpenMonitor Advanced\logs`. */
  folder: string | null;
  /** Sensor ids, or null for all of them. */
  sensors: string[] | null;
  everyTicks: LogEveryTicks;
  /** 10 to 2048. */
  maxFileMb: number;
  hotkeyToggle: string | null;
  hotkeyPause: string | null;
}

/** The `overlay` section; names are lowercase executables, hotkeys canonical. */
export interface OverlaySettings {
  enabled: boolean;
  chartFps: ChartFps;
  /** 2 or 4. */
  textHz: 2 | 4;
  hideFromCapture: boolean;
  attach: 'window' | 'monitor';
  trackPcLatency: boolean;
  trackGpu: boolean;
  /** A `builtin-*` id or a lowercase UUID. */
  defaultProfile: string;
  /** Executable -> profile id; replaced whole by a patch. */
  gameProfiles: Record<string, string>;
  blockedGames: string[];
  hotkeyToggle: string | null;
  hotkeyNextProfile: string | null;
  hotkeyBenchmark: string | null;
  /** Where the overlay editor was last closed, in physical pixels; null centers it. */
  editorBounds: { x: number; y: number; width: number; height: number } | null;
}

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
  updates: {
    checkAutomatically: boolean;
  };
  sources: {
    vendorLibraries: { nvml: boolean; nvapi: boolean; adl: boolean; igcl: boolean };
    antiCheat: boolean;
    serviceModules: ServiceModules;
    smartDisabledDrives: string[];
    /** Disks that are off by default (USB) and that the user turned on. */
    smartEnabledDrives: string[];
  };
  advanced: {
    section?: string;
    window?: number;
    series: Record<string, string[]>;
  };
  view: { last?: ViewKind };
  rules: RulesSettings;
  /** CSV sensor log; `null` folder = the default one, `null` sensors = all. */
  log: LogSettings;
  /** In-game overlay. */
  overlay: OverlaySettings;
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

/** A fixed number, the device property plus its offset, or the fallback without the property. */
export type ThresholdSource = 'fixed' | 'property' | 'fallback';

/** One instance of a rule as the engine sees it now (`get_rule_status`); thresholds are resolved, in the base unit. */
export interface InstanceStatus {
  sensorId: string;
  level: 'ok' | 'warn' | 'crit';
  warn: number | null;
  crit: number | null;
  /** Where each resolved threshold came from, kept while the instance is retained; null without a threshold. */
  warnSource: ThresholdSource | null;
  critSource: ThresholdSource | null;
  valid: boolean;
  problem: 'order' | 'unitMismatch' | null;
}

/** Every effective rule, in order; a disabled rule or one without sensors has no instances. */
export interface RuleStatus {
  ruleId: string;
  instances: InstanceStatus[];
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
export type SettingsPatch = DeepPartial<Omit<Settings, 'version' | 'migrations' | 'rules'>> & {
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

/** A sensor report saved by `exportSensorReport`; mirrors `report::ExportedReport`. Only the name, never the path. */
export interface ExportedReport {
  fileName: string;
}

/** Why the last update check failed; mirrors `updates::UpdateError`. */
export type UpdateError = 'offline' | 'timeout' | 'tls' | 'http' | 'invalid';

/** The update check as the shell last reported it; `latest` is set only for a version newer than `current`, also in `error`. */
export interface UpdateStatus {
  state: 'idle' | 'checking' | 'upToDate' | 'available' | 'error';
  current: string;
  latest: { version: string } | null;
  checkedAtMs: number | null;
  error: UpdateError | null;
}

/** The only places `openKnownPath` opens (never a path the UI chooses). */
export type KnownPath = 'settingsFolder' | 'logsFolder' | 'thirdPartyNotices' | 'thirdPartyLicenses' | 'startupAppsSettings';

/** Overall level of the rules engine; `neutral` when there is nothing to judge yet. */
export type HealthLevel = 'neutral' | 'ok' | 'warn' | 'crit';

/** An instance in `warn` or `crit`, current or retained after its sensor was lost. Mirrors `Alert` in oma-core. */
export interface Alert {
  ruleId: string;
  sensorId: string;
  deviceId: string;
  /** Presentation fields kept from the last schema that had the sensor. */
  unit: Unit;
  sensorLabel: Label;
  level: 'ok' | 'warn' | 'crit';
  /** Raw value of the last fresh valid tick; null when there never was one. */
  value: number | null;
  /** Threshold of `level`; null for flags. */
  threshold: number | null;
  /** System time the instance entered `level`, display only. */
  sinceMs: number;
  /** Whether the latest tick had a value for the sensor. */
  valid: boolean;
  lastValidMs: number | null;
  messageKey: string;
  params: Record<string, string>;
}

/** The rules engine's verdict (`get_health`, `oma:health`); `revision` goes up on every change. */
export interface HealthReport {
  level: HealthLevel;
  sinceMs: number;
  revision: number;
  coverage: 'complete' | 'partial' | 'unavailable';
  unavailableTargets: { ruleId: string; sensorId: string }[];
  /** Crit first, then the oldest entry, then rule and sensor id. */
  alerts: Alert[];
}

/** Monotonic time in the current level, for the report of the same `revision` (`oma:health-clock`). */
export interface HealthClock {
  revision: number;
  levelElapsedMs: number;
}


/** State of the CSV log (`oma:log`, `get_log_status`); mirrors `LogStatus` in app/src-tauri/src/log. */
export type LogState = 'idle' | 'recording' | 'paused' | 'error';

/** Why the log is in `error`: an i18n key and the `{detail}` of `log.error.other`. */
export interface LogError {
  key: string;
  detail: string | null;
}

/** What became of one global hotkey of the log. */
export interface HotkeyStatus {
  requested: string | null;
  effective: string | null;
  state: 'active' | 'unset' | 'failed';
  /** i18n key of the failure. */
  reason: string | null;
}

export interface LogStatus {
  /** Global, grows with every observable change (counters and hotkeys included) and never restarts. */
  revision: number;
  state: LogState;
  session: number;
  path: string | null;
  part: number;
  partBytes: number;
  recordedMs: number;
  rows: number;
  bytes: number;
  dropped: number;
  error: LogError | null;
  hotkeys: { toggle: HotkeyStatus; pause: HotkeyStatus };
}

/** One profile the overlay can use; a built-in's `name` is the i18n key `overlay.template.<id>`. */
export interface OverlayProfileEntry {
  id: string;
  name: string;
  builtin: boolean;
}

/** A profile file that could not be used, with the reason it was rejected. */
export interface OverlayProfileDiagnostic {
  file: string;
  reason: string;
}

/** The overlay process: `failed` carries `crashing` or `incompatible` in `processReason`. */
export type OverlayProcessState = 'off' | 'starting' | 'running' | 'failed';

/** The frame engine as the overlay sees it: the service's states, or `unavailable` without the service. */
export type OverlayFramesState = 'off' | 'starting' | 'running' | 'denied' | 'tampered' | 'missing' | 'failed' | 'unavailable';

/** The overlay's state (`overlay-status`, `get_overlay_status`); mirrors `OverlayStatus` in app/src-tauri/src/overlay/controller.rs. */
export interface OverlayStatus {
  enabled: boolean;
  process: OverlayProcessState;
  processReason: 'crashing' | 'incompatible' | null;
  frames: OverlayFramesState;
  framesDetail: string | null;
  /** The followed game: its executable name and PID. */
  target: { name: string; pid: number } | null;
  activeProfile: string | null;
  profiles: OverlayProfileEntry[];
  diagnostics: OverlayProfileDiagnostic[];
  /** Hidden with the hotkey or the tray; not saved. */
  hiddenByUser: boolean;
  hotkeys: { toggle: HotkeyStatus; nextProfile: HotkeyStatus; benchmark: HotkeyStatus };
  /** The editor's preview window is open. */
  preview: boolean;
  /** The preview's process gave up, which closed the preview; until the next preview. */
  previewFailure: 'crashing' | 'incompatible' | null;
  benchmark: BenchmarkStatus;
}

/** The benchmark capture (`OverlayStatus.benchmark`); `elapsedS` is the time when the status was sent. */
export interface BenchmarkStatus {
  state: 'idle' | 'recording' | 'error';
  /** The recorded game's executable. */
  game: string | null;
  elapsedS: number | null;
  error: LogError | null;
}

/** A profile opened in the editor (`overlay_load_profile`); `json` is the whole profile, defaults included. */
export interface EditableProfile {
  id: string;
  builtin: boolean;
  json: string;
}

/** How an editor or overlay command rejects: an i18n key and the `{detail}` of its message. */
export interface CommandError {
  key: string;
  detail: string | null;
}

/** One low of `FrameMetrics`; snake_case like the overlay protocol (crates/oma-ipc/src/overlay.rs). */
export interface WireLow {
  window_s: number;
  definition: string;
  one_percent: number | null;
  point_one_percent: number | null;
}

/** The frame metrics as the overlay protocol carries them (snake_case, `FrameMetrics` in crates/oma-ipc/src/overlay.rs). */
export interface FrameMetrics {
  state: string;
  fps_displayed: number | null;
  fps_rendered: number | null;
  fps_presented: number | null;
  rendered_source: string | null;
  fg_suspected: boolean;
  frametime_displayed_ms: number | null;
  frametime_app_ms: number | null;
  fg_multiplier: number | null;
  stutter_count: number | null;
  stutter_percent: number | null;
  latency_pc_ms: number | null;
  latency_display_ms: number | null;
  /** `gpu`, `cpu` or `unknown`. */
  bound: string | null;
  lows: WireLow[];
}

/** One frame for the frametime chart (snake_case, `WireFrameTime`). */
export interface WireFrameTime {
  t_s: number;
  displayed_ms: number | null;
  app_ms: number | null;
}

/** The `overlay-editor-data` payload (app/src-tauri/src/overlay/editor_feed.rs): the latest metrics and the new frame times. */
export interface EditorData {
  metrics: FrameMetrics;
  frameTimes: WireFrameTime[];
}

export interface SummaryLows {
  onePercent: number;
  pointOnePercent: number;
}

/** Final figures of a benchmark session (`SessionSummary` in crates/oma-core/src/frames/session.rs). */
export interface SessionSummary {
  durationS: number;
  framesTotal: number;
  framesDisplayed: number;
  framesGenerated: number;
  fpsDisplayed: number;
  fpsRendered: number | null;
  /** `XeSS-FG`, `AFMF`, `FG` or `Reflex`. */
  renderedSource: string | null;
  lowsIntegral: SummaryLows;
  lowsPercentile: SummaryLows;
  frametimeMinMs: number;
  frametimeMaxMs: number;
  stutterCount: number;
  stutterPercent: number;
  fgMultiplier: number | null;
  latencyPcMs: number | null;
  latencyDisplayMs: number | null;
}

/** One saved benchmark (`benchmark_list`): the `.json` next to its CSV. */
export interface BenchmarkEntry {
  id: string;
  record: {
    format: number;
    game: string;
    /** Local time, `YYYY-MM-DDTHH:MM:SS`. */
    startedAt: string;
    endReason: 'user' | 'noTarget' | 'limit' | 'error' | 'shutdown';
    summary: SessionSummary;
  };
}
