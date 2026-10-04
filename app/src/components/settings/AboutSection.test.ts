import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { MOCK_SCHEMA, createMockBackend } from '../../lib/backend/mock';
import { MockSettings } from '../../lib/backend/mockSettings';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import { updates } from '../../lib/updates.svelte';
import type { UpdateStatus } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { disconnectSettings } from '../../test/settings';
import AboutSection from './AboutSection.svelte';
import SettingsView from './SettingsView.svelte';

const CHECKED_AT = Date.UTC(2026, 9, 4, 12, 30);

function status(over: Partial<UpdateStatus> = {}): UpdateStatus {
  return { state: 'idle', current: '0.4.0', latest: null, checkedAtMs: null, error: null, ...over };
}

let off: (() => void) | undefined;

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  off?.();
  off = undefined;
  disconnectSettings();
});

async function setup(initial: UpdateStatus = status(), persistence?: ConstructorParameters<typeof MockSettings>[0]) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.updateStatus = initial;
  if (persistence) backend.settings = new MockSettings(persistence);
  await settings.connect(backend);
  off = await updates.connect(backend);
  return backend;
}

const checkButton = () => screen.getByRole('button', { name: t('settings.about.checkNow') }) as HTMLButtonElement;

test('shows up to date after Check now', async () => {
  const backend = await setup();
  backend.checkResult = status({ state: 'upToDate', checkedAtMs: CHECKED_AT });
  render(AboutSection, { backend });
  await fireEvent.click(checkButton());
  await screen.findByText(t('settings.about.upToDate', { version: '0.4.0' }));
  const time = new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short' }).format(CHECKED_AT);
  expect(screen.getByText(t('settings.about.lastChecked', { time }))).toBeTruthy();
});

test('disables Check now while checking', async () => {
  const backend = await setup(status({ state: 'checking' }));
  render(AboutSection, { backend });
  expect(checkButton().disabled).toBe(true);
  expect(screen.getByText(t('settings.about.checking'))).toBeTruthy();
  backend.emitUpdateStatus(status({ state: 'upToDate', checkedAtMs: CHECKED_AT }));
  await vi.waitFor(() => expect(checkButton().disabled).toBe(false));
});

test('available shows version and release page button', async () => {
  const backend = await setup(status({ state: 'available', latest: { version: '0.5.0' }, checkedAtMs: CHECKED_AT }));
  render(AboutSection, { backend });
  expect(screen.getByText(t('settings.about.available', { version: '0.5.0' }))).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: t('settings.about.releasePage') }));
  expect(backend.openReleasePageCalls).toBe(1);
});

test('no release page button without a newer version', async () => {
  const backend = await setup(status({ state: 'upToDate', checkedAtMs: CHECKED_AT }));
  render(AboutSection, { backend });
  expect(screen.queryByRole('button', { name: t('settings.about.releasePage') })).toBeNull();
});

test.each(['offline', 'timeout', 'tls', 'http', 'invalid'] as const)('each error category has its text: %s', async (error) => {
  const backend = await setup(status({ state: 'error', error }));
  render(AboutSection, { backend });
  expect(screen.getByText(t(`settings.about.error.${error}`))).toBeTruthy();
});

test('a rejected check shows the invalid-response text and frees the button', async () => {
  const backend = await setup(status({ state: 'upToDate', checkedAtMs: CHECKED_AT }));
  backend.checkError = 'panicked';
  render(AboutSection, { backend });
  await fireEvent.click(checkButton());
  await screen.findByText(t('settings.about.error.invalid'));
  expect(checkButton().disabled).toBe(false);
});

