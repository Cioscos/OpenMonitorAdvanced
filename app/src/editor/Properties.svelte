<script lang="ts">
  import NumberInput from '../components/settings/controls/NumberInput.svelte';
  import type { EditorStore } from '../lib/editor/editor.svelte';
  import { bringForward, commonValue, deleteBlocks, MIXED, pasteBlocks, sendBackward, setPath } from '../lib/editor/ops';
  import { LIMITS, PANEL_DEFAULTS, type Kind, type Threshold, type VisibleIf } from '../lib/editor/profile';
  import { t } from '../lib/i18n/index.svelte';
  import type { Schema } from '../lib/types';
  import ThresholdsEditor from './ThresholdsEditor.svelte';
  import VisibleIfEditor from './VisibleIfEditor.svelte';

  // The selected blocks' properties (§6.2, §6.3, §7.1). With several blocks only the values they
  // share show; a different one shows «—», and an edit sets it on every selected block. Every
  // edit is one undo step through the store; on a built-in the whole panel is disabled.
  let { editor, fonts, schema }: { editor: EditorStore; fonts: readonly string[]; schema: Schema | null } = $props();

  const KINDS: Kind[] = ['text', 'graph', 'meter', 'sparkline', 'gauge'];
  const STYLES = ['labelStyle', 'valueStyle', 'unitStyle'] as const;
  const WEIGHTS = [100, 200, 300, 400, 500, 600, 700, 800, 900];
  const UNITS = ['auto', 'B', 'KB', 'MB', 'GB', 'TB', 'MHz', 'GHz', 'bit/s', 'kbit/s', 'Mbit/s', 'Gbit/s'];

  const sel = $derived(editor.selected);
  const mixedText = $derived(t('editor.props.mixed'));

  const get = (path: string) => commonValue(sel, path);
  /** A number, NaN (an empty field showing «—») when the blocks differ. */
  const num = (path: string) => {
    const v = get(path);
    return typeof v === 'number' ? v : NaN;
  };
  /** A select's value: '' (the «—» option) when the blocks differ. */
  const choice = (path: string) => {
    const v = get(path);
    return v === MIXED || v === undefined ? '' : String(v);
  };
  /** Every selected block has a non-null value at `path`. */
  const present = (path: string) => sel.length > 0 && sel.every((b) => commonValue([b], path) !== null);

  const clamp = (v: number, [lo, hi]: readonly [number, number]) => Math.min(Math.max(v, lo), hi);

  function set(path: string, value: unknown) {
    editor.apply({ ...editor.profile, blocks: setPath(editor.profile.blocks, editor.selection, path, value) });
  }
  /** A colour from `<input type="color">` (#rrggbb), keeping the alpha the blocks share. */
  function setColor(path: string, hex: string) {
    const old = get(path);
    set(path, hex.toUpperCase() + (typeof old === 'string' ? old.slice(7) : ''));
  }
  const rgb = (path: string) => {
    const v = get(path);
    return typeof v === 'string' ? v.slice(0, 7) : '#000000';
  };

  const sources = $derived(sel.map((b) => b.source));
  const allText = $derived(sources.every((s) => 'text' in s));
  const allLow = $derived(sources.every((s) => 'frames' in s && s.frames.startsWith('low-')));
  const kind = $derived(get('kind'));
  const statOp = $derived(get('stat.op'));

  function paste() {
    const r = pasteBlocks(editor.profile, editor.clipboard);
    if (r.ids.length === 0) return;
    editor.apply(r.profile);
    editor.select(r.ids);
  }
  function remove() {
    editor.apply(deleteBlocks(editor.profile, editor.selection));
    editor.select([]);
  }
</script>

