<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import uPlot from 'uplot';
  import 'uplot/dist/uPlot.min.css';
  import {
    TIME_AXIS_HEIGHT,
    TIME_AXIS_MIN_SPACE,
    Y_AXIS_PX,
    formatTimeTick,
    labelSafeIncrs,
    scaleOptions,
    timeAxisSpace,
    type DisplayScale,
  } from '../../lib/advanced/chartData';
  import { drawChartCanvas, timeTickFont, type ChartCanvasPath, type ChartHeldSegment } from '../../lib/advanced/chartCanvas';
  import { heldLengthPx, scrollOffsetPx, timeTicks } from '../../lib/chartCompositor';
  import type { createChartViewport } from '../../lib/advanced/chartViewport';
  import { DASH } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { Unit } from '../../lib/types';

  // One strip of the strip chart (spec §4): one uPlot for one unit, with the composited canvas,
  // the white dots, the held segments and the Y transitions extracted from HistoryChart. The
  // parent owns buffer, viewport and frame clock, and drives the strip through the methods below.
  let {
    unit,
    colors,
    labels,
    hidden,
    format,
    viewport,
    windowSeconds,
    height,
    timeAxis,
    syncKey,
    reducedMotion,
    oncursor,
  }: {
    unit: Unit;
    /** Colour and dash of each series, in strip order. */
    colors: Array<{ color: string; dash: readonly number[] }>;
    labels: string[];
    /** Series hidden from the legend, in strip order. */
    hidden: boolean[];
    /** Writes a plotted value for the axis (`displayScale(unit).format`). */
    format: DisplayScale['format'];
    viewport: ReturnType<typeof createChartViewport>;
    windowSeconds: number;
    /** CSS height of the uPlot, the time axis included. */
    height: number;
    /** Only the last strip shows the time labels. */
    timeAxis: boolean;
    /** uPlot cursor sync key, shared by the strips of one chart. */
    syncKey: string;
    reducedMotion: boolean;
    /** Data index under the cursor of this strip or of a synced one; null once it leaves. */
    oncursor: (idx: number | null) => void;
  } = $props();

  /** Radius of the white endpoint dot in CSS px; its layer is padded by it on every side. */
  const DOT_RADIUS = 3;

  let container: HTMLDivElement;
  let plot: uPlot | undefined;
  /** The latest data handed over by the parent; the plot may hold older data while off screen. */
  let latest: uPlot.AlignedData | undefined;
  /** `latest` is newer than the plot's data (the strip was off screen). */
  let stale = false;
  /** Without IntersectionObserver (jsdom) a strip is always on screen. */
  let visible = true;
  /** The window is hidden: nothing is drawn until the parent rebuilds the strip. */
  let paused = false;
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
  /** CSS width the plot was built or last sized at; only a new width resizes it. */
  let plotCssWidth = 0;

  type YRange = { min: number; max: number };
  /** Duration in ms of the move to a new automatic Y range. */
  const Y_TRANSITION_MS = 180;
  /** The Y range uPlot displays, by scale key: the last autoscale result or transition frame. */
  const yShown = new Map<string, YRange>();
  /** Running Y transitions, by scale key; empty between them, so frames stay transform-only. */
  const yMoves = new Map<string, { from: YRange; to: YRange; startMs: number }>();
  /** Set while the transition itself sets Y scales, which are then no autoscale target. */
  let settingY = false;

  /** Timestamp in ms of the latest sample handed over; data x values are seconds. */
  function lastTimestampMs(): number | null {
    const x = latest?.[0].at(-1);
    return x == null ? null : Math.round(x * 1000);
  }

  /** A solid line has no dash to pass on. */
  const dashOf = (dash: readonly number[]) => (dash.length > 0 ? dash : undefined);

  /**
   * uPlot `scale.range` of an automatic Y scale: the unit's own range (as uPlot would compute
   * it from `scaleOptions`) is the target. A changed target is not shown at once: uPlot keeps
   * the displayed range and the frames below move towards the target, restarting from wherever
   * the scale is when the next snapshot changes it again.
   */
  function yRange(scaleUnit: Parameters<typeof scaleOptions>[0]): uPlot.Range.Function {
    const config = scaleOptions(scaleUnit).range as uPlot.Range.Config | undefined;
    return (_u, dataMin, dataMax, key) => {
      if (settingY) return [dataMin, dataMax];
      // No values in view (a suspended device): keep the axis that was shown, else the unit's
      // soft bounds, else 0..1, so uPlot always has a valid Y scale.
      if (dataMin == null || dataMax == null) {
        const shown = yShown.get(key);
        if (shown) return [shown.min, shown.max];
        const soft = config as { min?: { soft?: number }; max?: { soft?: number } } | undefined;
        const lo = soft?.min?.soft;
        const hi = soft?.max?.soft;
        return lo !== undefined && hi !== undefined ? [lo, hi] : [0, 1];
      }
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
  export function finishY() {
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

  function createLayers(count: number) {
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
    for (let i = 0; i < count; i++) {
      const dot = document.createElement('div');
      dot.className = 'chart-dot';
      dot.hidden = true;
      dots.push(dot);
      dotClip.append(dot);
    }
    // The wrap shares uPlot's canvas origin.
    plot.over.parentElement!.append(canvasClip, dotClip);
  }

  /** One frame of the parent's clock: the Y transition, then the X translation. */
  export function frame(nowMonoMs: number) {
    if (!visible) return;
    stepY(nowMonoMs);
    drawFrame(nowMonoMs);
  }

  /**
   * Held segments of the series whose last real value is finite, inside its Y scale and whose
   * sample is still in the window, in canvas px; the matching dots are shown and placed.
   */
  function heldSegments(u: uPlot, paths: ChartCanvasPath[]): ChartHeldSegment[] {
    const ratio = uPlot.pxRatio;
    const { top, height: plotHeight } = u.bbox;
    const windowMs = windowSeconds * 1000;
    const visibleRightMs = (viewport.range(performance.now())?.max ?? baseRightMs / 1000) * 1000;
    const inWindow = heldLengthPx(lastTimestampMs(), visibleRightMs, windowMs, u.bbox.width / ratio) !== null;
    const lastX = u.data[0]?.at(-1);
    const segments: ChartHeldSegment[] = [];
    for (let i = 0; i < dots.length; i++) {
      const value = u.data[i + 1]?.at(-1);
      const scale = u.series[i + 1].scale!;
      const drawnY = value == null ? NaN : u.valToPos(value, scale, true);
      // While the scale moves towards a range that holds the value, the displayed range may
      // not hold it yet: keep the dot and held segment, clamped to the plot, instead of
      // blinking them out until the eased range catches up.
      const move = yMoves.get(scale);
      const inScale = move
        ? value != null && value >= move.to.min && value <= move.to.max
        : drawnY >= top && drawnY <= top + plotHeight;
      const y = move ? Math.min(top + plotHeight, Math.max(top, drawnY)) : drawnY;
      const show = inWindow && lastX != null && u.series[i + 1].show !== false
        && Number.isFinite(value) && Number.isFinite(y) && inScale;
      dots[i].hidden = !show;
      if (!show) continue;
      dots[i].style.top = `${(y - top) / ratio + DOT_RADIUS}px`;
      segments.push({ x: u.valToPos(lastX, 'x', true), y, color: paths[i].color, dash: paths[i].dash });
    }
    return segments;
  }

  function paint(u: uPlot, paths: ChartCanvasPath[], theme: { gridColor: string; textColor: string }) {
    if (!canvas || !canvasClip || !dotClip) return;
    const ratio = uPlot.pxRatio;
    const { left, top, width, height: plotHeight } = u.bbox;
    // Only the last strip has room for the time labels under its lines.
    const axisHeight = (timeAxis ? TIME_AXIS_HEIGHT : 0) * ratio;
    const leftOverscan = Y_AXIS_PX * ratio;
    // One window of overscan keeps memory bounded, even if telemetry stops.
    canvasClip.style.left = `${left / ratio}px`;
    canvasClip.style.top = `${top / ratio}px`;
    canvasClip.style.width = `${width / ratio}px`;
    canvasClip.style.height = `${(plotHeight + axisHeight) / ratio}px`;
    dotClip.style.left = `${left / ratio - DOT_RADIUS}px`;
    dotClip.style.top = `${top / ratio - DOT_RADIUS}px`;
    dotClip.style.width = `${width / ratio + DOT_RADIUS * 2}px`;
    dotClip.style.height = `${plotHeight / ratio + DOT_RADIUS * 2}px`;
    // Only labels need space behind the left edge; future ticks need a full window.
    const canvasWidth = Math.ceil(width * 2 + leftOverscan);
    const canvasHeight = Math.ceil(plotHeight + axisHeight);
    // Assigning a size reallocates (and clears) the backing store: do it only when it changes.
    if (canvas.width !== canvasWidth) canvas.width = canvasWidth;
    if (canvas.height !== canvasHeight) canvas.height = canvasHeight;
    canvas.style.width = `${(width * 2 + leftOverscan) / ratio}px`;
    canvas.style.height = `${(plotHeight + axisHeight) / ratio}px`;
    canvas.style.left = `${-leftOverscan / ratio}px`;
    heldPainted = false;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.save();
    ctx.translate(leftOverscan - left, -top);
    const right = baseRightMs / 1000;
    const held = heldSegments(u, paths);
    heldPainted = held.length > 0;
    drawChartCanvas(ctx, u, paths.filter((_, i) => u.series[i + 1].show !== false), timeTicks(right - windowSeconds * 2, right + windowSeconds, tickIncrement), tickIncrement, i18n.locale, width, theme, held, { timeLabels: timeAxis });
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
    if (paused || !plot || !canvas || !latest || !range) return;
    const width = plot.bbox.width / uPlot.pxRatio;
    const windowMs = windowSeconds * 1000;
    const offset = scrollOffsetPx(baseRightMs, range.max * 1000, windowMs, width);
    // Repaint when the overscan is exhausted, or when painted held segments must disappear
    // because their sample left the window.
    if (offset > width || (heldPainted && heldLengthPx(lastTimestampMs(), range.max * 1000, windowMs, width) === null)) {
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

  /** Creates (or recreates) the plot with `data`: x in seconds, then one column per series. */
  export function build(data: uPlot.AlignedData) {
    paused = false;
    autoscaleY = true;
    // A rebuilt plot shows the current range at once, without catching up a transition.
    yMoves.clear();
    yShown.clear();
    clearLayers();
    plot?.destroy();
    plot = undefined;
    latest = data;
    stale = false;
    const css = getComputedStyle(document.documentElement);
    const read = (token: string) => css.getPropertyValue(token).trim();
    const muted = read('--text-muted');
    const border = read('--border');
    const paths: ChartCanvasPath[] = colors.map(({ color, dash }) => ({ stroke: null, gapsClip: null, color, dash: dashOf(dash) }));
    plotCssWidth = Math.max(320, container.clientWidth || 800);
    const opts: uPlot.Options = {
      width: plotCssWidth,
      height,
      // StripLegend replaces uPlot's legend.
      legend: { show: false },
      cursor: {
        drag: { x: false, y: false, setScale: false },
        // The strips of one chart share the cursor: the same instant in every strip.
        sync: { key: syncKey },
        // uPlot's X mapping is the cached scale, drawn canvasOffsetPx to the left: hit-test
        // the drawn sample while the crosshair stays at the pointer. The default dataIdx
        // only skips undefined values, which aligned ChartBuffer columns never contain.
        dataIdx: (u) => u.posToIdx(u.cursor.left! + canvasOffsetPx),
      },
      scales: { x: { time: true }, [unit]: { auto: () => autoscaleY, range: yRange(unit) } },
      series: [
        { label: '', value: (_u, v) => (v == null ? DASH : new Date(v * 1000).toLocaleTimeString(i18n.locale)) },
        ...labels.map((label, i) => {
          const spline = uPlot.paths.spline!();
          const capture: uPlot.Series.PathBuilder = (u, seriesIdx, first, last) => {
            const result = spline(u, seriesIdx, first, last);
            paths[i] = {
              stroke: result?.stroke instanceof Path2D ? result.stroke : null,
              gapsClip: result?.clip ?? null,
              color: colors[i].color,
              dash: dashOf(colors[i].dash),
            };
            // uPlot owns data and Y scales; only the composited canvas strokes the path.
            return null;
          };
          return {
            label,
            scale: unit,
            stroke: colors[i].color,
            width: 1.5,
            paths: capture,
            show: !hidden[i],
            spanGaps: false,
            points: { show: false },
            value: (_u: uPlot, v: number | null) => format(v ?? null, i18n.locale, t),
          };
        }),
      ],
      hooks: {
        draw: [(u) => paint(u, paths, { gridColor: border, textColor: muted })],
        // Values follow dataIdx, but uPlot publishes its uncorrected index here.
        setCursor: [(u) => {
          u.cursor.idx = u.legend.idx = u.cursor.idxs![0];
          syncCursorOffset(u);
          // An off-screen strip may hold older data: its index is not the chart's.
          if (visible) oncursor(u.cursor.left !== undefined && u.cursor.left >= 0 ? u.cursor.idxs![0] ?? null : null);
        }],
      },
      axes: [
        {
          stroke: muted,
          // The other strips keep the vertical grid lines, but no room for time labels.
          size: timeAxis ? TIME_AXIS_HEIGHT : 0,
          grid: { show: false },
          ticks: { show: false },
          values: [],
          space: timeSpace,
          splits: (_u, _axis, min, max, increment) => {
            if (Number.isFinite(increment) && increment > 0) tickIncrement = increment;
            return timeTicks(min, max, tickIncrement);
          },
        },
        {
          scale: unit,
          side: 3,
          size: Y_AXIS_PX,
          stroke: muted,
          grid: { show: true, stroke: border, width: 1 },
          ticks: { show: true, stroke: border, width: 1 },
          // Increments coarse enough that the labels never repeat.
          incrs: labelSafeIncrs(unit, i18n.locale, t, format),
          values: (_u, splits) => splits.map((v) => format(v, i18n.locale, t)),
        },
      ],
    };
    plot = new uPlot(opts, data, container);
    createLayers(colors.length);
    rebase(performance.now());
  }

  /** A new sample: the parent's data, then a rebase on the shared viewport. Off screen, kept for later. */
  export function setData(data: uPlot.AlignedData, nowMonoMs: number) {
    latest = data;
    if (!plot || paused) return;
    if (!visible) {
      stale = true;
      return;
    }
    plot.batch(() => {
      plot!.setData(data, false);
      rebase(nowMonoMs);
    });
  }

  /** The window was hidden: stop drawing and drop running transitions until the next build. */
  export function reset() {
    paused = true;
    yMoves.clear();
  }

  export function destroy() {
    yMoves.clear();
    clearLayers();
    plot?.destroy();
    plot = undefined;
    latest = undefined;
  }

  // The legend hides and shows series: uPlot redraws, and the draw hook repaints the layers.
  $effect(() => {
    const next = hidden;
    untrack(() => {
      if (!plot) return;
      next.forEach((off, i) => {
        if (plot!.series[i + 1] && (plot!.series[i + 1].show !== false) === off) plot!.setSeries(i + 1, { show: !off });
      });
    });
  });

  onMount(() => {
    const resizeObserver =
      typeof ResizeObserver === 'undefined'
        ? undefined
        : new ResizeObserver((entries) => {
            // Only a new width sizes the plot. uPlot keeps scales, series and the time base,
            // and its draw hook repaints the layers at the new size.
            const width = Math.max(320, (entries.at(-1)?.contentRect.width ?? container.clientWidth) || 800);
            if (paused || !plot || width === plotCssWidth) return;
            plotCssWidth = width;
            plot.setSize({ width, height });
          });
    resizeObserver?.observe(container);
    // Off screen, a strip neither recomputes paths nor draws; back on screen it catches up.
    const intersectionObserver =
      typeof IntersectionObserver === 'undefined'
        ? undefined
        : new IntersectionObserver((entries) => {
            const next = entries.at(-1)?.isIntersecting ?? visible;
            if (next === visible) return;
            visible = next;
            if (!visible || !plot || paused || !latest) return;
            const data = latest;
            const resync = stale;
            stale = false;
            plot.batch(() => {
              if (resync) plot!.setData(data, false);
              rebase(performance.now());
            });
          });
    intersectionObserver?.observe(container);
    return () => {
      resizeObserver?.disconnect();
      intersectionObserver?.disconnect();
      destroy();
    };
  });
</script>

<div class="plot" bind:this={container} style:min-height="{height}px"></div>

<style>
  .plot {
    position: relative;
    min-width: 0;
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
