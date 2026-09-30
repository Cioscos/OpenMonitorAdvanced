import type { Backend, Unsubscribe } from '../lib/backend/backend';
import { MockSettings } from '../lib/backend/mockSettings';
import type {
  AppInfo,
  AutostartStatus,
  GpuProcess,
  HealthClock,
  HealthReport,
  HistorySeed,
  HistoryWindow,
  KnownPath,
  LegacyWebviewState,
  Schema,
  SensorStats,
  ServiceStatus,
  Session,
  SettingsPatch,
  SettingsState,
  Snapshot,
  StartupStatus,
  StatsReply,
  NavigationTarget,
  Rule,
  RuleStatus,
  LogStatus,
} from '../lib/types';
import defaultRulesFixture from './fixtures/default-rules.json';

const NO_HOTKEY = { requested: null, effective: null, state: 'unset', reason: null } as const;

/** A log status for tests: idle, no file, revision 0; override what matters. */
export function makeLogStatus(over: Partial<LogStatus> = {}): LogStatus {
  return {
    revision: 0,
    state: 'idle',
    session: 0,
    path: null,
    part: 0,
    partBytes: 0,
    recordedMs: 0,
    rows: 0,
    bytes: 0,
    dropped: 0,
    error: null,
    hotkeys: { toggle: { ...NO_HOTKEY }, pause: { ...NO_HOTKEY } },
    ...over,
  };
}

export interface HistoryCall {
  ids: string[];
  seconds: number;
  maxPoints: number | undefined;
}

/** Hand-driven backend for tests: emit events explicitly. */
export class FakeBackend implements Backend {
  schema: Schema;
  /** Oldest first; `getHistory(ids, seconds)` returns the last `seconds` samples (1 sample = 1 s). */
  history: HistoryWindow = { timestampsMs: [], series: [] };
  historyCalls: HistoryCall[] = [];
  schemaCalls = 0;
  startup: StartupStatus = { safeMode: false, reason: null, crashModule: null };
  enableCalls = 0;
  /** Stats by sensor id; ids without an entry read as null. `resetStats` deletes entries. */
  stats: Record<string, SensorStats> = {};
  statsCalls: string[][] = [];
  resetCalls: string[][] = [];
  session: Session = { startedAtMs: null, intervalMs: 1000 };
  /** Returned (copied) for every device id; `gpuProcessCalls` records the ids asked for. */
  gpuProcesses: GpuProcess[] = [];
  gpuProcessCalls: string[] = [];
  /** Current service status; `getServiceStatus` returns it, `emitServiceStatus` replaces it and notifies listeners. */
  serviceStatus: ServiceStatus = { state: 'connected', detail: null, pawnIo: null, sources: null };
  /** Set to reject `setAntiCheat`/`startService` with this error instead of resolving. */
  setAntiCheatError: string | null = null;
  startServiceError: string | null = null;
  setAntiCheatCalls: boolean[] = [];
  startServiceCalls = 0;
  /** In-memory settings with the core's rules; seed them with `settings.update(patch)` before rendering. */
  settings = new MockSettings();
  /** Order of the settings reads/subscriptions, to check that the UI subscribes first. */
  settingsCalls: string[] = [];
  /** Legacy states received by `importWebviewState`. */
  importCalls: LegacyWebviewState[] = [];
  /** Set to reject `importWebviewState` with this error instead of importing. */
  importError: string | null = null;
  /** What `takePendingView` returns (once). */
  pendingView: NavigationTarget | null = null;
  takePendingViewCalls = 0;
  /** Runs at the start of `takePendingView`, e.g. to emit an `oma:navigate` in the gap. */
  beforeTakePendingView: (() => void) | null = null;
  navigateSubscribedBeforePendingRead = false;
  /** What `getHealth`/`getHealthClock` return; `emitHealth`/`emitHealthClock` replace them and notify listeners. */
  health: HealthReport = { level: 'neutral', sinceMs: 0, revision: 0, coverage: 'complete', unavailableTargets: [], alerts: [] };
  healthClock: HealthClock = { revision: 0, levelElapsedMs: 0 };
  /** Order of the health reads and subscriptions, to check that the UI subscribes first. */
  healthCalls: string[] = [];
  /** Set to reject `getHealth` with this error. */
  healthError: string | null = null;
  /** What `getRuleStatus` returns; `ruleStatusCalls` counts the reads. */
  ruleStatus: RuleStatus[] = [];
  ruleStatusCalls = 0;
  /** Ids passed to `resetRuleOverride`, in order. */
  resetRuleOverrideCalls: string[] = [];
  autostart: AutostartStatus = { configured: false, effective: 'notConfigured', error: null };
  refreshAutostartCalls = 0;
  appInfo: AppInfo = {
    version: '0.1.0',
    serviceVersion: null,
    protocolVersion: 2,
    settingsPath: 'C:\\Users\\test\\AppData\\Roaming\\OpenMonitorAdvanced',
    logsPath: 'C:\\Users\\test\\AppData\\Local\\OpenMonitorAdvanced\\logs',
  };
  openKnownPathCalls: KnownPath[] = [];
  /** Set to reject `openKnownPath` with this text instead of resolving. */
  openKnownPathError: string | null = null;
  /** What `getLogStatus` and the four log commands return; `emitLogStatus` replaces it and notifies listeners. */
  logStatus: LogStatus = makeLogStatus();
  /** Every log call in order (`onLogStatus`, `getLogStatus`, `logStart`, ...), to check that the UI subscribes first. */
  logCalls: string[] = [];
  /** Set to reject `getLogStatus` and the four commands with this error. */
  logError: string | null = null;
  openLogFolderError: string | null = null;
  /** What `pickLogFolder` returns. */
  pickedLogFolder: string | null = null;
  /** Every `setLogHotkeysSuspended` call, in order. */
  hotkeySuspensions: boolean[] = [];
  #logListeners = new Set<(s: LogStatus) => void>();
  #schemaListeners = new Set<(s: Schema) => void>();
  #snapshotListeners = new Set<(s: Snapshot) => void>();
  #serviceListeners = new Set<(s: ServiceStatus) => void>();
  #settingsListeners = new Set<(s: SettingsState) => void>();
  #healthListeners = new Set<(r: HealthReport) => void>();
  #healthClockListeners = new Set<(c: HealthClock) => void>();
  #navigateListeners = new Set<(t: NavigationTarget) => void>();

