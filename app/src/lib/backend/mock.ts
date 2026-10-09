import type {
  Alert,
  AutostartEffective,
  GpuProcess,
  HealthClock,
  HealthReport,
  HistoryWindow,
  Label,
  LogState,
  LogStatus,
  OverlayFramesState,
  OverlayStatus,
  UpdateStatus,
  Rule,
  RuleStatus,
  Schema,
  Sensor,
  SensorKind,
  PawnIoStatus,
  ServiceSources,
  ServiceState,
  ServiceStatus,
  Snapshot,
  DiskStateEntry,
  Source,
  StartupStatus,
  Unit,
} from '../types';
import type { Backend } from './backend';
import defaultRulesFixture from '../../test/fixtures/default-rules.json';
import { decimateWindow } from './decimate';
import { MockSettings, parsePersistence } from './mockSettings';
import { StatsAccumulator } from './mockStats';
import { MOCK_BENCHMARKS, mockEditorData, mockProfileStore } from './mockEditor';
import { MOCK_VOLUMES, mockBench, mockBoardTable, mockPerformance, parseBenchScenario, parseDiskScenario, parsePerfScenario } from './mockPerformance';

const THREADS = 8;
const GIB = 1024 ** 3;
const MIB = 1024 ** 2;
const GPU = 'gpu/pci-0000:01:00.0';
const SERVICE_DEVICE = 'motherboard/lhm-mock';

const sensor = (id: string, deviceId: string, kind: SensorKind, unit: Unit, label: Label, source: Source = 'mock'): Sensor => ({
  id,
  deviceId,
  kind,
  unit,
  label,
  source,
  category: kind,
});

/** Same ids and label keys the Rust providers produce (crates/oma-win). */
export const MOCK_SCHEMA: Schema = {
  revision: 1,
  devices: [
    { id: 'cpu/0', kind: 'cpu', name: 'Mock Ryzen 7 7800X3D' },
    {
      id: GPU,
      kind: 'gpu',
      name: 'Mock GeForce RTX 4080',
      vendor: 'NVIDIA',
      properties: { pciAddress: '0000:01:00.0', integrated: 'false' },
    },
    { id: 'memory/0', kind: 'memory', name: 'RAM' },
    { id: 'storage/device-mock-ssd', kind: 'storage', name: 'Disk 0 (C:)', properties: { smartSelectable: 'true' } },
    { id: 'network/mock-eth', kind: 'network', name: 'Ethernet' },
  ],
  sensors: [
    sensor('cpu/0/load/total', 'cpu/0', 'load', 'percent', { key: 'cpu.load.total' }),
    ...Array.from({ length: THREADS }, (_, i) =>
      sensor(`cpu/0/load/thread-0-${i}`, 'cpu/0', 'load', 'percent', { key: 'cpu.load.thread', arg: String(i) }),
    ),
    sensor('cpu/0/clock/effective', 'cpu/0', 'clock', 'megahertz', { key: 'cpu.clock.effective' }),
    sensor(`${GPU}/load/core`, GPU, 'load', 'percent', { key: 'gpu.load.core' }),
    sensor(`${GPU}/data/memory-dedicated-used`, GPU, 'data', 'bytes', { key: 'gpu.memory.dedicatedUsed' }),
    sensor(`${GPU}/data/memory-dedicated-total`, GPU, 'data', 'bytes', { key: 'gpu.memory.dedicatedTotal' }),
    sensor(`${GPU}/temperature/core`, GPU, 'temperature', 'celsius', { key: 'gpu.temperature.core' }),
    { ...sensor(`${GPU}/temperature/hotspot`, GPU, 'temperature', 'celsius', { key: 'gpu.temperature.hotspot' }), experimental: true },
    sensor(`${GPU}/clock/core`, GPU, 'clock', 'megahertz', { key: 'gpu.clock.core' }),
    sensor(`${GPU}/power/board`, GPU, 'power', 'watt', { key: 'gpu.power.board' }),
    sensor('memory/0/load/used', 'memory/0', 'load', 'percent', { key: 'memory.load' }),
    sensor('memory/0/data/used', 'memory/0', 'data', 'bytes', { key: 'memory.used' }),
    sensor('memory/0/data/total', 'memory/0', 'data', 'bytes', { key: 'memory.total' }),
    sensor('storage/device-mock-ssd/throughput/read', 'storage/device-mock-ssd', 'throughput', 'bytes_per_second', { key: 'storage.read' }),
    sensor('storage/device-mock-ssd/throughput/write', 'storage/device-mock-ssd', 'throughput', 'bytes_per_second', { key: 'storage.write' }),
    sensor('storage/device-mock-ssd/load/active', 'storage/device-mock-ssd', 'load', 'percent', { key: 'storage.active' }),
    sensor('storage/device-mock-ssd/percent/volume-mock-guid', 'storage/device-mock-ssd', 'percent', 'percent', { key: 'storage.volumeUsed', arg: 'C:' }),
    sensor('storage/device-mock-ssd/data/volume-mock-guid-free', 'storage/device-mock-ssd', 'data', 'bytes', { key: 'storage.volumeFree', arg: 'C:' }),
    sensor('network/mock-eth/throughput/down', 'network/mock-eth', 'throughput', 'bytes_per_second', { key: 'network.down' }),
    sensor('network/mock-eth/throughput/up', 'network/mock-eth', 'throughput', 'bytes_per_second', { key: 'network.up' }),
    sensor('network/mock-eth/throughput/link-speed', 'network/mock-eth', 'throughput', 'bits_per_second', { key: 'network.linkSpeed' }),
  ],
};

