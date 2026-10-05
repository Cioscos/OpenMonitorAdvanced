import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../lib/backend/mock';
import { EditorStore } from '../lib/editor/editor.svelte';
import { newBlock, withDefaults, type Threshold } from '../lib/editor/profile';
import { i18n } from '../lib/i18n/index.svelte';
import { FakeBackend } from '../test/fake-backend';
import Properties from './Properties.svelte';
import ThresholdsEditor from './ThresholdsEditor.svelte';
import VisibleIfEditor from './VisibleIfEditor.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

test('multi selection shows common properties and edits all', async () => {
  const editor = new EditorStore(new FakeBackend(MOCK_SCHEMA));
  const a = newBlock({ text: 'A' }, { x: 0, y: 0 }, []);
  const b = newBlock({ text: 'B' }, { x: 0, y: 4 }, [a]);
  b.style.valueStyle.size = 20;
  editor.apply({ ...withDefaults({ format: 1, name: 'p' }), blocks: [a, b] });
  editor.select([a.id, b.id]);
  render(Properties, { editor, fonts: ['Segoe UI'], schema: MOCK_SCHEMA });
  expect(screen.getByText('2 blocks selected: common properties only.')).toBeTruthy();
  const value = within(screen.getByRole('group', { name: 'Value style' }));
  const size = value.getByLabelText('Size (pt)') as HTMLInputElement;
  // Different sizes show «—»; the same alignment shows its value.
  expect(size.value).toBe('');
  expect(size.placeholder).toBe('—');
  expect((screen.getByLabelText('Alignment') as HTMLSelectElement).value).toBe('left');
  await fireEvent.input(size, { target: { value: '16' } });
  await fireEvent.blur(size);
  expect(editor.profile.blocks.map((x) => x.style.valueStyle.size)).toEqual([16, 16]);
  // An optional object is set whole on every block.
  await fireEvent.click(value.getByLabelText('Outline'));
  expect(editor.profile.blocks.every((x) => x.style.valueStyle.outline?.width === 1)).toBe(true);
});

test('threshold editor keeps order and caps at eight', async () => {
  const t = (value: number): Threshold => ({ op: '>', value, color: '#FF0000', target: 'value' });
  const onChange = vi.fn();
  const view = render(ThresholdsEditor, { value: [t(1), t(2), t(3)], onChange });
  await fireEvent.click(screen.getByRole('button', { name: 'Add threshold' }));
  expect(onChange.mock.lastCall![0].map((x: Threshold) => x.value)).toEqual([1, 2, 3, 0]);
  const second = screen.getAllByLabelText('Value')[1];
  await fireEvent.input(second, { target: { value: '9' } });
  await fireEvent.blur(second);
  expect(onChange.mock.lastCall![0].map((x: Threshold) => x.value)).toEqual([1, 9, 3]);
  await fireEvent.click(screen.getAllByRole('button', { name: 'Remove' })[0]);
  expect(onChange.mock.lastCall![0].map((x: Threshold) => x.value)).toEqual([2, 3]);
  await view.rerender({ value: Array.from({ length: 8 }, (_, i) => t(i)) });
  expect((screen.getByRole('button', { name: 'Add threshold' }) as HTMLButtonElement).disabled).toBe(true);
});

test('visibleIf editor writes each form', async () => {
  const onChange = vi.fn();
  const view = render(VisibleIfEditor, { value: null, schema: MOCK_SCHEMA, onChange });
  const show = () => screen.getByLabelText('Show') as HTMLSelectElement;
  await fireEvent.change(show(), { target: { value: 'fg' } });
  expect(onChange).toHaveBeenLastCalledWith({ fg: 'active' });
  await fireEvent.change(show(), { target: { value: 'value' } });
  expect(onChange).toHaveBeenLastCalledWith({
    source: { frames: 'fps-displayed' },
    stat: { op: 'current', window: 1, definition: 'integral' },
    op: '<',
    value: 60,
  });
  await view.rerender({ value: onChange.mock.lastCall![0] });
  await fireEvent.change(screen.getByLabelText('Source'), { target: { value: 'sensor:cpu/0/load/total' } });
  expect(onChange.mock.lastCall![0].source).toEqual({ sensor: 'cpu/0/load/total' });
  await fireEvent.change(screen.getByLabelText('Condition'), { target: { value: '>=' } });
  expect(onChange.mock.lastCall![0].op).toBe('>=');
  await fireEvent.change(show(), { target: { value: 'always' } });
  expect(onChange).toHaveBeenLastCalledWith(null);
});
