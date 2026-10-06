import catalog from '../../../../testdata/performance/catalog.json';
import type {
  CoreState,
  ErrorRecord,
  Outcome,
  Phase,
  Plan,
  PhaseInfo,
  RunStatus,
  StartRequest,
  StressSession,
  StressSessionSummary,
  SystemInfo,
} from '../types';

// A fake stress test for `pnpm dev`: every test lasts 60 s on an 8-core CPU, whatever its preset.
// `?perf=error` starts one at load that finds an error on core 2 during «one core at a time»;
// `?perf=pass` starts one that passes. Without the parameter a test starts only from the UI, and
// it passes.

const CORES = 8;
const ERROR_CORE = 2;
const TEST_S = 60;
const GIB = 1024 ** 3;

type Scenario = 'pass' | 'error' | null;

export function parsePerfScenario(search: string): Scenario {
  const value = new URLSearchParams(search).get('perf');
  return value === 'pass' || value === 'error' ? value : null;
}

const presetSeconds = (r: StartRequest): number | undefined =>
  (catalog.presets as Record<string, Record<string, number>>)[`${r.component}.${r.objective}`]?.[r.preset];

const phase = (over: Partial<Phase> & Pick<Phase, 'kernel' | 'duration_s'>): Phase => ({
  alt_kernel: null,
  isa: 'avx2',
  size: 'auto',
  mode: 'steady',
  placement: 'all_logical',
  per_core_s: null,
  both_smt: false,
  cores: null,
  patterns: [],
  stop_on_error: false,
  ...over,
});

/** A quarter at full load, two thirds one core at a time, the rest with a variable load. */
function phasesFor(request: StartRequest, totalS: number): Phase[] {
  const stop = request.objective === 'overclock';
  if (request.component === 'ram') {
    return [
      phase({ kernel: 'k3', duration_s: totalS / 2, size: 'ram', stop_on_error: stop }),
      phase({ kernel: 'k10', duration_s: totalS / 2, isa: 'sse2', size: 'ram', patterns: [...catalog.patterns] as Phase['patterns'], stop_on_error: stop }),
    ];
  }
  const cycle = Math.round((totalS * 2) / 3);
  return [
    phase({ kernel: 'k1', duration_s: totalS / 4, stop_on_error: stop }),
    phase({ kernel: 'k2', duration_s: cycle, size: 'l2', placement: 'core_cycle', per_core_s: cycle / CORES, cores: [...Array(CORES).keys()], stop_on_error: stop }),
    phase({ kernel: 'k5', duration_s: totalS - totalS / 4 - cycle, mode: 'variable', stop_on_error: stop }),
  ];
}

const info = (p: Phase): PhaseInfo => ({ kernel: p.kernel, mode: p.mode, placement: p.placement, durationS: p.duration_s, isa: p.isa });

/** Where a run stands `elapsedMs` into its phases. */
function progress(phases: Phase[], elapsedMs: number) {
  let start = 0;
  for (const [index, p] of phases.entries()) {
    const end = start + p.duration_s * 1000;
    if (elapsedMs < end || index === phases.length - 1) {
      const current = p.placement === 'core_cycle' && p.per_core_s ? Math.min(CORES - 1, Math.floor((elapsedMs - start) / (p.per_core_s * 1000))) : null;
      return { index, current };
    }
    start = end;
  }
  return { index: 0, current: null };
}

/** The moment the error scenario fails: halfway through core 2's turn. */
function errorAtMs(phases: Phase[]): number {
  let start = 0;
  for (const p of phases) {
    if (p.placement === 'core_cycle' && p.per_core_s) return start + (ERROR_CORE + 0.5) * p.per_core_s * 1000;
    start += p.duration_s * 1000;
  }
  return Infinity;
}

const reading = (ms: number) => ({
  tempC: Math.round(78 + 6 * Math.sin(ms / 7000)),
  powerW: Math.round(118 + 10 * Math.sin(ms / 5000)),
  clockMhz: Math.round(4950 + 40 * Math.sin(ms / 3000)),
});

function coreStates(phases: Phase[], elapsedMs: number, failed: boolean, finished: boolean): CoreState[] {
  let start = 0;
  for (const p of phases) {
    if (p.placement === 'core_cycle' && p.per_core_s) {
      return [...Array(CORES).keys()].map((core) => {
        const from = start + core * p.per_core_s! * 1000;
        if (failed && core === ERROR_CORE) return 'failed';
        if (elapsedMs >= from + p.per_core_s! * 1000) return 'passed';
        return elapsedMs >= from && !finished ? 'testing' : 'untested';
      });
    }
    start += p.duration_s * 1000;
  }
  return [];
}

