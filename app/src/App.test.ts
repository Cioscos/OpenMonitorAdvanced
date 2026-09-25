import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n } from './lib/i18n/index.svelte';
beforeEach(() => { localStorage.clear(); i18n.locale = 'en'; });
afterEach(cleanup);
import { flushSync } from 'svelte';
import App from './App.svelte';
import { MOCK_SCHEMA, mockValues } from './lib/backend/mock';
import { LiveStore } from './lib/live.svelte';
import { FakeBackend } from './test/fake-backend';

test('simple view shows the banner and the CPU, RAM and network tiles', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: Date.now(), values: mockValues(1) });
  flushSync();

  expect(screen.getByText('Monitoring active')).toBeTruthy();
  expect(screen.getByText('Basic mode')).toBeTruthy();
  expect(screen.getByText('CPU')).toBeTruthy();
  expect(screen.getByText('RAM')).toBeTruthy();
  expect(screen.getByText('Network · Disks')).toBeTruthy();
  expect(screen.getByText((text) => text.startsWith('C: 65% · '))).toBeTruthy();
});

test('clicking a tile opens the advanced view, the toggle goes back', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  flushSync();

  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: 1000, values: mockValues(1) });
  flushSync();
  await fireEvent.click(screen.getByText('CPU').closest('button')!);
  expect(screen.getByText('The Advanced view arrives in milestone 3.')).toBeTruthy();
  await fireEvent.click(screen.getByRole('tab', { name: 'Simple' }));
  expect(screen.getByText('Monitoring active')).toBeTruthy();
});
