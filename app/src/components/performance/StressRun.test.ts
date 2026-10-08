import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { MOCK_SCHEMA, SERVICE_MOCK_SCHEMA } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { PhaseInfo, RunStatus, Schema } from '../../lib/types';
import { FakeBackend, makeRunStatus, makeStressSession, makeSystemInfo } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import { performanceStore } from '../../lib/performance/performance.svelte';
import { FakeUplot } from '../../test/uplot-stub';
import PerformanceView from './PerformanceView.svelte';

const PHASES: PhaseInfo[] = [
  { kernel: 'k1', mode: 'steady', placement: 'all_logical', durationS: 600, isa: 'avx2' },
  { kernel: 'k2', mode: 'steady', placement: 'core_cycle', durationS: 1200, isa: 'avx2' },
  { kernel: 'k5', mode: 'variable', placement: 'all_logical', durationS: 300, isa: 'avx2' },
];

/** 5 min into the first phase of a 35-minute overclock test, no errors. */
const RUNNING = (over: Partial<RunStatus> = {}): RunStatus =>
  makeRunStatus({
    state: 'running',
    sessionId: 'run-1',
    objective: 'overclock',
    elapsedMs: 312_000,
    totalMs: 2_100_000,
    phases: PHASES,
    tempC: 78,
    tempMaxC: 87,
    stopC: 84,
    powerW: 61,
    clockMhz: 4850,
    checks: 1284,
    ...over,
  });

beforeEach(() => {
  FakeUplot.instances.length = 0;
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  performanceStore.diskDeviceId = null;
  cleanup();
  disconnectSettings();
});

async function setup(status: RunStatus = RUNNING(), schema: Schema = SERVICE_MOCK_SCHEMA) {
  const backend: FakeBackend = await connectSettings();
  backend.performanceSystemInfo = makeSystemInfo();
  backend.performanceStatusValue = status;
  const store = new LiveStore();
  store.applySchema(schema);
  const view = render(PerformanceView, { backend, store, page: 'run' });
  await screen.findByRole('button', { name: t('performance.run.stop') });
  return { backend, view };
}

const tile = (label: string) => [...document.querySelectorAll<HTMLElement>('.tile')].find((e) => e.querySelector('.label')?.textContent === label)!;

test('run_shows_tiles_phases_and_stop', async () => {
  await setup(RUNNING({ events: [
      { atMs: 1000, code: 'mystery', params: {} },
      { atMs: 200_000, code: 'whea', params: { id: '19', apic: '4', core: '2' } },
      { atMs: 300_000, code: 'thermal_stop', params: { temp: '96' } },
    ] }));
  // Header: objective and component, state, elapsed and total time.
  expect(screen.getByRole('heading', { name: `${t('performance.objective.overclock')} · CPU` })).toBeTruthy();
  expect(screen.getByText(t('performance.run.pill.ok'))).toBeTruthy();
  expect(screen.getByText('00:05:12').parentElement?.textContent).toContain('00:35:00');
  // The phases, each with its kernel's term, the current one marked.
  const phases = screen.getByRole('list', { name: t('performance.run.phases') });
  const items = within(phases).getAllByRole('listitem');
  expect(items.map((li) => li.querySelector('.term')?.textContent)).toEqual(['k1', 'k2', 'k5'].map((k) => t(`glossary.mode.${k}.name`)));
  expect(items[0].getAttribute('aria-current')).toBe('step');
  expect(items.map((li) => li.querySelector('.visually-hidden')?.textContent)).toEqual(['now', 'todo', 'todo'].map((s) => t(`performance.run.phase.${s}`)));
  // Five tiles: temperature with max, thermal stop and Tjmax; power; clock; errors with checks; WHEA.
  const temp = tile(t('performance.run.temp'));
  expect(temp.textContent).toContain('78 °C');
  expect(temp.textContent).toContain(t('performance.run.max', { value: '87 °C' }));
  expect([...temp.querySelectorAll('.term')].map((e) => e.textContent)).toEqual([t('glossary.thermalStop.name'), t('glossary.tjmax.name')]);
  expect(temp.textContent).toContain('84 °C');
  expect(tile(t('glossary.packagePower.name')).querySelector('.label .term')).toBeTruthy();
  expect(tile(t('glossary.packagePower.name')).textContent).toContain('61 W');
  expect(tile(t('performance.run.clock')).querySelector('.label .term')?.textContent?.toLowerCase()).toBe(t('glossary.clock.name').toLowerCase());
  expect(tile(t('performance.run.clock')).textContent).toContain('4.85 GHz');
  const errors = tile(t('performance.run.errors'));
  expect(errors.querySelector('.value')?.textContent).toBe('0');
  expect(errors.querySelector('.term')?.textContent).toBe(t('performance.run.checks'));
  expect(errors.textContent).toContain('1,284');
  const whea = tile(t('performance.run.whea'));
  expect(whea.textContent).toContain(t('performance.run.wheaSub', { corrected: 0, fatal: 0 }));
  // Events, newest first and translated; an unknown code shows itself.
  const log = screen.getByRole('list', { name: t('performance.run.events') });
  const lines = within(log).getAllByRole('listitem').map((li) => li.textContent);
  expect(lines[0]).toContain('00:05:00');
  expect(lines[0]).toContain(t('performance.event.thermal_stop', { temp: '96' }));
  expect(lines[2]).toContain('mystery');
  // A line can carry several terms: WHEA, the core number and the APIC ID.
  const terms = [...within(log).getAllByRole('listitem')[1].querySelectorAll('.term')].map((e) => e.textContent);
  expect(terms).toEqual(['WHEA', 'core 2', 'APIC ID']);
});

