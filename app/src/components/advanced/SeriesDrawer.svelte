<script lang="ts">
  import { onMount } from 'svelte';
  import { LEGEND_GAP_PX, LEGEND_PX, TIME_AXIS_HEIGHT, Y_AXIS_PX } from '../../lib/advanced/chartData';
  import type { DrawerGroup } from '../../lib/advanced/drawer';
  import { sensorLabel } from '../../lib/advanced/labels';
  import { t } from '../../lib/i18n/index.svelte';
  import type { Unit } from '../../lib/types';

  // The «Series» drawer (spec §3): a non-modal dialog over the column of lines only, between
  // the strips' Y axes and their legends and above the time labels, so none is ever covered. Sensor names are text only.
  let {
    id,
    groups,
    chosen,
    filter = $bindable(''),
    valueText,
    unitTitle,
    unitSymbol = () => null,
    ontoggle,
    onsetmany,
    onreset,
    onclose,
  }: {
    /** Element id, for the button's `aria-controls`. */
    id?: string;
    /** Sensors matching the filter, by unit (`drawerGroups`). */
    groups: DrawerGroup[];
    chosen: string[];
    filter?: string;
    /** The sensor's value now, formatted like the legend. */
    valueText: (id: string) => string;
    /** Group heading: the same as the legend title of that unit. */
    unitTitle: (unit: Unit) => string;
    /** The symbol after the heading, as in the legend; null for none. */
    unitSymbol?: (unit: Unit) => string | null;
    ontoggle: (id: string) => void;
    /** Ticks (`on`) or unticks the given ids: the group's rows left by the filter. */
    onsetmany: (ids: string[], on: boolean) => void;
    /** Charts the page's default series again. */
    onreset: () => void;
    onclose: () => void;
  } = $props();

  let root: HTMLDivElement;
  let search: HTMLInputElement;
  let twoColumns = $state(false);

  onMount(() => {
    search.focus();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver((entries) => {
      twoColumns = (entries.at(-1)?.contentRect.width ?? 0) >= 520;
    });
    observer.observe(root);
    return () => observer.disconnect();
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    event.preventDefault();
    onclose();
  }
</script>

<div
  bind:this={root}
  {id}
  class="drawer"
  role="dialog"
  aria-label={t('advanced.chart.series')}
  tabindex="-1"
  style:left="{Y_AXIS_PX}px"
  style:right="{LEGEND_PX + LEGEND_GAP_PX}px"
  style:top="0px"
  style:bottom="{TIME_AXIS_HEIGHT}px"
  {onkeydown}
>
  <input
    bind:this={search}
    bind:value={filter}
    class="search"
    type="search"
    placeholder={t('advanced.chart.search')}
    aria-label={t('advanced.chart.search')}
    autocomplete="off"
    spellcheck="false"
  />
  <button type="button" class="reset" onclick={onreset}>{t('advanced.chart.restoreDefaults')}</button>
  <div class="groups" style:grid-template-columns={twoColumns ? 'repeat(2, minmax(0, 1fr))' : 'minmax(0, 1fr)'}>
    {#each groups as group (group.unit)}
      {@const unitName = unitTitle(group.unit)}
      {@const symbol = unitSymbol(group.unit)}
      {@const title = symbol ? `${unitName} · ${symbol}` : unitName}
      {@const ids = group.sensors.map((s) => s.id)}
      <section class="group" role="group" aria-label={title}>
        <header>
          <span class="title">{unitName}{#if symbol}{' · '}<span class="symbol">{symbol}</span>{/if}</span>
          <span class="bulk">
            <button type="button" onclick={() => onsetmany(ids, true)}>{t('advanced.chart.all')}</button>
            <span aria-hidden="true">·</span>
            <button type="button" onclick={() => onsetmany(ids, false)}>{t('advanced.chart.none')}</button>
          </span>
        </header>
        <ul>
          {#each group.sensors as sensor (sensor.id)}
            {@const name = sensorLabel(sensor, t)}
            <li>
              <label>
                <input type="checkbox" aria-label={name} checked={chosen.includes(sensor.id)} onchange={() => ontoggle(sensor.id)} />
                <span class="name" title={name}>{name}</span>
                <span class="value">{valueText(sensor.id)}</span>
              </label>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
</div>

<style>
  /* Translucent, so the lines show through; no backdrop-filter, the chart below repaints every frame. */
  .drawer {
    position: absolute;
    z-index: 2;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    border: 1px solid var(--accent);
    border-radius: 8px;
    box-shadow: 0 10px 30px var(--shadow);
    font-size: 12px;
    animation: drawer-in 150ms ease-out;
  }
  .drawer:focus {
    outline: none;
  }
  @keyframes drawer-in {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .drawer {
      animation: none;
    }
  }
  .search {
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
  }
  .search:focus-visible {
    outline: none;
    border-color: var(--accent);
  }
  /* The search stays put; the groups scroll under it. */
  .groups {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    display: grid;
    gap: 10px 18px;
    align-content: start;
    align-items: start;
  }
  header {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 3px;
    padding-bottom: 3px;
    border-bottom: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 11px;
  }
  /* Same voice as the legend titles. */
  .title {
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .symbol {
    text-transform: none;
  }
  .bulk {
    display: flex;
    gap: 4px;
    white-space: nowrap;
  }
  .reset {
    align-self: flex-end;
    font-size: 11.5px;
  }
  .bulk button,
  .reset {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
    font: inherit;
    cursor: pointer;
  }
  .bulk button:hover,
  .bulk button:focus-visible,
  .reset:hover,
  .reset:focus-visible {
    color: var(--accent);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  label {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 6px;
    padding: 1px 2px;
    border-radius: 4px;
    cursor: pointer;
  }
  label:hover {
    background: var(--surface-2);
  }
  input[type='checkbox'] {
    margin: 0;
    accent-color: var(--accent);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .value {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
</style>
