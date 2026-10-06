import catalog from '../../../../testdata/performance/catalog.json';

// The glossary of the Performance view (spec M8 §3.7): every term is `glossary.<term>` in the
// catalogs, with `glossary.<term>.name` as its name. The catalog of kernels, modes, instruction
// sets and RAM patterns is the one `oma-core` writes (DA19), so a new entry there fails the
// glossary test until it has its texts.

/** The kernels and the load modes: `mode.k1`, `mode.steady`, `mode.coreCycle`… */
export const MODE_TERMS: string[] = [...catalog.kernels, ...catalog.modes].map((id) => `mode.${id}`);
export const ISA_TERMS: string[] = catalog.isa.map((id) => `isa.${id}`);
export const PATTERN_TERMS: string[] = catalog.patterns.map((id) => `mode.pattern.${id}`);

/** The technical terms of the pages (table T2 of the plan). */
export const TERMS: string[] = [
  'fft',
  'ntt',
  'linpack',
  'fma',
  'smt',
  'ccd',
  'coreType',
  'coreNumber',
  'apicId',
  'tjmax',
  'thermalStop',
  'throttling',
  'whea',
  'curveOptimizer',
  'pbo',
  'expoXmp',
  'vrm',
  'cState',
  'boost',
  'imc',
  'cache',
  'check',
  'reference',
  'seed',
  'iteration',
  'ramShare',
];
