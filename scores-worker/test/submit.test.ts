import { env } from "cloudflare:test";
import { beforeEach, describe, expect, it } from "vitest";
import submissions from "../../testdata/scores/submissions.json";
import type { Env } from "../src/env.ts";
import { DAILY_CAP, MAX_BODY_BYTES } from "../src/rules.ts";
import { handleSubmit } from "../src/submit.ts";

const TODAY = "2026-10-08";
const VALID = JSON.stringify(submissions.valid.find((v) => v.name === "cpu")!.body);
const SEED = '{"format":1,"generatedAt":"2026-10-08T00:00:00Z","rows":[]}';

function envWith(success: boolean, keys: string[] = []): Env {
  return { ...env, SUBMIT_LIMIT: { limit: async ({ key }: { key: string }) => (keys.push(key), { success }) } };
}
const post = (body: BodyInit | null, headers?: HeadersInit) =>
  new Request("https://scores.test/v1/submit", { method: "POST", body, headers });
const count = async (table: string) =>
  (await env.DB.prepare(`SELECT COUNT(*) AS c FROM ${table}`).first<{ c: number }>())!.c;

beforeEach(async () => {
  await env.DB.batch([
    env.DB.prepare("DELETE FROM entries"),
    env.DB.prepare("DELETE FROM daily"),
    env.DB.prepare("UPDATE published SET body = ? WHERE id = 1").bind(SEED),
  ]);
});

describe("submit", () => {
  it("missing_limiter_skips_rate_limit", async () => {
    const { SUBMIT_LIMIT: _omitted, ...noLimiter } = envWith(false);
    const r = await handleSubmit(post(VALID), noLimiter as Env, TODAY);
    expect(r.status).toBe(201);
  });

  it("valid_cpu_submission_is_201_with_two_rows", async () => {
    const r = await handleSubmit(post(VALID), envWith(true), TODAY);
    expect(r.status).toBe(201);
    expect(await r.json()).toEqual({ ok: true });
    const { results } = await env.DB.prepare("SELECT * FROM entries").all<Record<string, unknown>>();
    expect(results).toHaveLength(2);
    expect(new Set(results.map((x) => x.submission)).size).toBe(1);
    expect(results.every((x) => x.day === TODAY && x.model === "AMD Ryzen 7 7800X3D 8-Core Processor")).toBe(true);
    expect(results.map((x) => x.board).sort()).toEqual(["cpu-multi", "cpu-single"]);
    expect(results[0].flags).toBe("[]");
  });

  it("stored_row_has_no_ip", async () => {
    const r = await handleSubmit(post(VALID, { "CF-Connecting-IP": "203.0.113.7" }), envWith(true), TODAY);
    expect(await r.text()).not.toContain("203.0.113.7");
    const { results } = await env.DB.prepare("SELECT * FROM entries").all();
    expect(results.length).toBe(2);
    expect(JSON.stringify(results)).not.toContain("203.0.113.7");
  });

  it("content_length_over_limit_is_413", async () => {
    const r = await handleSubmit(post(VALID, { "Content-Length": String(MAX_BODY_BYTES + 1) }), envWith(true), TODAY);
    expect(r.status).toBe(413);
    expect(await r.json()).toEqual({ error: "body_too_large" });
  });

  it("chunked_body_over_limit_is_413", async () => {
    const stream = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(new Uint8Array(MAX_BODY_BYTES).fill(32));
        c.enqueue(new Uint8Array(1).fill(32));
        c.close();
      },
    });
    const req = new Request("https://scores.test/v1/submit", {
      method: "POST",
      body: stream,
      duplex: "half",
    } as RequestInit);
    expect(req.headers.get("Content-Length")).toBeNull();
    const r = await handleSubmit(req, envWith(true), TODAY);
    expect(r.status).toBe(413);
  });

  it("invalid_json_is_bad_json", async () => {
    for (const body of ["{nope", null]) {
      const r = await handleSubmit(post(body), envWith(true), TODAY);
      expect(r.status).toBe(400);
      expect(await r.json()).toEqual({ error: "bad_json" });
    }
    const bad = await handleSubmit(post(new Uint8Array([0xff, 0xfe])), envWith(true), TODAY);
    expect(await bad.json()).toEqual({ error: "bad_json" });
  });

  it("rate_limited_is_429_before_parsing", async () => {
    const r = await handleSubmit(post("not json"), envWith(false), TODAY);
    expect(r.status).toBe(429);
    expect(await r.json()).toEqual({ error: "rate_limited" });
  });

  it("implausible_is_400", async () => {
    const table = {
      format: 1,
      generatedAt: "x",
      rows: [{ category: "cpu-single", scoreVersion: "cpu-1", model: "AMD Ryzen 7 7800X3D 8-Core Processor", value: 100, n: 5, source: "community" }],
    };
    await env.DB.prepare("UPDATE published SET body = ? WHERE id = 1").bind(JSON.stringify(table)).run();
    const r = await handleSubmit(post(VALID), envWith(true), TODAY);
    expect(r.status).toBe(400);
    expect(await r.json()).toEqual({ error: "implausible" });
    expect(await count("entries")).toBe(0);
  });

  it("broken_published_falls_back_to_author_rows", async () => {
    await env.DB.prepare("UPDATE published SET body = 'x' WHERE id = 1").run();
    const r = await handleSubmit(post(VALID), envWith(true), TODAY);
    expect(r.status).toBe(201);
  });

  it("daily_cap_is_503_and_inserts_nothing", async () => {
    await env.DB.prepare("INSERT INTO daily (day, n) VALUES (?, ?)").bind(TODAY, DAILY_CAP).run();
    const r = await handleSubmit(post(VALID), envWith(true), TODAY);
    expect(r.status).toBe(503);
    expect(await r.json()).toEqual({ error: "daily_cap" });
    expect(await count("entries")).toBe(0);
  });

  it("limiter_key_is_the_client_ip", async () => {
    const keys: string[] = [];
    await handleSubmit(post("{}", { "CF-Connecting-IP": "198.51.100.9" }), envWith(true, keys), TODAY);
    await handleSubmit(post("{}"), envWith(true, keys), TODAY);
    expect(keys).toEqual(["198.51.100.9", "unknown"]);
  });
});
