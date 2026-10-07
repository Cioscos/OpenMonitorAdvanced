import { catalogs, type Locale } from '../i18n/index.svelte';
import performanceSection from '../../components/settings/PerformanceSection.svelte?raw';
import baseline from '../../../../crates/oma-core/src/scores/cpu-1-baseline.json';
import { BENCH_TERMS, GPU_TERMS, ISA_TERMS, MODE_TERMS, PATTERN_TERMS, SCORE_TERMS, TERMS } from './glossary';

const LOCALES: Locale[] = ['en', 'it'];

function missing(keys: string[]): string[] {
  return LOCALES.flatMap((locale) => keys.filter((key) => !catalogs[locale][key]?.trim()).map((key) => `${locale}:${key}`));
}

test('every_catalog_mode_isa_and_pattern_has_an_entry', () => {
  const terms = [...MODE_TERMS, ...ISA_TERMS, ...PATTERN_TERMS];
  // testdata/performance/catalog.json: 9 CPU and 5 GPU kernels, 8 load modes, 3 instruction sets, 5 RAM patterns.
  expect(terms).toHaveLength(30);
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