  constructor(schema: Schema) {
    this.schema = schema;
  }

  async getSchema(): Promise<Schema> {
    this.schemaCalls++;
    return this.schema;
  }

  async getHistory(ids: string[], seconds: number, maxPoints?: number): Promise<HistorySeed> {
    this.historyCalls.push({ ids, seconds, maxPoints });
    const keep = Math.max(0, Math.floor(seconds));
    const from = Math.max(0, this.history.timestampsMs.length - keep);
    return {
      revision: this.schema.revision,
      seq: 0,
      timestampsMs: this.history.timestampsMs.slice(from),
      series: ids.map((_, i) => (this.history.series[i] ?? []).slice(from)),
    };
  }

  async onSchema(cb: (s: Schema) => void): Promise<Unsubscribe> {
    this.#schemaListeners.add(cb);
    return () => this.#schemaListeners.delete(cb);
  }

  async onSnapshot(cb: (s: Snapshot) => void): Promise<Unsubscribe> {
    this.#snapshotListeners.add(cb);
    return () => this.#snapshotListeners.delete(cb);
  }

  async getStartupStatus(): Promise<StartupStatus> {
    return this.startup;
  }

  async enableVendorLibraries(): Promise<StartupStatus> {
    this.enableCalls++;
    this.startup = { ...this.startup, safeMode: false };
    return this.startup;
  }

  async getStats(ids: string[]): Promise<StatsReply> {
    this.statsCalls.push(ids);
    return { revision: this.schema.revision, stats: ids.map((id) => this.stats[id] ?? null) };
  }

  async resetStats(ids: string[]): Promise<void> {
    this.resetCalls.push(ids);
    for (const id of ids) delete this.stats[id];
  }

  async getSession(): Promise<Session> {
    return this.session;
  }

  async getGpuProcesses(deviceId: string): Promise<GpuProcess[]> {
    this.gpuProcessCalls.push(deviceId);
    return [...this.gpuProcesses];
  }

  async getServiceStatus(): Promise<ServiceStatus> {
    return this.serviceStatus;
  }

  async onServiceStatus(cb: (s: ServiceStatus) => void): Promise<Unsubscribe> {
    this.#serviceListeners.add(cb);
    return () => this.#serviceListeners.delete(cb);
  }

  async setAntiCheat(enabled: boolean): Promise<ServiceStatus> {
    this.setAntiCheatCalls.push(enabled);
    if (this.setAntiCheatError !== null) throw new Error(this.setAntiCheatError);
    return this.serviceStatus;
  }

  async startService(): Promise<ServiceStatus> {
    this.startServiceCalls++;
    if (this.startServiceError !== null) throw new Error(this.startServiceError);
    return this.serviceStatus;
  }

  async getSettings(): Promise<SettingsState> {
    this.settingsCalls.push('getSettings');
    return this.settings.state();
  }

  async updateSettings(patch: SettingsPatch): Promise<SettingsState> {
    return this.settings.update(patch);
  }

  async resetRuleOverride(ruleId: string): Promise<SettingsState> {
    this.resetRuleOverrideCalls.push(ruleId);
    return this.settings.resetRuleOverride(ruleId);
  }

  async onSettings(cb: (s: SettingsState) => void): Promise<Unsubscribe> {
    this.settingsCalls.push('onSettings');
    this.#settingsListeners.add(cb);
    const off = this.settings.subscribe(cb);
    return () => {
      this.#settingsListeners.delete(cb);
      off();
    };
  }

