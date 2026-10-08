import type { Env } from "./env.ts";
import { json } from "./http.ts";

export { json };

const REFERENCE_PATH = "/v1/reference-scores.json";
const SUBMIT_PATH = "/v1/submit";

async function getReference(request: Request, env: Env): Promise<Response> {
  const row = await env.DB.prepare("SELECT body, etag FROM published WHERE id = 1").first<{
    body: string;
    etag: string;
  }>();
  if (!row) return json(404, { error: "not_found" });
  if (request.headers.get("If-None-Match") === row.etag) {
    return new Response(null, { status: 304, headers: { ETag: row.etag } });
  }
  return new Response(row.body, {
    status: 200,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      ETag: row.etag,
      "Cache-Control": "public, max-age=3600",
    },
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
      return json(501, { error: "not_implemented" }); // W4
    }
    return json(404, { error: "not_found" });
  },

  async scheduled(_controller: ScheduledController, _env: Env, _ctx: ExecutionContext): Promise<void> {
    // Filled in by W5 (aggregation).
  },
} satisfies ExportedHandler<Env>;
