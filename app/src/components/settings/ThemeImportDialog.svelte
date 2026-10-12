<script lang="ts">
  import { onMount } from 'svelte';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { warningText } from '../../lib/theme/warnings';
  import type { ImportPreview } from '../../lib/types';
  import Term from '../common/Term.svelte';
  import ThemePreview from './ThemePreview.svelte';

  // The summary of a theme file before «Add» (spec M10 §4.2): name, author, preview, the number of
  // variables the file sets and the warnings. Escape cancels; focus returns to the opener on close.
  let {
    preview,
    error = null,
    busy = false,
    onAdd,
    onCancel,
  }: { preview: ImportPreview; error?: string | null; busy?: boolean; onAdd: () => void; onCancel: () => void } = $props();

  let add = $state<HTMLButtonElement>();
  onMount(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    add?.focus();
    return () => opener?.focus();
  });
</script>

<div class="scrim">
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="theme-import-title"
    tabindex="-1"
    onkeydown={(e) => e.key === 'Escape' && !busy && onCancel()}
  >
    <h2 id="theme-import-title">{t('themes.import.title')}</h2>
    <div class="head">
      <div class="thumb"><ThemePreview vars={preview.vars} /></div>
      <div class="who">
        <p class="name">{preview.name}</p>
        {#if preview.author}<p class="muted">{t('themes.by', { author: preview.author })}</p>{/if}
        <p class="muted">{t(`themes.scheme.${preview.scheme}`)}</p>
      </div>
    </div>
    <p>{t('themes.import.summary', { applied: preview.applied })}</p>
    {#if preview.warnings.length > 0}
      <h3>{t('themes.import.warnings')}</h3>
      <ul>
        {#each preview.warnings as w, i (i)}
          {@const text = warningText(w, t, i18n.locale)}
          <!-- A low contrast keeps the CSS names in the title, for theme authors. -->
          <li class:contrast={w.code === 'lowContrast'} title={w.code === 'lowContrast' ? w.detail : undefined}>
            {#if w.code === 'lowContrast'}<Term term="contrast">{text}</Term>{:else}{text}{/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="ghost" disabled={busy} onclick={onCancel}>{t('themes.import.cancel')}</button>
      <button type="button" class="go" bind:this={add} disabled={busy} onclick={onAdd}>{t('themes.import.add')}</button>
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
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(520px, 100%);
    max-height: 100%;
    padding: 18px 20px 16px;
    overflow: auto;
    background: var(--surface);
    border: 1px solid color-mix(in srgb, var(--accent) 55%, var(--border));
    border-radius: var(--radius);
    box-shadow: 0 0 32px color-mix(in srgb, var(--accent) 20%, transparent);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  h3 {
    margin: 4px 0 0;
    font-size: 13px;
    font-weight: 600;
  }
  p {
    margin: 0;
    font-size: 13px;
  }
  .head {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr);
    gap: 14px;
    align-items: center;
  }
  .who {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .name {
    font-size: 15px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  ul {
    max-height: 180px;
    margin: 0;
    padding: 8px 12px 8px 26px;
    overflow: auto;
    font-size: 12.5px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  li {
    overflow-wrap: anywhere;
  }
  li + li {
    margin-top: 4px;
  }
  li.contrast {
    color: var(--warn);
  }
  .error {
    padding: 8px 12px;
    border-left: 3px solid var(--crit);
    background: color-mix(in srgb, var(--crit) 8%, transparent);
    border-radius: 0 8px 8px 0;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
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
  button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  @media (max-width: 520px) {
    .head {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
