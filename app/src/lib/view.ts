/**
 * `settings` is the settings screen and `performance` the stress test view; only `simple` and
 * `advanced` are ever remembered as the last view.
 */
export type View = 'simple' | 'advanced' | 'settings' | 'performance';

import type { NavigationTarget } from './types';

/**
 * A page of the Performance view: a saved session's result is `result:<id>`; `score-cpu` is the
 * CPU benchmark, `score-gpu:<deviceId>` the one of a GPU and `score-disk` the disk one; `board` is the leaderboard.
 */
export type PerformancePage = 'new' | 'run' | 'history' | 'score-cpu' | 'score-disk' | 'board' | `score-gpu:${string}` | `result:${string}`;

/** The page a tray item or a toast asks for (`NavigationTarget.performance`), or null for none. */
export function performancePageOf(perf: NavigationTarget['performance']): PerformancePage | null {
  if (perf?.page === 'run' || perf?.page === 'score-cpu' || perf?.page === 'score-disk') return perf.page;
  if (perf?.page === 'result' && perf.sessionId) return `result:${perf.sessionId}`;
  if (perf?.page === 'score-gpu' && perf.deviceId) return `score-gpu:${perf.deviceId}`;
  return null;
}

/** A place inside the settings screen: the rules, optionally with "New rule" filled in for a sensor. */
export interface SettingsTarget {
  section: 'rules' | 'log' | 'benchmark' | 'performance' | 'about';
  newRuleSensor?: string;
}

// Opening the settings is a navigation inside the UI (not a shell event): the app registers the
// function that shows the screen, and components far from it, like a sensor row, call `openSettings`.
let opener: ((target: SettingsTarget) => void) | null = null;

/** Registers (or, with `null`, removes) the function `openSettings` calls. */
export function setSettingsOpener(open: ((target: SettingsTarget) => void) | null): void {
  opener = open;
}

/** Shows the settings screen on `target`; the view it was opened from stays the one Back returns to. */
export function openSettings(target: SettingsTarget): void {
  opener?.(target);
}
