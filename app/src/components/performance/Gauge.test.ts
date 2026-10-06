import { cleanup, render, screen } from '@testing-library/svelte';
import { angleFor } from '../../lib/performance/gauge';
import Gauge from './Gauge.svelte';

let reducedMotion = false;
let visibility: DocumentVisibilityState = 'visible';
let raf: ReturnType<typeof vi.fn>;
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });

beforeEach(() => {
  reducedMotion = false;
  visibility = 'visible';
  raf = vi.fn(() => 1);
  vi.stubGlobal('requestAnimationFrame', raf);
  vi.stubGlobal('cancelAnimationFrame', vi.fn());
  vi.stubGlobal('matchMedia', (query: string) => ({
    get matches() { return query.includes('reduced-motion') && reducedMotion; },
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
});
afterEach(cleanup);
afterEach(() => vi.unstubAllGlobals());

const base = { value: 1234, max: 2000, reference: null, label: 'Single core', unit: 'points', moving: false };
const needleAngle = (container: HTMLElement) =>
  container.querySelector('.needle')?.getAttribute('transform');

test('meter_has_value_and_label', () => {
  render(Gauge, base);
  const meter = screen.getByRole('meter', { name: 'Single core' });
  expect(meter.getAttribute('aria-valuenow')).toBe('1234');
  expect(meter.getAttribute('aria-valuemin')).toBe('0');
  expect(meter.getAttribute('aria-valuemax')).toBe('2000');
  expect(meter.textContent).toContain('1234');
  expect(meter.textContent).toContain('points');
});

test('meter_without_a_value_has_no_valuenow', () => {
  render(Gauge, { ...base, value: null });
  expect(screen.getByRole('meter').hasAttribute('aria-valuenow')).toBe(false);
});

test('reference_mark_only_with_a_reference', async () => {
  const { container, rerender } = render(Gauge, base);
  expect(container.querySelector('.reference')).toBeNull();
  await rerender({ ...base, reference: 1500 });
  expect(container.querySelector('.reference')).not.toBeNull();
});

test('reduced_motion_jumps_to_the_value', async () => {
  reducedMotion = true;
  const { container, rerender } = render(Gauge, { ...base, value: 0, moving: true });
  await rerender({ ...base, value: 1500, moving: true });
  expect(raf).not.toHaveBeenCalled();
  expect(needleAngle(container)).toBe(`rotate(${angleFor(1500, 2000)} 150 150)`);
});

test('no_animation_frame_when_not_moving', async () => {
  const { container, rerender } = render(Gauge, { ...base, value: 0 });
  await rerender({ ...base, value: 1500 });
  expect(raf).not.toHaveBeenCalled();
  expect(needleAngle(container)).toBe(`rotate(${angleFor(1500, 2000)} 150 150)`);
});

test('moving_needle_animates_only_while_visible', async () => {
  const { rerender } = render(Gauge, { ...base, value: 0, moving: true });
  await rerender({ ...base, value: 1500, moving: true });
  expect(raf).toHaveBeenCalled();
  raf.mockClear();
  visibility = 'hidden';
  document.dispatchEvent(new Event('visibilitychange'));
  await rerender({ ...base, value: 1800, moving: true });
  expect(raf).not.toHaveBeenCalled();
});
