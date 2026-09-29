<script lang="ts" module>
  export interface SegmentOption<V> {
    value: V;
    label: string;
    /** Shown under the choices and read with this option only. */
    note?: string;
  }
</script>

<script lang="ts" generics="T extends string | number">
  import Field from './Field.svelte';

  // One choice among a few, as native radio buttons (arrow keys move between them). Each choice
  // is sent at once; when the change is refused the buttons go back to the value in effect.
  let {
    id,
    label,
    description = null,
    options,
    value,
    disabled = false,
    error = null,
    onChange,
  }: {
    id: string;
    label: string;
    description?: string | null;
    options: SegmentOption<T>[];
    value: T;
    disabled?: boolean;
    error?: string | null;
    /** A promise resolving to `false` means the value was not taken. */
    onChange: (next: T) => unknown;
  } = $props();

  let group: HTMLDivElement | undefined = $state();

  const noteId = (index: number) => `${id}-note-${index}`;
  const describedBy = (index: number) =>
    [description ? `${id}-desc` : null, options[index].note ? noteId(index) : null, error !== null ? `${id}-error` : null]
      .filter(Boolean)
      .join(' ') || undefined;

  async function pick(next: T) {
    const taken = await onChange(next);
    if (taken === false && group) {
      for (const input of group.querySelectorAll('input')) input.checked = input.value === String(value);
    }
  }
</script>

<Field {id} {label} {description} {error}>
  {#snippet control()}
    <div
      class="segmented"
      role="radiogroup"
      aria-labelledby="{id}-label"
      aria-invalid={error !== null ? 'true' : undefined}
      bind:this={group}
    >
      {#each options as option, index (option.value)}
        <label class="segment">
          <input
            type="radio"
            name={id}
            value={String(option.value)}
            checked={option.value === value}
            {disabled}
            aria-describedby={describedBy(index)}
            onchange={() => pick(option.value)}
          />
          <span>{option.label}</span>
        </label>
      {/each}
    </div>
    {#each options as option, index (option.value)}
      {#if option.note}<p class="note" id={noteId(index)}>{option.note}</p>{/if}
    {/each}
  {/snippet}
</Field>

<style>
  .segmented {
    display: inline-flex;
    flex-wrap: wrap;
    padding: 3px;
    border-radius: 10px;
    background: var(--surface-2);
  }
  .segment {
    position: relative;
  }
  input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: pointer;
  }
  input:disabled {
    cursor: not-allowed;
  }
  span {
    display: block;
    padding: 5px 12px;
    border-radius: 8px;
    font-size: 13px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  input:checked + span {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
  }
  input:not(:checked):not(:disabled):hover + span {
    color: var(--text);
  }
  input:focus-visible + span {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  input:disabled + span {
    opacity: 0.45;
  }
  .note {
    max-width: 320px;
    margin: 0;
    font-size: 12px;
    line-height: 1.4;
    color: var(--text-muted);
    text-align: right;
  }
</style>
