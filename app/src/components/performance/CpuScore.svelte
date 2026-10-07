<script lang="ts">
  import { untrack } from 'svelte';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { benchStore } from '../../lib/performance/bench.svelte';
  import { pieces } from '../../lib/performance/format';
  import { fullScale } from '../../lib/performance/gauge';
  import { performanceStore } from '../../lib/performance/performance.svelte';
  import type { BenchMode, CpuScoreFile, CpuScoreSummary } from '../../lib/types';
  import Term from '../common/Term.svelte';
  import Gauge from './Gauge.svelte';

  // «Score › CPU» (M8a2, spec §3.3 and §4.6): two gauges with the live needle, the 48 phases as a
  // bar, Start or Stop, the reference ▲ (in memory only, DB10), then the last measurement in
  // detail and the saved ones. The benchmark and the stress test share one slot (DB8).

  const MODES: BenchMode[] = ['single', 'multi'];

  let reference = $state<'record' | 'last'>('record');
  let startError = $state<string | null>(null);
  let starting = $state(false);
  let confirming = $state<string | null>(null);

  const locale = $derived(i18n.locale);
  const status = $derived(benchStore.status);
  const running = $derived(benchStore.running);
  const latest = $derived<CpuScoreSummary | null>(benchStore.scores[0] ?? null);
  const step = $derived(status && status.step !== null ? (status.steps[status.step] ?? null) : null);
  const ref = $derived(reference === 'record' ? benchStore.record : benchStore.last);

  const finite = (v: number | null | undefined): number | null => (v != null && Number.isFinite(v) ? v : null);
  // Between phases (and during the warm-up pause) the rate is missing: the needle holds the last
  // live value of the mode in progress instead of falling to 0. Forgotten when the run ends.
  let held = $state<Record<BenchMode, number | null>>({ single: null, multi: null });
  $effect(() => {
    const live = running ? finite(status?.livePoints) : null;
    const mode = step?.mode;
    const run = running;
    untrack(() => {
      if (!run) held = { single: null, multi: null };
      else if (mode && live !== null) held = { ...held, [mode]: live };
    });
  });

  /** The live needle while the mode runs, then its score; without a run, the last saved one. */
  function valueOf(mode: BenchMode): number | null {
    if (running) return step?.mode === mode ? (finite(status!.livePoints) ?? held[mode]) : finite(status![mode]);
    if (status?.state === 'done') return finite(status[mode]);
    return finite(latest?.[mode]);
  }
  const values = $derived({ single: valueOf('single'), multi: valueOf('multi') });

  // DB6: during a measurement the full scale only grows, so the peak of the run is kept.
  let peak = $state({ single: 0, multi: 0 });
  $effect(() => {
    const { single, multi } = values;
    const live = running;
    untrack(() => {
      peak = live ? { single: Math.max(peak.single, single ?? 0), multi: Math.max(peak.multi, multi ?? 0) } : { single: 0, multi: 0 };
    });
  });
  const maxOf = (mode: BenchMode) => fullScale([benchStore.record[mode] ?? 0, ref[mode] ?? 0, values[mode] ?? 0, peak[mode]]);

  // A new status makes an earlier refusal stale.
  $effect(() => {
    void benchStore.status;
    untrack(() => (startError = null));
  });

  /** The phase in words, cut around the terms it carries. */
  const phase = $derived.by(() => {
    if (!running || !step) return null;
    const text = t('performance.score.phase', {
      kernel: t(`glossary.bench.${step.kernel}.name`),
      mode: t(`performance.score.${step.mode}`),
      rep: step.rep === 0 ? t('performance.score.warmup') : t('performance.score.rep', { n: step.rep }),
    });
    return pieces(text, [{ term: `bench.${step.kernel}` }, { term: step.mode === 'single' ? 'singleCore' : 'multiCore' }, { term: 'warmup' }], t);
  });

  /** Why the last run ended without a score, or why the shell refused the start. */
  const message = $derived.by(() => {
    if (startError) return startError;
    const error = status?.state === 'failed' ? status.error : null;
    if (!error) return null;
    if (error.startsWith('performance.start.')) return t('performance.score.error.start', { reason: t(error) });
    // `exited`, `failed`, `crashed`, `hung`: the run ended early, which says nothing on the hardware.
    return t('performance.score.error.failed');
  });

  // The measurement below the gauges: the one just saved, else the newest; loaded in full.
  const detailId = $derived(status?.state === 'done' && status.scoreId ? status.scoreId : (latest?.id ?? null));
  let detail = $state.raw<CpuScoreFile | null>(null);
  $effect(() => {
    const id = detailId;
    if (id === null) {
      detail = null;
      return;
    }
    let current = true;
    benchStore
      .score(id)
      .then((file) => current && (detail = file))
      .catch((error) => console.error('CPU score unavailable', error));
    return () => (current = false);
  });
  const shown = $derived(running ? null : detail);
  const flagsOf = (flags: string[]) => flags.filter((f) => f !== 'compute_error');
  const provisional = $derived(shown?.provisional ?? benchStore.provisional);
  const scaling = $derived(shown?.scaling != null ? pieces(t('performance.score.scaling', { pct: Math.round(shown.scaling * 100) }), [{ term: 'scaling' }], t) : null);

  const number = (v: number | null) =>
    v === null ? '–' : v.toLocaleString(locale, { maximumFractionDigits: v < 10 ? 2 : v < 100 ? 1 : 0 });
  const when = (at: string) => new Date(at).toLocaleString(locale, { dateStyle: 'short', timeStyle: 'short' });
  const markOf = (s: CpuScoreSummary) =>
    !s.valid
      ? { tone: 'crit', mark: '✕', text: t('performance.score.invalid') }
      : flagsOf(s.flags).length
        ? { tone: 'warn', mark: '!', text: flagsOf(s.flags).map((f) => t(`performance.score.flag.${f}`)).join(' ') }
        : { tone: 'ok', mark: '✓', text: t('performance.history.mark.ok') };
  const focusOnMount = (node: HTMLElement) => node.focus();

  async function start() {
    startError = null;
    starting = true;
    try {
      await benchStore.start();
    } catch (error) {
      const text = String(error);
      startError = text === 'busy' ? t('performance.score.error.busy') : t('performance.score.error.start', { reason: text });
    } finally {
      starting = false;
    }
  }

  function stop() {
    benchStore.stop().catch((error) => console.error('stopping the benchmark failed', error));
  }

  async function remove(id: string) {
    confirming = null;
    try {
      await benchStore.remove(id);
    } catch (error) {
      startError = String(error);
    }
  }
