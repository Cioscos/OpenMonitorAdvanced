<script lang="ts">
  import type { Snippet } from 'svelte';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import {
    conditionsFor,
    deltaFromDisplayIn,
    deltaToDisplayIn,
    fromDisplay,
    LEVELS,
    thresholdText,
    toDisplay,
    type DisplayScale,
    type LevelName,
  } from '../../lib/rules';
  import type { LevelSpec, Rule, RuleCondition, RuleStatus, Schema } from '../../lib/types';
  import Field from './controls/Field.svelte';
  import NumberInput from './controls/NumberInput.svelte';
  import Segmented from './controls/Segmented.svelte';
  import SelectField from './controls/SelectField.svelte';
  import Toggle from './controls/Toggle.svelte';

  // The fields of one rule (spec M5 §3.6): condition, both levels with threshold, duration and
  // notification, then the hysteresis. Numbers are typed in `scale` and stored in the base unit.
  // Every change goes through `onChange`, which applies it to the latest rule: saved at once for
  // an existing rule, kept in the draft of a new one.
  let {
    id,
    rule,
    builtin,
    shipped = null,
    scales,
    scale,
    status,
    schema,
    errors,
    onScale,
    onChange,
    before,
    after,
  }: {
    /** Base of the element ids. */
    id: string;
    rule: Rule;
    builtin: boolean;
    /** The built-in rule as shipped: a level switched back on starts from it. */
    shipped?: Rule | null;
    scales: DisplayScale[];
    scale: DisplayScale;
    status: readonly RuleStatus[];
    schema: Schema | null;
    /** Error keys of this rule, by path below it (`crit`, `warn.durationS`). */
    errors: Record<string, string>;
    onScale: (next: DisplayScale) => void;
    /** Resolves to whether the change was taken. */
    onChange: (change: (latest: Rule) => Partial<Rule>) => unknown;
    before?: Snippet;
    after?: Snippet;
  } = $props();

  const flag = $derived(rule.condition === 'flagActive');
  const errorText = (...paths: string[]) => {
    const key = paths.map((p) => errors[p]).find((k) => k !== undefined);
    return key === undefined ? null : t(key);
  };
  /** Errors that belong to no field of the panel (target, id, condition of a hand-edited file). */
  const FIELD_PATHS = new Set(['levels', 'warn', 'warn.threshold', 'warn.durationS', 'crit', 'crit.threshold', 'crit.durationS', 'hysteresis', 'hysteresis.amount', 'hysteresis.durationS']);
  const general = $derived(Object.entries(errors).filter(([path]) => !FIELD_PATHS.has(path)).map(([, key]) => t(key)));

  const describedBy = (field: string, hasError: boolean, ...extra: string[]) =>
    [...extra, hasError ? `${field}-error` : null].filter(Boolean).join(' ') || undefined;

  const other = (level: LevelName): LevelName => (level === 'warn' ? 'crit' : 'warn');

  /** The level a switched-on level starts from: as shipped, else a copy of the other level. */
  function seed(latest: Rule, level: LevelName): LevelSpec {
    const base = shipped?.[level] ?? latest[other(level)];
    if (latest.condition === 'flagActive') return { threshold: null, durationS: base?.durationS ?? 10 };
    return base ? structuredClone($state.snapshot(base)) : { threshold: { fixed: Number.NaN }, durationS: 30 };
  }

  const setLevel = (level: LevelName, on: boolean) => onChange((latest) => ({ [level]: on ? seed(latest, level) : null }));
  const setThreshold = (level: LevelName, typed: number) =>
    onChange((latest) => {
      const spec = latest[level];
      return spec ? { [level]: { ...spec, threshold: { fixed: fromDisplay(typed, scale) } } } : {};
    });
  const setDuration = (level: LevelName, seconds: number) =>
    onChange((latest) => {
      const spec = latest[level];
      return spec ? { [level]: { ...spec, durationS: seconds } } : {};
    });

  function setCondition(condition: RuleCondition) {
    return onChange(() => ({ condition }));
  }

  /** The value in the field: a fixed threshold as shown; empty for one read from a property. */
  function fieldValue(spec: LevelSpec): number {
    const threshold = spec.threshold;
    return threshold && 'fixed' in threshold ? toDisplay(threshold.fixed, scale) : Number.NaN;
  }
</script>

