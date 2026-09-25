<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend/backend';
  import { DASH, formatBytes, formatPercent } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { GpuProcess } from '../../lib/types';

  let { deviceId, backend }: { deviceId: string; backend: Backend } = $props();

  const INTERVAL_MS = 2000;
  const locale = $derived(i18n.locale);
  let rows = $state.raw<GpuProcess[] | null>(null);
  let inFlight = false;

  async function refresh() {
    if (document.visibilityState === 'hidden' || inFlight) return;
    inFlight = true;
    try {
      rows = await backend.getGpuProcesses(deviceId);
    } catch (error) {
      console.error('GPU process list unavailable', error);
    } finally {
      inFlight = false;
    }
  }

  function loadText(p: GpuProcess): string {
    if (p.loadPercent === null) return DASH;
    const load = formatPercent(p.loadPercent, locale);
    return p.engine ? `${load} · ${p.engine}` : load;
  }

  onMount(() => {
    const onVisibility = () => {
      if (document.visibilityState === 'visible') void refresh();
    };
    document.addEventListener('visibilitychange', onVisibility);
    const timer = setInterval(() => void refresh(), INTERVAL_MS);
    void refresh();
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  });
</script>

<section class="processes">
  <h3 class="label">{t('advanced.processes.title')}</h3>
  {#if rows !== null && rows.length === 0}
    <p class="empty">{t('advanced.processes.empty')}</p>
  {:else if rows !== null}
    <table>
      <thead>
        <tr>
          <th scope="col">{t('advanced.processes.name')}</th>
          <th scope="col" class="num">{t('advanced.processes.load')}</th>
          <th scope="col" class="num">{t('advanced.processes.dedicated')}</th>
          <th scope="col" class="num">{t('advanced.processes.shared')}</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as p (p.pid)}
          <tr>
            <th scope="row">{p.name} <span class="pid">{p.pid}</span></th>
            <td class="num">{loadText(p)}</td>
            <td class="num">{formatBytes(p.dedicatedBytes, locale)}</td>
            <td class="num">{formatBytes(p.sharedBytes, locale)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  .processes {
    /* The table (nowrap numbers, long engine names) may be wider than its grid cell:
       it scrolls inside the box instead of widening the whole page. */
    min-width: 0;
    overflow-x: auto;
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  h3 {
    margin: 0 0 8px;
    font-weight: 400;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 4px 8px;
    text-align: left;
    font-weight: 400;
    border-bottom: 1px solid var(--border);
  }
  thead th {
    color: var(--text-muted);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .pid {
    margin-left: 4px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .empty {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
