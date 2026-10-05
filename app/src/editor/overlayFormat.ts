// Number and unit of a value as the overlay draws them: a port of `format_value`,
// `format_frame_metric` and `number` in crates/oma-core/src/format.rs (with `format_options` of
// crates/oma-overlay/src/render/layout.rs). Keep it in step with that file.

import { DASH } from '../lib/format';
import type { FrameMetric, UnitChoice } from '../lib/editor/profile';
import type { Unit } from '../lib/types';

export interface OverlayFormat {
  /** Italian-style numbers: `,` as decimal separator, `.` for thousands. */
  decimalComma: boolean;
  temperature: 'c' | 'f';
  rate: 'bits' | 'bytes';
  flagOn: string;
  flagOff: string;
  /** Fixed decimals of sensor values; frame metrics keep their own. */
  decimals: number | null;
  /** Fixed display unit; one that does not fit the sensor's unit is `auto`. */
  unit: UnitChoice;
}

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];
const BIT_UNITS = ['bit/s', 'kbit/s', 'Mbit/s', 'Gbit/s', 'Tbit/s'];
const JOULE_UNITS = ['J', 'kJ', 'MJ', 'GJ'];
const FIXED_BYTES: Partial<Record<UnitChoice, number>> = { B: 0, KB: 1, MB: 2, GB: 3, TB: 4 };
const FIXED_BITS: Partial<Record<UnitChoice, number>> = { 'bit/s': 0, 'kbit/s': 1, 'Mbit/s': 2, 'Gbit/s': 3 };

/**
 * `Intl.NumberFormat` for `en` and `it` as the overlay writes it: `digits` decimals rounded half
 * away from zero, thousands grouped from four digits in English, five in Italian; no sign on a
 * value rounded to zero.
 */
export function overlayNumber(value: number, digits: number, decimalComma: boolean): string {
  const scale = 10 ** digits;
  const rounded = (Math.sign(value) * Math.round(Math.abs(value) * scale)) / scale;
  const [int, frac = ''] = Math.abs(rounded).toFixed(digits).split('.');
  const [group, decimal, from] = decimalComma ? ['.', ',', 5] : [',', '.', 4];
  let out = rounded < 0 ? '-' : '';
  for (let i = 0; i < int.length; i++) {
    if (int.length >= from && i > 0 && (int.length - i) % 3 === 0) out += group;
    out += int[i];
  }
  return frac === '' ? out : `${out}${decimal}${frac}`;
}

function scaled(value: number, step: number, units: readonly string[]): [number, number] {
  let unit = 0;
  while (Math.abs(value) >= step && unit < units.length - 1) {
    value /= step;
    unit++;
  }
  return [value, unit];
}

/** A sensor value, or a frame metric when `of` is `{ frames }`, as `[number, unit]`. */
export function formatParts(value: number | null, of: { unit: Unit } | { frames: FrameMetric }, o: OverlayFormat): [string, string] {
  if (value === null || !Number.isFinite(value)) return [DASH, ''];
  const v = value;
  const comma = o.decimalComma;
  if ('frames' in of) {
    switch (of.frames) {
      case 'fps-displayed':
      case 'fps-rendered':
      case 'fps-presented':
      case 'low-1':
      case 'low-01':
        return [overlayNumber(v, 0, comma), 'FPS'];
      case 'frametime-displayed':
      case 'frametime-app':
      case 'latency-pc':
      case 'latency-display':
        return [overlayNumber(v, 1, comma), 'ms'];
      case 'fg-multiplier':
        return [`×${overlayNumber(v, 1, comma)}`, ''];
      case 'stutter':
        return [overlayNumber(v, 0, comma), ''];
      case 'bound':
        return [DASH, ''];
    }
  }
  const digits = (fallback: number) => o.decimals ?? fallback;
  const n = (x: number, fallback: number) => overlayNumber(x, digits(fallback), comma);
  const stepped = (x: number, step: number, units: readonly string[], suffix: string): [string, string] => {
    const [s, unit] = scaled(x, step, units);
    return [n(s, unit !== 0 && s < 100 ? 1 : 0), `${units[unit]}${suffix}`];
  };
  const bytesFixed = (x: number, index: number, suffix: string): [string, string] => {
    const s = x / 1024 ** index;
    return [n(s, index !== 0 && Math.abs(s) < 100 ? 1 : 0), `${BYTE_UNITS[index]}${suffix}`];
  };
  const throughput = (bytesPerSecond: number, rate: 'bits' | 'bytes'): [string, string] => {
    const fixedBytes = FIXED_BYTES[o.unit];
    if (fixedBytes !== undefined) return bytesFixed(bytesPerSecond, fixedBytes, '/s');
    const fixedBits = FIXED_BITS[o.unit];
    if (fixedBits === undefined && rate === 'bytes') return stepped(bytesPerSecond, 1024, BYTE_UNITS, '/s');
    const bits = bytesPerSecond * 8;
    const [s, unit] = fixedBits === undefined ? scaled(bits, 1000, BIT_UNITS) : [bits / 1000 ** fixedBits, fixedBits];
    return [n(s, Math.abs(s) < 10 ? 1 : 0), BIT_UNITS[unit]];
  };
  switch (of.unit) {
    case 'celsius':
      return o.temperature === 'f' ? [n((v * 9) / 5 + 32, 0), '°F'] : [n(v, 0), '°C'];
    case 'percent':
      return [n(v, 0), '%'];
    case 'megahertz':
      if (o.unit === 'GHz' || (o.unit !== 'MHz' && v >= 1000)) return [n(v / 1000, 2), 'GHz'];
      return [n(v, 0), 'MHz'];
    case 'watt':
      return [n(v, 0), 'W'];
    case 'volt':
      return [n(v, 3), 'V'];
    case 'ampere':
      return [n(v, 1), 'A'];
    case 'rpm':
      return [n(v, 0), 'RPM'];
    case 'bytes': {
      const fixed = FIXED_BYTES[o.unit];
      return fixed === undefined ? stepped(v, 1024, BYTE_UNITS, '') : bytesFixed(v, fixed, '');
    }
    case 'bytes_per_second':
      return throughput(v, o.rate);
    case 'bits_per_second':
      return throughput(v / 8, 'bits');
    case 'joule':
      return stepped(v, 1000, JOULE_UNITS, '');
    case 'boolean':
      return [v >= 0.5 ? o.flagOn : o.flagOff, ''];
    case 'pcie_generation':
      return [`Gen ${Math.floor(v + 0.5)}`, ''];
    case 'lanes':
      return [`x${Math.floor(v + 0.5)}`, ''];
    case 'hours':
      return [n(v, 0), 'h'];
    case 'count':
      return [n(v, 0), ''];
    default:
      // An unknown unit is shown as a plain number, as in the overlay.
      return [n(v, 0), ''];
  }
}

/** Number and unit as one text: no space before `%`, none without a unit (`join`). */
export const joinParts = ([number, unit]: [string, string]): string => (unit === '' ? number : unit === '%' ? `${number}%` : `${number} ${unit}`);
