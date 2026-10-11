// The RAM page's two JSON properties (DN3): `memoryModules` (SPD per module) and `memoryTimings`
// (in-use timings from the memory controller). The text is untrusted: everything is validated here
// and anything unexpected is dropped, never thrown.

export const MAX_MODULES = 16;
export const MAX_CHANNELS = 12;

const MODULE_NUMBERS = [
  'capacityBytes',
  'tempHighC',
  'tempCritC',
  'tckMinNs',
  'taaNs',
  'trcdNs',
  'trpNs',
  'trasNs',
  'trcNs',
  'trfc1Ns',
  'trfc2Ns',
  'trfcSbNs',
] as const;

/** In-use numeric fields of one channel. */
export const TIMING_NUMBERS = [
  'mclkMhz',
  'dataRate',
  'cl',
  'trcdRd',
  'trcdWr',
  'trp',
  'tras',
  'trc',
  'trrdS',
  'trrdL',
  'tfaw',
  'twtrS',
  'twtrL',
  'tcwl',
  'twr',
  'trtp',
  'trefi',
  'trfc',
  'trfc2',
  'trfcSb',
  'trfcNs',
] as const;

export const REFRESH_MODES = ['normal', 'fgr', 'mixed', 'pbOnly'] as const;
export type RefreshMode = (typeof REFRESH_MODES)[number];

export type MemoryModule = { slot: string } & { [K in (typeof MODULE_NUMBERS)[number]]?: number };

export type ChannelTimings = {
  /** Controller channel letter; absent when every channel has the same values. */
  channel?: string;
  cmd2t?: boolean;
  gdm?: boolean;
  refreshMode?: RefreshMode;
} & { [K in (typeof TIMING_NUMBERS)[number]]?: number };

export interface MemoryTimingsInfo {
  generation: string;
  channels: ChannelTimings[];
}

export interface MemoryInfo {
  modules: MemoryModule[];
  timings: MemoryTimingsInfo | null;
}

const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v) && v >= 0;
const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

function json(text: string | undefined): unknown {
  if (!text) return undefined;
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

function module(raw: unknown): MemoryModule | null {
  if (!isObject(raw) || typeof raw.slot !== 'string') return null;
  const out: MemoryModule = { slot: raw.slot };
  for (const key of MODULE_NUMBERS) {
    const v = raw[key];
    if (v === undefined || v === null) continue;
    if (!isNum(v)) return null; // a module with a garbled number is not trusted at all
    out[key] = v;
  }
  return out;
}

function channel(raw: unknown): ChannelTimings | null {
  if (!isObject(raw)) return null;
  const out: ChannelTimings = {};
  if (typeof raw.channel === 'string') out.channel = raw.channel;
  for (const key of TIMING_NUMBERS) {
    const v = raw[key];
    if (isNum(v)) out[key] = v;
  }
  if (typeof raw.cmd2t === 'boolean') out.cmd2t = raw.cmd2t;
  if (typeof raw.gdm === 'boolean') out.gdm = raw.gdm;
  if ((REFRESH_MODES as readonly unknown[]).includes(raw.refreshMode)) out.refreshMode = raw.refreshMode as RefreshMode;
  return out.cl === undefined || out.mclkMhz === undefined ? null : out;
}

function timings(raw: unknown): MemoryTimingsInfo | null {
  if (!isObject(raw)) return null;
  const generation = typeof raw.generation === 'string' ? raw.generation : '';
  const list = Array.isArray(raw.channels) ? raw.channels.slice(0, MAX_CHANNELS) : [raw];
  const channels = list.map(channel).filter((c) => c !== null);
  return channels.length > 0 ? { generation, channels } : null;
}

/** Validates the two RAM properties of a device; `null` when neither holds anything usable. */
export function parseMemoryTimings(properties: Record<string, string> | undefined): MemoryInfo | null {
  const rawModules = json(properties?.memoryModules);
  const modules = Array.isArray(rawModules)
    ? rawModules.slice(0, MAX_MODULES).map(module).filter((m) => m !== null)
    : [];
  const info = timings(json(properties?.memoryTimings));
  return modules.length === 0 && info === null ? null : { modules, timings: info };
}

/** JEDEC rounding of a time in ns to clock cycles. */
export const jedecCycles = (ns: number, tckNs: number): number => Math.ceil(ns / tckNs - 0.01);

/** Transfers per second (MT/s) of a DDR clock period. */
export const jedecRate = (tckNs: number): number => Math.round(2000 / tckNs);
