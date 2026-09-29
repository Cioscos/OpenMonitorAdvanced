// Advanced view state, kept in the settings file (`advanced.*`). The old `localStorage` keys are
// read only once, by `migrateLegacyState`.
import { settings } from '../settings.svelte';

export const STORED_WINDOWS = [60, 300, 1800, 3600] as const;
export type StoredWindow = (typeof STORED_WINDOWS)[number];

const advanced = () => settings.state?.settings.advanced;

/** Saves are fire and forget: a rejected one is logged and the view keeps working for this session. */
function save(patch: Parameters<typeof settings.update>[0]): void {
  settings.update(patch).catch((error) => console.error('cannot save the Advanced view state', error));
}

export function loadSection(): string | null {
  return advanced()?.section || null;
}

export function saveSection(sectionId: string): void {
  save({ advanced: { section: sectionId } });
}

/** The saved chart window, or null when unset or not one of 60/300/1800/3600. */
export function loadWindow(): StoredWindow | null {
  const value = advanced()?.window;
  return STORED_WINDOWS.find((w) => w === value) ?? null;
}

export function saveWindow(seconds: StoredWindow): void {
  save({ advanced: { window: seconds } });
}

/** Sensor ids charted on a section, or null when never saved. */
export function loadSeries(sectionId: string): string[] | null {
  return advanced()?.series[sectionId] ?? null;
}

export function saveSeries(sectionId: string, ids: string[]): void {
  save({ advanced: { series: { [sectionId]: [...ids] } } });
}
