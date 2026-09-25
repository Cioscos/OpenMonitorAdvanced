import {
  DASH,
  formatBytes,
  formatClock,
  formatDuration,
  formatPercent,
  formatPower,
  formatRate,
  formatTemperature,
  formatValue,
} from './format';
import { translate } from './i18n/index.svelte';
import type { Unit } from './types';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
const tIt = (key: string, params?: Record<string, string | number>) => translate('it', key, params);

test('null values render as a dash', () => {
  expect(formatPercent(null, 'en')).toBe(DASH);
  expect(formatBytes(null, 'en')).toBe(DASH);
  expect(formatRate(null, 'bits', 'en')).toBe(DASH);
  expect(formatClock(null, 'en')).toBe(DASH);
  expect(formatTemperature(null, 'en')).toBe(DASH);
  expect(formatPower(Number.NaN, 'en')).toBe(DASH);
});

test('percent has no decimals', () => {
  expect(formatPercent(35.4, 'en')).toBe('35%');
  expect(formatPercent(99.6, 'it')).toBe('100%');
});

test('bytes use binary steps with Windows-style unit names', () => {
  expect(formatBytes(512, 'en')).toBe('512 B');
  expect(formatBytes(1536, 'en')).toBe('1.5 KB');
  expect(formatBytes(17.9 * 1024 ** 3, 'en')).toBe('17.9 GB');
  expect(formatBytes(17.9 * 1024 ** 3, 'it')).toBe('17,9 GB');
  expect(formatBytes(250 * 1024 ** 2, 'en')).toBe('250 MB');
});

test('rates in bits use decimal steps', () => {
  expect(formatRate(6_000_000, 'bits', 'en')).toBe('48 Mbit/s');
  expect(formatRate(125_000, 'bits', 'it')).toBe('1,0 Mbit/s');
  expect(formatRate(0, 'bits', 'en')).toBe('0.0 bit/s');
});

test('rates in bytes reuse byte units', () => {
  expect(formatRate(120 * 1024 ** 2, 'bytes', 'en')).toBe('120 MB/s');
});

test('clock switches to GHz from 1000 MHz', () => {
  expect(formatClock(4383.7, 'en')).toBe('4.38 GHz');
  expect(formatClock(4383.7, 'it')).toBe('4,38 GHz');
  expect(formatClock(800, 'en')).toBe('800 MHz');
});

test('temperature and power have no decimals', () => {
  expect(formatTemperature(54.4, 'en')).toBe('54 °C');
  expect(formatTemperature(99.6, 'it')).toBe('100 °C');
  expect(formatPower(147.8, 'en')).toBe('148 W');
  expect(formatPower(1234, 'en')).toBe('1,234 W');
});

test('durations', () => {
  expect(formatDuration(5 * 60_000, tEn)).toBe('5 min');
  expect(formatDuration(125 * 60_000, tEn)).toBe('2 h 5 min');
  expect(formatDuration(-1, tEn)).toBe('0 min');
});

// `Record<Unit, …>` makes this table fail to compile when a Unit is added without a case.
const EXAMPLES: Record<Unit, [number, string]> = {
  celsius: [54.4, '54 °C'],
  percent: [35.4, '35%'],
  megahertz: [2520, '2.52 GHz'],
  watt: [147.8, '148 W'],
  volt: [1.075, '1.075 V'],
  ampere: [12.34, '12.3 A'],
  rpm: [1650, '1,650 RPM'],
  bytes: [17.9 * 1024 ** 3, '17.9 GB'],
  bytes_per_second: [120 * 1024 ** 2, '120 MB/s'],
  bits_per_second: [1e9, '1.0 Gbit/s'],
  joule: [12_345, '12.3 kJ'],
  boolean: [1, 'Active'],
  pcie_generation: [4, 'Gen 4'],
  lanes: [16, 'x16'],
};

test('formatValue formats every unit', () => {
  for (const [unit, [value, expected]] of Object.entries(EXAMPLES) as [Unit, [number, string]][]) {
    expect(formatValue(value, unit, 'en', tEn), unit).toBe(expected);
  }
});

test('formatValue renders missing values as a dash for every unit', () => {
  for (const unit of Object.keys(EXAMPLES) as Unit[]) {
    expect(formatValue(null, unit, 'en', tEn), unit).toBe(DASH);
    expect(formatValue(Number.NaN, unit, 'en', tEn), unit).toBe(DASH);
  }
});

test('formatValue follows the locale', () => {
  expect(formatValue(1.075, 'volt', 'it', tIt)).toBe('1,075 V');
  expect(formatValue(12_000, 'rpm', 'it', tIt)).toBe('12.000 RPM');
  expect(formatValue(0, 'boolean', 'it', tIt)).toBe('No');
  expect(formatValue(1, 'boolean', 'it', tIt)).toBe('Attivo');
  expect(formatValue(0, 'boolean', 'en', tEn)).toBe('No');
});

test('formatValue rounds link values and scales energy', () => {
  expect(formatValue(3.9999, 'pcie_generation', 'en', tEn)).toBe('Gen 4');
  expect(formatValue(8, 'lanes', 'en', tEn)).toBe('x8');
  expect(formatValue(950, 'joule', 'en', tEn)).toBe('950 J');
  expect(formatValue(2.5e6, 'joule', 'en', tEn)).toBe('2.5 MJ');
});

test('formatValue shows byte rates in bits on request, like the Simple view network tile', () => {
  expect(formatValue(6_000_000, 'bytes_per_second', 'en', tEn, { rate: 'bits' })).toBe('48 Mbit/s');
  expect(formatValue(6_000_000, 'bytes_per_second', 'en', tEn, { rate: 'bytes' })).toBe('5.7 MB/s');
  expect(formatValue(6_000_000, 'bytes_per_second', 'en', tEn)).toBe('5.7 MB/s');
  // Units other than bytes_per_second ignore the option.
  expect(formatValue(1e9, 'bits_per_second', 'en', tEn, { rate: 'bytes' })).toBe('1.0 Gbit/s');
});
