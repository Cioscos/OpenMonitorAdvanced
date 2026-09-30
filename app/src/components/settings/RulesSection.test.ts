import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { createMockBackend, SERVICE_MOCK_SCHEMA } from '../../lib/backend/mock';
import { MockSettings } from '../../lib/backend/mockSettings';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import { newCustomRule } from '../../lib/rules';
import { settings } from '../../lib/settings.svelte';
import type { Rule, RuleStatus, Schema, Sensor, SettingsDiagnostic, SettingsPatch, ThresholdSource } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import defaultRulesFixture from '../../test/fixtures/default-rules.json';
import { disconnectSettings } from '../../test/settings';
import RulesSection from './RulesSection.svelte';

const GPU = 'gpu/pci-0000:01:00.0';
const SSD = 'storage/device-mock-ssd';
const USB = 'storage/device-mock-usb';
const DEFAULTS = defaultRulesFixture as Rule[];

const sensor = (id: string, deviceId: string, kind: Sensor['kind'], unit: Sensor['unit'], key: string): Sensor => ({
  id,
  deviceId,
  kind,
  unit,
  label: { key },
  source: 'lhm',
  category: kind,
});

/** The service mock with TjMax on the CPU, disk limits and a GPU throttling flag. */
const SCHEMA: Schema = {
  ...SERVICE_MOCK_SCHEMA,
  devices: SERVICE_MOCK_SCHEMA.devices.map((d) =>
    d.id === 'cpu/0'
      ? { ...d, properties: { tjMaxC: '95' } }
      : d.kind === 'storage'
        ? { ...d, properties: { ...d.properties, tempWarningC: d.id === SSD ? '70' : '75', tempCriticalC: '85' } }
        : d,
  ),
  sensors: [
    ...SERVICE_MOCK_SCHEMA.sensors,
    sensor(`${SSD}/temperature/drive`, SSD, 'temperature', 'celsius', 'storage.temperature'),
    sensor(`${USB}/temperature/drive`, USB, 'temperature', 'celsius', 'storage.temperature'),
    sensor(`${GPU}/flag/throttle-thermal`, GPU, 'flag', 'boolean', 'gpu.throttle.thermal'),
  ],
};

const instance = (sensorId: string, warn: number | null, crit: number | null, source: ThresholdSource = 'fixed') => ({
  sensorId,
  level: 'ok' as const,
  warn,
  crit,
  warnSource: warn === null ? null : source,
  critSource: crit === null ? null : source,
  valid: true,
  problem: null,
});

/** Every default rule, as the engine reports it on SCHEMA. */
function status(cpuCrit = 95): RuleStatus[] {
  return DEFAULTS.map((rule) => {
    switch (rule.id) {
      case 'cpu-temp':
        return { ruleId: rule.id, instances: [instance('cpu/0/temperature/package', cpuCrit - 10, cpuCrit, 'property')] };
      case 'gpu-temp':
        return { ruleId: rule.id, instances: [instance(`${GPU}/temperature/core`, 83, 90)] };
      case 'disk-temp':
        return {
          ruleId: rule.id,
          instances: [instance(`${SSD}/temperature/drive`, 70, 85, 'property'), instance(`${USB}/temperature/drive`, 75, 85, 'property')],
        };
      default:
        return { ruleId: rule.id, instances: [] };
    }
  });
}

const customOn = (sensorId: string, over: Partial<Rule> = {}): Rule => {
  const found = SCHEMA.sensors.find((s) => s.id === sensorId)!;
  return { ...newCustomRule(found), warn: { threshold: { fixed: 80 }, durationS: 30 }, crit: { threshold: { fixed: 90 }, durationS: 10 }, ...over };
};

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(() => {
  cleanup();
  disconnectSettings();
  vi.useRealTimers();
});

interface SetupOptions {
  patch?: SettingsPatch;
  diagnostics?: SettingsDiagnostic[];
  /** Makes each patch wait this long before it applies (real timers). */
  slowMs?: number;
  /** The shell's `oma:settings` event never arrives: only command replies update the store. */
  noEvents?: boolean;
}

