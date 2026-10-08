import { formatBytes } from '../format';
import type { Params, Translate } from '../i18n/index.svelte';
import type { DataSize, DiskErrorKind, ErrorRecord, KernelId, LoadMode, Phase, Schema, Sensor, SessionEvent } from '../types';
import { diskErrorText, formatBytes as formatDiskBytes, formatMbs } from './disk';

/** The planned length shown to the user: the disk fill ends when the file is written, not on time (R7). */
export function timedSeconds(phases: Phase[]): number {
  return phases.reduce((sum, p) => sum + (p.kernel === 'disk_fill' ? 0 : p.duration_s), 0);
}

/** A disk phase that can end before its time: at its write cap, after its cycles, or V3 once the disk is full. */
export function endsEarly(phase: Phase): boolean {
  return phase.kernel === 'v3' || phase.disk?.write_cap_bytes != null || phase.disk?.cycles != null;
}

/** «5 min», «1 h 30 min», «8 h»; seconds only under an hour («1 min 30 s»). Same units in every language. */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const parts = [h && `${h} h`, m && `${m} min`, h === 0 && s % 60 && `${s % 60} s`].filter(Boolean);
  return parts.length ? parts.join(' ') : '0 s';
}

/** The size each kernel works on by default (`oma-core::load::plan::default_size`): its name already says it. */
const DEFAULT_SIZE: Partial<Record<KernelId, DataSize>> = { k2: 'l2', k5: 'l2', k7: 'l3', k3: 'ram', k10: 'ram' };
const SIZE_LABEL: Record<DataSize, string | null> = { l1: 'L1', l2: 'L2', l3: 'L3', ram: 'RAM', auto: null, fixed: null };

/** The data size of a phase when it is not the kernel's own («L3»), else null. */
export function sizeLabel(phase: Phase): string | null {
  return phase.size === (DEFAULT_SIZE[phase.kernel] ?? 'auto') ? null : SIZE_LABEL[phase.size];
}

/** The GPU kernels (`s1`…`s6`), which have no instruction set. */
export function isGpuKernel(kernel: KernelId): boolean {
  return kernel.startsWith('s');
}

/** «FFT piccole · core e cache · AVX2»: the kernel's name (T1), its size when not its own, and the set (not for a GPU). */
export function phaseLabel(phase: Phase, t: Translate): string {
  const isa = isGpuKernel(phase.kernel) ? null : t(`glossary.isa.${phase.isa}.name`);
  return [t(`glossary.mode.${phase.kernel}.name`), sizeLabel(phase), isa].filter(Boolean).join(' · ');
}

/** The glossary term of a load mode: the wire's snake_case (`pause_resume`) is camelCase there (`mode.pauseResume`). */
export function modeTerm(mode: LoadMode): string {
  return `mode.${mode.replace(/_(\w)/g, (_, c: string) => c.toUpperCase())}`;
}

/** A text whose `[term]` carries a tooltip, cut around it: `[before, term, after]` (no brackets: `[text, '', '']`). */
export function marked(text: string): [string, string, string] {
  const match = /\[([^\]]+)\]/.exec(text);
  return match ? [text.slice(0, match.index), match[1], text.slice(match.index + match[0].length)] : [text, '', ''];
}

/** `text` cut around the first `word` (any case), so the word can carry its term; for fixed texts without brackets. */
export function around(text: string, word: string): [string, string, string] {
  const at = text.toLowerCase().indexOf(word.toLowerCase());
  return at < 0 ? [text, '', ''] : [text.slice(0, at), text.slice(at, at + word.length), text.slice(at + word.length)];
}

/** A run of text, carrying a glossary term or not. */
export interface Piece {
  text: string;
  term: string | null;
}

/**
 * `text` cut into pieces so each term's word (its `glossary.<term>.name` unless `word` is given,
 * any case, first occurrence) carries its term; a word that is not there is skipped.
 */
export function pieces(text: string, terms: { term: string; word?: string }[], t: Translate): Piece[] {
  let out: Piece[] = [{ text, term: null }];
  for (const { term, word = t(`glossary.${term}.name`) } of terms) {
    const i = out.findIndex((p) => p.term === null && p.text.toLowerCase().includes(word.toLowerCase()));
    if (i < 0) continue;
    const [before, hit, after] = around(out[i].text, word);
    const split = [{ text: before, term: null }, { text: hit, term }, { text: after, term: null }].filter((p) => p.text !== '');
    out = [...out.slice(0, i), ...split, ...out.slice(i + 1)];
  }
  return out;
}

