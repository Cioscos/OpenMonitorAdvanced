import type {
  AppInfo,
  AutostartStatus,
  GpuProcess,
  HealthClock,
  HealthReport,
  HistorySeed,
  KnownPath,
  LegacyWebviewState,
  Schema,
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
  UpdateStatus,
} from '../types';

export type Unsubscribe = () => void;

/** Everything the UI needs from the sampling core (Tauri, or a mock in the browser). */
export interface Backend {
  getSchema(): Promise<Schema>;
  /**
   * Last `seconds` of history (at most 3600). With `maxPoints` the core returns a
   * min/max envelope of at most that many rows instead of the raw samples.
   */
  getHistory(ids: string[], seconds: number, maxPoints?: number): Promise<HistorySeed>;
  onSchema(cb: (schema: Schema) => void): Promise<Unsubscribe>;
  onSnapshot(cb: (snapshot: Snapshot) => void): Promise<Unsubscribe>;
  /** The power state of every identified disk. */
  getDiskStates(): Promise<DiskStateEntry[]>;
  /** Always the full list, emitted when it changes; an empty list revokes the earlier states. */
  onDiskStates(cb: (states: DiskStateEntry[]) => void): Promise<Unsubscribe>;
  /** GPU safe-mode status of this session. */
  getStartupStatus(): Promise<StartupStatus>;
  /** Loads the GPU vendor libraries without a restart; returns the new status. */
  enableVendorLibraries(): Promise<StartupStatus>;
  /** Min/max/avg since the app started (or the last reset), one entry per id. */
  getStats(ids: string[]): Promise<StatsReply>;
  /** Restarts min/max/avg of these sensors; unknown ids are ignored. */
  resetStats(ids: string[]): Promise<void>;
  /** Start time and sampling interval of the core. */
  getSession(): Promise<Session>;
  /** Processes using this GPU, busiest first, at most 20; empty for an unknown device. */
  getGpuProcesses(deviceId: string): Promise<GpuProcess[]>;
  /** Current status of the OpenMonitor Advanced sensor service. */
  getServiceStatus(): Promise<ServiceStatus>;
  onServiceStatus(cb: (status: ServiceStatus) => void): Promise<Unsubscribe>;
  /** Turns anti-cheat compatible mode on or off; the command's reply may precede the SCM outcome. */
  setAntiCheat(enabled: boolean): Promise<ServiceStatus>;
  /** Starts the service when it is unreachable; the command's reply may precede the SCM outcome. */
  startService(): Promise<ServiceStatus>;
  /** The settings with their revision and persistence state. */
  getSettings(): Promise<SettingsState>;
  /** Applies a patch; rejects with a `PatchError` (`{ field, key }`) and changes nothing when it is invalid. */
  updateSettings(patch: SettingsPatch): Promise<SettingsState>;
  /** Drops the override of a built-in rule ("Restore"); rejects with `rules.error.unknownRule` for other ids. */
  resetRuleOverride(ruleId: string): Promise<SettingsState>;
  /** Every applied change, also those made from the tray. Listeners must drop states with an old `seq`. */
  onSettings(cb: (state: SettingsState) => void): Promise<Unsubscribe>;
  /**
   * Imports the old `localStorage` state once. Resolves only after the values and the marker are on
   * disk; rejects with `persist_failed` (try again at the next start) or `read_only`.
   */
  importWebviewState(legacy: LegacyWebviewState): Promise<SettingsState>;
  /**
   * The latest request of a tray item or a toast, returned once: read at mount, and after each
   * `onNavigate` event as its acknowledgment (a page still loading may have missed the event).
   */
  takePendingView(): Promise<NavigationTarget | null>;
  /** A tray item or a toast asked for a view while the window was open. */
  onNavigate(cb: (target: NavigationTarget) => void): Promise<Unsubscribe>;
  /** The rules engine's current verdict. */
  getHealth(): Promise<HealthReport>;
  /** Emitted when the report changes. Listeners must drop reports with an old `revision`. */
  onHealth(cb: (report: HealthReport) => void): Promise<Unsubscribe>;
  /** Time in the current level, monotonic; belongs to the report of the same `revision`. */
  getHealthClock(): Promise<HealthClock>;
  /** At most once a second while the window is open. */
  onHealthClock(cb: (clock: HealthClock) => void): Promise<Unsubscribe>;
  /** Resolved thresholds, levels and problems of every effective rule; poll it at most once a second. */
  getRuleStatus(): Promise<RuleStatus[]>;
  /** The built-in rules as shipped, in display order (`oma_core::rules::default_rules`). */
  getDefaultRules(): Promise<Rule[]>;
  /** Re-reads the start-up entry as Windows sees it. */
  refreshAutostart(): Promise<AutostartStatus>;
  /** Versions and folders for the About page. */
  getAppInfo(): Promise<AppInfo>;
  /** Opens one of the fixed places with the shell; rejects with the system's text. */
  openKnownPath(target: KnownPath): Promise<void>;
  /** The CSV log's current status. */
  getLogStatus(): Promise<LogStatus>;
  /** Emitted at every change while the window exists. Listeners must drop statuses with an old `revision`. */
  onLogStatus(cb: (status: LogStatus) => void): Promise<Unsubscribe>;
  /** The four commands reply with the status they produced; it goes through the same `revision` filter as the events. */
  logStart(): Promise<LogStatus>;
  logPause(): Promise<LogStatus>;
  logResume(): Promise<LogStatus>;
  logStop(): Promise<LogStatus>;
  /** Opens the log folder; rejects with the i18n key `log.error.folderMissing` or the system's text. */
  openLogFolder(): Promise<void>;
  /** Folder picker; null when the user cancels. */
  pickLogFolder(): Promise<string | null>;
  /**
   * While a hotkey capture box has focus, the log hotkeys are released (so the box receives their
   * keys) and their presses ignored; false registers them again.
   */
  setLogHotkeysSuspended(suspended: boolean): Promise<void>;
  /** Asks GitHub for the latest release now; replies with the resulting status (rejects only if the background task panics). */
  checkUpdates(): Promise<UpdateStatus>;
  getUpdateStatus(): Promise<UpdateStatus>;
  /** Opens the page of the newer release in the browser; rejects when no newer release is known. */
  openReleasePage(): Promise<void>;
  onUpdateStatus(cb: (status: UpdateStatus) => void): Promise<Unsubscribe>;
}
