import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { i18n, t } from '../../lib/i18n/index.svelte';
import type { VolumeChoice } from '../../lib/types';
import { FakeBackend, makeVolume } from '../../test/fake-backend';
import { connectSettings, disconnectSettings } from '../../test/settings';
import DiskTarget from './DiskTarget.svelte';

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

const C = makeVolume();
const E = makeVolume({
  root: 'E:\\',
  label: 'STICK',
  folder: 'E:\\',
  deviceId: 'disk-e',
  model: 'Fake USB Stick',
  kind: 'usb',
  removable: true,
  system: false,
  totalBytes: 64 * 1024 ** 3,
  freeBytes: 30 * 1024 ** 3,
});

async function setup(over: { volumes?: VolumeChoice[]; value?: VolumeChoice | null; asking?: boolean } = {}) {
  const backend: FakeBackend = await connectSettings();
  const onChange = vi.fn();
  const onWake = vi.fn();
  const onCancelWake = vi.fn();
  const result = render(DiskTarget, {
    backend,
    volumes: over.volumes ?? [C, E],
    value: over.value === undefined ? C : over.value,
    onChange,
    asking: over.asking ?? false,
    onWake,
    onCancelWake,
  });
  return { backend, onChange, onWake, onCancelWake, ...result };
}

test('lists_volumes_with_model_and_kind', async () => {
  const { onChange } = await setup();
  const menu = screen.getByRole('combobox', { name: t('performance.disk.volume') }) as HTMLSelectElement;
  const labels = [...menu.options].map((o) => o.textContent?.trim());
  expect(labels).toEqual([
    t('performance.disk.volumeItem', { root: 'C:\\', model: 'Fake NVMe SSD', kind: t('performance.disk.kind.nvme') }),
    t('performance.disk.volumeItem', { root: 'E:\\', model: 'Fake USB Stick', kind: t('performance.disk.kind.usb') }),
  ]);
  expect(menu.value).toBe('C:\\');
  // The space left, and the folder the test will use.
  expect(screen.getByText(t('performance.disk.free', { free: '400.0 GiB', total: '1,000.0 GiB' }))).toBeTruthy();
  expect(screen.getByText(t('performance.disk.folder', { folder: C.folder }))).toBeTruthy();
  await fireEvent.change(menu, { target: { value: 'E:\\' } });
  expect(onChange).toHaveBeenCalledWith(E);
});

test('a_volume_without_a_model_shows_its_label', async () => {
  await setup({ volumes: [makeVolume({ model: null, deviceId: null })], value: null });
  const menu = screen.getByRole('combobox', { name: t('performance.disk.volume') }) as HTMLSelectElement;
  expect(menu.textContent).toContain('Windows');
});

test('pick_folder_probes_and_shows_errors', async () => {
  const { backend, onChange } = await setup();
  // Cancelling the picker probes nothing.
  await fireEvent.click(screen.getByRole('button', { name: t('performance.disk.pick') }));
  await waitFor(() => expect(backend.performanceCalls).toContain('performanceDiskPick'));
  expect(backend.diskProbeCalls).toEqual([]);
  // A folder is probed, and the volume it answers with is the new target.
  const probed = makeVolume({ root: 'D:\\', folder: 'D:\\Tests', deviceId: 'disk-d', model: 'Fake HDD', kind: 'hdd' });
  backend.diskPickResult = 'D:\\Tests';
  backend.diskProbeResult = probed;
  await fireEvent.click(screen.getByRole('button', { name: t('performance.disk.pick') }));
  await waitFor(() => expect(onChange).toHaveBeenCalledWith(probed));
  expect(backend.diskProbeCalls).toEqual(['D:\\Tests']);
  // The refusals, each with its own text.
  for (const code of ['remote', 'not_writable', 'not_found', 'link']) {
    backend.diskProbeResult = `disk:${code}`;
    await fireEvent.click(screen.getByRole('button', { name: t('performance.disk.pick') }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain(t(`performance.disk.error.${code}`)));
  }
  backend.diskProbeResult = 'disk:no_space';
  await fireEvent.click(screen.getByRole('button', { name: t('performance.disk.pick') }));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain(t('performance.disk.error.no_space', { size: '1.0 GiB' })));
  // Nothing was chosen by a refused folder.
  expect(onChange).toHaveBeenCalledTimes(1);
});

test('removable_and_sync_show_their_warning', async () => {
  const view = await setup({ value: E });
  expect(screen.getByText(t('performance.disk.warn.removable'))).toBeTruthy();
  expect(screen.queryByText(t('performance.disk.warn.sync'))).toBeNull();
  await view.rerender({ value: makeVolume({ sync: true, virtualDisk: true }) });
  expect(screen.getByText(t('performance.disk.warn.sync'))).toBeTruthy();
  expect(screen.getByText(t('performance.disk.warn.virtual'))).toBeTruthy();
  expect(screen.queryByText(t('performance.disk.warn.removable'))).toBeNull();
  // A volume that is not one recognised disk: no temperature stop, no standby check (R14).
  await view.rerender({ value: makeVolume({ deviceId: null }) });
  expect(screen.getByText(t('performance.disk.warn.noDevice'))).toBeTruthy();
});

test('standby_asks_for_consent_then_starts_with_wake', async () => {
  const { onWake, onCancelWake } = await setup({ asking: true });
  const dialog = screen.getByRole('alertdialog');
  expect(dialog.textContent).toContain(t('performance.disk.standby.title'));
  expect(dialog.textContent).toContain(t('performance.disk.standby.body'));
  await fireEvent.click(screen.getByRole('button', { name: t('performance.disk.standby.confirm') }));
  expect(onWake).toHaveBeenCalledOnce();
  await fireEvent.click(screen.getByRole('button', { name: t('performance.history.cancel') }));
  expect(onCancelWake).toHaveBeenCalledOnce();
});

test('no_dialog_unless_asking', async () => {
  await setup();
  expect(screen.queryByRole('alertdialog')).toBeNull();
});
