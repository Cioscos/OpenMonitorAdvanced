<script lang="ts">
  import catalog from '../../../../testdata/performance/catalog.json';
  import type { Backend } from '../../lib/backend';
  import { formatBytes } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { estimatedWrites, formatBytes as formatDiskBytes } from '../../lib/performance/disk';
  import { around, errorText, formatDuration, kernelTerm, marked, modeTerm, sizeLabel, timedSeconds } from '../../lib/performance/format';
  import { performanceStore } from '../../lib/performance/performance.svelte';
  import { settings } from '../../lib/settings.svelte';
  import type { Custom, GpuChoice, Isa, Objective, Phase, Plan, Preset, StartRequest, StressComponent, VolumeChoice } from '../../lib/types';
  import Term from '../common/Term.svelte';
  import DiskTarget from './DiskTarget.svelte';
  import RiskNotice from './RiskNotice.svelte';
  import WizardCustomize from './WizardCustomize.svelte';

  // The stress test wizard (spec M8 §3.4, layout B of the mockup): component, objective, duration,
  // then the summary of the planned phases with «Personalizza» and «Avvia». The plan comes from the
  // shell (`performancePreview`), never from the UI.
  let { backend, onStarted }: { backend: Backend; onStarted: () => unknown } = $props();

  const STEPS = ['component', 'objective', 'duration', 'summary'] as const;
  const PRESETS: Preset[] = ['quick', 'standard', 'long', 'night'];
  /** DA10: under this share the RAM tests cannot run at all. */
  const RAM_MIN = 256 * 1024 ** 2;
  /** «Personalizza» previews after a short pause, so a burst of edits sends one request. */
  const DEBOUNCE_MS = 150;

  let step = $state(0);
  let component = $state<StressComponent>('cpu');
  /** The device id of the chosen GPU (DG13), kept by id so «Repeat» works after a restart. */
  let gpuId = $state<string | null>(null);
  let objective = $state<Objective>('normal');
  let preset = $state<Preset>('standard');
  let custom = $state<Custom | null>(null);
  let customizing = $state(false);
  let plan = $state.raw<Plan | null>(null);
  /** The profile's plan (no «Personalizza»): its kernels and minutes are what the panel edits. */
  let base = $state.raw<Plan | null>(null);
  let previewError = $state<string | null>(null);
  let asking = $state(false);
  /** A spun-down HDD: the shell refused to start until the user agrees to wake it (M8c DC6). */
  let standbyAsking = $state(false);
  /** The volume the user chose for a disk test; the system one until then. */
  let picked = $state<VolumeChoice | null>(null);
  let starting = $state(false);
  let startError = $state<string | null>(null);

  const system = $derived(performanceStore.system);
  const volumes = $derived(system?.volumes ?? []);
  const volume = $derived(volumes.find((v) => v.root === picked?.root) ?? picked ?? volumes.find((v) => v.system) ?? volumes[0] ?? null);
  const ramOk = $derived((system?.ramBudget ?? 0) >= RAM_MIN);
  const presetSeconds = $derived((catalog.presets as Record<string, Partial<Record<Preset, number>>>)[`${component}.${objective}`] ?? {});
  const presets = $derived(PRESETS.filter((p) => presetSeconds[p] !== undefined));
  const gpu = $derived(component === 'gpu' ? (system?.gpus.find((g) => g.deviceId === gpuId) ?? null) : null);
  const disk = $derived(component === 'disk');
  const request = $derived<StartRequest>({
    component,
    objective,
    preset,
    custom: custom && $state.snapshot(custom),
    retryCore: null,
    gpu: component === 'gpu' ? gpuId : undefined,
    disk: disk && volume ? { folder: volume.folder, wake: false } : undefined,
  });
  /** What the disk test writes at most (DC7), from the preview's own plan. */
  const writes = $derived(disk && plan ? estimatedWrites(plan) : 0);
  const total = $derived(plan ? timedSeconds(plan.phases) : 0);
  /**
   * The summary's rows: with «Personalizza», every phase of the profile stays in place and those of an
   * unticked mode read «excluded», so the list keeps its length and the panel below does not jump.
   * The other rows take the preview's phase (minutes, set) in order; until it arrives, the profile's.
   */
  const rows = $derived.by(() => {
    if (!plan) return [];
    if (!base || !custom) return plan.phases.map((p) => ({ p, off: false }));
    const off = new Set(custom.modes.filter((m) => !m.enabled || m.minutes === 0).map((m) => m.kernel));
    let i = 0;
    return base.phases.map((b) => (off.has(b.kernel) ? { p: b, off: true } : { p: plan!.phases[i]?.kernel === b.kernel ? plan!.phases[i++]! : b, off: false }));
  });
  const bestIsa = $derived((catalog.isa as Isa[]).find((i) => system?.isa.includes(i)) ?? null);
  const canNext = $derived(step === 0 ? system !== null && (component === 'cpu' || (component === 'ram' ? ramOk : disk ? volume !== null : gpuId !== null)) : true);
  const canStart = $derived(plan !== null && !performanceStore.running && !starting);
  const choice = $derived([
    component === 'gpu' ? (gpu?.name ?? t('performance.wizard.gpu')) : t(`performance.wizard.${component}`),
    t(disk ? `performance.objective.disk.${objective}` : `performance.objective.${objective}`),
    t(`performance.preset.${preset}`),
  ]);

  /** A new component, objective or duration means a new plan: «Personalizza» starts again from it. */
  function choose(apply: () => void) {
    apply();
    if (!presets.includes(preset)) preset = 'standard';
    custom = null;
    customizing = false;
    plan = base = null;
    previewError = null;
    // A preview still on its way is for the old choice.
    generation++;
  }

  // The summary previews the plan of the current choices; a superseded reply is dropped.
  let generation = 0;
  $effect(() => {
    if (step !== STEPS.length - 1) return;
    const next = request;
    const id = ++generation;
    const timer = setTimeout(
      () =>
        backend
          .performancePreview(next)
          .then((reply) => {
            if (id !== generation) return;
            plan = reply;
            if (next.custom === null) base = reply;
            previewError = null;
          })
          .catch((error) => {
            if (id !== generation) return;
            plan = null;
            previewError = errorText(error, t, i18n.locale);
          }),
      next.custom ? DEBOUNCE_MS : 0,
    );
    return () => clearTimeout(timer);
  });

  function toggleCustomize() {
    if (!customizing && custom === null && base) {
      const kernels = [...new Set(base.phases.map((p) => p.kernel))];
      custom = {
        modes: kernels.map((kernel) => ({ kernel, enabled: true, minutes: null })),
        isa: null,
        threads: 'allLogical',
        bothSmt: false,
        stopOnFirstError: null,
        ...(disk ? { compressible: false } : {}),
      };
    }
    customizing = !customizing;
  }

  function requestStart() {
    if (!canStart) return;
    if (settings.state?.settings.performance.riskNoticeSeen) void start();
    else asking = true;
  }

  async function confirmRisk(dontShowAgain: boolean) {
    asking = false;
    // Start stays off while the setting is written.
    starting = true;
    if (dontShowAgain) await settings.update({ performance: { riskNoticeSeen: true } });
    await start();
  }

  async function start(wake = false) {
    starting = true;
    startError = null;
    standbyAsking = false;
    try {
      const sent = wake && request.disk ? { ...request, disk: { ...request.disk, wake } } : request;
      const result = await performanceStore.start(sent, disk ? (volume?.deviceId ?? null) : null);
      if (result.ok) onStarted();
      else startError = t(`performance.wizard.${result.reason}`);
    } catch (error) {
      if (disk && String(error) === 'disk:standby') standbyAsking = true;
      else startError = t('performance.wizard.startError', { reason: errorText(error, t, i18n.locale) });
    } finally {
      starting = false;
    }
  }

  const noService = $derived(around(t('performance.warn.noService'), t('glossary.thermalStop.name')));
  const overclockHint = $derived(
    component === 'gpu' ? [t('performance.objective.overclock.hint.gpu'), '', ''] : around(t('performance.objective.overclock.hint'), t('glossary.curveOptimizer.name')),
  );
  const gpuShared = $derived(around(t('performance.warn.gpuShared'), t('glossary.vram.name')));
  /** The GPU tile's detail with its memory word carrying the term: VRAM, or the RAM an integrated GPU shares. */
  const gpuDetail = (g: GpuChoice) =>
    g.integrated ? around(t('performance.wizard.gpu.integrated'), 'RAM') : around(t('performance.wizard.gpu.detail', { vram: formatBytes(g.dedicatedBytes, i18n.locale) }), t('glossary.vram.name'));

  const cpuDetail = $derived(system && marked(t('performance.wizard.cpu.detail', { cores: system.cores, threads: system.logical })));
  const onePerCore = $derived(marked(t('performance.wizard.onePerCore')));

  // Each new step takes the focus to its title, so a screen reader reads where it is; not on mount.
  let title = $state<HTMLElement>();
  let shownStep = 0;
  $effect(() => {
    if (step !== shownStep) title?.focus();
    shownStep = step;
  });

  const PLACEMENT_TERM: Record<Phase['placement'], string | null> = { all_logical: 'mode.allCore', core_cycle: 'mode.coreCycle', one_per_core: null };
