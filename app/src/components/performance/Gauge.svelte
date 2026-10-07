<script lang="ts">
  import { untrack } from 'svelte';
  import { angleFor, smooth, START_ANGLE, SWEEP, ticks } from '../../lib/performance/gauge';

  // Style C of the approved mockup («Ibrido con riferimento»): metal bezel, purple carbon dial,
  // lit arc under the ticks, pink needle, white ▲ at the reference and a dot-matrix display.
  let {
    value,
    max,
    reference,
    label,
    unit,
    moving,
  }: { value: number | null; max: number; reference: number | null; label: string; unit: string; moving: boolean } =
    $props();

  const uid = $props.id();
  const C = 150;
  const ARC_R = 127;
  const TICK_OUTER = 122;
  /** Below this gap (a fraction of the full scale) the needle snaps to the value. */
  const SNAP = 1e-4;

  function polar(r: number, deg: number): [number, number] {
    const a = (deg * Math.PI) / 180;
    return [C + r * Math.cos(a), C + r * Math.sin(a)];
  }

  function arc(r: number, a0: number, a1: number): string {
    const end = Math.max(a1, a0 + 0.01);
    const [x0, y0] = polar(r, a0);
    const [x1, y1] = polar(r, end);
    return `M${x0} ${y0} A${r} ${r} 0 ${end - a0 > 180 ? 1 : 0} 1 ${x1} ${y1}`;
  }

  const target = $derived(value ?? 0);
  let shown = $state(untrack(() => value ?? 0));
  let visible = $state(document.visibilityState === 'visible');

  $effect(() => {
    const onVisibility = () => (visible = document.visibilityState === 'visible');
    document.addEventListener('visibilitychange', onVisibility);
    return () => document.removeEventListener('visibilitychange', onVisibility);
  });

  // Animates only while measuring, with the page visible and the needle away from the value;
  // otherwise (and with reduced motion) the needle jumps, so an idle page costs no frames.
  $effect(() => {
    const goal = target;
    const near = (v: number) => Math.abs(goal - v) <= max * SNAP;
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (!moving || !visible || reduced || untrack(() => near(shown))) {
      shown = goal;
      return;
    }
    let last = performance.now();
    let frame = requestAnimationFrame(function step(now) {
      const next = smooth(shown, goal, now - last);
      last = now;
      shown = near(next) ? goal : next;
      if (shown !== goal) frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  });

  const angle = $derived(angleFor(shown, max));
  const dial = $derived(
    ticks(max).map((t) => {
      const len = t.kind === 'major' ? 14 : t.kind === 'mid' ? 10 : 6;
      const [x1, y1] = polar(TICK_OUTER, t.angle);
      const [x2, y2] = polar(TICK_OUTER - len, t.angle);
      const [lx, ly] = polar(96, t.angle);
      return { ...t, x1, y1, x2, y2, lx, ly };
    }),
  );
  const marker = $derived.by(() => {
    if (reference === null) return null;
    const a = angleFor(reference, max);
    return [polar(136, a), polar(144, a - 3), polar(144, a + 3)].map(([x, y]) => `${x},${y}`).join(' ');
  });
  const readout = $derived(value === null ? '' : String(Math.round(shown)));
  const ghost = $derived('8'.repeat(Math.max(4, readout.length)));
  const ends = [START_ANGLE, START_ANGLE + SWEEP].map((a) => polar(124, a));
</script>

<div
  class="gauge"
  role="meter"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={max}
  aria-valuenow={value === null ? undefined : Math.round(value)}
>
  <svg viewBox="0 0 300 300" aria-hidden="true">
    <defs>
      <linearGradient id="{uid}-bezel" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color="#6d6386" />
        <stop offset="0.45" stop-color="#231b33" />
        <stop offset="1" stop-color="#4d4463" />
      </linearGradient>
      <radialGradient id="{uid}-vignette" cx="0.5" cy="0.5" r="0.5">
        <stop offset="0.55" stop-color="#000" stop-opacity="0" />
        <stop offset="1" stop-color="#000" stop-opacity="0.7" />
      </radialGradient>
      <pattern id="{uid}-carbon" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
        <rect width="6" height="6" fill="#150f22" />
        <rect width="3" height="3" fill="#1e1630" />
        <rect x="3" y="3" width="3" height="3" fill="#1e1630" />
      </pattern>
      <pattern id="{uid}-dots" width="2.6" height="2.6" patternUnits="userSpaceOnUse">
        <circle cx="1.3" cy="1.3" r="1" fill="#fff" />
      </pattern>
      <mask id="{uid}-matrix">
        <rect width="300" height="300" fill="url(#{uid}-dots)" />
      </mask>
      <linearGradient id="{uid}-arc" gradientUnits="userSpaceOnUse" x1="30" y1="250" x2="270" y2="250">
        <stop offset="0" class="from" />
        <stop offset="1" class="to" />
      </linearGradient>
      <radialGradient id="{uid}-hub" cx="0.35" cy="0.35" r="0.7">
        <stop offset="0" stop-color="#8a80a3" />
        <stop offset="1" stop-color="#1c1628" />
      </radialGradient>
    </defs>

    <circle cx={C} cy={C} r="147" fill="url(#{uid}-bezel)" />
    <circle cx={C} cy={C} r="137" fill="#08060d" />
    <circle cx={C} cy={C} r="133" fill="url(#{uid}-carbon)" />
    <circle cx={C} cy={C} r="133" fill="url(#{uid}-vignette)" />
    <path d="M40 120 A112 112 0 0 1 260 120 Q150 70 40 120 Z" fill="#fff" opacity="0.045" />

    <path class="track" d={arc(ARC_R, START_ANGLE, START_ANGLE + SWEEP)} />
    <path class="lit" d={arc(ARC_R, START_ANGLE, angle)} stroke="url(#{uid}-arc)" />

    {#each dial as t (t.angle)}
      <line class="tick {t.kind}" x1={t.x1} y1={t.y1} x2={t.x2} y2={t.y2} />
      {#if t.label !== undefined}
        <text class="tick-label" x={t.lx} y={t.ly + 5}>{t.label}</text>
      {/if}
    {/each}
    {#each ends as [x, y], i (i)}
      <circle class="end" cx={x} cy={y} r="2.5" />
    {/each}

    {#if marker !== null}
      <polygon class="reference" points={marker} />
    {/if}

    <text class="title" x={C} y="116">{label}</text>
    <rect class="lcd" x="92" y="180" width="116" height="32" rx="4" />
    <g mask="url(#{uid}-matrix)">
      <text class="digits ghost" x="202" y="205">{ghost}</text>
      <text class="digits" x="202" y="205">{readout}</text>
    </g>
    <text class="unit" x={C} y="236">{unit}</text>

    <g class="needle" transform="rotate({angle} {C} {C})">
      <polygon points="{C - 22},{C - 3.2} {C + 112},{C - 1} {C + 112},{C + 1} {C - 22},{C + 3.2}" />
    </g>
    <circle cx={C} cy={C} r="15" fill="url(#{uid}-hub)" stroke="#0b0812" stroke-width="2" />
    <circle cx={C} cy={C} r="6" fill="#08060d" />
  </svg>
</div>

<style>
  .gauge {
    width: 100%;
    max-width: 300px;
    aspect-ratio: 1;
  }

  svg {
    display: block;
    width: 100%;
    height: 100%;
  }

  .from {
    stop-color: var(--accent-2);
  }

  .to {
    stop-color: var(--accent);
  }

  .track,
  .lit {
    fill: none;
    stroke-width: 4;
    stroke-linecap: round;
  }

  .track {
    stroke: var(--border);
  }

  .tick {
    stroke: #cfc6e0;
    stroke-linecap: round;
    opacity: 0.75;
    stroke-width: 1.4;
  }

  .tick.mid {
    stroke-width: 2.5;
  }

  .tick.major {
    stroke-width: 3.5;
  }

  .tick-label {
    font-family: 'Orbitron', var(--font);
    font-size: 15px;
    fill: #b9aecf;
    text-anchor: middle;
  }

  .end {
    fill: var(--accent-2);
  }

  .reference {
    fill: var(--text);
  }

  .title {
    font-family: 'Orbitron', var(--font);
    font-size: 23px;
    fill: #cfc6e0;
    opacity: 0.85;
    text-anchor: middle;
  }

  .unit {
    font-family: 'Orbitron', var(--font);
    font-size: 18px;
    fill: var(--accent-2);
    text-anchor: middle;
  }

  .lcd {
    fill: #07050c;
    stroke: var(--border);
  }

  .digits {
    font-family: 'Share Tech Mono', monospace;
    font-size: 25px;
    fill: var(--accent-2);
    text-anchor: end;
  }

  .ghost {
    opacity: 0.1;
  }

  .needle polygon {
    fill: var(--accent);
  }
</style>
