<script lang="ts">
  import NumberInput from '../components/settings/controls/NumberInput.svelte';
  import type { EditorStore } from '../lib/editor/editor.svelte';
  import { uniqueName } from '../lib/editor/ops';
  import { LIMITS, type Anchor, type Panel } from '../lib/editor/profile';
  import { t, translate } from '../lib/i18n/index.svelte';
  import type { OverlayProfileEntry } from '../lib/types';

  // The editor's bar (§7.1): the profile and its file actions, the profile's placement and
  // panel, the preview, «Use now», undo and redo. On a built-in only the actions that do not
  // change it stay enabled. «Save as», «Rename» and «Delete» ask in a small dialog.
  let {
    editor,
    profiles,
    previewOpen,
    previewBusy,
    onSelect,
    onPreview,
    onUseNow,
  }: {
    editor: EditorStore;
    /** The catalog, built-ins with an i18n key as name. */
    profiles: readonly OverlayProfileEntry[];
    previewOpen: boolean;
    /** An open request waits for the preview window's answer. */
    previewBusy: boolean;
    /** Another profile picked in the selector; the parent asks first when there are changes. */
    onSelect: (id: string) => unknown;
    onPreview: () => unknown;
    onUseNow: () => unknown;
  } = $props();

  const ANCHORS: Anchor[] = ['top-left', 'top', 'top-right', 'left', 'center', 'right', 'bottom-left', 'bottom', 'bottom-right'];

  const label = (p: OverlayProfileEntry) => (p.builtin ? t(p.name) : p.name);
  /** The selector's entries; the open profile is listed even before the catalog knows it. */
  const entries = $derived.by(() => {
    const list = profiles.map((p) => ({ id: p.id, name: label(p) }));
    if (!list.some((p) => p.id === editor.profileId)) list.push({ id: editor.profileId ?? '', name: editor.profile.name });
    return list;
  });
  const ro = $derived(editor.builtin);
  const panel = $derived(editor.profile.panel);

  type Ask = { kind: 'saveAs' | 'rename'; name: string } | { kind: 'delete' };
  let ask = $state<Ask | null>(null);

  function select(target: HTMLSelectElement) {
    const id = target.value;
    // The selector shows the open profile until the new one is loaded (or the switch cancelled).
    target.value = editor.profileId ?? '';
    onSelect(id);
  }

  async function confirm() {
    const a = ask;
    if (a === null) return;
    ask = null;
    if (a.kind === 'delete') {
      await editor.remove();
      return;
    }
    const wanted = a.name.trim();
    if (wanted === '') return;
    // Unique among the saved profiles' names as the selector shows them (DD11), the open
    // profile's own aside for a rename.
    // The built-ins count in both languages, as in the shell's `create()`.
    const others = profiles.flatMap((p) =>
      p.builtin ? [translate('en', p.name), translate('it', p.name)] : a.kind === 'saveAs' || p.id !== editor.profileId ? [p.name] : [],
    );
    const name = uniqueName(others, wanted);
    if (a.kind === 'saveAs') await editor.saveAs(name);
    else editor.rename(name);
  }

  const setPanel = (patch: Partial<Panel>) => editor.apply({ ...editor.profile, panel: { ...panel, ...patch } });
  const clamp = (v: number, [lo, hi]: readonly [number, number]) => Math.min(Math.max(v, lo), hi);
  const focus = (node: HTMLElement) => node.focus();
</script>

