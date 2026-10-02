import { FakeBackend } from '../test/fake-backend';
import { MOCK_SCHEMA, mockValues } from './backend/mock';
import { LiveStore, connect } from './live.svelte';

const snapshot = (seq: number, revision = 1) => ({
  revision,
  seq,
  timestampMs: 1000 * seq,
  values: mockValues(seq),
});

test('irregular snapshots retain their exact shared times', () => {
  const store = new LiveStore(3);
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot({ ...snapshot(1), timestampMs: 1_025 });
  store.applySnapshot({ ...snapshot(2), timestampMs: 3_700 });
  expect(store.seriesTimestampsMs()).toEqual([1_025, 3_700]);
  expect(store.series('cpu/0/load/total')).toHaveLength(2);
});

test('seedHistory aligns missing sensor readings with timestamps', () => {
  const store = new LiveStore(5);
  store.applySchema(MOCK_SCHEMA);
  const ids = MOCK_SCHEMA.sensors.map((sensor) => sensor.id);
  store.seedHistory(ids, {
    revision: 1,
    seq: 2,
    timestampsMs: [120, 480, 2_400],
    series: ids.map((_, i) => i === 0 ? [10, null, 30] : []),
  });
  expect(store.seriesTimestampsMs()).toEqual([120, 480, 2_400]);
  const cpu = store.series('cpu/0/load/total');
  expect(cpu[0]).toBe(10);
  expect(Number.isNaN(cpu[1])).toBe(true);
  expect(cpu[2]).toBe(30);
  expect(store.series(ids[1])).toHaveLength(3);
  expect(store.series(ids[1]).every(Number.isNaN)).toBe(true);
});

test('schema changes backfill new sensors to the shared timeline', () => {
  const store = new LiveStore(4);
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot(snapshot(1));
  store.applySnapshot(snapshot(2));
  const added = { ...MOCK_SCHEMA.sensors[0], id: 'new/sensor' };
  store.applySchema({ ...MOCK_SCHEMA, revision: 2, sensors: [...MOCK_SCHEMA.sensors, added] });
  expect(store.seriesTimestampsMs()).toEqual([1_000, 2_000]);
  expect(store.series(added.id)).toHaveLength(2);
  expect(store.series(added.id).every(Number.isNaN)).toBe(true);
  store.applySnapshot({ revision: 2, seq: 3, timestampMs: 2_700, values: [...mockValues(3), 99] });
  expect(store.seriesTimestampsMs()).toEqual([1_000, 2_000, 2_700]);
  expect(store.series(added.id).slice(2)).toEqual([99]);
});

test('schema replacement backfills sensors when none of the old ids remain', () => {
  const store = new LiveStore(4);
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot(snapshot(1));
  store.applySnapshot(snapshot(2));
  const replacement = { ...MOCK_SCHEMA.sensors[0], id: 'replacement/sensor' };
  store.applySchema({ ...MOCK_SCHEMA, revision: 2, sensors: [replacement] });
  expect(store.seriesTimestampsMs()).toEqual([1_000, 2_000]);
  expect(store.series(replacement.id)).toHaveLength(2);
  expect(store.series(replacement.id).every(Number.isNaN)).toBe(true);
});

test('timestamp rollback clears times and sensor samples together', () => {
  const store = new LiveStore(3);
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot(snapshot(1));
  store.applySnapshot(snapshot(2));
  store.applySnapshot({ ...snapshot(3), timestampMs: 500 });
  expect(store.seriesTimestampsMs()).toEqual([500]);
  expect(store.series('cpu/0/load/total')).toEqual([mockValues(3)[0]]);
});

test('capacity evicts the same oldest timestamp and sensor sample', () => {
  const store = new LiveStore(2);
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot({ ...snapshot(1), timestampMs: 1_025 });
  store.applySnapshot({ ...snapshot(2), timestampMs: 1_800 });
  store.applySnapshot({ ...snapshot(3), timestampMs: 4_100 });
  expect(store.seriesTimestampsMs()).toEqual([1_800, 4_100]);
  expect(store.series('cpu/0/load/total')).toEqual([mockValues(2)[0], mockValues(3)[0]]);
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

const withQuality = (seq: number, quality?: number[], revision = 1) => ({ ...snapshot(seq, revision), quality });
const sensorCount = MOCK_SCHEMA.sensors.length;
const ID = MOCK_SCHEMA.sensors[0].id;
const ID2 = MOCK_SCHEMA.sensors[1].id;

test('quality defaults to fresh when the payload has none', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  expect(store.quality(ID)).toBe(0);
  store.applySnapshot(snapshot(1));
  expect(store.quality(ID)).toBe(0);
  store.applySnapshot(withQuality(2, [1, 2]));
  expect(store.quality(ID)).toBe(0);
  store.applySnapshot(withQuality(3, Array.from({ length: sensorCount }, () => 7)));
  expect(store.quality(ID)).toBe(0);
  expect(store.quality('no/such/sensor')).toBe(0);
});

test('quality follows the snapshot', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  const codes = Array.from({ length: sensorCount }, () => 0);
  codes[0] = 1;
  codes[1] = 2;
  store.applySnapshot(withQuality(1, codes));
  expect(store.quality(ID)).toBe(1);
  expect(store.quality(ID2)).toBe(2);
  store.applySnapshot(snapshot(2));
  expect(store.quality(ID)).toBe(0);
  expect(store.quality(ID2)).toBe(0);
});

