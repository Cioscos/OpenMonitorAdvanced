import { formatBytes, formatPercent, formatValue, type FormatOptions } from '../format';
import { catalogs, i18n, t as translateNow, type Translate } from '../i18n/index.svelte';
import type { ValueOf } from '../select';
import type { Device, DeviceKind, Schema, Sensor, SensorStats, Source, Unit } from '../types';

/** Statistics since app start (or the last reset) of one sensor; null while unknown. */
export type StatsOf = (id: string) => SensorStats | null;

export interface SensorGroup {
  category: string;
  sensors: Sensor[];
}

/** Table order of the sensor categories (spec §7.3); unknown categories follow alphabetically. */
export const CATEGORY_ORDER = ['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'counter', 'throughput', 'link', 'energy', 'flag'];

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
  // Averaging a PCIe generation or lane count is meaningless (e.g. "Gen 3" from Gen 1 and
  // Gen 4 readings): show the dash, same as a missing value.
  if (unit === 'pcie_generation' || unit === 'lanes') return formatValue(null, unit, locale, t, opts);
  return unit === 'boolean' ? formatPercent(stats.avg * 100, locale) : formatValue(stats.avg, unit, locale, t, opts);
}

/** One headline figure of a page (spec §7.3: 4 KPIs per component). */
export interface KpiDef {
  id: string;
  labelKey: string;
  value: (valueOf: ValueOf, stats: StatsOf) => number | null;
  unit: Unit;
  /** Optional small text under the value, already translated. */
  secondary?: (valueOf: ValueOf, stats: StatsOf) => string | null;
}

const KPI_COUNT = 4;

const sensorsWith = (schema: Schema, deviceIds: string[], key: string): Sensor[] =>
  schema.sensors.filter((s) => deviceIds.includes(s.deviceId) && s.label.key === key);

/** First sensor of the page with any of `keys`, in the order of the keys (fallbacks). */
function firstOf(schema: Schema, deviceIds: string[], ...keys: string[]): Sensor | undefined {
  for (const key of keys) {
    const found = sensorsWith(schema, deviceIds, key)[0];
    if (found) return found;
  }
  return undefined;
}

/** First sensor of the page in `category` (unlike `firstOf`, matched by category, not label key). */
function firstByCategory(schema: Schema, deviceIds: string[], category: string): Sensor | undefined {
  return schema.sensors.find((s) => deviceIds.includes(s.deviceId) && s.category === category);
}

const kpi = (id: string, unit: Unit, value: KpiDef['value'], secondary?: KpiDef['secondary']): KpiDef => ({
  id,
  labelKey: `advanced.kpi.${id}`,
  unit,
  value,
  ...(secondary ? { secondary } : {}),
});

/** Live value of a sensor. */
const live = (id: string, sensor: Sensor | undefined): KpiDef | null =>
  sensor ? kpi(id, sensor.unit, (valueOf) => valueOf(sensor.id)) : null;

/** Highest value since start (or reset), from the core statistics. */
const peak = (id: string, sensor: Sensor | undefined): KpiDef | null =>
  sensor ? kpi(id, sensor.unit, (_valueOf, stats) => stats(sensor.id)?.max ?? null) : null;

function maxOf(valueOf: ValueOf, sensors: Sensor[]): number | null {
  const values = sensors.map((s) => valueOf(s.id)).filter((v): v is number => v !== null);
  return values.length ? Math.max(...values) : null;
}

function candidates(kind: DeviceKind, schema: Schema, ids: string[]): (KpiDef | null)[] {
  const find = (...keys: string[]) => firstOf(schema, ids, ...keys);
  switch (kind) {
    case 'cpu': {
      const threads = sensorsWith(schema, ids, 'cpu.load.thread');
      return [
        live('load', find('cpu.load.total')),
        live('temperature', find('cpu.temperature.package', 'cpu.temperature.tctl')),
        live('power', find('cpu.power.package')),
        live('clock', find('cpu.clock.effective')),
        threads.length ? kpi('busiestThread', threads[0].unit, (valueOf) => maxOf(valueOf, threads)) : null,
        peak('peakLoad', find('cpu.load.total')),
      ];
    }
    case 'motherboard':
    case 'fan_controller':
    case 'psu':
      return [
        live('temperature', firstByCategory(schema, ids, 'temperature')),
        live('fan', firstByCategory(schema, ids, 'fan')),
        live('voltage', firstByCategory(schema, ids, 'voltage')),
        live('power', firstByCategory(schema, ids, 'power')),
      ];
    case 'gpu': {
      const used = find('gpu.memory.dedicatedUsed');
      const total = find('gpu.memory.dedicatedTotal');
      return [
        live('load', find('gpu.load.core')),
        live('temperature', find('gpu.temperature.core', 'gpu.temperature.hotspot')),
        live('power', find('gpu.power.board', 'gpu.power.limitPercent')),
        used
          ? kpi(
              'vram',
              used.unit,
              (valueOf) => valueOf(used.id),
              total
                ? (valueOf) => {
                    const bytes = valueOf(total.id);
                    return bytes === null ? null : translateNow('advanced.kpi.vramOf', { total: formatBytes(bytes, i18n.locale) });
                  }
                : undefined,
            )
          : null,
        live('clock', find('gpu.clock.core')),
      ];
    }
    case 'memory': {
      const used = find('memory.used');
      const total = find('memory.total');
      return [
        live('load', find('memory.load')),
        live('used', used),
        live('total', total),
        used && total
          ? kpi('available', total.unit, (valueOf) => {
              const all = valueOf(total.id);
              const inUse = valueOf(used.id);
              return all === null || inUse === null ? null : Math.max(0, all - inUse);
            })
          : null,
      ];
    }
    case 'storage': {
      const free = find('storage.volumeFree');
      return [
        live('active', find('storage.active')),
        live('read', find('storage.read')),
        live('write', find('storage.write')),
        live('temperature', find('storage.temperature')),
        free ? kpi('freeSpace', free.unit, (valueOf) => valueOf(free.id), () => free.label.arg ?? null) : null,
      ];
    }
    case 'network':
      return [
        live('down', find('network.down')),
        live('up', find('network.up')),
        live('linkSpeed', find('network.linkSpeed')),
        peak('peakDown', find('network.down')),
      ];
    default:
      return [];
  }
}

