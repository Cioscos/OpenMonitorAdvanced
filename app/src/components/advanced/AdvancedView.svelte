<script lang="ts">
  import { untrack } from 'svelte';
  import { resolveSection, sidebarEntries } from '../../lib/advanced/nav';
  import { loadSection, saveSection } from '../../lib/advanced/persist';
  import type { Backend } from '../../lib/backend';
  import { t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { ServiceStatus } from '../../lib/types';
  import DevicePage from './DevicePage.svelte';
  import Sidebar from './Sidebar.svelte';

  let {
    store,
    backend,
    service = null,
    focus = null,
    onFocused,
  }: {
    store: LiveStore;
    backend: Backend;
    service?: ServiceStatus | null;
    /** A device page to open (a clicked toast); `onFocused` reports it handled. */
    focus?: { deviceId: string } | null;
    onFocused?: () => void;
  } = $props();

  // The section the user asked for. It is kept (and stays saved) while its device is
  // missing, so the page comes back when the device does.
  let wanted = $state(loadSection());
  const entries = $derived(store.schema ? sidebarEntries(store.schema) : []);
  const current = $derived(entries.find((e) => e.id === resolveSection(entries, wanted)) ?? null);

  /** The device of the last toast is gone: the page says so until another page is chosen. */
  let deviceGone = $state(false);

  function select(id: string) {
    deviceGone = false;
    wanted = id;
    saveSection(id);
  }

  // A toast's device is looked up once the schema is known: its page, or the notice.
  $effect(() => {
    const request = focus;
    if (request === null || store.schema === null) return;
    untrack(() => {
      const entry = entries.find((e) => e.deviceIds.includes(request.deviceId));
      if (entry) select(entry.id);
      else deviceGone = true;
      onFocused?.();
    });
  });
</script>

<div class="advanced">
  <Sidebar {entries} selected={current?.id ?? null} onSelect={select} />
  {#if current || deviceGone}
    <section class="page">
      {#if deviceGone}<p class="notice" role="status">{t('health.deviceGone')}</p>{/if}
      {#if current}
        <header>
          <h2>{t(current.labelKey)}</h2>
          {#if current.labelArg}<p class="device">{current.labelArg}</p>{/if}
        </header>
        {#key current.id}
          <DevicePage entry={current} {store} {backend} {service} />
        {/key}
      {/if}
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
  .notice {
    margin: 0;
    padding: 10px 14px;
    border: 1px solid var(--warn);
    border-radius: var(--radius);
    color: var(--text);
    font-size: 13px;
  }
  .device {
    margin: 0;
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
