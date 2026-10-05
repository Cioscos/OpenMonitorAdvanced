import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../lib/backend/mock';
import { newBlock, withDefaults } from '../lib/editor/profile';
import { i18n } from '../lib/i18n/index.svelte';
import { settings } from '../lib/settings.svelte';
import { FakeBackend, makeOverlayStatus } from '../test/fake-backend';
import EditorApp from './EditorApp.svelte';

const USER_ID = '00000000-0000-4000-8000-000000000001';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  settings.state = null;
});

/** The editor's window: `close()` runs the close handlers as Tauri does, destroying unless prevented. */
function fakeWindow() {
  const handlers: ((e: { preventDefault(): void }) => unknown)[] = [];
  const win = {
    destroyed: 0,
    async onCloseRequested(cb: (e: { preventDefault(): void }) => unknown) {
      handlers.push(cb);
      return () => {};
    },
    async destroy() {
      win.destroyed++;
    },
    async close() {
      let prevented = false;
      for (const h of handlers) await h({ preventDefault: () => (prevented = true) });
      if (!prevented) await win.destroy();
    },
  };
  return win;
}

/** The app on the user profile «Mine» (one text block), or on the built-in «Gaming». */
async function setup(open: 'user' | 'builtin' = 'user') {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const mine = withDefaults({ format: 1, name: 'Mine' });
  mine.blocks = [newBlock({ text: 'A' }, { x: 0, y: 0 }, [])];
  backend.profiles[USER_ID] = { id: USER_ID, builtin: false, json: JSON.stringify(mine) };
  backend.profiles['builtin-gaming'] = { id: 'builtin-gaming', builtin: true, json: JSON.stringify(withDefaults({ format: 1, name: 'Gaming' })) };
  const base = makeOverlayStatus();
  backend.overlayStatus = makeOverlayStatus({
    activeProfile: open === 'user' ? USER_ID : 'builtin-gaming',
    profiles: [...base.profiles, { id: USER_ID, name: 'Mine', builtin: false }],
  });
  const win = fakeWindow();
  render(EditorApp, { backend, appWindow: win });
  await screen.findByText(open === 'user' ? 'Mine' : 'Gaming', { selector: '.profile' });
  return { backend, win };
}

const toolbar = () => within(screen.getByRole('toolbar', { name: 'Overlay editor' }));

/** An edit: the profile's horizontal offset becomes `x`. */
async function edit(x = 5) {
  const input = within(screen.getByRole('group', { name: 'Offset (cells)' })).getByLabelText('X (cells)');
  await fireEvent.input(input, { target: { value: String(x) } });
  await fireEvent.blur(input);
  await screen.findByText('●');
}

test('a new profile is named in the language of the settings', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await backend.settings.update({ general: { language: 'it' } });
  // No profile file to open: the editor starts a new profile.
  render(EditorApp, { backend, appWindow: null });
  expect(await screen.findByText('Nuovo profilo', { selector: '.profile' })).toBeTruthy();
});

