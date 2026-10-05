import {
  bringForward,
  commonValue,
  deleteBlocks,
  duplicateBlocks,
  firstFreeCell,
  MIXED,
  moveBlocks,
  pasteBlocks,
  resizeBlocks,
  sendBackward,
  setPath,
  snap,
} from './ops';
import { blockDefaults, newBlock, withDefaults, type Block, type Profile } from './profile';

const block = (id: string, x: number, y: number, w = 4, h = 2): Block => ({
  ...blockDefaults(),
  id,
  rect: { x, y, w, h },
  source: { text: id },
  kind: 'text',
});
const profileOf = (...blocks: Block[]): Profile => withDefaults({ format: 1, name: 'p', blocks });
const rects = (p: Profile) => p.blocks.map((b) => b.rect);

test('move snaps to whole cells and stays in limits', () => {
  expect(snap(13, 8)).toBe(2);
  expect(snap(-3, 8)).toBe(0);
  const p = profileOf(block('a', 1, 1), block('b', 5, 3), block('c', 9, 9));
  const moved = moveBlocks(p, ['a', 'b'], 2, 1);
  expect(rects(moved)).toEqual([
    { x: 3, y: 2, w: 4, h: 2 },
    { x: 7, y: 4, w: 4, h: 2 },
    { x: 9, y: 9, w: 4, h: 2 },
  ]);
  // The group keeps its shape at the edge: the delta is cut, not each block.
  expect(rects(moveBlocks(p, ['a', 'b'], -5, -9)).slice(0, 2)).toEqual([
    { x: 0, y: 0, w: 4, h: 2 },
    { x: 4, y: 2, w: 4, h: 2 },
  ]);
  expect(moveBlocks(p, ['c'], 1000, 1000).blocks[2].rect).toEqual({ x: 400, y: 400, w: 4, h: 2 });
  expect(moveBlocks(p, ['a'], 1.4, 0).blocks[0].rect.x).toBe(2);
  expect(p.blocks[0].rect.x).toBe(1);
});

test('resize keeps w and h in 1..200', () => {
  const p = profileOf(block('a', 0, 0, 4, 2), block('b', 0, 0, 199, 1));
  expect(rects(resizeBlocks(p, ['a', 'b'], -10, 3))).toEqual([
    { x: 0, y: 0, w: 1, h: 5 },
    { x: 0, y: 0, w: 189, h: 4 },
  ]);
  expect(rects(resizeBlocks(p, ['b'], 5, -5))[1]).toEqual({ x: 0, y: 0, w: 200, h: 1 });
});

test('duplicate gives new ids', () => {
  const p = profileOf(block('a', 1, 1), block('b', 2, 2));
  const { profile, ids } = duplicateBlocks(p, ['a', 'b']);
  expect(profile.blocks).toHaveLength(4);
  expect(new Set(profile.blocks.map((b) => b.id)).size).toBe(4);
  expect(ids).toHaveLength(2);
  expect(ids).not.toContain('a');
  expect(profile.blocks.find((b) => b.id === ids[0])!.rect).toEqual({ x: 2, y: 2, w: 4, h: 2 });
});

test('paste offsets by one cell', () => {
  const p = profileOf(block('a', 3, 4));
  const clip = [structuredClone(p.blocks[0])];
  const once = pasteBlocks(p, clip);
  expect(once.ids).toHaveLength(1);
  expect(once.profile.blocks[1].rect).toEqual({ x: 4, y: 5, w: 4, h: 2 });
  expect(once.profile.blocks[1].id).not.toBe('a');
  const at = pasteBlocks(p, clip, { x: 10, y: 0 });
  expect(at.profile.blocks[1].rect).toEqual({ x: 10, y: 0, w: 4, h: 2 });
});

test('paste stops at the block limit', () => {
  const full = profileOf(...Array.from({ length: 256 }, (_, i) => block(`b${i}`, 0, 0)));
  const { profile, ids } = pasteBlocks(full, [block('x', 0, 0)]);
  expect(profile.blocks).toHaveLength(256);
  expect(ids).toEqual([]);
});

test('delete and layer order', () => {
  const p = profileOf(block('a', 0, 0), block('b', 0, 0));
  expect(deleteBlocks(p, ['a']).blocks.map((b) => b.id)).toEqual(['b']);
  expect(bringForward(p, ['a']).blocks[0].z).toBe(1);
  expect(sendBackward(p, ['b']).blocks[1].z).toBe(-1);
});

