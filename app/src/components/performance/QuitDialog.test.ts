import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import QuitDialog from './QuitDialog.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

test('quit_dialog_confirm_calls_the_backend', async () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(QuitDialog, { onConfirm, onCancel });
  expect(screen.getByRole('dialog', { name: t('performance.quit.title') })).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: t('performance.quit.confirm') }));
  expect(onConfirm).toHaveBeenCalledTimes(1);
  expect(onCancel).not.toHaveBeenCalled();
});

test('quit_dialog_cancel_keeps_the_test', async () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(QuitDialog, { onConfirm, onCancel });
  await fireEvent.click(screen.getByRole('button', { name: t('performance.quit.cancel') }));
  await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  expect(onCancel).toHaveBeenCalledTimes(2);
  expect(onConfirm).not.toHaveBeenCalled();
});
