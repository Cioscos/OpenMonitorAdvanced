// Builds crates/oma-core/src/scores/reference-scores.json from the author's local score files (read-only).
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { authorRows } from "../src/table.ts";

const dir =
  process.argv[2] ?? join(process.env.LOCALAPPDATA ?? "", "OpenMonitorAdvanced", "performance", "scores");
const files: unknown[] = [];
if (existsSync(dir)) {
  for (const name of readdirSync(dir).filter((n) => n.endsWith(".json")).sort()) {
    try {
      files.push(JSON.parse(readFileSync(join(dir, name), "utf8")));
    } catch {
      // unreadable file: skipped
    }
  }
} else {
  console.log(`folder not found: ${dir}; writing an empty table`);
}
const rows = authorRows(files);
const table = { format: 1, generatedAt: new Date().toISOString().replace(/\.\d+Z$/, "Z"), license: "CC0-1.0", rows };
const out = new URL("../../crates/oma-core/src/scores/reference-scores.json", import.meta.url);
writeFileSync(out, JSON.stringify(table, null, 2) + "\n");
console.log(`${rows.length} rows from ${files.length} files`);
for (const r of rows) console.log(`${r.category} ${r.scoreVersion} ${r.model}: ${r.value} (n=${r.n})`);
