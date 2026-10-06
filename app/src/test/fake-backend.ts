import type { Backend, Unsubscribe } from '../lib/backend/backend';
import { MockSettings } from '../lib/backend/mockSettings';
import type {
  AppInfo,
  ExportedReport,
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
  DiskStateEntry,
  StartupStatus,
  StatsReply,
  NavigationTarget,
  Rule,
  RuleStatus,
  LogStatus,
  OverlayStatus,
  UpdateStatus,
  BenchmarkEntry,
  CommandError,
  EditableProfile,
  EditorData,
  Plan,
  RunStatus,
  StartRequest,
  StressSession,
  StressSessionSummary,
  SystemInfo,
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

/** An overlay status for tests: off, no game, the four built-in profiles; override what matters. */
export function makeOverlayStatus(over: Partial<OverlayStatus> = {}): OverlayStatus {
  return {
    enabled: false,
    process: 'off',
    processReason: null,
    frames: 'off',
    framesDetail: null,
    target: null,
    activeProfile: null,
    profiles: ['builtin-minimal-fps', 'builtin-gaming', 'builtin-full', 'builtin-bar'].map((id) => ({
      id,
      name: `overlay.template.${id}`,
      builtin: true,
    })),
    diagnostics: [],
    hiddenByUser: false,
    hotkeys: { toggle: { ...NO_HOTKEY }, nextProfile: { ...NO_HOTKEY }, benchmark: { ...NO_HOTKEY } },
    preview: false,
    previewFailure: null,
    benchmark: { state: 'idle', game: null, elapsedS: null, error: null },
    ...over,
  };
}

/** A stress test run status for tests: idle, before any test; override what matters. */
export function makeRunStatus(over: Partial<RunStatus> = {}): RunStatus {
  return {
    state: 'idle',
    sessionId: '',
    component: 'cpu',
    objective: 'normal',
    preset: 'quick',
    elapsedMs: 0,
    totalMs: 0,
    phaseIndex: 0,
    phases: [],
    tempC: null,
    tempMaxC: null,
    stopC: null,
    powerW: null,
    clockMhz: null,
    checks: 0,
    errors: 0,
    wheaCorrected: 0,
    wheaFatal: 0,
    cores: [],
    currentCore: null,
    events: [],
    warnings: [],
    outcome: null,
    ...over,
  };
}

/** An 8-core, 16-thread CPU with AVX2, 32 GB of RAM and the service connected; override what matters. */
export function makeSystemInfo(over: Partial<SystemInfo> = {}): SystemInfo {
  return {
    cpuModel: 'Fake Ryzen 7 7800X3D',
    logical: 16,
    cores: 8,
    isa: ['avx2', 'sse2'],
    ramTotal: 32 * 1024 ** 3,
    ramBudget: 20 * 1024 ** 3,
    serviceConnected: true,
    tjmaxC: 89,
    stopC: 84,
    hypervisor: false,
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
  /** What `getOverlayStatus` returns; `emitOverlayStatus` replaces it and notifies listeners. */
  overlayStatus: OverlayStatus = makeOverlayStatus();
  /** Every overlay call in order (`onOverlayStatus`, `getOverlayStatus`, `overlayRetry`, ...). */
  overlayCalls: string[] = [];
  /** Every `setLogHotkeysSuspended` call, in order. */
  hotkeySuspensions: boolean[] = [];
  /** What `getUpdateStatus` returns; `emitUpdateStatus` replaces it and notifies listeners. */
  updateStatus: UpdateStatus = { state: 'idle', current: '0.4.0', latest: null, checkedAtMs: null, error: null };
  /** What `checkUpdates` returns (and emits); null means it returns `updateStatus` unchanged. */
  checkResult: UpdateStatus | null = null;
  /** Set to reject `checkUpdates` with this error. */
  checkError: string | null = null;
  checkUpdatesCalls = 0;
  openReleasePageCalls = 0;
  /** What `exportSensorReport` returns; null is a cancelled dialog. */
  exportResult: ExportedReport | null = null;
  /** Set to reject `exportSensorReport` with this text instead of resolving. */
  exportError: string | null = null;
  exportSensorReportCalls = 0;
  revealSensorReportCalls = 0;
  /** The profile files by id; the editor commands read and write them. */
  profiles: Record<string, EditableProfile> = {};
  /** Set to reject every editor profile command with this error. */
  editorError: CommandError | null = null;
  /** Editor calls in order (`overlaySaveProfile:<id>`, `overlayEditorDirty:true`, ...), without `overlayEditorProfile`. */
  editorCalls: string[] = [];
  /** Every `overlayEditorProfile` argument, in order. */
  editorProfiles: (string | null)[] = [];
  /** Every `overlayPreview` argument, in order. */
  previews: (string | null)[] = [];
  /** What `overlayImportProfile` imports (stored under a new id); null is a cancelled dialog. */
  importProfileJson: string | null = null;
  /** What `overlayExportProfile` answers. */
  exportProfileResult = true;
  fontFamilies: string[] = ['Segoe UI', 'Consolas'];
  /** What `benchmarkList` returns; `benchmarkCalls` records the benchmark commands. */
  benchmarks: BenchmarkEntry[] = [];
  benchmarkCalls: string[] = [];
  #nextProfile = 1;
  #editorDataListeners = new Set<(d: EditorData) => void>();
  #previewListeners = new Set<(e: { open: boolean }) => void>();
  #editorQuitListeners = new Set<() => void>();
  #updateListeners = new Set<(s: UpdateStatus) => void>();
  #logListeners = new Set<(s: LogStatus) => void>();
  #overlayListeners = new Set<(s: OverlayStatus) => void>();
  /** What `getDiskStates` answers. */
  diskStates: DiskStateEntry[] = [];
  #diskListeners = new Set<(s: DiskStateEntry[]) => void>();
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

  async getDiskStates(): Promise<DiskStateEntry[]> {
    return [...this.diskStates];
  }

  async onDiskStates(cb: (s: DiskStateEntry[]) => void): Promise<Unsubscribe> {
    this.#diskListeners.add(cb);
    return () => this.#diskListeners.delete(cb);
  }

  /** Number of live `onDiskStates` listeners. */
  get diskStateListeners(): number {
    return this.#diskListeners.size;
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

  async getOverlayStatus(): Promise<OverlayStatus | null> {
    this.overlayCalls.push('getOverlayStatus');
    return this.overlayStatus;
  }

  async onOverlayStatus(cb: (s: OverlayStatus) => void): Promise<Unsubscribe> {
    this.overlayCalls.push('onOverlayStatus');
    this.#overlayListeners.add(cb);
    return () => this.#overlayListeners.delete(cb);
  }

  async overlayRetry(): Promise<void> {
    this.overlayCalls.push('overlayRetry');
  }

  async overlayReloadProfiles(): Promise<void> {
    this.overlayCalls.push('overlayReloadProfiles');
  }

  async setOverlayHidden(hidden: boolean): Promise<void> {
    this.overlayCalls.push(`setOverlayHidden:${hidden}`);
  }

  #editor(call: string): void {
    this.editorCalls.push(call);
    if (this.editorError !== null) throw this.editorError;
  }

  #store(json: string): string {
    const id = `00000000-0000-4000-8000-${String(1000 + this.#nextProfile++).padStart(12, '0')}`;
    this.profiles[id] = { id, builtin: false, json };
    return id;
  }

  async overlayLoadProfile(id: string): Promise<EditableProfile> {
    this.#editor(`overlayLoadProfile:${id}`);
    const profile = this.profiles[id];
    if (profile === undefined) throw { key: 'editor.error.notFound', detail: null } satisfies CommandError;
    return { ...profile };
  }

  async overlaySaveProfile(id: string | null, json: string): Promise<string> {
    this.#editor(`overlaySaveProfile:${id}`);
    if (id === null) return this.#store(json);
    this.profiles[id] = { id, builtin: false, json };
    return id;
  }

  async overlayDeleteProfile(id: string): Promise<void> {
    this.#editor(`overlayDeleteProfile:${id}`);
    delete this.profiles[id];
  }

  async overlayDuplicateProfile(id: string): Promise<string> {
    this.#editor(`overlayDuplicateProfile:${id}`);
    const from = this.profiles[id];
    if (from === undefined) throw { key: 'editor.error.notFound', detail: null } satisfies CommandError;
    return this.#store(from.json);
  }

  async overlayImportProfile(): Promise<string | null> {
    this.#editor('overlayImportProfile');
    return this.importProfileJson === null ? null : this.#store(this.importProfileJson);
  }

  async overlayExportProfile(id: string): Promise<boolean> {
    this.#editor(`overlayExportProfile:${id}`);
    return this.exportProfileResult;
  }

  async overlayFontFamilies(): Promise<string[]> {
    return [...this.fontFamilies];
  }

  async overlayPreview(json: string | null): Promise<void> {
    this.previews.push(json);
  }

  async overlayEditorProfile(json: string | null): Promise<void> {
    this.editorProfiles.push(json);
  }

  async overlayUseNow(id: string): Promise<void> {
    this.editorCalls.push(`overlayUseNow:${id}`);
  }

  async overlayEditorDirty(dirty: boolean): Promise<void> {
    this.editorCalls.push(`overlayEditorDirty:${dirty}`);
  }

  async openOverlayEditor(): Promise<void> {
    this.editorCalls.push('openOverlayEditor');
  }

  async appQuitConfirmed(): Promise<void> {
    this.editorCalls.push('appQuitConfirmed');
  }

  async onOverlayEditorData(cb: (d: EditorData) => void): Promise<Unsubscribe> {
    this.#editorDataListeners.add(cb);
    return () => this.#editorDataListeners.delete(cb);
  }

  async onOverlayPreview(cb: (e: { open: boolean }) => void): Promise<Unsubscribe> {
    this.#previewListeners.add(cb);
    return () => this.#previewListeners.delete(cb);
  }

  /** What the stress test reads return; `performanceCalls` records every call in order. */
  performanceStatusValue: RunStatus = makeRunStatus();
  performanceSystemInfo: SystemInfo = makeSystemInfo();
  performanceSessions: StressSessionSummary[] = [];
  /** Full sessions by id, for `performanceSession`. */
  performanceSessionsById: Record<string, StressSession> = {};
  performancePlan: Plan = { seed: 1, ram_bytes: 0, phases: [] };
  performanceCalls: string[] = [];
  performanceStartRequests: StartRequest[] = [];
  performancePreviewRequests: StartRequest[] = [];
  /** Set to reject `performanceStart` with this text instead of starting. */
  performanceStartError: string | null = null;
  readonly performanceStatusListeners = new Set<(status: RunStatus) => void>();

  async performanceSystem(): Promise<SystemInfo> {
    this.performanceCalls.push('performanceSystem');
    return structuredClone(this.performanceSystemInfo);
  }

  async performancePreview(request: StartRequest): Promise<Plan> {
    this.performanceCalls.push('performancePreview');
    this.performancePreviewRequests.push(structuredClone(request));
    return structuredClone(this.performancePlan);
  }

  async performanceStart(request: StartRequest): Promise<string> {
    this.performanceCalls.push('performanceStart');
    if (this.performanceStartError !== null) throw this.performanceStartError;
    this.performanceStartRequests.push(structuredClone(request));
    return 'fake-session';
  }

  async performanceStop(): Promise<void> {
    this.performanceCalls.push('performanceStop');
  }

  async performanceStatus(): Promise<RunStatus> {
    this.performanceCalls.push('performanceStatus');
    return structuredClone(this.performanceStatusValue);
  }

  async performanceHistory(): Promise<StressSessionSummary[]> {
    this.performanceCalls.push('performanceHistory');
    return structuredClone(this.performanceSessions);
  }

  async performanceSession(id: string): Promise<StressSession | null> {
    this.performanceCalls.push(`performanceSession:${id}`);
    return structuredClone(this.performanceSessionsById[id] ?? null);
  }

  async performanceDelete(id: string): Promise<void> {
    this.performanceCalls.push(`performanceDelete:${id}`);
    this.performanceSessions = this.performanceSessions.filter((s) => s.id !== id);
    delete this.performanceSessionsById[id];
  }

  async performanceExport(id: string): Promise<string | null> {
    this.performanceCalls.push(`performanceExport:${id}`);
    return 'oma-stress-20261006-090507.json';
  }

  async onPerformanceStatus(cb: (status: RunStatus) => void): Promise<Unsubscribe> {
    this.performanceCalls.push('onPerformanceStatus');
    this.performanceStatusListeners.add(cb);
    return () => this.performanceStatusListeners.delete(cb);
  }

  /** Replaces the status and notifies the listeners, like a `performance-status` event. */
  emitPerformanceStatus(status: RunStatus): void {
    this.performanceStatusValue = status;
    this.performanceStatusListeners.forEach((cb) => cb(structuredClone(status)));
  }

  readonly performanceQuitListeners = new Set<() => void>();
  performanceQuitCalls = 0;

  async onPerformanceQuit(cb: () => void): Promise<Unsubscribe> {
    this.performanceQuitListeners.add(cb);
    return () => this.performanceQuitListeners.delete(cb);
  }

  async performanceQuitConfirmed(): Promise<void> {
    this.performanceQuitCalls++;
  }

  async onOverlayEditorQuit(cb: () => void): Promise<Unsubscribe> {
    this.#editorQuitListeners.add(cb);
    return () => this.#editorQuitListeners.delete(cb);
  }

  emitEditorData(data: EditorData): void {
    this.#editorDataListeners.forEach((cb) => cb(data));
  }

  emitPreview(open: boolean): void {
    this.#previewListeners.forEach((cb) => cb({ open }));
  }

  emitEditorQuit(): void {
    this.#editorQuitListeners.forEach((cb) => cb());
  }

  async benchmarkToggle(): Promise<void> {
    this.benchmarkCalls.push('benchmarkToggle');
  }

  async benchmarkList(): Promise<BenchmarkEntry[]> {
    this.benchmarkCalls.push('benchmarkList');
    return structuredClone(this.benchmarks);
  }

  async benchmarkOpenCsv(id: string): Promise<void> {
    this.benchmarkCalls.push(`benchmarkOpenCsv:${id}`);
  }

  async benchmarkOpenFolder(): Promise<void> {
    this.benchmarkCalls.push('benchmarkOpenFolder');
  }

  async benchmarkDelete(id: string): Promise<void> {
    this.benchmarkCalls.push(`benchmarkDelete:${id}`);
    this.benchmarks = this.benchmarks.filter((b) => b.id !== id);
  }

  async checkUpdates(): Promise<UpdateStatus> {
    this.checkUpdatesCalls++;
    if (this.checkError !== null) throw this.checkError;
    if (this.checkResult !== null) this.emitUpdateStatus(this.checkResult);
    return this.updateStatus;
  }

  async getUpdateStatus(): Promise<UpdateStatus> {
    return this.updateStatus;
  }

  async openReleasePage(): Promise<void> {
    this.openReleasePageCalls++;
  }

  async exportSensorReport(): Promise<ExportedReport | null> {
    this.exportSensorReportCalls++;
    if (this.exportError !== null) throw this.exportError;
    return this.exportResult;
  }

  async revealSensorReport(): Promise<void> {
    this.revealSensorReportCalls++;
  }

  async onUpdateStatus(cb: (s: UpdateStatus) => void): Promise<Unsubscribe> {
    this.#updateListeners.add(cb);
    return () => this.#updateListeners.delete(cb);
  }

  emitUpdateStatus(status: UpdateStatus): void {
    this.updateStatus = status;
    this.#updateListeners.forEach((cb) => cb(status));
  }

  /** Number of live `oma:log` listeners. */
  get logListenerCount(): number {
    return this.#logListeners.size;
  }

  emitLogStatus(status: LogStatus): void {
    this.logStatus = status;
    this.#logListeners.forEach((cb) => cb(status));
  }

  /** Number of live `overlay-status` listeners. */
  get overlayListenerCount(): number {
    return this.#overlayListeners.size;
  }

  emitOverlayStatus(status: OverlayStatus): void {
    this.overlayStatus = status;
    this.#overlayListeners.forEach((cb) => cb(status));
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

  emitDiskStates(states: DiskStateEntry[]): void {
    this.diskStates = states;
    this.#diskListeners.forEach((cb) => cb(states));
  }

  emitServiceStatus(status: ServiceStatus): void {
    this.serviceStatus = status;
    this.#serviceListeners.forEach((cb) => cb(status));
  }
}
