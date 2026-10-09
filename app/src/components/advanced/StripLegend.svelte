<script lang="ts" module>
  /** One series of a strip, with its values already formatted. */
  export type StripLegendRow = {
    id: string;
    label: string;
    color: string;
    dash: readonly number[];
    now: string;
    min: string;
    max: string;
    hidden: boolean;
  };
</script>

<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';

  // The legend of one strip (spec §2.4): uPlot's own legend is off. Sensor names are text only.
  let {
    title,
    symbol = null,
    rows,
    height,
    ontoggle,
    onremove,
  }: {
    /** The unit's name. */
    title: string;
    /** The symbol its values are drawn in, shown after the name in its own case; null for none. */
    symbol?: string | null;
    rows: StripLegendRow[];
    /** CSS height of the strip: the legend never makes it taller, it scrolls instead. */
    height?: number;
    /** The name was clicked: hide or show the line. */
    ontoggle: (id: string) => void;
    /** The × was clicked: drop the series from the selection. */
    onremove: (id: string) => void;
  } = $props();
</script>

<div class="legend" role="group" aria-label={symbol ? `${title} · ${symbol}` : title} style:height={height === undefined ? undefined : `${height}px`} style:overflow-y="auto">
  <p class="title">{title}{#if symbol}{' · '}<span class="symbol">{symbol}</span>{/if}</p>
  <ul>
    {#each rows as row (row.id)}
      <li class="row" class:off={row.hidden}>
        <svg class="swatch" viewBox="0 0 14 8" width="14" height="8" aria-hidden="true">
          <line
            x1="1"
            y1="4"
            x2="13"
            y2="4"
            stroke={row.color}
            stroke-width="2"
            stroke-linecap="round"
            stroke-dasharray={row.dash.length > 0 ? row.dash.join(' ') : undefined}
          />
        </svg>
        <button type="button" class="name" aria-pressed={!row.hidden} title={row.label} onclick={() => ontoggle(row.id)}>
          {row.label}
        </button>
        <span class="now">{row.now}</span>
        <button
          type="button"
          class="remove"
          aria-label={t('advanced.chart.remove', { name: row.label })}
          title={t('advanced.chart.remove', { name: row.label })}
          onclick={() => onremove(row.id)}
        >×</button>
        <span class="range">
          {t('advanced.table.min')} <span class="min">{row.min}</span>
          · {t('advanced.table.max')} <span class="max">{row.max}</span>
        </span>
      </li>
    {/each}
  </ul>
</div>

<style>
  /* Bounded to its strip's height (inline): with many series the legend scrolls, the strip stays put. */
  .legend {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    overscroll-behavior: contain;
    font-size: 12px;
  }
  .title {
    position: sticky;
    top: 0;
    z-index: 1;
    margin: 0;
    padding-bottom: 2px;
    background: var(--surface);
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  /* MHz, bit/s and °F keep their case. */
  .symbol {
    text-transform: none;
  }
  ul {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  /* Swatch, name, value now and ×; min and max in a small line under the name. */
  .row {
    display: grid;
    grid-template-columns: 14px minmax(0, 1fr) auto auto;
    column-gap: 6px;
    align-items: center;
    line-height: 1.25;
  }
  .row.off {
    opacity: 0.45;
  }
  .name {
    overflow: hidden;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .name:hover {
    color: var(--accent);
  }
  .now {
    color: var(--text);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .remove {
    padding: 0 3px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--text-muted);
    font: inherit;
    line-height: 1;
    cursor: pointer;
  }
  .remove:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .range {
    grid-column: 2 / 5;
    color: var(--text-muted);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
