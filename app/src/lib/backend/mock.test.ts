import { catalogs } from '../i18n/index.svelte';
import {
  MOCK_HISTORY_SECONDS,
  MOCK_SCHEMA,
  SERVICE_MOCK_SCHEMA,
  createMockBackend,
  mockGpuProcesses,
  mockValues,
  parseOverlayFrames,
  parsePawnIoStatus,
  parseServiceState,
  sortGpuProcesses,
} from './mock';
import type { OverlayStatus, Snapshot } from '../types';

const GPU = 'gpu/pci-0000:01:00.0';
const CPU_LOAD = 'cpu/0/load/total';

afterEach(() => {
  history.replaceState(null, '', '/');
});

test('mock values align with the mock schema', () => {
  for (const tick of [0, 1, 50, 1000]) {
    expect(mockValues(tick)).toHaveLength(MOCK_SCHEMA.sensors.length);
  }
});

test('every mock sensor label has a translation', () => {
  for (const sensor of MOCK_SCHEMA.sensors) {
    expect(catalogs.en[`sensor.${sensor.label.key}`], sensor.label.key).toBeDefined();
  }
});

test('mock gpu is a discrete card with an experimental hotspot', () => {
  const gpu = MOCK_SCHEMA.devices.find((d) => d.kind === 'gpu');
  expect(gpu?.properties?.integrated).toBe('false');
  const hotspot = MOCK_SCHEMA.sensors.find((s) => s.id === 'gpu/pci-0000:01:00.0/temperature/hotspot');
  expect(hotspot?.experimental).toBe(true);
});

test('mock backend never starts in safe mode', async () => {
  const backend = createMockBackend();
  expect(await backend.getStartupStatus()).toEqual({ safeMode: false, reason: null, crashModule: null });
  expect((await backend.enableVendorLibraries()).safeMode).toBe(false);
});

test('mock backend emits one snapshot per interval while subscribed', async () => {
  vi.useFakeTimers();
  try {
    const backend = createMockBackend(1000);
    const seqs: number[] = [];
    const off = await backend.onSnapshot((s) => seqs.push(s.seq));
    vi.advanceTimersByTime(3000);
    off();
    vi.advanceTimersByTime(3000);
    expect(seqs).toEqual([1, 2, 3]);
  } finally {
    vi.useRealTimers();
  }
});

test('mock history returns one series per id', async () => {
  const backend = createMockBackend();
  const h = await backend.getHistory([CPU_LOAD, 'unknown'], 10);
  expect(h.timestampsMs).toHaveLength(10);
  expect(h.series[0]).toHaveLength(10);
  expect(h.series[1].every((v) => v === null)).toBe(true);
});

test('mock history honors windows up to one hour', async () => {
  const backend = createMockBackend();
  expect((await backend.getHistory([CPU_LOAD], 1800)).timestampsMs).toHaveLength(1800);
  expect((await backend.getHistory([CPU_LOAD], 3600)).timestampsMs).toHaveLength(MOCK_HISTORY_SECONDS);
  expect((await backend.getHistory([CPU_LOAD], 7200)).timestampsMs).toHaveLength(MOCK_HISTORY_SECONDS);
});

test('mock history decimates to a min/max envelope when maxPoints is given', async () => {
  const backend = createMockBackend();
  const raw = await backend.getHistory([CPU_LOAD], 3600);
  const env = await backend.getHistory([CPU_LOAD], 3600, 900);
  expect(env.timestampsMs).toHaveLength(900);
  expect(env.series[0]).toHaveLength(900);
  // First bucket = raw samples 0..7 (3600 / 450 = 8 per bucket).
  const first = raw.series[0].slice(0, 8) as number[];
  expect(env.series[0][0]).toBe(Math.min(...first));
  expect(env.series[0][1]).toBe(Math.max(...first));
  // Short windows are returned raw even with maxPoints.
  expect((await backend.getHistory([CPU_LOAD], 60, 900)).timestampsMs).toHaveLength(60);
});

