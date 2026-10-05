import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { overlay } from '../../lib/overlay.svelte';
import { settings } from '../../lib/settings.svelte';
import type { OverlayStatus, SettingsPatch } from '../../lib/types';
import { FakeBackend, makeOverlayStatus } from '../../test/fake-backend';
import { disconnectSettings } from '../../test/settings';
import OverlaySection from './OverlaySection.svelte';
import SettingsView from './SettingsView.svelte';

const USER_PROFILE = '0f9c2b7e-51a4-4d3e-9a8b-2c1d0e6f7a10';

let stopOverlay: (() => void) | undefined;

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  stopOverlay?.();
  stopOverlay = undefined;
  disconnectSettings();
});

async function setup(seed?: SettingsPatch, status?: Partial<OverlayStatus>) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  if (status) backend.overlayStatus = makeOverlayStatus(status);
  await settings.connect(backend);
  if (seed) await settings.update(seed);
  stopOverlay = await overlay.connect(backend);
  const patches: SettingsPatch[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (patch) => {
    patches.push(patch);
    return update(patch);
  };
  render(OverlaySection, { backend });
  return { backend, patches };
}

const GAME = { name: 'EldenRing.exe', pid: 4242 };
const button = (key: string) => screen.getByRole('button', { name: t(key) }) as HTMLButtonElement;

test('section order includes overlay between log and sources', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await settings.connect(backend);
  render(SettingsView, { store: new LiveStore(), backend, service: null, onBack: vi.fn() });
  const nav = screen.getByRole('navigation', { name: t('settings.sections') });
  const names = within(nav)
    .getAllByRole('button')
    .map((b) => b.textContent?.trim());
  expect(names.slice(1)).toEqual(['general', 'rules', 'log', 'overlay', 'benchmark', 'sources', 'about'].map((s) => t(`settings.section.${s}`)));
  await fireEvent.click(within(nav).getByRole('button', { name: t('settings.section.overlay') }));
  expect(screen.getByRole('switch', { name: t('overlay.enabled') })).toBeTruthy();
});

test('enabled switch patches the setting', async () => {
  const { patches } = await setup();
  const enabled = screen.getByRole('switch', { name: t('overlay.enabled') });
  expect(enabled.getAttribute('aria-checked')).toBe('false');
  await fireEvent.click(enabled);
  await waitFor(() => expect(patches).toEqual([{ overlay: { enabled: true } }]));
});

test('shows the service text when frames are unavailable', async () => {
  await setup({ overlay: { enabled: true } }, { enabled: true, frames: 'unavailable', process: 'running' });
  expect(screen.getByText(t('overlay.state.unavailable'))).toBeTruthy();
  expect(screen.queryByText(t('overlay.state.running'))).toBeNull();
});

test('no engine or process state while the overlay is off', async () => {
  const { backend } = await setup(undefined, { enabled: false, frames: 'unavailable', process: 'off' });
  expect(screen.queryByText(t('overlay.state.unavailable'))).toBeNull();
  expect(screen.queryByRole('status')).toBeNull();
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: true, frames: 'unavailable', process: 'running' }));
  await screen.findByText(t('overlay.state.unavailable'));
});

test('process failures have their own text', async () => {
  const { backend } = await setup(undefined, { enabled: true, frames: 'running', process: 'failed', processReason: 'crashing' });
  expect(screen.getByText(t('overlay.state.processFailed'))).toBeTruthy();
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: true, frames: 'running', process: 'failed', processReason: 'incompatible' }));
  await screen.findByText(t('overlay.state.incompatible'));
});

test('retry visible only on failed or denied', async () => {
  const { backend } = await setup(undefined, { enabled: true, frames: 'running', process: 'running' });
  const retry = () => screen.queryByRole('button', { name: t('overlay.retry') });
  expect(retry()).toBeNull();
  for (const frames of ['off', 'starting', 'unavailable', 'missing', 'tampered'] as const) {
    backend.emitOverlayStatus(makeOverlayStatus({ enabled: true, frames }));
    await waitFor(() => expect(screen.getByText(t(`overlay.state.${frames}`))).toBeTruthy());
    expect(retry()).toBeNull();
  }
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: true, frames: 'denied' }));
  await waitFor(() => expect(retry()).not.toBeNull());
  expect(screen.getByText(t('overlay.state.denied'))).toBeTruthy();
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: true, frames: 'failed' }));
  await waitFor(() => expect(screen.getByText(t('overlay.state.failed'))).toBeTruthy());
  await fireEvent.click(retry()!);
  expect(backend.overlayCalls).toContain('overlayRetry');
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: true, frames: 'running', process: 'failed', processReason: 'crashing' }));
  await waitFor(() => expect(screen.getByText(t('overlay.state.processFailed'))).toBeTruthy());
  expect(retry()).not.toBeNull();
});

