<script lang="ts">
  import type { Backend } from '../../lib/backend';
  import { DASH, formatClock, formatPower, formatTapeCounter, formatTemperature } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { around, errorText, hresultText, percentText, pieces, verdictTitle } from '../../lib/performance/format';
  import { performanceStore } from '../../lib/performance/performance.svelte';
  import type { ErrorRecord, Isa, KernelId, StartRequest, StressSession } from '../../lib/types';
  import type { PerformancePage } from '../../lib/view';
  import Term from '../common/Term.svelte';
  import CoreGrid from './CoreGrid.svelte';
  import EventLog from './EventLog.svelte';

  // The result of a saved session (spec M8 §3.5): the verdict with where and when it happened, the
  // advice, the actions, the session summary, the cores, the errors and the event log. The u64
  // digests (`expected`, `actual`, `seed`) lose precision in JS: they stay in the JSON export.
  let { backend, id, onOpen }: { backend: Backend; id: string; onOpen: (page: PerformancePage) => void } = $props();

  const CRIT = ['errors', 'errors_core', 'crashed', 'hung', 'system_crash', 'failed_to_start', 'device_lost', 'low_stability'];

  let session = $state.raw<StressSession | null>(null);
  let loading = $state<'loading' | 'ready' | 'missing' | 'error'>('loading');
  let loadError = $state('');
  let starting = $state(false);
  let actionError = $state<string | null>(null);
  let exported = $state<string | null>(null);
  let errorsOpen = $state(false);

  $effect(() => {
    const current = id;
    let stale = false;
    loading = 'loading';
    session = null;
    actionError = exported = null;
    backend
      .performanceSession(current)
      .then((s) => {
        if (stale) return;
        session = s;
        loading = s ? 'ready' : 'missing';
      })
      .catch((error) => {
        if (stale) return;
        loadError = String(error);
        loading = 'error';
      });
    return () => {
      stale = true;
    };
  });

  const locale = $derived(i18n.locale);
  const detail = $derived(session?.outcomeDetail ?? null);
  const verdict = $derived(detail?.verdict ?? session?.outcome ?? null);
  const title = $derived(verdictTitle(detail ?? { verdict, params: {} }, t));
  const tone = $derived(verdict === 'passed' ? 'ok' : verdict && CRIT.includes(verdict) ? 'crit' : 'warn');
  /**
   * The error the verdict speaks of, only when the verdict is about errors: the first one of the
   * unstable core, else the first of all. A crash, a hang or a stop speaks of where it happened.
   */
  const firstError = $derived<ErrorRecord | null>(
    verdict === 'errors' || verdict === 'errors_core'
      ? ((detail?.core != null ? session?.cores.find((c) => c.core === detail.core)?.firstError : null) ?? session?.errors[0] ?? null)
      : verdict === 'device_lost'
        ? (session?.errors.find((e) => e.kind === 'device_lost') ?? null)
        : null,
  );
  /** Where and when: at that error, else where the test ended (the journal's place after a crash). */
  const facts = $derived.by(() => {
    if (!session || verdict === 'passed') return null;
    const e = firstError;
    const phase = e?.phase ?? detail?.phase ?? null;
    if (phase === null) return null;
    // A GPU has no instruction set to show.
    const isa: Isa | null = session.component === 'gpu' ? null : (e?.isa ?? session.plan.phases[phase]?.isa ?? null);
    const kernel: KernelId | null = e?.kernel ?? detail?.kernel ?? null;
    return {
      phase,
      kernel,
      isa,
      core: e ? e.core : (detail?.core ?? null),
      atMs: e ? e.atMs : (detail?.atMs ?? null),
      clockMhz: e ? e.clockMhz : (detail?.clockMhz ?? null),
      tempC: e ? e.tempC : (detail?.tempC ?? null),
      iteration: e?.iteration ?? null,
      loadPercent: e?.load_percent ?? null,
    };
  });
  const kindText = $derived(
    firstError ? pieces(t(`performance.result.kind.${firstError.kind}`), firstError.kind === 'device_lost' ? [{ term: 'tdr', word: 'TDR' }] : [{ term: 'reference' }], t) : null,
  );
  /** The driver's code of a lost device; an exit without an error record gives none. */
  const deviceLostCode = $derived(firstError?.kind === 'device_lost' ? t('performance.result.deviceLostCode', { code: hresultText(firstError.actual) }) : null);
  const stabilityText = $derived(session?.stability == null ? null : t('performance.result.stability', { stability: percentText(session.stability, locale) }));
  const advice = $derived(verdict === 'errors_core' ? around(t('performance.advice.core'), t('glossary.curveOptimizer.name')) : null);
  /** «Retry only core N»: only when every error is on that one core. */
  const retry = $derived(verdict === 'errors_core' && session?.component !== 'gpu' && firstError?.core != null ? { core: firstError.core, kernel: firstError.kernel } : null);

  // The core grid only for a plan that tested cores one at a time, like the live page: in an all-core
  // phase every core works at once and none would read «tested».
  const showCores = $derived(!!session?.cores.length && session.plan.phases.some((p) => p.placement === 'core_cycle'));

  const totalMs = $derived(session?.plan.phases.reduce((sum, p) => sum + p.duration_s * 1000, 0) ?? 0);
  const durationMs = $derived(session?.endedAt ? Math.max(0, Date.parse(session.endedAt) - Date.parse(session.startedAt)) : null);
  const phaseCounts = $derived.by(() => {
    const phases = session?.phases ?? [];
    const count = (outcome: string) => phases.filter((p) => p.outcome === outcome).length;
    const counts = { passed: count('passed'), errors: count('errors'), skipped: count('skipped'), stopped: count('stopped'), notRun: Math.max(0, (session?.plan.phases.length ?? 0) - phases.length) };
    return Object.entries(counts)
      // «Stopped» only when a stop cut a phase: the other counts always show.
      .filter(([key, n]) => key !== 'stopped' || n > 0)
      .map(([key, n]) => t(`performance.result.phaseCount.${key}${n === 1 ? '.one' : ''}`, { n }))
      .join(' · ');
  });
  const checks = $derived(session?.phases.reduce((sum, p) => sum + p.checks, 0) ?? 0);
  const counter = $derived(new Intl.NumberFormat(locale));
  const whea = $derived.by(() => {
    const w = session?.whea;
    if (!w) return null;
    const by = (ids: string[]) => ids.reduce((sum, i) => sum + (w.byId[i] ?? 0), 0);
    // The live `whea` events name the core of each APIC (the session's topology at the time).
    const coreOf = new Map<string, string>();
    for (const e of session!.events) if (e.code === 'whea' && e.params.apic !== undefined && e.params.core !== undefined) coreOf.set(e.params.apic, e.params.core);
    return {
      corrected: by(['17', '19']),
      fatal: by(['18']),
      ids: Object.entries(w.byId),
      apics: Object.entries(w.byApic).map(([apic, n]) => ({ apic, n, core: coreOf.get(apic) ?? null })),
      unreadable: w.unreadable,
    };
  });
  const notes = $derived(
    Object.fromEntries((session?.cores ?? []).filter((c) => c.firstError).map((c) => [c.core, formatTapeCounter(c.firstError!.atMs)])),
  );
  const started = $derived(session ? new Date(session.startedAt).toLocaleString(locale, { dateStyle: 'short', timeStyle: 'short' }) : '');

  const maxAvg = (max: number | null, avg: number | null, format: (v: number | null, l: string) => string) =>
    t('performance.result.maxAvg', { max: format(max, locale), avg: format(avg, locale) });

  async function start(request: StartRequest) {
    starting = true;
    actionError = null;
    try {
      const result = await performanceStore.start(request);
      if (result.ok) onOpen('run');
      else actionError = t(`performance.wizard.${result.reason}`);
    } catch (error) {
      actionError = t('performance.wizard.startError', { reason: errorText(error, t) });
    } finally {
      starting = false;
    }
  }

  async function exportJson() {
    actionError = exported = null;
    try {
      const name = await backend.performanceExport(id);
      if (name) exported = t('performance.result.exported', { name });
    } catch (error) {
      actionError = t('performance.result.exportError', { reason: errorText(error, t) });
    }
  }
