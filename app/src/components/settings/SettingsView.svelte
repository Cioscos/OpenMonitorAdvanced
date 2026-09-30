<script lang="ts">
  import type { Backend } from '../../lib/backend';
  import { t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import { settings } from '../../lib/settings.svelte';
  import type { ServiceStatus } from '../../lib/types';
  import type { SettingsTarget } from '../../lib/view';
  import AboutSection from './AboutSection.svelte';
  import GeneralSection from './GeneralSection.svelte';
  import PersistenceNotice from './PersistenceNotice.svelte';
  import RulesSection from './RulesSection.svelte';
  import SourcesSection from './SourcesSection.svelte';

  // The settings screen (spec M5 §2.7): sections on the left, the chosen one on the right. Every
  // control sends its change at once; there is no Save button. The CSV log arrives with M5c.
  let {
    store,
    backend,
    service,
    target = null,
    onBack,
  }: { store: LiveStore; backend: Backend; service: ServiceStatus | null; target?: SettingsTarget | null; onBack: () => void } = $props();

  type Section = 'general' | 'rules' | 'sources' | 'about';
  const SECTIONS: Section[] = ['general', 'rules', 'sources', 'about'];
  // Opened on a target (a sensor row's "Create rule…"), the screen starts on that section.
  // svelte-ignore state_referenced_locally
  let section = $state<Section>(target?.section ?? 'general');
  // The sensor to prefill "New rule" with is handed over once: after the first section change a
  // remounted Rules section starts without a draft.
  // svelte-ignore state_referenced_locally
  let newRuleSensor = $state(target?.newRuleSensor);

  function show(id: Section) {
    section = id;
    newRuleSensor = undefined;
  }
</script>

<div class="settings">
  <nav aria-label={t('settings.sections')}>
    <button type="button" class="back" onclick={onBack}>
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
        <path d="M10 3 5 8l5 5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      {t('settings.back')}
    </button>
    <p class="title">{t('settings.title')}</p>
    {#each SECTIONS as id (id)}
      <button
        type="button"
        class="entry"
        class:on={section === id}
        aria-current={section === id ? 'page' : undefined}
        onclick={() => show(id)}
      >
        {t(`settings.section.${id}`)}
      </button>
    {/each}
  </nav>

  <section class="content" aria-labelledby="settings-section-title">
    <h2 id="settings-section-title">{t(`settings.section.${section}`)}</h2>
    {#if settings.state}
      <PersistenceNotice persistence={settings.state.persistence} />
    {/if}
    {#if section === 'general'}
      <GeneralSection {store} {backend} />
    {:else if section === 'rules'}
      <RulesSection {store} {backend} {newRuleSensor} />
    {:else if section === 'sources'}
      <SourcesSection {store} {backend} {service} />
    {:else}
      <AboutSection {backend} />
    {/if}
  </section>
</div>

<style>
  .settings {
    display: grid;
    grid-template-columns: 180px minmax(0, 1fr);
    gap: 24px;
    align-items: start;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
    position: sticky;
    top: 76px;
  }
  .back {
    display: inline-flex;
    align-items: center;
    align-self: flex-start;
    gap: 6px;
    margin-bottom: 10px;
    padding: 5px 12px 5px 8px;
    font-size: 13px;
    color: var(--text-muted);
    cursor: pointer;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .back:hover {
    color: var(--text);
  }
  .title {
    margin: 0 0 4px;
    padding: 0 12px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .entry {
    padding: 8px 12px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    border-radius: 0 8px 8px 0;
  }
  .entry:hover {
    background: var(--surface);
  }
  .entry.on {
    background: var(--surface-2);
    border-left-color: var(--accent);
  }
  .back:focus-visible,
  .entry:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .content {
    display: flex;
    flex-direction: column;
    gap: 20px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 20px;
  }
  @media (max-width: 720px) {
    .settings {
      grid-template-columns: minmax(0, 1fr);
    }
    nav {
      position: static;
      flex-direction: row;
      flex-wrap: wrap;
      align-items: center;
    }
    .back {
      margin: 0 8px 0 0;
    }
    .title {
      display: none;
    }
    .entry {
      border-left: 0;
      border-bottom: 2px solid transparent;
      border-radius: 8px 8px 0 0;
    }
    .entry.on {
      border-bottom-color: var(--accent);
    }
  }
</style>
