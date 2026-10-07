import type { Backend, Unsubscribe } from '../backend/backend';
import type { BenchStatus, CpuScoreFile, CpuScoreSummary } from '../types';

/** A benchmark is under way from `starting` until it ends (`done`, `stopped` or `failed`). */
export function isBenchRunning(status: BenchStatus | null): boolean {
  return status?.state === 'starting' || status?.state === 'running' || status?.state === 'stopping';
}

/** A single and a multi core score; either can be missing. */
export interface ScorePair {
  single: number | null;
  multi: number | null;
}

const maxOf = (values: (number | null)[]): number | null => {
  const present = values.filter((v): v is number => v !== null);
  return present.length ? Math.max(...present) : null;
};

/**
 * The CPU benchmark as the shell last reported it, and the saved scores, for the «Score › CPU»
 * page. Connected only while the Performance view is on screen.
 */
class BenchStore {
  status = $state.raw<BenchStatus | null>(null);
  /** Saved scores, newest first. */
  scores = $state.raw<CpuScoreSummary[]>([]);
  /** The score scale is not calibrated yet (DB1). */
  provisional = $state.raw(false);
  readonly running = $derived(isBenchRunning(this.status));
  /** Valid scores on the current scale: once calibrated, provisional ones are not comparable. */
  readonly #comparable = $derived(this.scores.filter((s) => s.valid && (this.provisional || !s.provisional)));
  /** The best single and the best multi core of the comparable scores. */
  readonly record: ScorePair = $derived.by(() => {
    const valid = this.#comparable;
    return { single: maxOf(valid.map((s) => s.single)), multi: maxOf(valid.map((s) => s.multi)) };
  });
  /** The newest comparable score. */
  readonly last: ScorePair = $derived.by(() => {
    const s = this.#comparable[0];
    return { single: s?.single ?? null, multi: s?.multi ?? null };
  });
  #backend: Backend | null = null;
  #generation = 0;

  /**
   * Subscribes to `performance-bench` before reading, so a change made in between is not lost; a
   * status read that comes back after an event does not overwrite it. Failure removes the listener.
   */
  async connect(backend: Backend): Promise<Unsubscribe> {
    const generation = ++this.#generation;
    this.#backend = backend;
    this.status = null;
    let eventSeen = false;
    let off: Unsubscribe | null = null;
    const stop = () => {
      off?.();
      off = null;
      if (this.#generation === generation) {
        this.#generation++;
        this.#backend = null;
        this.status = null;
      }
    };
    try {
      off = await backend.onPerformanceBench((next) => {
        if (this.#generation !== generation) return;
        eventSeen = true;
        this.#accept(next);
      });
      const [status, scores, baseline] = await Promise.all([
        backend.performanceBenchStatus(),
        backend.performanceScores(),
        backend.performanceBaseline(),
      ]);
      if (this.#generation === generation) {
        if (!eventSeen) this.status = status;
        this.scores = scores;
        this.provisional = baseline.provisional;
      }
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  /** Starts the benchmark: the score id; rejects with the shell's reason (`busy` or a text). */
  async start(): Promise<string> {
    if (this.#backend === null) throw 'notConnected';
    return this.#backend.performanceBenchStart();
  }

  async stop(): Promise<void> {
    await this.#backend?.performanceBenchStop();
  }

  async refresh(): Promise<void> {
    const backend = this.#backend;
    if (backend === null) return;
    const generation = this.#generation;
    const scores = await backend.performanceScores();
    if (this.#generation === generation) this.scores = scores;
  }

  /** A saved score in full, or null when it is gone or the store is not connected. */
  async score(id: string): Promise<CpuScoreFile | null> {
    return (await this.#backend?.performanceScore(id)) ?? null;
  }

  async remove(id: string): Promise<void> {
    await this.#backend?.performanceScoreDelete(id);
    await this.refresh();
  }

  #accept(next: BenchStatus) {
    const savedNow = next.state === 'done' && this.status?.state !== 'done';
    this.status = next;
    if (savedNow) this.refresh().catch((error) => console.error('CPU scores unavailable', error));
  }
}

export const benchStore = new BenchStore();
