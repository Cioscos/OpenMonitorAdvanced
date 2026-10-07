import defaultRulesFixture from '../../test/fixtures/default-rules.json';
import { applyOverride } from '../rules';
import type {
  LegacyWebviewState,
  LevelSpec,
  PatchError,
  Persistence,
  Rule,
  RuleOverride,
  Settings,
  SettingsDiagnostic,
  SettingsPatch,
  SettingsState,
} from '../types';
import type { Unsubscribe } from './backend';

/** Accepted values, the same sets as `oma_core::settings`. */
const INTERVALS = [500, 1000, 1500, 2000, 2500, 3000, 3500, 4000, 4500, 5000];
const WINDOWS = [60, 300, 1800, 3600];
const FPS = [15, 30, 60];
const EVERY_TICKS = [1, 2, 5, 10, 30, 60];
const MAX_FILE_MB = [10, 2048] as const;
const MAX_LOG_SENSORS = 4096;
const TEXT_HZ = [2, 4];
const MAX_GAMES = 256;
const MAX_EXE_BYTES = 260;
const BUILTIN_PROFILES = ['builtin-minimal-fps', 'builtin-gaming', 'builtin-full', 'builtin-bar'];

const ENUMS: Record<string, readonly string[]> = {
  'general.language': ['system', 'en', 'it'],
  'general.temperatureUnit': ['c', 'f'],
  'general.throughputUnit': ['bits', 'bytes'],
  'general.defaultView': ['simple', 'advanced', 'last'],
  'view.last': ['simple', 'advanced'],
  'overlay.attach': ['window', 'monitor'],
};
const NUMBERS: Record<string, readonly number[]> = {
  'general.intervalMs': INTERVALS,
  'general.chartFps': FPS,
  'advanced.window': WINDOWS,
  'log.everyTicks': EVERY_TICKS,
  'overlay.chartFps': FPS,
  'overlay.textHz': TEXT_HZ,
};

/** The built-in rules, from the fixture that a Rust test keeps equal to `oma_core::rules::default_rules`. */
const DEFAULT_RULES = defaultRulesFixture as Rule[];
const BUILTIN_RULES = DEFAULT_RULES.map((rule) => rule.id);
const MAX_DURATION_S = 600;
const MAX_CUSTOM_RULES = 256;
const CUSTOM_ID = /^custom-[0-9a-f-]{36}$/;

/** `overrides`: free keys, each an object of `OVERRIDE_SHAPE`. */
type Node = 'leaf' | 'nullable' | 'free' | 'overrides' | { [key: string]: Node };

const OVERRIDE_SHAPE: { [key: string]: Node } = { enabled: 'leaf', warn: 'nullable', crit: 'nullable', hysteresis: 'leaf', notify: 'leaf' };

/** Patchable shape, as in `patch.rs`. */
const SHAPE: { [key: string]: Node } = {
  general: { language: 'leaf', temperatureUnit: 'leaf', throughputUnit: 'leaf', intervalMs: 'leaf', chartFps: 'leaf', defaultView: 'leaf' },
  tray: { closeToTray: 'leaf', autostart: 'leaf', iconSensor: 'nullable' },
  updates: { checkAutomatically: 'leaf' },
  sources: {
    vendorLibraries: { nvml: 'leaf', nvapi: 'leaf', adl: 'leaf', igcl: 'leaf' },
    antiCheat: 'leaf',
    serviceModules: { cpu: 'leaf', motherboard: 'leaf', memory: 'leaf', storage: 'leaf', controller: 'leaf', psu: 'leaf' },
    smartDisabledDrives: 'leaf',
    smartEnabledDrives: 'leaf',
  },
  advanced: { section: 'nullable', window: 'nullable', series: 'free' },
  view: { last: 'nullable' },
  log: { folder: 'nullable', sensors: 'nullable', everyTicks: 'leaf', maxFileMb: 'leaf', hotkeyToggle: 'nullable', hotkeyPause: 'nullable' },
  overlay: {
    enabled: 'leaf',
    chartFps: 'leaf',
    textHz: 'leaf',
    hideFromCapture: 'leaf',
    attach: 'leaf',
    trackPcLatency: 'leaf',
    trackGpu: 'leaf',
    defaultProfile: 'leaf',
    gameProfiles: 'free',
    blockedGames: 'leaf',
    hotkeyToggle: 'nullable',
    hotkeyNextProfile: 'nullable',
    hotkeyBenchmark: 'nullable',
    editorBounds: 'nullable',
  },
  performance: { thermalStop: 'leaf', cpuStopC: 'nullable', gpuStopC: 'leaf', stopOnFirstError: 'nullable', ramSharePercent: 'leaf', riskNoticeSeen: 'leaf' },
  rules: { overrides: 'overrides', custom: 'leaf' },
};
const READ_ONLY = ['version', 'migrations'];

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const fail = (field: string, key: string): never => {
  throw { field, key } satisfies PatchError;
};

