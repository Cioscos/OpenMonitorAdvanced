<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { sensorLabel } from '../../lib/advanced/labels';
  import type { Backend } from '../../lib/backend';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import {
    applyOverride,
    customPatch,
    defaultScale,
    errorsOf,
    fixedValues,
    isModified,
    newCustomRule,
    overrideIsRedundant,
    overrideOf,
    overridePatch,
    ruleEntries,
    ruleName,
    scalesFor,
    targetLabel,
    type DisplayScale,
    type RuleEntry,
    type ThresholdContext,
  } from '../../lib/rules';
  import { settings } from '../../lib/settings.svelte';
  import type { Rule, RuleStatus, RulesSettings, Sensor } from '../../lib/types';
  import { display } from '../../lib/units.svelte';
  import Group from './controls/Group.svelte';
  import SelectField from './controls/SelectField.svelte';
  import RuleEditor from './RuleEditor.svelte';
  import RuleRow from './RuleRow.svelte';

  // Settings › Rules and alerts (spec M5 §3.6). The built-in rules come from the core
  // (`get_default_rules`), their resolved thresholds from `get_rule_status`, read while the section
  // is open. Changes to existing rules are saved at once; a new rule is a draft until "Create".
  // `newRuleSensor` opens "New rule" already holding that sensor (from a row of the Advanced view).
  let { store, backend, newRuleSensor }: { store: LiveStore; backend: Backend; newRuleSensor?: string } = $props();

  const STATUS_INTERVAL_MS = 1000;

  let defaults = $state.raw<Rule[] | null>(null);
  let defaultsFailed = $state(false);
  let status = $state.raw<RuleStatus[]>([]);
  /** The rule whose panel is open, with the unit its numbers are typed in (kept while it is open). */
  let editing = $state.raw<{ id: string; scales: DisplayScale[]; scale: DisplayScale } | null>(null);
  /** The "New rule" panel: nothing is saved until "Create". */
  let draft = $state.raw<{ query: string; sensor: Sensor | null; rule: Rule | null; scales: DisplayScale[]; scale: DisplayScale | null } | null>(
    null,
  );

  const schema = $derived(store.schema);
  const rules = $derived<RulesSettings>(settings.state?.settings.rules ?? { overrides: {}, custom: [] });
  const entries = $derived(ruleEntries(defaults ?? [], rules));
  const builtinEntries = $derived(entries.filter((e) => e.builtin));
  const customEntries = $derived(entries.filter((e) => !e.builtin));
  const prefs = $derived({ temperature: display.temperature, throughput: display.throughput });
  const excluded = $derived(
    (settings.state?.diagnostics ?? []).flatMap((d) => (d.kind === 'invalidRule' ? [{ path: d.path, key: d.key }] : [])),
  );
  /** Where the errors of the rule being created land: the index it would take. */
  const draftBase = $derived(`rules.custom.${rules.custom.length}`);

  onMount(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    backend.getDefaultRules().then(
      (list) => {
        if (!stopped) defaults = list;
      },
      (error) => {
        console.error('built-in rules unavailable', error);
        if (!stopped) defaultsFailed = true;
      },
    );
    // At most one request in flight, the next one a second after the reply: resolved thresholds and
    // problems change with the schema and the settings, without a new health report.
    const poll = async () => {
      try {
        const next = await backend.getRuleStatus();
        if (!stopped) status = next;
      } catch (error) {
        console.error('rule status unavailable', error);
      }
      if (!stopped) timer = setTimeout(poll, STATUS_INTERVAL_MS);
    };
    void poll();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  });

  // Changes run one after the other, each built from the settings as the previous one left them, so
  // two quick edits of the custom list never undo each other.
  let queue: Promise<unknown> = Promise.resolve();
  function enqueue(job: () => Promise<boolean>): Promise<boolean> {
    const run = queue.then(job);
    queue = run.catch(() => undefined);
    return run;
  }
  const latest = (): RulesSettings | null => settings.state?.settings.rules ?? null;

  /** Applies `change` to the latest version of the rule: an override for a built-in rule, the list for a custom one. */
  function changeRule(entry: RuleEntry, change: (rule: Rule) => Partial<Rule>): Promise<boolean> {
    const id = entry.rule.id;
    return enqueue(async () => {
      const current = latest();
      if (current === null) return false;
      if (entry.builtin) {
        const shipped = defaults?.find((r) => r.id === id);
        if (!shipped) return false;
        const over = overrideOf(change(applyOverride(shipped, current.overrides[id])));
        // A patch replaces whole fields but cannot delete one (R5). When nothing differs from the
        // shipped rule any more the entry is dropped whole, as by "Restore"; a partly redundant
        // entry stays and `isModified` ignores its redundant fields.
        if (overrideIsRedundant({ ...current.overrides[id], ...over }, shipped)) {
          return id in current.overrides ? settings.resetRuleOverride(id) : true;
        }
        return settings.update(overridePatch(id, over));
      }
      if (!current.custom.some((r) => r.id === id)) return false;
      return settings.update(customPatch(current.custom.map((r) => (r.id === id ? { ...r, ...change(r) } : r))));
    });
  }

  function deleteRule(id: string) {
    if (editing?.id === id) editing = null;
    return enqueue(async () => {
      const current = latest();
      if (current === null || !current.custom.some((r) => r.id === id)) return false;
      return settings.update(customPatch(current.custom.filter((r) => r.id !== id)));
    });
  }

  // Through the store: it takes the state the command returns and clears the rule's errors.
  const reset = (id: string) => enqueue(() => settings.resetRuleOverride(id));

  function toggleEditor(entry: RuleEntry) {
    if (editing?.id === entry.rule.id) {
      editing = null;
      return;
    }
    const scales = scalesFor(entry.rule.unit, prefs);
    editing = { id: entry.rule.id, scales, scale: defaultScale(scales, fixedValues(entry.rule)) };
    closeDraft();
  }

  const context = (scale: DisplayScale): ThresholdContext => ({ status, schema, scale, locale: i18n.locale, t });
  const rowScale = (rule: Rule) => defaultScale(scalesFor(rule.unit, prefs), fixedValues(rule));

  // --- New rule ---

  /** Forgets the errors a refused "Create" left at the draft's path. */
  function clearDraftErrors() {
    const kept = Object.entries(settings.errors).filter(([field]) => field !== draftBase && !field.startsWith(`${draftBase}.`));
    if (kept.length !== Object.keys(settings.errors).length) settings.errors = Object.fromEntries(kept);
  }

  function openDraft() {
    editing = null;
    clearDraftErrors();
    draft = { query: '', sensor: null, rule: null, scales: [], scale: null };
  }

  function closeDraft() {
    if (draft === null) return;
    clearDraftErrors();
    draft = null;
  }

  /** A rule on `sensor` whose thresholds are still to be typed (NaN reads as an empty field). */
  function blankRule(sensor: Sensor): Rule {
    const rule = newCustomRule(sensor);
    const blank = (level: Rule['warn']) => (level?.threshold ? { ...level, threshold: { fixed: Number.NaN } } : level);
    return { ...rule, warn: blank(rule.warn), crit: blank(rule.crit) };
  }

  function pickSensor(id: string) {
    if (draft === null) return;
    const sensor = schema?.sensors.find((s) => s.id === id) ?? null;
    if (sensor === null) {
      draft = { ...draft, sensor: null, rule: null, scales: [], scale: null };
    } else if (draft.rule !== null && draft.rule.unit === sensor.unit) {
      // Same unit: the rule typed so far only changes target.
      draft = { ...draft, sensor, rule: { ...draft.rule, target: { sensor: sensor.id } } };
    } else {
      const scales = scalesFor(sensor.unit, prefs);
      draft = { ...draft, sensor, rule: blankRule(sensor), scales, scale: defaultScale(scales, []) };
    }
  }

  // Once the schema is known, the requested sensor fills the draft (once: later edits are the user's).
  let prefilled = false;
  $effect(() => {
    if (prefilled || newRuleSensor === undefined || schema === null) return;
    prefilled = true;
    if (!schema.sensors.some((s) => s.id === newRuleSensor)) return;
    untrack(() => {
      openDraft();
      pickSensor(newRuleSensor);
    });
  });

  function changeDraft(change: (rule: Rule) => Partial<Rule>) {
    if (draft?.rule == null) return false;
    draft = { ...draft, rule: { ...draft.rule, ...change(draft.rule) } };
    return true;
  }

  /** Every level has its threshold (none for a flag), and there is at least one level. */
  const draftReady = $derived.by(() => {
    const rule = draft?.rule;
    if (!rule || (rule.warn === null && rule.crit === null)) return false;
    return [rule.warn, rule.crit].every((level) => {
      if (level === null) return true;
      const threshold = level.threshold;
      if (threshold === null) return rule.condition === 'flagActive';
      return !('fixed' in threshold) || Number.isFinite(threshold.fixed);
    });
  });

  function create() {
    const rule = draft?.rule;
    if (!rule || !draftReady) return;
    const added = $state.snapshot(rule) as Rule;
    void enqueue(async () => {
      const current = latest();
      if (current === null) return false;
      const taken = await settings.update(customPatch([...current.custom, added]));
      if (taken && draft?.rule?.id === added.id) draft = null;
      return taken;
    });
  }

  /** The sensors whose device or label contains the search, grouped by device; the chosen one stays. */
  const sensorItems = $derived.by(() => {
    if (draft === null || schema === null) return [];
    const query = draft.query.trim().toLocaleLowerCase(i18n.locale);
    const chosen = draft.sensor?.id ?? null;
    const groups = schema.devices
      .map((device) => {
        const deviceMatch = device.name.toLocaleLowerCase(i18n.locale).includes(query);
        return {
          group: device.name,
          options: schema.sensors
            .filter((s) => s.deviceId === device.id)
            .map((s) => ({ value: s.id, label: sensorLabel(s, t) }))
            .filter((o) => o.value === chosen || deviceMatch || o.label.toLocaleLowerCase(i18n.locale).includes(query)),
        };
      })
      .filter((g) => g.options.length > 0);
    return [{ value: '', label: t('rules.editor.sensor.choose') }, ...groups];
  });
  const noMatch = $derived(sensorItems.length <= 1);
