import { catalogs } from '../i18n/index.svelte';
import { MOCK_SCHEMA, createMockBackend, mockValues } from './mock';

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
  const h = await backend.getHistory(['cpu/0/load/total', 'unknown'], 10);
  expect(h.timestampsMs).toHaveLength(10);
  expect(h.series[0]).toHaveLength(10);
  expect(h.series[1].every((v) => v === null)).toBe(true);
});
