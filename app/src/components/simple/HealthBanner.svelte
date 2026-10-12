<script lang="ts">
  import { sinceText } from '../../lib/health.svelte';
  import { t } from '../../lib/i18n/index.svelte';
  import type { HealthLevel } from '../../lib/types';

  let {
    level,
    title,
    items = [],
    elapsedMs,
    compact = false,
  }: {
    level: HealthLevel;
    title: string;
    /** One row per problem, worst first; empty unless several alerts are active. */
    items?: { text: string; level: 'ok' | 'warn' | 'crit' }[];
    /** Time in the current level, from the core's monotonic clock. */
    elapsedMs: number;
    /** Scrolled away with a problem active: the title line only, details hidden. */
    compact?: boolean;
  } = $props();

  const listId = 'health-problems';
  let open = $state(false);
  let toggle = $state<HTMLButtonElement | undefined>();
  // The list belongs to the problems it shows: it closes when they go away.
  const expandable = $derived(items.length > 0);
  const expanded = $derived(open && expandable && !compact);

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
  class:compact
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
          {title}
          aria-expanded={expanded}
          aria-controls={expanded ? listId : undefined}
          onclick={() => (open = !open)}
        >
          <span class="label">{title}</span><span class="chevron" class:flipped={expanded} aria-hidden="true">▾</span>
        </button>
      {:else}
        <div class="title" {title}>{title}</div>
      {/if}
      {#if !compact}<div class="sub">{sinceText(elapsedMs, t)}</div>{/if}
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
  /* A strip as wide as the window: the dot and the title (with the time under it), the list drops down over the page. */
  .banner {
    --state: var(--text-muted);
    position: relative;
    display: flex;
    align-items: center;
    height: var(--strip-h);
    pointer-events: auto;
    padding: 0 20px;
    border-bottom: 1px solid var(--border);
    background: linear-gradient(90deg, color-mix(in srgb, var(--state) 14%, var(--bg)), var(--bg) 70%);
    backdrop-filter: blur(8px);
  }
  .banner.compact {
    height: var(--strip-compact-h);
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
    gap: 10px;
    min-width: 0;
  }
  .text {
    min-width: 0;
    line-height: 1.15;
  }
  .dot {
    display: grid;
    flex: none;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    font-size: 14px;
    color: var(--state);
    background: color-mix(in srgb, var(--state) 18%, transparent);
  }
  .compact .dot {
    width: 18px;
    height: 18px;
    font-size: 12px;
  }
  .title {
    overflow: hidden;
    font-size: 14px;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    max-width: 100%;
    gap: 8px;
    padding: 1px 8px;
    margin: -1px -8px;
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
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .chevron {
    flex: none;
    font-size: 12px;
    color: var(--state);
  }
  .chevron.flipped {
    display: inline-block;
    transform: rotate(180deg);
  }
  .sub {
    font-size: 11px;
    color: var(--text-muted);
  }
  .problems {
    position: absolute;
    top: 100%;
    left: 20px;
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
    width: min(560px, calc(100% - 40px));
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--border);
    box-shadow: 0 8px 24px var(--shadow);
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
