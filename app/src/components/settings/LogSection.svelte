<script lang="ts">
  import type { Backend } from '../../lib/backend';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import { log } from '../../lib/log.svelte';
  import { folderErrorText } from '../../lib/log/messages';
  import { settings } from '../../lib/settings.svelte';
  import type { LogEveryTicks, SettingsPatch } from '../../lib/types';
  import HotkeyInput from './HotkeyInput.svelte';
  import Field from './controls/Field.svelte';
  import Group from './controls/Group.svelte';
  import NumberInput from './controls/NumberInput.svelte';
  import Segmented from './controls/Segmented.svelte';
  import Toggle from './controls/Toggle.svelte';
  import SensorTree from './SensorTree.svelte';

  // Settings › CSV log (spec M5c): where the files go, which sensors, how often, how big, and the
  // global shortcuts. Every control sends its change at once, like the other sections.
  let { store, backend }: { store: LiveStore; backend: Backend } = $props();

  const EVERY_TICKS: LogEveryTicks[] = [1, 2, 5, 10, 30, 60];

  const current = $derived(settings.state?.settings.log ?? null);
  const intervalMs = $derived(settings.state?.settings.general.intervalMs ?? 1000);
  const selected = $derived(new Set(current?.sensors ?? []));
  const hotkeys = $derived(log.status?.hotkeys ?? null);
  let folderError = $state<string | null>(null);

  const errorOf = (field: string) => {
    const key = settings.errors[field];
    return key === undefined ? null : t(key);
  };
  const send = (patch: SettingsPatch) => settings.update(patch);
  /** A failed call leaves the hotkeys as they were: nothing to show for it. */
  const suspendHotkeys = (suspended: boolean) => backend.setLogHotkeysSuspended(suspended).catch(() => {});

  async function chooseFolder() {
    folderError = null;
    try {
      const picked = await backend.pickLogFolder();
      // Cancelling changes nothing.
      if (picked !== null) await send({ log: { folder: picked } });
    } catch (error) {
      folderError = folderErrorText(error, t);
    }
  }

  async function openFolder() {
    folderError = null;
    try {
      await backend.openLogFolder();
    } catch (error) {
      folderError = folderErrorText(error, t);
    }
  }

  /** "Every all" starts the list from every sensor there is now, so unticking a few is easy. */
  function setAllSensors(all: boolean) {
    return send({ log: { sensors: all ? null : (store.schema?.sensors.map((s) => s.id) ?? []) } });
  }

  const number = $derived(new Intl.NumberFormat(i18n.locale, { maximumFractionDigits: 1 }));
  function everyText(ticks: number): string {
    const ms = intervalMs * ticks;
    return ms < 60_000
      ? t('settings.log.time.seconds', { value: number.format(ms / 1000) })
      : t('settings.log.time.minutes', { value: number.format(ms / 60_000) });
  }
</script>

{#if current}
  <Group id="log-folder-group" title={t('settings.log.group.folder')}>
    <Field id="log-folder" label={t('settings.log.folder')} description={t('settings.log.folder.hint')} error={folderError ?? errorOf('log.folder')}>
      {#snippet control()}
        <div class="buttons">
          <button type="button" class="action" onclick={chooseFolder}>{t('settings.log.folder.choose')}</button>
          <button type="button" class="action" onclick={openFolder}>{t('settings.log.folder.open')}</button>
          <button type="button" class="action" disabled={current.folder === null} onclick={() => send({ log: { folder: null } })}>
            {t('settings.log.folder.reset')}
          </button>
        </div>
      {/snippet}
      {#snippet notes()}
        <p class="path" class:default={current.folder === null}>{current.folder ?? t('settings.log.folder.default')}</p>
      {/snippet}
    </Field>
  </Group>

  <Group id="log-sensors" title={t('settings.log.group.sensors')}>
    <Toggle
      id="log-all-sensors"
      label={t('settings.log.sensors.all')}
      description={t('settings.log.sensors.all.hint')}
      checked={current.sensors === null}
      error={errorOf('log.sensors')}
      onChange={setAllSensors}
    />
    {#if current.sensors !== null && store.schema}
      <div class="tree">
        <SensorTree schema={store.schema} {selected} onChange={(next) => send({ log: { sensors: next } })} />
      </div>
    {/if}
  </Group>

  <Group id="log-files" title={t('settings.log.group.files')}>
    <Segmented
      id="log-every"
      label={t('settings.log.interval')}
      description={t('settings.log.interval.effective', { time: everyText(current.everyTicks) })}
      options={EVERY_TICKS.map((ticks) => ({ value: ticks, label: String(ticks) }))}
      value={current.everyTicks}
      error={errorOf('log.everyTicks')}
      onChange={(everyTicks) => send({ log: { everyTicks } })}
    />
    <Field id="log-max-size" label={t('settings.log.maxSize')} labelFor="log-max-size-input" description={t('settings.log.maxSize.hint')} error={errorOf('log.maxFileMb')}>
      {#snippet control()}
        <NumberInput
          id="log-max-size-input"
          integer
          value={current.maxFileMb}
          unit={t('settings.log.maxSize.unit')}
          invalid={errorOf('log.maxFileMb') !== null}
          describedBy={errorOf('log.maxFileMb') !== null ? 'log-max-size-error' : undefined}
          onCommit={(maxFileMb) => send({ log: { maxFileMb } })}
        />
      {/snippet}
    </Field>
  </Group>

  <Group id="log-hotkeys" title={t('settings.log.group.hotkeys')}>
    <HotkeyInput
      id="log-hotkey-toggle"
      label={t('settings.log.hotkey.toggle')}
      value={current.hotkeyToggle}
      status={hotkeys?.toggle ?? null}
      error={errorOf('log.hotkeyToggle')}
      onChange={(hotkeyToggle) => send({ log: { hotkeyToggle } })}
      onCapture={suspendHotkeys}
    />
    <HotkeyInput
      id="log-hotkey-pause"
      label={t('settings.log.hotkey.pause')}
      value={current.hotkeyPause}
      status={hotkeys?.pause ?? null}
      error={errorOf('log.hotkeyPause') ?? errorOf('log.hotkeyDuplicate')}
      onChange={(hotkeyPause) => send({ log: { hotkeyPause } })}
      onCapture={suspendHotkeys}
    />
    <p class="hint">{t('settings.log.hotkey.hint')}</p>
  </Group>
{/if}

<style>
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: flex-end;
  }
  .action {
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
  .path {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 12.5px;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .path.default {
    color: var(--text-muted);
  }
  .tree {
    padding: 12px 16px;
  }
  .hint {
    margin: 0;
    padding: 10px 16px;
    font-size: 12.5px;
    color: var(--text-muted);
  }
</style>
