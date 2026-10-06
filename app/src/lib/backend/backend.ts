import type {
  AppInfo,
  ExportedReport,
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
  OverlayStatus,
  UpdateStatus,
  BenchmarkEntry,
  EditableProfile,
  EditorData,
  Plan,
  RunStatus,
  StartRequest,
  StressSession,
  StressSessionSummary,
  SystemInfo,
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
  /** The overlay's current status; null where the shell has no overlay (off Windows). */
  getOverlayStatus(): Promise<OverlayStatus | null>;
  /** Emitted at every change, already in order: the last event is the newest status. */
  onOverlayStatus(cb: (status: OverlayStatus) => void): Promise<Unsubscribe>;
  /** «Try again» on a failed frame engine or overlay process; the outcome arrives as an event. */
  overlayRetry(): Promise<void>;
  /** Reads the profile folder again; the new catalog arrives as an event. */
  overlayReloadProfiles(): Promise<void>;
  /** Hides or shows the overlay like the hotkey and the tray (not saved); the outcome arrives as an event. */
  setOverlayHidden(hidden: boolean): Promise<void>;
  /** Opens the editor's profile: a built-in (read-only, name translated) or a user profile. Editor commands reject with a `CommandError`. */
  overlayLoadProfile(id: string): Promise<EditableProfile>;
  /** Validates and writes the profile; without `id` under a new id with a unique name. Replies with the id. */
  overlaySaveProfile(id: string | null, json: string): Promise<string>;
  overlayDeleteProfile(id: string): Promise<void>;
  /** A user copy of a profile (a built-in bound to this PC by role); replies with the new id. */
  overlayDuplicateProfile(id: string): Promise<string>;
  /** File dialog and import; null when the dialog is cancelled. */
  overlayImportProfile(): Promise<string | null>;
  /** Save dialog and export; false when the dialog is cancelled. */
  overlayExportProfile(id: string): Promise<boolean>;
  /** The installed font families, read once per session. */
  overlayFontFamilies(): Promise<string[]>;
  /** Shows `json` in the preview window (opening it), or closes it with null. */
  overlayPreview(json: string | null): Promise<void>;
  /** The profile on the editor's canvas, for its low windows; null when the editor closes. */
  overlayEditorProfile(json: string | null): Promise<void>;
  /** Makes a saved profile the active one until the target changes (DD9). */
  overlayUseNow(id: string): Promise<void>;
  /** Tells the shell whether the editor holds unsaved changes (the tray's «Quit» asks first). */
  overlayEditorDirty(dirty: boolean): Promise<void>;
  openOverlayEditor(): Promise<void>;
  /** The editor's answer to `overlay-editor-quit` after «Save» or «Discard»: the app exits. */
  appQuitConfirmed(): Promise<void>;
  /** Metrics and new frame times for the canvas, only while the editor is open. */
  onOverlayEditorData(cb: (data: EditorData) => void): Promise<Unsubscribe>;
  /** The preview window opened or closed. */
  onOverlayPreview(cb: (event: { open: boolean }) => void): Promise<Unsubscribe>;
  /** The tray's «Quit» waits for the editor to save or discard its changes. */
  onOverlayEditorQuit(cb: () => void): Promise<Unsubscribe>;
  /** What the machine offers for a stress test. */
  performanceSystem(): Promise<SystemInfo>;
  /** The plan a request would run; rejects with the reason it cannot be built. */
  performancePreview(request: StartRequest): Promise<Plan>;
  /** Starts a test and replies with its session id; rejects with the reason (one test at a time). */
  performanceStart(request: StartRequest): Promise<string>;
  /** Asks the running test to stop; it is saved as stopped by the user. */
  performanceStop(): Promise<void>;
  performanceStatus(): Promise<RunStatus>;
  /** The saved sessions, newest first. */
  performanceHistory(): Promise<StressSessionSummary[]>;
  /** A saved session, or null when it is gone. */
  performanceSession(id: string): Promise<StressSession | null>;
  /** Deletes a saved session; rejects for the running one. */
  performanceDelete(id: string): Promise<void>;
  /** Save dialog and export as JSON: the file name, or null when the dialog is cancelled. */
  performanceExport(id: string): Promise<string | null>;
  /** The run status on every change and at 1 Hz during a test, only while a window is open. */
  onPerformanceStatus(cb: (status: RunStatus) => void): Promise<Unsubscribe>;
  /** The tray's «Quit» during a stress test: the window asks before it goes on. */
  onPerformanceQuit(cb: () => void): Promise<Unsubscribe>;
  /** The answer «stop and quit»: the test ends (saved as stopped) and the app exits. */
  performanceQuitConfirmed(): Promise<void>;
  /** Starts or stops a benchmark capture, like the hotkey; the outcome arrives in `OverlayStatus.benchmark`. */
  benchmarkToggle(): Promise<void>;
  /** The saved benchmarks, newest first. */
  benchmarkList(): Promise<BenchmarkEntry[]>;
  /** Opens a benchmark's CSV; rejects with the system's text. */
  benchmarkOpenCsv(id: string): Promise<void>;
  /** Opens the benchmarks folder; rejects with `log.error.folderMissing` or the system's text. */
  benchmarkOpenFolder(): Promise<void>;
  benchmarkDelete(id: string): Promise<void>;
  /** Asks GitHub for the latest release now; replies with the resulting status (rejects only if the background task panics). */
  checkUpdates(): Promise<UpdateStatus>;
  getUpdateStatus(): Promise<UpdateStatus>;
  /** Opens the page of the newer release in the browser; rejects when no newer release is known. */
  openReleasePage(): Promise<void>;
  onUpdateStatus(cb: (status: UpdateStatus) => void): Promise<Unsubscribe>;
  /** Builds the anonymous sensor report and saves it where the user chooses; null when the dialog is cancelled; rejects with the system's text. */
  exportSensorReport(): Promise<ExportedReport | null>;
  /** Opens the folder of the last exported report; rejects when there is none or with the system's text. */
  revealSensorReport(): Promise<void>;
}
