import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import Sparkline from './Sparkline.svelte';
import source from './Sparkline.svelte?raw';

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

/** The HTML layer that the compositor translates between samples. */
function scroller(container: HTMLElement): HTMLElement {
  return container.querySelector('.sparkline-scroll') as HTMLElement;
}

function translateCssPx(container: HTMLElement): number {
  return Number(/translateX\((-?\d+(?:\.\d+)?)px\)/.exec(scroller(container).style.transform)?.[1] ?? 0);
}

/** Rendered CSS x of the path end and of the held segment, for a sparkline `cssWidth` wide. */
function connectionPositions(container: HTMLElement, cssWidth = 300): { pathX: number; heldX: number; heldEndX: number } {
  const svg = container.querySelector('svg') as SVGSVGElement;
  const path = svg.querySelector('path') as SVGPathElement;
  const held = svg.querySelector('.held-line') as SVGLineElement;
  const numbers = (path.getAttribute('d') ?? '').match(/-?\d+(?:\.\d+)?/g) ?? [];
  const endpointX = Number(numbers.at(-2));
  const viewBoxWidth = Number(svg.getAttribute('viewBox')!.split(' ')[2]);
  // The SVG spans twice the tile, so one user unit is (2 * cssWidth / viewBoxWidth) CSS px.
  const scale = 2 * cssWidth / viewBoxWidth;
  const translate = translateCssPx(container);
  return {
    pathX: endpointX * scale + translate,
    heldX: Number(held.getAttribute('x1')) * scale + translate,
    heldEndX: Number(held.getAttribute('x2')) * scale + translate,
  };
}

function renderAtDoubleWidth(values: number[], timestampsMs: number[]) {
  vi.spyOn(HTMLDivElement.prototype, 'getBoundingClientRect').mockReturnValue({ width: 300 } as DOMRect);
  return render(Sparkline, { values, timestampsMs, max: 100 });
}

test('keeps the sampled path connected to the held line at double CSS width', async () => {
  const time = clock();
  const view = renderAtDoubleWidth([50], [1_000]);
  await time.frame(1_000);
  const { pathX, heldX, heldEndX } = connectionPositions(view.container);
  expect(pathX).toBeCloseTo(299, 4);
  expect(heldX).toBeCloseTo(299, 4);
  expect(heldEndX).toBeGreaterThanOrEqual(300);
});

test('keeps the connection at double width when a delayed snapshot rebases the path', async () => {
  const time = clock();
  const view = renderAtDoubleWidth([50], [10_000]);
  await time.frame(1_500);
  time.at(1_600);
  await view.rerender({ values: [50, 75], timestampsMs: [10_000, 11_000], max: 100 });
  await time.frame(2_600);
  const { pathX, heldX, heldEndX } = connectionPositions(view.container);
  expect(pathX).toBeCloseTo(heldX, 4);
  expect(heldEndX).toBeGreaterThanOrEqual(300);
  expect((view.container.querySelector('.endpoint') as HTMLElement).style.left).toBe('100%');
});

test('keeps the connection at double width after timestamp rollback', async () => {
  const time = clock();
  const view = renderAtDoubleWidth([50], [10_000]);
  await time.frame(1_500);
  time.at(1_600);
  await view.rerender({ values: [25], timestampsMs: [5_000], max: 100 });
  await time.frame(2_600);
  const { pathX, heldX, heldEndX } = connectionPositions(view.container);
  expect(pathX).toBeCloseTo(heldX, 4);
  expect(heldEndX).toBeGreaterThanOrEqual(300);
  expect((view.container.querySelector('.endpoint') as HTMLElement).style.left).toBe('100%');
});

