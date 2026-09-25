import type { Backend } from '../backend/backend';
import type { SensorStats } from '../types';
import type { StatsOf } from './pages';

export const STATS_INTERVAL_MS = 1000;

/**
 * Reads min/max/avg from the core (decision D1) for the sensors of a page, once per
 * interval and only while the document is visible; `reset` clears them in the core.
 */
export class StatsPoller {
  /** Latest statistics by sensor id. */
  byId = $state.raw<ReadonlyMap<string, SensorStats>>(new Map());
  readonly #backend: Backend;
  readonly #ids: () => string[];
  readonly #revision: () => number | null;
  #byRevision = $state<number | null>(null);
  readonly #intervalMs: number;
  #timer: ReturnType<typeof setInterval> | undefined;
  #inFlight = false;
  /** Number of the newest request; older replies are dropped. */
  #latest = 0;

  constructor(backend: Backend, ids: () => string[], revision: () => number | null, intervalMs = STATS_INTERVAL_MS) {
    this.#backend = backend;
    this.#ids = ids;
    this.#revision = revision;
    this.#intervalMs = intervalMs;
  }

  readonly statsOf: StatsOf = (id) =>
    this.#byRevision === this.#revision() ? this.byId.get(id) ?? null : null;

  /** Polls now and then every interval; returns the stop function. */
  start(): () => void {
    if (this.#timer === undefined) {
      this.#timer = setInterval(() => {
        if (!this.#inFlight) void this.poll();
      }, this.#intervalMs);
      document.addEventListener('visibilitychange', this.#onVisibility);
      void this.poll();
    }
    return () => this.stop();
  }

  stop(): void {
    clearInterval(this.#timer);
    this.#timer = undefined;
    document.removeEventListener('visibilitychange', this.#onVisibility);
  }

  readonly #onVisibility = () => {
    if (document.visibilityState === 'visible') void this.poll();
  };

  async poll(): Promise<void> {
    if (document.visibilityState === 'hidden') return;
    const ids = this.#ids();
    const revision = this.#revision();
    const request = ++this.#latest;
    if (ids.length === 0) {
      this.#inFlight = false;
      this.byId = new Map();
      return;
    }
    this.#inFlight = true;
    try {
      const reply = await this.#backend.getStats(ids);
      if (request !== this.#latest || revision !== this.#revision() || reply.revision !== revision) return;
      const next = new Map<string, SensorStats>();
      ids.forEach((id, i) => {
        const stats = reply.stats[i];
        if (stats) next.set(id, stats);
      });
      this.byId = next;
      this.#byRevision = revision;
    } catch (error) {
      console.error('sensor statistics unavailable', error);
    } finally {
      if (request === this.#latest) this.#inFlight = false;
    }
  }

  /** Clears min/max/avg of every sensor of the page in the core, then reads them again. */
  async reset(): Promise<void> {
    const ids = this.#ids();
    try {
      await this.#backend.resetStats(ids);
    } catch (error) {
      console.error('cannot reset the sensor statistics', error);
      return;
    }
    const cleared = new Map(this.byId);
    for (const id of ids) cleared.delete(id);
    this.byId = cleared;
    await this.poll();
  }
}
