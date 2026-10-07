import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { FakeBackend, makeSystemInfo } from '../../test/fake-backend';
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

test('sidebar lists one score entry per gpu', async () => {
  const backend: FakeBackend = await connectSettings();
  backend.performanceSystemInfo = makeSystemInfo({
    gpus: [
      { deviceId: 'gpu-a', name: 'Fake GeForce RTX 4080', integrated: false, dedicatedBytes: 16 * 1024 ** 3 },
      { deviceId: 'gpu-b', name: 'Fake Radeon Graphics', integrated: true, dedicatedBytes: 512 * 1024 ** 2 },
    ],
  });
  render(PerformanceView, { backend, store: new LiveStore(), page: 'history' });
  const group = await screen.findByRole('group', { name: t('performance.nav.score') });
  await waitFor(() => expect(group.querySelectorAll('button')).toHaveLength(3));
  expect([...group.querySelectorAll('button')].map((b) => b.textContent?.trim())).toEqual([
    t('performance.nav.scoreCpu'),
    'Fake GeForce RTX 4080',
    'Fake Radeon Graphics',
  ]);
  await fireEvent.click(screen.getByRole('button', { name: 'Fake Radeon Graphics' }));
  await screen.findByRole('heading', { name: t('performance.score.gpu.title') });
  expect(screen.getByRole('button', { name: 'Fake Radeon Graphics' }).getAttribute('aria-current')).toBe('page');
  expect(screen.getByRole('button', { name: 'Fake GeForce RTX 4080' }).getAttribute('aria-current')).toBeNull();
  // The GPU list is read once, when the view opens.
  expect(backend.performanceCalls.filter((c) => c === 'performanceSystem')).toHaveLength(1);
});
