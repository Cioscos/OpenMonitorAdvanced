import { formatBytes } from '../format';
import type { Params, Translate } from '../i18n/index.svelte';
import type { DataSize, KernelId, Phase, Schema, Sensor, SessionEvent } from '../types';

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

/** «FFT piccole · core e cache · AVX2»: the kernel's name (T1), its size when not its own, and the set. */
export function phaseLabel(phase: Phase, t: Translate): string {
  return [t(`glossary.mode.${phase.kernel}.name`), sizeLabel(phase), t(`glossary.isa.${phase.isa}.name`)].filter(Boolean).join(' · ');
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
  ram_reduced: ['phase', 'threads'],
  ram_insufficient: ['phase'],
  k9_needs_two_cores: ['phase', 'mode.k9'],
  whea: ['whea', 'apicId'],
  whea_unreadable: ['whea'],
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
  if (code === 'ram_reduced' && p.value !== undefined) p.value = formatBytes(Number(p.value), locale);
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
 * recovered `phase` (counted from 1) reads «during phase 3».
 */
export function verdictTitle(detail: { verdict: string | null; params: Record<string, string> } | null, t: Translate): string {
  if (!detail?.verdict) return t('performance.result.unknown');
  const params: Params = { ...detail.params };
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

/**
 * A refusal of the shell in words: `build:<code>` (a plan that cannot be built) and `busy` are
 * translated, anything else is the shell's own text.
 */
export function errorText(error: unknown, t: Translate): string {
  const text = String(error);
  if (text === 'busy') return t('performance.wizard.busy');
  if (!text.startsWith('build:')) return text;
  const key = `performance.wizard.error.${text.slice('build:'.length)}`;
  return t(key) === key ? text : t(key);
}
