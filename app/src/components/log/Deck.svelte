<script lang="ts">
  import { DASH, formatBytes, formatTapeCounter } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { canDo, log } from '../../lib/log.svelte';
  import { folderErrorText, logErrorText } from '../../lib/log/messages';

  let {
    id,
    openFolder,
    recordedMs,
  }: {
    id: string;
    openFolder: () => Promise<void>;
    /** The tape counter, which the recorder runs on between the core's statuses. */
    recordedMs: number;
  } = $props();

  const status = $derived(log.status);
  const logState = $derived(status?.state ?? 'idle');
  // No key works before the first status or while a command is in flight.
  const can = $derived(status === null || log.busy ? { rec: false, pause: false, stop: false } : canDo(logState));
  const fileName = $derived(status?.path ? status.path.split(/[\\/]/).pop()! : null);
  const hasPart = $derived(status !== null && status.part > 0);
  const rows = $derived(status ? new Intl.NumberFormat(i18n.locale).format(status.rows) : DASH);
  const reason = $derived(logState === 'error' ? logErrorText(status?.error, t) : '');
  let folderError = $state<string | null>(null);

  /** The store rethrows a failed command; its state and reason arrive with the status, so nothing more is shown. */
  function run(command: () => Promise<void>) {
    command().catch((error) => console.error('log command failed', error));
  }

  function rec() {
    if (!can.rec) return;
    run(() => (logState === 'paused' ? log.resume() : log.start()));
  }

  function pause() {
    if (can.pause) run(() => log.pause());
  }

  function stop() {
    if (can.stop) run(() => log.stop());
  }

  async function showFolder() {
    try {
      await openFolder();
      folderError = null;
    } catch (error) {
      folderError = folderErrorText(error, t);
    }
  }
</script>

