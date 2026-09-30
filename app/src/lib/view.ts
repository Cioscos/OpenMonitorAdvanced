/** `settings` is the settings screen; only `simple` and `advanced` are ever remembered as the last view. */
export type View = 'simple' | 'advanced' | 'settings';

/** A place inside the settings screen: today only the rules, optionally with "New rule" filled in for a sensor. */
export interface SettingsTarget {
  section: 'rules';
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
