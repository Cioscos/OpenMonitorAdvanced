<script lang="ts">
  import { untrack } from 'svelte';
  import { boardStore } from '../../lib/performance/board.svelte';
  import { benchStore } from '../../lib/performance/bench.svelte';
  import { BOARDS, boardView, ownRows, versionOf } from '../../lib/performance/board';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { Board } from '../../lib/types';
  import Term from '../common/Term.svelte';

  // The «Leaderboard» page (spec M8 §7.2): the rows of one category around the user's best score,
  // the percentile, and where the table comes from. Model names are text, never markup.
  let board = $state<Board>('cpu-single');

  const table = $derived(boardStore.table);
  const locale = $derived(i18n.locale);
  const version = $derived(table ? versionOf(table, board) : '');
  const view = $derived(table ? boardView(table, ownRows(benchStore.scores, board, version), board) : null);
  const top = $derived(Math.max(1, ...(view?.rows ?? []).map((r) => (r.kind === 'table' ? r.row.value : r.own.value))));
  // A download error only counts while the setting is on: a stale one means nothing after turning it off.
  const reason = $derived(table?.enabled ? (table.error ?? boardStore.error) : null);
  const KNOWN = ['offline', 'timeout', 'tls', 'http', 'invalid'];

  const num = (v: number) => v.toLocaleString(locale, { maximumFractionDigits: 0 });
  const caption = $derived.by(() => {
    if (!table || !view) return '';
    const count = view.models;
    if (!table.communityAt) return t('performance.board.caption.bundled', { version, count });
    const date = new Date(table.communityAt).toLocaleDateString(locale, { dateStyle: 'long' });
    return t('performance.board.caption.updated', { version, count, date });
  });

  // The download runs once, when the page opens and the local table has been read.
  let asked = false;
  $effect(() => {
    if (table && !asked) {
      asked = true;
      untrack(() => void boardStore.refresh(false));
    }
  });
</script>

<div class="board">
  <div class="tabs" role="tablist" aria-label={t('performance.board.tabs')}>
    {#each BOARDS as b (b)}
      <button type="button" role="tab" class="tab" class:on={board === b} aria-selected={board === b} onclick={() => (board = b)}>
        {t(`performance.board.${b}`)}
      </button>
    {/each}
  </div>

  <section class="panel">
    <h3><Term term="board">{t('performance.board.heading', { board: t(`performance.board.${board}`) })}</Term></h3>
    {#if view && view.rows.length}
      <ol class="rows" aria-label={t(`performance.board.${board}`)}>
        {#each view.rows as r, i (r.kind === 'own' ? `own:${r.own.scoreId}` : `${r.row.key}:${r.row.source}:${i}`)}
          {@const own = r.kind === 'own'}
          {@const value = r.kind === 'own' ? r.own.value : r.row.value}
          <li class="row" class:own>
            <span class="name">
              {r.kind === 'own' ? r.own.model : r.row.model}
              {#if r.kind === 'own'}<em class="you">{t('performance.board.you')}</em>{/if}
            </span>
            <span class="bar" aria-hidden="true"><span class="fill" style:width="{(100 * value) / top}%"></span></span>
            <span class="value">{num(value)}</span>
            <span class="source">
              {#if r.kind === 'table'}
                <span title={t('performance.board.n', { n: r.row.n })}>
                  <Term term={r.row.source === 'author' ? 'sourceAuthor' : 'sourceCommunity'}>
                    {t(`performance.board.source.${r.row.source}`)}
                  </Term>
                </span>
              {/if}
            </span>
          </li>
        {/each}
      </ol>
    {:else}
      <p class="muted">{t('performance.board.empty')}</p>
    {/if}

    {#if view?.percentile != null}
      <p class="pct">
        <strong>{t('performance.board.percentile', { pct: view.percentile })}</strong>
        <Term term="percentile" />
      </p>
    {/if}

    <p class="muted caption">{caption} <Term term="scoreVersion" /></p>
    <div class="actions">
      <button type="button" class="refresh" disabled={!table?.enabled || boardStore.loading} onclick={() => boardStore.refresh(true)}>
        {t('performance.board.refresh')}
      </button>
      {#if table && !table.enabled}
        <span class="muted">{t('performance.board.disabled')}</span>
      {:else if reason}
        <span class="error" role="status">{t('performance.board.failed', { reason: t(`performance.board.error.${KNOWN.includes(reason) ? reason : 'http'}`) })}</span>
      {/if}
    </div>
  </section>
</div>

<style>
  .board {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .tab {
    padding: 6px 14px;
    font-weight: 600;
    cursor: pointer;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 999px;
  }
  .tab.on {
    background: var(--surface-2);
    border-color: var(--accent);
    color: var(--accent);
  }
  .tab:focus-visible,
  .refresh:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(120px, 1.4fr) minmax(80px, 2fr) 72px 112px;
    align-items: center;
    gap: 12px;
    padding: 6px 10px;
    border-left: 2px solid transparent;
    border-radius: 0 8px 8px 0;
  }
  .row.own {
    background: var(--surface-2);
    border-left-color: var(--accent);
    box-shadow: 0 0 14px color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .you {
    margin-left: 6px;
    font-size: 12px;
    font-style: normal;
    color: var(--accent);
  }
  .bar {
    height: 8px;
    background: var(--surface);
    border-radius: 4px;
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--accent-2);
    border-radius: 4px;
  }
  .own .fill {
    background: var(--accent);
  }
  .value {
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .source {
    font-size: 12px;
    color: var(--text-muted);
  }
  .pct {
    margin: 4px 0 0;
  }
  .pct strong {
    color: var(--ok);
  }
  .muted {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
  }
  .refresh {
    padding: 6px 14px;
    cursor: pointer;
  }
  .refresh:disabled {
    cursor: default;
    opacity: 0.55;
  }
  .error {
    font-size: 13px;
    color: var(--warn);
  }
</style>
