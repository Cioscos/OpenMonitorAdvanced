import { MOCK_SCHEMA } from '../lib/backend/mock';
import { FakeBackend } from './fake-backend';

test('fake history returns the last `seconds` samples and records the call', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.history = { timestampsMs: [1000, 2000, 3000], series: [[1, 2, 3], [4, 5, 6]] };
  const h = await backend.getHistory(['a', 'b', 'c'], 2, 900);
  expect(h.timestampsMs).toEqual([2000, 3000]);
  expect(h.series).toEqual([[2, 3], [5, 6], []]);
  expect(backend.historyCalls).toEqual([{ ids: ['a', 'b', 'c'], seconds: 2, maxPoints: 900 }]);
  expect((await backend.getHistory(['a'], 60)).series).toEqual([[1, 2, 3]]);
  expect(backend.historyCalls[1].maxPoints).toBeUndefined();
});

test('fake stats are settable, recorded and cleared by a reset', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.stats = { a: { min: 1, max: 3, avg: 2, count: 3 } };
  expect(await backend.getStats(['a', 'b'])).toEqual({ revision: 1, stats: [{ min: 1, max: 3, avg: 2, count: 3 }, null] });
  await backend.resetStats(['a']);
  expect((await backend.getStats(['a'])).stats).toEqual([null]);
  expect(backend.statsCalls).toEqual([['a', 'b'], ['a']]);
  expect(backend.resetCalls).toEqual([['a']]);
});

test('fake session and gpu processes are settable', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  expect(await backend.getSession()).toEqual({ startedAtMs: null, intervalMs: 1000 });
  backend.session = { startedAtMs: 5, intervalMs: 2000 };
  expect(await backend.getSession()).toEqual({ startedAtMs: 5, intervalMs: 2000 });
  const row = { pid: 1, name: 'x.exe', loadPercent: 5, engine: '3D', dedicatedBytes: 1, sharedBytes: 2 };
  backend.gpuProcesses = [row];
  expect(await backend.getGpuProcesses('gpu/x')).toEqual([row]);
  expect(backend.gpuProcessCalls).toEqual(['gpu/x']);
});
