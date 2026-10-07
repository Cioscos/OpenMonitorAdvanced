import { i18n, t } from '../i18n/index.svelte';
import type { Phase } from '../types';
import { MOCK_SCHEMA, SERVICE_MOCK_SCHEMA } from '../backend/mock';
import { cpuChartSensors, errorText, gpuChartSensors, eventText, formatDuration, marked, phaseLabel, verdictTitle } from './format';

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
  const text = (code: string, params: Record<string, string> = {}) => ev(code, params).map((p) => p.text).join('');
  const terms = (code: string, params: Record<string, string> = {}) => ev(code, params).filter((p) => p.term).map((p) => [p.term, p.text]);
  expect(text('ram_insufficient', { phase: '0' })).toBe(t('performance.event.ram_insufficient', { phase: 1 }));
  expect(terms('ram_insufficient', { phase: '0' })).toEqual([['phase', 'Fase']]);
  expect(text('ram_reduced', { phase: '2', value: String(4 * 1024 ** 3) })).toBe('Fase 3: memoria per thread ridotta a 4,0 GB');
  expect(terms('ram_reduced', { phase: '2', value: '1' })).toEqual([['phase', 'Fase'], ['threads', 'thread']]);
  expect(text('whea', { id: '19', apic: '4', core: '2' })).toBe('Errore WHEA 19 sul core 2 (APIC ID 4)');
  expect(terms('whea', { id: '19', apic: '4', core: '2' })).toEqual([['whea', 'WHEA'], ['coreNumber', 'core 2'], ['apicId', 'APIC ID']]);
  expect(text('whea18', { record: '7', apic: '6' })).toBe(t('performance.event.whea', { id: 18, where: t('performance.event.where.apic', { apic: 6 }) }));
  expect(terms('bugcheck')).toEqual([['bugcheck', 'BugCheck']]);
  expect(terms('kernelPower41')).toEqual([['kernelPower41', 'Kernel-Power 41']]);
  expect(ev('mystery')).toEqual([{ text: 'mystery', term: null }]);
  i18n.locale = 'en';
});

test('verdict_title_translates_a_reason_key', () => {
  expect(verdictTitle({ verdict: 'failed_to_start', params: { reason: 'performance.start.nothing_ran' } }, t)).toBe(
    t('performance.outcome.failed_to_start', { reason: t('performance.start.nothing_ran') }),
  );
  expect(verdictTitle({ verdict: 'failed_to_start', params: { reason: 'start failed' } }, t)).toBe(
    t('performance.outcome.failed_to_start', { reason: 'start failed' }),
  );
});

test('verdict_title_reads_the_recovered_phase', () => {
  expect(verdictTitle({ verdict: 'system_crash', params: { phase: '3' } }, t)).toBe(t('performance.outcome.system_crash', { phase: t('performance.result.phaseN', { n: 3 }) }));
  expect(verdictTitle({ verdict: 'errors_core', params: { core: '2' } }, t)).toBe(t('performance.outcome.errors_core', { core: 2 }));
  expect(verdictTitle(null, t)).toBe(t('performance.result.unknown'));
});

test('chart_sensors_follow_da5', () => {
  expect(cpuChartSensors(MOCK_SCHEMA)).toEqual([]);
  expect(cpuChartSensors(SERVICE_MOCK_SCHEMA).map((s) => s.id)).toEqual(['cpu/0/temperature/package', 'cpu/0/power/package']);
  const tdie = { ...SERVICE_MOCK_SCHEMA.sensors[0], id: 'cpu/0/temperature/tdie' };
  expect(cpuChartSensors({ ...SERVICE_MOCK_SCHEMA, sensors: [...SERVICE_MOCK_SCHEMA.sensors, tdie] })[0].id).toBe('cpu/0/temperature/tdie');
  expect(cpuChartSensors(null)).toEqual([]);
});

test('gpu chart sensors come from the status device, not from a guess', () => {
  const first = MOCK_SCHEMA.sensors.filter((x) => x.deviceId.startsWith('gpu/'));
  const other = first.map((x) => ({ ...x, id: x.id.replace('gpu/pci-0000:01:00.0', 'gpu/pci-0000:0c:00.0'), deviceId: 'gpu/pci-0000:0c:00.0' }));
  const schema = { ...MOCK_SCHEMA, sensors: [...first, ...other] };
  expect(gpuChartSensors(schema, 'gpu/pci-0000:0c:00.0').map((x) => x.id)).toEqual(['gpu/pci-0000:0c:00.0/temperature/core', 'gpu/pci-0000:0c:00.0/power/board']);
  expect(gpuChartSensors(schema, 'gpu/pci-0000:01:00.0')[0].deviceId).toBe('gpu/pci-0000:01:00.0');
  expect(gpuChartSensors(schema, null)).toEqual([]);
});
