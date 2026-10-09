import type { Backend, Unsubscribe } from '../backend/backend';
import { boardStore } from './board.svelte';
import type { BenchMode, BenchStatus, DiskBenchRequest, ScoreFile, ScoreSummary } from '../types';

/** A benchmark is under way from `starting` until it ends (`done`, `stopped` or `failed`). */
export function isBenchRunning(status: BenchStatus | null): boolean {
  return status?.state === 'starting' || status?.state === 'running' || status?.state === 'stopping';
}

/**
 * The CPU benchmark, the one of a GPU by its schema device id (M8b2 DH12), or the disk one (M8c):
 * its status and history cover every disk, while the record and the last measurement are those of
 * `deviceId` when it is given (DC15).
 */
export type ScoreTarget = { category: 'cpu' } | { category: 'gpu'; deviceId: string } | { category: 'disk'; deviceId?: string | null };

/** The scores of each mode: `single`/`multi` for a CPU, `compute`/`graphics` for a GPU, `read`/`write` (MB/s) for a disk; any can be missing. */
export type ScoreSet = Record<BenchMode, number | null>;

const MODES: BenchMode[] = ['single', 'multi', 'compute', 'graphics', 'read', 'write'];

/** The value of `mode` in a status or a score summary: the disk's `read` and `write` are `readMBs` and `writeMBs`. */
export function modeValue(s: BenchStatus | ScoreSummary, mode: BenchMode): number | null {
  return mode === 'read' ? s.readMBs : mode === 'write' ? s.writeMBs : s[mode];
}

/** Whether a score or a status belongs to `target`. */
const owns = (target: ScoreTarget, s: { category: string; deviceId: string | null }) =>
  s.category === target.category && (target.category !== 'gpu' || s.deviceId === target.deviceId);

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
  /** The CPU, GPU and disk score scales are not calibrated yet (DB1, DH1). */
  #provisional = $state.raw({ cpu: false, gpu: false, disk: false });
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
    return Object.fromEntries(MODES.map((m) => [m, maxOf(valid.map((s) => modeValue(s, m)))])) as ScoreSet;
  }

  /** The newest comparable score of `target`. */
  lastFor(target: ScoreTarget): ScoreSet {
    const s = this.#comparable(target)[0];
    return Object.fromEntries(MODES.map((m) => [m, s ? modeValue(s, m) : null])) as ScoreSet;
  }

  /** The disk points (comparable B1 measurements of the same disk): the best and the newest. */
  pointsFor(target: ScoreTarget): { record: number | null; last: number | null } {
    const points = this.#comparable(target)
      .filter((s) => s.diskProfile !== 'b2' && s.points !== null)
      .map((s) => s.points);
    return { record: maxOf(points), last: points[0] ?? null };
  }

  /** Valid scores on the current scale: once calibrated, provisional ones are not comparable. */
  #comparable(target: ScoreTarget): ScoreSummary[] {
    const provisional = this.provisionalFor(target);
    // A disk's record is its own; `undefined` compares every disk, `null` (not one recognised disk) none.
    const diskId = target.category === 'disk' ? target.deviceId : undefined;
    if (diskId === null) return [];
    return this.scoresFor(target).filter(
      (s) => s.valid && (provisional || !s.provisional) && (diskId === undefined || s.deviceId === diskId),
    );
  }
  /**
   * Subscribes to `performance-bench` before reading, so a change made in between is not lost; a
   * status read that comes back after an event does not overwrite it. Failure removes the listener.
   */
  async connect(backend: Backend): Promise<Unsubscribe> {
    const generation = ++this.#generation;
    this.#backend = backend;
    this.status = null;
    // Stale scores would let a page ask for a detail before the backend is set, and keep the miss.
    this.scores = [];
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
        this.#provisional = { cpu: baseline.provisional, gpu: baseline.gpuProvisional, disk: baseline.diskProvisional };
      }
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  /**
   * Starts the benchmark of `target`: the score id; rejects with the shell's reason (`busy`,
   * `build:no_gpu`, `disk:standby`… or a text). A disk benchmark needs its `disk` request.
   */
  async start(target: ScoreTarget, disk?: DiskBenchRequest): Promise<string> {
    if (this.#backend === null) throw 'notConnected';
    if (target.category === 'disk') {
      if (!disk) throw 'noTarget';
      return this.#backend.performanceDiskBenchStart(disk);
    }
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
    if (savedNow) {
      this.refresh().catch((error) => console.error('scores unavailable', error));
      // A finished benchmark may be due a table download (the store ignores it when not connected).
      void boardStore.refresh(false);
    }
  }
}

export const benchStore = new BenchStore();
