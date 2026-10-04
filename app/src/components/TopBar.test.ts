import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../lib/backend/mock';
import { updates } from '../lib/updates.svelte';
import { FakeBackend } from '../test/fake-backend';
import { i18n, t } from '../lib/i18n/index.svelte';
import type { ServiceState, ServiceStatus, UpdateStatus } from '../lib/types';
import TopBar from './TopBar.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

const NOT_CONNECTED: ServiceState[] = ['notInstalled', 'antiCheat', 'starting', 'unreachable', 'incompatible'];

function setup(service: ServiceStatus | null, overrides: Partial<{ onLeaveAntiCheat: () => Promise<unknown>; onStartService: () => Promise<unknown> }> = {}) {
  const onLeaveAntiCheat = overrides.onLeaveAntiCheat ?? vi.fn().mockResolvedValue(undefined);
  const onStartService = overrides.onStartService ?? vi.fn().mockResolvedValue(undefined);
  render(TopBar, { view: 'simple', onViewChange: () => {}, onSettings: () => {}, service, onLeaveAntiCheat, onStartService, onOpenLogFolder: async () => {} });
  return { onLeaveAntiCheat, onStartService };
}

test('the gear opens the settings and shows when they are open', async () => {
  const onSettings = vi.fn();
  const props = { onViewChange: () => {}, onSettings, service: null, onLeaveAntiCheat: vi.fn(), onStartService: vi.fn(), onOpenLogFolder: vi.fn() };
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

test('the badge stays hidden while connected with PawnIO working', () => {
  setup({ state: 'connected', detail: null, pawnIo: 'ok', sources: null });
  expect(document.querySelector('details.badge')).toBeNull();
});

test('a PawnIO problem stays visible in the badge while connected (spec §2.8)', () => {
  for (const pawnIo of ['missing', 'unavailable', 'rebootPending', 'unknown'] as const) {
    setup({ state: 'connected', detail: null, pawnIo, sources: null });
    const details = document.querySelector('details.badge') as HTMLDetailsElement;
    expect(details).toBeTruthy();
    // Connected is not basic mode: the badge names the driver instead.
    expect(details.querySelector('summary')?.textContent).toBe(t('settings.sources.pawnIo'));
    expect(within(details).getByText(t(`settings.sources.pawnIo.${pawnIo}`))).toBeTruthy();
    expect(within(details).queryByRole('button')).toBeNull();
    cleanup();
  }
});

test('a pending PawnIO install asks for a restart, not a shutdown', () => {
  setup({ state: 'connected', detail: null, pawnIo: 'rebootPending', sources: null });
  const text = screen.getByText(t('settings.sources.pawnIo.rebootPending')).textContent ?? '';
  expect(text).toMatch(/restart/i);
  expect(text).toMatch(/not a shutdown/i);
});

test('the badge of a non-connected state keeps its basic-mode text without a PawnIO line', () => {
  setup({ state: 'unreachable', detail: null, pawnIo: 'missing', sources: null });
  expect(screen.getByText(t('service.baseMode'))).toBeTruthy();
  expect(screen.queryByText(t('settings.sources.pawnIo.missing'))).toBeNull();
});

describe('update dot on the gear', () => {
  let off: (() => void) | undefined;
  afterEach(() => {
    off?.();
    off = undefined;
  });

  async function connect(initial: UpdateStatus) {
    const backend = new FakeBackend(MOCK_SCHEMA);
    backend.updateStatus = initial;
    off = await updates.connect(backend);
    return backend;
  }
  const upd = (over: Partial<UpdateStatus> = {}): UpdateStatus => ({ state: 'idle', current: '0.4.0', latest: null, checkedAtMs: null, error: null, ...over });
  const gear = () => screen.getByRole('button', { name: new RegExp(t('settings.title')) });
  const WITH_UPDATE = () => t('settings.titleWithUpdate');

  test('no dot and no text without a newer version', async () => {
    await connect(upd({ state: 'upToDate' }));
    setup(null);
    expect(gear().querySelector('.dot')).toBeNull();
    expect(gear().getAttribute('title')).toBe(t('settings.title'));
    expect(screen.getByRole('button', { name: t('settings.title') })).toBeTruthy();
  });

  test('dot and accessible text with a newer version, also after an error', async () => {
    await connect(upd({ state: 'error', error: 'offline', latest: { version: '0.5.0' } }));
    setup(null);
    expect(gear().getAttribute('title')).toBe(WITH_UPDATE());
    expect(gear().getAttribute('aria-label')).toBeNull();
    expect(gear().querySelector('.dot')?.getAttribute('aria-hidden')).toBe('true');
    expect(screen.getByRole('button', { name: WITH_UPDATE() })).toBeTruthy();
  });

  test('the dot appears after an update-status event', async () => {
    const backend = await connect(upd());
    setup(null);
    expect(gear().querySelector('.dot')).toBeNull();
    backend.emitUpdateStatus(upd({ state: 'available', latest: { version: '0.5.0' } }));
    expect(await screen.findByRole('button', { name: WITH_UPDATE() })).toBeTruthy();
    expect(gear().querySelector('.dot')).not.toBeNull();
  });
});
