<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { folderErrorText, logErrorText } from '../../lib/log/messages';
  import { overlay } from '../../lib/overlay.svelte';
  import type { BenchmarkEntry } from '../../lib/types';
  import Group from './controls/Group.svelte';

  // Settings › Benchmark (spec M7d §8): start or stop a capture, then the saved sessions.
  let { backend }: { backend: Backend } = $props();

  const status = $derived(overlay.status);
  const bench = $derived(status?.benchmark ?? null);
  const recording = $derived(bench?.state === 'recording');
  /** The capture needs the engine, which only runs with the overlay on (DD6). */
  const overlayOn = $derived(status?.enabled ?? false);

  let entries = $state.raw<BenchmarkEntry[]>([]);
  let loaded = $state(false);
  let failure = $state<string | null>(null);
  let confirming = $state<string | null>(null);

  async function load() {
    try {
      entries = await backend.benchmarkList();
    } catch (error) {
      failure = folderErrorText(error, t);
    }
    loaded = true;
  }
  onMount(() => void load());

  // The history is read again when a capture leaves `recording`, whatever way it ended.
  let wasRecording = false;
  $effect(() => {
    const now = recording;
    if (wasRecording && !now) void load();
    wasRecording = now;
  });

  // One interval, only while recording: the shell sends `elapsedS` once, the UI counts on from it.
  let elapsed = $state(0);
  $effect(() => {
    elapsed = bench?.elapsedS ?? 0;
    if (!recording) return;
    const timer = setInterval(() => (elapsed += 1), 1000);
    return () => clearInterval(timer);
  });

  const clock = (s: number) => {
    const n = Math.max(0, Math.floor(s));
    return `${String(Math.floor(n / 60)).padStart(2, '0')}:${String(n % 60).padStart(2, '0')}`;
  };

  const int = $derived(new Intl.NumberFormat(i18n.locale));
  const one = $derived(new Intl.NumberFormat(i18n.locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 }));
  const when = $derived(new Intl.DateTimeFormat(i18n.locale, { dateStyle: 'medium', timeStyle: 'short' }));
  const dateOf = (e: BenchmarkEntry) => when.format(new Date(e.record.startedAt));
  const ms = (v: number) => `${one.format(v)} ms`;

  /** Label and value of each figure the summary has; absent ones (null) are left out. */
  function rows(e: BenchmarkEntry): [string, string][] {
    const s = e.record.summary;
    const out: [string, string | null][] = [
      [t('benchmark.duration'), clock(s.durationS)],
      [t('benchmark.fpsDisplayed'), one.format(s.fpsDisplayed)],
      [t('benchmark.fpsRendered'), s.fpsRendered === null ? null : `${one.format(s.fpsRendered)}${s.renderedSource ? ` (${s.renderedSource})` : ''}`],
      [t('benchmark.lowsIntegral'), `${one.format(s.lowsIntegral.onePercent)} / ${one.format(s.lowsIntegral.pointOnePercent)}`],
      [t('benchmark.lowsPercentile'), `${one.format(s.lowsPercentile.onePercent)} / ${one.format(s.lowsPercentile.pointOnePercent)}`],
      [t('benchmark.frametime'), `${one.format(s.frametimeMinMs)} / ${ms(s.frametimeMaxMs)}`],
      [t('benchmark.fgMultiplier'), s.fgMultiplier === null ? null : `×${one.format(s.fgMultiplier)}`],
      [t('benchmark.latencyPc'), s.latencyPcMs === null ? null : ms(s.latencyPcMs)],
      [t('benchmark.latencyDisplay'), s.latencyDisplayMs === null ? null : ms(s.latencyDisplayMs)],
    ];
    return out.filter((r): r is [string, string] => r[1] !== null);
  }

  /** Runs a backend call, showing its failure (a key or the system's text) in the page. */
  async function run(call: () => Promise<void>) {
    failure = null;
    try {
      await call();
    } catch (error) {
      failure = folderErrorText(error, t);
    }
  }

  async function remove(id: string) {
    confirming = null;
    await run(() => backend.benchmarkDelete(id));
    await load();
  }

  const shownError = $derived(failure ?? (bench?.state === 'error' ? logErrorText(bench.error, t) : null));
