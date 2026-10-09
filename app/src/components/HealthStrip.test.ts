import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { MOCK_SCHEMA, mockValues } from '../lib/backend/mock';
import { health } from '../lib/health.svelte';
import { i18n } from '../lib/i18n/index.svelte';
import { LiveStore } from '../lib/live.svelte';
import { settings } from '../lib/settings.svelte';
import type { Alert, HealthReport } from '../lib/types';
import { connectSettings, disconnectSettings } from '../test/settings';
import { FakeBackend } from '../test/fake-backend';
import HealthStrip from './HealthStrip.svelte';

const GPU = 'gpu/pci-0000:01:00.0';
let offHealth: (() => void) | undefined;
let frames: FrameRequestCallback[] = [];

function scrollTo(y: number) {
  Object.defineProperty(window, 'scrollY', { value: y, configurable: true });
}
/** Scrolls to `y` and runs the animation frame the listener asked for. */
async function scroll(y: number) {
  scrollTo(y);
  window.dispatchEvent(new Event('scroll'));
  const pending = frames;
  frames = [];
  pending.forEach((cb) => cb(0));
  await Promise.resolve();
}
const chrome = () => document.documentElement.style.getPropertyValue('--chrome-height');
const strip = () => document.querySelector('.strip') as HTMLElement;
const has = (cls: string) => strip().classList.contains(cls);

beforeEach(() => {
  i18n.locale = 'en';
  frames = [];
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb));
  vi.stubGlobal('cancelAnimationFrame', () => {});
  scrollTo(0);
});
afterEach(() => {
  cleanup();
  disconnectSettings();
  offHealth?.();
  offHealth = undefined;
  vi.unstubAllGlobals();
  document.documentElement.style.removeProperty('--chrome-height');
});

const gpuAlert = (over: Partial<Alert> = {}): Alert => ({
  ruleId: 'gpu-temp',
  sensorId: `${GPU}/temperature/core`,
  deviceId: GPU,
  unit: 'celsius',
  sensorLabel: { key: 'gpu.temperature.core' },
  level: 'crit',
  value: 92,
  threshold: 90,
  sinceMs: 1,
  valid: true,
  lastValidMs: 1,
  messageKey: 'rule.gpu-temp.message',
  params: { device: 'Mock GeForce RTX 4080' },
  ...over,
});
const healthReport = (over: Partial<HealthReport>): HealthReport => ({
  level: 'ok', sinceMs: 0, revision: 1, coverage: 'complete', unavailableTargets: [], alerts: [], ...over,
});

async function withHealth(report: HealthReport, elapsedMs = 0): Promise<FakeBackend> {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.health = report;
  backend.healthClock = { revision: report.revision, levelElapsedMs: elapsedMs };
  offHealth = await health.connect(backend);
  return backend;
}

function setup(timestampMs = 2000) {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  if (timestampMs > 0) store.applySnapshot({ revision: 1, seq: 1, timestampMs, values: mockValues(1) });
  return render(HealthStrip, { store });
}

test('banner_shows_all_clear_with_duration', async () => {
  await withHealth(healthReport({}), 12 * 60_000);
  setup();
  expect(screen.getByText('All clear')).toBeTruthy();
  expect(screen.getByText('for 12 min')).toBeTruthy();
});

test('banner says less than a minute during the first minute', async () => {
  const backend = await withHealth(healthReport({}), 59_999);
  setup();
  expect(screen.getByText('for less than a minute')).toBeTruthy();
  expect(screen.queryByText('for 0 min')).toBeNull();
  backend.emitHealthClock({ revision: 1, levelElapsedMs: 60_000 });
  await vi.waitFor(() => expect(screen.getByText('for 1 min')).toBeTruthy());
  i18n.locale = 'it';
  backend.emitHealthClock({ revision: 1, levelElapsedMs: 0 });
  await vi.waitFor(() => expect(screen.getByText('da meno di un minuto')).toBeTruthy());
});