test('keeps path bytes while translating the sampled curves between frames', async () => {
  const time = clock();
  const { container } = render(Sparkline, { values: [20, 80], timestampsMs: [0, 1_000], max: 100 });
  const path = container.querySelector('path') as SVGPathElement;
  const initialPath = path.getAttribute('d');
  await time.frame(1_000);
  const firstTransform = scroller(container).style.transform;
  await time.frame(2_000);
  expect(path.getAttribute('d')).toBe(initialPath);
  expect(scroller(container).style.transform).not.toBe(firstTransform);
  expect(scroller(container).style.transform).toMatch(/translateX\(-/);
});

test('frames between samples only translate the HTML scroll layer', async () => {
  const time = clock();
  const { container } = render(Sparkline, { values: [20, 80], timestampsMs: [0, 1_000], max: 100 });
  await time.frame(1_000);
  const records: MutationRecord[] = [];
  const observer = new MutationObserver((batch) => records.push(...batch));
  observer.observe(container, { attributes: true, subtree: true, childList: true, characterData: true });
  const before = scroller(container).style.cssText;
  for (const at of [1_020, 1_040, 1_060, 2_000, 3_000]) await time.frame(at);
  records.push(...observer.takeRecords());
  observer.disconnect();
  expect(records.length).toBeGreaterThan(0);
  for (const record of records) {
    expect(record.type).toBe('attributes');
    expect(record.target).toBe(scroller(container));
    expect(record.attributeName).toBe('style');
  }
  // Only the transform changed; everything else about the layer is as it was.
  const strip = (css: string) => css.replace(/transform:[^;]*;?/, '').trim();
  expect(strip(scroller(container).style.cssText)).toBe(strip(before));
  expect(scroller(container).style.willChange).toBe('transform');
});

test('keeps a flat held segment connected to a fixed right endpoint', async () => {
  const time = clock();
  const { container } = render(Sparkline, { values: [50], timestampsMs: [1_000], max: 100 });
  await time.frame(1_000);
  const held = container.querySelector('.held-line') as SVGLineElement;
  const marker = container.querySelector('.endpoint') as HTMLElement;
  expect(held).toBeTruthy();
  // jsdom lays out nothing, so the tile falls back to the 150-unit default width.
  const { heldX, heldEndX } = connectionPositions(container, 150);
  expect(heldX).toBeCloseTo(149.5, 1);
  expect(heldEndX).toBeGreaterThanOrEqual(150);
  expect(held.getAttribute('x2')).toBe('300');
  expect(held.getAttribute('y1')).toBe(held.getAttribute('y2'));
  expect(held.parentElement!.closest('.sparkline-scroll')).toBe(scroller(container));
  expect(marker.closest('.sparkline-scroll')).toBeNull();
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

test.each([[0, '34px'], [100, '0px'], [50, '17px']])('the endpoint at %i%% is whole at the right edge while path and held segment stay clipped', (value, top) => {
  const { container } = render(Sparkline, { values: [value], timestampsMs: [1_000], max: 100 });
  const root = container.querySelector('.sparkline') as HTMLElement;
  const marker = container.querySelector('.endpoint') as HTMLElement;
  // Centred on the right edge and on the value, even at 0% (bottom) and 100% (top).
  expect(marker.style.left).toBe('100%');
  expect(marker.style.top).toBe(top);
  const clippers = (element: Element) => {
    const found: Element[] = [];
    for (let node = element.parentElement; node && node !== root.parentElement; node = node.parentElement) {
      if (getComputedStyle(node).overflow === 'hidden') found.push(node);
    }
    return found;
  };
  // Nothing between the dot and the tile clips it, so its outer half shows past the edges.
  // jsdom applies no component stylesheet: check the tile's own rule in the source too.
  expect(clippers(marker)).toEqual([]);
  expect(/\.sparkline \{[^}]*overflow:\s*hidden/.test(source)).toBe(false);
  // The path and the held segment are clipped to the tile's box by an inner wrapper.
  const clip = container.querySelector('.sparkline-clip') as HTMLElement;
  expect(clip.parentElement).toBe(root);
  expect(clip.style.inset).toBe('0px');
  expect(clippers(scroller(container))).toEqual([clip]);
  expect(clippers(container.querySelector('.held-line')!)).toContain(clip);
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

function scrolledLayer(container: HTMLElement): HTMLElement {
  return scroller(container);
}

test('sparklines mounted on different frames scroll on the same ~60 vsyncs per second', async () => {
  const time = clock();
  const views: ReturnType<typeof render>[] = [];
  let at = 0;
  for (let i = 0; i < 4; i++) {
    views.push(render(Sparkline, { values: [50], timestampsMs: [1_000], max: 100 }));
    await time.frame(at);
    at += 1000 / 164;
  }
  const drawn = new Set<number>();
  let previous = views.map((view) => scrolledLayer(view.container).style.transform);
  for (let i = 0; i < 164; i++, at += 1000 / 164) {
    await time.frame(at);
    const current = views.map((view) => scrolledLayer(view.container).style.transform);
    const changed = current.map((transform, j) => transform !== previous[j]);
    if (changed.some(Boolean)) {
      drawn.add(i);
      expect(changed.every(Boolean)).toBe(true);
    }
    previous = current;
  }
  expect(drawn.size).toBeGreaterThanOrEqual(59);
  expect(drawn.size).toBeLessThanOrEqual(61);
});

test('a resize while scrolling re-measures the translation on the same time base and disconnects on unmount', async () => {
  const time = clock();
  let resize!: ResizeObserverCallback;
  const observe = vi.fn();
  const disconnect = vi.fn();
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: ResizeObserverCallback) { resize = callback; }
    observe = observe;
    disconnect = disconnect;
  });
  const width = vi.spyOn(HTMLDivElement.prototype, 'getBoundingClientRect').mockReturnValue({ width: 150 } as DOMRect);
  const view = render(Sparkline, { values: [20, 80], timestampsMs: [0, 1_000], max: 100 });
  await tick();
  await time.frame(30_000);
  // 30 s of a 5 min window across a 150 px tile.
  expect(translateCssPx(view.container)).toBeCloseTo(-15, 6);
  const path = view.container.querySelector('path')!.getAttribute('d');
  width.mockReturnValue({ width: 300 } as DOMRect);
  resize([], {} as ResizeObserver);
  expect(translateCssPx(view.container)).toBeCloseTo(-30, 6);
  expect(view.container.querySelector('path')!.getAttribute('d')).toBe(path);
  await time.frame(60_000);
  expect(translateCssPx(view.container)).toBeCloseTo(-60, 6);
  expect(observe).toHaveBeenCalledOnce();
  view.unmount();
  expect(disconnect).toHaveBeenCalledOnce();
  expect(time.pending()).toBe(0);
});

test('samples reuse the tracked tile width instead of forcing a layout each time', async () => {
  const time = clock();
  let resize!: ResizeObserverCallback;
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: ResizeObserverCallback) { resize = callback; }
    observe() {}
    disconnect() {}
  });
  const rect = vi.spyOn(HTMLDivElement.prototype, 'getBoundingClientRect').mockReturnValue({ width: 300 } as DOMRect);
  const view = render(Sparkline, { values: [50], timestampsMs: [0], max: 100 });
  await tick();
  const measured = rect.mock.calls.length;
  expect(measured).toBeLessThanOrEqual(1);
  for (let i = 1; i <= 5; i++) {
    time.at(i * 1000);
    await view.rerender({ values: Array(i + 1).fill(50), timestampsMs: Array.from({ length: i + 1 }, (_, j) => j * 1000), max: 100 });
    await tick();
  }
  expect(rect).toHaveBeenCalledTimes(measured);
  // The observer reports new widths itself.
  resize([{ contentRect: { width: 600 } } as ResizeObserverEntry], {} as ResizeObserver);
  await time.frame(35_000);
  expect(rect).toHaveBeenCalledTimes(measured);
  // 30 s after the last sample's edge, across a 600 px tile.
  expect(translateCssPx(view.container)).toBeCloseTo(-60, 6);
});

test('asks for frames only while it has a curve to scroll', async () => {
  const time = clock();
  const view = render(Sparkline, { values: [], timestampsMs: [], max: 100 });
  await tick();
  expect(time.pending()).toBe(0);
  await view.rerender({ values: [NaN], timestampsMs: [1_000], max: 100 });
  await tick();
  expect(time.pending()).toBe(0);
  await view.rerender({ values: [NaN, 50], timestampsMs: [1_000, 2_000], max: 100 });
  await tick();
  expect(time.pending()).toBe(1);
  await time.frame(2_500);
  expect(time.pending()).toBe(1);
  await view.rerender({ values: [], timestampsMs: [], max: 100 });
  await tick();
  expect(time.pending()).toBe(0);
  view.unmount();
  expect(time.pending()).toBe(0);
});
