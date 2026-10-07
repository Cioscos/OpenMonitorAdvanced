import type { Backend, Unsubscribe } from '../backend/backend';
import type { BenchMode, BenchStatus, ScoreFile, ScoreSummary } from '../types';

/** A benchmark is under way from `starting` until it ends (`done`, `stopped` or `failed`). */
export function isBenchRunning(status: BenchStatus | null): boolean {
  return status?.state === 'starting' || status?.state === 'running' || status?.state === 'stopping';
}

/** The CPU benchmark, or the one of a GPU by its schema device id (M8b2 DH12). */
export type ScoreTarget = { category: 'cpu' } | { category: 'gpu'; deviceId: string };

/** The scores of each mode: `single`/`multi` for a CPU, `compute`/`graphics` for a GPU; any can be missing. */
export type ScoreSet = Record<BenchMode, number | null>;

const MODES: BenchMode[] = ['single', 'multi', 'compute', 'graphics'];

/** Whether a score or a status belongs to `target`. */
const owns = (target: ScoreTarget, s: { category: string; deviceId: string | null }) =>
  s.category === target.category && (target.category === 'cpu' || s.deviceId === target.deviceId);

const maxOf = (values: (number | null)[]): number | null => {
  const present = values.filter((v): v is number => v !== null);
  return present.length ? Math.max(...present) : null;
};

/**
 * The benchmark (CPU or GPU, one at a time) as the shell last reported it, and the saved scores,
 * for the «Score» pages; the `…For(target)` reads keep each page to its own. Connected only while
 * the Performance view is on screen.
 */
class BenchStore {
  status = $state.raw<BenchStatus | null>(null);
  /** Saved scores of every target, newest first. */
  scores = $state.raw<ScoreSummary[]>([]);
  /** The CPU and the GPU score scales are not calibrated yet (DB1, DH1). */
  #provisional = $state.raw({ cpu: false, gpu: false });
  /** Any benchmark under way: one at a time for the whole app (DB8). */
  readonly running = $derived(isBenchRunning(this.status));
  #backend: Backend | null = null;
  #generation = 0;

  /** The status, when it is the benchmark of `target`. */
  statusFor(target: ScoreTarget): BenchStatus | null {
    return this.status && owns(target, this.status) ? this.status : null;
  }

  scoresFor(target: ScoreTarget): ScoreSummary[] {
    return this.scores.filter((s) => owns(target, s));
  }

  provisionalFor(target: ScoreTarget): boolean {
    return this.#provisional[target.category];
  }

  /** The best score of each mode among the comparable ones of `target`. */
  recordFor(target: ScoreTarget): ScoreSet {
    const valid = this.#comparable(target);
    return Object.fromEntries(MODES.map((m) => [m, maxOf(valid.map((s) => s[m]))])) as ScoreSet;
  }

  /** The newest comparable score of `target`. */
  lastFor(target: ScoreTarget): ScoreSet {
    const s = this.#comparable(target)[0];
    return Object.fromEntries(MODES.map((m) => [m, s?.[m] ?? null])) as ScoreSet;
  }

  /** Valid scores on the current scale: once calibrated, provisional ones are not comparable. */
  #comparable(target: ScoreTarget): ScoreSummary[] {
    const provisional = this.provisionalFor(target);
    return this.scoresFor(target).filter((s) => s.valid && (provisional || !s.provisional));
  }
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
        this.#provisional = { cpu: baseline.provisional, gpu: baseline.gpuProvisional };
      }
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  /** Starts the benchmark of `target`: the score id; rejects with the shell's reason (`busy`, `build:no_gpu` or a text). */
  async start(target: ScoreTarget): Promise<string> {
    if (this.#backend === null) throw 'notConnected';
    return target.category === 'gpu' ? this.#backend.performanceGpuBenchStart(target.deviceId) : this.#backend.performanceBenchStart();
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
  async score(id: string): Promise<ScoreFile | null> {
    return (await this.#backend?.performanceScore(id)) ?? null;
  }

  async remove(id: string): Promise<void> {
    await this.#backend?.performanceScoreDelete(id);
    await this.refresh();
  }

  #accept(next: BenchStatus) {
    const savedNow = next.state === 'done' && this.status?.state !== 'done';
    this.status = next;
    if (savedNow) this.refresh().catch((error) => console.error('scores unavailable', error));
  }
}

export const benchStore = new BenchStore();
