import type { DeviceKind, Schema } from '../types';

/** One sidebar entry of the Advanced view: a page over one or more devices. */
export interface SidebarEntry {
  /** Section id: the device id (the CPU entry is always 'cpu/0'). Persisted in localStorage. */
  id: string;
  kind: DeviceKind;
  deviceIds: string[];
  /** `advanced.section.<kind>`. */
  labelKey: string;
  /** Device name shown under the label (not translated); absent for memory. */
  labelArg?: string;
}

export type SimpleTile = 'cpu' | 'gpu' | 'memory' | 'storage' | 'network';

const CPU_SECTION = 'cpu/0';
const MEMORY_SECTION = 'memory/0';
/** Kinds with one entry for all their devices; every other kind gets one entry per device. */
const MERGED: readonly DeviceKind[] = ['cpu', 'memory'];
const ORDER: readonly DeviceKind[] = ['cpu', 'gpu', 'memory', 'storage', 'network', 'motherboard', 'battery', 'fan_controller', 'psu'];

/**
 * Sidebar order (spec §7.3): CPU, every GPU (integrated ones too), memory, one entry per
 * disk, one per network adapter, then any other kind. Devices without sensors are skipped.
 */
export function sidebarEntries(schema: Schema): SidebarEntry[] {
  const withSensors = new Set(schema.sensors.map((s) => s.deviceId));
  const entries: SidebarEntry[] = [];
  for (const kind of ORDER) {
    const devices = schema.devices.filter((d) => d.kind === kind && withSensors.has(d.id));
    if (devices.length === 0) continue;
    const labelKey = `advanced.section.${kind}`;
    if (MERGED.includes(kind)) {
      entries.push({
        id: kind === 'cpu' ? CPU_SECTION : devices[0].id,
        kind,
        deviceIds: devices.map((d) => d.id),
        labelKey,
        labelArg: kind === 'cpu' ? devices[0].name : undefined,
      });
    } else {
      for (const d of devices) entries.push({ id: d.id, kind, deviceIds: [d.id], labelKey, labelArg: d.name });
    }
  }
  return entries;
}

/** Section a Simple view tile opens; null lets the Advanced view keep its last page. */
export function sectionForTile(tile: SimpleTile, deviceId?: string): string | null {
  switch (tile) {
    case 'cpu':
      return CPU_SECTION;
    case 'memory':
      return deviceId ?? MEMORY_SECTION;
    default:
      return deviceId ?? null;
  }
}

/** The wanted section if it still exists, else the first entry (CPU), else null. */
export function resolveSection(entries: SidebarEntry[], wanted: string | null): string | null {
  if (wanted !== null && entries.some((e) => e.id === wanted)) return wanted;
  return entries[0]?.id ?? null;
}
