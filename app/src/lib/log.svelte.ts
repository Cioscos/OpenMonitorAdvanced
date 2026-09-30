import type { Backend, Unsubscribe } from './backend/backend';
import type { LogState, LogStatus } from './types';

/** Which of the tape recorder's keys work in a state; REC also resumes from paused. */
export function canDo(state: LogState): { rec: boolean; pause: boolean; stop: boolean } {
  return {
    rec: state !== 'recording',
    pause: state === 'recording',
    stop: state === 'recording' || state === 'paused',
  };
}

/**
 * The CSV log as the core last reported it. The getter, the `oma:log` events and the replies of
 * the commands all pass through one filter: only a status with a newer `revision` is accepted
 * (the revision is global and never restarts), so a late answer cannot undo a newer state.
 */
class LogStore {
  status = $state.raw<LogStatus | null>(null);
  /** A command is in flight; the tape keys stay disabled until it settles. */
  busy = $state(false);
  #backend: Backend | null = null;
  /** Bumped by every `connect` and disconnect, so a superseded connection's late callbacks are dropped. */
  #generation = 0;

  /**
   * Subscribes to `oma:log` before reading the status, so a change made in between is not
   * lost (the read also covers a window reopened later). Failure removes the listener.
   */
  async connect(backend: Backend): Promise<Unsubscribe> {
    const generation = ++this.#generation;
    this.status = null;
    this.busy = false;
    this.#backend = backend;
    let off: Unsubscribe | null = null;
    const stop = () => {
      off?.();
      off = null;
      if (this.#generation === generation) {
        this.#generation++;
        this.status = null;
        this.busy = false;
        this.#backend = null;
      }
    };
    try {
      off = await backend.onLogStatus((next) => {
        if (this.#generation === generation) this.#accept(next);
      });
      const current = await backend.getLogStatus();
      if (this.#generation === generation) this.#accept(current);
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  start(): Promise<void> {
    return this.#run((b) => b.logStart());
  }

  pause(): Promise<void> {
    return this.#run((b) => b.logPause());
  }

  resume(): Promise<void> {
    return this.#run((b) => b.logResume());
  }

  stop(): Promise<void> {
    return this.#run((b) => b.logStop());
  }

  /** Runs one command; ignored while another is in flight or when not connected. A rejection is rethrown after `busy` is freed. */
  async #run(command: (backend: Backend) => Promise<LogStatus>): Promise<void> {
    const backend = this.#backend;
    if (backend === null || this.busy) return;
    const generation = this.#generation;
    this.busy = true;
    try {
      const reply = await command(backend);
      if (this.#generation === generation) this.#accept(reply);
    } finally {
      if (this.#generation === generation) this.busy = false;
    }
  }

  #accept(next: LogStatus): void {
    if (this.status === null || next.revision > this.status.revision) this.status = next;
  }
}

/** The app-wide instance. */
export const log = new LogStore();