test('builtin profiles are read only until duplicated', async () => {
  const { backend } = await setup('builtin');
  expect(screen.getByText('Built-in profiles are read-only: duplicate it to edit.')).toBeTruthy();
  expect((toolbar().getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(true);
  expect((toolbar().getByRole('button', { name: 'Delete' }) as HTMLButtonElement).disabled).toBe(true);
  expect((toolbar().getByRole('radio', { name: 'Top right' }) as HTMLInputElement).disabled).toBe(true);
  await fireEvent.click(toolbar().getByRole('button', { name: 'Duplicate' }));
  await waitFor(() => expect((toolbar().getByRole('button', { name: 'Save' }) as HTMLButtonElement).disabled).toBe(false));
  expect((toolbar().getByRole('radio', { name: 'Top right' }) as HTMLInputElement).disabled).toBe(false);
  expect(backend.editorCalls).toContain('overlayDuplicateProfile:builtin-gaming');
});

test('save as asks a name and selects the new profile', async () => {
  const { backend } = await setup();
  await fireEvent.click(toolbar().getByRole('button', { name: 'Save as…' }));
  const dialog = within(screen.getByRole('dialog', { name: 'Save as…' }));
  // The name taken already gets a number, as the shell would give it (DD11).
  await fireEvent.input(dialog.getByLabelText('Name'), { target: { value: 'mine' } });
  await fireEvent.click(dialog.getByRole('button', { name: 'Save' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  const id = Object.keys(backend.profiles).find((k) => k !== USER_ID && k !== 'builtin-gaming')!;
  expect(JSON.parse(backend.profiles[id].json).name).toBe('mine (2)');
  await waitFor(() => expect((toolbar().getByLabelText('Profile') as HTMLSelectElement).value).toBe(id));
});

test('delete asks for confirmation', async () => {
  const { backend } = await setup();
  await fireEvent.click(toolbar().getByRole('button', { name: 'Delete' }));
  const dialog = within(screen.getByRole('dialog'));
  expect(dialog.getByText('Delete the profile «Mine»?')).toBeTruthy();
  expect(backend.editorCalls).not.toContain(`overlayDeleteProfile:${USER_ID}`);
  await fireEvent.click(dialog.getByRole('button', { name: 'Delete' }));
  await waitFor(() => expect(backend.editorCalls).toContain(`overlayDeleteProfile:${USER_ID}`));
});

test('font list comes from the backend', async () => {
  const { backend } = await setup();
  backend.fontFamilies = ['Segoe UI', 'Bahnschrift'];
  cleanup();
  render(EditorApp, { backend, appWindow: null });
  await fireEvent.click(within(await screen.findByRole('listbox', { name: 'Blocks' })).getByRole('option'));
  const style = within(screen.getByRole('group', { name: 'Value style' }));
  await waitFor(() => expect(style.getByRole('option', { name: 'Bahnschrift' })).toBeTruthy());
});

test('preview is sent 100 ms after the last edit', async () => {
  const { backend } = await setup();
  await fireEvent.click(toolbar().getByRole('button', { name: 'Preview' }));
  expect(backend.previews).toHaveLength(1);
  backend.emitPreview(true);
  await toolbar().findByRole('button', { name: 'Close preview' });
  vi.useFakeTimers();
  await edit(5);
  await vi.advanceTimersByTimeAsync(60);
  await edit(6);
  await vi.advanceTimersByTimeAsync(99);
  expect(backend.previews).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(backend.previews).toHaveLength(2);
  expect(JSON.parse(backend.previews[1]!).offset.x).toBe(6);
  await fireEvent.click(toolbar().getByRole('button', { name: 'Close preview' }));
  expect(backend.previews.at(-1)).toBeNull();
});

test('use now saves then activates', async () => {
  const { backend } = await setup();
  await edit();
  await fireEvent.click(toolbar().getByRole('button', { name: 'Use now' }));
  await waitFor(() => expect(backend.editorCalls).toContain(`overlayUseNow:${USER_ID}`));
  const calls = backend.editorCalls.filter((c) => c.startsWith('overlaySaveProfile') || c.startsWith('overlayUseNow'));
  expect(calls).toEqual([`overlaySaveProfile:${USER_ID}`, `overlayUseNow:${USER_ID}`]);
});

test('closing_with_changes_asks', async () => {
  const { backend, win } = await setup();
  await edit();
  await win.close();
  const dialog = within(await screen.findByRole('dialog', { name: 'Unsaved changes' }));
  expect(dialog.getByText('Save the changes to «Mine»?')).toBeTruthy();
  expect(win.destroyed).toBe(0);
  await fireEvent.click(dialog.getByRole('button', { name: 'Save' }));
  await waitFor(() => expect(win.destroyed).toBe(1));
  expect(backend.editorCalls).toContain(`overlaySaveProfile:${USER_ID}`);
});

test('cancel_keeps_the_editor_open', async () => {
  const { backend, win } = await setup();
  await edit();
  await win.close();
  await fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Cancel' }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(win.destroyed).toBe(0);
  expect(screen.getByText('●')).toBeTruthy();
  expect(backend.editorCalls.some((c) => c.startsWith('overlaySaveProfile'))).toBe(false);
});

test('discard_closes_without_saving', async () => {
  const { backend, win } = await setup();
  await edit();
  await win.close();
  await fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Discard' }));
  await waitFor(() => expect(win.destroyed).toBe(1));
  expect(backend.editorCalls.some((c) => c.startsWith('overlaySaveProfile'))).toBe(false);
});

test('closing without changes does not ask', async () => {
  const { win } = await setup();
  await win.close();
  expect(win.destroyed).toBe(1);
  expect(screen.queryByRole('dialog')).toBeNull();
});

test('quit request from the tray asks and confirms', async () => {
  const { backend } = await setup();
  await edit();
  backend.emitEditorQuit();
  await fireEvent.click(within(await screen.findByRole('dialog', { name: 'Unsaved changes' })).getByRole('button', { name: 'Discard' }));
  await waitFor(() => expect(backend.editorCalls).toContain('appQuitConfirmed'));
  expect(backend.editorCalls.some((c) => c.startsWith('overlaySaveProfile'))).toBe(false);
});

test('switching profile with changes asks first', async () => {
  const { backend } = await setup();
  await edit();
  await fireEvent.change(toolbar().getByLabelText('Profile'), { target: { value: 'builtin-gaming' } });
  const dialog = within(await screen.findByRole('dialog', { name: 'Unsaved changes' }));
  expect(backend.editorCalls).not.toContain('overlayLoadProfile:builtin-gaming');
  await fireEvent.click(dialog.getByRole('button', { name: 'Cancel' }));
  expect((toolbar().getByLabelText('Profile') as HTMLSelectElement).value).toBe(USER_ID);
  await fireEvent.change(toolbar().getByLabelText('Profile'), { target: { value: 'builtin-gaming' } });
  await fireEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Discard' }));
  await screen.findByText('Gaming', { selector: '.profile' });
});