async function setup(options: SetupOptions = {}) {
  const backend = new FakeBackend(SCHEMA);
  if (options.diagnostics) backend.settings = new MockSettings({ kind: 'ok' }, options.diagnostics);
  backend.ruleStatus = status();
  if (options.noEvents) backend.onSettings = async () => () => {};
  await settings.connect(backend);
  if (options.patch) await settings.update(options.patch);
  const store = new LiveStore();
  store.applySchema(SCHEMA);
  const patches: SettingsPatch[] = [];
  const update = backend.updateSettings.bind(backend);
  backend.updateSettings = async (p) => {
    patches.push(JSON.parse(JSON.stringify(p)) as SettingsPatch);
    if (options.slowMs) await new Promise((resolve) => setTimeout(resolve, options.slowMs));
    return update(p);
  };
  const view = render(RulesSection, { store, backend });
  await screen.findByRole('row', { name: t('rule.gpu-temp.name') });
  return { backend, patches, view };
}

const row = (name: string) => screen.getByRole('row', { name });
const open = (name: string) => fireEvent.click(within(row(name)).getByRole('button', { name: t('rules.edit') }));
const input = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
async function typeAndEnter(label: string, value: string) {
  const field = input(label);
  await fireEvent.input(field, { target: { value } });
  await fireEvent.keyDown(field, { key: 'Enter' });
}
const custom = () => settings.state?.settings.rules.custom ?? [];

test('table_lists_defaults_with_resolved_property_thresholds', async () => {
  await setup();
  const names = screen.getAllByRole('row').slice(1, DEFAULTS.length + 1).map((r) => r.getAttribute('aria-label') ?? '');
  expect(names).toEqual(DEFAULTS.map((r) => t(`rule.${r.id}.name`)));

  const cpu = row(t('rule.cpu-temp.name'));
  expect(cpu.textContent).toContain(t('rules.threshold.fromProperty', { value: '85 °C', property: t('rules.property.tjMaxC') }));
  expect(cpu.textContent).toContain(t('rules.threshold.fromProperty', { value: '95 °C', property: t('rules.property.tjMaxC') }));
  expect(cpu.textContent).toContain('30 s');

  // Several instances read as a range.
  expect(row(t('rule.disk-temp.name')).textContent).toContain('70–75 °C');

  const gpu = row(t('rule.gpu-temp.name'));
  expect(gpu.textContent).toContain(t('rules.target.every.gpu'));
  expect(gpu.textContent).toContain('83 °C');
  expect(gpu.textContent).toContain('90 °C');
  expect(within(gpu).getByRole('switch', { name: t('rules.enabledFor', { name: t('rule.gpu-temp.name') }) }).getAttribute('aria-checked')).toBe(
    'true',
  );
  const bell = within(gpu).getByRole('button', { name: t('rules.notify.crit') });
  expect(bell.getAttribute('aria-pressed')).toBe('true');
  expect(within(gpu).getByRole('button', { name: t('rules.notify.warn') }).getAttribute('aria-pressed')).toBe('false');

  // A flag has no threshold, only its level.
  expect(row(t('rule.cpu-throttle.name')).textContent).toContain(t('rules.threshold.flag'));
  expect(screen.getByText(t('rules.limits.title'))).toBeTruthy();
});

test('fahrenheit thresholds are shown in the chosen unit', async () => {
  await setup({ patch: { general: { temperatureUnit: 'f' } } });
  expect(row(t('rule.gpu-temp.name')).textContent).toContain('181.4 °F');
});

test('editing_a_threshold_sends_a_fixed_override', async () => {
  const { patches } = await setup();
  await open(t('rule.cpu-temp.name'));
  // The property threshold is shown resolved, and becomes fixed once edited.
  expect(screen.getByText(t('rules.threshold.fromProperty', { value: '95 °C', property: t('rules.property.tjMaxC') }), { selector: '.resolved' })).toBeTruthy();
  await typeAndEnter(t('rules.editor.critThreshold'), '100');
  await vi.waitFor(() => expect(patches).toHaveLength(1));
  expect(patches[0]).toEqual({ rules: { overrides: { 'cpu-temp': { crit: { threshold: { fixed: 100 }, durationS: 10 } } } } });
  await vi.waitFor(() => expect(within(row(t('rule.cpu-temp.name'))).getByText(t('rules.modified'))).toBeTruthy());
});

