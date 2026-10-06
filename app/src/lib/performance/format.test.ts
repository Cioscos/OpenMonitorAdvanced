import { i18n, t } from '../i18n/index.svelte';
import type { Phase } from '../types';
import { formatDuration, phaseLabel } from './format';

const phase = (over: Partial<Phase>): Phase => ({
  kernel: 'k2',
  alt_kernel: null,
  isa: 'avx2',
  size: 'l2',
  mode: 'steady',
  placement: 'all_logical',
  duration_s: 60,
  per_core_s: null,
  both_smt: false,
  cores: null,
  patterns: [],
  stop_on_error: false,
  ...over,
});

test('format_duration_cases', () => {
  expect(formatDuration(300)).toBe('5 min');
  expect(formatDuration(5400)).toBe('1 h 30 min');
  expect(formatDuration(28_800)).toBe('8 h');
  expect(formatDuration(90)).toBe('1 min 30 s');
  expect(formatDuration(45)).toBe('45 s');
  expect(formatDuration(0)).toBe('0 s');
  // Past an hour the seconds go: the plans move in minutes there.
  expect(formatDuration(3690)).toBe('1 h 1 min');
});

test('phase_label_names_the_kernel_size_and_set', () => {
  i18n.locale = 'it';
  // The kernel's own size is part of its name.
  expect(phaseLabel(phase({}), t)).toBe('FFT piccole · core e cache · AVX2');
  expect(phaseLabel(phase({ kernel: 'k5', size: 'l3' }), t)).toBe('Interi esatti (NTT) · L3 · AVX2');
  expect(phaseLabel(phase({ kernel: 'k8', size: 'auto', isa: 'sse2' }), t)).toBe('Crittografia e compressione · SSE2');
  i18n.locale = 'en';
});
