<script lang="ts">
  import { resolveSection, sidebarEntries } from '../../lib/advanced/nav';
  import { loadSection, saveSection } from '../../lib/advanced/persist';
  import type { Backend } from '../../lib/backend';
  import { t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { ServiceStatus } from '../../lib/types';
  import DevicePage from './DevicePage.svelte';
  import Sidebar from './Sidebar.svelte';

  let { store, backend, service = null }: { store: LiveStore; backend: Backend; service?: ServiceStatus | null } = $props();

  // The section the user asked for. It is kept (and stays saved) while its device is
  // missing, so the page comes back when the device does.
  let wanted = $state(loadSection());
  const entries = $derived(store.schema ? sidebarEntries(store.schema) : []);
  const current = $derived(entries.find((e) => e.id === resolveSection(entries, wanted)) ?? null);

  function select(id: string) {
    wanted = id;
    saveSection(id);
  }
</script>

<div class="advanced">
  <Sidebar {entries} selected={current?.id ?? null} onSelect={select} />
  {#if current}
    <section class="page">
      <header>
        <h2>{t(current.labelKey)}</h2>
        {#if current.labelArg}<p class="device">{current.labelArg}</p>{/if}
      </header>
      {#key current.id}
        <DevicePage entry={current} {store} {backend} {service} />
      {/key}
    </section>
  {/if}
</div>

<style>
  .advanced {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr);
    gap: 20px;
    align-items: start;
  }
  .page {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 20px;
  }
  .device {
    margin: 0;
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
