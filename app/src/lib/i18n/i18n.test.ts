import { catalogs, detectLocale, translate } from './index.svelte';

test('both catalogs define the same keys', () => {
  expect(Object.keys(catalogs.it).sort()).toEqual(Object.keys(catalogs.en).sort());
});

test('interpolates named parameters', () => {
  expect(translate('en', 'health.since', { duration: '5 min' })).toBe('for 5 min');
  expect(translate('it', 'health.since', { duration: '5 min' })).toBe('da 5 min');
});

test('unknown keys fall back to the key itself', () => {
  expect(translate('it', 'nope.nope')).toBe('nope.nope');
});

test('missing parameters stay visible', () => {
  expect(translate('en', 'health.since')).toBe('for {duration}');
});

test('picks the first supported language', () => {
  expect(detectLocale(['it-IT', 'en-US'])).toBe('it');
  expect(detectLocale(['de-DE', 'it'])).toBe('it');
  expect(detectLocale(['de-DE'])).toBe('en');
  expect(detectLocale([])).toBe('en');
});
