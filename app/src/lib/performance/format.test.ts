import { i18n, t } from '../i18n/index.svelte';
import type { Phase } from '../types';
import { MOCK_SCHEMA, SERVICE_MOCK_SCHEMA } from '../backend/mock';
import { cpuChartSensors, errorText, eventText, formatDuration, marked, phaseLabel } from './format';

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

test('marked_cuts_around_the_bracketed_term', () => {
  expect(marked('Un [thread] per core')).toEqual(['Un ', 'thread', ' per core']);
  expect(marked('no term')).toEqual(['no term', '', '']);
});

test('shell_errors_are_translated_by_code', () => {
  expect(errorText('build:too_long', t)).toBe(t('performance.wizard.error.too_long'));
  expect(errorText('build:ram_budget', t)).toBe(t('performance.wizard.error.ram_budget'));
  expect(errorText('busy', t)).toBe(t('performance.wizard.busy'));
  // An unknown code and any other text stay as the shell wrote them.
  expect(errorText('build:something_new', t)).toBe('build:something_new');
  expect(errorText('oma-load.exe not found', t)).toBe('oma-load.exe not found');
});

test('event_text_counts_phases_from_one_and_reads_recovered_whea', () => {
  i18n.locale = 'it';
  const ev = (code: string, params: Record<string, string> = {}) => eventText({ atMs: 0, code, params }, t, 'it');
  expect(ev('ram_insufficient', { phase: '0' })).toEqual({ text: t('performance.event.ram_insufficient', { phase: 1 }), term: null });
  expect(ev('ram_reduced', { phase: '2', value: String(4 * 1024 ** 3) }).text).toBe(t('performance.event.ram_reduced', { phase: 3, value: '4,0 GB' }));
  expect(ev('whea', { id: '19', apic: '4', core: '2' })).toEqual({ text: 'Errore WHEA 19 sul core 2', term: 'whea' });
  expect(ev('whea18', { record: '7', apic: '6' }).text).toBe("Errore WHEA 18 dall'APIC ID 6");
  expect(ev('mystery')).toEqual({ text: 'mystery', term: null });
  i18n.locale = 'en';
});

test('chart_sensors_follow_da5', () => {
  expect(cpuChartSensors(MOCK_SCHEMA)).toEqual([]);
  expect(cpuChartSensors(SERVICE_MOCK_SCHEMA).map((s) => s.id)).toEqual(['cpu/0/temperature/package', 'cpu/0/power/package']);
  const tdie = { ...SERVICE_MOCK_SCHEMA.sensors[0], id: 'cpu/0/temperature/tdie' };
  expect(cpuChartSensors({ ...SERVICE_MOCK_SCHEMA, sensors: [...SERVICE_MOCK_SCHEMA.sensors, tdie] })[0].id).toBe('cpu/0/temperature/tdie');
  expect(cpuChartSensors(null)).toEqual([]);
});
