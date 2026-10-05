import { FakeBackend } from '../../test/fake-backend';
import { MOCK_SCHEMA } from '../backend/mock';
import { EditorStore } from './editor.svelte';
import { newBlock } from './profile';

const USER_ID = '00000000-0000-4000-8000-000000000001';

function setup() {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.profiles[USER_ID] = {
    id: USER_ID,
    builtin: false,
    json: JSON.stringify({ format: 1, name: 'Mine', blocks: [] }),
  };
  return { backend, store: new EditorStore(backend) };
}

test('dirty after an edit and clean after save', async () => {
  const { backend, store } = setup();
  expect(await store.load(USER_ID)).toBe(true);
  expect(store.dirty).toBe(false);
  store.apply({ ...store.profile, blocks: [newBlock({ text: 'hi' }, { x: 0, y: 0 }, [])] });
  expect(store.dirty).toBe(true);
  expect(backend.editorCalls).toContain('overlayEditorDirty:true');
  expect(await store.save()).toBe(true);
  expect(store.dirty).toBe(false);
  expect(backend.editorCalls.at(-1)).toBe('overlayEditorDirty:false');
  expect(JSON.parse(backend.profiles[USER_ID].json).blocks).toHaveLength(1);
  // Undo leaves the saved state; redo comes back to it.
  store.undo();
  expect(store.dirty).toBe(true);
  store.redo();
  expect(store.dirty).toBe(false);
});

test('built-in profiles are read-only', async () => {
  const { backend, store } = setup();
  backend.profiles['builtin-gaming'] = { id: 'builtin-gaming', builtin: true, json: JSON.stringify({ format: 1, name: 'Gaming' }) };
  await store.load('builtin-gaming');
  store.apply({ ...store.profile, name: 'x' });
  expect(store.profile.name).toBe('Gaming');
  expect(await store.save()).toBe(false);
  expect(store.error?.key).toBe('editor.error.readOnly');
  expect(await store.duplicate()).toBe(true);
  expect(store.builtin).toBe(false);
  expect(store.profileId).not.toBe('builtin-gaming');
});

test('the edited profile reaches the canvas debounced, and null on close', async () => {
  vi.useFakeTimers();
  try {
    const { backend, store } = setup();
    await store.load(USER_ID);
    vi.advanceTimersByTime(250);
    backend.editorProfiles.length = 0;
    store.apply({ ...store.profile, name: 'a' });
    store.apply({ ...store.profile, name: 'b' });
    expect(backend.editorProfiles).toEqual([]);
    vi.advanceTimersByTime(250);
    expect(backend.editorProfiles.map((j) => j && JSON.parse(j).name)).toEqual(['b']);
    store.close();
    vi.advanceTimersByTime(250);
    expect(backend.editorProfiles).toEqual([expect.any(String), null]);
  } finally {
    vi.useRealTimers();
  }
});

test('save as stores a new profile and opens it', async () => {
  const { backend, store } = setup();
  await store.load(USER_ID);
  expect(await store.saveAs('Copy')).toBe(true);
  expect(store.profileId).not.toBe(USER_ID);
  expect(JSON.parse(backend.profiles[store.profileId!].json).name).toBe('Copy');
  expect(store.dirty).toBe(false);
});

test('a failed command keeps the error for the UI', async () => {
  const { backend, store } = setup();
  backend.editorError = { key: 'editor.error.io', detail: 'disk full' };
  expect(await store.load(USER_ID)).toBe(false);
  expect(store.error).toEqual({ key: 'editor.error.io', detail: 'disk full' });
});
