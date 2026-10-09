<script lang="ts">
  import { onMount } from 'svelte';
  import { bannerText, health } from '../lib/health.svelte';
  import { t, i18n } from '../lib/i18n/index.svelte';
  import type { LiveStore } from '../lib/live.svelte';
  import { display } from '../lib/units.svelte';
  import type { HealthReport } from '../lib/types';
  import HealthBanner from './simple/HealthBanner.svelte';

  let { store }: { store: LiveStore } = $props();

  /** Scrolling down past this many px takes the strip away. */
  const AWAY_PX = 24;
  const NEUTRAL: HealthReport = { level: 'neutral', sinceMs: 0, revision: 0, coverage: 'complete', unavailableTargets: [], alerts: [] };
  const report = $derived(health.report ?? NEUTRAL);
  const banner = $derived(bannerText(report, store.schema, t, i18n.locale, display.temperature, display.throughput));
  const items = $derived(banner.items.map((text, i) => ({ text, level: report.alerts[i].level })));
  // A problem never leaves the screen: it shrinks to its title line instead of hiding.
  const calm = $derived(report.level === 'ok' || report.level === 'neutral');
  // Only the transition between "at the top / scrolling up" and "scrolling down" is state.
  let away = $state(false);
  const hidden = $derived(away && calm);
  const compact = $derived(away && !calm);
  const animated = !window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

  onMount(() => {
    let last = window.scrollY;
    let frame = 0;
    const update = () => {
      frame = 0;
      const y = window.scrollY;
      const next = y > AWAY_PX && (y > last || (y === last && away));
      last = y;
      if (next !== away) away = next;
    };
    // One passive listener; the work happens at most once per frame.
    const onScroll = () => {
      if (frame === 0) frame = requestAnimationFrame(update);
    };
    window.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      window.removeEventListener('scroll', onScroll);
      if (frame !== 0) cancelAnimationFrame(frame);
    };
  });

  // The sticky navs sit under the top bar plus whatever is left of the strip.
  $effect(() => {
    if (store.timestampMs <= 0) return;
    const root = document.documentElement.style;
    root.setProperty(
      '--chrome-height',
      hidden ? 'var(--topbar-h)' : `calc(var(--topbar-h) + var(${compact ? '--strip-compact-h' : '--strip-h'}))`,
    );
    return () => root.removeProperty('--chrome-height');
  });
</script>

{#if store.timestampMs > 0}
  <div class="strip" class:hidden class:compact class:animated>
    <HealthBanner level={report.level} title={banner.title} {items} elapsedMs={health.elapsedMs ?? 0} {compact} />
  </div>
{/if}

<style>
  /* Sticks under the top bar (which stacks above it). Hiding moves it up behind the bar. */
  .strip {
    position: sticky;
    top: var(--topbar-h);
    z-index: var(--z-strip);
    flex: none;
    /* Constant in-flow box: the compact banner shrinks inside it, so the page never moves. */
    height: var(--strip-h);
    pointer-events: none;
  }
  .animated {
    transition:
      transform 200ms ease,
      opacity 200ms ease,
      visibility 0s linear 0s;
  }
  .hidden {
    transform: translateY(-100%);
    opacity: 0;
    visibility: hidden;
    pointer-events: none;
  }
  .hidden.animated {
    transition:
      transform 200ms ease,
      opacity 200ms ease,
      visibility 0s linear 200ms;
  }
</style>
