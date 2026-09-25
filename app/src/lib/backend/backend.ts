import type { HistorySeed, Schema, Snapshot, StartupStatus } from '../types';

export type Unsubscribe = () => void;

/** Everything the UI needs from the sampling core (Tauri, or a mock in the browser). */
export interface Backend {
  getSchema(): Promise<Schema>;
  getHistory(ids: string[], seconds: number): Promise<HistorySeed>;
  onSchema(cb: (schema: Schema) => void): Promise<Unsubscribe>;
  onSnapshot(cb: (snapshot: Snapshot) => void): Promise<Unsubscribe>;
  /** GPU safe-mode status of this session. */
  getStartupStatus(): Promise<StartupStatus>;
  /** Loads the GPU vendor libraries without a restart; returns the new status. */
  enableVendorLibraries(): Promise<StartupStatus>;
}
