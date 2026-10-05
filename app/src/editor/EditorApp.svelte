<script lang="ts" module>
  /** The editor's own window: what `getCurrentWindow()` gives in the app, a fake in tests. */
  export interface EditorWindow {
    onCloseRequested(handler: (event: { preventDefault(): void }) => unknown): Promise<() => void>;
    destroy(): Promise<void>;
  }

  /** Delay before an edit reaches the preview window (§7.4). */
  export const PREVIEW_DEBOUNCE_MS = 100;
  /** How long the «Preview» button waits for `overlay-preview` before it can be pressed again. */
  export const PREVIEW_OPEN_TIMEOUT_MS = 2000;
</script>

<script lang="ts">
  // Root of the `overlay-editor` window: the toolbar, the source palette, the canvas and the
  // properties, plus the unsaved-changes dialog (§7.2, DD13). It opens the profile and keeps the
  // language, the sensors, the canvas's frame data, the profile catalog, the preview and the
  // shell's view of the editor in step.
  import { isTauri } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onDestroy, onMount } from 'svelte';
  import { createBackend, type Backend } from '../lib/backend';
  import { asCommandError, EditorStore } from '../lib/editor/editor.svelte';
  import { t } from '../lib/i18n/index.svelte';
  import { LiveStore, connect } from '../lib/live.svelte';
  import { settings } from '../lib/settings.svelte';
  import type { OverlayProfileEntry } from '../lib/types';
  import Canvas from './Canvas.svelte';
  import { FrameFeed } from './feed.svelte';
  import Palette from './Palette.svelte';
  import Properties from './Properties.svelte';
  import Toolbar from './Toolbar.svelte';
  import UnsavedDialog from './UnsavedDialog.svelte';

  let {
    backend = createBackend(),
    appWindow = isTauri() ? getCurrentWindow() : null,
  }: { backend?: Backend; appWindow?: EditorWindow | null } = $props();
  // svelte-ignore state_referenced_locally
  const editor = new EditorStore(backend);
  const live = new LiveStore();
  const feed = new FrameFeed();
  let canvas: Canvas | undefined = $state();
  /** The first profile is open: edits made before would be replaced by it. */
  let ready = $state(false);
  let profiles = $state.raw<OverlayProfileEntry[]>([]);
  let fonts = $state.raw<string[]>([]);
  let previewOpen = $state(false);
  let previewError = $state<string | null>(null);
  /** An open request waits for `overlay-preview`; `previewTimer` gives up after a while. */
  let previewBusy = $state(false);
  let previewTimer: ReturnType<typeof setTimeout> | undefined;
  /**
   * What runs once the unsaved changes are saved or discarded, in order, the window's own
   * destruction last (a tray quit asked meanwhile must still reach the shell); empty while
   * nothing asks.
   */
  let pending = $state.raw<{ action: () => unknown; last: boolean }[]>([]);

  onMount(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    const stops: (() => void)[] = [];
    const subscriptions: (() => Promise<() => void>)[] = [
      // The sensors (with their history) and the frame data the canvas draws.
      () => connect(live, backend),
      () => feed.connect(backend),
      () => backend.onOverlayStatus((s) => (profiles = s.profiles)),
      () =>
        backend.onOverlayPreview((e) => {
          previewOpen = e.open;
          previewDone();
        }),
      // «Quit» from the tray with unsaved changes (DD13).
      () => backend.onOverlayEditorQuit(() => ask(() => backend.appQuitConfirmed())),
    ];
    if (appWindow !== null) {
      const win = appWindow;
      subscriptions.push(() =>
        win.onCloseRequested((event) => {
          if (!editor.dirty) return;
          event.preventDefault();
          ask(() => win.destroy(), true);
        }),
      );
    }
    for (const start of subscriptions) {
      start().then(
        (stop) => (cancelled ? stop() : stops.push(stop)),
        (error) => console.error('editor: subscription unavailable', error),
      );
    }
    backend.overlayFontFamilies().then(
      (list) => (fonts = list),
      (error) => console.error('editor: font list unavailable', error),
    );
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
      if (status !== null) {
        profiles = status.profiles;
        previewOpen = status.preview;
      }
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

  onDestroy(() => {
    clearTimeout(previewTimer);
    editor.close();
  });

  /**
   * Runs `action` now, or after the user saved or discarded the changes; a request while the
   * question is open joins the ones already waiting. `last` runs after the others (closing).
   */
  function ask(action: () => unknown, last = false) {
    if (editor.dirty) pending = [...pending, { action, last }];
    else void action();
  }

  async function answer(save: boolean) {
    const actions = [...pending.filter((p) => !p.last), ...pending.filter((p) => p.last)];
    pending = [];
    if (save && !(await editor.save())) return;
    for (const { action } of actions) await action();
  }

  // ---- preview (§7.4): every edit, 100 ms after the last one, while the window is open ----

  /** The profile the preview shows, so reopening or an unchanged profile sends nothing. */
  let previewed: string | null = null;

  async function sendPreview(json: string | null) {
    previewed = json;
    try {
      await backend.overlayPreview(json);
      previewError = null;
    } catch (e) {
      previewError = asCommandError(e).detail ?? '';
      previewDone();
    }
  }

  function previewDone() {
    previewBusy = false;
    clearTimeout(previewTimer);
    previewTimer = undefined;
  }

  $effect(() => {
    if (!previewOpen) return;
    const json = JSON.stringify(editor.profile);
    if (json === previewed) return;
    const timer = setTimeout(() => void sendPreview(json), PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  });

  function togglePreview() {
    if (previewOpen) return sendPreview(null);
    previewBusy = true;
    clearTimeout(previewTimer);
    previewTimer = setTimeout(previewDone, PREVIEW_OPEN_TIMEOUT_MS);
    return sendPreview(JSON.stringify(editor.profile));
  }

  /** «Use now» (DD9): saves the profile (a new one gets its id), then shows it in game. */
  async function useNow() {
    if (!editor.builtin && (editor.dirty || editor.profileId === null) && !(await editor.save())) return;
    const id = editor.profileId;
    if (id === null) return;
    try {
      await backend.overlayUseNow(id);
    } catch (e) {
      editor.error = asCommandError(e);
    }
  }
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
  {#if previewError !== null}
    <p class="error" role="alert">{t('editor.error.preview', { detail: previewError })}</p>
  {/if}
  {#if ready}
    <Toolbar {editor} {profiles} {previewOpen} {previewBusy} onSelect={(id) => ask(() => editor.load(id))} onPreview={togglePreview} onUseNow={useNow} />
    <div class="body">
      <Palette schema={live.schema} onAdd={(source) => canvas?.addSource(source)} onDrop={(source, x, y) => canvas?.dropAt(source, x, y)} />
      <Canvas bind:this={canvas} {editor} {live} {feed} />
      <Properties {editor} {fonts} schema={live.schema} />
    </div>
  {/if}
  {#if pending.length > 0}
    <UnsavedDialog name={editor.profile.name} onSave={() => answer(true)} onDiscard={() => answer(false)} onCancel={() => (pending = [])} />
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
    grid-template-columns: 220px minmax(0, 1fr) 300px;
    gap: 16px;
    min-height: 0;
  }
  .error {
    margin: 0;
    color: var(--crit);
  }
</style>
