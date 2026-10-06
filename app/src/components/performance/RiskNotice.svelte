<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';

  // «Prima di iniziare» (spec M8 §3.4): shown before a start until «Non mostrare più» is ticked
  // once. Escape and «Annulla» start nothing.
  let { onConfirm, onCancel }: { onConfirm: (dontShowAgain: boolean) => unknown; onCancel: () => unknown } = $props();

  let dontShow = $state(false);
  const focus = (node: HTMLElement) => node.focus();
</script>

<div class="scrim">
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="perf-risk-title"
    aria-describedby="perf-risk-body"
    tabindex="-1"
    onkeydown={(e) => e.key === 'Escape' && onCancel()}
  >
    <h2 id="perf-risk-title">{t('performance.risk.title')}</h2>
    <p id="perf-risk-body">{t('performance.risk.body')}</p>
    <label class="check"><input type="checkbox" bind:checked={dontShow} />{t('performance.risk.dontShow')}</label>
    <div class="actions">
      <button type="button" class="ghost" use:focus onclick={() => onCancel()}>{t('performance.risk.cancel')}</button>
      <button type="button" class="go" onclick={() => onConfirm(dontShow)}>{t('performance.wizard.start')}</button>
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
    width: min(460px, 100%);
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
    margin: 0 0 14px;
    line-height: 1.5;
    color: var(--text-muted);
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 16px;
    font-size: 13px;
  }
  .check input {
    accent-color: var(--accent);
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
  .go {
    font-weight: 600;
    color: var(--on-accent);
    background: var(--accent);
    border: 1px solid var(--accent);
  }
  button:focus-visible,
  .check input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
