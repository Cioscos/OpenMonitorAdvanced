import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n } from './lib/i18n/index.svelte';
beforeEach(() => { localStorage.clear(); i18n.locale = 'en'; });
afterEach(cleanup);
import { flushSync } from 'svelte';
import App from './App.svelte';
import { SECTION_KEY } from './lib/advanced/persist';
import { MOCK_SCHEMA, mockValues } from './lib/backend/mock';
import { LiveStore } from './lib/live.svelte';
import type { Schema } from './lib/types';
import { FakeBackend } from './test/fake-backend';

const IGPU = 'gpu/pci-0000:11:00.0';

/** MOCK_SCHEMA plus an integrated AMD GPU with a load sensor (value 12). */
const withIntegratedGpu = (schema: Schema): Schema => ({
  ...schema,
  devices: [
    ...schema.devices,
    { id: IGPU, kind: 'gpu', name: 'AMD Radeon(TM) Graphics', vendor: 'AMD', properties: { integrated: 'true' } },
  ],
  sensors: [
    ...schema.sensors,
    { id: `${IGPU}/load/core`, deviceId: IGPU, kind: 'load', unit: 'percent', label: { key: 'gpu.load.core' }, source: 'pdh', category: 'load' },
  ],
});

test('simple view shows the banner and the CPU, RAM and network tiles', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: Date.now(), values: mockValues(1) });
  flushSync();

  expect(screen.getByText('Monitoring active')).toBeTruthy();
  expect(screen.getByText('Basic mode')).toBeTruthy();
  expect(screen.getByText('CPU')).toBeTruthy();
  expect(screen.getByText('RAM')).toBeTruthy();
  expect(screen.getByText('Network · Disks')).toBeTruthy();
  expect(screen.getByText((text) => text.startsWith('C: 65% · '))).toBeTruthy();
  expect(screen.queryByText('Re-enable')).toBeNull();
});

test('simple view shows only discrete gpus', async () => {
  const backend = new FakeBackend(withIntegratedGpu(MOCK_SCHEMA));
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: Date.now(), values: [...mockValues(1), 12] });
  flushSync();

  expect(screen.getAllByText('GPU')).toHaveLength(1);
  expect(screen.getByText('Mock GeForce RTX 4080')).toBeTruthy();
  expect(screen.getByText((text) => text.startsWith('VRAM ') && text.endsWith(' / 16.0 GB'))).toBeTruthy();
  expect(screen.queryByText('AMD Radeon(TM) Graphics')).toBeNull();
});

test('simple view falls back to the integrated gpu', async () => {
  const noDiscrete: Schema = {
    ...MOCK_SCHEMA,
    devices: MOCK_SCHEMA.devices.filter((d) => d.kind !== 'gpu'),
    sensors: MOCK_SCHEMA.sensors.filter((s) => !s.deviceId.startsWith('gpu/')),
  };
  const backend = new FakeBackend(withIntegratedGpu(noDiscrete));
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  flushSync();

  expect(screen.getAllByText('GPU')).toHaveLength(1);
  expect(screen.getByText('AMD Radeon(TM) Graphics')).toBeTruthy();
});

test('safe mode notice reenables vendor libraries', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.startup = { safeMode: true, reason: 'crash', crashModule: 'nvml.dll' };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(screen.getByText('GPU safe mode')).toBeTruthy());
  expect(screen.getByText((text) => text.startsWith('The previous session crashed in nvml.dll.'))).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Re-enable' }));

  expect(backend.enableCalls).toBe(1);
  await vi.waitFor(() => expect(screen.queryByText('GPU safe mode')).toBeNull());
});

test('safe mode notice explains the --safe flag in Italian', async () => {
  i18n.locale = 'it';
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.startup = { safeMode: true, reason: 'flag', crashModule: null };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(screen.getByText('Modalità sicura GPU')).toBeTruthy());
  expect(screen.getByText((text) => text.startsWith('Avvio con --safe:'))).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Riattiva' })).toBeTruthy();
});


test('clicking the GPU tile opens its Advanced page, the toggle goes back', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: 1000, values: mockValues(1) });
  flushSync();

  await fireEvent.click(screen.getByText('Mock GeForce RTX 4080').closest('button')!);
  expect(screen.getByRole('heading', { level: 2 }).textContent).toBe('GPU');
  expect(localStorage.getItem(SECTION_KEY)).toBe('gpu/pci-0000:01:00.0');
  expect(localStorage.getItem('oma.view')).toBe('advanced');

  await fireEvent.click(screen.getByRole('tab', { name: 'Simple' }));
  expect(screen.getByText('Monitoring active')).toBeTruthy();
});

test('the network tile opens the network page', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  flushSync();

  await fireEvent.click(screen.getByText('Network · Disks').closest('button')!);
  expect(screen.getByRole('heading', { level: 2 }).textContent).toBe('Network');
  expect(localStorage.getItem(SECTION_KEY)).toBe('network/mock-eth');
});

test('the health banner counts from the start of the core session', async () => {
  const now = 50_000_000;
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.session = { startedAtMs: now - 125 * 60_000, intervalMs: 1000 };
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: now, values: mockValues(1) });

  await vi.waitFor(() => expect(screen.getByText('for 2 h 5 min')).toBeTruthy());
});

test('without a session start the banner counts from the first snapshot', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: 50_000_000, values: mockValues(1) });
  flushSync();

  expect(screen.getByText('for 0 min')).toBeTruthy();
});

test('the stale badge appears after five silent seconds and goes away with new data', async () => {
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  try {
    vi.setSystemTime(1_000_000);
    const backend = new FakeBackend(MOCK_SCHEMA);
    const store = new LiveStore();
    render(App, { backend, store });
    await vi.waitFor(() => expect(store.schema).not.toBeNull());
    backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: Date.now(), values: mockValues(1) });

    vi.advanceTimersByTime(5000);
    flushSync();
    expect(screen.queryByText('Data not updating')).toBeNull();

    vi.advanceTimersByTime(1000);
    flushSync();
    expect(screen.getByText('Data not updating')).toBeTruthy();

    backend.emitSnapshot({ revision: 1, seq: 2, timestampMs: Date.now(), values: mockValues(2) });
    vi.advanceTimersByTime(1000);
    flushSync();
    expect(screen.queryByText('Data not updating')).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

test('the stale badge also appears when no snapshot ever arrives', async () => {
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  try {
    vi.setSystemTime(1_000_000);
    const backend = new FakeBackend(MOCK_SCHEMA);
    const store = new LiveStore();
    render(App, { backend, store });
    await vi.waitFor(() => expect(store.schema).not.toBeNull());

    vi.advanceTimersByTime(6000);
    flushSync();
    expect(screen.getByText('Data not updating')).toBeTruthy();
  } finally {
    vi.useRealTimers();
  }
});
