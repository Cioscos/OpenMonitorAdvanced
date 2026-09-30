import { sensorLabel } from './advanced/labels';
import { DASH } from './format';
import { catalogs, type Translate } from './i18n/index.svelte';
import type {
  LevelSpec,
  Rule,
  RuleCondition,
  RuleOverride,
  RulesSettings,
  RuleStatus,
  Schema,
  Sensor,
  SettingsPatch,
  TemperatureUnit,
  ThroughputUnit,
  Unit,
} from './types';
import { deltaFromDisplay, deltaToDisplay, fromDisplayTemperature, temperatureSymbol, toDisplayTemperature } from './units.svelte';

// Helpers of Settings › Rules and alerts (spec M5 §3.6). Rules keep thresholds and hysteresis in the
// sensor's base unit (°C, byte/s, %); everything here that says "display" is in the unit shown.

export type LevelName = 'warn' | 'crit';
export const LEVELS: readonly LevelName[] = ['warn', 'crit'];

/** The fields a built-in rule override may set. */
const OVERRIDE_FIELDS = ['enabled', 'warn', 'crit', 'hysteresis', 'notify'] as const;

/** Ids of the built-in rules, in the order the backend lists them (`get_default_rules`). */
export function defaultRuleIds(defaults: readonly Rule[]): string[] {
  return defaults.map((rule) => rule.id);
}

/** Whether the override of `ruleId` changes anything: an entry without fields does not. */
export function isModified(ruleId: string, rules: RulesSettings): boolean {
  const over = rules.overrides[ruleId];
  return over !== undefined && OVERRIDE_FIELDS.some((field) => over[field] !== undefined);
}

/** `rule` with the fields `over` sets replacing its own, like `Rule::with_override`. */
export function applyOverride(rule: Rule, over: RuleOverride | undefined): Rule {
  if (over === undefined) return rule;
  return {
    ...rule,
    enabled: over.enabled ?? rule.enabled,
    warn: over.warn !== undefined ? over.warn : rule.warn,
    crit: over.crit !== undefined ? over.crit : rule.crit,
    hysteresis: over.hysteresis ?? rule.hysteresis,
    notify: over.notify ?? rule.notify,
  };
}

/** One row of the rules table: an effective rule and where its settings live. */
export interface RuleEntry {
  rule: Rule;
  builtin: boolean;
  /** Settings path of its errors: `rules.overrides.<id>` or `rules.custom.<index>`. */
  base: string;
}

/** The built-in rules with their overrides, then the custom rules (`effective_rules`). */
export function ruleEntries(defaults: readonly Rule[], rules: RulesSettings): RuleEntry[] {
  return [
    ...defaults.map((rule) => ({ rule: applyOverride(rule, rules.overrides[rule.id]), builtin: true, base: `rules.overrides.${rule.id}` })),
    ...rules.custom.map((rule, i) => ({ rule, builtin: false, base: `rules.custom.${i}` })),
  ];
}

const sensorOf = (rule: Rule, schema: Schema | null): Sensor | undefined =>
  'sensor' in rule.target ? schema?.sensors.find((s) => s.id === (rule.target as { sensor: string }).sensor) : undefined;

/** `rule.<id>.name` for a built-in rule, the sensor's label for a custom one. */
export function ruleName(rule: Rule, builtin: boolean, schema: Schema | null, t: Translate): string {
  if (builtin) return t(`rule.${rule.id}.name`);
  const sensor = sensorOf(rule, schema);
  return sensor ? sensorLabel(sensor, t) : t('rule.custom.name');
}

/** "Every GPU" for a selector, "device › sensor" for one sensor. */
export function targetLabel(rule: Rule, schema: Schema | null, t: Translate): string {
  const target = rule.target;
  if (!('sensor' in target)) return t(`rules.target.every.${target.deviceKind}`);
  const sensor = sensorOf(rule, schema);
  if (!sensor) return t('rules.target.missing', { id: target.sensor });
  const device = schema?.devices.find((d) => d.id === sensor.deviceId);
  return `${device?.name ?? sensor.deviceId} › ${sensorLabel(sensor, t)}`;
}

/** The conditions a sensor of `unit` admits: only `flagActive` for flags, never for the others. */
export function conditionsFor(unit: Unit): RuleCondition[] {
  return unit === 'boolean' ? ['flagActive'] : ['above', 'below'];
}

/** A custom rule on `sensor` with the defaults of the core: a critical level for a flag, a warning otherwise. */
export function newCustomRule(sensor: Sensor): Rule {
  const flag = sensor.unit === 'boolean';
  return {
    id: `custom-${crypto.randomUUID()}`,
    target: { sensor: sensor.id },
    unit: sensor.unit,
    condition: conditionsFor(sensor.unit)[0],
    warn: flag ? null : { threshold: { fixed: 0 }, durationS: 30 },
    crit: flag ? { threshold: null, durationS: 10 } : null,
    hysteresis: { amount: 3, durationS: 10 },
    enabled: true,
    notify: { warn: false, crit: true },
  };
}

