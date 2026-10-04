<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { SidebarEntry } from '../../lib/advanced/nav';
  import { defaultSeries, kpisFor, propertyRows } from '../../lib/advanced/pages';
  import { StatsPoller } from '../../lib/advanced/statsPoller.svelte';
  import type { Backend } from '../../lib/backend';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { ServiceStatus } from '../../lib/types';
  import { display } from '../../lib/units.svelte';
  import DeviceInfo from './DeviceInfo.svelte';
  import GpuProcesses from './GpuProcesses.svelte';
  import HistoryChart from './HistoryChart.svelte';
  import KpiRow from './KpiRow.svelte';
  import SensorTable from './SensorTable.svelte';
  import ServiceNotice from './ServiceNotice.svelte';

  // Pages whose extra sensors (temperatures, disk health…) come only from the sensor service.
  const SERVICE_NOTICE_KINDS: SidebarEntry['kind'][] = ['cpu', 'memory', 'storage'];

  // One Advanced page (spec §7.3). AdvancedView renders the heading and re-keys this
  // component per section, so timers and polling start from scratch on every page.
  let {
    entry,
    store,
    backend,
    service = null,
  }: { entry: SidebarEntry; store: LiveStore; backend: Backend; service?: ServiceStatus | null } = $props();

  const showServiceNotice = $derived(
    service !== null && service.state !== 'connected' && SERVICE_NOTICE_KINDS.includes(entry.kind),
  );

  const schema = $derived(store.schema);
  const devices = $derived(schema?.devices.filter((d) => entry.deviceIds.includes(d.id)) ?? []);
  const sensors = $derived(schema?.sensors.filter((s) => entry.deviceIds.includes(s.deviceId)) ?? []);
  const kpis = $derived(schema ? kpisFor(entry.kind, schema, entry.deviceIds) : []);
  const defaults = $derived(schema ? defaultSeries(entry.kind, schema, entry.deviceIds) : []);
  const hasProperties = $derived(devices.some((d) => propertyRows(d, i18n.locale, t).length > 0));
  // Network traffic follows the throughput setting, like the Simple view's network tile; disks
  // and everything else stay in bytes. The KPIs, the table and the chart all take `rate`.
  const rate = $derived(entry.kind === 'network' ? display.throughput : 'bytes');
  const valueOf = (id: string) => store.value(id);
  const qualityOf = (id: string) => store.quality(id);
  // A disk that is not being read says so; `active` and `unknown` say nothing. The state is read
  // from the store, which follows hot-plug and wake-up, never from a discovery property.
  const power = $derived(entry.kind === 'storage' ? store.diskPower(entry.deviceIds[0]) : undefined);
  // The backend of a mounted page never changes.
  const stats = new StatsPoller(untrack(() => backend), () => sensors.map((s) => s.id), () => schema?.revision ?? null);

  onMount(() => stats.start());
</script>

{#if schema}
  <div class="page">
    {#if showServiceNotice}<ServiceNotice />{/if}
    {#if power === 'standby' || power === 'idle'}
      <div><span class="tag disk-state">{t(`storage.power.${power}`)}</span></div>
    {/if}
    <KpiRow {kpis} {valueOf} {qualityOf} statsOf={stats.statsOf} {rate} />
    <HistoryChart sectionId={entry.id} {sensors} {defaults} {schema} {store} {backend} {rate} />
    <SensorTable {sensors} {valueOf} {qualityOf} {stats} {rate} />
    {#if hasProperties || entry.kind === 'gpu'}
      <div class="extra">
        {#if hasProperties}<DeviceInfo {devices} />{/if}
        {#if entry.kind === 'gpu'}<GpuProcesses deviceId={entry.deviceIds[0]} {backend} />{/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  .tag {
    padding: 1px 6px;
    font-size: 10px;
    letter-spacing: 0.06em;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .extra {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 14px;
    align-items: start;
  }
</style>
