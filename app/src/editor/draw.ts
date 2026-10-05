// Canvas2D drawing of a profile on the editor's canvas, close to the overlay's renderer
// (crates/oma-overlay/src/render/{layout,mod,shapes,text}.rs) and to the evaluation of
// crates/oma-core/src/overlay/eval.rs: thresholds, `visibleIf`, statistics, chart geometry,
// meter and gauge ranges (DD16). Fonts are the browser's, so it is close, not identical.

import { DASH, formatValue } from '../lib/format';
import type { Translate } from '../lib/i18n/index.svelte';
import { LIMITS, type Block, type CompareOp, type FrameMetric, type Profile, type RangeBound, type Rgba, type Source, type Stat, type TextStyle, type Threshold, type VisibleIf, type YAxis } from '../lib/editor/profile';
import { footprint } from '../lib/editor/geometry';
import type { FrameMetrics, Unit, WireFrameTime } from '../lib/types';

/** What the drawing reads: values, chart samples and the sensors of the schema. */
export interface Readout {
  /** The value of `source` after `stat`; null when absent (frames while the engine is not running, texts, `bound`). */
  value(source: Source, stat: Stat): number | null;
  /** Chart samples `[t_s, value]` of a sensor or a frame metric, oldest first. */
  samples(source: Source): [number, number][];
  frameTimes: readonly WireFrameTime[];
  /** The latest metrics while the frame engine runs, else null. */
  metrics: FrameMetrics | null;
  sensor(id: string): { label: string; unit: Unit } | undefined;
  t: Translate;
  locale: string;
}

/** Where the profile lies on the canvas: the pixel position of cell (0, 0) and the cell size. */
export interface View {
  origin: [number, number];
  cell: number;
}

/** The automatic range of `meter` and `gauge` looks this far back (DD16). */
export const AUTO_RANGE_S = 60;
const GRID = 'rgba(255, 255, 255, 0.188)';
const GAUGE_START_DEG = 135;
/** Opacity of a block its `visibleIf` hides: drawn anyway, so it stays editable. */
export const HIDDEN_ALPHA = 0.3;

// ---- evaluation (eval.rs) ----

function compare(op: CompareOp, value: number, limit: number): boolean {
  switch (op) {
    case '>':
      return value > limit;
    case '>=':
      return value >= limit;
    case '<':
      return value < limit;
    case '<=':
      return value <= limit;
  }
}

const finite = (v: number | null | undefined): v is number => typeof v === 'number' && Number.isFinite(v);

/** `current` is the last sample; `min`, `avg` and `max` cover the `stat.window` seconds before the newest one (inclusive). */
export function statOf(samples: readonly [number, number][], stat: Stat): number | null {
  const newest = samples.at(-1);
  if (newest === undefined) return null;
  if (stat.op === 'current') return newest[1];
  const from = newest[0] - stat.window;
  let lo = Infinity;
  let hi = -Infinity;
  let sum = 0;
  let n = 0;
  for (let i = samples.length - 1; i >= 0 && samples[i][0] >= from; i--) {
    const v = samples[i][1];
    lo = Math.min(lo, v);
    hi = Math.max(hi, v);
    sum += v;
    n++;
  }
  if (n === 0) return null;
  return stat.op === 'min' ? lo : stat.op === 'max' ? hi : sum / n;
}

/** The colour of the first true rule for `target`, null when none applies. */
export function thresholdColor(thresholds: readonly Threshold[], target: Threshold['target'], value: number | null): Rgba | null {
  if (!finite(value)) return null;
  return thresholds.find((t) => t.target === target && compare(t.op, value, t.value))?.color ?? null;
}

/** Frame generation in use: multiplier above 1.2, or the «FG?» heuristic. */
export const fgActive = (m: FrameMetrics | null): boolean => m !== null && (m.fg_suspected || (m.fg_multiplier ?? 0) > 1.2);

/** No condition shows; an absent source hides. */
export function isVisible(cond: VisibleIf | null, readout: Readout, fg: boolean): boolean {
  if (cond === null) return true;
  if ('fg' in cond) return fg;
  const v = readout.value(cond.source, cond.stat);
  return finite(v) && compare(cond.op, v, cond.value);
}

// ---- texts (layout.rs) ----

export interface TextParts {
  label: string;
  value: string;
  unit: string;
}

