import type { LogStatus } from '../types';

/**
 * The tape counter of the CSV log. The core reports `recordedMs` at most once a second on its
 * tick clock (spec M5 §4.4), and the ticks are about a second apart with some jitter, so a
 * status comes one or two seconds after the previous one and its value sits just below or above
 * a whole second: shown as reported, the seconds skip or stall. While recording, the counter
 * therefore runs on locally from the last status whose recorded time advanced, and every status
 * resyncs it. It never goes back while a session is active and never runs more than `maxGapMs`
 * (the L3 gap threshold) past the last status, so a stalled core cannot be outrun for long. It
 * waits for the core's recorded time to advance before running, because the tick clock counts
 * nothing before the first tick after a start or a resume. Paused, it stays where it was; idle
 * and error show the core's final value. One timer, set to the next second of the counter, runs
 * only while the counter runs and the document is visible.
 */
export function createTapeClock(options: { now?: () => number; maxGapMs?: () => number } = {}): {
  /** The recorded time to show, in ms. */
  readonly recordedMs: number;
  /** Takes the log's latest status (`log.status`). */
  update(status: LogStatus | null): void;
  destroy(): void;
} {
  const now = options.now ?? (() => performance.now());
  const maxGapMs = options.maxGapMs ?? (() => 5_000);
  let shown = $state(0);
  /** `shown` without its dependency, for reads inside the caller's effect. */
  let current = 0;
  let session: number | null = null;
  /** The highest recorded time the core reported in `session`. */
  let reported = 0;
  /** Set while the counter runs: its last resync and when it happened. */
  let origin: { recordedMs: number; atMs: number } | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const set = (ms: number) => {
    current = ms;
    shown = ms;
  };
  /** Where the running counter is now, capped at one gap past its origin. */
  const running = () => (origin ? origin.recordedMs + Math.min(Math.max(0, now() - origin.atMs), maxGapMs()) : current);
  const stop = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  };
  const tick = () => {
    stop();
    if (!origin) return;
    set(Math.max(current, running()));
    const next = (Math.floor(current / 1000) + 1) * 1000;
    const cap = origin.recordedMs + maxGapMs();
    if (document.hidden || next > cap) return;
    timer = setTimeout(tick, Math.max(1, next - running()));
  };

  const update = (status: LogStatus | null) => {
    if (status === null) {
      origin = null;
      session = null;
      reported = 0;
      stop();
      set(0);
      return;
    }
    const sameSession = status.session === session;
    if (status.state === 'recording') {
      if (!sameSession) {
        session = status.session;
        reported = status.recordedMs;
        origin = null;
        set(status.recordedMs);
      } else if (status.recordedMs > reported) {
        reported = status.recordedMs;
        origin = { recordedMs: status.recordedMs, atMs: now() };
      }
      tick();
      return;
    }
    // Paused keeps what was shown; idle and error show the core's final value.
    const shownNow = Math.max(current, running());
    origin = null;
    stop();
    set(status.state === 'paused' && sameSession ? Math.max(shownNow, status.recordedMs) : status.recordedMs);
    session = status.session;
    reported = status.recordedMs;
  };

  document.addEventListener('visibilitychange', tick);
  return {
    get recordedMs() {
      return shown;
    },
    update,
    destroy() {
      stop();
      origin = null;
      document.removeEventListener('visibilitychange', tick);
    },
  };
}
