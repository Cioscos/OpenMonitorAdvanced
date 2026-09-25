import { cleanup, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { DASH } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import type { GpuProcess } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import GpuProcesses from './GpuProcesses.svelte';

const GPU = 'gpu/pci-0000:01:00.0';
const MIB = 1024 ** 2;
const GAME: GpuProcess = { pid: 4242, name: 'game.exe', loadPercent: 87.4, engine: '3D', dedicatedBytes: 3 * 1024 * MIB, sharedBytes: 120 * MIB };
const DWM: GpuProcess = { pid: 1480, name: 'dwm.exe', loadPercent: null, engine: null, dedicatedBytes: 250 * MIB, sharedBytes: null };

let visibility: DocumentVisibilityState = 'visible';
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
const setVisibility = (state: DocumentVisibilityState) => {
  visibility = state;
  document.dispatchEvent(new Event('visibilitychange'));
};

beforeEach(() => {
  i18n.locale = 'en';
  visibility = 'visible';
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function setup(rows: GpuProcess[] = [GAME, DWM], failFirst = false) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.gpuProcesses = rows;
  if (failFirst) vi.spyOn(backend, 'getGpuProcesses').mockRejectedValueOnce(new Error('offline'));
  const view = render(GpuProcesses, { deviceId: GPU, backend });
  return { backend, ...view };
}

const cells = (name: string) => [...screen.getByText(name).closest('tr')!.querySelectorAll('td')].map((td) => td.textContent);

test('lists the processes of the gpu with load, engine and memory', async () => {
  const { backend } = setup();
  await vi.advanceTimersByTimeAsync(0);
  flushSync();
  expect(backend.gpuProcessCalls).toEqual([GPU]);
  expect(screen.getByText('4242')).toBeTruthy();
  expect(cells('game.exe')).toEqual(['87% · 3D', '3.0 GB', '120 MB']);
  expect(cells('dwm.exe')).toEqual([DASH, '250 MB', DASH]);
});

test('refreshes every 2 s only while visible', async () => {
  const { backend } = setup();
  await vi.advanceTimersByTimeAsync(0);
  await vi.advanceTimersByTimeAsync(4000);
  expect(backend.gpuProcessCalls).toHaveLength(3);
  setVisibility('hidden');
  await vi.advanceTimersByTimeAsync(6000);
  expect(backend.gpuProcessCalls).toHaveLength(3);
  setVisibility('visible');
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.gpuProcessCalls).toHaveLength(4);
});

test('stops refreshing when unmounted', async () => {
  const { backend, unmount } = setup();
  await vi.advanceTimersByTimeAsync(0);
  unmount();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(backend.gpuProcessCalls).toHaveLength(1);
});

test('says so when no process uses the gpu', async () => {
  setup([]);
  await vi.advanceTimersByTimeAsync(0);
  flushSync();
  expect(screen.getByText(t('advanced.processes.empty'))).toBeTruthy();
});

test('a failed request is logged and the next one retries', async () => {
  const error = vi.spyOn(console, 'error').mockImplementation(() => {});
  setup([GAME, DWM], true);
  await vi.advanceTimersByTimeAsync(0);
  expect(screen.queryByText('game.exe')).toBeNull();
  expect(error).toHaveBeenCalledWith('GPU process list unavailable', expect.any(Error));
  await vi.advanceTimersByTimeAsync(2000);
  flushSync();
  expect(screen.getByText('game.exe')).toBeTruthy();
  error.mockRestore();
});