test('fahrenheit input is saved in celsius and shown again as typed', async () => {
  const { patches } = await setup({ patch: { general: { temperatureUnit: 'f' } } });
  await open(t('rule.gpu-temp.name'));
  await typeAndEnter(t('rules.editor.warnThreshold'), '185');
  await vi.waitFor(() => expect(patches).toHaveLength(1));
  expect(patches[0]).toEqual({ rules: { overrides: { 'gpu-temp': { warn: { threshold: { fixed: 85 }, durationS: 30 } } } } });
  await typeAndEnter(t('rules.editor.hysteresisAmount'), '9');
  await vi.waitFor(() => expect(patches).toHaveLength(2));
  expect(patches[1]).toEqual({ rules: { overrides: { 'gpu-temp': { hysteresis: { amount: 5, durationS: 10 } } } } });
  await vi.waitFor(() => expect(input(t('rules.editor.warnThreshold')).value).toBe('185'));
  expect(input(t('rules.editor.hysteresisAmount')).value).toBe('9');
});

test('reset_calls_the_backend', async () => {
  const { backend } = await setup({ patch: { rules: { overrides: { 'gpu-temp': { enabled: false } } } } });
  const gpu = row(t('rule.gpu-temp.name'));
  expect(within(gpu).getByText(t('rules.modified'))).toBeTruthy();
  // An empty override is not a change: no badge, nothing to restore.
  expect(within(row(t('rule.ram-used.name'))).queryByRole('button', { name: t('rules.restore') })).toBeNull();
  await fireEvent.click(within(gpu).getByRole('button', { name: t('rules.restore') }));
  await vi.waitFor(() => expect(backend.resetRuleOverrideCalls).toEqual(['gpu-temp']));
  await vi.waitFor(() => expect(within(row(t('rule.gpu-temp.name'))).queryByText(t('rules.modified'))).toBeNull());
});

test('an override emptied on load shows no badge', async () => {
  await setup({ patch: { rules: { overrides: { 'ram-used': {} } } } });
  expect(within(row(t('rule.ram-used.name'))).queryByText(t('rules.modified'))).toBeNull();
  expect(within(row(t('rule.ram-used.name'))).queryByRole('button', { name: t('rules.restore') })).toBeNull();
});

test('reset_accepts_returned_settings_state', async () => {
  const { backend } = await setup({ patch: { rules: { overrides: { 'gpu-temp': { enabled: false } } } }, noEvents: true });
  settings.errors = { 'rules.overrides.gpu-temp.crit': 'rules.error.order', 'general.intervalMs': 'settings.error.range' };
  const before = settings.state!.seq;
  await fireEvent.click(within(row(t('rule.gpu-temp.name'))).getByRole('button', { name: t('rules.restore') }));
  await vi.waitFor(() => expect(settings.state!.seq).toBeGreaterThan(before));
  expect(backend.resetRuleOverrideCalls).toEqual(['gpu-temp']);
  expect(settings.state!.settings.rules.overrides).toEqual({});
  expect(settings.errors).toEqual({ 'general.intervalMs': 'settings.error.range' });
  await vi.waitFor(() => expect(within(row(t('rule.gpu-temp.name'))).queryByText(t('rules.modified'))).toBeNull());
});

test('validation_error_appears_next_to_the_field', async () => {
  const { patches } = await setup();
  await open(t('rule.gpu-temp.name'));
  await typeAndEnter(t('rules.editor.critThreshold'), '80');
  await vi.waitFor(() => expect(patches).toHaveLength(1));
  const field = input(t('rules.editor.critThreshold'));
  await vi.waitFor(() => expect(field.getAttribute('aria-invalid')).toBe('true'));
  const error = document.getElementById(field.getAttribute('aria-describedby')!.split(' ').find((id) => id.endsWith('-error'))!);
  expect(error?.textContent).toBe(t('rules.error.order'));
  expect(settings.state?.settings.rules.overrides).toEqual({});

  // A valid value clears the error of the rule.
  await typeAndEnter(t('rules.editor.critThreshold'), '95');
  await vi.waitFor(() => expect(input(t('rules.editor.critThreshold')).getAttribute('aria-invalid')).toBeNull());
  expect(screen.queryByText(t('rules.error.order'))).toBeNull();
});