</script>

<div class="score">
  <section class="cluster" aria-label={t('performance.score.title')}>
    <div class="gauges">
      {#each MODES as mode (mode)}
        <figure>
          <Gauge
            value={values[mode]}
            max={maxOf(mode)}
            reference={ref[mode]}
            label={t(`performance.score.${mode}`)}
            unit={t('performance.score.points')}
            moving={running}
          />
          <figcaption>
            <Term term={mode === 'single' ? 'singleCore' : 'multiCore'}>{t(`performance.score.${mode}`)}</Term>
            <span class="sub">
              {#if ref[mode] !== null}<span class="mark-value">▲ {ref[mode]}</span>{/if}
              <Term term="benchPoints" />
            </span>
          </figcaption>
        </figure>
      {/each}
    </div>

    {#if status && status.segments.length > 0 && (running || status.state !== 'done')}
      <ol class="segments" aria-hidden="true">
        {#each status.segments as segment, index (index)}
          <li class={segment} class:multi={status.steps[index]?.mode === 'multi'}></li>
        {/each}
      </ol>
    {/if}
    {#if phase}
      <p class="phase" aria-live="polite">
        {#each phase as p, i (i)}{#if p.term}<Term term={p.term}>{p.text}</Term>{:else}{p.text}{/if}{/each}
      </p>
    {/if}

    <div class="controls">
      {#if running}
        <button type="button" class="stop" disabled={status?.state === 'stopping'} onclick={stop}>{t('performance.score.stop')}</button>
      {:else}
        <button
          type="button"
          class="go"
          disabled={performanceStore.running || starting}
          title={performanceStore.running ? t('performance.score.error.busy') : undefined}
          onclick={start}>{t('performance.score.start')}</button
        >
      {/if}
      <p class="duration">{t('performance.score.duration')}</p>
      <span class="reference">
        <span id="score-reference-label"><Term term="referenceMark">{t('performance.score.reference')}</Term></span>
        <select aria-labelledby="score-reference-label" bind:value={reference}>
          <option value="record">{t('performance.score.reference.record')}</option>
          <option value="last">{t('performance.score.reference.last')}</option>
        </select>
      </span>
    </div>
  </section>

  {#if message}<p class="notice crit" role="alert">{message}</p>{/if}
  {#if provisional}<p class="notice warn">{t('performance.score.provisional')}</p>{/if}

  {#if shown}
    {#if !shown.valid}<p class="notice crit" role="alert">{t('performance.score.invalid')}</p>{/if}
    {#if flagsOf(shown.flags).length}
      <ul class="flags">
        {#each flagsOf(shown.flags) as flag (flag)}<li class="notice warn">{t(`performance.score.flag.${flag}`)}</li>{/each}
      </ul>
    {/if}
    {#if scaling}
      <p class="scaling">{#each scaling as p, i (i)}{#if p.term}<Term term={p.term}>{p.text}</Term>{:else}{p.text}{/if}{/each}</p>
    {/if}

    <section class="panel">
      <h3>{t('performance.score.detail')} <span class="note">(<Term term="median" />)</span></h3>
      <table aria-label={t('performance.score.detail')}>
        <thead>
          <tr>
            <th scope="col">{t('performance.score.col.kernel')}</th>
            <th scope="col" class="num">{t('performance.score.single')}</th>
            <th scope="col" class="num">{t('performance.score.multi')}</th>
          </tr>
        </thead>
        <tbody>
          {#each shown.kernels as k (k.id)}
            <tr>
              <th scope="row"><Term term={`bench.${k.id}`} /></th>
              <td class="num">{number(k.single)} <span class="unit">{k.unit}</span></td>
              <td class="num">{number(k.multi)} <span class="unit">{k.unit}</span></td>
            </tr>
          {/each}
        </tbody>
      </table>
    </section>
  {/if}

  <section class="panel">
    <h3 id="score-history-title">{t('performance.score.history')}</h3>
    {#if benchStore.scores.length === 0}
      <p class="muted">{t('performance.score.history.empty')}</p>
    {:else}
      <table aria-labelledby="score-history-title">
        <thead>
          <tr>
            <th scope="col"><span class="visually-hidden">{t('performance.score.history')}</span></th>
            <th scope="col" class="num">{t('performance.score.single')}</th>
            <th scope="col" class="num">{t('performance.score.multi')}</th>
            <th scope="col"></th>
            <th scope="col"></th>
          </tr>
        </thead>
        <tbody>
          {#each benchStore.scores as s (s.id)}
            {@const mark = markOf(s)}
            <tr>
              <td>{when(s.at)}</td>
              <td class="num">{s.single ?? '–'}</td>
              <td class="num">{s.multi ?? '–'}</td>
              <td><span class="mark {mark.tone}" role="img" aria-label={mark.text} title={mark.text}>{mark.mark}</span></td>
              <td class="buttons">
                {#if confirming === s.id}
                  <button type="button" class="action danger" use:focusOnMount onclick={() => remove(s.id)}>{t('performance.score.delete')}</button>
                  <button type="button" class="action" onclick={() => (confirming = null)}>{t('performance.history.cancel')}</button>
                {:else}
                  <button type="button" class="action" onclick={() => (confirming = s.id)}>{t('performance.score.delete')}</button>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</div>

<style>
  .score {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  /* The instrument cluster: the one loud element of the page. */
  .cluster {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 20px;
    background:
      radial-gradient(ellipse at 50% 0%, color-mix(in srgb, var(--accent) 10%, transparent), transparent 70%),
      var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .gauges {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 300px));
    gap: 24px;
    justify-content: center;
  }
  figure {
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: center;
    margin: 0;
  }
  figcaption {
    display: flex;
    flex-direction: column;
    gap: 2px;
    align-items: center;
    font-weight: 600;
  }
  .sub {
    display: flex;
    gap: 6px;
    font-size: 13px;
    font-weight: 400;
    color: var(--text-muted);
  }
  .mark-value {
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  /* 48 cells, single then multi: cyan for the single half, pink for the multi half. */
  .segments {
    display: flex;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .segments li {
    --on: var(--accent-2);
    flex: 1;
    height: 8px;
    background: var(--surface-2);
    border-radius: 2px;
  }
  .segments li.multi {
    --on: var(--accent);
  }
  .segments li.multi:nth-child(25) {
    margin-left: 6px;
  }
  .segments li.done {
    background: var(--on);
  }
  .segments li.running {
    background: color-mix(in srgb, var(--on) 55%, transparent);
    box-shadow: 0 0 8px var(--on);
  }
  .segments li.failed {
    background: var(--crit);
  }
  .phase {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
    text-align: center;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    align-items: center;
  }
  .duration {
    flex: 1;
    min-width: 200px;
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .reference {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 13px;
  }
  select {
    padding: 4px 8px;
    font: inherit;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .go,
  .stop,
  .action {
    padding: 6px 14px;
    font: inherit;
    font-size: 14px;
    cursor: pointer;
    border-radius: 8px;
  }
  .go {
    font-weight: 600;
    color: var(--on-accent);
    background: var(--accent);
    border: 1px solid var(--accent);
  }
  .stop {
    font-weight: 600;
    color: var(--crit);
    background: transparent;
    border: 1px solid var(--crit);
  }
  .stop:hover:not(:disabled) {
    background: color-mix(in srgb, var(--crit) 10%, transparent);
  }
  .action {
    padding: 4px 10px;
    font-size: 13px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .action.danger {
    border-color: color-mix(in srgb, var(--crit) 60%, var(--border));
  }
  button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  button:focus-visible,
  select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
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
  .flags {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .scaling {
    margin: 0;
    font-weight: 600;
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .note {
    font-weight: 400;
    color: var(--text-muted);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 6px 8px;
    text-align: left;
    border-bottom: 1px solid var(--border);
  }
  thead th {
    font-weight: 600;
    color: var(--text-muted);
  }
  tbody th {
    font-weight: 400;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .unit {
    color: var(--text-muted);
  }
  .mark {
    font-weight: 700;
  }
  .mark.ok {
    color: var(--ok);
  }
  .mark.warn {
    color: var(--warn);
  }
  .mark.crit {
    color: var(--crit);
  }
  .buttons {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
  }
  .muted {
    margin: 0;
    color: var(--text-muted);
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  @media (max-width: 720px) {
    .gauges {
      grid-template-columns: minmax(0, 300px);
    }
  }
</style>
