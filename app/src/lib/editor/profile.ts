// The overlay profile format (spec §6) as the editor edits it. Names, defaults and limits are
// copied from crates/oma-core/src/overlay/profile.rs: keep them in step with that file.

export type Anchor = 'top-left' | 'top' | 'top-right' | 'left' | 'center' | 'right' | 'bottom-left' | 'bottom' | 'bottom-right';

export type FrameMetric =
  | 'fps-displayed'
  | 'fps-rendered'
  | 'fps-presented'
  | 'frametime-displayed'
  | 'frametime-app'
  | 'low-1'
  | 'low-01'
  | 'fg-multiplier'
  | 'stutter'
  | 'latency-pc'
  | 'latency-display'
  | 'bound';

/** What a block shows: a JSON object with exactly one key. */
export type Source = { sensor: string } | { frames: FrameMetric } | { text: string };

export type StatOp = 'current' | 'min' | 'avg' | 'max';
export type LowDefinitionKey = 'integral' | 'percentile';
export interface Stat {
  op: StatOp;
  window: number;
  definition: LowDefinitionKey;
}

export type Kind = 'text' | 'graph' | 'meter' | 'sparkline' | 'gauge';
export type Align = 'left' | 'center' | 'right';
export type UnitChoice = 'auto' | 'B' | 'KB' | 'MB' | 'GB' | 'TB' | 'MHz' | 'GHz' | 'bit/s' | 'kbit/s' | 'Mbit/s' | 'Gbit/s';
/** `#RRGGBB` or `#RRGGBBAA`. */
export type Rgba = string;

export interface Outline {
  width: number;
  color: Rgba;
}
export interface Shadow {
  dx: number;
  dy: number;
  color: Rgba;
}
export interface TextStyle {
  font: string;
  size: number;
  weight: number;
  italic: boolean;
  color: Rgba;
  outline: Outline | null;
  shadow: Shadow | null;
}

export type GraphMode = 'line' | 'area' | 'bars' | 'frametime';
export interface YAxis {
  mode: 'auto' | 'fixed';
  min: number;
  max: number;
}
export interface GraphStyle {
  mode: GraphMode;
  rangeS: number;
  y: YAxis;
  line: { color: Rgba; width: number };
  fill: { color: Rgba; alpha: number };
  gridLines: number;
  showMinAvgMax: boolean;
  showValue: boolean;
}

export type RangeBound = 'auto' | { fixed: number };
export interface RangeStyle {
  orientation: 'horizontal' | 'vertical';
  min: RangeBound;
  max: RangeBound;
}

export interface Style {
  labelStyle: TextStyle;
  valueStyle: TextStyle;
  unitStyle: TextStyle;
  align: Align;
  decimals: number | null;
  unit: UnitChoice;
  label: string | null;
  graph: GraphStyle;
  meter: RangeStyle;
  gauge: RangeStyle;
}

export type CompareOp = '>' | '>=' | '<' | '<=';
export interface Threshold {
  op: CompareOp;
  value: number;
  color: Rgba;
  target: 'value' | 'graph' | 'panel';
}
export interface Comparison {
  source: Source;
  stat: Stat;
  op: CompareOp;
  value: number;
}
export type VisibleIf = { fg: 'active' } | Comparison;

export interface Panel {
  color: Rgba;
  opacity: number;
  radius: number;
  padding: number;
}

export interface CellRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Block {
  id: string;
  rect: CellRect;
  z: number;
  source: Source;
  stat: Stat;
  kind: Kind;
  style: Style;
  thresholds: Threshold[];
  visibleIf: VisibleIf | null;
  panel: Panel | null;
}

export interface Profile {
  format: number;
  name: string;
  anchor: Anchor;
  offset: { x: number; y: number };
  scale: number;
  panel: Panel;
  blocks: Block[];
}

export const LIMITS = {
  format: 1,
  maxBlocks: 256,
  maxTextBytes: 65_536,
  maxProfileBytes: 1_048_576,
  /** Edge of one cell at scale 1.0 and 96 dpi, in logical pixels. */
  cellPx: 8,
  blockId: { maxLength: 64 },
  rect: { x: [0, 400], y: [0, 400], w: [1, 200], h: [1, 200] },
  scale: [0.5, 3.0],
  panel: { opacity: [0, 1], radius: [0, 32], padding: [0, 4] },
  text: { fontLength: 64, size: [6, 72], weight: [100, 900], outlineWidth: [0.5, 4], shadow: [-8, 8] },
  decimals: [0, 3],
  statWindow: [1, 300],
  graph: { rangeS: [5, 300], lineWidth: [0.5, 4], fillAlpha: [0, 1], gridLines: [0, 8] },
  thresholds: 8,
} as const;