/** The KPIs of a page: the first 4 of its kind whose sensors exist, in definition order. */
export function kpisFor(kind: DeviceKind, schema: Schema, deviceIds: string[]): KpiDef[] {
  return candidates(kind, schema, deviceIds)
    .filter((k): k is KpiDef => k !== null)
    .slice(0, KPI_COUNT);
}

const DEFAULT_SERIES: Partial<Record<DeviceKind, string[][]>> = {
  cpu: [['cpu.load.total'], ['cpu.clock.effective']],
  gpu: [['gpu.load.core'], ['gpu.temperature.core', 'gpu.temperature.hotspot']],
  memory: [['memory.load']],
  storage: [['storage.read'], ['storage.write']],
  network: [['network.down'], ['network.up']],
};

/** Kinds whose default series are the first two temperature sensors (LHM-fed, no fixed label keys). */
const TEMPERATURE_LED_KINDS: DeviceKind[] = ['motherboard', 'fan_controller', 'psu'];

/** Series charted when the user has not chosen any; the first sensor for other kinds. */
export function defaultSeries(kind: DeviceKind, schema: Schema, deviceIds: string[]): string[] {
  const ids = TEMPERATURE_LED_KINDS.includes(kind)
    ? schema.sensors
        .filter((s) => deviceIds.includes(s.deviceId) && s.category === 'temperature')
        .slice(0, 2)
        .map((s) => s.id)
    : (DEFAULT_SERIES[kind] ?? []).map((keys) => firstOf(schema, deviceIds, ...keys)?.id).filter((id): id is string => id !== undefined);
  if (ids.length > 0) return ids;
  const first = schema.sensors.find((s) => deviceIds.includes(s.deviceId));
  return first ? [first.id] : [];
}

/** Display order of the static device properties; unknown keys follow alphabetically. */
export const PROPERTY_ORDER = [
  'pciAddress',
  'integrated',
  'pcieMaxGen',
  'pcieMaxWidth',
  'powerLimitDefaultW',
  'powerLimitMinW',
  'powerLimitMaxW',
  'tempSlowdownC',
  'tempShutdownC',
  'tempMaxC',
  'tempWarningC',
  'tempCriticalC',
  'tjMaxC',
];

/**
 * How numeric properties are shown. The labels of the W limits already carry the unit
 * (Task 9), so those values are plain numbers; the °C limits are temperatures, shown in the
 * temperature unit with its symbol; the PCIe ones read "Gen 4" and "x16".
 */
const PROPERTY_FORMATS: Record<string, Unit | 'number'> = {
  pcieMaxGen: 'pcie_generation',
  pcieMaxWidth: 'lanes',
  powerLimitDefaultW: 'number',
  powerLimitMinW: 'number',
  powerLimitMaxW: 'number',
  tempSlowdownC: 'celsius',
  tempShutdownC: 'celsius',
  tempMaxC: 'celsius',
  tempWarningC: 'celsius',
  tempCriticalC: 'celsius',
  tjMaxC: 'celsius',
};

/** Properties meant for other screens: `smartSelectable` feeds the Settings' SMART switches. */
const HIDDEN_PROPERTIES = new Set(['smartSelectable']);

export interface PropertyRow {
  key: string;
  label: string;
  value: string;
}

/** Device properties as translated label/value rows (`property.<key>`), in display order. */
export function propertyRows(device: Device, locale: string, t: Translate): PropertyRow[] {
  const rank = (key: string) => {
    const i = PROPERTY_ORDER.indexOf(key);
    return i < 0 ? PROPERTY_ORDER.length : i;
  };
  return Object.entries(device.properties ?? {})
    .filter(([key]) => !HIDDEN_PROPERTIES.has(key))
    .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
    .map(([key, raw]) => {
      const labelKey = `property.${key}`;
      const label = labelKey in catalogs.en ? t(labelKey) : key;
      const format = PROPERTY_FORMATS[key];
      const number = Number(raw);
      let value = raw;
      if (key === 'integrated') value = t(raw === 'true' ? 'advanced.info.yes' : 'advanced.info.no');
      else if (format && raw.trim() !== '' && Number.isFinite(number)) {
        value = format === 'number' ? new Intl.NumberFormat(locale).format(number) : formatValue(number, format, locale, t);
      }
      return { key, label, value };
    });
}
