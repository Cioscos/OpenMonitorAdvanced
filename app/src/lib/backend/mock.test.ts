import { catalogs } from '../i18n/index.svelte';
import { MOCK_HISTORY_SECONDS, MOCK_SCHEMA, createMockBackend, mockGpuProcesses, mockValues, sortGpuProcesses } from './mock';

const GPU = 'gpu/pci-0000:01:00.0';
const CPU_LOAD = 'cpu/0/load/total';

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
