<script lang="ts">
  import catalog from '../../../../testdata/performance/catalog.json';
  import { t } from '../../lib/i18n/index.svelte';
  import { kernelTerm, marked } from '../../lib/performance/format';
  import type { Custom, Isa, KernelId, Plan } from '../../lib/types';
  import Term from '../common/Term.svelte';

  // «Personalizza» (spec M8 §3.4, DA12): the modes of the profile's plan with their minutes, the
  // instruction set, the threads and «stop at the first error». Every change edits `custom` in
  // place; the wizard previews the plan again with it.
  let { custom = $bindable(), base, isa, gpu = false, disk = false }: { custom: Custom; base: Plan; isa: Isa[]; gpu?: boolean; disk?: boolean } = $props();

  /** The profile's seconds of each kernel, in plan order. */
  const kernels = $derived.by(() => {
    const seconds = new Map<KernelId, number>();
    for (const p of base.phases) seconds.set(p.kernel, (seconds.get(p.kernel) ?? 0) + p.duration_s);
    return seconds;
  });
  const sets = $derived((catalog.isa as Isa[]).filter((i) => isa.includes(i)));
  const hasCycle = $derived(base.phases.some((p) => p.placement === 'core_cycle'));
  /** The profile's «stop at the first error»: all, none, or mixed (null) across its phases. */
  const profileStops = $derived.by(() => {
    const stops = base.phases.filter((p) => p.stop_on_error).length;
    return stops === 0 ? false : stops === base.phases.length ? true : null;
  });
  /** The profile's minutes of a kernel, never 0. */
  const profileMinutes = (kernel: KernelId) => Math.max(1, Math.ceil((kernels.get(kernel) ?? 0) / 60));
  const all = $derived(marked(t('performance.custom.threads.all')));
  const onePerCore = $derived(marked(t('performance.custom.threads.onePerCore')));
  const hint = $derived(marked(t('performance.custom.threads.hint')));
  const MAX_MINUTES = 24 * 60;

  function setMinutes(input: HTMLInputElement, kernel: KernelId) {
    const edit = custom.modes.find((m) => m.kernel === kernel)!;
    const next = Math.round(Number(input.value));
    if (input.value !== '' && next >= 1 && next <= MAX_MINUTES) edit.minutes = next;
    else input.value = String(edit.minutes ?? profileMinutes(kernel));
  }
</script>

<section class="customize" aria-label={t('performance.wizard.customize')}>
  <fieldset>
    <legend>{t('performance.custom.modes')}</legend>
    <ul class="modes">
      {#each custom.modes as edit (edit.kernel)}
        <li class:off={!edit.enabled}>
          <label class="check">
            <input type="checkbox" bind:checked={edit.enabled} />
            <Term term={kernelTerm(edit.kernel)} />
          </label>
          <span class="minutes">
            <input
              type="number"
              min="1"
              max={MAX_MINUTES}
              step="1"
              disabled={!edit.enabled}
              aria-label={t('performance.custom.minutesOf', { name: t(`glossary.${kernelTerm(edit.kernel)}.name`) })}
              value={edit.minutes ?? profileMinutes(edit.kernel)}
              onchange={(e) => setMinutes(e.currentTarget, edit.kernel)}
            />
            {t('performance.custom.minutes')}
          </span>
        </li>
      {/each}
    </ul>
  </fieldset>

  {#if !gpu && !disk}
  <fieldset>
    <legend>{t('performance.custom.isa')}</legend>
    <div class="options">
      <label class="option"><input type="radio" name="custom-isa" checked={custom.isa === null} onchange={() => (custom.isa = null)} />{t('performance.custom.isa.auto')}</label>
      {#each sets as set (set)}
        <label class="option"><input type="radio" name="custom-isa" checked={custom.isa === set} onchange={() => (custom.isa = set)} /><Term term={`isa.${set}`} /></label>
      {/each}
    </div>
  </fieldset>

  <fieldset>
    <legend><Term term="threads" /></legend>
    <div class="options">
      <label class="option"><input type="radio" name="custom-threads" checked={custom.threads === 'allLogical'} onchange={() => (custom.threads = 'allLogical')} />{all[0]}<Term term="threads">{all[1]}</Term>{all[2]}</label>
      <label class="option"><input type="radio" name="custom-threads" checked={custom.threads === 'onePerCore'} onchange={() => (custom.threads = 'onePerCore')} />{onePerCore[0]}<Term term="threads">{onePerCore[1]}</Term>{onePerCore[2]}</label>
    </div>
    <p class="hint">{hint[0]}<Term term="mode.allCore">{hint[1]}</Term>{hint[2]}</p>
  </fieldset>
  {/if}

  <div class="flags">
    {#if disk}
      <label class="check"><input type="checkbox" bind:checked={custom.compressible} /><Term term="compressible">{t('performance.custom.compressible')}</Term></label>
    {/if}
    {#if hasCycle}
      <label class="check"><input type="checkbox" bind:checked={custom.bothSmt} />{t('performance.custom.bothSmt')} (<Term term="smt" />)</label>
    {/if}
    <label class="check">
      <input
        type="checkbox"
        checked={custom.stopOnFirstError ?? profileStops ?? false}
        indeterminate={custom.stopOnFirstError === null && profileStops === null}
        onchange={(e) => (custom.stopOnFirstError = e.currentTarget.checked)}
      />
      {t('performance.custom.stopOnFirstError')}
    </label>
  </div>
</section>

<style>
  .customize {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 16px 24px;
    padding: 16px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-left: 2px solid var(--accent-2);
    border-radius: var(--radius);
  }
  fieldset {
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  fieldset:first-child {
    grid-row: span 3;
  }
  legend {
    padding: 0;
    margin-bottom: 8px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-muted);
  }
  .modes {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .modes li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .modes li.off .check {
    color: var(--text-muted);
  }
  .minutes {
    flex: none;
    font-size: 13px;
    color: var(--text-muted);
  }
  .minutes input {
    width: 64px;
    padding: 4px 6px;
    font: inherit;
    color: var(--text);
    text-align: right;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .minutes input:disabled {
    opacity: 0.45;
  }
  .options {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
  }
  .check,
  .option {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 14px;
  }
  .flags {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  input[type='checkbox'],
  input[type='radio'] {
    accent-color: var(--accent);
  }
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .hint {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
