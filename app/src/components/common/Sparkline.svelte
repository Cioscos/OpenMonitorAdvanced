<script lang="ts">
  import { sparklinePath } from '../../lib/sparkline';

  let {
    values,
    capacity,
    color = 'var(--accent)',
    min = 0,
    max,
  }: { values: number[]; capacity: number; color?: string; min?: number; max?: number } = $props();

  const WIDTH = 150;
  const HEIGHT = 34;
  const path = $derived(sparklinePath(values, WIDTH, HEIGHT, min, max, capacity));
</script>

<svg class="sparkline" viewBox="0 0 {WIDTH} {HEIGHT}" preserveAspectRatio="none" aria-hidden="true">
  <path
    d={path}
    fill="none"
    stroke={color}
    stroke-width="2"
    stroke-linejoin="round"
    stroke-linecap="round"
    vector-effect="non-scaling-stroke"
  />
</svg>

<style>
  .sparkline {
    display: block;
    width: 100%;
    height: 34px;
  }
</style>
