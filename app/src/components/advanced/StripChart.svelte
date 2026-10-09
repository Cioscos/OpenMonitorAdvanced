<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import type uPlot from 'uplot';
  import {
    ChartBuffer,
    DEFAULT_WINDOW,
    LEGEND_GAP_PX,
    LEGEND_PX,
    STRIP_HEIGHT,
    STRIP_SINGLE_HEIGHT,
    TIME_AXIS_HEIGHT,
    WINDOWS,
    displayScale,
    initialStripSeries,
    knownSelection,
    maxPointsFor,
    scaleSymbol,
    stripLayout,
    stripPalette,
    windowExtent,
    type DisplayScale,
    type Strip,
    type WindowSeconds,
  } from '../../lib/advanced/chartData';
  import { createChartViewport } from '../../lib/advanced/chartViewport';
  import { sensorLabel } from '../../lib/advanced/labels';
  import { loadSeries, loadWindow, saveSeries, saveWindow } from '../../lib/advanced/persist';
  import type { Backend } from '../../lib/backend/backend';
  import { subscribeChartFrame } from '../../lib/chartFrameClock';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { HistorySeed, Schema, Sensor, ThroughputUnit } from '../../lib/types';
  import { display } from '../../lib/units.svelte';
  import ChartStrip from './ChartStrip.svelte';
  import StripLegend, { type StripLegendRow } from './StripLegend.svelte';

  // The history chart of the Advanced view (spec §2, §4): one strip per unit, stacked, all on
  // one buffer, one viewport and one frame subscription, so they scroll together. Each strip
  // owns its uPlot; this component never touches uPlot itself.
  let {
    sectionId,
    sensors,
    defaults,
    schema,
    store,
    backend,
    rate = 'bytes',
    fixedWindow,
  }: {
    /** Section id; parents re-key the component when it changes. */
    sectionId: string;
    /** Sensors of the page: the series the picker offers. */
    sensors: Sensor[];
    defaults: string[];
    schema: Schema;
    store: LiveStore;
    backend: Backend;
    /** How `bytes_per_second` series are drawn: 'bits' on network pages when so chosen. */
    rate?: ThroughputUnit;
    /** A window in seconds that the user cannot change; not saved. */
    fixedWindow?: number;
  } = $props();

  const candidateIds = $derived(sensors.map((s) => s.id));
  let windowSeconds = $state<number>(untrack(() => fixedWindow) ?? loadWindow() ?? DEFAULT_WINDOW);
  let chosen = $state<string[]>(untrack(() => initialStripSeries(loadSeries(sectionId), candidateIds, defaults)));
  const selected = $derived(knownSelection(chosen, candidateIds));
  const selectionKey = $derived(selected.join('\n'));
  let paused = $state(document.visibilityState === 'hidden');
  let reducedMotion = $state(false);
  /** The cursor sync key of uPlot, kept across rebuilds. */
  const syncKey = untrack(() => `chart-${sectionId}`);

  /** The strips on screen, one per unit of the buffer's series; set at every reseed. */
  let strips = $state.raw<Strip[]>([]);
  /** Per strip: how values are drawn, colours with dashes and series names; set at every build. */
  let scales = $state.raw<DisplayScale[]>([]);
  let colors = $state.raw<Array<Array<{ color: string; dash: readonly number[] }>>>([]);
  let labels = $state.raw<string[][]>([]);
  /** Per strip: its columns of the buffer, converted, and the window extent of each series. */
  let stripData = $state.raw<uPlot.AlignedData[]>([]);
  let extents = $state.raw<Array<Array<{ min: number; max: number } | null>>>([]);
  /** Per strip: every series is empty over the visible window and its latest reading is suspended. */
  let suspended = $state.raw<boolean[]>([]);
  /** Series the user hid in a legend, by id: every rebuild shows them hidden again. */
  let hiddenIds = $state.raw(new Set<string>());
  /** Data index under the cursor of any strip; null without a cursor. */
  let cursorIdx = $state<number | null>(null);
  let viewport = $state.raw(createChartViewport(DEFAULT_WINDOW));

  const refs: Record<string, ChartStrip | null | undefined> = {};
  const stripRefs = () => strips.map((strip) => refs[strip.unit]).filter((ref): ref is ChartStrip => !!ref);

  let buffer: ChartBuffer | undefined;
  let generation = 0;
  let viewportRevision: number | undefined;
  let destroyed = false;

  /** Frame subscription, live only while the strips have plots. */
  let stopFrames: (() => void) | undefined;

  function endFrames() {
    stopFrames?.();
    stopFrames = undefined;
  }

  function startFrames() {
    endFrames();
    // The Y transitions run on the chart's frame subscription, before the X translations.
    stopFrames = subscribeChartFrame(onChartFrame, display.chartFps);
  }

  function onChartFrame(now: number) {
    if (paused) return;
    for (const ref of stripRefs()) ref.frame(now);
  }

  function chooseWindow(w: WindowSeconds) {
    windowSeconds = w;
    saveWindow(w);
  }

  // The selection API the series drawer (M3) uses.
  function toggle(id: string) {
    chosen = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id];
    saveSeries(sectionId, chosen);
  }

  /** Ticks (`on`) or unticks every id of `ids` at once, keeping the order of the selection. */
  function setMany(ids: string[], on: boolean) {
    const set = new Set(ids);
    chosen = on ? [...selected, ...ids.filter((id) => !selected.includes(id))] : selected.filter((id) => !set.has(id));
    saveSeries(sectionId, chosen);
  }
  /** Units of the strips, in order: the drawer lists them first. */
  const stripUnits = $derived(strips.map((s) => s.unit));

  function toggleHidden(id: string) {
    const next = new Set(hiddenIds);
    if (!next.delete(id)) next.add(id);
    hiddenIds = next;
  }

  /** Display scale, palette and names of every strip, from the theme, units and locale of now. */
  function configure() {
    const css = getComputedStyle(document.documentElement);
    const read = (token: string) => css.getPropertyValue(token).trim();
    // Units apply at draw time: the buffer keeps °C and byte/s, and the data handed to the
    // strips, their axes and the legends are converted here, so a unit change only rebuilds.
    const units = { temperature: display.temperature, rate };
    const byId = new Map(sensors.map((s) => [s.id, s]));
    scales = strips.map((strip) => displayScale(strip.unit, units));
    // The palette restarts in every strip; past eight series it repeats with a dash.
    colors = strips.map((strip) => stripPalette(strip.ids.length).map(({ token, dash }) => ({ color: read(token), dash })));
    labels = strips.map((strip) => strip.ids.map((id) => {
      const sensor = byId.get(id);
      return sensor ? sensorLabel(sensor, t) : id;
    }));
  }

  /** The strips' columns of the buffer, with the legend extents and the suspended notices. */
  function collect() {
    if (!buffer) return;
    const data = strips.map((strip, s) => buffer!.columns(strip.indices, strip.ids.map(() => scales[s]?.convert)));
    const last = buffer.lastTimestampMs;
    const since = last === null ? Infinity : (last - buffer.windowSeconds * 1000) / 1000;
    // ChartBuffer columns are plain arrays, never typed ones.
    extents = data.map(([xs, ...columns]) => columns.map((column) => windowExtent(xs as number[], column as (number | null)[], since)));
    // True when no series has a value inside the visible window (the one retained sample
    // before its left edge does not count) and the store's latest quality of each is suspended.
    suspended = strips.map((strip, s) => last !== null
      && strip.ids.every((id) => store.quality(id) === 2) && extents[s].every((extent) => extent === null));
    stripData = data;
  }

  async function reseed(ids: string[], seconds: number) {
    const token = ++generation;
    const revision = schema.revision;
    const previousEdge = viewportRevision === revision ? viewport.range(performance.now())?.max : undefined;
    viewportRevision = revision;
    // Never append values from a new schema to a plot of the previous source/unit.
    buffer = undefined;
    endFrames();
    for (const ref of stripRefs()) ref.destroy();
    cursorIdx = null;
    stripData = [];
    extents = [];
    suspended = [];
    viewport = createChartViewport(seconds);
    if (previousEdge !== undefined) viewport.sample(previousEdge * 1000, performance.now());
    if (reducedMotion) viewport.suspend(performance.now());
    // The new layout shows at once, legends included, while the history loads.
    strips = stripLayout(ids, schema);
    configure();
    let history: HistorySeed = { revision, seq: 0, timestampsMs: [], series: [] };
    if (ids.length > 0) {
      try {
        history = await backend.getHistory(ids, seconds, maxPointsFor(seconds));
      } catch (error) {
        console.error('chart history unavailable', error);
      }
      // The strips of the new layout are mounted before they are built.
      await tick();
    }
    if (token !== generation || destroyed || paused || schema.revision !== revision) return;
    if (ids.length > 0 && history.revision !== revision) return;
    const next = new ChartBuffer(ids, seconds);
    next.seed(history);
    // A snapshot applied while the request was in flight is newer than the seed.
    if (store.timestampMs > (next.lastTimestampMs ?? 0)) next.append(store.timestampMs, ids.map((id) => store.measured(id)));
    next.trim(next.lastTimestampMs ?? 0);
    buffer = next;
    if (next.lastTimestampMs !== null) viewport.sample(next.lastTimestampMs, performance.now());
    build();
  }

  /** (Re)builds every strip from the buffer: after a reseed, and on unit, locale, theme or DPR changes. */
  function build() {
    endFrames();
    cursorIdx = null;
    if (!buffer || strips.length === 0) {
      for (const ref of stripRefs()) ref.destroy();
      return;
    }
    // A series that left the selection comes back visible.
    hiddenIds = new Set(buffer.ids.filter((id) => hiddenIds.has(id)));
    configure();
    collect();
    for (const [s, strip] of strips.entries()) refs[strip.unit]?.build(stripData[s]);
    startFrames();
  }

  function tail(timestampMs: number) {
    if (paused || !buffer || strips.length === 0 || timestampMs <= 0) return;
    // LiveStore has already rejected duplicate/out-of-order sequences. A lower
    // timestamp here is a wall-clock rollback, so begin a new chart segment.
    if (buffer.lastTimestampMs !== null && timestampMs < buffer.lastTimestampMs) {
      buffer = new ChartBuffer(buffer.ids, buffer.windowSeconds);
      viewport.reset();
    }
    buffer.append(timestampMs, buffer.ids.map((id) => store.measured(id)));
    buffer.trim(timestampMs);
    viewport.sample(timestampMs, performance.now());
    collect();
    const now = performance.now();
    for (const [s, strip] of strips.entries()) refs[strip.unit]?.setData(stripData[s], now);
  }

  const fallbackScale = displayScale('count', { temperature: 'c', rate: 'bytes' });

  /** Legend rows of strip `s`: the cursor's values when there is one, else the latest readings. */
  function rowsOf(s: number, strip: Strip): StripLegendRow[] {
    const scale = scales[s] ?? fallbackScale;
    const data = stripData[s];
    const show = (value: number | null) => scale.format(value, i18n.locale, t);
    return strip.ids.map((id, k) => {
      let now: number | null;
      if (cursorIdx === null) {
        // The latest snapshot, not the value the held segment projects.
        const measured = store.measured(id);
        now = measured === null || !scale.convert ? measured : scale.convert(measured);
      } else now = data?.[k + 1]?.[cursorIdx] ?? null;
      const extent = extents[s]?.[k] ?? null;
      const color = colors[s]?.[k];
      return {
        id,
        label: labels[s]?.[k] ?? id,
        color: color?.color ?? 'transparent',
        dash: color?.dash ?? [],
        now: show(now),
        min: show(extent?.min ?? null),
        max: show(extent?.max ?? null),
        hidden: hiddenIds.has(id),
      };
    });
  }

  /** «Temperature · °F»: the unit's name and the symbol its values are drawn in, when it has one. */
  function titleOf(s: number, strip: Strip): string {
    const name = t(`advanced.chart.unit.${strip.unit}`);
    const symbol = scaleSymbol(scales[s] ?? displayScale(strip.unit, { temperature: display.temperature, rate }), i18n.locale, t);
    return symbol ? `${name} · ${symbol}` : name;
  }

  /** Swatch colour of a picked series: its strip's palette entry. */
  function swatchOf(id: string): string {
    const strip = strips.find((x) => x.ids.includes(id));
    const k = strip?.ids.indexOf(id) ?? -1;
    return strip && k >= 0 ? `var(${stripPalette(k + 1)[k].token})` : 'transparent';
  }

  const heightOf = (s: number) => (strips.length === 1 ? STRIP_SINGLE_HEIGHT : STRIP_HEIGHT + (s === strips.length - 1 ? TIME_AXIS_HEIGHT : 0));

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

  // Locale affects the cached time labels, the Y labels and the legends; the units change the
  // plotted data, axes and legends. All rebuild the strips from the buffer, without new history.
  $effect(() => {
    void i18n.locale;
    void display.temperature;
    void rate;
    untrack(() => { if (!paused && buffer) build(); });
  });

  // A new frame rate replaces the running subscription; without plots there is none.
  $effect(() => {
    const fps = display.chartFps;
    untrack(() => {
      if (!stopFrames) return;
      stopFrames();
      stopFrames = subscribeChartFrame(onChartFrame, fps);
    });
  });

  onMount(() => {
    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    reducedMotion = motionQuery.matches;
    if (reducedMotion) viewport.suspend(performance.now());
    const onMotionChange = () => {
      reducedMotion = motionQuery.matches;
      if (reducedMotion) {
        viewport.suspend(performance.now());
        // The shared clock stops too: show the target ranges now.
        for (const ref of stripRefs()) ref.finishY();
      } else viewport.resume(performance.now());
    };
    motionQuery.addEventListener('change', onMotionChange);
    const onVisibility = () => {
      paused = document.visibilityState === 'hidden';
      if (paused) {
        generation++; // Invalidate history that is still in flight.
        viewport.reset();
        // The visible rebuild shows the current ranges at once.
        for (const ref of stripRefs()) ref.reset();
      }
    };
    document.addEventListener('visibilitychange', onVisibility);
    const refresh = () => { if (!paused && buffer) build(); };
    const themeObserver = new MutationObserver(refresh);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'style', 'data-theme'] });
    let densityQuery: MediaQueryList;
    const onDensity = () => {
      densityQuery?.removeEventListener('change', onDensity);
      densityQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      densityQuery.addEventListener('change', onDensity);
      refresh();
    };
    densityQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    densityQuery.addEventListener('change', onDensity);
    return () => {
      destroyed = true;
      generation++;
      document.removeEventListener('visibilitychange', onVisibility);
      themeObserver.disconnect();
      densityQuery.removeEventListener('change', onDensity);
      motionQuery.removeEventListener('change', onMotionChange);
      endFrames();
    };
  });
