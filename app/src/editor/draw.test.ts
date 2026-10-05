import { t } from '../lib/i18n/index.svelte';
import { newBlock, withDefaults, type Block, type Profile } from '../lib/editor/profile';
import type { FrameMetrics } from '../lib/types';
import { drawProfile, isVisible, statOf, thresholdColor, type Readout } from './draw';

/** One call on the fake context, with the drawing state it was made in. */
interface Call {
  name: string;
  args: unknown[];
  fillStyle: unknown;
  globalAlpha: number;
}

/** A 2D context that records its calls; `save`/`restore` keep the state like the real one. */
function fakeContext() {
  const calls: Call[] = [];
  let state: Record<string, unknown> = { fillStyle: '#000', globalAlpha: 1, font: '10px sans-serif' };
  const stack: Record<string, unknown>[] = [];
  const ctx = new Proxy({} as Record<string | symbol, unknown>, {
    get(_, name) {
      if (typeof name !== 'string') return undefined;
      if (name in state) return state[name];
      return (...args: unknown[]) => {
        calls.push({ name, args, fillStyle: state.fillStyle, globalAlpha: state.globalAlpha as number });
        if (name === 'save') stack.push({ ...state });
        if (name === 'restore') state = stack.pop() ?? state;
        if (name === 'measureText') return { width: String(args[0]).length * 6, fontBoundingBoxAscent: 8, fontBoundingBoxDescent: 2 };
        return undefined;
      };
    },
    set(_, name, value) {
      state[name as string] = value;
      return true;
    },
  });
  return { ctx: ctx as unknown as CanvasRenderingContext2D, calls };
}

const SENSOR = 'gpu/0/temperature/core';

function readout(values: Record<string, number | null>, metrics: FrameMetrics | null = null): Readout {
  return {
    value: (source) => ('sensor' in source ? (values[source.sensor] ?? null) : null),
    samples: () => [],
    frameTimes: [],
    metrics,
    sensor: (id) => (id === SENSOR ? { label: 'Core temperature', unit: 'celsius' } : undefined),
    t,
    format: { decimalComma: false, temperature: 'c', rate: 'bits', flagOn: 'On', flagOff: 'Off' },
  };
}

function profileWith(...blocks: Block[]): Profile {
  return { ...withDefaults({ format: 1, name: 'p' }), blocks };
}

const texts = (calls: Call[]) => calls.filter((c) => c.name === 'fillText');

test('hidden blocks are drawn translucent', () => {
  const block = newBlock({ sensor: SENSOR }, { x: 0, y: 0 }, []);
  block.visibleIf = { source: { sensor: SENSOR }, stat: { op: 'current', window: 1, definition: 'integral' }, op: '>', value: 90 };
  const { ctx, calls } = fakeContext();
  drawProfile(ctx, profileWith(block), { origin: [0, 0], cell: 8 }, readout({ [SENSOR]: 50 }));
  const drawn = texts(calls);
  expect(drawn.length).toBeGreaterThan(0);
  expect(drawn.every((c) => c.globalAlpha === 0.3)).toBe(true);
  expect(calls.some((c) => c.name === 'setLineDash' && (c.args[0] as number[]).length > 0)).toBe(true);

  // Shown: fully opaque, no dashed border.
  const shown = fakeContext();
  drawProfile(shown.ctx, profileWith(block), { origin: [0, 0], cell: 8 }, readout({ [SENSOR]: 95 }));
  expect(texts(shown.calls).every((c) => c.globalAlpha === 1)).toBe(true);
});

test('threshold colour applies to its target', () => {
  const block = newBlock({ sensor: SENSOR }, { x: 0, y: 0 }, []);
  block.thresholds = [
    { op: '>', value: 80, color: '#FF0000', target: 'value' },
    { op: '>', value: 50, color: '#00FF00', target: 'panel' },
  ];
  const { ctx, calls } = fakeContext();
  drawProfile(ctx, profileWith(block), { origin: [0, 0], cell: 8 }, readout({ [SENSOR]: 85 }));
  const value = texts(calls).find((c) => c.args[0] === '85');
  expect(value?.fillStyle).toBe('#FF0000');
  // The label keeps its own colour; the panel turns green.
  expect(texts(calls).find((c) => c.args[0] === 'Core temperature')?.fillStyle).toBe('#FFFFFF');
  expect(calls.some((c) => c.name === 'fill' && String(c.fillStyle).startsWith('rgba(0, 255, 0'))).toBe(true);

  expect(thresholdColor(block.thresholds, 'value', 70)).toBeNull();
  expect(thresholdColor(block.thresholds, 'panel', 70)).toBe('#00FF00');
});

test('missing sensor shows the absent text', () => {
  const block = newBlock({ sensor: 'gpu/9/temperature/core' }, { x: 0, y: 0 }, []);
  const { ctx, calls } = fakeContext();
  drawProfile(ctx, profileWith(block), { origin: [0, 0], cell: 8 }, readout({}));
  expect(texts(calls).map((c) => c.args[0])).toContain(t('overlay.text.sensorAbsent'));
});

test('visibleIf follows the overlay semantics', () => {
  const r = readout({ [SENSOR]: null });
  const cond = { source: { sensor: SENSOR }, stat: { op: 'current' as const, window: 1, definition: 'integral' as const }, op: '<' as const, value: 10 };
  // No condition shows; an absent source hides.
  expect(isVisible(null, r, false)).toBe(true);
  expect(isVisible(cond, r, false)).toBe(false);
  expect(isVisible({ fg: 'active' }, r, true)).toBe(true);
  expect(isVisible({ fg: 'active' }, r, false)).toBe(false);
});

test('statOf covers the window before the newest sample', () => {
  const samples: [number, number][] = [
    [0, 100],
    [5, 10],
    [8, 20],
    [10, 30],
  ];
  expect(statOf(samples, { op: 'current', window: 5, definition: 'integral' })).toBe(30);
  expect(statOf(samples, { op: 'max', window: 5, definition: 'integral' })).toBe(30);
  expect(statOf(samples, { op: 'min', window: 5, definition: 'integral' })).toBe(10);
  expect(statOf(samples, { op: 'avg', window: 5, definition: 'integral' })).toBe(20);
  expect(statOf([], { op: 'avg', window: 5, definition: 'integral' })).toBeNull();
});

test('values use the block decimals and the drawing settings', () => {
  const block = newBlock({ sensor: SENSOR }, { x: 0, y: 0 }, []);
  block.style.decimals = 1;
  const r = readout({ [SENSOR]: 85.25 });
  r.format = { ...r.format, decimalComma: true, temperature: 'f' };
  const { ctx, calls } = fakeContext();
  drawProfile(ctx, profileWith(block), { origin: [0, 0], cell: 8 }, r);
  const drawn = texts(calls).map((c) => c.args[0]);
  expect(drawn).toContain('185,5');
  expect(drawn).toContain('°F');
});
