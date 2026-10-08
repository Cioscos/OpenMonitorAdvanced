import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { ErrorRecord, StressSession } from '../../lib/types';
import { performanceStore } from '../../lib/performance/performance.svelte';
import { FakeBackend, makeStressSession, makeSystemInfo, makeVolume } from '../../test/fake-backend';
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

const GPU_REQUEST = { component: 'gpu' as const, objective: 'overclock' as const, preset: 'standard' as const, custom: null, retryCore: null, gpu: 'gpu/pci-0000:01:00.0' };
const gpuSession = (over: Partial<StressSession>): StressSession =>
  makeStressSession({ id: 'gpu', component: 'gpu', device: 'Fake RTX 4080', objective: 'overclock', request: GPU_REQUEST, ...over });

test('device lost result shows the driver code', async () => {
  // As the RunController writes it: the error record (GPU clock, load level) and its event.
  const lost = error({ kind: 'device_lost', kernel: 's1', isa: 'sse2', phase: 0, core: null, logical: null, iteration: 4, expected: 0, actual: 0x887a0006, load_percent: 65, clockMhz: 2850 });
  await setup(
    gpuSession({
      outcome: 'device_lost',
      outcomeDetail: { verdict: 'device_lost', params: { code: '0x887A0006' }, phase: 0, kernel: 's1', core: null, tempC: 70, clockMhz: 2850, atMs: 134_000 },
      errors: [lost],
      events: [{ atMs: 134_000, code: 'device_lost', params: { phase: '0', code: '0x887A0006' } }],
    }),
  );
  expect(screen.getByText(t('glossary.deviceLost.name'), { selector: '.log .term, .log .term *' })).toBeTruthy();
  expect(screen.getByRole('heading', { name: t('performance.outcome.device_lost') })).toBeTruthy();
  const code = screen.getByText(t('performance.result.deviceLostCode', { code: 'DXGI_ERROR_DEVICE_HUNG (0x887A0006)' }));
  expect(code.closest('.term')).toBeTruthy();
  // The load level and the clock at that moment.
  expect(fact(t('glossary.loadLevel.name'))).toBe('65 %');
  expect(fact(t('glossary.clock.name'))).toBe('2.85 GHz');
});

test('device lost without an error has no driver code', async () => {
  await setup(gpuSession({ outcome: 'device_lost', outcomeDetail: { verdict: 'device_lost', params: {}, phase: 0, kernel: 's1', core: null, tempC: null, clockMhz: null, atMs: 9000 } }));
  expect(screen.getByRole('heading', { name: t('performance.outcome.device_lost') })).toBeTruthy();
  expect(screen.queryByText(t('performance.result.deviceLostCode', { code: '' }), { exact: false })).toBeNull();
});

test('low stability result shows the percent', async () => {
  await setup(
    gpuSession({
      outcome: 'low_stability',
      outcomeDetail: { verdict: 'low_stability', params: { stability: '95.3' }, phase: null, kernel: null, core: null, tempC: null, clockMhz: null, atMs: null },
      stability: 0.9532,
    }),
  );
  expect(screen.getByRole('heading', { name: t('performance.outcome.low_stability', { stability: '95.3' }) })).toBeTruthy();
  const line = screen.getByText(t('performance.result.stability', { stability: '95.3' }));
  expect(line.closest('.term')).toBeTruthy();
});

test('gpu result has no retry-core action', async () => {
  const first = error({ kernel: 's6', isa: 'sse2', phase: 0, core: 2 });
  const { backend } = await setup(
    gpuSession({
      outcome: 'errors',
      outcomeDetail: { verdict: 'errors_core', params: { core: '2' }, phase: 0, kernel: 's6', core: 2, tempC: 70, clockMhz: 2850, atMs: 134_000 },
      cores: [{ core: 2, state: 'failed', firstError: first }],
      errors: [first],
    }),
  );
  expect(screen.queryByRole('button', { name: t('performance.result.retryCore', { core: 2 }) })).toBeNull();
  // «Repeat» still sends the GPU request, device id included.
  await fireEvent.click(screen.getByRole('button', { name: t('performance.result.repeat') }));
  await waitFor(() => expect(backend.performanceStartRequests).toEqual([GPU_REQUEST]));
});

