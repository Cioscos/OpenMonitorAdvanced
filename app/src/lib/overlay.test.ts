import { MOCK_SCHEMA } from './backend/mock';
import { FakeBackend, makeOverlayStatus } from '../test/fake-backend';
import { overlay, retryVisible } from './overlay.svelte';
import type { OverlayStatus } from './types';

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

let off: (() => void) | undefined;
afterEach(() => {
  off?.();
  off = undefined;
  vi.restoreAllMocks();
});

test('overlay store reads then listens', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.overlayStatus = makeOverlayStatus({ enabled: true, frames: 'running' });
  off = await overlay.connect(backend);
  // Subscribed before reading, so a change in between is not lost.
  expect(backend.overlayCalls).toEqual(['onOverlayStatus', 'getOverlayStatus']);
  expect(overlay.status?.frames).toBe('running');
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: true, frames: 'failed' }));
  expect(overlay.status?.frames).toBe('failed');
});

test('overlay store keeps an event that beat the read', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const read = deferred<OverlayStatus | null>();
  vi.spyOn(backend, 'getOverlayStatus').mockReturnValue(read.promise);
  const connecting = overlay.connect(backend);
  await vi.waitFor(() => expect(backend.overlayListenerCount).toBe(1));
  backend.emitOverlayStatus(makeOverlayStatus({ frames: 'denied' }));
  read.resolve(makeOverlayStatus({ frames: 'starting' }));
  off = await connecting;
  expect(overlay.status?.frames).toBe('denied');
});

test('overlay store drops a superseded connection', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const first = await overlay.connect(backend);
  first();
  expect(backend.overlayListenerCount).toBe(0);
  expect(overlay.status).toBeNull();
  backend.emitOverlayStatus(makeOverlayStatus({ frames: 'running' }));
  expect(overlay.status).toBeNull();
  // Reopening reads the status again.
  off = await overlay.connect(backend);
  expect(overlay.status?.frames).toBe('running');
});

test('overlay store commands reach the backend only while connected', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await overlay.retry();
  expect(backend.overlayCalls).toEqual([]);
  off = await overlay.connect(backend);
  await overlay.retry();
  await overlay.reloadProfiles();
  expect(backend.overlayCalls.slice(2)).toEqual(['overlayRetry', 'overlayReloadProfiles']);
});

test('retryVisible follows the engine and the process', () => {
  const states = ['off', 'starting', 'running', 'unavailable', 'missing', 'tampered', 'denied', 'failed'] as const;
  expect(states.filter((frames) => retryVisible(makeOverlayStatus({ frames })))).toEqual(['denied', 'failed']);
  expect(retryVisible(makeOverlayStatus({ frames: 'running', process: 'failed', processReason: 'crashing' }))).toBe(true);
  expect(retryVisible(makeOverlayStatus({ frames: 'running', process: 'starting' }))).toBe(false);
  expect(retryVisible(null)).toBe(false);
});
