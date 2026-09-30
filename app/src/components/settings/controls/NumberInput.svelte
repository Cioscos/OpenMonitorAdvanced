<script lang="ts">
  import { i18n } from '../../../lib/i18n/index.svelte';
  import { formatNumber, parseNumber } from '../../../lib/rules';

  // A number typed as text. While typing the text is only a local draft: an empty field, a lone sign
  // or a separator without digits is never sent. Enter or leaving the field commits a complete number;
  // anything else goes back to the value in effect, and so does Escape. A refused number stays in the
  // field so it can be corrected next to its error.
  let {
    id,
    value,
    integer = false,
    unit = null,
    placeholder = null,
    invalid = false,
    describedBy = undefined,
    disabled = false,
    onCommit,
  }: {
    id: string;
    /** In the unit shown; NaN shows an empty field. */
    value: number;
    integer?: boolean;
    /** Shown after the field. */
    unit?: string | null;
    placeholder?: string | null;
    invalid?: boolean;
    describedBy?: string;
    disabled?: boolean;
    /** A promise resolving to `false` means the number was refused. */
    onCommit: (next: number) => unknown;
  } = $props();

  const shown = $derived(formatNumber(value, i18n.locale));
  /** The text being typed; null while the field shows the value in effect. */
  let draft = $state<string | null>(null);
  const described = $derived([describedBy, unit ? `${id}-unit` : undefined].filter(Boolean).join(' ') || undefined);

  async function commit() {
    if (draft === null) return;
    const typed = draft;
    draft = null;
    const next = parseNumber(typed, integer);
    if (next === null || (Number.isFinite(value) && formatNumber(next, i18n.locale) === shown)) return;
    const taken = await onCommit(next);
    if (taken === false && draft === null) draft = typed;
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      void commit();
    } else if (event.key === 'Escape' && draft !== null) {
      // Only the draft goes; the settings screen keeps Escape for leaving when nothing is typed.
      event.stopPropagation();
      draft = null;
    }
  }
</script>

<span class="number">
  <input
    {id}
    type="text"
    inputmode={integer ? 'numeric' : 'decimal'}
    autocomplete="off"
    spellcheck="false"
    value={draft ?? shown}
    placeholder={placeholder ?? undefined}
    {disabled}
    aria-invalid={invalid ? 'true' : undefined}
    aria-describedby={described}
    oninput={(event) => (draft = event.currentTarget.value)}
    onkeydown={onKeydown}
    onblur={() => void commit()}
  />
  {#if unit}<span class="unit" id="{id}-unit">{unit}</span>{/if}
</span>

<style>
  .number {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  input {
    width: 104px;
    padding: 6px 10px;
    font: inherit;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    text-align: right;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  input::placeholder {
    color: var(--text-muted);
    opacity: 0.8;
  }
  input:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  input[aria-invalid='true'] {
    border-color: var(--crit);
  }
  input:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .unit {
    min-width: 3.5em;
    font-size: 12.5px;
    color: var(--text-muted);
  }
</style>