/** A patch of the override of a built-in rule; each field it names replaces the stored one whole. */
export function overridePatch(ruleId: string, change: Partial<RuleOverride>): SettingsPatch {
  return { rules: { overrides: { [ruleId]: change } } };
}

/** A patch replacing the whole list of custom rules. */
export function customPatch(rules: Rule[]): SettingsPatch {
  return { rules: { custom: rules } };
}

/** The override fields of a change to a rule. */
export function overrideOf(change: Partial<Rule>): RuleOverride {
  const over: RuleOverride = {};
  for (const field of OVERRIDE_FIELDS) {
    if (field in change) (over as Record<string, unknown>)[field] = change[field];
  }
  return over;
}

// --- Display units ---

export interface UnitPrefs {
  temperature: TemperatureUnit;
  throughput: ThroughputUnit;
}

/** A unit a threshold is shown and typed in: `base = display × per`, or a temperature scale. */
export interface DisplayScale {
  id: string;
  symbol: string;
  per: number;
  temperature?: TemperatureUnit;
}

const SYMBOLS: Partial<Record<Unit, string>> = {
  percent: '%',
  megahertz: 'MHz',
  watt: 'W',
  volt: 'V',
  ampere: 'A',
  rpm: 'RPM',
  joule: 'J',
  hours: 'h',
};

const scale = (symbol: string, per: number): DisplayScale => ({ id: symbol, symbol, per });
/** kbit/s, Mbit/s, Gbit/s for a base unit worth `bits` bits. */
const bitRates = (bits: number) => [scale('kbit/s', 1e3 / bits), scale('Mbit/s', 1e6 / bits), scale('Gbit/s', 1e9 / bits)];

/** The units a sensor of `unit` can be shown in, smallest first (binary steps for bytes, as the app shows them). */
export function scalesFor(unit: Unit, prefs: UnitPrefs): DisplayScale[] {
  switch (unit) {
    case 'celsius': {
      const symbol = temperatureSymbol(prefs.temperature);
      return [{ id: symbol, symbol, per: 1, temperature: prefs.temperature }];
    }
    case 'bytes_per_second':
      return prefs.throughput === 'bits' ? bitRates(8) : [scale('KB/s', 1024), scale('MB/s', 1024 ** 2), scale('GB/s', 1024 ** 3)];
    case 'bits_per_second':
      return bitRates(1);
    case 'bytes':
      return [scale('MB', 1024 ** 2), scale('GB', 1024 ** 3), scale('TB', 1024 ** 4)];
    default: {
      const symbol = SYMBOLS[unit] ?? '';
      return [{ id: symbol || unit, symbol, per: 1 }];
    }
  }
}

/** The largest unit in which every value is at least 1; the middle one without values. */
export function defaultScale(scales: readonly DisplayScale[], values: readonly number[]): DisplayScale {
  if (scales.length === 1) return scales[0];
  const sizes = values.filter((v) => Number.isFinite(v) && v !== 0).map(Math.abs);
  if (sizes.length === 0) return scales[Math.min(1, scales.length - 1)];
  const smallest = Math.min(...sizes);
  return [...scales].reverse().find((s) => smallest / s.per >= 1) ?? scales[0];
}

/** A threshold in the base unit, as shown. */
export function toDisplay(base: number, s: DisplayScale): number {
  return s.temperature ? toDisplayTemperature(base, s.temperature) : base / s.per;
}

/** A threshold as typed, in the base unit. */
export function fromDisplay(value: number, s: DisplayScale): number {
  return s.temperature ? fromDisplayTemperature(value, s.temperature) : value * s.per;
}

/** A difference (hysteresis) in the base unit, as shown: scaled, never offset. */
export function deltaToDisplayIn(base: number, s: DisplayScale): number {
  return s.temperature ? deltaToDisplay(base, s.temperature) : base / s.per;
}

export function deltaFromDisplayIn(value: number, s: DisplayScale): number {
  return s.temperature ? deltaFromDisplay(value, s.temperature) : value * s.per;
}

/** The fixed thresholds of a rule, to pick a unit that shows them well. */
export function fixedValues(rule: Rule): number[] {
  return LEVELS.flatMap((level) => {
    const threshold = rule[level]?.threshold;
    return threshold && 'fixed' in threshold ? [threshold.fixed] : [];
  });
}

// --- Numbers typed and shown ---

const inputFormats = new Map<string, Intl.NumberFormat>();

