import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { performanceStore } from '../../lib/performance/performance.svelte';
import type { Phase, Plan, SettingsPatch, StartRequest, SystemInfo } from '../../lib/types';
import { formatBytes as formatDiskBytes } from '../../lib/performance/disk';
import { makeRunStatus, makeSystemInfo, makeVolume, type FakeBackend } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import PerformanceView from './PerformanceView.svelte';
import StressWizard from './StressWizard.svelte';

const MIB = 1024 ** 2;

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

/** 10 + 20 + 5 minutes: full load, one core at a time, variable. */
const PLAN: Plan = {
  seed: 1,
  ram_bytes: 4 * 1024 * MIB,
  phases: [
    phase({ kernel: 'k1', duration_s: 600 }),
    phase({ kernel: 'k2', duration_s: 1200, size: 'l2', placement: 'core_cycle', per_core_s: 150, cores: [0, 1, 2, 3, 4, 5, 6, 7] }),
    phase({ kernel: 'k5', duration_s: 300, size: 'l3', mode: 'variable' }),
  ],
};

let off: (() => void) | undefined;

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  off?.();
  off = undefined;
  disconnectSettings();
});

interface Setup {
  system?: Partial<SystemInfo>;
  settings?: SettingsPatch;
  running?: boolean;
  /** Renders the whole view instead of the wizard alone. */
  view?: boolean;
}

