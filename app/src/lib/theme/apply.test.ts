import { applyTheme } from './apply';

const root = () => document.createElement('div');

test('sets_every_variable_and_color_scheme', () => {
  const el = root();
  const keys = applyTheme({ '--bg': '#000000', '--accent': '#ff00ff', '--color-scheme': 'dark' }, el, []);
  expect(keys.sort()).toEqual(['--accent', '--bg', '--color-scheme']);
  expect(el.style.getPropertyValue('--bg')).toBe('#000000');
  expect(el.style.getPropertyValue('--accent')).toBe('#ff00ff');
  expect(el.style.getPropertyValue('color-scheme')).toBe('dark');
});

test('removes_variables_of_the_previous_theme', () => {
  const el = root();
  const first = applyTheme({ '--bg': '#000000', '--series-5': '#111111', '--color-scheme': 'dark' }, el, []);
  applyTheme({ '--bg': '#ffffff' }, el, first);
  expect(el.style.getPropertyValue('--bg')).toBe('#ffffff');
  expect(el.style.getPropertyValue('--series-5')).toBe('');
  expect(el.style.getPropertyValue('--color-scheme')).toBe('');
  // Without --color-scheme the property goes back to the stylesheet's.
  expect(el.style.getPropertyValue('color-scheme')).toBe('');
});

test('light_scheme_sets_color_scheme_light', () => {
  const el = root();
  applyTheme({ '--color-scheme': 'light' }, el, []);
  expect(el.style.getPropertyValue('color-scheme')).toBe('light');
});
