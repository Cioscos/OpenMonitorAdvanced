import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import Sparkline from './Sparkline.svelte';

let restoreClock: (() => void) | null = null;

afterEach(() => {
  cleanup();
  restoreClock?.();
  restoreClock = null;
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function clock() {
  let now = 0;
  let visible = true;
  let reduced = false;
  let id = 0;
  const frames = new Map<number, FrameRequestCallback>();
  const motionListeners = new Set<(event: MediaQueryListEvent) => void>();
  const added = vi.spyOn(document, 'addEventListener');
  const removed = vi.spyOn(document, 'removeEventListener');
  const originalVisibility = Object.getOwnPropertyDescriptor(document, 'visibilityState');
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visible ? 'visible' : 'hidden' });
  restoreClock = () => {
    if (originalVisibility) Object.defineProperty(document, 'visibilityState', originalVisibility);
  };
  vi.spyOn(performance, 'now').mockImplementation(() => now);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    frames.set(++id, callback);
    return id;
  });
  vi.stubGlobal('cancelAnimationFrame', (frameId: number) => frames.delete(frameId));
  vi.stubGlobal('matchMedia', (query: string) => ({
    get matches() { return reduced; },
    media: query,
    addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => motionListeners.add(listener),
    removeEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => motionListeners.delete(listener),
  }));
  return {
    at(ms: number) { now = ms; },
    async frame(ms: number) {
      now = ms;
      const [frameId, callback] = [...frames][0] ?? [];
      if (frameId === undefined || !callback) throw new Error('No pending frame');
      frames.delete(frameId);
      callback(ms);
      await tick();
    },
    visibility(next: boolean, ms: number) {
      now = ms;
      visible = next;
      document.dispatchEvent(new Event('visibilitychange'));
    },
    motion(next: boolean, ms: number) {
      now = ms;
      reduced = next;
      for (const listener of motionListeners) listener({ matches: next } as MediaQueryListEvent);
    },
    pending: () => frames.size,
    motionListeners: () => motionListeners.size,
    visibilityListeners: () =>
      added.mock.calls.filter(([type]) => type === 'visibilitychange').length
      - removed.mock.calls.filter(([type]) => type === 'visibilitychange').length,
  };
}

function lineStart(container: HTMLElement): number {
  const path = container.querySelectorAll('path')[1].getAttribute('d') ?? '';
  return Number(/^M([\d.]+)/.exec(path)?.[1]);
}

test('keeps path bytes while translating the sampled curves between frames', async () => {
  const time = clock();
  const { container } = render(Sparkline, { values: [20, 80], timestampsMs: [0, 1_000], max: 100 });
  const group = container.querySelector('svg g') as SVGGElement;
  const path = group.querySelector('path') as SVGPathElement;
  const initialPath = path.getAttribute('d');
  await time.frame(1_000);
  const firstTransform = group.style.transform;
  await time.frame(2_000);
  expect(path.getAttribute('d')).toBe(initialPath);
  expect(group.style.transform).not.toBe(firstTransform);
  expect(group.style.transform).toMatch(/translateX\(-/);
});

test('keeps a flat held segment connected to a fixed right endpoint', async () => {
  const time = clock();
  const { container } = render(Sparkline, { values: [50], timestampsMs: [1_000], max: 100 });
  await time.frame(1_000);
  const held = container.querySelector('.held-line') as SVGLineElement;
  const marker = container.querySelector('.endpoint') as HTMLElement;
  expect(held).toBeTruthy();
  expect(Number(held.getAttribute('x1'))).toBeCloseTo(149.5, 1);
  expect(held.getAttribute('x2')).toBe('150');
  expect(held.getAttribute('y1')).toBe(held.getAttribute('y2'));
  expect(marker.style.left).toBe('100%');
});

test('hides the held segment and marker for a missing final sample or expired tail', async () => {
  const time = clock();
  const view = render(Sparkline, { values: [50, NaN], timestampsMs: [0, 1_000], max: 100 });
  expect(view.container.querySelector('.held-line')).toBeNull();
  expect(view.container.querySelector('.endpoint')).toBeNull();
  await view.rerender({ values: [50], timestampsMs: [1_000], max: 100 });
  expect(view.container.querySelector('.endpoint')).toBeTruthy();
  await time.frame(301_100);
  expect((view.container.querySelector('.held-line') as SVGLineElement).style.display).toBe('none');
  expect((view.container.querySelector('.endpoint') as HTMLElement).style.display).toBe('none');
});

test('renders one white endpoint over a colored line and translucent glow', () => {
  const { container } = render(Sparkline, {
    values: [0, 50], timestampsMs: [0, 1_000], color: '#2ab0ff', max: 100,
  });
  const paths = container.querySelectorAll('path');
  expect(paths).toHaveLength(2);
  expect(paths[0].getAttribute('stroke')).toBe('#2ab0ff');
  expect(Number(paths[0].getAttribute('opacity'))).toBeLessThan(1);
  expect(paths[1].getAttribute('stroke')).toBe('#2ab0ff');
  const points = container.querySelectorAll('.endpoint');
  expect(points).toHaveLength(1);
  expect(getComputedStyle(points[0]).backgroundColor).toBe('rgb(255, 255, 255)');
});

test('endpoint stays circular and five pixels wide as the tile width changes', () => {
  const { container } = render(Sparkline, { values: [50], timestampsMs: [1_000], max: 100 });
  const chart = container.querySelector('.sparkline') as HTMLElement;
  const marker = container.querySelector('.endpoint') as HTMLElement;
  expect(marker).toBeTruthy();
  const dimensions: string[] = [];
  for (const width of ['150px', '450px']) {
    chart.style.width = width;
    const style = getComputedStyle(marker);
    dimensions.push(`${style.width}x${style.height}`);
    expect(style.borderRadius).toBe('50%');
  }
  expect(dimensions).toEqual(['5pxx5px', '5pxx5px']);
});

test('unmount cancels the shared animation frame', () => {
  const pending = new Set<number>();
  let nextId = 1;
  vi.stubGlobal('requestAnimationFrame', () => {
    const id = nextId++;
    pending.add(id);
    return id;
  });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => pending.delete(id));
  const view = render(Sparkline, { values: [1], timestampsMs: [1_000] });
  expect(pending.size).toBe(1);
  view.unmount();
  expect(pending.size).toBe(0);
});

