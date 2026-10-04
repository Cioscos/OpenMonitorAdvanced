import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import App from '../../App.svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { MockSettings } from '../../lib/backend/mockSettings';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import type { Persistence, ServiceStatus, SettingsPatch } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { disconnectSettings } from '../../test/settings';
import SettingsView from './SettingsView.svelte';

const CONNECTED: ServiceStatus = { state: 'connected', detail: null, pawnIo: 'ok', sources: null };

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

async function setup(persistence?: Persistence) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  if (persistence) backend.settings = new MockSettings(persistence);
  await settings.connect(backend);
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  const patches: SettingsPatch[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (patch) => {
    patches.push(patch);
    return update(patch);
  };
  const onBack = vi.fn();
  render(SettingsView, { store, backend, service: CONNECTED, onBack });
  return { backend, store, patches, onBack };
}

test('gear opens settings and Escape returns to the previous view', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  render(App, { backend, store: new LiveStore() });
  const advanced = await screen.findByRole('tab', { name: t('view.advanced') });
  await fireEvent.click(advanced);
  await vi.waitFor(() => expect(screen.getByRole('navigation', { name: t('advanced.sidebar') })).toBeTruthy());

  const gear = screen.getByRole('button', { name: t('settings.title') });
  expect((gear as HTMLButtonElement).disabled).toBe(false);
  await fireEvent.click(gear);
  expect(screen.getByRole('navigation', { name: t('settings.sections') })).toBeTruthy();
  expect(gear.getAttribute('aria-pressed')).toBe('true');
  expect(screen.queryByRole('navigation', { name: t('advanced.sidebar') })).toBeNull();

  await fireEvent.keyDown(window, { key: 'Escape' });
  await vi.waitFor(() => expect(screen.getByRole('navigation', { name: t('advanced.sidebar') })).toBeTruthy());
  expect(screen.queryByRole('navigation', { name: t('settings.sections') })).toBeNull();
  expect(document.activeElement).toBe(gear);
  // The settings screen is never remembered as the last view.
  expect(settings.state?.settings.view.last).toBe('advanced');

  await fireEvent.click(gear);
  await fireEvent.click(screen.getByRole('button', { name: t('settings.back') }));
  await vi.waitFor(() => expect(screen.getByRole('navigation', { name: t('advanced.sidebar') })).toBeTruthy());
  expect(document.activeElement).toBe(gear);
});

test('each control sends its patch', async () => {
  const { patches, backend } = await setup();
  const radio = (name: string) => fireEvent.click(screen.getByRole('radio', { name }));
  const select = (label: string, value: string) => fireEvent.change(screen.getByLabelText(label), { target: { value } });

  await radio(t('settings.general.language.en'));
  await radio(t('settings.general.temperature.f'));
  await radio(t('settings.general.throughput.bytes'));
  await select(t('settings.general.interval'), '2500');
  await radio(t('settings.general.fps.value', { fps: 30 }));
  await radio(t('settings.general.defaultView.advanced'));
  await fireEvent.click(screen.getByRole('switch', { name: t('settings.general.closeToTray') }));
  await fireEvent.click(screen.getByRole('switch', { name: t('settings.general.autostart') }));
  await select(t('settings.general.iconSensor'), 'cpu/0/load/total');
  await vi.waitFor(() => expect(settings.state?.settings.tray.iconSensor).toBe('cpu/0/load/total'));
  await select(t('settings.general.iconSensor'), '');

  await vi.waitFor(() => expect(patches).toHaveLength(10));
  expect(patches).toEqual([
    { general: { language: 'en' } },
    { general: { temperatureUnit: 'f' } },
    { general: { throughputUnit: 'bytes' } },
    { general: { intervalMs: 2500 } },
    { general: { chartFps: 30 } },
    { general: { defaultView: 'advanced' } },
    { tray: { closeToTray: false } },
    { tray: { autostart: true } },
    { tray: { iconSensor: 'cpu/0/load/total' } },
    { tray: { iconSensor: null } },
  ]);
  // The start-up entry is read again when the section opens and after the switch.
  await vi.waitFor(() => expect(backend.refreshAutostartCalls).toBe(2));
});

test('fps 60 shows the CPU hint', async () => {
  await setup();
  const sixty = screen.getByRole('radio', { name: t('settings.general.fps.value', { fps: 60 }) });
  const hint = screen.getByText(t('settings.general.fps.hint60'));
  expect(sixty.getAttribute('aria-describedby')).toContain(hint.id);
  expect(screen.getByRole('radio', { name: t('settings.general.fps.value', { fps: 30 }) }).getAttribute('aria-describedby') ?? '').not.toContain(hint.id);
});

