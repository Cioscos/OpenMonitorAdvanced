<script lang="ts">
  import { formatTapeCounter } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { around, eventText } from '../../lib/performance/format';
  import type { SessionEvent } from '../../lib/types';
  import Term from '../common/Term.svelte';

  // The session's event log (spec M8 §3.5), newest first: the time since the start and the event
  // in words, with its technical term explained.
  let { events }: { events: SessionEvent[] } = $props();

  const lines = $derived(
    events
      .map((event) => {
        const { text, term } = eventText(event, t, i18n.locale);
        return { at: formatTapeCounter(event.atMs), term, parts: term ? around(text, t(`glossary.${term}.name`)) : ([text, '', ''] as const) };
      })
      .reverse(),
  );
</script>

{#if lines.length === 0}
  <p class="empty">{t('performance.run.noEvents')}</p>
{:else}
  <ol class="log" aria-label={t('performance.run.events')}>
    {#each lines as line, index (index)}
      <li><time>{line.at}</time> <span>{line.parts[0]}{#if line.parts[1] && line.term}<Term term={line.term}>{line.parts[1]}</Term>{/if}{line.parts[2]}</span></li>
    {/each}
  </ol>
{/if}

<style>
  .log {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 13px;
    line-height: 1.6;
    color: var(--text-muted);
  }
  time {
    color: var(--text);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
  .empty {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
</style>