test('reduced motion keeps the sparkline static without scheduling frames', () => {
  const raf = vi.fn();
  vi.stubGlobal('requestAnimationFrame', raf);
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: true,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  const { container } = render(Sparkline, { values: [1], timestampsMs: [1_000] });
  expect(container.querySelector('.endpoint')).toBeTruthy();
  expect(raf).not.toHaveBeenCalled();
});

test('a delayed snapshot cannot move the already scrolling time edge backward', async () => {
  const time = clock();
  const view = render(Sparkline, { values: [50], timestampsMs: [10_000], max: 100 });
  await time.frame(1_500);
  const before = lineStart(view.container);
  time.at(1_600);
  await view.rerender({ values: [50, 75], timestampsMs: [10_000, 11_000], max: 100 });
  expect(lineStart(view.container)).toBeLessThanOrEqual(before);
  expect(view.container.querySelector('.endpoint')).toBeTruthy();
});

test('a lower timestamp after rollback starts a new time epoch', async () => {
  const time = clock();
  const view = render(Sparkline, { values: [50], timestampsMs: [10_000], max: 100 });
  await time.frame(1_500);
  time.at(1_600);
  await view.rerender({ values: [25], timestampsMs: [5_000], max: 100 });
  expect((view.container.querySelector('.endpoint') as HTMLElement).style.left).toBe('100%');
});

for (const mode of ['visibility', 'motion'] as const) {
  for (const withSnapshot of [false, true]) {
    test(`${mode} pause resumes without elapsed-time jump ${withSnapshot ? 'after a snapshot' : 'without snapshots'}`, async () => {
      const time = clock();
      const view = render(Sparkline, { values: [50], timestampsMs: [10_000], max: 100 });
      await time.frame(500);
      if (mode === 'visibility') time.visibility(false, 1_000);
      else time.motion(true, 1_000);
      expect(time.pending()).toBe(0);
      if (withSnapshot) {
        time.at(600_000);
        await view.rerender({ values: [50, 75], timestampsMs: [10_000, 20_000], max: 100 });
      }
      if (mode === 'visibility') time.visibility(true, 1_200_000);
      else time.motion(false, 1_200_000);
      await time.frame(1_200_000);
      const endpoint = view.container.querySelector('.endpoint') as HTMLElement;
      expect(endpoint).toBeTruthy();
      expect(Number.parseFloat(endpoint.style.left)).toBeGreaterThan(withSnapshot ? 99.9 : 99);
      view.unmount();
      expect(time.pending()).toBe(0);
      expect(time.motionListeners()).toBe(0);
      expect(time.visibilityListeners()).toBe(0);
    });
  }
}