function errorRecord(phases: Phase[]): ErrorRecord {
  const atMs = errorAtMs(phases);
  return {
    phase: phases.findIndex((p) => p.placement === 'core_cycle'),
    kernel: 'k2',
    isa: 'avx2',
    kind: 'mismatch',
    logical: ERROR_CORE * 2,
    core: ERROR_CORE,
    iteration: 18_422,
    expected: 3_735_928_559,
    actual: 3_735_928_551,
    seed: 2_870_177_450,
    atMs,
    ...reading(atMs),
  };
}

const VERDICT: Partial<Record<Outcome, string>> = { errors: 'errors_core' };

function buildSession(id: string, startedAtMs: number, request: StartRequest, phases: Phase[], elapsedMs: number, outcome: Outcome): StressSession {
  const failed = outcome === 'errors';
  const error = failed ? errorRecord(phases) : null;
  const samples = Array.from({ length: Math.floor(elapsedMs / 5000) + 1 }, (_, i) => ({ tMs: i * 5000, ...reading(i * 5000) }));
  const avg = (key: 'tempC' | 'powerW' | 'clockMhz') => Math.round(samples.reduce((sum, s) => sum + s[key], 0) / samples.length);
  const max = (key: 'tempC' | 'powerW' | 'clockMhz') => Math.max(...samples.map((s) => s[key]));
  let start = 0;
  return {
    format: 1,
    id,
    startedAt: new Date(startedAtMs).toISOString(),
    endedAt: new Date(startedAtMs + elapsedMs).toISOString(),
    component: request.component,
    device: request.component === 'cpu' ? 'Mock Ryzen 7 7800X3D' : '32 GB RAM',
    objective: request.objective,
    preset: request.preset,
    request,
    plan: { seed: 2_870_177_450, ram_bytes: request.component === 'ram' ? 20 * GIB : 0, phases },
    outcome,
    outcomeDetail: {
      verdict: VERDICT[outcome] ?? outcome,
      params: failed ? { core: String(ERROR_CORE) } : {},
      phase: error?.phase ?? null,
      kernel: error?.kernel ?? null,
      core: error?.core ?? null,
      tempC: error?.tempC ?? null,
      clockMhz: error?.clockMhz ?? null,
      atMs: error?.atMs ?? null,
    },
    phases: phases.map((p, index) => {
      const from = start;
      start += p.duration_s * 1000;
      const ran = Math.max(0, Math.min(elapsedMs, start) - from);
      const errored = error !== null && error.phase === index;
      return {
        index,
        kernel: p.kernel,
        outcome: errored ? 'errors' : ran === 0 ? 'skipped' : ran < p.duration_s * 1000 ? 'stopped' : 'passed',
        durationMs: ran,
        checks: Math.floor(ran / 100),
        errors: errored ? 1 : 0,
        skipped: ran === 0 ? 'stopped' : null,
      };
    }),
    cores: coreStates(phases, elapsedMs, failed, true).map((state, core) => ({ core, state, firstError: failed && core === ERROR_CORE ? error : null })),
    errors: error ? [error] : [],
    errorsDropped: 0,
    eventsDropped: 0,
    whea: { byId: {}, byApic: {}, unreadable: false, lastRecord: null },
    stats: {
      tempMaxC: max('tempC'),
      tempAvgC: avg('tempC'),
      powerMaxW: max('powerW'),
      powerAvgW: avg('powerW'),
      clockMaxMhz: max('clockMhz'),
      clockAvgMhz: avg('clockMhz'),
    },
    samples,
    events: outcome === 'stopped_user' ? [{ atMs: elapsedMs, code: 'user_stop', params: {} }] : [],
    appVersion: '0.5.0',
    loadVersion: '0.5.0',
  };
}

const summary = (s: StressSession): StressSessionSummary => ({
  id: s.id,
  startedAt: s.startedAt,
  component: s.component,
  objective: s.objective,
  preset: s.preset,
  durationMs: s.phases.reduce((sum, p) => sum + p.durationMs, 0),
  outcome: s.outcome,
  verdict: s.outcomeDetail?.verdict ?? null,
  params: s.outcomeDetail?.params ?? {},
});

function seeded(): StressSession[] {
  const day = 24 * 3600 * 1000;
  const normal: StartRequest = { component: 'cpu', objective: 'normal', preset: 'quick', custom: null, retryCore: null };
  const overclock: StartRequest = { component: 'cpu', objective: 'overclock', preset: 'standard', custom: null, retryCore: null };
  const quickPhases = phasesFor(normal, presetSeconds(normal)!);
  const ocPhases = phasesFor(overclock, presetSeconds(overclock)!);
  return [
    buildSession('7d3a6c2e-1f0b-4c55-9a7e-2b1f9e8d4c01', Date.now() - day, normal, quickPhases, presetSeconds(normal)! * 1000, 'passed'),
    buildSession('a41f0e9b-58c3-4d2a-b6e1-0c9d7f3e2a10', Date.now() - 3 * day, overclock, ocPhases, errorAtMs(ocPhases), 'errors'),
  ];
}

