<script lang="ts">
  import { onMount } from 'svelte';
  import AdvancedPlaceholder from './components/advanced/AdvancedPlaceholder.svelte';
  import SafeModeNotice from './components/SafeModeNotice.svelte';
  import SimpleView from './components/simple/SimpleView.svelte';
  import TopBar from './components/TopBar.svelte';
  import { createBackend, type Backend } from './lib/backend';
  import { LiveStore, connect } from './lib/live.svelte';
  import type { StartupStatus } from './lib/types';
  import type { View } from './lib/view';

  let { backend = createBackend(), store = new LiveStore() }: { backend?: Backend; store?: LiveStore } = $props();
  let view = $state<View>((localStorage.getItem('oma.view') === 'advanced') ? 'advanced' : 'simple');
  let visible = $state(!document.hidden);
  let startup = $state<StartupStatus | null>(null);
  $effect(() => { localStorage.setItem('oma.view', view); });

  onMount(() => {
    const visibility = () => { visible = !document.hidden; };
    document.addEventListener('visibilitychange', visibility);
    let off: (() => void) | undefined;
    let cancelled = false;
    connect(store, backend)
      .then((unsubscribe) => {
        if (cancelled) unsubscribe();
        else off = unsubscribe;
      })
      .catch((error) => console.error('backend connection failed', error));
    backend
      .getStartupStatus()
      .then((status) => {
        if (!cancelled) startup = status;
      })
      .catch((error) => console.error('startup status unavailable', error));
    return () => {
      cancelled = true;
      document.removeEventListener('visibilitychange', visibility);
      off?.();
    };
  });

  async function enableVendorLibraries() {
    try {
      startup = await backend.enableVendorLibraries();
    } catch (error) {
      console.error('cannot re-enable the GPU vendor libraries', error);
    }
  }
</script>

<TopBar {view} onViewChange={(v) => (view = v)} serviceAvailable={false} />
<main>
  {#if startup?.safeMode}
    <SafeModeNotice status={startup} onEnable={enableVendorLibraries} />
  {/if}
  {#if visible}
  {#if view === 'simple'}
    <SimpleView {store} onOpenAdvanced={() => (view = 'advanced')} />
  {:else}
    <AdvancedPlaceholder />
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
