import fixture from '../../../../testdata/performance/disk-writes.json';
import type { Plan } from '../types';
import { benchWrites, estimatedWrites, diskFullScale, formatBytes, formatLatency, GIB } from './disk';

test('disk_full_scale_from_the_kind', () => {
  expect(diskFullScale([], 'nvme')).toBe(10000);
  expect(diskFullScale([], 'sata_ssd')).toBe(1000);
  expect(diskFullScale([], 'hdd')).toBe(500);
  expect(diskFullScale([], 'usb')).toBe(2000);
  // The others (virtual, other) start from 1000 MB/s: 1.1 x 1000 up to 2000.
  expect(diskFullScale([], 'virtual')).toBe(2000);
  expect(diskFullScale([], 'other')).toBe(2000);
});

test('disk_full_scale_follows_the_values', () => {
  expect(diskFullScale([9500], 'nvme')).toBe(20000);
  expect(diskFullScale([450], 'hdd')).toBe(500);
  // Values that are not numbers, or zero, do not count.
  expect(diskFullScale([NaN, 0], 'nvme')).toBe(10000);
  // A low value never shrinks the dial below the estimate of the kind.
  expect(diskFullScale([100], 'nvme')).toBe(10000);
});

test('format_bytes', () => {
  expect(formatBytes(41 * GIB)).toBe('41.0 GiB');
  expect(formatBytes(1.25 * GIB)).toBe('1.3 GiB');
  expect(formatBytes(GIB)).toBe('1.0 GiB');
  expect(formatBytes(512 * 1024 ** 2)).toBe('512 MiB');
  expect(formatBytes(0)).toBe('0 MiB');
});

test('bench_writes_are_41_gib_for_both_profiles', () => {
  expect(benchWrites('b1')).toBe(41 * GIB);
  expect(benchWrites('b2')).toBe(41 * GIB);
});

test('format_latency', () => {
  expect(formatLatency(null)).toBe('–');
  expect(formatLatency(53.4)).toBe('53 µs');
  expect(formatLatency(8.2)).toBe('8.2 µs');
  expect(formatLatency(1210)).toBe('1.21 ms');
  expect(formatLatency(25000)).toBe('25 ms');
});

test('estimated_writes_matches_the_rust_formula', () => {
  expect(fixture.cases.length).toBeGreaterThan(3);
  for (const c of fixture.cases) {
    const plan: Plan = {
      seed: 1,
      ram_bytes: 0,
      disk: { dir: 'C:\t', file_bytes: c.fileBytes, compressible: false, reserve_bytes: GIB },
      phases: c.phases.map((p) => ({
        kernel: p.kernel,
        duration_s: p.durationS,
        disk: { write_cap_bytes: p.writeCapBytes, cycles: p.cycles, rate_limit_bps: p.rateLimitBps },
      })) as unknown as Plan['phases'],
    };
    expect(estimatedWrites(plan), c.label).toBe(c.expectedBytes);
  }
});
