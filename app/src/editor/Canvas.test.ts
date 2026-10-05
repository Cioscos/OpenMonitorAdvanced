import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../lib/backend/mock';
import { EditorStore } from '../lib/editor/editor.svelte';
import { newBlock, withDefaults } from '../lib/editor/profile';
import { i18n } from '../lib/i18n/index.svelte';
import { LiveStore } from '../lib/live.svelte';
import { settings } from '../lib/settings.svelte';
import { FakeBackend } from '../test/fake-backend';
import Canvas from './Canvas.svelte';
import EditorApp from './EditorApp.svelte';
import { FrameFeed } from './feed.svelte';

const blocks = () => within(screen.getByRole('listbox', { name: 'Blocks' }));

// The canvas is 960×540 CSS px at the top-left of the page and simulates 1920×1080 at 96 dpi
// (jsdom has no screen size): half scale, so a cell is 4 px. A top-left profile with the default
// offset (1 cell) and padding (1 cell) puts cell (0, 0) at (8, 8).
const CELL = 4;
const at = (cx: number, cy: number) => ({ clientX: 8 + cx * CELL, clientY: 8 + cy * CELL, pointerId: 1, button: 0 });

beforeEach(() => {
  i18n.locale = 'en';
  vi.spyOn(HTMLCanvasElement.prototype, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 960, 540));
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  settings.state = null;
});

function setup() {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const editor = new EditorStore(backend);
  const a = newBlock({ text: 'A' }, { x: 0, y: 0 }, []);
  const b = newBlock({ text: 'B' }, { x: 0, y: 4 }, [a]);
  editor.apply({ ...withDefaults({ format: 1, name: 'p' }), blocks: [a, b] });
  render(Canvas, { editor, live: new LiveStore(), feed: new FrameFeed() });
  const canvas = screen.getByRole('application', { name: 'Profile canvas' });
  const rect = (id: string) => editor.profile.blocks.find((x) => x.id === id)!.rect;
  return { editor, canvas, rect, a: a.id, b: b.id };
}

test('drag moves by whole cells', async () => {
  const { canvas, rect, b } = setup();
  await fireEvent.pointerDown(canvas, at(1, 1));
  await fireEvent.pointerMove(canvas, { ...at(1, 1), clientX: at(1, 1).clientX + 13, clientY: at(1, 1).clientY + 9 });
  await fireEvent.pointerUp(canvas, at(4, 3));
  expect(rect('b1')).toMatchObject({ x: 3, y: 2, w: 12, h: 2 });
  expect(rect(b)).toMatchObject({ x: 0, y: 4 });
});

test('handles resize', async () => {
  const { canvas, rect } = setup();
  await fireEvent.pointerDown(canvas, at(1, 1));
  await fireEvent.pointerUp(canvas, at(1, 1));
  // The handle sits on the bottom-right corner of the selected block (12, 2).
  await fireEvent.pointerDown(canvas, at(12, 2));
  await fireEvent.pointerMove(canvas, at(14, 3));
  await fireEvent.pointerUp(canvas, at(14, 3));
  expect(rect('b1')).toMatchObject({ x: 0, y: 0, w: 14, h: 3 });
});

test('shift click adds to the selection', async () => {
  const { editor, canvas, a, b } = setup();
  await fireEvent.pointerDown(canvas, at(1, 1));
  await fireEvent.pointerUp(canvas, at(1, 1));
  expect([...editor.selection]).toEqual([a]);
  await fireEvent.pointerDown(canvas, { ...at(1, 5), shiftKey: true });
  await fireEvent.pointerUp(canvas, at(1, 5));
  expect([...editor.selection].sort()).toEqual([a, b].sort());
  await fireEvent.pointerDown(canvas, { ...at(1, 1), ctrlKey: true });
  await fireEvent.pointerUp(canvas, at(1, 1));
  expect([...editor.selection]).toEqual([b]);
  // A click on empty space clears it.
  await fireEvent.pointerDown(canvas, at(100, 100));
  expect(editor.selection.size).toBe(0);
});

