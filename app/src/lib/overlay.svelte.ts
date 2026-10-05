import type { Backend, Unsubscribe } from './backend/backend';
import type { OverlayStatus } from './types';

/** «Try again» helps only when the frame engine failed or was denied, or the overlay process failed. */
export function retryVisible(status: OverlayStatus | null): boolean {
  if (status === null) return false;
  return status.frames === 'failed' || status.frames === 'denied' || status.process === 'failed';
}

/**
 * The overlay as the shell last reported it. The status has no revision, but the shell stores it
 * before emitting `overlay-status`, so the newest status always arrives as an event: once an event
 * came, the initial read (which may be older) is dropped.
 */
class OverlayStore {
  status = $state.raw<OverlayStatus | null>(null);
  #backend: Backend | null = null;
  /** Bumped by every `connect` and disconnect, so a superseded connection's late callbacks are dropped. */
  #generation = 0;

  /**
   * Subscribes to `overlay-status` before reading the status, so a change made in between is not
   * lost (the read also covers a window reopened later). Failure removes the listener.
   */
  async connect(backend: Backend): Promise<Unsubscribe> {
    const generation = ++this.#generation;
    this.status = null;
    this.#backend = backend;
    let off: Unsubscribe | null = null;
    let eventSeen = false;
    const stop = () => {
      off?.();
      off = null;
      if (this.#generation === generation) {
        this.#generation++;
        this.status = null;
        this.#backend = null;
      }
    };
    try {
      off = await backend.onOverlayStatus((next) => {
        if (this.#generation !== generation) return;
        eventSeen = true;
        this.status = next;
      });
      const current = await backend.getOverlayStatus();
      if (this.#generation === generation && !eventSeen) this.status = current;
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  /** «Try again» on a failed frame engine or overlay process; the outcome arrives as a status. */
  async retry(): Promise<void> {
    await this.#backend?.overlayRetry();
  }

  /** Reads the profile folder again; the new catalog arrives as a status. */
  async reloadProfiles(): Promise<void> {
    await this.#backend?.overlayReloadProfiles();
  }
}

/** The app-wide instance. */
export const overlay = new OverlayStore();
