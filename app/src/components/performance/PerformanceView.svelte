<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend';
  import { t } from '../../lib/i18n/index.svelte';
  import { performanceStore } from '../../lib/performance/performance.svelte';
  import type { PerformancePage } from '../../lib/view';

  // The Performance view (spec M8 §3.1): the sidebar with the «Stress test» group on the left, the
  // page on the right. The store is connected only while the view is on screen. The pages
  // themselves arrive with the wizard (new), the run and result screens (run, result) and the
  // history (history).
  let { backend, page = $bindable('new') }: { backend: Backend; page?: PerformancePage } = $props();

  onMount(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    performanceStore
      .connect(backend)
      .then((unsubscribe) => {
        if (cancelled) unsubscribe();
        else off = unsubscribe;
      })
      .catch((error) => console.error('stress test status unavailable', error));
    return () => {
      cancelled = true;
      off?.();
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

  const current = $derived(page === 'run' || page === 'new' ? 'test' : 'history');
  const title = $derived(
    page === 'new'
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
  }
</style>
