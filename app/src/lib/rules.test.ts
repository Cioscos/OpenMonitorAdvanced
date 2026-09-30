import defaultRulesFixture from '../test/fixtures/default-rules.json';
import { MOCK_SCHEMA } from './backend/mock';
import { MockSettings } from './backend/mockSettings';
import { translate } from './i18n/index.svelte';
import {
  applyOverride,
  conditionsFor,
  customPatch,
  defaultRuleIds,
  defaultScale,
  deltaFromDisplayIn,
  deltaToDisplayIn,
  errorsOf,
  fromDisplay,
  isModified,
  newCustomRule,
  overridePatch,
  parseNumber,
  resolvedRange,
  scalesFor,
  targetLabel,
  thresholdText,
  toDisplay,
} from './rules';
import type { LevelSpec, Rule, RulesSettings, Sensor } from './types';

const DEFAULTS = defaultRulesFixture as Rule[];
const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
const rules = (overrides: RulesSettings['overrides'] = {}, custom: Rule[] = []): RulesSettings => ({ overrides, custom });
const sensorOf = (id: string): Sensor => {
  const sensor = MOCK_SCHEMA.sensors.find((s) => s.id === id);
  if (!sensor) throw new Error(id);
  return sensor;
};
const FLAG: Sensor = {
  id: 'gpu/pci-0000:01:00.0/flag/throttle-thermal',
  deviceId: 'gpu/pci-0000:01:00.0',
  kind: 'flag',
  unit: 'boolean',
  label: { key: 'gpu.throttle.thermal' },
  source: 'nvml',
  category: 'flag',
};

test('default_ids_follow_backend_catalog', () => {
  expect(defaultRuleIds(DEFAULTS)).toEqual(DEFAULTS.map((r) => r.id));
  expect(defaultRuleIds(DEFAULTS)[0]).toBe('cpu-temp');
  // Whatever the backend sends, in its order: no list of ids kept in TypeScript.
  const reversed = [...DEFAULTS].reverse().slice(0, 3);
  expect(defaultRuleIds(reversed)).toEqual(['battery-low', 'ram-used', 'volume-used']);
  expect(defaultRuleIds([])).toEqual([]);
});

test('is_modified_follows_overrides', () => {
  expect(isModified('gpu-temp', rules())).toBe(false);
  // An override left empty (all its fields dropped on load) changes nothing.
  expect(isModified('gpu-temp', rules({ 'gpu-temp': {} }))).toBe(false);
  expect(isModified('gpu-temp', rules({ 'gpu-temp': { enabled: false } }))).toBe(true);
  expect(isModified('gpu-temp', rules({ 'gpu-temp': { warn: null } }))).toBe(true);
  expect(isModified('gpu-temp', rules({ 'ram-used': { enabled: false } }))).toBe(false);
});

test('applying an override replaces the fields it sets', () => {
  const gpu = DEFAULTS.find((r) => r.id === 'gpu-temp')!;
  const level: LevelSpec = { threshold: { fixed: 85 }, durationS: 5 };
  const effective = applyOverride(gpu, { warn: level, crit: null, notify: { warn: true, crit: true } });
  expect(effective.warn).toEqual(level);
  expect(effective.crit).toBeNull();
  expect(effective.notify).toEqual({ warn: true, crit: true });
  expect(effective.hysteresis).toEqual(gpu.hysteresis);
  expect(applyOverride(gpu, undefined)).toEqual(gpu);
});

