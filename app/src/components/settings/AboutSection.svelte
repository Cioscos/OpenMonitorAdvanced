<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import { settings } from '../../lib/settings.svelte';
  import { updates } from '../../lib/updates.svelte';
  import type { AppInfo, KnownPath } from '../../lib/types';
  import Group from './controls/Group.svelte';

  let { backend }: { backend: Backend } = $props();

  let info = $state<AppInfo | null>(null);
  /** The target whose last open failed, with the system's text. */
  let failure = $state<{ target: KnownPath; text: string } | null>(null);

  onMount(() => {
    backend
      .getAppInfo()
      .then((value) => (info = value))
      .catch((error) => console.error('app info unavailable', error));
  });

  const update = $derived(updates.state);
  const checking = $derived(update?.state === 'checking');
  const readOnly = $derived(settings.state?.persistence.kind === 'readOnly');
  const statusText = $derived.by(() => {
    if (update === null) return null;
    switch (update.state) {
      case 'checking':
        return t('settings.about.checking');
      case 'upToDate':
        return t('settings.about.upToDate', { version: update.current });
      case 'available':
        return t('settings.about.available', { version: update.latest?.version ?? '' });
      case 'error':
        return t(`settings.about.error.${update.error ?? 'invalid'}`);
      default:
        return null;
    }
  });
  const isError = $derived(update?.state === 'error');
  const lastChecked = $derived(
    update?.checkedAtMs == null
      ? null
      : t('settings.about.lastChecked', {
          time: new Intl.DateTimeFormat(i18n.locale, { dateStyle: 'medium', timeStyle: 'short' }).format(update.checkedAtMs),
        }),
  );
  /** A newer release is known, also when the last check failed. */
  const newer = $derived(update?.latest ?? null);
  let releaseFailure = $state<string | null>(null);

  async function openRelease() {
    releaseFailure = null;
    try {
      await backend.openReleasePage();
    } catch (error) {
      releaseFailure = t('settings.openFailed', { reason: String(error) });
    }
  }

  /** The last export: the saved file's name or the error text; a cancelled dialog shows nothing. */
  let report = $state<{ saved: string } | { failed: string } | null>(null);
  let exporting = $state(false);
  let revealFailure = $state<string | null>(null);

  async function exportReport() {
    exporting = true;
    revealFailure = null;
    try {
      const saved = await backend.exportSensorReport();
      report = saved === null ? null : { saved: saved.fileName };
    } catch (error) {
      report = { failed: String(error) };
    } finally {
      exporting = false;
    }
  }

  async function revealReport() {
    revealFailure = null;
    try {
      await backend.revealSensorReport();
    } catch (error) {
      revealFailure = t('settings.openFailed', { reason: String(error) });
    }
  }

  async function open(target: KnownPath) {
    failure = null;
    try {
      await backend.openKnownPath(target);
    } catch (error) {
      failure = { target, text: t('settings.openFailed', { reason: String(error) }) };
    }
  }
</script>

