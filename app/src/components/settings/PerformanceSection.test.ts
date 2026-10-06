import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { settings } from '../../lib/settings.svelte';
import type { SettingsPatch } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { disconnectSettings } from '../../test/settings';
import PerformanceSection from './PerformanceSection.svelte';

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

async function setup(seed?: SettingsPatch) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  await settings.connect(backend);
  if (seed) await settings.update(seed);
  const patches: SettingsPatch[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (patch) => {
    patches.push(patch);
    return update(patch);
  };
  render(PerformanceSection);
  return { patches };
}

test('disabling_thermal_stop_asks_first', async () => {
  const { patches } = await setup();
  const toggle = screen.getByRole('switch', { name: t('settings.performance.thermalStop') });
  await fireEvent.click(toggle);
  expect(patches).toEqual([]);
  expect(screen.getByText(t('settings.performance.thermalStop.confirm'))).toBeTruthy();
  // Cancelling changes nothing; confirming sends the change.
  await fireEvent.click(screen.getByRole('button', { name: t('settings.performance.cancel') }));
  expect(patches).toEqual([]);
  await fireEvent.click(toggle);
  await fireEvent.click(screen.getByRole('button', { name: t('settings.performance.thermalStop.off') }));
  await waitFor(() => expect(patches).toEqual([{ performance: { thermalStop: false } }]));
  // Turning it back on asks nothing.
  await fireEvent.click(screen.getByRole('switch', { name: t('settings.performance.thermalStop') }));
  await waitFor(() => expect(patches.at(-1)).toEqual({ performance: { thermalStop: true } }));
});

test('automatic_threshold_sends_null', async () => {
  const { patches } = await setup({ performance: { cpuStopC: 90 } });
  patches.length = 0;
  await fireEvent.click(screen.getByRole('button', { name: t('settings.performance.cpuStop.useAuto') }));
  await waitFor(() => expect(patches).toEqual([{ performance: { cpuStopC: null } }]));
  expect(screen.getByText(t('settings.performance.cpuStop.auto'))).toBeTruthy();
});

test('reset_risk_notice', async () => {
  const { patches } = await setup({ performance: { riskNoticeSeen: true } });
  patches.length = 0;
  await fireEvent.click(screen.getByRole('button', { name: t('settings.performance.riskNotice.reset') }));
  await waitFor(() => expect(patches).toEqual([{ performance: { riskNoticeSeen: false } }]));
});

test('first_error_sends_patches', async () => {
  const { patches } = await setup();
  await fireEvent.click(screen.getByRole('radio', { name: t('settings.performance.firstError.yes') }));
  await waitFor(() => expect(patches).toEqual([{ performance: { stopOnFirstError: true } }]));
  await fireEvent.click(screen.getByRole('radio', { name: t('settings.performance.firstError.profile') }));
  await waitFor(() => expect(patches.at(-1)).toEqual({ performance: { stopOnFirstError: null } }));
});

test('ram_share_commit_sends_patch', async () => {
  const { patches } = await setup();
  const input = screen.getByLabelText(t('settings.performance.ramShare'));
  await fireEvent.input(input, { target: { value: '50' } });
  await fireEvent.blur(input);
  await waitFor(() => expect(patches).toEqual([{ performance: { ramSharePercent: 50 } }]));
});

test('manual_threshold_starts_at_95_and_has_a_label', async () => {
  const { patches } = await setup();
  await fireEvent.click(screen.getByRole('button', { name: t('settings.performance.cpuStop.set') }));
  await waitFor(() => expect(patches).toEqual([{ performance: { cpuStopC: 95 } }]));
  expect(await screen.findByLabelText(t('settings.performance.cpuStop'))).toBeTruthy();
});

test('escape_cancels_the_thermal_confirmation', async () => {
  const { patches } = await setup();
  await fireEvent.click(screen.getByRole('switch', { name: t('settings.performance.thermalStop') }));
  await fireEvent.keyDown(screen.getByText(t('settings.performance.thermalStop.confirm')), { key: 'Escape' });
  expect(screen.queryByText(t('settings.performance.thermalStop.confirm'))).toBeNull();
  expect(patches).toEqual([]);
});
