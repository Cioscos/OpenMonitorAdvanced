import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n } from './lib/i18n/index.svelte';
beforeEach(() => { localStorage.clear(); i18n.locale = 'en'; });
afterEach(() => { cleanup(); settings.state = null; settings.errors = {}; });
import { flushSync } from 'svelte';
import App from './App.svelte';
import { MOCK_SCHEMA, mockValues } from './lib/backend/mock';
import { LiveStore } from './lib/live.svelte';
import { settings } from './lib/settings.svelte';
import type { Schema } from './lib/types';
import { FakeBackend, makeLogStatus } from './test/fake-backend';

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
  expect(screen.queryByText('Basic mode')).toBeNull();
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
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { language: 'it' } });
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
  await vi.waitFor(() => expect(settings.state?.settings.advanced.section).toBe('gpu/pci-0000:01:00.0'));
  await vi.waitFor(() => expect(settings.state?.settings.view.last).toBe('advanced'));

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
  await vi.waitFor(() => expect(settings.state?.settings.advanced.section).toBe('network/mock-eth'));
});

test('the health banner shows the core clock, not a difference of system times', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.healthClock = { revision: 0, levelElapsedMs: 125 * 60_000 };
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: 50_000_000, values: mockValues(1) });

  await vi.waitFor(() => expect(screen.getByText('for 2 h 5 min')).toBeTruthy());
  backend.emitHealthClock({ revision: 0, levelElapsedMs: 126 * 60_000 });
  await vi.waitFor(() => expect(screen.getByText('for 2 h 6 min')).toBeTruthy());
});

test('the recorder follows the log and Esc in its deck leaves the settings open', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.logStatus = makeLogStatus({ revision: 3, session: 1, state: 'recording', recordedMs: 767_000 });
  const store = new LiveStore();
  render(App, { backend, store });
  const recorder = (await screen.findByRole('button', { name: 'Recording, 00:12:47' })) as HTMLButtonElement;
  backend.emitLogStatus(makeLogStatus({ revision: 4, session: 1, state: 'paused', recordedMs: 768_000 }));
  await vi.waitFor(() => expect(recorder.getAttribute('aria-label')).toBe('Recording paused, 00:12:48'));

  await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  await vi.waitFor(() => expect(screen.getByRole('button', { name: 'Back' })).toBeTruthy());
  await fireEvent.click(recorder);
  await fireEvent.keyDown(recorder, { key: 'Escape' });
  expect(recorder.getAttribute('aria-expanded')).toBe('false');
  expect(screen.getByRole('button', { name: 'Back' })).toBeTruthy();

  await fireEvent.click(recorder);
  await fireEvent.click(screen.getByRole('button', { name: 'Open folder' }));
  expect(backend.logCalls).toContain('openLogFolder');
});

test('without a clock the banner starts from zero', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: 50_000_000, values: mockValues(1) });
  flushSync();

  expect(screen.getByText('for less than a minute')).toBeTruthy();
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

test('the service badge shows the reason and reacts to the anti-cheat toggle', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.serviceStatus = { state: 'antiCheat', detail: null, pawnIo: null, sources: null };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(screen.getByText('Basic mode')).toBeTruthy());
  expect(screen.getByText('Anti-cheat compatible mode is on: this app will not connect to or start the sensor service.')).toBeTruthy();

  await fireEvent.click(screen.getByRole('button', { name: 'Turn off anti-cheat mode' }));
  expect(backend.setAntiCheatCalls).toEqual([false]);
});

test('command failures are shown without a false success', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.serviceStatus = { state: 'unreachable', detail: null, pawnIo: null, sources: null };
  backend.startServiceError = 'persist_failed';
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(screen.getByText('Basic mode')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: 'Start' }));

  await vi.waitFor(() => expect(screen.getByText('Could not save or apply the change. Try again.')).toBeTruthy());
  // Still unreachable: no false success, no raw error code shown.
  expect(screen.getByText('The sensor service is not running or not responding.')).toBeTruthy();
  expect(screen.queryByText('persist_failed')).toBeNull();
});

