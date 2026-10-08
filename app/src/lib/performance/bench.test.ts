import { MOCK_SCHEMA } from '../backend/mock';
import { FakeBackend, makeBenchStatus, makeDiskBenchStatus, makeDiskScoreFile, makeGpuBenchStatus, makeGpuScoreFile, makeScoreFile } from '../../test/fake-backend';
import type { BenchStatus } from '../types';
import { benchStore, type ScoreTarget } from './bench.svelte';

const CPU: ScoreTarget = { category: 'cpu' };

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
  expect(benchStore.provisionalFor(CPU)).toBe(true);
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
  expect(benchStore.recordFor(CPU)).toMatchObject({ single: 1600, multi: 13000 });
  expect(benchStore.lastFor(CPU)).toMatchObject({ single: 1400, multi: 13000 });
  backend.scoreFiles = [];
  await benchStore.refresh();
  expect(benchStore.recordFor(CPU)).toMatchObject({ single: null, multi: null });
  expect(benchStore.lastFor(CPU)).toMatchObject({ single: null, multi: null });
});

test('provisional_scores_stay_out_of_record_and_last_once_calibrated', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.scoreFiles = [
    makeScoreFile({ id: 'old', provisional: true, scores: { single: 9000, multi: 90000 } }),
    makeScoreFile({ id: 'b', scores: { single: 1400, multi: 13000 } }),
  ];
  off = await benchStore.connect(backend);
  expect(benchStore.recordFor(CPU)).toMatchObject({ single: 1400, multi: 13000 });
  backend.scoreFiles = [backend.scoreFiles[0]];
  await benchStore.refresh();
  expect(benchStore.lastFor(CPU)).toMatchObject({ single: null, multi: null });
  // While the scale itself is provisional, provisional scores are the only ones there are.
  off();
  backend.baselineProvisional = true;
  off = await benchStore.connect(backend);
  expect(benchStore.recordFor(CPU)).toMatchObject({ single: 9000, multi: 90000 });
  expect(benchStore.lastFor(CPU)).toMatchObject({ single: 9000, multi: 90000 });
});

test('scores_and_record_are_per_target', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.baselineGpuProvisional = true;
  backend.scoreFiles = [
    makeGpuScoreFile('gpu-a', { id: 'ga2', provisional: true, scores: { compute: 1400, graphics: 1600 } }),
    makeGpuScoreFile('gpu-b', { id: 'gb', provisional: true, scores: { compute: 9, graphics: 8 } }),
    makeScoreFile({ id: 'c', scores: { single: 1500, multi: 12000 } }),
    makeGpuScoreFile('gpu-a', { id: 'ga1', provisional: true, scores: { compute: 1500, graphics: 1450 } }),
    makeGpuScoreFile('gpu-a', { id: 'bad', valid: false, flags: ['device_lost'], scores: { compute: 9000, graphics: null } }),
  ];
  off = await benchStore.connect(backend);
  const A: ScoreTarget = { category: 'gpu', deviceId: 'gpu-a' };
  const B: ScoreTarget = { category: 'gpu', deviceId: 'gpu-b' };
  expect(benchStore.scoresFor(CPU).map((s) => s.id)).toEqual(['c']);
  expect(benchStore.scoresFor(A).map((s) => s.id)).toEqual(['ga2', 'ga1', 'bad']);
  expect(benchStore.scoresFor({ category: 'gpu', deviceId: 'gone' })).toEqual([]);
  expect(benchStore.recordFor(A)).toMatchObject({ compute: 1500, graphics: 1600 });
  expect(benchStore.lastFor(A)).toMatchObject({ compute: 1400, graphics: 1600 });
  expect(benchStore.recordFor(B)).toMatchObject({ compute: 9, graphics: 8 });
  expect(benchStore.recordFor(CPU)).toMatchObject({ single: 1500, multi: 12000, compute: null, graphics: null });
  expect(benchStore.provisionalFor(CPU)).toBe(false);
  expect(benchStore.provisionalFor(A)).toBe(true);
  // The status belongs to the target it runs on only; `running` is any benchmark.
  backend.emitBench(makeGpuBenchStatus('gpu-a'));
  expect(benchStore.statusFor(A)?.deviceId).toBe('gpu-a');
  expect(benchStore.statusFor(B)).toBeNull();
  expect(benchStore.statusFor(CPU)).toBeNull();
  expect(benchStore.running).toBe(true);
  backend.emitBench(makeBenchStatus());
  expect(benchStore.statusFor(CPU)?.category).toBe('cpu');
  expect(benchStore.statusFor(A)).toBeNull();
  // Start goes to the command of the target.
  await benchStore.start(A);
  await benchStore.start(CPU);
  expect(backend.performanceCalls).toEqual(expect.arrayContaining(['performanceGpuBenchStart:gpu-a', 'performanceBenchStart']));
});

test('disk scores are shared by the disks, the record and the last one are per device', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.baselineDiskProvisional = true;
  backend.scoreFiles = [
    makeDiskScoreFile('disk-e', { id: 'e', provisional: true, scores: { readMBs: 400, writeMBs: 120 } }),
    makeDiskScoreFile('disk-c', { id: 'c2', provisional: true, scores: { readMBs: 6000, writeMBs: 6100 } }),
    makeDiskScoreFile('disk-c', { id: 'bad', valid: false, flags: ['io_error'], scores: { readMBs: 9999, writeMBs: null } }),
    makeDiskScoreFile('disk-c', { id: 'c1', provisional: true, scores: { readMBs: 6900, writeMBs: 5800 } }),
    makeScoreFile({ id: 'cpu', scores: { single: 1500, multi: 12000 } }),
  ];
  off = await benchStore.connect(backend);
  const all: ScoreTarget = { category: 'disk' };
  const C: ScoreTarget = { category: 'disk', deviceId: 'disk-c' };
  const E: ScoreTarget = { category: 'disk', deviceId: 'disk-e' };
  expect(benchStore.provisionalFor(all)).toBe(true);
  expect(benchStore.scoresFor(all).map((s) => s.id)).toEqual(['e', 'c2', 'bad', 'c1']);
  expect(benchStore.scoresFor(CPU).map((s) => s.id)).toEqual(['cpu']);
  expect(benchStore.recordFor(C)).toMatchObject({ read: 6900, write: 6100 });
  expect(benchStore.lastFor(C)).toMatchObject({ read: 6000, write: 6100 });
  expect(benchStore.recordFor(E)).toMatchObject({ read: 400, write: 120 });
  // A volume that is not one recognised disk has no record to compare with; no id compares every disk.
  expect(benchStore.recordFor({ category: 'disk', deviceId: null })).toMatchObject({ read: null, write: null });
  expect(benchStore.recordFor(all)).toMatchObject({ read: 6900, write: 6100 });
  expect(benchStore.recordFor(CPU)).toMatchObject({ read: null, write: null });
  // The status of a disk benchmark belongs to the disk pages only.
  backend.emitBench(makeDiskBenchStatus('disk-c'));
  expect(benchStore.statusFor(all)?.category).toBe('disk');
  expect(benchStore.statusFor(CPU)).toBeNull();
  expect(benchStore.statusFor({ category: 'gpu', deviceId: 'disk-c' })).toBeNull();
  // Start goes to the disk command, with its request.
  const request = { folder: 'C:\\Temp', profile: 'b1' as const, compressible: false, wake: false };
  await benchStore.start(all, request);
  expect(backend.diskStartRequests).toEqual([request]);
  await expect(benchStore.start(all)).rejects.toBe('noTarget');
});
