import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { log } from '../../lib/log.svelte';
import { settings } from '../../lib/settings.svelte';
import type { HotkeyStatus, LogStatus, SettingsPatch } from '../../lib/types';
import { FakeBackend, makeLogStatus } from '../../test/fake-backend';
import { disconnectSettings } from '../../test/settings';
import LogSection from './LogSection.svelte';
import SettingsView from './SettingsView.svelte';

let stopLog: (() => void) | undefined;

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  stopLog?.();
  stopLog = undefined;
  disconnectSettings();
});

const hotkey = (over: Partial<HotkeyStatus> = {}): HotkeyStatus => ({ requested: null, effective: null, state: 'unset', reason: null, ...over });

async function setup(seed?: SettingsPatch, status?: Partial<LogStatus>) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  if (status) backend.logStatus = makeLogStatus({ revision: 1, ...status });
  await settings.connect(backend);
  if (seed) await settings.update(seed);
  stopLog = await log.connect(backend);
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  const patches: SettingsPatch[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (patch) => {
    patches.push(patch);
    return update(patch);
  };
  render(LogSection, { store, backend });
  return { backend, store, patches };
}

const ids = (device: string, category?: string) =>
  MOCK_SCHEMA.sensors.filter((s) => s.deviceId === device && (category === undefined || s.category === category)).map((s) => s.id);
const GPU = MOCK_SCHEMA.devices.find((d) => d.kind === 'gpu')!.id;

test('section_order_includes_log', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await settings.connect(backend);
  render(SettingsView, { store: new LiveStore(), backend, service: null, onBack: vi.fn() });
  const nav = screen.getByRole('navigation', { name: t('settings.sections') });
  const names = within(nav)
    .getAllByRole('button')
    .map((b) => b.textContent?.trim());
  expect(names.slice(1)).toEqual(['general', 'rules', 'log', 'sources', 'about'].map((s) => t(`settings.section.${s}`)));
  await fireEvent.click(within(nav).getByRole('button', { name: t('settings.section.log') }));
  expect(screen.getByRole('heading', { name: t('settings.section.log'), level: 2 })).toBeTruthy();
  expect(screen.getByRole('switch', { name: t('settings.log.sensors.all') })).toBeTruthy();
});

test('folder_pick_saves_and_reset_clears', async () => {
  const { backend, patches } = await setup();
  expect(screen.getByText(t('settings.log.folder.default'))).toBeTruthy();
  backend.pickedLogFolder = 'D:\\Logs';
  await fireEvent.click(screen.getByRole('button', { name: t('settings.log.folder.choose') }));
  await waitFor(() => expect(patches).toEqual([{ log: { folder: 'D:\\Logs' } }]));
  await screen.findByText('D:\\Logs');
  await fireEvent.click(screen.getByRole('button', { name: t('settings.log.folder.reset') }));
  await waitFor(() => expect(patches[1]).toEqual({ log: { folder: null } }));
  await screen.findByText(t('settings.log.folder.default'));
});

test('folder_pick_cancel_does_not_reset_the_folder', async () => {
  const { backend, patches } = await setup({ log: { folder: 'E:\\Keep' } });
  patches.length = 0;
  backend.pickedLogFolder = null;
  await fireEvent.click(screen.getByRole('button', { name: t('settings.log.folder.choose') }));
  await waitFor(() => expect(backend.logCalls).toContain('pickLogFolder'));
  expect(patches).toEqual([]);
  expect(screen.getByText('E:\\Keep')).toBeTruthy();
});

test('open_folder_error_is_shown', async () => {
  const { backend } = await setup();
  backend.openLogFolderError = 'log.error.folderMissing';
  await fireEvent.click(screen.getByRole('button', { name: t('settings.log.folder.open') }));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain(t('log.error.folderMissing'));
});