{#snippet mixedOption(path: string)}
  {#if get(path) === MIXED}<option value="" disabled>{mixedText}</option>{/if}
{/snippet}

{#snippet number(id: string, label: string, path: string, range: readonly [number, number] | null = null, integer = false)}
  <label for={id}>{label}</label>
  <NumberInput {id} {integer} value={num(path)} placeholder={mixedText} onCommit={(v) => set(path, range ? clamp(v, range) : v)} />
{/snippet}

{#snippet check(id: string, label: string, path: string)}
  <label for={id}>{label}</label>
  <input {id} type="checkbox" checked={get(path) === true} indeterminate={get(path) === MIXED} onchange={(e) => set(path, e.currentTarget.checked)} />
{/snippet}

{#snippet color(id: string, label: string, path: string)}
  <label for={id}>{label}</label>
  <input {id} type="color" value={rgb(path)} title={get(path) === MIXED ? mixedText : undefined} onchange={(e) => setColor(path, e.currentTarget.value)} />
{/snippet}

<aside class="props" aria-labelledby="props-title">
  <h2 id="props-title">{t('editor.props.title')}</h2>
  {#if sel.length === 0}
    <p class="empty">{t('editor.props.none')}</p>
  {:else}
    {#if sel.length > 1}<p class="multi">{t('editor.props.multi', { n: sel.length })}</p>{/if}
    <fieldset class="all" disabled={editor.builtin}>
      <div class="grid">
        <label for="p-kind">{t('editor.props.kind')}</label>
        <select id="p-kind" value={choice('kind')} onchange={(e) => set('kind', e.currentTarget.value)}>
          {@render mixedOption('kind')}
          {#each KINDS as k (k)}<option value={k}>{t(`editor.props.kind.${k}`)}</option>{/each}
        </select>
        {@render number('p-x', t('editor.props.x'), 'rect.x', LIMITS.rect.x, true)}
        {@render number('p-y', t('editor.props.y'), 'rect.y', LIMITS.rect.y, true)}
        {@render number('p-w', t('editor.props.w'), 'rect.w', LIMITS.rect.w, true)}
        {@render number('p-h', t('editor.props.h'), 'rect.h', LIMITS.rect.h, true)}
        {@render number('p-z', t('editor.props.z'), 'z', LIMITS.z, true)}
      </div>
      <div class="actions">
        <button type="button" onclick={() => editor.apply(bringForward(editor.profile, editor.selection))}>{t('editor.props.forward')}</button>
        <button type="button" onclick={() => editor.apply(sendBackward(editor.profile, editor.selection))}>{t('editor.props.backward')}</button>
        <button type="button" onclick={() => editor.copy()}>{t('editor.props.copy')}</button>
        <button type="button" disabled={editor.clipboard.length === 0} onclick={paste}>{t('editor.props.paste')}</button>
        <button type="button" class="danger" onclick={remove}>{t('editor.props.remove')}</button>
      </div>

      <div class="grid">
        {#if allText}
          <label for="p-text">{t('editor.props.text')}</label>
          <input id="p-text" type="text" maxlength={LIMITS.textChars} value={choice('source.text')} placeholder={mixedText} onchange={(e) => set('source.text', e.currentTarget.value)} />
        {/if}
        <label for="p-stat">{t('editor.props.stat')}</label>
        <select id="p-stat" value={choice('stat.op')} onchange={(e) => set('stat.op', e.currentTarget.value)}>
          {@render mixedOption('stat.op')}
          {#each ['current', 'min', 'avg', 'max'] as op (op)}<option value={op}>{t(`editor.props.stat.${op}`)}</option>{/each}
        </select>
        {#if statOp !== 'current' || allLow}
          {@render number('p-window', t('editor.props.window'), 'stat.window', LIMITS.statWindow, true)}
        {/if}
        {#if allLow}
          <label for="p-def">{t('editor.props.definition')}</label>
          <select id="p-def" value={choice('stat.definition')} onchange={(e) => set('stat.definition', e.currentTarget.value)}>
            {@render mixedOption('stat.definition')}
            <option value="integral">{t('editor.props.definition.integral')}</option>
            <option value="percentile">{t('editor.props.definition.percentile')}</option>
          </select>
        {/if}
        <label for="p-label">{t('editor.props.label')}</label>
        <input
          id="p-label"
          type="text"
          maxlength={LIMITS.textChars}
          value={typeof get('style.label') === 'string' ? (get('style.label') as string) : ''}
          placeholder={get('style.label') === MIXED ? mixedText : t('editor.props.auto')}
          onchange={(e) => set('style.label', e.currentTarget.value === '' ? null : e.currentTarget.value)}
        />
        <label for="p-align">{t('editor.props.align')}</label>
        <select id="p-align" value={choice('style.align')} onchange={(e) => set('style.align', e.currentTarget.value)}>
          {@render mixedOption('style.align')}
          {#each ['left', 'center', 'right'] as a (a)}<option value={a}>{t(`editor.props.align.${a}`)}</option>{/each}
        </select>
        <label for="p-decimals">{t('editor.props.decimals')}</label>
        <select
          id="p-decimals"
          value={get('style.decimals') === null ? 'auto' : choice('style.decimals')}
          onchange={(e) => set('style.decimals', e.currentTarget.value === 'auto' ? null : Number(e.currentTarget.value))}
        >
          {@render mixedOption('style.decimals')}
          <option value="auto">{t('editor.props.auto')}</option>
          {#each [0, 1, 2, 3] as d (d)}<option value={String(d)}>{d}</option>{/each}
        </select>
        <label for="p-unit">{t('editor.props.unit')}</label>
        <select id="p-unit" value={choice('style.unit')} onchange={(e) => set('style.unit', e.currentTarget.value)}>
          {@render mixedOption('style.unit')}
          {#each UNITS as u (u)}<option value={u}>{u === 'auto' ? t('editor.props.auto') : u}</option>{/each}
        </select>
      </div>

      {#each STYLES as key (key)}
        {@const p = `style.${key}`}
        <fieldset class="section">
          <legend>{t(`editor.props.${key}`)}</legend>
          <div class="grid">
            <label for="p-{key}-font">{t('editor.props.font')}</label>
            <select id="p-{key}-font" value={choice(`${p}.font`)} onchange={(e) => set(`${p}.font`, e.currentTarget.value)}>
              {@render mixedOption(`${p}.font`)}
              {#if typeof get(`${p}.font`) === 'string' && !fonts.includes(get(`${p}.font`) as string)}
                <option value={get(`${p}.font`)}>{get(`${p}.font`)}</option>
              {/if}
              {#each fonts as f (f)}<option value={f}>{f}</option>{/each}
            </select>
            {@render number(`p-${key}-size`, t('editor.props.size'), `${p}.size`, LIMITS.text.size)}
            <label for="p-{key}-weight">{t('editor.props.weight')}</label>
            <select id="p-{key}-weight" value={choice(`${p}.weight`)} onchange={(e) => set(`${p}.weight`, Number(e.currentTarget.value))}>
              {@render mixedOption(`${p}.weight`)}
              {#each WEIGHTS as w (w)}<option value={String(w)}>{w}</option>{/each}
            </select>
            {@render check(`p-${key}-italic`, t('editor.props.italic'), `${p}.italic`)}
            {@render color(`p-${key}-color`, t('editor.props.color'), `${p}.color`)}
            <label for="p-{key}-outline">{t('editor.props.outline')}</label>
            <input
              id="p-{key}-outline"
              type="checkbox"
              checked={present(`${p}.outline`)}
              indeterminate={get(`${p}.outline`) === MIXED && !present(`${p}.outline`)}
              onchange={(e) => set(`${p}.outline`, e.currentTarget.checked ? { width: 1, color: '#000000' } : null)}
            />
            {#if present(`${p}.outline`)}
              {@render number(`p-${key}-ow`, t('editor.props.lineWidth'), `${p}.outline.width`, LIMITS.text.outlineWidth)}
              {@render color(`p-${key}-oc`, t('editor.props.color'), `${p}.outline.color`)}
            {/if}
            <label for="p-{key}-shadow">{t('editor.props.shadow')}</label>
            <input
              id="p-{key}-shadow"
              type="checkbox"
              checked={present(`${p}.shadow`)}
              indeterminate={get(`${p}.shadow`) === MIXED && !present(`${p}.shadow`)}
              onchange={(e) => set(`${p}.shadow`, e.currentTarget.checked ? { dx: 1, dy: 1, color: '#000000' } : null)}
            />
            {#if present(`${p}.shadow`)}
              {@render number(`p-${key}-dx`, t('editor.props.dx'), `${p}.shadow.dx`, LIMITS.text.shadow)}
              {@render number(`p-${key}-dy`, t('editor.props.dy'), `${p}.shadow.dy`, LIMITS.text.shadow)}
              {@render color(`p-${key}-sc`, t('editor.props.color'), `${p}.shadow.color`)}
            {/if}
          </div>
        </fieldset>
      {/each}

      {#if kind === 'graph' || kind === 'sparkline'}
        <fieldset class="section">
          <legend>{t('editor.props.graph')}</legend>
          <div class="grid">
            <label for="p-gmode">{t('editor.props.graph')}</label>
            <select id="p-gmode" value={choice('style.graph.mode')} onchange={(e) => set('style.graph.mode', e.currentTarget.value)}>
              {@render mixedOption('style.graph.mode')}
              {#each ['line', 'area', 'bars', 'frametime'] as m (m)}<option value={m}>{t(`editor.props.graph.${m}`)}</option>{/each}
            </select>
            {@render number('p-grange', t('editor.props.range'), 'style.graph.rangeS', LIMITS.graph.rangeS, true)}
            <label for="p-gyauto">{t('editor.props.yAuto')}</label>
            <input
              id="p-gyauto"
              type="checkbox"
              checked={get('style.graph.y.mode') === 'auto'}
              indeterminate={get('style.graph.y.mode') === MIXED}
              onchange={(e) => set('style.graph.y.mode', e.currentTarget.checked ? 'auto' : 'fixed')}
            />
            {#if get('style.graph.y.mode') === 'fixed'}
              {@render number('p-gymin', t('editor.props.min'), 'style.graph.y.min')}
              {@render number('p-gymax', t('editor.props.max'), 'style.graph.y.max')}
            {/if}
            {@render color('p-glc', t('editor.props.lineColor'), 'style.graph.line.color')}
            {@render number('p-glw', t('editor.props.lineWidth'), 'style.graph.line.width', LIMITS.graph.lineWidth)}
            {@render color('p-gfc', t('editor.props.fill'), 'style.graph.fill.color')}
            {@render number('p-gfa', t('editor.panel.opacity'), 'style.graph.fill.alpha', LIMITS.graph.fillAlpha)}
            {@render number('p-ggrid', t('editor.props.gridLines'), 'style.graph.gridLines', LIMITS.graph.gridLines, true)}
            {@render check('p-gmam', t('editor.props.showMinAvgMax'), 'style.graph.showMinAvgMax')}
            {@render check('p-gval', t('editor.props.showValue'), 'style.graph.showValue')}
          </div>
        </fieldset>
      {:else if kind === 'meter' || kind === 'gauge'}
        {@const p = `style.${kind}`}
        <fieldset class="section">
          <legend>{t(`editor.props.kind.${kind}`)}</legend>
          <div class="grid">
            <label for="p-orient">{t('editor.props.orientation')}</label>
            <select id="p-orient" value={choice(`${p}.orientation`)} onchange={(e) => set(`${p}.orientation`, e.currentTarget.value)}>
              {@render mixedOption(`${p}.orientation`)}
              <option value="horizontal">{t('editor.props.orientation.horizontal')}</option>
              <option value="vertical">{t('editor.props.orientation.vertical')}</option>
            </select>
            {#each ['min', 'max'] as const as end (end)}
              {@const bound = get(`${p}.${end}`)}
              {@const label = t(end === 'min' ? 'editor.props.rangeMin' : 'editor.props.rangeMax')}
              <label for="p-r{end}">{label}</label>
              <span class="bound">
                <label class="auto">
                  <input
                    type="checkbox"
                    aria-label="{label}: {t('editor.props.auto')}"
                    checked={bound === 'auto'}
                    indeterminate={bound === MIXED}
                    onchange={(e) => set(`${p}.${end}`, e.currentTarget.checked ? 'auto' : { fixed: 0 })}
                  />{t('editor.props.auto')}
                </label>
                {#if typeof bound === 'object' && bound !== null}
                  <NumberInput id="p-r{end}" value={num(`${p}.${end}.fixed`)} placeholder={mixedText} onCommit={(v) => set(`${p}.${end}`, { fixed: v })} />
                {/if}
              </span>
            {/each}
          </div>
        </fieldset>
      {/if}

      <fieldset class="section">
        <legend>{t('editor.thresholds')}</legend>
        {#if get('thresholds') === MIXED}
          <p class="empty">{mixedText}</p>
        {:else}
          <ThresholdsEditor value={get('thresholds') as Threshold[]} onChange={(v) => set('thresholds', v)} />
        {/if}
      </fieldset>

      <fieldset class="section">
        <legend>{t('editor.visibleIf')}</legend>
        {#if get('visibleIf') === MIXED}
          <p class="empty">{mixedText}</p>
        {:else}
          <VisibleIfEditor value={get('visibleIf') as VisibleIf | null} {schema} onChange={(v) => set('visibleIf', v)} />
        {/if}
      </fieldset>

      <fieldset class="section">
        <legend>{t('editor.panel.own')}</legend>
        <div class="grid">
          <label for="p-panel">{t('editor.panel.own')}</label>
          <input
            id="p-panel"
            type="checkbox"
            checked={present('panel')}
            indeterminate={get('panel') === MIXED && !present('panel')}
            onchange={(e) => set('panel', e.currentTarget.checked ? { ...PANEL_DEFAULTS } : null)}
          />
          {#if present('panel')}
            {@render color('p-pc', t('editor.panel.color'), 'panel.color')}
            {@render number('p-po', t('editor.panel.opacity'), 'panel.opacity', LIMITS.panel.opacity)}
            {@render number('p-pr', t('editor.panel.radius'), 'panel.radius', LIMITS.panel.radius)}
            {@render number('p-pp', t('editor.panel.padding'), 'panel.padding', LIMITS.panel.padding, true)}
          {/if}
        </div>
      </fieldset>
    </fieldset>
  {/if}
</aside>

<style>
  .props {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
    overflow: auto;
    padding-right: 4px;
  }
  h2 {
    margin: 0;
    font-size: 12.5px;
    font-weight: 600;
  }
  .empty,
  .multi {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .multi {
    padding: 6px 8px;
    color: var(--text);
    background: color-mix(in srgb, var(--accent-2) 14%, transparent);
    border-left: 2px solid var(--accent-2);
    border-radius: 4px;
  }
  .all {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  .section {
    min-width: 0;
    margin: 0;
    padding: 8px 10px 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  legend {
    padding: 0 4px;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--accent);
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 6px 10px;
    align-items: center;
  }
  .grid > label,
  .props :global(.visible-if > label) {
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .actions button {
    padding: 4px 9px;
    font: inherit;
    font-size: 12px;
    color: var(--text);
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .actions button:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .actions .danger:hover:not(:disabled) {
    color: var(--crit);
    border-color: var(--crit);
  }
  .actions button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .bound {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--text-muted);
  }
  /* The native controls of this panel and of the threshold and visibility editors in it. */
  .props :global(select),
  .props :global(input[type='text']:not([inputmode])) {
    max-width: 150px;
    padding: 4px 8px;
    font: inherit;
    font-size: 12.5px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .props :global(input[inputmode]) {
    width: 72px;
    padding: 4px 8px;
  }
  .props :global(input[type='color']) {
    width: 34px;
    height: 24px;
    padding: 0 2px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .props :global(input[type='checkbox']) {
    justify-self: end;
    accent-color: var(--accent);
  }
  .props :global(:is(select, input, button):focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .props :global(:is(select, input):disabled) {
    cursor: not-allowed;
    opacity: 0.45;
  }
</style>
