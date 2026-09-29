import { MOCK_SCHEMA, mockValues } from '../backend/mock';
import { connectSettings, disconnectSettings } from '../../test/settings';
import { DASH, formatValue } from '../format';
import { catalogs, i18n, translate } from '../i18n/index.svelte';
import type { DeviceKind, Schema, Sensor, SensorStats } from '../types';
import {
  CATEGORY_ORDER,
  PROPERTY_ORDER,
  categoryLabel,
  defaultSeries,
  formatAverage,
  groupSensors,
  kpisFor,
  propertyRows,
  sourceCode,
  type StatsOf,
} from './pages';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
const GPU = 'gpu/pci-0000:01:00.0';

const sensor = (id: string, category: string): Sensor => ({
  id,
  deviceId: 'd',
  kind: 'load',
  unit: 'percent',
  label: { key: 'k' },
  source: 'mock',
  category,
});

test('categories follow the table order, unknown ones last and alphabetical', () => {
  const groups = groupSensors([
    sensor('a', 'zeta'),
    sensor('b', 'load'),
    sensor('c', 'alpha'),
    sensor('d', 'temperature'),
    sensor('e', 'load'),
    sensor('f', 'flag'),
  ]);
  expect(groups.map((g) => g.category)).toEqual(['temperature', 'load', 'flag', 'alpha', 'zeta']);
  expect(groups[1].sensors.map((s) => s.id)).toEqual(['b', 'e']);
});

test('gpu sensors of the mock group by category in table order', () => {
  const groups = groupSensors(MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU));
  expect(groups.map((g) => g.category)).toEqual(['temperature', 'load', 'clock', 'power', 'data']);
  expect(groups[0].sensors.map((s) => s.id)).toEqual([`${GPU}/temperature/core`, `${GPU}/temperature/hotspot`]);
  expect(groupSensors([])).toEqual([]);
});

