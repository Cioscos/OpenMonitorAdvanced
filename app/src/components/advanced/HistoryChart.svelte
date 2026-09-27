<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import uPlot from 'uplot';
  import 'uplot/dist/uPlot.min.css';
  import {
    ChartBuffer,
    DEFAULT_WINDOW,
    MAX_SERIES,
    TIME_AXIS_MIN_SPACE,
    PALETTE_TOKENS,
    WINDOWS,
    canAdd,
    fitSelection,
    formatTimeTick,
    initialSeries,
    labelSafeIncrs,
    maxPointsFor,
    scaleLayout,
    scaleOptions,
    seriesPalette,
    timeAxisSpace,
    type WindowSeconds,
  } from '../../lib/advanced/chartData';
  import { drawChartCanvas, timeTickFont, type ChartCanvasPath, type ChartHeldSegment } from '../../lib/advanced/chartCanvas';
  import { heldLengthPx, scrollOffsetPx, timeTicks } from '../../lib/chartCompositor';
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
  /** Radius of the white endpoint dot in CSS px; its layer is padded by it on every side. */
  const DOT_RADIUS = 3;

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
  let viewportRevision: number | undefined;
  let destroyed = false;
  let canvas: HTMLCanvasElement | undefined;
  let canvasClip: HTMLDivElement | undefined;
  let dotClip: HTMLDivElement | undefined;
  let dots: HTMLDivElement[] = [];
  let baseRightMs = 0;
  let canvasOffsetPx = 0;
  /** The canvas holds held segments that must vanish when their sample leaves the window. */
  let heldPainted = false;
  let tickIncrement = 60;
  let autoscaleY = true;
  let labelContext: CanvasRenderingContext2D | null | undefined;

  type YRange = { min: number; max: number };
  /** Duration in ms of the move to a new automatic Y range. */
  const Y_TRANSITION_MS = 180;
  /** The Y range uPlot displays, by scale key: the last autoscale result or transition frame. */
  const yShown = new Map<string, YRange>();
  /** Running Y transitions, by scale key; empty between them, so frames stay transform-only. */
  const yMoves = new Map<string, { from: YRange; to: YRange; startMs: number }>();
  /** Set while the transition itself sets Y scales, which are then no autoscale target. */
  let settingY = false;

  /**
   * uPlot `scale.range` of an automatic Y scale: the unit's own range (as uPlot would compute
   * it from `scaleOptions`) is the target. A changed target is not shown at once: uPlot keeps
   * the displayed range and the frames below move towards the target, restarting from wherever
   * the scale is when the next snapshot changes it again.
   */
  function yRange(unit: Parameters<typeof scaleOptions>[0]): uPlot.Range.Function {
    const config = scaleOptions(unit).range as uPlot.Range.Config | undefined;
    return (_u, dataMin, dataMax, key) => {
      if (settingY || dataMin == null || dataMax == null) return [dataMin, dataMax];
      const [min, max] = config ? uPlot.rangeNum(dataMin, dataMax, config) : uPlot.rangeNum(dataMin, dataMax, 0.1, true);
      if (min == null || max == null) return [min, max];
      const shown = yShown.get(key);
      const move = yMoves.get(key);
      if (!shown || reducedMotion || paused || (shown.min === min && shown.max === max)) {
        yMoves.delete(key);
        yShown.set(key, { min, max });
        return [min, max];
      }
      // A snapshot that keeps the running target keeps its pace.
      if (!move || move.to.min !== min || move.to.max !== max) {
        yMoves.set(key, { from: shown, to: { min, max }, startMs: performance.now() });
      }
      return [shown.min, shown.max];
    };
  }

  /** Commit Y ranges to uPlot in one redraw: its Y labels, the paths and the canvas follow. */
  function applyY(ranges: Array<[string, YRange]>) {
    if (!plot || ranges.length === 0) return;
    settingY = true;
    try {
      plot.batch(() => {
        for (const [key, range] of ranges) {
          yShown.set(key, range);
          plot!.setScale(key, { ...range });
        }
      });
    } finally {
      settingY = false;
    }
  }

  /** One transition frame: ease-out cubic from the range shown at the snapshot to its target. */
  function stepY(nowMonoMs: number) {
    if (yMoves.size === 0 || paused) return;
    const ranges: Array<[string, YRange]> = [];
    for (const [key, { from, to, startMs }] of yMoves) {
      const progress = Math.min(1, Math.max(0, (nowMonoMs - startMs) / Y_TRANSITION_MS));
      if (progress >= 1) yMoves.delete(key);
      const k = 1 - (1 - progress) ** 3;
      ranges.push([key, progress >= 1 ? to : { min: from.min + (to.min - from.min) * k, max: from.max + (to.max - from.max) * k }]);
    }
    applyY(ranges);
  }

  /** Jump every running transition to its target (reduced motion). */
  function finishY() {
    const ranges = [...yMoves].map(([key, { to }]): [string, YRange] => [key, to]);
    yMoves.clear();
    applyY(ranges);
  }

  /** Width in CSS px of an X label, measured with the font the canvas paints it in. */
  function labelWidthPx(text: string): number {
    labelContext ??= document.createElement('canvas').getContext('2d');
    // Without a 2D context, assume a generous average glyph width.
    if (!labelContext) return text.length * 8;
    labelContext.font = timeTickFont(1);
    return labelContext.measureText(text).width;
  }

  /**
   * uPlot `axis.space` for the time axis: labels differ in width with the increment (seconds
   * below a minute) and the locale, so require enough spacing for the labels of the increment
   * uPlot will pick. Each resize, DPR or locale rebuild asks again.
   */
  function timeSpace(u: uPlot, axisIdx: number, min: number, max: number, plotWidth: number): number {
    const incrs = u.axes[axisIdx].incrs;
    const table = typeof incrs === 'function' ? incrs(u, axisIdx, min, max, plotWidth, TIME_AXIS_MIN_SPACE) : incrs ?? [];
    const widths = new Map<string, number>();
    const measure = (text: string) => widths.get(text) ?? widths.set(text, labelWidthPx(text)).get(text)!;
    // Both ends and the other half of the day cover the widest digits and AM/PM forms.
    return timeAxisSpace(min, max, plotWidth, table, (incr) =>
      Math.max(...[min, max, min + 43_200].map((time) => measure(formatTimeTick(time, i18n.locale, incr)))));
  }

  function clearLayers() {
    canvasClip?.remove();
    dotClip?.remove();
    canvas = undefined;
    canvasClip = undefined;
    dotClip = undefined;
    dots = [];
    canvasOffsetPx = 0;
    heldPainted = false;
  }

  function createLayers(colors: string[]) {
    if (!plot) return;
    clearLayers();
    canvasClip = document.createElement('div');
    canvasClip.className = 'chart-canvas-clip';
    canvas = document.createElement('canvas');
    canvas.className = 'chart-canvas';
    canvasClip.append(canvas);
    // Held segments are canvas strokes clipped to the plot; the dots have their own padded
    // layer so the right edge and the Y extrema never cut them in half.
    dotClip = document.createElement('div');
    dotClip.className = 'chart-dot-clip';
    for (let i = 0; i < colors.length; i++) {
      const dot = document.createElement('div');
      dot.className = 'chart-dot';
      dot.hidden = true;
      dots.push(dot);
      dotClip.append(dot);
    }
    // The wrap shares uPlot's canvas origin; its sibling legend has its own layout.
    plot.over.parentElement!.append(canvasClip, dotClip);
  }

  /**
   * Held segments of the series whose last real value is finite, inside its Y scale and whose
   * sample is still in the window, in canvas px; the matching dots are shown and placed.
   */
  function heldSegments(u: uPlot, paths: ChartCanvasPath[]): ChartHeldSegment[] {
    const ratio = uPlot.pxRatio;
    const { top, height } = u.bbox;
    const windowMs = buffer!.windowSeconds * 1000;
    const visibleRightMs = (viewport.range(performance.now())?.max ?? baseRightMs / 1000) * 1000;
    const inWindow = heldLengthPx(buffer!.lastTimestampMs, visibleRightMs, windowMs, u.bbox.width / ratio) !== null;
    const lastX = u.data[0]?.at(-1);
    const segments: ChartHeldSegment[] = [];
    for (let i = 0; i < dots.length; i++) {
      const value = u.data[i + 1]?.at(-1);
      const y = value == null ? NaN : u.valToPos(value, u.series[i + 1].scale!, true);
      const show = inWindow && lastX != null && u.series[i + 1].show !== false
        && Number.isFinite(value) && Number.isFinite(y) && y >= top && y <= top + height;
      dots[i].hidden = !show;
      if (!show) continue;
      dots[i].style.top = `${(y - top) / ratio + DOT_RADIUS}px`;
      segments.push({ x: u.valToPos(lastX, 'x', true), y, color: paths[i].color });
    }
    return segments;
  }

  function paint(u: uPlot, paths: ChartCanvasPath[], theme: { gridColor: string; textColor: string }) {
    if (!canvas || !canvasClip || !dotClip) return;
    const ratio = uPlot.pxRatio;
    const { left, top, width, height } = u.bbox;
    const axisHeight = 40 * ratio;
    const leftOverscan = 72 * ratio;
    // One window of overscan keeps memory bounded, even if telemetry stops.
    canvasClip.style.left = `${left / ratio}px`;
    canvasClip.style.top = `${top / ratio}px`;
    canvasClip.style.width = `${width / ratio}px`;
    canvasClip.style.height = `${(height + axisHeight) / ratio}px`;
    dotClip.style.left = `${left / ratio - DOT_RADIUS}px`;
    dotClip.style.top = `${top / ratio - DOT_RADIUS}px`;
    dotClip.style.width = `${width / ratio + DOT_RADIUS * 2}px`;
    dotClip.style.height = `${height / ratio + DOT_RADIUS * 2}px`;
    // Only labels need space behind the left edge; future ticks need a full window.
    canvas.width = Math.ceil(width * 2 + leftOverscan);
    canvas.height = Math.ceil(height + axisHeight);
    canvas.style.width = `${(width * 2 + leftOverscan) / ratio}px`;
    canvas.style.height = `${(height + axisHeight) / ratio}px`;
    canvas.style.left = `${-leftOverscan / ratio}px`;
    heldPainted = false;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.save();
    ctx.translate(leftOverscan - left, -top);
    const seconds = buffer!.windowSeconds;
    const right = baseRightMs / 1000;
    const held = heldSegments(u, paths);
    heldPainted = held.length > 0;
    drawChartCanvas(ctx, u, paths.filter((_, i) => u.series[i + 1].show !== false), timeTicks(right - seconds * 2, right + seconds, tickIncrement), tickIncrement, i18n.locale, width, theme, held);
    ctx.restore();
    autoscaleY = false;
    drawFrame(performance.now());
  }

  function rebase(nowMonoMs: number, snapshot = true) {
    const range = viewport.range(nowMonoMs);
    if (paused || !plot || !range) return;
    baseRightMs = range.max * 1000;
    // The committed X scale makes uPlot re-run a visible cursor itself, at offset zero.
    canvasOffsetPx = 0;
    // uPlot commits the scale, and repaints, in a microtask: until then no painted held
    // segment may ask the frame below for another rebase.
    heldPainted = false;
    autoscaleY = snapshot;
    plot.setScale('x', range);
    drawFrame(nowMonoMs);
  }

  /**
   * Cursor points sit on the cached X scale; while the pointer is over the plot a CSS offset
   * moves them onto the scrolled curve. Without a pointer nothing is written, so scrolling
   * frames do not restyle the overlay; leaving clears the offset once.
   */
  function syncCursorOffset(u: uPlot) {
    const left = u.cursor.left;
    const next = left !== undefined && left >= 0 ? `${-canvasOffsetPx}px` : '';
    if (u.over.style.getPropertyValue('--chart-cursor-offset') === next) return;
    if (next) u.over.style.setProperty('--chart-cursor-offset', next);
    else u.over.style.removeProperty('--chart-cursor-offset');
  }

  // Between samples a frame only writes the canvas transform, which the compositor applies
  // without restyling, relayout or repainting: held segments scroll inside the canvas and the
  // fixed dots change only when the canvas is repainted.
  function drawFrame(nowMonoMs: number) {
    const range = viewport.range(nowMonoMs);
    if (paused || !plot || !canvas || !buffer || !range) return;
    const width = plot.bbox.width / uPlot.pxRatio;
    const windowMs = buffer.windowSeconds * 1000;
    const offset = scrollOffsetPx(baseRightMs, range.max * 1000, windowMs, width);
    // Repaint when the overscan is exhausted, or when painted held segments must disappear
    // because their sample left the window.
    if (offset > width || (heldPainted && heldLengthPx(buffer.lastTimestampMs, range.max * 1000, windowMs, width) === null)) {
      rebase(nowMonoMs, false);
      return;
    }
    canvas.style.transform = `translateX(${-offset}px)`;
    const moved = offset !== canvasOffsetPx;
    canvasOffsetPx = offset;
    syncCursorOffset(plot);
    // A still pointer sees the curve scroll under it: hit-test it again, as the former
    // per-frame setScale did, without touching scales or canvases.
    const { left, top } = plot.cursor;
    if (moved && left !== undefined && left >= 0 && top !== undefined) plot.setCursor({ left, top });
  }

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
    const previousEdge = viewportRevision === revision ? viewport.range(performance.now())?.max : undefined;
    viewportRevision = revision;
    // Never append values from a new schema to a plot of the previous source/unit.
    buffer = undefined;
    viewport = createChartViewport(seconds);
    if (previousEdge !== undefined) viewport.sample(previousEdge * 1000, performance.now());
    if (reducedMotion) viewport.suspend(performance.now());
    clearLayers();
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
    autoscaleY = true;
    // A rebuilt plot shows the current range at once, without catching up a transition.
    yMoves.clear();
    yShown.clear();
    clearLayers();
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
    // The horizontal grid follows the primary axis (side 3): its ticks line up with the grid,
    // so they stay on; the secondary axis (side 1) has no grid of its own, so a fixed tick mark
    // there would sit off the scrolling grid, and is left off. Both axes restrict uPlot's split
    // increments to ones coarse enough that their labels never repeat.
    const axis = (unit: (typeof scales)[number], side: 1 | 3): uPlot.Axis => ({
      scale: unit,
      side,
      size: 72,
      stroke: muted,
      grid: { show: side === 3, stroke: border, width: 1 },
      ticks: { show: side === 3, stroke: border, width: 1 },
      incrs: labelSafeIncrs(unit, i18n.locale, t),
      values: (_u, splits) => splits.map((v) => formatValue(v, unit, i18n.locale, t)),
    });
    const paths: ChartCanvasPath[] = ids.map((_, i) => ({ stroke: null, gapsClip: null, color: palette[i] }));
    const opts: uPlot.Options = {
      width: Math.max(320, container.clientWidth || 800),
      height: HEIGHT,
      cursor: {
        drag: { x: false, y: false, setScale: false },
        // uPlot's X mapping is the cached scale, drawn canvasOffsetPx to the left: hit-test
        // the drawn sample while the crosshair stays at the pointer. The default dataIdx
        // only skips undefined values, which aligned ChartBuffer columns never contain.
        dataIdx: (u) => u.posToIdx(u.cursor.left! + canvasOffsetPx),
      },
      scales: Object.fromEntries([['x', { time: true }], ...scales.map((unit) => [unit, { auto: () => autoscaleY, range: yRange(unit) }])]),
      series: [
        { label: '', value: (_u, v) => (v == null ? DASH : new Date(v * 1000).toLocaleTimeString(i18n.locale)) },
        ...ids.map((id, i) => {
          const sensor = byId.get(id);
          const unit = seriesScale[i];
          const spline = uPlot.paths.spline!();
          const capture: uPlot.Series.PathBuilder = (u, seriesIdx, first, last) => {
            const result = spline(u, seriesIdx, first, last);
            paths[i] = { stroke: result?.stroke instanceof Path2D ? result.stroke : null, gapsClip: result?.clip ?? null, color: palette[i] };
            // uPlot owns data and Y scales; only the composited canvas strokes the path.
            return null;
          };
          return {
            label: sensor ? sensorLabel(sensor, t) : id,
            scale: unit,
            stroke: palette[i],
            width: 1.5,
            paths: capture,
            spanGaps: false,
            points: { show: false },
            value: (_u: uPlot, v: number | null) => formatValue(v ?? null, unit, i18n.locale, t),
          };
        }),
      ],
      hooks: {
        draw: [(u) => paint(u, paths, { gridColor: border, textColor: muted })],
        // Legend values follow dataIdx, but uPlot publishes its uncorrected index here.
        setCursor: [(u) => {
          u.cursor.idx = u.legend.idx = u.cursor.idxs![0];
          syncCursorOffset(u);
        }],
      },
      axes: [
        {
          stroke: muted,
          size: 40,
          grid: { show: false },
          ticks: { show: false },
          values: [],
          space: timeSpace,
          splits: (_u, _axis, min, max, increment) => {
            if (Number.isFinite(increment) && increment > 0) tickIncrement = increment;
            return timeTicks(min, max, tickIncrement);
          },
        },
        axis(scales[0], 3),
        ...(scales[1] ? [axis(scales[1], 1)] : []),
      ],
    };
    plot = new uPlot(opts, buffer.data(), container);
    createLayers(ids.map((_, i) => palette[i]));
    rebase(performance.now());
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
    plot.batch(() => {
      plot!.setData(buffer!.data(), false);
      rebase(performance.now());
    });
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

  // Locale affects both the cached time labels and uPlot's legend/Y labels.
  $effect(() => {
    void i18n.locale;
    untrack(() => { if (!paused && buffer) build(buffer.ids); });
  });

  onMount(() => {
    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    reducedMotion = motionQuery.matches;
    if (reducedMotion) viewport.suspend(performance.now());
    const onMotionChange = () => {
      reducedMotion = motionQuery.matches;
      if (reducedMotion) {
        viewport.suspend(performance.now());
        // The shared clock stops too: show the target range now.
        finishY();
      } else viewport.resume(performance.now());
    };
    motionQuery.addEventListener('change', onMotionChange);
    // The Y transition runs on the chart's frame subscription, before its X translation.
    const stopFrames = subscribeChartFrame((now) => {
      stepY(now);
      drawFrame(now);
    });
    const onVisibility = () => {
      paused = document.visibilityState === 'hidden';
      if (paused) {
        generation++; // Invalidate history that is still in flight.
        viewport.reset();
        // The visible rebuild shows the current range at once.
        yMoves.clear();
      }
    };
    document.addEventListener('visibilitychange', onVisibility);
    const observer =
      typeof ResizeObserver === 'undefined'
        ? undefined
        : new ResizeObserver(() => {
            if (!paused && buffer) build(buffer.ids);
          });
    observer?.observe(container);
    const refresh = () => { if (!paused && buffer) build(buffer.ids); };
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
      observer?.disconnect();
      themeObserver.disconnect();
      densityQuery.removeEventListener('change', onDensity);
      motionQuery.removeEventListener('change', onMotionChange);
      stopFrames();
      yMoves.clear();
      clearLayers();
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
  .plot :global(.chart-canvas-clip),
  .plot :global(.chart-dot-clip) {
    position: absolute;
    overflow: hidden;
    pointer-events: none;
  }
  .plot :global(.chart-canvas) {
    position: absolute;
    top: 0;
    will-change: transform;
  }
  .plot :global(.chart-dot) {
    position: absolute;
    /* Centred on the plot's right edge, one radius inside the padded layer. */
    right: 3px;
    width: 6px;
    height: 6px;
    background: white;
    border-radius: 50%;
    transform: translate(50%, -50%);
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
  .plot :global(.u-cursor-pt) {
    /* uPlot places points on the cached X scale; follow the scrolled canvas. The
       translate property composes with the transform uPlot writes inline. */
    translate: var(--chart-cursor-offset, 0px) 0;
  }
  .plot :global(.u-select) {
    background: color-mix(in srgb, var(--text-muted) 12%, transparent);
  }
</style>
