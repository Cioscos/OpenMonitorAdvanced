import type {
  AutostartEffective,
  GpuProcess,
  HistoryWindow,
  Label,
  Schema,
  Sensor,
  SensorKind,
  PawnIoStatus,
  ServiceSources,
  ServiceState,
  ServiceStatus,
  Snapshot,
  Source,
  StartupStatus,
  Unit,
} from '../types';
import type { Backend } from './backend';
import { decimateWindow } from './decimate';
import { MockSettings, parsePersistence } from './mockSettings';
import { StatsAccumulator } from './mockStats';

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
];

/** `MOCK_SCHEMA` plus the sensor-service devices and sensors above. */
export const SERVICE_MOCK_SCHEMA: Schema = {
  revision: MOCK_SCHEMA.revision,
  devices: [
    ...MOCK_SCHEMA.devices,
    { id: SERVICE_DEVICE, kind: 'motherboard', name: 'Mock Motherboard' },
    // A USB disk whose bridge hides the serial: its SMART cannot be switched off on its own.
    { id: 'storage/device-mock-usb', kind: 'storage', name: 'Disk 1 (USB)', properties: { smartSelectable: 'false' } },
  ],
  sensors: [...MOCK_SCHEMA.sensors, ...SERVICE_SENSORS],
};

/** Deterministic plausible values for `SERVICE_SENSORS`, same tick as `mockValues`. */
function serviceMockValues(t: number): number[] {
  const wave = (period: number, phase = 0) => (Math.sin((t + phase) / period) + 1) / 2;
  return [40 + 20 * wave(13), 30 + 40 * wave(9, 1), 800 + 200 * wave(15), 750 + 150 * wave(17, 2), 12 + 0.2 * wave(21)];
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
    smartDisabledDrives: [],
    reconfiguration: 'applied',
    smartBlockedBy: [],
  };
}

/** Values for `schema`'s sensors at tick `t`; adds the service sensors' values when present. */
function valuesFor(schema: Schema, t: number): (number | null)[] {
  return schema === SERVICE_MOCK_SCHEMA ? [...mockValues(t), ...serviceMockValues(t)] : mockValues(t);
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
  const stats = new StatsAccumulator();
  const settings = new MockSettings(parsePersistence(typeof location === 'undefined' ? '' : location.search));
  const listeners = new Set<(s: Snapshot) => void>();
  const serviceListeners = new Set<(s: ServiceStatus) => void>();
  const emit = () => {
    seq++;
    const snapshot: Snapshot = { revision: schema.revision, seq, timestampMs: Date.now(), values: valuesFor(schema, seq) };
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
    onSettings: async (cb) => settings.subscribe(cb),
    importWebviewState: async (legacy) => settings.import(legacy),
    takePendingView: async () => null,
    onNavigate: async () => () => {},
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
  };
}
