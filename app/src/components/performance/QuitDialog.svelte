<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';

  // «Stop the test and quit?» (DA16): the tray's «Quit» while a stress test runs. Escape keeps the test.
  let { onConfirm, onCancel }: { onConfirm: () => unknown; onCancel: () => unknown } = $props();

  const focus = (node: HTMLElement) => node.focus();
</script>

<div class="scrim">
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="perf-quit-title"
    aria-describedby="perf-quit-body"
    tabindex="-1"
    onkeydown={(e) => e.key === 'Escape' && onCancel()}
  >
    <h2 id="perf-quit-title">{t('performance.quit.title')}</h2>
    <p id="perf-quit-body">{t('performance.quit.body')}</p>
    <div class="actions">
      <button type="button" class="ghost" use:focus onclick={() => onCancel()}>{t('performance.quit.cancel')}</button>
      <button type="button" class="danger" onclick={() => onConfirm()}>{t('performance.quit.confirm')}</button>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    padding: 16px;
    background: color-mix(in srgb, var(--bg) 70%, transparent);
  }
  .dialog {
    width: min(420px, 100%);
    padding: 18px 20px 16px;
    background: var(--surface);
    border: 1px solid color-mix(in srgb, var(--warn) 55%, var(--border));
    border-radius: var(--radius);
    box-shadow: 0 0 32px color-mix(in srgb, var(--warn) 20%, transparent);
  }
  h2 {
    margin: 0 0 6px;
    font-size: 15px;
    font-weight: 600;
  }
  p {
    margin: 0 0 16px;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  button {
    padding: 6px 14px;
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    border-radius: 8px;
  }
  .ghost {
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .danger {
    font-weight: 600;
    color: var(--on-accent);
    background: var(--crit);
    border: 1px solid var(--crit);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