  async importWebviewState(legacy: LegacyWebviewState): Promise<SettingsState> {
    this.importCalls.push(legacy);
    if (this.importError !== null) throw this.importError;
    return this.settings.import(legacy);
  }

  async takePendingView(): Promise<NavigationTarget | null> {
    this.takePendingViewCalls++;
    this.navigateSubscribedBeforePendingRead = this.#navigateListeners.size > 0;
    this.beforeTakePendingView?.();
    const view = this.pendingView;
    this.pendingView = null;
    return view;
  }

  async onNavigate(cb: (t: NavigationTarget) => void): Promise<Unsubscribe> {
    this.#navigateListeners.add(cb);
    return () => this.#navigateListeners.delete(cb);
  }

  async getHealth(): Promise<HealthReport> {
    this.healthCalls.push('getHealth');
    if (this.healthError !== null) throw new Error(this.healthError);
    return this.health;
  }

  async onHealth(cb: (r: HealthReport) => void): Promise<Unsubscribe> {
    this.healthCalls.push('onHealth');
    this.#healthListeners.add(cb);
    return () => this.#healthListeners.delete(cb);
  }

  async getHealthClock(): Promise<HealthClock> {
    this.healthCalls.push('getHealthClock');
    return this.healthClock;
  }

  async onHealthClock(cb: (c: HealthClock) => void): Promise<Unsubscribe> {
    this.healthCalls.push('onHealthClock');
    this.#healthClockListeners.add(cb);
    return () => this.#healthClockListeners.delete(cb);
  }

  /** Number of live health listeners (report plus clock). */
  get healthListenerCount(): number {
    return this.#healthListeners.size + this.#healthClockListeners.size;
  }

  async getRuleStatus(): Promise<RuleStatus[]> {
    this.ruleStatusCalls++;
    return structuredClone(this.ruleStatus);
  }

  /** The Rust table, through the fixture its parity test keeps current. */
  async getDefaultRules(): Promise<Rule[]> {
    return structuredClone(defaultRulesFixture as Rule[]);
  }

  async refreshAutostart(): Promise<AutostartStatus> {
    this.refreshAutostartCalls++;
    return this.autostart;
  }

  async getAppInfo(): Promise<AppInfo> {
    return this.appInfo;
  }

  async openKnownPath(target: KnownPath): Promise<void> {
    this.openKnownPathCalls.push(target);
    if (this.openKnownPathError !== null) throw this.openKnownPathError;
  }

  async getLogStatus(): Promise<LogStatus> {
    this.logCalls.push('getLogStatus');
    if (this.logError !== null) throw new Error(this.logError);
    return this.logStatus;
  }

  async onLogStatus(cb: (s: LogStatus) => void): Promise<Unsubscribe> {
    this.logCalls.push('onLogStatus');
    this.#logListeners.add(cb);
    return () => this.#logListeners.delete(cb);
  }

  async #logCommand(name: string): Promise<LogStatus> {
    this.logCalls.push(name);
    if (this.logError !== null) throw new Error(this.logError);
    return this.logStatus;
  }

  logStart = () => this.#logCommand('logStart');
  logPause = () => this.#logCommand('logPause');
  logResume = () => this.#logCommand('logResume');
  logStop = () => this.#logCommand('logStop');

  async openLogFolder(): Promise<void> {
    this.logCalls.push('openLogFolder');
    if (this.openLogFolderError !== null) throw this.openLogFolderError;
  }

  async pickLogFolder(): Promise<string | null> {
    this.logCalls.push('pickLogFolder');
    return this.pickedLogFolder;
  }

  async setLogHotkeysSuspended(suspended: boolean): Promise<void> {
    this.hotkeySuspensions.push(suspended);
  }

  /** Number of live `oma:log` listeners. */
  get logListenerCount(): number {
    return this.#logListeners.size;
  }

  emitLogStatus(status: LogStatus): void {
    this.logStatus = status;
    this.#logListeners.forEach((cb) => cb(status));
  }

  /** Delivers an arbitrary state to the `onSettings` listeners (e.g. a stale one). */
  emitSettings(state: SettingsState): void {
    this.#settingsListeners.forEach((cb) => cb(state));
  }

  emitHealth(report: HealthReport): void {
    this.health = report;
    this.#healthListeners.forEach((cb) => cb(report));
  }

  emitHealthClock(clock: HealthClock): void {
    this.healthClock = clock;
    this.#healthClockListeners.forEach((cb) => cb(clock));
  }

  emitNavigate(target: NavigationTarget): void {
    this.#navigateListeners.forEach((cb) => cb(target));
  }

  emitSchema(schema: Schema): void {
    this.schema = schema;
    this.#schemaListeners.forEach((cb) => cb(schema));
  }

  emitSnapshot(snapshot: Snapshot): void {
    this.#snapshotListeners.forEach((cb) => cb(snapshot));
  }

  emitServiceStatus(status: ServiceStatus): void {
    this.serviceStatus = status;
    this.#serviceListeners.forEach((cb) => cb(status));
  }
}
