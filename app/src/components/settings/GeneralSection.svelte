<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import { openFailureText } from '../../lib/openFailure';
  import { settings } from '../../lib/settings.svelte';
  import { INTERVALS_MS, iconSensorChoices, reasonText } from '../../lib/settingsView';
  import type { AutostartStatus, ChartFps, DefaultView, Language, SettingsPatch, TemperatureUnit, ThroughputUnit } from '../../lib/types';
  import Group from './controls/Group.svelte';
  import Segmented from './controls/Segmented.svelte';
  import SelectField from './controls/SelectField.svelte';
  import Toggle from './controls/Toggle.svelte';

  let { store, backend }: { store: LiveStore; backend: Backend } = $props();

  const current = $derived(settings.state?.settings ?? null);
  const autostartEffect = $derived(settings.state?.applyStatus.autostart ?? null);
  /** What Windows makes of the start-up entry, read when the section opens and after a change. */
  let autostart = $state<AutostartStatus | null>(null);
  let openError = $state<string | null>(null);

  const errorOf = (field: string) => {
    const key = settings.errors[field];
    return key === undefined ? null : t(key);
  };
  const send = (patch: SettingsPatch) => settings.update(patch);

  async function refreshAutostart() {
    try {
      autostart = await backend.refreshAutostart();
    } catch (error) {
      console.error('cannot read the start-up entry', error);
    }
  }
  onMount(() => {
    void refreshAutostart();
  });

  async function setAutostart(enabled: boolean) {
    const taken = await send({ tray: { autostart: enabled } });
    if (taken) await refreshAutostart();
    return taken;
  }

  async function openStartupSettings() {
    openError = null;
    try {
      await backend.openKnownPath('startupAppsSettings');
    } catch (error) {
      openError = openFailureText(error);
    }
  }

  const seconds = $derived(new Intl.NumberFormat(i18n.locale, { maximumFractionDigits: 1 }));
  const intervals = $derived(
    INTERVALS_MS.map((ms) => ({ value: String(ms), label: t('settings.general.interval.value', { seconds: seconds.format(ms / 1000) }) })),
  );
  const sensorGroups = $derived(iconSensorChoices(store.schema, t));
  const iconSensorItems = $derived.by(() => {
    const selected = current?.tray.iconSensor ?? null;
    const known = selected === null || sensorGroups.some((g) => g.options.some((o) => o.value === selected));
    return [
      { value: '', label: t('settings.general.iconSensor.auto') },
      // A sensor that is not in the schema now (e.g. the service is off) stays selected.
      ...(known ? [] : [{ value: selected, label: t('settings.general.iconSensor.missing') }]),
      ...sensorGroups,
    ];
  });
  const windowsNote = $derived.by(() => {
    if (!current?.tray.autostart || autostart === null) return null;
    if (autostart.effective === 'disabledByWindows') return t('settings.general.autostart.disabledByWindows');
    if (autostart.effective === 'unknown') return t('settings.general.autostart.managedByWindows');
    return null;
  });
</script>

