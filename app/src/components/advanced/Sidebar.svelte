<script lang="ts">
  import type { SidebarEntry } from '../../lib/advanced/nav';
  import { t } from '../../lib/i18n/index.svelte';

  let {
    entries,
    selected,
    onSelect,
  }: { entries: SidebarEntry[]; selected: string | null; onSelect: (id: string) => void } = $props();
</script>

<nav class="sidebar" aria-label={t('advanced.sidebar')}>
  {#each entries as entry (entry.id)}
    <button
      type="button"
      class:on={entry.id === selected}
      aria-current={entry.id === selected ? 'page' : undefined}
      onclick={() => onSelect(entry.id)}
    >
      <span class="name">{t(entry.labelKey)}</span>
      {#if entry.labelArg}<span class="arg">{entry.labelArg}</span>{/if}
    </button>
  {/each}
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 4px;
    position: sticky;
    top: 76px;
  }
  button {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 8px 12px;
    text-align: left;
    cursor: pointer;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    border-radius: 0 8px 8px 0;
  }
  button:hover {
    background: var(--surface);
  }
  button.on {
    background: var(--surface-2);
    border-left-color: var(--accent);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .name {
    font-weight: 600;
  }
  .arg {
    overflow: hidden;
    font-size: 12px;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