test('field error is shown next to the field', async () => {
  const { backend } = await setup();
  backend.updateSettings = async () => {
    throw { field: 'general.intervalMs', key: 'settings.error.range' };
  };
  const interval = screen.getByLabelText(t('settings.general.interval')) as HTMLSelectElement;
  await fireEvent.change(interval, { target: { value: '2500' } });

  const error = await screen.findByText(t('settings.error.range'));
  expect(interval.closest('.field')?.contains(error)).toBe(true);
  expect(interval.getAttribute('aria-invalid')).toBe('true');
  expect(interval.getAttribute('aria-describedby')).toContain(error.id);
  // The control shows the value that is really in effect.
  await vi.waitFor(() => expect(interval.value).toBe('1000'));
  expect(screen.queryAllByText(t('settings.error.range'))).toHaveLength(1);
});

test('persistence notice for recovered, readOnly and error', async () => {
  const path = 'C:\\Users\\me\\AppData\\Roaming\\OpenMonitorAdvanced\\settings.json.bad-1';
  const cases: [Persistence, string][] = [
    [{ kind: 'recovered', path }, t('settings.persistence.recovered', { path })],
    [{ kind: 'readOnly', reason: 'newer_version' }, t('settings.persistence.readOnly')],
    [{ kind: 'error', reason: 'access denied' }, t('settings.persistence.error', { reason: 'access denied' })],
  ];
  for (const [persistence, text] of cases) {
    await setup(persistence);
    expect(screen.getByText(text)).toBeTruthy();
    cleanup();
    disconnectSettings();
  }
  for (const persistence of [{ kind: 'ok' }, { kind: 'pending' }] as Persistence[]) {
    await setup(persistence);
    expect(document.querySelector('.persistence')).toBeNull();
    cleanup();
    disconnectSettings();
  }
});

test('autostart shows what Windows makes of the entry', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.autostart = { configured: true, effective: 'disabledByWindows', error: null };
  await settings.connect(backend);
  await settings.update({ tray: { autostart: true } });
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  render(SettingsView, { store, backend, service: CONNECTED, onBack: () => {} });

  expect(await screen.findByText(t('settings.general.autostart.disabledByWindows'))).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: t('settings.general.autostart.openStartupSettings') }));
  expect(backend.openKnownPathCalls).toEqual(['startupAppsSettings']);

  backend.autostart = { configured: true, effective: 'unknown', error: null };
  cleanup();
  render(SettingsView, { store, backend, service: CONNECTED, onBack: () => {} });
  expect(await screen.findByText(t('settings.general.autostart.managedByWindows'))).toBeTruthy();
});

test('the tray icon sensor lists temperature and load sensors by device', async () => {
  await setup();
  const select = screen.getByLabelText(t('settings.general.iconSensor')) as HTMLSelectElement;
  expect(select.value).toBe('');
  expect(select.options[0].textContent).toBe(t('settings.general.iconSensor.auto'));
  const groups = [...select.querySelectorAll('optgroup')].map((g) => g.label);
  expect(groups).toEqual(['Mock Ryzen 7 7800X3D', 'Mock GeForce RTX 4080', 'RAM', 'Disk 0 (C:)']);
  const values = [...select.options].map((o) => o.value);
  expect(values).toContain('gpu/pci-0000:01:00.0/temperature/core');
  expect(values).not.toContain('gpu/pci-0000:01:00.0/clock/core');
  expect(values).not.toContain('network/mock-eth/throughput/down');
});

test('about shows versions and opens known paths', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.appInfo = { ...backend.appInfo, version: '1.2.3', serviceVersion: '4.5.6', protocolVersion: 2 };
  await settings.connect(backend);
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  render(SettingsView, { store, backend, service: CONNECTED, onBack: () => {} });
  await fireEvent.click(screen.getByRole('button', { name: t('settings.section.about') }));

  const about = await screen.findByRole('region', { name: t('settings.section.about') });
  await vi.waitFor(() => expect(within(about).getByText('1.2.3')).toBeTruthy());
  expect(within(about).getByText('4.5.6')).toBeTruthy();
  expect(within(about).getByText('2')).toBeTruthy();
  expect(within(about).getByText('GPL-3.0-or-later')).toBeTruthy();
  expect(within(about).getByText(backend.appInfo.settingsPath!)).toBeTruthy();
  expect(within(about).getByText(backend.appInfo.logsPath!)).toBeTruthy();

  await fireEvent.click(within(about).getByRole('button', { name: t('settings.about.thirdParty') }));
  await fireEvent.click(within(about).getByRole('button', { name: `${t('settings.about.openFolder')} ${t('settings.about.settingsFolder')}` }));
  await fireEvent.click(within(about).getByRole('button', { name: `${t('settings.about.openFolder')} ${t('settings.about.logsFolder')}` }));
  expect(backend.openKnownPathCalls).toEqual(['thirdPartyNotices', 'settingsFolder', 'logsFolder']);

  backend.openKnownPathError = 'The system cannot find the path specified. (os error 3)';
  await fireEvent.click(within(about).getByRole('button', { name: `${t('settings.about.openFolder')} ${t('settings.about.logsFolder')}` }));
  expect(await within(about).findByText(t('settings.openFailed', { reason: backend.openKnownPathError }))).toBeTruthy();
});

