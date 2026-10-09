<script lang="ts">
  import { formatBytes, formatClock, formatPercent, formatPower, formatRate, formatTemperature, formatValue } from '../../lib/format';
  import { simpleGrid } from '../../lib/simpleGrid';
  import { sensorLabel } from '../../lib/advanced/labels';
  import { sectionForTile } from '../../lib/advanced/nav';
  import { bannerText, health } from '../../lib/health.svelte';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { display } from '../../lib/units.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { DeviceKind, HealthReport } from '../../lib/types';
  import {
    cpuSummary,
    diskRows,
    gpuSummaries,
    memorySummary,
    networkSummary,
    simpleViewGpus,
    storageSummary,
    sumSeries,
  } from '../../lib/select';
  import AnimatedNumber from '../common/AnimatedNumber.svelte';
  import Term from '../common/Term.svelte';
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
  const disks = $derived(store.schema ? diskRows(store.schema, valueOf, (id) => store.diskPower(id)) : []);
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

  const rpm = (v: number) => formatValue(v, 'rpm', locale, t);
  const sensorById = $derived(new Map(store.schema?.sensors.map((s) => [s.id, s])));
  /** A sensor's display name; the id when the schema no longer has it. */
  const nameOf = (id: string) => {
    const sensor = sensorById.get(id);
    return sensor ? sensorLabel(sensor, t) : id;
  };
  const pctOf = (used: number | null, total: number | null) =>
    used !== null && total ? Math.min(100, (used / total) * 100) : 0;
</script>