test('new_rule_picks_the_condition_from_the_unit', () => {
  const flag = newCustomRule(FLAG);
  expect(flag.id).toMatch(/^custom-[0-9a-f-]{36}$/);
  expect(flag.target).toEqual({ sensor: FLAG.id });
  expect(flag.unit).toBe('boolean');
  expect(flag.condition).toBe('flagActive');
  for (const level of [flag.warn, flag.crit]) if (level) expect(level.threshold).toBeNull();
  expect(flag.warn ?? flag.crit).not.toBeNull();

  const temp = newCustomRule(sensorOf('gpu/pci-0000:01:00.0/temperature/core'));
  expect(temp.condition).toBe('above');
  expect(temp.unit).toBe('celsius');
  expect(temp.warn?.threshold).toEqual({ fixed: expect.any(Number) });
  expect(temp.hysteresis).toEqual({ amount: 3, durationS: 10 });
  expect(temp.notify).toEqual({ warn: false, crit: true });
  expect(temp.enabled).toBe(true);
  expect(newCustomRule(FLAG).id).not.toBe(flag.id);

  expect(conditionsFor('boolean')).toEqual(['flagActive']);
  expect(conditionsFor('celsius')).toEqual(['above', 'below']);
  expect(conditionsFor('bytes_per_second')).toEqual(['above', 'below']);
});

test('override_patch_replaces_a_whole_level', () => {
  const first: LevelSpec = { threshold: { fixed: 85 }, durationS: 30 };
  expect(overridePatch('gpu-temp', { warn: first })).toEqual({ rules: { overrides: { 'gpu-temp': { warn: first } } } });
  expect(overridePatch('gpu-temp', { crit: null })).toEqual({ rules: { overrides: { 'gpu-temp': { crit: null } } } });

  // Applied to the settings, the second level replaces the first whole and leaves the other fields.
  const mock = new MockSettings();
  mock.update(overridePatch('gpu-temp', { warn: first, enabled: false }));
  const second: LevelSpec = { threshold: { property: 'tempMaxC', offset: -5, fallback: 84 }, durationS: 20 };
  const state = mock.update(overridePatch('gpu-temp', { warn: second }));
  expect(state.settings.rules.overrides['gpu-temp']).toEqual({ warn: second, enabled: false });

  const custom = [newCustomRule(FLAG)];
  expect(customPatch(custom)).toEqual({ rules: { custom } });
});

test('targets read as every device of a kind or as device and sensor', () => {
  const gpu = DEFAULTS.find((r) => r.id === 'gpu-temp')!;
  expect(targetLabel(gpu, MOCK_SCHEMA, tEn)).toBe(tEn('rules.target.every.gpu'));
  const custom = newCustomRule(sensorOf('gpu/pci-0000:01:00.0/temperature/core'));
  expect(targetLabel(custom, MOCK_SCHEMA, tEn)).toBe(`Mock GeForce RTX 4080 › ${tEn('sensor.gpu.temperature.core')}`);
  const gone: Rule = { ...custom, target: { sensor: 'gpu/pci-9/temperature/core' } };
  expect(targetLabel(gone, MOCK_SCHEMA, tEn)).toBe(tEn('rules.target.missing', { id: 'gpu/pci-9/temperature/core' }));
});

test('temperature scales convert thresholds with the offset and hysteresis without', () => {
  const [f] = scalesFor('celsius', { temperature: 'f', throughput: 'bits' });
  expect(f.symbol).toBe('°F');
  expect(fromDisplay(185, f)).toBe(85);
  expect(toDisplay(85, f)).toBe(185);
  expect(deltaToDisplayIn(3, f)).toBeCloseTo(5.4, 10);
  expect(deltaFromDisplayIn(5.4, f)).toBeCloseTo(3, 10);
});

test('throughput scales follow the chosen unit and prefix', () => {
  const bits = scalesFor('bytes_per_second', { temperature: 'c', throughput: 'bits' });
  expect(bits.map((s) => s.symbol)).toEqual(['kbit/s', 'Mbit/s', 'Gbit/s']);
  const mbit = bits[1];
  expect(fromDisplay(100, mbit)).toBe(12_500_000);
  expect(toDisplay(12_500_000, mbit)).toBe(100);
  expect(deltaFromDisplayIn(8, mbit)).toBe(1_000_000);
  expect(defaultScale(bits, [12_500_000]).symbol).toBe('Mbit/s');

  const bytes = scalesFor('bytes_per_second', { temperature: 'c', throughput: 'bytes' });
  expect(bytes.map((s) => s.symbol)).toEqual(['KB/s', 'MB/s', 'GB/s']);
  expect(fromDisplay(1, bytes[1])).toBe(1024 * 1024);

  // A link speed is in bits whatever the setting.
  expect(scalesFor('bits_per_second', { temperature: 'c', throughput: 'bytes' })[1].symbol).toBe('Mbit/s');
  expect(scalesFor('percent', { temperature: 'c', throughput: 'bits' })[0].symbol).toBe('%');
});

