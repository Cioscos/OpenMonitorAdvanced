<script lang="ts">
  import { sensorLabel } from '../../lib/advanced/labels';
  import { categoryLabel, formatAverage, groupSensors, sourceCode } from '../../lib/advanced/pages';
  import type { StatsPoller } from '../../lib/advanced/statsPoller.svelte';
  import { formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { ValueOf } from '../../lib/select';
  import type { Sensor } from '../../lib/types';
  import { openSettings } from '../../lib/view';

  let {
    sensors,
    valueOf,
    qualityOf,
    stats,
    rate = 'bytes',
  }: {
    sensors: Sensor[];
    valueOf: ValueOf;
    /** 2 marks a value repeated while the sensor is suspended (a disk in standby): the last reading. */
    qualityOf: (id: string) => 0 | 1 | 2;
    stats: StatsPoller;
    /** 'bits' on network pages: traffic in bit/s, like the Simple view. */
    rate?: 'bits' | 'bytes';
  } = $props();

  const groups = $derived(groupSensors(sensors));
  const locale = $derived(i18n.locale);
  const opts = $derived({ rate });
  let resetting = $state(false);

  async function reset() {
    resetting = true;
    try {
      await stats.reset();
    } finally {
      resetting = false;
    }
  }
</script>

<section class="sensors">
  <div class="bar">
    <button type="button" class="reset" disabled={resetting} onclick={reset}>{t('advanced.table.reset')}</button>
  </div>
  <table>
    <thead>
      <tr>
        <th scope="col">{t('advanced.table.sensor')}</th>
        <th scope="col" class="num">{t('advanced.table.current')}</th>
        <th scope="col" class="num">{t('advanced.table.min')}</th>
        <th scope="col" class="num">{t('advanced.table.max')}</th>
        <th scope="col" class="num">{t('advanced.table.avg')}</th>
        <th scope="col" class="act"><span class="visually-hidden">{t('advanced.table.actions')}</span></th>
      </tr>
    </thead>
    {#each groups as group (group.category)}
      <tbody>
        <tr class="group">
          <th scope="rowgroup" colspan="6">{categoryLabel(group.category, t)}</th>
        </tr>
        {#each group.sensors as sensor (sensor.id)}
          {@const s = stats.statsOf(sensor.id)}
          {@const current = valueOf(sensor.id)}
          {@const stale = current !== null && qualityOf(sensor.id) === 2}
          <tr tabindex="0">
            <th scope="row">
              <span class="name">{sensorLabel(sensor, t)}</span>
              {#if sensor.experimental}<span class="tag exp">{t('advanced.experimental')}</span>{/if}
              <span class="tag source" title={t(`source.${sensor.source}`)}>{sourceCode(sensor.source)}</span>
            </th>
            <td class="num" class:stale
              >{#if stale}<span class="last">{t('value.lastReading')}</span> {/if}{formatValue(current, sensor.unit, locale, t, opts)}</td
            >
            <td class="num">{formatValue(s?.min ?? null, sensor.unit, locale, t, opts)}</td>
            <td class="num">{formatValue(s?.max ?? null, sensor.unit, locale, t, opts)}</td>
            <td class="num">{formatAverage(s, sensor.unit, locale, t, opts)}</td>
            <td class="act">
              <button
                type="button"
                class="rule"
                aria-label={t('advanced.table.createRule')}
                title={t('advanced.table.createRule')}
                onclick={() => openSettings({ section: 'rules', newRuleSensor: sensor.id })}
              >
                <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
                  <path d="M8 3.5v9M3.5 8h9" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
                </svg>
              </button>
            </td>
          </tr>
        {/each}
      </tbody>
    {/each}
  </table>
</section>

<style>
  .sensors {
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .bar {
    display: flex;
    justify-content: flex-end;
    margin-bottom: 8px;
  }
  .reset {
    padding: 5px 12px;
    font-size: 13px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    cursor: pointer;
  }
  .reset:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .reset:disabled {
    opacity: 0.5;
    cursor: default;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 5px 8px;
    text-align: left;
    font-weight: 400;
    border-bottom: 1px solid var(--border);
  }
  thead th {
    color: var(--text-muted);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .group th {
    padding-top: 14px;
    color: var(--accent-2);
    font-weight: 600;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .stale {
    color: var(--text-muted);
  }
  .last {
    margin-right: 4px;
    font-size: 11px;
  }
  .tag {
    margin-left: 6px;
    padding: 1px 6px;
    font-size: 10px;
    letter-spacing: 0.06em;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-muted);
    vertical-align: middle;
  }
  .exp {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 45%, transparent);
  }
  /* Spec §7.3: the source badge shows on hover or with focus; opacity keeps it in the
     accessibility tree (unlike display/visibility), so screen readers still read it. */
  .source {
    opacity: 0;
    cursor: help;
    transition: opacity 0.15s;
  }
  tr:hover .source,
  tr:focus-within .source {
    opacity: 1;
  }
  .act {
    width: 1%;
    padding-block: 2px;
    text-align: right;
  }
  /* Like the source badge: hidden until the row is hovered or focused, but still in the accessibility tree. */
  .rule {
    display: inline-flex;
    padding: 4px;
    color: var(--text-muted);
    cursor: pointer;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    opacity: 0;
    transition: opacity 0.15s;
  }
  tr:hover .rule,
  tr:focus-within .rule,
  .rule:focus-visible {
    opacity: 1;
  }
  .rule:hover {
    color: var(--text);
    border-color: var(--border);
  }
  .rule:focus-visible {
    outline: 2px solid var(--accent);
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  tbody tr:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
</style>
