<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import uPlot from 'uplot';
  import 'uplot/dist/uPlot.min.css';
  import {
    ChartBuffer,
    DEFAULT_WINDOW,
    MAX_SERIES,
    PALETTE_TOKENS,
    WINDOWS,
    canAdd,
    fitSelection,
    formatTimeTick,
    initialSeries,
    maxPointsFor,
    scaleLayout,
    scaleOptions,
    seriesPalette,
    type WindowSeconds,
  } from '../../lib/advanced/chartData';
  import { createChartViewport } from '../../lib/advanced/chartViewport';
  import { sensorLabel } from '../../lib/advanced/labels';
  import { loadSeries, loadWindow, saveSeries, saveWindow } from '../../lib/advanced/persist';
  import type { Backend } from '../../lib/backend/backend';
  import { DASH, formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { HistorySeed, Schema, Sensor } from '../../lib/types';
  import { subscribeChartFrame } from '../../lib/chartFrameClock';

  let {
    sectionId,
    sensors,
    defaults,
    schema,
    store,
    backend,
  }: {
    /** Section id; parents re-key the component when it changes. */
    sectionId: string;
    /** Sensors of the page: the series the picker offers. */
    sensors: Sensor[];
    defaults: string[];
    schema: Schema;
    store: LiveStore;
    backend: Backend;
  } = $props();

  const HEIGHT = 260;

  const candidateIds = $derived(sensors.map((s) => s.id));
  let windowSeconds = $state<WindowSeconds>(loadWindow() ?? DEFAULT_WINDOW);
  let chosen = $state<string[]>(untrack(() => initialSeries(loadSeries(sectionId), candidateIds, defaults, schema)));
  const selected = $derived(fitSelection(chosen, candidateIds, schema));
  const selectionKey = $derived(selected.join('\n'));
  let paused = $state(document.visibilityState === 'hidden');

  let container: HTMLDivElement;
  let plot: uPlot | undefined;
  let buffer: ChartBuffer | undefined;
  let viewport = createChartViewport(DEFAULT_WINDOW);
  let reducedMotion = false;
  let generation = 0;
  let destroyed = false;

  function chooseWindow(w: WindowSeconds) {
    windowSeconds = w;
    saveWindow(w);
  }

  function toggle(id: string) {
    if (selected.includes(id)) chosen = selected.filter((x) => x !== id);
    else if (canAdd(selected, id, schema)) chosen = [...selected, id];
    else return;
    saveSeries(sectionId, chosen);
  }

  async function reseed(ids: string[], seconds: WindowSeconds) {
    const token = ++generation;
    const revision = schema.revision;
    // Never append values from a new schema to a plot of the previous source/unit.
    buffer = undefined;
    viewport = createChartViewport(seconds);
    if (reducedMotion) viewport.suspend(performance.now());
    plot?.destroy();
    plot = undefined;
    let history: HistorySeed = { revision, seq: 0, timestampsMs: [], series: [] };
    if (ids.length > 0) {
      try {
        history = await backend.getHistory(ids, seconds, maxPointsFor(seconds));
      } catch (error) {
        console.error('chart history unavailable', error);
      }
    }
    if (token !== generation || destroyed || paused || schema.revision !== revision) return;
    if (ids.length > 0 && history.revision !== revision) return;
    const next = new ChartBuffer(ids, seconds);
    next.seed(history);
    // A snapshot applied while the request was in flight is newer than the seed.
    if (store.timestampMs > (next.lastTimestampMs ?? 0)) next.append(store.timestampMs, ids.map((id) => store.value(id)));
    next.trim(next.lastTimestampMs ?? 0);
    buffer = next;
    if (next.lastTimestampMs !== null) viewport.sample(next.lastTimestampMs, performance.now());
    build(ids);
  }

  function build(ids: string[]) {
    plot?.destroy();
    plot = undefined;
    if (!buffer || ids.length === 0) return;
    const css = getComputedStyle(document.documentElement);
    const read = (token: string) => css.getPropertyValue(token).trim();
    const palette = seriesPalette(read);
    const muted = read('--text-muted');
    const border = read('--border');
    const { scales, seriesScale } = scaleLayout(ids, schema);
    const byId = new Map(sensors.map((s) => [s.id, s]));
    // Known limit until the M5 unit settings: network sensors carry BytesPerSecond, so the
    // axis and legend below stay in byte/s even though the KPIs and table (formatRate) show
    // the same values converted to bit/s (see docs/follow-ups.md).
    const axis = (unit: (typeof scales)[number], side: 1 | 3): uPlot.Axis => ({
      scale: unit,
      side,
      size: 72,
      stroke: muted,
      grid: { show: side === 3, stroke: border, width: 1 },
      ticks: { stroke: border, width: 1 },
      values: (_u, splits) => splits.map((v) => formatValue(v, unit, i18n.locale, t)),
    });
    const opts: uPlot.Options = {
      width: Math.max(320, container.clientWidth || 800),
      height: HEIGHT,
      cursor: { drag: { x: false, y: false, setScale: false } },
      scales: Object.fromEntries([['x', { time: true }], ...scales.map((unit) => [unit, scaleOptions(unit)])]),
      series: [
        { label: '', value: (_u, v) => (v == null ? DASH : new Date(v * 1000).toLocaleTimeString(i18n.locale)) },
        ...ids.map((id, i) => {
          const sensor = byId.get(id);
          const unit = seriesScale[i];
          return {
            label: sensor ? sensorLabel(sensor, t) : id,
            scale: unit,
            stroke: palette[i],
            width: 1.5,
            points: { show: false },
            value: (_u: uPlot, v: number | null) => formatValue(v ?? null, unit, i18n.locale, t),
          };
        }),
      ],
      axes: [
        {
          stroke: muted,
          grid: { stroke: border, width: 1 },
          ticks: { stroke: border, width: 1 },
          values: (_u, splits) => splits.map((v) => formatTimeTick(v, i18n.locale)),
        },
        axis(scales[0], 3),
        ...(scales[1] ? [axis(scales[1], 1)] : []),
      ],
    };
    plot = new uPlot(opts, buffer.data(), container);
    drawScale(performance.now());
  }

  function drawScale(nowMonoMs: number) {
    const range = viewport.range(nowMonoMs);
    if (!paused && plot && range) plot.setScale('x', range);
  }

  function tail(timestampMs: number) {
    if (paused || !buffer || !plot || timestampMs <= 0) return;
    // LiveStore has already rejected duplicate/out-of-order sequences. A lower
    // timestamp here is a wall-clock rollback, so begin a new chart segment.
    if (buffer.lastTimestampMs !== null && timestampMs < buffer.lastTimestampMs) {
      buffer = new ChartBuffer(buffer.ids, buffer.windowSeconds);
      viewport.reset();
    }
    buffer.append(timestampMs, buffer.ids.map((id) => store.value(id)));
    buffer.trim(timestampMs);
    viewport.sample(timestampMs, performance.now());
    plot.setData(buffer.data(), false);
    drawScale(performance.now());
  }

  // Reseed on selection, window or schema change, and when the window becomes visible again.
  $effect(() => {
    const ids = selectionKey ? selectionKey.split('\n') : [];
    const seconds = windowSeconds;
    void schema.revision;
    if (paused) return;
    untrack(() => void reseed(ids, seconds));
  });

  // Live tail: one point per snapshot applied to the store.
  $effect(() => {
    const timestampMs = store.timestampMs;
    untrack(() => tail(timestampMs));
  });

  onMount(() => {
    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    reducedMotion = motionQuery.matches;
    if (reducedMotion) viewport.suspend(performance.now());
    const onMotionChange = () => {
      reducedMotion = motionQuery.matches;
      if (reducedMotion) viewport.suspend(performance.now());
      else viewport.resume(performance.now());
    };
    motionQuery.addEventListener('change', onMotionChange);
    const stopFrames = subscribeChartFrame(drawScale);
    const onVisibility = () => {
      paused = document.visibilityState === 'hidden';
      if (paused) generation++; // Invalidate history that is still in flight.
    };
    document.addEventListener('visibilitychange', onVisibility);
    const observer =
      typeof ResizeObserver === 'undefined'
        ? undefined
        : new ResizeObserver(() => {
            if (!paused) plot?.setSize({ width: Math.max(320, container.clientWidth), height: HEIGHT });
          });
    observer?.observe(container);
    return () => {
      destroyed = true;
      generation++;
      document.removeEventListener('visibilitychange', onVisibility);
      observer?.disconnect();
      motionQuery.removeEventListener('change', onMotionChange);
      stopFrames();
      plot?.destroy();
      plot = undefined;
    };
  });
</script>

<section class="chart">
  <div class="controls">
    <div class="windows" role="group" aria-label={t('advanced.chart.window.label')}>
      {#each WINDOWS as w (w)}
        <button type="button" aria-pressed={windowSeconds === w} class:on={windowSeconds === w} onclick={() => chooseWindow(w)}>
          {t(`advanced.chart.window.${w}`)}
        </button>
      {/each}
    </div>
    <details class="picker">
      <summary>{t('advanced.chart.series')} · {selected.length}/{MAX_SERIES}</summary>
      <p class="hint">{t('advanced.chart.maxSeries')}</p>
      <ul>
        {#each sensors as sensor (sensor.id)}
          {@const index = selected.indexOf(sensor.id)}
          <li>
            <label>
              <input
                type="checkbox"
                checked={index >= 0}
                disabled={index < 0 && !canAdd(selected, sensor.id, schema)}
                onchange={() => toggle(sensor.id)}
              />
              <i class="swatch" style:background={index >= 0 ? `var(${PALETTE_TOKENS[index]})` : 'transparent'}></i>
              {sensorLabel(sensor, t)}
            </label>
          </li>
        {/each}
      </ul>
    </details>
  </div>
  <div class="plot" bind:this={container}></div>
  {#if selected.length === 0}
    <p class="empty">{t('advanced.chart.empty')}</p>
  {/if}
</section>

<style>
  .chart {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 10px;
    /* Without this, the chart (a flex item in DevicePage's column) cannot shrink below
       uPlot's rendered canvas width, and the page grows past the sidebar layout. */
    min-width: 0;
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }
  .windows {
    display: flex;
    padding: 3px;
    border-radius: 10px;
    background: var(--surface-2);
  }
  .windows button {
    padding: 4px 12px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .windows button.on {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
  }
  .picker {
    min-width: 220px;
    font-size: 13px;
  }
  .picker summary {
    cursor: pointer;
    color: var(--text-muted);
    text-align: right;
  }
  .picker ul {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 2px 12px;
    margin: 6px 0 0;
    padding: 8px;
    list-style: none;
    background: var(--surface-2);
    border-radius: 8px;
  }
  .picker label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .picker label:has(input:disabled) {
    opacity: 0.45;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    border: 1px solid var(--border);
  }
  .hint {
    margin: 6px 0 0;
    color: var(--text-muted);
    font-size: 12px;
  }
  .plot {
    min-height: 260px;
  }
  .empty {
    position: absolute;
    inset: 50% 0 auto;
    margin: 0;
    text-align: center;
    color: var(--text-muted);
  }
  .plot :global(.u-legend) {
    color: var(--text);
    font-size: 12px;
  }
  /* uPlot.min.css hard-codes #607D8B / rgba(0,0,0,.07) for cursor and select chrome;
     keep them on our palette even though drag-select is disabled. */
  .plot :global(.u-cursor-x),
  .plot :global(.u-cursor-y) {
    border-color: var(--text-muted);
  }
  .plot :global(.u-select) {
    background: color-mix(in srgb, var(--text-muted) 12%, transparent);
  }
</style>
