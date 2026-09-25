// Advanced view state in localStorage (settings.json arrives with milestone 5).
// Storage can be missing or throw (quota, privacy mode): every access is guarded.

export const SECTION_KEY = 'oma.advanced.section';
export const WINDOW_KEY = 'oma.advanced.window';
export const seriesKey = (sectionId: string) => `oma.advanced.series.${sectionId}`;

export const STORED_WINDOWS = [60, 300, 1800, 3600] as const;
export type StoredWindow = (typeof STORED_WINDOWS)[number];

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not persisted: the view still works for this session.
  }
}

export function loadSection(): string | null {
  const value = read(SECTION_KEY);
  return value ? value : null;
}

export function saveSection(sectionId: string): void {
  write(SECTION_KEY, sectionId);
}

/** The saved chart window, or null when absent or not one of 60/300/1800/3600. */
export function loadWindow(): StoredWindow | null {
  const value = Number(read(WINDOW_KEY));
  return STORED_WINDOWS.find((w) => w === value) ?? null;
}

export function saveWindow(seconds: StoredWindow): void {
  write(WINDOW_KEY, String(seconds));
}

/** Sensor ids charted on a section, or null when never saved or unreadable. */
export function loadSeries(sectionId: string): string[] | null {
  const raw = read(seriesKey(sectionId));
  if (raw === null) return null;
  try {
    const value: unknown = JSON.parse(raw);
    return Array.isArray(value) && value.every((v) => typeof v === 'string') ? value : null;
  } catch {
    return null;
  }
}

export function saveSeries(sectionId: string, ids: string[]): void {
  write(seriesKey(sectionId), JSON.stringify(ids));
}
