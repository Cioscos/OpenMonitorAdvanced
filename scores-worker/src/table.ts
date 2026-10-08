// Reference table (spec 8.4): parsing, the plausibility check and the author rows.
import { normalizeModel, PLAUSIBLE_MAX, PLAUSIBLE_MIN, type Board } from "./rules.ts";

export interface TableRow {
  category: Board;
  scoreVersion: string;
  model: string;
  value: number;
  n: number;
  source: "author" | "community";
}

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const BOARDS: readonly string[] = ["cpu-single", "cpu-multi", "gpu-compute", "gpu-graphics", "disk"];

export function median(values: number[]): number {
  const s = [...values].sort((a, b) => a - b);
  const mid = s.length >> 1;
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

export function parseTable(text: string): TableRow[] | null {
  let t: unknown;
  try {
    t = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObj(t) || t.format !== 1 || !Array.isArray(t.rows)) return null;
  const rows: TableRow[] = [];
  for (const r of t.rows) {
    if (
      isObj(r) &&
      typeof r.category === "string" && BOARDS.includes(r.category) &&
      typeof r.scoreVersion === "string" &&
      typeof r.model === "string" &&
      typeof r.value === "number" && Number.isFinite(r.value) && r.value > 0 &&
      typeof r.n === "number" &&
      (r.source === "author" || r.source === "community")
    ) {
      rows.push(r as unknown as TableRow);
    }
  }
  return rows;
}

export function plausible(board: Board, scoreVersion: string, key: string, value: number, rows: TableRow[]): boolean {
  const same = rows.filter((r) => r.category === board && r.scoreVersion === scoreVersion);
  if (same.length === 0) return true;
  const own = same.filter((r) => normalizeModel(r.model).key === key);
  if (own.length) {
    const ref = median(own.map((r) => r.value));
    return PLAUSIBLE_MIN * ref <= value && value <= PLAUSIBLE_MAX * ref;
  }
  // Unknown model: the band of the whole category, so slow hardware (HDD, USB stick, iGPU) still passes.
  const values = same.map((r) => r.value);
  return PLAUSIBLE_MIN * Math.min(...values) <= value && value <= PLAUSIBLE_MAX * Math.max(...values);
}

const KNOWN_VERSION: Record<string, string> = { cpu: "cpu-1", gpu: "gpu-1", disk: "disk-1" };
const SCORE_KEYS: Record<string, [string, Board][]> = {
  cpu: [["single", "cpu-single"], ["multi", "cpu-multi"]],
  gpu: [["compute", "gpu-compute"], ["graphics", "gpu-graphics"]],
  disk: [["points", "disk"]],
};

export function authorRows(files: unknown[]): TableRow[] {
  const groups = new Map<string, { board: Board; version: string; display: string; key: string; values: number[] }>();
  for (const f of files) {
    if (!isObj(f) || f.valid !== true || f.provisional !== false) continue;
    const cat = f.category;
    if (typeof cat !== "string" || !(cat in KNOWN_VERSION) || f.scoreVersion !== KNOWN_VERSION[cat]) continue;
    if (cat === "disk" && f.diskProfile !== "b1") continue;
    if (!isObj(f.scores) || !isObj(f.device) || typeof f.device.model !== "string") continue;
    const m = normalizeModel(f.device.model);
    if (!m.display) continue;
    for (const [k, board] of SCORE_KEYS[cat]) {
      const v = f.scores[k];
      if (typeof v !== "number" || !Number.isFinite(v) || v <= 0) continue;
      const id = `${board}\n${f.scoreVersion}\n${m.key}`;
      const g = groups.get(id) ?? { board, version: f.scoreVersion, display: m.display, key: m.key, values: [] };
      g.values.push(v);
      groups.set(id, g);
    }
  }
  return [...groups.values()]
    .sort((a, b) => cmp(a.board, b.board) || cmp(a.version, b.version) || cmp(a.key, b.key))
    .map((g) => ({
      category: g.board,
      scoreVersion: g.version,
      model: g.display,
      value: Math.round(median(g.values)),
      n: g.values.length,
      source: "author" as const,
    }));
}

const cmp = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
