<script lang="ts">
  import NumberInput from '../components/settings/controls/NumberInput.svelte';
  import { LIMITS, type CompareOp, type Threshold } from '../lib/editor/profile';
  import { t } from '../lib/i18n/index.svelte';

  // A block's thresholds (§6.3): an ordered list, the first true rule wins for each target; at
  // most eight. Every edit hands the whole new list to `onChange`.
  let { value, onChange }: { value: readonly Threshold[]; onChange: (next: Threshold[]) => unknown } = $props();

  const uid = $props.id();
  const OPS: CompareOp[] = ['>', '>=', '<', '<='];
  const TARGETS: Threshold['target'][] = ['value', 'graph', 'panel'];

  const edit = (i: number, patch: Partial<Threshold>) => onChange(value.map((x, j) => (j === i ? { ...x, ...patch } : x)));
  const add = () => onChange([...value, { op: '>', value: 0, color: '#FF3B5C', target: 'value' }]);
  const remove = (i: number) => onChange(value.filter((_, j) => j !== i));
</script>

<ol class="thresholds">
  {#each value as threshold, i (i)}
    <li>
      <select aria-label={t('editor.condition')} value={threshold.op} onchange={(e) => edit(i, { op: e.currentTarget.value as CompareOp })}>
        {#each OPS as op (op)}<option value={op}>{op.replace('>=', '≥').replace('<=', '≤')}</option>{/each}
      </select>
      <label class="sr" for="{uid}-v{i}">{t('editor.thresholds.value')}</label>
      <NumberInput id="{uid}-v{i}" value={threshold.value} onCommit={(v) => edit(i, { value: v })} />
      <input
        type="color"
        aria-label={t('editor.props.color')}
        value={threshold.color.slice(0, 7)}
        onchange={(e) => edit(i, { color: e.currentTarget.value.toUpperCase() + threshold.color.slice(7) })}
      />
      <select aria-label={t('editor.thresholds.target')} value={threshold.target} onchange={(e) => edit(i, { target: e.currentTarget.value as Threshold['target'] })}>
        {#each TARGETS as target (target)}<option value={target}>{t(`editor.thresholds.target.${target}`)}</option>{/each}
      </select>
      <button type="button" class="remove" aria-label={t('editor.thresholds.remove')} title={t('editor.thresholds.remove')} onclick={() => remove(i)}>×</button>
    </li>
  {/each}
</ol>
<button type="button" class="add" disabled={value.length >= LIMITS.thresholds} onclick={add}>{t('editor.thresholds.add')}</button>
<p class="hint">{t('editor.thresholds.hint')}</p>

<style>
  .thresholds {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0 0 8px;
    padding: 0;
    list-style: none;
    counter-reset: rule;
  }
  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding-left: 22px;
    position: relative;
  }
  /* The order matters (first true wins), so the rules are numbered. */
  li::before {
    counter-increment: rule;
    content: counter(rule);
    position: absolute;
    left: 0;
    width: 16px;
    font-size: 11px;
    color: var(--text-muted);
    text-align: right;
  }
  li :global(input[type='text']) {
    width: 64px;
  }
  .remove {
    padding: 2px 8px;
    font: inherit;
    color: var(--text-muted);
    cursor: pointer;
    background: none;
    border: 1px solid transparent;
    border-radius: 6px;
  }
  .remove:hover {
    color: var(--crit);
    border-color: var(--border);
  }
  .add {
    padding: 4px 10px;
    font: inherit;
    font-size: 12.5px;
    color: var(--accent);
    cursor: pointer;
    background: none;
    border: 1px dashed color-mix(in srgb, var(--accent) 50%, var(--border));
    border-radius: 8px;
  }
  .add:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .hint {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
