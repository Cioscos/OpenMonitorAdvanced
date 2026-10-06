<script lang="ts">
  import type { Backend } from '../../lib/backend';
  import { formatClock, formatPower, formatTapeCounter, formatTemperature } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import { around, cpuChartSensors, marked } from '../../lib/performance/format';
  import { performanceStore } from '../../lib/performance/performance.svelte';
  import type { PerformancePage } from '../../lib/view';
  import type { PhaseInfo, RunWarning } from '../../lib/types';
  import HistoryChart from '../advanced/HistoryChart.svelte';
  import AnimatedNumber from '../common/AnimatedNumber.svelte';
  import Term from '../common/Term.svelte';
  import CoreGrid from './CoreGrid.svelte';
  import EventLog from './EventLog.svelte';

  // The test under way (spec M8 §3.5, mockup stress-run.html): header with the stop button, the
  // phases, five tiles, the temperature and power chart, the cores of a «one core at a time» phase,
  // warnings and the event log. Everything comes from the store's 1 Hz status and the live store:
  // the page adds no timer of its own. When the test ends it moves on to its result.
  let { backend, store, onOpen }: { backend: Backend; store: LiveStore; onOpen: (page: PerformancePage) => void } = $props();

  /** The chart's window (spec §3.5). */
  const CHART_SECONDS = 600;
  const WARN_TERMS: Partial<Record<RunWarning, string>> = { noService: 'thermalStop', ramReduced: 'ramShare', ramInsufficient: 'ramShare' };
  const PLACEMENT_TERM: Record<PhaseInfo['placement'], string | null> = { all_logical: 'mode.allCore', core_cycle: 'mode.coreCycle', one_per_core: null };

  const status = $derived(performanceStore.status);
  const system = $derived(performanceStore.system);
  const locale = $derived(i18n.locale);
  const active = $derived(status?.state === 'starting' || status?.state === 'running');
  const phase = $derived(status?.phases[status.phaseIndex] ?? null);
  /** Each phase's share of the test and how much of it has run (0…1). */
  const tape = $derived.by(() => {
    if (!status) return [];
    let start = 0;
    return status.phases.map((p, index) => {
      const done = index < status.phaseIndex ? 1 : index > status.phaseIndex ? 0 : Math.min(1, Math.max(0, (status.elapsedMs / 1000 - start) / (p.durationS || 1)));
      start += p.durationS;
      return { phase: p, done };
    });
  });
  const pill = $derived.by(() => {
    if (status?.state === 'starting') return { tone: 'muted', text: t('performance.run.pill.starting') };
    if (status?.state === 'stopping') return { tone: 'warn', text: t('performance.run.pill.stopping') };
    return status && status.errors > 0 ? { tone: 'crit', text: t('performance.run.pill.errors', { n: status.errors }) } : { tone: 'ok', text: t('performance.run.pill.ok') };
  });
  const chartSensors = $derived(cpuChartSensors(store.schema));
  const counter = $derived(new Intl.NumberFormat(locale));
  const wheaLabel = $derived(around(t('performance.run.whea'), 'WHEA'));
  const clockLabel = $derived(around(t('performance.run.clock'), t('glossary.clock.name')));
  const onePerCore = $derived(marked(t('performance.wizard.onePerCore')));

  // The test is over: its result is the page to see.
  $effect(() => {
    if (status?.state === 'finished' && status.sessionId) onOpen(`result:${status.sessionId}`);
  });

  function stop() {
    performanceStore.stop().catch((error) => console.error('stopping the stress test failed', error));
  }
</script>

