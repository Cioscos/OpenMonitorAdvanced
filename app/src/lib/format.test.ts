import { DASH, formatBytes, formatClock, formatDuration, formatPercent, formatRate } from './format';
import { translate } from './i18n/index.svelte';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);

test('null values render as a dash', () => {
  expect(formatPercent(null, 'en')).toBe(DASH);
  expect(formatBytes(null, 'en')).toBe(DASH);
  expect(formatRate(null, 'bits', 'en')).toBe(DASH);
  expect(formatClock(null, 'en')).toBe(DASH);
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

test('durations', () => {
  expect(formatDuration(5 * 60_000, tEn)).toBe('5 min');
  expect(formatDuration(125 * 60_000, tEn)).toBe('2 h 5 min');
  expect(formatDuration(-1, tEn)).toBe('0 min');
});
