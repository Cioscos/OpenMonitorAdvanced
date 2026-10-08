import { AUTHOR_ROWS } from "./author.ts";
import type { Env } from "./env.ts";
import { json } from "./http.ts";
import { DAILY_CAP, MAX_BODY_BYTES, validateSubmission } from "./rules.ts";
import { parseTable, plausible } from "./table.ts";

// Reads at most MAX_BODY_BYTES; null when the body is larger.
async function readLimited(request: Request): Promise<string | null | undefined> {
  const length = request.headers.get("Content-Length");
  if (length !== null && Number(length) > MAX_BODY_BYTES) return null;
  if (!request.body) return undefined;
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > MAX_BODY_BYTES) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const all = new Uint8Array(total);
  let at = 0;
  for (const c of chunks) {
    all.set(c, at);
    at += c.byteLength;
  }
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(all);
  } catch {
    return undefined;
  }
}

export async function handleSubmit(request: Request, env: Env, today: string): Promise<Response> {
  const text = await readLimited(request);
  if (text === null) return json(413, { error: "body_too_large" });

  const key = request.headers.get("CF-Connecting-IP") ?? "unknown";
  const { success } = await env.SUBMIT_LIMIT.limit({ key });
  if (!success) return json(429, { error: "rate_limited" });

  let body: unknown;
  try {
    if (text === undefined) throw new Error();
    body = JSON.parse(text);
  } catch {
    return json(400, { error: "bad_json" });
  }
  const v = validateSubmission(body);
  if (!v.ok) return json(400, { error: v.error });
  const s = v.value;

  const pub = await env.DB.prepare("SELECT body FROM published WHERE id = 1").first<{ body: string }>();
  const rows = [...AUTHOR_ROWS, ...((pub && parseTable(pub.body)) ?? [])];
  if (!s.values.every((x) => plausible(x.board, s.scoreVersion, s.model.key, x.value, rows))) {
    return json(400, { error: "implausible" });
  }

  const cap = await env.DB.prepare(
    "INSERT INTO daily (day, n) VALUES (?, 1) ON CONFLICT (day) DO UPDATE SET n = n + 1 RETURNING n",
  )
    .bind(today)
    .first<{ n: number }>();
  if (!cap || cap.n > DAILY_CAP) return json(503, { error: "daily_cap" });

  const submission = crypto.randomUUID();
  const insert = env.DB.prepare(
    "INSERT INTO entries (submission, day, board, score_version, model_key, model, value, overclock, app_version, os_build, ram_gb, flags) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
  );
  await env.DB.batch(
    s.values.map((x) =>
      insert.bind(
        submission, today, x.board, s.scoreVersion, s.model.key, s.model.display, x.value,
        s.overclock ? 1 : 0, s.appVersion, s.osBuild, s.ramGB, JSON.stringify(s.flags),
      ),
    ),
  );
  return json(201, { ok: true });
}
