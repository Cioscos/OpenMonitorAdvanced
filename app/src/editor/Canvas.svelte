<script lang="ts">
  import { onMount } from 'svelte';
  import type { EditorStore } from '../lib/editor/editor.svelte';
  import { placeWithExtra, profileFrame } from '../lib/editor/geometry';
  import { deleteBlocks, duplicateBlocks, firstFreeCell, moveBlocks, pasteBlocks, resizeBlocks, snap } from '../lib/editor/ops';
  import { LIMITS, newBlock, type Block, type Profile, type Source } from '../lib/editor/profile';
  import { i18n, t } from '../lib/i18n/index.svelte';
  import type { LiveStore } from '../lib/live.svelte';
  import { blockRect, byZ, drawProfile, sourceName, type View } from './draw';
  import { makeReadout, type FrameFeed } from './feed.svelte';

  // The profile on a simulated screen (§7.1, §7.3): the blocks drawn like the overlay, moved and
  // resized with hand-written pointer events (DD4), edited from the keyboard, and listed beside
  // the canvas for keyboard selection. Every edit goes through `ops.ts` and the store's history.
  let { editor, live, feed }: { editor: EditorStore; live: LiveStore; feed: FrameFeed } = $props();

  type Resolution = 'current' | '1920x1080' | '2560x1440' | '3840x2160';
  const PRESETS: Resolution[] = ['1920x1080', '2560x1440', '3840x2160'];
  let resolution = $state<Resolution>('current');

  let canvas: HTMLCanvasElement;
  /** The focusable, pointer-handling box around the canvas. */
  let stage: HTMLDivElement;
  let size = $state({ w: 0, h: 0 });

  /** The simulated screen in physical pixels and its dpi. */
  const screenArea = $derived.by(() => {
    if (resolution === 'current') {
      const dpr = window.devicePixelRatio || 1;
      const w = Math.round(window.screen.width * dpr);
      const h = Math.round(window.screen.height * dpr);
      // No screen size (a test page): the commonest monitor.
      if (w > 0 && h > 0) return { w, h, dpi: 96 * dpr };
    }
    const [w, h] = (resolution === 'current' ? '1920x1080' : resolution).split('x').map(Number);
    return { w, h, dpi: 96 };
  });

  interface Layout {
    /** Canvas pixels per screen pixel, and where the screen starts on the canvas. */
    k: number;
    screen: { x: number; y: number; w: number; h: number };
    /** The profile's panel on the canvas; null without blocks. */
    panel: { x: number; y: number; w: number; h: number } | null;
    view: View;
  }

  /** A 1×1 block at cell (0, 0): where an empty profile would put its first cell. */
  const PROBE = newBlock({ text: '' }, { x: 0, y: 0 }, []);
  PROBE.rect = { x: 0, y: 0, w: 1, h: 1 };

  function layout(profile: Profile): Layout {
    const { w, h, dpi } = screenArea;
    const k = Math.min(size.w / w, size.h / h) || 0;
    const sx = (size.w - w * k) / 2;
    const sy = (size.h - h * k) / 2;
    const area = { x: 0, y: 0, w, h };
    const shown = profile.blocks.length > 0 ? profile : { ...profile, blocks: [PROBE] };
    const placed = placeWithExtra(shown, area, dpi, null)!;
    const frame = profileFrame(shown, dpi)!;
    const p = placed.profile!;
    return {
      k,
      screen: { x: sx, y: sy, w: w * k, h: h * k },
      panel: profile.blocks.length > 0 ? { x: sx + p.x * k, y: sy + p.y * k, w: p.w * k, h: p.h * k } : null,
      view: { origin: [sx + (p.x + frame.origin[0]) * k, sy + (p.y + frame.origin[1]) * k], cell: frame.cell * k },
    };
  }

  // ---- drawing, coalesced to one frame per change ----

  const raf = (cb: () => void): number => (typeof requestAnimationFrame === 'function' ? requestAnimationFrame(cb) : (setTimeout(cb, 16) as unknown as number));
  let pending = 0;
  function schedule() {
    if (pending !== 0) return;
    pending = raf(() => {
      pending = 0;
      paint();
    });
  }

  $effect(() => {
    // Everything the picture depends on; no continuous loop (§11).
    void [editor.profile, editor.selection, feed.version, live.timestampMs, live.schema, size.w, size.h, screenArea, i18n.locale];
    schedule();
  });

  const HANDLE = 6;

  function paint() {
    const ctx = canvas?.getContext('2d');
    if (!ctx || size.w === 0) return;
    const dpr = window.devicePixelRatio || 1;
    const bw = Math.round(size.w * dpr);
    const bh = Math.round(size.h * dpr);
    if (canvas.width !== bw || canvas.height !== bh) {
      canvas.width = bw;
      canvas.height = bh;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.w, size.h);
    const L = layout(editor.profile);
    const s = L.screen;
    ctx.fillStyle = '#140F1E';
    ctx.fillRect(s.x, s.y, s.w, s.h);
    // The cell grid, when cells are big enough to tell apart.
    const { cell, origin } = L.view;
    if (cell >= 4) {
      ctx.strokeStyle = 'rgba(255, 79, 216, 0.07)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let x = origin[0] - Math.floor((origin[0] - s.x) / cell) * cell; x <= s.x + s.w; x += cell) {
        ctx.moveTo(Math.round(x) + 0.5, s.y);
        ctx.lineTo(Math.round(x) + 0.5, s.y + s.h);
      }
      for (let y = origin[1] - Math.floor((origin[1] - s.y) / cell) * cell; y <= s.y + s.h; y += cell) {
        ctx.moveTo(s.x, Math.round(y) + 0.5);
        ctx.lineTo(s.x + s.w, Math.round(y) + 0.5);
      }
      ctx.stroke();
    }
    ctx.save();
    ctx.beginPath();
    ctx.rect(s.x, s.y, s.w, s.h);
    ctx.clip();
    drawProfile(ctx, editor.profile, L.view, makeReadout(live, feed, t, i18n.locale));
    ctx.restore();
    if (L.panel !== null) {
      // The profile's footprint as the overlay places it.
      ctx.setLineDash([3, 3]);
      ctx.strokeStyle = 'rgba(76, 201, 240, 0.6)';
      ctx.lineWidth = 1;
      ctx.strokeRect(Math.round(L.panel.x) + 0.5, Math.round(L.panel.y) + 0.5, Math.round(L.panel.w) - 1, Math.round(L.panel.h) - 1);
      ctx.setLineDash([]);
    }
    ctx.strokeStyle = '#FF4FD8';
    ctx.fillStyle = '#FF4FD8';
    ctx.lineWidth = 1.5;
    for (const b of editor.selected) {
      const r = blockRect(b, L.view);
      ctx.strokeRect(r.x, r.y, r.w, r.h);
      ctx.fillRect(r.x + r.w - HANDLE / 2, r.y + r.h - HANDLE / 2, HANDLE, HANDLE);
    }
  }

  onMount(() => {
    const measure = () => {
      const r = canvas.getBoundingClientRect();
      size = { w: r.width, h: r.height };
    };
    measure();
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(measure) : null;
    observer?.observe(canvas);
    return () => {
      observer?.disconnect();
      if (pending !== 0) (typeof cancelAnimationFrame === 'function' ? cancelAnimationFrame : clearTimeout)(pending);
    };
  });

  // ---- pointer ----

  function point(e: { clientX: number; clientY: number }): [number, number] {
    const r = canvas.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  }

  const inside = (r: { x: number; y: number; w: number; h: number }, [x, y]: [number, number]) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h;

  function blockAt(p: [number, number], view: View): Block | null {
    return byZ(editor.profile.blocks).reverse().find((b) => inside(blockRect(b, view), p)) ?? null;
  }

  function handleAt([x, y]: [number, number], view: View): boolean {
    return editor.selected.some((b) => {
      const r = blockRect(b, view);
      return Math.abs(x - (r.x + r.w)) <= HANDLE && Math.abs(y - (r.y + r.h)) <= HANDLE;
    });
  }

  let gesture: { kind: 'move' | 'resize'; start: Profile; ids: string[]; at: [number, number]; cell: number; last: string } | null = null;
  let cursor = $state('default');

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    stage.focus();
    const { view } = layout(editor.profile);
    const p = point(e);
    let kind: 'move' | 'resize' = 'resize';
    if (!handleAt(p, view)) {
      kind = 'move';
      const b = blockAt(p, view);
      const toggle = e.shiftKey || e.ctrlKey || e.metaKey;
      if (b === null) {
        if (!toggle) editor.select([]);
        return;
      }
      if (toggle) {
        const next = new Set(editor.selection);
        if (next.has(b.id)) next.delete(b.id);
        else next.add(b.id);
        editor.select(next);
        if (!next.has(b.id)) return;
      } else if (!editor.selection.has(b.id)) {
        editor.select([b.id]);
      }
    }
    stage.setPointerCapture?.(e.pointerId);
    editor.begin();
    gesture = { kind, start: editor.profile, ids: [...editor.selection], at: p, cell: view.cell, last: '0,0' };
  }

  function move(e: PointerEvent) {
    const p = point(e);
    if (gesture === null) {
      const { view } = layout(editor.profile);
      cursor = handleAt(p, view) ? 'nwse-resize' : blockAt(p, view) ? 'move' : 'default';
      return;
    }
    const dx = snap(p[0] - gesture.at[0], gesture.cell);
    const dy = snap(p[1] - gesture.at[1], gesture.cell);
    if (`${dx},${dy}` === gesture.last) return;
    gesture.last = `${dx},${dy}`;
    const edit = gesture.kind === 'move' ? moveBlocks : resizeBlocks;
    editor.apply(edit(gesture.start, gesture.ids, dx, dy));
  }

  function up() {
    if (gesture === null) return;
    gesture = null;
    editor.commit();
  }

  // ---- adding blocks (from the palette) ----

  function add(source: Source, cell: { x: number; y: number }) {
    const profile = editor.profile;
    if (editor.builtin || profile.blocks.length >= LIMITS.maxBlocks) return;
    const block = newBlock(source, cell, profile.blocks);
    editor.apply({ ...profile, blocks: [...profile.blocks, block] });
    editor.select([block.id]);
  }

  /** Adds a block for `source` at the first free cell, top left first (Enter in the palette). */
  export function addSource(source: Source): void {
    const probe = newBlock(source, { x: 0, y: 0 }, []);
    add(source, firstFreeCell(editor.profile.blocks, probe.rect.w, probe.rect.h));
  }

  /** Adds a block for `source` at the cell under a page point; false when the point is off the canvas. */
  export function dropAt(source: Source, clientX: number, clientY: number): boolean {
    const r = canvas.getBoundingClientRect();
    const [x, y] = [clientX - r.left, clientY - r.top];
    if (x < 0 || y < 0 || x >= r.width || y >= r.height) return false;
    const { view } = layout(editor.profile);
    const cellOf = (v: number, o: number, max: number) => Math.min(Math.max(Math.floor((v - o) / view.cell), 0), max);
    add(source, { x: cellOf(x, view.origin[0], LIMITS.rect.x[1]), y: cellOf(y, view.origin[1], LIMITS.rect.y[1]) });
    return true;
  }

  // ---- keyboard ----

  /** The editing shortcuts shared by the canvas and the block list; true when handled. */
  function shortcut(e: KeyboardEvent): boolean {
    const mod = e.ctrlKey || e.metaKey;
    const key = e.key.toLowerCase();
    const ids = editor.selection;
    if (mod && key === 'z' && !e.shiftKey) editor.undo();
    else if (mod && (key === 'y' || (key === 'z' && e.shiftKey))) editor.redo();
    else if (mod && key === 'c') editor.copy();
    else if (mod && key === 'v') {
      const r = pasteBlocks(editor.profile, editor.clipboard);
      if (r.ids.length > 0 && !editor.builtin) {
        editor.apply(r.profile);
        editor.select(r.ids);
      }
    } else if (mod && key === 'd') {
      const r = duplicateBlocks(editor.profile, ids);
      if (r.ids.length > 0 && !editor.builtin) {
        editor.apply(r.profile);
        editor.select(r.ids);
      }
    } else if ((e.key === 'Delete' || e.key === 'Backspace') && ids.size > 0) editor.apply(deleteBlocks(editor.profile, ids));
    else if (e.key === 'Escape') editor.select([]);
    else return false;
    return true;
  }

  const ARROWS: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };

  function canvasKey(e: KeyboardEvent) {
    const arrow = ARROWS[e.key];
    if (arrow !== undefined && editor.selection.size > 0) {
      const edit = e.shiftKey ? resizeBlocks : moveBlocks;
      editor.apply(edit(editor.profile, editor.selection, arrow[0], arrow[1]));
    } else if (!shortcut(e)) return;
    e.preventDefault();
  }

  // ---- block list ----

  const names = $derived.by(() => {
    void live.schema;
    void i18n.locale;
    const readout = makeReadout(live, feed, t, i18n.locale);
    return new Map(editor.profile.blocks.map((b) => [b.id, sourceName(b.source, readout) || b.id]));
  });
  let active = $state<string | null>(null);

  function pick(id: string, e: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) {
    active = id;
    if (e.shiftKey || e.ctrlKey || e.metaKey) {
      const next = new Set(editor.selection);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      editor.select(next);
    } else editor.select([id]);
  }

  function listKey(e: KeyboardEvent) {
    const blocks = editor.profile.blocks;
    const i = blocks.findIndex((b) => b.id === active);
    let next = -1;
    if (e.key === 'ArrowDown') next = Math.min(i + 1, blocks.length - 1);
    else if (e.key === 'ArrowUp') next = Math.max(i - 1, 0);
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = blocks.length - 1;
    else if (e.key === ' ' && i >= 0) {
      pick(blocks[i].id, { shiftKey: false, ctrlKey: true, metaKey: false });
      e.preventDefault();
      return;
    } else if (shortcut(e)) {
      e.preventDefault();
      return;
    } else return;
    e.preventDefault();
    if (next < 0 || blocks.length === 0) return;
    // Shift extends the selection; otherwise the active block is the selection.
    pick(blocks[next].id, { shiftKey: false, ctrlKey: e.shiftKey && !editor.selection.has(blocks[next].id), metaKey: false });
  }
