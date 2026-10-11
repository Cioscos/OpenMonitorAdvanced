<script lang="ts">
  import { jedecCycles, jedecRate, parseMemoryTimings, type ChannelTimings, type MemoryModule } from '../../lib/advanced/memoryTimings';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { Device } from '../../lib/types';
  import Term from '../common/Term.svelte';

  let { devices }: { devices: Device[] } = $props();

  // The first device with usable properties; every slot name and number below renders as text.
  const info = $derived(devices.map((d) => parseMemoryTimings(d.properties)).find((i) => i !== null) ?? null);
  const timings = $derived(info?.timings ?? null);
  const modules = $derived(info?.modules ?? []);
  const channels = $derived(timings?.channels ?? []);

  const num = (n: number, digits = 1) => new Intl.NumberFormat(i18n.locale, { maximumFractionDigits: digits }).format(n);
  const cell = (n: number | undefined, suffix = '') => (n === undefined ? '–' : `${num(n)}${suffix}`);
  const flag = (v: boolean | undefined) => (v === undefined ? '–' : t(v ? 'advanced.memory.on' : 'advanced.memory.off'));
  const primary = (c: ChannelTimings) => [c.cl, c.trcdRd, c.trp, c.tras].map((v) => v ?? '–').join('-');
  // Each refresh row shows its own time: cycles / MCLK (the service's trfcNs belongs to one of them only).
  const withNs = (c: ChannelTimings, cycles: number | undefined) =>
    cycles === undefined ? '–' : `${num(cycles)}${c.mclkMhz ? ` (${num((cycles / c.mclkMhz) * 1000)} ns)` : ''}`;
  const trfc = (c: ChannelTimings) => withNs(c, c.trfc);
  const speed = (c: ChannelTimings) => (c.dataRate === undefined ? '–' : `${num(c.dataRate, 0)} MT/s`);
  const gen = $derived(timings?.generation ? timings.generation.toUpperCase() : '');

  /** Secondary timings in cycles, as (label, field) pairs; the glossary term is the field name. Fields a channel lacks show «–». */
  const SECONDARY: [string, keyof ChannelTimings][] = [
    ['tRCDWR', 'trcdWr'],
    ['tRRD_S', 'trrdS'],
    ['tRRD_L', 'trrdL'],
    ['tFAW', 'tfaw'],
    ['tWTR_S', 'twtrS'],
    ['tWTR_L', 'twtrL'],
    ['tCWL', 'tcwl'],
    ['tWR', 'twr'],
    ['tRTP', 'trtp'],
    ['tREFI', 'trefi'],
    ['tRFC2', 'trfc2'],
    ['tRFCsb', 'trfcSb'],
  ];
  const visibleSecondary = $derived(SECONDARY.filter(([, key]) => channels.some((c) => c[key] !== undefined)));

  const spd = (m: MemoryModule) => (m.tckMinNs ? jedecRate(m.tckMinNs) : undefined);
  const cycles = (m: MemoryModule) => {
    const tck = m.tckMinNs;
    if (!tck) return '–';
    return [m.taaNs, m.trcdNs, m.trpNs, m.trasNs].map((ns) => (ns ? jedecCycles(ns, tck) : '–')).join('-');
  };
  const limits = (m: MemoryModule) =>
    m.tempHighC === undefined && m.tempCritC === undefined ? '–' : `${cell(m.tempHighC)} / ${cell(m.tempCritC)} °C`;
</script>

