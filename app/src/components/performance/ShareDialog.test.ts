import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { FakeBackend } from '../../test/fake-backend';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import ShareDialog from './ShareDialog.svelte';

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

async function open(configure: (b: FakeBackend) => void = () => {}) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  configure(backend);
  const onClose = vi.fn();
  render(ShareDialog, { backend, scoreId: 'score-a', onClose });
  return { backend, onClose };
}
const send = () => screen.getByRole('button', { name: t('performance.share.send') }) as HTMLButtonElement;
const previewText = () => screen.getByLabelText(t('performance.share.preview')).textContent;

test('preview_shows_the_exact_json_and_the_note', async () => {
  await open();
  await waitFor(() => expect(previewText()).toBe('{ "overclock": false }'));
  expect(screen.getByText(t('performance.share.note'))).toBeTruthy();
  expect(screen.getByRole('dialog', { name: t('performance.share.title') })).toBeTruthy();
});

test('overclock_box_reloads_the_preview', async () => {
  const { backend } = await open();
  await waitFor(() => expect(previewText()).toBe('{ "overclock": false }'));
  const fast = backend.performanceSharePreview.bind(backend);
  backend.performanceSharePreview = (id, overclock) => new Promise((r) => setTimeout(() => r(fast(id, overclock)), 30));
  await fireEvent.click(screen.getByRole('checkbox'));
  expect(send().disabled).toBe(true);
  await waitFor(() => expect(previewText()).toBe('{ "overclock": true }'));
  expect(backend.performanceCalls).toContain('performanceSharePreview:score-a:true');
});

test('send_success_closes_and_marks_shared', async () => {
  const { backend, onClose } = await open();
  await waitFor(() => expect(send().disabled).toBe(false));
  await fireEvent.click(send());
  await waitFor(() => expect(onClose).toHaveBeenCalledWith(true));
  expect(backend.performanceCalls).toContain('performanceShareSend:score-a:false');
});

test('share_error_is_translated_and_retry_is_possible', async () => {
  const { backend, onClose } = await open();
  await waitFor(() => expect(send().disabled).toBe(false));
  backend.shareError = 'rate_limited';
  await fireEvent.click(send());
  expect((await screen.findByRole('alert')).textContent).toBe(t('performance.share.error.rate_limited'));
  expect(onClose).not.toHaveBeenCalled();
  expect(send().disabled).toBe(false);
  backend.shareError = null;
  await fireEvent.click(send());
  await waitFor(() => expect(onClose).toHaveBeenCalledWith(true));
});

test('send_button_is_disabled_while_sending', async () => {
  const { backend } = await open();
  await waitFor(() => expect(send().disabled).toBe(false));
  let release!: () => void;
  backend.performanceShareSend = (id, overclock) => {
    backend.performanceCalls.push(`performanceShareSend:${id}:${overclock}`);
    return new Promise<void>((resolve) => (release = resolve));
  };
  await fireEvent.click(send());
  await fireEvent.click(screen.getByRole('button', { name: t('performance.share.sending') }));
  expect(backend.performanceCalls.filter((c) => c.startsWith('performanceShareSend'))).toHaveLength(1);
  release();
});

test('unknown_error_code_uses_the_fallback', async () => {
  await open((b) => (b.shareError = 'weird'));
  expect((await screen.findByRole('alert')).textContent).toBe(t('performance.share.error.unknown'));
});

test('escape_cancels_and_focus_starts_on_the_first_control', async () => {
  const opener = document.createElement('button');
  document.body.append(opener);
  opener.focus();
  const { onClose } = await open();
  expect(document.activeElement).toBe(screen.getByRole('checkbox'));
  await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  expect(onClose).toHaveBeenCalledWith(false);
  cleanup();
  expect(document.activeElement).toBe(opener);
  opener.remove();
});