test('commonValue reports MIXED', () => {
  const a = block('a', 0, 0);
  const b = block('b', 0, 0);
  b.style.valueStyle.size = 20;
  expect(commonValue([a, b], 'style.valueStyle.size')).toBe(MIXED);
  expect(commonValue([a, b], 'style.valueStyle.font')).toBe('Segoe UI');
  expect(commonValue([a, b], 'kind')).toBe('text');
  expect(commonValue([a, b], 'style.graph.y')).toEqual({ mode: 'auto', min: 0, max: 100 });
});

test('setPath applies to every selected block', () => {
  const blocks = [block('a', 0, 0), block('b', 0, 0), block('c', 0, 0)];
  const next = setPath(blocks, ['a', 'c'], 'style.valueStyle.color', '#FF00FF');
  expect(next.map((b) => b.style.valueStyle.color)).toEqual(['#FF00FF', '#FFFFFF', '#FF00FF']);
  expect(blocks[0].style.valueStyle.color).toBe('#FFFFFF');
  expect(next[1]).toBe(blocks[1]);
});

test('new graph block for frametime sources', () => {
  const existing = [block('b1', 0, 0)];
  const graph = newBlock({ frames: 'frametime-displayed' }, { x: 2, y: 3 }, existing);
  expect(graph.kind).toBe('graph');
  expect(graph.style.graph.mode).toBe('frametime');
  expect(graph.rect).toEqual({ x: 2, y: 3, w: 20, h: 4 });
  expect(graph.id).not.toBe('b1');
  const text = newBlock({ sensor: 'cpu/0/load/total' }, { x: 0, y: 0 }, existing);
  expect(text.kind).toBe('text');
  expect(text.rect).toEqual({ x: 0, y: 0, w: 12, h: 2 });
  expect(newBlock({ frames: 'fps-displayed' }, { x: 0, y: 0 }, []).kind).toBe('text');
});

test('the default objects of a block are distinct', () => {
  const b = newBlock({ text: 't' }, { x: 0, y: 0 }, []);
  const { labelStyle, valueStyle, unitStyle, meter, gauge } = b.style;
  expect(new Set([labelStyle, valueStyle, unitStyle]).size).toBe(3);
  expect(meter).not.toBe(gauge);
  valueStyle.size = 30;
  expect(labelStyle.size).toBe(12);
  expect(newBlock({ text: 't' }, { x: 0, y: 0 }, []).style.valueStyle.size).toBe(12);
  const [loaded] = withDefaults({ format: 1, name: 'p', blocks: [{ id: 'a', rect: { x: 0, y: 0, w: 1, h: 1 }, source: { text: 't' }, kind: 'text' } as Block] }).blocks;
  expect(loaded.style.labelStyle).not.toBe(loaded.style.unitStyle);
});

test('defaults are filled inside a panel, a comparison and the thresholds', () => {
  const partial = {
    id: 'a',
    rect: { x: 0, y: 0, w: 1, h: 1 },
    source: { text: 't' },
    kind: 'text',
    panel: { opacity: 0.8 },
    visibleIf: { source: { frames: 'fps-displayed' }, op: '<', value: 60 },
    thresholds: [{ op: '>', value: 90, color: '#FF0000' }],
  } as unknown as Block;
  const [b] = withDefaults({ format: 1, name: 'p', blocks: [partial] }).blocks;
  expect(b.panel).toEqual({ color: '#000000', opacity: 0.8, radius: 4, padding: 1 });
  expect(b.visibleIf).toEqual({ source: { frames: 'fps-displayed' }, stat: { op: 'current', window: 1, definition: 'integral' }, op: '<', value: 60 });
  expect(b.thresholds).toEqual([{ op: '>', value: 90, color: '#FF0000', target: 'value' }]);
  expect(withDefaults({ format: 1, name: 'p', blocks: [{ ...partial, visibleIf: { fg: 'active' } }] }).blocks[0].visibleIf).toEqual({ fg: 'active' });
});

test('firstFreeCell finds the top-left-most free cell', () => {
  const a = newBlock({ text: 'a' }, { x: 0, y: 0 }, []);
  const b = newBlock({ text: 'b' }, { x: 12, y: 0 }, [a]);
  expect(firstFreeCell([], 12, 2)).toEqual({ x: 0, y: 0 });
  expect(firstFreeCell([a], 12, 2)).toEqual({ x: 12, y: 0 });
  expect(firstFreeCell([a, b], 12, 2)).toEqual({ x: 24, y: 0 });
});
