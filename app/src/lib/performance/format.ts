import type { Translate } from '../i18n/index.svelte';
import type { DataSize, KernelId, Phase } from '../types';

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
const SIZE_LABEL: Record<DataSize, string | null> = { l1: 'L1', l2: 'L2', l3: 'L3', ram: 'RAM', auto: null };

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
