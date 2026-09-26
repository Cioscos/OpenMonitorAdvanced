import { t } from '../i18n/index.svelte';
import type { Sensor } from '../types';
import { sensorLabel } from './labels';

const sensor = (label: Sensor['label']): Sensor => ({
  id: 'x',
  deviceId: 'motherboard/x',
  kind: 'fan',
  unit: 'rpm',
  label,
  source: 'lhm',
  category: 'fan',
});

test('raw lhm labels show the driver text', () => {
  expect(sensorLabel(sensor({ key: 'lhm.raw', arg: 'Fan #1' }), t)).toBe('Fan #1');
});

test('a known sensor key is translated without an arg', () => {
  expect(sensorLabel(sensor({ key: 'cpu.temperature.package' }), t)).toBe(t('sensor.cpu.temperature.package'));
});
