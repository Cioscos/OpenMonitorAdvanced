export type ChartFps = 60 | 30 | 15;

type Subscriber = {
  callback: (monotonicMs: number) => void;
  intervalMs: number;
  lastMs: number;
};

const subscribers = new Set<Subscriber>();
let frameId: number | null = null;
let motionQuery: MediaQueryList | null = null;
let paused = false;

function isPaused(): boolean {
  return document.visibilityState !== 'visible' || motionQuery?.matches === true;
}

function scheduleFrame(): void {
  if (frameId === null && subscribers.size > 0 && !isPaused()) {
    frameId = requestAnimationFrame(onFrame);
  }
}

function onFrame(): void {
  frameId = null;
  if (isPaused()) return;

  const now = performance.now();
  for (const subscriber of subscribers) {
    // rAF timestamps can arrive a fraction of a millisecond before the nominal refresh interval.
    if (now - subscriber.lastMs >= subscriber.intervalMs - 0.1) {
      // Preserve the target phase on high-refresh displays; discard missed periods after a stall.
      subscriber.lastMs = now - subscriber.lastMs >= 2 * subscriber.intervalMs
        ? now
        : subscriber.lastMs + subscriber.intervalMs;
      subscriber.callback(now);
    }
  }
  scheduleFrame();
}

function onEnvironmentChange(): void {
  const nextPaused = isPaused();
  if (nextPaused === paused) return;
  paused = nextPaused;
  if (paused) {
    if (frameId !== null) {
      cancelAnimationFrame(frameId);
      frameId = null;
    }
  } else {
    for (const subscriber of subscribers) subscriber.lastMs = -Infinity;
    scheduleFrame();
  }
}

export function subscribeChartFrame(callback: (monotonicMs: number) => void, fps: ChartFps = 60): () => void {
  const subscriber: Subscriber = { callback, intervalMs: 1000 / fps, lastMs: -Infinity };
  subscribers.add(subscriber);
  if (subscribers.size === 1) {
    motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    paused = isPaused();
    document.addEventListener('visibilitychange', onEnvironmentChange);
    motionQuery.addEventListener('change', onEnvironmentChange);
  }
  scheduleFrame();

  return () => {
    if (!subscribers.delete(subscriber)) return;
    if (subscribers.size === 0) {
      if (frameId !== null) cancelAnimationFrame(frameId);
      frameId = null;
      document.removeEventListener('visibilitychange', onEnvironmentChange);
      motionQuery?.removeEventListener('change', onEnvironmentChange);
      motionQuery = null;
    }
  };
}
