import { MOCK_SCHEMA } from '../backend/mock';
import type { SensorStats, StatsReply } from '../types';
import { FakeBackend } from '../../test/fake-backend';
import { StatsPoller } from './statsPoller.svelte';

const A = 'cpu/0/load/total';
const B = 'cpu/0/clock/effective';
const STATS: SensorStats = { min: 1, max: 9, avg: 5, count: 3 };

let visibility: DocumentVisibilityState = 'visible';
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
const setVisibility = (state: DocumentVisibilityState) => {
  visibility = state;
  document.dispatchEvent(new Event('visibilitychange'));
};

function setup(ids = [A, B]) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const getStats = vi.fn(async (req: string[]): Promise<StatsReply> => ({ revision: 1, stats: req.map((id) => (id === A ? STATS : null)) }));
  const resetStats = vi.fn(async () => {});
  backend.getStats = getStats;
  backend.resetStats = resetStats;
  let revision = 1;
  return { poller: new StatsPoller(backend, () => ids, () => revision), getStats, resetStats, setRevision: (value: number) => { revision = value; } };
}

beforeEach(() => {
  visibility = 'visible';
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

test('polls at start and then every second', async () => {
  const { poller, getStats } = setup();
  const stop = poller.start();
  await vi.advanceTimersByTimeAsync(0);
  expect(getStats).toHaveBeenCalledTimes(1);
  expect(getStats).toHaveBeenCalledWith([A, B]);
  expect(poller.statsOf(A)).toEqual(STATS);
  expect(poller.statsOf(B)).toBeNull();
  await vi.advanceTimersByTimeAsync(3000);
  expect(getStats).toHaveBeenCalledTimes(4);
  stop();
  await vi.advanceTimersByTimeAsync(5000);
  expect(getStats).toHaveBeenCalledTimes(4);
});

test('stops polling while hidden and polls at once when visible again', async () => {
  const { poller, getStats } = setup();
  const stop = poller.start();
  await vi.advanceTimersByTimeAsync(0);
  setVisibility('hidden');
  await vi.advanceTimersByTimeAsync(5000);
  expect(getStats).toHaveBeenCalledTimes(1);
  setVisibility('visible');
  await vi.advanceTimersByTimeAsync(0);
  expect(getStats).toHaveBeenCalledTimes(2);
  stop();
});

test('a slow reply does not pile up requests', async () => {
  const { poller, getStats } = setup();
  let release!: () => void;
  getStats.mockImplementationOnce(
    (req) => new Promise((done) => (release = () => done({ revision: 1, stats: req.map(() => null) }))),
  );
  const stop = poller.start();
  await vi.advanceTimersByTimeAsync(3000);
  expect(getStats).toHaveBeenCalledTimes(1);
  release();
  await vi.advanceTimersByTimeAsync(1000);
  expect(getStats).toHaveBeenCalledTimes(2);
  stop();
});

test('reset clears the page sensors in the core and reads them again', async () => {
  const { poller, getStats, resetStats } = setup();
  await poller.poll();
  expect(poller.statsOf(A)).toEqual(STATS);
  getStats.mockImplementationOnce(async (req) => ({ revision: 1, stats: req.map(() => null) }));
  await poller.reset();
  expect(resetStats).toHaveBeenCalledWith([A, B]);
  expect(getStats).toHaveBeenCalledTimes(2);
  expect(poller.statsOf(A)).toBeNull();
});

test('a reply that started before a reset is dropped', async () => {
  const { poller, getStats } = setup();
  let release!: () => void;
  getStats.mockImplementationOnce(
    (req) => new Promise((done) => (release = () => done({ revision: 1, stats: req.map(() => STATS) }))),
  );
  const stale = poller.poll();
  getStats.mockImplementationOnce(async (req) => ({ revision: 1, stats: req.map(() => null) }));
  await poller.reset();
  release();
  await stale;
  expect(poller.statsOf(A)).toBeNull();
});

test('errors are logged and polling goes on', async () => {
  const { poller, getStats, resetStats } = setup();
  const error = vi.spyOn(console, 'error').mockImplementation(() => {});
  getStats.mockRejectedValueOnce(new Error('offline'));
  resetStats.mockRejectedValueOnce(new Error('offline'));
  await poller.poll();
  await poller.reset();
  expect(error).toHaveBeenCalledWith('sensor statistics unavailable', expect.any(Error));
  expect(error).toHaveBeenCalledWith('cannot reset the sensor statistics', expect.any(Error));
  await poller.poll();
  expect(poller.statsOf(A)).toEqual(STATS);
  error.mockRestore();
});

test('a page without sensors asks nothing', async () => {
  const { poller, getStats } = setup([]);
  await poller.poll();
  expect(getStats).not.toHaveBeenCalled();
});

test('cached statistics disappear immediately when the schema changes', async () => {
  const { poller, setRevision } = setup();
  await poller.poll();
  expect(poller.statsOf(A)).toEqual(STATS);
  setRevision(2);
  expect(poller.statsOf(A)).toBeNull();
});

test('a late reply from the previous schema is dropped and the new schema retries', async () => {
  const { poller, getStats, setRevision } = setup();
  let release!: () => void;
  getStats.mockImplementationOnce(
    (req) => new Promise((done) => (release = () => done({ revision: 1, stats: req.map(() => STATS) }))),
  );
  const old = poller.poll();
  setRevision(2);
  release();
  await old;
  expect(poller.statsOf(A)).toBeNull();
  getStats.mockImplementationOnce(async (req) => ({ revision: 2, stats: req.map(() => STATS) }));
  await poller.poll();
  expect(poller.statsOf(A)).toEqual(STATS);
});

test('a reply newer than the displayed schema is not published', async () => {
  const { poller, getStats } = setup();
  getStats.mockImplementationOnce(async (req) => ({ revision: 2, stats: req.map(() => STATS) }));
  await poller.poll();
  expect(poller.statsOf(A)).toBeNull();
});
