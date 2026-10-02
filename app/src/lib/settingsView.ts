import { sensorLabel } from './advanced/labels';
import { catalogs, type Translate } from './i18n/index.svelte';
import type { Schema, SourceDrive } from './types';

/** Accepted update intervals, as in `oma_core::settings`: 0.5 s to 5 s in steps of 0.5 s. */
export const INTERVALS_MS = Array.from({ length: 10 }, (_, i) => (i + 1) * 500);

/** A backend reason code in words when the catalog knows it (`settings.reason.<code>`), else as is. */
export function reasonText(reason: string, t: Translate): string {
  const key = `settings.reason.${reason}`;
  return key in catalogs.en ? t(key) : reason;
}

export interface SensorChoice {
  group: string;
  options: { value: string; label: string }[];
}

/** Temperature and load sensors of the schema, one group per device in schema order. */
export function iconSensorChoices(schema: Schema | null, t: Translate): SensorChoice[] {
  if (schema === null) return [];
  return schema.devices
    .map((device) => ({
      group: device.name,
      options: schema.sensors
        .filter((s) => s.deviceId === device.id && (s.kind === 'temperature' || s.kind === 'load'))
        .map((s) => ({ value: s.id, label: sensorLabel(s, t) })),
    }))
    .filter((group) => group.options.length > 0);
}

/**
 * The drives that keep SMART closed, by name: the device of the schema when the drive matches one,
 * else the model the service read, else "Disk N" with the drive number.
 */
export function blockingDiskNames(drives: SourceDrive[], schema: Schema | null, t: Translate): string[] {
  const names: string[] = [];
  for (const drive of drives) {
    if (!drive.blocksSmart) continue;
    const device = drive.deviceId === null ? undefined : schema?.devices.find((d) => d.id === drive.deviceId);
    const name = device?.name ?? drive.model ?? t('settings.sources.smart.diskNumber', { n: drive.physicalDrive });
    if (!names.includes(name)) names.push(name);
  }
  return names;
}
