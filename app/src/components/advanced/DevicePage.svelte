<script lang="ts">
  import type { SidebarEntry } from '../../lib/advanced/nav';
  import type { Backend } from '../../lib/backend';
  import { formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';

  // Minimal page: the live value of every sensor of the section. The full device page
  // (KPIs, history chart, sensor table, device info) replaces this file with the same props.
  let { entry, store }: { entry: SidebarEntry; store: LiveStore; backend: Backend } = $props();

  const sensors = $derived(store.schema?.sensors.filter((s) => entry.deviceIds.includes(s.deviceId)) ?? []);
</script>

<ul class="sensors">
  {#each sensors as sensor (sensor.id)}
    <li>
      <span>{t(`sensor.${sensor.label.key}`, { arg: sensor.label.arg ?? '' })}</span>
      <span class="value">{formatValue(store.value(sensor.id), sensor.unit, i18n.locale, t)}</span>
    </li>
  {/each}
</ul>

<style>
  .sensors {
    margin: 0;
    padding: 0;
    list-style: none;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 14px;
    border-top: 1px solid var(--border);
  }
  li:first-child {
    border-top: 0;
  }
  .value {
    font-variant-numeric: tabular-nums;
  }
</style>
