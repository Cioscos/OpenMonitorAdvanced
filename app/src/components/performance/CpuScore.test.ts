import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { CpuScoreFile } from '../../lib/types';
import { FakeBackend, makeBenchStatus, makeRunStatus, makeScoreFile } from '../../test/fake-backend';
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

async function setup(scores: CpuScoreFile[] = [], provisional = false) {
  const backend: FakeBackend = await connectSettings();
  backend.scoreFiles = structuredClone(scores);
  backend.baselineProvisional = provisional;
  render(PerformanceView, { backend, store: new LiveStore(), page: 'score-cpu' });
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceScores'));
  await screen.findByRole('heading', { name: t('performance.score.title') });
  return backend;
}
const meter = (key: 'single' | 'multi') => screen.getByRole('meter', { name: t(`performance.score.${key}`) });
const now = (key: 'single' | 'multi') => meter(key).getAttribute('aria-valuenow');
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
  // A value that is not a number does not reach the gauge.
  backend.emitBench(makeBenchStatus({ step: 25, livePoints: NaN, single: 1500 }));
  await waitFor(() => expect(now('multi')).toBeNull());
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
