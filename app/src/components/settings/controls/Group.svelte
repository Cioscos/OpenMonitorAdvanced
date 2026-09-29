<script lang="ts">
  import type { Snippet } from 'svelte';

  // A titled block of settings rows. The title runs into a thin neon horizon line.
  let { id, title, children }: { id: string; title: string; children: Snippet } = $props();
</script>

<section class="group" aria-labelledby="{id}-title">
  <h3 id="{id}-title">{title}</h3>
  <div class="rows">{@render children()}</div>
</section>

<style>
  .group {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  h3 {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
  }
  h3::after {
    content: '';
    flex: 1;
    height: 1px;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--accent) 70%, transparent),
      color-mix(in srgb, var(--accent-2) 45%, transparent) 40%,
      transparent
    );
  }
  .rows {
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .rows > :global(* + *) {
    border-top: 1px solid var(--border);
  }
</style>
