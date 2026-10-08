import { catalogs, type Locale } from '../i18n/index.svelte';
import performanceSection from '../../components/settings/PerformanceSection.svelte?raw';
import baseline from '../../../../crates/oma-core/src/scores/cpu-1-baseline.json';
import gpuBaseline from '../../../../crates/oma-core/src/scores/gpu-1-baseline.json';
import diskBaseline from '../../../../crates/oma-core/src/scores/disk-1-baseline.json';
import catalog from '../../../../testdata/performance/catalog.json';
import { BENCH_TERMS, DISK_BENCH_TERMS, DISK_MODE_TERMS, DISK_TERMS, GPU_BENCH_TERMS, GPU_SCORE_TERMS, GPU_TERMS, ISA_TERMS, MODE_TERMS, PATTERN_TERMS, SCORE_TERMS, TERMS } from './glossary';

const LOCALES: Locale[] = ['en', 'it'];

function missing(keys: string[]): string[] {
  return LOCALES.flatMap((locale) => keys.filter((key) => !catalogs[locale][key]?.trim()).map((key) => `${locale}:${key}`));
}

test('every_catalog_mode_isa_and_pattern_has_an_entry', () => {
  const terms = [...MODE_TERMS, ...DISK_MODE_TERMS, ...ISA_TERMS, ...PATTERN_TERMS];
  // testdata/performance/catalog.json: 9 CPU, 6 GPU and 9 disk kernels, 8 load modes, 3 instruction sets, 5 RAM patterns.
  // (The plan said 39; the catalog lists 9 disk kernels, disk_fill included, so 40.)
  expect(terms).toHaveLength(40);
  expect(terms).toHaveLength(catalog.kernels.length + catalog.gpuKernels.length + catalog.diskKernels.length + catalog.modes.length + catalog.isa.length + catalog.patterns.length);
  expect(missing(terms.flatMap((term) => [`glossary.${term}`, `glossary.${term}.name`]))).toEqual([]);
  expect(missing(TERMS.map((term) => `glossary.${term}`))).toEqual([]);
});

test('every_term_used_in_performance_pages_has_a_key', () => {
  const pages = import.meta.glob<string>('../../components/performance/**/*.svelte', { query: '?raw', import: 'default', eager: true });
  const sources = [...Object.values(pages), performanceSection];
  // `<Term term="…">`, and the `term="…"` of the settings controls that wrap their label in one.
  const used = new Set(sources.flatMap((source) => [...source.matchAll(/\sterm="([^"]+)"/g)].map((m) => m[1])));
  // The settings section alone marks Tjmax, the thermal stop and the RAM share.
  expect([...used]).toEqual(expect.arrayContaining(['tjmax', 'thermalStop', 'ramShare']));
  expect(missing([...used].map((term) => `glossary.${term}`))).toEqual([]);
  // A `<Term term="…" />` without children shows the term's name.
  const named = sources.flatMap((source) => [...source.matchAll(/<Term\s+term="([^"]+)"\s*\/>/g)].map((m) => m[1]));
  expect(named).toContain('tjmax');
  expect(missing(named.map((term) => `glossary.${term}.name`))).toEqual([]);
});

test('every_bench_kernel_has_an_entry', () => {
  // The six workloads of the score scale `oma-core` ships (DB3).
  expect([...BENCH_TERMS].sort()).toEqual(Object.keys(baseline.single).map((id) => `bench.${id}`).sort());
  expect(TERMS).toEqual(expect.arrayContaining(SCORE_TERMS));
  expect(missing([...BENCH_TERMS, ...SCORE_TERMS].flatMap((term) => [`glossary.${term}`, `glossary.${term}.name`]))).toEqual([]);
});

test('every_gpu_term_has_an_entry', () => {
  // Table T2 of the M8b1 plan.
  expect([...GPU_TERMS].sort()).toEqual(['artifact', 'deviceLost', 'loadLevel', 'pcieReplay', 'stability', 'tdr', 'vram']);
  expect(TERMS).toEqual(expect.arrayContaining(GPU_TERMS));
  expect(missing(GPU_TERMS.flatMap((term) => [`glossary.${term}`, `glossary.${term}.name`]))).toEqual([]);
});

test('every_gpu_bench_load_has_an_entry', () => {
  // The six loads of the GPU score scale `oma-core` ships (DH2), and the terms of table T2 of the M8b2 plan.
  const loads = [...Object.keys(gpuBaseline.compute), ...Object.keys(gpuBaseline.graphics)];
  expect([...GPU_BENCH_TERMS].sort()).toEqual(loads.map((id) => `gpuBench.${id}`).sort());
  expect([...GPU_SCORE_TERMS].sort()).toEqual(['computeScore', 'gbps', 'gpixels', 'gpuMedian', 'graphicsScore', 'spread', 'tflops']);
  expect(TERMS).toEqual(expect.arrayContaining(GPU_SCORE_TERMS));
  expect(missing([...GPU_BENCH_TERMS, ...GPU_SCORE_TERMS].flatMap((term) => [`glossary.${term}`, `glossary.${term}.name`]))).toEqual([]);
});

test('every_disk_test_and_term_has_an_entry', () => {
  // The tests of the default profile are in the score scale `oma-core` ships (DC5); the NVMe profile adds two.
  const scaled = Object.keys(diskBaseline.read).map((id) => `diskBench.${id}`);
  expect(DISK_BENCH_TERMS).toEqual(expect.arrayContaining(scaled));
  expect(DISK_BENCH_TERMS).toHaveLength(6);
  expect(TERMS).toEqual(expect.arrayContaining(DISK_TERMS));
  expect(missing([...DISK_BENCH_TERMS, ...DISK_TERMS].flatMap((term) => [`glossary.${term}`, `glossary.${term}.name`]))).toEqual([]);
});

test('every_disk_mode_has_an_entry', () => {
  // `disk_fill` is `mode.diskFill` in the catalogs; n1-n4 and v1-v4 keep their ids (table T4).
  expect([...DISK_MODE_TERMS].sort()).toEqual(['mode.diskFill', ...['n1', 'n2', 'n3', 'n4', 'v1', 'v2', 'v3', 'v4'].map((id) => `mode.${id}`)].sort());
  expect(missing(DISK_MODE_TERMS.flatMap((term) => [`glossary.${term}`, `glossary.${term}.name`]))).toEqual([]);
});

test('every_disk_bench_test_has_an_entry', () => {
  // The default profile's tests are the keys of the score scale; the NVMe profile adds two.
  const scaled = Object.keys(diskBaseline.read).map((id) => `diskBench.${id}`);
  expect(DISK_BENCH_TERMS).toEqual(expect.arrayContaining([...scaled, 'diskBench.seq128k_q32t1', 'diskBench.rnd4k_q32t16']));
  expect(missing(DISK_BENCH_TERMS.flatMap((term) => [`glossary.${term}`, `glossary.${term}.name`]))).toEqual([]);
});

test('every_disk_term_has_an_entry', () => {
  expect([...DISK_TERMS].sort()).toEqual(['compressible', 'dataError', 'dataUnitsWritten', 'diskPoints', 'iops', 'latency', 'mbs', 'queueDepth', 'seqRnd', 'slcCache']);
  expect(TERMS).toEqual(expect.arrayContaining(DISK_TERMS));
  expect(missing(DISK_TERMS.flatMap((term) => [`glossary.${term}`, `glossary.${term}.name`]))).toEqual([]);
});
