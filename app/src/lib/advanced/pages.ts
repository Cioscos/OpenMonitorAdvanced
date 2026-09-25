import { formatPercent, formatValue, type FormatOptions } from '../format';
import { catalogs, type Translate } from '../i18n/index.svelte';
import type { Sensor, SensorStats, Source, Unit } from '../types';

/** Statistics since app start (or the last reset) of one sensor; null while unknown. */
export type StatsOf = (id: string) => SensorStats | null;

export interface SensorGroup {
  category: string;
  sensors: Sensor[];
}

/** Table order of the sensor categories (spec §7.3); unknown categories follow alphabetically. */
export const CATEGORY_ORDER = ['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'throughput', 'link', 'energy', 'flag'];

/** Groups by `category`, keeping the schema order inside each group. */
export function groupSensors(sensors: Sensor[]): SensorGroup[] {
  const groups = new Map<string, Sensor[]>();
  for (const sensor of sensors) {
    const list = groups.get(sensor.category);
    if (list) list.push(sensor);
    else groups.set(sensor.category, [sensor]);
  }
  const rank = (category: string) => {
    const i = CATEGORY_ORDER.indexOf(category);
    return i < 0 ? CATEGORY_ORDER.length : i;
  };
  return [...groups.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
    .map(([category, list]) => ({ category, sensors: list }));
}

/** Group heading: `advanced.category.<name>` when translated, else the raw category. */
export function categoryLabel(category: string, t: Translate): string {
  const key = `advanced.category.${category}`;
  return key in catalogs.en ? t(key) : category;
}

/** Short badge text for a source, e.g. "NVML", "IP HELPER". */
export function sourceCode(source: Source): string {
  return source.replace(/_/g, ' ').toUpperCase();
}

/** Average column: for on/off flags it is the share of time the flag was on. */
export function formatAverage(
  stats: SensorStats | null,
  unit: Unit,
  locale: string,
  t: Translate,
  opts: FormatOptions = {},
): string {
  if (!stats) return formatValue(null, unit, locale, t, opts);
  return unit === 'boolean' ? formatPercent(stats.avg * 100, locale) : formatValue(stats.avg, unit, locale, t, opts);
}
