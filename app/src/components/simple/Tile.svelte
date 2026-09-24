<script lang="ts">
  import type { Snippet } from 'svelte';

  let { label, onclick, children }: { label: string; onclick?: () => void; children: Snippet } = $props();
</script>

<button class="tile" type="button" {onclick}>
  <div class="label">{label}</div>
  {@render children()}
</button>

<style>
  .tile {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    padding: 14px;
    text-align: left;
    cursor: pointer;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    transition: border-color 0.15s;
  }
  .tile:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .tile:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