{#if current}
  <Group id="general-display" title={t('settings.general.group.display')}>
    <Segmented
      id="language"
      label={t('settings.general.language')}
      options={[
        { value: 'system' as Language, label: t('settings.general.language.system') },
        { value: 'en' as Language, label: t('settings.general.language.en') },
        { value: 'it' as Language, label: t('settings.general.language.it') },
      ]}
      value={current.general.language}
      error={errorOf('general.language')}
      onChange={(language) => send({ general: { language } })}
    />
    <Segmented
      id="temperature-unit"
      label={t('settings.general.temperature')}
      options={[
        { value: 'c' as TemperatureUnit, label: t('settings.general.temperature.c') },
        { value: 'f' as TemperatureUnit, label: t('settings.general.temperature.f') },
      ]}
      value={current.general.temperatureUnit}
      error={errorOf('general.temperatureUnit')}
      onChange={(temperatureUnit) => send({ general: { temperatureUnit } })}
    />
    <Segmented
      id="throughput-unit"
      label={t('settings.general.throughput')}
      options={[
        { value: 'bits' as ThroughputUnit, label: t('settings.general.throughput.bits') },
        { value: 'bytes' as ThroughputUnit, label: t('settings.general.throughput.bytes') },
      ]}
      value={current.general.throughputUnit}
      error={errorOf('general.throughputUnit')}
      onChange={(throughputUnit) => send({ general: { throughputUnit } })}
    />
  </Group>

  <Group id="general-refresh" title={t('settings.general.group.refresh')}>
    <SelectField
      id="interval"
      label={t('settings.general.interval')}
      description={t('settings.general.interval.hint')}
      items={intervals}
      value={String(current.general.intervalMs)}
      error={errorOf('general.intervalMs')}
      onChange={(ms) => send({ general: { intervalMs: Number(ms) } })}
    />
    <Segmented
      id="chart-fps"
      label={t('settings.general.fps')}
      options={([60, 30, 15] as ChartFps[]).map((fps) => ({
        value: fps,
        label: t('settings.general.fps.value', { fps }),
        note: fps === 60 ? t('settings.general.fps.hint60') : undefined,
      }))}
      value={current.general.chartFps}
      error={errorOf('general.chartFps')}
      onChange={(chartFps) => send({ general: { chartFps } })}
    />
  </Group>

  <Group id="general-startup" title={t('settings.general.group.startup')}>
    <Segmented
      id="default-view"
      label={t('settings.general.defaultView')}
      description={t('settings.general.defaultView.hint')}
      options={(['simple', 'advanced', 'last'] as DefaultView[]).map((view) => ({ value: view, label: t(`settings.general.defaultView.${view}`) }))}
      value={current.general.defaultView}
      error={errorOf('general.defaultView')}
      onChange={(defaultView) => send({ general: { defaultView } })}
    />
    <Toggle
      id="autostart"
      label={t('settings.general.autostart')}
      description={t('settings.general.autostart.hint')}
      checked={current.tray.autostart}
      error={errorOf('tray.autostart')}
      notesIds={windowsNote !== null ? ['autostart-windows'] : []}
      onChange={setAutostart}
    >
      {#snippet notes()}
        {#if windowsNote !== null}
          <div class="windows">
            <p id="autostart-windows">{windowsNote}</p>
            <button type="button" class="link" onclick={openStartupSettings}>{t('settings.general.autostart.openStartupSettings')}</button>
          </div>
        {/if}
        {#if openError !== null}<p class="failed">{openError}</p>{/if}
        {#if autostartEffect?.kind === 'failed'}
          <p class="failed">{t('settings.general.autostart.failed', { reason: reasonText(autostartEffect.reason, t) })}</p>
        {/if}
      {/snippet}
    </Toggle>
  </Group>

  <Group id="general-tray" title={t('settings.general.group.tray')}>
    <Toggle
      id="close-to-tray"
      label={t('settings.general.closeToTray')}
      description={t('settings.general.closeToTray.hint')}
      checked={current.tray.closeToTray}
      error={errorOf('tray.closeToTray')}
      onChange={(closeToTray) => send({ tray: { closeToTray } })}
    />
    <SelectField
      id="icon-sensor"
      label={t('settings.general.iconSensor')}
      description={t('settings.general.iconSensor.hint')}
      items={iconSensorItems}
      value={current.tray.iconSensor ?? ''}
      error={errorOf('tray.iconSensor')}
      onChange={(id) => send({ tray: { iconSensor: id === '' ? null : id } })}
    />
  </Group>
{/if}

<style>
  .windows {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 12px;
    padding: 8px 10px;
    border-left: 2px solid var(--warn);
    background: color-mix(in srgb, var(--warn) 6%, transparent);
    border-radius: 0 8px 8px 0;
  }
  .windows p {
    flex: 1 1 260px;
    color: var(--text);
  }
  .failed {
    color: var(--crit);
  }
  .link {
    padding: 0;
    font-size: 12.5px;
    color: var(--accent-2);
    cursor: pointer;
    background: none;
    border: 0;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: 2px;
  }
</style>
