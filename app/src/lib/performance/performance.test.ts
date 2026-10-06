import { MOCK_SCHEMA } from '../backend/mock';
import { FakeBackend, makeRunStatus } from '../../test/fake-backend';
import type { RunStatus, StartRequest, StressSessionSummary } from '../types';
import { performanceStore } from './performance.svelte';

const REQUEST: StartRequest = { component: 'cpu', objective: 'normal', preset: 'quick', custom: null, retryCore: null };

let off: (() => void) | undefined;
afterEach(() => {
  off?.();
  off = undefined;
});

test('connect_subscribes_before_reading', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.performanceStatusValue = makeRunStatus({ state: 'running', sessionId: 'a' });
  off = await performanceStore.connect(backend);
  expect(backend.performanceCalls[0]).toBe('onPerformanceStatus');
  expect(backend.performanceCalls.slice(1).sort()).toEqual(['performanceHistory', 'performanceStatus', 'performanceSystem']);
  expect(performanceStore.status?.sessionId).toBe('a');
  expect(performanceStore.system?.cpuModel).toBe(backend.performanceSystemInfo.cpuModel);
  // An event that came before the read's reply is not overwritten by it.
  off();
  backend.performanceCalls = [];
  let resolve!: (status: RunStatus) => void;
  vi.spyOn(backend, 'performanceStatus').mockReturnValue(new Promise((r) => (resolve = r)));
  const connecting = performanceStore.connect(backend);
  await vi.waitFor(() => expect(backend.performanceStatusListeners.size).toBe(1));
  backend.emitPerformanceStatus(makeRunStatus({ state: 'finished', sessionId: 'b', outcome: 'passed' }));
  resolve(makeRunStatus({ state: 'running', sessionId: 'b' }));
  off = await connecting;
  expect(performanceStore.status?.state).toBe('finished');
});

test('running_follows_status', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  off = await performanceStore.connect(backend);
  expect(performanceStore.running).toBe(false);
  for (const [state, running] of [
    ['starting', true],
    ['running', true],
    ['stopping', true],
    ['finished', false],
    ['idle', false],
  ] as const) {
    backend.emitPerformanceStatus(makeRunStatus({ state }));
    expect(performanceStore.running).toBe(running);
  }
});

test('start_while_running_is_ignored', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  // Not connected: refused without asking the shell.
  expect(await performanceStore.start(REQUEST)).toEqual({ ok: false, reason: 'notConnected' });
  off = await performanceStore.connect(backend);
  expect(await performanceStore.start(REQUEST)).toEqual({ ok: true, id: 'fake-session' });
  expect(backend.performanceStartRequests).toEqual([REQUEST]);
  backend.emitPerformanceStatus(makeRunStatus({ state: 'running', sessionId: 'fake-session' }));
  expect(await performanceStore.start(REQUEST)).toEqual({ ok: false, reason: 'busy' });
  expect(backend.performanceStartRequests).toHaveLength(1);
  // A finished test refreshes the history, and a new start goes through.
  backend.performanceCalls = [];
  backend.emitPerformanceStatus(makeRunStatus({ state: 'finished', sessionId: 'fake-session', outcome: 'passed' }));
  await vi.waitFor(() => expect(backend.performanceCalls).toContain('performanceHistory'));
  expect(await performanceStore.start(REQUEST)).toEqual({ ok: true, id: 'fake-session' });
  expect(backend.performanceStartRequests).toHaveLength(2);
  // The shell's refusal is the rejection.
  backend.performanceStartError = 'a stress test is already running';
  backend.emitPerformanceStatus(makeRunStatus({ state: 'finished', sessionId: 'x', outcome: 'passed' }));
  await expect(performanceStore.start(REQUEST)).rejects.toBe('a stress test is already running');
});

test('reconnecting_keeps_the_history_until_the_new_list_arrives', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const entry = { id: 'old', startedAt: '2026-10-05T10:00:00Z', component: 'cpu', objective: 'normal', preset: 'quick', durationMs: 300_000, outcome: 'passed', verdict: 'passed', params: {} } as const;
  backend.performanceSessions = [entry];
  off = await performanceStore.connect(backend);
  off();
  let resolve!: (list: StressSessionSummary[]) => void;
  vi.spyOn(backend, 'performanceHistory').mockReturnValue(new Promise((r) => (resolve = r)));
  const connecting = performanceStore.connect(backend);
  await vi.waitFor(() => expect(backend.performanceStatusListeners.size).toBe(1));
  expect(performanceStore.history.map((s) => s.id)).toEqual(['old']);
  resolve([{ ...entry, id: 'new' }, entry]);
  off = await connecting;
  expect(performanceStore.history.map((s) => s.id)).toEqual(['new', 'old']);
});

test('service_state_updates_the_system_info_live', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  off = await performanceStore.connect(backend);
  expect(performanceStore.system?.serviceConnected).toBe(true);
  backend.emitServiceStatus({ state: 'unreachable', detail: null, pawnIo: null, sources: null });
  expect(performanceStore.system?.serviceConnected).toBe(false);
  backend.emitServiceStatus({ state: 'connected', detail: null, pawnIo: null, sources: null });
  expect(performanceStore.system?.serviceConnected).toBe(true);
  // Disconnected, the store no longer listens.
  off();
  off = undefined;
  backend.emitServiceStatus({ state: 'unreachable', detail: null, pawnIo: null, sources: null });
  expect(performanceStore.system).toBeNull();
});