test('delete_removes_a_custom_rule', async () => {
  const keep = customOn(`${GPU}/power/board`);
  const drop = customOn(`${GPU}/temperature/core`);
  const { patches } = await setup({ patch: { rules: { custom: [keep, drop] } } });
  const name = t('sensor.gpu.temperature.core');
  await fireEvent.click(within(row(name)).getByRole('button', { name: t('rules.delete') }));
  await vi.waitFor(() => expect(patches).toHaveLength(1));
  expect(patches[0]).toEqual({ rules: { custom: [keep] } });
  await vi.waitFor(() => expect(screen.queryByRole('row', { name })).toBeNull());
  expect(custom()).toEqual([keep]);
});

async function openNewRule(query: string, sensorId: string) {
  await fireEvent.click(screen.getByRole('button', { name: t('rules.new') }));
  await fireEvent.input(screen.getByRole('searchbox', { name: t('rules.editor.search') }), { target: { value: query } });
  await fireEvent.change(screen.getByLabelText(t('rules.editor.sensor')), { target: { value: sensorId } });
}

test('create_rule_for_a_flag_offers_only_flag_active', async () => {
  await setup();
  await openNewRule('throttl', `${GPU}/flag/throttle-thermal`);
  const options = within(screen.getByLabelText(t('rules.editor.sensor'))).getAllByRole('option');
  expect(options.map((o) => (o as HTMLOptionElement).value)).toEqual(['', `${GPU}/flag/throttle-thermal`]);
  const conditions = within(screen.getByRole('radiogroup', { name: t('rules.editor.condition') })).getAllByRole('radio');
  expect(conditions.map((r) => (r as HTMLInputElement).value)).toEqual(['flagActive']);
  expect(screen.queryByLabelText(t('rules.editor.warnThreshold'))).toBeNull();

  await fireEvent.input(screen.getByRole('searchbox', { name: t('rules.editor.search') }), { target: { value: 'rtx' } });
  await fireEvent.change(screen.getByLabelText(t('rules.editor.sensor')), { target: { value: `${GPU}/temperature/core` } });
  const next = within(screen.getByRole('radiogroup', { name: t('rules.editor.condition') })).getAllByRole('radio');
  expect(next.map((r) => (r as HTMLInputElement).value)).toEqual(['above', 'below']);
});

test('creating a rule adds it once it is complete', async () => {
  const { patches } = await setup();
  await openNewRule('rtx', `${GPU}/temperature/core`);
  // Only the critical level is on, with an empty threshold; each switch says which level it uses.
  const warnLevel = screen.getByRole('switch', { name: 'Use the Warning level' });
  const critLevel = screen.getByRole('switch', { name: 'Use the Critical level' });
  expect(warnLevel.getAttribute('aria-checked')).toBe('false');
  expect(critLevel.getAttribute('aria-checked')).toBe('true');
  expect(screen.queryByLabelText(t('rules.editor.warnThreshold'))).toBeNull();
  expect(input(t('rules.editor.critThreshold')).value).toBe('');
  expect(input(t('rules.editor.critDuration')).value).toBe('10');
  const create = screen.getByRole('button', { name: t('rules.create') }) as HTMLButtonElement;
  expect(create.disabled).toBe(true);
  await typeAndEnter(t('rules.editor.critThreshold'), '80');
  expect(patches).toEqual([]);
  expect(create.disabled).toBe(false);
  await fireEvent.click(create);
  await vi.waitFor(() => expect(custom()).toHaveLength(1));
  expect(patches).toHaveLength(1);
  expect(custom()[0]).toMatchObject({ target: { sensor: `${GPU}/temperature/core` }, condition: 'above', warn: null, crit: { threshold: { fixed: 80 }, durationS: 10 } });
  expect(screen.getByRole('row', { name: t('sensor.gpu.temperature.core') })).toBeTruthy();
});

test('the level switches name their level in Italian too', () => {
  i18n.locale = 'it';
  expect(t('rules.editor.warnLevel')).toBe('Usa il livello Attenzione');
  expect(t('rules.editor.critLevel')).toBe('Usa il livello Critico');
});

