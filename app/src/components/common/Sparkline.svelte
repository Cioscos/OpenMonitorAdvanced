<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { createChartViewport } from '../../lib/advanced/chartViewport';
  import { heldLengthPx, scrollOffsetPx } from '../../lib/chartCompositor';
  import { subscribeChartFrame } from '../../lib/chartFrameClock';
  import { sparklineGeometry } from '../../lib/sparkline';

  let {
    values,
    timestampsMs,
    color = 'var(--accent)',
    min = 0,
    max,
  }: { values: number[]; timestampsMs: number[]; color?: string; min?: number; max?: number } = $props();

  const WIDTH = 150;
  const HEIGHT = 34;
  /** The scrolled SVG spans two tiles: the held segment still reaches the edge after a full window. */
  const SCROLL_WIDTH = WIDTH * 2;
  const WINDOW_MS = 5 * 60_000;
  const viewport = createChartViewport(WINDOW_MS / 1000);
  let baseRightMs = $state(0);
  let lastTimestampMs: number | null = null;
  let root = $state<HTMLDivElement>();
  let scroller = $state<HTMLDivElement>();
  let heldGlow = $state<SVGLineElement>();
  let heldLine = $state<SVGLineElement>();
  let endpoint = $state<HTMLSpanElement>();
  let plotWidthPx = WIDTH;
  /** Held segment and dot visibility last written; null forces the next write. */
  let heldShown: boolean | null = null;
  const geometry = $derived(sparklineGeometry(values, timestampsMs, baseRightMs, WINDOW_MS, WIDTH, HEIGHT, min, max));

  function measure() {
    plotWidthPx = root?.getBoundingClientRect().width || root?.clientWidth || WIDTH;
  }

  // Between samples a frame only writes the scroll layer's transform, which the compositor
  // applies without restyling, relayout or repainting the SVG. The held segment is drawn
  // inside that layer out to its right end, and the fixed dot changes only at samples.
  function updateVisual(now: number) {
    const visibleRightMs = (viewport.range(now)?.max ?? 0) * 1000;
    // CSS px of the rendered tile, so path, held segment and dot meet at every width.
    if (scroller) scroller.style.transform = `translateX(${-scrollOffsetPx(baseRightMs, visibleRightMs, WINDOW_MS, plotWidthPx)}px)`;
    if (!heldLine || !heldGlow || !endpoint) return;
    const show = geometry.endpoint !== null && Number.isFinite(values.at(-1))
      && heldLengthPx(lastTimestampMs, visibleRightMs, WINDOW_MS, WIDTH) !== null;
    if (show === heldShown) return;
    heldShown = show;
    heldLine.style.display = show ? '' : 'none';
    heldGlow.style.display = show ? '' : 'none';
    endpoint.style.display = show ? '' : 'none';
  }

  $effect(() => {
    const latest = timestampsMs.at(-1);
    if (latest === undefined) {
      viewport.reset();
      lastTimestampMs = null;
      baseRightMs = 0;
      return;
    }
    if (lastTimestampMs !== null && latest < lastTimestampMs) viewport.reset();
    lastTimestampMs = latest;
    const now = performance.now();
    viewport.sample(latest, now);
    baseRightMs = (viewport.range(now)?.max ?? latest / 1000) * 1000;
    void tick().then(() => {
      measure();
      // The held elements may have been re-rendered for the new sample.
      heldShown = null;
      updateVisual(performance.now());
    });
  });

  onMount(() => {
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => {
      measure();
      updateVisual(performance.now());
    });
    if (root) resize?.observe(root);
    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    const syncPause = () => {
      const now = performance.now();
      if (document.visibilityState !== 'visible' || motionQuery.matches) viewport.suspend(now);
      else viewport.resume(now);
      updateVisual(now);
    };
    document.addEventListener('visibilitychange', syncPause);
    motionQuery.addEventListener('change', syncPause);
    syncPause();
    const stopFrames = subscribeChartFrame((now) => {
      updateVisual(now);
    });
    return () => {
      stopFrames();
      resize?.disconnect();
      document.removeEventListener('visibilitychange', syncPause);
      motionQuery.removeEventListener('change', syncPause);
    };
  });
</script>

<div class="sparkline" bind:this={root} aria-hidden="true">
  <!-- Its own compositor layer: frames translate it without repainting the SVG. -->
  <div class="sparkline-scroll" bind:this={scroller} style:will-change="transform">
    <svg viewBox="0 0 {SCROLL_WIDTH} {HEIGHT}" preserveAspectRatio="none">
      <path
        d={geometry.path}
        fill="none"
        stroke={color}
        stroke-width="6"
        opacity="0.18"
        stroke-linejoin="round"
        stroke-linecap="round"
        vector-effect="non-scaling-stroke"
      />
      <path
        d={geometry.path}
        fill="none"
        stroke={color}
        stroke-width="2"
        stroke-linejoin="round"
        stroke-linecap="round"
        vector-effect="non-scaling-stroke"
      />
      {#if geometry.endpoint}
        <!-- Visual projection only: the last real value held out to the layer's right end. -->
        <line bind:this={heldGlow} class="held-glow" x1={geometry.endpoint.x} x2={SCROLL_WIDTH} y1={geometry.endpoint.y} y2={geometry.endpoint.y} stroke={color} stroke-width="6" opacity="0.18" vector-effect="non-scaling-stroke" />
        <line bind:this={heldLine} class="held-line" x1={geometry.endpoint.x} x2={SCROLL_WIDTH} y1={geometry.endpoint.y} y2={geometry.endpoint.y} stroke={color} stroke-width="2" vector-effect="non-scaling-stroke" />
      {/if}
    </svg>
  </div>
  {#if geometry.endpoint}
    <span bind:this={endpoint}
      class="endpoint"
      style="position: absolute; width: 5px; height: 5px; border-radius: 50%; background: white; transform: translate(-50%, -50%); pointer-events: none;"
      style:left="100%"
      style:top="{geometry.endpoint.y}px"
    ></span>
  {/if}
</div>

<style>
  .sparkline {
    position: relative;
    display: block;
    width: 100%;
    height: 34px;
    overflow: hidden;
  }
  .sparkline-scroll {
    position: absolute;
    top: 0;
    left: 0;
    /* Two tiles wide, the right one holding the held segment while the layer scrolls. */
    width: 200%;
    height: 100%;
  }
  svg {
    display: block;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }
</style>
