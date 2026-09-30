import type { Backend, Unsubscribe } from './backend/backend';
import { i18n, resolveLocale } from './i18n/index.svelte';
import type { LegacyWebviewState, PatchError, SettingsPatch, SettingsState, ViewKind } from './types';

/** The `localStorage` keys of the web view before the settings file existed (spec M5 §2.4). */
export const LEGACY_SECTION_KEY = 'oma.advanced.section';
export const LEGACY_WINDOW_KEY = 'oma.advanced.window';
export const LEGACY_VIEW_KEY = 'oma.view';
const LEGACY_SERIES_PREFIX = 'oma.advanced.series.';
export const legacySeriesKey = (sectionId: string) => `${LEGACY_SERIES_PREFIX}${sectionId}`;

const isPatchError = (value: unknown): value is PatchError =>
  typeof value === 'object' && value !== null && typeof (value as PatchError).field === 'string' && typeof (value as PatchError).key === 'string';

/** Dotted paths of the fields a patch sets (objects are walked, arrays and scalars are leaves). */
function patchPaths(patch: unknown, parent = ''): string[] {
  if (typeof patch !== 'object' || patch === null || Array.isArray(patch)) return parent ? [parent] : [];
  return Object.entries(patch).flatMap(([key, value]) => patchPaths(value, parent ? `${parent}.${key}` : key));
}

/** The settings as the core holds them, kept current from `oma:settings` events. */
export class SettingsStore {
  /** Null until the first read. Replaced as a whole, never mutated. */
  state = $state.raw<SettingsState | null>(null);
  /** Field path to the i18n key of its last failed patch; a later successful patch of the field clears it. */
  errors = $state<Record<string, string>>({});
  #backend: Backend | null = null;

  /**
   * Subscribes to `oma:settings` before reading, so a change made in between is not lost, and
   * keeps only states newer than the current one (spec M5 §2.3, P5).
   */
  async connect(backend: Backend): Promise<Unsubscribe> {
    this.#backend = backend;
    const off = await backend.onSettings((state) => this.accept(state));
    try {
      this.accept(await backend.getSettings());
    } catch (error) {
      off();
      if (this.#backend === backend) this.#backend = null;
      throw error;
    }
    return () => {
      off();
      if (this.#backend === backend) this.#backend = null;
    };
  }

  /** Takes `next` if its `seq` is greater than the current one; returns whether it did. */
  accept(next: SettingsState): boolean {
    if (this.state !== null && next.seq <= this.state.seq) return false;
    this.state = next;
    const locale = resolveLocale(next.settings.general.language, navigator.languages);
    i18n.locale = locale;
    document.documentElement.lang = locale;
    return true;
  }

  /** Sends a patch. `false` when it was rejected (see `errors`) or could not be sent. */
  async update(patch: SettingsPatch): Promise<boolean> {
    const backend = this.#backend;
    if (backend === null) return false;
    try {
      this.accept(await backend.updateSettings(patch));
    } catch (error) {
      if (isPatchError(error)) {
        this.errors = { ...this.errors, [error.field]: error.key };
      } else {
        console.error('settings update failed', error);
      }
      return false;
    }
    this.#clear(patchPaths(patch));
    return true;
  }

  /** Drops the override of a built-in rule. `false` when it was rejected (see `errors`) or could not be sent. */
  async resetRuleOverride(ruleId: string): Promise<boolean> {
    const backend = this.#backend;
    if (backend === null) return false;
    try {
      this.accept(await backend.resetRuleOverride(ruleId));
    } catch (error) {
      if (isPatchError(error)) {
        this.errors = { ...this.errors, [error.field]: error.key };
      } else {
        console.error('rule reset failed', error);
      }
      return false;
    }
    this.#clear([`rules.overrides.${ruleId}`]);
    return true;
  }

  /** Forgets the errors of these fields and of everything below them. */
  #clear(paths: string[]): void {
    const remaining = Object.entries(this.errors).filter(([field]) => !paths.some((p) => field === p || field.startsWith(`${p}.`)));
    if (remaining.length !== Object.keys(this.errors).length) this.errors = Object.fromEntries(remaining);
  }
}

/** The app-wide instance. */
export const settings = new SettingsStore();

function legacyKeys(storage: Storage): string[] {
  const found: string[] = [];
  for (let i = 0; i < storage.length; i++) {
    const key = storage.key(i);
    if (key === LEGACY_SECTION_KEY || key === LEGACY_WINDOW_KEY || key === LEGACY_VIEW_KEY || key?.startsWith(LEGACY_SERIES_PREFIX)) {
      found.push(key);
    }
  }
  return found;
}

/** What the old keys held; values that are not valid are left out (the core would ignore them too). */
function readLegacy(storage: Storage, keys: string[]): LegacyWebviewState {
  const legacy: LegacyWebviewState = { series: {} };
  for (const key of keys) {
    const value = storage.getItem(key);
    if (value === null) continue;
    if (key === LEGACY_SECTION_KEY) {
      if (value !== '') legacy.section = value;
    } else if (key === LEGACY_WINDOW_KEY) {
      const seconds = Number(value);
      if (value !== '' && Number.isInteger(seconds)) legacy.window = seconds;
    } else if (key === LEGACY_VIEW_KEY) {
      if (value === 'simple' || value === 'advanced') legacy.view = value;
    } else {
      try {
        const ids: unknown = JSON.parse(value);
        if (Array.isArray(ids) && ids.every((id) => typeof id === 'string')) {
          legacy.series[key.slice(LEGACY_SERIES_PREFIX.length)] = ids;
        }
      } catch {
        // Unreadable: nothing to import for this section.
      }
    }
  }
  return legacy;
}

/**
 * Moves the old `localStorage` state into the settings, once (spec M5 §2.4). The keys are deleted
 * only after the core confirms the values and the marker are on disk; any failure leaves them for
 * the next start. Once migrated, leftover keys are just deleted.
 */
export async function migrateLegacyState(backend: Backend, store: SettingsStore, storage: Storage): Promise<void> {
  const state = store.state;
  if (state === null) return;
  try {
    const keys = legacyKeys(storage);
    // A marker that is only in memory (an earlier import could not save) still needs the command.
    const migrated = state.settings.migrations.webviewV1 && state.persistedRevision >= state.revision;
    if (!migrated) {
      store.accept(await backend.importWebviewState(readLegacy(storage, keys)));
    }
    for (const key of keys) storage.removeItem(key);
  } catch (error) {
    console.warn('web view state not migrated; retrying at the next start', error);
  }
}

/**
 * The view to open: one a tray item asked for, else the chosen default, else the view shown when
 * the app was last closed, else Simple.
 */
export function initialView(state: SettingsState, pending: ViewKind | null): ViewKind {
  if (pending !== null) return pending;
  const { defaultView } = state.settings.general;
  if (defaultView !== 'last') return defaultView;
  return state.settings.view.last ?? 'simple';
}