/** Deep copy through JSON, like the IPC boundary: also copes with Svelte proxies. */
const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

const join = (parent: string, key: string) => (parent ? `${parent}.${key}` : key);

function checkShape(patch: Record<string, unknown>, shape: { [key: string]: Node }, parent: string): void {
  for (const [key, value] of Object.entries(patch)) {
    const field = join(parent, key);
    const node = shape[key];
    if (node === undefined) fail(field, 'settings.error.unknownField');
    if (node === 'leaf' || node === 'free') {
      if (value === null) fail(field, 'settings.error.null');
    } else if (node === 'overrides') {
      if (value === null) fail(field, 'settings.error.null');
      if (isObject(value)) {
        for (const [id, over] of Object.entries(value)) {
          if (over === null) fail(`${field}.${id}`, 'settings.error.null');
          if (isObject(over)) checkShape(over, OVERRIDE_SHAPE, `${field}.${id}`);
        }
      }
    } else if (node !== 'nullable') {
      if (value === null) fail(field, 'settings.error.null');
      if (isObject(value)) checkShape(value, node as { [key: string]: Node }, field);
    }
  }
}

/** Validates the values a merged result holds at `path` the way the strict decoder does. */
function checkValue(path: string, value: unknown): void {
  if (value === null || value === undefined) return;
  const enumValues = ENUMS[path];
  if (enumValues) {
    if (typeof value !== 'string' || !enumValues.includes(value)) fail(path, 'settings.error.type');
    return;
  }
  const numbers = NUMBERS[path];
  if (numbers) {
    if (typeof value !== 'number') fail(path, 'settings.error.type');
    if (!numbers.includes(value as number)) fail(path, 'settings.error.range');
  }
}

function checkTypes(merged: Record<string, unknown>): void {
  for (const path of Object.keys({ ...ENUMS, ...NUMBERS })) {
    const [section, key] = path.split('.');
    const group = merged[section];
    if (isObject(group)) checkValue(path, group[key]);
  }
  const { tray, sources, advanced } = merged as { tray: Record<string, unknown>; sources: Record<string, unknown>; advanced: Record<string, unknown> };
  for (const key of ['closeToTray', 'autostart']) if (typeof tray[key] !== 'boolean') fail(`tray.${key}`, 'settings.error.type');
  if (tray.iconSensor !== null && typeof tray.iconSensor !== 'string') fail('tray.iconSensor', 'settings.error.type');
  if (typeof (merged.updates as Record<string, unknown>).checkAutomatically !== 'boolean') fail('updates.checkAutomatically', 'settings.error.type');
  if (typeof sources.antiCheat !== 'boolean') fail('sources.antiCheat', 'settings.error.type');
  if (advanced.section != null && typeof advanced.section !== 'string') fail('advanced.section', 'settings.error.type');
  const series = advanced.series;
  if (!isObject(series) || !Object.values(series).every((ids) => Array.isArray(ids) && ids.every((id) => typeof id === 'string'))) {
    fail('advanced.series', 'settings.error.type');
  }
}

/** The performance checks of the strict decoder: the threshold and the RAM share are integers in range. */
function checkPerformance(p: Record<string, unknown>): void {
  for (const key of ['thermalStop', 'riskNoticeSeen']) if (typeof p[key] !== 'boolean') fail(`performance.${key}`, 'settings.error.type');
  if (p.stopOnFirstError !== null && typeof p.stopOnFirstError !== 'boolean') fail('performance.stopOnFirstError', 'settings.error.type');
  const ranges = [
    ['cpuStopC', 60, 110],
    ['gpuStopC', 60, 110],
    ['ramSharePercent', 10, 90],
  ] as const;
  for (const [key, min, max] of ranges) {
    const value = p[key];
    if (value === null && key === 'cpuStopC') continue;
    if (typeof value !== 'number') fail(`performance.${key}`, 'settings.error.type');
    if (!Number.isInteger(value) || (value as number) < min || (value as number) > max) fail(`performance.${key}`, 'settings.error.range');
  }
}

