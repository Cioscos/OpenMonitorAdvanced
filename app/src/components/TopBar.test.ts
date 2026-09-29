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
  render(TopBar, { view: 'simple', onViewChange: () => {}, onSettings: () => {}, service, onLeaveAntiCheat, onStartService });
  return { onLeaveAntiCheat, onStartService };
}

test('the gear opens the settings and shows when they are open', async () => {
  const onSettings = vi.fn();
  const props = { onViewChange: () => {}, onSettings, service: null, onLeaveAntiCheat: vi.fn(), onStartService: vi.fn() };
  const { rerender } = render(TopBar, { view: 'simple', ...props });
  const gear = screen.getByRole('button', { name: t('settings.title') }) as HTMLButtonElement;
  expect(gear.disabled).toBe(false);
  expect(gear.getAttribute('aria-pressed')).toBe('false');
  await fireEvent.click(gear);
  expect(onSettings).toHaveBeenCalledTimes(1);
  await rerender({ view: 'settings', ...props });
  expect(gear.getAttribute('aria-pressed')).toBe('true');
  // Neither view tab is selected on the settings screen.
  expect(screen.getAllByRole('tab').map((tab) => tab.getAttribute('aria-selected'))).toEqual(['false', 'false']);
});

test('the badge is hidden while connected', () => {
  setup({ state: 'connected', detail: null, pawnIo: null, sources: null });
  expect(screen.queryByText(t('service.baseMode'))).toBeNull();
});

test('the badge is hidden when there is no service status yet', () => {
  setup(null);
  expect(screen.queryByText(t('service.baseMode'))).toBeNull();
});

test('the badge explains each state', () => {
  for (const state of NOT_CONNECTED) {
    setup({ state, detail: null, pawnIo: null, sources: null });
    expect(screen.getByText(t('service.baseMode'))).toBeTruthy();
    expect(screen.getByText(t(`service.state.${state}`))).toBeTruthy();
    cleanup();
  }
});

test('the badge explains the detail when there is one', () => {
  setup({ state: 'unreachable', detail: 'disconnected', pawnIo: null, sources: null });
  expect(screen.getByText(t('service.state.unreachable'))).toBeTruthy();
  expect(screen.getByText(t('service.detail.disconnected'))).toBeTruthy();
});

test('the badge offers to leave anti-cheat mode', async () => {
  const { onLeaveAntiCheat } = setup({ state: 'antiCheat', detail: null, pawnIo: null, sources: null });
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.leaveAntiCheat') }));
  expect(onLeaveAntiCheat).toHaveBeenCalledTimes(1);
});

test('the badge offers to start an unreachable service', async () => {
  const { onStartService } = setup({ state: 'unreachable', detail: null, pawnIo: null, sources: null });
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.start') }));
  expect(onStartService).toHaveBeenCalledTimes(1);
});

test('no action while starting, not installed or incompatible', () => {
  for (const state of ['starting', 'notInstalled', 'incompatible'] as ServiceState[]) {
    setup({ state, detail: null, pawnIo: null, sources: null });
    expect(screen.queryByRole('button', { name: t('service.action.leaveAntiCheat') })).toBeNull();
    expect(screen.queryByRole('button', { name: t('service.action.start') })).toBeNull();
    cleanup();
  }
});

test('command failures are shown without a false success', async () => {
  const onLeaveAntiCheat = vi.fn().mockRejectedValue(new Error('persist_failed'));
  setup({ state: 'antiCheat', detail: null, pawnIo: null, sources: null }, { onLeaveAntiCheat });
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.leaveAntiCheat') }));
  await vi.waitFor(() => expect(screen.getByText(t('service.action.failed'))).toBeTruthy());
  expect(screen.queryByText('persist_failed')).toBeNull();
});

test('anti-cheat stop failure is visible', async () => {
  const onLeaveAntiCheat = vi.fn().mockRejectedValue(new Error('stop_failed'));
  setup({ state: 'antiCheat', detail: null, pawnIo: null, sources: null }, { onLeaveAntiCheat });
  expect(screen.queryByText(t('service.action.failed'))).toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.leaveAntiCheat') }));
  await vi.waitFor(() => expect(screen.getByText(t('service.action.failed'))).toBeTruthy());
});

test('the badge is a closed disclosure by default and opens on a command failure (R23)', async () => {
  const onLeaveAntiCheat = vi.fn().mockRejectedValue(new Error('persist_failed'));
  setup({ state: 'antiCheat', detail: null, pawnIo: null, sources: null }, { onLeaveAntiCheat });
  const details = document.querySelector('details.badge') as HTMLDetailsElement;
  expect(details).toBeTruthy();
  expect(details.open).toBe(false);

  await fireEvent.click(screen.getByRole('button', { name: t('service.action.leaveAntiCheat') }));
  await vi.waitFor(() => expect(details.open).toBe(true));
});

test('the live status region does not wrap the action button', () => {
  setup({ state: 'antiCheat', detail: null, pawnIo: null, sources: null });
  const status = screen.getByRole('status');
  expect(status.querySelector('button')).toBeNull();
  expect(status.textContent).toContain(t('service.state.antiCheat'));
});
