<script lang="ts">
  import { onMount } from 'svelte';
  import { formatBytes, formatClock, formatPercent, formatPower, formatRate, formatTemperature } from '../../lib/format';
  import { simpleGrid } from '../../lib/simpleGrid';
  import { sectionForTile } from '../../lib/advanced/nav';
  import { bannerText, health } from '../../lib/health.svelte';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { display } from '../../lib/units.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { DeviceKind, HealthReport } from '../../lib/types';
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

  let {
    store,
    onOpenAdvanced,
  }: {
    store: LiveStore;
    /** Opens the Advanced view on a section (null keeps its last page). */
    onOpenAdvanced: (section: string | null) => void;
  } = $props();

  const NEUTRAL: HealthReport = { level: 'neutral', sinceMs: 0, revision: 0, coverage: 'complete', unavailableTargets: [], alerts: [] };
  const valueOf = (id: string) => store.value(id);
  const locale = $derived(i18n.locale);
  const cpu = $derived(store.schema ? cpuSummary(store.schema, valueOf) : null);
  const gpus = $derived(store.schema ? simpleViewGpus(gpuSummaries(store.schema, valueOf)) : []);
  const mem = $derived(store.schema ? memorySummary(store.schema, valueOf) : null);
  const disk = $derived(store.schema ? storageSummary(store.schema, valueOf) : null);
  const net = $derived(store.schema ? networkSummary(store.schema, valueOf) : null);
  const netSeries = $derived(net ? sumSeries(net.downIds.map((id) => store.series(id))) : []);
  const seriesTimestampsMs = $derived(store.seriesTimestampsMs());
  // Until the first report arrives the banner only says that monitoring runs.
  const report = $derived(health.report ?? NEUTRAL);
  const banner = $derived(bannerText(report, store.schema, t, locale, display.temperature, display.throughput));
  const bannerItems = $derived(banner.items.map((text, i) => ({ text, level: report.alerts[i].level })));
  // The core's monotonic time in the level, never a difference of system times.
  const elapsedMs = $derived(health.elapsedMs ?? 0);
  const firstDevice = (kind: DeviceKind) => store.schema?.devices.find((d) => d.kind === kind)?.id;
  const netDiskSection = $derived(
    net ? sectionForTile('network', firstDevice('network')) : sectionForTile('storage', firstDevice('storage')),
  );
  // Tiles in render order: CPU, one per GPU, memory, network and disks.
  const tileCount = $derived((cpu ? 1 : 0) + gpus.length + (mem ? 1 : 0) + (net || disk ? 1 : 0));
  const layout = $derived(simpleGrid(tileCount));
  const gpuBase = $derived(cpu ? 1 : 0);
  const memAt = $derived(gpuBase + gpus.length);
  const netDiskAt = $derived(memAt + (mem ? 1 : 0));

  let simpleEl: HTMLDivElement;
  let innerHeight = $state(window.innerHeight);
  let simpleTop = $state(0);
  // Fill mode (tall window, at most two rows) also needs a content width of 900 px: the container query in CSS.
  const fill = $derived(innerHeight >= 720 && layout.rows <= 2);

  onMount(() => {
    const measure = () => {
      innerHeight = window.innerHeight;
      simpleTop = simpleEl.getBoundingClientRect().top;
    };
    measure();
    window.addEventListener('resize', measure);
    return () => window.removeEventListener('resize', measure);
  });
</script>

<div class="host">
<div class="simple" class:fill bind:this={simpleEl} style:--simple-top="{simpleTop}px">
  {#if store.timestampMs > 0}<HealthBanner level={report.level} title={banner.title} items={bannerItems} {elapsedMs} />{/if}

  <div class="grid" style:--cols={layout.columns} style:--rows={layout.rows}>
    {#if cpu}
      <Tile label={t('tile.cpu')} span={layout.spans[0]} onclick={() => onOpenAdvanced(sectionForTile('cpu'))}>
        <div class="big"><AnimatedNumber value={cpu.load} format={(v) => formatPercent(v, locale)} /></div>
        <div class="sub">{cpu.name} · {formatClock(cpu.clockMhz, locale)}</div>
        {#if cpu.loadId}
          <div class="spark"><Sparkline values={store.series(cpu.loadId)} timestampsMs={seriesTimestampsMs} max={100} /></div>
        {/if}
      </Tile>
    {/if}

    {#each gpus as gpu, i (gpu.deviceId)}
      <Tile label={t('tile.gpu')} span={layout.spans[gpuBase + i]} onclick={() => onOpenAdvanced(sectionForTile('gpu', gpu.deviceId))}>
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
          <div class="spark"><Sparkline values={store.series(gpu.loadId)} timestampsMs={seriesTimestampsMs} max={100} /></div>
        {/if}
      </Tile>
    {/each}

    {#if mem}
      <Tile label={t('tile.memory')} span={layout.spans[memAt]} onclick={() => onOpenAdvanced(sectionForTile('memory', firstDevice('memory')))}>
        <div class="big">
          {formatBytes(mem.usedBytes, locale)} <span class="unit">/ {formatBytes(mem.totalBytes, locale)}</span>
        </div>
        <div class="bar" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={mem.usedPct ?? 0}>
          <i style:width="{mem.usedPct ?? 0}%"></i>
        </div>
        <div class="sub">{formatPercent(mem.usedPct, locale)}</div>
        {#if mem.loadId}<div class="spark"><Sparkline values={store.series(mem.loadId)} timestampsMs={seriesTimestampsMs} max={100} /></div>{/if}
      </Tile>
    {/if}

    {#if net || disk}
      <Tile label={t('tile.netDisk')} span={layout.spans[netDiskAt]} onclick={() => onOpenAdvanced(netDiskSection)}>
        {#if net}
          <div class="big rate">
            ↓ {formatRate(net.downBps, display.throughput, locale)}
            <span class="unit">↑ {formatRate(net.upBps, display.throughput, locale)}</span>
          </div>
          <div class="spark"><Sparkline values={netSeries} timestampsMs={seriesTimestampsMs} color="var(--accent-2)" /></div>
        {/if}
        {#if disk}
          {#if !net}<div class="spark"><Sparkline values={sumSeries(disk.readIds.map((id) => store.series(id)))} timestampsMs={seriesTimestampsMs} /></div>{/if}
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
</div>

<style>
  .host {
    container: simple / inline-size;
  }
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
  .spark {
    flex: 1;
    min-height: 34px;
  }
  /* Wide content: the columns and spans of simpleGrid; a tall window fills the height with two rows at most. */
  @container simple (min-width: 900px) {
    .grid {
      grid-template-columns: repeat(var(--cols), 1fr);
    }
    .grid > :global(.tile) {
      grid-column: span var(--span, 1);
    }
    .simple.fill {
      height: calc(100dvh - var(--simple-top) - 20px);
    }
    .simple.fill .grid {
      flex: 1;
      min-height: 0;
      grid-template-rows: repeat(var(--rows), 1fr);
    }
  }
  .big {
    font-size: clamp(28px, 2.2vw, 40px);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .big.rate {
    font-size: clamp(20px, 1.6vw, 30px);
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