test('licence row has notices and texts buttons', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await settings.connect(backend);
  const store = new LiveStore();
  render(SettingsView, { store, backend, service: null, onBack: () => {} });
  await fireEvent.click(screen.getByRole('button', { name: t('settings.section.about') }));
  const about = await screen.findByRole('region', { name: t('settings.section.about') });
  expect(within(about).getByRole('button', { name: t('settings.about.thirdParty') })).toBeTruthy();
  await fireEvent.click(within(about).getByRole('button', { name: t('settings.about.licenseTexts') }));
  expect(backend.openKnownPathCalls).toEqual(['thirdPartyLicenses']);
});

test('about without a service version says so', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await settings.connect(backend);
  const store = new LiveStore();
  render(SettingsView, { store, backend, service: null, onBack: () => {} });
  await fireEvent.click(screen.getByRole('button', { name: t('settings.section.about') }));
  expect(await screen.findByText(t('settings.about.serviceUnknown'))).toBeTruthy();
});

test('sections are reachable from the keyboard and marked as current', async () => {
  await setup();
  const general = screen.getByRole('button', { name: t('settings.section.general') });
  expect(general.getAttribute('aria-current')).toBe('page');
  const sources = screen.getByRole('button', { name: t('settings.section.sources') });
  sources.focus();
  await fireEvent.click(sources);
  flushSync();
  expect(sources.getAttribute('aria-current')).toBe('page');
  expect(screen.getByRole('region', { name: t('settings.section.sources') })).toBeTruthy();
});

test('back_returns_to_advanced', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  render(App, { backend, store: new LiveStore() });
  await fireEvent.click(await screen.findByRole('tab', { name: t('view.advanced') }));
  const create = await screen.findAllByRole('button', { name: t('advanced.table.createRule') });
  await fireEvent.click(create[0]);

  // Settings on Rules, with "New rule" already holding the sensor of that row.
  expect(screen.getByRole('heading', { name: t('settings.section.rules') })).toBeTruthy();
  expect(screen.getByRole('group', { name: t('rules.new') })).toBeTruthy();
  await vi.waitFor(() => expect((screen.getByLabelText(t('rules.editor.sensor')) as HTMLSelectElement).value).not.toBe(''));
  expect(screen.queryByRole('navigation', { name: t('advanced.sidebar') })).toBeNull();

  await fireEvent.click(screen.getByRole('button', { name: t('settings.back') }));
  await vi.waitFor(() => expect(screen.getByRole('navigation', { name: t('advanced.sidebar') })).toBeTruthy());

  // Esc does the same, and the gear opens the general section again.
  await fireEvent.click(screen.getAllByRole('button', { name: t('advanced.table.createRule') })[0]);
  await fireEvent.keyDown(window, { key: 'Escape' });
  await vi.waitFor(() => expect(screen.getByRole('navigation', { name: t('advanced.sidebar') })).toBeTruthy());
  await fireEvent.click(screen.getByRole('button', { name: t('settings.title') }));
  expect(screen.getByRole('heading', { name: t('settings.section.general') })).toBeTruthy();
});

test('the new-rule sensor prefills the draft only once', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  render(App, { backend, store: new LiveStore() });
  await fireEvent.click(await screen.findByRole('tab', { name: t('view.advanced') }));
  const create = await screen.findAllByRole('button', { name: t('advanced.table.createRule') });
  await fireEvent.click(create[0]);
  await vi.waitFor(() => expect(screen.getByRole('group', { name: t('rules.new') })).toBeTruthy());

  // Leaving Rules and coming back does not reopen a pre-filled draft.
  await fireEvent.click(screen.getByRole('button', { name: t('settings.section.general') }));
  await fireEvent.click(screen.getByRole('button', { name: t('settings.section.rules') }));
  flushSync();
  expect(screen.getByRole('heading', { name: t('settings.section.rules') })).toBeTruthy();
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(screen.queryByRole('group', { name: t('rules.new') })).toBeNull();
});
