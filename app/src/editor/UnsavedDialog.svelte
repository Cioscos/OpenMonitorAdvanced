<script lang="ts">
  import { t } from '../lib/i18n/index.svelte';

  // Save, discard or cancel before the profile's changes would be lost (§7.2, DD13): closing the
  // window, «Quit» from the tray or opening another profile. Escape cancels.
  let { name, onSave, onDiscard, onCancel }: { name: string; onSave: () => unknown; onDiscard: () => unknown; onCancel: () => unknown } = $props();

  const focus = (node: HTMLElement) => node.focus();
</script>

<div class="scrim">
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="unsaved-title"
    aria-describedby="unsaved-body"
    tabindex="-1"
    onkeydown={(e) => e.key === 'Escape' && onCancel()}
  >
    <h2 id="unsaved-title">{t('editor.unsaved.title')}</h2>
    <p id="unsaved-body">{t('editor.unsaved.body', { name })}</p>
    <div class="actions">
      <button type="button" class="ghost" onclick={() => onCancel()}>{t('editor.unsaved.cancel')}</button>
      <button type="button" class="ghost" onclick={() => onDiscard()}>{t('editor.unsaved.discard')}</button>
      <button type="button" class="primary" use:focus onclick={() => onSave()}>{t('editor.unsaved.save')}</button>
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
    border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--border));
    border-radius: var(--radius);
    box-shadow: 0 0 32px color-mix(in srgb, var(--accent-2) 25%, transparent);
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
  .primary {
    font-weight: 600;
    color: var(--on-accent);
    background: var(--accent);
    border: 1px solid var(--accent);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
