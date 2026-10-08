// Daily aggregation (DW8): all in SQL, so the work stays on D1 and not on the Worker's CPU budget.
// Shared by the cron trigger and by scripts/recompute.ts.

const MEDIANS = `WITH ranked AS (
  SELECT board, score_version, model_key,
    MIN(model) OVER (PARTITION BY board, score_version, model_key) AS model,
    value,
    ROW_NUMBER() OVER w AS rn,
    COUNT(*) OVER (PARTITION BY board, score_version, model_key) AS n
  FROM entries
  WHERE overclock = 0 AND NOT EXISTS (SELECT 1 FROM hidden_models h WHERE h.model_key = entries.model_key)
  WINDOW w AS (PARTITION BY board, score_version, model_key ORDER BY value)
)
SELECT board, score_version, model_key, MIN(model) AS model,
  CAST(ROUND(AVG(value)) AS INTEGER) AS value, MAX(n) AS n
FROM ranked
WHERE n >= 3 AND rn IN ((n + 1) / 2, (n + 2) / 2)
GROUP BY board, score_version, model_key`;

export const AGGREGATE_SQL: string[] = [
  "DELETE FROM entries WHERE day < date('now', '-24 months')",
  "DELETE FROM daily WHERE day < date('now', '-7 days')",
  `INSERT OR REPLACE INTO published (id, body, etag)
SELECT 1,
  json_object('format', 1, 'generatedAt', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 'rows',
    json((SELECT json_group_array(json_object('category', board, 'scoreVersion', score_version,
      'model', model, 'value', value, 'n', n, 'source', 'community'))
      FROM (${MEDIANS} ORDER BY board, score_version, model_key)))),
  '"' || strftime('%Y%m%d%H%M%S', 'now') || '"'`,
];

export function aggregateFile(): string {
  return AGGREGATE_SQL.join(";\n") + ";\n";
}