{#snippet failed(target: KnownPath)}
  {#if failure?.target === target}<p class="failed" role="alert">{failure.text}</p>{/if}
{/snippet}

{#snippet folder(label: string, path: string | null, target: KnownPath)}
  <div class="row">
    <dt id="about-{target}">{label}</dt>
    <dd>
      <span class="path">{path ?? t('settings.about.unavailable')}</span>
      <!-- Named "Open folder" plus the folder, so each button says which one it opens. -->
      <button
        type="button"
        id="about-{target}-open"
        aria-labelledby="about-{target}-open about-{target}"
        disabled={path === null}
        onclick={() => open(target)}>{t('settings.about.openFolder')}</button
      >
      {@render failed(target)}
    </dd>
  </div>
{/snippet}

<Group id="about" title={t('app.title')}>
  <dl>
    <div class="row">
      <dt>{t('settings.about.version')}</dt>
      <dd class="value">{info?.version ?? ''}</dd>
    </div>
    <div class="row">
      <dt>{t('settings.about.serviceVersion')}</dt>
      <dd class="value">
        {#if info}
          {#if info.serviceVersion !== null}{info.serviceVersion}{:else}<span class="muted">{t('settings.about.serviceUnknown')}</span>{/if}
        {/if}
      </dd>
    </div>
    <div class="row">
      <dt>{t('settings.about.protocol')}</dt>
      <dd class="value">{info?.protocolVersion ?? ''}</dd>
    </div>
    <div class="row">
      <dt>{t('settings.about.license')}</dt>
      <dd>
        <span class="value">GPL-3.0-or-later</span>
        <button type="button" onclick={() => open('thirdPartyNotices')}>{t('settings.about.thirdParty')}</button>
        {@render failed('thirdPartyNotices')}
      </dd>
    </div>
    <div class="row">
      <dt>{t('settings.about.updates')}</dt>
      <dd>
        <button type="button" disabled={checking || update === null} onclick={() => updates.check()}>{t('settings.about.checkNow')}</button>
        <span class="status" class:error={isError} role="status">
          {#if statusText !== null}{statusText}{/if}
        </span>
        {#if newer !== null}
          <button type="button" onclick={openRelease}>{t('settings.about.releasePage')}</button>
        {/if}
        {#if lastChecked !== null}<span class="muted when">{lastChecked}</span>{/if}
        {#if releaseFailure !== null}<p class="failed" role="alert">{releaseFailure}</p>{/if}
        <label class="auto">
          <input
            type="checkbox"
            checked={settings.state?.settings.updates.checkAutomatically ?? false}
            disabled={readOnly || settings.state === null}
            aria-describedby="about-updates-note"
            onchange={(event) => {
              const box = event.currentTarget;
              void settings.update({ updates: { checkAutomatically: box.checked } }).then(() => {
                // The box shows the state in effect, never a guess.
                box.checked = settings.state?.settings.updates.checkAutomatically ?? false;
              });
            }}
          />
          {t('settings.about.checkAutomatically')}
        </label>
        <p id="about-updates-note" class="note">{t('settings.about.updatesNote')}</p>
      </dd>
    </div>
    <div class="row">
      <dt>{t('settings.about.report')}</dt>
      <dd>
        <button type="button" disabled={exporting} aria-describedby="about-report-note" onclick={exportReport}
          >{t('settings.about.exportReport')}</button
        >
        {#if report !== null && 'saved' in report}
          <span class="status" role="status">{t('settings.about.reportSaved', { file: report.saved })}</span>
          <button type="button" onclick={revealReport}>{t('settings.about.openFolder')}</button>
        {/if}
        {#if report !== null && 'failed' in report}<p class="failed" role="alert">{report.failed}</p>{/if}
        {#if revealFailure !== null}<p class="failed" role="alert">{revealFailure}</p>{/if}
        <p id="about-report-note" class="note">{t('settings.about.reportNote')}</p>
      </dd>
    </div>
    {@render folder(t('settings.about.settingsFolder'), info?.settingsPath ?? null, 'settingsFolder')}
    {@render folder(t('settings.about.logsFolder'), info?.logsPath ?? null, 'logsFolder')}
  </dl>
</Group>

<style>
  dl {
    margin: 0;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(120px, 200px) minmax(0, 1fr);
    align-items: baseline;
    gap: 6px 24px;
    padding: 12px 16px;
  }
  .row + .row {
    border-top: 1px solid var(--border);
  }
  dt {
    font-weight: 600;
  }
  dd {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 8px 14px;
    min-width: 0;
    margin: 0;
  }
  .value {
    font-variant-numeric: tabular-nums;
    user-select: text;
  }
  .path {
    flex: 1 1 240px;
    min-width: 0;
    font-size: 13px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
    user-select: text;
  }
  .muted {
    color: var(--text-muted);
  }
  button {
    flex: none;
    padding: 4px 12px;
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  button:hover:not(:disabled) {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .status.error {
    color: var(--crit);
  }
  .when {
    font-size: 13px;
  }
  .auto {
    flex-basis: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .auto:has(input:disabled) {
    cursor: not-allowed;
    opacity: 0.6;
  }
  .note {
    flex-basis: 100%;
    margin: 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .failed {
    flex-basis: 100%;
    margin: 0;
    font-size: 12.5px;
    color: var(--crit);
  }
  @media (max-width: 560px) {
    .row {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