test('mock stats accumulate from the emitted ticks and reset per id', async () => {
  vi.useFakeTimers();
  try {
    const backend = createMockBackend(1000);
    expect((await backend.getStats([CPU_LOAD])).stats).toEqual([null]);
    const off = await backend.onSnapshot(() => {});
    vi.advanceTimersByTime(3000);
    const loads = [1, 2, 3].map((t) => mockValues(t)[0] as number);
    const reply = await backend.getStats([CPU_LOAD, 'unknown']);
    expect(reply.revision).toBe(MOCK_SCHEMA.revision);
    expect(reply.stats[0]?.min).toBe(Math.min(...loads));
    expect(reply.stats[0]?.max).toBe(Math.max(...loads));
    expect(reply.stats[0]?.avg).toBeCloseTo((loads[0] + loads[1] + loads[2]) / 3, 10);
    expect(reply.stats[0]?.count).toBe(3);
    expect(reply.stats[1]).toBeNull();

    await backend.resetStats([CPU_LOAD]);
    expect((await backend.getStats([CPU_LOAD])).stats).toEqual([null]);
    vi.advanceTimersByTime(1000);
    expect((await backend.getStats([CPU_LOAD])).stats[0]?.count).toBe(1);
    off();
  } finally {
    vi.useRealTimers();
  }
});

test('mock session starts at the first tick', async () => {
  vi.useFakeTimers();
  try {
    vi.setSystemTime(1_000_000);
    const backend = createMockBackend(500);
    expect(await backend.getSession()).toEqual({ startedAtMs: null, intervalMs: 500 });
    const off = await backend.onSnapshot(() => {});
    vi.advanceTimersByTime(1500);
    expect(await backend.getSession()).toEqual({ startedAtMs: 1_000_500, intervalMs: 500 });
    off();
  } finally {
    vi.useRealTimers();
  }
});

test('mock gpu processes are sorted by load then dedicated memory', async () => {
  const backend = createMockBackend();
  const list = await backend.getGpuProcesses(GPU);
  expect(list).toHaveLength(mockGpuProcesses(0).length);
  const loads = list.map((p) => p.loadPercent ?? -1);
  expect(loads).toEqual([...loads].sort((a, b) => b - a));
  expect(list.at(-1)?.loadPercent).toBeNull();
  expect(list.filter((p) => p.loadPercent === 0).map((p) => p.name)).toEqual(['explorer.exe', 'System']);
  expect(await backend.getGpuProcesses('gpu/unknown')).toEqual([]);
});

test('the URL selects the initial service state, defaulting to connected', () => {
  expect(parseServiceState('')).toBe('connected');
  expect(parseServiceState('?service=antiCheat')).toBe('antiCheat');
  expect(parseServiceState('?service=bogus')).toBe('connected');
});

test('the URL selects the PawnIO status, defaulting to ok', async () => {
  expect(parsePawnIoStatus('')).toBe('ok');
  expect(parsePawnIoStatus('?pawnio=rebootPending')).toBe('rebootPending');
  expect(parsePawnIoStatus('?pawnio=missing&service=connected')).toBe('missing');
  expect(parsePawnIoStatus('?pawnio=bogus')).toBe('ok');

  history.replaceState(null, '', '?service=connected&pawnio=rebootPending');
  const status = await createMockBackend().getServiceStatus();
  expect(status.pawnIo).toBe('rebootPending');
  // PawnIO is only known while connected.
  history.replaceState(null, '', '?service=unreachable&pawnio=rebootPending');
  expect((await createMockBackend().getServiceStatus()).pawnIo).toBeNull();
  history.replaceState(null, '', '');
});

