<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../../lib/i18n/index.svelte';
  import type { Backend } from '../../lib/backend';
  import Term from '../common/Term.svelte';

  // «Share» (DZ12, DZ16, DZ17): the exact text that will be posted, the overclock box and the note.
  // Escape cancels; focus goes to the first control and back to the opener on close.
  let { backend, scoreId, onClose }: { backend: Backend; scoreId: string; onClose: (shared: boolean) => void } = $props();

  const CODES = [
    'bad_json', 'bad_schema', 'bad_format', 'unknown_version', 'not_valid', 'bad_value', 'implausible', 'body_too_large',
    'rate_limited', 'daily_cap', 'offline', 'timeout', 'tls', 'http', 'invalid', 'provisional', 'shared', 'not_found',
  ];
  const errorText = (error: unknown) => {
    const code = String(error);
    return t(CODES.includes(code) ? `performance.share.error.${code}` : 'performance.share.error.unknown');
  };

  let overclock = $state(false);
  let preview = $state<string | null>(null);
  let error = $state<string | null>(null);
  let sending = $state(false);
  let loads = 0;

  $effect(() => {
    const flag = overclock;
    const load = ++loads;
    backend
      .performanceSharePreview(scoreId, flag)
      .then((text) => load === loads && ((preview = text), (error = null)))
      .catch((e) => load === loads && ((preview = null), (error = errorText(e))));
  });

  async function send() {
    if (sending) return;
    sending = true;
    error = null;
    try {
      await backend.performanceShareSend(scoreId, overclock);
      onClose(true);
    } catch (e) {
      error = errorText(e);
      sending = false;
    }
  }

  let first = $state<HTMLInputElement>();
  onMount(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    first?.focus();
    return () => opener?.focus();
  });
</script>

<div class="scrim">
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="perf-share-title"
    tabindex="-1"
    onkeydown={(e) => e.key === 'Escape' && !sending && onClose(false)}
  >
    <h2 id="perf-share-title">{t('performance.share.title')}</h2>
    <pre aria-label={t('performance.share.preview')}>{preview ?? '…'}</pre>
    <label class="check">
      <input type="checkbox" bind:this={first} bind:checked={overclock} disabled={sending} />
      <Term term="overclock">{t('performance.share.overclock')}</Term>
    </label>
    <p class="note"><Term term="anonymousShare">{t('performance.share.note')}</Term></p>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="button" class="ghost" disabled={sending} onclick={() => onClose(false)}>{t('performance.share.cancel')}</button>
      <button type="button" class="go" disabled={sending || preview === null} onclick={send}>
        {sending ? t('performance.share.sending') : t('performance.share.send')}
      </button>
    </div>
  </div>
</div>

<style>
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
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(560px, 100%);
    max-height: 100%;
    padding: 18px 20px 16px;
    overflow: auto;
    background: var(--surface);
    border: 1px solid color-mix(in srgb, var(--accent) 55%, var(--border));
    border-radius: var(--radius);
    box-shadow: 0 0 32px color-mix(in srgb, var(--accent) 20%, transparent);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  pre {
    max-height: 240px;
    margin: 0;
    padding: 10px 12px;
    overflow: auto;
    font: 12px/1.45 ui-monospace, 'Cascadia Mono', Consolas, monospace;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 13px;
  }
  .note {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .error {
    margin: 0;
    padding: 8px 12px;
    font-size: 13px;
    border-left: 3px solid var(--crit);
    background: color-mix(in srgb, var(--crit) 8%, transparent);
    border-radius: 0 8px 8px 0;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  button {
    padding: 6px 14px;
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    border-radius: 8px;
  }
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
  button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
