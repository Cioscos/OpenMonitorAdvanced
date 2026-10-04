<script lang="ts">
  import type { Backend } from '../../lib/backend';
  import { t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import { settings } from '../../lib/settings.svelte';
  import { blockingDiskNames, reasonText } from '../../lib/settingsView';
  import type { Device, ServiceModules, ServiceStatus } from '../../lib/types';
  import ServiceExplainer from '../ServiceExplainer.svelte';
  import Group from './controls/Group.svelte';
  import Toggle from './controls/Toggle.svelte';

  let { store, backend, service }: { store: LiveStore; backend: Backend; service: ServiceStatus | null } = $props();

  const VENDORS = ['nvml', 'nvapi', 'adl', 'igcl'] as const;
  const MODULES: (keyof ServiceModules)[] = ['cpu', 'motherboard', 'memory', 'storage', 'controller', 'psu'];

  const current = $derived(settings.state?.settings ?? null);
  const applyStatus = $derived(settings.state?.applyStatus ?? null);
  const connected = $derived(service?.state === 'connected');
  // The service's global state: other users may keep on what this app turned off.
  const sources = $derived(connected ? (service?.sources ?? null) : null);
  const disks = $derived(store.schema?.devices.filter((d) => d.kind === 'storage') ?? []);
  const storageOn = $derived(current?.sources.serviceModules.storage ?? true);
  const blockedBy = $derived(sources ? blockingDiskNames(sources.drives, store.schema, t) : []);

  let antiCheatFailed = $state(false);

  const errorOf = (field: string) => {
    const key = settings.errors[field];
    return key === undefined ? null : t(key);
  };

  const smartError = $derived(errorOf('sources.smartDisabledDrives') ?? errorOf('sources.smartEnabledDrives'));

  async function setAntiCheat(enabled: boolean) {
    antiCheatFailed = false;
    try {
      await backend.setAntiCheat(enabled);
    } catch {
      antiCheatFailed = true;
    }
  }

  // USB disks start with SMART off: the user switches them on, which is not the same as "not off".
  const startsOff = (disk: Device) => disk.properties?.smartDefault === 'off';
  // Switched off wins over switched on, as in the backend, should a disk be in both lists.
  const smartOn = (disk: Device) => {
    const off = (current?.sources.smartDisabledDrives ?? []).includes(disk.id);
    return startsOff(disk) ? (current?.sources.smartEnabledDrives ?? []).includes(disk.id) && !off : !off;
  };

  function setSmart(disk: Device, on: boolean) {
    const id = disk.id;
    const off = current?.sources.smartDisabledDrives ?? [];
    const enabled = current?.sources.smartEnabledDrives ?? [];
    // Both lists change together and stay disjoint. Disks that are not plugged in now keep their choice.
    const smartDisabledDrives = on ? off.filter((d) => d !== id) : off.includes(id) ? off : [...off, id];
    const smartEnabledDrives = !on ? enabled.filter((d) => d !== id) : startsOff(disk) && !enabled.includes(id) ? [...enabled, id] : enabled;
    return settings.update({ sources: { smartEnabledDrives, smartDisabledDrives } });
  }
</script>

{#if current}
  <Group id="sources-gpu" title={t('settings.sources.group.gpu')}>
    {#each VENDORS as vendor (vendor)}
      <Toggle
        id="vendor-{vendor}"
        label={t(`settings.sources.vendor.${vendor}`)}
        checked={current.sources.vendorLibraries[vendor]}
        error={errorOf(`sources.vendorLibraries.${vendor}`)}
        onChange={(on) => settings.update({ sources: { vendorLibraries: { [vendor]: on } } })}
      />
    {/each}
    <div class="note">
      <p>{t('settings.sources.vendor.note')}</p>
      {#if applyStatus?.vendorLibraries.kind === 'failed'}
        <p class="failed">{t('settings.applyFailed', { reason: reasonText(applyStatus.vendorLibraries.reason, t) })}</p>
      {/if}
    </div>
  </Group>

  <Group id="sources-service" title={t('settings.sources.group.service')}>
    {#if service}
      <div class="row">
        <span class="name">{t('settings.sources.status')}</span>
        <ServiceExplainer
          {service}
          onLeaveAntiCheat={() => backend.setAntiCheat(false)}
          onStartService={() => backend.startService()}
        />
      </div>
    {/if}
    <Toggle
      id="anti-cheat"
      label={t('settings.sources.antiCheat')}
      description={t('settings.sources.antiCheat.hint')}
      checked={current.sources.antiCheat}
      error={antiCheatFailed ? t('service.action.failed') : errorOf('sources.antiCheat')}
      onChange={setAntiCheat}
    />
    {#if connected && service?.pawnIo}
      <div class="row">
        <span class="name">{t('settings.sources.pawnIo')}</span>
        <p class:warn={service.pawnIo !== 'ok'}>{t(`settings.sources.pawnIo.${service.pawnIo}`)}</p>
      </div>
    {/if}
  </Group>

  <Group id="sources-modules" title={t('settings.sources.group.modules')}>
    {#if !connected}
      <p class="note">{t('settings.sources.needsService')}</p>
    {:else if applyStatus?.service.kind === 'pending'}
      <p class="note" role="status">{t('settings.sources.applying')}</p>
    {:else if applyStatus?.service.kind === 'failed'}
      <p class="note failed" role="status">
        {t('settings.sources.applyFailed', { reason: reasonText(applyStatus.service.reason, t) })}
      </p>
    {/if}
    {#each MODULES as module (module)}
      {@const on = current.sources.serviceModules[module]}
      <!-- Only a status that has taken this app's request blames another user: the service status
           arrives on the next tick, after the settings, and may still be from before the request. -->
      {@const keptOn =
        !on &&
        sources?.reconfiguration === 'applied' &&
        sources.requestedDisabledModules.includes(module) &&
        sources.activeModules.includes(module)}
      <Toggle
        id="module-{module}"
        label={t(`settings.sources.module.${module}`)}
        checked={on}
        disabled={!connected}
        error={errorOf(`sources.serviceModules.${module}`)}
        notesIds={keptOn ? [`module-${module}-kept`] : []}
        onChange={(next) => settings.update({ sources: { serviceModules: { [module]: next } } })}
      >
        {#snippet notes()}
          {#if keptOn}<p id="module-{module}-kept">{t('settings.sources.module.keptOn')}</p>{/if}
        {/snippet}
      </Toggle>
    {/each}
  </Group>

  <Group id="sources-disks" title={t('settings.sources.group.disks')}>
    {#if connected && !storageOn}
      <p class="note">{t('settings.sources.smart.storageOff')}</p>
    {/if}
    {#each disks as disk (disk.id)}
      {@const selectable = disk.properties?.smartSelectable === 'true'}
      <Toggle
        id="smart-{disk.id}"
        label={disk.name}
        description={!selectable
          ? t('settings.sources.smart.noDescriptor')
          : startsOff(disk)
            ? t('settings.sources.smart.usbWarning')
            : null}
        checked={smartOn(disk)}
        disabled={!connected || !storageOn || !selectable}
        onChange={(on) => setSmart(disk, on)}
      />
    {:else}
      <p class="note">{t('settings.sources.smart.none')}</p>
    {/each}
    <div class="note">
      {#if blockedBy.length > 0}
        <p class="warn">{t('settings.sources.smart.blocked', { disk: blockedBy.join(', ') })}</p>
      {/if}
      <p>{t('settings.sources.smart.limit')}</p>
      {#if smartError !== null}
        <p class="failed" role="alert">{smartError}</p>
      {/if}
    </div>
  </Group>
{/if}

<style>
  .row {
    display: grid;
    grid-template-columns: minmax(120px, 200px) minmax(0, 1fr);
    gap: 6px 24px;
    padding: 12px 16px;
  }
  .row p {
    margin: 0;
    color: var(--text-muted);
  }
  .name {
    font-weight: 600;
  }
  .note {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 10px 16px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .note p {
    margin: 0;
  }
  .row p.warn,
  .note .warn {
    color: var(--warn);
  }
  .failed,
  .note .failed {
    color: var(--crit);
  }
  @media (max-width: 560px) {
    .row {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