/** The canonical spelling of a hotkey (`Ctrl+Alt+Shift+R`), or null when unreadable or with fewer than two modifiers. */
export function canonicalHotkey(text: string): string | null {
  const parts = text.split('+').map((part) => part.trim());
  if (parts.some((part) => part === '')) return null;
  const key = parts.pop() as string;
  const found = new Set<string>();
  for (const part of parts) {
    const name = { ctrl: 'Ctrl', control: 'Ctrl', alt: 'Alt', shift: 'Shift' }[part.toLowerCase()];
    if (!name || found.has(name)) return null;
    found.add(name);
  }
  if (found.size < 2) return null;
  const fn = /^f([1-9]\d?)$/i.exec(key);
  const spelled = /^[a-z0-9]$/i.test(key) ? key.toUpperCase() : fn && Number(fn[1]) <= 24 ? `F${fn[1]}` : null;
  if (!spelled) return null;
  return ['Ctrl', 'Alt', 'Shift'].filter((name) => found.has(name)).concat(spelled).join('+');
}

/** `is_absolute_folder`: `X:` plus a slash, or a UNC share. */
const isAbsoluteFolder = (path: string) => /^[A-Za-z]:[\\/]/.test(path) || /^\\\\[^\\]+\\[^\\]+/.test(path);

/** The log checks of the strict decoder; hotkeys are stored canonical. */
function checkLog(log: Record<string, unknown>): void {
  const { folder, sensors, maxFileMb, hotkeyToggle, hotkeyPause } = log;
  if (folder !== null && typeof folder !== 'string') fail('log.folder', 'settings.error.type');
  if (typeof folder === 'string' && !isAbsoluteFolder(folder)) fail('log.folder', 'settings.error.folder');
  if (sensors !== null) {
    if (!Array.isArray(sensors) || !sensors.every((id) => typeof id === 'string')) fail('log.sensors', 'settings.error.type');
    const ids = sensors as string[];
    if (ids.length > MAX_LOG_SENSORS || ids.some((id) => id === '') || new Set(ids).size !== ids.length) fail('log.sensors', 'settings.error.sensors');
  }
  if (typeof maxFileMb !== 'number') fail('log.maxFileMb', 'settings.error.type');
  if ((maxFileMb as number) < MAX_FILE_MB[0] || (maxFileMb as number) > MAX_FILE_MB[1] || !Number.isInteger(maxFileMb)) fail('log.maxFileMb', 'settings.error.range');
  for (const key of ['hotkeyToggle', 'hotkeyPause'] as const) {
    const value = log[key];
    if (value === null) continue;
    if (typeof value !== 'string') fail(`log.${key}`, 'settings.error.type');
    const canonical = canonicalHotkey(value as string);
    if (!canonical) fail(`log.${key}`, 'settings.error.hotkey');
    log[key] = canonical;
  }
  if (log.hotkeyPause !== null && log.hotkeyPause === log.hotkeyToggle) fail('log.hotkeyPause', 'settings.error.hotkeyDuplicate');
  void hotkeyToggle;
  void hotkeyPause;
}

const OVERLAY_HOTKEYS = ['hotkeyToggle', 'hotkeyNextProfile', 'hotkeyBenchmark'] as const;

/** `normalize_exe`: lowercase, `.exe` after a stem, no backslash, slash or colon, 260 bytes at most; null when invalid. */
const normalizeExe = (name: string): string | null => {
  const lower = name.toLowerCase();
  const valid = lower.length > 4 && lower.endsWith('.exe') && !/[\\/:]/.test(lower) && new TextEncoder().encode(lower).length <= MAX_EXE_BYTES;
  return valid ? lower : null;
};

/** `is_profile_id`: a built-in id or a lowercase `8-4-4-4-12` UUID. */
const isProfileId = (id: string) => BUILTIN_PROFILES.includes(id) || /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id);

