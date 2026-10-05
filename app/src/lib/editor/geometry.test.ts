import cases from '../../../../testdata/overlay/geometry-cases.json';
import { blockPx, cellPx, footprint, place, placeWithExtra, profileFrame } from './geometry';
import { BLOCK_DEFAULTS, type Block, type Profile, withDefaults } from './profile';

type Rect = { x: number; y: number; w: number; h: number };

test('geometry matches the shared fixture', () => {
  expect(cases.length).toBeGreaterThanOrEqual(17);
  for (const c of cases) {
    const profile = withDefaults(c.profile as unknown as Profile);
    const extra = c.extraCells === null ? null : (c.extraCells as [number, number]);
    const got = placeWithExtra(profile, c.area as Rect, c.dpi, extra);
    expect({ name: c.name, ...got }).toEqual({ name: c.name, ...c.expected });
  }
});

const block = (x: number, y: number, w: number, h: number): Block => ({
  ...structuredClone(BLOCK_DEFAULTS),
  id: `b${x}-${y}`,
  rect: { x, y, w, h },
  source: { text: 't' },
  kind: 'text',
});

test('footprint is the union of the blocks, null without blocks', () => {
  expect(footprint([])).toBeNull();
  expect(footprint([block(2, 3, 4, 1), block(1, 5, 2, 2)])).toEqual({ x: 1, y: 3, w: 5, h: 4 });
});

test('cell, place and block pixels follow the Rust formulas', () => {
  expect(cellPx(1.5, 144)).toBe(18);
  const profile = withDefaults({ format: 1, name: 'p', blocks: [block(2, 1, 10, 2)] });
  expect(place(profile, { x: 0, y: 0, w: 1920, h: 1080 }, 96)).toEqual({ x: 8, y: 8, w: 96, h: 32 });
  const frame = profileFrame(profile, 96)!;
  expect(frame.origin).toEqual([-8, 0]);
  expect(blockPx(profile.blocks[0], frame.origin, frame.cell)).toEqual({ x: 8, y: 8, w: 80, h: 16 });
});