test('banner_lists_problems_and_opens_with_keyboard', async () => {
  const ram = gpuAlert({ ruleId: 'ram-used', sensorId: 'memory/0/load/used', deviceId: 'memory/0', unit: 'percent', level: 'warn', value: 91, messageKey: 'rule.ram-used.message', params: {} });
  const backend = await withHealth(healthReport({ level: 'crit', alerts: [gpuAlert(), ram] }));
  setup();
  const toggle = screen.getByRole('button', { name: /2 problems/ });
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(screen.queryByText(/overheating/)).toBeNull();

  // A button opens on Enter/Space, which the browser turns into a click; jsdom needs it spelled out.
  toggle.focus();
  await fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  const rows = screen.getAllByRole('listitem').map((li) => li.textContent);
  expect(rows).toEqual(['Mock GeForce RTX 4080 overheating (92 °C)', 'Memory almost full (91%)']);

  await fireEvent.keyDown(toggle, { key: 'Escape' });
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(document.activeElement).toBe(toggle);

  // The list follows the report: with the problems gone the button goes too.
  backend.emitHealth(healthReport({ revision: 2 }));
  await vi.waitFor(() => expect(screen.getByText('All clear')).toBeTruthy());
  expect(screen.queryByRole('button', { name: /problems/ })).toBeNull();
});

test('the banner reformats when the temperature unit changes', async () => {
  await connectSettings({ general: { temperatureUnit: 'c' } });
  await withHealth(healthReport({ level: 'crit', alerts: [gpuAlert()] }));
  setup();
  expect(document.body.textContent).toContain('overheating (92 °C)');
  await settings.update({ general: { temperatureUnit: 'f' } });
  await vi.waitFor(() => expect(document.body.textContent).toContain('overheating (198 °F)'));
});

test('nothing shows before the first snapshot', async () => {
  await withHealth(healthReport({}));
  setup(0);
  expect(screen.queryByRole('status')).toBeNull();
});

test('at the top the strip is shown; down past 24 px hides it and up shows it again', async () => {
  await withHealth(healthReport({}));
  setup();
  expect(has('hidden')).toBe(false);
  expect(chrome()).toBe('calc(var(--topbar-h) + var(--strip-h))');

  await scroll(20);
  expect(has('hidden')).toBe(false);
  await scroll(100);
  expect(has('hidden')).toBe(true);
  expect(chrome()).toBe('var(--topbar-h)');
  await scroll(90);
  expect(has('hidden')).toBe(false);
  expect(chrome()).toBe('calc(var(--topbar-h) + var(--strip-h))');
  await scroll(200);
  expect(has('hidden')).toBe(true);
  await scroll(0);
  expect(has('hidden')).toBe(false);
});

test('with a warning the strip collapses to the title line and never hides', async () => {
  await withHealth(healthReport({ level: 'warn', alerts: [gpuAlert({ level: 'warn' })] }));
  setup();
  expect(has('compact')).toBe(false);
  expect(screen.getByText(/for less than a minute/)).toBeTruthy();

  await scroll(300);
  expect(has('hidden')).toBe(false);
  expect(has('compact')).toBe(true);
  expect(chrome()).toBe('calc(var(--topbar-h) + var(--strip-compact-h))');
  expect(screen.getByText(/overheating/)).toBeTruthy();
  expect(screen.queryByText(/for less than a minute/)).toBeNull();

  await scroll(250);
  expect(has('compact')).toBe(false);
  expect(chrome()).toBe('calc(var(--topbar-h) + var(--strip-h))');
});

test('the strip only animates when motion is allowed', async () => {
  await withHealth(healthReport({}));
  setup();
  expect(has('animated')).toBe(true);
  cleanup();
  vi.stubGlobal('matchMedia', (query: string) => ({ matches: true, media: query, addEventListener() {}, removeEventListener() {} }));
  setup();
  expect(has('animated')).toBe(false);
});

test('one passive scroll listener, removed on unmount, and no work without a change of state', async () => {
  const add = vi.spyOn(window, 'addEventListener');
  const remove = vi.spyOn(window, 'removeEventListener');
  await withHealth(healthReport({}));
  const view = setup();
  const scrollAdds = add.mock.calls.filter(([type]) => type === 'scroll');
  expect(scrollAdds).toHaveLength(1);
  expect(scrollAdds[0][2]).toEqual({ passive: true });

  // Many events inside one frame ask for a single frame callback.
  scrollTo(50);
  for (let i = 0; i < 5; i++) window.dispatchEvent(new Event('scroll'));
  expect(frames).toHaveLength(1);
  frames[0](0);
  await Promise.resolve();
  frames = [];
  const classes = strip().className;
  await scroll(80);
  expect(strip().className).toBe(classes);

  view.unmount();
  expect(remove.mock.calls.some(([type]) => type === 'scroll')).toBe(true);
  expect(chrome()).toBe('');
  add.mockRestore();
  remove.mockRestore();
});