test('a GPU result shows no instruction set', async () => {
  const lost = error({ kind: 'device_lost', kernel: 's1', isa: 'sse2', phase: 0, core: null, actual: 0x887a0005 });
  await setup(
    gpuSession({
      outcome: 'device_lost',
      outcomeDetail: { verdict: 'device_lost', params: {}, phase: 0, kernel: 's1', core: null, tempC: null, clockMhz: null, atMs: 1000 },
      errors: [lost],
    }),
  );
  expect(fact(t('glossary.phase.name'))).toContain(t('glossary.mode.s1.name'));
  expect(fact(t('glossary.phase.name'))).not.toContain(t('glossary.isa.sse2.name'));
  // Nor does the errors table.
  await fireEvent.click(screen.getByText(t('performance.result.errors', { n: 1 })));
  const table = await screen.findByRole('table');
  expect(table.textContent).not.toContain(t('glossary.isa.sse2.name'));
});

test('low stability title is in the locale format', async () => {
  await setup(
    gpuSession({
      outcome: 'low_stability',
      outcomeDetail: { verdict: 'low_stability', params: { stability: '95.3' }, phase: null, kernel: null, core: null, tempC: null, clockMhz: null, atMs: null },
      stability: 0.9532,
    }),
  );
  // The settings set the language at connection: switch after.
  i18n.locale = 'it';
  expect(await screen.findByRole('heading', { name: t('performance.outcome.low_stability', { stability: '95,3' }) })).toBeTruthy();
});

test('a GPU result labels the board power, not the CPU package', async () => {
  await setup(gpuSession({}));
  const labels = [...document.querySelectorAll('dt')].map((d) => d.textContent);
  expect(labels).toContain(t('performance.run.power'));
  expect(labels).not.toContain(t('glossary.packagePower.name'));
});

test('a fact without readings shows one dash, not max and average dashes', async () => {
  const s = gpuSession({});
  await setup({ ...s, stats: { ...s.stats, powerMaxW: null, powerAvgW: null } });
  const dd = [...document.querySelectorAll('dt')].find((d) => d.textContent === t('performance.run.power'))!.nextElementSibling!;
  expect(dd.textContent).toBe('—');
});

// --- Disk (M8c) ---

const GIB = 1024 ** 3;
const diskError = (over: Partial<ErrorRecord> = {}): ErrorRecord => ({
  phase: 1,
  kernel: 'v1',
  isa: 'sse2',
  kind: 'bit_flip',
  logical: null,
  core: null,
  iteration: 3,
  expected: 0,
  actual: 5,
  seed: 3,
  atMs: 90_000,
  tempC: 52,
  clockMhz: null,
  transient: true,
  ...over,
});
const diskPhase = (kernel: 'disk_fill' | 'n2' | 'v1') => ({ ...makeStressSession().plan.phases[0], kernel, isa: 'sse2' as const });

/** A data stability test that found a flipped bit that read right the second time, and a misplaced block that stayed wrong. */
const DISK = (over: Partial<StressSession> = {}): StressSession =>
  makeStressSession({
    id: 'disk',
    component: 'disk',
    device: 'Fake NVMe SSD',
    objective: 'overclock',
    request: { component: 'disk', objective: 'overclock', preset: 'standard', custom: null, retryCore: null, disk: { folder: '', wake: false } },
    plan: { seed: 1, ram_bytes: 0, disk: { dir: '', file_bytes: 8 * GIB, compressible: false, reserve_bytes: GIB }, phases: [diskPhase('disk_fill'), diskPhase('v1')] },
    outcome: 'errors',
    outcomeDetail: { verdict: 'errors', params: {}, phase: 1, kernel: 'v1', core: null, tempC: 52, clockMhz: null, atMs: 90_000 },
    errors: [diskError(), diskError({ kind: 'misplaced', iteration: 262_144, expected: 7, actual: 9, transient: false, atMs: 95_000 }), diskError({ kind: 'io_error', iteration: 0, actual: 23, transient: null })],
    disk: { deviceId: 'disk/a', volume: 'C:', kind: 'nvme', fileBytes: 8 * GIB, readBytes: 40 * GIB, writtenBytes: 24 * GIB, hostWrittenBeforeGib: 1000, hostWrittenAfterGib: 1012.5, slc: null },
    ...over,
  });

