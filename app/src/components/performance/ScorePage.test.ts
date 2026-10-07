import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { ScoreFile } from '../../lib/types';
import {
  FakeBackend,
  makeBenchStatus,
  makeGpuBenchStatus,
  makeGpuScoreFile,
  makeRunStatus,
  makeScoreFile,
  makeSystemInfo,
} from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import PerformanceView from './PerformanceView.svelte';

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

async function setup(scores: ScoreFile[] = [], provisional = false) {
  const backend: FakeBackend = await connectSettings();
  backend.scoreFiles = structuredClone(scores);
  backend.baselineProvisional = provisional;
  render(PerformanceView, { backend, store: new LiveStore(), page: 'score-cpu' });
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceScores'));
  await screen.findByRole('heading', { name: t('performance.score.title') });
  return backend;
}
const meter = (key: 'single' | 'multi' | 'compute' | 'graphics') => screen.getByRole('meter', { name: t(`performance.score.${key}`) });
const now = (key: 'single' | 'multi' | 'compute' | 'graphics') => meter(key).getAttribute('aria-valuenow');
const startButton = () => screen.getByRole('button', { name: t('performance.score.start') });

test('start_runs_and_shows_both_scores', async () => {
  const backend = await setup();
  await fireEvent.click(startButton());
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceBenchStart'));
  backend.emitBench(makeBenchStatus({ state: 'running' }));
  // While it runs: «Stop», and the sidebar entry carries the dot.
  const stop = await screen.findByRole('button', { name: t('performance.score.stop') });
  const entry = screen.getByRole('button', { name: t('performance.nav.scoreCpu') });
  expect(entry.classList.contains('live')).toBe(true);
  expect(entry.textContent).toContain('●');
  await fireEvent.click(stop);
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceBenchStop'));
  // Done: both scores, the workload table in true units and the scaling.
  backend.scoreFiles = [makeScoreFile({ id: 'score-a' })];
  backend.emitBench(makeBenchStatus({ state: 'done', step: null, single: 1500, multi: 12000, scoreId: 'score-a' }));
  await waitFor(() => expect(now('single')).toBe('1500'));
  expect(now('multi')).toBe('12000');
  expect(entry.textContent).not.toContain('●');
  const table = await screen.findByRole('table', { name: t('performance.score.detail') });
  const ntt = within(table).getByText(t('glossary.bench.ntt.name')).closest('tr')!;
  expect(ntt.textContent).toContain('950');
  expect(ntt.textContent).toContain('Mop/s');
  const [before] = t('performance.score.scaling', { pct: 50 }).split(':');
  expect(screen.getByText(new RegExp(`${before}`)).closest('p')?.textContent).toBe(t('performance.score.scaling', { pct: 50 }));
});

test('live_gauge_moves_with_status', async () => {
  const backend = await setup();
  backend.emitBench(makeBenchStatus({ step: 1, livePoints: 1234.4 }));
  await waitFor(() => expect(now('single')).toBe('1234'));
  expect(now('multi')).toBeNull();
  // The phase in words: the workload, the mode and the repetition.
  expect(screen.getByText(new RegExp(t('performance.score.rep', { n: 1 })))).toBeTruthy();
  // Between phases the rate is missing: the needle holds its last value instead of falling to 0.
  backend.emitBench(makeBenchStatus({ step: 2, livePoints: null }));
  await waitFor(() => expect(screen.getByText(new RegExp(t('performance.score.rep', { n: 2 })))).toBeTruthy());
  expect(now('single')).toBe('1234');
  // A value above the full scale raises it; it never drops back during the measurement.
  backend.emitBench(makeBenchStatus({ step: 2, livePoints: 5000 }));
  await waitFor(() => expect(meter('single').getAttribute('aria-valuemax')).toBe('10000'));
  backend.emitBench(makeBenchStatus({ step: 3, livePoints: 1000 }));
  await waitFor(() => expect(now('single')).toBe('1000'));
  expect(meter('single').getAttribute('aria-valuemax')).toBe('10000');
  // The multi phases move the multi needle; the single score stays.
  backend.emitBench(makeBenchStatus({ step: 24, livePoints: 9000, single: 1500 }));
  await waitFor(() => expect(now('multi')).toBe('9000'));
  expect(now('single')).toBe('1500');
  // A value that is not a number does not reach the gauge: the needle holds.
  backend.emitBench(makeBenchStatus({ step: 25, livePoints: NaN, single: 1500 }));
  await waitFor(() => expect(screen.getByText(new RegExp(t('performance.score.rep', { n: 1 })))).toBeTruthy());
  expect(now('multi')).toBe('9000');
  // The run is over: the held needle goes; the done status shows the scores.
  backend.emitBench(makeBenchStatus({ state: 'done', step: null, livePoints: null, single: 1500, multi: 12000, scoreId: 'x' }));
  await waitFor(() => expect(now('multi')).toBe('12000'));
});

