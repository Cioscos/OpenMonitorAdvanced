import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { settings } from '../../lib/settings.svelte';
import type { EffectStatus, PawnIoStatus, Schema, ServiceSources, ServiceStatus, SettingsPatch } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { disconnectSettings } from '../../test/settings';
import SourcesSection from './SourcesSection.svelte';

const SSD = 'storage/device-mock-ssd';
const USB = 'storage/device-mock-usb';
const MODULES = ['cpu', 'motherboard', 'memory', 'storage', 'controller', 'psu'];

/** The mock schema with a selectable SSD and a USB disk whose descriptor has no serial. */
const SCHEMA: Schema = {
  ...MOCK_SCHEMA,
  devices: [
    ...MOCK_SCHEMA.devices.map((d) => (d.id === SSD ? { ...d, properties: { smartSelectable: 'true' } } : d)),
    { id: USB, kind: 'storage', name: 'USB disk', properties: { smartSelectable: 'false' } },
  ],
};

const sources = (over: Partial<ServiceSources> = {}): ServiceSources => ({
  activeModules: MODULES,
  smartDisabledDrives: [],
  reconfiguration: 'applied',
  smartBlockedBy: [],
  ...over,
});
const connected = (over: Partial<ServiceStatus> = {}): ServiceStatus => ({
  state: 'connected',
  detail: null,
  pawnIo: 'ok',
  sources: sources(),
  ...over,
});

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
});

async function setup(service: ServiceStatus | null, patch?: SettingsPatch) {
  const backend = new FakeBackend(SCHEMA);
  await settings.connect(backend);
  if (patch) await settings.update(patch);
  const store = new LiveStore();
  store.applySchema(SCHEMA);
  const patches: SettingsPatch[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (p) => {
    patches.push(p);
    return update(p);
  };
  const view = render(SourcesSection, { store, backend, service });
  return { backend, patches, view };
}

const toggle = (name: string) => screen.getByRole('switch', { name }) as HTMLButtonElement;

test('sources: vendor toggles patch vendorLibraries', async () => {
  const { patches } = await setup(connected());
  expect(screen.getByText(t('settings.sources.vendor.note'))).toBeTruthy();
  for (const vendor of ['nvml', 'nvapi', 'adl', 'igcl']) {
    const sw = toggle(t(`settings.sources.vendor.${vendor}`));
    expect(sw.getAttribute('aria-checked')).toBe('true');
    await fireEvent.click(sw);
  }
  await vi.waitFor(() => expect(patches).toHaveLength(4));
  expect(patches).toEqual(['nvml', 'nvapi', 'adl', 'igcl'].map((v) => ({ sources: { vendorLibraries: { [v]: false } } })));
  await vi.waitFor(() => expect(toggle(t('settings.sources.vendor.nvml')).getAttribute('aria-checked')).toBe('false'));
});

test('anti-cheat toggle calls setAntiCheat', async () => {
  const { backend, patches } = await setup(connected());
  await fireEvent.click(toggle(t('settings.sources.antiCheat')));
  await vi.waitFor(() => expect(backend.setAntiCheatCalls).toEqual([true]));
  expect(patches).toEqual([]);
});

test('anti-cheat failure is shown', async () => {
  const { backend } = await setup(connected());
  backend.setAntiCheatError = 'persist_failed';
  await fireEvent.click(toggle(t('settings.sources.antiCheat')));
  expect(await screen.findByText(t('service.action.failed'))).toBeTruthy();
});

test('service controls are disabled without the service', async () => {
  for (const service of [null, { state: 'unreachable', detail: null, pawnIo: null, sources: null } as ServiceStatus]) {
    await setup(service);
    for (const module of MODULES) expect(toggle(t(`settings.sources.module.${module}`)).disabled).toBe(true);
    expect(toggle('Disk 0 (C:)').disabled).toBe(true);
    expect(screen.getByText(t('settings.sources.needsService'))).toBeTruthy();
    // The anti-cheat mode and the GPU libraries do not need the service.
    expect(toggle(t('settings.sources.antiCheat')).disabled).toBe(false);
    expect(toggle(t('settings.sources.vendor.nvml')).disabled).toBe(false);
    cleanup();
    disconnectSettings();
  }
});

test('the service status reuses the badge explanation and action', async () => {
  const { backend } = await setup({ state: 'unreachable', detail: 'disconnected', pawnIo: null, sources: null });
  expect(screen.getByText(t('service.state.unreachable'))).toBeTruthy();
  expect(screen.getByText(t('service.detail.disconnected'))).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: t('service.action.start') }));
  expect(backend.startServiceCalls).toBe(1);
  cleanup();
  disconnectSettings();
  await setup(connected());
  expect(screen.getByText(t('service.state.connected'))).toBeTruthy();
});