/** A hard disk in standby: it keeps its last temperature (quality 2) and its history. */
const MOCK_HDD = 'storage/device-mock-hdd';
const MOCK_HDD_SENSOR = `${MOCK_HDD}/temperature/drive`;

/**
 * Sensors added only while the sensor service is connected (spec §6): CPU temperature and
 * power straight from the driver, plus a motherboard device with LHM's raw fan and voltage
 * readings (label key `lhm.raw`, the driver's own text as `arg`).
 */
const SERVICE_SENSORS: Sensor[] = [
  sensor('cpu/0/temperature/package', 'cpu/0', 'temperature', 'celsius', { key: 'cpu.temperature.package' }, 'lhm'),
  sensor('cpu/0/power/package', 'cpu/0', 'power', 'watt', { key: 'cpu.power.package' }, 'lhm'),
  sensor(`${SERVICE_DEVICE}/fan/fan-1`, SERVICE_DEVICE, 'fan', 'rpm', { key: 'lhm.raw', arg: 'Fan #1' }, 'lhm'),
  sensor(`${SERVICE_DEVICE}/fan/fan-2`, SERVICE_DEVICE, 'fan', 'rpm', { key: 'lhm.raw', arg: 'Fan #2' }, 'lhm'),
  sensor(`${SERVICE_DEVICE}/voltage/vin3`, SERVICE_DEVICE, 'voltage', 'volt', { key: 'lhm.raw', arg: 'VIN3' }, 'lhm'),
  // Last: its value is the disk's last reading while the disk sleeps (`MOCK_HDD_SENSOR`).
  sensor(MOCK_HDD_SENSOR, MOCK_HDD, 'temperature', 'celsius', { key: 'storage.temperature' }, 'lhm'),
];

/** `MOCK_SCHEMA` plus the sensor-service devices and sensors above. */
export const SERVICE_MOCK_SCHEMA: Schema = {
  revision: MOCK_SCHEMA.revision,
  devices: [
    ...MOCK_SCHEMA.devices,
    { id: SERVICE_DEVICE, kind: 'motherboard', name: 'Mock Motherboard' },
    { id: MOCK_HDD, kind: 'storage', name: 'Disk 2 (HDD)', properties: { smartSelectable: 'true' } },
    // A USB disk whose bridge hides the serial: its SMART cannot be switched off on its own.
    { id: 'storage/device-mock-usb', kind: 'storage', name: 'Disk 1 (USB)', properties: { smartSelectable: 'false' } },
  ],
  sensors: [...MOCK_SCHEMA.sensors, ...SERVICE_SENSORS],
};

/** Deterministic plausible values for `SERVICE_SENSORS`, same tick as `mockValues`. */
function serviceMockValues(t: number): number[] {
  const wave = (period: number, phase = 0) => (Math.sin((t + phase) / period) + 1) / 2;
  return [40 + 20 * wave(13), 30 + 40 * wave(9, 1), 800 + 200 * wave(15), 750 + 150 * wave(17, 2), 12 + 0.2 * wave(21), 36 + 2 * wave(25)];
}

/** Deterministic plausible values for tick `t`, in MOCK_SCHEMA sensor order. */
export function mockValues(t: number): (number | null)[] {
  const wave = (period: number, phase = 0) => (Math.sin((t + phase) / period) + 1) / 2;
  const total = 20 + 50 * wave(9);
  const threads = Array.from({ length: THREADS }, (_, i) => Math.min(100, total * (0.6 + 0.1 * i)));
  const memTotal = 32 * GIB;
  const memUsed = memTotal * (0.5 + 0.1 * wave(30));
  const gpuLoad = 10 + 80 * wave(11, 3);
  return [
    total,
    ...threads,
    4200 + 400 * wave(7),
    gpuLoad,
    (2 + 6 * wave(40)) * GIB,
    16 * GIB,
    40 + gpuLoad * 0.3,
    52 + gpuLoad * 0.35,
    1500 + 12 * gpuLoad,
    25 + 2.9 * gpuLoad,
    (memUsed / memTotal) * 100,
    memUsed,
    memTotal,
    120e6 * wave(5),
    30e6 * wave(6, 2),
    60 * wave(5),
    65,
    700 * GIB,
    6e6 * wave(4),
    4e5 * wave(4, 1),
    1e9,
  ];
}