const idle = (): RunStatus => ({
  state: 'idle',
  sessionId: '',
  component: 'cpu',
  objective: 'normal',
  preset: 'quick',
  elapsedMs: 0,
  totalMs: 0,
  phaseIndex: 0,
  phases: [],
  tempC: null,
  tempMaxC: null,
  stopC: null,
  powerW: null,
  clockMhz: null,
  checks: 0,
  errors: 0,
  wheaCorrected: 0,
  wheaFatal: 0,
  cores: [],
  currentCore: null,
  events: [],
  warnings: [],
  outcome: null,
});

export function mockPerformance(scenario: Scenario, serviceConnected: () => boolean) {
  const listeners = new Set<(status: RunStatus) => void>();
  let sessions = seeded();
  let status = idle();
  let run: { id: string; request: StartRequest; phases: Phase[]; startedAtMs: number; fails: boolean; stopAtMs: number | null; timer: ReturnType<typeof setInterval> } | null = null;

  const publish = (next: RunStatus) => {
    status = next;
    listeners.forEach((cb) => cb(structuredClone(status)));
  };
  const finish = (elapsedMs: number, outcome: Outcome) => {
    const r = run!;
    clearInterval(r.timer);
    run = null;
    sessions = [buildSession(r.id, r.startedAtMs, r.request, r.phases, elapsedMs, outcome), ...sessions];
    publish({ ...status, state: 'finished', elapsedMs, outcome, errors: outcome === 'errors' ? 1 : 0, cores: coreStates(r.phases, elapsedMs, outcome === 'errors', true).map((state, core) => ({ core, state })), currentCore: null });
  };
  const tick = () => {
    const r = run!;
    const elapsedMs = Math.min(Date.now() - r.startedAtMs, TEST_S * 1000);
    if (r.fails && elapsedMs >= errorAtMs(r.phases)) return finish(errorAtMs(r.phases), 'errors');
    if (r.stopAtMs !== null && Date.now() >= r.stopAtMs) return finish(elapsedMs, 'stopped_user');
    if (elapsedMs >= TEST_S * 1000) return finish(elapsedMs, 'passed');
    const at = progress(r.phases, elapsedMs);
    const now = reading(elapsedMs);
    publish({
      ...status,
      state: r.stopAtMs !== null ? 'stopping' : 'running',
      elapsedMs,
      phaseIndex: at.index,
      ...now,
      tempMaxC: Math.max(status.tempMaxC ?? 0, now.tempC),
      checks: Math.floor(elapsedMs / 100),
      cores: coreStates(r.phases, elapsedMs, false, false).map((state, core) => ({ core, state })),
      currentCore: at.current,
    });
  };

  const api = {
    system: (): SystemInfo => ({
      cpuModel: 'Mock Ryzen 7 7800X3D',
      logical: CORES * 2,
      cores: CORES,
      isa: ['avx512', 'avx2', 'sse2'],
      ramTotal: 32 * GIB,
      ramBudget: 20 * GIB,
      serviceConnected: serviceConnected(),
      tjmaxC: 89,
      stopC: 84,
      hypervisor: false,
    }),
    preview(request: StartRequest): Plan {
      const total = presetSeconds(request);
      if (total === undefined) throw `the ${request.preset} preset does not exist for this test`;
      return { seed: 2_870_177_450, ram_bytes: request.component === 'ram' ? 20 * GIB : 0, phases: phasesFor(request, total) };
    },
    start(request: StartRequest): string {
      if (run !== null) throw 'a stress test is already running';
      const id = crypto.randomUUID();
      const phases = phasesFor(request, TEST_S);
      run = { id, request, phases, startedAtMs: Date.now(), fails: scenario === 'error', stopAtMs: null, timer: setInterval(tick, 1000) };
      publish({
        ...idle(),
        state: 'starting',
        sessionId: id,
        component: request.component,
        objective: request.objective,
        preset: request.preset,
        totalMs: TEST_S * 1000,
        phases: phases.map(info),
        stopC: 84,
        cores: coreStates(phases, 0, false, false).map((state, core) => ({ core, state })),
        warnings: serviceConnected() ? [] : ['noService', 'tempMissing'],
      });
      return id;
    },
    stop() {
      if (run === null || run.stopAtMs !== null) return;
      run.stopAtMs = Date.now() + 500;
      publish({ ...status, state: 'stopping' });
    },
    status: () => structuredClone(status),
    history: () => sessions.map(summary),
    session: (id: string) => structuredClone(sessions.find((s) => s.id === id) ?? null),
    remove(id: string) {
      if (run?.id === id) throw 'the session is still running';
      sessions = sessions.filter((s) => s.id !== id);
    },
    subscribe(cb: (status: RunStatus) => void) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
  if (scenario !== null) api.start({ component: 'cpu', objective: 'overclock', preset: 'standard', custom: null, retryCore: null });
  return api;
}