test('disk errors show kind offset and transient', async () => {
  await setup(DISK());
  expect(screen.getByRole('heading', { name: t('performance.outcome.errors') })).toBeTruthy();
  // The first error: its kind and its place in the file (block 3 x 4096 bytes), and that a re-read was right.
  const lead = t('performance.result.dataErrorAt', { kind: t('performance.result.error.bit_flip', { bits: 5 }), offset: '12 KiB' });
  const sentences = [...document.querySelectorAll('.verdict p')].map((p) => p.textContent);
  expect(sentences).toContain(`${lead} · ${t('performance.result.transient')}`);
  expect([...document.querySelectorAll('.verdict p .term')].map((e) => e.textContent)).toEqual([t('performance.result.error.bit_flip', { bits: 5 })]);
  // No core, no instruction set, no retry of a core.
  expect(screen.queryByText(t('performance.result.fact.core'))).toBeNull();
  expect(screen.queryByRole('button', { name: /core/i })).toBeNull();
  await fireEvent.click(screen.getByText(t('performance.result.errors', { n: 3 })));
  await screen.findByRole('table');
  const rows = [...document.querySelectorAll('tbody tr')].map((r) => r.textContent ?? '');
  expect(rows).toHaveLength(3);
  expect(rows[0]).toContain(t('performance.result.error.bit_flip', { bits: 5 }));
  expect(rows[0]).toContain(t('performance.result.transient'));
  // Block 262 144 is 1 GiB in.
  expect(rows[1]).toContain(t('performance.result.error.misplaced'));
  expect(rows[1]).toContain('1.0 GiB');
  expect(rows[1]).toContain(t('performance.result.persistent'));
  // A read or write error shows the Win32 code; whether a re-read helps is not known.
  expect(rows[2]).toContain(t('performance.result.error.io_error', { code: 23 }));
  expect(rows[2]).not.toContain(t('performance.result.persistent'));
  expect(rows[2]).not.toContain(t('performance.result.transient'));
});

test('slc result and thermal suspect', async () => {
  const slc = { cacheBytes: 60 * GIB, steadyBps: 400_000_000, thermalSuspect: true };
  await setup(DISK({ id: 'slc', outcome: 'passed', outcomeDetail: { verdict: 'passed', params: {}, phase: 1, kernel: 'n2', core: null, tempC: 52, clockMhz: null, atMs: 1 }, errors: [], plan: { ...DISK().plan, phases: [diskPhase('disk_fill'), diskPhase('n2')] }, disk: { ...DISK().disk!, slc } }));
  const line = screen.getByText((_, node) => node?.tagName === 'P' && !!node.textContent?.startsWith(t('performance.result.slc', { size: '60.0 GiB', speed: '400 MB/s' })));
  expect(line.querySelector('.term')?.textContent).toBe(t('glossary.slcCache.name'));
  expect(screen.getByText(t('performance.result.slcThermal'))).toBeTruthy();
});

test('no slc fall is said when the sustained write ran without one', async () => {
  await setup(DISK({ id: 'no-slc', outcome: 'passed', errors: [], phases: [{ index: 1, kernel: 'n2', outcome: 'passed', durationMs: 1, checks: 1, errors: 0, skipped: null }], plan: { ...DISK().plan, phases: [diskPhase('disk_fill'), diskPhase('n2')] } }));
  expect(screen.getByText(t('performance.result.slcNone'))).toBeTruthy();
  expect(screen.queryByText(t('performance.result.slcThermal'))).toBeNull();
});

test('smart written is shown', async () => {
  await setup(DISK({ id: 'smart' }));
  const line = screen.getByText((_, node) => node?.tagName === 'DD' && node.textContent === t('performance.result.smartWritten', { size: '12.5 GiB' }));
  expect(line.querySelector('.term')?.textContent).toBe('SMART');
  cleanup();
  // Without the counter, nothing is claimed.
  await setup(DISK({ id: 'smart-none', disk: { ...DISK().disk!, hostWrittenBeforeGib: null } }));
  expect(screen.queryByText(t('performance.result.smartWritten', { size: '12.5 GiB' }))).toBeNull();
});

test('stopped_disk_full verdict', async () => {
  await setup(DISK({ id: 'full', outcome: 'stopped_disk_full', outcomeDetail: { verdict: 'stopped_disk_full', params: {}, phase: 1, kernel: 'v1', core: null, tempC: 50, clockMhz: null, atMs: 60_000 }, errors: [] }));
  const heading = screen.getByRole('heading', { name: t('performance.outcome.stopped_disk_full') });
  expect(heading.closest('.verdict')?.classList.contains('warn')).toBe(true);
});

test('repeating a disk test finds the folder of its volume again', async () => {
  const { backend } = await setup(DISK({ id: 'again' }));
  performanceStore.system = makeSystemInfo({ volumes: [makeVolume({ root: 'C:\\', folder: 'C:\\Users\\me\\Temp' })] });
  await fireEvent.click(screen.getByRole('button', { name: t('performance.result.repeat') }));
  await waitFor(() => expect(backend.performanceStartRequests).toHaveLength(1));
  expect(backend.performanceStartRequests[0]).toMatchObject({ component: 'disk', disk: { folder: 'C:\\Users\\me\\Temp', wake: false } });
});
