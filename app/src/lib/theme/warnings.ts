import type { Locale, Translate } from '../i18n/index.svelte';
import type { ThemeWarning } from '../types';

/** `<fg>/<bg> <ratio>`, the detail of a `lowContrast` warning (`oma_core::theme::contrast_warnings`). */
const PAIR = /^--([\w-]+)\/--([\w-]+) (\d+(?:\.\d+)?)$/;

/** A role label, or the CSS name when the catalogs do not know the variable. */
function role(t: Translate, key: string, name: string): string {
  const label = t(key);
  return label === key ? `--${name}` : label;
}

/**
 * The text of an import warning. A low contrast names the two roles («il testo secondario su
 * sfondo») with the ratio in the reader's number format (spec M10 §2.5).
 */
export function warningText(w: ThemeWarning, t: Translate, locale: Locale): string {
  if (w.code !== 'lowContrast') return t(`themes.warn.${w.code}`, { line: w.line, detail: w.detail });
  const m = PAIR.exec(w.detail);
  if (!m) return w.detail;
  const text = t('themes.warn.lowContrast', {
    fg: role(t, `themes.role.${m[1]}`, m[1]),
    bg: role(t, `themes.role.${m[2]}.bg`, m[2]),
    ratio: new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(Number(m[3])),
  });
  return text.charAt(0).toUpperCase() + text.slice(1);
}