/** The overlay checks of the strict decoder; names and hotkeys are stored normalised. */
function checkOverlay(overlay: Record<string, unknown>, log: Record<string, unknown>): void {
  for (const key of ['enabled', 'hideFromCapture', 'trackPcLatency', 'trackGpu']) {
    if (typeof overlay[key] !== 'boolean') fail(`overlay.${key}`, 'settings.error.type');
  }
  if (typeof overlay.defaultProfile !== 'string') fail('overlay.defaultProfile', 'settings.error.type');
  if (!isProfileId(overlay.defaultProfile as string)) fail('overlay.defaultProfile', 'settings.error.profileId');
  const profiles = overlay.gameProfiles;
  if (!isObject(profiles)) return fail('overlay.gameProfiles', 'settings.error.type');
  const normalised: Record<string, string> = {};
  for (const [name, id] of Object.entries(profiles)) {
    const path = `overlay.gameProfiles.${name}`;
    const exe = normalizeExe(name);
    if (exe === null) fail(path, 'settings.error.exe');
    if (typeof id !== 'string') fail(path, 'settings.error.type');
    if (!isProfileId(id as string)) fail(path, 'settings.error.profileId');
    normalised[exe as string] = id as string;
  }
  if (Object.keys(normalised).length > MAX_GAMES) fail('overlay.gameProfiles', 'settings.error.range');
  overlay.gameProfiles = normalised;
  const blocked = overlay.blockedGames;
  if (!Array.isArray(blocked)) return fail('overlay.blockedGames', 'settings.error.type');
  const games = blocked.map((name, i) => {
    if (typeof name !== 'string') fail(`overlay.blockedGames.${i}`, 'settings.error.type');
    return normalizeExe(name as string) ?? fail(`overlay.blockedGames.${i}`, 'settings.error.exe');
  });
  if (games.length > MAX_GAMES || new Set(games).size !== games.length) fail('overlay.blockedGames', 'settings.error.range');
  overlay.blockedGames = games;
  // Hotkeys differ from every earlier one: log toggle, log pause, then these in order.
  const taken = [log.hotkeyToggle, log.hotkeyPause].filter((x): x is string => typeof x === 'string');
  for (const key of OVERLAY_HOTKEYS) {
    const value = overlay[key];
    if (value === null) continue;
    if (typeof value !== 'string') fail(`overlay.${key}`, 'settings.error.type');
    const canonical = canonicalHotkey(value as string);
    if (!canonical) fail(`overlay.${key}`, 'settings.error.hotkey');
    if (taken.includes(canonical as string)) fail(`overlay.${key}`, 'settings.error.hotkeyDuplicate');
    taken.push(canonical as string);
    overlay[key] = canonical;
  }
  checkEditorBounds(overlay.editorBounds);
}

/** `WindowBounds::from_json`: four integers, at least 1100×700, nothing beyond ±32768. */
function checkEditorBounds(bounds: unknown): void {
  if (bounds === null) return;
  if (!isObject(bounds)) return fail('overlay.editorBounds', 'settings.error.type');
  const keys = ['x', 'y', 'width', 'height'];
  const values = keys.map((key) => bounds[key]);
  const valid =
    Object.keys(bounds).length === 4 &&
    values.every((v) => Number.isInteger(v) && Math.abs(v as number) <= 32768) &&
    (bounds.width as number) >= 1100 &&
    (bounds.height as number) >= 700;
  if (!valid) fail('overlay.editorBounds', 'settings.error.range');
}

/** The only rule checks of the mock: level shape, durations, threshold order and hysteresis. */
function checkLevels(path: string, rule: { warn?: LevelSpec | null; crit?: LevelSpec | null; hysteresis?: unknown }): void {
  for (const name of ['warn', 'crit'] as const) {
    const level = rule[name];
    if (level === undefined || level === null) continue;
    if (!isObject(level) || typeof level.durationS !== 'number') fail(`${path}.${name}`, 'settings.error.type');
    if (level.durationS < 0 || level.durationS > MAX_DURATION_S) fail(`${path}.${name}.durationS`, 'rules.error.duration');
  }
  const hysteresis = rule.hysteresis;
  if (hysteresis !== undefined) {
    if (!isObject(hysteresis) || typeof hysteresis.amount !== 'number' || typeof hysteresis.durationS !== 'number') {
      fail(`${path}.hysteresis`, 'settings.error.type');
    }
    const { amount, durationS } = hysteresis as { amount: number; durationS: number };
    if (amount < 0) fail(`${path}.hysteresis.amount`, 'rules.error.hysteresis');
    if (durationS < 0 || durationS > MAX_DURATION_S) fail(`${path}.hysteresis.durationS`, 'rules.error.duration');
  }
}

const fixedOf = (level: LevelSpec | null | undefined): number | undefined =>
  level?.threshold && 'fixed' in level.threshold ? level.threshold.fixed : undefined;

