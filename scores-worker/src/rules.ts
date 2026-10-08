// Submission rules shared (by fixtures in testdata/scores/) with the app.

export const MAX_BODY_BYTES = 16384;
export const VALUE_CAP = 100000;
export const PLAUSIBLE_MIN = 0.2;
export const PLAUSIBLE_MAX = 5;
export const MIN_ENTRIES = 3;
export const DAILY_CAP = 2000;
export const MODEL_MAX = 128;

export type Board = "cpu-single" | "cpu-multi" | "gpu-compute" | "gpu-graphics" | "disk";
export type Category = "cpu" | "gpu" | "disk";
export type ErrorCode =
  | "bad_json"
  | "bad_schema"
  | "bad_format"
  | "unknown_version"
  | "not_valid"
  | "bad_value"
  | "implausible"
  | "body_too_large"
  | "rate_limited"
  | "daily_cap"
  | "not_found"
  | "method_not_allowed";

export interface Submission {
  category: Category;
  scoreVersion: string;
  overclock: boolean;
  appVersion: string;
  osBuild: string;
  ramGB: number;
  flags: string[];
  model: { display: string; key: string };
  values: { board: Board; value: number }[];
}

const KNOWN_VERSION: Record<Category, string> = { cpu: "cpu-1", gpu: "gpu-1", disk: "disk-1" };
const SCORE_KEYS: Record<Category, [string, Board][]> = {
  cpu: [["single", "cpu-single"], ["multi", "cpu-multi"]],
  gpu: [["compute", "gpu-compute"], ["graphics", "gpu-graphics"]],
  disk: [["points", "disk"]],
};

export function normalizeModel(raw: string): { display: string; key: string } {
  const display = raw
    .replace(/\((?:R|TM)\)/gi, "")
    .replace(/[®™]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return { display, key: display.toLowerCase() };
}

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => typeof v === "object" && v !== null && !Array.isArray(v);
const BAD_SCHEMA = { ok: false, error: "bad_schema" } as const;

export function validateSubmission(
  body: unknown,
): { ok: true; value: Submission } | { ok: false; error: ErrorCode } {
  if (!isObj(body)) return BAD_SCHEMA;
  const { appVersion, category, scoreVersion, valid, overclock, scores, kernels, hardware, flags } = body;
  if (typeof appVersion !== "string" || appVersion.length > 16 || !/^\d+\.\d+\.\d+$/.test(appVersion)) return BAD_SCHEMA;
  if (category !== "cpu" && category !== "gpu" && category !== "disk") return BAD_SCHEMA;
  if (typeof scoreVersion !== "string") return BAD_SCHEMA;
  if (typeof valid !== "boolean" || typeof overclock !== "boolean") return BAD_SCHEMA;
  if (!isObj(scores)) return BAD_SCHEMA;
  const values: { board: Board; value: number }[] = [];
  for (const [key, board] of SCORE_KEYS[category]) {
    const value = scores[key];
    if (typeof value !== "number") return BAD_SCHEMA;
    values.push({ board, value });
  }
  if (!Array.isArray(kernels) || kernels.length > 32) return BAD_SCHEMA;
  if (!isObj(hardware)) return BAD_SCHEMA;
  const { model: rawModel, ramGB, osBuild } = hardware;
  if (typeof rawModel !== "string" || /[\u0000-\u001f\u007f-\u009f\p{Cf}]/u.test(rawModel)) return BAD_SCHEMA;
  const model = normalizeModel(rawModel);
  if (model.display.length < 1 || model.display.length > MODEL_MAX) return BAD_SCHEMA;
  if (typeof ramGB !== "number" || !Number.isInteger(ramGB) || ramGB < 1 || ramGB > 4096) return BAD_SCHEMA;
  if (typeof osBuild !== "string" || !/^\d{4,6}(\.\d{1,6})?$/.test(osBuild)) return BAD_SCHEMA;
  if (
    !Array.isArray(flags) ||
    flags.length > 16 ||
    !flags.every((f) => typeof f === "string" && /^[a-z0-9_]{1,32}$/.test(f))
  ) {
    return BAD_SCHEMA;
  }

  if (body.format !== 1) return { ok: false, error: "bad_format" };
  if (scoreVersion !== KNOWN_VERSION[category]) return { ok: false, error: "unknown_version" };
  if (valid !== true) return { ok: false, error: "not_valid" };
  if (values.some((v) => !Number.isFinite(v.value) || v.value <= 0 || v.value > VALUE_CAP)) {
    return { ok: false, error: "bad_value" };
  }
  return {
    ok: true,
    value: { category, scoreVersion, overclock, appVersion, osBuild, ramGB, flags: flags as string[], model, values },
  };
}
