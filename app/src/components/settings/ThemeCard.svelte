<script lang="ts">
  import { tick } from 'svelte';
  import { t } from '../../lib/i18n/index.svelte';
  import type { ThemeSummary } from '../../lib/types';
  import ThemePreview from './ThemePreview.svelte';

  // A tile of the theme gallery (spec M10 §4.1, §4.2): a click applies the theme; a user theme also
  // has «Export» and «Delete», the latter confirmed in place. Names are always text.
  let {
    theme,
    active,
    onPick,
    onExport,
    onDelete,
  }: { theme: ThemeSummary; active: boolean; onPick: () => void; onExport: () => void; onDelete: () => void } = $props();

  let confirming = $state(false);
  let deleteButton = $state<HTMLButtonElement>();
  let keepButton = $state<HTMLButtonElement>();

  // The question takes the focus on its safe answer; «Keep» gives it back to «Delete».
  async function ask() {
    confirming = true;
    await tick();
    keepButton?.focus();
  }
  async function keep() {
    confirming = false;
    await tick();
    deleteButton?.focus();
  }
  const name = $derived(theme.builtin ? t(`themes.builtin.${theme.id}`) : (theme.name ?? t('themes.unnamed')));
</script>

<li class="card" class:on={active}>
  <button type="button" class="pick" aria-current={active ? 'true' : undefined} onclick={onPick}>
    <ThemePreview vars={theme.vars} />
    <span class="text">
      <span class="name">{name}</span>
      <span class="meta">
        {#if theme.author}<span class="author">{t('themes.by', { author: theme.author })}</span>{/if}
        <span class="scheme">{t(`themes.scheme.${theme.scheme}`)}</span>
      </span>
    </span>
    {#if active}<span class="badge">{t('themes.active')}</span>{/if}
  </button>
  {#if !theme.builtin}
    <div class="actions">
      {#if confirming}
        <p role="alert">{t('themes.deleteConfirm')}</p>
        <button type="button" class="danger" onclick={onDelete}>{t('themes.deleteYes')}</button>
        <button type="button" bind:this={keepButton} onclick={keep}>{t('themes.deleteCancel')}</button>
      {:else}
        <button type="button" onclick={onExport}>{t('themes.export')}</button>
        <button type="button" bind:this={deleteButton} onclick={ask}>{t('themes.delete')}</button>
      {/if}
    </div>
  {/if}
</li>

<style>
  .card {
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    transition: border-color 120ms ease;
  }
  .card:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .card.on {
    border-color: var(--accent);
    box-shadow:
      0 0 0 1px var(--accent),
      0 0 22px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .pick {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px 10px 12px;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
    background: none;
    border: 0;
    border-radius: var(--radius);
  }
  .pick:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    padding: 0 2px;
  }
  .name {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .author {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .scheme {
    padding: 0 7px;
    line-height: 18px;
    border: 1px solid var(--border);
    border-radius: 9px;
  }
  .badge {
    position: absolute;
    top: 16px;
    right: 16px;
    padding: 0 9px;
    font-size: 11px;
    font-weight: 700;
    line-height: 20px;
    color: var(--on-accent);
    background: var(--accent);
    border-radius: 10px;
    box-shadow: 0 2px 8px var(--shadow);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 0 10px 10px;
  }
  .actions p {
    flex: 1 1 100%;
    margin: 0;
    font-size: 12.5px;
  }
  .actions button {
    padding: 4px 11px;
    font: inherit;
    font-size: 12.5px;
    color: var(--text);
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 7px;
  }
  .actions button:hover {
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .actions .danger {
    color: var(--crit);
    border-color: color-mix(in srgb, var(--crit) 55%, var(--border));
  }
  .actions button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  @media (prefers-reduced-motion: reduce) {
    .card {
      transition: none;
    }
  }
</style>