test('stop_calls_the_backend', async () => {
  const { backend } = await setup();
  await fireEvent.click(screen.getByRole('button', { name: t('performance.run.stop') }));
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceStop'));
  backend.emitPerformanceStatus(RUNNING({ state: 'stopping' }));
  await screen.findByText(t('performance.run.pill.stopping'));
  expect((screen.getByRole('button', { name: t('performance.run.stop') }) as HTMLButtonElement).disabled).toBe(true);
});

test('core_grid_only_in_core_cycle_phases', async () => {
  const { backend } = await setup();
  expect(screen.queryByRole('list', { name: t('performance.run.cores') })).toBeNull();
  backend.emitPerformanceStatus(
    RUNNING({
      phaseIndex: 1,
      elapsedMs: 900_000,
      currentCore: 3,
      cores: [
        { core: 0, state: 'passed' },
        { core: 1, state: 'failed' },
        { core: 2, state: 'passed' },
        { core: 3, state: 'testing' },
        { core: 4, state: 'untested' },
      ],
    }),
  );
  const grid = await screen.findByRole('list', { name: t('performance.run.cores') });
  const cells = within(grid).getAllByRole('listitem');
  expect(cells).toHaveLength(5);
  expect(cells[3].getAttribute('aria-current')).toBe('true');
  expect(cells[3].textContent).toBe(`${t('performance.core.label', { core: 3 })}${t('performance.core.testing')}`);
  expect(cells[1].textContent).toContain(t('performance.core.failed'));
  expect(cells[4].textContent).toContain(t('performance.core.untested'));
  // The grid's title explains the numbering.
  expect(document.querySelector('.cores-panel .term')?.textContent).toBe(t('glossary.mode.coreCycle.name'));
  expect([...document.querySelectorAll('.cores-panel .term')].map((e) => e.textContent)).toContain(t('glossary.coreNumber.name'));
  backend.emitPerformanceStatus(RUNNING({ phaseIndex: 2, elapsedMs: 1_900_000 }));
  await waitFor(() => expect(screen.queryByRole('list', { name: t('performance.run.cores') })).toBeNull());
});

test('warnings_are_listed', async () => {
  await setup(RUNNING({ warnings: ['noService', 'ramReduced', 'wheaUnreadable'], tempC: null, stopC: null }));
  const alerts = screen.getAllByRole('note').map((n) => n.textContent);
  expect(alerts).toEqual([t('performance.warn.noService'), t('performance.warn.ramReduced'), t('performance.warn.wheaUnreadable')]);
  // The thermal stop inside the first one is a term.
  expect(screen.getAllByRole('note')[0].querySelector('.term')?.textContent?.toLowerCase()).toBe(t('glossary.thermalStop.name').toLowerCase());
  // Without a threshold the tile says the stop is off.
  expect(tile(t('performance.run.temp')).textContent).toContain(t('performance.run.stopOff'));
});

