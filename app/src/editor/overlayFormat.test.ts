import { DASH } from '../lib/format';
import { formatParts, joinParts, type OverlayFormat } from './overlayFormat';

// Cases from the tests of crates/oma-core/src/format.rs.
const opts = (o: Partial<OverlayFormat> = {}): OverlayFormat => ({
  decimalComma: false,
  temperature: 'c',
  rate: 'bits',
  flagOn: 'On',
  flagOff: 'Off',
  decimals: null,
  unit: 'auto',
  ...o,
});
const fmt = (v: number, unit: Parameters<typeof formatParts>[1], o: OverlayFormat) => joinParts(formatParts(v, unit, o));

test('fixed unit converts MB, GB, MHz and GHz', () => {
  const o = (unit: OverlayFormat['unit']) => opts({ unit });
  expect(formatParts(3_145_728, { unit: 'bytes' }, o('MB'))).toEqual(['3.0', 'MB']);
  expect(formatParts(3_145_728, { unit: 'bytes' }, o('GB'))).toEqual(['0.0', 'GB']);
  expect(formatParts(2_147_483_648, { unit: 'bytes' }, o('GB'))).toEqual(['2.0', 'GB']);
  expect(formatParts(2_000_000, { unit: 'bytes_per_second' }, o('MB'))).toEqual(['1.9', 'MB/s']);
  expect(formatParts(3_200, { unit: 'megahertz' }, o('GHz'))).toEqual(['3.20', 'GHz']);
  expect(formatParts(3_200, { unit: 'megahertz' }, o('MHz'))).toEqual(['3,200', 'MHz']);
  expect(formatParts(1.5, { unit: 'megahertz' }, o('GHz'))).toEqual(['0.00', 'GHz']);
  // Incompatible choices fall back to auto.
  expect(fmt(48, { unit: 'percent' }, o('GB'))).toBe('48%');
  expect(fmt(800, { unit: 'megahertz' }, o('MB'))).toBe('800 MHz');
});

test('decimals override', () => {
  const o = opts({ decimals: 2 });
  expect(fmt(48, { unit: 'percent' }, o)).toBe('48.00%');
  expect(fmt(1.2346, { unit: 'volt' }, o)).toBe('1.23 V');
  expect(fmt(1_536, { unit: 'bytes' }, o)).toBe('1.50 KB');
  expect(fmt(1.2346, { unit: 'volt' }, opts({ decimals: 0 }))).toBe('1 V');
});

test('decimal comma for Italian, rates and temperatures', () => {
  const o = opts({ decimalComma: true });
  expect(formatParts(12.5, { unit: 'ampere' }, o)[0]).toBe('12,5');
  expect(fmt(15_000, { unit: 'count' }, o)).toBe('15.000');
  expect(fmt(1_250_000, { unit: 'bytes_per_second' }, opts())).toBe('10 Mbit/s');
  expect(fmt(1_536, { unit: 'bytes_per_second' }, opts({ rate: 'bytes' }))).toBe('1.5 KB/s');
  expect(fmt(100, { unit: 'celsius' }, opts({ temperature: 'f' }))).toBe('212 °F');
});

test('absent values and flags', () => {
  expect(formatParts(null, { unit: 'celsius' }, opts())).toEqual([DASH, '']);
  expect(formatParts(NaN, { unit: 'percent' }, opts())[0]).toBe(DASH);
  const o = opts({ flagOn: 'Sì', flagOff: 'No' });
  expect(fmt(1, { unit: 'boolean' }, o)).toBe('Sì');
  expect(fmt(0, { unit: 'boolean' }, o)).toBe('No');
});

test('frame metric formats', () => {
  const it = opts({ decimalComma: true, decimals: 3 });
  expect(formatParts(143.6, { frames: 'fps-displayed' }, it)).toEqual(['144', 'FPS']);
  expect(formatParts(6.95, { frames: 'frametime-displayed' }, it)).toEqual(['7,0', 'ms']);
  expect(formatParts(1.99, { frames: 'fg-multiplier' }, it)).toEqual(['×2,0', '']);
  expect(formatParts(3, { frames: 'stutter' }, it)).toEqual(['3', '']);
  expect(formatParts(null, { frames: 'latency-pc' }, it)).toEqual([DASH, '']);
});
