<script lang="ts">
  // Root of the `overlay-editor` window: the source palette and the canvas (D14); D15 adds the
  // toolbar, the properties and the unsaved-changes dialog. It opens the profile and keeps the
  // language, the sensors, the canvas's frame data and the shell's view of the editor in step.
  import { onDestroy, onMount } from 'svelte';
  import { createBackend, type Backend } from '../lib/backend';
  import { EditorStore } from '../lib/editor/editor.svelte';
  import { t } from '../lib/i18n/index.svelte';
  import { LiveStore, connect } from '../lib/live.svelte';
  import { settings } from '../lib/settings.svelte';
  import Canvas from './Canvas.svelte';
  import { FrameFeed } from './feed.svelte';
  import Palette from './Palette.svelte';

  let { backend = createBackend() }: { backend?: Backend } = $props();
  // svelte-ignore state_referenced_locally
  const editor = new EditorStore(backend);
  const live = new LiveStore();
  const feed = new FrameFeed();
  let canvas: Canvas | undefined = $state();
  /** The first profile is open: edits made before would be replaced by it. */
  let ready = $state(false);

  onMount(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    const stops: (() => void)[] = [];
    // The sensors (with their history) and the frame data the canvas draws.
    for (const start of [() => connect(live, backend), () => feed.connect(backend)]) {
      start().then(
        (stop) => (cancelled ? stop() : stops.push(stop)),
        (error) => console.error('editor: canvas data unavailable', error),
      );
    }
    void (async () => {
      try {
        off = await settings.connect(backend);
      } catch (error) {
        console.error('editor: settings unavailable', error);
      }
      if (cancelled) return;
      // The profile the overlay shows now, else the default one; else a new profile, named only
      // now that the settings have set the language.
      const status = await backend.getOverlayStatus().catch(() => null);
      const id = status?.activeProfile ?? settings.state?.settings.overlay.defaultProfile;
      if (cancelled) return;
      if (id === undefined || !(await editor.load(id))) editor.newProfile();
      ready = true;
    })();
    return () => {
      cancelled = true;
      off?.();
      stops.forEach((stop) => stop());
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
  {#if ready}
  <div class="body">
    <Palette schema={live.schema} onAdd={(source) => canvas?.addSource(source)} onDrop={(source, x, y) => canvas?.dropAt(source, x, y)} />
    <Canvas bind:this={canvas} {editor} {live} {feed} />
  </div>
  {/if}
</main>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 10px;
    box-sizing: border-box;
    height: 100vh;
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
    margin: 0;
    color: var(--text-muted);
  }
  .body {
    display: grid;
    flex: 1;
    grid-template-columns: 240px minmax(0, 1fr);
    gap: 16px;
    min-height: 0;
  }
  .error {
    color: var(--crit);
  }
</style>
