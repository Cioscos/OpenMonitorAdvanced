import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { ErrorRecord, StressSession } from '../../lib/types';
import { FakeBackend, makeStressSession, makeSystemInfo } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import PerformanceView from './PerformanceView.svelte';

const error = (over: Partial<ErrorRecord> = {}): ErrorRecord => ({
  phase: 1,
  kernel: 'k2',
  isa: 'avx2',
  kind: 'mismatch',
  logical: 4,
  core: 2,
  iteration: 17,
  expected: 1,
  actual: 2,
  seed: 3,
  atMs: 134_000,
  tempC: 71,
  clockMhz: 5100,
  ...over,
});

/** An overclock test that found errors on core 2 only, in its second phase (one core at a time). */
const UNSTABLE = (): StressSession => {
  const first = error();
  const s = makeStressSession({
    id: 'bad',
    objective: 'overclock',
    request: { component: 'cpu', objective: 'overclock', preset: 'standard', custom: null, retryCore: null },
    outcome: 'errors',
    outcomeDetail: { verdict: 'errors_core', params: { core: '2' }, phase: 1, kernel: 'k2', core: 2, tempC: 75, clockMhz: 5100, atMs: 200_000 },
    cores: [
      { core: 0, state: 'passed', firstError: null },
      { core: 1, state: 'passed', firstError: null },
      { core: 2, state: 'failed', firstError: first },
      { core: 3, state: 'untested', firstError: null },
    ],
    errors: [first, error({ atMs: 140_000, iteration: 18 })],
    errorsDropped: 5,
    whea: { byId: { '19': 2 }, byApic: { '4': 2 }, unreadable: false, lastRecord: 9 },
    events: [{ atMs: 150_000, code: 'whea', params: { id: '19', apic: '4', core: '2' } }],
  });
  const cycle = { ...s.plan.phases[0], kernel: 'k2' as const, placement: 'core_cycle' as const, per_core_s: 60 };
  return { ...s, plan: { ...s.plan, phases: [s.plan.phases[0], cycle] } };
};

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

async function setup(session: StressSession) {
  const backend: FakeBackend = await connectSettings();
  backend.performanceSystemInfo = makeSystemInfo();
  backend.performanceSessionsById[session.id] = session;
  render(PerformanceView, { backend, store: new LiveStore(), page: `result:${session.id}` });
  await screen.findByRole('button', { name: t('performance.result.export') });
  return { backend };
}

const fact = (name: string) => screen.getByText(name, { selector: '.facts dt, .facts dt *' }).closest('div')?.querySelector('dd')?.textContent;

test('result_shows_unstable_core_with_advice_and_retry', async () => {
  const { backend } = await setup(UNSTABLE());
  expect(screen.getByRole('heading', { name: t('performance.outcome.errors_core', { core: 2 }) })).toBeTruthy();
  // Where and when: the first error of the core, not the end of the test.
  expect(fact(t('glossary.phase.name'))).toMatch(/^2 · /);
  expect(fact(t('glossary.phase.name'))).toContain(t('glossary.mode.k2.name'));
  expect(fact(t('performance.result.fact.core'))).toBe('2');
  expect(fact(t('performance.result.fact.time'))).toBe('00:02:14');
  expect(fact(t('glossary.clock.name'))).toBe('5.10 GHz');
  expect(fact(t('performance.result.fact.temp'))).toBe('71 °C');
  expect(fact(t('glossary.iteration.name'))).toBe('17');
  const sentences = [...document.querySelectorAll('.verdict p')].map((p) => p.textContent);
  expect(sentences).toEqual([t('performance.result.kind.mismatch'), t('performance.advice.core')]);
  expect([...document.querySelectorAll('.verdict p .term')].map((e) => e.textContent?.toLowerCase())).toEqual(
    [t('glossary.reference.name'), t('glossary.curveOptimizer.name')].map((name) => name.toLowerCase()),
  );
  // The digests never show (they lose precision in JS).
  expect(document.body.textContent).not.toContain('expected');
  // WHEA by APIC with its core, from the session's own events.
  expect(screen.getByText(t('performance.result.wheaApicCore', { apic: 4, core: 2, n: 2 }))).toBeTruthy();
  // WHEA by ID says whether the hardware corrected it, with the term.
  const byIdText = `WHEA ${t('performance.result.wheaId', { id: 19, kind: t('performance.result.wheaKind.corrected'), n: 2 })}`;
  const byId = screen.getByText((_, node) => node?.tagName === 'LI' && node.textContent === byIdText);
  expect(byId.querySelector('.term')?.textContent).toBe('WHEA');
  // Phase counts agree in number.
  expect(screen.getByText(t('performance.result.phaseCount.passed.one'), { exact: false })).toBeTruthy();
  // Cores and the error list with the dropped ones counted.
  const cores = screen.getByRole('list', { name: t('performance.run.cores') });
  expect(within(cores).getAllByRole('listitem')[3].textContent).toContain(t('performance.core.untestedFinal'));
  expect(screen.getByText(t('performance.result.errorsDropped', { n: 5 }))).toBeTruthy();

  await fireEvent.click(screen.getByRole('button', { name: t('performance.result.retryCore', { core: 2 }) }));
  await waitFor(() => expect(backend.performanceStartRequests).toHaveLength(1));
  expect(backend.performanceStartRequests[0]).toEqual({
    component: 'cpu',
    objective: 'overclock',
    preset: 'standard',
    custom: null,
    retryCore: { core: 2, kernel: 'k2' },
  });
});

