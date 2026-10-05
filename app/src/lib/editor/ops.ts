// Pure edits of a profile: every function returns new objects and leaves its arguments alone.
// Rectangles stay inside the limits of profile.rs (x, y 0–400; w, h 1–200).

import { freeBlockId, LIMITS, type Block, type Profile } from './profile';

/** `commonValue` of blocks that disagree. */
export const MIXED: unique symbol = Symbol('mixed');

type Ids = Iterable<string>;

const clamp = (v: number, [lo, hi]: readonly [number, number]) => Math.min(Math.max(v, lo), hi);

/** A pixel distance as whole cells. */
export const snap = (px: number, cell: number): number => Math.round(px / cell) || 0;

function mapSelected(profile: Profile, ids: Ids, f: (b: Block) => Block): Profile {
  const set = new Set(ids);
  return { ...profile, blocks: profile.blocks.map((b) => (set.has(b.id) ? f(b) : b)) };
}

/** Moves the blocks by whole cells; the delta is cut so the group keeps its shape at the edges. */
export function moveBlocks(profile: Profile, ids: Ids, dx: number, dy: number): Profile {
  const set = new Set(ids);
  const moving = profile.blocks.filter((b) => set.has(b.id));
  if (moving.length === 0) return profile;
  const { x: xr, y: yr } = LIMITS.rect;
  const cut = (d: number, values: number[], [lo, hi]: readonly [number, number]) =>
    clamp(Math.round(d), [lo - Math.min(...values), hi - Math.max(...values)]);
  const mx = cut(dx, moving.map((b) => b.rect.x), xr);
  const my = cut(dy, moving.map((b) => b.rect.y), yr);
  return mapSelected(profile, set, (b) => ({ ...b, rect: { ...b.rect, x: b.rect.x + mx, y: b.rect.y + my } }));
}

/** Grows or shrinks the blocks by whole cells, each kept within 1–200. */
export function resizeBlocks(profile: Profile, ids: Ids, dw: number, dh: number): Profile {
  return mapSelected(profile, ids, (b) => ({
    ...b,
    rect: { ...b.rect, w: clamp(b.rect.w + Math.round(dw), LIMITS.rect.w), h: clamp(b.rect.h + Math.round(dh), LIMITS.rect.h) },
  }));
}

/**
 * Adds copies of `clip` with new ids, one cell right and down of the originals, or with their
 * top-left corner at `at`. Stops at the block limit; `ids` are the added blocks.
 */
export function pasteBlocks(profile: Profile, clip: readonly Block[], at?: { x: number; y: number }): { profile: Profile; ids: string[] } {
  const room = LIMITS.maxBlocks - profile.blocks.length;
  const copies = clip.slice(0, Math.max(room, 0));
  if (copies.length === 0) return { profile, ids: [] };
  const dx = at === undefined ? 1 : at.x - Math.min(...copies.map((b) => b.rect.x));
  const dy = at === undefined ? 1 : at.y - Math.min(...copies.map((b) => b.rect.y));
  const taken = new Set(profile.blocks.map((b) => b.id));
  const added = copies.map((b) => {
    const id = freeBlockId(taken);
    taken.add(id);
    const r = b.rect;
    return { ...structuredClone(b), id, rect: { ...r, x: clamp(r.x + dx, LIMITS.rect.x), y: clamp(r.y + dy, LIMITS.rect.y) } };
  });
  return { profile: { ...profile, blocks: [...profile.blocks, ...added] }, ids: added.map((b) => b.id) };
}

/** Copies of the blocks one cell right and down, with new ids. */
export function duplicateBlocks(profile: Profile, ids: Ids): { profile: Profile; ids: string[] } {
  const set = new Set(ids);
  return pasteBlocks(profile, profile.blocks.filter((b) => set.has(b.id)));
}

export function deleteBlocks(profile: Profile, ids: Ids): Profile {
  const set = new Set(ids);
  return { ...profile, blocks: profile.blocks.filter((b) => !set.has(b.id)) };
}

export const bringForward = (profile: Profile, ids: Ids): Profile => mapSelected(profile, ids, (b) => ({ ...b, z: b.z + 1 }));
export const sendBackward = (profile: Profile, ids: Ids): Profile => mapSelected(profile, ids, (b) => ({ ...b, z: b.z - 1 }));

const read = (value: unknown, path: string): unknown =>
  path.split('.').reduce<unknown>((v, key) => (typeof v === 'object' && v !== null ? (v as Record<string, unknown>)[key] : undefined), value);

/** The value at the dotted `path` if every block has the same one, else `MIXED`; undefined without blocks. */
export function commonValue(blocks: readonly Block[], path: string): unknown {
  if (blocks.length === 0) return undefined;
  const first = read(blocks[0], path);
  const key = JSON.stringify(first);
  return blocks.every((b) => JSON.stringify(read(b, path)) === key) ? first : MIXED;
}

function write(target: unknown, keys: string[], value: unknown): unknown {
  if (keys.length === 0) return structuredClone(value);
  const obj = (typeof target === 'object' && target !== null ? target : {}) as Record<string, unknown>;
  const [key, ...rest] = keys;
  return { ...obj, [key]: write(obj[key], rest, value) };
}

/**
 * Sets the dotted `path` to `value` in the selected blocks; the others are returned as they are.
 * A nullable object (`outline`, `shadow`, `panel`, `visibleIf`) is set whole: a leaf path under a
 * null one (`style.valueStyle.outline.width`) would create a partial object the format rejects.
 */
export function setPath(blocks: readonly Block[], ids: Ids, path: string, value: unknown): Block[] {
  const set = new Set(ids);
  const keys = path.split('.');
  return blocks.map((b) => (set.has(b.id) ? (write(b, keys, value) as Block) : b));
}
