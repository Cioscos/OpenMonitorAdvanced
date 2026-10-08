// Manual recompute of the published table: runs the same SQL as the cron against the REMOTE D1 database.
// Run by the user only (`pnpm recompute`).
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { aggregateFile } from "../src/aggregate.ts";

const FILE = ".wrangler/tmp/aggregate.sql";
mkdirSync(".wrangler/tmp", { recursive: true });
writeFileSync(FILE, aggregateFile());
console.log("This will modify the REMOTE database oma-scores (recompute the published table).");
const r = spawnSync("pnpm", ["exec", "wrangler", "d1", "execute", "oma-scores", "--remote", "--file", FILE], {
  stdio: "inherit",
  shell: true, // pnpm is a .cmd shim on Windows
});
process.exit(r.status ?? 1);