</script>

<section class="canvas-pane">
  <div class="bar">
    <label class="resolution">
      <span>{t('editor.resolution')}</span>
      <select bind:value={resolution}>
        <option value="current">{t('editor.resolution.current')}</option>
        {#each PRESETS as preset (preset)}
          <option value={preset}>{preset.replace('x', ' × ')}</option>
        {/each}
      </select>
    </label>
  </div>
  <div class="work">
    <!-- An application widget (§7.2): it takes the focus and handles its own pointer and keys. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      bind:this={stage}
      class="stage"
      style:cursor
      role="application"
      aria-label={t('editor.canvas.label')}
      tabindex="0"
      onpointerdown={down}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={up}
      onkeydown={canvasKey}
    >
      <canvas bind:this={canvas} class="canvas"></canvas>
    </div>
    <div class="blocks">
      <h2 id="editor-blocks">{t('editor.canvas.blocks')}</h2>
      <ul
        role="listbox"
        aria-labelledby="editor-blocks"
        aria-multiselectable="true"
        aria-activedescendant={active !== null && names.has(active) ? `editor-block-${active}` : undefined}
        tabindex="0"
        onkeydown={listKey}
      >
        {#each editor.profile.blocks as block (block.id)}
          <!-- The keys are the list's (aria-activedescendant); a click picks the option. -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <li
            id="editor-block-{block.id}"
            role="option"
            aria-selected={editor.selection.has(block.id)}
            tabindex="-1"
            data-kind={block.kind}
            class:active={active === block.id}
            onclick={(e) => pick(block.id, e)}
          >
            {names.get(block.id)}
          </li>
        {/each}
      </ul>
    </div>
  </div>
  <p class="note">{t('editor.canvas.note')}</p>
</section>

<style>
  .canvas-pane {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    min-height: 0;
  }
  .bar {
    display: flex;
    justify-content: flex-end;
  }
  .resolution {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  select {
    padding: 4px 8px;
    font: inherit;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .work {
    display: flex;
    flex: 1;
    gap: 12px;
    min-height: 0;
  }
  .stage {
    flex: 1;
    touch-action: none;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .stage:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .blocks {
    display: flex;
    flex-direction: column;
    width: 180px;
    min-height: 0;
  }
  h2 {
    margin: 0 0 6px;
    font-size: 12.5px;
    font-weight: 600;
  }
  ul {
    flex: 1;
    margin: 0;
    padding: 4px;
    overflow: auto;
    list-style: none;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  ul:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  li {
    padding: 4px 8px;
    overflow: hidden;
    font-size: 13px;
    white-space: nowrap;
    text-overflow: ellipsis;
    cursor: pointer;
    border-left: 2px solid transparent;
    border-radius: 4px;
  }
  li[aria-selected='true'] {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-left-color: var(--accent);
  }
  li.active {
    outline: 1px dashed color-mix(in srgb, var(--accent) 60%, transparent);
  }
  .note {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
