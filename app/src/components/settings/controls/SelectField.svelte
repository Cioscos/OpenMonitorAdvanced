<script lang="ts" module>
  export interface SelectOption {
    value: string;
    label: string;
  }
  export interface SelectGroup {
    group: string;
    options: SelectOption[];
  }
</script>

<script lang="ts">
  import Field from './Field.svelte';

  // A choice from a longer list, as a native <select>. The choice is sent at once; when it is
  // refused the list goes back to the value in effect.
  let {
    id,
    label,
    description = null,
    items,
    value,
    disabled = false,
    error = null,
    onChange,
  }: {
    id: string;
    label: string;
    description?: string | null;
    items: (SelectOption | SelectGroup)[];
    value: string;
    disabled?: boolean;
    error?: string | null;
    /** A promise resolving to `false` means the value was not taken. */
    onChange: (next: string) => unknown;
  } = $props();

  const describedBy = $derived(
    [description ? `${id}-desc` : null, error !== null ? `${id}-error` : null].filter(Boolean).join(' ') || undefined,
  );

  async function pick(select: HTMLSelectElement) {
    const taken = await onChange(select.value);
    if (taken === false) select.value = value;
  }
</script>

<Field {id} {label} labelFor="{id}-input" {description} {error}>
  {#snippet control()}
    <select
      id="{id}-input"
      {value}
      {disabled}
      aria-describedby={describedBy}
      aria-invalid={error !== null ? 'true' : undefined}
      onchange={(event) => pick(event.currentTarget)}
    >
      {#each items as item ('group' in item ? `group:${item.group}` : item.value)}
        {#if 'group' in item}
          <optgroup label={item.group}>
            {#each item.options as option (option.value)}
              <option value={option.value}>{option.label}</option>
            {/each}
          </optgroup>
        {:else}
          <option value={item.value}>{item.label}</option>
        {/if}
      {/each}
    </select>
  {/snippet}
</Field>

<style>
  select {
    min-width: 160px;
    max-width: min(320px, 100%);
    padding: 6px 30px 6px 10px;
    font: inherit;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
    background:
      linear-gradient(45deg, transparent 50%, var(--text-muted) 50%) calc(100% - 15px) 55% / 5px 5px no-repeat,
      linear-gradient(135deg, var(--text-muted) 50%, transparent 50%) calc(100% - 10px) 55% / 5px 5px no-repeat,
      var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    appearance: none;
  }
  select:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  select:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  optgroup,
  option {
    color: var(--text);
    background: var(--surface);
  }
</style>