test('result_without_single_core_has_no_retry', async () => {
  const session = UNSTABLE();
  session.outcomeDetail = { ...session.outcomeDetail!, verdict: 'errors', params: {}, core: null };
  const { backend } = await setup(session);
  expect(screen.getByRole('heading', { name: t('performance.outcome.errors') })).toBeTruthy();
  expect(screen.queryByRole('button', { name: /core/i })).toBeNull();
  expect(screen.queryByText(t('performance.advice.core'))).toBeNull();
  // «Repeat» sends the session's own request.
  await fireEvent.click(screen.getByRole('button', { name: t('performance.result.repeat') }));
  await waitFor(() => expect(backend.performanceStartRequests).toEqual([session.request]));
});

test('result_without_core_cycle_has_no_core_grid', async () => {
  // A normal test loads every core at once: no core is tested on its own, so no «not tested» cells.
  await setup(makeStressSession({ cores: [0, 1, 2, 3].map((core) => ({ core, state: 'untested' as const, firstError: null })) }));
  expect(screen.queryByRole('list', { name: t('performance.run.cores') })).toBeNull();
});

test('phase_cut_by_a_stop_counts_as_stopped', async () => {
  await setup(
    makeStressSession({
      outcome: 'stopped_thermal',
      phases: [{ index: 0, kernel: 'k1', outcome: 'stopped', durationMs: 2000, checks: 10, errors: 0, skipped: null }],
    }),
  );
  const counts = screen.getByText(t('performance.result.phaseCount.stopped.one'), { exact: false }).textContent;
  expect(counts).toContain(t('performance.result.phaseCount.passed', { n: 0 }));
});

test('export_calls_the_backend', async () => {
  const { backend } = await setup(makeStressSession({ id: 'ok' }));
  await fireEvent.click(screen.getByRole('button', { name: t('performance.result.export') }));
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceExport:ok'));
  await screen.findByText(t('performance.result.exported', { name: 'oma-stress-20261006-090507.json' }));
});

test('system_crash_result_shows_the_phase', async () => {
  await setup(
    makeStressSession({
      id: 'crash',
      outcome: 'system_crash',
      outcomeDetail: { verdict: 'system_crash', params: { phase: '3' }, phase: 2, kernel: 'k5', core: null, tempC: null, clockMhz: null, atMs: null },
      events: [{ atMs: 900_000, code: 'bugcheck', params: { record: '77' } }],
    }),
  );
  const phase = t('performance.result.phaseN', { n: '3' });
  expect(screen.getByRole('heading', { name: t('performance.outcome.system_crash', { phase }) })).toBeTruthy();
  expect(fact(t('glossary.phase.name'))).toContain(t('glossary.mode.k5.name'));
  const log = screen.getByRole('list', { name: t('performance.run.events') });
  expect(log.textContent).toContain(t('performance.event.bugcheck'));
});

test('crash_facts_come_from_the_journal_not_an_earlier_error', async () => {
  await setup(
    makeStressSession({
      id: 'crash2',
      outcome: 'system_crash',
      outcomeDetail: { verdict: 'system_crash', params: { phase: '3' }, phase: 2, kernel: 'k5', core: 5, tempC: null, clockMhz: null, atMs: null },
      errors: [error({ phase: 0, kernel: 'k1', core: 1 })],
    }),
  );
  expect(fact(t('glossary.phase.name'))).toContain('3');
  expect(fact(t('glossary.phase.name'))).toContain(t('glossary.mode.k5.name'));
  expect(fact(t('performance.result.fact.core'))).toBe('5');
  expect(screen.queryByText(t('performance.result.fact.time'), { selector: '.facts dt' })).toBeNull();
  expect(screen.queryByText(t('glossary.iteration.name'), { selector: '.facts dt *' })).toBeNull();
  expect(document.querySelector('.verdict p')).toBeNull();
});