<div class="editor">
  {@render before?.()}

  {#if general.length > 0}
    <div class="general" role="alert">
      {#each general as text, i (i)}<p>{text}</p>{/each}
    </div>
  {/if}

  <div class="rows">
    {#if !builtin}
      <Segmented
        id="{id}-condition"
        label={t('rules.editor.condition')}
        options={conditionsFor(rule.unit).map((value) => ({ value, label: t(`rules.condition.${value}`) }))}
        value={rule.condition}
        error={errorText('condition')}
        onChange={setCondition}
      />
    {/if}
    {#if scales.length > 1}
      <SelectField
        id="{id}-scale"
        label={t('rules.editor.scale')}
        items={scales.map((s) => ({ value: s.id, label: s.symbol }))}
        value={scale.id}
        onChange={(next) => onScale(scales.find((s) => s.id === next) ?? scale)}
      />
    {/if}
    <Toggle
      id="{id}-enabled"
      label={t('rules.editor.enabled')}
      checked={rule.enabled}
      onChange={(enabled) => onChange(() => ({ enabled }))}
    />
  </div>

  <div class="levels">
    {#each LEVELS as level (level)}
      {@const spec = rule[level]}
      {@const field = `${id}-${level}`}
      <section class="level {level}" aria-labelledby="{field}-level-label">
        <Toggle
          id="{field}-level"
          label={t(`rules.editor.${level}Level`)}
          checked={spec !== null}
          onChange={(on) => setLevel(level, on)}
        />
        {#if spec}
          {#if !flag}
            {@const property = spec.threshold !== null && 'property' in spec.threshold}
            <Field
              id="{field}-threshold"
              label={t(`rules.editor.${level}Threshold`)}
              labelFor="{field}-threshold-input"
              error={errorText(`${level}.threshold`, level)}
            >
              {#snippet control()}
                <NumberInput
                  id="{field}-threshold-input"
                  value={fieldValue(spec)}
                  unit={scale.symbol}
                  invalid={errorText(`${level}.threshold`, level) !== null}
                  describedBy={describedBy(`${field}-threshold`, errorText(`${level}.threshold`, level) !== null, ...(property ? [`${field}-resolved`] : []))}
                  onCommit={(typed) => setThreshold(level, typed)}
                />
              {/snippet}
              {#snippet notes()}
                {#if property}
                  <p class="resolved" id="{field}-resolved">
                    {thresholdText(rule, level, { status, schema, scale, locale: i18n.locale, t })}
                  </p>
                  <p>{t('rules.editor.propertyHint')}</p>
                {/if}
              {/snippet}
            </Field>
          {/if}
          <Field
            id="{field}-duration"
            label={t(`rules.editor.${level}Duration`)}
            labelFor="{field}-duration-input"
            description={t('rules.editor.duration.hint')}
            error={errorText(`${level}.durationS`)}
          >
            {#snippet control()}
              <NumberInput
                id="{field}-duration-input"
                value={spec.durationS}
                integer
                unit="s"
                invalid={errorText(`${level}.durationS`) !== null}
                describedBy={describedBy(`${field}-duration`, errorText(`${level}.durationS`) !== null, `${field}-duration-desc`)}
                onCommit={(seconds) => setDuration(level, seconds)}
              />
            {/snippet}
          </Field>
          <Toggle
            id="{field}-notify"
            label={t(`rules.notify.${level}`)}
            checked={rule.notify[level]}
            onChange={(on) => onChange((latest) => ({ notify: { ...latest.notify, [level]: on } }))}
          />
        {/if}
      </section>
    {/each}
    {#if errorText('levels') !== null}
      <p class="levels-error" role="alert">{errorText('levels')}</p>
    {/if}
  </div>

  <div class="rows">
    {#if !flag}
      <Field
        id="{id}-hysteresis"
        label={t('rules.editor.hysteresisAmount')}
        labelFor="{id}-hysteresis-input"
        description={t('rules.editor.hysteresisAmount.hint')}
        error={errorText('hysteresis.amount', 'hysteresis')}
      >
        {#snippet control()}
          <NumberInput
            id="{id}-hysteresis-input"
            value={deltaToDisplayIn(rule.hysteresis.amount, scale)}
            unit={scale.symbol}
            invalid={errorText('hysteresis.amount', 'hysteresis') !== null}
            describedBy={describedBy(`${id}-hysteresis`, errorText('hysteresis.amount', 'hysteresis') !== null, `${id}-hysteresis-desc`)}
            onCommit={(typed) => onChange((latest) => ({ hysteresis: { ...latest.hysteresis, amount: deltaFromDisplayIn(typed, scale) } }))}
          />
        {/snippet}
      </Field>
    {/if}
    <Field
      id="{id}-hold"
      label={t('rules.editor.hysteresisDuration')}
      labelFor="{id}-hold-input"
      description={t('rules.editor.hysteresisDuration.hint')}
      error={errorText('hysteresis.durationS')}
    >
      {#snippet control()}
        <NumberInput
          id="{id}-hold-input"
          value={rule.hysteresis.durationS}
          integer
          unit="s"
          invalid={errorText('hysteresis.durationS') !== null}
          describedBy={describedBy(`${id}-hold`, errorText('hysteresis.durationS') !== null, `${id}-hold-desc`)}
          onCommit={(seconds) => onChange((latest) => ({ hysteresis: { ...latest.hysteresis, durationS: seconds } }))}
        />
      {/snippet}
    </Field>
  </div>

  {@render after?.()}
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .rows,
  .level {
    min-width: 0;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .rows > :global(* + *),
  .level > :global(* + *) {
    border-top: 1px solid var(--border);
  }
  .levels {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 12px;
  }
  /* The level's colour runs down its edge: amber for a warning, red for critical. */
  .level {
    border-left: 3px solid var(--level);
  }
  .level.warn {
    --level: var(--warn);
  }
  .level.crit {
    --level: var(--crit);
  }
  .resolved {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .general,
  .levels-error {
    margin: 0;
    padding: 8px 12px;
    font-size: 12.5px;
    color: var(--crit);
    border-left: 2px solid var(--crit);
    background: color-mix(in srgb, var(--crit) 6%, transparent);
    border-radius: 0 8px 8px 0;
  }
  .levels-error {
    grid-column: 1 / -1;
  }
  .general p {
    margin: 0;
  }
</style>
