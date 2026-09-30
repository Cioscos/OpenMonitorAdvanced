import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { formatRate, formatTemperature } from '../../lib/format';
import { i18n } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import { health } from '../../lib/health.svelte';
import type { Alert, HealthReport } from '../../lib/types';
import { connectSettings, disconnectSettings } from '../../test/settings';
import { FakeBackend } from '../../test/fake-backend';
import SimpleView from './SimpleView.svelte';

beforeEach(() => { i18n.locale = 'en'; });
let offHealth: (() => void) | undefined;
afterEach(() => { cleanup(); disconnectSettings(); offHealth?.(); offHealth = undefined; });

function setup() {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  const values = mockValues(1);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 2000, values });
  render(SimpleView, { store, onOpenAdvanced: () => {} });
  const valueOf = (id: string) => values[MOCK_SCHEMA.sensors.findIndex((s) => s.id === id)] as number;
  return { valueOf };
}

const NIC = 'network/mock-eth';
const DISK = 'storage/device-mock-ssd';
const GPU = 'gpu/pci-0000:01:00.0';
const text = () => document.body.textContent ?? '';

test('the network tile follows the throughput setting and the disk line stays in bytes', async () => {
  await connectSettings({ general: { throughputUnit: 'bits' } });
  const { valueOf } = setup();
  const down = valueOf(`${NIC}/throughput/down`);
  const read = valueOf(`${DISK}/throughput/read`);
  expect(text()).toContain(`↓ ${formatRate(down, 'bits', 'en')}`);

  await settings.update({ general: { throughputUnit: 'bytes' } });
  await vi.waitFor(() => expect(text()).toContain(`↓ ${formatRate(down, 'bytes', 'en')}`));
  expect(text()).not.toContain('bit/s');
  // Disks read the same in both modes.
  expect(text()).toContain(formatRate(read, 'bytes', 'en'));
  await settings.update({ general: { throughputUnit: 'bits' } });
  await vi.waitFor(() => expect(text()).toContain(`↓ ${formatRate(down, 'bits', 'en')}`));
  expect(text()).toContain(formatRate(read, 'bytes', 'en'));
});

test('the GPU tile shows its temperature in the chosen unit', async () => {
  await connectSettings({ general: { temperatureUnit: 'f' } });
  const { valueOf } = setup();
  const celsius = valueOf(`${GPU}/temperature/core`);
  expect(text()).toContain(formatTemperature(celsius, 'en', 'f'));
  expect(text()).toContain('°F');

  await settings.update({ general: { temperatureUnit: 'c' } });
  await vi.waitFor(() => expect(text()).toContain(formatTemperature(celsius, 'en', 'c')));
  expect(text()).not.toContain('°F');
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

test('banner_shows_all_clear_with_duration', async () => {
  await withHealth(healthReport({}), 12 * 60_000);
  setup();
  expect(screen.getByText('All clear')).toBeTruthy();
  expect(screen.getByText('for 12 min')).toBeTruthy();
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
  expect(text()).toContain('overheating (92 °C)');
  await settings.update({ general: { temperatureUnit: 'f' } });
  await vi.waitFor(() => expect(text()).toContain('overheating (198 °F)'));
});
