// The author's reference rows, bundled from the generated table.
import table from "../../crates/oma-core/src/scores/reference-scores.json";
import { parseTable, type TableRow } from "./table.ts";

export const AUTHOR_ROWS: TableRow[] = parseTable(JSON.stringify(table)) ?? [];
