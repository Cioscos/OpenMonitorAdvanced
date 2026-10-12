// The browser backend's themes: the built-ins from their files and user themes in memory.
// The parse is minimal (`--x: y;` declarations only), for development, not validation.
import type { CommandError, ImportPreview, ResolvedTheme, ThemeScheme, ThemeSummary } from '../types';
import { uniqueName } from '../editor/ops';
import en from '../i18n/en.json';
import it from '../i18n/it.json';
import synthwave from '../../themes/builtin/builtin-synthwave.css?raw';
import midnight from '../../themes/builtin/builtin-midnight.css?raw';
import daylight from '../../themes/builtin/builtin-daylight.css?raw';
import contrast from '../../themes/builtin/builtin-contrast.css?raw';
import type { MockSettings } from './mockSettings';

const parseVars = (css: string): Record<string, string> =>
  Object.fromEntries([...css.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));

const BUILTINS: [string, Record<string, string>][] = [
  ['builtin-synthwave', parseVars(synthwave)],
  ['builtin-midnight', parseVars(midnight)],
  ['builtin-daylight', parseVars(daylight)],
  ['builtin-contrast', parseVars(contrast)],
];

const schemeOf = (vars: Record<string, string>): ThemeScheme => (vars['--color-scheme'] === 'light' ? 'light' : 'dark');
const resolve = (vars: Record<string, string>) => ({
  ...BUILTINS[schemeOf(vars) === 'light' ? 2 : 0][1],
  ...vars,
});
const notFound = (): CommandError => ({ key: 'themes.error.notFound', detail: null });

/** What «Import theme…» reads in the browser, where there is no file dialog. */
const SAMPLE = { name: 'Nord Night', author: 'Mock', vars: { '--bg': '#2e3440', '--surface': '#3b4252', '--accent': '#88c0d0' } };

export function mockThemes(settings: MockSettings) {
  const user = new Map<string, { name: string; author: string | null; vars: Record<string, string> }>();
  let pending: number | null = null;
  let nextToken = 1;
  const summary = (id: string): ThemeSummary => {
    const t = user.get(id)!;
    return { id, name: t.name, author: t.author, scheme: schemeOf(t.vars), builtin: false, vars: resolve(t.vars) };
  };
  const builtinNames = BUILTINS.flatMap(([id]) =>
    [en, it].map((c) => (c as Record<string, string>)[`themes.builtin.${id}`]),
  );
  return {
    list: (): ThemeSummary[] => [
      ...BUILTINS.map(([id, vars]) => ({ id, name: null, author: 'OpenMonitor Advanced', scheme: schemeOf(vars), builtin: true, vars })),
      ...[...user.keys()].map(summary).sort((a, b) => a.name!.localeCompare(b.name!)),
    ],
    preview: (): ImportPreview => {
      pending = nextToken++;
      const vars = resolve(SAMPLE.vars);
      return { token: pending, name: SAMPLE.name, author: SAMPLE.author, scheme: schemeOf(vars), vars, applied: 3, warnings: [] };
    },
    confirm: (token: number): ThemeSummary => {
      if (token !== pending) throw notFound();
      pending = null;
      const id = crypto.randomUUID();
      const name = uniqueName([...[...user.values()].map((t) => t.name), ...builtinNames], SAMPLE.name);
      user.set(id, { name, author: SAMPLE.author, vars: { ...SAMPLE.vars } });
      return summary(id);
    },
    has: (id: string) => user.has(id),
    remove: (id: string) => {
      if (!user.delete(id)) throw notFound();
      if (settings.state().settings.appearance.theme === id) settings.update({ appearance: { theme: 'builtin-synthwave' } });
    },
    resolved: (): ResolvedTheme => {
      const id = settings.state().settings.appearance.theme;
      const builtin = BUILTINS.find(([b]) => b === id);
      if (builtin) return { id, vars: builtin[1], fallback: false };
      if (user.has(id)) return { id, vars: summary(id).vars, fallback: false };
      return { id: 'builtin-synthwave', vars: BUILTINS[0][1], fallback: !id.startsWith('builtin-') };
    },
  };
}