test('the mock serves lhm sensors only when the service is connected', async () => {
  history.replaceState(null, '', '?service=connected');
  const connected = createMockBackend();
  const schema = await connected.getSchema();
  expect(schema).toBe(SERVICE_MOCK_SCHEMA);
  expect(schema.devices.some((d) => d.id === 'motherboard/lhm-mock')).toBe(true);
  expect(schema.sensors.some((s) => s.label.key === 'lhm.raw')).toBe(true);
  expect(schema.sensors.some((s) => s.label.key === 'cpu.temperature.package')).toBe(true);
  expect(await connected.getServiceStatus()).toEqual({
    state: 'connected',
    detail: null,
    pawnIo: 'ok',
    sources: {
      activeModules: ['cpu', 'motherboard', 'memory', 'storage', 'controller', 'psu'],
      requestedDisabledModules: [],
      smartDisabledDrives: [],
      reconfiguration: 'applied',
      drives: [],
    },
  });

  for (const state of ['notInstalled', 'antiCheat', 'starting', 'unreachable', 'incompatible']) {
    history.replaceState(null, '', `?service=${state}`);
    const backend = createMockBackend();
    const otherSchema = await backend.getSchema();
    expect(otherSchema).toBe(MOCK_SCHEMA);
    expect(otherSchema.devices.some((d) => d.id === 'motherboard/lhm-mock')).toBe(false);
    expect(await backend.getServiceStatus()).toEqual({ state, detail: null, pawnIo: null, sources: null });
  }
});

test('setAntiCheat and startService change the status and notify listeners', async () => {
  const backend = createMockBackend();
  const seen: string[] = [];
  await backend.onServiceStatus((s) => seen.push(s.state));

  const afterAntiCheat = await backend.setAntiCheat(true);
  expect(afterAntiCheat).toEqual({ state: 'antiCheat', detail: null, pawnIo: null, sources: null });

  const afterLeaving = await backend.setAntiCheat(false);
  expect(afterLeaving.state).toBe('unreachable');

  const afterStart = await backend.startService();
  expect(afterStart.state).toBe('connected');
  expect(afterStart.pawnIo).toBe('ok');
  expect(afterStart.sources?.reconfiguration).toBe('applied');
  expect(seen).toEqual(['antiCheat', 'unreachable', 'connected']);
});

test('gpu process lists are capped at 20 rows', () => {
  const many = Array.from({ length: 30 }, (_, i) => ({
    pid: i,
    name: `p${i}.exe`,
    loadPercent: i,
    engine: '3D',
    dedicatedBytes: 0,
    sharedBytes: 0,
  }));
  const sorted = sortGpuProcesses(many);
  expect(sorted).toHaveLength(20);
  expect(sorted[0].pid).toBe(29);
});

test('mock backend rejects an off-step interval', async () => {
  const backend = createMockBackend();
  await expect(backend.updateSettings({ general: { intervalMs: 700 } })).rejects.toEqual({
    field: 'general.intervalMs',
    key: 'settings.error.range',
  });
  expect((await backend.getSettings()).settings.general.intervalMs).toBe(1000);
});

test('mock backend keeps update checks off by default and accepts a patch', async () => {
  const backend = createMockBackend();
  expect((await backend.getSettings()).settings.updates).toEqual({ checkAutomatically: false });
  const state = await backend.updateSettings({ updates: { checkAutomatically: true } });
  expect(state.settings.updates.checkAutomatically).toBe(true);
  await expect(backend.updateSettings({ updates: { checkAutomatically: 'yes' } } as never)).rejects.toEqual({
    field: 'updates.checkAutomatically',
    key: 'settings.error.type',
  });
});