/** A frame metric as number and unit (`format_frame_metric`). */
function formatFrameMetric(metric: FrameMetric, value: number | null, locale: string): [string, string] {
  if (!finite(value)) return [DASH, ''];
  const n = (digits: number) => new Intl.NumberFormat(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits, useGrouping: false }).format(value);
  switch (metric) {
    case 'fps-displayed':
    case 'fps-rendered':
    case 'fps-presented':
    case 'low-1':
    case 'low-01':
      return [n(0), 'FPS'];
    case 'frametime-displayed':
    case 'frametime-app':
    case 'latency-pc':
    case 'latency-display':
      return [n(1), 'ms'];
    case 'fg-multiplier':
      return [`×${n(1)}`, ''];
    case 'stutter':
      return [n(0), ''];
    case 'bound':
      return [DASH, ''];
  }
}

/** `formatValue`'s text split into number and unit, as the overlay draws them apart. */
function splitUnit(text: string): [string, string] {
  const m = /^([-−]?[\d.,   ]*\d)\s*(.*)$/.exec(text);
  return m === null ? [text, ''] : [m[1], m[2]];
}

/** Number and unit of a block's value (`format_block_value`). */
// ponytail: `style.decimals` and `style.unit` are not applied here (the app's formatter has no
// such options); the preview shows them exactly.
function formatBlockValue(block: Block, readout: Readout, value: number | null): [string, string] {
  const { source } = block;
  if ('text' in source) return [source.text, ''];
  if ('sensor' in source) {
    const info = readout.sensor(source.sensor);
    if (info === undefined) return [readout.t('overlay.text.sensorAbsent'), ''];
    return splitUnit(formatValue(value, info.unit, readout.locale, readout.t));
  }
  const m = readout.metrics;
  if (m === null) return [DASH, ''];
  if (source.frames === 'bound') return m.bound === 'gpu' || m.bound === 'cpu' ? [readout.t(`overlay.text.bound.${m.bound}`), ''] : [DASH, ''];
  if (source.frames === 'fps-rendered' && m.fg_suspected) return [readout.t('overlay.text.fgSuspected'), ''];
  return formatFrameMetric(source.frames, value, readout.locale);
}

function blockLabel(block: Block, readout: Readout): string {
  if (block.style.label !== null) return block.style.label;
  const { source } = block;
  if ('text' in source) return '';
  if ('sensor' in source) return readout.sensor(source.sensor)?.label ?? '';
  const name = readout.t(`overlay.text.metric.${source.frames}`);
  return source.frames === 'low-1' || source.frames === 'low-01' ? `${name}${readout.t(`overlay.text.low.${block.stat.definition}`)}` : name;
}

/** Label, value and unit of a block (spec §6.2). */
export function textParts(block: Block, readout: Readout, value = readout.value(block.source, block.stat)): TextParts {
  const [v, unit] = formatBlockValue(block, readout, value);
  return { label: blockLabel(block, readout), value: v, unit };
}

/** The name of a block's source, for lists: the sensor, the metric or the text. */
export function sourceName(source: Source, readout: Pick<Readout, 'sensor' | 't'>): string {
  if ('text' in source) return source.text;
  if ('sensor' in source) return readout.sensor(source.sensor)?.label ?? readout.t('overlay.text.sensorAbsent');
  return readout.t(`overlay.text.metric.${source.frames}`);
}

// ---- geometry (layout.rs) ----

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

const stylePx = (v: number, cell: number) => (v * cell) / LIMITS.cellPx;
const fontPx = (pt: number, cell: number) => (pt * 4 * cell) / 3 / LIMITS.cellPx;

/** Text and shape areas of a block (`areas`). */
function areas(block: Block, r: Rect, cell: number): { text: Rect; shape: Rect } {
  switch (block.kind) {
    case 'sparkline': {
      const tw = (r.w * 3) / 5;
      const inset = (r.h * 3) / 20;
      return { text: { ...r, w: tw }, shape: { x: r.x + tw, y: r.y + inset, w: r.w - tw, h: r.h - 2 * inset } };
    }
    case 'meter': {
      if (block.style.meter.orientation === 'vertical') {
        const bw = Math.min(Math.max(r.w * 0.25, 2), r.w);
        const tx = Math.min(r.x + bw + cell / 2, r.x + r.w);
        return { text: { ...r, x: tx, w: r.x + r.w - tx }, shape: { ...r, w: bw } };
      }
      if (r.h < 2 * cell) return { text: r, shape: r };
      const bh = Math.min(Math.max((r.h * 3) / 10, 2), r.h);
      return { text: { ...r, h: r.h - bh }, shape: { ...r, y: r.y + r.h - bh, h: bh } };
    }
    case 'gauge': {
      const side = Math.min(r.w, r.h);
      return { text: r, shape: { x: r.x + (r.w - side) / 2, y: r.y + (r.h - side) / 2, w: side, h: side } };
    }
    default:
      return { text: r, shape: r };
  }
}