async function setup({ system, settings, running, view }: Setup = {}) {
  const backend = await connectSettings(settings);
  backend.performanceSystemInfo = makeSystemInfo(system);
  backend.performancePlan = structuredClone(PLAN);
  if (running) backend.performanceStatusValue = makeRunStatus({ state: 'running', sessionId: 'x' });
  const patches: SettingsPatch[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (patch) => {
    patches.push(patch);
    return update(patch);
  };
  const onStarted = vi.fn();
  if (view) {
    render(PerformanceView, { backend, store: new LiveStore() });
  } else {
    off = await performanceStore.connect(backend);
    render(StressWizard, { backend, onStarted });
  }
  await screen.findByRole('radio', { name: 'CPU' });
  return { backend, patches, onStarted };
}

const next = () => fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.next') }));
const back = () => fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.back') }));
const radio = (name: string) => screen.getByRole('radio', { name }) as HTMLInputElement;
const preset = (id: string, duration: string) => `${t(`performance.preset.${id}`)} · ${duration}`;
/** A text as shown: the brackets that mark its term go. */
const plain = (key: string, params?: Record<string, string | number>) => t(key, params).replace(/[[\]]/g, '');

/** Through the first three steps with the defaults, to the summary. */
async function toSummary(backend: FakeBackend) {
  await next();
  await next();
  await next();
  await screen.findByRole('heading', { name: t('performance.wizard.step.summary') });
  await waitFor(() => expect(backend.performancePreviewRequests.length).toBeGreaterThan(0));
  await screen.findByText(t('performance.wizard.total', { duration: '35 min' }));
}

const lastPreview = (backend: FakeBackend): StartRequest => backend.performancePreviewRequests.at(-1)!;

test('wizard_walks_four_steps_and_back', async () => {
  const { backend } = await setup();
  // 1 · Component: the CPU with its model and size, the RAM with its total and share.
  expect(radio('CPU').checked).toBe(true);
  expect(screen.getByText('Fake Ryzen 7 7800X3D')).toBeTruthy();
  const detail = screen.getByText((_, node) => node?.textContent === plain('performance.wizard.cpu.detail', { cores: 8, threads: 16 }) && node.tagName === 'SPAN');
  expect(detail.querySelector('.term')?.textContent).toBe('threads');
  expect(radio('RAM').disabled).toBe(false);
  expect(document.querySelector('[aria-current="step"]')?.textContent).toContain(t('performance.wizard.step.component'));
  await next();
  // 2 · Objective: two large tiles with the T4 texts.
  expect(radio(t('performance.objective.normal')).checked).toBe(true);
  await fireEvent.click(radio(t('performance.objective.overclock')));
  await next();
  // 3 · Duration: the presets of the overclock profile (no «quick»).
  expect(screen.queryByRole('radio', { name: preset('quick', '5 min') })).toBeNull();
  expect(radio(preset('standard', '1 h')).checked).toBe(true);
  await fireEvent.click(radio(preset('night', '8 h')));
  await next();
  // 4 · Summary, previewed with the choices.
  await screen.findByRole('heading', { name: t('performance.wizard.step.summary') });
  await waitFor(() =>
    expect(lastPreview(backend)).toEqual({ component: 'cpu', objective: 'overclock', preset: 'night', custom: null, retryCore: null }),
  );
  // Back keeps every choice.
  await back();
  expect(radio(preset('night', '8 h')).checked).toBe(true);
  await back();
  expect(radio(t('performance.objective.overclock')).checked).toBe(true);
  // Back to normal: «night» does not exist there, the duration falls back to standard.
  await fireEvent.click(radio(t('performance.objective.normal')));
  await back();
  expect(radio('CPU').checked).toBe(true);
  expect(screen.queryByRole('button', { name: t('performance.wizard.back') })).toBeNull();
  await next();
  await next();
  expect(radio(preset('standard', '30 min')).checked).toBe(true);
  expect(radio(preset('quick', '5 min'))).toBeTruthy();
});

test('ram_is_disabled_with_reason_below_budget', async () => {
  await setup({ system: { ramBudget: 200 * MIB } });
  expect(radio('RAM').disabled).toBe(true);
  const reason = screen.getByText(t('performance.wizard.ram.low'));
  expect(radio('RAM').getAttribute('aria-describedby')).toBe(reason.id);
  expect(radio('CPU').disabled).toBe(false);
  cleanup();
  off?.();
  await setup({ system: { ramBudget: 256 * MIB } });
  expect(radio('RAM').disabled).toBe(false);
  expect(screen.queryByText(t('performance.wizard.ram.low'))).toBeNull();
});

test('no_service_shows_the_warning_but_allows_cpu', async () => {
  const { backend } = await setup({ system: { serviceConnected: false } });
  const warning = () => [...document.querySelectorAll('.warn')].map((n) => n.textContent?.trim());
  expect(warning()).toContain(t('performance.warn.noService'));
  // The thermal stop in it is a term.
  expect(document.querySelector('.warn .term')?.textContent?.toLowerCase()).toBe(t('glossary.thermalStop.name').toLowerCase());
  expect(radio('CPU').disabled).toBe(false);
  await toSummary(backend);
  expect(warning()).toContain(t('performance.warn.noService'));
  expect(screen.getByRole('button', { name: t('performance.wizard.start') })).toHaveProperty('disabled', false);
});

test('summary_lists_phases_with_terms', async () => {
  const { backend } = await setup({ system: { hypervisor: true } });
  backend.performancePlan.phases[1].both_smt = true;
  await toSummary(backend);
  const list = screen.getByRole('list', { name: t('performance.wizard.phases') });
  const rows = within(list).getAllByRole('listitem');
  expect(rows).toHaveLength(3);
  const terms = (row: HTMLElement) => [...row.querySelectorAll('.term')].map((n) => n.textContent);
  expect(terms(rows[0])).toEqual([t('glossary.mode.k1.name'), 'AVX2', t('glossary.mode.steady.name'), t('glossary.mode.allCore.name')]);
  // Both threads of the core are marked with the SMT term.
  expect(terms(rows[1])).toEqual([t('glossary.mode.k2.name'), 'AVX2', t('glossary.mode.steady.name'), t('glossary.mode.coreCycle.name'), t('performance.wizard.bothSmt')]);
  // A size that is not the kernel's own carries the cache term.
  expect(terms(rows[2])).toEqual([t('glossary.mode.k5.name'), 'L3', 'AVX2', t('glossary.mode.variable.name'), t('glossary.mode.allCore.name')]);
  expect(rows[1].textContent).toContain('20 min');
  // Every term is reachable with Tab.
  expect(rows[0].querySelector('.term')?.getAttribute('tabindex')).toBe('0');
  // The warnings: the detected set, the RAM share and the virtual machine.
  const warnings = document.querySelector('.warnings')!;
  expect([...warnings.querySelectorAll('.term')].map((n) => n.textContent)).toEqual(
    expect.arrayContaining(['AVX2', t('glossary.ramShare.name'), t('glossary.vm.name')]),
  );
  expect(warnings.textContent).toContain('4.0 GB');
});

test('customize_rebuilds_the_preview_and_total', async () => {
  const { backend } = await setup();
  backend.performancePlan.phases[0].stop_on_error = true;
  await toSummary(backend);
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.customize') }));
  const panel = screen.getByRole('region', { name: t('performance.wizard.customize') });
  // One row per kernel of the plan, with its minutes.
  const k5 = within(panel).getByRole('checkbox', { name: t('glossary.mode.k5.name') }) as HTMLInputElement;
  expect(k5.checked).toBe(true);
  expect((within(panel).getByRole('spinbutton', { name: t('performance.custom.minutesOf', { name: t('glossary.mode.k2.name') }) }) as HTMLInputElement).value).toBe('20');
  // Without K5 the plan is shorter.
  backend.performancePlan = { ...PLAN, phases: PLAN.phases.slice(0, 2) };
  await fireEvent.click(k5);
  await waitFor(() => expect(lastPreview(backend).custom?.modes).toContainEqual({ kernel: 'k5', enabled: false, minutes: null }));
  await screen.findByText(t('performance.wizard.total', { duration: '30 min' }));
  // The K5 phase stays in the list, marked excluded, so the panel below does not move.
  const rows = within(screen.getByRole('list', { name: t('performance.wizard.phases') })).getAllByRole('listitem');
  expect(rows).toHaveLength(PLAN.phases.length);
  const k5Row = rows[PLAN.phases.findIndex((p) => p.kernel === 'k5')]!;
  expect(k5Row.textContent).toContain(t('performance.wizard.excluded'));
  // The profile stops in the first phase only: «stop at the first error» is mixed until set.
  const stop = within(panel).getByRole('checkbox', { name: t('performance.custom.stopOnFirstError') }) as HTMLInputElement;
  expect(stop.indeterminate).toBe(true);
  // Minutes, set, threads, both SMT threads and the first error go into the same Custom.
  backend.performancePlan = PLAN;
  const minutes = within(panel).getByRole('spinbutton', { name: t('performance.custom.minutesOf', { name: t('glossary.mode.k1.name') }) });
  await fireEvent.input(minutes, { target: { value: '15' } });
  await fireEvent.change(minutes, { target: { value: '15' } });
  await fireEvent.click(within(panel).getByRole('radio', { name: 'SSE2' }));
  await fireEvent.click(within(panel).getByRole('radio', { name: plain('performance.custom.threads.onePerCore') }));
  await fireEvent.click(within(panel).getByRole('checkbox', { name: new RegExp(t('performance.custom.bothSmt')) }));
  await fireEvent.click(within(panel).getByRole('checkbox', { name: t('performance.custom.stopOnFirstError') }));
  await waitFor(() =>
    expect(lastPreview(backend).custom).toEqual({
      modes: [
        { kernel: 'k1', enabled: true, minutes: 15 },
        { kernel: 'k2', enabled: true, minutes: null },
        { kernel: 'k5', enabled: false, minutes: null },
      ],
      isa: 'sse2',
      threads: 'onePerCore',
      bothSmt: true,
      stopOnFirstError: true,
    }),
  );
  await screen.findByText(t('performance.wizard.total', { duration: '35 min' }));
  // The debounce sends one preview for a burst of changes, not one per change.
  expect(backend.performancePreviewRequests.length).toBeLessThan(8);
});

test('avx512_hidden_when_unsupported', async () => {
  const { backend } = await setup({ system: { isa: ['avx2', 'sse2'] } });
  await toSummary(backend);
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.customize') }));
  const panel = screen.getByRole('region', { name: t('performance.wizard.customize') });
  expect(within(panel).getByRole('radio', { name: t('performance.custom.isa.auto') })).toHaveProperty('checked', true);
  expect(within(panel).getByRole('radio', { name: 'AVX2' })).toBeTruthy();
  expect(within(panel).queryByRole('radio', { name: 'AVX-512' })).toBeNull();
  cleanup();
  off?.();
  const again = await setup({ system: { isa: ['avx512', 'avx2', 'sse2'] } });
  await toSummary(again.backend);
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.customize') }));
  expect(screen.getByRole('radio', { name: 'AVX-512' })).toBeTruthy();
});

test('risk_notice_shows_once_then_never', async () => {
  const { backend, patches } = await setup();
  await toSummary(backend);
  const start = () => fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.start') }));
  // Cancelling starts nothing and writes nothing.
  await start();
  let dialog = screen.getByRole('dialog', { name: t('performance.risk.title') });
  expect(dialog.textContent).toContain(t('performance.risk.body'));
  await fireEvent.click(within(dialog).getByRole('button', { name: t('performance.risk.cancel') }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(backend.performanceStartRequests).toEqual([]);
  // «Don't show again» and Start: the setting is written and the test starts.
  await start();
  dialog = screen.getByRole('dialog', { name: t('performance.risk.title') });
  await fireEvent.click(within(dialog).getByRole('checkbox', { name: t('performance.risk.dontShow') }));
  await fireEvent.click(within(dialog).getByRole('button', { name: t('performance.wizard.start') }));
  await waitFor(() => expect(backend.performanceStartRequests).toHaveLength(1));
  expect(patches).toEqual([{ performance: { riskNoticeSeen: true } }]);
  // Seen: no notice any more.
  cleanup();
  off?.();
  const seen = await setup({ settings: { performance: { riskNoticeSeen: true } } });
  await toSummary(seen.backend);
  await start();
  expect(screen.queryByRole('dialog')).toBeNull();
  await waitFor(() => expect(seen.backend.performanceStartRequests).toHaveLength(1));
  expect(seen.patches).toEqual([]);
});

test('start_goes_to_the_run_page', async () => {
  const { backend } = await setup({ view: true, settings: { performance: { riskNoticeSeen: true } } });
  await toSummary(backend);
  // A refusal of the shell stays on the page with its reason.
  backend.performanceStartError = 'oma-load.exe not found';
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.start') }));
  expect((await screen.findByRole('alert')).textContent).toContain('oma-load.exe not found');
  // A plan the shell cannot build comes as a code, in words.
  backend.performanceStartError = 'build:too_long';
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.start') }));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain(t('performance.wizard.error.too_long')));
  backend.performanceStartError = null;
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.start') }));
  await screen.findByRole('heading', { name: t('performance.run.title') });
  expect(backend.performanceStartRequests).toEqual([{ component: 'cpu', objective: 'normal', preset: 'standard', custom: null, retryCore: null }]);
});

test('start_disabled_while_running', async () => {
  const { backend, onStarted } = await setup({ running: true, settings: { performance: { riskNoticeSeen: true } } });
  await toSummary(backend);
  const start = screen.getByRole('button', { name: t('performance.wizard.start') }) as HTMLButtonElement;
  expect(start.disabled).toBe(true);
  expect(screen.getByText(t('performance.wizard.running'))).toBeTruthy();
  await fireEvent.click(start);
  expect(backend.performanceStartRequests).toEqual([]);
  expect(onStarted).not.toHaveBeenCalled();
  // When the test finishes, Start is back.
  backend.emitPerformanceStatus(makeRunStatus({ state: 'finished', sessionId: 'x', outcome: 'passed' }));
  await waitFor(() => expect(start.disabled).toBe(false));
});

test('a_late_preview_reply_is_dropped', async () => {
  const { backend } = await setup();
  const replies: ((plan: Plan) => void)[] = [];
  vi.spyOn(backend, 'performancePreview').mockImplementation(() => new Promise((resolve) => replies.push(resolve)));
  await next();
  await next();
  await next();
  await waitFor(() => expect(replies).toHaveLength(1));
  // Back to the durations: the standard plan answers only now, for a choice already left.
  await back();
  await fireEvent.click(radio(preset('quick', '5 min')));
  replies[0](PLAN);
  await new Promise((r) => setTimeout(r, 0));
  await next();
  await waitFor(() => expect(replies).toHaveLength(2));
  await new Promise((r) => setTimeout(r, 0));
  expect(screen.queryByText(t('performance.wizard.total', { duration: '35 min' }))).toBeNull();
  expect(screen.getByRole('button', { name: t('performance.wizard.customize') })).toHaveProperty('disabled', true);
  // Two replies out of order: only the newer one counts.
  await back();
  await next();
  await waitFor(() => expect(replies).toHaveLength(3));
  replies[2]({ ...PLAN, phases: PLAN.phases.slice(0, 1) });
  await screen.findByText(t('performance.wizard.total', { duration: '10 min' }));
  replies[1](PLAN);
  await new Promise((r) => setTimeout(r, 0));
  expect(screen.getByText(t('performance.wizard.total', { duration: '10 min' }))).toBeTruthy();
});

test('each_step_moves_the_focus_to_its_title', async () => {
  await setup();
  expect(document.activeElement?.tagName).not.toBe('H3');
  await next();
  expect(document.activeElement?.textContent).toBe(t('performance.wizard.step.objective'));
  await back();
  expect(document.activeElement?.textContent).toBe(t('performance.wizard.step.component'));
});

const GPUS = [
  { deviceId: 'gpu-a', name: 'Fake RTX 4080', integrated: false, dedicatedBytes: 16 * 1024 * MIB },
  { deviceId: 'gpu-b', name: 'Fake Radeon Graphics', integrated: true, dedicatedBytes: 512 * MIB },
];
const GPU_PLAN: Plan = {
  seed: 1,
  ram_bytes: 0,
  phases: [
    phase({ kernel: 's5', alt_kernel: 's1', duration_s: 630, isa: 'sse2' }),
    phase({ kernel: 's1', mode: 'ramp', duration_s: 270, isa: 'sse2' }),
  ],
};

async function gpuSetup(gpus = GPUS) {
  const ctx = await setup({ system: { gpus }, settings: { performance: { riskNoticeSeen: true } } });
  ctx.backend.performancePlan = structuredClone(GPU_PLAN);
  return ctx;
}

test('the GPU plan names every load mode by its glossary term', async () => {
  const { backend } = await gpuSetup();
  backend.performancePlan.phases.push(phase({ kernel: 's1', mode: 'pause_resume', duration_s: 120, isa: 'sse2' }));
  await fireEvent.click(radio('Fake RTX 4080'));
  await next();
  await next();
  await next();
  await screen.findByText(t('glossary.mode.pauseResume.name'));
  expect(document.body.textContent).not.toContain('mode.');
});

test('lists one tile per GPU', async () => {
  await gpuSetup();
  const a = radio('Fake RTX 4080');
  expect(a.closest('label')?.textContent).toContain(plain('performance.wizard.gpu.detail', { vram: '16.0 GB' }));
  expect(radio('Fake Radeon Graphics').closest('label')?.textContent).toContain(t('performance.wizard.gpu.integrated'));
});

test('shows the no-GPU tile when none is available', async () => {
  await gpuSetup([]);
  const none = radio(t('performance.wizard.gpu.none'));
  expect(none.disabled).toBe(true);
});

test('GPU choice sends its deviceId', async () => {
  const { backend } = await gpuSetup();
  await fireEvent.click(radio('Fake Radeon Graphics'));
  await next();
  await next();
  await next();
  await waitFor(() => expect(lastPreview(backend)).toMatchObject({ component: 'gpu', gpu: 'gpu-b' }));
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.start') }));
  await waitFor(() => expect(backend.performanceStartRequests.at(-1)).toMatchObject({ component: 'gpu', gpu: 'gpu-b' }));
});

test('GPU presets come from gpu.<objective>', async () => {
  await gpuSetup();
  await fireEvent.click(radio('Fake RTX 4080'));
  await next();
  await next();
  expect(radio(preset('quick', '5 min'))).toBeTruthy();
  expect(radio(preset('long', '30 min'))).toBeTruthy();
  expect(screen.queryByRole('radio', { name: preset('night', '8 h') })).toBeNull();
  await back();
  await fireEvent.click(radio(t('performance.objective.overclock')));
  await next();
  expect(radio(preset('night', '2 h'))).toBeTruthy();
  expect(screen.queryByRole('radio', { name: preset('quick', '5 min') })).toBeNull();
});

test('integrated GPU summary warns about shared memory', async () => {
  const { backend } = await gpuSetup();
  await fireEvent.click(radio('Fake Radeon Graphics'));
  await next();
  await next();
  await next();
  await waitFor(() => expect(backend.performancePreviewRequests.length).toBeGreaterThan(0));
  await screen.findByText((_, node) => node?.tagName === 'P' && node.textContent === t('performance.warn.gpuShared'));
  expect(screen.queryByText(t('performance.wizard.isaDetected'), { exact: false })).toBeNull();
  expect(document.querySelector('.isa')).toBeNull();
  expect(screen.queryByText(t('performance.warn.noService'), { exact: false })).toBeNull();
  // A dedicated GPU has no such warning.
  await back();
  await back();
  await back();
  await fireEvent.click(radio('Fake RTX 4080'));
  await next();
  await next();
  await next();
  await waitFor(() => expect(lastPreview(backend).gpu).toBe('gpu-a'));
  expect(screen.queryByText((_, node) => node?.textContent === t('performance.warn.gpuShared'))).toBeNull();
});

test('customize hides isa and threads for the GPU', async () => {
  const { backend } = await gpuSetup();
  await fireEvent.click(radio('Fake RTX 4080'));
  await next();
  await next();
  await next();
  await waitFor(() => expect(backend.performancePreviewRequests.length).toBeGreaterThan(0));
  await screen.findByText(t('performance.wizard.total', { duration: '15 min' }));
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.customize') }));
  const panel = screen.getByRole('region', { name: t('performance.wizard.customize') });
  // S5 and S1 appear once each, with minutes and «stop at the first error».
  expect(within(panel).getAllByRole('spinbutton')).toHaveLength(2);
  expect(within(panel).getByRole('checkbox', { name: t('performance.custom.stopOnFirstError') })).toBeTruthy();
  expect(within(panel).queryByText(t('performance.custom.isa'))).toBeNull();
  expect(within(panel).queryByRole('radio')).toBeNull();
});

test('the GPU tile marks VRAM and shared system RAM with terms', async () => {
  await gpuSetup();
  const dedicated = radio('Fake RTX 4080').closest('label')!;
  expect(dedicated.querySelector('.term')?.textContent).toBe('VRAM');
  expect(dedicated.textContent).toContain(plain('performance.wizard.gpu.detail', { vram: '16.0 GB' }));
  const shared = radio('Fake Radeon Graphics').closest('label')!;
  expect(shared.querySelector('.term')?.textContent).toBe('RAM');
  expect(shared.textContent).toContain(t('performance.wizard.gpu.integrated'));
});

test('the shared memory warning marks VRAM with a term', async () => {
  await gpuSetup();
  await fireEvent.click(radio('Fake Radeon Graphics'));
  await next();
  await next();
  await next();
  const warning = await screen.findByText((_, node) => node?.tagName === 'P' && node.textContent === t('performance.warn.gpuShared'));
  expect(warning.querySelector('.term')?.textContent).toBe('VRAM');
});

test('the overclock hint of a GPU speaks of the GPU, not of Curve Optimizer', async () => {
  await gpuSetup();
  await fireEvent.click(radio('Fake RTX 4080'));
  await next();
  const hint = () => document.getElementById('wizard-overclock-hint')?.textContent;
  expect(hint()).toBe(t('performance.objective.overclock.hint.gpu'));
  await back();
  await fireEvent.click(radio('CPU'));
  await next();
  expect(hint()).toBe(t('performance.objective.overclock.hint'));
});

test('a GPU summary asks to leave the PC idle for the stability', async () => {
  const { backend } = await gpuSetup();
  await fireEvent.click(radio('Fake RTX 4080'));
  await next();
  await next();
  await next();
  await waitFor(() => expect(backend.performancePreviewRequests.length).toBeGreaterThan(0));
  const note = await screen.findByText((_, node) => node?.tagName === 'P' && !!node.textContent?.includes(t('performance.wizard.gpuIdle')));
  expect(note.querySelector('.term')?.textContent).toBe(t('glossary.stability.name'));
});

// --- Disk (M8c) ---

const GIB = 1024 ** 3;
const diskPhase = (kernel: Phase['kernel'], duration_s: number, write_cap_bytes: number | null) =>
  phase({ kernel, duration_s, isa: 'sse2', stop_on_error: false, disk: { block_bytes: 4096, seq_block_bytes: 1048576, random_percent: 0, read_percent: 50, queue: 4, threads: 1, write_cap_bytes, cycles: null, rate_limit_bps: null } });
/** The fixture's «standard normal» plan: 8 GiB of preparation, then N1 200, N3 (reads), N2 50 and N4 50 GiB of writes: 308 GiB. */
const DISK_PLAN: Plan = {
  seed: 1,
  ram_bytes: 0,
  disk: { dir: 'C:\\Temp', file_bytes: 8 * GIB, compressible: false, reserve_bytes: GIB },
  phases: [diskPhase('disk_fill', 1200, null), diskPhase('n1', 540, 200 * GIB), diskPhase('n3', 450, null), diskPhase('n2', 360, 50 * GIB), diskPhase('n4', 450, 50 * GIB)],
};
const VOLUMES = [
  makeVolume(),
  makeVolume({ root: 'E:\\', label: 'Stick', folder: 'E:\\Temp', deviceId: 'disk-e', model: 'Fake Stick', kind: 'usb', removable: true, system: false }),
];

async function diskSetup(over: Parameters<typeof setup>[0] = {}) {
  const ctx = await setup({ ...over, system: { volumes: VOLUMES, ...over.system }, settings: { performance: { riskNoticeSeen: true } } });
  ctx.backend.performancePlan = structuredClone(DISK_PLAN);
  return ctx;
}
const diskTile = () => radio(t('performance.wizard.disk'));
async function toDiskSummary(backend: FakeBackend) {
  await fireEvent.click(diskTile());
  await next();
  await next();
  await next();
  await screen.findByRole('heading', { name: t('performance.wizard.step.summary') });
  await waitFor(() => expect(backend.performancePreviewRequests.length).toBeGreaterThan(0));
  await screen.findByText(t('performance.wizard.total', { duration: '50 min' }));
}

test('disk tile shows the volume picker', async () => {
  await diskSetup();
  expect(screen.queryByRole('combobox', { name: t('performance.disk.volume') })).toBeNull();
  expect(diskTile().closest('label')?.textContent).toContain(plain('performance.wizard.disk.detail', { kind: t('performance.disk.kind.nvme'), free: formatDiskBytes(400 * GIB) }));
  await fireEvent.click(diskTile());
  const picker = screen.getByRole('combobox', { name: t('performance.disk.volume') }) as HTMLSelectElement;
  // The system volume is the default.
  expect(picker.value).toBe('C:\\');
  expect(picker.options).toHaveLength(2);
});

test('the disk tile is off without a volume', async () => {
  await diskSetup({ system: { volumes: [] } });
  expect(radio(t('performance.wizard.disk.none')).disabled).toBe(true);
});

test('disk objectives use the disk texts', async () => {
  await diskSetup();
  await fireEvent.click(diskTile());
  await next();
  expect(radio(t('performance.objective.disk.normal'))).toBeTruthy();
  expect(radio(t('performance.objective.disk.overclock'))).toBeTruthy();
  expect(screen.getByText(t('performance.objective.disk.normal.detail'))).toBeTruthy();
  expect(screen.getByText(t('performance.objective.disk.overclock.detail'))).toBeTruthy();
  expect(screen.queryByText(t('performance.objective.overclock.hint'))).toBeNull();
  // The disk presets come from `disk.<objective>`: no quick one for the data stability.
  await fireEvent.click(radio(t('performance.objective.disk.overclock')));
  await next();
  expect(radio(preset('standard', '1 h 10 min'))).toBeTruthy();
  expect(radio(preset('long', '3 h 10 min'))).toBeTruthy();
  expect(screen.queryByRole('radio', { name: preset('quick', '10 min') })).toBeNull();
});

test('summary shows the estimated writes', async () => {
  const { backend } = await diskSetup();
  await toDiskSummary(backend);
  expect(lastPreview(backend)).toMatchObject({ component: 'disk', disk: { folder: VOLUMES[0].folder, wake: false } });
  expect(screen.getByText(t('performance.disk.writes', { size: formatDiskBytes(308 * GIB) }))).toBeTruthy();
  // The phases carry their glossary terms, with no instruction set, thread or service lines.
  expect(screen.getByText(t('glossary.mode.diskFill.name'))).toBeTruthy();
  expect(screen.getByText(t('glossary.mode.n2.name'))).toBeTruthy();
  expect(document.querySelector('.isa')).toBeNull();
  expect(screen.queryByText(t('performance.wizard.isaDetected'), { exact: false })).toBeNull();
  expect(document.body.textContent).not.toContain('mode.');
});

test('customize offers compressible for the disk only', async () => {
  const { backend } = await diskSetup();
  await toDiskSummary(backend);
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.customize') }));
  const panel = screen.getByRole('region', { name: t('performance.wizard.customize') });
  const box = within(panel).getByRole('checkbox', { name: t('performance.custom.compressible') });
  expect(within(panel).queryByText(t('performance.custom.isa'))).toBeNull();
  expect(within(panel).queryByRole('radio')).toBeNull();
  await fireEvent.click(box);
  await waitFor(() => expect(lastPreview(backend).custom).toMatchObject({ compressible: true }));
  cleanup();
  // The CPU has no such option.
  const cpu = await setup();
  await toSummary(cpu.backend);
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.customize') }));
  expect(screen.queryByRole('checkbox', { name: t('performance.custom.compressible') })).toBeNull();
});

test('standby disk asks for consent', async () => {
  const { backend } = await diskSetup();
  await toDiskSummary(backend);
  backend.performanceStartError = 'disk:standby';
  await fireEvent.click(screen.getByRole('button', { name: t('performance.wizard.start') }));
  const dialog = await screen.findByRole('alertdialog');
  expect(dialog.textContent).toContain(t('performance.disk.standby.title'));
  expect(backend.performanceStartRequests).toHaveLength(0);
  backend.performanceStartError = null;
  await fireEvent.click(within(dialog).getByRole('button', { name: t('performance.disk.standby.confirm') }));
  await waitFor(() => expect(backend.performanceStartRequests.at(-1)).toMatchObject({ component: 'disk', disk: { folder: VOLUMES[0].folder, wake: true } }));
  expect(screen.queryByRole('alertdialog')).toBeNull();
});

test('a disk summary names a refused folder in words', async () => {
  const { backend } = await diskSetup();
  backend.performancePreview = async () => {
    throw 'disk:not_writable';
  };
  await fireEvent.click(diskTile());
  await next();
  await next();
  await next();
  await screen.findByText(t('performance.wizard.previewError', { reason: t('performance.disk.error.not_writable') }));
});