test('incomplete_numeric_input_is_not_saved', async () => {
  const { patches } = await setup();
  await open(t('rule.gpu-temp.name'));
  for (const text of ['', '-', '8.', '8,']) {
    await typeAndEnter(t('rules.editor.warnThreshold'), text);
    await fireEvent.blur(input(t('rules.editor.warnThreshold')));
  }
  await typeAndEnter(t('rules.editor.warnDuration'), '1.5');
  expect(patches).toEqual([]);
  // A draft that was not saved goes back to the value in effect.
  expect(input(t('rules.editor.warnThreshold')).value).toBe('83');

  await fireEvent.input(input(t('rules.editor.warnThreshold')), { target: { value: '84,5' } });
  await fireEvent.blur(input(t('rules.editor.warnThreshold')));
  await vi.waitFor(() => expect(patches).toHaveLength(1));
  expect(patches[0]).toEqual({ rules: { overrides: { 'gpu-temp': { warn: { threshold: { fixed: 84.5 }, durationS: 30 } } } } });
});

test('cancel_new_rule_does_not_persist', async () => {
  const { patches } = await setup();
  await openNewRule('rtx', `${GPU}/temperature/core`);
  await typeAndEnter(t('rules.editor.critThreshold'), '80');
  await fireEvent.click(screen.getByRole('button', { name: t('rules.cancel') }));
  expect(screen.queryByLabelText(t('rules.editor.critThreshold'))).toBeNull();
  expect(patches).toEqual([]);
  expect(custom()).toEqual([]);
});

test('rapid_custom_edits_preserve_both_changes', async () => {
  const a = customOn(`${GPU}/temperature/core`);
  const b = customOn(`${GPU}/power/board`, { warn: { threshold: { fixed: 200 }, durationS: 30 }, crit: { threshold: { fixed: 300 }, durationS: 10 } });
  const { patches } = await setup({ patch: { rules: { custom: [a, b] } }, slowMs: 20 });
  const aName = t('sensor.gpu.temperature.core');
  const bName = t('sensor.gpu.power.board');
  // Both clicks before the first patch has come back.
  await fireEvent.click(within(row(aName)).getByRole('switch', { name: t('rules.enabledFor', { name: aName }) }));
  await fireEvent.click(within(row(bName)).getByRole('button', { name: t('rules.notify.warn') }));
  await vi.waitFor(() => expect(patches).toHaveLength(2), { timeout: 2000 });
  await vi.waitFor(() => expect(custom()[1]?.notify.warn).toBe(true), { timeout: 2000 });
  expect(custom()[0].enabled).toBe(false);
  expect(custom()[1].notify).toEqual({ warn: true, crit: true });
});

test('throughput_threshold_and_hysteresis_round_trip', async () => {
  await setup();
  await openNewRule('ethernet', 'network/mock-eth/throughput/down');
  expect((screen.getByLabelText(t('rules.editor.scale')) as HTMLSelectElement).value).toBe('Mbit/s');
  await typeAndEnter(t('rules.editor.critThreshold'), '100');
  // A change of preference while the draft is open keeps the draft's unit.
  await settings.update({ general: { throughputUnit: 'bytes' } });
  expect((screen.getByLabelText(t('rules.editor.scale')) as HTMLSelectElement).value).toBe('Mbit/s');
  expect(input(t('rules.editor.critThreshold')).value).toBe('100');
  await typeAndEnter(t('rules.editor.hysteresisAmount'), '8');
  await fireEvent.click(screen.getByRole('button', { name: t('rules.create') }));
  await vi.waitFor(() => expect(custom()).toHaveLength(1));
  expect(custom()[0].crit?.threshold).toEqual({ fixed: 12_500_000 });
  expect(custom()[0].hysteresis.amount).toBe(1_000_000);

  await settings.update({ general: { throughputUnit: 'bits' } });
  await open(t('sensor.network.down'));
  expect((screen.getByLabelText(t('rules.editor.scale')) as HTMLSelectElement).value).toBe('Mbit/s');
  expect(input(t('rules.editor.critThreshold')).value).toBe('100');
  expect(input(t('rules.editor.hysteresisAmount')).value).toBe('8');
});

