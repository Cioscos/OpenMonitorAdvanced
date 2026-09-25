import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { StatsPoller } from '../../lib/advanced/statsPoller.svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { DASH, formatValue } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import type { Sensor } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import SensorTable from './SensorTable.svelte';

const GPU = 'gpu/pci-0000:01:00.0';
const LOAD = `${GPU}/load/core`;
const THROTTLE: Sensor = {
  id: `${GPU}/flag/throttle-power`,
  deviceId: GPU,
  kind: 'flag',
  unit: 'boolean',
  label: { key: 'gpu.throttle.power' },
  source: 'nvml',
  category: 'flag',
};
const sensors = [...MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU), THROTTLE];
const ids = sensors.map((s) => s.id);
const current: Record<string, number> = { [LOAD]: 63, [THROTTLE.id]: 1 };
const valueOf = (id: string) => current[id] ?? null;

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

function setup() {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.stats = {
    [LOAD]: { min: 5, max: 95, avg: 40.4, count: 10 },
    [THROTTLE.id]: { min: 0, max: 1, avg: 0.25, count: 8 },
  };
  const stats = new StatsPoller(backend, () => ids, () => MOCK_SCHEMA.revision);
  render(SensorTable, { sensors, valueOf, stats });
  return { backend, stats };
}

/** Text of the value cells in the row of the sensor labelled `label`. */
const cells = (label: string) => [...screen.getByText(label).closest('tr')!.querySelectorAll('td')].map((td) => td.textContent);

test('groups follow the category order', () => {
  setup();
  const headings = screen.getAllByRole('columnheader').filter((th) => th.getAttribute('scope') === 'colgroup');
  expect(headings.map((h) => h.textContent)).toEqual(
    ['temperature', 'load', 'clock', 'power', 'data', 'flag'].map((c) => t(`advanced.category.${c}`)),
  );
});

test('rows show current, min, max and average in the sensor unit', async () => {
  const { stats } = setup();
  await stats.poll();
  flushSync();
  expect(cells(t('sensor.gpu.load.core'))).toEqual(['63%', '5%', '95%', '40%']);
  expect(cells(t('sensor.gpu.temperature.core'))).toEqual([DASH, DASH, DASH, DASH]);
});

test('flags read as on/off and their average as the share of time on', async () => {
  const { stats } = setup();
  await stats.poll();
  flushSync();
  expect(cells(t('sensor.gpu.throttle.power'))).toEqual([t('flag.on'), t('flag.off'), t('flag.on'), '25%']);
  expect(t('flag.on')).toBe(formatValue(1, 'boolean', 'en', t));
});

test('experimental sensors are marked and every row has its source badge', () => {
  setup();
  const hotspotRow = screen.getByText(t('sensor.gpu.temperature.hotspot')).closest('tr')!;
  expect(hotspotRow.textContent).toContain(t('advanced.experimental'));
  expect(screen.getAllByText(t('advanced.experimental'))).toHaveLength(1);
  expect(screen.getByText('NVML').getAttribute('title')).toBe(t('source.nvml'));
  expect(screen.getAllByTitle(t('source.mock'))).toHaveLength(sensors.length - 1);
});

test('reset clears the page sensors in the core and reads them again', async () => {
  const { backend, stats } = setup();
  await stats.poll();
  flushSync();
  expect(cells(t('sensor.gpu.load.core'))[1]).toBe('5%');

  await fireEvent.click(screen.getByRole('button', { name: t('advanced.table.reset') }));
  await vi.waitFor(() => expect(backend.statsCalls).toHaveLength(2));
  expect(backend.resetCalls).toEqual([ids]);
  flushSync();
  // FakeBackend drops the reset entries, as the core does until the next sample.
  expect(cells(t('sensor.gpu.load.core'))).toEqual(['63%', DASH, DASH, DASH]);
});

test('labels follow the language', async () => {
  i18n.locale = 'it';
  const { stats } = setup();
  await stats.poll();
  flushSync();
  expect(screen.getByRole('button', { name: 'Azzera min/max' })).toBeTruthy();
  expect(screen.getByRole('columnheader', { name: 'Media' })).toBeTruthy();
  expect(cells(t('sensor.gpu.load.core'))).toEqual(['63%', '5%', '95%', '40%']);
  expect(screen.getByText('Indicatori di stato')).toBeTruthy();
});

test('network pages show byte rates in bits, like the Simple view', async () => {
  const DOWN = 'network/mock-eth/throughput/down';
  const netSensors = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === 'network/mock-eth');
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.stats = { [DOWN]: { min: 125_000, max: 6_000_000, avg: 1_000_000, count: 4 } };
  const stats = new StatsPoller(backend, () => netSensors.map((s) => s.id), () => MOCK_SCHEMA.revision);
  render(SensorTable, { sensors: netSensors, valueOf: (id: string) => (id === DOWN ? 6_000_000 : null), stats, rate: 'bits' });
  await stats.poll();
  flushSync();
  expect(cells(t('sensor.network.down'))).toEqual(['48 Mbit/s', '1.0 Mbit/s', '48 Mbit/s', '8.0 Mbit/s']);
});