test('an overlay hidden by the user says so and offers to show it', async () => {
  const { backend } = await setup(undefined, { enabled: true, frames: 'running', process: 'running', hiddenByUser: false });
  const show = () => screen.queryByRole('button', { name: t('overlay.show') });
  expect(screen.queryByText(t('overlay.hidden'))).toBeNull();
  expect(show()).toBeNull();
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: true, frames: 'running', process: 'running', hiddenByUser: true }));
  await screen.findByText(t('overlay.hidden'));
  expect(screen.getByText(t('overlay.state.running'))).toBeTruthy();
  await fireEvent.click(show()!);
  expect(backend.overlayCalls).toContain('setOverlayHidden:false');
  // With the overlay off there is nothing to show.
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: false, frames: 'off', process: 'off', hiddenByUser: true }));
  await waitFor(() => expect(screen.queryByText(t('overlay.hidden'))).toBeNull());
});

test('associate patches gameProfiles with the whole map', async () => {
  const { patches } = await setup(
    { overlay: { gameProfiles: { 'cs2.exe': 'builtin-bar' } } },
    { enabled: true, frames: 'running', target: GAME, activeProfile: 'builtin-full' },
  );
  patches.length = 0;
  expect(screen.getByText(t('overlay.current', { name: GAME.name }))).toBeTruthy();
  await fireEvent.click(button('overlay.associate'));
  await waitFor(() =>
    expect(patches).toEqual([{ overlay: { gameProfiles: { 'cs2.exe': 'builtin-bar', 'eldenring.exe': 'builtin-full' } } }]),
  );
});

test('block patches blockedGames with the whole list', async () => {
  const { patches } = await setup({ overlay: { blockedGames: ['cs2.exe'] } }, { enabled: true, target: GAME });
  patches.length = 0;
  await fireEvent.click(button('overlay.block'));
  await waitFor(() => expect(patches).toEqual([{ overlay: { blockedGames: ['cs2.exe', 'eldenring.exe'] } }]));
  // Already excluded: nothing more to do.
  await waitFor(() => expect(button('overlay.block').disabled).toBe(true));
});

test('buttons disabled without a target', async () => {
  await setup(undefined, { enabled: true, target: null });
  expect(screen.getByText(t('overlay.current.none'))).toBeTruthy();
  expect(button('overlay.associate').disabled).toBe(true);
  expect(button('overlay.block').disabled).toBe(true);
});

test('remove patches without the entry', async () => {
  const { patches } = await setup({
    overlay: { gameProfiles: { 'cs2.exe': 'builtin-bar', 'eldenring.exe': 'builtin-full' }, blockedGames: ['cs2.exe', 'doom.exe'] },
  });
  patches.length = 0;
  const profiles = screen.getByRole('list', { name: t('overlay.gameProfiles') });
  const cs2 = within(profiles).getByText('cs2.exe').closest('li') as HTMLElement;
  expect(cs2.textContent).toContain(t('overlay.template.builtin-bar'));
  await fireEvent.click(within(cs2).getByRole('button', { name: new RegExp(t('overlay.remove')) }));
  await waitFor(() => expect(patches).toEqual([{ overlay: { gameProfiles: { 'eldenring.exe': 'builtin-full' } } }]));

  const blocked = screen.getByRole('list', { name: t('overlay.blockedGames') });
  const doom = within(blocked).getByText('doom.exe').closest('li') as HTMLElement;
  await fireEvent.click(within(doom).getByRole('button', { name: new RegExp(t('overlay.remove')) }));
  await waitFor(() => expect(patches[1]).toEqual({ overlay: { blockedGames: ['cs2.exe'] } }));
});

