import { settings } from './settings.svelte';
import type { ChartFps, TemperatureUnit, ThroughputUnit } from './types';

// Deriveds, so a settings change that leaves a value alone does not re-run what reads it.
const temperature = $derived<TemperatureUnit>(settings.state?.settings.general.temperatureUnit ?? 'c');
const throughput = $derived<ThroughputUnit>(settings.state?.settings.general.throughputUnit ?? 'bits');
const chartFps = $derived<ChartFps>(settings.state?.settings.general.chartFps ?? 60);

/**
 * How the UI shows values, read from the settings store (defaults until it has state). The
 * getters are reactive: anything that renders or derives from them updates on a change.
 * Units are display only: sensors, history and thresholds stay in °C and bytes per second.
 */
export const display: { readonly temperature: TemperatureUnit; readonly throughput: ThroughputUnit; readonly chartFps: ChartFps } = {
  get temperature() {
    return temperature;
  },
  get throughput() {
    return throughput;
  },
  get chartFps() {
    return chartFps;
  },
};

/** A temperature in °C as a number of `unit` degrees. */
export function toDisplayTemperature(celsius: number, unit: TemperatureUnit): number {
  return unit === 'f' ? (celsius * 9) / 5 + 32 : celsius;
}

export function temperatureSymbol(unit: TemperatureUnit): '°C' | '°F' {
  return unit === 'f' ? '°F' : '°C';
}
