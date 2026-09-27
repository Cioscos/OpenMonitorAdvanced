<script lang="ts">
  import { onMount } from 'svelte';
  import { createChartViewport } from '../../lib/advanced/chartViewport';
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
  let rightEdgeMs = $state(0);
  let lastTimestampMs: number | null = null;
  const geometry = $derived(sparklineGeometry(values, timestampsMs, rightEdgeMs, WINDOW_MS, WIDTH, HEIGHT, min, max));

  $effect(() => {
    const latest = timestampsMs.at(-1);
    if (latest === undefined) {
      viewport.reset();
      lastTimestampMs = null;
      return;
    }
    if (lastTimestampMs !== null && latest < lastTimestampMs) viewport.reset();
    lastTimestampMs = latest;
    const now = performance.now();
    viewport.sample(latest, now);
    rightEdgeMs = (viewport.range(now)?.max ?? latest / 1000) * 1000;
  });

  onMount(() => {
    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    const syncPause = () => {
      const now = performance.now();
      if (document.visibilityState !== 'visible' || motionQuery.matches) viewport.suspend(now);
      else viewport.resume(now);
      const range = viewport.range(now);
      if (range) rightEdgeMs = range.max * 1000;
    };
    document.addEventListener('visibilitychange', syncPause);
    motionQuery.addEventListener('change', syncPause);
    syncPause();
    const stopFrames = subscribeChartFrame((now) => {
      const range = viewport.range(now);
      if (range) rightEdgeMs = range.max * 1000;
    });
    return () => {
      stopFrames();
      document.removeEventListener('visibilitychange', syncPause);
      motionQuery.removeEventListener('change', syncPause);
    };
  });
</script>

<div class="sparkline" aria-hidden="true">
  <svg viewBox="0 0 {WIDTH} {HEIGHT}" preserveAspectRatio="none">
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
  </svg>
  {#if geometry.endpoint}
    <span
      class="endpoint"
      style="position: absolute; width: 5px; height: 5px; border-radius: 50%; background: white; transform: translate(-50%, -50%); pointer-events: none;"
      style:left="{(geometry.endpoint.x / WIDTH) * 100}%"
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
  }
</style>