test('mock backend rejects an unknown field and null on a plain field', async () => {
  const backend = createMockBackend();
  await expect(backend.updateSettings({ general: { nope: 1 } } as never)).rejects.toEqual({
    field: 'general.nope',
    key: 'settings.error.unknownField',
  });
  await expect(backend.updateSettings({ general: { language: null } } as never)).rejects.toEqual({
    field: 'general.language',
    key: 'settings.error.null',
  });
  await expect(backend.updateSettings({ general: { language: 'fr' } } as never)).rejects.toEqual({
    field: 'general.language',
    key: 'settings.error.type',
  });
  await expect(backend.updateSettings({ migrations: {} } as never)).rejects.toEqual({
    field: 'migrations',
    key: 'settings.error.readOnlyField',
  });
});

test('mock backend applies a valid log patch and stores hotkeys canonical', async () => {
  const backend = createMockBackend();
  const state = await backend.updateSettings({
    log: { everyTicks: 30, maxFileMb: 512, sensors: ['a', 'b'], hotkeyPause: 'shift + ctrl + p' },
  });
  expect(state.settings.log).toEqual({
    folder: null,
    sensors: ['a', 'b'],
    everyTicks: 30,
    maxFileMb: 512,
    hotkeyToggle: 'Ctrl+Alt+Shift+R',
    hotkeyPause: 'Ctrl+Shift+P',
  });
  const back = await backend.updateSettings({ log: { sensors: null } });
  expect(back.settings.log.sensors).toBeNull();
});

test('mock backend rejects log values the way the core does', async () => {
  const backend = createMockBackend();
  const cases: [Parameters<typeof backend.updateSettings>[0], string, string][] = [
    [{ log: { everyTicks: 7 } } as never, 'log.everyTicks', 'settings.error.range'],
    [{ log: { maxFileMb: 9 } }, 'log.maxFileMb', 'settings.error.range'],
    [{ log: { folder: 'logs' } }, 'log.folder', 'settings.error.folder'],
    [{ log: { hotkeyToggle: 'Ctrl+R' } }, 'log.hotkeyToggle', 'settings.error.hotkey'],
    [{ log: { hotkeyPause: 'alt+shift+ctrl+r' } }, 'log.hotkeyPause', 'settings.error.hotkeyDuplicate'],
    [{ log: { sensors: ['a', 'a'] } }, 'log.sensors', 'settings.error.sensors'],
    [{ log: { sensors: [''] } }, 'log.sensors', 'settings.error.sensors'],
  ];
  for (const [patch, field, key] of cases) {
    await expect(backend.updateSettings(patch)).rejects.toEqual({ field, key });
  }
  expect((await backend.getSettings()).settings.log.everyTicks).toBe(1);
});

const RULE_ID = 'custom-00000000-0000-4000-8000-000000000001';
const customRule = (warn: number, crit: number, durationS = 0) => ({
  id: RULE_ID,
  target: { sensor: 'cpu/0/temperature/package' },
  unit: 'celsius' as const,
  condition: 'above' as const,
  warn: { threshold: { fixed: warn }, durationS },
  crit: { threshold: { fixed: crit }, durationS: 0 },
  hysteresis: { amount: 3, durationS: 10 },
  enabled: true,
  notify: { warn: false, crit: true },
});

test('mock rule overrides replace level objects whole', async () => {
  const backend = createMockBackend();
  await backend.updateSettings({
    rules: {
      overrides: {
        'gpu-temp': { enabled: false, warn: { threshold: { property: 'tjMaxC', offset: -10, fallback: 80 }, durationS: 30 } },
      },
    },
  });
  const state = await backend.updateSettings({ rules: { overrides: { 'gpu-temp': { warn: { threshold: { fixed: 85 }, durationS: 20 } } } } });
  expect(state.settings.rules.overrides['gpu-temp']).toEqual({ enabled: false, warn: { threshold: { fixed: 85 }, durationS: 20 } });
  expect(state.diagnostics).toEqual([]);
});

