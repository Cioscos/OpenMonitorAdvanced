import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { Board, BoardRow } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import PerformanceView from './PerformanceView.svelte';

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

const row = (i: number, board: Board = 'cpu-single', over: Partial<BoardRow> = {}): BoardRow => ({
  board,
  scoreVersion: board.startsWith('gpu') ? 'gpu-1' : board === 'disk' ? 'disk-1' : 'cpu-1',
  model: `Model ${i}`,
  key: `model ${i}`,
  value: i * 100,
  n: 5,
  source: 'community',
  ...over,
});

async function setup(configure: (b: FakeBackend) => void = () => {}) {
  const backend: FakeBackend = await connectSettings();
  backend.boardTable.rows = [...Array.from({ length: 12 }, (_, i) => row(12 - i)), row(77, 'gpu-compute', { model: 'Some GPU', source: 'author' })];
  configure(backend);
  render(PerformanceView, { backend, store: new LiveStore(), page: 'board' });
  await screen.findByRole('tablist', { name: t('performance.board.tabs') });
  await waitFor(() => expect(document.querySelector('.row')).not.toBeNull());
  return backend;
}

test('tabs_switch_the_category', async () => {
  await setup();
  expect(screen.queryByText('Some GPU')).toBeNull();
  await fireEvent.click(screen.getByRole('tab', { name: t('performance.board.gpu-compute') }));
  expect(screen.getByText('Some GPU')).toBeTruthy();
  expect(screen.queryByText('Model 12')).toBeNull();
  expect(screen.getByRole('tab', { name: t('performance.board.gpu-compute') }).getAttribute('aria-selected')).toBe('true');
});

test('caption_says_bundled_or_updated', async () => {
  await setup();
  expect(screen.getByText(t('performance.board.caption.bundled', { version: 'cpu-1', count: 12 }), { exact: false })).toBeTruthy();
  cleanup();
  disconnectSettings();
  await setup((b) => (b.boardTable.communityAt = '2026-10-09T03:00:00Z'));
  expect(screen.queryByText(/table included in the app/)).toBeNull();
  expect(screen.getByText(/table updated on/)).toBeTruthy();
});

test('refresh_now_is_disabled_when_the_setting_is_off', async () => {
  await setup((b) => {
    b.boardTable.enabled = false;
    b.boardTable.error = 'offline'; // stale: must not show
  });
  expect((screen.getByRole('button', { name: t('performance.board.refresh') }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText(t('performance.board.disabled'))).toBeTruthy();
  expect(screen.queryByRole('status')).toBeNull();
});

test('opening_the_page_refreshes_once', async () => {
  const backend = await setup();
  
  await waitFor(() => expect(backend.performanceCalls.filter((c) => c === 'performanceBoardRefresh')).toHaveLength(1));
  await fireEvent.click(screen.getByRole('tab', { name: t('performance.board.disk') }));
  expect(backend.performanceCalls.filter((c) => c === 'performanceBoardRefresh')).toHaveLength(1);
});

test('a_download_error_is_shown_with_its_reason', async () => {
  await setup((b) => (b.boardTable.error = 'timeout'));
  expect(screen.getByRole('status').textContent).toBe(t('performance.board.failed', { reason: t('performance.board.error.timeout') }));
});

test('source_badges_have_tooltips', async () => {
  await setup((b) => (b.boardTable.rows[0].source = 'author'));
  const badges = document.querySelectorAll('.source .term');
  expect(badges.length).toBeGreaterThan(0);
  expect(document.body.textContent).toContain(t('performance.board.source.author'));
  expect(screen.getAllByTitle(t('performance.board.n', { n: 5 })).length).toBe(10);
});

test('model_names_render_as_text', async () => {
  await setup((b) => (b.boardTable.rows[0].model = '<img src=x onerror=alert(1)>'));
  expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeTruthy();
  expect(document.querySelector('img')).toBeNull();
});