test('all_sensors_switch_seeds_the_list', async () => {
  const { patches } = await setup();
  const all = screen.getByRole('switch', { name: t('settings.log.sensors.all') });
  expect(all.getAttribute('aria-checked')).toBe('true');
  const sensorGroup = () => screen.queryByRole('group', { name: t('settings.log.sensors.tree') });
  expect(sensorGroup()).toBeNull();
  await fireEvent.click(all);
  await waitFor(() => expect(patches).toEqual([{ log: { sensors: MOCK_SCHEMA.sensors.map((s) => s.id) } }]));
  await waitFor(() => expect(sensorGroup()).not.toBeNull());
  // Plain nested lists of checkboxes: no tree roles without the tree's arrow keys.
  expect(screen.queryByRole('tree')).toBeNull();
  expect(screen.queryAllByRole('treeitem')).toEqual([]);
  expect(within(sensorGroup()!).getAllByRole('checkbox').length).toBeGreaterThan(0);
  expect(screen.getByRole('switch', { name: t('settings.log.sensors.all') }).getAttribute('aria-checked')).toBe('false');
  // Turning it back on records everything again.
  await fireEvent.click(screen.getByRole('switch', { name: t('settings.log.sensors.all') }));
  await waitFor(() => expect(patches[1]).toEqual({ log: { sensors: null } }));
});

test('tree_checks_a_device_and_a_category', async () => {
  const { patches } = await setup({ log: { sensors: [] } });
  patches.length = 0;
  const eth = screen.getByRole('checkbox', { name: /Ethernet/ }) as HTMLInputElement;
  expect(eth.checked).toBe(false);
  await fireEvent.click(eth);
  await waitFor(() => expect(patches[0]).toEqual({ log: { sensors: ids('network/mock-eth') } }));
  await waitFor(() => expect((screen.getByRole('checkbox', { name: /Ethernet/ }) as HTMLInputElement).checked).toBe(true));

  const gpu = screen.getByRole('checkbox', { name: /RTX 4080/ }).closest('li') as HTMLElement;
  await fireEvent.click(within(gpu).getByRole('checkbox', { name: t('advanced.category.temperature') }));
  await waitFor(() => expect(patches[1].log?.sensors).toEqual([...ids('network/mock-eth'), ...ids(GPU, 'temperature')]));
  // The device is now mixed: neither checked nor clear.
  await waitFor(() => {
    const box = screen.getByRole('checkbox', { name: /RTX 4080/ }) as HTMLInputElement;
    expect(box.indeterminate).toBe(true);
    expect(box.checked).toBe(false);
  });
});

test('missing_selected_ids_are_kept_and_counted', async () => {
  const { patches } = await setup({ log: { sensors: ['gone/0/load/x', 'cpu/0/load/total'] } });
  patches.length = 0;
  expect(screen.getByText(t('settings.log.sensors.missing', { count: 1 }))).toBeTruthy();
  await fireEvent.click(screen.getByRole('checkbox', { name: /Ethernet/ }));
  await waitFor(() => expect(patches[0].log?.sensors).toEqual(['gone/0/load/x', 'cpu/0/load/total', ...ids('network/mock-eth')]));
});

test('interval_shows_the_effective_time', async () => {
  await setup({ general: { intervalMs: 2000 }, log: { everyTicks: 30 } });
  expect(screen.getByText(/every 1 min/i)).toBeTruthy();
  await fireEvent.click(screen.getByRole('radio', { name: '5' }));
  await screen.findByText(/every 10 s/i);
});

test('max_size_out_of_range_shows_the_field_error', async () => {
  const { patches } = await setup();
  const field = screen.getByRole('textbox', { name: t('settings.log.maxSize') });
  await fireEvent.input(field, { target: { value: '5' } });
  await fireEvent.keyDown(field, { key: 'Enter' });
  await waitFor(() => expect(patches).toEqual([{ log: { maxFileMb: 5 } }]));
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toBe(t('settings.error.range'));
});

const toggleBox = () => screen.getByRole('textbox', { name: t('settings.log.hotkey.toggle') });