<!-- tabindex -1: a click on the deck's text keeps the focus inside, so Esc still reaches the recorder. -->
<div {id} class="deck" data-state={logState} role="group" aria-label={t('log.deck.label')} tabindex="-1">
  <div class="cassette">
    <div class="tape-label" title={status?.path ?? undefined}>
      <span class="name" class:empty={fileName === null}>{fileName ?? t('log.deck.noFile')}</span>
    </div>
    <svg class="window" viewBox="0 0 220 56" aria-hidden="true" focusable="false">
      <rect class="glass" x="30" y="4" width="160" height="48" rx="24" />
      <circle class="pack" cx="68" cy="28" r="21" />
      <circle class="pack" cx="152" cy="28" r="15" />
      {#each [68, 152] as cx (cx)}
        <g class="reel" class:spin={logState === 'recording'} transform="translate({cx} 28)">
          <g class="hub">
            <circle r="10.5" />
            <path d="M0 -4.5V-9M3.9 2.25L7.8 4.5M-3.9 2.25L-7.8 4.5" />
          </g>
        </g>
      {/each}
    </svg>
  </div>

  <div class="display">
    <div class="readout">
      <span class="state">{t(`log.deck.state.${logState}`)}</span>
      <span class="time">{formatTapeCounter(recordedMs)}</span>
    </div>
    <dl class="stats">
      <div><dt>{t('log.deck.rows')}</dt><dd>{rows}</dd></div>
      <div><dt>{t('log.deck.part')}</dt><dd>{hasPart ? status!.part : DASH}</dd></div>
      <div><dt>{t('log.deck.size')}</dt><dd>{hasPart ? formatBytes(status!.partBytes, i18n.locale) : DASH}</dd></div>
    </dl>
    {#if status && status.dropped > 0}
      <p class="dropped">{t('log.deck.dropped', { count: status.dropped })}</p>
    {/if}
  </div>

  {#if reason}
    <p class="reason">
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
        <path d="M8 1.8L15 14.2H1Z" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" />
        <path d="M8 6.2V9.6M8 11.4V11.9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
      </svg>
      <span>{reason}</span>
    </p>
  {/if}

  <div class="transport">
    <button
      type="button"
      class="key rec"
      aria-pressed={logState === 'recording'}
      aria-disabled={!can.rec}
      onclick={rec}
    >
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false"><circle cx="8" cy="8" r="6" /></svg>
      <span>{logState === 'paused' ? t('log.deck.resume') : t('log.deck.rec')}</span>
    </button>
    <button type="button" class="key pause" aria-pressed={logState === 'paused'} aria-disabled={!can.pause} onclick={pause}>
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
        <rect x="3" y="2" width="3.6" height="12" rx="0.6" /><rect x="9.4" y="2" width="3.6" height="12" rx="0.6" />
      </svg>
      <span>{t('log.deck.pause')}</span>
    </button>
    <button type="button" class="key stop" aria-disabled={!can.stop} onclick={stop}>
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false"><rect x="2.5" y="2.5" width="11" height="11" rx="1" /></svg>
      <span>{t('log.deck.stop')}</span>
    </button>
  </div>

  <div class="foot">
    {#if folderError}
      <p class="folder-error" role="alert">{folderError}</p>
    {/if}
    <button type="button" class="folder" onclick={showFolder}>{t('log.deck.openFolder')}</button>
  </div>
</div>

<style>
  .deck {
    --state: var(--text-muted);
    --mono: 'Cascadia Mono', Consolas, monospace;
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    z-index: 2;
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: 304px;
    max-width: calc(100vw - 32px);
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: 0 12px 32px color-mix(in srgb, var(--bg) 70%, transparent);
    cursor: default;
  }
  .deck:focus {
    outline: none;
  }
  .deck[data-state='recording'],
  .deck[data-state='error'] {
    --state: var(--crit);
  }
  .deck[data-state='paused'] {
    --state: var(--warn);
  }

  /* The cassette: a label strip with the file name over the window with the two reels. */
  .cassette {
    padding: 8px 10px 6px;
    border: 1px solid color-mix(in srgb, var(--state) 45%, var(--border));
    border-radius: 8px;
    background: var(--surface-2);
  }
  .tape-label {
    padding: 4px 8px;
    border-radius: 3px;
    border-left: 3px solid var(--accent);
    background: var(--bg);
  }
  .name {
    display: block;
    overflow: hidden;
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--text);
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .name.empty {
    font-family: var(--font);
    color: var(--text-muted);
  }
  .window {
    display: block;
    width: 100%;
    height: auto;
    margin-top: 6px;
  }
  .glass {
    fill: var(--bg);
    stroke: var(--border);
  }
  .pack {
    fill: color-mix(in srgb, var(--text-muted) 20%, var(--bg));
  }
  .hub {
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 2;
    stroke-linecap: round;
  }
  .deck[data-state='recording'] .hub {
    stroke: var(--text);
  }
  .reel.spin .hub {
    animation: reel 2.4s linear infinite;
  }
  @keyframes reel {
    to {
      transform: rotate(360deg);
    }
  }

  /* The display: the recorded time as the deck's counter, then the part's figures. */
  .display {
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    font-variant-numeric: tabular-nums;
  }
  .readout {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  .state {
    font-size: 12px;
    font-weight: 600;
    color: var(--state);
  }
  .time {
    font-family: var(--mono);
    font-size: 26px;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .deck[data-state='recording'] .time,
  .deck[data-state='paused'] .time {
    color: var(--accent-2);
  }
  .stats {
    display: grid;
    grid-template-columns: 1fr auto auto;
    gap: 12px;
    margin: 8px 0 0;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
  .stats div {
    min-width: 0;
  }
  .stats dt {
    font-size: 11px;
    color: var(--text-muted);
  }
  .stats dd {
    margin: 2px 0 0;
    font-family: var(--mono);
    font-size: 13px;
    color: var(--text);
    white-space: nowrap;
  }
  .dropped {
    margin: 6px 0 0;
    font-size: 11px;
    color: var(--text-muted);
  }

  .reason {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 0;
    font-size: 13px;
    color: var(--text);
  }
  .reason svg {
    flex: none;
    margin-top: 2px;
    color: var(--crit);
  }

  /* Transport keys: the symbol carries the neon, the outline follows it; pressed keys sit lower. */
  .transport {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .key {
    --key: var(--text-muted);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 8px 4px 6px;
    font-size: 11px;
    color: var(--text-muted);
    cursor: pointer;
    border: 1px solid color-mix(in srgb, var(--key) 45%, var(--border));
    border-radius: 8px;
    background: var(--surface-2);
    box-shadow: 0 2px 0 var(--bg);
  }
  .key svg {
    fill: var(--key);
  }
  .key.rec {
    --key: var(--crit);
  }
  .key.pause {
    --key: var(--warn);
  }
  .key.stop {
    --key: var(--text);
  }
  .key:hover:not([aria-disabled='true']) {
    border-color: var(--key);
    color: var(--text);
  }
  .key[aria-pressed='true'] {
    color: var(--text);
    border-color: var(--key);
    background: color-mix(in srgb, var(--key) 12%, var(--surface-2));
    box-shadow: inset 0 2px 0 var(--bg);
    transform: translateY(1px);
  }
  .key[aria-disabled='true'] {
    cursor: default;
    opacity: 0.38;
  }
  .key[aria-pressed='true'][aria-disabled='true'] {
    opacity: 1;
  }

  .foot {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
  }
  .folder {
    padding: 5px 10px;
    font-size: 12px;
    color: var(--text-muted);
    cursor: pointer;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: transparent;
  }
  .folder:hover {
    color: var(--text);
  }
  .folder-error {
    align-self: stretch;
    margin: 0;
    font-size: 12px;
    color: var(--warn);
  }
  .key:focus-visible,
  .folder:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
