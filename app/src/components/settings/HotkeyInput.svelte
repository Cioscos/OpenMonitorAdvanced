<script lang="ts">
  import { catalogs, t } from '../../lib/i18n/index.svelte';
  import { readHotkeyKey } from '../../lib/log/hotkey';
  import type { HotkeyStatus } from '../../lib/types';
  import Field from './controls/Field.svelte';

  // A box that records the combination pressed in it. Esc gives up, Delete or Backspace clears it,
  // and a combination the core would refuse (fewer than two of Ctrl, Alt and Shift) is never sent.
  let {
    id,
    label,
    value,
    status,
    error = null,
    onChange,
  }: {
    id: string;
    label: string;
    value: string | null;
    /** What became of the registration; null while the log status is not known yet. */
    status: HotkeyStatus | null;
    /** Already translated. */
    error?: string | null;
    onChange: (next: string | null) => unknown;
  } = $props();

  let listening = $state(false);
  let refused = $state(false);
  const shownError = $derived(refused ? t('settings.error.hotkey') : error);

  function onKeydown(event: KeyboardEvent) {
    const key = readHotkeyKey(event);
    if (key.kind === 'ignore') return;
    event.preventDefault();
    if (key.kind === 'cancel') {
      // Only the capture goes; the settings screen keeps Escape for leaving when nothing is captured.
      event.stopPropagation();
      refused = false;
      event.currentTarget instanceof HTMLElement && event.currentTarget.blur();
    } else if (key.kind === 'clear') {
      refused = false;
      void onChange(null);
    } else if (key.kind === 'refused') {
      refused = true;
    } else {
      refused = false;
      void onChange(key.hotkey);
    }
  }

  const statusText = $derived.by(() => {
    if (status === null) return null;
    if (status.state === 'active') return t('settings.log.hotkey.active');
    if (status.state === 'unset') return t('settings.log.hotkey.unset');
    const reason = status.reason !== null && status.reason in catalogs.en ? t(status.reason) : t('log.hotkey.failed');
    return status.effective !== null ? `${reason}. ${t('settings.log.hotkey.keepsActive', { hotkey: status.effective })}` : reason;
  });
</script>

<Field {id} {label} labelFor={id} error={shownError}>
  {#snippet control()}
    <input
      {id}
      type="text"
      readonly
      class="box"
      class:listening
      autocomplete="off"
      spellcheck="false"
      value={listening ? t('settings.log.hotkey.listening') : (value ?? '')}
      placeholder={t('settings.log.hotkey.placeholder')}
      aria-invalid={shownError !== null ? 'true' : undefined}
      aria-describedby={[statusText !== null ? `${id}-status` : null, shownError !== null ? `${id}-error` : null].filter(Boolean).join(' ') || undefined}
      onfocus={() => (listening = true)}
      onblur={() => {
        listening = false;
        refused = false;
      }}
      onkeydown={onKeydown}
    />
  {/snippet}
  {#snippet notes()}
    {#if statusText !== null}
      <p id="{id}-status" class="status" class:failed={status?.state === 'failed'} class:active={status?.state === 'active'}>{statusText}</p>
    {/if}
  {/snippet}
</Field>

<style>
  .box {
    width: 220px;
    max-width: 100%;
    padding: 6px 10px;
    font: inherit;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    text-align: center;
    color: var(--text);
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .box::placeholder {
    color: var(--text-muted);
  }
  .box:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .box:focus-visible,
  .box.listening {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    color: var(--accent-2);
  }
  .box[aria-invalid='true'] {
    border-color: var(--crit);
  }
  .status::before {
    content: '';
    display: inline-block;
    width: 7px;
    height: 7px;
    margin-right: 8px;
    border-radius: 50%;
    background: var(--text-muted);
  }
  .status.active::before {
    background: var(--ok, var(--accent-2));
  }
  .status.failed {
    color: var(--warn);
  }
  .status.failed::before {
    background: var(--warn);
  }
</style>
