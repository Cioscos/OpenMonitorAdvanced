import type { Params } from './i18n/index.svelte';

export type HealthLevel = 'neutral' | 'ok' | 'warn' | 'crit';

export interface HealthState {
  level: HealthLevel;
  messageKey: string;
  params?: Params;
  sinceMs: number;
}

/**
 * Milestone 1 has no rules engine yet (milestone 5): the banner only reports
 * that monitoring is running and for how long.
 */
export function monitoringHealth(startedAtMs: number): HealthState {
  return { level: 'neutral', messageKey: 'health.monitoring', sinceMs: startedAtMs };
}