/** Visible minimum to maximum with a 10% margin, or the fixed range (`y_range`). */
function yRange(values: number[], y: YAxis): [number, number] | null {
  if (y.mode === 'fixed') return y.max > y.min ? [y.min, y.max] : null;
  if (values.length === 0) return null;
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  if (hi === lo) {
    const half = Math.max(Math.abs(lo) * 0.1, 0.5);
    return [lo - half, hi + half];
  }
  const margin = (hi - lo) * 0.1;
  return [lo - margin, hi + margin];
}

/** Chart points of the last `rangeS` seconds up to the newest sample (`graph_points`). */
function graphPoints(samples: readonly [number, number][], r: Rect, rangeS: number, y: YAxis): [number, number][] {
  const now = samples.at(-1)?.[0];
  if (now === undefined || rangeS <= 0) return [];
  const visible = samples.filter(([t, v]) => t >= now - rangeS && t <= now && Number.isFinite(v));
  const range = yRange(
    visible.map(([, v]) => v),
    y,
  );
  if (range === null) return [];
  const [lo, hi] = range;
  return visible.map(([t, v]) => [r.x + r.w * (1 - (now - t) / rangeS), r.y + r.h * (1 - Math.min(Math.max((v - lo) / (hi - lo), 0), 1))]);
}

/** One bar per frame, as wide as its frametime, the newest on the right edge (`frametime_bars`). */
function frametimeBars(samples: readonly [number, number][], r: Rect, rangeS: number): Rect[] {
  const newest = samples.at(-1)?.[0];
  if (newest === undefined || rangeS <= 0) return [];
  const visible = samples.filter(([t, ft]) => t > newest - rangeS && Number.isFinite(ft) && ft > 0);
  const top = Math.max(0, ...visible.map(([, ft]) => ft)) * 1.1;
  if (top <= 0) return [];
  const xOf = (t: number) => r.x + r.w * (1 - (newest - t) / rangeS);
  return visible.map(([t, ft]) => {
    const x1 = xOf(t);
    const x0 = Math.max(xOf(t - ft / 1000), r.x);
    const bh = (r.h * ft) / top;
    return { x: x0, y: r.y + r.h - bh, w: x1 - x0, h: bh };
  });
}

/** Range of a meter or gauge (`value_range`): 0–100 for a percentage, else up to the highest recent value. */
function valueRange(min: RangeBound, max: RangeBound, percent: boolean, value: number | null, recentMax: number | null): [number, number] {
  const v = value ?? 0;
  const lo = min !== 'auto' ? min.fixed : percent ? 0 : Math.min(v, 0);
  const hi = max !== 'auto' ? max.fixed : percent ? 100 : Math.max(v, recentMax ?? v, 0, lo);
  return [lo, hi];
}

const fraction = (v: number | null, lo: number, hi: number) => (!finite(v) || !(hi > lo) ? 0 : Math.min(Math.max((v - lo) / (hi - lo), 0), 1));

// ---- drawing ----

/** `#RRGGBB[AA]` with its alpha multiplied by `opacity`, as `rgba()`. */
export function rgba(hex: Rgba, opacity = 1): string {
  const n = (i: number) => parseInt(hex.slice(i, i + 2), 16);
  const a = hex.length >= 9 ? n(7) / 255 : 1;
  return `rgba(${n(1)}, ${n(3)}, ${n(5)}, ${+(a * Math.min(Math.max(opacity, 0), 1)).toFixed(3)})`;
}

function roundRect(ctx: CanvasRenderingContext2D, r: Rect, radius: number, fill: string) {
  ctx.fillStyle = fill;
  ctx.beginPath();
  ctx.roundRect(r.x, r.y, r.w, r.h, Math.max(0, Math.min(radius, r.w / 2, r.h / 2)));
  ctx.fill();
}

const fontOf = (s: TextStyle, px: number) => `${s.italic ? 'italic ' : ''}${s.weight} ${px}px "${s.font}", sans-serif`;