</script>

<section class="chart" style:--legend-px="{LEGEND_PX}px" style:--legend-gap="{LEGEND_GAP_PX}px">
  <div class="controls">
    {#if fixedWindow === undefined}
      <div class="windows" role="group" aria-label={t('advanced.chart.window.label')}>
        {#each WINDOWS as w (w)}
          <button type="button" aria-pressed={windowSeconds === w} class:on={windowSeconds === w} onclick={() => chooseWindow(w)}>
            {t(`advanced.chart.window.${w}`)}
          </button>
        {/each}
      </div>
    {/if}
    <details class="picker">
      <summary>{t('advanced.chart.series')} · {selected.length}</summary>
      <ul>
        {#each sensors as sensor (sensor.id)}
          {@const on = selected.includes(sensor.id)}
          <li>
            <label>
              <input type="checkbox" checked={on} onchange={() => toggle(sensor.id)} />
              <i class="swatch" style:background={on ? swatchOf(sensor.id) : 'transparent'}></i>
              {sensorLabel(sensor, t)}
            </label>
          </li>
        {/each}
      </ul>
    </details>
  </div>
  {#if selected.length === 0}
    <div class="placeholder" style:min-height="{STRIP_SINGLE_HEIGHT}px">
      <p class="empty">{t('advanced.chart.empty')}</p>
    </div>
  {:else}
    <div class="strips">
      {#each strips as strip, s (strip.unit)}
        <div class="strip">
          <div class="lines">
            <ChartStrip
              bind:this={refs[strip.unit]}
              unit={strip.unit}
              colors={colors[s] ?? []}
              labels={labels[s] ?? []}
              hidden={strip.ids.map((id) => hiddenIds.has(id))}
              format={(scales[s] ?? fallbackScale).format}
              {viewport}
              {windowSeconds}
              height={heightOf(s)}
              timeAxis={s === strips.length - 1}
              {syncKey}
              {reducedMotion}
              oncursor={(idx) => (cursorIdx = idx)}
            />
            {#if suspended[s]}<p class="empty">{t('chart.suspended')}</p>{/if}
          </div>
          <StripLegend title={titleOf(s, strip)} rows={rowsOf(s, strip)} height={heightOf(s)} ontoggle={toggleHidden} onremove={toggle} />
        </div>
      {/each}
    </div>
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
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    border: 1px solid var(--border);
  }
  .strips {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  /* Lines on the left, the strip's legend on the right. */
  .strip {
    display: grid;
    grid-template-columns: minmax(0, 1fr) var(--legend-px);
    gap: var(--legend-gap);
    align-items: start;
  }
  .lines,
  .placeholder {
    position: relative;
    min-width: 0;
  }
  .empty {
    position: absolute;
    inset: 50% 0 auto;
    margin: 0;
    text-align: center;
    color: var(--text-muted);
  }
</style>
