import { connectSettings, disconnectSettings } from '../test/settings';
import { formatTemperature, formatValue } from './format';
import { translate } from './i18n/index.svelte';
import { settings } from './settings.svelte';
import { display, temperatureSymbol, toDisplayTemperature } from './units.svelte';

afterEach(disconnectSettings);

const tEn = (key: string) => translate('en', key);

test('fahrenheit conversion', () => {
  expect(toDisplayTemperature(0, 'f')).toBe(32);
  expect(toDisplayTemperature(100, 'f')).toBe(212);
  expect(toDisplayTemperature(-40, 'f')).toBe(-40);
  expect(toDisplayTemperature(37.5, 'c')).toBe(37.5);
  expect(temperatureSymbol('c')).toBe('°C');
  expect(temperatureSymbol('f')).toBe('°F');
  expect(formatTemperature(100, 'en', 'f')).toBe('212 °F');
  expect(formatTemperature(100, 'en', 'c')).toBe('100 °C');
  expect(formatTemperature(36.6, 'en', 'f')).toBe('98 °F');
  expect(formatTemperature(null, 'en', 'f')).toBe('—');
  expect(formatTemperature(Number.NaN, 'en')).toBe('—');
});

test('display follows the settings store and falls back to the defaults', async () => {
  expect(settings.state).toBeNull();
  expect([display.temperature, display.throughput, display.chartFps]).toEqual(['c', 'bits', 60]);

  await connectSettings({ general: { temperatureUnit: 'f', throughputUnit: 'bytes', chartFps: 30 } });
  expect([display.temperature, display.throughput, display.chartFps]).toEqual(['f', 'bytes', 30]);
  expect(formatTemperature(100, 'en')).toBe('212 °F');
  expect(formatValue(100, 'celsius', 'en', tEn)).toBe('212 °F');

  await settings.update({ general: { temperatureUnit: 'c' } });
  expect(formatValue(100, 'celsius', 'en', tEn)).toBe('100 °C');
});
