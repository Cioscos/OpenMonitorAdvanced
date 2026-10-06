<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import type { CoreState } from '../../lib/types';

  // The cores of a «one core at a time» test (spec M8 §3.5): one cell per core with its state, the
  // core under test lit. `final` is the result page, where an untested core will not be tested.
  // The parent's title carries the terms (the numbering), so the cells stay plain text.
  let {
    cores,
    current = null,
    final = false,
    notes = {},
  }: {
    cores: { core: number; state: CoreState }[];
    current?: number | null;
    final?: boolean;
    /** Extra text after a core's state, e.g. when its first error came. */
    notes?: Record<number, string>;
  } = $props();

  const stateText = (state: CoreState) => t(`performance.core.${state === 'untested' && final ? 'untestedFinal' : state}`);
</script>

<ul class="cores" aria-label={t('performance.run.cores')}>
  {#each cores as c (c.core)}
    <li class={c.state} class:now={c.core === current && c.state === 'testing'} aria-current={c.core === current && c.state === 'testing' ? 'true' : undefined}>
      <b>{t('performance.core.label', { core: c.core })}</b><span>{stateText(c.state)}{#if notes[c.core]} · {notes[c.core]}{/if}</span>
    </li>
  {/each}
</ul>

<style>
  .cores {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(92px, 1fr));
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  b {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  span {
    font-size: 11px;
    color: var(--text-muted);
  }
  .passed {
    border-color: color-mix(in srgb, var(--ok) 50%, transparent);
  }
  .passed span {
    color: var(--ok);
  }
  .failed {
    background: color-mix(in srgb, var(--crit) 8%, var(--surface-2));
    border-color: var(--crit);
  }
  .failed span {
    color: var(--crit);
  }
  /* The core under test glows like a lit tube. */
  .now {
    border-color: var(--accent);
    box-shadow: 0 0 10px color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .now span {
    color: var(--accent);
  }
</style>
