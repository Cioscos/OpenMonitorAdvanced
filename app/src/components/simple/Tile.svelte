<script lang="ts">
  import type { Snippet } from 'svelte';

  let { label, onclick, span, children }: { label: string; onclick?: () => void; span?: number; children: Snippet } = $props();
</script>

<!-- The label button's ::after covers the whole tile, so a click anywhere opens it while content
     with its own focusable parts (tooltips) lives outside the button. -->
<div class="tile" style:--span={span}>
  <button class="open" type="button" {onclick}>{label}</button>
  {@render children()}
</div>

<style>
  .tile {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    min-height: 0;
    padding: 14px;
    text-align: left;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    transition: border-color 0.15s;
  }
  .tile:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .open {
    align-self: flex-start;
    padding: 0;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
    background: none;
    border: 0;
  }
  .open::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: var(--radius);
  }
  .open:focus-visible {
    outline: none;
  }
  .open:focus-visible::after {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