const ACCENT = '#00E5FF';

const TEXT_STYLE: TextStyle = { font: 'Segoe UI', size: 12, weight: 600, italic: false, color: '#FFFFFF', outline: null, shadow: null };
const RANGE_STYLE: RangeStyle = { orientation: 'horizontal', min: 'auto', max: 'auto' };

export const PANEL_DEFAULTS: Panel = { color: '#000000', opacity: 0.35, radius: 4, padding: 1 };

/** Every field of a profile but `format`, `name` and `blocks`. */
export const PROFILE_DEFAULTS: Omit<Profile, 'format' | 'name' | 'blocks'> = {
  anchor: 'top-left',
  offset: { x: 1, y: 1 },
  scale: 1,
  panel: PANEL_DEFAULTS,
};

/** Every field of a block but `id`, `rect`, `source` and `kind`. */
export const BLOCK_DEFAULTS: Omit<Block, 'id' | 'rect' | 'source' | 'kind'> = {
  z: 0,
  stat: { op: 'current', window: 1, definition: 'integral' },
  style: {
    labelStyle: TEXT_STYLE,
    valueStyle: TEXT_STYLE,
    unitStyle: TEXT_STYLE,
    align: 'left',
    decimals: null,
    unit: 'auto',
    label: null,
    graph: {
      mode: 'line',
      rangeS: 60,
      y: { mode: 'auto', min: 0, max: 100 },
      line: { color: ACCENT, width: 1.5 },
      fill: { color: ACCENT, alpha: 0.25 },
      gridLines: 2,
      showMinAvgMax: false,
      showValue: true,
    },
    meter: RANGE_STYLE,
    gauge: RANGE_STYLE,
  },
  thresholds: [],
  visibleIf: null,
  panel: null,
};

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

/** `value` with the missing keys taken from `defaults`, as serde's `#[serde(default)]` does. */
function merge(defaults: unknown, value: unknown): unknown {
  if (value === undefined) return structuredClone(defaults);
  if (!isObject(defaults) || !isObject(value)) return value;
  const out: Record<string, unknown> = {};
  for (const key of new Set([...Object.keys(defaults), ...Object.keys(value)])) out[key] = merge(defaults[key], value[key]);
  return out;
}

/** A partial profile (as a file may omit defaults) with every default filled in. */
export function withDefaults(partial: Partial<Profile> & Pick<Profile, 'format' | 'name'>): Profile {
  const profile = merge(PROFILE_DEFAULTS, { ...partial, blocks: undefined }) as Profile;
  profile.blocks = (partial.blocks ?? []).map((b) => merge(BLOCK_DEFAULTS, b) as Block);
  return profile;
}

/** An empty profile called `name`. */
export function emptyProfile(name: string): Profile {
  return withDefaults({ format: LIMITS.format, name });
}

/** The first `b<n>` id not in `taken`. */
export function freeBlockId(taken: ReadonlySet<string>): string {
  let n = 1;
  while (taken.has(`b${n}`)) n++;
  return `b${n}`;
}

const isFrametime = (source: Source) => 'frames' in source && source.frames.startsWith('frametime-');

/**
 * A new block for `source` at `cell` (§7.2): a `graph` of the frame times for `frametime-*`
 * sources (20×4 cells), a `text` otherwise (12×2).
 */
export function newBlock(source: Source, cell: { x: number; y: number }, existing: readonly Block[]): Block {
  const graph = isFrametime(source);
  const block = merge(BLOCK_DEFAULTS, {
    id: freeBlockId(new Set(existing.map((b) => b.id))),
    rect: { x: cell.x, y: cell.y, w: graph ? 20 : 12, h: graph ? 4 : 2 },
    source: structuredClone(source),
    kind: graph ? 'graph' : 'text',
  }) as Block;
  if (graph) block.style.graph.mode = 'frametime';
  return block;
}
