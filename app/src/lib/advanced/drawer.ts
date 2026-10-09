import type { Sensor, Unit } from '../types';

export interface DrawerGroup {
  unit: Unit;
  sensors: Sensor[];
}

/** Case- and accent-insensitive form of a text, for the drawer search. */
export function foldText(text: string): string {
  return text.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
}

/** Sensors matching `filter`, grouped by unit: the strips' units first, then the others in schema order. */
export function drawerGroups(sensors: Sensor[], stripUnits: Unit[], filter: string, label: (s: Sensor) => string): DrawerGroup[] {
  const needle = foldText(filter.trim());
  const groups = new Map<Unit, Sensor[]>(stripUnits.map((u) => [u, []]));
  for (const s of sensors) {
    if (!foldText(label(s)).includes(needle)) continue;
    const list = groups.get(s.unit);
    if (list) list.push(s);
    else groups.set(s.unit, [s]);
  }
  return [...groups].filter(([, list]) => list.length > 0).map(([unit, list]) => ({ unit, sensors: list }));
}
