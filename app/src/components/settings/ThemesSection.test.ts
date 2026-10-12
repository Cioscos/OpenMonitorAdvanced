import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import type { ImportPreview, SettingsPatch, ThemeSummary } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { disconnectSettings } from '../../test/settings';
import SettingsView from './SettingsView.svelte';
import ThemesSection from './ThemesSection.svelte';

const UUID_A = '0f9c2b7e-51a4-4d3e-9a8b-2c1d0e6f7a10';
const UUID_B = '6a1e3d2c-9b8f-4e7d-8c6b-5a4f3e2d1c0b';
const VARS = { '--bg': '#0f0a1a', '--surface': '#181126', '--accent': '#ff4fd8', '--text': '#f5eefe' };

const builtin = (id: string, scheme: 'dark' | 'light' = 'dark'): ThemeSummary => ({
  id,
  name: null,
  author: 'OpenMonitor Advanced',
  scheme,
  builtin: true,
  vars: VARS,
});
const user = (id: string, name: string): ThemeSummary => ({ id, name, author: 'Me', scheme: 'dark', builtin: false, vars: VARS });

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

async function setup(themes: ThemeSummary[], seed?: SettingsPatch) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.themes = themes;
  await settings.connect(backend);
  if (seed) await settings.update(seed);
  const patches: SettingsPatch[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (patch) => {
    patches.push(patch);
    return update(patch);
  };
  render(ThemesSection, { backend });
  await screen.findByRole('button', { name: new RegExp(t('themes.builtin.builtin-synthwave')) });
  return { backend, patches };
}

const BUILTINS = [builtin('builtin-synthwave'), builtin('builtin-midnight'), builtin('builtin-daylight', 'light'), builtin('builtin-contrast')];
const gallery = () => screen.getByRole('list', { name: t('settings.section.themes') });
const cardNames = () => [...gallery().querySelectorAll('.pick .name')].map((n) => n.textContent?.trim());

test('section order puts themes after general', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await settings.connect(backend);
  render(SettingsView, { store: new LiveStore(), backend, service: null, onBack: vi.fn() });
  const nav = screen.getByRole('navigation', { name: t('settings.sections') });
  const names = within(nav).getAllByRole('button').map((b) => b.textContent?.trim());
  expect(names.slice(1, 3)).toEqual([t('settings.section.general'), t('settings.section.themes')]);
});

test('lists_builtins_then_user_themes_by_name', async () => {
  // The backend lists the user themes by name; the gallery keeps its order.
  await setup([...BUILTINS, user(UUID_B, 'Aurora'), user(UUID_A, 'Nord')]);
  expect(cardNames()).toEqual(['Synthwave', 'Midnight', 'Daylight', 'High contrast', 'Aurora', 'Nord']);
  expect(within(gallery()).getAllByText(t('themes.scheme.light'))).toHaveLength(1);
});

test('click_applies_and_saves_theme', async () => {
  const { patches } = await setup([...BUILTINS, user(UUID_A, 'Nord')]);
  await fireEvent.click(screen.getByRole('button', { name: /Nord/ }));
  await waitFor(() => expect(patches).toEqual([{ appearance: { theme: UUID_A } }]));
});

test('active_theme_is_marked', async () => {
  await setup([...BUILTINS, user(UUID_A, 'Nord')], { appearance: { theme: 'builtin-daylight' } });
  const current = within(gallery())
    .getAllByRole('button')
    .filter((b) => b.getAttribute('aria-current') === 'true');
  expect(current).toHaveLength(1);
  expect(current[0].textContent).toContain('Daylight');
});

function previewOf(over: Partial<ImportPreview> = {}): ImportPreview {
  return { token: 7, name: 'Nord Night', author: 'Arctic', scheme: 'dark', vars: VARS, applied: 3, warnings: [], ...over };
}

