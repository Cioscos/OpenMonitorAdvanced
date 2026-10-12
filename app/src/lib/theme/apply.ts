/**
 * Sets the theme's variables on `root` (spec M10 §5.4), `color-scheme` from `--color-scheme`, and
 * removes the variables of the `previous` theme that `vars` lacks, so the stylesheet's values
 * (`theme.css`) show through again. Returns the keys it set, the `previous` of the next call.
 */
export function applyTheme(vars: Record<string, string>, root: { style: CSSStyleDeclaration }, previous: Iterable<string>): string[] {
  const keys = Object.keys(vars);
  for (const key of previous) if (!(key in vars)) root.style.removeProperty(key);
  for (const key of keys) root.style.setProperty(key, vars[key]);
  const scheme = vars['--color-scheme'];
  if (scheme === undefined) root.style.removeProperty('color-scheme');
  else root.style.setProperty('color-scheme', scheme);
  return keys;
}
