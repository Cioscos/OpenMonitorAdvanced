<script lang="ts">
  import { onMount } from 'svelte';
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
  let rightEdgeMs = $state(0);
  let sampleMonotonicMs = 0;
  let sampleTimestampMs = 0;
  const geometry = $derived(sparklineGeometry(values, timestampsMs, rightEdgeMs, WINDOW_MS, WIDTH, HEIGHT, min, max));

  $effect(() => {
    const latest = timestampsMs.at(-1);
    if (latest === undefined) return;
    sampleTimestampMs = latest;
    sampleMonotonicMs = performance.now();
    rightEdgeMs = latest;
  });

  onMount(() => subscribeChartFrame((now) => {
    if (timestampsMs.length > 0) rightEdgeMs = sampleTimestampMs + Math.max(0, now - sampleMonotonicMs);
  }));
</script>

<svg class="sparkline" viewBox="0 0 {WIDTH} {HEIGHT}" preserveAspectRatio="none" aria-hidden="true">
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
    <circle cx={geometry.endpoint.x} cy={geometry.endpoint.y} r="2.5" fill="white" />
  {/if}
</svg>

<style>
  .sparkline {
    display: block;
    width: 100%;
    height: 34px;
  }
</style>
