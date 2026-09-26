<script lang="ts">
  import { onMount } from 'svelte';
  import AdvancedView from './components/advanced/AdvancedView.svelte';
  import SafeModeNotice from './components/SafeModeNotice.svelte';
  import SimpleView from './components/simple/SimpleView.svelte';
  import TopBar from './components/TopBar.svelte';
  import { saveSection } from './lib/advanced/persist';
  import { createBackend, type Backend } from './lib/backend';
  import { LiveStore, connect } from './lib/live.svelte';
  import { isStale } from './lib/stale';
  import type { ServiceStatus, Session, StartupStatus } from './lib/types';
  import type { View } from './lib/view';

  let { backend = createBackend(), store = new LiveStore() }: { backend?: Backend; store?: LiveStore } = $props();
  let view = $state<View>((localStorage.getItem('oma.view') === 'advanced') ? 'advanced' : 'simple');
  let visible = $state(!document.hidden);
  let startup = $state<StartupStatus | null>(null);
  let session = $state<Session | null>(null);
  let service = $state<ServiceStatus | null>(null);
  // Until the first snapshot arrives, silence is measured from the moment the window opened.
  const openedAtMs = Date.now();
  let nowMs = $state(openedAtMs);
  const stale = $derived(isStale(store.lastReceivedAtMs ?? openedAtMs, nowMs, session?.intervalMs ?? 1000));
  $effect(() => { localStorage.setItem('oma.view', view); });

  onMount(() => {
    const visibility = () => { visible = !document.hidden; };
    document.addEventListener('visibilitychange', visibility);
    const clock = setInterval(() => { nowMs = Date.now(); }, 1000);
    let off: (() => void) | undefined;
    let offService: (() => void) | undefined;
    let cancelled = false;
    // Ordering race (spec §6): a late `getServiceStatus` reply must never overwrite a status
    // already delivered by `oma:service`, so the event subscription is set up first and this
    // flag guards the initial read.
    let serviceEventSeen = false;
    connect(store, backend)
      .then((unsubscribe) => {
        if (cancelled) unsubscribe();
        else off = unsubscribe;
      })
      .catch((error) => console.error('backend connection failed', error));
    backend
      .onServiceStatus((status) => {
        serviceEventSeen = true;
        service = status;
      })
      .then((unsubscribe) => {
        if (cancelled) unsubscribe();
        else offService = unsubscribe;
      })
      .catch((error) => console.error('service status events unavailable', error));
    backend
      .getServiceStatus()
      .then((status) => {
        if (!cancelled && !serviceEventSeen) service = status;
      })
      .catch((error) => console.error('service status unavailable', error));
    backend
      .getStartupStatus()
      .then((status) => {
        if (!cancelled) startup = status;
      })
      .catch((error) => console.error('startup status unavailable', error));
    backend
      .getSession()
      .then((value) => {
        if (!cancelled) session = value;
      })
      .catch((error) => console.error('session unavailable', error));
    return () => {
      cancelled = true;
      clearInterval(clock);
      document.removeEventListener('visibilitychange', visibility);
      off?.();
      offService?.();
    };
  });

  async function enableVendorLibraries() {
    try {
      startup = await backend.enableVendorLibraries();
    } catch (error) {
      console.error('cannot re-enable the GPU vendor libraries', error);
    }
  }

  function openAdvanced(section: string | null) {
    if (section !== null) saveSection(section);
    view = 'advanced';
  }
</script>

<TopBar
  {view}
  onViewChange={(v) => (view = v)}
  {service}
  onLeaveAntiCheat={() => backend.setAntiCheat(false)}
  onStartService={() => backend.startService()}
  {stale}
/>
<main>
  {#if startup?.safeMode}
    <SafeModeNotice status={startup} onEnable={enableVendorLibraries} />
  {/if}
  {#if visible}
  {#if view === 'simple'}
    <SimpleView {store} startedAtMs={session?.startedAtMs ?? null} onOpenAdvanced={openAdvanced} />
  {:else}
    <AdvancedView {store} {backend} {service} />
  {/if}
  {/if}
</main>

<style>
  main {
    max-width: 1100px;
    margin: 0 auto;
    padding: 20px;
  }
</style>