<div class="toolbar" role="toolbar" aria-label={t('editor.title')}>
  <div class="row">
    <label class="profile">
      <span>{t('editor.profile')}</span>
      <select value={editor.profileId ?? ''} onchange={(e) => select(e.currentTarget)}>
        {#each entries as entry (entry.id)}<option value={entry.id}>{entry.name}</option>{/each}
      </select>
    </label>
    <div class="cluster">
      <button type="button" class="primary" disabled={ro} onclick={() => editor.save()}>{t('editor.save')}</button>
      <button type="button" onclick={() => (ask = { kind: 'saveAs', name: editor.profile.name })}>{t('editor.saveAs')}</button>
      <button type="button" disabled={ro} onclick={() => (ask = { kind: 'rename', name: editor.profile.name })}>{t('editor.rename')}</button>
      <button type="button" onclick={() => editor.duplicate()}>{t('editor.duplicate')}</button>
      <button type="button" disabled={ro} onclick={() => (ask = { kind: 'delete' })}>{t('editor.delete')}</button>
    </div>
    <div class="cluster">
      <button type="button" onclick={() => editor.importFile()}>{t('editor.import')}</button>
      <button type="button" disabled={editor.profileId === null} onclick={() => editor.exportFile()}>{t('editor.export')}</button>
    </div>
    <div class="cluster">
      <button type="button" disabled={ro || !editor.history.canUndo} onclick={() => editor.undo()}>{t('editor.undo')}</button>
      <button type="button" disabled={ro || !editor.history.canRedo} onclick={() => editor.redo()}>{t('editor.redo')}</button>
    </div>
    <div class="cluster end">
      <button type="button" class:live={previewOpen} aria-pressed={previewOpen} disabled={previewBusy} onclick={() => onPreview()}>
        {previewOpen ? t('editor.preview.close') : t('editor.preview')}
      </button>
      <button type="button" class="primary" title={t('editor.useNow.hint')} onclick={() => onUseNow()}>{t('editor.useNow')}</button>
    </div>
  </div>

  <div class="row layout">
    <div class="anchor" role="radiogroup" aria-label={t('editor.anchor')}>
      {#each ANCHORS as anchor (anchor)}
        <input
          type="radio"
          name="editor-anchor"
          value={anchor}
          aria-label={t(`editor.anchor.${anchor}`)}
          title={t(`editor.anchor.${anchor}`)}
          disabled={ro}
          checked={editor.profile.anchor === anchor}
          onchange={() => editor.apply({ ...editor.profile, anchor })}
        />
      {/each}
    </div>
    <span class="caption">{t('editor.anchor')}</span>
    <fieldset class="group">
      <legend>{t('editor.offset')}</legend>
      <label for="tb-ox">{t('editor.props.x')}</label>
      <NumberInput disabled={ro} id="tb-ox" integer value={editor.profile.offset.x} onCommit={(x) => editor.apply({ ...editor.profile, offset: { ...editor.profile.offset, x: clamp(x, LIMITS.offset) } })} />
      <label for="tb-oy">{t('editor.props.y')}</label>
      <NumberInput disabled={ro} id="tb-oy" integer value={editor.profile.offset.y} onCommit={(y) => editor.apply({ ...editor.profile, offset: { ...editor.profile.offset, y: clamp(y, LIMITS.offset) } })} />
    </fieldset>
    <label class="group scale">
      <span>{t('editor.scale')}</span>
      <input
        type="range"
        min={LIMITS.scale[0]}
        max={LIMITS.scale[1]}
        step="0.05"
        disabled={ro}
        value={editor.profile.scale}
        onchange={(e) => editor.apply({ ...editor.profile, scale: Math.round(Number(e.currentTarget.value) * 20) / 20 })}
      />
      <output>×{editor.profile.scale.toFixed(2)}</output>
    </label>
    <fieldset class="group">
      <legend>{t('editor.panel')}</legend>
      <input
        type="color"
        disabled={ro}
        aria-label={t('editor.panel.color')}
        title={t('editor.panel.color')}
        value={panel.color.slice(0, 7)}
        onchange={(e) => setPanel({ color: e.currentTarget.value.toUpperCase() + panel.color.slice(7) })}
      />
      <label for="tb-po">{t('editor.panel.opacity')}</label>
      <NumberInput disabled={ro} id="tb-po" value={panel.opacity} onCommit={(v) => setPanel({ opacity: clamp(v, LIMITS.panel.opacity) })} />
      <label for="tb-pr">{t('editor.panel.radius')}</label>
      <NumberInput disabled={ro} id="tb-pr" value={panel.radius} onCommit={(v) => setPanel({ radius: clamp(v, LIMITS.panel.radius) })} />
      <label for="tb-pp">{t('editor.panel.padding')}</label>
      <NumberInput disabled={ro} id="tb-pp" integer value={panel.padding} onCommit={(v) => setPanel({ padding: clamp(v, LIMITS.panel.padding) })} />
    </fieldset>
  </div>
</div>

{#if ask !== null}
  <div class="scrim">
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      aria-label={ask.kind === 'saveAs' ? t('editor.saveAs') : ask.kind === 'rename' ? t('editor.rename') : t('editor.delete')}
      onkeydown={(e) => e.key === 'Escape' && (ask = null)}
    >
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void confirm();
        }}
      >
        {#if ask.kind === 'delete'}
          <p>{t('editor.delete.confirm', { name: editor.profile.name })}</p>
        {:else}
          <label for="tb-name">{t('editor.name')}</label>
          <input id="tb-name" type="text" maxlength={LIMITS.textChars} bind:value={ask.name} use:focus autocomplete="off" spellcheck="false" />
        {/if}
        <div class="actions">
          <button type="button" onclick={() => (ask = null)}>{t('editor.unsaved.cancel')}</button>
          <button type="submit" class="primary" class:danger={ask.kind === 'delete'}>
            {ask.kind === 'saveAs' ? t('editor.save') : ask.kind === 'rename' ? t('editor.rename') : t('editor.delete')}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .toolbar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  .layout {
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
  .cluster {
    display: flex;
    gap: 4px;
  }
  .end {
    margin-left: auto;
  }
  .profile,
  .group,
  .scale {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 0;
    font-size: 12.5px;
    color: var(--text-muted);
    border: 0;
  }
  legend {
    float: left;
    margin-right: 4px;
    padding: 0;
    color: var(--text);
  }
  .toolbar :global(input[inputmode]) {
    width: 56px;
    padding: 4px 8px;
  }
  select,
  input[type='text'] {
    padding: 4px 8px;
    font: inherit;
    font-size: 13px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .profile select {
    min-width: 180px;
    max-width: 260px;
  }
  button {
    padding: 4px 11px;
    font: inherit;
    font-size: 12.5px;
    color: var(--text);
    white-space: nowrap;
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  button:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  button.primary {
    font-weight: 600;
    color: var(--on-accent);
    background: var(--accent);
    border-color: var(--accent);
  }
  button.live {
    color: var(--accent-2);
    border-color: var(--accent-2);
    box-shadow: 0 0 10px color-mix(in srgb, var(--accent-2) 40%, transparent);
  }
  button.danger {
    color: var(--text);
    background: var(--crit);
    border-color: var(--crit);
  }
  button:disabled,
  input:disabled,
  select:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  :is(button, select, input):focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  /* Nine anchors as a 3×3 pad: where on the screen the profile sits. */
  .anchor {
    display: grid;
    grid-template-columns: repeat(3, 12px);
    gap: 3px;
    padding: 4px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .anchor input {
    width: 12px;
    height: 12px;
    margin: 0;
    cursor: pointer;
    background: color-mix(in srgb, var(--text-muted) 35%, transparent);
    border-radius: 2px;
    appearance: none;
  }
  .anchor input:checked {
    background: var(--accent);
    box-shadow: 0 0 6px var(--accent);
  }
  .caption {
    margin-left: -8px;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  input[type='range'] {
    width: 110px;
    accent-color: var(--accent);
  }
  output {
    min-width: 3.2em;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  input[type='color'] {
    width: 30px;
    height: 24px;
    padding: 0 2px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
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
    gap: 8px;
    width: min(380px, 100%);
    padding: 16px 18px 14px;
    background: var(--surface);
    border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--border));
    border-radius: var(--radius);
  }
  form {
    display: contents;
  }
  .dialog p {
    margin: 0 0 6px;
  }
  .dialog label {
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
</style>
