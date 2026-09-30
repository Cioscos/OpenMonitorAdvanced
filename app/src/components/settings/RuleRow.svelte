<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import { LEVELS, thresholdText, type LevelName, type ThresholdContext } from '../../lib/rules';
  import type { Rule, RuleStatus } from '../../lib/types';

  // One row of the rules table: name, target and what the engine makes of the rule, then each level
  // (threshold, duration, bell), the on/off switch and the actions. Every control acts at once.
  let {
    rule,
    builtin,
    name,
    target,
    status,
    context,
    modified,
    expanded,
    editorId,
    onEdit,
    onEnabled,
    onNotify,
    onReset,
    onDelete,
  }: {
    rule: Rule;
    builtin: boolean;
    name: string;
    target: string;
    /** This rule's entry of `get_rule_status`, when there is one yet. */
    status: RuleStatus | undefined;
    context: ThresholdContext;
    modified: boolean;
    expanded: boolean;
    editorId: string;
    onEdit: () => void;
    onEnabled: (enabled: boolean) => unknown;
    onNotify: (level: LevelName, on: boolean) => unknown;
    onReset: () => unknown;
    onDelete: () => unknown;
  } = $props();

  /** What the engine makes of the rule now: off, no sensor here, a problem, or how many sensors it watches. */
  const summary = $derived.by((): { text: string; problem: boolean } => {
    if (!rule.enabled) return { text: t('rules.status.disabled'), problem: false };
    const instances = status?.instances ?? [];
    if (status !== undefined && instances.length === 0) return { text: t('rules.status.none'), problem: false };
    for (const problem of ['order', 'unitMismatch'] as const) {
      const count = instances.filter((i) => i.problem === problem).length;
      if (count > 0) {
        const key = problem === 'order' && count === 1 ? 'rules.status.problem.order.one' : `rules.status.problem.${problem}`;
        return { text: t(key, { count }), problem: true };
      }
    }
    if (instances.length === 0) return { text: '', problem: false };
    return { text: instances.length === 1 ? t('rules.status.one') : t('rules.status.count', { count: instances.length }), problem: false };
  });
</script>

<tr aria-label={name} class:off={!rule.enabled} class:open={expanded}>
  <th scope="row">
    <div class="rule">
      <span class="name">
        {name}
        {#if modified}<span class="badge">{t('rules.modified')}</span>{/if}
      </span>
      <span class="target">{target}</span>
      {#if summary.text}<span class="state" class:problem={summary.problem}>{summary.text}</span>{/if}
    </div>
  </th>
  {#each LEVELS as level (level)}
    {@const spec = rule[level]}
    <td class="level {level}">
      {#if spec}
        <div class="cell">
          <span class="value">{thresholdText(rule, level, context)}</span>
          <button
            type="button"
            class="bell"
            aria-pressed={rule.notify[level]}
            aria-label={t(`rules.notify.${level}`)}
            title={t(`rules.notify.${level}`)}
            onclick={() => onNotify(level, !rule.notify[level])}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
              <path
                d="M8 2.2a3.8 3.8 0 0 0-3.8 3.8v2.6L3 10.6v.9h10v-.9l-1.2-2V6A3.8 3.8 0 0 0 8 2.2ZM6.4 12.6a1.7 1.7 0 0 0 3.2 0"
                fill={rule.notify[level] ? 'currentColor' : 'none'}
                stroke="currentColor"
                stroke-width="1.3"
                stroke-linejoin="round"
              />
            </svg>
          </button>
        </div>
        <span class="duration">{t('rules.duration', { s: spec.durationS })}</span>
      {:else}
        <span class="none">—</span>
      {/if}
    </td>
  {/each}
  <td class="enabled">
    <button
      type="button"
      role="switch"
      class="switch"
      aria-checked={rule.enabled}
      aria-label={t('rules.enabledFor', { name })}
      onclick={() => onEnabled(!rule.enabled)}
    >
      <span class="knob" aria-hidden="true"></span>
    </button>
  </td>
  <td class="actions">
    <button type="button" class="action" aria-expanded={expanded} aria-controls={expanded ? editorId : undefined} onclick={onEdit}>
      {t('rules.edit')}
    </button>
    {#if builtin && modified}
      <button type="button" class="action" onclick={onReset}>{t('rules.restore')}</button>
    {/if}
    {#if !builtin}
      <button type="button" class="action danger" onclick={onDelete}>{t('rules.delete')}</button>
    {/if}
  </td>
</tr>

<style>
  tr > * {
    padding: 10px 12px;
    text-align: left;
    vertical-align: top;
    border-top: 1px solid var(--border);
  }
  tr.open > * {
    background: var(--surface-2);
  }
  th {
    font-weight: normal;
  }
  .rule {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .name {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-weight: 600;
  }
  .badge {
    padding: 1px 7px;
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 55%, transparent);
    border-radius: 999px;
  }
  .target,
  .state,
  .duration {
    font-size: 12px;
    color: var(--text-muted);
  }
  .state.problem {
    color: var(--warn);
  }
  tr.off .rule .name,
  tr.off .value {
    color: var(--text-muted);
  }
  /* A thin tick in the level's colour before each threshold. */
  .level .cell {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .value {
    position: relative;
    padding-left: 10px;
    font-variant-numeric: tabular-nums;
  }
  .value::before {
    content: '';
    position: absolute;
    top: 3px;
    bottom: 3px;
    left: 0;
    width: 3px;
    border-radius: 2px;
    background: var(--level);
  }
  .level.warn {
    --level: var(--warn);
  }
  .level.crit {
    --level: var(--crit);
  }
  tr.off .value::before {
    background: var(--border);
  }
  .duration {
    display: block;
    padding-left: 10px;
  }
  .none {
    color: var(--text-muted);
  }
  .bell {
    display: inline-flex;
    padding: 3px;
    color: var(--text-muted);
    cursor: pointer;
    background: none;
    border: 0;
    border-radius: 6px;
  }
  .bell[aria-pressed='true'] {
    color: var(--level);
  }
  .bell:hover {
    background: var(--surface-2);
  }
  .switch {
    position: relative;
    width: 34px;
    height: 20px;
    padding: 0;
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 999px;
    transition:
      background-color 0.15s,
      border-color 0.15s;
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--text-muted);
    transition: transform 0.15s;
  }
  .switch[aria-checked='true'] {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch[aria-checked='true'] .knob {
    transform: translateX(14px);
    background: var(--on-accent);
  }
  .actions {
    white-space: nowrap;
    text-align: right;
  }
  .action {
    padding: 4px 10px;
    font-size: 12.5px;
    color: var(--text);
    cursor: pointer;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .action + .action {
    margin-left: 6px;
  }
  .action:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .action[aria-expanded='true'] {
    border-color: var(--accent);
  }
  .action.danger:hover {
    color: var(--crit);
    border-color: color-mix(in srgb, var(--crit) 55%, var(--border));
  }
  .bell:focus-visible,
  .switch:focus-visible,
  .action:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  @media (max-width: 720px) {
    tr {
      display: grid;
      grid-template-columns: 1fr 1fr auto;
      border-top: 1px solid var(--border);
    }
    tr > * {
      border-top: 0;
    }
    th,
    .actions {
      grid-column: 1 / -1;
    }
    .actions {
      text-align: left;
    }
  }
</style>
