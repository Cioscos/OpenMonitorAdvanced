import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { overlay } from '../../lib/overlay.svelte';
import { settings } from '../../lib/settings.svelte';
import type { BenchmarkEntry, OverlayStatus } from '../../lib/types';
import { FakeBackend, makeOverlayStatus } from '../../test/fake-backend';
import { disconnectSettings } from '../../test/settings';
import BenchmarkSection from './BenchmarkSection.svelte';

let stopOverlay: (() => void) | undefined;

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  stopOverlay?.();
  stopOverlay = undefined;
  disconnectSettings();
  vi.useRealTimers();
});

const entry = (id: string, game: string, startedAt: string, durationS = 62.4): BenchmarkEntry => ({
  id,
  record: {
    format: 1,
    game,
    startedAt,
    endReason: 'user',
    summary: {
      durationS,
      framesTotal: 8986,
      framesDisplayed: 8986,
      framesGenerated: 4493,
      fpsDisplayed: 144.1,
      fpsRendered: 72,
      renderedSource: 'FG',
      lowsIntegral: { onePercent: 101.2, pointOnePercent: 88.4 },
      lowsPercentile: { onePercent: 99.5, pointOnePercent: 80.1 },
      frametimeMinMs: 4.1,
      frametimeMaxMs: 31.2,
      stutterCount: 3,
      stutterPercent: 0.4,
      fgMultiplier: 2,
      latencyPcMs: null,
      latencyDisplayMs: null,
    },
  },
});

async function setup(status: Partial<OverlayStatus> = { enabled: true }, entries: BenchmarkEntry[] = []) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.overlayStatus = makeOverlayStatus(status);
  backend.benchmarks = entries;
  await settings.connect(backend);
  stopOverlay = await overlay.connect(backend);
  render(BenchmarkSection, { backend });
  return backend;
}

test('start disabled with the overlay off', async () => {
  const backend = await setup({ enabled: false });
  const start = screen.getByRole('button', { name: t('benchmark.start') }) as HTMLButtonElement;
  expect(start.disabled).toBe(true);
  expect(screen.getByText(t('benchmark.needsOverlay'))).toBeTruthy();
  await fireEvent.click(start);
  expect(backend.benchmarkCalls).not.toContain('benchmarkToggle');
});

test('start toggles the capture when the overlay is on', async () => {
  const backend = await setup({ enabled: true });
  await fireEvent.click(screen.getByRole('button', { name: t('benchmark.start') }));
  expect(backend.benchmarkCalls).toContain('benchmarkToggle');
});

test('recording shows the game and the time', async () => {
  vi.useFakeTimers();
  await setup({ enabled: true, benchmark: { state: 'recording', game: 'eldenring.exe', elapsedS: 59, error: null } });
  expect(screen.getByText(t('benchmark.recording', { game: 'eldenring.exe', time: '00:59' }))).toBeTruthy();
  expect(screen.getByRole('button', { name: t('benchmark.stop') })).toBeTruthy();
  await vi.advanceTimersByTimeAsync(2000);
  expect(screen.getByText(t('benchmark.recording', { game: 'eldenring.exe', time: '01:01' }))).toBeTruthy();
});

test('history lists newest first with the summary', async () => {
  await setup({ enabled: true }, [entry('a', 'new.exe', '2026-10-05T21:30:00'), entry('b', 'old.exe', '2026-10-04T10:00:00')]);
  const games = await screen.findAllByText(/\.exe$/);
  expect(games.map((g) => g.textContent)).toEqual(['new.exe', 'old.exe']);
  expect(screen.getAllByText(t('benchmark.frames', { total: '8,986', displayed: '8,986', generated: '4,493' })).length).toBe(2);
  expect(screen.getAllByText(t('benchmark.fpsDisplayed')).length).toBe(2);
});

test('open CSV and folder call the backend', async () => {
  const backend = await setup({ enabled: true }, [entry('a', 'new.exe', '2026-10-05T21:30:00')]);
  await fireEvent.click(await screen.findByRole('button', { name: t('benchmark.openCsv') }));
  await fireEvent.click(screen.getByRole('button', { name: t('benchmark.openFolder') }));
  expect(backend.benchmarkCalls).toContain('benchmarkOpenCsv:a');
  expect(backend.benchmarkCalls).toContain('benchmarkOpenFolder');
});

test('delete asks then removes the entry', async () => {
  const backend = await setup({ enabled: true }, [entry('a', 'new.exe', '2026-10-05T21:30:00')]);
  await fireEvent.click(await screen.findByRole('button', { name: t('benchmark.delete') }));
  expect(backend.benchmarkCalls).not.toContain('benchmarkDelete:a');
  expect(screen.getByText(/Delete the session of new\.exe/)).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: t('rules.cancel') }));
  expect(backend.benchmarkCalls).not.toContain('benchmarkDelete:a');
  await fireEvent.click(screen.getByRole('button', { name: t('benchmark.delete') }));
  await fireEvent.click(screen.getByRole('button', { name: t('benchmark.delete') }));
  await waitFor(() => expect(backend.benchmarkCalls).toContain('benchmarkDelete:a'));
  await waitFor(() => expect(screen.queryByText('new.exe')).toBeNull());
  expect(screen.getByText(t('benchmark.history.empty'))).toBeTruthy();
});

test('history reloads when a capture ends', async () => {
  const backend = await setup({ enabled: true, benchmark: { state: 'recording', game: 'g.exe', elapsedS: 5, error: null } });
  await waitFor(() => expect(backend.benchmarkCalls.filter((c) => c === 'benchmarkList').length).toBe(1));
  backend.benchmarks = [entry('a', 'g.exe', '2026-10-05T21:30:00')];
  backend.emitOverlayStatus(makeOverlayStatus({ enabled: true }));
  await screen.findByText('g.exe');
  expect(backend.benchmarkCalls.filter((c) => c === 'benchmarkList').length).toBe(2);
});

test('capture errors and open failures are shown', async () => {
  const backend = await setup({ enabled: true, benchmark: { state: 'error', game: null, elapsedS: null, error: { key: 'log.error.diskFull', detail: null } } });
  expect(screen.getByRole('alert').textContent).toContain(t('log.error.diskFull'));
  backend.benchmarkOpenFolder = async () => Promise.reject('log.error.folderMissing');
  await fireEvent.click(screen.getByRole('button', { name: t('benchmark.openFolder') }));
  await screen.findByText(t('log.error.folderMissing'));
});
