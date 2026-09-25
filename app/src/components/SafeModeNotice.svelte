<script lang="ts">
  import { t } from '../lib/i18n/index.svelte';
  import type { StartupStatus } from '../lib/types';

  let { status, onEnable }: { status: StartupStatus; onEnable: () => Promise<void> } = $props();

  let busy = $state(false);
  const message = $derived(
    status.reason === 'crash'
      ? t('safe.crash', { module: status.crashModule ? t('safe.crashIn', { module: status.crashModule }) : '' })
      : t('safe.flag'),
  );

  async function enable() {
    busy = true;
    try {
      await onEnable();
    } finally {
      busy = false;
    }
  }
</script>

<section class="notice" role="status">
  <div class="text">
    <div class="title">{t('safe.title')}</div>
    <p>{message}</p>
  </div>
  <button type="button" disabled={busy} onclick={enable}>{busy ? t('safe.enabling') : t('safe.enable')}</button>
</section>

<style>
  .notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 14px;
    padding: 12px 16px;
    border: 1px solid color-mix(in srgb, var(--warn) 45%, transparent);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--warn) 8%, var(--surface));
  }
  .title {
    font-weight: 600;
    color: var(--warn);
  }
  p {
    margin: 2px 0 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  button {
    flex: none;
    padding: 6px 14px;
    border: 1px solid color-mix(in srgb, var(--warn) 60%, transparent);
    border-radius: 8px;
    background: var(--surface-2);
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.6;
    cursor: progress;
  }
</style>
