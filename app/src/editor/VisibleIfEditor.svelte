<script lang="ts">
  import NumberInput from '../components/settings/controls/NumberInput.svelte';
  import { sensorLabel } from '../lib/advanced/labels';
  import { FRAME_METRICS, LIMITS, type CompareOp, type Comparison, type FrameMetric, type Source, type StatOp, type VisibleIf } from '../lib/editor/profile';
  import { t } from '../lib/i18n/index.svelte';
  import type { Schema } from '../lib/types';

  // When a block shows (§6.3): always, while frame generation is active, or while a value (a
  // sensor or frame metric, with a statistic) meets a condition.
  let { value, schema, onChange }: { value: VisibleIf | null; schema: Schema | null; onChange: (next: VisibleIf | null) => unknown } = $props();

  const uid = $props.id();
  const OPS: CompareOp[] = ['>', '>=', '<', '<='];
  const STATS: StatOp[] = ['current', 'min', 'avg', 'max'];

  const form = $derived(value === null ? 'always' : 'fg' in value ? 'fg' : 'value');
  const cmp = $derived(value !== null && 'source' in value ? value : null);

  const DEFAULT: Comparison = { source: { frames: 'fps-displayed' }, stat: { op: 'current', window: 1, definition: 'integral' }, op: '<', value: 60 };

  function pickForm(next: string) {
    if (next === 'always') onChange(null);
    else if (next === 'fg') onChange({ fg: 'active' });
    else onChange(structuredClone(DEFAULT));
  }

  const encode = (s: Source) => ('frames' in s ? `frames:${s.frames}` : 'sensor' in s ? `sensor:${s.sensor}` : '');
  function decode(v: string): Source {
    const at = v.indexOf(':');
    return v.slice(0, at) === 'frames' ? { frames: v.slice(at + 1) as FrameMetric } : { sensor: v.slice(at + 1) };
  }

  const edit = (patch: Partial<Comparison>) => cmp !== null && onChange({ ...cmp, ...patch });
  const sensors = $derived((schema?.sensors ?? []).map((s) => ({ id: s.id, name: sensorLabel(s, t) })));
</script>

<div class="visible-if">
  <label for="{uid}-form">{t('editor.visibleIf')}</label>
  <select id="{uid}-form" value={form} onchange={(e) => pickForm(e.currentTarget.value)}>
    <option value="always">{t('editor.visibleIf.always')}</option>
    <option value="fg">{t('editor.visibleIf.fg')}</option>
    <option value="value">{t('editor.visibleIf.value')}</option>
  </select>
  {#if cmp !== null}
    <label for="{uid}-source">{t('editor.visibleIf.source')}</label>
    <select id="{uid}-source" value={encode(cmp.source)} onchange={(e) => edit({ source: decode(e.currentTarget.value) })}>
      <optgroup label={t('editor.palette.frames')}>
        {#each FRAME_METRICS as metric (metric)}<option value="frames:{metric}">{t(`overlay.text.metric.${metric}`)}</option>{/each}
      </optgroup>
      {#if sensors.length > 0}
        <optgroup label={t('editor.visibleIf.sensors')}>
          {#each sensors as sensor (sensor.id)}<option value="sensor:{sensor.id}">{sensor.name}</option>{/each}
        </optgroup>
      {/if}
    </select>
    <label for="{uid}-stat">{t('editor.props.stat')}</label>
    <select id="{uid}-stat" value={cmp.stat.op} onchange={(e) => edit({ stat: { ...cmp.stat, op: e.currentTarget.value as StatOp } })}>
      {#each STATS as op (op)}<option value={op}>{t(`editor.props.stat.${op}`)}</option>{/each}
    </select>
    {#if cmp.stat.op !== 'current'}
      <label for="{uid}-window">{t('editor.props.window')}</label>
      <NumberInput
        id="{uid}-window"
        integer
        value={cmp.stat.window}
        onCommit={(v) => edit({ stat: { ...cmp.stat, window: Math.min(Math.max(v, LIMITS.statWindow[0]), LIMITS.statWindow[1]) } })}
      />
    {/if}
    <label for="{uid}-op">{t('editor.condition')}</label>
    <select id="{uid}-op" value={cmp.op} onchange={(e) => edit({ op: e.currentTarget.value as CompareOp })}>
      {#each OPS as op (op)}<option value={op}>{op.replace('>=', '≥').replace('<=', '≤')}</option>{/each}
    </select>
    <label for="{uid}-value">{t('editor.thresholds.value')}</label>
    <NumberInput id="{uid}-value" value={cmp.value} onCommit={(v) => edit({ value: v })} />
  {/if}
</div>

<style>
  .visible-if {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 6px 10px;
    align-items: center;
  }
  label {
    font-size: 12.5px;
    color: var(--text-muted);
  }
</style>