/** The glossary terms each event's text mentions. */
const EVENT_TERMS: Record<string, string[]> = {
  thermal_stop: ['thermalStop'],
  reference_invalid: ['phase', 'reference'],
  reference_invalid_gpu: ['phase'],
  ram_reduced: ['phase', 'threads'],
  ram_insufficient: ['phase'],
  k9_needs_two_cores: ['phase', 'mode.k9'],
  whea: ['whea', 'apicId'],
  whea_unreadable: ['whea'],
  vram_allocated: ['vram'],
  vram_reduced: ['vram'],
  vram_bits: ['vram'],
  vram_words: ['vram'],
  device_lost: ['phase', 'deviceLost'],
  artifact_tiles: ['artifact'],
  pcie_replay: ['pcieReplay'],
  bugcheck: ['bugcheck'],
  kernelPower41: ['kernelPower41'],
};

/**
 * A line of the event log in words (`performance.event.<code>`), cut around the terms it
 * mentions. Phases count from 1 as a person does; `whea17`…`whea19` (found after a crash) read
 * like the live `whea`. An unknown code shows itself.
 */
export function eventText(event: SessionEvent, t: Translate, locale: string): Piece[] {
  const recovered = /^whea(\d+)$/.exec(event.code);
  const code = recovered ? 'whea' : event.code;
  const p: Params = { ...event.params };
  if (recovered) p.id = recovered[1];
  if (p.phase !== undefined && Number.isFinite(Number(p.phase))) p.phase = Number(p.phase) + 1;
  if ((code === 'ram_reduced' || code === 'vram_allocated' || code === 'vram_reduced') && p.value !== undefined) p.value = formatBytes(Number(p.value), locale);
  if (code === 'file_bytes' || code === 'slc_cliff' || code === 'disk_sector') p.size = formatDiskBytes(Number(p.value), locale);
  if (code === 'slc_steady') p.speed = formatMbs(Number(p.value), locale);
  if (code === 'vram_bits' && p.value !== undefined) p.value = `0x${Number(p.value).toString(16).toUpperCase()}`;
  const terms: { term: string; word?: string }[] = (EVENT_TERMS[code] ?? []).map((term) => ({ term }));
  if (code === 'whea') {
    const where = p.core === undefined ? 'apic' : p.apic === undefined ? 'core' : 'coreApic';
    p.where = p.core === undefined && p.apic === undefined ? '' : t(`performance.event.where.${where}`, p);
    if (p.core !== undefined) terms.push({ term: 'coreNumber', word: t('performance.core.label', { core: p.core }) });
  }
  const key = `performance.event.${code}`;
  const text = t(key, p);
  return text === key ? [{ text: event.code, term: null }] : pieces(text, terms, t);
}

/**
 * A verdict's title: the T3 text (`performance.outcome.<verdict>`) with its parameters; a
 * recovered `phase` (counted from 1) reads «during phase 3», and the `stability` percent
 * («95.3») is in the locale's format.
 */
export function verdictTitle(detail: { verdict: string | null; params: Record<string, string> } | null, t: Translate, locale = 'en'): string {
  if (!detail?.verdict) return t('performance.result.unknown');
  const params: Params = { ...detail.params };
  if (params.stability !== undefined && Number.isFinite(Number(params.stability))) params.stability = percentText(Number(params.stability) / 100, locale);
  if (params.phase !== undefined) params.phase = t('performance.result.phaseN', { n: params.phase });
  // A `failed_to_start` reason is a text, or the key of one (`performance.start.*`).
  if (typeof params.reason === 'string' && params.reason.startsWith('performance.start.')) params.reason = t(params.reason);
  return t(`performance.outcome.${detail.verdict}`, params);
}

/** DA5: the CPU temperature (first present) and package power, as the chart's series. */
const CPU_TEMPERATURES = ['tdie', 'tctl', 'package', 'core-max'].map((name) => `cpu/0/temperature/${name}`);
export function cpuChartSensors(schema: Schema | null): Sensor[] {
  const byId = new Map(schema?.sensors.map((s) => [s.id, s]));
  const temperature = CPU_TEMPERATURES.map((id) => byId.get(id)).find(Boolean);
  return [temperature, byId.get('cpu/0/power/package')].filter((s): s is Sensor => s !== undefined);
}

/** DA5/DG12: the GPU temperature (core, else hotspot) and board power of a GPU device, as the chart's series; `deviceId` is the test's GPU (`RunStatus.gpuDeviceId`). */
export function gpuChartSensors(schema: Schema | null, deviceId: string | null): Sensor[] {
  const byId = new Map(schema?.sensors.map((s) => [s.id, s]));
  const device = deviceId;
  if (device === null) return [];
  const temperature = ['core', 'hotspot'].map((name) => byId.get(`${device}/temperature/${name}`)).find(Boolean);
  return [temperature, byId.get(`${device}/power/board`)].filter((s): s is Sensor => s !== undefined);
}