test('mock backend rejects misordered thresholds, long durations and unknown rules', async () => {
  const backend = createMockBackend();
  await expect(backend.updateSettings({ rules: { custom: [customRule(90, 80)] } })).rejects.toEqual({
    field: 'rules.custom.0.crit',
    key: 'rules.error.order',
  });
  await expect(backend.updateSettings({ rules: { custom: [customRule(80, 90, 601)] } })).rejects.toEqual({
    field: 'rules.custom.0.warn.durationS',
    key: 'rules.error.duration',
  });
  await expect(
    backend.updateSettings({ rules: { overrides: { 'gpu-temp': { hysteresis: { amount: 1, durationS: 601 } } } } }),
  ).rejects.toEqual({ field: 'rules.overrides.gpu-temp.hysteresis.durationS', key: 'rules.error.duration' });
  await expect(backend.updateSettings({ rules: { overrides: { nope: { enabled: false } } } })).rejects.toEqual({
    field: 'rules.overrides.nope',
    key: 'rules.error.unknownRule',
  });
  expect((await backend.getSettings()).settings.rules).toEqual({ overrides: {}, custom: [] });
});

test('mock resetRuleOverride removes the entry and rejects an unknown rule', async () => {
  const backend = createMockBackend();
  await backend.updateSettings({ rules: { overrides: { 'gpu-temp': { enabled: false } } } });
  expect((await backend.resetRuleOverride('gpu-temp')).settings.rules.overrides).toEqual({});
  // Nothing to remove: not an error.
  await expect(backend.resetRuleOverride('gpu-temp')).resolves.toBeDefined();
  await expect(backend.resetRuleOverride(RULE_ID)).rejects.toEqual({
    field: `rules.overrides.${RULE_ID}`,
    key: 'rules.error.unknownRule',
  });
});

test('mock settings keep revisions and a growing seq, and events follow updates', async () => {
  const backend = createMockBackend();
  const seen: number[] = [];
  await backend.onSettings((state) => seen.push(state.seq));
  const before = await backend.getSettings();

  const after = await backend.updateSettings({ general: { intervalMs: 2000 }, advanced: { series: { cpu: ['a'] } } });

  expect(after.settings.general.intervalMs).toBe(2000);
  expect(after.settings.advanced.series).toEqual({ cpu: ['a'] });
  expect(after.revision).toBe(before.revision + 1);
  expect(after.seq).toBeGreaterThan(before.seq);
  expect(seen).toEqual([after.seq]);
  // Unset stays absent, like the Rust encoding.
  expect('section' in after.settings.advanced).toBe(false);
  expect(after.settings.tray.iconSensor).toBeNull();
});

test('?settings= simulates the persistence states', async () => {
  const kinds: Record<string, string> = { recovered: 'recovered', readOnly: 'readOnly', error: 'error' };
  for (const [param, kind] of Object.entries(kinds)) {
    history.replaceState(null, '', `/?settings=${param}`);
    const state = await createMockBackend().getSettings();
    expect(state.persistence.kind).toBe(kind);
  }
  history.replaceState(null, '', '/');
  expect((await createMockBackend().getSettings()).persistence).toEqual({ kind: 'ok' });
});

test('mock navigation and autostart answers', async () => {
  const backend = createMockBackend();
  expect(await backend.takePendingView()).toBeNull();
  await expect(backend.onNavigate(() => {})).resolves.toBeTypeOf('function');
  expect(await backend.refreshAutostart()).toEqual({ configured: false, effective: 'notConfigured', error: null });
  await backend.updateSettings({ tray: { autostart: true } });
  expect(await backend.refreshAutostart()).toEqual({ configured: true, effective: 'enabled', error: null });
});

test('mock autostart state from the URL', async () => {
  history.replaceState(null, '', '/?autostart=disabledByWindows');
  const backend = createMockBackend();
  await backend.updateSettings({ tray: { autostart: true } });
  expect((await backend.refreshAutostart()).effective).toBe('disabledByWindows');
  history.replaceState(null, '', '/?autostart=unknown');
  expect((await createMockBackend().refreshAutostart()).effective).toBe('unknown');
});

