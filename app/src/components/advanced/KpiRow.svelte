<script lang="ts">
  import type { KpiDef, StatsOf } from '../../lib/advanced/pages';
  import { formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { ValueOf } from '../../lib/select';
  import AnimatedNumber from '../common/AnimatedNumber.svelte';

  let {
    kpis,
    valueOf,
    qualityOf,
    statsOf,
    rate = 'bytes',
  }: {
    kpis: KpiDef[];
    valueOf: ValueOf;
    /** Quality of a sensor by id: 2 is a value repeated while the sensor is suspended. */
    qualityOf: (id: string) => 0 | 1 | 2;
    statsOf: StatsOf;
    /** 'bits' on network pages: traffic in bit/s, like the Simple view. */
    rate?: 'bits' | 'bytes';
  } = $props();
  const locale = $derived(i18n.locale);
</script>

<div class="kpis">
  {#each kpis as kpi (kpi.id)}
    {@const secondary = kpi.secondary?.(valueOf, statsOf) ?? null}
    {@const value = kpi.value(valueOf, statsOf)}
    {@const stale = value !== null && kpi.sensorId !== undefined && qualityOf(kpi.sensorId) === 2}
    <div class="kpi">
      <div class="label">{t(kpi.labelKey)}</div>
      <div class="value" class:stale>
        <AnimatedNumber {value} format={(v) => formatValue(v, kpi.unit, locale, t, { rate })} />
      </div>
      {#if stale || secondary}
        <div class="secondary">
          {#if stale}<span class="last">{t('value.lastReading')}</span>{/if}
          {#if stale && secondary}{' · '}{/if}
          {#if secondary}{secondary}{/if}
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .kpis {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: 12px;
  }
  .kpi {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    padding: 12px 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .value {
    font-size: 24px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .stale {
    color: var(--text-muted);
  }
  .secondary {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