/** The mock never starts in GPU safe mode. */
export const MOCK_STARTUP: StartupStatus = { safeMode: false, reason: null, crashModule: null };

/** The mock's update check never ran. */
const MOCK_UPDATE_STATUS: UpdateStatus = { state: 'idle', current: '0.1.0', latest: null, checkedAtMs: null, error: null };

/** Same cap as the core (`MAX_HISTORY_SECONDS`): one hour at 1 s. */
export const MOCK_HISTORY_SECONDS = 3600;

/** Plausible processes on the mock GPU at tick `t`, unsorted. */
export function mockGpuProcesses(t: number): GpuProcess[] {
  const wave = (period: number, phase = 0) => (Math.sin((t + phase) / period) + 1) / 2;
  const game = 20 + 70 * wave(11, 3);
  const encoder = 5 + 10 * wave(7);
  return [
    { pid: 4, name: 'System', loadPercent: 0, engine: null, dedicatedBytes: 0, sharedBytes: 2 * MIB },
    { pid: 1188, name: 'dwm.exe', loadPercent: 1 + 3 * wave(5), engine: '3D', dedicatedBytes: 310 * MIB, sharedBytes: 48 * MIB },
    { pid: 6020, name: 'explorer.exe', loadPercent: 0, engine: null, dedicatedBytes: 42 * MIB, sharedBytes: 12 * MIB },
    { pid: 9412, name: 'game.exe', loadPercent: game, engine: '3D', dedicatedBytes: 5.5 * GIB, sharedBytes: 180 * MIB },
    { pid: 10764, name: 'obs64.exe', loadPercent: encoder, engine: 'VideoEncode', dedicatedBytes: 620 * MIB, sharedBytes: 64 * MIB },
    { pid: 12880, name: 'msedgewebview2.exe', loadPercent: null, engine: null, dedicatedBytes: null, sharedBytes: null },
  ];
}

const desc = (a: number | null, b: number | null) => (b ?? -1) - (a ?? -1);

/** Core order: busiest first, then most dedicated memory; at most 20 rows. */
export function sortGpuProcesses(list: GpuProcess[]): GpuProcess[] {
  return [...list]
    .sort((a, b) => desc(a.loadPercent, b.loadPercent) || desc(a.dedicatedBytes, b.dedicatedBytes))
    .slice(0, 20);
}

const MOCK_IDS = MOCK_SCHEMA.sensors.map((s) => s.id);
const SERVICE_IDS = SERVICE_MOCK_SCHEMA.sensors.map((s) => s.id);

const VALID_SERVICE_STATES: ServiceState[] = ['notInstalled', 'antiCheat', 'starting', 'connected', 'unreachable', 'incompatible'];

/** Initial service state from `?service=<state>` in the URL; defaults to `connected`. */
export function parseServiceState(search: string): ServiceState {
  const raw = new URLSearchParams(search).get('service');
  return (VALID_SERVICE_STATES as string[]).includes(raw ?? '') ? (raw as ServiceState) : 'connected';
}

const VALID_PAWN_IO: PawnIoStatus[] = ['ok', 'missing', 'unavailable', 'unknown', 'rebootPending'];

/** PawnIO status from `?pawnio=<status>` in the URL; defaults to `ok`. Only shown while connected. */
export function parsePawnIoStatus(search: string): PawnIoStatus {
  const raw = new URLSearchParams(search).get('pawnio');
  return (VALID_PAWN_IO as string[]).includes(raw ?? '') ? (raw as PawnIoStatus) : 'ok';
}

/** What Windows makes of a configured start-up entry, from `?autostart=disabledByWindows|unknown`. */
function parseAutostart(search: string): AutostartEffective | null {
  const raw = new URLSearchParams(search).get('autostart');
  return raw === 'disabledByWindows' || raw === 'unknown' ? raw : null;
}

/** Every module on, nothing pending: the service as the mock finds it. */
function mockSources(): ServiceSources {
  return {
    activeModules: ['cpu', 'motherboard', 'memory', 'storage', 'controller', 'psu'],
    requestedDisabledModules: [],
    smartDisabledDrives: [],
    reconfiguration: 'applied',
    drives: [],
  };
}

