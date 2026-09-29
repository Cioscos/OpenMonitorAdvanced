import type { Backend, Unsubscribe } from '../lib/backend/backend';
import { MockSettings } from '../lib/backend/mockSettings';
import type {
  AppInfo,
  AutostartStatus,
  GpuProcess,
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
  ViewKind,
} from '../lib/types';

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
  pendingView: ViewKind | null = null;
  takePendingViewCalls = 0;
  /** Runs at the start of `takePendingView`, e.g. to emit an `oma:navigate` in the gap. */
  beforeTakePendingView: (() => void) | null = null;
  navigateSubscribedBeforePendingRead = false;
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
  #schemaListeners = new Set<(s: Schema) => void>();
  #snapshotListeners = new Set<(s: Snapshot) => void>();
  #serviceListeners = new Set<(s: ServiceStatus) => void>();
  #settingsListeners = new Set<(s: SettingsState) => void>();
  #navigateListeners = new Set<(v: ViewKind) => void>();

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

  async takePendingView(): Promise<ViewKind | null> {
    this.takePendingViewCalls++;
    this.navigateSubscribedBeforePendingRead = this.#navigateListeners.size > 0;
    this.beforeTakePendingView?.();
    const view = this.pendingView;
    this.pendingView = null;
    return view;
  }

  async onNavigate(cb: (v: ViewKind) => void): Promise<Unsubscribe> {
    this.#navigateListeners.add(cb);
    return () => this.#navigateListeners.delete(cb);
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

  /** Delivers an arbitrary state to the `onSettings` listeners (e.g. a stale one). */
  emitSettings(state: SettingsState): void {
    this.#settingsListeners.forEach((cb) => cb(state));
  }

  emitNavigate(view: ViewKind): void {
    this.#navigateListeners.forEach((cb) => cb(view));
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