{#if status && status.state === 'idle'}
  <p class="muted">{t('performance.run.none')}</p>
{:else if status}
  <div class="run">
    <header>
      <h3>{t(`performance.objective.${status.objective}`)} · {t(status.component === 'cpu' ? 'performance.wizard.cpu' : 'performance.wizard.ram')}</h3>
      <span class="pill {pill.tone}">{pill.text}</span>
      <span class="time"><b>{formatTapeCounter(status.elapsedMs)}</b> / {formatTapeCounter(status.totalMs)}</span>
      <button type="button" class="stop" disabled={!active} onclick={stop}>{t('performance.run.stop')}</button>
    </header>

    <ol class="tape" aria-label={t('performance.run.phases')}>
      {#each tape as segment, index (index)}
        <li style:flex-grow={segment.phase.durationS} style:--p={segment.done} class:now={index === status.phaseIndex} aria-current={index === status.phaseIndex ? 'step' : undefined}>
          <span class="bar"></span>
          <span class="name"><Term term={`mode.${segment.phase.kernel}`} /></span>
          <span class="visually-hidden">{t(`performance.run.phase.${index < status.phaseIndex ? 'done' : index === status.phaseIndex ? 'now' : 'todo'}`)}</span>
        </li>
      {/each}
    </ol>
    {#if phase}
      <p class="current">
        <Term term="phase" /> {t('performance.run.phaseOf', { n: status.phaseIndex + 1, total: status.phases.length })}:
        <Term term={`mode.${phase.kernel}`} /> · <Term term={`isa.${phase.isa}`} /> · <Term term={`mode.${phase.mode}`} /> ·
        {#if PLACEMENT_TERM[phase.placement]}<Term term={PLACEMENT_TERM[phase.placement]!} />{:else}{onePerCore[0]}<Term term="threads">{onePerCore[1]}</Term>{onePerCore[2]}{/if}
      </p>
    {/if}

    <div class="tiles">
      <div class="tile">
        <div class="label">{t('performance.run.temp')}</div>
        <div class="value"><AnimatedNumber value={status.tempC} format={(v) => formatTemperature(v, locale)} /></div>
        <div class="sub">
          {t('performance.run.max', { value: formatTemperature(status.tempMaxC, locale) })} · <Term term="thermalStop" />
          {status.stopC === null ? t('performance.run.stopOff') : t('performance.run.stopAt', { temp: formatTemperature(status.stopC, locale) })}{#if system?.tjmaxC != null}
            · <Term term="tjmax" /> {formatTemperature(system.tjmaxC, locale)}{/if}
        </div>
      </div>
      <div class="tile">
        <div class="label"><Term term="packagePower" /></div>
        <div class="value"><AnimatedNumber value={status.powerW} format={(v) => formatPower(v, locale)} /></div>
      </div>
      <div class="tile">
        <div class="label">{clockLabel[0]}{#if clockLabel[1]}<Term term="clock">{clockLabel[1]}</Term>{/if}{clockLabel[2]}</div>
        <div class="value"><AnimatedNumber value={status.clockMhz} format={(v) => formatClock(v, locale)} /></div>
        <div class="sub">{t('performance.run.clockSub')}</div>
      </div>
      <div class="tile">
        <div class="label">{t('performance.run.errors')}</div>
        <div class="value" class:ok={status.errors === 0} class:crit={status.errors > 0}>{counter.format(status.errors)}</div>
        <div class="sub"><Term term="check">{t('performance.run.checks')}</Term>: {counter.format(status.checks)}</div>
      </div>
      <div class="tile">
        <div class="label">{wheaLabel[0]}<Term term="whea">{wheaLabel[1]}</Term>{wheaLabel[2]}</div>
        <div class="value" class:ok={status.wheaCorrected + status.wheaFatal === 0} class:warn={status.wheaCorrected > 0 && status.wheaFatal === 0} class:crit={status.wheaFatal > 0}>
          {counter.format(status.wheaCorrected + status.wheaFatal)}
        </div>
        <div class="sub">{t('performance.run.wheaSub', { corrected: status.wheaCorrected, fatal: status.wheaFatal })}</div>
      </div>
    </div>

    {#if status.warnings.length > 0}
      <div class="warnings">
        {#each status.warnings as warning (warning)}
          {@const text = t(`performance.warn.${warning}`)}
          {@const term = WARN_TERMS[warning]}
          {@const parts = term ? around(text, t(`glossary.${term}.name`)) : [text, '', '']}
          <p role="note" class="warn">{parts[0]}{#if parts[1] && term}<Term {term}>{parts[1]}</Term>{/if}{parts[2]}</p>
        {/each}
      </div>
    {/if}

    <div class="panels" class:two={phase?.placement === 'core_cycle'}>
      <section class="panel" aria-label={t('performance.run.chart')}>
        <p class="label">{t('performance.run.chart')}</p>
        {#if chartSensors.length > 0 && store.schema}
          <HistoryChart sectionId="performance" sensors={chartSensors} defaults={chartSensors.map((s) => s.id)} schema={store.schema} {store} {backend} fixedWindow={CHART_SECONDS} />
        {:else}
          <p class="muted">{t('performance.run.noChart')}</p>
        {/if}
      </section>
      {#if phase?.placement === 'core_cycle'}
        <section class="panel cores-panel">
          <p class="label"><Term term="mode.coreCycle" /> · <Term term="coreNumber" /></p>
          <CoreGrid cores={status.cores} current={status.currentCore} />
        </section>
      {/if}
    </div>

    <section class="panel">
      <p class="label">{t('performance.run.events')}</p>
      <EventLog events={status.events} />
    </section>
  </div>
{/if}

<style>
  .run {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
  }
  h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  .pill {
    padding: 3px 10px;
    font-size: 12px;
    font-weight: 600;
    border-radius: 999px;
  }
  .pill.ok {
    color: var(--ok);
    background: color-mix(in srgb, var(--ok) 12%, transparent);
  }
  .pill.crit {
    color: var(--crit);
    background: color-mix(in srgb, var(--crit) 14%, transparent);
  }
  .pill.warn {
    color: var(--warn);
    background: color-mix(in srgb, var(--warn) 12%, transparent);
  }
  .pill.muted {
    color: var(--text-muted);
    background: var(--surface-2);
  }
  .time {
    margin-left: auto;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .time b {
    font-size: 15px;
    color: var(--text);
  }
  .stop {
    padding: 6px 14px;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    color: var(--crit);
    background: transparent;
    border: 1px solid var(--crit);
    border-radius: 8px;
  }
  .stop:hover:not(:disabled) {
    background: color-mix(in srgb, var(--crit) 10%, transparent);
  }
  .stop:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .stop:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  /* The plan as a strip of tape, as in the wizard: the phases run fill with light. */
  .tape {
    display: flex;
    gap: 3px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .tape li {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex-basis: 0;
    min-width: 0;
  }
  .bar {
    height: 8px;
    border-radius: 3px;
    background: linear-gradient(90deg, var(--accent-2) calc(var(--p) * 100%), var(--surface-2) 0);
  }
  .now .bar {
    background: linear-gradient(90deg, var(--accent) calc(var(--p) * 100%), var(--surface-2) 0);
    box-shadow: 0 0 8px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .name {
    overflow: hidden;
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .now .name {
    color: var(--text);
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .current {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 10px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 10px 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .value {
    font-size: 22px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .value.ok {
    color: var(--ok);
  }
  .value.warn {
    color: var(--warn);
  }
  .value.crit {
    color: var(--crit);
  }
  .sub {
    font-size: 12px;
    color: var(--text-muted);
  }
  .warnings {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .warn {
    margin: 0;
    font-size: 13px;
    color: var(--warn);
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
  }
  .panel > .label {
    margin: 0;
  }
  .muted {
    margin: 0;
    font-size: 13px;
    color: var(--text-muted);
  }
  @media (max-width: 900px) {
    .panels.two {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