test('reference_menu_switches_between_record_and_last', async () => {
  await setup([
    makeScoreFile({ id: 'new', at: '2026-10-07T12:00:00Z', scores: { single: 1400, multi: 11000 } }),
    makeScoreFile({ id: 'old', at: '2026-10-06T12:00:00Z', scores: { single: 1600, multi: 13000 } }),
  ]);
  const menu = screen.getByRole('combobox', { name: t('performance.score.reference') }) as HTMLSelectElement;
  expect(menu.value).toBe('record');
  const marks = () => [...document.querySelectorAll('.mark-value')].map((m) => m.textContent?.trim());
  expect(marks()).toEqual(['▲ 1600', '▲ 13000']);
  await fireEvent.change(menu, { target: { value: 'last' } });
  expect(marks()).toEqual(['▲ 1400', '▲ 11000']);
});

test('invalid_score_shows_the_message', async () => {
  const backend = await setup([makeScoreFile({ id: 'bad', valid: false, flags: ['compute_error'], scores: { single: 1500, multi: null } })]);
  backend.emitBench(makeBenchStatus({ state: 'done', step: null, single: 1500, multi: null, flags: ['compute_error'], scoreId: 'bad' }));
  expect((await screen.findByRole('alert')).textContent).toContain(t('performance.score.invalid'));
});

test('flags_show_their_text', async () => {
  const flags = ['battery', 'thermal_throttle', 'busy_system', 'virtual_machine', 'no_sensors'];
  await setup([makeScoreFile({ flags, provisional: true })], true);
  for (const flag of flags) expect(await screen.findByText(t(`performance.score.flag.${flag}`))).toBeTruthy();
  expect(screen.getByText(t('performance.score.provisional'))).toBeTruthy();
  // The history marks the measurement with its warnings.
  const row = within(screen.getByRole('table', { name: t('performance.score.history') })).getAllByRole('row')[1];
  expect(row.textContent).toContain('!');
});

