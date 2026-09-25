import { FakeBackend } from '../test/fake-backend';
import { MOCK_SCHEMA, mockValues } from './backend/mock';
import { LiveStore, connect } from './live.svelte';

const snapshot = (seq: number, revision = 1) => ({
  revision,
  seq,
  timestampMs: 1000 * seq,
  values: mockValues(seq),
});

test('applySnapshot updates values, timestamps and series', () => {
  const store = new LiveStore(3);
  store.applySchema(MOCK_SCHEMA);
  expect(store.applySnapshot(snapshot(1))).toBe(true);
  expect(store.applySnapshot(snapshot(2))).toBe(true);
  expect(store.value('cpu/0/load/total')).toBe(mockValues(2)[0]);
  expect(store.series('cpu/0/load/total')).toEqual([mockValues(1)[0], mockValues(2)[0]]);
  expect(store.firstTimestampMs).toBe(1000);
  expect(store.timestampMs).toBe(2000);
});

test('rejects snapshot of another revision', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  expect(store.applySnapshot(snapshot(1, 2))).toBe(false);
  expect(store.value('cpu/0/load/total')).toBeNull();
});

test('unknown sensors read as null and empty series', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  expect(store.value('nope')).toBeNull();
  expect(store.series('nope')).toEqual([]);
});

test('connect seeds sparklines from history', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.history = { timestampsMs: [500, 1500], series: [[10, 20]] };
  const store = new LiveStore(5);
  const off = await connect(store, backend);
  expect(store.series('cpu/0/load/total')).toEqual([10, 20]);
  expect(store.firstTimestampMs).toBe(500);
  off();
});

test('late history keeps snapshots received while the request was pending', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  let resolve!: (h: import('./types').HistorySeed) => void;
  backend.getHistory = () => new Promise((done) => { resolve = done; });
  const store = new LiveStore();
  const connecting = connect(store, backend);
  await vi.waitFor(() => expect(resolve).toBeDefined());
  backend.emitSnapshot(snapshot(2));
  resolve({ revision: 1, seq: 1, timestampsMs: [1000], series: MOCK_SCHEMA.sensors.map((_, i) => [mockValues(1)[i]]) });
  const off = await connecting;
  expect(store.series('cpu/0/load/total')).toEqual([mockValues(1)[0], mockValues(2)[0]]);
  expect(store.timestampMs).toBe(2000);
  backend.emitSnapshot(snapshot(1));
  expect(store.timestampMs).toBe(2000);
  off();
});

test('failed connection unsubscribes and does not mutate the store', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.getHistory = async () => { throw new Error('offline'); };
  const store = new LiveStore();
  await expect(connect(store, backend)).rejects.toThrow('offline');
  backend.emitSchema(MOCK_SCHEMA);
  backend.emitSnapshot(snapshot(1));
  expect(store.schema).toBeNull();
});

test('connect refetches schema on revision mismatch', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  const off = await connect(store, backend);
  expect(backend.schemaCalls).toBe(1);
  backend.schema = { ...MOCK_SCHEMA, revision: 7 };
  backend.emitSnapshot(snapshot(1, 7));
  await vi.waitFor(() => expect(backend.schemaCalls).toBe(2));
  off();
});

test('connect_retries_refresh_after_transient_failure', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  const off = await connect(store, backend);
  expect(store.schema?.revision).toBe(1);

  const newSchema = { ...MOCK_SCHEMA, revision: 2 };
  let failNext = true;
  const originalGetSchema = backend.getSchema.bind(backend);
  backend.getSchema = async () => {
    if (failNext) {
      failNext = false;
      throw new Error('transient');
    }
    return originalGetSchema();
  };

  const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  backend.schema = newSchema;
  backend.emitSnapshot(snapshot(1, 2));
  // First attempt fails; the store stays on the old revision and keeps its subscriptions.
  await vi.waitFor(() => expect(errorSpy).toHaveBeenCalledWith('backend refresh failed', expect.any(Error)));
  expect(store.schema?.revision).toBe(1);

  // A later event for the same (still pending) revision must trigger a fresh retry.
  backend.emitSnapshot(snapshot(2, 2));
  await vi.waitFor(() => expect(store.schema?.revision).toBe(2));
  expect(store.applySnapshot(snapshot(3, 2))).toBe(true);

  errorSpy.mockRestore();
  off();
});

test('applySnapshot records the local arrival time of new snapshots only', () => {
  const now = vi.spyOn(Date, 'now').mockReturnValue(50_000);
  try {
    const store = new LiveStore();
    store.applySchema(MOCK_SCHEMA);
    expect(store.lastReceivedAtMs).toBeNull();
    store.applySnapshot(snapshot(1));
    expect(store.lastReceivedAtMs).toBe(50_000);

    now.mockReturnValue(60_000);
    store.applySnapshot(snapshot(1)); // duplicate
    store.applySnapshot(snapshot(2, 9)); // other revision
    expect(store.lastReceivedAtMs).toBe(50_000);
    store.applySnapshot(snapshot(2));
    expect(store.lastReceivedAtMs).toBe(60_000);
  } finally {
    now.mockRestore();
  }
});
