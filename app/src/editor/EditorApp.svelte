<script lang="ts">
  // Root of the `overlay-editor` window. D14 adds the canvas and the palette, D15 the toolbar,
  // the properties and the unsaved-changes dialog; this shell opens the profile and keeps the
  // language and the shell's view of the editor in step.
  import { onDestroy, onMount } from 'svelte';
  import { createBackend, type Backend } from '../lib/backend';
  import { EditorStore } from '../lib/editor/editor.svelte';
  import { t } from '../lib/i18n/index.svelte';
  import { settings } from '../lib/settings.svelte';

  let { backend = createBackend() }: { backend?: Backend } = $props();
  // svelte-ignore state_referenced_locally
  const editor = new EditorStore(backend);

  onMount(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      try {
        off = await settings.connect(backend);
      } catch (error) {
        console.error('editor: settings unavailable', error);
      }
      if (cancelled) return;
      // The profile the overlay shows now, else the default one.
      const status = await backend.getOverlayStatus().catch(() => null);
      const id = status?.activeProfile ?? settings.state?.settings.overlay.defaultProfile;
      if (!cancelled && id !== undefined) await editor.load(id);
    })();
    return () => {
      cancelled = true;
      off?.();
    };
  });

  onDestroy(() => editor.close());
</script>

<main class="editor">
  <header>
    <h1>{t('editor.title')}</h1>
    <p class="profile">
      {editor.profile.name}{#if editor.dirty}<span
          class="dirty"
          aria-hidden="true">●</span
        >{/if}
    </p>
  </header>
  {#if editor.builtin}
    <p class="note">{t('editor.error.readOnly')}</p>
  {/if}
  {#if editor.error !== null}
    <p class="error" role="alert">{t(editor.error.key, { detail: editor.error.detail ?? '' })}</p>
  {/if}
</main>

<style>
  .editor {
    min-height: 100vh;
    padding: 12px 16px;
    background: var(--bg);
    color: var(--text);
    font-family: var(--font);
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 16px;
  }
  h1 {
    margin: 0;
    font-size: 1.1rem;
    font-weight: 600;
  }
  .profile {
    margin: 0;
    color: var(--text-muted);
  }
  .dirty {
    margin-left: 6px;
    color: var(--accent);
  }
  .note {
    color: var(--text-muted);
  }
  .error {
    color: var(--crit);
  }
</style>
