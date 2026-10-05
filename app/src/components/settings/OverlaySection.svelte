<script lang="ts">
  import type { Backend } from '../../lib/backend';
  import { t } from '../../lib/i18n/index.svelte';
  import { overlay, retryVisible } from '../../lib/overlay.svelte';
  import { settings } from '../../lib/settings.svelte';
  import type { ChartFps, OverlayProfileEntry, OverlaySettings, OverlayStatus, SettingsPatch } from '../../lib/types';
  import HotkeyInput from './HotkeyInput.svelte';
  import Field from './controls/Field.svelte';
  import Group from './controls/Group.svelte';
  import Segmented from './controls/Segmented.svelte';
  import SelectField from './controls/SelectField.svelte';
  import Toggle from './controls/Toggle.svelte';

  // Settings › Overlay (spec M7 §9): the switch and the engine's state, the game being followed,
  // the profiles, how the overlay draws and measures, the per-game lists and the shortcuts. Every
  // control sends its change at once, like the other sections.
  let { backend }: { backend: Backend } = $props();

  const BUILTINS = ['builtin-minimal-fps', 'builtin-gaming', 'builtin-full', 'builtin-bar'];

  const current = $derived(settings.state?.settings.overlay ?? null);
  const status = $derived(overlay.status);
  const target = $derived(status?.target ?? null);
  /** The catalog the shell reported, or the built-ins until it does. */
  const profiles = $derived<OverlayProfileEntry[]>(
    status?.profiles ?? BUILTINS.map((id) => ({ id, name: `overlay.template.${id}`, builtin: true })),
  );

  const errorOf = (field: string) => {
    const key = settings.errors[field];
    return key === undefined ? null : t(key);
  };
  const send = (patch: { overlay: Partial<OverlaySettings> }) => settings.update(patch as SettingsPatch);
  /** A failed call leaves the hotkeys as they were: nothing to show for it. */
  const suspendHotkeys = (suspended: boolean) => backend.setLogHotkeysSuspended(suspended).catch(() => {});

  /** A built-in's name is a catalog key; a user profile's is its own; an unknown id shows as it is. */
  function profileName(id: string): string {
    const entry = profiles.find((p) => p.id === id);
    if (entry) return entry.builtin ? t(entry.name) : entry.name;
    return BUILTINS.includes(id) ? t(`overlay.template.${id}`) : id;
  }

  const profileItems = $derived.by(() => {
    const items = profiles.map((p) => ({ value: p.id, label: p.builtin ? t(p.name) : p.name }));
    // A default whose file went away still shows, so the list never claims another choice.
    if (current && !items.some((i) => i.value === current.defaultProfile)) {
      items.push({ value: current.defaultProfile, label: profileName(current.defaultProfile) });
    }
    return items;
  });

  /** The engine's text first, then the overlay process's own failure. */
  function stateLines(s: OverlayStatus): string[] {
    const lines = [t(`overlay.state.${s.frames}`)];
    if (s.process === 'failed') lines.push(t(s.processReason === 'incompatible' ? 'overlay.state.incompatible' : 'overlay.state.processFailed'));
    return lines;
  }

  type Tone = 'ok' | 'busy' | 'idle' | 'bad';
  function tone(s: OverlayStatus): Tone {
    if (s.process === 'failed') return 'bad';
    switch (s.frames) {
      case 'running':
        return 'ok';
      case 'starting':
        return 'busy';
      case 'off':
        return 'idle';
      default:
        return 'bad';
    }
  }

  const activeProfile = $derived(status?.activeProfile ?? current?.defaultProfile ?? null);
  /** The settings keep executables lowercase, while the process may report another case. */
  const targetExe = $derived(target?.name.toLowerCase() ?? null);
  const blocked = $derived(targetExe !== null && current !== null && current.blockedGames.includes(targetExe));

  function associate() {
    if (!current || targetExe === null || activeProfile === null) return;
    return send({ overlay: { gameProfiles: { ...current.gameProfiles, [targetExe]: activeProfile } } });
  }

  function block() {
    if (!current || targetExe === null || blocked) return;
    return send({ overlay: { blockedGames: [...current.blockedGames, targetExe] } });
  }

  function removeGameProfile(exe: string) {
    if (!current) return;
    const next = { ...current.gameProfiles };
    delete next[exe];
    return send({ overlay: { gameProfiles: next } });
  }

  function removeBlocked(exe: string) {
    if (!current) return;
    return send({ overlay: { blockedGames: current.blockedGames.filter((name) => name !== exe) } });
  }

  const gameProfiles = $derived(current ? Object.entries(current.gameProfiles).sort(([a], [b]) => a.localeCompare(b)) : []);
