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
    onCapture,
  }: {
    id: string;
    label: string;
    value: string | null;
    /** What became of the registration; null while the log status is not known yet. */
    status: HotkeyStatus | null;
    /** Already translated. */
    error?: string | null;
    onChange: (next: string | null) => unknown;
    /** True while the box has focus, so the global hotkeys can step aside; false when it loses it or goes away focused. */
    onCapture: (capturing: boolean) => unknown;
  } = $props();

  let listening = $state(false);
  let box: HTMLInputElement | undefined = $state();

  /** The shell resumes the hotkeys when the window loses focus, and the page may see no blur. */
  function onWindowFocus() {
    if (box !== undefined && document.activeElement === box) {
      listening = true;
      void onCapture(true);
    }
  }

  function setListening(next: boolean) {
    if (next === listening) return;
    listening = next;
    void onCapture(next);
  }

  $effect(() => () => {
    // Removed while focused: no blur follows, so resume here.
    if (listening) void onCapture(false);
  });
  let refused = $state(false);
  const shownError = $derived(refused ? t('settings.error.hotkey') : error);

  function onKeydown(event: KeyboardEvent) {
    const key = readHotkeyKey(event);
    if (key.kind === 'ignore') return;
    event.preventDefault();
    const target = event.currentTarget;
    // A held key repeats its keydown: only the first press counts (Esc still gives up).
    if (event.repeat && key.kind !== 'cancel') return;
    if (key.kind === 'cancel') {
      // Only the capture goes; the settings screen keeps Escape for leaving when nothing is captured.
      event.stopPropagation();
      refused = false;
      target instanceof HTMLElement && target.blur();
    } else if (key.kind === 'clear') {
      refused = false;
      void onChange(null);
      // Leave the box so it shows the saved value instead of the listening text.
      target instanceof HTMLElement && target.blur();
    } else if (key.kind === 'refused') {
      refused = true;
    } else {
      refused = false;
      void onChange(key.hotkey);
      target instanceof HTMLElement && target.blur();
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

<svelte:window onfocus={onWindowFocus} />

<Field {id} {label} labelFor={id} error={shownError}>
  {#snippet control()}
    <input
      bind:this={box}
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
      onfocus={() => setListening(true)}
      onblur={() => {
        setListening(false);
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
