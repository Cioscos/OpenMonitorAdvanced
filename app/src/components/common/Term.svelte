<script lang="ts" module>
  const GAP = 6;
  const MARGIN = 8;

  interface Box {
    left: number;
    top: number;
    width: number;
    height: number;
  }

  /**
   * Where the tooltip goes: centred under the term, above it when there is no room below (and more
   * room above), and always at least `MARGIN` px inside the window.
   */
  export function placeTip(
    anchor: Box,
    size: { width: number; height: number },
    viewport: { width: number; height: number },
  ): { left: number; top: number } {
    const below = anchor.top + anchor.height + GAP;
    const above = anchor.top - GAP - size.height;
    const fitsBelow = below + size.height <= viewport.height - MARGIN;
    const roomAbove = anchor.top - MARGIN;
    const roomBelow = viewport.height - MARGIN - (anchor.top + anchor.height);
    const top = fitsBelow || roomBelow >= roomAbove ? below : above;
    const clamp = (value: number, max: number) => Math.max(MARGIN, Math.min(value, max));
    return {
      left: clamp(anchor.left + anchor.width / 2 - size.width / 2, viewport.width - size.width - MARGIN),
      top: clamp(top, viewport.height - size.height - MARGIN),
    };
  }
</script>

<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import { t } from '../../lib/i18n/index.svelte';

  // A technical term explained in plain words (spec M8 §3.7): underlined with dots, reached with
  // Tab, its `glossary.<term>` text shown on hover and focus and closed with Esc. Do not put it
  // inside a button or a link: it is focusable itself.
  let { term, children }: { term: string; children?: Snippet } = $props();

  const uid = $props.id();
  const id = `term-${uid}`;
  const nameKey = $derived(`glossary.${term}.name`);
  const name = $derived(t(nameKey) === nameKey ? term : t(nameKey));

  let open = $state(false);
  let anchor = $state<HTMLElement | undefined>();
  let tip = $state<HTMLElement | undefined>();
  let position = $state({ left: 0, top: 0 });
  let closing: ReturnType<typeof setTimeout> | undefined;

  async function show() {
    clearTimeout(closing);
    if (open) return;
    open = true;
    await tick();
    place();
  }
  function hide() {
    clearTimeout(closing);
    open = false;
  }
  /** Leaving the term or the tooltip waits a moment, so the pointer can cross from one to the other. */
  function hideSoon() {
    clearTimeout(closing);
    closing = setTimeout(hide, 150);
  }
  function place() {
    if (!open || !anchor || !tip) return;
    const viewport = { width: document.documentElement.clientWidth || window.innerWidth, height: window.innerHeight };
    position = placeTip(anchor.getBoundingClientRect(), { width: tip.offsetWidth, height: tip.offsetHeight }, viewport);
  }
  // Esc closes the tooltip first, on the way down, so the settings screen behind it stays open.
  function onKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !open) return;
    event.preventDefault();
    hide();
  }

  $effect(() => () => clearTimeout(closing));

  // The tooltip lives under <body>: outside labels (whose text names a control), and away from
  // ancestors with a filter or transform, under which `position: fixed` stops following the window.
  // In the template it sits inside the term, never as a sibling: Svelte removes a block by walking
  // from its first to its last node, and a last node moved under <body> would make that walk
  // swallow everything after the block.
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
</script>

<svelte:window onkeydowncapture={onKeydown} onresize={place} onscrollcapture={place} />

<!-- A focusable span is the pattern of the spec (§3.7): Tab reaches the term so its tooltip can be read. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_static_element_interactions -->
<span
  class="term"
  tabindex="0"
  aria-describedby={id}
  bind:this={anchor}
  onmouseenter={show}
  onmouseleave={hideSoon}
  onfocus={show}
  onblur={hide}
>
  {#if children}{@render children()}{:else}{name}{/if}<span
    class="tip"
    role="tooltip"
    {id}
    hidden={!open}
    bind:this={tip}
    use:portal
    style:left="{position.left}px"
    style:top="{position.top}px"
    onmouseenter={show}
    onmouseleave={hideSoon}>{t(`glossary.${term}`)}</span>
</span>

<style>
  .term {
    cursor: help;
    text-decoration: underline dotted color-mix(in srgb, var(--accent-2) 70%, transparent);
    text-decoration-thickness: 1.5px;
    text-underline-offset: 3px;
    border-radius: 3px;
  }
  .term:hover {
    text-decoration-color: var(--accent-2);
  }
  .term:focus-visible {
    outline: 2px solid var(--accent-2);
    outline-offset: 2px;
  }
  .tip {
    position: fixed;
    z-index: 20;
    width: max-content;
    max-width: min(300px, calc(100vw - 16px));
    padding: 8px 12px;
    font-size: 12.5px;
    font-weight: 400;
    line-height: 1.5;
    letter-spacing: normal;
    text-align: left;
    text-transform: none;
    white-space: normal;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid color-mix(in srgb, var(--accent-2) 35%, var(--border));
    border-left: 2px solid var(--accent-2);
    border-radius: 8px;
    box-shadow:
      0 8px 24px color-mix(in srgb, var(--bg) 70%, transparent),
      0 0 12px color-mix(in srgb, var(--accent-2) 12%, transparent);
    user-select: text;
  }
  .tip[hidden] {
    display: none;
  }
</style>
