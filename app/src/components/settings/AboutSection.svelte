<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend';
  import { t } from '../../lib/i18n/index.svelte';
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
