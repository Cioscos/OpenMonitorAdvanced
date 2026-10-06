<script lang="ts">
  import type { Backend } from '../../lib/backend';
  import { around, errorText, formatDuration, verdictTitle } from '../../lib/performance/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { performanceStore } from '../../lib/performance/performance.svelte';
  import type { StressComponent, StressSessionSummary } from '../../lib/types';
  import type { PerformancePage } from '../../lib/view';
  import Term from '../common/Term.svelte';
  import Segmented from '../settings/controls/Segmented.svelte';

  // The saved stress tests (spec M8 §3.6), newest first: a click opens the result; «Repeat» starts
  // the same request again without the wizard; «Delete» asks first. The store keeps the list.
  let { backend, onOpen }: { backend: Backend; onOpen: (page: PerformancePage) => void } = $props();

  const CRIT = ['errors', 'errors_core', 'crashed', 'hung', 'system_crash', 'failed_to_start'];
  const MARKS = { ok: '✓', warn: '!', crit: '✕' } as const;

  let filter = $state<'all' | StressComponent>('all');
  let confirming = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let message = $state<string | null>(null);

  const locale = $derived(i18n.locale);
  const rows = $derived(performanceStore.history.filter((s) => filter === 'all' || s.component === filter));
  const when = (s: StressSessionSummary) => new Date(s.startedAt).toLocaleString(locale, { dateStyle: 'short', timeStyle: 'short' });
  const verdictOf = (s: StressSessionSummary) => s.verdict ?? s.outcome;
  const toneOf = (s: StressSessionSummary) => {
    const v = verdictOf(s);
    return v === 'passed' ? 'ok' : v && CRIT.includes(v) ? 'crit' : 'warn';
  };
  /** The question of the delete confirmation: the date in full, so it reads naturally. */
  const whenLong = (s: StressSessionSummary) => new Date(s.startedAt).toLocaleString(locale, { dateStyle: 'long', timeStyle: 'short' });
  /** «Unstable · core 2» cut around «core 2», which carries its term; any other verdict is left whole. */
  const verdictPieces = (s: StressSessionSummary): [string, string, string] => {
    const title = verdictTitle({ verdict: verdictOf(s), params: s.params }, t);
    return verdictOf(s) === 'errors_core' && s.params.core !== undefined ? around(title, t('performance.core.label', { core: s.params.core })) : [title, '', ''];
  };
  /** Moves the focus to the element that appears, so the keyboard follows the confirmation. */
  const focusOnMount = (node: HTMLElement) => node.focus();

  async function remove(id: string) {
    confirming = null;
    message = null;
    try {
      await backend.performanceDelete(id);
    } catch (error) {
      message = errorText(error, t);
    }
    try {
      await performanceStore.refreshHistory();
    } catch (error) {
      message = errorText(error, t);
    }
  }

  async function repeat(id: string) {
    busy = id;
    message = null;
    try {
      const session = await backend.performanceSession(id);
      if (!session) {
        message = t('performance.history.missing');
        return;
      }
      const result = await performanceStore.start(session.request);
      if (result.ok) onOpen('run');
      else message = t(`performance.wizard.${result.reason}`);
    } catch (error) {
      message = t('performance.wizard.startError', { reason: errorText(error, t) });
    } finally {
      busy = null;
    }
  }
</script>

<div class="history">
  <Segmented
    id="history-filter"
    label={t('performance.history.filter')}
    options={[
      { value: 'all', label: t('performance.history.all') },
      { value: 'cpu', label: t('performance.wizard.cpu') },
      { value: 'ram', label: t('performance.wizard.ram') },
    ]}
    value={filter}
    onChange={(next) => (filter = next as 'all' | StressComponent)}
  />

  {#if message}<p class="message" role="alert">{message}</p>{/if}

  {#if performanceStore.history.length === 0}
    <div class="empty">
      <p>{t('performance.history.empty')}</p>
      <button type="button" class="action" onclick={() => onOpen('new')}>{t('performance.history.emptyAction')}</button>
    </div>
  {:else if rows.length === 0}
    <p class="none">{t('performance.history.noMatch')}</p>
  {:else}
    <ul aria-label={t('performance.history.list')}>
      {#each rows as s (s.id)}
        {@const tone = toneOf(s)}
        {@const [before, word, after] = verdictPieces(s)}
        <li class="row {tone}">
          <button type="button" class="open" onclick={() => onOpen(`result:${s.id}`)}>
            <span class="when">{when(s)}</span>
            <span class="what">
              {t(`performance.wizard.${s.component}`)} · {t(`performance.objective.${s.objective}`)} · {t(`performance.preset.${s.preset}`)} · {formatDuration(s.durationMs / 1000)}
            </span>
          </button>
          <p class="verdict">
            <span class="mark" role="img" aria-label={t(`performance.history.mark.${tone}`)}>{MARKS[tone]}</span>
            <span>{before}{#if word}<Term term="coreNumber">{word}</Term>{/if}{after}</span>
          </p>
          <div class="buttons">
            {#if confirming === s.id}
              <span class="ask">{t('performance.history.deleteConfirm', { when: whenLong(s) })}</span>
              <button type="button" class="action danger" use:focusOnMount onclick={() => remove(s.id)}>{t('performance.history.delete')}</button>
              <button type="button" class="action" onclick={() => (confirming = null)}>{t('performance.history.cancel')}</button>
            {:else}
              <button type="button" class="action" disabled={busy !== null} onclick={() => repeat(s.id)}>{t('performance.history.repeat')}</button>
              <button type="button" class="action" disabled={busy !== null} onclick={() => (confirming = s.id)}>{t('performance.history.delete')}</button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .history {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  ul {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 8px 16px;
    align-items: center;
    padding: 12px 16px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-left: 3px solid var(--tone);
    border-radius: 8px;
  }
  .row.ok {
    --tone: var(--ok);
  }
  .row.warn {
    --tone: var(--warn);
  }
  .row.crit {
    --tone: var(--crit);
  }
  .open {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
    background: none;
    border: 0;
  }
  .open:hover .when {
    color: var(--accent);
  }
  .open:focus-visible,
  .action:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .when {
    font-weight: 600;
  }
  .what {
    font-size: 13px;
    color: var(--text-muted);
  }
  .verdict {
    display: flex;
    gap: 8px;
    align-items: baseline;
    margin: 0;
    color: var(--tone);
  }
  .mark {
    flex: none;
    font-weight: 700;
  }
  .buttons {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    justify-content: flex-end;
  }
  .ask {
    margin-right: auto;
    font-size: 13px;
  }
  .action {
    padding: 6px 12px;
    font: inherit;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .action:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .action:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .action.danger {
    border-color: color-mix(in srgb, var(--crit) 60%, var(--border));
  }
  .empty {
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: flex-start;
    color: var(--text-muted);
  }
  .none {
    margin: 0;
    color: var(--text-muted);
  }
  .empty p,
  .message {
    margin: 0;
  }
  .message {
    color: var(--crit);
  }
  @media (max-width: 720px) {
    .row {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