test('checkbox patches updates.checkAutomatically and shows the note', async () => {
  const backend = await setup();
  const patches: unknown[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (patch) => {
    patches.push(patch);
    return update(patch);
  };
  render(AboutSection, { backend });
  expect(screen.getByText(t('settings.about.updatesNote'))).toBeTruthy();
  const box = screen.getByRole('checkbox', { name: t('settings.about.checkAutomatically') }) as HTMLInputElement;
  expect(box.checked).toBe(false);
  await fireEvent.click(box);
  expect(patches).toEqual([{ updates: { checkAutomatically: true } }]);
  await vi.waitFor(() => expect(box.checked).toBe(true));
});

test('checkbox is disabled when the settings are read-only', async () => {
  const backend = await setup(status(), { kind: 'readOnly', reason: 'locked' });
  render(AboutSection, { backend });
  expect((screen.getByRole('checkbox', { name: t('settings.about.checkAutomatically') }) as HTMLInputElement).disabled).toBe(true);
});

async function renderSettings(initial: UpdateStatus) {
  const backend = await setup(initial);
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  render(SettingsView, { store, backend, service: null, onBack: () => {} });
  return backend;
}

test('badge on About only when available', async () => {
  const backend = await renderSettings(status({ state: 'upToDate', checkedAtMs: CHECKED_AT }));
  expect(screen.queryByText(t('settings.about.updateBadge'))).toBeNull();
  backend.emitUpdateStatus(status({ state: 'available', latest: { version: '0.5.0' } }));
  const badge = await screen.findByText(t('settings.about.updateBadge'));
  expect(badge.closest('button')?.textContent).toContain(t('settings.section.about'));
});

test('error_status_keeps_available_badge', async () => {
  const backend = await renderSettings(status({ state: 'error', error: 'offline', latest: { version: '0.5.0' } }));
  expect(screen.getByText(t('settings.about.updateBadge'))).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: new RegExp(t('settings.section.about')) }));
  expect(screen.getByText(t('settings.about.error.offline'))).toBeTruthy();
  expect(backend.openReleasePageCalls).toBe(0);
});

test('mock backend implements update commands', async () => {
  const backend = createMockBackend();
  expect((await backend.getUpdateStatus()).state).toBe('idle');
  const off = await backend.onUpdateStatus(() => {});
  off();
  await expect(backend.openReleasePage()).resolves.toBeUndefined();
});

const exportButton = () => screen.getByRole('button', { name: t('settings.about.exportReport') });

test('export shows saved message and Open folder', async () => {
  const backend = await setup();
  backend.exportResult = { fileName: 'oma-report-20261004-090507.json' };
  render(AboutSection, { backend });
  expect(screen.getByText(t('settings.about.reportNote'))).toBeTruthy();
  await fireEvent.click(exportButton());
  await screen.findByText(t('settings.about.reportSaved', { file: 'oma-report-20261004-090507.json' }));
  expect(backend.exportSensorReportCalls).toBe(1);
  await fireEvent.click(screen.getByRole('button', { name: t('settings.about.openFolder') }));
  expect(backend.revealSensorReportCalls).toBe(1);
});

test('cancelled export shows nothing', async () => {
  const backend = await setup();
  backend.exportResult = null;
  render(AboutSection, { backend });
  await fireEvent.click(exportButton());
  await vi.waitFor(() => expect(backend.exportSensorReportCalls).toBe(1));
  expect(screen.queryByText(/oma-report/)).toBeNull();
  expect(screen.queryByRole('button', { name: t('settings.about.openFolder') })).toBeNull();
  expect(screen.queryAllByRole('alert')).toHaveLength(0);
});

test('failed export shows the error text', async () => {
  const backend = await setup();
  backend.exportError = 'Access is denied. (os error 5)';
  render(AboutSection, { backend });
  await fireEvent.click(exportButton());
  const alert = await screen.findByRole('alert');
  expect(alert.textContent).toContain('Access is denied. (os error 5)');
  expect(screen.queryByRole('button', { name: t('settings.about.openFolder') })).toBeNull();
});

test('mock backend implements report commands', async () => {
  const backend = createMockBackend();
  expect((await backend.exportSensorReport())?.fileName).toMatch(/^oma-report-\d{8}-\d{6}\.json$/);
  await expect(backend.revealSensorReport()).resolves.toBeUndefined();
});
