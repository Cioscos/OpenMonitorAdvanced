import type { Translate } from '../i18n/index.svelte';
import type { DiskKind, DiskProfile, Plan, StartRequest, StressSession, VolumeChoice } from '../types';
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

/**
 * The most a disk plan writes (M8c DC7), the same formula as `oma-core::load::plan::estimated_writes`:
 * the file for the preparation, V1's file times its cycles, V2's rate times its time, and the write
 * cap of every other phase that writes.
 */
export function estimatedWrites(plan: Plan): number {
  const file = plan.disk?.file_bytes ?? 0;
  return plan.phases.reduce((sum, p) => {
    const job = p.disk;
    if (!job) return sum;
    switch (p.kernel) {
      case 'disk_fill':
        return sum + file;
      case 'v1':
        return sum + file * (job.cycles ?? 1);
      case 'v2':
        return sum + (job.rate_limit_bps ?? 0) * p.duration_s;
      case 'n1':
      case 'n2':
      case 'n4':
      case 'v3':
      case 'v4':
        return sum + (job.write_cap_bytes ?? 0);
      default:
        return sum;
    }
  }, 0);
}

/** A speed in B/s as «3,200 MB/s» (10^6 bytes, like the dials). */
export function formatMbs(bytesPerSecond: number | null | undefined, locale = 'en'): string {
  if (bytesPerSecond == null || !Number.isFinite(bytesPerSecond)) return '–';
  return `${Math.round(bytesPerSecond / 1e6).toLocaleString(locale)} MB/s`;
}

/**
 * The request that repeats a saved disk test. The session keeps no folder (it may hold the user's
 * name), so it is found again from the volume: null when that volume is gone. Other components repeat as saved.
 */
export function repeatRequest(session: StressSession, volumes: VolumeChoice[]): StartRequest | null {
  if (session.component !== 'disk') return session.request;
  const letter = session.disk?.volume.toLowerCase();
  const volume = volumes.find((v) => v.root.slice(0, 2).toLowerCase() === letter);
  return volume ? { ...session.request, disk: { folder: volume.folder, wake: false } } : null;
}
