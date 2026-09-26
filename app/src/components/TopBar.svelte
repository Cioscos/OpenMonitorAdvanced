<script lang="ts">
  import { t } from '../lib/i18n/index.svelte';
  import type { ServiceStatus } from '../lib/types';
  import type { View } from '../lib/view';

  let {
    view,
    onViewChange,
    service,
    onLeaveAntiCheat,
    onStartService,
    stale = false,
  }: {
    view: View;
    onViewChange: (view: View) => void;
    service: ServiceStatus | null;
    onLeaveAntiCheat: () => Promise<unknown>;
    onStartService: () => Promise<unknown>;
    stale?: boolean;
  } = $props();

  let busy = $state(false);
  let failed = $state(false);

  const showBadge = $derived(service !== null && service.state !== 'connected');

  async function run(action: () => Promise<unknown>) {
    busy = true;
    failed = false;
    try {
      await action();
    } catch {
      failed = true;
    } finally {
      busy = false;
    }
  }
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
    {#if showBadge && service}
      <div class="badge" role="status">
        <span class="label">{t('service.baseMode')}</span>
        <div class="panel">
          <p>{t(`service.state.${service.state}`)}</p>
          {#if service.detail}<p>{t(`service.detail.${service.detail}`)}</p>{/if}
          {#if service.state === 'antiCheat'}
            <button type="button" disabled={busy} onclick={() => run(onLeaveAntiCheat)}>{t('service.action.leaveAntiCheat')}</button>
          {:else if service.state === 'unreachable'}
            <button type="button" disabled={busy} onclick={() => run(onStartService)}>{t('service.action.start')}</button>
          {/if}
          {#if failed}<p class="error">{t('service.action.failed')}</p>{/if}
        </div>
      </div>
    {/if}
    <button class="icon" type="button" disabled title={t('settings.comingSoon')} aria-label={t('settings.title')}>⚙</button>
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
  .label {
    font-weight: 600;
  }
  .panel {
    max-width: 280px;
  }
  .panel p {
    margin: 6px 0 0;
    color: var(--text-muted);
  }
  .panel .error {
    color: var(--crit);
  }
  .panel button {
    margin-top: 8px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }
  .panel button:disabled {
    opacity: 0.6;
    cursor: progress;
  }
  .stale {
    padding: 4px 10px;
    font-size: 12px;
    border-radius: 999px;
    color: var(--crit);
    border: 1px solid color-mix(in srgb, var(--crit) 45%, transparent);
  }
  .icon {
    width: 32px;
    height: 32px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
  }
  .icon:disabled {
    opacity: 0.5;
  }
</style>
