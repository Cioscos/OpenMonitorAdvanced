<script lang="ts">
  import { sensorLabel } from '../lib/advanced/labels';
  import { sidebarEntries } from '../lib/advanced/nav';
  import { categoryLabel, groupSensors } from '../lib/advanced/pages';
  import { FRAME_METRICS, type Source } from '../lib/editor/profile';
  import { t } from '../lib/i18n/index.svelte';
  import type { Schema } from '../lib/types';

  // The sources a block can show (§7.1): the sensors by device and category, the frame metrics
  // and a new text. Each item is dragged onto the canvas with pointer events, or added with Enter
  // at the first free cell.
  let {
    schema,
    onAdd,
    onDrop,
  }: {
    schema: Schema | null;
    /** Enter on an item. */
    onAdd: (source: Source) => unknown;
    /** An item released at a point of the page; the canvas decides whether it lands on it. */
    onDrop: (source: Source, clientX: number, clientY: number) => unknown;
  } = $props();

  let query = $state('');
  const needle = $derived(query.trim().toLocaleLowerCase());
  const searching = $derived(needle !== '');

  const devices = $derived.by(() => {
    if (schema === null) return [];
    const sensors = schema.sensors.map((s) => ({ ...s, name: sensorLabel(s, t) })).filter((s) => !searching || s.name.toLocaleLowerCase().includes(needle));
    return sidebarEntries(schema)
      .map((entry) => {
        const own = sensors.filter((s) => entry.deviceIds.includes(s.deviceId));
        return {
          id: entry.id,
          label: t(entry.labelKey),
          arg: entry.labelArg,
          categories: groupSensors(own).map((g) => ({ key: `${entry.id}|${g.category}`, name: categoryLabel(g.category, t), sensors: g.sensors as typeof own })),
        };
      })
      .filter((d) => d.categories.length > 0);
  });

  /** The item being dragged and where the pointer is, for the ghost label. */
  let drag = $state.raw<{ source: Source; name: string; x: number; y: number; moved: boolean } | null>(null);

  function down(event: PointerEvent, source: Source, name: string) {
    if (event.button !== 0) return;
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
    drag = { source, name, x: event.clientX, y: event.clientY, moved: false };
  }

  function move(event: PointerEvent) {
    if (drag === null) return;
    drag = { ...drag, x: event.clientX, y: event.clientY, moved: true };
  }

  function up(event: PointerEvent) {
    if (drag === null) return;
    const { source } = drag;
    drag = null;
    onDrop(source, event.clientX, event.clientY);
  }

  function key(event: KeyboardEvent, source: Source) {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    onAdd(source);
  }
</script>

{#snippet item(source: Source, name: string)}
  <li>
    <button
      type="button"
      class="item"
      onpointerdown={(e) => down(e, source, name)}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={() => (drag = null)}
      onlostpointercapture={() => (drag = null)}
      onkeydown={(e) => key(e, source)}>{name}</button
    >
  </li>
{/snippet}

<aside class="palette">
  <input
    type="search"
    class="search"
    bind:value={query}
    placeholder={t('editor.palette.search')}
    aria-label={t('editor.palette.search')}
    autocomplete="off"
    spellcheck="false"
  />
  <div class="scroll">
    {#each devices as device (device.id)}
      <details class="device" open>
        <summary>{device.label}{#if device.arg}<span class="arg">{device.arg}</span>{/if}</summary>
        {#each device.categories as category (category.key)}
          <details class="category" open={searching}>
            <summary>{category.name}</summary>
            <ul>
              {#each category.sensors as sensor (sensor.id)}
                {@render item({ sensor: sensor.id }, sensor.name)}
              {/each}
            </ul>
          </details>
        {/each}
      </details>
    {/each}
    <section role="group" aria-labelledby="palette-frames">
      <h2 id="palette-frames">{t('editor.palette.frames')}</h2>
      <ul>
        {#each FRAME_METRICS as metric (metric)}
          {@render item({ frames: metric }, t(`overlay.text.metric.${metric}`))}
        {/each}
      </ul>
    </section>
    <section role="group" aria-labelledby="palette-text">
      <h2 id="palette-text">{t('editor.palette.text')}</h2>
      <ul>
        {@render item({ text: t('editor.palette.newText') }, t('editor.palette.newText'))}
      </ul>
    </section>
  </div>
</aside>

{#if drag?.moved}
  <div class="ghost" style:left="{drag.x + 12}px" style:top="{drag.y + 8}px" aria-hidden="true">{drag.name}</div>
{/if}

<style>
  .palette {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 0;
  }
  .search {
    padding: 6px 10px;
    font: inherit;
    font-size: 13px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .search:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-right: 4px;
  }
  ul {
    margin: 0 0 4px;
    padding: 0;
    list-style: none;
  }
  summary {
    padding: 4px 2px;
    font-size: 12.5px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .device > summary {
    font-weight: 600;
    color: var(--text);
  }
  .arg {
    margin-left: 6px;
    font-weight: 400;
    color: var(--text-muted);
  }
  .category {
    margin-left: 10px;
    border-left: 1px solid var(--border);
    padding-left: 6px;
  }
  h2 {
    margin: 12px 0 4px;
    font-size: 12.5px;
    font-weight: 600;
  }
  .item {
    display: block;
    width: 100%;
    padding: 4px 8px;
    font: inherit;
    font-size: 13px;
    color: var(--text);
    text-align: left;
    cursor: grab;
    touch-action: none;
    background: none;
    border: 1px solid transparent;
    border-radius: 6px;
  }
  .item:hover {
    background: var(--surface-2);
    border-color: var(--border);
  }
  .item:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .ghost {
    position: fixed;
    z-index: 10;
    padding: 3px 8px;
    font-size: 12.5px;
    color: var(--on-accent);
    pointer-events: none;
    background: var(--accent);
    border-radius: 6px;
    box-shadow: 0 0 12px color-mix(in srgb, var(--accent) 60%, transparent);
  }
</style>
