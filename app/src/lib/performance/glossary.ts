import catalog from '../../../../testdata/performance/catalog.json';

// The glossary of the Performance view (spec M8 §3.7): every term is `glossary.<term>` in the
// catalogs, with `glossary.<term>.name` as its name. The catalog of kernels, modes, instruction
// sets and RAM patterns is the one `oma-core` writes (DA19), so a new entry there fails the
// glossary test until it has its texts.

/** The CPU and GPU kernels and the load modes: `mode.k1`, `mode.s5`, `mode.steady`, `mode.ramp`… */
export const MODE_TERMS: string[] = [...catalog.kernels, ...catalog.gpuKernels, ...catalog.modes].map((id) => `mode.${id}`);
export const ISA_TERMS: string[] = catalog.isa.map((id) => `isa.${id}`);
export const PATTERN_TERMS: string[] = catalog.patterns.map((id) => `mode.pattern.${id}`);

/** The six workloads of the CPU benchmark (M8a2 DB3): `bench.ntt`, `bench.gemm`… */
export const BENCH_TERMS: string[] = ['ntt', 'hash', 'compress', 'sort', 'fft', 'gemm'].map((id) => `bench.${id}`);

/** The terms of the CPU score page (table T2 of the M8a2 plan). */
export const SCORE_TERMS: string[] = ['benchPoints', 'singleCore', 'multiCore', 'scaling', 'referenceMark', 'warmup', 'median'];

/** The six loads of the GPU benchmark (M8b2 DH2): `gpuBench.fma`, `gpuBench.fill`… */
export const GPU_BENCH_TERMS: string[] = ['fma', 'int_hash', 'bandwidth', 'fill', 'texture', 'overdraw'].map((id) => `gpuBench.${id}`);

/** The terms of the GPU score pages (table T2 of the M8b2 plan). */
export const GPU_SCORE_TERMS: string[] = ['computeScore', 'graphicsScore', 'spread', 'tflops', 'gbps'];

/** The terms of the GPU stress pages (table T2 of the M8b1 plan). */
export const GPU_TERMS: string[] = ['tdr', 'vram', 'deviceLost', 'stability', 'pcieReplay', 'artifact', 'loadLevel'];

/** The technical terms of the pages (table T2 of the plan). */
export const TERMS: string[] = [
  ...SCORE_TERMS,
  ...GPU_SCORE_TERMS,
  ...GPU_TERMS,
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
