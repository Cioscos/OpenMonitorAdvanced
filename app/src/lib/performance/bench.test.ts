import { MOCK_SCHEMA } from '../backend/mock';
import { FakeBackend, makeBenchStatus, makeScoreFile } from '../../test/fake-backend';
import type { BenchStatus } from '../types';
import { benchStore } from './bench.svelte';

let off: (() => void) | undefined;
afterEach(() => {
  off?.();
  off = undefined;
});

test('connect_subscribes_before_reading', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.benchStatusValue = makeBenchStatus({ scoreId: 'a' });
  backend.baselineProvisional = true;
  off = await benchStore.connect(backend);
  expect(backend.performanceCalls[0]).toBe('onPerformanceBench');
  expect(backend.performanceCalls.slice(1).sort()).toEqual(['performanceBaseline', 'performanceBenchStatus', 'performanceScores']);
  expect(benchStore.status?.scoreId).toBe('a');
  expect(benchStore.running).toBe(true);
  expect(benchStore.provisional).toBe(true);
  // An event that came before the read's reply is not overwritten by it.
  off();
  let resolve!: (status: BenchStatus | null) => void;
  vi.spyOn(backend, 'performanceBenchStatus').mockReturnValue(new Promise((r) => (resolve = r)));
  const connecting = benchStore.connect(backend);
  await vi.waitFor(() => expect(backend.benchListeners.size).toBe(1));
  backend.emitBench(makeBenchStatus({ state: 'done', scoreId: 'b' }));
  resolve(makeBenchStatus({ state: 'running' }));
  off = await connecting;
  expect(benchStore.status?.state).toBe('done');
  expect(benchStore.running).toBe(false);
});

test('record_ignores_invalid_scores', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.scoreFiles = [
    makeScoreFile({ id: 'bad', valid: false, flags: ['compute_error'], scores: { single: 9000, multi: 90000 } }),
    makeScoreFile({ id: 'b', scores: { single: 1400, multi: 13000 } }),
    makeScoreFile({ id: 'c', scores: { single: 1600, multi: null } }),
  ];
  off = await benchStore.connect(backend);
  expect(benchStore.record).toEqual({ single: 1600, multi: 13000 });
  expect(benchStore.last).toEqual({ single: 1400, multi: 13000 });
  backend.scoreFiles = [];
  await benchStore.refresh();
  expect(benchStore.record).toEqual({ single: null, multi: null });
  expect(benchStore.last).toEqual({ single: null, multi: null });
});
