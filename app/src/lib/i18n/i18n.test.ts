import type { Source } from '../types';
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

// `Record<Source, true>` stops compiling when a Source is added to types.ts without an entry here.
const SOURCES: Record<Source, true> = {
  pdh: true,
  win32: true,
  ip_helper: true,
  dxgi: true,
  d3dkmt: true,
  nvml: true,
  nvapi: true,
  adl: true,
  igcl: true,
  pnp: true,
  mock: true,
};

test('every sensor source has a badge name', () => {
  for (const source of Object.keys(SOURCES)) {
    expect(catalogs.en[`source.${source}`], source).toBeDefined();
  }
});

// Device property keys produced by oma-win (GPU layers and storage).
const PROPERTIES = [
  'pciAddress',
  'integrated',
  'pcieMaxGen',
  'pcieMaxWidth',
  'powerLimitMinW',
  'powerLimitMaxW',
  'powerLimitDefaultW',
  'tempSlowdownC',
  'tempShutdownC',
  'tempMaxC',
  'tempWarningC',
  'tempCriticalC',
];

test('every device property has a label', () => {
  for (const key of PROPERTIES) {
    expect(catalogs.en[`property.${key}`], key).toBeDefined();
  }
});

test('flag values and the stale badge are translated', () => {
  expect(translate('en', 'flag.on')).toBe('Active');
  expect(translate('it', 'flag.on')).toBe('Attivo');
  expect(translate('en', 'flag.off')).toBe('No');
  expect(translate('it', 'status.stale')).toBe('Dati non aggiornati');
});
