<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend';
  import { asCommandError } from '../../lib/editor/editor.svelte';
  import { t } from '../../lib/i18n/index.svelte';
  import { settings } from '../../lib/settings.svelte';
  import type { ImportPreview, ThemeSummary } from '../../lib/types';
  import ThemeCard from './ThemeCard.svelte';
  import ThemeImportDialog from './ThemeImportDialog.svelte';

  // «Themes» (spec M10 §4): the gallery, import with its summary, export and delete. A click on a
  // tile saves `appearance.theme`; the theme store applies it in every window.
  let { backend }: { backend: Backend } = $props();

  let themes = $state<ThemeSummary[]>([]);
  let error = $state<string | null>(null);
  let pending = $state<ImportPreview | null>(null);
  let pendingError = $state<string | null>(null);
  let busy = $state(false);
  const active = $derived(settings.state?.settings.appearance.theme ?? null);

  const errorText = (e: unknown) => {
    const { key, detail } = asCommandError(e);
    return key.startsWith('themes.error.') ? t(key, { detail: detail ?? '' }) : t('themes.error.io', { detail: detail ?? key });
  };

  async function load() {
    try {
      themes = await backend.themesList();
    } catch (e) {
      error = errorText(e);
    }
  }
  onMount(() => {
    void load();
  });

  /** Runs `action`, showing its failure above the gallery. */
  async function attempt(action: () => Promise<unknown>) {
    error = null;
    try {
      await action();
    } catch (e) {
      error = errorText(e);
    }
  }

  const pick = (id: string) => attempt(() => settings.update({ appearance: { theme: id } }));
  const exportTheme = (id: string) => attempt(() => backend.themeExport(id));
  const remove = (id: string) =>
    attempt(async () => {
      try {
        await backend.themeDelete(id);
      } finally {
        await load();
      }
    });

  const startImport = () =>
    attempt(async () => {
      pendingError = null;
      pending = await backend.themeImportPreview();
    });

  async function confirmImport() {
    if (!pending || busy) return;
    busy = true;
    pendingError = null;
    try {
      const added = await backend.themeImportConfirm(pending.token);
      await settings.update({ appearance: { theme: added.id } });
      await load();
      pending = null;
    } catch (e) {
      pendingError = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="themes">
  <div class="toolbar">
    <button type="button" class="import" onclick={startImport}>{t('themes.import')}</button>
  </div>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <ul class="gallery" aria-label={t('settings.section.themes')}>
    {#each themes as theme (theme.id)}
      <ThemeCard
        {theme}
        active={theme.id === active}
        onPick={() => pick(theme.id)}
        onExport={() => exportTheme(theme.id)}
        onDelete={() => remove(theme.id)}
      />
    {/each}
  </ul>
</div>

{#if pending}
  <ThemeImportDialog preview={pending} error={pendingError} {busy} onAdd={confirmImport} onCancel={() => (pending = null)} />
{/if}

<style>
  .themes {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .import {
    padding: 6px 14px;
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid color-mix(in srgb, var(--accent) 55%, var(--border));
    border-radius: 8px;
  }
  .import:hover {
    border-color: var(--accent);
  }
  .import:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .error {
    margin: 0;
    padding: 8px 12px;
    font-size: 13px;
    border-left: 3px solid var(--crit);
    background: color-mix(in srgb, var(--crit) 8%, transparent);
    border-radius: 0 8px 8px 0;
  }
  .gallery {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 14px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
</style>