test('anti-cheat stop failure is visible', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.serviceStatus = { state: 'antiCheat', detail: null, pawnIo: null, sources: null };
  backend.setAntiCheatError = 'stop_failed';
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(screen.getByText('Basic mode')).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: 'Turn off anti-cheat mode' }));

  await vi.waitFor(() => expect(screen.getByText('Could not save or apply the change. Try again.')).toBeTruthy());
});

test('late initial status cannot overwrite a newer event', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.serviceStatus = { state: 'starting', detail: null, pawnIo: null, sources: null };
  // Delay the initial read so it resolves after the event below, but with the value it saw
  // (`starting`) at call time: a naive implementation would let this stale reply win.
  backend.getServiceStatus = () => {
    const snapshot = backend.serviceStatus;
    return new Promise((resolve) => setTimeout(() => resolve(snapshot), 10));
  };
  render(App, { backend, store: new LiveStore() });

  // The event arrives first and reports 'connected'; the stale 'starting' read must not win.
  backend.emitServiceStatus({ state: 'connected', detail: null, pawnIo: null, sources: null });
  await new Promise((resolve) => setTimeout(resolve, 20));

  expect(screen.queryByText('Basic mode')).toBeNull();
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

const pageTitle = () => screen.getByRole('heading', { level: 2 }).textContent;

/** Simple is on screen once the app is ready: its tab is selected and the Advanced page is absent. */
async function simpleShown() {
  await vi.waitFor(() => expect(settings.state).not.toBeNull());
  await vi.waitFor(() => expect(screen.getByText('CPU')).toBeTruthy());
  expect(screen.getByRole('tab', { name: 'Simple' }).getAttribute('aria-selected')).toBe('true');
  expect(screen.queryByRole('heading', { level: 2 })).toBeNull();
}

test('the last view is restored', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ view: { last: 'advanced' } });
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(pageTitle()).toBe('CPU'));
});

test('the default view beats the last one', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' }, view: { last: 'advanced' } });
  render(App, { backend, store: new LiveStore() });

  await simpleShown();
});

test('a view requested from the tray at start beats the default', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' } });
  backend.pendingView = { view: 'advanced' };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(pageTitle()).toBe('CPU'));
  expect(backend.takePendingViewCalls).toBe(1);
  // The requested view is now the last one shown.
  await vi.waitFor(() => expect(settings.state?.settings.view.last).toBe('advanced'));
});

test('a navigate event that arrives before the initial view is chosen wins', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' } });
  // The shell emitted `oma:navigate` before the page was listening.
  // One-shot: the acknowledging read must not emit it again.
  backend.beforeTakePendingView = () => {
    backend.beforeTakePendingView = null;
    backend.emitNavigate({ view: 'advanced' });
  };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(pageTitle()).toBe('CPU'));
  expect(backend.navigateSubscribedBeforePendingRead).toBe(true);
  // The first read and the acknowledging one; the event came once, so nothing loops.
  expect(backend.takePendingViewCalls).toBe(2);
});

test('a navigate event that arrives while the pending read is in flight is acknowledged', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' } });
  // The shell records a request right after answering the first read: the reply is null but the
  // event reaches the page before the view is chosen.
  const take = backend.takePendingView.bind(backend);
  let first = true;
  backend.takePendingView = async () => {
    const view = await take();
    if (first) {
      first = false;
      backend.pendingView = { view: 'advanced' };
      backend.emitNavigate({ view: 'advanced' });
    }
    return view;
  };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(pageTitle()).toBe('CPU'));
  await vi.waitFor(() => expect(backend.takePendingViewCalls).toBe(2));
  expect(backend.pendingView).toBeNull();
});

test('a navigate event while the window is open switches the view and remembers it', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' } });
  render(App, { backend, store: new LiveStore() });
  await simpleShown();

  backend.emitNavigate({ view: 'advanced' });
  await vi.waitFor(() => expect(pageTitle()).toBe('CPU'));
  await vi.waitFor(() => expect(settings.state?.settings.view.last).toBe('advanced'));

  backend.emitNavigate({ view: 'simple' });
  await simpleShown();
  await vi.waitFor(() => expect(settings.state?.settings.view.last).toBe('simple'));
});