test('rule_status_refreshes_without_health_event', async () => {
  vi.useFakeTimers();
  const { backend } = await setup();
  const text = (value: string) => t('rules.threshold.fromProperty', { value, property: t('rules.property.tjMaxC') });
  expect(row(t('rule.cpu-temp.name')).textContent).toContain(text('95 °C'));
  backend.ruleStatus = status(90);
  await vi.advanceTimersByTimeAsync(1000);
  await vi.waitFor(() => expect(row(t('rule.cpu-temp.name')).textContent).toContain(text('90 °C')));
  expect(backend.healthCalls).toEqual([]);
});

test('status_poll_stops_on_unmount', async () => {
  vi.useFakeTimers();
  const { backend, view } = await setup();
  await vi.advanceTimersByTimeAsync(3000);
  const calls = backend.ruleStatusCalls;
  expect(calls).toBeGreaterThanOrEqual(3);
  expect(calls).toBeLessThanOrEqual(4);

  // One request at a time: a slow reply holds the next one back.
  let release: () => void = () => {};
  backend.getRuleStatus = async () => {
    backend.ruleStatusCalls++;
    await new Promise<void>((resolve) => (release = resolve));
    return backend.ruleStatus;
  };
  await vi.advanceTimersByTimeAsync(5000);
  const held = backend.ruleStatusCalls;
  expect(held).toBe(calls + 1);
  release();
  view.unmount();
  await vi.advanceTimersByTimeAsync(5000);
  expect(backend.ruleStatusCalls).toBe(held);
});

test('default_catalog_matches_rust_fixture', async () => {
  const fake = new FakeBackend(SCHEMA);
  expect(await fake.getDefaultRules()).toEqual(DEFAULTS);
  expect(await createMockBackend().getDefaultRules()).toEqual(DEFAULTS);
  // The mock's settings know the same built-in ids: an override of each is accepted.
  const mock = new MockSettings();
  const state = mock.update({ rules: { overrides: Object.fromEntries(DEFAULTS.map((r) => [r.id, { enabled: false }])) } });
  expect(Object.keys(state.settings.rules.overrides)).toEqual(DEFAULTS.map((r) => r.id));
});

test('rules left out of the settings file are listed', async () => {
  await setup({ diagnostics: [{ kind: 'invalidRule', path: 'rules.custom.2', key: 'rules.error.order' }, { kind: 'wrongType', path: 'general.language' }] });
  const list = screen.getByRole('list', { name: t('rules.excluded') });
  expect(within(list).getAllByRole('listitem')).toHaveLength(1);
  expect(list.textContent).toContain('rules.custom.2');
  expect(list.textContent).toContain(t('rules.error.order'));
});

test('enabling a rule again drops its now-empty override', async () => {
  const { backend } = await setup();
  const name = t('rule.gpu-temp.name');
  const toggle = () => within(row(name)).getByRole('switch', { name: t('rules.enabledFor', { name }) });
  await fireEvent.click(toggle());
  await vi.waitFor(() => expect(within(row(name)).getByText(t('rules.modified'))).toBeTruthy());
  await fireEvent.click(toggle());
  await vi.waitFor(() => expect(within(row(name)).queryByText(t('rules.modified'))).toBeNull());
  // Nothing left to keep: the entry goes, as with "Restore".
  expect(backend.resetRuleOverrideCalls).toEqual(['gpu-temp']);
  expect(settings.state!.settings.rules.overrides).toEqual({});
});

test('the out-of-order summary agrees with the count', async () => {
  const problem = (sensorId: string) => ({ ...instance(sensorId, 90, 80), valid: false, problem: 'order' as const });
  // Fake timers: the next poll comes a second after the first reply, as long as `waitFor` waits.
  vi.useFakeTimers();
  const { backend } = await setup();
  backend.ruleStatus = status().map((s) =>
    s.ruleId === 'disk-temp' ? { ...s, instances: [problem(`${SSD}/temperature/drive`)] } : s.ruleId === 'gpu-temp' ? { ...s, instances: [problem('a'), problem('b')] } : s,
  );
  await vi.advanceTimersByTimeAsync(1000);
  await vi.waitFor(() => expect(row(t('rule.disk-temp.name')).textContent).toContain('out of order on 1 sensor:'));
  expect(row(t('rule.gpu-temp.name')).textContent).toContain('out of order on 2 sensors:');
});