test('import_shows_summary_with_warnings_then_adds', async () => {
  const { backend, patches } = await setup([...BUILTINS]);
  backend.themeImportPreview = async () =>
    previewOf({
      warnings: [
        { line: 4, code: 'unknownVariable', detail: '--nope' },
        { line: 0, code: 'lowContrast', detail: '--text/--bg 2.1' },
      ],
    });
  const confirmed: number[] = [];
  backend.themeImportConfirm = async (token) => {
    confirmed.push(token);
    const added = user(UUID_A, 'Nord Night');
    backend.themes = [...BUILTINS, added];
    return added;
  };
  await fireEvent.click(screen.getByRole('button', { name: t('themes.import') }));
  const dialog = await screen.findByRole('dialog');
  expect(within(dialog).getByText('Nord Night')).toBeTruthy();
  expect(within(dialog).getByText(t('themes.import.summary', { applied: 3 }))).toBeTruthy();
  expect(within(dialog).getByText(t('themes.warn.unknownVariable', { line: 4, detail: '--nope' }))).toBeTruthy();
  expect(within(dialog).getByText('The text on background has low contrast: 2.1:1')).toBeTruthy();
  await fireEvent.click(within(dialog).getByRole('button', { name: t('themes.import.add') }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(confirmed).toEqual([7]);
  expect(patches).toEqual([{ appearance: { theme: UUID_A } }]);
  await waitFor(() => expect(cardNames()).toContain('Nord Night'));
});

test('import_cancel_adds_nothing', async () => {
  const { backend, patches } = await setup([...BUILTINS]);
  backend.themeImportPreview = async () => previewOf();
  let confirms = 0;
  backend.themeImportConfirm = async () => {
    confirms++;
    return user(UUID_A, 'x');
  };
  await fireEvent.click(screen.getByRole('button', { name: t('themes.import') }));
  const dialog = await screen.findByRole('dialog');
  await fireEvent.click(within(dialog).getByRole('button', { name: t('themes.import.cancel') }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(confirms).toBe(0);
  expect(patches).toEqual([]);
  expect(cardNames()).toHaveLength(4);
});

test('import_error_shows_message', async () => {
  const { backend } = await setup([...BUILTINS]);
  backend.themeImportPreview = async () => {
    throw { key: 'themes.error.empty', detail: null };
  };
  await fireEvent.click(screen.getByRole('button', { name: t('themes.import') }));
  expect((await screen.findByRole('alert')).textContent).toContain(t('themes.error.empty'));
  expect(screen.queryByRole('dialog')).toBeNull();
});

test('delete_asks_confirmation_and_hides_for_builtins', async () => {
  const { backend } = await setup([...BUILTINS, user(UUID_A, 'Nord')]);
  const deleted: string[] = [];
  const remove = backend.themeDelete.bind(backend);
  backend.themeDelete = async (id) => {
    deleted.push(id);
    return remove(id);
  };
  // Only the user theme has «Delete» (and «Export»).
  expect(screen.getAllByRole('button', { name: t('themes.delete') })).toHaveLength(1);
  expect(screen.getAllByRole('button', { name: t('themes.export') })).toHaveLength(1);
  await fireEvent.click(screen.getByRole('button', { name: t('themes.delete') }));
  expect(screen.getByText(t('themes.deleteConfirm'))).toBeTruthy();
  expect(deleted).toEqual([]);
  // Cancel keeps it.
  await fireEvent.click(screen.getByRole('button', { name: t('themes.deleteCancel') }));
  expect(screen.queryByText(t('themes.deleteConfirm'))).toBeNull();
  await fireEvent.click(screen.getByRole('button', { name: t('themes.delete') }));
  await fireEvent.click(screen.getByRole('button', { name: t('themes.deleteYes') }));
  await waitFor(() => expect(deleted).toEqual([UUID_A]));
  await waitFor(() => expect(cardNames()).not.toContain('Nord'));
});

test('user_name_is_text_not_html', async () => {
  await setup([...BUILTINS, user(UUID_A, '<img src=x>')]);
  expect(cardNames()).toContain('<img src=x>');
  expect(gallery().querySelector('img')).toBeNull();
});

test('delete_confirmation_moves_focus', async () => {
  await setup([...BUILTINS, user(UUID_A, 'Nord')]);
  const del = screen.getByRole('button', { name: t('themes.delete') });
  del.focus();
  await fireEvent.click(del);
  // The safe choice gets the focus, and the question is announced.
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: t('themes.deleteCancel') })));
  expect(screen.getByRole('alert').textContent).toContain(t('themes.deleteConfirm'));
  await fireEvent.click(screen.getByRole('button', { name: t('themes.deleteCancel') }));
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: t('themes.delete') })));
  await fireEvent.click(screen.getByRole('button', { name: t('themes.delete') }));
  await fireEvent.click(screen.getByRole('button', { name: t('themes.deleteYes') }));
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: t('themes.import') })));
});

test('template_button_calls_backend', async () => {
  const { backend } = await setup(BUILTINS);
  let calls = 0;
  backend.themeTemplateSave = async () => {
    calls++;
    return true;
  };
  await fireEvent.click(screen.getByRole('button', { name: t('themes.templateSave') }));
  await waitFor(() => expect(calls).toBe(1));
});
