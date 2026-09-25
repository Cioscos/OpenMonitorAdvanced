<script lang="ts">
  import type { KpiDef, StatsOf } from '../../lib/advanced/pages';
  import { formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { ValueOf } from '../../lib/select';
  import AnimatedNumber from '../common/AnimatedNumber.svelte';

  let {
    kpis,
    valueOf,
    statsOf,
    rate = 'bytes',
  }: {
    kpis: KpiDef[];
    valueOf: ValueOf;
    statsOf: StatsOf;
    /** 'bits' on network pages: traffic in bit/s, like the Simple view. */
    rate?: 'bits' | 'bytes';
  } = $props();
  const locale = $derived(i18n.locale);
</script>

<div class="kpis">
  {#each kpis as kpi (kpi.id)}
    {@const secondary = kpi.secondary?.(valueOf, statsOf) ?? null}
    <div class="kpi">
      <div class="label">{t(kpi.labelKey)}</div>
      <div class="value">
        <AnimatedNumber value={kpi.value(valueOf, statsOf)} format={(v) => formatValue(v, kpi.unit, locale, t, { rate })} />
      </div>
      {#if secondary}<div class="secondary">{secondary}</div>{/if}
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
  .secondary {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
