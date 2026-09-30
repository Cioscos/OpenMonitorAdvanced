<script lang="ts">
  import { sinceText } from '../../lib/health.svelte';
  import { t } from '../../lib/i18n/index.svelte';
  import type { HealthLevel } from '../../lib/types';

  let {
    level,
    title,
    items = [],
    elapsedMs,
  }: {
    level: HealthLevel;
    title: string;
    /** One row per problem, worst first; empty unless several alerts are active. */
    items?: { text: string; level: 'ok' | 'warn' | 'crit' }[];
    /** Time in the current level, from the core's monotonic clock. */
    elapsedMs: number;
  } = $props();

  const listId = 'health-problems';
  let open = $state(false);
  let toggle = $state<HTMLButtonElement | undefined>();
  // The list belongs to the problems it shows: it closes when they go away.
  const expandable = $derived(items.length > 0);
  const expanded = $derived(open && expandable);

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && expanded) {
      event.preventDefault();
      open = false;
      toggle?.focus();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  class="banner"
  class:ok={level === 'ok'}
  class:warn={level === 'warn'}
  class:crit={level === 'crit'}
  role="status"
  onkeydown={onKeydown}
>
  <div class="head">
    <div class="dot" aria-hidden="true">{level === 'neutral' ? '•' : level === 'ok' ? '✓' : '!'}</div>
    <div class="text">
      {#if expandable}
        <button
          bind:this={toggle}
          type="button"
          class="title toggle"
          aria-expanded={expanded}
          aria-controls={expanded ? listId : undefined}
          onclick={() => (open = !open)}
        >
          {title}<span class="chevron" class:flipped={expanded} aria-hidden="true">▾</span>
        </button>
      {:else}
        <div class="title">{title}</div>
      {/if}
      <div class="sub">{sinceText(elapsedMs, t)}</div>
    </div>
  </div>
  {#if expanded}
    <ul id={listId} class="problems">
      {#each items as item, i (i)}
        <li class={item.level}><span class="pip" aria-hidden="true"></span>{item.text}</li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .banner {
    --state: var(--text-muted);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 18px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: linear-gradient(135deg, color-mix(in srgb, var(--state) 14%, var(--surface)), var(--surface));
  }
  .ok { --state: var(--ok); }
  .warn {
    --state: var(--warn);
  }
  .crit {
    --state: var(--crit);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .text {
    min-width: 0;
  }
  .dot {
    display: grid;
    flex: none;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    font-size: 20px;
    color: var(--state);
    background: color-mix(in srgb, var(--state) 18%, transparent);
  }
  .title {
    font-size: 20px;
    font-weight: 600;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 2px 8px;
    margin: -2px -8px;
    border: 1px solid transparent;
    border-radius: 8px;
    background: none;
    text-align: left;
    cursor: pointer;
  }
  .toggle:hover {
    border-color: color-mix(in srgb, var(--state) 45%, var(--border));
  }
  .toggle:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .chevron {
    flex: none;
    font-size: 14px;
    color: var(--state);
  }
  .chevron.flipped {
    display: inline-block;
    transform: rotate(180deg);
  }
  .sub {
    font-size: 12px;
    color: var(--text-muted);
  }
  .problems {
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--border);
  }
  .problems li {
    --row: var(--text-muted);
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 9px 12px;
    font-size: 14px;
    background: var(--surface-2);
  }
  .problems li.warn {
    --row: var(--warn);
  }
  .problems li.crit {
    --row: var(--crit);
  }
  .pip {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--row);
  }
</style>