test('hotkey_capture_stores_the_canonical_form', async () => {
  const { patches } = await setup();
  patches.length = 0;
  await fireEvent.focus(toggleBox());
  await fireEvent.keyDown(toggleBox(), { code: 'ControlLeft', key: 'Control', ctrlKey: true });
  expect(patches).toEqual([]);
  await fireEvent.keyDown(toggleBox(), { code: 'KeyL', key: 'l', ctrlKey: true, shiftKey: true });
  await waitFor(() => expect(patches).toEqual([{ log: { hotkeyToggle: 'Ctrl+Shift+L' } }]));
  await fireEvent.keyDown(screen.getByRole('textbox', { name: t('settings.log.hotkey.pause') }), {
    code: 'F12',
    key: 'F12',
    ctrlKey: true,
    altKey: true,
  });
  await waitFor(() => expect(patches[1]).toEqual({ log: { hotkeyPause: 'Ctrl+Alt+F12' } }));
  // Delete clears it.
  await fireEvent.keyDown(toggleBox(), { code: 'Delete', key: 'Delete' });
  await waitFor(() => expect(patches[2]).toEqual({ log: { hotkeyToggle: null } }));
});

test('hotkey_capture_suspends_the_global_hotkeys_while_focused', async () => {
  const { backend } = await setup();
  const pauseBox = screen.getByRole('textbox', { name: t('settings.log.hotkey.pause') });
  await fireEvent.focus(toggleBox());
  expect(backend.hotkeySuspensions).toEqual([true]);
  await fireEvent.blur(toggleBox());
  await fireEvent.focus(pauseBox);
  expect(backend.hotkeySuspensions).toEqual([true, false, true]);
  await fireEvent.blur(pauseBox);
  expect(backend.hotkeySuspensions).toEqual([true, false, true, false]);
});

test('hotkey_capture_destroyed_while_focused_resumes_the_hotkeys', async () => {
  const { backend } = await setup();
  await fireEvent.focus(toggleBox());
  cleanup();
  expect(backend.hotkeySuspensions).toEqual([true, false]);
});

test('hotkey_capture_destroyed_unfocused_leaves_the_hotkeys_alone', async () => {
  const { backend } = await setup();
  cleanup();
  expect(backend.hotkeySuspensions).toEqual([]);
});

test('hotkey_capture_ignores_key_repeat', async () => {
  const { patches } = await setup({ log: { hotkeyToggle: 'Ctrl+Alt+Shift+R' } });
  patches.length = 0;
  await fireEvent.focus(toggleBox());
  await fireEvent.keyDown(toggleBox(), { code: 'KeyL', key: 'l', ctrlKey: true, shiftKey: true, repeat: true });
  await fireEvent.keyDown(toggleBox(), { code: 'Delete', key: 'Delete', repeat: true });
  await fireEvent.keyDown(toggleBox(), { code: 'Backspace', key: 'Backspace', repeat: true });
  await fireEvent.keyDown(toggleBox(), { code: 'KeyL', key: 'l', ctrlKey: true, repeat: true });
  expect(patches).toEqual([]);
  expect(screen.queryByRole('alert')).toBeNull();
  // The first press of the same keys still counts.
  await fireEvent.keyDown(toggleBox(), { code: 'KeyL', key: 'l', ctrlKey: true, shiftKey: true });
  await waitFor(() => expect(patches).toEqual([{ log: { hotkeyToggle: 'Ctrl+Shift+L' } }]));
});

test('hotkey_with_one_modifier_is_refused', async () => {
  const { patches } = await setup();
  patches.length = 0;
  await fireEvent.keyDown(toggleBox(), { code: 'KeyL', key: 'l', ctrlKey: true });
  expect(patches).toEqual([]);
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toBe(t('settings.error.hotkey'));
  // Esc gives up and drops the message.
  await fireEvent.keyDown(toggleBox(), { code: 'Escape', key: 'Escape' });
  expect(screen.queryByRole('alert')).toBeNull();
});

test('hotkey_status_in_use_shows_the_effective_one', async () => {
  await setup(undefined, {
    hotkeys: {
      toggle: hotkey({ requested: 'Ctrl+Alt+Shift+Q', effective: 'Ctrl+Alt+Shift+R', state: 'failed', reason: 'log.hotkey.inUse' }),
      pause: hotkey(),
    },
  });
  const text = document.body.textContent ?? '';
  expect(text).toContain(t('log.hotkey.inUse'));
  expect(text).toContain(t('settings.log.hotkey.keepsActive', { hotkey: 'Ctrl+Alt+Shift+R' }));
  expect(text).toContain(t('settings.log.hotkey.unset'));
});