test('chart_hidden_without_cpu_sensors', async () => {
  await setup(RUNNING(), MOCK_SCHEMA);
  expect(screen.getByText(t('performance.run.noChart'))).toBeTruthy();
  expect(FakeUplot.instances).toHaveLength(0);
  cleanup();
  const { backend } = await setup(RUNNING(), SERVICE_MOCK_SCHEMA);
  expect(screen.queryByText(t('performance.run.noChart'))).toBeNull();
  await waitFor(() => expect(backend.historyCalls[0]).toEqual({ ids: ['cpu/0/temperature/package', 'cpu/0/power/package'], seconds: 600, maxPoints: undefined }));
  // The window is fixed at 10 minutes: no window buttons.
  expect(screen.queryByRole('group', { name: t('advanced.chart.window.label') })).toBeNull();
});

test('finishing_moves_to_the_result', async () => {
  const { backend } = await setup();
  backend.performanceSessionsById['run-1'] = makeStressSession({ id: 'run-1' });
  backend.emitPerformanceStatus(RUNNING({ state: 'finished', outcome: 'passed' }));
  await screen.findByRole('heading', { name: t('performance.outcome.passed') });
  expect(backend.performanceCalls).toContain('performanceSession:run-1');
  expect(screen.queryByRole('button', { name: t('performance.run.stop') })).toBeNull();
});

const GPU_PHASES: PhaseInfo[] = [
  { kernel: 's5', mode: 'steady', placement: 'all_logical', durationS: 630, isa: 'sse2' },
  { kernel: 's1', mode: 'ramp', placement: 'all_logical', durationS: 270, isa: 'sse2' },
];
const GPU_RUNNING = (over: Partial<RunStatus> = {}) => RUNNING({ component: 'gpu', phases: GPU_PHASES, phaseIndex: 1, currentCore: null, ...over });

test('a pause and resume phase shows its glossary name', async () => {
  await setup(GPU_RUNNING({ phases: [GPU_PHASES[0], { ...GPU_PHASES[1], mode: 'pause_resume' }] }));
  expect(screen.getAllByText(t('glossary.mode.pauseResume.name')).length).toBeGreaterThan(0);
  expect(document.body.textContent).not.toContain('mode.pause_resume');
});

test('gpu run hides the core grid', async () => {
  // Even a phase that says «one core at a time» has no cores to show on a GPU.
  await setup(GPU_RUNNING({ phases: [{ ...GPU_PHASES[0], placement: 'core_cycle' }, GPU_PHASES[1]], phaseIndex: 0, cores: [{ core: 0, state: 'testing' }] }));
  expect(screen.queryByRole('list', { name: t('performance.run.cores') })).toBeNull();
  expect(screen.getByRole('heading', { name: `${t('performance.objective.overclock')} · GPU` })).toBeTruthy();
  // The tiles speak of the GPU: its clock, no Tjmax.
  expect(tile(t('performance.run.clock.gpu')).textContent).toContain('4.85 GHz');
  expect(tile(t('performance.run.temp')).textContent).not.toContain(t('glossary.tjmax.name'));
});

test('ramp shows the load level', async () => {
  const { backend } = await setup(GPU_RUNNING({ loadPercent: 65 }));
  const current = document.querySelector('.current')!;
  expect(current.textContent).toContain(t('performance.result.loadLevel', { level: 65 }));
  expect(current.querySelector('.term:last-child')?.textContent).toBe(t('performance.result.loadLevel', { level: 65 }));
  // Phases without a level (null) show none.
  backend.emitPerformanceStatus(GPU_RUNNING({ loadPercent: null }));
  await waitFor(() => expect(document.querySelector('.current')?.textContent).not.toContain('65'));
});

test('gpu warnings carry their terms and the reduced size', async () => {
  await setup(
    GPU_RUNNING({
      warnings: ['pcieReplay', 'vramReduced'],
      events: [{ atMs: 5000, code: 'vram_reduced', params: { phase: '0', value: String(2 * 1024 ** 3) } }],
    }),
  );
  const [pcie, vram] = [...document.querySelectorAll('.warnings p')];
  expect(pcie.textContent).toBe(t('performance.warn.pcieReplay'));
  expect(pcie.querySelector('.term')?.textContent).toBe('PCIe');
  expect(vram.textContent).toBe(t('performance.warn.vramReduced', { size: '2.0 GB' }));
  expect(vram.querySelector('.term')?.textContent).toBe('VRAM');
});