test('mock anti-cheat mode is kept in the settings, like the shell does', async () => {
  const backend = createMockBackend();
  await backend.setAntiCheat(true);
  expect((await backend.getSettings()).settings.sources.antiCheat).toBe(true);
  await backend.setAntiCheat(false);
  expect((await backend.getSettings()).settings.sources.antiCheat).toBe(false);
});

test('mock app info and known paths', async () => {
  const backend = createMockBackend();
  const info = await backend.getAppInfo();
  expect(info.protocolVersion).toBe(2);
  expect(info.serviceVersion).not.toBeNull();
  await expect(backend.openKnownPath('logsFolder')).resolves.toBeUndefined();
});

test('mock webview import fills only unset fields and sets the marker once', async () => {
  const backend = createMockBackend();
  await backend.updateSettings({ advanced: { window: 60 } });
  const state = await backend.importWebviewState({ section: 'gpu/x', window: 3600, series: { 'gpu/x': ['a'] }, view: 'advanced' });
  expect(state.settings.advanced.window).toBe(60);
  expect(state.settings.advanced.section).toBe('gpu/x');
  expect(state.settings.view.last).toBe('advanced');
  expect(state.settings.migrations.webviewV1).toBe(true);
  const again = await backend.importWebviewState({ section: 'other', series: {} });
  expect(again.settings.advanced.section).toBe('gpu/x');
  expect(again.seq).toBe(state.seq);
});

describe('mock log recorder', () => {
  afterEach(() => vi.useRealTimers());

  test('mock_recorder_counts_and_stops', async () => {
    vi.useFakeTimers();
    const backend = createMockBackend();
    const seen: number[] = [];
    const off = await backend.onLogStatus((s) => seen.push(s.revision));
    expect((await backend.getLogStatus()).state).toBe('idle');

    const started = await backend.logStart();
    expect(started.state).toBe('recording');
    expect(started.session).toBeGreaterThan(0);
    await vi.advanceTimersByTimeAsync(3000);
    const counting = await backend.getLogStatus();
    expect(counting.recordedMs).toBeGreaterThanOrEqual(3000);
    expect(counting.rows).toBeGreaterThan(0);
    expect(counting.revision).toBeGreaterThan(started.revision);

    const paused = await backend.logPause();
    expect(paused.state).toBe('paused');
    await vi.advanceTimersByTimeAsync(3000);
    expect((await backend.getLogStatus()).rows).toBe(paused.rows);
    expect((await backend.logResume()).state).toBe('recording');

    // A fake threshold opens a new part.
    await vi.advanceTimersByTimeAsync(400_000);
    expect((await backend.getLogStatus()).part).toBeGreaterThan(1);

    const stopped = await backend.logStop();
    expect(stopped.state).toBe('idle');
    const before = seen.length;
    await vi.advanceTimersByTimeAsync(5000);
    expect(seen.length).toBe(before);
    expect(seen).toEqual([...seen].sort((a, b) => a - b));
    off();
  });

  test('the ?log knob sets the initial state', async () => {
    for (const state of ['recording', 'paused', 'error'] as const) {
      history.replaceState(null, '', `/?log=${state}`);
      expect((await createMockBackend().getLogStatus()).state).toBe(state);
    }
    expect((await createMockBackend().getLogStatus()).error).not.toBeNull();
  });
});