<div class="host">
<div class="simple">
  {#if store.timestampMs > 0}<HealthBanner level={report.level} title={banner.title} items={bannerItems} {elapsedMs} />{/if}

  <div class="grid" style:--cols={layout.columns}>
    {#if cpu}
      <Tile label={t('tile.cpu')} span={layout.spans[0]} onclick={() => onOpenAdvanced(sectionForTile('cpu'))}>
        <div class="big"><AnimatedNumber value={cpu.load} format={(v) => formatPercent(v, locale)} /></div>
        <div class="sub">{cpu.name}</div>
        <div class="chips">
          {#if cpu.temperatureC !== null}<div class="chip"><span>{t('tile.temperature')}</span> {formatTemperature(cpu.temperatureC, locale)}</div>{/if}
          {#if cpu.powerW !== null}<div class="chip"><span>{t('tile.power')}</span> {formatPower(cpu.powerW, locale)}</div>{/if}
          {#if cpu.clockMhz !== null}<div class="chip"><span><Term term="clock">{t('tile.clock')}</Term></span> {formatClock(cpu.clockMhz, locale)}</div>{/if}
          {#if cpu.busiestThread !== null}
            <div class="chip"><span><Term term="busiestThread">{t('tile.busiestThread')}</Term></span> {formatPercent(cpu.busiestThread, locale)}</div>
          {/if}
        </div>
        {#if cpu.threads.length > 0}
          <div class="mlab"><Term term="threadLoad">{t('tile.threadLoad')}</Term></div>
          <div class="threads">
            {#each cpu.threads as thread (thread.id)}
              <i
                role="meter"
                aria-label={nameOf(thread.id)}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={thread.value ?? 0}
                title={formatPercent(thread.value, locale)}
                style:height="{thread.value ?? 0}%"
              ></i>
            {/each}
          </div>
        {/if}
        {#if cpu.loadId}
          <div class="spark"><Sparkline values={store.series(cpu.loadId)} timestampsMs={seriesTimestampsMs} max={100} /></div>
        {/if}
      </Tile>
    {/if}

    {#each gpus as gpu, i (gpu.deviceId)}
      <Tile label={t('tile.gpu')} span={layout.spans[gpuBase + i]} onclick={() => onOpenAdvanced(sectionForTile('gpu', gpu.deviceId))}>
        <div class="big"><AnimatedNumber value={gpu.load} format={(v) => formatPercent(v, locale)} /></div>
        <div class="sub">{gpu.name}</div>
        <div class="chips">
          {#if gpu.temperatureC !== null}<div class="chip"><span>{t('tile.temperature')}</span> {formatTemperature(gpu.temperatureC, locale)}</div>{/if}
          {#if gpu.hotspotC !== null}
            <div class="chip"><span><Term term="hotspot">{t('tile.hotspot')}</Term></span> {formatTemperature(gpu.hotspotC, locale)}</div>
          {/if}
          {#if gpu.clockMhz !== null}<div class="chip"><span><Term term="gpuClock">{t('tile.clock')}</Term></span> {formatClock(gpu.clockMhz, locale)}</div>{/if}
          {#if gpu.powerW !== null}
            <div class="chip">
              <span>{t('tile.power')}</span> {formatPower(gpu.powerW, locale)}
              {#if gpu.powerLimitW !== null}<Term term="powerLimit">/ {formatPower(gpu.powerLimitW, locale)}</Term>{/if}
            </div>
          {/if}
          {#if gpu.fanPct !== null || gpu.fanRpm !== null}
            <div class="chip">
              <span>{t('tile.fan')}</span>
              {[gpu.fanPct !== null ? formatPercent(gpu.fanPct, locale) : null, gpu.fanRpm !== null ? rpm(gpu.fanRpm) : null].filter(Boolean).join(' · ')}
            </div>
          {/if}
        </div>
        {#if gpu.memUsedBytes !== null && gpu.memTotalBytes !== null}
          <div class="mlab">
            <Term term="vram" /><span>{formatBytes(gpu.memUsedBytes, locale)} / {formatBytes(gpu.memTotalBytes, locale)}</span>
          </div>
          <div class="bar" role="meter" aria-label={t('glossary.vram.name')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pctOf(gpu.memUsedBytes, gpu.memTotalBytes)}>
            <i style:width="{pctOf(gpu.memUsedBytes, gpu.memTotalBytes)}%"></i>
          </div>
        {/if}
        {#if gpu.throttle}
          <div class="chips">
            <div class="chip warn"><Term term="throttling">{t(`tile.throttle.${gpu.throttle}`)}</Term></div>
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
        <div class="mlab">
          <span>{formatPercent(mem.usedPct, locale)}</span>
          {#if mem.availableBytes !== null}<span>{t('tile.available')} {formatBytes(mem.availableBytes, locale)}</span>{/if}
        </div>
        {#if mem.dimmTempsC.length > 0}
          <div class="chips">
            <div class="chip">
              <span><Term term="dimm">{t('tile.dimm')}</Term></span> {mem.dimmTempsC.map((c) => formatTemperature(c, locale)).join(' · ')}
            </div>
          </div>
        {/if}
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
          {#if net.adapterName}
            <div class="sub">
              {net.adapterName}{#if net.linkBps !== null}{' · '}<Term term="linkSpeed">{t('tile.link')}</Term> {formatRate(net.linkBps / 8, display.throughput, locale)}{/if}
            </div>
          {/if}
          <div class="spark"><Sparkline values={netSeries} timestampsMs={seriesTimestampsMs} color="var(--accent-2)" /></div>
        {/if}
        {#if disk}
          {#if !net}<div class="spark"><Sparkline values={sumSeries(disk.readIds.map((id) => store.series(id)))} timestampsMs={seriesTimestampsMs} /></div>{/if}
          <div class="disks">
            <span class="h"></span>
            <span class="h"><Term term="diskActivity">{t('tile.disk.activity')}</Term></span>
            <span class="h">{t('tile.disk.read')}</span>
            <span class="h">{t('tile.disk.write')}</span>
            <span class="h">{t('tile.disk.temperature')}</span>
            {#each disks as row (row.deviceId)}
              <div class="disk-row">
                <span title={row.name}>{[row.letter ?? row.name, row.usedPct !== null ? formatPercent(row.usedPct, locale) : null].filter(Boolean).join(' · ')}</span>
                {#if row.standby}
                  <span class="standby">{t('storage.power.standby')}</span>
                {:else}
                  <span
                    class="act"
                    role="meter"
                    aria-label="{t('tile.disk.activity')} {row.letter ?? row.name}"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={row.activePct ?? 0}
                    title={row.activePct !== null ? formatPercent(row.activePct, locale) : ''}
                  ><i style:width="{row.activePct ?? 0}%"></i></span>
                  <span>{row.readBps !== null ? formatRate(row.readBps, 'bytes', locale) : ''}</span>
                  <span>{row.writeBps !== null ? formatRate(row.writeBps, 'bytes', locale) : ''}</span>
                  <span>{row.temperatureC !== null ? formatTemperature(row.temperatureC, locale) : ''}</span>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </Tile>
    {/if}
  </div>
</div>
</div>

<style>
  /* Centred in main's free height; the padding adds to main's 20 px for a margin of at least 32 px. */
  .host {
    container: simple / inline-size;
    margin-block: auto;
    padding-block: 12px;
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
  /* Clicks on the sparkline fall through to the tile's overlay. */
  .spark {
    height: 34px;
    pointer-events: none;
  }
  /* Bars and names with a tooltip sit above the overlay, like the terms. */
  .threads,
  .disks [title] {
    position: relative;
    z-index: 1;
  }
  /* Wide content: the columns and spans of simpleGrid; tiles keep their natural height. */
  @container simple (min-width: 900px) {
    .grid {
      grid-template-columns: repeat(var(--cols), 1fr);
    }
    .grid > :global(.tile) {
      grid-column: span var(--span, 1);
    }
    .spark {
      height: 80px;
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
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0;
    margin: 2px 0 0;
  }
  .chip {
    padding: 2px 8px;
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    background: var(--surface-2);
    border-radius: 6px;
  }
  .chip span {
    margin-right: 4px;
    color: var(--text-muted);
  }
  .chip.warn {
    color: var(--warn);
    border: 1px solid color-mix(in srgb, var(--warn) 40%, transparent);
  }
  .mlab {
    display: flex;
    justify-content: space-between;
    margin-top: 4px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .threads {
    display: grid;
    grid-auto-columns: 1fr;
    grid-auto-flow: column;
    gap: 3px;
    align-items: end;
    height: 26px;
  }
  .threads i {
    min-height: 2px;
    background: linear-gradient(to top, var(--accent), color-mix(in srgb, var(--accent) 35%, transparent));
    border-radius: 2px;
  }
  .disks {
    display: grid;
    grid-template-columns: auto 1fr auto auto auto;
    gap: 3px 10px;
    align-items: center;
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  .disk-row {
    display: contents;
  }
  .disks .h {
    font-size: 10px;
    color: var(--text-muted);
  }
  .disks .act {
    height: 5px;
    overflow: hidden;
    background: var(--border);
    border-radius: 3px;
  }
  .disks .act i {
    display: block;
    height: 100%;
    background: var(--accent-2);
  }
  .disks .standby {
    grid-column: 2 / -1;
    color: var(--text-muted);
  }
</style>