test('start_is_disabled_while_a_stress_test_runs', async () => {
  const backend = await setup();
  backend.emitPerformanceStatus(makeRunStatus({ state: 'running', sessionId: 'x' }));
  // The stress test brings its page forward; the user comes back to the score.
  await screen.findByRole('heading', { name: t('performance.run.title') });
  await fireEvent.click(screen.getByRole('button', { name: t('performance.nav.scoreCpu') }));
  await waitFor(() => expect((startButton() as HTMLButtonElement).disabled).toBe(true));
  expect(startButton().getAttribute('title')).toBe(t('performance.score.error.busy'));
  backend.emitPerformanceStatus(makeRunStatus({ state: 'finished', sessionId: 'x', outcome: 'passed' }));
  await waitFor(() => expect((startButton() as HTMLButtonElement).disabled).toBe(false));
  // The shell's refusals and a start that failed, in words.
  backend.benchStartError = 'busy';
  await fireEvent.click(startButton());
  expect((await screen.findByRole('alert')).textContent).toContain(t('performance.score.error.busy'));
  backend.emitBench(makeBenchStatus({ state: 'failed', step: null, error: 'performance.start.missing' }));
  await waitFor(() =>
    expect(screen.getByRole('alert').textContent).toContain(t('performance.score.error.start', { reason: t('performance.start.missing') })),
  );
  // A run that ended early is not a hardware verdict.
  for (const error of ['exited', 'failed', 'crashed', 'hung']) {
    backend.emitBench(makeBenchStatus({ state: 'failed', step: null, error }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toBe(t('performance.score.error.failed')));
  }
});

test('history_lists_and_deletes', async () => {
  const backend = await setup([
    makeScoreFile({ id: 'new', at: '2026-10-07T12:00:00Z', scores: { single: 1400, multi: 11000 } }),
    makeScoreFile({ id: 'old', at: '2026-10-06T12:00:00Z', scores: { single: 1600, multi: 13000 } }),
  ]);
  const table = screen.getByRole('table', { name: t('performance.score.history') });
  const rows = () => within(table).getAllByRole('row').slice(1);
  expect(rows()).toHaveLength(2);
  expect(rows()[0].textContent).toContain('1400');
  expect(rows()[1].textContent).toContain('13000');
  // Delete asks first.
  await fireEvent.click(within(rows()[1]).getByRole('button', { name: t('performance.score.delete') }));
  expect(backend.performanceCalls).not.toContain('performanceScoreDelete:old');
  await fireEvent.click(within(rows()[1]).getByRole('button', { name: t('performance.score.delete') }));
  await waitFor(() => expect(rows()).toHaveLength(1));
  expect(backend.performanceCalls).toContain('performanceScoreDelete:old');
});

test('empty_history_says_so', async () => {
  await setup();
  expect(screen.getByText(t('performance.score.history.empty'))).toBeTruthy();
});

test('table_headers_explain_single_and_multi_core', async () => {
  await setup([makeScoreFile({ id: 'score-a' })]);
  for (const name of [t('performance.score.detail'), t('performance.score.history')]) {
    const headers = within(screen.getByRole('table', { name })).getAllByRole('columnheader');
    for (const key of ['single', 'multi'] as const) {
      const th = headers.find((h) => h.textContent?.includes(t(`performance.score.${key}`)))!;
      expect(th.querySelector('.term'), `${name} ${key}`).not.toBeNull();
    }
  }
});

const GPU = { deviceId: 'gpu-a', name: 'Fake GeForce RTX 4080', integrated: false, dedicatedBytes: 16 * 1024 ** 3 };

async function setupGpu(scores: ScoreFile[] = [], gpus = [GPU], deviceId = GPU.deviceId) {
  const backend: FakeBackend = await connectSettings();
  backend.scoreFiles = structuredClone(scores);
  backend.performanceSystemInfo = makeSystemInfo({ gpus });
  render(PerformanceView, { backend, store: new LiveStore(), page: `score-gpu:${deviceId}` });
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceScores'));
  await screen.findByRole('heading', { name: t('performance.score.gpu.title') });
  return backend;
}

test('cpu page ignores gpu scores and status', async () => {
  const backend = await setup([
    makeGpuScoreFile('gpu-a', { id: 'g', at: '2026-10-08T12:00:00Z', scores: { compute: 7777, graphics: 8888 } }),
    makeScoreFile({ id: 'c', at: '2026-10-07T12:00:00Z', scores: { single: 1400, multi: 11000 } }),
  ]);
  const rows = within(screen.getByRole('table', { name: t('performance.score.history') })).getAllByRole('row').slice(1);
  expect(rows).toHaveLength(1);
  expect(rows[0].textContent).toContain('1400');
  // The newest score is a GPU one: the CPU page still shows its own.
  await waitFor(() => expect(now('single')).toBe('1400'));
  expect([...document.querySelectorAll('.mark-value')].map((m) => m.textContent?.trim())).toEqual(['▲ 1400', '▲ 11000']);
  // A GPU benchmark under way moves nothing here, and the CPU start stays refused.
  backend.emitBench(makeGpuBenchStatus('gpu-a', { livePoints: 5000 }));
  // It brings its own page forward; the user comes back to the CPU.
  await screen.findByRole('heading', { name: t('performance.score.gpu.title') });
  await fireEvent.click(screen.getByRole('button', { name: t('performance.nav.scoreCpu') }));
  await waitFor(() => expect((startButton() as HTMLButtonElement).disabled).toBe(true));
  expect(screen.queryByRole('button', { name: t('performance.score.stop') })).toBeNull();
  expect(now('single')).toBe('1400');
  expect(screen.queryByRole('meter', { name: t('performance.score.compute') })).toBeNull();
});

test('gpu page runs and shows compute and graphics', async () => {
  const backend = await setupGpu();
  expect(screen.getByText(GPU.name, { selector: '.device' })).toBeTruthy();
  expect(screen.getByText(t('performance.score.gpu.duration'))).toBeTruthy();
  await fireEvent.click(startButton());
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceGpuBenchStart:gpu-a'));
  backend.emitBench(makeGpuBenchStatus('gpu-a', { livePoints: 812.4 }));
  await waitFor(() => expect(now('compute')).toBe('812'));
  expect(now('graphics')).toBeNull();
  const phase = t('performance.score.gpu.phase', { kernel: t('glossary.gpuBench.fma.name'), group: t('performance.score.compute') });
  expect(document.querySelector('.phase')?.textContent).toBe(phase);
  // The sidebar dot is on this GPU's entry, not on the CPU's.
  const entry = screen.getByRole('button', { name: new RegExp(GPU.name) });
  expect(entry.textContent).toContain('●');
  expect(screen.getByRole('button', { name: t('performance.nav.scoreCpu') }).textContent).not.toContain('●');
  // A Graphics load moves the Graphics needle; Compute shows its score.
  backend.emitBench(makeGpuBenchStatus('gpu-a', { step: 4, livePoints: 1300, compute: 1500 }));
  await waitFor(() => expect(now('graphics')).toBe('1300'));
  expect(now('compute')).toBe('1500');
  await fireEvent.click(screen.getByRole('button', { name: t('performance.score.stop') }));
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceBenchStop'));
  backend.scoreFiles = [makeGpuScoreFile('gpu-a', { id: 'g1', scores: { compute: 1500, graphics: 1450 } })];
  backend.emitBench(makeGpuBenchStatus('gpu-a', { state: 'done', step: null, compute: 1500, graphics: 1450, scoreId: 'g1' }));
  await waitFor(() => expect(now('graphics')).toBe('1450'));
  expect(now('compute')).toBe('1500');
  await waitFor(() => expect(meter('compute').getAttribute('aria-valuemax')).toBe('2000'));
  const history = await screen.findByRole('table', { name: t('performance.score.history') });
  expect(within(history).getAllByRole('row')[1].textContent).toContain('1450');
});

test('gpu start without the gpu says it is missing', async () => {
  const backend = await setupGpu();
  backend.benchStartError = 'build:no_gpu';
  await fireEvent.click(startButton());
  expect((await screen.findByRole('alert')).textContent).toBe(t('performance.score.gpu.missing'));
});

test('gpu detail shows value and spread', async () => {
  await setupGpu([makeGpuScoreFile('gpu-a')]);
  const table = await screen.findByRole('table', { name: t('performance.score.detail') });
  const headers = within(table).getAllByRole('columnheader').map((h) => h.textContent);
  expect(headers).toEqual(expect.arrayContaining([t('performance.score.col.value'), t('performance.score.col.spread')]));
  const fma = within(table).getByText(t('glossary.gpuBench.fma.name')).closest('tr')!;
  expect(fma.textContent).toContain('47.2');
  expect(fma.textContent).toContain('TFLOPS');
  expect(fma.textContent).toContain('1.2%');
  expect(fma.querySelectorAll('.term').length).toBeGreaterThanOrEqual(2);
  const bandwidth = within(table).getByText(t('glossary.gpuBench.bandwidth.name')).closest('tr')!;
  expect(bandwidth.textContent).toContain('593');
  expect(bandwidth.textContent).toContain('GB/s');
  const fill = within(table).getByText(t('glossary.gpuBench.fill.name')).closest('tr')!;
  expect(fill.textContent).toContain('Gpixel/s');
  // No CPU columns on a GPU page.
  expect(headers.join(' ')).not.toContain(t('performance.score.single'));
});

test('gpu flags and invalid reasons show their text', async () => {
  const flags = ['throttling', 'busy_gpu', 'battery', 'vram_reduced'];
  await setupGpu([makeGpuScoreFile('gpu-a', { flags })]);
  for (const flag of flags) expect(await screen.findByText(t(`performance.score.flag.${flag}`))).toBeTruthy();
  for (const [flag, key] of [
    ['device_lost', 'performance.score.invalid.device_lost'],
    ['hung', 'performance.score.invalid.hung'],
    ['compute_error', 'performance.score.invalid'],
  ]) {
    cleanup();
    disconnectSettings();
    await setupGpu([makeGpuScoreFile('gpu-a', { valid: false, flags: [flag], scores: { graphics: null } })]);
    expect((await screen.findByRole('alert')).textContent).toBe(t(key));
    // The reason is the message, not a warning too.
    expect(screen.queryByText(t(`performance.score.flag.${flag}`))).toBeNull();
  }
});

test('missing gpu page disables start', async () => {
  await setupGpu([makeGpuScoreFile('gone', { id: 'old', scores: { compute: 321 } })], [GPU], 'gone');
  expect(screen.getByText(t('performance.score.gpu.missing'))).toBeTruthy();
  expect((startButton() as HTMLButtonElement).disabled).toBe(true);
  const rows = within(screen.getByRole('table', { name: t('performance.score.history') })).getAllByRole('row').slice(1);
  expect(rows).toHaveLength(1);
  expect(rows[0].textContent).toContain('321');
});