</script>

{#snippet noServiceWarning()}
  <p class="warn">{noService[0]}{#if noService[1]}<Term term="thermalStop">{noService[1]}</Term>{/if}{noService[2]}</p>
{/snippet}

<div class="wizard">
  <ol class="steps" aria-label={t('performance.wizard.steps')}>
    {#each STEPS as id, index (id)}
      <li class:done={index < step} class:on={index === step} aria-current={index === step ? 'step' : undefined}>
        <span class="n">{index + 1}</span>
        <span>{t(`performance.wizard.step.${id}`)}{#if index < step && index < choice.length}<span class="picked">: {choice[index]} ✓</span>{/if}</span>
      </li>
    {/each}
  </ol>

  <h3 tabindex="-1" bind:this={title}>{t(`performance.wizard.step.${STEPS[step]}`)}</h3>

  {#if step === 0}
    {#if system === null}
      <p class="muted">{t('performance.wizard.loading')}</p>
    {:else}
      <div class="tiles" role="radiogroup" aria-label={t('performance.wizard.step.component')}>
        <label class="tile" class:on={component === 'cpu'}>
          <input type="radio" name="wizard-component" aria-labelledby="wizard-cpu" checked={component === 'cpu'} onchange={() => choose(() => (component = 'cpu'))} />
          <b id="wizard-cpu">{t('performance.wizard.cpu')}</b>
          <span>{system.cpuModel}</span>
          {#if cpuDetail}<span class="muted">{cpuDetail[0]}{#if cpuDetail[1]}<Term term="threads">{cpuDetail[1]}</Term>{/if}{cpuDetail[2]}</span>{/if}
        </label>
        <label class="tile" class:on={component === 'ram'} class:disabled={!ramOk}>
          <input type="radio" name="wizard-component" aria-labelledby="wizard-ram" aria-describedby={ramOk ? undefined : 'wizard-ram-low'} disabled={!ramOk} checked={component === 'ram'} onchange={() => choose(() => (component = 'ram'))} />
          <b id="wizard-ram">{t('performance.wizard.ram')}</b>
          <span>{t('performance.wizard.ram.detail', { total: formatBytes(system.ramTotal, i18n.locale) })}</span>
          <span class="muted"><Term term="ramShare" />: {formatBytes(system.ramBudget, i18n.locale)}</span>
          {#if !ramOk}<span class="reason" id="wizard-ram-low">{t('performance.wizard.ram.low')}</span>{/if}
        </label>
        <label class="tile" class:on={disk} class:disabled={volumes.length === 0}>
          <input type="radio" name="wizard-component" aria-labelledby="wizard-disk" disabled={volumes.length === 0} checked={disk} onchange={() => choose(() => (component = 'disk'))} />
          <b id="wizard-disk">{t(volumes.length === 0 ? 'performance.wizard.disk.none' : 'performance.wizard.disk')}</b>
          {#if volume}<span class="muted">{t('performance.wizard.disk.detail', { kind: t(`performance.disk.kind.${volume.kind}`), free: formatDiskBytes(volume.freeBytes, i18n.locale) })}</span>{/if}
        </label>
        {#each system.gpus as g, index (g.deviceId)}
          {@const detail = gpuDetail(g)}
          <label class="tile" class:on={component === 'gpu' && gpuId === g.deviceId}>
            <input type="radio" name="wizard-component" aria-labelledby="wizard-gpu-{index}" checked={component === 'gpu' && gpuId === g.deviceId} onchange={() => choose(() => ((component = 'gpu'), (gpuId = g.deviceId)))} />
            <b id="wizard-gpu-{index}">{g.name}</b>
            <span class="muted">{detail[0]}{#if detail[1]}<Term term="vram">{detail[1]}</Term>{/if}{detail[2]}</span>
          </label>
        {:else}
          <label class="tile disabled">
            <input type="radio" name="wizard-component" aria-labelledby="wizard-gpu-none" disabled />
            <b id="wizard-gpu-none">{t('performance.wizard.gpu.none')}</b>
          </label>
        {/each}
      </div>
      {#if disk}
        <DiskTarget {backend} {volumes} value={volume} onChange={(v) => choose(() => (picked = v))} />
      {:else if component !== 'gpu' && !system.serviceConnected}{@render noServiceWarning()}{/if}
    {/if}
  {:else if step === 1}
    <div class="tiles big" role="radiogroup" aria-label={t('performance.wizard.step.objective')}>
      <label class="tile" class:on={objective === 'normal'}>
        <input type="radio" name="wizard-objective" aria-labelledby="wizard-normal" aria-describedby="wizard-normal-hint" checked={objective === 'normal'} onchange={() => choose(() => (objective = 'normal'))} />
        <b id="wizard-normal">{t(disk ? 'performance.objective.disk.normal' : 'performance.objective.normal')}</b>
        <span class="muted" id="wizard-normal-hint">{t(disk ? 'performance.objective.disk.normal.detail' : 'performance.objective.normal.hint')}</span>
      </label>
      <label class="tile" class:on={objective === 'overclock'}>
        <input type="radio" name="wizard-objective" aria-labelledby="wizard-overclock" aria-describedby="wizard-overclock-hint" checked={objective === 'overclock'} onchange={() => choose(() => (objective = 'overclock'))} />
        <b id="wizard-overclock">{t(disk ? 'performance.objective.disk.overclock' : 'performance.objective.overclock')}</b>
        <span class="muted" id="wizard-overclock-hint">{#if disk}{t('performance.objective.disk.overclock.detail')}{:else}{overclockHint[0]}{#if overclockHint[1]}<Term term="curveOptimizer">{overclockHint[1]}</Term>{/if}{overclockHint[2]}{/if}</span>
      </label>
    </div>
  {:else if step === 2}
    <div class="chips" role="radiogroup" aria-label={t('performance.wizard.step.duration')}>
      {#each presets as id (id)}
        <label class="chip" class:on={preset === id}>
          <input type="radio" name="wizard-preset" checked={preset === id} onchange={() => choose(() => (preset = id))} />{t(`performance.preset.${id}`)} · {formatDuration(presetSeconds[id]!)}
        </label>
      {/each}
    </div>
  {:else}
    {#if plan}
      <div class="tape" aria-hidden="true">
        {#each rows.filter((r) => !r.off) as { p }, index (index)}<span style:flex-grow={p.duration_s} style:--c="var(--{p.placement === 'core_cycle' ? 'accent-2' : p.mode === 'steady' ? 'accent' : 'warn'})"></span>{/each}
      </div>
      <p class="total">{t('performance.wizard.total', { duration: formatDuration(total) })}</p>
      <ol class="phases" aria-label={t('performance.wizard.phases')}>
        {#each rows as { p, off }, index (index)}
          <li class:off class:gpu={component === 'gpu'} class:disk style:--c="var(--{p.placement === 'core_cycle' ? 'accent-2' : p.mode === 'steady' ? 'accent' : 'warn'})">
            <span class="name"><Term term={kernelTerm(p.kernel)} />{#if p.alt_kernel}{' + '}<Term term={`mode.${p.alt_kernel}`} />{/if}{#if sizeLabel(p)}{' · '}<Term term="cache">{sizeLabel(p)}</Term>{/if}</span>
            {#if component !== 'gpu' && !disk}<span class="isa"><Term term={`isa.${p.isa}`} /></span>{/if}
            {#if !disk}<span class="load"><Term term={modeTerm(p.mode)} />{#if component === 'gpu'}{''}{:else}{' · '}{#if PLACEMENT_TERM[p.placement]}<Term term={PLACEMENT_TERM[p.placement]!} />{:else}{onePerCore[0]}<Term term="threads">{onePerCore[1]}</Term>{onePerCore[2]}{/if}{#if p.both_smt}{' · '}<Term term="smt">{t('performance.wizard.bothSmt')}</Term>{/if}{/if}</span>{/if}
            <span class="dur">{off ? t('performance.wizard.excluded') : p.kernel === 'disk_fill' ? t('performance.wizard.untilFull') : formatDuration(p.duration_s)}</span>
          </li>
        {/each}
      </ol>
    {:else if previewError}
      <p class="error" role="alert">{t('performance.wizard.previewError', { reason: previewError })}</p>
    {/if}

    {#if disk}
      <DiskTarget {backend} {volumes} value={volume} onChange={() => {}} disabled asking={standbyAsking} onWake={() => start(true)} onCancelWake={() => (standbyAsking = false)} />
    {/if}

    {#if system}
      <div class="warnings">
        {#if component !== 'gpu' && !disk && !system.serviceConnected}{@render noServiceWarning()}{/if}
        {#if disk && plan}<p class="note">{t('performance.disk.writes', { size: formatDiskBytes(writes, i18n.locale) })}</p>{/if}
        {#if component === 'gpu'}<p class="note"><Term term="stability" />: {t('performance.wizard.gpuIdle')}</p>{/if}
        {#if gpu?.integrated}<p class="warn">{gpuShared[0]}{#if gpuShared[1]}<Term term="vram">{gpuShared[1]}</Term>{/if}{gpuShared[2]}</p>{/if}
        {#if bestIsa && component !== 'gpu' && !disk}<p class="note">{t('performance.wizard.isaDetected')} <Term term={`isa.${bestIsa}`} /></p>{/if}
        {#if plan && plan.ram_bytes > 0}<p class="note"><Term term="ramShare" />: {formatBytes(plan.ram_bytes, i18n.locale)}</p>{/if}
        {#if system.hypervisor}<p class="warn"><Term term="vm" />: {t('performance.wizard.vm')}</p>{/if}
      </div>
    {/if}

    <div>
      <button type="button" class="ghost" aria-expanded={customizing} disabled={base === null} onclick={toggleCustomize}>{t('performance.wizard.customize')}</button>
    </div>
    {#if customizing && custom && base && system}
      <WizardCustomize bind:custom {base} isa={system.isa} gpu={component === 'gpu'} {disk} />
    {/if}
  {/if}

  {#if startError}<p class="error" role="alert">{startError}</p>{/if}

  <div class="nav">
    {#if step > 0}
      <button type="button" class="ghost" onclick={() => step--}>{t('performance.wizard.back')}</button>
    {:else}
      <span></span>
    {/if}
    {#if step < STEPS.length - 1}
      <button type="button" class="go" disabled={!canNext} onclick={() => step++}>{t('performance.wizard.next')}</button>
    {:else}
      <span class="start">
        {#if performanceStore.running}<span class="muted">{t('performance.wizard.running')}</span>{/if}
        <button type="button" class="go" disabled={!canStart} onclick={requestStart}>{t('performance.wizard.start')}</button>
      </span>
    {/if}
  </div>
</div>

{#if asking}
  <RiskNotice onConfirm={confirmRisk} onCancel={() => (asking = false)} />
{/if}

<style>
  .wizard {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 880px;
  }
  .steps {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .steps li {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    padding: 6px 10px;
    font-size: 13px;
    color: var(--text-muted);
    background: var(--surface-2);
    border-radius: 6px;
  }
  .steps li > span:last-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .steps .n {
    font-variant-numeric: tabular-nums;
  }
  .steps li.on {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 14%, var(--surface-2));
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .steps li.done {
    color: var(--ok);
  }
  h3 {
    outline: none;
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }
  .tile {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 14px 16px;
    line-height: 1.4;
    cursor: pointer;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .big .tile {
    min-height: 110px;
  }
  .tile b {
    font-size: 15px;
  }
  .tile:hover:not(.disabled) {
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .tile.on {
    background: color-mix(in srgb, var(--accent) 8%, var(--surface));
    border-color: var(--accent);
  }
  .tile.disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }
  /* The radio stays in the accessibility tree and takes focus; the tile shows the choice. */
  .tile input,
  .chip input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .tile:has(input:focus-visible),
  .chip:has(input:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .chip {
    position: relative;
    padding: 8px 16px;
    font-size: 14px;
    cursor: pointer;
    color: var(--text-muted);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .chip.on {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-2));
    border-color: var(--accent);
  }
  /* The plan as a strip of tape: one segment per phase, as long as the phase. */
  .tape {
    display: flex;
    gap: 2px;
    height: 8px;
  }
  .tape span {
    flex-basis: 0;
    min-width: 3px;
    background: var(--c);
    border-radius: 2px;
    box-shadow: 0 0 8px color-mix(in srgb, var(--c) 45%, transparent);
  }
  .total {
    margin: -6px 0 0;
    font-size: 13px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .phases {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--border);
  }
  .phases li {
    display: grid;
    grid-template-columns: minmax(0, 2.4fr) 70px minmax(0, 2fr) 90px;
    align-items: baseline;
    gap: 12px;
    padding: 8px 0 8px 12px;
    font-size: 14px;
    border-bottom: 1px solid var(--border);
    box-shadow: inset 2px 0 0 var(--c);
  }
  .phases li.gpu {
    grid-template-columns: minmax(0, 2.4fr) minmax(0, 2fr) 90px;
  }
  .load,
  .isa {
    color: var(--text-muted);
  }
  .phases li.disk {
    grid-template-columns: minmax(0, 1fr) 90px;
  }
  .phases li.off {
    box-shadow: none;
    opacity: 0.45;
  }
  .phases li.off .name {
    text-decoration: line-through;
  }
  .dur {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .warnings {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .warnings p,
  .warn {
    margin: 0;
    font-size: 13px;
  }
  .note {
    color: var(--text-muted);
  }
  .warn {
    color: var(--warn);
  }
  .error {
    margin: 0;
    font-size: 13px;
    color: var(--crit);
  }
  .muted {
    color: var(--text-muted);
  }
  .reason {
    font-size: 13px;
    color: var(--warn);
  }
  .nav {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .start {
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 13px;
  }
  button {
    padding: 8px 20px;
    font-size: 14px;
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
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  @media (max-width: 720px) {
    .tiles,
    .steps {
      grid-template-columns: minmax(0, 1fr);
    }
    .phases li {
      grid-template-columns: minmax(0, 1fr) auto;
    }
  }
</style>
