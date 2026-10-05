<script lang="ts">
  import { t } from '../lib/i18n/index.svelte';

  // A red, dismissible banner for an error the user should read and then clear.
  let { message, onDismiss }: { message: string; onDismiss: () => void } = $props();
</script>

<div class="banner" role="alert">
  <span class="text">{message}</span>
  <button type="button" class="close" aria-label={t('editor.error.dismiss')} onclick={onDismiss}>×</button>
</div>

<style>
  .banner {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 8px 8px 8px 14px;
    font-size: 13px;
    line-height: 1.45;
    border: 1px solid color-mix(in srgb, var(--crit) 55%, transparent);
    border-left: 3px solid var(--crit);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--crit) 10%, var(--surface));
    color: var(--text);
  }
  .text {
    flex: 1;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .close {
    flex: none;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--crit);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
  }
  .close:hover {
    background: color-mix(in srgb, var(--crit) 20%, transparent);
  }
  .close:focus-visible {
    outline: 2px solid var(--crit);
  }
</style>
