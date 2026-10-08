import { AGGREGATE_SQL } from "./aggregate.ts";
import type { Env } from "./env.ts";
import { json } from "./http.ts";
import { handleSubmit } from "./submit.ts";

export { json };

const REFERENCE_PATH = "/v1/reference-scores.json";
const SUBMIT_PATH = "/v1/submit";

async function getReference(request: Request, env: Env): Promise<Response> {
  const row = await env.DB.prepare("SELECT body, etag FROM published WHERE id = 1").first<{
    body: string;
    etag: string;
  }>();
  if (!row) return json(404, { error: "not_found" });
  const headers = { ETag: row.etag, "Cache-Control": "public, max-age=3600" };
  // Weak comparison (RFC 9110): the edge may turn our strong ETag into W/"..." when it compresses.
  const tags = (request.headers.get("If-None-Match") ?? "").split(",").map((t) => t.trim().replace(/^W\//, ""));
  if (tags.includes("*") || tags.includes(row.etag)) return new Response(null, { status: 304, headers });
  return new Response(row.body, {
    status: 200,
    headers: { "Content-Type": "application/json; charset=utf-8", ...headers },
  });
}

export default {
  async fetch(request: Request, env: Env, _ctx: ExecutionContext): Promise<Response> {
    const { pathname } = new URL(request.url);
    if (pathname === REFERENCE_PATH) {
      if (request.method !== "GET") return json(405, { error: "method_not_allowed" });
      return getReference(request, env);
    }
    if (pathname === SUBMIT_PATH) {
      if (request.method !== "POST") return json(405, { error: "method_not_allowed" });
      return handleSubmit(request, env, new Date().toISOString().slice(0, 10));
    }
    return json(404, { error: "not_found" });
  },

  scheduled(_controller: ScheduledController, env: Env, ctx: ExecutionContext): void {
    ctx.waitUntil(env.DB.batch(AGGREGATE_SQL.map((s) => env.DB.prepare(s))));
  },
} satisfies ExportedHandler<Env>;
