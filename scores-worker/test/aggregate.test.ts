import { createExecutionContext, createScheduledController, env, waitOnExecutionContext } from "cloudflare:test";
import { beforeEach, describe, expect, it } from "vitest";
import fixture from "../../testdata/scores/aggregate.json";
import { AGGREGATE_SQL, aggregateFile } from "../src/aggregate.ts";
import worker from "../src/index.ts";
import { normalizeModel } from "../src/rules.ts";
import { parseTable } from "../src/table.ts";

const SEED = '{"format":1,"generatedAt":"2026-10-08T00:00:00Z","rows":[]}';
const today = () => new Date().toISOString().slice(0, 10);
const runAggregate = () => env.DB.batch(AGGREGATE_SQL.map((s) => env.DB.prepare(s)));
const published = () => env.DB.prepare("SELECT body, etag FROM published WHERE id = 1").first<{ body: string; etag: string }>();
const insert = (day: string, board: string, version: string, model: string, value: number, overclock = false) =>
  env.DB.prepare(
    "INSERT INTO entries (submission, day, board, score_version, model_key, model, value, overclock, app_version, os_build, ram_gb, flags) VALUES ('s', ?, ?, ?, ?, ?, ?, ?, '0.0.0', '26100', 16, '[]')",
  ).bind(day, board, version, normalizeModel(model).key, model, value, overclock ? 1 : 0);

beforeEach(async () => {
  await env.DB.batch([
    env.DB.prepare("DELETE FROM entries"),
    env.DB.prepare("DELETE FROM hidden_models"),
    env.DB.prepare("UPDATE published SET body = ?, etag = '\"empty\"' WHERE id = 1").bind(SEED),
  ]);
});

describe("aggregate", () => {
  it("aggregate_matches_the_fixture", async () => {
    await env.DB.batch([
      ...fixture.entries.map((e) => insert(today(), e.board, e.scoreVersion, e.model, e.value, e.overclock)),
      ...fixture.hidden.map((k) => env.DB.prepare("INSERT INTO hidden_models (model_key) VALUES (?)").bind(k)),
    ]);
    await runAggregate();
    const p = (await published())!;
    expect(parseTable(p.body)).toEqual(fixture.expected);
    expect(JSON.parse(p.body).generatedAt).toMatch(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/);
    expect(p.etag).not.toBe('"empty"');
  });

  it("old_entries_are_deleted", async () => {
    await env.DB.batch([insert("2000-01-01", "disk", "disk-1", "Old", 1), insert(today(), "disk", "disk-1", "New", 1)]);
    await runAggregate();
    const { results } = await env.DB.prepare("SELECT day FROM entries").all<{ day: string }>();
    expect(results.map((r) => r.day)).toEqual([today()]);
  });

  it("empty_entries_publish_an_empty_table", async () => {
    await runAggregate();
    expect(JSON.parse((await published())!.body).rows).toEqual([]);
  });

  it("scheduled_runs_the_aggregation", async () => {
    const ctx = createExecutionContext();
    await worker.scheduled(createScheduledController(), env, ctx);
    await waitOnExecutionContext(ctx);
    const p = (await published())!;
    expect(p.etag).not.toBe('"empty"');
    expect(p.body).not.toBe(SEED);
  });

  it("aggregate_file_joins_the_statements", () => {
    expect(aggregateFile()).toBe(AGGREGATE_SQL.join(";\n") + ";\n");
  });
});
