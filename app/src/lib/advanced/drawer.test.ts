import type { Sensor, Unit } from '../types';
import { drawerGroups, foldText } from './drawer';

const s = (id: string, unit: Unit, name: string) => ({ id, unit, label: name }) as unknown as Sensor;
const label = (x: Sensor) => x.label as unknown as string;
const sensors = [s('1', 'watt', 'Potenza'), s('2', 'percent', 'Carico'), s('3', 'celsius', 'Temperatura'), s('4', 'percent', 'Carico 2')];

test('groups_follow_strip_units_then_schema_order', () => {
  const groups = drawerGroups(sensors, ['celsius'], '', label);
  expect(groups.map((g) => g.unit)).toEqual(['celsius', 'watt', 'percent']);
  expect(groups[2].sensors.map((x) => x.id)).toEqual(['2', '4']);
});

test('search_ignores_case_and_accents', () => {
  for (const q of ['temperatúra', 'TEMPERATURA', ' temp ']) {
    expect(drawerGroups(sensors, [], q, label).flatMap((g) => g.sensors.map((x) => x.id))).toEqual(['3']);
  }
  expect(foldText('Città')).toBe('citta');
});

test('empty_groups_are_dropped', () => {
  expect(drawerGroups(sensors, ['celsius', 'volt'], 'carico', label).map((g) => g.unit)).toEqual(['percent']);
});
