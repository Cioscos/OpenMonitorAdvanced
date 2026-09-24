import type { HistorySeed, Schema, Snapshot } from '../types';

export type Unsubscribe = () => void;

/** Everything the UI needs from the sampling core (Tauri, or a mock in the browser). */
export interface Backend {
  getSchema(): Promise<Schema>;
  getHistory(ids: string[], seconds: number): Promise<HistorySeed>;
  onSchema(cb: (schema: Schema) => void): Promise<Unsubscribe>;
  onSnapshot(cb: (snapshot: Snapshot) => void): Promise<Unsubscribe>;
}
