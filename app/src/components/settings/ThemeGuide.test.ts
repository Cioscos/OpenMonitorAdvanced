import { cleanup, render } from '@testing-library/svelte';
import variables from '../../themes/variables.json';
import en from '../../lib/i18n/en.json';
import it from '../../lib/i18n/it.json';
import { i18n, t } from '../../lib/i18n/index.svelte';
import ThemeGuide from './ThemeGuide.svelte';

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(cleanup);

test('every_variable_has_a_description_in_both_languages', () => {
  for (const dict of [en, it] as Record<string, string>[]) {
    for (const v of variables) {
      for (const key of [`themes.var.${v.name.slice(2)}`, `themes.group.${v.group}`, `themes.kind.${v.kind}`]) {
        expect(dict[key], key).toBeTruthy();
      }
    }
  }
});

test('guide_keys_match_in_both_languages', () => {
  const keys = (d: object) => Object.keys(d).filter((k) => k.startsWith('themes.guide.') || k.startsWith('themes.group.') || k.startsWith('themes.kind.')).sort();
  expect(keys(en)).toEqual(keys(it));
});

test('guide_table_lists_every_variable', () => {
  const { container } = render(ThemeGuide);
  expect(container.querySelectorAll('tbody tr')).toHaveLength(variables.length);
  for (const v of variables) expect(container.textContent).toContain(v.name);
});

test('example_parses_as_three_variables', () => {
  const known = new Set(variables.map((v) => v.name));
  const body = t('themes.guide.example');
  const decls = [...body.matchAll(/^\s*(--[a-z0-9-]+)\s*:\s*[^;]+;/gm)].map((m) => m[1]);
  expect(decls).toHaveLength(3);
  for (const d of decls) expect(known.has(d)).toBe(true);
});

test('header_example_has_each_directive_on_its_own_line', () => {
  for (const locale of ['en', 'it'] as const) {
    i18n.locale = locale;
    const lines = t('themes.guide.headerExample').split('\n').map((l) => l.replace(/^\/\*|\*\/$/g, '').trim());
    expect(lines).toHaveLength(3);
    expect(lines[0]).toMatch(/^@oma-theme \d+$/);
    expect(lines[1]).toMatch(/^@name \S/);
    expect(lines[2]).toMatch(/^@author \S/);
    expect(lines.join('')).not.toContain(',');
  }
});
