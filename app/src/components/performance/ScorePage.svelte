<script lang="ts">
  import { untrack } from 'svelte';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { Backend } from '../../lib/backend';
  import { benchStore, isBenchRunning, modeValue, type ScoreTarget } from '../../lib/performance/bench.svelte';
  import { benchWrites, diskErrorText, diskFullScale, formatBytes, formatLatency, MIN_FREE_BYTES } from '../../lib/performance/disk';
  import { pieces } from '../../lib/performance/format';
  import { fullScale, gpuFullScale } from '../../lib/performance/gauge';
  import { performanceStore } from '../../lib/performance/performance.svelte';
  import type { BenchMode, DiskProfile, ScoreFile, ScoreSummary, VolumeChoice } from '../../lib/types';
  import Term from '../common/Term.svelte';
  import DiskTarget from './DiskTarget.svelte';
  import Gauge from './Gauge.svelte';

  // «Score › CPU» (M8a2, spec §3.3 and §4.6) and «Score › <GPU>» (M8b2 DH12): two gauges with the
  // live needle, the phases as a bar, Start or Stop, the reference ▲ (in memory only, DB10), then
  // the last measurement in detail and the saved ones. Everything is the target's own: another
  // target's status and scores never show here. One benchmark or stress test at a time (DB8).
  // A GPU target without `name` is a GPU that is no longer in the system: its history only.
  // «Score › Disk» (M8c): the same cluster with Read and Write in MB/s, the volume menu on top; the
  // history covers every disk, the record and the reference are those of the chosen disk (DC15).

  let { target, backend }: { target: ScoreTarget & { name?: string; integrated?: boolean; unavailable?: boolean }; backend?: Backend } = $props();

  const gpu = $derived(target.category === 'gpu');
  const disk = $derived(target.category === 'disk');
  const noGpu = $derived(target.category === 'gpu' && target.name === undefined);
  // The GPU list could not be read: the GPU is not known to be gone, so no «gone» claim.
  const missing = $derived(noGpu && !target.unavailable);
  const MODES = $derived<BenchMode[]>(gpu ? ['compute', 'graphics'] : disk ? ['read', 'write'] : ['single', 'multi']);
  const TERM: Record<BenchMode, string> = { single: 'singleCore', multi: 'multiCore', compute: 'computeScore', graphics: 'graphicsScore', read: 'mbs', write: 'mbs' };
  /** The invalid reasons (DH9, DC12): each is the message of a measurement that is not valid, never a warning. */
  const REASONS = ['compute_error', 'device_lost', 'hung', 'io_error', 'disk_full'];

  let reference = $state<'record' | 'last'>('record');
  let startError = $state<string | null>(null);
  let starting = $state(false);
  let confirming = $state<string | null>(null);
  // The disk target: the volume the test would run on (the system one until the user chooses), the
  // options of «Customize» and the consent to wake a spun-down HDD.
  let picked = $state<VolumeChoice | null>(null);
  let profile = $state<DiskProfile>('b1');
  let compressible = $state(false);
  let asking = $state(false);
  const volumes = $derived(performanceStore.system?.volumes ?? []);
  const volume = $derived(picked ?? volumes.find((v) => v.system) ?? volumes[0] ?? null);

  const locale = $derived(i18n.locale);
  const status = $derived(benchStore.statusFor(target));
  const running = $derived(isBenchRunning(status));
  const scores = $derived(benchStore.scoresFor(target));
  const latest = $derived<ScoreSummary | null>(scores[0] ?? null);
  const step = $derived(status && status.step !== null ? (status.steps[status.step] ?? null) : null);
  // The record and the last measurement of a disk are those of the chosen disk.
  const own = $derived<ScoreTarget>(disk ? { category: 'disk', deviceId: volume?.deviceId ?? null } : target);
  const record = $derived(benchStore.recordFor(own));
  const ref = $derived(reference === 'record' ? record : benchStore.lastFor(own));

  const finite = (v: number | null | undefined): number | null => (v != null && Number.isFinite(v) ? v : null);
  // Between phases (and during the warm-up pause) the rate is missing: the needle holds the last
  // live value of the mode in progress instead of falling to 0. Forgotten when the run ends.
  let held = $state<Partial<Record<BenchMode, number>>>({});
  /** The live needle of the step under way: the points, or the disk's speed in its direction. */
  const liveNow = (): number | null =>
    disk ? (step?.mode === 'read' ? (status?.liveRead ?? null) : step?.mode === 'write' ? (status?.liveWrite ?? null) : null) : (status?.livePoints ?? null);
  $effect(() => {
    const live = running ? finite(liveNow()) : null;
    const mode = step?.mode;
    const run = running;
    untrack(() => {
      if (!run) held = {};
      else if (mode && live !== null) held = { ...held, [mode]: live };
    });
  });

  /** The live needle while the mode runs, then its score; without a run, the last saved one. */
  function valueOf(mode: BenchMode): number | null {
    if (running) return step?.mode === mode ? (finite(liveNow()) ?? held[mode] ?? null) : finite(modeValue(status!, mode));
    if (status?.state === 'done') return finite(modeValue(status, mode));
    return latest ? finite(modeValue(latest, mode)) : null;
  }
  const values = $derived(Object.fromEntries(MODES.map((m) => [m, valueOf(m)])) as Partial<Record<BenchMode, number | null>>);

  /** DB6, DH7: the CPU dial never drops below 1500 points; a GPU one starts from its estimate. */
  const scaleOf = (mode: BenchMode) => {
    const marks = [record[mode] ?? 0, ref[mode] ?? 0, values[mode] ?? 0];
    return disk ? diskFullScale(marks, volume?.kind ?? 'other') : gpu ? gpuFullScale(marks, target.integrated ?? false) : fullScale(marks);
  };
  // During a measurement the full scale only grows: the largest one of the run is kept.
  let heldScale = $state<Partial<Record<BenchMode, number>>>({});
  $effect(() => {
    const scales = MODES.map((m) => [m, scaleOf(m)] as const);
    const run = running;
    untrack(() => {
      heldScale = run ? Object.fromEntries(scales.map(([m, s]) => [m, Math.max(heldScale[m] ?? 0, s)])) : {};
    });
  });
  const maxOf = (mode: BenchMode) => Math.max(heldScale[mode] ?? 0, scaleOf(mode));

  // A new status makes an earlier refusal stale.
  $effect(() => {
    void benchStore.status;
    untrack(() => (startError = null));
  });

  /** The phase in words, cut around the terms it carries. */
  const phase = $derived.by(() => {
    if (!running || !step) return null;
    if (disk) {
      if (step.kernel === 'disk_fill') return [{ text: t('performance.score.disk.fill'), term: null }];
      const text = t('performance.score.disk.phase', {
        test: t(`glossary.diskBench.${step.kernel}.name`),
        direction: t(`performance.score.${step.mode}`),
        rep: step.rep === 0 ? t('performance.score.disk.warmup') : t('performance.score.disk.measure', { n: step.rep }),
      });
      return pieces(text, [{ term: `diskBench.${step.kernel}` }, { term: 'warmup' }], t);
    }
    if (gpu) {
      const text = t('performance.score.gpu.phase', { kernel: t(`glossary.gpuBench.${step.kernel}.name`), group: t(`performance.score.${step.mode}`) });
      return pieces(text, [{ term: `gpuBench.${step.kernel}` }, { term: TERM[step.mode] }], t);
    }
    const text = t('performance.score.phase', {
      kernel: t(`glossary.bench.${step.kernel}.name`),
      mode: t(`performance.score.${step.mode}`),
      rep: step.rep === 0 ? t('performance.score.warmup') : t('performance.score.rep', { n: step.rep }),
    });
    return pieces(text, [{ term: `bench.${step.kernel}` }, { term: TERM[step.mode] }, { term: 'warmup' }], t);
  });

  /** Why the last run ended without a score, or why the shell refused the start. */
  const message = $derived.by(() => {
    if (startError) return startError;
    const error = status?.state === 'failed' ? status.error : null;
    if (!error) return null;
    if (error.startsWith('performance.start.')) return t(disk ? 'performance.outcome.failed_to_start' : 'performance.score.error.start', { reason: t(error) });
    // `exited`, `failed`, `crashed`, `hung`: the run ended early, which says nothing on the hardware.
    return t('performance.score.error.failed');
  });

  // The measurement below the gauges: the one just saved, else the newest; loaded in full.
  const detailId = $derived(status?.state === 'done' && status.scoreId ? status.scoreId : (latest?.id ?? null));
  let detail = $state.raw<ScoreFile | null>(null);
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
      .catch((error) => console.error('score unavailable', error));
    return () => (current = false);
  });
  const shown = $derived(running ? null : detail);
  const flagsOf = (flags: string[]) => flags.filter((f) => !REASONS.includes(f));
  const invalidText = (flags: string[]) => {
    const reason = ['device_lost', 'hung', 'io_error', 'disk_full'].find((f) => flags.includes(f));
    return reason ? t(`performance.score.invalid.${reason}`) : t('performance.score.invalid');
  };
  const provisional = $derived(shown?.provisional ?? benchStore.provisionalFor(target));
  const scaling = $derived(shown?.scaling != null ? pieces(t('performance.score.scaling', { pct: Math.round(shown.scaling * 100) }), [{ term: 'scaling' }], t) : null);

  const number = (v: number | null) =>
    v === null ? '–' : v.toLocaleString(locale, { maximumFractionDigits: v < 10 ? 2 : v < 100 ? 1 : 0 });
  const percent = (v: number | null) => (v === null ? '–' : v.toLocaleString(locale, { style: 'percent', maximumFractionDigits: 1 }));
  const whole = (v: number | null) => (v === null ? '–' : Math.round(v));
  /** A read / write pair of the disk table. */
  const pair = (read: string, write: string) => `${read} / ${write}`;
  /** The model of the disk behind a saved score: the volume's own name when it is in the system, else its id. */
  const modelOf = (s: ScoreSummary) => volumes.find((v) => v.deviceId === s.deviceId)?.model ?? s.deviceId ?? '–';
  const points = $derived(shown?.scores.points ?? null);
  const when = (at: string) => new Date(at).toLocaleString(locale, { dateStyle: 'short', timeStyle: 'short' });
  const markOf = (s: ScoreSummary) =>
    !s.valid
      ? { tone: 'crit', mark: '✕', text: invalidText(s.flags) }
      : flagsOf(s.flags).length
        ? { tone: 'warn', mark: '!', text: flagsOf(s.flags).map((f) => t(`performance.score.flag.${f}`)).join(' ') }
        : { tone: 'ok', mark: '✓', text: t('performance.history.mark.ok') };
  const focusOnMount = (node: HTMLElement) => node.focus();

  const busy = $derived(performanceStore.running || benchStore.running);

  async function start(wake = false) {
    startError = null;
    asking = false;
    starting = true;
    try {
      if (disk) {
        if (!volume) return;
        await benchStore.start(target, { folder: volume.folder, profile, compressible, wake });
      } else await benchStore.start(target);
    } catch (error) {
      const text = String(error);
      if (disk && text === 'disk:standby') asking = true;
      else
        startError =
          text === 'busy'
            ? t('performance.score.error.busy')
            : text === 'build:no_gpu'
              ? t('performance.score.gpu.missing')
              : (diskErrorText(text, t, locale) ??
                (text === 'build:no_space'
                  ? t('performance.disk.error.no_space', { size: formatBytes(MIN_FREE_BYTES, locale) })
                  : text === 'build:no_disk'
                    ? t('performance.outcome.failed_to_start', { reason: t('performance.start.no_disk') })
                    : t('performance.score.error.start', { reason: text })));
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
  {#if gpu && target.name}<p class="device">{target.name}</p>{/if}
  {#if disk && backend}
    <DiskTarget
      {backend}
      {volumes}
      value={volume}
      disabled={running}
      {asking}
      onChange={(next) => (picked = next)}
      onWake={() => start(true)}
      onCancelWake={() => (asking = false)}
    />
  {/if}
  <section class="cluster" aria-label={t(disk ? 'performance.score.disk.title' : gpu ? 'performance.score.gpu.title' : 'performance.score.title')}>
    <div class="gauges">
      {#each MODES as mode (mode)}
        <figure>
          <Gauge
            value={values[mode] ?? null}
            max={maxOf(mode)}
            reference={ref[mode]}
            label={t(`performance.score.${mode}`)}
            unit={disk ? 'MB/s' : t('performance.score.points')}
            moving={running}
          />
          <figcaption>
            {#if disk}{t(`performance.score.${mode}`)}{:else}<Term term={TERM[mode]}>{t(`performance.score.${mode}`)}</Term>{/if}
            <span class="sub">
              {#if ref[mode] !== null}<span class="mark-value">▲ {Math.round(ref[mode])}</span>{/if}
              {#if disk}<Term term="mbs" />{:else}<Term term="benchPoints" />{/if}
            </span>
          </figcaption>
        </figure>
      {/each}
    </div>

    {#if status && status.segments.length > 0 && (running || status.state !== 'done')}
      <ol class="segments" aria-hidden="true">
        {#each status.segments as segment, index (index)}
          {@const own = status.steps[index]?.mode}
          <li class={segment} class:second={own === MODES[1]} class:gap={index > 0 && own !== status.steps[index - 1]?.mode}></li>
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
          disabled={busy || starting || noGpu || (disk && !volume)}
          title={busy ? t('performance.score.error.busy') : undefined}
          onclick={() => start()}>{t('performance.score.start')}</button
        >
      {/if}
      <p class="duration">
        {disk
          ? t('performance.score.disk.duration', { size: formatBytes(benchWrites(profile), locale) })
          : t(gpu ? 'performance.score.gpu.duration' : 'performance.score.duration')}
      </p>
      <span class="reference">
        <span id="score-reference-label"><Term term="referenceMark">{t('performance.score.reference')}</Term></span>
        <select aria-labelledby="score-reference-label" bind:value={reference}>
          <option value="record">{t('performance.score.reference.record')}</option>
          <option value="last">{t('performance.score.reference.last')}</option>
        </select>
      </span>
    </div>
    {#if disk}
      <details class="custom">
        <summary>{t('performance.wizard.customize')}</summary>
        <div class="options">
          <label class="option">
            <span>{t('performance.score.disk.profile')}</span>
            <select aria-label={t('performance.score.disk.profile')} bind:value={profile} disabled={running}>
              <option value="b1">{t('performance.score.disk.profile.b1')}</option>
              <option value="b2">{t('performance.score.disk.profile.b2')}</option>
            </select>
          </label>
          <label class="option check">
            <input type="checkbox" bind:checked={compressible} disabled={running} />
            <Term term="compressible">{t('performance.score.disk.compressible')}</Term>
          </label>
        </div>
      </details>
    {/if}
  </section>

  {#if missing}<p class="notice warn">{t('performance.score.gpu.missing')}</p>{/if}
  {#if noGpu && target.unavailable}<p class="notice warn">{t('performance.score.gpu.unavailable')}</p>{/if}
  {#if message}<p class="notice crit" role="alert">{message}</p>{/if}
  {#if provisional}<p class="notice warn">{t('performance.score.provisional')}</p>{/if}

  {#if shown}
    {#if !shown.valid}<p class="notice crit" role="alert">{invalidText(shown.flags)}</p>{/if}
    {#if flagsOf(shown.flags).length}
      <ul class="flags">
        {#each flagsOf(shown.flags) as flag (flag)}<li class="notice warn">{t(`performance.score.flag.${flag}`)}</li>{/each}
      </ul>
    {/if}
    {#if scaling}
      <p class="scaling">{#each scaling as p, i (i)}{#if p.term}<Term term={p.term}>{p.text}</Term>{:else}{p.text}{/if}{/each}</p>
    {/if}

    {#if disk}
      {#if points !== null}
        <p class="points"><Term term="diskPoints">{t('performance.score.disk.points', { points: points.toLocaleString(locale) })}</Term></p>
      {:else if shown.diskProfile === 'b2'}
        <p class="muted">{t('performance.score.disk.noPoints')}</p>
      {/if}
    {/if}

    <section class="panel">
      {#if disk}
        <h3>{t('performance.score.detail')} <span class="note">(<Term term="seqRnd" />, <Term term="queueDepth" />)</span></h3>
        <table aria-label={t('performance.score.detail')}>
          <thead>
            <tr>
              <th scope="col">{t('performance.score.col.test')}</th>
              <th scope="col" class="num">{t('performance.score.read')}</th>
              <th scope="col" class="num">{t('performance.score.write')}</th>
              <th scope="col" class="num"><Term term="iops">{t('performance.score.col.iops')}</Term></th>
              <th scope="col" class="num"><Term term="latency">{t('performance.score.col.meanLat')}</Term></th>
              <th scope="col" class="num"><Term term="latency">{t('performance.score.col.p99Lat')}</Term></th>
            </tr>
          </thead>
          <tbody>
            {#each shown.kernels.filter((k) => k.read || k.write) as k (k.id)}
              <tr>
                <th scope="row"><Term term={`diskBench.${k.id}`} /></th>
                <td class="num">{number(k.read?.mbs ?? null)}</td>
                <td class="num">{number(k.write?.mbs ?? null)}</td>
                <td class="num">{pair(number(k.read?.iops ?? null), number(k.write?.iops ?? null))}</td>
                <td class="num">{pair(formatLatency(k.read?.meanLatUs, locale), formatLatency(k.write?.meanLatUs, locale))}</td>
                <td class="num">{pair(formatLatency(k.read?.p99LatUs, locale), formatLatency(k.write?.p99LatUs, locale))}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else if gpu}
        <h3>{t('performance.score.detail')} <span class="note">(<Term term="gpuMedian" />)</span></h3>
        <table aria-label={t('performance.score.detail')}>
          <thead>
            <tr>
              <th scope="col">{t('performance.score.col.kernel')}</th>
              <th scope="col" class="num">{t('performance.score.col.value')}</th>
              <th scope="col" class="num"><Term term="spread">{t('performance.score.col.spread')}</Term></th>
            </tr>
          </thead>
          <tbody>
            {#each shown.kernels as k (k.id)}
              <tr>
                <th scope="row"><Term term={`gpuBench.${k.id}`} /></th>
                <td class="num"
                  >{number(k.value)}
                  <span class="unit"
                    >{#if k.unit === 'TFLOPS' || k.unit === 'TIOPS'}<Term term="tflops">{k.unit}</Term>{:else if k.unit === 'GB/s'}<Term
                        term="gbps">{k.unit}</Term
                      >{:else if k.unit === 'Gpixel/s' || k.unit === 'Gtexel/s'}<Term term="gpixels">{k.unit}</Term>{:else}{k.unit}{/if}</span
                  ></td
                >
                <td class="num">{percent(k.spread)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <h3>{t('performance.score.detail')} <span class="note">(<Term term="median" />)</span></h3>
        <table aria-label={t('performance.score.detail')}>
          <thead>
            <tr>
              <th scope="col">{t('performance.score.col.kernel')}</th>
              <th scope="col" class="num"><Term term="singleCore">{t('performance.score.single')}</Term></th>
              <th scope="col" class="num"><Term term="multiCore">{t('performance.score.multi')}</Term></th>
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
      {/if}
    </section>
  {/if}

  <section class="panel">
    <h3 id="score-history-title">{t('performance.score.history')}</h3>
    {#if scores.length === 0}
      <p class="muted">{t('performance.score.history.empty')}</p>
    {:else}
      <table aria-labelledby="score-history-title">
        <thead>
          <tr>
            <th scope="col"><span class="visually-hidden">{t('performance.score.history')}</span></th>
            {#if disk}<th scope="col">{t('performance.score.col.disk')}</th>{/if}
            {#each MODES as mode (mode)}
              <th scope="col" class="num">{#if disk}{t(`performance.score.${mode}`)}{:else}<Term term={TERM[mode]}>{t(`performance.score.${mode}`)}</Term>{/if}</th>
            {/each}
            <th scope="col"></th>
            <th scope="col"></th>
          </tr>
        </thead>
        <tbody>
          {#each scores as s (s.id)}
            {@const mark = markOf(s)}
            <tr>
              <td>{when(s.at)}</td>
              {#if disk}<td>{modelOf(s)}</td>{/if}
              {#each MODES as mode (mode)}<td class="num">{whole(modeValue(s, mode))}</td>{/each}
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
  /* The GPU's name, right under the page title. */
  .device {
    margin: -12px 0 0;
    font-size: 14px;
    color: var(--text-muted);
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
  /* One cell per phase, the first mode then the second (single then multi, compute then
     graphics): cyan for the first half, pink for the second, with a gap between the two. */
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
  .segments li.second {
    --on: var(--accent);
  }
  .segments li.gap {
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
  .custom {
    font-size: 13px;
  }
  .custom summary {
    width: fit-content;
    cursor: pointer;
    color: var(--text-muted);
  }
  .custom summary:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .options {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    align-items: center;
    margin-top: 8px;
  }
  .option {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .points {
    margin: 0;
    font-weight: 600;
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
