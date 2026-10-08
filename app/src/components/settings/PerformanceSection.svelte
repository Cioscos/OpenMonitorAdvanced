<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import { settings } from '../../lib/settings.svelte';
  import Term from '../common/Term.svelte';
  import type { SettingsPatch } from '../../lib/types';
  import Field from './controls/Field.svelte';
  import Group from './controls/Group.svelte';
  import NumberInput from './controls/NumberInput.svelte';
  import Segmented from './controls/Segmented.svelte';
  import Toggle from './controls/Toggle.svelte';

  // Settings › Performance (spec M8 §3.8): the safety limits and the share of RAM of the stress
  // test. Every control sends its change at once, like the other sections.
  const current = $derived(settings.state?.settings.performance ?? null);
  /** The question shown under the switch while turning the thermal stop off waits for an answer. */
  let askingOff = $state(false);

  const errorOf = (field: string) => {
    const key = settings.errors[field];
    return key === undefined ? null : t(key);
  };
  const send = (patch: SettingsPatch['performance']) => settings.update({ performance: patch });

  function toggleThermal(next: boolean) {
    if (next) return send({ thermalStop: true });
    askingOff = true;
  }
  async function confirmOff() {
    askingOff = false;
    await send({ thermalStop: false });
  }

  type FirstError = 'profile' | 'yes' | 'no';
  const firstError = $derived<FirstError>(current?.stopOnFirstError == null ? 'profile' : current.stopOnFirstError ? 'yes' : 'no');
  const FIRST_ERROR: Record<FirstError, boolean | null> = { profile: null, yes: true, no: false };
  // «Automatic (Tjmax − 5 °C or 95 °C)»: Tjmax, the same word in every language, gets its tooltip.
  const autoText = $derived(t('settings.performance.cpuStop.auto').split('Tjmax'));
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && askingOff && (askingOff = false)} />