test('a navigate event is acknowledged by taking the pending request', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  render(App, { backend, store: new LiveStore() });
  await simpleShown();
  const calls = backend.takePendingViewCalls;

  // The shell keeps the request until the page takes it, even with the window open.
  backend.pendingView = { view: 'advanced' };
  backend.emitNavigate({ view: 'advanced' });
  await vi.waitFor(() => expect(backend.takePendingViewCalls).toBe(calls + 1));
  expect(backend.pendingView).toBeNull();
});

test('a toast opens the page of its device', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' } });
  backend.pendingView = { view: 'advanced', deviceId: 'storage/device-mock-ssd' };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(pageTitle()).toBe('Disk'));
  expect(screen.getByText('Disk 0 (C:)', { selector: '.device' })).toBeTruthy();
  expect(screen.queryByText('The device of this alert is no longer available.')).toBeNull();
  await vi.waitFor(() => expect(settings.state?.settings.advanced.section).toBe('storage/device-mock-ssd'));
});

test('a toast clicked with the window open switches to its device', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' } });
  render(App, { backend, store: new LiveStore() });
  await simpleShown();

  backend.emitNavigate({ view: 'advanced', deviceId: 'gpu/pci-0000:01:00.0' });
  await vi.waitFor(() => expect(pageTitle()).toBe('GPU'));

  // Already on the Advanced view: another device's toast still moves the page.
  backend.emitNavigate({ view: 'advanced', deviceId: 'storage/device-mock-ssd' });
  await vi.waitFor(() => expect(pageTitle()).toBe('Disk'));
});

test('missing device shows fallback', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' } });
  backend.pendingView = { view: 'advanced', deviceId: 'storage/gone' };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(pageTitle()).toBe('CPU'));
  expect(screen.getByText('The device of this alert is no longer available.').getAttribute('role')).toBe('status');

  // Choosing a page clears the message.
  await fireEvent.click(screen.getByRole('button', { name: /RAM/ }));
  await vi.waitFor(() => expect(screen.queryByText('The device of this alert is no longer available.')).toBeNull());
});

test('switching view saves the last one', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { defaultView: 'simple' } });
  render(App, { backend, store: new LiveStore() });
  await simpleShown();

  await fireEvent.click(screen.getByRole('tab', { name: 'Advanced' }));
  await vi.waitFor(() => expect(settings.state?.settings.view.last).toBe('advanced'));
  await fireEvent.click(screen.getByRole('tab', { name: 'Simple' }));
  await vi.waitFor(() => expect(settings.state?.settings.view.last).toBe('simple'));
});

test('starting without a request does not write the view', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  render(App, { backend, store: new LiveStore() });
  await simpleShown();

  expect(settings.state?.settings.view.last).toBeUndefined();
});

test('the legacy view key is imported once and removed', async () => {
  localStorage.setItem('oma.view', 'advanced');
  const backend = new FakeBackend(MOCK_SCHEMA);
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(pageTitle()).toBe('CPU'));
  expect(backend.importCalls).toHaveLength(1);
  expect(localStorage.getItem('oma.view')).toBeNull();
});

test('the stale threshold follows the sampling interval of the settings', async () => {
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  try {
    vi.setSystemTime(1_000_000);
    const backend = new FakeBackend(MOCK_SCHEMA);
    // The session still reports 1 s; the setting (5 s, so 25 s of silence) is what counts.
    await backend.settings.update({ general: { intervalMs: 5000 } });
    const store = new LiveStore();
    render(App, { backend, store });
    await vi.waitFor(() => expect(store.schema).not.toBeNull());
    await vi.waitFor(() => expect(settings.state).not.toBeNull());

    vi.advanceTimersByTime(20_000);
    flushSync();
    expect(screen.queryByText('Data not updating')).toBeNull();

    vi.advanceTimersByTime(6000);
    flushSync();
    expect(screen.getByText('Data not updating')).toBeTruthy();
  } finally {
    vi.useRealTimers();
  }
});
