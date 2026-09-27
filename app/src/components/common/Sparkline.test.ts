import { cleanup, render } from '@testing-library/svelte';
import Sparkline from './Sparkline.svelte';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
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
