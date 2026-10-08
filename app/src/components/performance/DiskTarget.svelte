<script lang="ts">
  import type { Backend } from '../../lib/backend';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { diskErrorText, formatBytes } from '../../lib/performance/disk';
  import type { VolumeChoice } from '../../lib/types';

  // «Where to test» (M8c DC6, DC15), shared by the disk score page and the stress wizard: the menu
  // of the local volumes with the disk's model and kind, «Choose folder…» for any other place, the
  // space left and the warnings that do not stop a test. `asking` shows the consent to spin up a
  // spun-down HDD: the page that tried to start sets it and retries with `wake` on `onWake`.
  // The folder is probed (it writes one tiny file) only after the user picked it, never on mount.
  let {
    backend,
    volumes,
    value,
    onChange,
    asking = false,
    onWake,
    onCancelWake,
    disabled = false,
  }: {
    backend: Backend;
    volumes: VolumeChoice[];
    value: VolumeChoice | null;
    onChange: (volume: VolumeChoice) => void;
    asking?: boolean;
    onWake?: () => void;
    onCancelWake?: () => void;
    disabled?: boolean;
  } = $props();

  const locale = $derived(i18n.locale);
  // A folder on a volume the list does not have (a probed one) still appears in the menu.
  const options = $derived(value && !volumes.some((v) => v.root === value.root) ? [...volumes, value] : volumes);
  let picking = $state(false);
  let error = $state<string | null>(null);

  const label = (v: VolumeChoice) =>
    t('performance.disk.volumeItem', { root: v.root, model: v.model ?? v.label, kind: t(`performance.disk.kind.${v.kind}`) });
  /** The part of the disk that is full, 0-1, for the bar. */
  const usedShare = $derived(value && value.totalBytes > 0 ? Math.min(1, Math.max(0, 1 - value.freeBytes / value.totalBytes)) : 0);

  function choose(root: string) {
    const picked = options.find((v) => v.root === root);
    if (!picked) return;
    error = null;
    onChange(picked);
  }

  async function pick() {
    picking = true;
    error = null;
    try {
      const folder = await backend.performanceDiskPick();
      if (folder === null) return;
      onChange(await backend.performanceDiskProbe(folder));
    } catch (e) {
      error = diskErrorText(e, t, locale) ?? String(e);
    } finally {
      picking = false;
    }
  }

  const focusOnMount = (node: HTMLElement) => node.focus();
</script>

<div class="target">
  <div class="row">
    <label class="menu">
      <span class="caption">{t('performance.disk.volume')}</span>
      <select aria-label={t('performance.disk.volume')} value={value?.root ?? ''} disabled={disabled || options.length === 0} onchange={(e) => choose(e.currentTarget.value)}>
        {#if value === null}<option value="" disabled></option>{/if}
        {#each options as v (v.root)}<option value={v.root}>{label(v)}</option>{/each}
      </select>
    </label>
    <button type="button" class="pick" disabled={disabled || picking} onclick={pick}>{t('performance.disk.pick')}</button>
  </div>

  {#if value}
    <div class="space">
      <span class="bar" aria-hidden="true"><span class="used" style:width={`${usedShare * 100}%`}></span></span>
      <span class="free">{t('performance.disk.free', { free: formatBytes(value.freeBytes, locale), total: formatBytes(value.totalBytes, locale) })}</span>
    </div>
    <p class="folder">{t('performance.disk.folder', { folder: value.folder })}</p>
  {/if}

  {#if error}<p class="notice crit" role="alert">{error}</p>{/if}
  {#if value}
    {#if value.removable}<p class="notice warn">{t('performance.disk.warn.removable')}</p>{/if}
    {#if value.sync}<p class="notice warn">{t('performance.disk.warn.sync')}</p>{/if}
    {#if value.virtualDisk}<p class="notice warn">{t('performance.disk.warn.virtual')}</p>{/if}
    {#if value.deviceId === null}<p class="notice warn">{t('performance.disk.warn.noDevice')}</p>{/if}
  {/if}
</div>

{#if asking}
  <div class="scrim">
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="disk-standby-title"
      aria-describedby="disk-standby-body"
      tabindex="-1"
      onkeydown={(e) => e.key === 'Escape' && onCancelWake?.()}
    >
      <h2 id="disk-standby-title">{t('performance.disk.standby.title')}</h2>
      <p id="disk-standby-body">{t('performance.disk.standby.body')}</p>
      <div class="actions">
        <button type="button" class="ghost" use:focusOnMount onclick={() => onCancelWake?.()}>{t('performance.history.cancel')}</button>
        <button type="button" class="go" onclick={() => onWake?.()}>{t('performance.disk.standby.confirm')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .target {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent-2);
    border-radius: 0 var(--radius) var(--radius) 0;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: end;
  }
  .menu {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 4px;
    min-width: 220px;
  }
  .caption {
    font-size: 12px;
    color: var(--text-muted);
  }
  select {
    padding: 6px 8px;
    font: inherit;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .pick,
  .ghost,
  .go {
    padding: 6px 14px;
    font: inherit;
    font-size: 14px;
    cursor: pointer;
    border-radius: 8px;
  }
  .pick,
  .ghost {
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .go {
    font-weight: 600;
    color: var(--on-accent);
    background: var(--accent);
    border: 1px solid var(--accent);
  }
  button:disabled,
  select:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  button:focus-visible,
  select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  /* The space left, as the fuel gauge of the target: the used part in pink on a dark rail. */
  .space {
    display: flex;
    gap: 10px;
    align-items: center;
    font-size: 13px;
  }
  .bar {
    flex: 0 0 120px;
    height: 6px;
    overflow: hidden;
    background: var(--surface-2);
    border-radius: 3px;
  }
  .used {
    display: block;
    height: 100%;
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent);
  }
  .free {
    font-variant-numeric: tabular-nums;
  }
  .folder {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .notice {
    margin: 0;
    padding: 8px 12px;
    font-size: 13px;
    border-left: 3px solid var(--tone);
    background: color-mix(in srgb, var(--tone) 8%, transparent);
    border-radius: 0 8px 8px 0;
  }
  .notice.warn {
    --tone: var(--warn);
  }
  .notice.crit {
    --tone: var(--crit);
  }
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    padding: 16px;
    background: color-mix(in srgb, var(--bg) 70%, transparent);
  }
  .dialog {
    width: min(420px, 100%);
    padding: 18px 20px 16px;
    background: var(--surface);
    border: 1px solid color-mix(in srgb, var(--warn) 55%, var(--border));
    border-radius: var(--radius);
    box-shadow: 0 0 32px color-mix(in srgb, var(--warn) 20%, transparent);
  }
  h2 {
    margin: 0 0 6px;
    font-size: 15px;
    font-weight: 600;
  }
  .dialog p {
    margin: 0 0 16px;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
</style>
