import { MOCK_SCHEMA } from './backend/mock';
import { defaultSettings } from './backend/mockSettings';
import type { Rule, SettingsState } from './types';
import { i18n } from './i18n/index.svelte';
import {
  LEGACY_SECTION_KEY,
  LEGACY_VIEW_KEY,
  LEGACY_WINDOW_KEY,
  SettingsStore,
  initialView,
  legacySeriesKey,
  migrateLegacyState,
} from './settings.svelte';
import { FakeBackend } from '../test/fake-backend';

let unsubscribe: (() => void) | undefined;

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  unsubscribe?.();
  unsubscribe = undefined;
  vi.restoreAllMocks();
});

const stateWith = (backend: FakeBackend, seq: number): SettingsState => ({ ...backend.settings.state(), seq });

test('stale settings event is ignored', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  backend.emitSettings({ ...stateWith(backend, 5), revision: 5 });
  backend.emitSettings({ ...stateWith(backend, 4), revision: 4 });

  expect(store.state?.seq).toBe(5);
  expect(store.state?.revision).toBe(5);
});

test('subscribes before reading', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  expect(backend.settingsCalls).toEqual(['onSettings', 'getSettings']);
  expect(store.state).not.toBeNull();
});

test('an event delivered before the read reply beats the older reply', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const older = backend.settings.state();
  backend.getSettings = async () => {
    // A change lands while the read is in flight; the reply still carries the state it saw.
    backend.emitSettings({ ...older, seq: older.seq + 1, revision: 7 });
    return older;
  };
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  expect(store.state?.revision).toBe(7);
});

/** A custom rule on the CPU package temperature with fixed thresholds. */
const customRule = (warn: number, crit: number): Rule => ({
  id: 'custom-00000000-0000-4000-8000-000000000001',
  target: { sensor: 'cpu/0/temperature/package' },
  unit: 'celsius',
  condition: 'above',
  warn: { threshold: { fixed: warn }, durationS: 0 },
  crit: { threshold: { fixed: crit }, durationS: 0 },
  hysteresis: { amount: 3, durationS: 10 },
  enabled: true,
  notify: { warn: false, crit: true },
});

test('a rejected rule patch puts the error on the rule field', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  expect(await store.update({ rules: { custom: [customRule(90, 80)] } })).toBe(false);

  expect(store.errors).toEqual({ 'rules.custom.0.crit': 'rules.error.order' });
  expect(store.state?.settings.rules.custom).toEqual([]);

  // Replacing the list with a valid one clears the errors below it.
  expect(await store.update({ rules: { custom: [customRule(80, 90)] } })).toBe(true);
  expect(store.errors).toEqual({});
  expect(store.state?.settings.rules.custom).toHaveLength(1);
});

test('an accepted override patch clears every error of that rule', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  // The critical threshold below the warning one: the error points at `crit`.
  const low = { threshold: { fixed: 80 }, durationS: 10 };
  expect(await store.update({ rules: { overrides: { 'gpu-temp': { crit: low } } } })).toBe(false);
  expect(store.errors).toEqual({ 'rules.overrides.gpu-temp.crit': 'rules.error.order' });

  // Lowering the warning instead makes the whole rule valid, so its error goes too.
  const warn = { threshold: { fixed: 70 }, durationS: 30 };
  expect(await store.update({ rules: { overrides: { 'gpu-temp': { warn } } } })).toBe(true);
  expect(store.errors).toEqual({});
});

test('resetting a rule override drops its entry and reports an unknown rule', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);
  await store.update({ rules: { overrides: { 'gpu-temp': { enabled: false }, 'ram-used': { enabled: false } } } });

  expect(await store.resetRuleOverride('gpu-temp')).toBe(true);
  expect(store.state?.settings.rules.overrides).toEqual({ 'ram-used': { enabled: false } });

  expect(await store.resetRuleOverride('no-such-rule')).toBe(false);
  expect(store.errors).toEqual({ 'rules.overrides.no-such-rule': 'rules.error.unknownRule' });
});

test('failed patch exposes the field error', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  const ok = await store.update({ general: { intervalMs: 700 } });

  expect(ok).toBe(false);
  expect(store.errors).toEqual({ 'general.intervalMs': 'settings.error.range' });
  expect(store.state?.settings.general.intervalMs).toBe(1000);

  // A later valid patch of the same field clears its error.
  expect(await store.update({ general: { intervalMs: 1500 } })).toBe(true);
  expect(store.errors).toEqual({});
  expect(store.state?.settings.general.intervalMs).toBe(1500);
});

test('a successful update applies the returned state', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  expect(await store.update({ view: { last: 'advanced' } })).toBe(true);
  expect(store.state?.settings.view.last).toBe('advanced');
});

test('system language follows the browser', async () => {
  vi.spyOn(navigator, 'languages', 'get').mockReturnValue(['it-IT', 'en']);
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  expect(store.state?.settings.general.language).toBe('system');
  expect(i18n.locale).toBe('it');
});

