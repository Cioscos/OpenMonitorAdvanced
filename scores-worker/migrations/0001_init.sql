CREATE TABLE entries (
  id INTEGER PRIMARY KEY,
  submission TEXT NOT NULL,
  day TEXT NOT NULL,
  board TEXT NOT NULL,
  score_version TEXT NOT NULL,
  model_key TEXT NOT NULL,
  model TEXT NOT NULL,
  value REAL NOT NULL,
  overclock INTEGER NOT NULL,
  app_version TEXT NOT NULL,
  os_build TEXT NOT NULL,
  ram_gb INTEGER NOT NULL,
  flags TEXT NOT NULL
);
CREATE INDEX entries_group ON entries (board, score_version, model_key, value);
CREATE INDEX entries_day ON entries (day);

CREATE TABLE hidden_models (model_key TEXT PRIMARY KEY);

CREATE TABLE published (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  body TEXT NOT NULL,
  etag TEXT NOT NULL
);
INSERT INTO published (id, body, etag)
VALUES (1, '{"format":1,"generatedAt":"2026-10-08T00:00:00Z","rows":[]}', '"empty"');

CREATE TABLE daily (day TEXT PRIMARY KEY, n INTEGER NOT NULL);
