// Cell and pixel geometry of the overlay panel: a port of crates/oma-core/src/overlay/geometry.rs
// and of `profile_frame`/`block_px` in crates/oma-overlay/src/render/layout.rs. The shared fixture
// testdata/overlay/geometry-cases.json keeps the two in step. Where Rust divides integers it
// truncates toward zero (`Math.trunc`), and `f64::round` rounds halves away from zero.

import { LIMITS, type Block, type CellRect, type Profile } from './profile';

/** Integer pixel rectangle (physical pixels). */
export interface PxRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Placed {
  window: PxRect;
  profile: PxRect | null;
  extra: PxRect | null;
}

const round = (v: number) => Math.sign(v) * Math.round(Math.abs(v));
const half = (v: number) => Math.trunc(v / 2);

/** Size of one layout cell in physical pixels. */
export function cellPx(scale: number, dpi: number): number {
  return (LIMITS.cellPx * scale * dpi) / 96;
}

/** Union of the block rectangles, null without blocks. */
export function footprint(blocks: readonly Block[]): CellRect | null {
  if (blocks.length === 0) return null;
  const x0 = Math.min(...blocks.map((b) => b.rect.x));
  const y0 = Math.min(...blocks.map((b) => b.rect.y));
  const x1 = Math.max(...blocks.map((b) => b.rect.x + b.rect.w));
  const y1 = Math.max(...blocks.map((b) => b.rect.y + b.rect.h));
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

const LEFT = new Set(['top-left', 'left', 'bottom-left']);
const RIGHT = new Set(['top-right', 'right', 'bottom-right']);
const TOP = new Set(['top-left', 'top', 'top-right']);
const BOTTOM = new Set(['bottom-left', 'bottom', 'bottom-right']);

/** Anchored top-left corner and size of a `w` x `h` box, before clamping. */
function anchored(profile: Profile, area: PxRect, w: number, h: number, cell: number): PxRect {
  const ox = round(profile.offset.x * cell);
  const oy = round(profile.offset.y * cell);
  const a = profile.anchor;
  const x = LEFT.has(a) ? area.x + ox : RIGHT.has(a) ? area.x + (area.w - w) - ox : area.x + half(area.w - w);
  const y = TOP.has(a) ? area.y + oy : BOTTOM.has(a) ? area.y + (area.h - h) - oy : area.y + half(area.h - h);
  return { x, y, w, h };
}

function clampAxis(pos: number, len: number, start: number, span: number): number {
  const max = start + Math.max(span - len, 0);
  return Math.min(Math.max(pos, start), max);
}

const clampRect = (r: PxRect, area: PxRect): PxRect => ({
  ...r,
  x: clampAxis(r.x, r.w, area.x, area.w),
  y: clampAxis(r.y, r.h, area.y, area.h),
});

function panelSize(profile: Profile, dpi: number): { w: number; h: number; cell: number } | null {
  const f = footprint(profile.blocks);
  if (f === null) return null;
  const cell = cellPx(profile.scale, dpi);
  const pad = profile.panel.padding;
  return { w: round((f.w + 2 * pad) * cell), h: round((f.h + 2 * pad) * cell), cell };
}

/** Pixel rectangle of the whole panel anchored in `area`, kept inside it; null without blocks. */
export function place(profile: Profile, area: PxRect, dpi: number): PxRect | null {
  const size = panelSize(profile, dpi);
  return size === null ? null : clampRect(anchored(profile, area, size.w, size.h, size.cell), area);
}

/**
 * Like `place`, with an extra box of `extraCells` (the benchmark box) attached to the profile:
 * below it for top and middle anchors, above it for bottom anchors, aligned to the anchor side.
 * The window is the union of both, kept inside `area`; null if there is nothing to place.
 */
export function placeWithExtra(profile: Profile, area: PxRect, dpi: number, extraCells: [number, number] | null): Placed | null {
  if (extraCells === null) {
    const r = place(profile, area, dpi);
    return r === null ? null : { window: r, profile: r, extra: null };
  }
  const ecell = cellPx(profile.scale, dpi);
  const ew = round(extraCells[0] * ecell);
  const eh = round(extraCells[1] * ecell);
  const size = panelSize(profile, dpi);
  if (size === null) {
    const e = clampRect(anchored(profile, area, ew, eh, ecell), area);
    return { window: e, profile: null, extra: e };
  }
  const p = anchored(profile, area, size.w, size.h, size.cell);
  const a = profile.anchor;
  const x = LEFT.has(a) ? p.x : RIGHT.has(a) ? p.x + p.w - ew : p.x + half(p.w - ew);
  const y = BOTTOM.has(a) ? p.y - eh : p.y + p.h;
  const e = { x, y, w: ew, h: eh };
  const x0 = Math.min(p.x, e.x);
  const y0 = Math.min(p.y, e.y);
  const u = { x: x0, y: y0, w: Math.max(p.x + p.w, e.x + e.w) - x0, h: Math.max(p.y + p.h, e.y + e.h) - y0 };
  const window = clampRect(u, area);
  const shift = (r: PxRect) => ({ ...r, x: r.x + window.x - u.x, y: r.y + window.y - u.y });
  return { window, profile: shift(p), extra: shift(e) };
}

/** Where the profile is drawn inside its panel: the cell size and the pixel position of cell (0, 0). */
export function profileFrame(profile: Profile, dpi: number): { cell: number; origin: [number, number] } | null {
  const f = footprint(profile.blocks);
  if (f === null) return null;
  const cell = cellPx(profile.scale, dpi);
  const pad = profile.panel.padding;
  return { cell, origin: [(pad - f.x) * cell, (pad - f.y) * cell] };
}

/** The block's rectangle in pixels; `origin` is where cell (0, 0) lies. */
export function blockPx(block: Block, origin: [number, number], cell: number): PxRect {
  const r = block.rect;
  return {
    x: Math.fround(origin[0] + r.x * cell),
    y: Math.fround(origin[1] + r.y * cell),
    w: Math.fround(r.w * cell),
    h: Math.fround(r.h * cell),
  };
}