test('a rejected snapshot cannot overwrite quality', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  const held = Array.from({ length: sensorCount }, () => 1);
  store.applySnapshot(withQuality(5, held));
  const fresh = Array.from({ length: sensorCount }, () => 0);
  expect(store.applySnapshot(withQuality(6, fresh, 2))).toBe(false); // Other revision.
  expect(store.applySnapshot(withQuality(4, fresh))).toBe(true); // Out of order.
  expect(store.quality(ID)).toBe(1);
  expect(store.applySnapshot({ ...withQuality(7, fresh), values: [] })).toBe(false); // Wrong length.
  expect(store.quality(ID)).toBe(1);
});

test('quality resets when the schema changes or the history is seeded', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot(withQuality(1, Array.from({ length: sensorCount }, () => 2)));
  const ids = MOCK_SCHEMA.sensors.map((s) => s.id);
  store.seedHistory(ids, { revision: 1, seq: 2, timestampsMs: [10], series: ids.map(() => [1]) });
  expect(store.quality(ID)).toBe(0);
  store.applySnapshot(withQuality(3, Array.from({ length: sensorCount }, () => 2)));
  store.applySchema({ ...MOCK_SCHEMA, revision: 2 });
  expect(store.quality(ID)).toBe(0);
});

test('disk power comes from the backend and updates on the event', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.diskStates = [{ deviceId: 'storage/a', power: 'standby' }];
  const store = new LiveStore();
  const off = await connect(store, backend);
  expect(store.diskPower('storage/a')).toBe('standby');
  expect(store.diskPower('storage/b')).toBeUndefined();
  backend.emitDiskStates([
    { deviceId: 'storage/a', power: 'active' },
    { deviceId: 'storage/b', power: 'idle' },
  ]);
  expect(store.diskPower('storage/a')).toBe('active');
  expect(store.diskPower('storage/b')).toBe('idle');
  off();
});

test('an empty disk event clears old power states', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.diskStates = [{ deviceId: 'storage/a', power: 'standby' }];
  const store = new LiveStore();
  const off = await connect(store, backend);
  backend.emitDiskStates([]);
  expect(store.diskPower('storage/a')).toBeUndefined();
  off();
});

test('an event received during bootstrap wins over the initial disk query', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  let answer!: (states: import('./types').DiskStateEntry[]) => void;
  backend.getDiskStates = () => new Promise((done) => { answer = done; });
  const store = new LiveStore();
  const connecting = connect(store, backend);
  await vi.waitFor(() => expect(answer).toBeDefined());
  backend.emitDiskStates([{ deviceId: 'storage/a', power: 'active' }]);
  answer([{ deviceId: 'storage/a', power: 'standby' }]);
  const off = await connecting;
  expect(store.diskPower('storage/a')).toBe('active');
  off();
});

test('disconnect removes the disk listener', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  backend.diskStates = [{ deviceId: 'storage/a', power: 'standby' }];
  const off = await connect(store, backend);
  expect(backend.diskStateListeners).toBe(1);
  expect(store.diskPower('storage/a')).toBe('standby');
  off();
  expect(backend.diskStateListeners).toBe(0);
  expect(store.diskPower('storage/a')).toBeUndefined();
  backend.emitDiskStates([{ deviceId: 'storage/b', power: 'active' }]);
  expect(store.diskPower('storage/b')).toBeUndefined();
});

test('a failing initial disk query clears stale power states and still connects', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.getDiskStates = async () => { throw new Error('offline'); };
  const store = new LiveStore();
  store.setDiskStates([{ deviceId: 'storage/a', power: 'standby' }]);
  const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  const off = await connect(store, backend);
  expect(errorSpy).toHaveBeenCalledWith('disk states query failed', expect.any(Error));
  expect(store.diskPower('storage/a')).toBeUndefined();
  off();
  errorSpy.mockRestore();
});

/** Quality codes with the second sensor at `code`. */
const second = (code: number) => Array.from({ length: sensorCount }, (_, i) => (i === 1 ? code : 0));

test('a suspended value is not appended to the live series', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot(snapshot(1));
  store.applySnapshot(withQuality(2, second(2)));
  expect(store.seriesTimestampsMs()).toEqual([1_000, 2_000]);
  expect(store.series(ID2)[0]).toBe(mockValues(1)[1]);
  expect(Number.isNaN(store.series(ID2)[1])).toBe(true);
  expect(store.measured(ID2)).toBeNull();
  // Only the suspended sensor; a held value is a regular repeat and stays.
  expect(store.series(ID)).toEqual([mockValues(1)[0], mockValues(2)[0]]);
  expect(store.measured(ID)).toBe(mockValues(2)[0]);
  store.applySnapshot(withQuality(3, second(1)));
  expect(store.series(ID2)[2]).toBe(mockValues(3)[1]);
  expect(store.measured(ID2)).toBe(mockValues(3)[1]);
  // Codes that do not fit the schema read as fresh.
  store.applySnapshot(withQuality(4, [2, 2]));
  expect(store.series(ID2)[3]).toBe(mockValues(4)[1]);
  expect(store.measured('no/such/sensor')).toBeNull();
});

test('the current value of a suspended sensor stays the last reading', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot(snapshot(1));
  const suspended = withQuality(2, second(2));
  store.applySnapshot(suspended);
  expect(store.value(ID2)).toBe(suspended.values[1]);
  expect(store.values).toEqual(suspended.values);
  expect(store.quality(ID2)).toBe(2);
});

test('the series resumes at the next fresh value', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot(snapshot(1));
  store.applySnapshot(withQuality(2, second(2)));
  store.applySnapshot(withQuality(3, second(2)));
  store.applySnapshot(snapshot(4));
  const series = store.series(ID2);
  expect(series.map(Number.isNaN)).toEqual([false, true, true, false]);
  expect(series[3]).toBe(mockValues(4)[1]);
  expect(store.seriesTimestampsMs()).toEqual([1_000, 2_000, 3_000, 4_000]);
});
