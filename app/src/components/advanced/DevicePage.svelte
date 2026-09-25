<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { SidebarEntry } from '../../lib/advanced/nav';
  import { defaultSeries, kpisFor } from '../../lib/advanced/pages';
  import { StatsPoller } from '../../lib/advanced/statsPoller.svelte';
  import type { Backend } from '../../lib/backend';
  import type { LiveStore } from '../../lib/live.svelte';
  import DeviceInfo from './DeviceInfo.svelte';
  import GpuProcesses from './GpuProcesses.svelte';
  import HistoryChart from './HistoryChart.svelte';
  import KpiRow from './KpiRow.svelte';
  import SensorTable from './SensorTable.svelte';

  // One Advanced page (spec §7.3). AdvancedView renders the heading and re-keys this
  // component per section, so timers and polling start from scratch on every page.
  let { entry, store, backend }: { entry: SidebarEntry; store: LiveStore; backend: Backend } = $props();

  const schema = $derived(store.schema);
  const devices = $derived(schema?.devices.filter((d) => entry.deviceIds.includes(d.id)) ?? []);
  const sensors = $derived(schema?.sensors.filter((s) => entry.deviceIds.includes(s.deviceId)) ?? []);
  const kpis = $derived(schema ? kpisFor(entry.kind, schema, entry.deviceIds) : []);
  const defaults = $derived(schema ? defaultSeries(entry.kind, schema, entry.deviceIds) : []);
  const hasProperties = $derived(devices.some((d) => Object.keys(d.properties ?? {}).length > 0));
  // Network traffic in bit/s, the unit of the Simple view's network tile. `rate` only reaches
  // KpiRow and SensorTable below: HistoryChart keeps plotting the sensor's stored BytesPerSecond,
  // so the chart's axis/legend stay in byte/s until the M5 unit settings (docs/follow-ups.md).
  const rate = $derived(entry.kind === 'network' ? 'bits' : 'bytes');
  const valueOf = (id: string) => store.value(id);
  // The backend of a mounted page never changes.
  const stats = new StatsPoller(untrack(() => backend), () => sensors.map((s) => s.id), () => schema?.revision ?? null);

  onMount(() => stats.start());
</script>

{#if schema}
  <div class="page">
    <KpiRow {kpis} {valueOf} statsOf={stats.statsOf} {rate} />
    <HistoryChart sectionId={entry.id} {sensors} {defaults} {schema} {store} {backend} />
    <SensorTable {sensors} {valueOf} {stats} {rate} />
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
  .extra {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 14px;
    align-items: start;
  }
</style>