interface Measured {
  w: number;
  ascent: number;
  descent: number;
}

function measure(ctx: CanvasRenderingContext2D, text: string, s: TextStyle, cell: number): Measured {
  if (text === '') return { w: 0, ascent: 0, descent: 0 };
  const px = fontPx(s.size, cell);
  ctx.font = fontOf(s, px);
  const m = ctx.measureText(text);
  return { w: m.width, ascent: m.fontBoundingBoxAscent ?? px * 0.8, descent: m.fontBoundingBoxDescent ?? px * 0.2 };
}

/** Draws `text` with its top-left corner at `at`: shadow, outline (a stroke under the fill), fill. */
function drawText(ctx: CanvasRenderingContext2D, text: string, s: TextStyle, cell: number, at: [number, number], fill: Rgba, ascent: number) {
  if (text === '') return;
  ctx.save();
  ctx.font = fontOf(s, fontPx(s.size, cell));
  ctx.textBaseline = 'alphabetic';
  if (s.shadow !== null) {
    ctx.shadowColor = s.shadow.color;
    ctx.shadowOffsetX = stylePx(s.shadow.dx, cell);
    ctx.shadowOffsetY = stylePx(s.shadow.dy, cell);
    ctx.shadowBlur = 0;
  }
  if (s.outline !== null) {
    ctx.lineJoin = 'round';
    ctx.lineWidth = stylePx(s.outline.width, cell) * 2;
    ctx.strokeStyle = s.outline.color;
    ctx.strokeText(text, at[0], at[1] + ascent);
    // The fill on top of the outline casts no second shadow.
    ctx.shadowColor = 'transparent';
  }
  ctx.fillStyle = fill;
  ctx.fillText(text, at[0], at[1] + ascent);
  ctx.restore();
}

/** Top-left corners of label, value and unit on one shared baseline (`row_positions`). */
function rowPositions(r: Rect, [label, value, unit]: Measured[], align: Block['style']['align'], gap: number, unitGap: number): [number, number][] {
  const ascent = Math.max(label.ascent, value.ascent, unit.ascent);
  const descent = Math.max(label.descent, value.descent, unit.descent);
  const baseline = r.y + (r.h - (ascent + descent)) / 2 + ascent;
  const start = label.w > 0 ? r.x + label.w + gap : r.x;
  const ug = unit.w > 0 && value.w > 0 ? unitGap : 0;
  const free = Math.max(r.x + r.w - start - (value.w + ug + unit.w), 0);
  const vx = align === 'left' ? start : align === 'center' ? start + free / 2 : start + free;
  return [
    [r.x, baseline - label.ascent],
    [vx, baseline - value.ascent],
    [vx + value.w + ug, baseline - unit.ascent],
  ];
}

function chartSamples(block: Block, readout: Readout): [number, number][] {
  const src = block.source;
  if ('frames' in src && block.kind === 'graph' && block.style.graph.mode === 'frametime' && src.frames.startsWith('frametime-')) {
    const app = src.frames === 'frametime-app';
    const out: [number, number][] = [];
    for (const f of readout.frameTimes) {
      const ms = app ? f.app_ms : f.displayed_ms;
      if (ms !== null) out.push([f.t_s, ms]);
    }
    return out;
  }
  return readout.samples(src);
}

