import { subscribeChartFrame, type ChartFps } from './chartFrameClock';

let now = 0;
let nextFrameId = 1;
let reducedMotion = false;
let visibility: DocumentVisibilityState = 'visible';
const frames = new Map<number, FrameRequestCallback>();
const motionListeners = new Set<(event: MediaQueryListEvent) => void>();
const cleanups: Array<() => void> = [];
let requested = 0;
let cancelled = 0;
let originalVisibility: PropertyDescriptor | undefined;

function frame(at: number): void {
  now = at;
  const [id, callback] = [...frames][0] ?? [];
  if (id === undefined || !callback) throw new Error('No animation frame pending');
  frames.delete(id);
  callback(at);
}

function subscribe(callback: (ms: number) => void, fps?: ChartFps): () => void {
  const stop = subscribeChartFrame(callback, fps);
  cleanups.push(stop);
  return stop;
}

function setVisibility(state: DocumentVisibilityState): void {
  visibility = state;
  document.dispatchEvent(new Event('visibilitychange'));
}

function setReducedMotion(matches: boolean): void {
  reducedMotion = matches;
  const event = { matches, media: '(prefers-reduced-motion: reduce)' } as MediaQueryListEvent;
  for (const listener of motionListeners) listener(event);
}

beforeEach(() => {
  now = 0;
  nextFrameId = 1;
  requested = 0;
  cancelled = 0;
  reducedMotion = false;
  visibility = 'visible';
  frames.clear();
  motionListeners.clear();
  cleanups.length = 0;
  originalVisibility = Object.getOwnPropertyDescriptor(document, 'visibilityState');
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
  vi.spyOn(performance, 'now').mockImplementation(() => now);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = nextFrameId++;
    frames.set(id, callback);
    requested++;
    return id;
  });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => {
    if (frames.delete(id)) cancelled++;
  });
  vi.stubGlobal('matchMedia', (query: string) => ({
    get matches() { return reducedMotion; },
    media: query,
    addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => motionListeners.add(listener),
    removeEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => motionListeners.delete(listener),
  }));
});

afterEach(() => {
  for (const stop of cleanups) stop();
  if (originalVisibility) Object.defineProperty(document, 'visibilityState', originalVisibility);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

test('subscribers share one animation frame loop', () => {
  const first = vi.fn();
  const second = vi.fn();
  subscribe(first);
  subscribe(second);
  expect(requested).toBe(1);
  expect(frames.size).toBe(1);

  frame(100);
  expect(first).toHaveBeenCalledWith(100);
  expect(second).toHaveBeenCalledWith(100);
  expect(requested).toBe(2);
  expect(frames.size).toBe(1);
});

test('60, 30, and 15 FPS subscribers use elapsed monotonic time', () => {
  const calls = [vi.fn(), vi.fn(), vi.fn()];
  subscribe(calls[0], 60);
  subscribe(calls[1], 30);
  subscribe(calls[2], 15);
  for (const at of [0, 16, 17, 32, 34, 50, 68, 85]) frame(at);
  expect(calls[0].mock.calls.map(([ms]) => ms)).toEqual([0, 17, 34, 50, 68, 85]);
  expect(calls[1].mock.calls.map(([ms]) => ms)).toEqual([0, 34, 68]);
  expect(calls[2].mock.calls.map(([ms]) => ms)).toEqual([0, 68]);
});

test('60 Hz display timing stays smooth despite sub-millisecond jitter', () => {
  const calls = vi.fn();
  subscribe(calls, 60);
  for (const at of [0, 16.66, 33.32, 49.98]) frame(at);
  expect(calls.mock.calls.map(([ms]) => ms)).toEqual([0, 16.66, 33.32, 49.98]);
});

test('164 Hz rAF keeps 60, 30, and 15 FPS near their target rates over one second', () => {
  const calls = [vi.fn(), vi.fn(), vi.fn()];
  subscribe(calls[0], 60);
  subscribe(calls[1], 30);
  subscribe(calls[2], 15);
  for (let i = 0; i <= 164; i++) frame(i * 1000 / 164);
  expect(calls[0].mock.calls.length).toBeGreaterThanOrEqual(59);
  expect(calls[0].mock.calls.length).toBeLessThanOrEqual(61);
  expect(calls[1].mock.calls.length).toBeGreaterThanOrEqual(29);
  expect(calls[1].mock.calls.length).toBeLessThanOrEqual(31);
  expect(calls[2].mock.calls.length).toBeGreaterThanOrEqual(14);
  expect(calls[2].mock.calls.length).toBeLessThanOrEqual(16);
  expect(requested).toBe(166);
});

test('a long rAF stall delivers one current frame and resets cadence', () => {
  const calls = vi.fn();
  subscribe(calls, 60);
  for (const at of [0, 17, 1_000, 1_006, 1_012, 1_018]) frame(at);
  expect(calls.mock.calls.map(([ms]) => ms)).toEqual([0, 17, 1_000, 1_018]);
});

test('resume delivers one current frame without burst on subsequent rAF ticks', () => {
  const calls = vi.fn();
  subscribe(calls, 60);
  frame(0);
  setVisibility('hidden');
  setVisibility('visible');
  for (const at of [1_000, 1_006, 1_012, 1_018]) frame(at);
  expect(calls.mock.calls.map(([ms]) => ms)).toEqual([0, 1_000, 1_018]);
});

test('an unchanged visibility event does not bypass the FPS cap', () => {
  const calls = vi.fn();
  subscribe(calls, 30);
  frame(0);
  setVisibility('visible');
  frame(10);
  expect(calls.mock.calls.map(([ms]) => ms)).toEqual([0]);
});

test('the last unsubscribe cancels the frame and removes listeners', () => {
  const stopFirst = subscribe(vi.fn());
  const stopSecond = subscribe(vi.fn());
  expect(motionListeners.size).toBe(1);
  stopFirst();
  expect(frames.size).toBe(1);
  stopSecond();
  stopSecond();
  expect(frames.size).toBe(0);
  expect(cancelled).toBe(1);
  expect(motionListeners.size).toBe(0);
  setVisibility('hidden');
  setVisibility('visible');
  expect(frames.size).toBe(0);
});

test('visibility and reduced motion stop and resume the frame loop without replay', () => {
  const calls = vi.fn();
  subscribe(calls, 30);
  frame(0);
  setVisibility('hidden');
  expect(frames.size).toBe(0);
  now = 2_000;
  setVisibility('visible');
  expect(frames.size).toBe(1);
  frame(2_000);
  expect(calls.mock.calls.map(([ms]) => ms)).toEqual([0, 2_000]);

  setReducedMotion(true);
  expect(frames.size).toBe(0);
  now = 4_000;
  setReducedMotion(false);
  expect(frames.size).toBe(1);
  frame(4_000);
  expect(calls.mock.calls.map(([ms]) => ms)).toEqual([0, 2_000, 4_000]);
});