/** At least one level, then the levels, then the order of two fixed thresholds (`validate_rule`). */
function checkRule(path: string, rule: { warn?: LevelSpec | null; crit?: LevelSpec | null; condition?: string; hysteresis?: unknown }): void {
  if (!rule.warn && !rule.crit) fail(`${path}.levels`, 'rules.error.noLevel');
  checkLevels(path, rule);
  const [warn, crit] = [fixedOf(rule.warn), fixedOf(rule.crit)];
  if (warn !== undefined && crit !== undefined && (rule.condition === 'below' ? crit > warn : crit < warn)) {
    fail(`${path}.crit`, 'rules.error.order');
  }
}

/** A subset of `validate_rules`: enough for the UI to show errors next to the fields. */
function checkRules(rules: unknown): void {
  if (!isObject(rules)) return fail('rules', 'settings.error.type');
  const { overrides, custom } = rules;
  if (!isObject(overrides)) fail('rules.overrides', 'settings.error.type');
  if (!Array.isArray(custom)) fail('rules.custom', 'settings.error.type');
  for (const [id, over] of Object.entries(overrides as Record<string, unknown>)) {
    const path = `rules.overrides.${id}`;
    const builtin = DEFAULT_RULES.find((rule) => rule.id === id);
    if (!builtin) return fail(path, 'rules.error.unknownRule');
    if (!isObject(over)) fail(path, 'settings.error.type');
    // Like `validate_override`: the rule the override produces is checked whole.
    checkRule(path, applyOverride(builtin, over as RuleOverride));
  }
  const rulesList = custom as unknown[];
  if (rulesList.length > MAX_CUSTOM_RULES) fail('rules.custom', 'rules.error.tooMany');
  const seen = new Set<string>();
  rulesList.forEach((item, i) => {
    const path = `rules.custom.${i}`;
    if (!isObject(item) || typeof item.id !== 'string') return fail(path, 'settings.error.type');
    if (!CUSTOM_ID.test(item.id)) fail(`${path}.id`, 'rules.error.customId');
    checkRule(path, item as Parameters<typeof checkRule>[1]);
    if (seen.has(item.id)) fail(`${path}.id`, 'rules.error.duplicateId');
    seen.add(item.id);
  });
}

/** Objects merge recursively, everything else replaces, and so do the fields of a rule override. */
function merge(base: Record<string, unknown>, patch: Record<string, unknown>, path: string[] = []): void {
  for (const [key, value] of Object.entries(patch)) {
    if (value === undefined) continue;
    const existing = base[key];
    const wholeField =
      (path.length === 3 && path[0] === 'rules' && path[1] === 'overrides') || (path.length === 1 && path[0] === 'overlay' && (key === 'gameProfiles' || key === 'editorBounds'));
    if (isObject(existing) && isObject(value) && !wholeField) merge(existing, value, [...path, key]);
    else base[key] = clone(value);
  }
}

/** Settings of a fresh install, encoded like `oma_core::settings::encode`. */
export function defaultSettings(): Settings {
  return {
    version: 1,
    general: { language: 'system', temperatureUnit: 'c', throughputUnit: 'bits', intervalMs: 1000, chartFps: 60, defaultView: 'last' },
    tray: { closeToTray: true, autostart: false, iconSensor: null },
    updates: { checkAutomatically: false },
    sources: {
      vendorLibraries: { nvml: true, nvapi: true, adl: true, igcl: true },
      antiCheat: false,
      serviceModules: { cpu: true, motherboard: true, memory: true, storage: true, controller: true, psu: true },
      smartDisabledDrives: [],
      smartEnabledDrives: [],
    },
    advanced: { series: {} },
    view: {},
    rules: { overrides: {}, custom: [] },
    log: { folder: null, sensors: null, everyTicks: 1, maxFileMb: 100, hotkeyToggle: 'Ctrl+Alt+Shift+R', hotkeyPause: null },
    overlay: {
      enabled: false,
      chartFps: 30,
      textHz: 2,
      hideFromCapture: false,
      attach: 'window',
      trackPcLatency: false,
      trackGpu: false,
      defaultProfile: 'builtin-gaming',
      gameProfiles: {},
      blockedGames: [],
      hotkeyToggle: null,
      hotkeyNextProfile: null,
      hotkeyBenchmark: null,
      editorBounds: null,
    },
    performance: { thermalStop: true, cpuStopC: null, gpuStopC: 90, stopOnFirstError: null, ramSharePercent: 70, riskNoticeSeen: false },
    migrations: { serviceV1: false, webviewV1: false },
  };
}