/** A number for an input: up to `digits` decimals, no grouping; empty for NaN. */
export function formatNumber(value: number, locale: string, digits = 3): string {
  if (!Number.isFinite(value)) return '';
  const key = `${locale}:${digits}`;
  let format = inputFormats.get(key);
  if (!format) {
    format = new Intl.NumberFormat(locale, { maximumFractionDigits: digits, useGrouping: false });
    inputFormats.set(key, format);
  }
  // `+ 0` turns -0 into 0.
  return format.format(value + 0);
}

/**
 * A complete number (point or comma as the decimal separator), else null: empty text, a lone sign
 * and a separator without digits after it are drafts. With `integer`, only whole non-negative numbers.
 */
export function parseNumber(text: string, integer: boolean): number | null {
  const s = text.trim();
  if (!(integer ? /^\d+$/ : /^[-+]?\d+(?:[.,]\d+)?$/).test(s)) return null;
  const value = Number(s.replace(',', '.'));
  return Number.isFinite(value) ? value : null;
}

/** A base value in `s`, for the table: up to one decimal. */
export function formatIn(base: number, s: DisplayScale, locale: string): string {
  const text = formatNumber(toDisplay(base, s), locale, 1);
  return s.symbol ? `${text} ${s.symbol}` : text;
}

/** "85–90 °C", or one value when both ends read the same. */
export function formatRangeIn(min: number, max: number, s: DisplayScale, locale: string): string {
  const [low, high] = [formatNumber(toDisplay(min, s), locale, 1), formatNumber(toDisplay(max, s), locale, 1)];
  const text = low === high ? low : `${low}–${high}`;
  return s.symbol ? `${text} ${s.symbol}` : text;
}

// --- Rule status ---

/** Lowest and highest resolved threshold of `level` over the instances of a rule; null without any. */
export function resolvedRange(status: readonly RuleStatus[], ruleId: string, level: LevelName): { min: number; max: number } | null {
  const values = (status.find((s) => s.ruleId === ruleId)?.instances ?? [])
    .map((i) => i[level])
    .filter((v): v is number => v !== null && Number.isFinite(v));
  return values.length === 0 ? null : { min: Math.min(...values), max: Math.max(...values) };
}

/** Whether the instances' devices have the property (`property`), none has it (`fallback`) or some do (`mixed`). */
function propertySource(status: RuleStatus | undefined, property: string, schema: Schema | null): 'property' | 'fallback' | 'mixed' {
  const found = (status?.instances ?? []).map((i) => {
    const sensor = schema?.sensors.find((s) => s.id === i.sensorId);
    const raw = schema?.devices.find((d) => d.id === sensor?.deviceId)?.properties?.[property];
    return raw !== undefined && raw.trim() !== '' && Number.isFinite(Number(raw));
  });
  if (found.every(Boolean)) return 'property';
  return found.some(Boolean) ? 'mixed' : 'fallback';
}

/** A short name of a device property for "95 °C, from TjMax". */
export function propertyName(property: string, t: Translate): string {
  const key = `rules.property.${property}`;
  return key in catalogs.en ? t(key) : t(`property.${property}`);
}

export interface ThresholdContext {
  status: readonly RuleStatus[];
  schema: Schema | null;
  scale: DisplayScale;
  locale: string;
  t: Translate;
}

/** A level's threshold as the table shows it: fixed, resolved from a property per instance, or a flag. */
export function thresholdText(rule: Rule, level: LevelName, ctx: ThresholdContext): string {
  const spec: LevelSpec | null = rule[level];
  if (spec === null) return DASH;
  const threshold = spec.threshold;
  if (threshold === null) return ctx.t('rules.threshold.flag');
  if ('fixed' in threshold) return formatIn(threshold.fixed, ctx.scale, ctx.locale);
  const range = resolvedRange(ctx.status, rule.id, level);
  if (range === null) return ctx.t('rules.threshold.fallback', { value: formatIn(threshold.fallback, ctx.scale, ctx.locale) });
  const value = formatRangeIn(range.min, range.max, ctx.scale, ctx.locale);
  const property = propertyName(threshold.property, ctx.t);
  const source = propertySource(
    ctx.status.find((s) => s.ruleId === rule.id),
    threshold.property,
    ctx.schema,
  );
  if (source === 'fallback') return ctx.t('rules.threshold.fallback', { value });
  return ctx.t(source === 'property' ? 'rules.threshold.fromProperty' : 'rules.threshold.mixed', { value, property });
}

// --- Errors ---

/** The errors of the rule at `base` (`rules.overrides.gpu-temp`), by path below it (`crit`, `warn.durationS`). */
export function errorsOf(errors: Record<string, string>, base: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [field, key] of Object.entries(errors)) {
    if (field === base) out[''] = key;
    else if (field.startsWith(`${base}.`)) out[field.slice(base.length + 1)] = key;
  }
  return out;
}