test('every known category has a heading in both languages', () => {
  expect(CATEGORY_ORDER).toEqual(['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'counter', 'throughput', 'link', 'energy', 'flag']);
  for (const category of CATEGORY_ORDER) {
    expect(catalogs.en[`advanced.category.${category}`], category).toBeDefined();
    expect(catalogs.it[`advanced.category.${category}`], category).toBeDefined();
  }
  expect(categoryLabel('temperature', tEn)).toBe('Temperatures');
  expect(categoryLabel('mystery', tEn)).toBe('mystery');
});

test('every source has a badge description', () => {
  for (const source of ['pdh', 'win32', 'ip_helper', 'dxgi', 'd3dkmt', 'nvml', 'nvapi', 'adl', 'igcl', 'pnp', 'mock']) {
    expect(catalogs.en[`source.${source}`], source).toBeDefined();
  }
  expect(sourceCode('ip_helper')).toBe('IP HELPER');
  expect(sourceCode('nvml')).toBe('NVML');
});

test('the average of a flag is the share of time it was on', () => {
  expect(formatAverage({ min: 0, max: 1, avg: 0.25, count: 8 }, 'boolean', 'en', tEn)).toBe('25%');
  expect(formatAverage({ min: 30, max: 60, avg: 44.6, count: 8 }, 'percent', 'en', tEn)).toBe('45%');
  expect(formatAverage(null, 'celsius', 'en', tEn)).toBe(DASH);
  expect(formatAverage({ min: 0, max: 2e6, avg: 1e6, count: 4 }, 'bytes_per_second', 'en', tEn, { rate: 'bits' })).toBe('8.0 Mbit/s');
});

test('the average of a PCIe link unit is a dash: averaging generations or lanes is meaningless', () => {
  expect(formatAverage({ min: 1, max: 4, avg: 2.5, count: 4 }, 'pcie_generation', 'en', tEn)).toBe(DASH);
  expect(formatAverage({ min: 1, max: 16, avg: 8.5, count: 4 }, 'lanes', 'en', tEn)).toBe(DASH);
});

const values = mockValues(3);
const valueOf = (id: string) => {
  const i = MOCK_SCHEMA.sensors.findIndex((s) => s.id === id);
  return i < 0 ? null : values[i];
};
const noStats: StatsOf = () => null;
const DISK = 'storage/device-mock-ssd';
const NIC = 'network/mock-eth';
const ids = (kind: DeviceKind, deviceId: string, schema: Schema = MOCK_SCHEMA) => kpisFor(kind, schema, [deviceId]).map((k) => k.id);
const kpi = (kind: DeviceKind, deviceId: string, id: string, schema: Schema = MOCK_SCHEMA) =>
  kpisFor(kind, schema, [deviceId]).find((k) => k.id === id)!;
const without = (...keys: string[]): Schema => ({ ...MOCK_SCHEMA, sensors: MOCK_SCHEMA.sensors.filter((s) => !keys.includes(s.label.key)) });
const plus = (schema: Schema, extra: Sensor): Schema => ({ ...schema, sensors: [...schema.sensors, extra] });

test('cpu kpis without the service are unchanged: load, clock, busiest thread and peak load', () => {
  expect(ids('cpu', 'cpu/0')).toEqual(['load', 'clock', 'busiestThread', 'peakLoad']);
  expect(kpi('cpu', 'cpu/0', 'load').value(valueOf, noStats)).toBe(valueOf('cpu/0/load/total'));
  const threads = MOCK_SCHEMA.sensors.filter((s) => s.label.key === 'cpu.load.thread').map((s) => valueOf(s.id)!);
  expect(kpi('cpu', 'cpu/0', 'busiestThread').value(valueOf, noStats)).toBe(Math.max(...threads));
  expect(kpi('cpu', 'cpu/0', 'busiestThread').value(() => null, noStats)).toBeNull();
  const peak: SensorStats = { min: 1, max: 97, avg: 30, count: 5 };
  const stats: StatsOf = (id) => (id === 'cpu/0/load/total' ? peak : null);
  expect(kpi('cpu', 'cpu/0', 'peakLoad').value(valueOf, stats)).toBe(97);
  expect(kpi('cpu', 'cpu/0', 'peakLoad').value(valueOf, noStats)).toBeNull();
});

test('cpu kpis put temperature and power before clock with the service', () => {
  const temperature: Sensor = {
    id: 'cpu/0/temperature/package',
    deviceId: 'cpu/0',
    kind: 'temperature',
    unit: 'celsius',
    label: { key: 'cpu.temperature.package' },
    source: 'lhm',
    category: 'temperature',
  };
  const power: Sensor = {
    id: 'cpu/0/power/package',
    deviceId: 'cpu/0',
    kind: 'power',
    unit: 'watt',
    label: { key: 'cpu.power.package' },
    source: 'lhm',
    category: 'power',
  };
  const schema = plus(plus(MOCK_SCHEMA, temperature), power);
  expect(ids('cpu', 'cpu/0', schema)).toEqual(['load', 'temperature', 'power', 'clock']);
});

test('motherboard, fan controller and psu kpis take the first temperature, fan, voltage and power', () => {
  const DEVICE = 'motherboard/x';
  const schema: Schema = {
    revision: 1,
    devices: [{ id: DEVICE, kind: 'motherboard', name: 'MB' }],
    sensors: [
      { id: `${DEVICE}/fan/1`, deviceId: DEVICE, kind: 'fan', unit: 'rpm', label: { key: 'lhm.raw', arg: 'Fan #1' }, source: 'lhm', category: 'fan' },
      { id: `${DEVICE}/temperature/1`, deviceId: DEVICE, kind: 'temperature', unit: 'celsius', label: { key: 'lhm.raw', arg: 'System' }, source: 'lhm', category: 'temperature' },
      { id: `${DEVICE}/voltage/1`, deviceId: DEVICE, kind: 'voltage', unit: 'volt', label: { key: 'lhm.raw', arg: 'VIN0' }, source: 'lhm', category: 'voltage' },
      { id: `${DEVICE}/power/1`, deviceId: DEVICE, kind: 'power', unit: 'watt', label: { key: 'lhm.raw', arg: 'CPU' }, source: 'lhm', category: 'power' },
    ],
  };
  for (const kind of ['motherboard', 'fan_controller', 'psu'] as const) {
    expect(ids(kind, DEVICE, schema)).toEqual(['temperature', 'fan', 'voltage', 'power']);
  }
});

test('gpu kpis: load, temperature, power and vram with its total', () => {
  i18n.locale = 'en';
  expect(ids('gpu', GPU)).toEqual(['load', 'temperature', 'power', 'vram']);
  const vram = kpi('gpu', GPU, 'vram');
  expect(vram.unit).toBe('bytes');
  expect(vram.secondary?.(valueOf, noStats)).toBe(tEn('advanced.kpi.vramOf', { total: '16.0 GB' }));
  expect(vram.secondary?.(() => null, noStats)).toBeNull();
  expect(kpi('gpu', GPU, 'power').unit).toBe('watt');
});

test('gpu kpis fall back to hotspot and power percentage', () => {
  const powerPct: Sensor = {
    id: `${GPU}/percent/power-limit`,
    deviceId: GPU,
    kind: 'percent',
    unit: 'percent',
    label: { key: 'gpu.power.limitPercent' },
    source: 'd3dkmt',
    category: 'percent',
  };
  const schema = plus(without('gpu.temperature.core', 'gpu.power.board'), powerPct);
  expect(kpi('gpu', GPU, 'temperature', schema).value(valueOf, noStats)).toBe(valueOf(`${GPU}/temperature/hotspot`));
  expect(kpi('gpu', GPU, 'power', schema).unit).toBe('percent');
});

test('a gpu with few sensors shows only what exists, clock included', () => {
  const schema = without('gpu.temperature.core', 'gpu.temperature.hotspot', 'gpu.power.board', 'gpu.memory.dedicatedUsed');
  expect(ids('gpu', GPU, schema)).toEqual(['load', 'clock']);
});

test('memory kpis: load, used, total and available', () => {
  expect(ids('memory', 'memory/0')).toEqual(['load', 'used', 'total', 'available']);
  const available = kpi('memory', 'memory/0', 'available').value(valueOf, noStats);
  expect(available).toBe(valueOf('memory/0/data/total')! - valueOf('memory/0/data/used')!);
  expect(kpi('memory', 'memory/0', 'available').value(() => null, noStats)).toBeNull();
});

test('storage kpis use the temperature when the disk reports it, else the free space', () => {
  expect(ids('storage', DISK)).toEqual(['active', 'read', 'write', 'freeSpace']);
  expect(kpi('storage', DISK, 'freeSpace').secondary?.(valueOf, noStats)).toBe('C:');
  const temperature: Sensor = {
    id: `${DISK}/temperature/drive`,
    deviceId: DISK,
    kind: 'temperature',
    unit: 'celsius',
    label: { key: 'storage.temperature' },
    source: 'win32',
    category: 'temperature',
  };
  expect(ids('storage', DISK, plus(MOCK_SCHEMA, temperature))).toEqual(['active', 'read', 'write', 'temperature']);
});

test('network kpis: down, up, link speed and peak download', () => {
  expect(ids('network', NIC)).toEqual(['down', 'up', 'linkSpeed', 'peakDown']);
  expect(kpi('network', NIC, 'linkSpeed').unit).toBe('bits_per_second');
  const stats: StatsOf = (id) => (id === `${NIC}/throughput/down` ? { min: 0, max: 5e6, avg: 1e6, count: 9 } : null);
  expect(kpi('network', NIC, 'peakDown').value(valueOf, stats)).toBe(5e6);
});

test('kinds without kpi definitions get none', () => {
  expect(kpisFor('motherboard', MOCK_SCHEMA, ['cpu/0'])).toEqual([]);
});

test('every kpi label is translated in both languages', () => {
  const pages: [DeviceKind, string][] = [['cpu', 'cpu/0'], ['gpu', GPU], ['memory', 'memory/0'], ['storage', DISK], ['network', NIC]];
  const keys = ['advanced.kpi.vramOf', 'advanced.kpi.temperature', 'advanced.kpi.clock', 'advanced.info.yes', 'advanced.info.no'];
  for (const [kind, id] of pages) keys.push(...kpisFor(kind, MOCK_SCHEMA, [id]).map((k) => k.labelKey));
  for (const key of keys) {
    expect(catalogs.en[key], key).toBeDefined();
    expect(catalogs.it[key], key).toBeDefined();
  }
});

test('default series per kind', () => {
  expect(defaultSeries('cpu', MOCK_SCHEMA, ['cpu/0'])).toEqual(['cpu/0/load/total', 'cpu/0/clock/effective']);
  expect(defaultSeries('gpu', MOCK_SCHEMA, [GPU])).toEqual([`${GPU}/load/core`, `${GPU}/temperature/core`]);
  expect(defaultSeries('gpu', without('gpu.temperature.core'), [GPU])).toEqual([`${GPU}/load/core`, `${GPU}/temperature/hotspot`]);
  expect(defaultSeries('memory', MOCK_SCHEMA, ['memory/0'])).toEqual(['memory/0/load/used']);
  expect(defaultSeries('storage', MOCK_SCHEMA, [DISK])).toEqual([`${DISK}/throughput/read`, `${DISK}/throughput/write`]);
  expect(defaultSeries('network', MOCK_SCHEMA, [NIC])).toEqual([`${NIC}/throughput/down`, `${NIC}/throughput/up`]);
  expect(defaultSeries('motherboard', MOCK_SCHEMA, ['cpu/0'])).toEqual(['cpu/0/load/total']);
  expect(defaultSeries('battery', MOCK_SCHEMA, ['none'])).toEqual([]);
});

test('device properties are translated, formatted and ordered', () => {
  const rows = propertyRows(
    {
      id: GPU,
      kind: 'gpu',
      name: 'GPU',
      properties: {
        tempSlowdownC: '94',
        zeta: 'abc',
        pcieMaxWidth: '16',
        integrated: 'false',
        pcieMaxGen: '4',
        pciAddress: '0000:01:00.0',
        powerLimitDefaultW: '320',
        tempMaxC: 'n/a',
      },
    },
    'en',
    tEn,
  );
  expect(rows.map((r) => r.key)).toEqual(['pciAddress', 'integrated', 'pcieMaxGen', 'pcieMaxWidth', 'powerLimitDefaultW', 'tempSlowdownC', 'tempMaxC', 'zeta']);
  expect(rows.map((r) => r.value)).toEqual([
    '0000:01:00.0',
    tEn('advanced.info.no'),
    formatValue(4, 'pcie_generation', 'en', tEn),
    formatValue(16, 'lanes', 'en', tEn),
    '320',
    '94 °C',
    'n/a',
    'abc',
  ]);
  expect(rows[0].label).toBe(tEn('property.pciAddress'));
  expect(rows.at(-1)?.label).toBe('zeta');
  expect(propertyRows({ id: GPU, kind: 'gpu', name: 'GPU', properties: { powerLimitMaxW: '12345.5', integrated: 'true' } }, 'it', (k) => translate('it', k)).map((r) => r.value)).toEqual(['Sì', '12.345,5']);
  expect(propertyRows({ id: 'x', kind: 'cpu', name: 'x' }, 'en', tEn)).toEqual([]);
});

test('the SMART switch property is for the settings, not the device page', () => {
  const disk = { id: 'storage/a', kind: 'storage' as const, name: 'Disk', properties: { smartSelectable: 'true', tempWarningC: '70' } };
  expect(propertyRows(disk, 'en', tEn).map((r) => r.key)).toEqual(['tempWarningC']);
  expect(propertyRows({ ...disk, properties: { smartSelectable: 'false' } }, 'en', tEn)).toEqual([]);
});

test('temperature limits follow the temperature unit and their labels carry no unit', async () => {
  const device = { id: GPU, kind: 'gpu' as const, name: 'GPU', properties: { tempSlowdownC: '94', tempCriticalC: '100' } };
  expect(propertyRows(device, 'en', tEn).map((r) => r.value)).toEqual(['94 °C', '100 °C']);
  await connectSettings({ general: { temperatureUnit: 'f' } });
  try {
    expect(propertyRows(device, 'en', tEn).map((r) => r.value)).toEqual(['201 °F', '212 °F']);
  } finally {
    disconnectSettings();
  }
  for (const locale of ['en', 'it'] as const) {
    for (const key of PROPERTY_ORDER.filter((k) => k.startsWith('temp'))) {
      expect(translate(locale, `property.${key}`)).not.toContain('°');
    }
  }
});

test('every known property has a label in both languages', () => {
  for (const key of PROPERTY_ORDER) {
    expect(catalogs.en[`property.${key}`], key).toBeDefined();
    expect(catalogs.it[`property.${key}`], key).toBeDefined();
  }
});
