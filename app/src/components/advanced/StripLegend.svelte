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
    rows,
    ontoggle,
    onremove,
  }: {
    title: string;
    rows: StripLegendRow[];
    /** The name was clicked: hide or show the line. */
    ontoggle: (id: string) => void;
    /** The × was clicked: drop the series from the selection. */
    onremove: (id: string) => void;
  } = $props();
</script>

<div class="legend" role="group" aria-label={title}>
  <p class="title">{title}</p>
  <div class="head" aria-hidden="true">
    <span>{t('advanced.table.current')}</span>
    <span>{t('advanced.table.min')}</span>
    <span>{t('advanced.table.max')}</span>
  </div>
  <ul>
    {#each rows as row (row.id)}
      <li class="row" class:off={row.hidden}>
        <svg class="swatch" viewBox="0 0 18 10" width="18" height="10" aria-hidden="true">
          <line
            x1="1"
            y1="5"
            x2="17"
            y2="5"
            stroke={row.color}
            stroke-width="2"
            stroke-linecap="round"
            stroke-dasharray={row.dash.length > 0 ? row.dash.join(' ') : undefined}
          />
        </svg>
        <button type="button" class="name" aria-pressed={!row.hidden} title={row.label} onclick={() => ontoggle(row.id)}>
          {row.label}
        </button>
        <button
          type="button"
          class="remove"
          aria-label={t('advanced.chart.remove', { name: row.label })}
          title={t('advanced.chart.remove', { name: row.label })}
          onclick={() => onremove(row.id)}
        >×</button>
        <span class="values">
          <span class="now">{row.now}</span>
          <span class="min">{row.min}</span>
          <span class="max">{row.max}</span>
        </span>
      </li>
    {/each}
  </ul>
</div>

<style>
  .legend {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    font-size: 12px;
  }
  .title {
    margin: 0;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .head,
  .values {
    display: grid;
    grid-template-columns: 1.3fr 1fr 1fr;
    gap: 6px;
  }
  .head {
    padding-left: 24px;
    color: var(--text-muted);
    font-size: 10px;
  }
  ul {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto;
    column-gap: 6px;
    align-items: center;
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
  .remove {
    padding: 0 4px;
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
  .values {
    grid-column: 2 / 4;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
  .now {
    color: var(--text);
    font-weight: 600;
  }
</style>
