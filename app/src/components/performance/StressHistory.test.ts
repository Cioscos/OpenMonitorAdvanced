import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { StressSessionSummary } from '../../lib/types';
import { FakeBackend, makeRunStatus, makeStressSession, makeSystemInfo } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import PerformanceView from './PerformanceView.svelte';

const summary = (over: Partial<StressSessionSummary>): StressSessionSummary => ({
  id: 'a',
  startedAt: '2026-10-06T10:00:00Z',
  component: 'cpu',
  objective: 'normal',
  preset: 'quick',
  durationMs: 300_000,
  outcome: 'passed',
  verdict: 'passed',
  params: {},
  ...over,
});

const SESSIONS = [
  summary({ id: 'new', startedAt: '2026-10-06T12:00:00Z', component: 'ram', outcome: 'errors', verdict: 'errors_core', params: { core: '2' } }),
  summary({ id: 'old' }),
];

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

async function setup() {
  const backend: FakeBackend = await connectSettings();
  backend.performanceSystemInfo = makeSystemInfo();
  backend.performanceSessions = structuredClone(SESSIONS);
  backend.performanceSessionsById.old = makeStressSession({ id: 'old', request: { component: 'cpu', objective: 'overclock', preset: 'long', custom: null, retryCore: null } });
  render(PerformanceView, { backend, store: new LiveStore(), page: 'history' });
  // The store keeps the previous list until the new one arrives.
  await waitFor(() => expect(screen.queryAllByRole('listitem')).toHaveLength(2));
  return backend;
}
const items = () => within(screen.getByRole('list', { name: t('performance.history.list') })).getAllByRole('listitem');

test('lists_sessions_in_the_order_the_backend_gives', async () => {
  await setup();
  const rows = items();
  expect(rows).toHaveLength(2);
  expect(rows[0].textContent).toContain('RAM');
  expect(rows[0].textContent).toContain(t('performance.outcome.errors_core', { core: 2 }));
  expect(rows[0].querySelector('.term')?.textContent).toBe('core 2');
  expect(rows[1].textContent).toContain(t('performance.outcome.passed'));
});

test('filter_by_component', async () => {
  await setup();
  await fireEvent.click(screen.getByRole('radio', { name: 'CPU' }));
  expect(items()).toHaveLength(1);
  expect(items()[0].textContent).not.toContain('RAM');
});

test('delete_asks_then_removes', async () => {
  const backend = await setup();
  await fireEvent.click(within(items()[1]).getByRole('button', { name: t('performance.history.delete') }));
  expect(backend.performanceCalls).not.toContain('performanceDelete:old');
  await fireEvent.click(within(items()[1]).getByRole('button', { name: t('performance.history.delete') }));
  await waitFor(() => expect(items()).toHaveLength(1));
  expect(backend.performanceCalls).toContain('performanceDelete:old');
});

test('repeat_starts_with_the_same_request', async () => {
  const backend = await setup();
  await fireEvent.click(within(items()[1]).getByRole('button', { name: t('performance.history.repeat') }));
  await waitFor(() => expect(backend.performanceStartRequests).toHaveLength(1));
  expect(backend.performanceStartRequests[0]).toMatchObject({ objective: 'overclock', preset: 'long' });
});

test('empty_history_invites_a_new_test', async () => {
  const backend: FakeBackend = await connectSettings();
  render(PerformanceView, { backend, store: new LiveStore(), page: 'history' });
  await screen.findByText(t('performance.history.empty'));
  await fireEvent.click(within(document.querySelector('.empty') as HTMLElement).getByRole('button'));
  await screen.findByRole('heading', { name: t('performance.nav.new') });
});

test('a filter without matches says so, not that nothing is saved', async () => {
  await setup();
  await fireEvent.click(screen.getByRole('radio', { name: 'RAM' }));
  await fireEvent.click(within(items()[0]).getByRole('button', { name: t('performance.history.delete') }));
  await fireEvent.click(within(items()[0]).getByRole('button', { name: t('performance.history.delete') }));
  await screen.findByText(t('performance.history.noMatch'));
  expect(screen.queryByText(t('performance.history.empty'))).toBeNull();
});

test('only an errors_core verdict gets a core term', async () => {
  const backend: FakeBackend = await connectSettings();
  backend.performanceSessions = [summary({ id: 'f', outcome: 'failed_to_start', verdict: 'failed_to_start', params: { reason: 'core 3 is parked' } })];
  render(PerformanceView, { backend, store: new LiveStore(), page: 'history' });
  // The store keeps the previous list until the new one arrives.
  await screen.findByText(/core 3 is parked/);
  const row = screen.getAllByRole('listitem')[0];
  expect(row.querySelector('.term')).toBeNull();
});

test('a failing delete shows the message and the confirmation gives way', async () => {
  const backend = await setup();
  backend.performanceDelete = async () => {
    throw 'disk locked';
  };
  await fireEvent.click(within(items()[1]).getByRole('button', { name: t('performance.history.delete') }));
  const confirm = within(items()[1]).getAllByRole('button', { name: t('performance.history.delete') })[0];
  expect(document.activeElement).toBe(confirm);
  await fireEvent.click(confirm);
  expect((await screen.findByRole('alert')).textContent).toContain('disk locked');
});

test('repeat of a session that is gone says so', async () => {
  const backend = await setup();
  await fireEvent.click(within(items()[0]).getByRole('button', { name: t('performance.history.repeat') }));
  expect((await screen.findByRole('alert')).textContent).toBe(t('performance.history.missing'));
  expect(backend.performanceStartRequests).toHaveLength(0);
});

test('repeat while a test runs says busy', async () => {
  const backend = await setup();
  backend.emitPerformanceStatus(makeRunStatus({ state: 'running' }));
  await fireEvent.click(await screen.findByRole('button', { name: t('performance.nav.history') }));
  await fireEvent.click(within(items()[1]).getByRole('button', { name: t('performance.history.repeat') }));
  expect((await screen.findByRole('alert')).textContent).toBe(t('performance.wizard.busy'));
});
