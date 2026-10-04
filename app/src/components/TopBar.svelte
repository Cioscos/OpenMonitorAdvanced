<script lang="ts">
  import { t } from '../lib/i18n/index.svelte';
  import { updates } from '../lib/updates.svelte';
  import type { ServiceStatus } from '../lib/types';
  import type { View } from '../lib/view';
  import Recorder from './log/Recorder.svelte';
  import ServiceExplainer from './ServiceExplainer.svelte';

  let {
    view,
    onViewChange,
    onSettings,
    service,
    onLeaveAntiCheat,
    onStartService,
    onOpenLogFolder,
    stale = false,
    gear = $bindable(),
  }: {
    view: View;
    onViewChange: (view: View) => void;
    /** The gear: opens the settings, or leaves them when they are open. */
    onSettings: () => void;
    service: ServiceStatus | null;
    onLeaveAntiCheat: () => Promise<unknown>;
    onStartService: () => Promise<unknown>;
    /** The deck's "Open folder": rejects with a `log.error.*` key or the system's text. */
    onOpenLogFolder: () => Promise<void>;
    stale?: boolean;
    /** The gear button, so focus can return to it when the settings close. */
    gear?: HTMLButtonElement;
  } = $props();
  const hasUpdate = $derived(updates.state?.latest != null);
  const gearLabel = $derived(hasUpdate ? t('settings.titleWithUpdate') : t('settings.title'));

  // Collapsed by default (R23): the badge must not occupy permanent space for users without
  // the service. A command failure opens it so the error is visible.
  let open = $state(false);

  // Spec §2.8: a PawnIO state other than `ok` stays visible while the service is connected.
  const pawnIoProblem = $derived(service?.state === 'connected' && service.pawnIo !== null && service.pawnIo !== 'ok');
  const showBadge = $derived(service !== null && (service.state !== 'connected' || pawnIoProblem));
</script>

<header class="topbar">
  <div class="brand"><span class="logo" aria-hidden="true"></span>{t('app.title')}</div>

  <div class="seg" role="tablist" aria-label={t('view.label')}>
    <button role="tab" aria-selected={view === 'simple'} class:on={view === 'simple'} onclick={() => onViewChange('simple')}>
      {t('view.simple')}
    </button>
    <button
      role="tab"
      aria-selected={view === 'advanced'}
      class:on={view === 'advanced'}
      onclick={() => onViewChange('advanced')}
    >
      {t('view.advanced')}
    </button>
  </div>

  <div class="right">
    {#if stale}
      <span class="stale" role="status">{t('status.stale')}</span>
    {/if}
    <Recorder openFolder={onOpenLogFolder} />
    {#if showBadge && service}
      <details class="badge" bind:open>
        <summary>{pawnIoProblem ? t('settings.sources.pawnIo') : t('service.baseMode')}</summary>
        <div class="panel">
          <ServiceExplainer {service} {onLeaveAntiCheat} {onStartService} onFailed={() => (open = true)} showPawnIo />
        </div>
      </details>
    {/if}
    <button
      class="icon"
      class:on={view === 'settings'}
      type="button"
      title={gearLabel}
      aria-pressed={view === 'settings'}
      bind:this={gear}
      onclick={onSettings}
    >
      <!-- A gear: eight teeth drawn as a dashed ring around the body ring. -->
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" aria-hidden="true" focusable="false">
        <circle cx="12" cy="12" r="9" stroke-width="3" stroke-dasharray="3.53 3.54" />
        <circle cx="12" cy="12" r="6.6" stroke-width="2.4" />
      </svg>
      <span class="sr-only">{gearLabel}</span>
      {#if hasUpdate}
        <span class="dot" aria-hidden="true"></span>
      {/if}
    </button>
  </div>
</header>

<style>
  .topbar {
    position: sticky;
    top: 0;
    z-index: 1;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 12px;
    padding: 12px 20px;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    border-bottom: 1px solid var(--border);
    backdrop-filter: blur(8px);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    font-weight: 600;
  }
  .logo {
    width: 14px;
    height: 14px;
    border-radius: 4px;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
  }
  .seg {
    display: flex;
    padding: 3px;
    border-radius: 10px;
    background: var(--surface-2);
  }
  .seg button {
    padding: 6px 14px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
  }
  .right {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 10px;
  }
  .badge {
    padding: 4px 10px;
    font-size: 12px;
    border-radius: var(--radius);
    color: var(--warn);
    border: 1px solid color-mix(in srgb, var(--warn) 45%, transparent);
  }
  .badge summary {
    cursor: help;
    font-weight: 600;
  }
  .panel {
    max-width: 280px;
    margin-top: 6px;
  }
  .stale {
    padding: 4px 10px;
    font-size: 12px;
    border-radius: 999px;
    color: var(--crit);
    border: 1px solid color-mix(in srgb, var(--crit) 45%, transparent);
  }
  .icon {
    position: relative;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    padding: 0;
    color: var(--text-muted);
    cursor: pointer;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
  }
  .icon:hover {
    color: var(--text);
  }
  .icon.on {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 60%, var(--border));
  }
  .dot {
    position: absolute;
    right: 3px;
    bottom: 3px;
    width: 8px;
    height: 8px;
    background: var(--accent);
    border: 1.5px solid var(--surface);
    border-radius: 50%;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .icon:focus-visible,
  .seg button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