/** Draws one block in `r` (pixels); `panelFill` is the profile's panel when the block has none. */
function drawBlock(ctx: CanvasRenderingContext2D, block: Block, r: Rect, profile: Profile, cell: number, readout: Readout) {
  const value = readout.value(block.source, block.stat);
  const color = (target: Threshold['target']) => thresholdColor(block.thresholds, target, value);
  const st = block.style;
  const panelColor = color('panel');
  if (block.panel !== null || panelColor !== null) {
    const p = block.panel ?? profile.panel;
    roundRect(ctx, r, stylePx(p.radius, cell), rgba(panelColor ?? p.color, p.opacity));
  }
  ctx.save();
  ctx.beginPath();
  ctx.rect(r.x, r.y, r.w, r.h);
  ctx.clip();

  const g = st.graph;
  const graphColor = color('graph');
  const line = graphColor ?? g.line.color;
  const fill = rgba(graphColor ?? g.fill.color, g.fill.alpha);
  const area = areas(block, r, cell);
  const shape = area.shape;
  const percent = 'sensor' in block.source && readout.sensor(block.source.sensor)?.unit === 'percent';
  const recentMax = () => statOf(readout.samples(block.source), { op: 'max', window: AUTO_RANGE_S, definition: 'integral' });
  let gaugeFraction = 0;
  switch (block.kind) {
    case 'graph':
    case 'sparkline': {
      if (block.kind === 'graph' && g.gridLines > 0) {
        ctx.strokeStyle = GRID;
        ctx.lineWidth = 1;
        ctx.beginPath();
        for (let i = 1; i <= g.gridLines; i++) {
          const y = Math.round(shape.y + (shape.h * i) / (g.gridLines + 1)) + 0.5;
          ctx.moveTo(shape.x, y);
          ctx.lineTo(shape.x + shape.w, y);
        }
        ctx.stroke();
      }
      const samples = chartSamples(block, readout);
      const frametime = block.kind === 'graph' && g.mode === 'frametime' && 'frames' in block.source && block.source.frames.startsWith('frametime-');
      if (frametime) {
        ctx.fillStyle = line;
        for (const b of frametimeBars(samples, shape, g.rangeS)) ctx.fillRect(b.x, b.y, b.w, b.h);
        break;
      }
      const pts = graphPoints(samples, shape, g.rangeS, g.y);
      const mode = block.kind === 'sparkline' ? 'line' : g.mode;
      if (mode === 'bars') {
        ctx.fillStyle = line;
        pts.forEach(([x, y], i) => {
          const x0 = i === 0 ? Math.max(x - 2, shape.x) : pts[i - 1][0];
          const w = x - x0 > 2 ? x - x0 - 1 : x - x0;
          ctx.fillRect(x - w, y, w, shape.y + shape.h - y);
        });
        break;
      }
      if (pts.length < 2) break;
      if (mode === 'area') {
        ctx.fillStyle = fill;
        ctx.beginPath();
        pts.forEach(([x, y], i) => (i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)));
        ctx.lineTo(pts[pts.length - 1][0], shape.y + shape.h);
        ctx.lineTo(pts[0][0], shape.y + shape.h);
        ctx.closePath();
        ctx.fill();
      }
      ctx.strokeStyle = line;
      ctx.lineWidth = stylePx(g.line.width, cell);
      ctx.lineJoin = 'round';
      ctx.beginPath();
      pts.forEach(([x, y], i) => (i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)));
      ctx.stroke();
      break;
    }
    case 'meter': {
      const [lo, hi] = valueRange(st.meter.min, st.meter.max, percent, value, recentMax());
      const f = fraction(value, lo, hi);
      ctx.fillStyle = fill;
      ctx.fillRect(shape.x, shape.y, shape.w, shape.h);
      ctx.fillStyle = line;
      if (st.meter.orientation === 'horizontal') ctx.fillRect(shape.x, shape.y, shape.w * f, shape.h);
      else ctx.fillRect(shape.x, shape.y + shape.h * (1 - f), shape.w, shape.h * f);
      break;
    }
    case 'gauge': {
      const [lo, hi] = valueRange(st.gauge.min, st.gauge.max, percent, value, recentMax());
      gaugeFraction = fraction(value, lo, hi);
      const stroke = Math.max(shape.w * 0.1, 1);
      const radius = Math.max(shape.w / 2 - stroke / 2, 0);
      const cx = shape.x + shape.w / 2;
      const cy = shape.y + shape.h / 2;
      const rad = (deg: number) => (deg * Math.PI) / 180;
      ctx.lineWidth = stroke;
      ctx.strokeStyle = fill;
      ctx.beginPath();
      ctx.arc(cx, cy, radius, rad(GAUGE_START_DEG), rad(GAUGE_START_DEG + 270));
      ctx.stroke();
      if (gaugeFraction > 0) {
        ctx.strokeStyle = line;
        ctx.beginPath();
        ctx.arc(cx, cy, radius, rad(GAUGE_START_DEG), rad(GAUGE_START_DEG + 270 * gaugeFraction));
        ctx.stroke();
      }
      break;
    }
    case 'text':
      break;
  }

  // Texts.
  const parts = textParts(block, readout, value);
  if (block.kind === 'graph' && !g.showValue) {
    parts.value = '';
    parts.unit = '';
  }
  const styles = [st.labelStyle, st.valueStyle, st.unitStyle];
  const strings = [parts.label, parts.value, parts.unit];
  const sizes = strings.map((s, i) => measure(ctx, s, styles[i], cell));
  const gap = cell / 2;
  const unitGap = parts.unit === '%' ? 0 : fontPx(st.valueStyle.size, cell) * 0.25;
  let at: [number, number][];
  if (block.kind === 'gauge') {
    const mid = rowPositions(shape, [{ w: 0, ascent: 0, descent: 0 }, sizes[1], sizes[2]], 'center', 0, unitGap);
    const l = sizes[0];
    at = [[shape.x + (shape.w - l.w) / 2, shape.y + shape.h - l.ascent - l.descent], mid[1], mid[2]];
  } else if (block.kind === 'graph') {
    const bandH = Math.max(...sizes.map((s) => s.ascent + s.descent));
    at = rowPositions({ x: r.x + gap / 2, y: r.y, w: Math.max(r.w - gap, 0), h: bandH }, sizes, 'right', gap, unitGap);
  } else {
    at = rowPositions(area.text, sizes, st.align, gap, unitGap);
  }
  const fills = [st.labelStyle.color, color('value') ?? st.valueStyle.color, st.unitStyle.color];
  strings.forEach((s, i) => drawText(ctx, s, styles[i], cell, at[i], fills[i], sizes[i].ascent));

  if (block.kind === 'graph' && g.showMinAvgMax) {
    const samples = chartSamples(block, readout);
    const w = (op: Stat['op']) => statOf(samples, { op, window: g.rangeS, definition: 'integral' });
    const avg = w('avg');
    if (avg !== null) {
      const fmt = (v: number | null) => ('frames' in block.source ? formatFrameMetric(block.source.frames, v, readout.locale) : formatBlockValue(block, readout, v));
      const [lo] = fmt(w('min'));
      const [mid] = fmt(avg);
      const [hi, unit] = fmt(w('max'));
      const text = `${lo} / ${mid} / ${hi}${unit === '' ? '' : unit === '%' ? '%' : ` ${unit}`}`;
      const s = measure(ctx, text, st.labelStyle, cell);
      drawText(ctx, text, st.labelStyle, cell, [r.x + gap / 2, r.y + r.h - s.ascent - s.descent], st.labelStyle.color, s.ascent);
    }
  }
  ctx.restore();
}

