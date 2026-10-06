import type { Backend, Unsubscribe } from '../backend/backend';
import type { RunStatus, StartRequest, StressSessionSummary, SystemInfo } from '../types';

/** A test is under way from `starting` until it has `finished`. */
export function isRunning(status: RunStatus | null): boolean {
  return status?.state === 'starting' || status?.state === 'running' || status?.state === 'stopping';
}

/**
 * The stress test as the shell last reported it, for the Performance view. It is connected only
 * while the view is on screen: without a test nothing here does periodic work.
 */
class PerformanceStore {
  status = $state.raw<RunStatus | null>(null);
  system = $state.raw<SystemInfo | null>(null);
  /** Saved sessions, newest first. */
  history = $state.raw<StressSessionSummary[]>([]);
  readonly running = $derived(isRunning(this.status));
  #backend: Backend | null = null;
  #starting = false;
  /** Bumped by every `connect` and disconnect, so a superseded connection's late replies are dropped. */
  #generation = 0;

  /**
   * Subscribes to `performance-status` before reading, so a change made in between is not lost; a
   * status read that comes back after an event does not overwrite it. Failure removes the listener.
   */
  async connect(backend: Backend): Promise<Unsubscribe> {
    const generation = ++this.#generation;
    this.#reset();
    this.#backend = backend;
    let eventSeen = false;
    let off: Unsubscribe | null = null;
    const stop = () => {
      off?.();
      off = null;
      if (this.#generation === generation) {
        this.#generation++;
        this.#reset();
      }
    };
    try {
      off = await backend.onPerformanceStatus((next) => {
        if (this.#generation !== generation) return;
        eventSeen = true;
        this.#accept(next);
      });
      const [status, system, history] = await Promise.all([
        backend.performanceStatus(),
        backend.performanceSystem(),
        backend.performanceHistory(),
      ]);
      if (this.#generation === generation) {
        if (!eventSeen) this.status = status;
        this.system = system;
        this.history = history;
      }
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  /** Starts a test: its session id, or null when one is already running or starting. Rejects with the shell's reason. */
  async start(request: StartRequest): Promise<string | null> {
    const backend = this.#backend;
    if (backend === null || this.running || this.#starting) return null;
    this.#starting = true;
    try {
      return await backend.performanceStart(request);
    } finally {
      this.#starting = false;
    }
  }

  async stop(): Promise<void> {
    await this.#backend?.performanceStop();
  }

  async refreshHistory(): Promise<void> {
    const backend = this.#backend;
    if (backend === null) return;
    const generation = this.#generation;
    const history = await backend.performanceHistory();
    if (this.#generation === generation) this.history = history;
  }

  #accept(next: RunStatus) {
    const finishedNow = next.state === 'finished' && this.status?.state !== 'finished';
    this.status = next;
    // The session just saved belongs in the history.
    if (finishedNow) this.refreshHistory().catch((error) => console.error('stress history unavailable', error));
  }

  #reset() {
    this.status = null;
    this.system = null;
    this.history = [];
    this.#backend = null;
    this.#starting = false;
  }
}

export const performanceStore = new PerformanceStore();
