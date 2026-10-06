<script lang="ts">
  import type { Snippet } from 'svelte';
  import Field from './Field.svelte';

  // An on/off setting. The switch shows the state in effect (`checked`), never a guess: a change
  // becomes visible when the new state arrives.
  let {
    id,
    label,
    term = null,
    description = null,
    checked,
    disabled = false,
    error = null,
    notesIds = [],
    onChange,
    notes,
  }: {
    id: string;
    label: string;
    /** A glossary term the label explains with a tooltip. */
    term?: string | null;
    description?: string | null;
    checked: boolean;
    disabled?: boolean;
    error?: string | null;
    /** Ids of the elements in `notes` that describe the switch. */
    notesIds?: string[];
    onChange: (next: boolean) => unknown;
    notes?: Snippet;
  } = $props();

  const describedBy = $derived(
    [description ? `${id}-desc` : null, ...notesIds, error !== null ? `${id}-error` : null].filter(Boolean).join(' ') || undefined,
  );
</script>

<Field {id} {label} {term} {description} {error} {notes}>
  {#snippet control()}
    <button
      type="button"
      role="switch"
      class="switch"
      aria-checked={checked}
      aria-labelledby="{id}-label"
      aria-describedby={describedBy}
      aria-invalid={error !== null ? 'true' : undefined}
      {disabled}
      onclick={() => onChange(!checked)}
    >
      <span class="knob" aria-hidden="true"></span>
    </button>
  {/snippet}
</Field>

<style>
  .switch {
    position: relative;
    flex: none;
    width: 42px;
    height: 24px;
    padding: 0;
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    transition:
      background-color 0.15s,
      border-color 0.15s,
      box-shadow 0.15s;
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--text-muted);
    transition:
      transform 0.15s,
      background-color 0.15s;
  }
  .switch[aria-checked='true'] {
    background: var(--accent);
    border-color: var(--accent);
    box-shadow: 0 0 10px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .switch[aria-checked='true'] .knob {
    transform: translateX(18px);
    background: var(--on-accent);
  }
  .switch:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .switch:disabled {
    cursor: not-allowed;
    opacity: 0.45;
    box-shadow: none;
  }
</style>
