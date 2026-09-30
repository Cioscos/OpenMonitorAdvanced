<script lang="ts">
  import { onMount, tick } from 'svelte';
  import AdvancedView from './components/advanced/AdvancedView.svelte';
  import SafeModeNotice from './components/SafeModeNotice.svelte';
  import SettingsView from './components/settings/SettingsView.svelte';
  import SimpleView from './components/simple/SimpleView.svelte';
  import TopBar from './components/TopBar.svelte';
  import { saveSection } from './lib/advanced/persist';
  import { createBackend, type Backend } from './lib/backend';
  import { LiveStore, connect } from './lib/live.svelte';
  import { initialView, migrateLegacyState, settings } from './lib/settings.svelte';
  import { isStale } from './lib/stale';
  import type { NavigationTarget, ServiceStatus, Session, StartupStatus, ViewKind } from './lib/types';
  import type { View } from './lib/view';

  let { backend = createBackend(), store = new LiveStore() }: { backend?: Backend; store?: LiveStore } = $props();
  // The first view is chosen once the settings and the tray's request are known (see `start`).
  let view = $state<View>('simple');
  let ready = $state(false);
  let visible = $state(!document.hidden);
  let startup = $state<StartupStatus | null>(null);
  let session = $state<Session | null>(null);
  let service = $state<ServiceStatus | null>(null);
  // Until the first snapshot arrives, silence is measured from the moment the window opened.
  const openedAtMs = Date.now();
  let nowMs = $state(openedAtMs);
  const intervalMs = $derived(settings.state?.settings.general.intervalMs ?? session?.intervalMs ?? 1000);
  const stale = $derived(isStale(store.lastReceivedAtMs ?? openedAtMs, nowMs, intervalMs));

  /** The view the settings screen goes back to. */
  let previous = $state<ViewKind>('simple');
  let gear = $state<HTMLButtonElement | undefined>();
  /** The device page a clicked toast asked for, until the Advanced view has opened it. */
  let focus = $state<{ deviceId: string } | null>(null);

  /** Shows a view and remembers Simple/Advanced as the last one (the settings screen is never saved). */
  function showView(next: View) {
    if (next === 'settings' && view !== 'settings') previous = view;
    view = next;
    if (next !== 'settings' && settings.state?.settings.view.last !== next) {
      settings.update({ view: { last: next } });
    }
  }

  /** Follows a tray item or a toast: its view and, for a toast, its device page. */
  function navigate(target: NavigationTarget) {
    if (target.deviceId !== undefined) focus = { deviceId: target.deviceId };
    showView(target.view);
  }

  onMount(() => {
    const visibility = () => { visible = !document.hidden; };
    document.addEventListener('visibilitychange', visibility);
    const clock = setInterval(() => { nowMs = Date.now(); }, 1000);
    let off: (() => void) | undefined;
    let offSettings: (() => void) | undefined;
    let offNavigate: (() => void) | undefined;
    let offService: (() => void) | undefined;
    let cancelled = false;
    // Ordering race (spec §6): a late `getServiceStatus` reply must never overwrite a status
    // already delivered by `oma:service`, so the event subscription is set up first and this
    // flag guards the initial read.
    let serviceEventSeen = false;
    start().catch((error) => {
      console.error('settings unavailable', error);
      if (!cancelled) ready = true;
    });
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
    // Subscribing to `oma:navigate` comes before asking for the pending view: the shell emits the
    // event to an open window, but on a cold WebView2 start the page may not be listening yet, so
    // it also keeps every request until the page takes it. An event that arrives before the first
    // view is chosen wins over the saved preferences; a later one is acknowledged by taking the
    // request.
    async function start() {
      let requested = null as NavigationTarget | null;
      let chosen = false;
      const unlistenNavigate = await backend.onNavigate((target) => {
        if (chosen) {
          navigate(target);
          backend.takePendingView().catch((error) => console.error('navigation request not acknowledged', error));
        } else requested = target;
      });
      if (cancelled) return unlistenNavigate();
      offNavigate = unlistenNavigate;
      const unlistenSettings = await settings.connect(backend);
      if (cancelled) return unlistenSettings();
      offSettings = unlistenSettings;
      await migrateLegacyState(backend, settings, localStorage);
      const pending = await backend.takePendingView();
      if (cancelled) return;
      const target = requested ?? pending;
      view = initialView(settings.state!, target?.view ?? null);
      chosen = true;
      ready = true;
      // A view the tray or a toast asked for is now the one last shown.
      if (target !== null) navigate(target);
    }
    return () => {
      cancelled = true;
      clearInterval(clock);
      document.removeEventListener('visibilitychange', visibility);
      off?.();
      offService?.();
      offSettings?.();
      offNavigate?.();
    };
  });

  async function enableVendorLibraries() {
    try {
      startup = await backend.enableVendorLibraries();
    } catch (error) {
      console.error('cannot re-enable the GPU vendor libraries', error);
    }
  }

  /** Esc and "Back" return to the view shown before, with the focus back on the gear. */
  async function closeSettings() {
    showView(previous);
    await tick();
    gear?.focus();
  }

  function toggleSettings() {
    if (view === 'settings') void closeSettings();
    else showView('settings');
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && view === 'settings' && !event.defaultPrevented) {
      event.preventDefault();
      void closeSettings();
    }
  }

  function openAdvanced(section: string | null) {
    if (section !== null) saveSection(section);
    showView('advanced');
  }
</script>

<svelte:window onkeydown={onKeydown} />

<TopBar
  {view}
  onViewChange={showView}
  onSettings={toggleSettings}
  bind:gear
  {service}
  onLeaveAntiCheat={() => backend.setAntiCheat(false)}
  onStartService={() => backend.startService()}
  {stale}
/>
<main>
  {#if startup?.safeMode}
    <SafeModeNotice status={startup} onEnable={enableVendorLibraries} />
  {/if}
  {#if visible && ready}
    {#if view === 'simple'}
      <SimpleView {store} startedAtMs={session?.startedAtMs ?? null} onOpenAdvanced={openAdvanced} />
    {:else if view === 'advanced'}
      <AdvancedView {store} {backend} {service} {focus} onFocused={() => (focus = null)} />
    {:else}
      <SettingsView {store} {backend} {service} onBack={closeSettings} />
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