</script>

{#if loading === 'loading'}
  <p class="muted">{t('performance.result.loading')}</p>
{:else if loading === 'missing'}
  <p class="muted">{t('performance.result.missing')}</p>
{:else if loading === 'error'}
  <p class="error" role="alert">{t('performance.result.loadError', { reason: loadError })}</p>
{:else if session}
  <div class="result">
    <section class="verdict {tone}">
      <h3>{title}</h3>
      {#if facts}
        <dl class="facts" aria-label={t('performance.result.facts')}>
          <div>
            <dt><Term term="phase" /></dt>
            <dd>
              {facts.phase + 1}{#if facts.kernel}{' · '}<Term term={`mode.${facts.kernel}`} />{/if}{#if facts.isa}{' · '}<Term term={`isa.${facts.isa}`} />{/if}
            </dd>
          </div>
          {#if facts.core !== null}
            <div><dt><Term term="coreNumber">{t('performance.result.fact.core')}</Term></dt><dd>{facts.core}</dd></div>
          {/if}
          {#if facts.loadPercent !== null}
            <div><dt><Term term="loadLevel" /></dt><dd>{facts.loadPercent} %</dd></div>
          {/if}
          {#if facts.atMs !== null}
            <div><dt>{t('performance.result.fact.time')}</dt><dd>{formatTapeCounter(facts.atMs)}</dd></div>
          {/if}
          {#if facts.clockMhz !== null}
            <div><dt><Term term="clock" /></dt><dd>{formatClock(facts.clockMhz, locale)}</dd></div>
          {/if}
          {#if facts.tempC !== null}
            <div><dt>{t('performance.result.fact.temp')}</dt><dd>{formatTemperature(facts.tempC, locale)}</dd></div>
          {/if}
          {#if facts.iteration !== null}
            <div><dt><Term term="iteration" /></dt><dd>{facts.iteration}</dd></div>
          {/if}
        </dl>
      {/if}
      {#if kindText}
        <p>{#each kindText as piece, index (index)}{#if piece.term}<Term term={piece.term}>{piece.text}</Term>{:else}{piece.text}{/if}{/each}</p>
      {/if}
      {#if deviceLostCode}
        <p><Term term="deviceLost">{deviceLostCode}</Term></p>
      {/if}
      {#if stabilityText}
        <p><Term term="stability">{stabilityText}</Term></p>
      {/if}
      {#if advice}
        <p>{advice[0]}{#if advice[1]}<Term term="curveOptimizer">{advice[1]}</Term>{/if}{advice[2]}</p>
      {/if}
    </section>

    <div class="actions">
      {#if retry}
        <button type="button" class="go" disabled={starting || performanceStore.running} onclick={() => start({ ...session!.request, retryCore: retry })}>
          {t('performance.result.retryCore', { core: retry.core })}
        </button>
      {/if}
      <button type="button" class="ghost" disabled={starting || performanceStore.running} onclick={() => start(session!.request)}>{t('performance.result.repeat')}</button>
      <button type="button" class="ghost" onclick={exportJson}>{t('performance.result.export')}</button>
    </div>
    {#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
    {#if exported}<p class="muted" role="status">{exported}</p>{/if}

    <div class="panels" class:two={showCores}>
      <section class="panel">
        <p class="label">{t('performance.result.session')}</p>
        <dl class="summary">
          <div><dt>{t('performance.result.device')}</dt><dd>{session.device}</dd></div>
          <div><dt>{t('performance.result.started')}</dt><dd>{started}</dd></div>
          <div>
            <dt>{t('performance.result.duration')}</dt>
            <dd>{t('performance.result.durationOf', { done: durationMs === null ? DASH : formatTapeCounter(durationMs), total: formatTapeCounter(totalMs) })}</dd>
          </div>
          <div><dt><Term term="phase">{t('performance.result.phases')}</Term></dt><dd>{phaseCounts}</dd></div>
          <div><dt><Term term="check" /></dt><dd>{counter.format(checks)}</dd></div>
          <div><dt>{t('performance.result.fact.temp')}</dt><dd>{maxAvg(session.stats.tempMaxC, session.stats.tempAvgC, formatTemperature)}</dd></div>
          <div><dt><Term term="packagePower" /></dt><dd>{maxAvg(session.stats.powerMaxW, session.stats.powerAvgW, formatPower)}</dd></div>
          <div><dt><Term term="clock" /></dt><dd>{maxAvg(session.stats.clockMaxMhz, session.stats.clockAvgMhz, formatClock)}</dd></div>
          {#if whea}
            <div>
              <dt><Term term="whea" /></dt>
              <dd>
                {#if whea.unreadable}{t('performance.warn.wheaUnreadable')}{:else}{t('performance.run.wheaSub', { corrected: whea.corrected, fatal: whea.fatal })}{/if}
                {#if whea.ids.length > 0}
                  <ul class="plain">
                    {#each whea.ids as [wheaId, n] (wheaId)}
                      <li><Term term="whea" /> {t('performance.result.wheaId', { id: wheaId, kind: t(`performance.result.wheaKind.${wheaId === '18' ? 'fatal' : 'corrected'}`), n })}</li>
                    {/each}
                  </ul>
                {/if}
              </dd>
            </div>
            {#if whea.apics.length > 0}
              <div>
                <dt><Term term="apicId" /></dt>
                <dd>
                  <ul class="plain">
                    {#each whea.apics as a (a.apic)}
                      <li>{a.core === null ? t('performance.result.wheaApic', { apic: a.apic, n: a.n }) : t('performance.result.wheaApicCore', { apic: a.apic, core: a.core, n: a.n })}</li>
                    {/each}
                  </ul>
                </dd>
              </div>
            {/if}
          {/if}
        </dl>
      </section>
      {#if showCores}
        <section class="panel">
          <p class="label"><Term term="coreNumber">{t('performance.result.cores')}</Term></p>
          <CoreGrid cores={session.cores} final {notes} />
        </section>
      {/if}
    </div>

    {#if session.errors.length > 0}
      <details class="panel" bind:open={errorsOpen}>
        <summary class="label">{t('performance.result.errors', { n: session.errors.length + session.errorsDropped })}</summary>
        {#if errorsOpen}
          <table>
            <thead>
              <tr>
                <th>{t('performance.result.fact.time')}</th>
                <th><Term term="phase" /></th>
                <th>{t('performance.result.col.mode')}</th>
                <th><Term term="coreNumber">{t('performance.result.fact.core')}</Term></th>
                <th>{t('performance.result.col.result')}</th>
                <th><Term term="clock" /></th>
                <th>{t('performance.result.fact.temp')}</th>
              </tr>
            </thead>
            <tbody>
              {#each session.errors as e, index (index)}
                <tr>
                  <td>{formatTapeCounter(e.atMs)}</td>
                  <td>{e.phase + 1}</td>
                  <td><Term term={`mode.${e.kernel}`} /> · <Term term={`isa.${e.isa}`} /></td>
                  <td>{e.core ?? DASH}</td>
                  <td>{t(`performance.result.kindShort.${e.kind}`)}</td>
                  <td>{formatClock(e.clockMhz, locale)}</td>
                  <td>{formatTemperature(e.tempC, locale)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </details>
      {#if session.errorsDropped > 0}<p class="muted">{t('performance.result.errorsDropped', { n: session.errorsDropped })}</p>{/if}
    {/if}

    <section class="panel">
      <p class="label">{t('performance.run.events')}</p>
      <EventLog events={session.events} />
    </section>
  </div>
{/if}

<style>
  .result {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .verdict {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 16px 18px;
    border: 1px solid var(--c);
    border-radius: var(--radius);
    background: color-mix(in srgb, var(--c) 6%, var(--surface));
  }
  .verdict.ok {
    --c: var(--ok);
  }
  .verdict.warn {
    --c: var(--warn);
  }
  .verdict.crit {
    --c: var(--crit);
  }
  h3 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    color: var(--c);
  }
  .verdict p {
    margin: 0;
    line-height: 1.5;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 24px;
    margin: 0;
  }
  .facts div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .facts dt {
    font-size: 12px;
    color: var(--text-muted);
  }
  .facts dd {
    margin: 0;
    font-size: 14px;
    font-variant-numeric: tabular-nums;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  button {
    padding: 7px 16px;
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
  .ghost {
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  button:focus-visible,
  summary:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .panels {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 12px;
  }
  .panels.two {
    grid-template-columns: minmax(0, 1.4fr) minmax(0, 1fr);
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    padding: 10px 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .panel > .label {
    margin: 0;
  }
  summary {
    cursor: pointer;
  }
  .summary {
    display: flex;
    flex-direction: column;
    margin: 0;
  }
  .summary div {
    display: grid;
    grid-template-columns: 120px minmax(0, 1fr);
    gap: 8px;
    padding: 5px 0;
    border-bottom: 1px solid var(--border);
  }
  .summary dt {
    font-size: 12px;
    color: var(--text-muted);
  }
  .summary dd {
    margin: 0;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  .plain {
    margin: 2px 0 0;
    padding: 0;
    list-style: none;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 5px 6px;
    text-align: left;
    border-bottom: 1px solid var(--border);
    font-variant-numeric: tabular-nums;
  }
  th {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-muted);
  }
  .muted {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .error {
    margin: 0;
    font-size: 13px;
    color: var(--crit);
  }
  @media (max-width: 900px) {
    .panels.two {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