test('builtin profile names are translated', async () => {
  const { patches } = await setup({ general: { language: 'it' } }, {
    profiles: [
      { id: 'builtin-gaming', name: 'overlay.template.builtin-gaming', builtin: true },
      { id: 'builtin-bar', name: 'overlay.template.builtin-bar', builtin: true },
      { id: USER_PROFILE, name: 'My stream layout', builtin: false },
    ],
    diagnostics: [{ file: 'broken.json', reason: 'unknown field `colour`' }],
  });
  patches.length = 0;
  const select = screen.getByRole('combobox', { name: t('overlay.defaultProfile') }) as HTMLSelectElement;
  const labels = Array.from(select.options).map((o) => o.textContent?.trim());
  expect(labels).toEqual([t('overlay.template.builtin-gaming'), t('overlay.template.builtin-bar'), 'My stream layout']);
  expect(labels[1]).toBe('Barra orizzontale');
  expect(select.value).toBe('builtin-gaming');
  await fireEvent.change(select, { target: { value: USER_PROFILE } });
  await waitFor(() => expect(patches).toEqual([{ overlay: { defaultProfile: USER_PROFILE } }]));
  expect(screen.getByText(t('overlay.profileInvalid', { file: 'broken.json', reason: 'unknown field `colour`' }))).toBeTruthy();
});

test('reload asks the backend', async () => {
  const { backend } = await setup();
  await fireEvent.click(button('overlay.reload'));
  expect(backend.overlayCalls).toContain('overlayReloadProfiles');
});

test('draw and measure controls patch their keys', async () => {
  const { patches } = await setup();
  patches.length = 0;
  await fireEvent.click(screen.getByRole('radio', { name: t('settings.general.fps.value', { fps: 15 }) }));
  await fireEvent.click(screen.getByRole('radio', { name: t('overlay.textHz.value', { n: 4 }) }));
  await fireEvent.click(screen.getByRole('radio', { name: t('overlay.attach.monitor') }));
  await fireEvent.click(screen.getByRole('switch', { name: t('overlay.hideFromCapture') }));
  await fireEvent.click(screen.getByRole('switch', { name: t('overlay.trackPcLatency') }));
  await fireEvent.click(screen.getByRole('switch', { name: t('overlay.trackGpu') }));
  await waitFor(() => expect(patches.length).toBe(6));
  expect(patches).toEqual([
    { overlay: { chartFps: 15 } },
    { overlay: { textHz: 4 } },
    { overlay: { attach: 'monitor' } },
    { overlay: { hideFromCapture: true } },
    { overlay: { trackPcLatency: true } },
    { overlay: { trackGpu: true } },
  ]);
});

test('hotkey capture suspends hotkeys', async () => {
  const { backend, patches } = await setup();
  patches.length = 0;
  const toggle = screen.getByRole('textbox', { name: t('overlay.hotkeyToggle') });
  const next = screen.getByRole('textbox', { name: t('overlay.hotkeyNextProfile') });
  toggle.focus();
  expect(backend.hotkeySuspensions).toEqual([true]);
  await fireEvent.keyDown(toggle, { code: 'KeyO', key: 'o', ctrlKey: true, shiftKey: true });
  await waitFor(() => expect(patches).toEqual([{ overlay: { hotkeyToggle: 'Ctrl+Shift+O' } }]));
  expect(backend.hotkeySuspensions).toEqual([true, false]);
  await fireEvent.focus(next);
  await fireEvent.blur(next);
  expect(backend.hotkeySuspensions).toEqual([true, false, true, false]);
});

test('open editor calls the backend', async () => {
  const { backend } = await setup();
  await fireEvent.click(button('overlay.openEditor'));
  expect(backend.editorCalls).toContain('openOverlayEditor');
});

test('benchmark hotkey capture suspends hotkeys', async () => {
  const { backend } = await setup();
  const input = screen.getByRole('textbox', { name: t('overlay.hotkeyBenchmark') });
  await fireEvent.focus(input);
  await waitFor(() => expect(backend.hotkeySuspensions).toEqual([true]));
});

test('retry and reload errors are shown', async () => {
  const { backend } = await setup({ overlay: { enabled: true } }, { enabled: true, frames: 'failed', process: 'running' });
  backend.overlayReloadProfiles = async () => Promise.reject('shell.error.missing');
  await fireEvent.click(button('overlay.reload'));
  await screen.findByText(t('overlay.error', { detail: t('shell.error.missing') }));
  backend.overlayRetry = async () => Promise.reject(new Error('boom'));
  await fireEvent.click(button('overlay.retry'));
  await screen.findByText(t('overlay.error', { detail: 'boom' }));
});