test('numbers are parsed only when complete', () => {
  for (const text of ['', ' ', '-', '+', '12.', '12,', '.5', '1e3', 'abc', '1.2.3']) expect(parseNumber(text, false), text).toBeNull();
  expect(parseNumber('12.5', false)).toBe(12.5);
  expect(parseNumber('12,5', false)).toBe(12.5);
  expect(parseNumber('-3', false)).toBe(-3);
  expect(parseNumber(' 85 ', false)).toBe(85);
  expect(parseNumber('30', true)).toBe(30);
  for (const text of ['1.5', '-3', '']) expect(parseNumber(text, true), text).toBeNull();
});

test('resolved thresholds span the instances of a rule', () => {
  const status = [
    {
      ruleId: 'disk-temp',
      instances: [
        { sensorId: 'a', level: 'ok' as const, warn: 70, crit: 85, valid: true, problem: null },
        { sensorId: 'b', level: 'ok' as const, warn: 75, crit: 80, valid: true, problem: null },
      ],
    },
    { ruleId: 'gpu-temp', instances: [] },
  ];
  expect(resolvedRange(status, 'disk-temp', 'warn')).toEqual({ min: 70, max: 75 });
  expect(resolvedRange(status, 'disk-temp', 'crit')).toEqual({ min: 80, max: 85 });
  expect(resolvedRange(status, 'gpu-temp', 'warn')).toBeNull();
  expect(resolvedRange(status, 'nope', 'warn')).toBeNull();
});

test('errors are picked by the path of the rule', () => {
  const errors = {
    'rules.overrides.gpu-temp.crit': 'rules.error.order',
    'rules.overrides.gpu-temp.warn.durationS': 'rules.error.duration',
    'rules.overrides.gpu-temp-x.warn': 'rules.error.threshold',
    'rules.custom.1.levels': 'rules.error.noLevel',
    'general.intervalMs': 'settings.error.range',
  };
  expect(errorsOf(errors, 'rules.overrides.gpu-temp')).toEqual({ crit: 'rules.error.order', 'warn.durationS': 'rules.error.duration' });
  expect(errorsOf(errors, 'rules.custom.1')).toEqual({ levels: 'rules.error.noLevel' });
  expect(errorsOf(errors, 'rules.custom.0')).toEqual({});
});

test('a property threshold without instances shows its property and the fallback', () => {
  const cpu = DEFAULTS.find((r) => r.id === 'cpu-temp')!;
  const scale = defaultScale(scalesFor('celsius', { temperature: 'c', throughput: 'bytes' }), []);
  const ctx = { status: [], schema: null, scale, locale: 'en', t: tEn };
  const text = thresholdText(cpu, 'crit', ctx);
  expect(text).toBe('from TjMax (fallback 95 °C)');
  const none = { ...ctx, status: [{ ruleId: 'cpu-temp', instances: [] }] };
  expect(thresholdText(cpu, 'crit', none)).toBe(text);
});

test('an override equal to the shipped rule is not a modification', () => {
  const shipped = DEFAULTS.find((r) => r.id === 'gpu-temp')!;
  const same = rules({ 'gpu-temp': { enabled: true, notify: shipped.notify, warn: shipped.warn } });
  expect(isModified('gpu-temp', same)).toBe(true);
  expect(isModified('gpu-temp', same, shipped)).toBe(false);
  expect(isModified('gpu-temp', rules({ 'gpu-temp': { enabled: false } }), shipped)).toBe(true);
  expect(isModified('gpu-temp', rules({ 'gpu-temp': { hysteresis: { amount: 5, durationS: 10 } } }), shipped)).toBe(true);
});
