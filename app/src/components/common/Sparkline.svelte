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
  const WINDOW_MS = 5 * 60_000;
  const viewport = createChartViewport(WINDOW_MS / 1000);
  let baseRightMs = $state(0);
  let lastTimestampMs: number | null = null;
  let svg = $state<SVGSVGElement>();
  let curves = $state<SVGGElement>();
  let heldGlow = $state<SVGLineElement>();
  let heldLine = $state<SVGLineElement>();
  let endpoint = $state<HTMLSpanElement>();
  let plotWidthPx = WIDTH;
  const geometry = $derived(sparklineGeometry(values, timestampsMs, baseRightMs, WINDOW_MS, WIDTH, HEIGHT, min, max));

  function updateVisual(now: number) {
    const visibleRightMs = (viewport.range(now)?.max ?? 0) * 1000;
    if (curves) {
      const offsetCssPx = scrollOffsetPx(baseRightMs, visibleRightMs, WINDOW_MS, plotWidthPx);
      // CSS transforms on an SVG group use the viewBox coordinate scale.
      const offsetSvgUnits = offsetCssPx * WIDTH / plotWidthPx;
      curves.style.transform = `translateX(${-offsetSvgUnits}px)`;
    }
    if (!heldLine || !heldGlow || !endpoint) return;
    const length = geometry.endpoint && Number.isFinite(values.at(-1))
      ? heldLengthPx(lastTimestampMs, visibleRightMs, WINDOW_MS, WIDTH)
      : null;
    const show = length !== null;
    heldLine.style.display = show ? '' : 'none';
    heldGlow.style.display = show ? '' : 'none';
    endpoint.style.display = show ? '' : 'none';
    if (length !== null) {
      const x = String(WIDTH - length);
      heldLine.setAttribute('x1', x);
      heldGlow.setAttribute('x1', x);
    }
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
      plotWidthPx = svg?.getBoundingClientRect().width || svg?.clientWidth || WIDTH;
      updateVisual(performance.now());
    });
  });

  onMount(() => {
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => {
      plotWidthPx = svg?.getBoundingClientRect().width || svg?.clientWidth || WIDTH;
      updateVisual(performance.now());
    });
    if (svg) resize?.observe(svg);
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

<div class="sparkline" aria-hidden="true">
  <svg bind:this={svg} viewBox="0 0 {WIDTH} {HEIGHT}" preserveAspectRatio="none">
    <g bind:this={curves}>
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
    </g>
    {#if geometry.endpoint}
      <line bind:this={heldGlow} class="held-glow" x1={geometry.endpoint.x} x2={WIDTH} y1={geometry.endpoint.y} y2={geometry.endpoint.y} stroke={color} stroke-width="6" opacity="0.18" vector-effect="non-scaling-stroke" />
      <line bind:this={heldLine} class="held-line" x1={geometry.endpoint.x} x2={WIDTH} y1={geometry.endpoint.y} y2={geometry.endpoint.y} stroke={color} stroke-width="2" vector-effect="non-scaling-stroke" />
    {/if}
  </svg>
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
  svg {
    display: block;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }
</style>