/** Persistence state from `?settings=recovered|readOnly|error` in the URL; `ok` otherwise. */
export function parsePersistence(search: string): Persistence {
  switch (new URLSearchParams(search).get('settings')) {
    case 'recovered':
      return { kind: 'recovered', path: 'C:\\Users\\mock\\AppData\\Roaming\\OpenMonitorAdvanced\\settings.json.bad-20260101-000000-mock' };
    case 'readOnly':
      return { kind: 'readOnly', reason: 'newer_version' };
    case 'error':
      return { kind: 'error', reason: 'access denied' };
    default:
      return { kind: 'ok' };
  }
}

/**
 * In-memory settings with the core's revisions, `seq` and patch rules, for the browser mock and the
 * tests. Nothing is saved: `persistedRevision` follows `revision` at once.
 */
export class MockSettings {
  #settings: Settings;
  #persistence: Persistence;
  #diagnostics: SettingsDiagnostic[];
  #revision = 0;
  #seq = 0;
  #listeners = new Set<(state: SettingsState) => void>();

  constructor(persistence: Persistence = { kind: 'ok' }, diagnostics: SettingsDiagnostic[] = []) {
    this.#settings = defaultSettings();
    this.#persistence = persistence;
    this.#diagnostics = diagnostics;
  }

  state(): SettingsState {
    return {
      settings: clone(this.#settings),
      revision: this.#revision,
      persistedRevision: this.#revision,
      seq: this.#seq,
      persistence: this.#persistence,
      applyStatus: { service: { kind: 'idle' }, autostart: { kind: 'idle' }, vendorLibraries: { kind: 'idle' } },
      diagnostics: clone(this.#diagnostics),
    };
  }

  subscribe(cb: (state: SettingsState) => void): Unsubscribe {
    this.#listeners.add(cb);
    return () => this.#listeners.delete(cb);
  }

  /** Throws a `PatchError` and changes nothing when the patch is invalid. */
  update(patch: SettingsPatch): SettingsState {
    if (!isObject(patch)) return fail('', 'settings.error.notObject');
    const readOnly = READ_ONLY.find((key) => key in patch);
    if (readOnly) fail(readOnly, 'settings.error.readOnlyField');
    checkShape(patch, SHAPE, '');
    const merged = clone(this.#settings) as unknown as Record<string, unknown>;
    merge(merged, patch as Record<string, unknown>);
    checkTypes(merged);
    checkLog(merged.log as Record<string, unknown>);
    checkOverlay(merged.overlay as Record<string, unknown>, merged.log as Record<string, unknown>);
    checkPerformance(merged.performance as Record<string, unknown>);
    checkRules(merged.rules);
    // An unset field is absent, never null (the Rust encoding).
    const advanced = merged.advanced as Record<string, unknown>;
    for (const key of ['section', 'window']) if (advanced[key] === null) delete advanced[key];
    const view = merged.view as Record<string, unknown>;
    if (view.last === null) delete view.last;
    return this.#commit(merged as unknown as Settings);
  }

  /** Drops the override of a built-in rule, like `reset_rule_override`; no entry changes nothing. */
  resetRuleOverride(ruleId: string): SettingsState {
    if (!BUILTIN_RULES.includes(ruleId)) fail(`rules.overrides.${ruleId}`, 'rules.error.unknownRule');
    if (!(ruleId in this.#settings.rules.overrides)) return this.state();
    const next = clone(this.#settings);
    delete next.rules.overrides[ruleId];
    return this.#commit(next);
  }

  /** Fills only what is unset, sets the marker once (`webviewV1`), like `import_webview`. */
  import(legacy: LegacyWebviewState): SettingsState {
    if (this.#settings.migrations.webviewV1) return this.state();
    if (this.#persistence.kind === 'readOnly') throw 'read_only';
    const next = clone(this.#settings);
    if (next.advanced.section === undefined && legacy.section) next.advanced.section = legacy.section;
    if (next.advanced.window === undefined && legacy.window !== undefined && WINDOWS.includes(legacy.window)) {
      next.advanced.window = legacy.window;
    }
    if (next.view.last === undefined && (legacy.view === 'simple' || legacy.view === 'advanced')) next.view.last = legacy.view;
    for (const [section, ids] of Object.entries(legacy.series)) next.advanced.series[section] ??= ids;
    next.migrations.webviewV1 = true;
    return this.#commit(next);
  }

  #commit(next: Settings): SettingsState {
    this.#settings = next;
    this.#revision++;
    this.#seq++;
    const state = this.state();
    this.#listeners.forEach((cb) => cb(state));
    return state;
  }
}