{#if timings || modules.length > 0}
  <div class="memory">
    {#if timings}
      <section class="card">
        <h3 class="label">
          {t('advanced.memory.timings.title')}{#if gen}<span class="gen">{gen}</span>{/if}
        </h3>
        <table class="grid">
          {#if channels.length > 1}
            <thead>
              <tr>
                <th></th>
                {#each channels as c, i (i)}<th>{t('advanced.memory.channel', { name: c.channel ?? String(i + 1) })}</th>{/each}
              </tr>
            </thead>
          {/if}
          <tbody>
            <tr>
              <th scope="row">{t('advanced.memory.rate')}</th>
              {#each channels as c, i (i)}<td>{speed(c)}</td>{/each}
            </tr>
            <tr>
              <th scope="row"><Term term="mclk" /></th>
              {#each channels as c, i (i)}<td>{cell(c.mclkMhz, ' MHz')}</td>{/each}
            </tr>
            <tr class="primary">
              <th scope="row"><Term term="cl">CL</Term>-<Term term="trcd">tRCD</Term>-<Term term="trp">tRP</Term>-<Term term="tras">tRAS</Term></th>
              {#each channels as c, i (i)}<td class="big">{primary(c)}</td>{/each}
            </tr>
            <tr>
              <th scope="row"><Term term="trc" /></th>
              {#each channels as c, i (i)}<td>{cell(c.trc)}</td>{/each}
            </tr>
            <tr>
              <th scope="row"><Term term="trfc" /></th>
              {#each channels as c, i (i)}<td>{trfc(c)}</td>{/each}
            </tr>
            <tr>
              <th scope="row"><Term term="commandRate" /></th>
              {#each channels as c, i (i)}<td>{c.cmd2t === undefined ? '–' : c.cmd2t ? '2T' : '1T'}</td>{/each}
            </tr>
            <tr>
              <th scope="row"><Term term="gdm" /></th>
              {#each channels as c, i (i)}<td>{flag(c.gdm)}</td>{/each}
            </tr>
            <tr>
              <th scope="row"><Term term="refreshMode" /></th>
              {#each channels as c, i (i)}<td>{c.refreshMode ? t(`advanced.memory.refresh.${c.refreshMode}`) : '–'}</td>{/each}
            </tr>
            {#if visibleSecondary.length > 0}
              <tr class="sub"><th scope="row" colspan={channels.length + 1}><Term term="secondaryTimings" /></th></tr>
              {#each visibleSecondary as [label, key] (key)}
                <tr>
                  <th scope="row" class="plain"><Term term={key}>{label}</Term></th>
                  {#each channels as c, i (i)}<td>{key === 'trfc2' || key === 'trfcSb' ? withNs(c, c[key]) : cell(c[key] as number | undefined)}</td>{/each}
                </tr>
              {/each}
            {/if}
          </tbody>
        </table>
      </section>
    {/if}

    {#if modules.length > 0}
      <section class="card">
        <h3 class="label">{t('advanced.memory.modules.title')} (<Term term="spd" />)</h3>
        <table class="grid modules">
          <thead>
            <tr>
              <th>{t('advanced.memory.col.slot')}</th>
              <th>{t('advanced.memory.col.capacity')}</th>
              <th><Term term="jedec">{t('advanced.memory.col.rate')} JEDEC</Term></th>
              <th><Term term="jedec">JEDEC</Term> <Term term="cl">CL</Term>-<Term term="trcd">tRCD</Term>-<Term term="trp">tRP</Term>-<Term term="tras">tRAS</Term></th>
              <th>{t('advanced.memory.col.limits')}</th>
            </tr>
          </thead>
          <tbody>
            {#each modules as m, i (i)}
              <tr>
                <td>{m.slot}</td>
                <td>{m.capacityBytes ? `${num(m.capacityBytes / 1024 ** 3)} GB` : '–'}</td>
                <td>{spd(m) ? `${num(spd(m)!, 0)} MT/s` : '–'}</td>
                <td>{cycles(m)}</td>
                <td>{limits(m)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <p class="note">{t('advanced.memory.spdNote')}</p>
      </section>
    {/if}
  </div>
{/if}

<style>
  .memory {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
    gap: 14px;
    align-items: start;
  }
  .card {
    padding: 14px;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  h3 {
    margin: 0 0 8px;
    font-weight: 400;
  }
  .gen {
    margin-left: 10px;
    padding: 1px 6px;
    font-size: 10px;
    letter-spacing: 0.06em;
    color: var(--accent-2);
    border: 1px solid color-mix(in srgb, var(--accent-2) 50%, var(--border));
    border-radius: 999px;
  }
  .grid {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  th,
  td {
    padding: 3px 12px 3px 0;
    text-align: left;
    font-weight: 400;
    white-space: nowrap;
  }
  th {
    color: var(--text-muted);
  }
  th.plain {
    padding-left: 12px;
  }
  .big {
    color: var(--accent-2);
    font-size: 15px;
  }
  .sub th {
    padding-top: 10px;
    font-size: 12px;
  }
  .modules thead th {
    font-size: 11px;
    white-space: normal;
    vertical-align: bottom;
  }
  .note {
    margin: 10px 0 0;
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
