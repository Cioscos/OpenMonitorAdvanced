<script lang="ts">
  import { onDestroy } from 'svelte';
  import { formatTapeCounter } from '../../lib/format';
  import { t } from '../../lib/i18n/index.svelte';
  import { createBlink } from '../../lib/log/blink.svelte';
  import { logErrorText } from '../../lib/log/messages';
  import { log } from '../../lib/log.svelte';
  import Deck from './Deck.svelte';

  let { openFolder }: { openFolder: () => Promise<void> } = $props();

  const deckId = 'log-deck';
  let open = $state(false);
  let root = $state<HTMLDivElement | undefined>();
  let button = $state<HTMLButtonElement | undefined>();

  const logState = $derived(log.status?.state ?? 'idle');
  const time = $derived(formatTapeCounter(log.status?.recordedMs ?? 0));
  const label = $derived(
    logState === 'error'
      ? t('log.recorder.error', { reason: logErrorText(log.status?.error, t) })
      : t(`log.recorder.${logState}`, { time }),
  );

  // The dot's only animation; the top bar stays mounted while the window is hidden, and the
  // blink watches `document.hidden` itself.
  const blink = createBlink();
  $effect(() => blink.setActive(logState === 'recording'));
  onDestroy(() => blink.destroy());

  // A press anywhere outside the recorder closes the deck; the focus stays where the user put it.
  $effect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (root && !root.contains(event.target as Node)) open = false;
    };
    document.addEventListener('pointerdown', outside, true);
    return () => document.removeEventListener('pointerdown', outside, true);
  });

  // Esc is handled here, before App's window handler, and marked so the settings stay open.
  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && open) {
      event.preventDefault();
      open = false;
      button?.focus();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="recorder" bind:this={root} onkeydown={onKeydown}>
  <button
    bind:this={button}
    type="button"
    class="tape"
    data-state={logState}
    aria-label={label}
    title={label}
    aria-expanded={open}
    aria-controls={deckId}
    onclick={() => (open = !open)}
  >
    {#if logState === 'recording'}
      <span class="rec-dot" class:on={blink.on} aria-hidden="true"></span>
      <span class="counter" aria-hidden="true">{time}</span>
    {:else if logState === 'paused'}
      <svg class="pause-icon" viewBox="0 0 12 12" width="12" height="12" aria-hidden="true" focusable="false">
        <rect x="2" y="1.5" width="2.8" height="9" rx="0.5" /><rect x="7.2" y="1.5" width="2.8" height="9" rx="0.5" />
      </svg>
      <span class="counter" aria-hidden="true">{time}</span>
    {:else if logState === 'error'}
      <svg class="error-icon" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false">
        <path d="M8 1.8L15 14.2H1Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" />
        <path d="M8 6.2V9.6M8 11.4V11.9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
      </svg>
    {:else}
      <!-- A cassette: the shell, its two reel holes and the head opening at the bottom edge. -->
      <svg class="cassette-icon" viewBox="0 0 22 16" width="22" height="16" fill="none" stroke="currentColor" aria-hidden="true" focusable="false">
        <rect x="1" y="1.5" width="20" height="13" rx="2" stroke-width="1.5" />
        <circle cx="7" cy="7" r="2" stroke-width="1.4" />
        <circle cx="15" cy="7" r="2" stroke-width="1.4" />
        <path d="M5.5 14.5L7 11.5H15L16.5 14.5" stroke-width="1.4" stroke-linejoin="round" />
      </svg>
    {/if}
  </button>
  {#if open}
    <Deck id={deckId} {openFolder} />
  {/if}
</div>

<style>
  .recorder {
    position: relative;
  }
  .tape {
    --state: var(--border);
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    min-width: 32px;
    padding: 0 9px;
    color: var(--text-muted);
    cursor: pointer;
    border: 1px solid var(--state);
    border-radius: 8px;
    background: var(--surface);
  }
  .tape:hover {
    color: var(--text);
  }
  .tape[aria-expanded='true'] {
    background: var(--surface-2);
  }
  .tape[data-state='recording'],
  .tape[data-state='error'] {
    --state: color-mix(in srgb, var(--crit) 55%, var(--border));
  }
  .tape[data-state='paused'] {
    --state: color-mix(in srgb, var(--warn) 55%, var(--border));
  }
  .tape:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .rec-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--crit) 22%, transparent);
  }
  .rec-dot.on {
    background: var(--crit);
  }
  .pause-icon {
    fill: var(--warn);
  }
  .error-icon {
    color: var(--crit);
  }
  .counter {
    font-family: 'Cascadia Mono', Consolas, monospace;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .tape[data-state='paused'] .counter {
    color: var(--text-muted);
  }
</style>
