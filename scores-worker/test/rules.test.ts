import { describe, expect, it } from "vitest";
import normalizeCases from "../../testdata/scores/normalize.json";
import formatChars from "../../testdata/scores/format-chars.json";
import submissions from "../../testdata/scores/submissions.json";
import { normalizeModel, validateSubmission } from "../src/rules.ts";

describe("rules", () => {
  it("normalize_matches_the_fixture", () => {
    for (const c of normalizeCases) expect(normalizeModel(c.input), c.input).toEqual({ display: c.display, key: c.key });
  });

  it("valid_submissions_pass", () => {
    const boards = { cpu: ["cpu-single", "cpu-multi"], gpu: ["gpu-compute", "gpu-graphics"], disk: ["disk"] };
    for (const c of submissions.valid) {
      const r = validateSubmission(c.body);
      expect(r.ok, c.name).toBe(true);
      if (r.ok) expect(r.value.values.map((v) => v.board), c.name).toEqual(boards[r.value.category]);
    }
  });

  it("invalid_submissions_give_their_code", () => {
    for (const c of submissions.invalid) expect(validateSubmission(c.body), c.name).toEqual({ ok: false, error: c.error });
  });

  it("rejects_non_object_bodies", () => {
    for (const b of [[], null, 1, "x"]) expect(validateSubmission(b)).toEqual({ ok: false, error: "bad_schema" });
  });

  it("too_many_kernels_is_bad_schema", () => {
    const body = { ...submissions.valid[0].body, kernels: new Array(33).fill({}) };
    expect(validateSubmission(body)).toEqual({ ok: false, error: "bad_schema" });
  });

  it("disk_values_are_points_only", () => {
    const body = submissions.valid.find((c) => c.name === "disk_with_throughput")!.body;
    const r = validateSubmission(body);
    expect(r.ok && r.value.values).toEqual([{ board: "disk", value: 900 }]);
  });

  it("format_chars_fixture_matches_the_runtime", () => {
    const re = /^\p{Cf}$/u;
    const ranges: number[][] = [];
    let start = -1;
    for (let c = 0; c <= 0x10ffff; c++) {
      const hit = c >= 0xd800 && c <= 0xdfff ? false : re.test(String.fromCodePoint(c));
      if (hit && start < 0) start = c;
      if (!hit && start >= 0) {
        ranges.push([start, c - 1]);
        start = -1;
      }
    }
    if (start >= 0) ranges.push([start, 0x10ffff]);
    expect(ranges).toEqual(formatChars.cf);
  });
});
