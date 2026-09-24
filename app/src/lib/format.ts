import type { Locale, Translate } from './i18n/index.svelte';

export const DASH = '—';

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];
const BIT_UNITS = ['bit/s', 'kbit/s', 'Mbit/s', 'Gbit/s', 'Tbit/s'];
const formatters = new Map<string, Intl.NumberFormat>();

function num(value: number, digits: number, locale: Locale): string {
  const key = `${locale}:${digits}`;
  let formatter = formatters.get(key);
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits });
    formatters.set(key, formatter);
  }
  return formatter.format(value);
}

const missing = (v: number | null): v is null => v === null || !Number.isFinite(v);

export function formatPercent(value: number | null, locale: Locale): string {
  return missing(value) ? DASH : `${num(value, 0, locale)}%`;
}

/** Binary steps (1024) with the unit names Windows shows (KB, MB, GB). */
export function formatBytes(bytes: number | null, locale: Locale): string {
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
export function formatRate(bytesPerSecond: number | null, mode: 'bits' | 'bytes', locale: Locale): string {
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

export function formatClock(mhz: number | null, locale: Locale): string {
  if (missing(mhz)) return DASH;
  return mhz >= 1000 ? `${num(mhz / 1000, 2, locale)} GHz` : `${num(mhz, 0, locale)} MHz`;
}

export function formatDuration(ms: number, t: Translate): string {
  const minutes = Math.max(0, Math.floor(ms / 60_000));
  if (minutes < 60) return t('duration.minutes', { n: minutes });
  return t('duration.hoursMinutes', { h: Math.floor(minutes / 60), m: minutes % 60 });
}
