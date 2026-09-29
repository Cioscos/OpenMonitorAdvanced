import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { i18n } from '../../lib/i18n/index.svelte';
import { settings } from '../../lib/settings.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { Schema } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import AdvancedView from './AdvancedView.svelte';

beforeEach(async () => {
  await connectSettings();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

const savedSection = () => settings.state?.settings.advanced.section;

const GPU = 'gpu/pci-0000:01:00.0';

function renderView(schema: Schema = MOCK_SCHEMA): LiveStore {
  const store = new LiveStore();
  store.applySchema(schema);
  store.applySnapshot({ revision: schema.revision, seq: 1, timestampMs: 1000, values: mockValues(1) });
  render(AdvancedView, { store, backend: new FakeBackend(schema) });
  return store;
}

const sidebar = () => screen.getByRole('navigation', { name: 'Components' });
const entryNames = () => [...sidebar().querySelectorAll('.name')].map((n) => n.textContent);
const title = () => screen.getByRole('heading', { level: 2 }).textContent;
const current = () => within(sidebar()).getByRole('button', { current: 'page' });

test('the sidebar lists the sections in spec order and opens on the CPU', () => {
  renderView();
  expect(entryNames()).toEqual(['CPU', 'GPU', 'RAM', 'Disk', 'Network']);
  expect(title()).toBe('CPU');
  expect(screen.getAllByText('Mock Ryzen 7 7800X3D').length).toBeGreaterThan(0);
  expect(current().textContent).toContain('CPU');
});

test('selecting an entry opens its page and remembers it', async () => {
  renderView();
  await fireEvent.click(within(sidebar()).getByRole('button', { name: /^GPU/ }));
  expect(title()).toBe('GPU');
  expect(current().textContent).toContain('Mock GeForce RTX 4080');
  await vi.waitFor(() => expect(savedSection()).toBe(GPU));
});

test('the last visited section is restored', async () => {
  await settings.update({ advanced: { section: 'network/mock-eth' } });
  renderView();
  expect(title()).toBe('Network');
  expect(current().textContent).toContain('Ethernet');
});

test('a missing section falls back to the CPU without forgetting the choice', async () => {
  await settings.update({ advanced: { section: GPU } });
  const noGpu: Schema = {
    ...MOCK_SCHEMA,
    devices: MOCK_SCHEMA.devices.filter((d) => d.kind !== 'gpu'),
    sensors: MOCK_SCHEMA.sensors.filter((s) => !s.deviceId.startsWith('gpu/')),
  };
  const store = renderView(noGpu);
  expect(title()).toBe('CPU');
  expect(savedSection()).toBe(GPU);

  // The GPU comes back (e.g. after a driver reload): so does its page.
  store.applySchema({ ...MOCK_SCHEMA, revision: 2 });
  flushSync();
  expect(title()).toBe('GPU');
});

test('section labels are translated', () => {
  i18n.locale = 'it';
  renderView();
  expect(screen.getByRole('navigation', { name: 'Componenti' })).toBeTruthy();
  expect([...screen.getByRole('navigation').querySelectorAll('.name')].map((n) => n.textContent)).toEqual([
    'CPU',
    'GPU',
    'RAM',
    'Disco',
    'Rete',
  ]);
});