test('explicit language wins', async () => {
  vi.spyOn(navigator, 'languages', 'get').mockReturnValue(['it-IT']);
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);
  expect(i18n.locale).toBe('it');

  await store.update({ general: { language: 'en' } });
  expect(i18n.locale).toBe('en');
  backend.emitSettings({ ...stateWith(backend, 99), settings: { ...backend.settings.state().settings, general: { ...backend.settings.state().settings.general, language: 'it' } } });
  expect(i18n.locale).toBe('it');
});

function seedLegacyKeys() {
  localStorage.setItem(LEGACY_SECTION_KEY, 'gpu/pci-0000:01:00.0');
  localStorage.setItem(LEGACY_WINDOW_KEY, '1800');
  localStorage.setItem(legacySeriesKey('cpu/0'), '["cpu/0/load/total"]');
  localStorage.setItem(LEGACY_VIEW_KEY, 'advanced');
  localStorage.setItem('unrelated', 'kept');
}

test('legacy keys are imported then removed', async () => {
  seedLegacyKeys();
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  await migrateLegacyState(backend, store, localStorage);

  expect(backend.importCalls).toEqual([
    {
      section: 'gpu/pci-0000:01:00.0',
      window: 1800,
      series: { 'cpu/0': ['cpu/0/load/total'] },
      view: 'advanced',
    },
  ]);
  expect(store.state?.settings.migrations.webviewV1).toBe(true);
  expect(store.state?.settings.view.last).toBe('advanced');
  expect(localStorage.length).toBe(1);
  expect(localStorage.getItem('unrelated')).toBe('kept');
});

test('legacy keys stay when the import fails', async () => {
  seedLegacyKeys();
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.importError = 'persist_failed';
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  await migrateLegacyState(backend, store, localStorage);

  expect(backend.importCalls).toHaveLength(1);
  expect(store.state?.settings.migrations.webviewV1).toBe(false);
  expect(localStorage.length).toBe(5);
});

test('already migrated only removes leftovers', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.importWebviewState({ series: {} });
  backend.importCalls.length = 0;
  seedLegacyKeys();
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  await migrateLegacyState(backend, store, localStorage);

  expect(backend.importCalls).toEqual([]);
  expect(localStorage.length).toBe(1);
  expect(localStorage.getItem('unrelated')).toBe('kept');
});

test('a marker that is only in memory still goes through the import command', async () => {
  seedLegacyKeys();
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);
  // An earlier import could not save: the marker is set but the revision is not on disk.
  const state = store.state!;
  store.state = {
    ...state,
    revision: state.revision + 1,
    persistedRevision: state.revision,
    settings: { ...state.settings, migrations: { ...state.settings.migrations, webviewV1: true } },
  };

  await migrateLegacyState(backend, store, localStorage);

  expect(backend.importCalls).toHaveLength(1);
  expect(localStorage.length).toBe(1);
});

test('a nothing-to-import migration still sets the marker', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  await migrateLegacyState(backend, store, localStorage);

  expect(backend.importCalls).toEqual([{ series: {} }]);
  expect(store.state?.settings.migrations.webviewV1).toBe(true);
});

test('malformed legacy values are dropped from the import but still removed', async () => {
  localStorage.setItem(LEGACY_WINDOW_KEY, 'abc');
  localStorage.setItem(LEGACY_VIEW_KEY, 'weird');
  localStorage.setItem(legacySeriesKey('cpu/0'), '{');
  localStorage.setItem(legacySeriesKey('gpu'), '[1,2]');
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);

  await migrateLegacyState(backend, store, localStorage);

  expect(backend.importCalls).toEqual([{ series: {} }]);
  expect(localStorage.length).toBe(0);
});

test('a storage that throws leaves the migration for the next start', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new SettingsStore();
  unsubscribe = await store.connect(backend);
  const broken = {
    get length(): number {
      throw new Error('denied');
    },
  } as unknown as Storage;

  await expect(migrateLegacyState(backend, store, broken)).resolves.toBeUndefined();
  expect(backend.importCalls).toEqual([]);
});

test('initial view: pending beats default beats last', () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const base = backend.settings.state();
  const withView = (defaultView: 'simple' | 'advanced' | 'last', last?: 'simple' | 'advanced'): SettingsState => ({
    ...base,
    settings: {
      ...base.settings,
      general: { ...base.settings.general, defaultView },
      view: last ? { last } : {},
    },
  });

  expect(initialView(withView('simple', 'advanced'), 'advanced')).toBe('advanced');
  expect(initialView(withView('advanced', 'simple'), 'simple')).toBe('simple');
  expect(initialView(withView('advanced', 'simple'), null)).toBe('advanced');
  expect(initialView(withView('simple', 'advanced'), null)).toBe('simple');
  expect(initialView(withView('last', 'advanced'), null)).toBe('advanced');
  expect(initialView(withView('last'), null)).toBe('simple');
});

test('overlay defaults in the mock match the Rust defaults', () => {
  // A literal copy of the `overlay` section in `defaults_match_the_spec` (crates/oma-core/src/settings/mod.rs).
  expect(defaultSettings().overlay).toEqual({
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
  });
});