</script>

<Group id="benchmark-capture" title={t('settings.section.benchmark')}>
  <div class="capture">
    <div class="state">
      {#if recording}
        <span class="led" aria-hidden="true"></span>
        <p class="rec" role="status">{t('benchmark.recording', { game: bench?.game ?? '', time: clock(elapsed) })}</p>
      {:else if !overlayOn}
        <p class="muted">{t('benchmark.needsOverlay')}</p>
      {:else}
        <p class="muted">{t('benchmark.hint')}</p>
      {/if}
    </div>
    <button type="button" class="action" class:stop={recording} disabled={!recording && !overlayOn} onclick={() => (recording || overlayOn) && run(() => backend.benchmarkToggle())}>
      {recording ? t('benchmark.stop') : t('benchmark.start')}
    </button>
  </div>
  {#if shownError}
    <p class="error" role="alert">{shownError}</p>
  {/if}
</Group>

<Group id="benchmark-history" title={t('benchmark.history')}>
  <div class="bar">
    <button type="button" class="action" onclick={() => run(() => backend.benchmarkOpenFolder())}>{t('benchmark.openFolder')}</button>
  </div>
  {#if loaded && entries.length === 0}
    <p class="muted pad">{t('benchmark.history.empty')}</p>
  {/if}
  <ul class="sessions">
    {#each entries as e (e.id)}
      <li>
        <div class="head">
          <span class="game">{e.record.game}</span>
          <span class="muted">{dateOf(e)}</span>
          <span class="reason">{t(`benchmark.end.${e.record.endReason}`)}</span>
        </div>
        <p class="frames">
          {t('benchmark.frames', {
            total: int.format(e.record.summary.framesTotal),
            displayed: int.format(e.record.summary.framesDisplayed),
            generated: int.format(e.record.summary.framesGenerated),
          })}
        </p>
        <dl>
          {#each rows(e) as [label, value] (label)}
            <div><dt>{label}</dt><dd>{value}</dd></div>
          {/each}
        </dl>
        <p class="frames">
          {t('benchmark.stutter', { count: int.format(e.record.summary.stutterCount), percent: `${one.format(e.record.summary.stutterPercent)}%` })}
        </p>
        <div class="buttons">
          {#if confirming === e.id}
            <span class="ask">{t('benchmark.delete.confirm', { game: e.record.game, date: dateOf(e) })}</span>
            <button type="button" class="action danger" onclick={() => remove(e.id)}>{t('benchmark.delete')}</button>
            <button type="button" class="action" onclick={() => (confirming = null)}>{t('rules.cancel')}</button>
          {:else}
            <button type="button" class="action" onclick={() => run(() => backend.benchmarkOpenCsv(e.id))}>{t('benchmark.openCsv')}</button>
            <button type="button" class="action" onclick={() => (confirming = e.id)}>{t('benchmark.delete')}</button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
</Group>

<style>
  .capture {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 24px;
    padding: 12px 16px;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .led {
    flex: none;
    width: 10px;
    height: 10px;
    background: var(--crit);
    border-radius: 50%;
    box-shadow: 0 0 10px color-mix(in srgb, var(--crit) 70%, transparent);
  }
  .rec {
    margin: 0;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    overflow-wrap: anywhere;
  }
  .muted {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .pad {
    padding: 10px 16px;
  }
  .error {
    margin: 0;
    padding: 0 16px 12px;
    font-size: 12.5px;
    color: var(--crit);
  }
  .bar {
    display: flex;
    justify-content: flex-end;
    padding: 8px 16px;
  }
  .action {
    flex: none;
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
  .action:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .action:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .action.stop,
  .action.danger {
    border-color: color-mix(in srgb, var(--crit) 60%, var(--border));
  }
  .sessions {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .sessions li {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 16px;
  }
  .sessions li + li {
    border-top: 1px solid var(--border);
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 12px;
  }
  .game {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 13px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .reason {
    margin-left: auto;
    font-size: 12px;
    color: var(--text-muted);
  }
  .frames {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  dl {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 4px 24px;
    margin: 0;
  }
  dl div {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
  }
  .ask {
    margin-right: auto;
    font-size: 13px;
  }
</style>
