/** Half of the recording dot's 1 Hz period: 500 ms lit, 500 ms dark. */
const HALF_PERIOD_MS = 500;
const REDUCED_MOTION = '(prefers-reduced-motion: reduce)';

/**
 * The recording dot's blink (spec M5 §4.4): one UI timer that flips a flag every 500 ms, so the
 * page repaints twice a second instead of running a 60 FPS animation. The timer runs only while
 * active and the document is visible; with reduced motion the dot stays lit. `on` is `true`
 * whenever the timer is not running, so a paused, hidden or reduced-motion dot is always lit.
 */
export function createBlink(options: { reducedMotion?: () => boolean } = {}): {
  readonly on: boolean;
  setActive(active: boolean): void;
  destroy(): void;
} {
  // The default follows the system setting live; a custom check is read on every change and tick.
  const media = options.reducedMotion === undefined ? window.matchMedia(REDUCED_MOTION) : null;
  const reducedMotion = options.reducedMotion ?? (() => media!.matches);
  let on = $state(true);
  let active = false;
  let timer: ReturnType<typeof setInterval> | undefined;

  const stop = () => {
    if (timer !== undefined) clearInterval(timer);
    timer = undefined;
    on = true;
  };
  const sync = () => {
    const run = active && !document.hidden && !reducedMotion();
    if (!run) stop();
    else if (timer === undefined) {
      on = true;
      timer = setInterval(() => {
        if (reducedMotion()) stop();
        else on = !on;
      }, HALF_PERIOD_MS);
    }
  };

  document.addEventListener('visibilitychange', sync);
  media?.addEventListener('change', sync);
  return {
    get on() {
      return on;
    },
    setActive(next: boolean) {
      active = next;
      sync();
    },
    destroy() {
      active = false;
      stop();
      document.removeEventListener('visibilitychange', sync);
      media?.removeEventListener('change', sync);
    },
  };
}
