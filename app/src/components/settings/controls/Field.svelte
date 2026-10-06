<script lang="ts">
  import type { Snippet } from 'svelte';
  import Term from '../../common/Term.svelte';

  // One settings row: the name and a short description on the left, the control on the right,
  // and under both the notes and the error of the field. Controls point `aria-describedby` at
  // `describedBy(id, …)` so a screen reader reads the description, notes and error with them.
  let {
    id,
    label,
    labelFor = null,
    term = null,
    description = null,
    error = null,
    control,
    notes,
  }: {
    /** Base of the ids of the row: `<id>-label`, `<id>-desc`, `<id>-error`. */
    id: string;
    label: string;
    /** The id of a native control the label names (`<label for>`); otherwise the label is a plain name. */
    labelFor?: string | null;
    /** A glossary term the label explains with a tooltip (`Term`). */
    term?: string | null;
    description?: string | null;
    /** Already translated. */
    error?: string | null;
    control: Snippet;
    notes?: Snippet;
  } = $props();
</script>

{#snippet name()}
  {#if term}<Term {term}>{label}</Term>{:else}{label}{/if}
{/snippet}

<div class="field" class:invalid={error !== null}>
  <div class="text">
    {#if labelFor}
      <label id="{id}-label" for={labelFor}>{@render name()}</label>
    {:else}
      <span class="name" id="{id}-label">{@render name()}</span>
    {/if}
    {#if description}<p class="description" id="{id}-desc">{description}</p>{/if}
  </div>
  <div class="control">{@render control()}</div>
  {#if notes}<div class="notes">{@render notes()}</div>{/if}
  {#if error !== null}<p class="error" id="{id}-error" role="alert">{error}</p>{/if}
</div>

<style>
  .field {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 6px 24px;
    padding: 12px 16px;
  }
  .text {
    flex: 1 1 220px;
    min-width: 0;
  }
  label,
  .name {
    font-weight: 600;
  }
  .description {
    margin: 2px 0 0;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .control {
    display: flex;
    flex: 0 1 auto;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
    max-width: 100%;
  }
  .notes {
    display: flex;
    flex-basis: 100%;
    flex-direction: column;
    gap: 6px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .notes :global(p) {
    margin: 0;
  }
  .error {
    flex-basis: 100%;
    margin: 0;
    font-size: 12.5px;
    color: var(--crit);
  }
</style>
