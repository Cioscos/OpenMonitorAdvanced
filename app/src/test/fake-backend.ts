import type { Backend, Unsubscribe } from '../lib/backend/backend';
import type { HistorySeed, HistoryWindow, Schema, Snapshot } from '../lib/types';

/** Hand-driven backend for tests: emit events explicitly. */
export class FakeBackend implements Backend {
  schema: Schema;
  history: HistoryWindow = { timestampsMs: [], series: [] };
  schemaCalls = 0;
  #schemaListeners = new Set<(s: Schema) => void>();
  #snapshotListeners = new Set<(s: Snapshot) => void>();

  constructor(schema: Schema) {
    this.schema = schema;
  }

  async getSchema(): Promise<Schema> {
    this.schemaCalls++;
    return this.schema;
  }

  async getHistory(ids: string[]): Promise<HistorySeed> {
    return { revision: this.schema.revision, seq: 0, timestampsMs: this.history.timestampsMs, series: ids.map((_, i) => this.history.series[i] ?? []) };
  }

  async onSchema(cb: (s: Schema) => void): Promise<Unsubscribe> {
    this.#schemaListeners.add(cb);
    return () => this.#schemaListeners.delete(cb);
  }

  async onSnapshot(cb: (s: Snapshot) => void): Promise<Unsubscribe> {
    this.#snapshotListeners.add(cb);
    return () => this.#snapshotListeners.delete(cb);
  }

  emitSchema(schema: Schema): void {
    this.schema = schema;
    this.#schemaListeners.forEach((cb) => cb(schema));
  }

  emitSnapshot(snapshot: Snapshot): void {
    this.#snapshotListeners.forEach((cb) => cb(snapshot));
  }
}
