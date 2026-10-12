import { translate } from '../i18n/index.svelte';
import { warningText } from './warnings';

const en = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
const it = (key: string, params?: Record<string, string | number>) => translate('it', key, params);

test('low_contrast_names_the_roles_with_a_localized_ratio', () => {
  const w = { line: 0, code: 'lowContrast' as const, detail: '--text-muted/--bg 2.1' };
  expect(warningText(w, en, 'en')).toBe('The secondary text on background has low contrast: 2.1:1');
  expect(warningText(w, it, 'it')).toBe('Il testo secondario su sfondo ha un contrasto basso: 2,1:1');
  const button = { line: 0, code: 'lowContrast' as const, detail: '--on-accent/--accent 3.0' };
  expect(warningText(button, it, 'it')).toBe("Il testo dei pulsanti su colore d'accento ha un contrasto basso: 3:1");
});

test('every_contrast_pair_has_role_labels', () => {
  const fg = ['text', 'text-muted', 'on-accent', 'accent', 'ok', 'warn', 'crit'];
  const bg = ['bg', 'surface', 'surface-2', 'accent'];
  for (const tr of [en, it]) {
    for (const n of fg) expect(tr(`themes.role.${n}`)).not.toBe(`themes.role.${n}`);
    for (const n of bg) expect(tr(`themes.role.${n}.bg`)).not.toBe(`themes.role.${n}.bg`);
  }
});

test('other_warnings_use_line_and_detail', () => {
  const w = { line: 4, code: 'unknownVariable' as const, detail: '--nope' };
  expect(warningText(w, en, 'en')).toBe('Line 4: --nope is not a theme variable; ignored.');
  // An unexpected contrast detail is shown as is.
  expect(warningText({ line: 0, code: 'lowContrast', detail: 'odd' }, en, 'en')).toBe('odd');
});
