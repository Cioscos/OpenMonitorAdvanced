import type { Translate } from '../i18n/index.svelte';
import type { DiskKind, DiskProfile } from '../types';
import { series } from './gauge';

// Pure helpers of the disk pages (M8c): the dial's scale, byte and latency formats and the estimate
// of what the benchmark writes.

export const GIB = 1024 ** 3;
const MIB = 1024 ** 2;

/** The gauge estimate, in MB/s, without a record, a reference or a needle (M8c DC14). */
const ESTIMATE: Record<DiskKind, number> = { nvme: 8000, sata_ssd: 600, hdd: 300, usb: 1000, virtual: 1000, other: 1000 };

/**
 * Full scale of a disk dial in MB/s: the first number of the 1-2-2.5-5 x 10^n series at or above
 * 1.1 x max(the kind's estimate, values), like the CPU dial's `fullScale` (spec §3.3).
 */
export function diskFullScale(values: number[], kind: DiskKind): number {
  return series(1.1 * Math.max(ESTIMATE[kind], ...values.filter(Number.isFinite)));
}

/** Gibibytes with one digit, or mebibytes below 1 GiB. */
export function formatBytes(bytes: number, locale = 'en'): string {
  if (bytes >= GIB) return `${(bytes / GIB).toLocaleString(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} GiB`;
  return `${Math.round(bytes / MIB).toLocaleString(locale)} MiB`;
}

/**
 * The most the disk benchmark writes: the file preparation (1 GiB) and the write caps of its four
 * tests (13 + 13 + 7 + 7 GiB), the same for both profiles (M8c DC5).
 */
export function benchWrites(_profile: DiskProfile): number {
  return 41 * GIB;
}

/** A latency in microseconds, in ms from 1000 µs; three significant digits at most. */
export function formatLatency(us: number | null | undefined, locale = 'en'): string {
  if (us == null || !Number.isFinite(us)) return '–';
  const fmt = (v: number, digits: number) => v.toLocaleString(locale, { maximumFractionDigits: digits });
  if (us >= 1000) {
    const ms = us / 1000;
    return `${fmt(ms, ms < 10 ? 2 : ms < 100 ? 1 : 0)} ms`;
  }
  return `${fmt(us, us < 10 ? 1 : 0)} µs`;
}

/** The free space a disk test needs on top of the Windows reserve (M8c DC6). */
export const MIN_FREE_BYTES = GIB;

/** The refusal of a disk command (`disk:<code>`) in words, or null for any other error. */
export function diskErrorText(error: unknown, t: Translate, locale = 'en'): string | null {
  const code = /^disk:(remote|not_writable|not_found|no_space|link)$/.exec(String(error))?.[1];
  return code ? t(`performance.disk.error.${code}`, { size: formatBytes(MIN_FREE_BYTES, locale) }) : null;
}
