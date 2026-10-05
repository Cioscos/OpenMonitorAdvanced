import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../lib/backend/mock';
import { i18n } from '../lib/i18n/index.svelte';
import { settings } from '../lib/settings.svelte';
import { FakeBackend } from '../test/fake-backend';
import EditorApp from './EditorApp.svelte';
import Palette from './Palette.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  settings.state = null;
});

test('palette search filters sensors', async () => {
  render(Palette, { schema: MOCK_SCHEMA, onAdd: vi.fn(), onDrop: vi.fn() });
  expect(screen.getByRole('button', { name: 'GPU load' })).toBeTruthy();
  await fireEvent.input(screen.getByRole('searchbox', { name: 'Search sensors' }), { target: { value: 'TEMPERATURE' } });
  expect(screen.getByRole('button', { name: 'Core temperature' })).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'GPU load' })).toBeNull();
  // The frame metrics and the text stay.
  expect(screen.getByRole('button', { name: 'FPS' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'New text' })).toBeTruthy();
});

test('the frame group lists the twelve metrics', () => {
  render(Palette, { schema: MOCK_SCHEMA, onAdd: vi.fn(), onDrop: vi.fn() });
  const group = screen.getByRole('group', { name: 'Frames' });
  expect(group.querySelectorAll('button')).toHaveLength(12);
});

test('enter on a palette item adds a block', async () => {
  render(EditorApp, { backend: new FakeBackend(MOCK_SCHEMA) });
  const item = await screen.findByRole('button', { name: 'New text' });
  await fireEvent.keyDown(item, { key: 'Enter' });
  await fireEvent.keyDown(await screen.findByRole('button', { name: 'GPU load' }), { key: 'Enter' });
  const options = await within(screen.getByRole('listbox', { name: 'Blocks' })).findAllByRole('option');
  expect(options.map((o) => o.textContent?.trim())).toEqual(['New text', 'GPU load']);
});
