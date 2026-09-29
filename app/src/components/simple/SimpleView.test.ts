import { cleanup, render } from '@testing-library/svelte';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { formatRate, formatTemperature } from '../../lib/format';
import { i18n } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import { connectSettings, disconnectSettings } from '../../test/settings';
import SimpleView from './SimpleView.svelte';

beforeEach(() => { i18n.locale = 'en'; });
afterEach(() => { cleanup(); disconnectSettings(); });

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