test('module switches patch serviceModules and explain a module kept on by another user', async () => {
  const { patches, view } = await setup(connected(), { sources: { serviceModules: { psu: false } } });
  // psu is off here but the service still runs it for someone else.
  const psu = toggle(t('settings.sources.module.psu'));
  expect(psu.getAttribute('aria-checked')).toBe('false');
  expect(psu.closest('.field')?.textContent).toContain(t('settings.sources.module.keptOn'));
  expect(screen.getAllByText(t('settings.sources.module.keptOn'))).toHaveLength(1);
  await view.rerender({ service: connected({ sources: sources({ activeModules: MODULES.filter((m) => m !== 'psu') }) }) });
  expect(screen.queryByText(t('settings.sources.module.keptOn'))).toBeNull();

  const motherboard = toggle(t('settings.sources.module.motherboard'));
  expect(motherboard.disabled).toBe(false);
  await fireEvent.click(motherboard);
  await vi.waitFor(() => expect(patches).toEqual([{ sources: { serviceModules: { motherboard: false } } }]));
});

test('a module the service has not dropped yet is not blamed on another user', async () => {
  const backend = new FakeBackend(SCHEMA);
  await settings.connect(backend);
  await settings.update({ sources: { serviceModules: { psu: false } } });
  const store = new LiveStore();
  store.applySchema(SCHEMA);
  render(SourcesSection, { store, backend, service: connected() });
  const state = backend.settings.state();
  backend.emitSettings({ ...state, seq: settings.state!.seq + 1, applyStatus: { ...state.applyStatus, service: { kind: 'pending' } } });
  expect(await screen.findByText(t('settings.sources.applying'))).toBeTruthy();
  expect(screen.queryByText(t('settings.sources.module.keptOn'))).toBeNull();
});

test('smart switches only for disks with a descriptor key, with the limit always shown', async () => {
  const { patches } = await setup(connected(), { sources: { smartDisabledDrives: ['storage/unplugged'] } });
  const ssd = toggle('Disk 0 (C:)');
  const usb = toggle('USB disk');
  expect(ssd.disabled).toBe(false);
  expect(ssd.getAttribute('aria-checked')).toBe('true');
  expect(usb.disabled).toBe(true);
  expect(usb.closest('.field')?.textContent).toContain(t('settings.sources.smart.noDescriptor'));
  expect(screen.getByText(t('settings.sources.smart.limit'))).toBeTruthy();

  await fireEvent.click(ssd);
  // Disks that are not plugged in now keep their choice.
  await vi.waitFor(() => expect(patches).toEqual([{ sources: { smartDisabledDrives: ['storage/unplugged', SSD] } }]));
  await vi.waitFor(() => expect(toggle('Disk 0 (C:)').getAttribute('aria-checked')).toBe('false'));
  await fireEvent.click(toggle('Disk 0 (C:)'));
  await vi.waitFor(() => expect(patches[1]).toEqual({ sources: { smartDisabledDrives: ['storage/unplugged'] } }));
});

test('smart switches wait for the disks module', async () => {
  await setup(connected(), { sources: { serviceModules: { storage: false } } });
  expect(toggle('Disk 0 (C:)').disabled).toBe(true);
  expect(screen.getByText(t('settings.sources.smart.storageOff'))).toBeTruthy();
});

test('a closed smart gate names the disk, or an unknown one', async () => {
  const { view } = await setup(connected({ sources: sources({ smartBlockedBy: [SSD, '0123abcd'] }) }));
  expect(
    screen.getByText(t('settings.sources.smart.blocked', { disk: `Disk 0 (C:), ${t('settings.sources.smart.unknownDisk')}` })),
  ).toBeTruthy();
  await view.rerender({ service: connected({ sources: sources({ smartBlockedBy: [] }) }) });
  expect(screen.queryByText((text) => text.startsWith('SMART is off for all disks because'))).toBeNull();
  expect(screen.getByText(t('settings.sources.smart.limit'))).toBeTruthy();
});

test('the pawnio state is explained', async () => {
  const states: PawnIoStatus[] = ['ok', 'missing', 'unavailable', 'unknown', 'rebootPending'];
  for (const pawnIo of states) {
    await setup(connected({ pawnIo }));
    expect(screen.getByText(t(`settings.sources.pawnIo.${pawnIo}`))).toBeTruthy();
    cleanup();
    disconnectSettings();
  }
  await setup({ state: 'unreachable', detail: null, pawnIo: null, sources: null });
  expect(screen.queryByText(t('settings.sources.pawnIo'))).toBeNull();
});

test('service apply status: pending and failed', async () => {
  const backend = new FakeBackend(SCHEMA);
  await settings.connect(backend);
  const store = new LiveStore();
  store.applySchema(SCHEMA);
  render(SourcesSection, { store, backend, service: connected() });
  const emit = (service: EffectStatus) => {
    const state = backend.settings.state();
    backend.emitSettings({ ...state, seq: settings.state!.seq + 1, applyStatus: { ...state.applyStatus, service } });
  };
  emit({ kind: 'pending' });
  expect(await screen.findByText(t('settings.sources.applying'))).toBeTruthy();
  emit({ kind: 'failed', reason: 'reconfigurationFailed' });
  expect(
    await screen.findByText(t('settings.sources.applyFailed', { reason: t('settings.reason.reconfigurationFailed') })),
  ).toBeTruthy();
  expect(screen.queryByText(t('settings.sources.applying'))).toBeNull();
});
