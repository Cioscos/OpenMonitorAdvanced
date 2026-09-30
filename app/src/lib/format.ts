import type { Translate } from './i18n/index.svelte';
import type { TemperatureUnit, ThroughputUnit, Unit } from './types';
import { display, temperatureSymbol, toDisplayTemperature } from './units.svelte';

export const DASH = '—';

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];
const BIT_UNITS = ['bit/s', 'kbit/s', 'Mbit/s', 'Gbit/s', 'Tbit/s'];
const JOULE_UNITS = ['J', 'kJ', 'MJ', 'GJ'];
const formatters = new Map<string, Intl.NumberFormat>();

// `locale` is a BCP 47 tag ('en', 'it'); Intl accepts any tag, so callers may pass a plain string.
function num(value: number, digits: number, locale: string): string {
  const key = `${locale}:${digits}`;
  let formatter = formatters.get(key);
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale, {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
      useGrouping: true,
    });
    formatters.set(key, formatter);
  }
  return formatter.format(value);
}

const missing = (v: number | null): v is null => v === null || !Number.isFinite(v);

export function formatPercent(value: number | null, locale: string): string {
  return missing(value) ? DASH : `${num(value, 0, locale)}%`;
}

/** Binary steps (1024) with the unit names Windows shows (KB, MB, GB). */
export function formatBytes(bytes: number | null, locale: string): string {
  if (missing(bytes)) return DASH;
  let value = bytes;
  let unit = 0;
  while (Math.abs(value) >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${num(value, digits, locale)} ${BYTE_UNITS[unit]}`;
}

/** Network-style rates: bits with decimal steps, or bytes with binary steps. */
export function formatRate(bytesPerSecond: number | null, mode: ThroughputUnit, locale: string): string {
  if (missing(bytesPerSecond)) return DASH;
  if (mode === 'bytes') return `${formatBytes(bytesPerSecond, locale)}/s`;
  let value = bytesPerSecond * 8;
  let unit = 0;
  while (Math.abs(value) >= 1000 && unit < BIT_UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${num(value, value < 10 ? 1 : 0, locale)} ${BIT_UNITS[unit]}`;
}

export function formatClock(mhz: number | null, locale: string): string {
  if (missing(mhz)) return DASH;
  return mhz >= 1000 ? `${num(mhz / 1000, 2, locale)} GHz` : `${num(mhz, 0, locale)} MHz`;
}

/** A temperature in °C, shown in `unit` (the temperature setting by default). */
export function formatTemperature(celsius: number | null, locale: string, unit: TemperatureUnit = display.temperature): string {
  return missing(celsius) ? DASH : formatTemperatureIn(toDisplayTemperature(celsius, unit), unit, locale);
}

/** A number of degrees already in `unit`, for chart axes and legends that plot converted data. */
export function formatTemperatureIn(degrees: number | null, unit: TemperatureUnit, locale: string): string {
  return missing(degrees) ? DASH : `${num(degrees, 0, locale)} ${temperatureSymbol(unit)}`;
}

export function formatPower(watt: number | null, locale: string): string {
  return missing(watt) ? DASH : `${num(watt, 0, locale)} W`;
}

/** Energy with decimal steps (J, kJ, MJ, GJ). */
function formatEnergy(joule: number, locale: string): string {
  let value = joule;
  let unit = 0;
  while (Math.abs(value) >= 1000 && unit < JOULE_UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${num(value, digits, locale)} ${JOULE_UNITS[unit]}`;
}

/** Options of `formatValue`. */
export interface FormatOptions {
  /**
   * How `bytes_per_second` is shown: 'bytes' (default: disks) or 'bits' (network pages,
   * the same unit as the Simple view's network tile).
   */
  rate?: 'bits' | 'bytes';
}

/** Any sensor value with its unit (Advanced view tables and KPIs). */
export function formatValue(
  value: number | null,
  unit: Unit,
  locale: string,
  t: Translate,
  opts: FormatOptions = {},
): string {
  if (missing(value)) return DASH;
  switch (unit) {
    case 'celsius':
      return formatTemperature(value, locale);
    case 'percent':
      return formatPercent(value, locale);
    case 'megahertz':
      return formatClock(value, locale);
    case 'watt':
      return formatPower(value, locale);
    case 'volt':
      return `${num(value, 3, locale)} V`;
    case 'ampere':
      return `${num(value, 1, locale)} A`;
    case 'rpm':
      return `${num(value, 0, locale)} RPM`;
    case 'bytes':
      return formatBytes(value, locale);
    case 'bytes_per_second':
      return formatRate(value, opts.rate ?? 'bytes', locale);
    case 'bits_per_second':
      return formatRate(value / 8, 'bits', locale);
    case 'joule':
      return formatEnergy(value, locale);
    case 'boolean':
      return t(value >= 0.5 ? 'flag.on' : 'flag.off');
    case 'pcie_generation':
      return `Gen ${Math.round(value)}`;
    case 'lanes':
      return `x${Math.round(value)}`;
    case 'hours':
      return `${num(value, 0, locale)} h`;
    case 'count':
      return num(value, 0, locale);
    default: {
      const unknown: never = unit;
      return `${num(value, 1, locale)} ${String(unknown)}`;
    }
  }
}

export function formatDuration(ms: number, t: Translate): string {
  const minutes = Math.max(0, Math.floor(ms / 60_000));
  if (minutes < 60) return t('duration.minutes', { n: minutes });
  return t('duration.hoursMinutes', { h: Math.floor(minutes / 60), m: minutes % 60 });
}

/** Tape-recorder counter of the CSV log: "00:12:47"; the hours keep growing past 99. */
export function formatTapeCounter(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(Math.floor(total / 3600))}:${pad(Math.floor(total / 60) % 60)}:${pad(total % 60)}`;
}
