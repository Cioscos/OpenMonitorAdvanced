<script lang="ts">
  import { propertyRows } from '../../lib/advanced/pages';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { Device } from '../../lib/types';

  let { devices }: { devices: Device[] } = $props();
</script>

<section class="info">
  <h3 class="label">{t('advanced.info.title')}</h3>
  {#each devices as device (device.id)}
    <dl>
      {#each propertyRows(device, i18n.locale, t) as row (row.key)}
        <dt>{row.label}</dt>
        <dd>{row.value}</dd>
      {/each}
    </dl>
  {/each}
</section>

<style>
  .info {
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  h3 {
    margin: 0 0 8px;
    font-weight: 400;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    margin: 0;
    font-size: 13px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
</style>
