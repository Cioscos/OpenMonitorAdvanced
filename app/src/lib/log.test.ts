import { MOCK_SCHEMA } from './backend/mock';
import { FakeBackend, makeLogStatus } from '../test/fake-backend';
import { canDo, log } from './log.svelte';
import type { LogStatus } from './types';

const status = (over: Partial<LogStatus> = {}) => makeLogStatus(over);

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

test('subscribes_before_reading', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.logStatus = status({ revision: 4, state: 'recording', session: 1 });
  off = await log.connect(backend);
  expect(backend.logCalls).toEqual(['onLogStatus', 'getLogStatus']);
  expect(log.status?.revision).toBe(4);
});

test('older_sessions_are_ignored', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.logStatus = status({ revision: 10, session: 2, state: 'recording' });
  off = await log.connect(backend);
  backend.emitLogStatus(status({ revision: 7, session: 1, state: 'error' }));
  expect(log.status?.revision).toBe(10);
  expect(log.status?.state).toBe('recording');
  backend.emitLogStatus(status({ revision: 11, session: 2, state: 'recording', rows: 3 }));
  expect(log.status?.rows).toBe(3);
  // The same revision is not newer either.
  backend.emitLogStatus(status({ revision: 11, session: 2, state: 'recording', rows: 99 }));
  expect(log.status?.rows).toBe(3);
});

test('late_getter_does_not_overwrite_paused_in_same_session', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const read = deferred<LogStatus>();
  vi.spyOn(backend, 'getLogStatus').mockReturnValue(read.promise);
  const connecting = log.connect(backend);
  await vi.waitFor(() => expect(backend.logListenerCount).toBe(1));
  backend.emitLogStatus(status({ revision: 9, session: 3, state: 'paused' }));
  read.resolve(status({ revision: 8, session: 3, state: 'recording' }));
  off = await connecting;
  expect(log.status?.state).toBe('paused');
  expect(log.status?.revision).toBe(9);
});

test('late_command_reply_does_not_overwrite_error', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.logStatus = status({ revision: 5, session: 1, state: 'recording' });
  off = await log.connect(backend);
  const reply = deferred<LogStatus>();
  vi.spyOn(backend, 'logPause').mockReturnValue(reply.promise);
  const pausing = log.pause();
  expect(log.busy).toBe(true);
  backend.emitLogStatus(status({ revision: 7, session: 1, state: 'error', error: { key: 'log.error.diskFull', detail: null } }));
  reply.resolve(status({ revision: 6, session: 1, state: 'paused' }));
  await pausing;
  expect(log.status?.state).toBe('error');
  expect(log.busy).toBe(false);
});

test('disconnected_callbacks_are_ignored', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.logStatus = status({ revision: 1 });
  const first = await log.connect(backend);
  first();
  first();
  expect(backend.logListenerCount).toBe(0);
  backend.emitLogStatus(status({ revision: 60, state: 'recording' }));
  expect(log.status).toBeNull();

  // A getter that answers after a reconnect belongs to the old connection and is dropped.
  const late = deferred<LogStatus>();
  vi.spyOn(backend, 'getLogStatus').mockReturnValueOnce(late.promise);
  const superseded = log.connect(backend);
  await vi.waitFor(() => expect(backend.logListenerCount).toBe(1));
  backend.logStatus = status({ revision: 70, state: 'paused' });
  off = await log.connect(backend);
  late.resolve(status({ revision: 99, state: 'recording' }));
  (await superseded)();
  expect(log.status?.revision).toBe(70);
  expect(log.status?.state).toBe('paused');
});

test('commands_update_the_status_and_clear_busy', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  off = await log.connect(backend);
  backend.logStatus = status({ revision: 1, state: 'recording', session: 1 });
  await log.start();
  expect(log.status?.state).toBe('recording');
  backend.logStatus = status({ revision: 2, state: 'paused', session: 1 });
  await log.pause();
  expect(log.status?.state).toBe('paused');
  backend.logStatus = status({ revision: 3, state: 'recording', session: 1 });
  await log.resume();
  expect(log.status?.state).toBe('recording');
  backend.logStatus = status({ revision: 4, state: 'idle', session: 1 });
  await log.stop();
  expect(log.status?.state).toBe('idle');
  expect(backend.logCalls.slice(2)).toEqual(['logStart', 'logPause', 'logResume', 'logStop']);
  expect(log.busy).toBe(false);

  backend.logError = 'boom';
  await expect(log.start()).rejects.toThrow('boom');
  expect(log.busy).toBe(false);
});

test('busy_blocks_a_second_command', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  off = await log.connect(backend);
  const reply = deferred<LogStatus>();
  vi.spyOn(backend, 'logStart').mockReturnValue(reply.promise);
  const starting = log.start();
  await log.stop();
  expect(backend.logCalls).not.toContain('logStop');
  reply.resolve(status({ revision: 1, state: 'recording' }));
  await starting;
});

test('can_do_by_state', () => {
  expect(canDo('idle')).toEqual({ rec: true, pause: false, stop: false });
  expect(canDo('recording')).toEqual({ rec: false, pause: true, stop: true });
  expect(canDo('paused')).toEqual({ rec: true, pause: false, stop: true });
  expect(canDo('error')).toEqual({ rec: true, pause: false, stop: false });
});
