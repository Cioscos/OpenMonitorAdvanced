<script lang="ts">
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import Term from '../common/Term.svelte';

  // The disk points on a bar (DZ14): the measure shown (`–` without points, as for an NVMe profile
  // B2) and one ▲ for the chosen reference. Full scale: the first multiple of 500 at or above
  // max(points, ▲, 1000).
  let { value, reference, referenceLabel }: { value: number | null; reference: number | null; referenceLabel: string | null } = $props();

  const max = $derived(Math.ceil(Math.max(value ?? 0, reference ?? 0, 1000) / 500) * 500);
  const pct = (v: number) => `${Math.min(100, (v / max) * 100)}%`;
  const text = (v: number) => Math.round(v).toLocaleString(i18n.locale);
  const markText = $derived(reference === null ? '' : t('performance.score.disk.pointsMark', { label: referenceLabel ?? '', points: text(reference) }));
</script>

<div class="points">
  <p class="head">
    <Term term="diskPoints">{value === null ? '–' : t('performance.score.disk.points', { points: text(value) })}</Term>
  </p>
  <div
    class="track"
    role="meter"
    aria-label={t('performance.score.disk.pointsBar')}
    aria-valuemin={0}
    aria-valuemax={max}
    aria-valuenow={value ?? undefined}
    aria-valuetext={value === null ? '–' : text(value)}
  >
    {#if value !== null}<span class="fill" style:width={pct(value)}></span>{/if}
    {#if reference !== null}
      <span class="mark" style:left={pct(reference)} role="img" aria-label={markText} title={markText}>▲</span>
    {/if}
  </div>
  <p class="scale" aria-hidden="true"><span>0</span><span>{text(max)}</span></p>
</div>

<style>
  .points {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .head {
    margin: 0;
    font-weight: 600;
  }
  .track {
    position: relative;
    height: 10px;
    margin-bottom: 14px;
    background: var(--surface-2);
    border-radius: 5px;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
    border-radius: 5px;
    box-shadow: 0 0 8px color-mix(in srgb, var(--accent) 60%, transparent);
  }
  /* The ▲ hangs under the track, centred on its value. */
  .mark {
    position: absolute;
    top: 100%;
    font-size: 11px;
    line-height: 1;
    color: var(--accent-2);
    transform: translateX(-50%);
  }
  .scale {
    display: flex;
    justify-content: space-between;
    margin: 0;
    font-size: 11px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