test('the mock with the service exposes a standby disk whose temperature is suspended', async () => {
  history.replaceState(null, '', '?service=connected');
  const backend = createMockBackend();
  const states = await backend.getDiskStates();
  expect(states).toEqual([{ deviceId: 'storage/device-mock-hdd', power: 'standby' }]);
  states.pop();
  expect(await backend.getDiskStates()).toHaveLength(1); // A copy, not the shared array.
  const schema = await backend.getSchema();
  const id = 'storage/device-mock-hdd/temperature/drive';
  const index = schema.sensors.findIndex((s) => s.id === id);
  expect(index).toBeGreaterThanOrEqual(0);
  const history_ = await backend.getHistory([id], 5);
  expect(history_.series[0].every((v) => typeof v === 'number')).toBe(true);
  const seen: Snapshot[] = [];
  const off = await backend.onSnapshot((s) => seen.push(s));
  await vi.waitFor(() => expect(seen.length).toBeGreaterThan(0), { timeout: 3000 });
  off();
  expect(seen[0].quality).toHaveLength(schema.sensors.length);
  expect(seen[0].quality?.[index]).toBe(2);
  expect(seen[0].quality?.filter((q) => q !== 0)).toHaveLength(1);
});

test('the mock without the service has no disk states and no quality', async () => {
  for (const state of ['unreachable', 'notInstalled', 'antiCheat']) {
    history.replaceState(null, '', `?service=${state}`);
    const backend = createMockBackend();
    expect(await backend.getDiskStates()).toEqual([]);
    const seen: Snapshot[] = [];
    const off = await backend.onSnapshot((s) => seen.push(s));
    await vi.waitFor(() => expect(seen.length).toBeGreaterThan(0), { timeout: 3000 });
    off();
    expect(seen[0].quality).toBeUndefined();
  }
});

test('mock overlay follows the settings and offers a retry', async () => {
  const backend = createMockBackend();
  const seen: OverlayStatus[] = [];
  const off = await backend.onOverlayStatus((s) => seen.push(s));
  const off0 = await backend.getOverlayStatus();
  expect(off0).toMatchObject({ enabled: false, process: 'off', frames: 'off', target: null });
  expect(off0!.profiles.filter((p) => p.builtin).map((p) => p.name)).toEqual(
    ['builtin-minimal-fps', 'builtin-gaming', 'builtin-full', 'builtin-bar'].map((id) => `overlay.template.${id}`),
  );
  await backend.updateSettings({ overlay: { enabled: true } });
  const on = seen.at(-1)!;
  expect(on).toMatchObject({ enabled: true, process: 'running', frames: 'running', activeProfile: 'builtin-gaming' });
  expect(on.target?.name).toMatch(/\.exe$/);
  await backend.updateSettings({ overlay: { gameProfiles: { [on.target!.name]: 'builtin-bar' } } });
  expect(seen.at(-1)!.activeProfile).toBe('builtin-bar');
  await backend.overlayRetry();
  expect(seen.at(-1)!.frames).toBe('running');
  await backend.setOverlayHidden(true);
  expect(seen.at(-1)!.hiddenByUser).toBe(true);
  await backend.setOverlayHidden(false);
  expect(seen.at(-1)!.hiddenByUser).toBe(false);
  off();
});

test('?overlay= simulates a frame engine state', () => {
  expect(parseOverlayFrames('?overlay=denied')).toBe('denied');
  expect(parseOverlayFrames('?overlay=bogus')).toBe('running');
  expect(parseOverlayFrames('')).toBe('running');
});

describe('mock overlay editor', () => {
  test('profiles round-trip and the catalog follows', async () => {
    const backend = createMockBackend();
    const builtin = await backend.overlayLoadProfile('builtin-gaming');
    expect(builtin.builtin).toBe(true);
    await expect(backend.overlaySaveProfile('builtin-gaming', builtin.json)).rejects.toMatchObject({ key: 'editor.error.readOnly' });
    const id = await backend.overlayDuplicateProfile('builtin-gaming');
    const copy = await backend.overlayLoadProfile(id);
    expect(copy.builtin).toBe(false);
    expect(JSON.parse(copy.json).blocks.length).toBeGreaterThan(0);
    expect((await backend.getOverlayStatus())!.profiles.map((p) => p.id)).toContain(id);
    await backend.overlayDeleteProfile(id);
    await expect(backend.overlayLoadProfile(id)).rejects.toMatchObject({ key: 'editor.error.notFound' });
  });
});