{#if current}
  <Group id="performance-safety" title={t('settings.performance.group.safety')}>
    <Toggle
      id="performance-thermal"
      label={t('settings.performance.thermalStop')}
      term="thermalStop"
      description={t('settings.performance.thermalStop.hint')}
      checked={current.thermalStop}
      error={errorOf('performance.thermalStop')}
      notesIds={askingOff ? ['performance-thermal-ask'] : []}
      onChange={toggleThermal}
    >
      {#snippet notes()}
        {#if askingOff}
          <div class="ask" id="performance-thermal-ask" role="alert">
            <span>{t('settings.performance.thermalStop.confirm')}</span>
            <button type="button" class="action danger" onclick={confirmOff}>{t('settings.performance.thermalStop.off')}</button>
            <button type="button" class="action" onclick={() => (askingOff = false)}>{t('settings.performance.cancel')}</button>
          </div>
        {/if}
      {/snippet}
    </Toggle>
    <Field id="performance-cpu-stop" labelFor={current.cpuStopC === null ? null : 'performance-cpu-stop-input'} label={t('settings.performance.cpuStop')} description={t('settings.performance.cpuStop.hint')} error={errorOf('performance.cpuStopC')}>
      {#snippet control()}
        <div class="buttons">
          {#if current.cpuStopC === null}
            <span class="auto">{autoText[0]}{#if autoText.length > 1}<Term term="tjmax" />{autoText.slice(1).join('Tjmax')}{/if}</span>
            <button type="button" class="action" onclick={() => send({ cpuStopC: 95 })}>{t('settings.performance.cpuStop.set')}</button>
          {:else}
            <NumberInput
              id="performance-cpu-stop-input"
              integer
              value={current.cpuStopC}
              unit="°C"
              invalid={errorOf('performance.cpuStopC') !== null}
              describedBy={errorOf('performance.cpuStopC') !== null ? 'performance-cpu-stop-error' : undefined}
              onCommit={(cpuStopC) => send({ cpuStopC })}
            />
            <button type="button" class="action" onclick={() => send({ cpuStopC: null })}>{t('settings.performance.cpuStop.useAuto')}</button>
          {/if}
        </div>
      {/snippet}
    </Field>
    <Field id="performance-gpu-stop" labelFor="performance-gpu-stop-input" label={t('settings.performance.gpuStopC')} description={t('settings.performance.gpuStopC.hint')} error={errorOf('performance.gpuStopC')}>
      {#snippet control()}
        <NumberInput
          id="performance-gpu-stop-input"
          integer
          value={current.gpuStopC}
          unit="°C"
          invalid={errorOf('performance.gpuStopC') !== null}
          describedBy={errorOf('performance.gpuStopC') !== null ? 'performance-gpu-stop-error' : undefined}
          onCommit={(gpuStopC) => send({ gpuStopC })}
        />
      {/snippet}
    </Field>
    <Field id="performance-disk-stop" labelFor={current.diskStopC === null ? null : 'performance-disk-stop-input'} label={t('settings.performance.diskStopC')} description={t('settings.performance.diskStopC.hint')} error={errorOf('performance.diskStopC')}>
      {#snippet control()}
        <div class="buttons">
          {#if current.diskStopC === null}
            <span class="auto">{t('settings.performance.diskStopC.auto')}</span>
            <button type="button" class="action" onclick={() => send({ diskStopC: 70 })}>{t('settings.performance.diskStopC.set')}</button>
          {:else}
            <NumberInput
              id="performance-disk-stop-input"
              integer
              value={current.diskStopC}
              unit="°C"
              invalid={errorOf('performance.diskStopC') !== null}
              describedBy={errorOf('performance.diskStopC') !== null ? 'performance-disk-stop-error' : undefined}
              onCommit={(diskStopC) => send({ diskStopC })}
            />
            <button type="button" class="action" onclick={() => send({ diskStopC: null })}>{t('settings.performance.diskStopC.useAuto')}</button>
          {/if}
        </div>
      {/snippet}
    </Field>
  </Group>

  <Group id="performance-test" title={t('settings.performance.group.test')}>
    <Segmented
      id="performance-first-error"
      label={t('settings.performance.firstError')}
      description={t('settings.performance.firstError.hint')}
      options={[
        { value: 'profile', label: t('settings.performance.firstError.profile') },
        { value: 'yes', label: t('settings.performance.firstError.yes') },
        { value: 'no', label: t('settings.performance.firstError.no') },
      ]}
      value={firstError}
      error={errorOf('performance.stopOnFirstError')}
      onChange={(next) => send({ stopOnFirstError: FIRST_ERROR[next] })}
    />
    <Field id="performance-ram" label={t('settings.performance.ramShare')} term="ramShare" labelFor="performance-ram-input" description={t('settings.performance.ramShare.hint')} error={errorOf('performance.ramSharePercent')}>
      {#snippet control()}
        <NumberInput
          id="performance-ram-input"
          integer
          value={current.ramSharePercent}
          unit="%"
          invalid={errorOf('performance.ramSharePercent') !== null}
          describedBy={errorOf('performance.ramSharePercent') !== null ? 'performance-ram-error' : undefined}
          onCommit={(ramSharePercent) => send({ ramSharePercent })}
        />
      {/snippet}
    </Field>
    <Field id="performance-notice" label={t('settings.performance.riskNotice')}>
      {#snippet control()}
        <button type="button" class="action" disabled={!current.riskNoticeSeen} onclick={() => send({ riskNoticeSeen: false })}>
          {t('settings.performance.riskNotice.reset')}
        </button>
      {/snippet}
    </Field>
  </Group>
{/if}

<style>
  .buttons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    justify-content: flex-end;
  }
  .auto {
    font-size: 13px;
    color: var(--text-muted);
  }
  .ask {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    font-size: 13px;
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
  .action.danger {
    border-color: color-mix(in srgb, var(--crit) 60%, var(--border));
  }
</style>