/** The pixel rectangle of a block on the canvas. */
export const blockRect = (block: Block, view: View): Rect => ({
  x: view.origin[0] + block.rect.x * view.cell,
  y: view.origin[1] + block.rect.y * view.cell,
  w: block.rect.w * view.cell,
  h: block.rect.h * view.cell,
});

/** The blocks in drawing order: by `z`, equal `z` in profile order. */
export const byZ = (blocks: readonly Block[]): Block[] => blocks.map((b, i) => [b, i] as const).sort((a, b) => a[0].z - b[0].z || a[1] - b[1]).map(([b]) => b);

/**
 * Draws the profile's panel and its blocks. A block its `visibleIf` hides is drawn at 30%
 * opacity with a dashed border, so it stays visible and editable.
 */
export function drawProfile(ctx: CanvasRenderingContext2D, profile: Profile, view: View, readout: Readout): void {
  const f = footprint(profile.blocks);
  if (f === null) return;
  const { cell } = view;
  const p = profile.panel;
  const pad = p.padding;
  const whole = { x: view.origin[0] + (f.x - pad) * cell, y: view.origin[1] + (f.y - pad) * cell, w: (f.w + 2 * pad) * cell, h: (f.h + 2 * pad) * cell };
  roundRect(ctx, whole, stylePx(p.radius, cell), rgba(p.color, p.opacity));
  const fg = fgActive(readout.metrics);
  for (const block of byZ(profile.blocks)) {
    const r = blockRect(block, view);
    const visible = isVisible(block.visibleIf, readout, fg);
    ctx.save();
    if (!visible) ctx.globalAlpha = HIDDEN_ALPHA;
    drawBlock(ctx, block, r, profile, cell, readout);
    if (!visible) {
      // The border at full opacity, so the hidden block is easy to find.
      ctx.globalAlpha = 1;
      ctx.setLineDash([4, 3]);
      ctx.strokeStyle = '#FFFFFF';
      ctx.lineWidth = 1;
      ctx.strokeRect(r.x + 0.5, r.y + 0.5, r.w - 1, r.h - 1);
    }
    ctx.restore();
  }
}