/** The four device-lost HRESULTs the GPU engine reports (DG4), by name. */
const HRESULT_NAMES: Record<number, string> = {
  0x887a0005: 'DXGI_ERROR_DEVICE_REMOVED',
  0x887a0006: 'DXGI_ERROR_DEVICE_HUNG',
  0x887a0007: 'DXGI_ERROR_DEVICE_RESET',
  0x887a0020: 'DXGI_ERROR_DRIVER_INTERNAL_ERROR',
};

/** «DXGI_ERROR_DEVICE_HUNG (0x887A0006)»; an HRESULT of no name is just the hex. */
export function hresultText(code: number): string {
  const hex = `0x${(code >>> 0).toString(16).toUpperCase().padStart(8, '0')}`;
  const name = HRESULT_NAMES[code >>> 0];
  return name ? `${name} (${hex})` : hex;
}

/** A 0–1 ratio as a percent with one decimal, in the locale's format («95.3»). */
export function percentText(ratio: number, locale: string): string {
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 1, minimumFractionDigits: 1 }).format(ratio * 100);
}

/**
 * A refusal of the shell in words: `build:<code>` (a plan that cannot be built) and `busy` are
 * translated, anything else is the shell's own text.
 */
export function errorText(error: unknown, t: Translate, locale = 'en'): string {
  const text = String(error);
  if (text === 'busy') return t('performance.wizard.busy');
  const disk = diskErrorText(text, t, locale);
  if (disk) return disk;
  if (text === 'build:no_disk') return t('performance.start.no_disk');
  if (!text.startsWith('build:')) return text;
  const key = `performance.wizard.error.${text.slice('build:'.length)}`;
  return t(key) === key ? text : t(key);
}

/** The disk stress kernels (`disk_fill`, N1-N4, V1-V4). */
export function isDiskKernel(kernel: KernelId): boolean {
  return kernel === 'disk_fill' || /^[nv][1-4]$/.test(kernel);
}

/** The glossary term of a kernel: its id, with the wire's snake_case in camelCase (`disk_fill` is `mode.diskFill`). */
export function kernelTerm(kernel: KernelId): string {
  return `mode.${kernel.replace(/_(\w)/g, (_, c: string) => c.toUpperCase())}`;
}

/**
 * M8c DC10: the test disk's temperature (`temperature/drive`, else the first other one) and its read
 * and write speeds, as the chart's series; `deviceId` is the disk the test runs on.
 */
export function diskChartSensors(schema: Schema | null, deviceId: string | null): Sensor[] {
  if (deviceId === null || !schema) return [];
  const own = schema.sensors.filter((s) => s.deviceId === deviceId);
  const temperature = own.find((s) => s.id === `${deviceId}/temperature/drive`) ?? own.find((s) => s.id.startsWith(`${deviceId}/temperature/`));
  return [temperature, ...['read', 'write'].map((name) => own.find((s) => s.id === `${deviceId}/throughput/${name}`))].filter((s): s is Sensor => s !== undefined);
}

/** Where a disk error is in the file: its 4 KiB block index (for V3, the file's index times 262 144 plus the block) times 4096. */
export function formatOffset(index: number, locale = 'en'): string {
  const bytes = index * 4096;
  return bytes < 1024 ** 2 ? `${(bytes / 1024).toLocaleString(locale)} KiB` : formatDiskBytes(bytes, locale);
}

/** A disk data error in words: `bit_flip` says how many bits (`actual`), `io_error` the Win32 code (`actual`). */
export function dataErrorKind(error: ErrorRecord, t: Translate): string {
  const kind = error.kind as DiskErrorKind;
  if (kind === 'bit_flip' && error.actual === 1) return t('performance.result.error.bit_flip_one');
  return t(`performance.result.error.${kind}`, { bits: error.actual, code: error.actual });
}

/** «3 wrong bits at 12 KiB in the file · transient…»: the whole line of a disk data error; `transient` null says nothing about a re-read. */
export function dataErrorLine(error: ErrorRecord, t: Translate, locale = 'en'): string {
  const where = t('performance.result.dataErrorAt', { kind: dataErrorKind(error, t), offset: formatOffset(error.iteration, locale) });
  return error.transient == null ? where : `${where} · ${t(error.transient ? 'performance.result.transient' : 'performance.result.persistent')}`;
}