</script>

{#if current}
  <Group id="overlay-power" title={t('settings.overlay.group.power')}>
    <Toggle
      id="overlay-enabled"
      label={t('overlay.enabled')}
      description={t('overlay.enabled.hint')}
      checked={current.enabled}
      error={errorOf('overlay.enabled')}
      onChange={(enabled) => send({ overlay: { enabled } })}
    />
    {#if status}
      <div class="status" data-tone={tone(status)}>
        <span class="led" aria-hidden="true"></span>
        <div class="lines" role="status">
          {#each stateLines(status) as line (line)}
            <p>{line}</p>
          {/each}
        </div>
        {#if retryVisible(status)}
          <button type="button" class="action" onclick={() => overlay.retry().catch(() => {})}>{t('overlay.retry')}</button>
        {/if}
      </div>
    {/if}
  </Group>

  <Group id="overlay-game" title={t('settings.overlay.group.game')}>
    <div class="game">
      <p class="current" class:none={target === null}>
        {target === null ? t('overlay.current.none') : t('overlay.current', { name: target.name })}
      </p>
      <div class="buttons">
        <button type="button" class="action" disabled={target === null || activeProfile === null} onclick={associate}>
          {t('overlay.associate')}
        </button>
        <button type="button" class="action" disabled={target === null || blocked} onclick={block}>{t('overlay.block')}</button>
      </div>
    </div>
    {#if errorOf('overlay.gameProfiles') ?? errorOf('overlay.blockedGames')}
      <p class="error" role="alert">{errorOf('overlay.gameProfiles') ?? errorOf('overlay.blockedGames')}</p>
    {/if}
  </Group>

  <Group id="overlay-profiles" title={t('settings.overlay.group.profiles')}>
    <SelectField
      id="overlay-default-profile"
      label={t('overlay.defaultProfile')}
      items={profileItems}
      value={current.defaultProfile}
      error={errorOf('overlay.defaultProfile')}
      onChange={(defaultProfile) => send({ overlay: { defaultProfile } })}
    />
    <div class="reload">
      <button type="button" class="action" onclick={() => overlay.reloadProfiles().catch(() => {})}>{t('overlay.reload')}</button>
      {#if status && status.diagnostics.length > 0}
        <ul class="diagnostics">
          {#each status.diagnostics as d (d.file)}
            <li>{t('overlay.profileInvalid', { file: d.file, reason: d.reason })}</li>
          {/each}
        </ul>
      {/if}
    </div>
  </Group>

  <Group id="overlay-draw" title={t('settings.overlay.group.draw')}>
    <Segmented
      id="overlay-chart-fps"
      label={t('overlay.chartFps')}
      options={([15, 30, 60] as ChartFps[]).map((fps) => ({
        value: fps,
        label: t('settings.general.fps.value', { fps }),
        note: fps === 60 ? t('settings.general.fps.hint60') : undefined,
      }))}
      value={current.chartFps}
      error={errorOf('overlay.chartFps')}
      onChange={(chartFps) => send({ overlay: { chartFps } })}
    />
    <Segmented
      id="overlay-text-hz"
      label={t('overlay.textHz')}
      options={([2, 4] as const).map((n) => ({ value: n, label: t('overlay.textHz.value', { n }) }))}
      value={current.textHz}
      error={errorOf('overlay.textHz')}
      onChange={(textHz) => send({ overlay: { textHz } })}
    />
    <Segmented
      id="overlay-attach"
      label={t('overlay.attach')}
      options={(['window', 'monitor'] as const).map((attach) => ({ value: attach, label: t(`overlay.attach.${attach}`) }))}
      value={current.attach}
      error={errorOf('overlay.attach')}
      onChange={(attach) => send({ overlay: { attach } })}
    />
    <Toggle
      id="overlay-hide-from-capture"
      label={t('overlay.hideFromCapture')}
      checked={current.hideFromCapture}
      error={errorOf('overlay.hideFromCapture')}
      onChange={(hideFromCapture) => send({ overlay: { hideFromCapture } })}
    />
  </Group>

  <Group id="overlay-measure" title={t('settings.overlay.group.measure')}>
    <Toggle
      id="overlay-track-pc-latency"
      label={t('overlay.trackPcLatency')}
      description={t('overlay.trackPcLatency.hint')}
      checked={current.trackPcLatency}
      error={errorOf('overlay.trackPcLatency')}
      onChange={(trackPcLatency) => send({ overlay: { trackPcLatency } })}
    />
    <Toggle
      id="overlay-track-gpu"
      label={t('overlay.trackGpu')}
      description={t('overlay.trackGpu.hint')}
      checked={current.trackGpu}
      error={errorOf('overlay.trackGpu')}
      onChange={(trackGpu) => send({ overlay: { trackGpu } })}
    />
  </Group>

  <Group id="overlay-game-profiles" title={t('overlay.gameProfiles')}>
    {#if gameProfiles.length === 0}
      <p class="empty" aria-hidden="true">—</p>
    {:else}
      <ul class="entries" aria-label={t('overlay.gameProfiles')}>
        {#each gameProfiles as [exe, id] (exe)}
          <li>
            <span class="exe">{exe}</span>
            <span class="profile">{profileName(id)}</span>
            <button type="button" class="action" aria-label="{t('overlay.remove')} {exe}" onclick={() => removeGameProfile(exe)}>
              {t('overlay.remove')}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </Group>

  <Group id="overlay-blocked-games" title={t('overlay.blockedGames')}>
    {#if current.blockedGames.length === 0}
      <p class="empty" aria-hidden="true">—</p>
    {:else}
      <ul class="entries" aria-label={t('overlay.blockedGames')}>
        {#each current.blockedGames as exe (exe)}
          <li>
            <span class="exe">{exe}</span>
            <button type="button" class="action" aria-label="{t('overlay.remove')} {exe}" onclick={() => removeBlocked(exe)}>
              {t('overlay.remove')}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </Group>

  <Group id="overlay-hotkeys" title={t('settings.overlay.group.hotkeys')}>
    <HotkeyInput
      id="overlay-hotkey-toggle"
      label={t('overlay.hotkeyToggle')}
      value={current.hotkeyToggle}
      status={status?.hotkeys.toggle ?? null}
      error={errorOf('overlay.hotkeyToggle')}
      onChange={(hotkeyToggle) => send({ overlay: { hotkeyToggle } })}
      onCapture={suspendHotkeys}
    />
    <HotkeyInput
      id="overlay-hotkey-next-profile"
      label={t('overlay.hotkeyNextProfile')}
      value={current.hotkeyNextProfile}
      status={status?.hotkeys.nextProfile ?? null}
      error={errorOf('overlay.hotkeyNextProfile')}
      onChange={(hotkeyNextProfile) => send({ overlay: { hotkeyNextProfile } })}
      onCapture={suspendHotkeys}
    />
    <p class="hint">{t('settings.log.hotkey.hint')}</p>
  </Group>
{/if}

<style>
  .status {
    --tone: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
  }
  .status[data-tone='ok'] {
    --tone: var(--ok);
  }
  .status[data-tone='busy'] {
    --tone: var(--warn);
  }
  .status[data-tone='bad'] {
    --tone: var(--crit);
  }
  /* The one lit element of the page: a signal lamp in the tone of the engine. */
  .led {
    flex: none;
    width: 10px;
    height: 10px;
    background: var(--tone);
    border-radius: 50%;
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--tone) 18%, transparent);
  }
  .status[data-tone='ok'] .led {
    box-shadow:
      0 0 0 3px color-mix(in srgb, var(--tone) 18%, transparent),
      0 0 10px color-mix(in srgb, var(--tone) 70%, transparent);
  }
  .lines {
    flex: 1;
    min-width: 0;
  }
  .lines p {
    margin: 0;
    font-size: 13px;
    line-height: 1.45;
  }
  .lines p + p {
    color: var(--crit);
  }
  .status[data-tone='bad'] .lines p:first-child {
    color: var(--tone);
  }
  .game {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 24px;
    padding: 12px 16px;
  }
  .current {
    margin: 0;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .current.none {
    font-weight: 400;
    color: var(--text-muted);
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: flex-end;
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
  .reload {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 12px 16px;
  }
  .diagnostics {
    margin: 0;
    padding-left: 18px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--warn);
    overflow-wrap: anywhere;
  }
  .entries {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .entries li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 16px;
  }
  .entries li + li {
    border-top: 1px solid var(--border);
  }
  .exe {
    flex: 1;
    min-width: 0;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 12.5px;
    overflow-wrap: anywhere;
  }
  .profile {
    font-size: 13px;
    color: var(--text-muted);
  }
  .empty,
  .hint {
    margin: 0;
    padding: 10px 16px;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .error {
    margin: 0;
    padding: 0 16px 12px;
    font-size: 12.5px;
    color: var(--crit);
  }
</style>
