import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n } from '../../../lib/i18n/index.svelte';
import NumberInput from './NumberInput.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

async function refused() {
  const onCommit = vi.fn(async () => false);
  const view = render(NumberInput, { id: 'n', value: 5, invalid: false, onCommit });
  const input = screen.getByRole('textbox') as HTMLInputElement;
  await fireEvent.input(input, { target: { value: '7' } });
  await fireEvent.blur(input);
  await vi.waitFor(() => expect(onCommit).toHaveBeenCalledWith(7));
  // The parent shows the error; the refused number stays next to it.
  await view.rerender({ invalid: true });
  expect(input.value).toBe('7');
  return { view, input };
}

test('a refused number goes when its error clears', async () => {
  const { view, input } = await refused();
  await view.rerender({ invalid: false });
  expect(input.value).toBe('5');
});

test('text typed after a refusal stays when the error clears', async () => {
  const { view, input } = await refused();
  await fireEvent.input(input, { target: { value: '8' } });
  await view.rerender({ invalid: false });
  expect(input.value).toBe('8');
});
