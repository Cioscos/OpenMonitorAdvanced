<script lang="ts">
  import { formatBytes, formatClock, formatPercent, formatPower, formatRate, formatTemperature } from '../../lib/format';
  import { monitoringHealth } from '../../lib/health';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import {
    cpuSummary,
    gpuSummaries,
    memorySummary,
    networkSummary,
    simpleViewGpus,
    storageSummary,
    sumSeries,
  } from '../../lib/select';
  import AnimatedNumber from '../common/AnimatedNumber.svelte';
  import Sparkline from '../common/Sparkline.svelte';
  import HealthBanner from './HealthBanner.svelte';
  import Tile from './Tile.svelte';

  let { store, onOpenAdvanced }: { store: LiveStore; onOpenAdvanced: () => void } = $props();

  const valueOf = (id: string) => store.value(id);
  const locale = $derived(i18n.locale);
  const cpu = $derived(store.schema ? cpuSummary(store.schema, valueOf) : null);
  const gpus = $derived(store.schema ? simpleViewGpus(gpuSummaries(store.schema, valueOf)) : []);
  const mem = $derived(store.schema ? memorySummary(store.schema, valueOf) : null);
  const disk = $derived(store.schema ? storageSummary(store.schema, valueOf) : null);
  const net = $derived(store.schema ? networkSummary(store.schema, valueOf) : null);
  const netSeries = $derived(net ? sumSeries(net.downIds.map((id) => store.series(id))) : []);
  const health = $derived(monitoringHealth(store.firstTimestampMs));
</script>

<div class="simple">
  {#if store.timestampMs > 0}<HealthBanner {health} nowMs={store.timestampMs} />{/if}

  <div class="grid">
    {#if cpu}
      <Tile label={t('tile.cpu')} onclick={onOpenAdvanced}>
        <div class="big"><AnimatedNumber value={cpu.load} format={(v) => formatPercent(v, locale)} /></div>
        <div class="sub">{cpu.name} · {formatClock(cpu.clockMhz, locale)}</div>
        {#if cpu.loadId}
          <Sparkline values={store.series(cpu.loadId)} capacity={store.capacity} max={100} />
        {/if}
      </Tile>
    {/if}

    {#each gpus as gpu (gpu.deviceId)}
      <Tile label={t('tile.gpu')} onclick={onOpenAdvanced}>
        <div class="big"><AnimatedNumber value={gpu.load} format={(v) => formatPercent(v, locale)} /></div>
        <div class="sub">{gpu.name}</div>
        <div class="sub">
          {formatTemperature(gpu.temperatureC, locale)} · {formatClock(gpu.clockMhz, locale)} · {formatPower(gpu.powerW, locale)}
        </div>
        {#if gpu.memUsedBytes !== null && gpu.memTotalBytes !== null}
          <div class="sub">
            {t('tile.vram', { used: formatBytes(gpu.memUsedBytes, locale), total: formatBytes(gpu.memTotalBytes, locale) })}
          </div>
        {/if}
        {#if gpu.loadId}
          <Sparkline values={store.series(gpu.loadId)} capacity={store.capacity} max={100} />
        {/if}
      </Tile>
    {/each}

    {#if mem}
      <Tile label={t('tile.memory')} onclick={onOpenAdvanced}>
        <div class="big">
          {formatBytes(mem.usedBytes, locale)} <span class="unit">/ {formatBytes(mem.totalBytes, locale)}</span>
        </div>
        <div class="bar" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={mem.usedPct ?? 0}>
          <i style:width="{mem.usedPct ?? 0}%"></i>
        </div>
        <div class="sub">{formatPercent(mem.usedPct, locale)}</div>
        {#if mem.loadId}<Sparkline values={store.series(mem.loadId)} capacity={store.capacity} max={100} />{/if}
      </Tile>
    {/if}

    {#if net || disk}
      <Tile label={t('tile.netDisk')} onclick={onOpenAdvanced}>
        {#if net}
          <div class="big rate">
            ↓ {formatRate(net.downBps, 'bits', locale)}
            <span class="unit">↑ {formatRate(net.upBps, 'bits', locale)}</span>
          </div>
          <Sparkline values={netSeries} capacity={store.capacity} color="var(--accent-2)" />
        {/if}
        {#if disk}
          {#if !net}<Sparkline values={sumSeries(disk.readIds.map((id) => store.series(id)))} capacity={store.capacity} />{/if}
          <div class="sub">
            {#if disk.volume}{disk.volume.letter} {formatPercent(disk.volume.usedPct, locale)}{' · '}{/if}{t('tile.diskIo', {
              read: formatRate(disk.readBps, 'bytes', locale),
              write: formatRate(disk.writeBps, 'bytes', locale),
            })}
          </div>
        {/if}
      </Tile>
    {/if}
  </div>
</div>

<style>
  .simple {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 12px;
  }
  .big {
    font-size: 28px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .big.rate {
    font-size: 20px;
  }
  .unit {
    font-size: 14px;
    font-weight: 400;
    color: var(--text-muted);
  }
  .sub {
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar {
    height: 6px;
    margin: 6px 0 2px;
    overflow: hidden;
    border-radius: 4px;
    background: var(--border);
  }
  .bar i {
    display: block;
    height: 100%;
    border-radius: 4px;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
    transition: width 0.3s ease-out;
  }
</style>