</script>

{#if excluded.length > 0}
  <div class="excluded" role="status">
    <p id="rules-excluded-title" class="excluded-title">{t('rules.excluded')}</p>
    <ul aria-labelledby="rules-excluded-title">
      {#each excluded as item (item.path)}
        <li><code>{item.path}</code> {t(item.key)}</li>
      {/each}
    </ul>
    <p>{t('rules.excluded.hint')}</p>
  </div>
{/if}

<p class="intro">{t('rules.intro')}</p>

{#snippet table(list: RuleEntry[], caption: string)}
  <div class="table">
    <table>
      <caption class="visually-hidden">{caption}</caption>
      <thead>
        <tr>
          <th scope="col">{t('rules.column.rule')}</th>
          <th scope="col">{t('rules.column.warn')}</th>
          <th scope="col">{t('rules.column.crit')}</th>
          <th scope="col">{t('rules.column.enabled')}</th>
          <th scope="col"><span class="visually-hidden">{t('rules.column.actions')}</span></th>
        </tr>
      </thead>
      <tbody>
        {#each list as entry (entry.rule.id)}
          {@const rule = entry.rule}
          {@const name = ruleName(rule, entry.builtin, schema, t)}
          {@const open = editing?.id === rule.id}
          {@const editorId = `rule-panel-${rule.id}`}
          <RuleRow
            {rule}
            builtin={entry.builtin}
            {name}
            target={targetLabel(rule, schema, t)}
            status={status.find((s) => s.ruleId === rule.id)}
            context={context(rowScale(rule))}
            modified={entry.builtin && isModified(rule.id, rules, defaults?.find((r) => r.id === rule.id))}
            expanded={open}
            {editorId}
            onEdit={() => toggleEditor(entry)}
            onEnabled={(enabled) => changeRule(entry, () => ({ enabled }))}
            onNotify={(level, on) => changeRule(entry, (r) => ({ notify: { ...r.notify, [level]: on } }))}
            onReset={() => reset(rule.id)}
            onDelete={() => deleteRule(rule.id)}
          />
          {#if open && editing}
            <tr class="panel" id={editorId}>
              <td colspan="5">
                <RuleEditor
                  id="rule-editor"
                  {rule}
                  builtin={entry.builtin}
                  shipped={defaults?.find((r) => r.id === rule.id) ?? null}
                  scales={editing.scales}
                  scale={editing.scale}
                  {status}
                  {schema}
                  errors={errorsOf(settings.errors, entry.base)}
                  onScale={(scale) => editing && (editing = { ...editing, scale })}
                  onChange={(change) => changeRule(entry, change)}
                />
              </td>
            </tr>
          {/if}
        {/each}
      </tbody>
    </table>
  </div>
{/snippet}

<Group id="rules-builtin" title={t('rules.group.builtin')}>
  {#if defaultsFailed}
    <p class="note failed">{t('rules.defaultsFailed')}</p>
  {:else}
    {@render table(builtinEntries, t('rules.group.builtin'))}
  {/if}
</Group>

<Group id="rules-custom" title={t('rules.group.custom')}>
  {#if customEntries.length > 0}
    {@render table(customEntries, t('rules.group.custom'))}
  {:else if draft === null}
    <p class="note">{t('rules.customEmpty')}</p>
  {/if}
  {#if draft === null}
    <div class="toolbar">
      <button type="button" class="primary" onclick={openDraft}>{t('rules.new')}</button>
    </div>
  {:else}
    <div class="creator" role="group" aria-labelledby="rule-new-title">
      <p class="creator-title" id="rule-new-title">{t('rules.new')}</p>
      {#snippet picker()}
        <div class="picker">
          <div class="search">
            <input
              type="search"
              value={draft?.query ?? ''}
              placeholder={t('rules.editor.search.placeholder')}
              aria-label={t('rules.editor.search')}
              oninput={(event) => draft && (draft = { ...draft, query: event.currentTarget.value })}
            />
          </div>
          <SelectField
            id="rule-sensor"
            label={t('rules.editor.sensor')}
            description={noMatch ? t('rules.editor.sensor.none') : null}
            items={sensorItems}
            value={draft?.sensor?.id ?? ''}
            onChange={pickSensor}
          />
        </div>
      {/snippet}
      {#snippet actions()}
        <div class="toolbar">
          <button type="button" class="primary" disabled={!draftReady} onclick={create}>{t('rules.create')}</button>
          <button type="button" class="secondary" onclick={closeDraft}>{t('rules.cancel')}</button>
        </div>
      {/snippet}
      {#if draft.rule && draft.scale}
        <RuleEditor
          id="rule-new"
          rule={draft.rule}
          builtin={false}
          scales={draft.scales}
          scale={draft.scale}
          {status}
          {schema}
          errors={errorsOf(settings.errors, draftBase)}
          onScale={(scale) => draft && (draft = { ...draft, scale })}
          onChange={changeDraft}
          before={picker}
          after={actions}
        />
      {:else}
        {@render picker()}
        {@render actions()}
      {/if}
    </div>
  {/if}
</Group>

<Group id="rules-limits" title={t('rules.limits.title')}>
  <ul class="limits">
    <li>{t('rules.limits.cpuThrottle')}</li>
    <li>{t('rules.limits.nvme')}</li>
    <li>{t('rules.limits.tjMax')}</li>
    <li>{t('rules.limits.sata')}</li>
  </ul>
</Group>

<style>
  .intro {
    max-width: 72ch;
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--text-muted);
  }
  .table {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  thead th {
    padding: 8px 12px;
    font-size: 12px;
    font-weight: 600;
    text-align: left;
    color: var(--text-muted);
  }
  thead th:first-child {
    width: 34%;
  }
  .panel > td {
    padding: 4px 12px 16px;
    background: var(--surface-2);
  }
  .note {
    margin: 0;
    padding: 12px 16px;
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .failed {
    color: var(--crit);
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 12px 16px;
  }
  .creator .toolbar {
    padding: 0;
  }
  .primary,
  .secondary {
    padding: 6px 14px;
    font-size: 13px;
    cursor: pointer;
    border-radius: 8px;
  }
  .primary {
    font-weight: 600;
    color: var(--on-accent);
    background: var(--accent);
    border: 1px solid var(--accent);
  }
  .primary:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .secondary {
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .primary:focus-visible,
  .secondary:focus-visible,
  .search input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .creator {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 16px 16px;
    background: var(--surface-2);
  }
  .creator-title {
    margin: 0;
    font-weight: 600;
  }
  .picker {
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  .search {
    display: block;
    padding: 12px 16px 0;
  }
  .search input {
    width: 100%;
    padding: 7px 10px;
    font: inherit;
    font-size: 13px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .excluded {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 14px;
    font-size: 12.5px;
    line-height: 1.45;
    border-left: 2px solid var(--warn);
    background: color-mix(in srgb, var(--warn) 6%, transparent);
    border-radius: 0 8px 8px 0;
  }
  .excluded p,
  .excluded ul {
    margin: 0;
  }
  .excluded ul {
    padding-left: 18px;
  }
  .excluded-title {
    font-weight: 600;
    color: var(--warn);
  }
  .excluded p:last-child {
    color: var(--text-muted);
  }
  code {
    font-size: 12px;
    color: var(--accent-2);
  }
  .limits {
    margin: 0;
    padding: 12px 16px 12px 34px;
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--text-muted);
  }
  /* Narrow windows: each rule becomes a small grid of its own (see RuleRow). */
  @media (max-width: 720px) {
    table,
    tbody {
      display: block;
    }
    thead {
      display: none;
    }
    .panel {
      display: block;
    }
    .panel > td {
      display: block;
    }
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }
</style>