/** Values for `schema`'s sensors at tick `t`; adds the service sensors' values when present. */
function valuesFor(schema: Schema, t: number): (number | null)[] {
  return schema === SERVICE_MOCK_SCHEMA ? [...mockValues(t), ...serviceMockValues(t)] : mockValues(t);
}

/** Seconds each phase of the demonstration cycle lasts. */
const HEALTH_PHASE_S = 8;

/** The alert of the demonstration cycle: the mock GPU's temperature at `value` °C. */
function mockGpuAlert(level: 'warn' | 'crit', value: number, sinceMs: number): Alert {
  return {
    ruleId: 'gpu-temp',
    sensorId: `${GPU}/temperature/core`,
    deviceId: GPU,
    unit: 'celsius',
    sensorLabel: { key: 'gpu.temperature.core' },
    level,
    value,
    threshold: level === 'crit' ? 90 : 83,
    sinceMs,
    valid: true,
    lastValidMs: sinceMs,
    messageKey: 'rule.gpu-temp.message',
    params: { device: 'Mock GeForce RTX 4080' },
  };
}

/**
 * Demonstration of the rules engine for `pnpm dev`: all clear, then a GPU warning, a GPU critical
 * alert, and back. Phases change every `HEALTH_PHASE_S` seconds; the clock counts in the phase.
 */
function mockHealthCycle(now: () => number = Date.now) {
  const startedAt = now();
  let phase = -1;
  let revision = 0;
  let phaseStartedAt = startedAt;
  let report: HealthReport = { level: 'ok', sinceMs: startedAt, revision, coverage: 'complete', unavailableTargets: [], alerts: [] };
  const advance = (): boolean => {
    const next = Math.floor((now() - startedAt) / (HEALTH_PHASE_S * 1000)) % 3;
    if (next === phase) return false;
    phase = next;
    phaseStartedAt = now();
    const level = (['ok', 'warn', 'crit'] as const)[next];
    const alerts = level === 'ok' ? [] : [mockGpuAlert(level, level === 'warn' ? 86 : 93, phaseStartedAt)];
    report = { level, sinceMs: phaseStartedAt, revision: ++revision, coverage: 'complete', unavailableTargets: [], alerts };
    return true;
  };
  advance();
  return {
    report: () => report,
    clock: (): HealthClock => ({ revision: report.revision, levelElapsedMs: now() - phaseStartedAt }),
    advance,
  };
}

/** The built-in rules, from the fixture that a Rust test keeps equal to `default_rules()`. */
export const MOCK_DEFAULT_RULES = defaultRulesFixture as Rule[];

/**
 * What the engine would report for the demonstration: the GPU temperature with its fixed thresholds
 * (and the level of the health cycle), the mock SSD's volume, every other rule without instances.
 */
function mockRuleStatus(level: HealthReport['level']): RuleStatus[] {
  return MOCK_DEFAULT_RULES.map((rule) => {
    if (rule.id === 'gpu-temp') {
      const current = level === 'warn' || level === 'crit' ? level : 'ok';
      return { ruleId: rule.id, instances: [{ sensorId: `${GPU}/temperature/core`, level: current, warn: 83, crit: 90, warnSource: 'fixed', critSource: 'fixed', valid: true, problem: null }] };
    }
    if (rule.id === 'volume-used') {
      const sensorId = 'storage/device-mock-ssd/percent/volume-mock-guid';
      return { ruleId: rule.id, instances: [{ sensorId, level: 'ok', warn: 90, crit: 97, warnSource: 'fixed', critSource: 'fixed', valid: true, problem: null }] };
    }
    return { ruleId: rule.id, instances: [] };
  });
}

const OVERLAY_FRAMES: readonly OverlayFramesState[] = ['off', 'starting', 'running', 'denied', 'tampered', 'missing', 'failed', 'unavailable'];

/** The frame engine state of the mock overlay while it is on, from `?overlay=` in the URL; `running` otherwise. */
export function parseOverlayFrames(search: string): OverlayFramesState {
  const raw = new URLSearchParams(search).get('overlay');
  return OVERLAY_FRAMES.find((state) => state === raw) ?? 'running';
}

/**
 * A fake overlay controller: off until `overlay.enabled`, then measuring a fixed game with the
 * profile its settings pick. The catalog has the four built-ins, one user profile and one
 * rejected file, so every part of the settings page has something to show.
 */