test('arrows move and shift arrows resize', async () => {
  const { editor, canvas, rect, a } = setup();
  editor.select([a]);
  await fireEvent.keyDown(canvas, { key: 'ArrowRight' });
  await fireEvent.keyDown(canvas, { key: 'ArrowDown' });
  expect(rect(a)).toMatchObject({ x: 1, y: 1, w: 12, h: 2 });
  await fireEvent.keyDown(canvas, { key: 'ArrowRight', shiftKey: true });
  await fireEvent.keyDown(canvas, { key: 'ArrowUp', shiftKey: true });
  expect(rect(a)).toMatchObject({ x: 1, y: 1, w: 13, h: 1 });
});

test('ctrl z undoes a drag in one step', async () => {
  const { editor, canvas, rect, a } = setup();
  await fireEvent.pointerDown(canvas, at(1, 1));
  for (let i = 1; i <= 5; i++) await fireEvent.pointerMove(canvas, at(1 + i, 1));
  await fireEvent.pointerUp(canvas, at(6, 1));
  expect(rect(a).x).toBe(5);
  await fireEvent.keyDown(canvas, { key: 'z', ctrlKey: true });
  expect(rect(a).x).toBe(0);
  await fireEvent.keyDown(canvas, { key: 'y', ctrlKey: true });
  expect(rect(a).x).toBe(5);
});

test('copy, paste, duplicate and delete go through the history', async () => {
  const { editor, canvas, a } = setup();
  editor.select([a]);
  await fireEvent.keyDown(canvas, { key: 'c', ctrlKey: true });
  await fireEvent.keyDown(canvas, { key: 'v', ctrlKey: true });
  expect(editor.profile.blocks).toHaveLength(3);
  await fireEvent.keyDown(canvas, { key: 'd', ctrlKey: true });
  expect(editor.profile.blocks).toHaveLength(4);
  await fireEvent.keyDown(canvas, { key: 'Delete' });
  expect(editor.profile.blocks).toHaveLength(3);
  await fireEvent.keyDown(canvas, { key: 'z', ctrlKey: true });
  expect(editor.profile.blocks).toHaveLength(4);
});

test('the block list selects blocks from the keyboard', async () => {
  const { editor, a, b } = setup();
  const list = screen.getByRole('listbox', { name: 'Blocks' });
  await fireEvent.keyDown(list, { key: 'ArrowDown' });
  expect([...editor.selection]).toEqual([a]);
  await fireEvent.keyDown(list, { key: 'ArrowDown' });
  expect([...editor.selection]).toEqual([b]);
  expect(blocks().getAllByRole('option').map((o) => o.getAttribute('aria-selected'))).toEqual(['false', 'true']);
});

// The palette and the canvas together.
async function app() {
  const backend = new FakeBackend(MOCK_SCHEMA);
  render(EditorApp, { backend });
  // The sensors come with the schema.
  return backend;
}

test('dropping a sensor creates a text block', async () => {
  await app();
  const item = await screen.findByRole('button', { name: 'GPU load' });
  await fireEvent.pointerDown(item, { clientX: 1200, clientY: 300, pointerId: 1, button: 0 });
  await fireEvent.pointerMove(item, { ...at(2, 3), pointerId: 1 });
  await fireEvent.pointerUp(item, { ...at(2, 3), pointerId: 1 });
  const option = await blocks().findByRole('option', { name: /GPU load/ });
  expect(option.dataset.kind).toBe('text');
});

test('dropping frametime creates a graph block', async () => {
  await app();
  const item = await screen.findByRole('button', { name: 'Frametime' });
  await fireEvent.pointerDown(item, { clientX: 1200, clientY: 300, pointerId: 1, button: 0 });
  await fireEvent.pointerUp(item, { ...at(2, 3), pointerId: 1 });
  const option = await blocks().findByRole('option', { name: /Frametime/ });
  expect(option.dataset.kind).toBe('graph');
});

test('dropping outside the canvas adds nothing', async () => {
  await app();
  const item = await screen.findByRole('button', { name: 'GPU load' });
  await fireEvent.pointerDown(item, { clientX: 1200, clientY: 300, pointerId: 1, button: 0 });
  await fireEvent.pointerUp(item, { clientX: 1200, clientY: 600, pointerId: 1 });
  await waitFor(() => expect(blocks().queryAllByRole('option')).toHaveLength(0));
});
