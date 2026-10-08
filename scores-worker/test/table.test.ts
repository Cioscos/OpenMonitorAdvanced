import { describe, expect, it } from "vitest";
import { authorRows, median, parseTable, plausible, type TableRow } from "../src/table.ts";

const row = (model: string, value: number, category: TableRow["category"] = "gpu-compute"): TableRow => ({
  category,
  scoreVersion: "gpu-1",
  model,
  value,
  n: 5,
  source: "author",
});

describe("table", () => {
  it("median_of_odd_and_even", () => {
    expect(median([3, 1, 2])).toBe(2);
    expect(median([1500, 1501])).toBe(1500.5);
  });

  it("parse_table_rejects_garbage", () => {
    expect(parseTable("not json")).toBeNull();
    expect(parseTable('{"format":2,"rows":[]}')).toBeNull();
    expect(parseTable('{"format":1,"rows":{}}')).toBeNull();
    expect(parseTable("null")).toBeNull();
  });

  it("parse_table_drops_bad_rows", () => {
    const good = row("A", 10);
    const text = JSON.stringify({ format: 1, rows: [good, { ...good, value: 0 }, { ...good, value: null }, 5] });
    expect(parseTable(text)).toEqual([good]);
  });

  it("plausibility_uses_the_model_median", () => {
    const rows = [row("Card", 1500), row("Other", 100)];
    for (const v of [300, 7500]) expect(plausible("gpu-compute", "gpu-1", "card", v, rows)).toBe(true);
    for (const v of [299, 7501]) expect(plausible("gpu-compute", "gpu-1", "card", v, rows)).toBe(false);
  });

  it("plausibility_falls_back_to_the_category", () => {
    const rows = [row("A", 1000), row("B", 1000), row("C", 1000, "gpu-graphics")];
    expect(plausible("gpu-compute", "gpu-1", "new", 200, rows)).toBe(true);
    expect(plausible("gpu-compute", "gpu-1", "new", 199, rows)).toBe(false);
    expect(plausible("gpu-compute", "gpu-2", "new", 1, rows)).toBe(true);
  });

  it("no_rows_is_plausible", () => {
    expect(plausible("disk", "disk-1", "x", 99999, [])).toBe(true);
  });

  it("plausibility_matches_the_normalized_model", () => {
    const rows = [row("AMD Radeon(TM) Graphics", 100), row("Big", 100000)];
    expect(plausible("gpu-compute", "gpu-1", "amd radeon graphics", 600, rows)).toBe(false);
    expect(plausible("gpu-compute", "gpu-1", "amd radeon graphics", 500, rows)).toBe(true);
  });

  it("author_rows_skip_provisional_and_b2", () => {
    const f = (points: number, extra: object = {}) => ({
      category: "disk",
      scoreVersion: "disk-1",
      provisional: false,
      valid: true,
      diskProfile: "b1",
      scores: { points },
      device: { model: "Fanxiang  S880(TM) 2TB" },
      ...extra,
    });
    const files = [
      f(995),
      f(996),
      f(990),
      f(5, { provisional: true }),
      f(7, { diskProfile: "b2" }),
      f(8, { valid: false }),
      f(9, { scoreVersion: "disk-0" }),
      "junk",
      null,
      { category: "cpu", scoreVersion: "cpu-1", provisional: true, valid: true, scores: { single: 1, multi: 2 }, device: { model: "X" } },
    ];
    expect(authorRows(files)).toEqual([
      { category: "disk", scoreVersion: "disk-1", model: "Fanxiang S880 2TB", value: 995, n: 3, source: "author" },
    ]);
  });
});
