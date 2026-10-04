import type { Backend, Unsubscribe } from './backend/backend';
import type { UpdateStatus } from './types';

/**
 * The update check as the shell last reported it. The `oma:update-status` events and the replies of
 * `check_updates` feed one state; the badge on Settings › About reads it even when the page is closed.
 */
class UpdatesStore {
  state = $state.raw<UpdateStatus | null>(null);
  #backend: Backend | null = null;
  /** Bumped by every `connect` and disconnect, so a superseded connection's late callbacks are dropped. */
  #generation = 0;
  /** Bumped by every status event, so a check reply older than an event is dropped. */
  #events = 0;

  /** Subscribes before reading, so a change in between is not lost. */
  async connect(backend: Backend): Promise<Unsubscribe> {
    const generation = ++this.#generation;
    this.state = null;
    this.#backend = backend;
    let off: Unsubscribe | null = null;
    const stop = () => {
      off?.();
      off = null;
      if (this.#generation === generation) {
        this.#generation++;
        this.state = null;
        this.#backend = null;
      }
    };
    try {
      off = await backend.onUpdateStatus((next) => {
        if (this.#generation === generation) {
          this.#events++;
          this.state = next;
        }
      });
      const current = await backend.getUpdateStatus();
      // An event that arrived during the read is newer than the read.
      if (this.#generation === generation && this.state === null) this.state = current;
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  /**
   * Runs a check now. The reply is dropped when a status event arrived meanwhile: the shell emits the
   * result of every check, so the event is at least as new. A rejection (the background task panicked)
   * keeps what was known and reports an invalid response.
   */
  async check(): Promise<void> {
    const backend = this.#backend;
    if (backend === null || this.state?.state === 'checking') return;
    const generation = this.#generation;
    const events = this.#events;
    try {
      const reply = await backend.checkUpdates();
      if (this.#generation === generation && this.#events === events) this.state = reply;
    } catch (error) {
      console.error('update check failed', error);
      if (this.#generation !== generation) return;
      const previous = this.state;
      this.state = {
        state: 'error',
        current: previous?.current ?? '',
        latest: previous?.latest ?? null,
        checkedAtMs: previous?.checkedAtMs ?? null,
        error: 'invalid',
      };
    }
  }
}

/** The app-wide instance. */
export const updates = new UpdatesStore();
