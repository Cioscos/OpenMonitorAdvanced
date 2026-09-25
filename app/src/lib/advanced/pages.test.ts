import { MOCK_SCHEMA } from '../backend/mock';
import { DASH } from '../format';
import { catalogs, translate } from '../i18n/index.svelte';
import type { Sensor } from '../types';
import { CATEGORY_ORDER, categoryLabel, formatAverage, groupSensors, sourceCode } from './pages';

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
  expect(CATEGORY_ORDER).toEqual(['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'throughput', 'link', 'energy', 'flag']);
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