test('the GPU clock tile carries the clock term in every language', async () => {
  i18n.locale = 'it';
  await setup(GPU_RUNNING());
  expect(tile(t('performance.run.clock.gpu')).querySelector('.label .term')?.textContent?.toLowerCase()).toBe(t('glossary.clock.name').toLowerCase());
});

// --- Disk (M8c) ---

const DISK_PHASES: PhaseInfo[] = [
  { kernel: 'disk_fill', mode: 'steady', placement: 'all_logical', durationS: 1200, isa: 'sse2' },
  { kernel: 'n1', mode: 'steady', placement: 'all_logical', durationS: 540, isa: 'sse2' },
];
const DISK_RUNNING = (over: Partial<RunStatus> = {}) =>
  RUNNING({
    component: 'disk',
    objective: 'normal',
    phases: DISK_PHASES,
    phaseIndex: 1,
    tempC: 47,
    stopC: 70,
    checks: 123_456,
    disk: { deviceId: 'disk/a', readBps: 3_200_000_000, writeBps: 1_500_000_000, writtenBytes: 12 * 1024 ** 3, readBytes: 30 * 1024 ** 3 },
    ...over,
  });
const sensor = (id: string, deviceId: string) => ({ ...MOCK_SCHEMA.sensors[0], id, deviceId });
const DISK_SCHEMA: Schema = {
  ...MOCK_SCHEMA,
  sensors: ['temperature/drive', 'throughput/read', 'throughput/write'].map((name) => sensor(`disk/a/${name}`, 'disk/a')),
};

test('disk run shows rates written and blocks without the core grid', async () => {
  performanceStore.diskDeviceId = 'disk/a';
  const { backend } = await setup(DISK_RUNNING({ cores: [{ core: 0, state: 'testing' }], currentCore: 0 }), DISK_SCHEMA);
  expect(screen.getByRole('heading', { name: `${t('performance.objective.disk.normal')} · ${t('performance.wizard.disk')}` })).toBeTruthy();
  // Temperature with its stop threshold; no Tjmax, power, clock or WHEA.
  expect(tile(t('performance.run.temp')).textContent).toContain('70');
  expect(tile(t('performance.run.diskRead')).querySelector('.label .term')?.textContent).toBe(t('performance.run.diskRead'));
  expect(tile(t('performance.run.diskRead')).textContent).toContain('3,200 MB/s');
  expect(tile(t('performance.run.diskWrite')).textContent).toContain('1,500 MB/s');
  expect(tile(t('performance.run.diskWritten')).textContent).toContain('12.0 GiB');
  const blocks = tile(t('performance.run.blocks'));
  expect(blocks.textContent).toContain('123,456');
  expect(blocks.querySelector('.term')).toBeTruthy();
  expect(tile(t('performance.run.errors')).textContent).toContain('0');
  expect(document.querySelectorAll('.tile')).toHaveLength(6);
  expect(screen.queryByRole('list', { name: t('performance.run.cores') })).toBeNull();
  expect(document.querySelector('.current')?.textContent).not.toContain(t('glossary.isa.sse2.name'));
  // The chart follows the test disk: its temperature and its two speeds.
  await waitFor(() => expect(backend.historyCalls[0]?.ids).toEqual(['disk/a/temperature/drive', 'disk/a/throughput/read', 'disk/a/throughput/write']));
  // The SMART warning carries its term.
  backend.emitPerformanceStatus(DISK_RUNNING({ warnings: ['smartMissing'] }));
  const warning = await screen.findByText((_, node) => node?.tagName === 'P' && node.textContent === t('performance.warn.smartMissing'));
  expect(warning.querySelector('.term')?.textContent).toBe('SMART');
});

test('the chart of a disk run comes from the status when the store does not know the disk', async () => {
  performanceStore.diskDeviceId = null;
  const { backend } = await setup(DISK_RUNNING(), DISK_SCHEMA);
  await waitFor(() => expect(backend.historyCalls[0]?.ids).toEqual(['disk/a/temperature/drive', 'disk/a/throughput/read', 'disk/a/throughput/write']));
});
