import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n, t } from '../lib/i18n/index.svelte';
import type { ServiceState, ServiceStatus } from '../lib/types';
import TopBar from './TopBar.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

const NOT_CONNECTED: ServiceState[] = ['notInstalled', 'antiCheat', 'starting', 'unreachable', 'incompatible'];

function setup(service: ServiceStatus | null, overrides: Partial<{ onLeaveAntiCheat: () => Promise<unknown>; onStartService: () => Promise<unknown> }> = {}) {
  const onLeaveAntiCheat = overrides.onLeaveAntiCheat ?? vi.fn().mockResolvedValue(undefined);
  const onStartService = overrides.onStartService ?? vi.fn().mockResolvedValue(undefined);
  render(TopBar, { view: 'simple', onViewChange: () => {}, service, onLeaveAntiCheat, onStartService });
  return { onLeaveAntiCheat, onStartService };
}

test('the badge is hidden while connected', () => {
  setup({ state: 'connected', detail: null });
  expect(screen.queryByText(t('service.baseMode'))).toBeNull();
});

test('the badge is hidden when there is no service status yet', () => {
  setup(null);
  expect(screen.queryByText(t('service.baseMode'))).toBeNull();
});

test('the badge explains each state', () => {
  for (const state of NOT_CONNECTED) {
    setup({ state, detail: null });
    expect(screen.getByText(t('service.baseMode'))).toBeTruthy();
    expect(screen.getByText(t(`service.state.${state}`))).toBeTruthy();
    cleanup();
  }
});

test('the badge explains the detail when there is one', () => {
  setup({ state: 'unreachable', detail: 'disconnected' });
  expect(screen.getByText(t('service.state.unreachable'))).toBeTruthy();
  expect(screen.getByText(t('service.detail.disconnected'))).toBeTruthy();
});

test('the badge offers to leave anti-cheat mode', async () => {
  const { onLeaveAntiCheat } = setup({ state: 'antiCheat', detail: null });
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.leaveAntiCheat') }));
  expect(onLeaveAntiCheat).toHaveBeenCalledTimes(1);
});

test('the badge offers to start an unreachable service', async () => {
  const { onStartService } = setup({ state: 'unreachable', detail: null });
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.start') }));
  expect(onStartService).toHaveBeenCalledTimes(1);
});

test('no action while starting, not installed or incompatible', () => {
  for (const state of ['starting', 'notInstalled', 'incompatible'] as ServiceState[]) {
    setup({ state, detail: null });
    expect(screen.queryByRole('button', { name: t('service.action.leaveAntiCheat') })).toBeNull();
    expect(screen.queryByRole('button', { name: t('service.action.start') })).toBeNull();
    cleanup();
  }
});

test('command failures are shown without a false success', async () => {
  const onLeaveAntiCheat = vi.fn().mockRejectedValue(new Error('persist_failed'));
  setup({ state: 'antiCheat', detail: null }, { onLeaveAntiCheat });
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.leaveAntiCheat') }));
  await vi.waitFor(() => expect(screen.getByText(t('service.action.failed'))).toBeTruthy());
  expect(screen.queryByText('persist_failed')).toBeNull();
});

test('anti-cheat stop failure is visible', async () => {
  const onLeaveAntiCheat = vi.fn().mockRejectedValue(new Error('stop_failed'));
  setup({ state: 'antiCheat', detail: null }, { onLeaveAntiCheat });
  expect(screen.queryByText(t('service.action.failed'))).toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.leaveAntiCheat') }));
  await vi.waitFor(() => expect(screen.getByText(t('service.action.failed'))).toBeTruthy());
});
