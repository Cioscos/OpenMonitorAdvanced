import { SELF } from "cloudflare:test";
import { describe, expect, it } from "vitest";

const SEED = '{"format":1,"generatedAt":"2026-10-08T00:00:00Z","rows":[]}';
const URL_REF = "https://scores.test/v1/reference-scores.json";

describe("router", () => {
  it("unknown_path_is_404", async () => {
    const r = await SELF.fetch("https://scores.test/x");
    expect(r.status).toBe(404);
    expect(await r.json()).toEqual({ error: "not_found" });
  });

  it("wrong_method_is_405", async () => {
    const r = await SELF.fetch(URL_REF, { method: "PUT" });
    expect(r.status).toBe(405);
    expect(await r.json()).toEqual({ error: "method_not_allowed" });
  });

  it("get_returns_the_seeded_table", async () => {
    const r = await SELF.fetch(URL_REF);
    expect(r.status).toBe(200);
    expect(await r.text()).toBe(SEED);
    expect(r.headers.get("ETag")).toBe('"empty"');
    expect(r.headers.get("Cache-Control")).toBe("public, max-age=3600");
    expect(r.headers.get("Content-Type")).toBe("application/json; charset=utf-8");
  });

  it("matching_etag_is_304", async () => {
    const r = await SELF.fetch(URL_REF, { headers: { "If-None-Match": '"empty"' } });
    expect(r.status).toBe(304);
    expect(await r.text()).toBe("");
  });

  it("submit_is_501_until_w4", async () => {
    const r = await SELF.fetch("https://scores.test/v1/submit", { method: "POST", body: "{}" });
    expect(r.status).toBe(501);
  });
});
