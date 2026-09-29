import type {
  LegacyWebviewState,
  PatchError,
  Persistence,
  Settings,
  SettingsPatch,
  SettingsState,
} from '../types';
import type { Unsubscribe } from './backend';

/** Accepted values, the same sets as `oma_core::settings`. */
const INTERVALS = [500, 1000, 1500, 2000, 2500, 3000, 3500, 4000, 4500, 5000];
const WINDOWS = [60, 300, 1800, 3600];
const FPS = [15, 30, 60];

const ENUMS: Record<string, readonly string[]> = {
  'general.language': ['system', 'en', 'it'],
  'general.temperatureUnit': ['c', 'f'],
  'general.throughputUnit': ['bits', 'bytes'],
  'general.defaultView': ['simple', 'advanced', 'last'],
  'view.last': ['simple', 'advanced'],
};
const NUMBERS: Record<string, readonly number[]> = {
  'general.intervalMs': INTERVALS,
  'general.chartFps': FPS,
  'advanced.window': WINDOWS,
};

type Node = 'leaf' | 'nullable' | 'free' | { [key: string]: Node };

/** Patchable shape, as in `patch.rs`. */
const SHAPE: { [key: string]: Node } = {
  general: { language: 'leaf', temperatureUnit: 'leaf', throughputUnit: 'leaf', intervalMs: 'leaf', chartFps: 'leaf', defaultView: 'leaf' },
  tray: { closeToTray: 'leaf', autostart: 'leaf', iconSensor: 'nullable' },
  sources: {
    vendorLibraries: { nvml: 'leaf', nvapi: 'leaf', adl: 'leaf', igcl: 'leaf' },
    antiCheat: 'leaf',
    serviceModules: { cpu: 'leaf', motherboard: 'leaf', memory: 'leaf', storage: 'leaf', controller: 'leaf', psu: 'leaf' },
    smartDisabledDrives: 'leaf',
  },
  advanced: { section: 'nullable', window: 'nullable', series: 'free' },
  view: { last: 'nullable' },
};
const READ_ONLY = ['version', 'migrations', 'rules', 'log'];

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
  if (typeof sources.antiCheat !== 'boolean') fail('sources.antiCheat', 'settings.error.type');
  if (advanced.section != null && typeof advanced.section !== 'string') fail('advanced.section', 'settings.error.type');
  const series = advanced.series;
  if (!isObject(series) || !Object.values(series).every((ids) => Array.isArray(ids) && ids.every((id) => typeof id === 'string'))) {
    fail('advanced.series', 'settings.error.type');
  }
}

/** Objects merge recursively, everything else replaces. */
function merge(base: Record<string, unknown>, patch: Record<string, unknown>): void {
  for (const [key, value] of Object.entries(patch)) {
    if (value === undefined) continue;
    const existing = base[key];
    if (isObject(existing) && isObject(value)) merge(existing, value);
    else base[key] = clone(value);
  }
}

/** Settings of a fresh install, encoded like `oma_core::settings::encode`. */
export function defaultSettings(): Settings {
  return {
    version: 1,
    general: { language: 'system', temperatureUnit: 'c', throughputUnit: 'bits', intervalMs: 1000, chartFps: 60, defaultView: 'last' },
    tray: { closeToTray: true, autostart: false, iconSensor: null },
    sources: {
      vendorLibraries: { nvml: true, nvapi: true, adl: true, igcl: true },
      antiCheat: false,
      serviceModules: { cpu: true, motherboard: true, memory: true, storage: true, controller: true, psu: true },
      smartDisabledDrives: [],
    },
    advanced: { series: {} },
    view: {},
    rules: { overrides: {}, custom: [] },
    log: {},
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
  #revision = 0;
  #seq = 0;
  #listeners = new Set<(state: SettingsState) => void>();

  constructor(persistence: Persistence = { kind: 'ok' }) {
    this.#settings = defaultSettings();
    this.#persistence = persistence;
  }

  state(): SettingsState {
    return {
      settings: clone(this.#settings),
      revision: this.#revision,
      persistedRevision: this.#revision,
      seq: this.#seq,
      persistence: this.#persistence,
      applyStatus: { service: { kind: 'idle' }, autostart: { kind: 'idle' }, vendorLibraries: { kind: 'idle' } },
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
    // An unset field is absent, never null (the Rust encoding).
    const advanced = merged.advanced as Record<string, unknown>;
    for (const key of ['section', 'window']) if (advanced[key] === null) delete advanced[key];
    const view = merged.view as Record<string, unknown>;
    if (view.last === null) delete view.last;
    return this.#commit(merged as unknown as Settings);
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
