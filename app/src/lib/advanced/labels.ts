import type { Translate } from '../i18n/index.svelte';
import type { Sensor } from '../types';

/** Translated sensor name: `sensor.<label.key>` with the optional `{arg}`. */
export function sensorLabel(sensor: Sensor, t: Translate): string {
  return t(`sensor.${sensor.label.key}`, sensor.label.arg === undefined ? {} : { arg: sensor.label.arg });
}
