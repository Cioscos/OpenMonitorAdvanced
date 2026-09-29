<script lang="ts">
  import { t } from '../lib/i18n/index.svelte';
  import type { ServiceStatus } from '../lib/types';

  // What the sensor service is doing and the one action that helps, if any. The same text and
  // action appear in the top bar's basic-mode badge and in Settings › Data sources.
  let {
    service,
    onLeaveAntiCheat,
    onStartService,
    onFailed = () => {},
  }: {
    service: ServiceStatus;
    onLeaveAntiCheat: () => Promise<unknown>;
    onStartService: () => Promise<unknown>;
    /** Called when the action fails, e.g. to open the badge so the error is visible. */
    onFailed?: () => void;
  } = $props();

  let busy = $state(false);
  let failed = $state(false);

  async function run(action: () => Promise<unknown>) {
    busy = true;
    failed = false;
    try {
      await action();
    } catch {
      failed = true;
      onFailed();
    } finally {
      busy = false;
    }
  }
</script>

<div class="explainer">
  <div class="status" role="status">
    <p>{t(`service.state.${service.state}`)}</p>
    {#if service.detail}<p>{t(`service.detail.${service.detail}`)}</p>{/if}
    {#if failed}<p class="error">{t('service.action.failed')}</p>{/if}
  </div>
  {#if service.state === 'antiCheat'}
    <button type="button" disabled={busy} onclick={() => run(onLeaveAntiCheat)}>{t('service.action.leaveAntiCheat')}</button>
  {:else if service.state === 'unreachable'}
    <button type="button" disabled={busy} onclick={() => run(onStartService)}>{t('service.action.start')}</button>
  {/if}
</div>

<style>
  .explainer {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }
  p {
    margin: 0;
    color: var(--text-muted);
  }
  p + p {
    margin-top: 6px;
  }
  .error {
    color: var(--crit);
  }
  button {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  button:disabled {
    opacity: 0.6;
    cursor: progress;
  }
</style>