function mockOverlay(settings: MockSettings, frames: OverlayFramesState) {
  const listeners = new Set<(s: OverlayStatus) => void>();
  const previewListeners = new Set<(e: { open: boolean }) => void>();
  let preview = false;
  const unset = { requested: null, effective: null, state: 'unset', reason: null } as const;
  let engine = frames;
  let hidden = false;
  const status = (): OverlayStatus => {
    const overlay = settings.state().settings.overlay;
    const target = overlay.enabled ? { name: 'cyberpunk2077.exe', pid: 14_320 } : null;
    const hotkey = (value: string | null) => (value === null ? { ...unset } : { requested: value, effective: value, state: 'active' as const, reason: null });
    return {
      enabled: overlay.enabled,
      process: overlay.enabled ? 'running' : 'off',
      processReason: null,
      frames: overlay.enabled ? engine : 'off',
      framesDetail: null,
      target,
      activeProfile: target === null ? null : (overlay.gameProfiles[target.name] ?? overlay.defaultProfile),
      profiles: profiles.entries(),
      diagnostics: [{ file: 'old-layout.json', reason: 'unknown field `colour` at line 12 column 7' }],
      hiddenByUser: overlay.enabled && hidden,
      hotkeys: { toggle: hotkey(overlay.hotkeyToggle), nextProfile: hotkey(overlay.hotkeyNextProfile), benchmark: hotkey(overlay.hotkeyBenchmark) },
      preview,
      previewFailure: null,
      benchmark: { state: 'idle', game: null, elapsedS: null, error: null },
    };
  };
  const emit = () => {
    const next = status();
    listeners.forEach((cb) => cb(next));
  };
  const profiles = mockProfileStore(emit);
  settings.subscribe(emit);
  return {
    get: status,
    profiles,
    /** No preview window in the browser: only the state is followed. */
    setPreview(json: string | null) {
      if (preview === (json !== null)) return;
      preview = json !== null;
      emit();
      previewListeners.forEach((cb) => cb({ open: preview }));
    },
    onPreview(cb: (e: { open: boolean }) => void) {
      previewListeners.add(cb);
      return () => {
        previewListeners.delete(cb);
      };
    },
    subscribe(cb: (s: OverlayStatus) => void) {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    retry() {
      engine = 'running';
      emit();
    },
    reload: emit,
    setHidden(next: boolean) {
      hidden = next;
      emit();
    },
  };
}

/** Bytes of a fake part before the mock recorder opens the next one. */
export const MOCK_LOG_PART_BYTES = 50_000;

export function parseLogState(search: string): LogState {
  const raw = new URLSearchParams(search).get('log');
  return raw === 'recording' || raw === 'paused' || raw === 'error' ? raw : 'idle';
}

/**
 * A fake CSV recorder: one row and ~180 bytes per second while recording, a new part past
 * `MOCK_LOG_PART_BYTES`, and a revision that grows with every change like the core's.
 */
function mockLogRecorder(initial: LogState) {
  const listeners = new Set<(s: LogStatus) => void>();
  let timer: ReturnType<typeof setInterval> | undefined;
  const noHotkey = { requested: null, effective: null, state: 'unset', reason: null } as const;
  let current: LogStatus = {
    revision: 1,
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
    hotkeys: { toggle: { ...noHotkey }, pause: { ...noHotkey } },
  };
  const pathFor = (session: number, part: number) =>
    `C:\\Users\\mock\\Documents\\OpenMonitorAdvanced\\logs\\oma-mock-${session}-part${part}.csv`;
  const set = (patch: Partial<LogStatus>): LogStatus => {
    current = { ...current, ...patch, revision: current.revision + 1 };
    listeners.forEach((cb) => cb(current));
    return current;
  };
  const sync = () => {
    const wanted = current.state === 'recording' && listeners.size > 0;
    if (wanted && timer === undefined) timer = setInterval(tick, 1000);
    if (!wanted && timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
  };
  const tick = () => {
    const partBytes = current.partBytes + 180;
    const rollover = partBytes > MOCK_LOG_PART_BYTES;
    set({
      recordedMs: current.recordedMs + 1000,
      rows: current.rows + 1,
      bytes: current.bytes + 180,
      part: rollover ? current.part + 1 : current.part,
      partBytes: rollover ? 0 : partBytes,
      path: rollover ? pathFor(current.session, current.part + 1) : current.path,
    });
  };
  const begin = (state: LogState) => {
    const session = current.session + 1;
    current = { ...current, session, state, part: 1, partBytes: 0, recordedMs: 0, rows: 0, bytes: 0, dropped: 0, path: pathFor(session, 1) };
    if (state === 'error') current.error = { key: 'log.error.diskFull', detail: null };
    if (state !== 'idle') current.revision++;
  };
  if (initial !== 'idle') {
    begin(initial);
    current.recordedMs = 767_000;
    current.rows = 767;
    current.bytes = 138_060;
  }
  const command = (patch: () => Partial<LogStatus>) => {
    const next = set(patch());
    sync();
    return next;
  };
  return {
    get: () => current,
    subscribe(cb: (s: LogStatus) => void) {
      listeners.add(cb);
      sync();
      return () => {
        listeners.delete(cb);
        sync();
      };
    },
    start: async () =>
      command(() => {
        begin('recording');
        return {};
      }),
    pause: async () => command(() => ({ state: current.state === 'recording' ? 'paused' : current.state })),
    resume: async () => command(() => ({ state: current.state === 'paused' ? 'recording' : current.state })),
    stop: async () => command(() => ({ state: 'idle', error: null })),
  };
}

/** Browser-only backend used by `pnpm dev` and component tests. */
export function createMockBackend(intervalMs = 1000): Backend {
  const initialState = parseServiceState(typeof location === 'undefined' ? '' : location.search);
  const schema = initialState === 'connected' ? SERVICE_MOCK_SCHEMA : MOCK_SCHEMA;
  const ids = schema === SERVICE_MOCK_SCHEMA ? SERVICE_IDS : MOCK_IDS;
  let seq = 0;
  let startup = MOCK_STARTUP;
  let startedAtMs: number | null = null;
  let timer: ReturnType<typeof setInterval> | undefined;
  const pawnIo = parsePawnIoStatus(typeof location === 'undefined' ? '' : location.search);
  const autostart = parseAutostart(typeof location === 'undefined' ? '' : location.search);
  const statusFor = (state: ServiceState): ServiceStatus =>
    state === 'connected'
      ? { state, detail: null, pawnIo, sources: mockSources() }
      : { state, detail: null, pawnIo: null, sources: null };
  let serviceStatus: ServiceStatus = statusFor(initialState);
  const diskStates: DiskStateEntry[] =
    schema === SERVICE_MOCK_SCHEMA ? [{ deviceId: MOCK_HDD, power: 'standby' }] : [];
  const stats = new StatsAccumulator();
  const settings = new MockSettings(parsePersistence(typeof location === 'undefined' ? '' : location.search));
  const listeners = new Set<(s: Snapshot) => void>();
  const serviceListeners = new Set<(s: ServiceStatus) => void>();
  const recorder = mockLogRecorder(parseLogState(typeof location === 'undefined' ? '' : location.search));
  const overlay = mockOverlay(settings, parseOverlayFrames(typeof location === 'undefined' ? '' : location.search));
  const editorData = mockEditorData();
  let benchmarks = structuredClone(MOCK_BENCHMARKS);
  const perf = mockPerformance(parsePerfScenario(typeof location === 'undefined' ? '' : location.search), () => serviceStatus.state === 'connected');
  const search = typeof location === 'undefined' ? '' : location.search;
  const bench = mockBench(parseBenchScenario(search), () => perf.running(), perf.system().gpus, perf.system().volumes, parseDiskScenario(search));
  const cycle = mockHealthCycle();
  const healthListeners = new Set<(r: HealthReport) => void>();
  const clockListeners = new Set<(c: HealthClock) => void>();
  let healthTimer: ReturnType<typeof setInterval> | undefined;
  const tickHealth = () => {
    if (cycle.advance()) healthListeners.forEach((cb) => cb(cycle.report()));
    clockListeners.forEach((cb) => cb(cycle.clock()));
  };
  const watchHealth = <T>(set: Set<T>, cb: T) => {
    set.add(cb);
    healthTimer ??= setInterval(tickHealth, 1000);
    return () => {
      set.delete(cb);
      if (healthListeners.size + clockListeners.size === 0 && healthTimer !== undefined) {
        clearInterval(healthTimer);
        healthTimer = undefined;
      }
    };
  };
  const emit = () => {
    seq++;
    const snapshot: Snapshot = { revision: schema.revision, seq, timestampMs: Date.now(), values: valuesFor(schema, seq) };
    // Only the service mock carries qualities: the sleeping disk's reading is suspended.
    if (schema === SERVICE_MOCK_SCHEMA) snapshot.quality = ids.map((id) => (id === MOCK_HDD_SENSOR ? 2 : 0));
    startedAtMs ??= snapshot.timestampMs;
    stats.push(ids, snapshot.values);
    listeners.forEach((cb) => cb(snapshot));
  };
  const setServiceStatus = (status: ServiceStatus) => {
    serviceStatus = status;
    serviceListeners.forEach((cb) => cb(status));
  };
  return {
    getSchema: async () => schema,
    getHistory: async (requestedIds, seconds, maxPoints) => {
      const n = Math.max(0, Math.min(Math.floor(seconds), MOCK_HISTORY_SECONDS));
      const now = Date.now();
      const rows = Array.from({ length: n }, (_, i) => valuesFor(schema, seq - n + 1 + i));
      const indices = requestedIds.map((id) => ids.indexOf(id));
      const raw: HistoryWindow = {
        timestampsMs: rows.map((_, i) => now - (n - 1 - i) * intervalMs),
        series: indices.map((k) => rows.map((row) => (k < 0 ? null : row[k]))),
      };
      const window =
        maxPoints === undefined ? raw : decimateWindow(raw, Math.min(Math.max(Math.floor(maxPoints), 2), MOCK_HISTORY_SECONDS));
      return { revision: schema.revision, seq, ...window };
    },
    onSchema: async () => () => {},
    getDiskStates: async () => [...diskStates],
    onDiskStates: async () => () => {},
    getStartupStatus: async () => startup,
    enableVendorLibraries: async () => {
      startup = { ...startup, safeMode: false };
      return startup;
    },
    getStats: async (requestedIds) => ({ revision: schema.revision, stats: stats.get(requestedIds) }),
    resetStats: async (requestedIds) => stats.reset(requestedIds),
    getSession: async () => ({ startedAtMs, intervalMs }),
    getGpuProcesses: async (deviceId) => (deviceId === GPU ? sortGpuProcesses(mockGpuProcesses(seq)) : []),
    onSnapshot: async (cb) => {
      listeners.add(cb);
      timer ??= setInterval(emit, intervalMs);
      return () => {
        listeners.delete(cb);
        if (listeners.size === 0 && timer !== undefined) {
          clearInterval(timer);
          timer = undefined;
        }
      };
    },
    getServiceStatus: async () => serviceStatus,
    onServiceStatus: async (cb) => {
      serviceListeners.add(cb);
      return () => serviceListeners.delete(cb);
    },
    setAntiCheat: async (enabled) => {
      // The shell stores the mode in the settings (`sources.antiCheat`) and emits `oma:settings`.
      if (settings.state().settings.sources.antiCheat !== enabled) settings.update({ sources: { antiCheat: enabled } });
      setServiceStatus(statusFor(enabled ? 'antiCheat' : 'unreachable'));
      return serviceStatus;
    },
    startService: async () => {
      setServiceStatus(statusFor('connected'));
      return serviceStatus;
    },
    getSettings: async () => settings.state(),
    // Rejects with a plain `{ field, key }` object, like the Tauri command.
    updateSettings: async (patch) => settings.update(patch),
    resetRuleOverride: async (ruleId) => settings.resetRuleOverride(ruleId),
    onSettings: async (cb) => settings.subscribe(cb),
    importWebviewState: async (legacy) => settings.import(legacy),
    takePendingView: async () => null,
    onNavigate: async () => () => {},
    getHealth: async () => cycle.report(),
    onHealth: async (cb) => watchHealth(healthListeners, cb),
    getHealthClock: async () => cycle.clock(),
    onHealthClock: async (cb) => watchHealth(clockListeners, cb),
    // The mock does not evaluate rules: the status follows the health cycle, not the settings.
    getRuleStatus: async () => mockRuleStatus(cycle.report().level),
    getDefaultRules: async () => structuredClone(MOCK_DEFAULT_RULES),
    refreshAutostart: async () => {
      const configured = settings.state().settings.tray.autostart;
      return { configured, effective: autostart ?? (configured ? 'enabled' : 'notConfigured'), error: null };
    },
    getAppInfo: async () => ({
      version: '0.1.0',
      serviceVersion: serviceStatus.state === 'connected' ? '0.1.0' : null,
      protocolVersion: 2,
      settingsPath: 'C:\\Users\\mock\\AppData\\Roaming\\OpenMonitorAdvanced',
      logsPath: 'C:\\Users\\mock\\AppData\\Local\\OpenMonitorAdvanced\\logs',
    }),
    // Nothing to open in the browser: the request is only logged.
    openKnownPath: async (target) => console.info('mock: open', target),
    getLogStatus: async () => recorder.get(),
    onLogStatus: async (cb) => recorder.subscribe(cb),
    logStart: () => recorder.start(),
    logPause: () => recorder.pause(),
    logResume: () => recorder.resume(),
    logStop: () => recorder.stop(),
    openLogFolder: async () => console.info('mock: open log folder'),
    // No native dialog in the browser: pretend the user picked a folder.
    pickLogFolder: async () => 'C:\\Users\\mock\\Documents\\OpenMonitorAdvanced\\logs',
    // No global hotkeys in the browser.
    setLogHotkeysSuspended: async () => {},
    getOverlayStatus: async () => overlay.get(),
    onOverlayStatus: async (cb) => overlay.subscribe(cb),
    overlayRetry: async () => overlay.retry(),
    // No profile folder in the browser: the catalog stays the same.
    overlayReloadProfiles: async () => overlay.reload(),
    setOverlayHidden: async (hidden) => overlay.setHidden(hidden),
    overlayLoadProfile: async (id) => overlay.profiles.load(id),
    overlaySaveProfile: async (id, json) => overlay.profiles.save(id, json),
    overlayDeleteProfile: async (id) => overlay.profiles.remove(id),
    overlayDuplicateProfile: async (id) => overlay.profiles.duplicate(id),
    // No file dialogs in the browser: import is always cancelled, export always succeeds.
    overlayImportProfile: async () => null,
    overlayExportProfile: async () => true,
    overlayFontFamilies: async () => ['Segoe UI', 'Segoe UI Variable', 'Bahnschrift', 'Consolas', 'Arial'],
    overlayPreview: async (json) => overlay.setPreview(json),
    overlayEditorProfile: async () => {},
    overlayUseNow: async (id) => console.info('mock: use profile now', id),
    overlayEditorDirty: async () => {},
    openOverlayEditor: async () => {
      window.open(`${location.pathname}?window=overlay-editor`, '_blank');
    },
    appQuitConfirmed: async () => console.info('mock: quit'),
    onOverlayEditorData: async (cb) => editorData.subscribe(cb),
    onOverlayPreview: async (cb) => overlay.onPreview(cb),
    onOverlayEditorQuit: async () => () => {},
    // Commands reject with plain text, like the Tauri commands.
    performanceSystem: async () => perf.system(),
    performancePreview: async (request) => perf.preview(request),
    performanceStart: async (request) => perf.start(request),
    performanceStop: async () => perf.stop(),
    performanceStatus: async () => perf.status(),
    performanceHistory: async () => perf.history(),
    performanceSession: async (id) => perf.session(id),
    performanceDelete: async (id) => perf.remove(id),
    // No native dialog in the browser: pretend the user saved the file.
    performanceExport: async () => 'oma-stress-20261006-090507.json',
    onPerformanceStatus: async (cb) => perf.subscribe(cb),
    onPerformanceQuit: async () => () => {},
    performanceQuitConfirmed: async () => console.info('mock: stop and quit'),
    performanceBenchStart: async () => bench.start(),
    performanceGpuBenchStart: async (deviceId) => bench.startGpu(deviceId),
    performanceDiskBenchStart: async (request) => bench.startDisk(request),
    performanceDiskProbe: async (folder) => {
      if (folder.startsWith('\\')) throw 'disk:remote';
      const volume = MOCK_VOLUMES.find((v) => folder.toUpperCase().startsWith(v.root.toUpperCase()));
      if (!volume) throw 'disk:not_found';
      return { ...volume, folder };
    },
    performanceDiskPick: async () => MOCK_VOLUMES[1].root + 'OMA tests',
    performanceBenchStop: async () => bench.stop(),
    performanceBenchStatus: async () => bench.status(),
    performanceScores: async () => bench.scores(),
    performanceScore: async (id) => bench.score(id),
    performanceScoreDelete: async (id) => bench.remove(id),
    performanceBoard: async () => mockBoardTable(),
    performanceSharePreview: async (id, overclock) => bench.sharePreview(id, overclock),
    performanceShareSend: async (id) => bench.share(id),
    performanceScoreExport: async (id) => (bench.score(id) ? 'oma-score-cpu-20261009-093000.json' : null),
    performanceBoardRefresh: async () => mockBoardTable(),
    performanceBaseline: async () => ({ provisional: true, gpuProvisional: true, diskProvisional: true }),
    onPerformanceBench: async (cb) => bench.subscribe(cb),
    benchmarkToggle: async () => console.info('mock: benchmark toggle'),
    benchmarkList: async () => structuredClone(benchmarks),
    benchmarkOpenCsv: async (id) => console.info('mock: open benchmark CSV', id),
    benchmarkOpenFolder: async () => console.info('mock: open benchmarks folder'),
    benchmarkDelete: async (id) => {
      benchmarks = benchmarks.filter((b) => b.id !== id);
    },
    // The browser has no network access to GitHub: the check always finds nothing to report.
    checkUpdates: async () => MOCK_UPDATE_STATUS,
    getUpdateStatus: async () => MOCK_UPDATE_STATUS,
    openReleasePage: async () => console.info('mock: open release page'),
    onUpdateStatus: async () => () => {},
    // No native dialog or file system in the browser: pretend the user saved the report.
    exportSensorReport: async () => ({ fileName: 'oma-report-20261004-090507.json' }),
    revealSensorReport: async () => console.info('mock: open report folder'),
  };
}
