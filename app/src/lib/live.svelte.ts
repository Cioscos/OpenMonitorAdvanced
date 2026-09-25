import type { Backend, Unsubscribe } from './backend/backend';
import { SeriesBuffer } from './series';
import type { HistorySeed, Schema, Snapshot } from './types';

/** Five minutes at the default 1 s interval. */
export const SPARKLINE_POINTS = 300;

/** Latest values plus a short in-UI history per sensor, for sparklines. */
export class LiveStore {
  readonly capacity: number;
  schema = $state.raw<Schema | null>(null);
  values = $state.raw<(number | null)[]>([]);
  timestampMs = $state(0);
  firstTimestampMs = $state(0);
  /** Local clock (Date.now()) when the last new snapshot arrived; drives the stale badge. */
  lastReceivedAtMs = $state<number | null>(null);
  /** Bumped whenever series change, so readers of `series()` re-run. */
  #tick = $state(0);
  #lastSeq = -1;
  #index = new Map<string, number>();
  #series = new Map<string, SeriesBuffer>();

  constructor(capacity = SPARKLINE_POINTS) {
    this.capacity = capacity;
  }

  applySchema(schema: Schema): void {
    if (this.schema && schema.revision <= this.schema.revision) return;
    this.schema = schema;
    this.#index = new Map(schema.sensors.map((s, i) => [s.id, i]));
    for (const id of [...this.#series.keys()]) {
      if (!this.#index.has(id)) this.#series.delete(id);
    }
    for (const s of schema.sensors) {
      if (!this.#series.has(s.id)) {
        const buffer = new SeriesBuffer(this.capacity);
        const previousLength = Math.max(0, ...[...this.#series.values()].map((b) => b.length));
        for (let i = 0; i < previousLength; i++) buffer.push(null);
        this.#series.set(s.id, buffer);
      }
    }
    this.values = schema.sensors.map(() => null);
    this.#tick++;
  }

  /** Returns false when the snapshot belongs to another schema revision. */
  applySnapshot(snapshot: Snapshot): boolean {
    const schema = this.schema;
    if (!schema || snapshot.revision !== schema.revision || snapshot.values.length !== schema.sensors.length) {
      return false;
    }
    if (snapshot.seq <= this.#lastSeq) return true; // Duplicate/out-of-order event.
    if (snapshot.timestampMs < this.timestampMs) {
      for (const buffer of this.#series.values()) buffer.clear();
      this.firstTimestampMs = snapshot.timestampMs;
    }
    this.#lastSeq = snapshot.seq;
    this.lastReceivedAtMs = Date.now();
    this.values = snapshot.values;
    schema.sensors.forEach((s, i) => this.#series.get(s.id)?.push(snapshot.values[i]));
    this.timestampMs = snapshot.timestampMs;
    if (this.firstTimestampMs === 0) this.firstTimestampMs = snapshot.timestampMs;
    this.#tick++;
    return true;
  }

  /** Replaces sparkline buffers with history fetched from the core. */
  seedHistory(ids: string[], history: HistorySeed): void {
    if (history.revision !== this.schema?.revision) return;
    this.#lastSeq = history.seq;
    ids.forEach((id, k) => {
      const buffer = this.#series.get(id);
      if (!buffer) return;
      buffer.clear();
      for (const v of history.series[k] ?? []) buffer.push(v);
    });
    const last = history.timestampsMs.at(-1);
    if (last !== undefined) {
      this.timestampMs = last;
      this.values = ids.map((_, i) => history.series[i]?.at(-1) ?? null);
    }
    const first = history.timestampsMs[0];
    if (first !== undefined && (this.firstTimestampMs === 0 || first < this.firstTimestampMs)) {
      this.firstTimestampMs = first;
    }
    this.#tick++;
  }

  value(id: string): number | null {
    const i = this.#index.get(id);
    return i === undefined ? null : (this.values[i] ?? null);
  }

  series(id: string): number[] {
    void this.#tick;
    return this.#series.get(id)?.toArray() ?? [];
  }
}

/** Wires a store to a backend: seeds history, follows schema and snapshot events. */
export async function connect(store: LiveStore, backend: Backend): Promise<Unsubscribe> {
  let stopped = false;
  let refreshing: Promise<void> | null = null;
  let initializing = true;
  let requestedRevision = 0;
  let queue: Snapshot[] = [];
  const off: Unsubscribe[] = [];
  const stop = () => { stopped = true; off.splice(0).forEach((fn) => fn()); queue = []; };
  const refresh = (): Promise<void> => {
    refreshing ??= (async () => {
      do {
        const schema = await backend.getSchema();
        const ids = schema.sensors.map((s) => s.id);
        const history = await backend.getHistory(ids, store.capacity);
        if (stopped) return;
        // Hardware may change between the two commands; never seed a different schema.
        if (history.revision !== schema.revision || schema.revision < requestedRevision) continue;
        store.applySchema(schema);
        store.seedHistory(ids, history);
        for (const snapshot of queue.sort((a, b) => a.seq - b.seq)) {
          if (snapshot.revision === schema.revision && snapshot.seq > history.seq) store.applySnapshot(snapshot);
        }
        queue = [];
        initializing = false;
        return;
      } while (!stopped);
    })().finally(() => { refreshing = null; });
    return refreshing;
  };
  // After startup, a failed refresh must not tear the connection down (spec §8: degrade,
  // don't freeze): log it and keep the subscriptions so the next schema/snapshot event
  // that needs a refresh (`refreshing` is reset by the `finally` above) retries it.
  const recover = () => { void refresh().catch((error) => { console.error('backend refresh failed', error); }); };
  try {
    off.push(await backend.onSchema((schema) => {
      if (stopped || schema.revision <= (store.schema?.revision ?? 0)) return;
      requestedRevision = Math.max(requestedRevision, schema.revision);
      if (!initializing) recover();
    }));
    off.push(await backend.onSnapshot((snapshot) => {
      if (stopped || snapshot.revision < (store.schema?.revision ?? 0)) return;
      requestedRevision = Math.max(requestedRevision, snapshot.revision);
      if (initializing || refreshing || snapshot.revision !== store.schema?.revision) {
        queue.push(snapshot);
        if (queue.length > store.capacity) queue.shift();
        if (!initializing) recover();
      } else if (!store.applySnapshot(snapshot)) recover();
    }));
    await refresh();
    return stop;
  } catch (error) {
    stop();
    throw error;
  }
}
