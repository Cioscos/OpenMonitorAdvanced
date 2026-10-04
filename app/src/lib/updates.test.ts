import { MOCK_SCHEMA } from './backend/mock';
import type { UpdateStatus } from './types';
import { updates } from './updates.svelte';
import { FakeBackend } from '../test/fake-backend';

function status(over: Partial<UpdateStatus> = {}): UpdateStatus {
  return { state: 'idle', current: '0.4.0', latest: null, checkedAtMs: null, error: null, ...over };
}

let off: (() => void) | undefined;

afterEach(() => {
  off?.();
  off = undefined;
});

/** A backend whose `checkUpdates` waits until the test answers it. */
async function setup() {
  const backend = new FakeBackend(MOCK_SCHEMA);
  let answer!: (s: UpdateStatus) => void;
  backend.checkUpdates = () =>
    new Promise<UpdateStatus>((resolve) => {
      answer = resolve;
    });
  off = await updates.connect(backend);
  return { backend, answer: (s: UpdateStatus) => answer(s) };
}

test('a check reply replaces the state when no event came meanwhile', async () => {
  const { answer } = await setup();
  const pending = updates.check();
  answer(status({ state: 'upToDate', checkedAtMs: 5 }));
  await pending;
  expect(updates.state).toEqual(status({ state: 'upToDate', checkedAtMs: 5 }));
});

test('a check reply older than a status event is skipped', async () => {
  const { backend, answer } = await setup();
  const pending = updates.check();
  const newer = status({ state: 'available', latest: { version: '0.5.0' }, checkedAtMs: 9 });
  backend.emitUpdateStatus(newer);
  answer(status({ state: 'upToDate', checkedAtMs: 5 }));
  await pending;
  expect(updates.state).toEqual(newer);
});
