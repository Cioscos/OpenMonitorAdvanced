import catalog from '../../../../testdata/performance/catalog.json';
import type {
  BenchKernel,
  BenchStatus,
  BenchStep,
  CoreState,
  CpuScoreFile,
  CpuScoreSummary,
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
// it passes. `?perf=gpu` starts a GPU test that passes, `?perf=device_lost` one whose GPU resets
// 85% in (DXGI_ERROR_DEVICE_HUNG, during a ramp) and `?perf=low_stability` one that ends at 95.3%.

const CORES = 8;
const ERROR_CORE = 2;
const TEST_S = 60;
const GIB = 1024 ** 3;

type Scenario = 'pass' | 'error' | 'gpu' | 'device_lost' | 'low_stability' | null;
const SCENARIOS = ['pass', 'error', 'gpu', 'device_lost', 'low_stability'];
const MOCK_GPU = 'gpu-mock-dedicated';
const LOW_STABILITY = 0.9532;

export function parsePerfScenario(search: string): Scenario {
  const value = new URLSearchParams(search).get('perf');
  return SCENARIOS.includes(value ?? '') ? (value as Scenario) : null;
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
  if (request.component === 'gpu') {
    return [
      phase({ kernel: 's5', alt_kernel: 's1', duration_s: Math.round(totalS * 0.7), isa: 'sse2', stop_on_error: stop }),
      phase({ kernel: 's1', mode: 'ramp', duration_s: totalS - Math.round(totalS * 0.7), isa: 'sse2', stop_on_error: stop }),
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
  // A GPU test has no cores: its failure comes 85% in, inside the closing ramp.
  if (phases[0]?.kernel.startsWith('s')) return phases.reduce((sum, p) => sum + p.duration_s * 1000, 0) * 0.85;
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

/** The load level of a GPU `ramp` phase at `elapsedMs` (20% to 100% in steps of 5), else null. */
function loadAt(phases: Phase[], elapsedMs: number): number | null {
  const at = progress(phases, elapsedMs).index;
  if (phases[at]?.mode !== 'ramp') return null;
  const from = phases.slice(0, at).reduce((sum, p) => sum + p.duration_s * 1000, 0);
  const share = Math.min(1, Math.max(0, (elapsedMs - from) / (phases[at].duration_s * 1000)));
  return 20 + 5 * Math.min(16, Math.floor(share * 17));
}

function errorRecord(phases: Phase[], lost = false): ErrorRecord {
  const atMs = errorAtMs(phases);
  if (lost) {
    return {
      phase: phases.findIndex((p) => p.mode === 'ramp'),
      kernel: 's1',
      isa: 'sse2',
      kind: 'device_lost',
      logical: null,
      core: null,
      iteration: 4_120,
      expected: 0,
      actual: 0x887a0006,
      seed: 2_870_177_450,
      atMs,
      load_percent: loadAt(phases, atMs),
      ...reading(atMs),
    };
  }
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
const isGpu = (request: StartRequest) => request.component === 'gpu';

function buildSession(id: string, startedAtMs: number, request: StartRequest, phases: Phase[], elapsedMs: number, outcome: Outcome): StressSession {
  const lost = outcome === 'device_lost';
  const failed = outcome === 'errors' || lost;
  const error = failed ? errorRecord(phases, lost) : null;
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
    device: request.component === 'cpu' ? 'Mock Ryzen 7 7800X3D' : isGpu(request) ? 'Mock GeForce RTX 4080' : '32 GB RAM',
    objective: request.objective,
    preset: request.preset,
    request,
    plan: { seed: 2_870_177_450, ram_bytes: request.component === 'ram' ? 20 * GIB : 0, phases },
    outcome,
    outcomeDetail: {
      verdict: VERDICT[outcome] ?? outcome,
      params: outcome === 'errors' ? { core: String(ERROR_CORE) } : outcome === 'low_stability' ? { stability: '95.3' } : {},
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
    cores: isGpu(request) ? [] : coreStates(phases, elapsedMs, failed, true).map((state, core) => ({ core, state, firstError: failed && core === ERROR_CORE ? error : null })),
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
    ...(isGpu(request) ? { stability: outcome === 'low_stability' ? LOW_STABILITY : outcome === 'passed' ? 0.99 : null, gpuDeviceId: request.gpu ?? null } : {}),
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
  loadPercent: null,
  stability: null,
  gpuDeviceId: null,
});

export function mockPerformance(scenario: Scenario, serviceConnected: () => boolean) {
  const listeners = new Set<(status: RunStatus) => void>();
  let sessions = seeded();
  let status = idle();
  let run: { id: string; request: StartRequest; phases: Phase[]; startedAtMs: number; fails: 'errors' | 'device_lost' | null; stopAtMs: number | null; timer: ReturnType<typeof setInterval> } | null = null;

  const publish = (next: RunStatus) => {
    status = next;
    listeners.forEach((cb) => cb(structuredClone(status)));
  };
  const finish = (elapsedMs: number, outcome: Outcome) => {
    const r = run!;
    clearInterval(r.timer);
    run = null;
    sessions = [buildSession(r.id, r.startedAtMs, r.request, r.phases, elapsedMs, outcome), ...sessions];
    publish({ ...status, state: 'finished', elapsedMs, outcome, errors: outcome === 'errors' || outcome === 'device_lost' ? 1 : 0, cores: isGpu(r.request) ? [] : coreStates(r.phases, elapsedMs, outcome === 'errors', true).map((state, core) => ({ core, state })), currentCore: null, loadPercent: null, stability: r.request.component === 'gpu' && outcome === 'low_stability' ? LOW_STABILITY : status.stability });
  };
  const tick = () => {
    const r = run!;
    const elapsedMs = Math.min(Date.now() - r.startedAtMs, TEST_S * 1000);
    if (r.fails && elapsedMs >= errorAtMs(r.phases)) return finish(errorAtMs(r.phases), r.fails);
    if (r.stopAtMs !== null && Date.now() >= r.stopAtMs) return finish(elapsedMs, 'stopped_user');
    if (elapsedMs >= TEST_S * 1000) return finish(elapsedMs, scenario === 'low_stability' && isGpu(r.request) ? 'low_stability' : 'passed');
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
      cores: isGpu(r.request) ? [] : coreStates(r.phases, elapsedMs, false, false).map((state, core) => ({ core, state })),
      currentCore: at.current,
      loadPercent: loadAt(r.phases, elapsedMs),
      stability: isGpu(r.request) && elapsedMs > 40_000 ? (scenario === 'low_stability' ? LOW_STABILITY : 0.99) : null,
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
      gpus: [
        { deviceId: MOCK_GPU, name: 'Mock GeForce RTX 4080', integrated: false, dedicatedBytes: 16 * GIB },
        { deviceId: 'gpu-mock-integrated', name: 'Mock Radeon Graphics', integrated: true, dedicatedBytes: 512 * 1024 ** 2 },
      ],
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
      run = { id, request, phases, startedAtMs: Date.now(), fails: scenario === 'error' ? 'errors' : scenario === 'device_lost' && isGpu(request) ? 'device_lost' : null, stopAtMs: null, timer: setInterval(tick, 1000) };
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
        gpuDeviceId: isGpu(request) ? (request.gpu ?? null) : null,
        cores: isGpu(request) ? [] : coreStates(phases, 0, false, false).map((state, core) => ({ core, state })),
        warnings: serviceConnected() || isGpu(request) ? [] : ['noService', 'tempMissing'],
      });
      return id;
    },
    stop() {
      if (run === null || run.stopAtMs !== null) return;
      run.stopAtMs = Date.now() + 500;
      publish({ ...status, state: 'stopping' });
    },
    status: () => structuredClone(status),
    running: () => run !== null,
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
  if (scenario !== null) {
    const gpu = scenario === 'gpu' || scenario === 'device_lost' || scenario === 'low_stability';
    api.start({ component: gpu ? 'gpu' : 'cpu', objective: 'overclock', preset: 'standard', custom: null, retryCore: null, ...(gpu ? { gpu: MOCK_GPU } : {}) });
  }
  return api;
}

// A fake CPU benchmark for `pnpm dev`: 48 steps over 20 s on the same 8-core CPU, about 1500
// single and 12 000 multi core points. `?bench=error` ends with a calculation error in the multi
// half, so the score is saved as not valid.

const BENCH_S = 20;
const BENCH_KERNELS: { id: BenchKernel; unit: string; single: number; multi: number }[] = [
  { id: 'ntt', unit: 'Mop/s', single: 428.2, multi: 5138 },
  { id: 'hash', unit: 'MB/s', single: 1406, multi: 16870 },
  { id: 'compress', unit: 'MB/s', single: 305, multi: 3660 },
  { id: 'sort', unit: 'Melem/s', single: 23.99, multi: 287.9 },
  { id: 'fft', unit: 'GFLOP/s', single: 8.623, multi: 103.5 },
  { id: 'gemm', unit: 'GFLOP/s', single: 21.39, multi: 256.6 },
];
const BENCH_STEPS: BenchStep[] = (['single', 'multi'] as const).flatMap((mode) =>
  BENCH_KERNELS.flatMap((k) => [0, 1, 2, 3].map((rep) => ({ kernel: k.id, mode, rep }))),
);

export function parseBenchScenario(search: string): 'error' | null {
  return new URLSearchParams(search).get('bench') === 'error' ? 'error' : null;
}

const scoreSummary = (f: CpuScoreFile): CpuScoreSummary => ({
  id: f.id,
  at: f.at,
  single: f.scores.single,
  multi: f.scores.multi,
  valid: f.valid,
  flags: f.flags,
  provisional: f.provisional,
});

function scoreFile(id: string, atMs: number, single: number, multi: number | null, valid: boolean): CpuScoreFile {
  const speed = (v: number, f: number) => Math.round(v * f * 100) / 100;
  return {
    format: 1,
    id,
    at: new Date(atMs).toISOString(),
    category: 'cpu',
    scoreVersion: 'cpu-1',
    provisional: true,
    isa: 'avx512',
    scores: { single, multi },
    kernels: BENCH_KERNELS.map((k) => ({ id: k.id, unit: k.unit, single: speed(k.single, single / 1500), multi: multi === null ? null : speed(k.multi, multi / 12000) })),
    device: { model: 'Mock Ryzen 7 7800X3D', cores: CORES, logical: CORES * 2 },
    flags: valid ? [] : ['compute_error'],
    valid,
    scaling: multi === null ? null : multi / single / (CORES * 2),
    samples: [],
    appVersion: '0.5.0',
    loadVersion: '0.5.0',
  };
}

export function mockBench(scenario: 'error' | null, stressRunning: () => boolean) {
  const listeners = new Set<(status: BenchStatus) => void>();
  const day = 24 * 3600 * 1000;
  let scores: CpuScoreFile[] = [
    scoreFile('5b0c3f7e-2d41-4e8a-9c6b-7a1e0f2d3c11', Date.now() - day, 1488, 11850, true),
    scoreFile('c2e9a8d1-6f3b-4a70-b5d4-3e8f1a0c9b22', Date.now() - 4 * day, 1512, 12040, true),
  ];
  let status: BenchStatus | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;

  const publish = (next: BenchStatus) => {
    status = next;
    listeners.forEach((cb) => cb(structuredClone(next)));
  };
  const end = (next: Partial<BenchStatus>) => {
    clearInterval(timer!);
    timer = null;
    publish({ ...status!, step: null, livePoints: null, ...next });
  };

  return {
    start(): string {
      if (timer !== null || stressRunning()) throw 'busy';
      const id = crypto.randomUUID();
      const startedAtMs = Date.now();
      const failAt = scenario === 'error' ? 30 : Infinity;
      const single = 1500 + Math.round(Math.random() * 40 - 20);
      const multi = 12000 + Math.round(Math.random() * 300 - 150);
      publish({ state: 'starting', step: null, steps: BENCH_STEPS, segments: BENCH_STEPS.map(() => 'pending'), livePoints: null, single: null, multi: null, flags: [], scoreId: null, error: null });
      timer = setInterval(() => {
        const elapsed = Date.now() - startedAtMs;
        const step = Math.min(BENCH_STEPS.length, Math.floor((elapsed / (BENCH_S * 1000)) * BENCH_STEPS.length));
        const singleDone = step >= 24;
        if (step >= failAt) {
          scores = [scoreFile(id, startedAtMs, single, null, false), ...scores];
          return end({ state: 'done', segments: BENCH_STEPS.map((_, i) => (i < failAt ? 'done' : i === failAt ? 'failed' : 'pending')), single, flags: ['compute_error'], scoreId: id });
        }
        if (step >= BENCH_STEPS.length) {
          scores = [scoreFile(id, startedAtMs, single, multi, true), ...scores];
          return end({ state: 'done', segments: BENCH_STEPS.map(() => 'done'), single, multi, scoreId: id });
        }
        const target = BENCH_STEPS[step].mode === 'single' ? single : multi;
        // Like oma-core, a new step has no rate until its first progress, and a warm-up
        // (its 2 s pause and reference send rate 0) none at all at this pace.
        const fresh = step !== status!.step || BENCH_STEPS[step].rep === 0;
        publish({
          ...status!,
          state: status!.state === 'stopping' ? 'stopping' : 'running',
          step,
          segments: BENCH_STEPS.map((_, i) => (i < step ? 'done' : i === step ? 'running' : 'pending')),
          livePoints: fresh ? null : target * (0.9 + 0.2 * Math.random()),
          single: singleDone ? single : null,
        });
      }, 250);
      return id;
    },
    stop() {
      if (timer === null || status?.state === 'stopping') return;
      publish({ ...status!, state: 'stopping' });
      setTimeout(() => timer !== null && end({ state: 'stopped' }), 500);
    },
    status: () => structuredClone(status),
    scores: () => scores.map(scoreSummary),
    score: (id: string) => structuredClone(scores.find((s) => s.id === id) ?? null),
    remove(id: string) {
      scores = scores.filter((s) => s.id !== id);
    },
    subscribe(cb: (status: BenchStatus) => void) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
}
