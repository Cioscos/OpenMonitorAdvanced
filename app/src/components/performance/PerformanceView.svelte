<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend';
  import type { LiveStore } from '../../lib/live.svelte';
  import { t } from '../../lib/i18n/index.svelte';
  import { benchStore } from '../../lib/performance/bench.svelte';
  import { performanceStore } from '../../lib/performance/performance.svelte';
  import type { PerformancePage } from '../../lib/view';
  import CpuScore from './CpuScore.svelte';
  import StressHistory from './StressHistory.svelte';
  import StressResult from './StressResult.svelte';
  import StressRun from './StressRun.svelte';
  import StressWizard from './StressWizard.svelte';

  // The Performance view (spec M8 §3.1): the sidebar with the «Score» and «Stress test» groups on
  // the left, the page on the right. The stores are connected only while the view is on screen.
  // `score-cpu` is the CPU benchmark; `new` is the wizard, `run` the test under way,
  // `result:<id>` a saved session; `history` lists the saved ones.
  // `store` is the app's live store, for the run page's chart.
  let { backend, store, page = $bindable('new') }: { backend: Backend; store: LiveStore; page?: PerformancePage } = $props();
  const open = (next: PerformancePage) => (page = next);

  onMount(() => {
    const offs: (() => void)[] = [];
    let cancelled = false;
    const keep = (unsubscribe: () => void) => {
      if (cancelled) unsubscribe();
      else offs.push(unsubscribe);
    };
    performanceStore
      .connect(backend)
      .then(keep)
      .catch((error) => console.error('stress test status unavailable', error));
    benchStore
      .connect(backend)
      .then(keep)
      .catch((error) => console.error('CPU benchmark status unavailable', error));
    return () => {
      cancelled = true;
      offs.forEach((off) => off());
    };
  });

  // A test that starts (here, from the tray or before the view opened) brings its page forward;
  // the user can still move to another page while it runs.
  let wasRunning = false;
  $effect(() => {
    const running = performanceStore.running;
    if (running && !wasRunning) page = 'run';
    wasRunning = running;
  });
  let benchWasRunning = false;
  $effect(() => {
    const running = benchStore.running;
    if (running && !benchWasRunning) page = 'score-cpu';
    benchWasRunning = running;
  });

  const current = $derived(page === 'score-cpu' ? 'score' : page === 'run' || page === 'new' ? 'test' : 'history');
  const title = $derived(
    page === 'score-cpu'
      ? t('performance.score.title')
      : page === 'new'
      ? t('performance.nav.new')
      : page === 'run'
        ? t('performance.run.title')
        : page === 'history'
          ? t('performance.nav.history')
          : t('performance.result.title'),
  );
</script>

<div class="performance">
  <nav aria-label={t('view.performance')}>
    <p class="group" id="performance-group-score">{t('performance.nav.score')}</p>
    <div class="entries" role="group" aria-labelledby="performance-group-score">
      <button
        type="button"
        class="entry"
        class:on={current === 'score'}
        class:live={benchStore.running}
        aria-current={current === 'score' ? 'page' : undefined}
        onclick={() => (page = 'score-cpu')}
      >
        {t('performance.nav.scoreCpu')}{#if benchStore.running}<span class="dot" aria-hidden="true"> ●</span>{/if}
      </button>
    </div>
    <p class="group" id="performance-group-stress">{t('performance.nav.stress')}</p>
    <div class="entries" role="group" aria-labelledby="performance-group-stress">
      <button
        type="button"
        class="entry"
        class:on={current === 'test'}
        class:live={performanceStore.running}
        aria-current={current === 'test' ? 'page' : undefined}
        onclick={() => (page = performanceStore.running ? 'run' : 'new')}
      >
        {performanceStore.running ? t('performance.nav.running') : t('performance.nav.new')}
      </button>
      <button
        type="button"
        class="entry"
        class:on={current === 'history'}
        aria-current={current === 'history' ? 'page' : undefined}
        onclick={() => (page = 'history')}
      >
        {t('performance.nav.history')}
      </button>
    </div>
  </nav>

  <section class="content" aria-labelledby="performance-page-title">
    <h2 id="performance-page-title">{title}</h2>
    {#if page === 'score-cpu'}
      <CpuScore />
    {:else if page === 'new'}
      <StressWizard {backend} onStarted={() => (page = 'run')} />
    {:else if page === 'run'}
      <StressRun {backend} {store} onOpen={open} />
    {:else if page === 'history'}
      <StressHistory {backend} onOpen={open} />
    {:else if page.startsWith('result:')}
      {#key page}<StressResult {backend} id={page.slice('result:'.length)} onOpen={open} />{/key}
    {/if}
  </section>
</div>

<style>
  .performance {
    display: grid;
    grid-template-columns: 180px minmax(0, 1fr);
    gap: 24px;
    align-items: start;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
    position: sticky;
    top: 76px;
  }
  .group {
    margin: 0 0 4px;
    padding: 0 12px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .entries {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .entries + .group {
    margin-top: 12px;
  }
  .entry {
    padding: 8px 12px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    border-radius: 0 8px 8px 0;
  }
  .entry:hover {
    background: var(--surface);
  }
  .entry.on {
    background: var(--surface-2);
    border-left-color: var(--accent);
  }
  /* A running test: the entry carries the accent and a faint glow, like a lit tube. */
  .entry.live {
    color: var(--accent);
    text-shadow: 0 0 10px color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .entry:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .content {
    display: flex;
    flex-direction: column;
    gap: 20px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 20px;
  }
  @media (max-width: 720px) {
    .performance {
      grid-template-columns: minmax(0, 1fr);
    }
    nav {
      position: static;
      flex-direction: row;
      flex-wrap: wrap;
      align-items: center;
    }
    .group {
      margin: 0 8px 0 0;
      padding: 0;
    }
    .entries {
      flex-direction: row;
      flex-wrap: wrap;
    }
    .entry {
      border-left: 0;
      border-bottom: 2px solid transparent;
      border-radius: 8px 8px 0 0;
    }
    .entry.on {
      border-bottom-color: var(--accent);
    }
    .entries + .group {
      margin: 0 8px 0 12px;
    }
  }
</style>
