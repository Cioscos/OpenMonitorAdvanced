export interface Env {
  DB: D1Database;
  // Optional: absent when the zone rate-limit rule replaces the binding (docs/benchmark-scoring.md).
  SUBMIT_LIMIT?: RateLimit;
}
