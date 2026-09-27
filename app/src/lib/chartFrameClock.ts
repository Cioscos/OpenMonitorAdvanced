export type ChartFps = 60 | 30 | 15;

type Subscriber = (monotonicMs: number) => void;

/** Subscribers of one rate share a phase, so they all draw on the same vsyncs. */
type RateGroup = {
  intervalMs: number;
  lastMs: number;
  subscribers: Set<Subscriber>;
};

const groups = new Map<number, RateGroup>();
let frameId: number | null = null;
let motionQuery: MediaQueryList | null = null;
let paused = false;

function isPaused(): boolean {
  return document.visibilityState !== 'visible' || motionQuery?.matches === true;
}

function scheduleFrame(): void {
  if (frameId === null && groups.size > 0 && !isPaused()) {
    frameId = requestAnimationFrame(onFrame);
  }
}

function onFrame(): void {
  frameId = null;
  if (isPaused()) return;

  const now = performance.now();
  // Snapshot first: a callback may subscribe or unsubscribe; newcomers wait for the next tick.
  const due: Array<[RateGroup, Subscriber[]]> = [];
  for (const group of groups.values()) {
    // rAF timestamps can arrive a fraction of a millisecond before the nominal refresh interval.
    if (now - group.lastMs < group.intervalMs - 0.1) continue;
    // Preserve the target phase on high-refresh displays; discard missed periods after a stall.
    group.lastMs = now - group.lastMs >= 2 * group.intervalMs ? now : group.lastMs + group.intervalMs;
    due.push([group, [...group.subscribers]]);
  }
  for (const [group, subscribers] of due) {
    for (const subscriber of subscribers) {
      if (group.subscribers.has(subscriber)) subscriber(now);
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
    for (const group of groups.values()) group.lastMs = -Infinity;
    scheduleFrame();
  }
}

export function subscribeChartFrame(callback: (monotonicMs: number) => void, fps: ChartFps = 60): () => void {
  const intervalMs = 1000 / fps;
  // Wrap the callback so the same function can hold two independent subscriptions.
  const subscriber: Subscriber = (monotonicMs) => callback(monotonicMs);
  const first = groups.size === 0;
  let group = groups.get(intervalMs);
  if (!group) {
    group = { intervalMs, lastMs: -Infinity, subscribers: new Set() };
    groups.set(intervalMs, group);
  }
  // A newcomer joins the rate's existing phase instead of starting its own.
  group.subscribers.add(subscriber);
  if (first) {
    motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    paused = isPaused();
    document.addEventListener('visibilitychange', onEnvironmentChange);
    motionQuery.addEventListener('change', onEnvironmentChange);
  }
  scheduleFrame();

  const owner = group;
  return () => {
    if (!owner.subscribers.delete(subscriber)) return;
    if (owner.subscribers.size === 0 && groups.get(intervalMs) === owner) groups.delete(intervalMs);
    if (groups.size === 0) {
      if (frameId !== null) cancelAnimationFrame(frameId);
      frameId = null;
      document.removeEventListener('visibilitychange', onEnvironmentChange);
      motionQuery?.removeEventListener('change', onEnvironmentChange);
      motionQuery = null;
    }
  };
}
