import { env, SELF } from "cloudflare:test";
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

  it("weak_and_listed_etags_are_304", async () => {
    for (const inm of ['W/"empty"', '"x", "empty"', "*"]) {
      const r = await SELF.fetch(URL_REF, { headers: { "If-None-Match": inm } });
      expect(r.status, inm).toBe(304);
    }
  });

  it("304_carries_etag_and_cache_control", async () => {
    const r = await SELF.fetch(URL_REF, { headers: { "If-None-Match": '"empty"' } });
    expect(r.headers.get("ETag")).toBe('"empty"');
    expect(r.headers.get("Cache-Control")).toBe("public, max-age=3600");
  });

  it("other_etag_is_200", async () => {
    const r = await SELF.fetch(URL_REF, { headers: { "If-None-Match": '"other"' } });
    expect(r.status).toBe(200);
  });

  it("rate_limit_binding_is_configured", () => {
    expect(env.SUBMIT_LIMIT).toBeDefined();
  });
});
