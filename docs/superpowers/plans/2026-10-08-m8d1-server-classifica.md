# M8d1 — Server della classifica: Cloudflare Worker anonimo: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** il server della classifica, cioè:
- un Cloudflare Worker che riceve gli invii anonimi dei punteggi (`POST /v1/submit`) e pubblica le mediane della community (`GET /v1/reference-scores.json`);
- il database D1 e l'aggregazione giornaliera;
- la tabella dell'autore inclusa nel repository;
- le fixture comuni con l'app;
- la moderazione e l'informativa sulla privacy.

**Architecture:**
- **Cartella `scores-worker/`:** progetto TypeScript a sé, con lockfile proprio. Contiene:
  - le regole pure (`src/rules.ts`: normalizzazione dei modelli e validazione);
  - la tabella e la plausibilità (`src/table.ts`);
  - i gestori HTTP (`src/index.ts`, `src/submit.ts`);
  - l'aggregazione, scritta tutta in SQL (`src/aggregate.ts`), eseguita dal cron e, a mano, da `pnpm recompute`.
- **Fixture comuni:** stanno in `testdata/scores/`. Le legge Vitest ora, e `cargo test` dalla M8d2.
- **Tabella dell'autore:** `crates/oma-core/src/scores/reference-scores.json`, generata dalle misure locali con `pnpm author-table`. La importa il Worker, per la plausibilità; l'app la include dalla M8d2.
- **Ordine dei task:**
  - W1: impalcatura, schema e CI;
  - W2: regole e fixture;
  - W3: tabella e plausibilità;
  - W4: invio;
  - W5: aggregazione e download;
  - W6: documenti;
  - W7: deploy e prove dal vivo con l'utente.

**Tech Stack:**
- per il Worker: TypeScript 6.0.3, Wrangler 4.149.0, `@cloudflare/vitest-plugin` 1.4.0 con Vitest 5.0.3, e `@cloudflare/workers-types` 5.20261008.1;
- la piattaforma: Cloudflare Workers Free e D1 (SQLite: funzioni finestra e JSON);
- per gli script: Node 22 (≥ 22.18, che esegue `.ts` senza flag) e pnpm 10.15.0.

**Spec:** `docs/superpowers/specs/2026-10-06-m8-prestazioni-design.md`. Le sezioni usate sono:
- **il §7.6 prima di tutto:** dove contraddice i §7.1–7.5, vale il §7.6;
- **le altre:** §7.1, §7.2 (la versione del punteggio), §8.4 (formato della tabella), §8.5 (formato dell'invio), §13, §14.

La ricerca è in `docs/superpowers/references/m8/research-gdpr.md` (non è un parere legale) e in `research-leaderboard.md` (§2c, §2d).

**Fatti verificati il 2026-10-08** sui documenti di Cloudflare:
- **Workers Free:**
  - 100.000 richieste al giorno;
  - 10 ms di CPU per richiesta HTTP e per cron;
  - 5 cron per account;
  - 50 query D1 per invocazione.
- **D1 Free:**
  - 5 milioni di righe lette e 100.000 scritte al giorno;
  - 500 MB per database;
  - al massimo 100 parametri per query;
  - la giurisdizione `eu` si sceglie solo alla creazione (`wrangler d1 create … --jurisdiction eu`).
- **Binding di rate limit:**
  - configurazione `[[ratelimits]]` con `simple.limit`, `simple.period` (10 o 60 s) e `namespace_id`; serve Wrangler ≥ 4.36;
  - è permissivo, eventualmente consistente e vale per singola sede di Cloudflare;
  - la disponibilità nel piano gratuito non è scritta nei documenti, e si verifica in W7.
- **Log:** Workers Logs esiste anche nel piano gratuito; noi lo spegniamo.

**Branch:** `feat/m8d1-scores-worker`, da creare da `main` (546edc8). Alla fine il merge in `main` si fa in locale, dopo le prove dal vivo o su richiesta dell'utente. Push e deploy solo su richiesta dell'utente.

**Esecuzione:**
- **Subagent-driven:** un implementer e una revisione per task, poi la revisione dell'intero branch.
- **Revisioni in più:** `security-review` dopo W4 (input dalla rete, rate limit, tetto giornaliero) e dopo W5 (SQL e ETag).
- **Prove dal vivo:** W7, con l'utente.

## Global Constraints

- **Lingua e formato:**
  - codice, commenti e messaggi di commit in inglese (conventional commits);
  - documentazione e prosa in italiano con gli accenti corretti;
  - fine riga LF ovunque;
  - ogni commit termina con le due righe:
    - `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
    - `Claude-Session: https://claude.ai/code/session_01B3yzfCRMU6TL8scsVZQ32s`.
- **Divieti per gli agenti:**
  - mai `wrangler login`, `wrangler deploy`, `wrangler d1 … --remote`, `wrangler secret`, né altri comandi che toccano l'account Cloudflare: li fa l'utente in W7;
  - mai push, merge in `main` o release;
  - mai ricerche a tutto il disco;
  - mai comandi elevati.
- **Anonimato (§7.6):**
  - l'indirizzo IP (`CF-Connecting-IP`) si usa solo come chiave del rate limit: mai salvato in D1, mai in una risposta;
  - nessun `console.*` in `scores-worker/src/`, perché la CI lo controlla;
  - `[observability] enabled = false` in `wrangler.toml`;
  - nessun id restituito al client: l'id `submission` resta interno, solo per raggruppare le righe di un invio;
  - nel database si salva solo il giorno (UTC), mai l'ora.
- **Dipendenze:** solo quelle del Tech Stack, con versioni esatte (niente `^` o `~`) in `scores-worker/package.json`. Nessuna dipendenza a runtime: il Worker usa solo le API della piattaforma. Il Worker non entra nell'installer, quindi non tocca `generate-licenses.ps1`.
- **TDD e debug:** prima il test che fallisce (`superpowers:test-driven-development`); davanti a un test che fallisce senza un motivo chiaro, `superpowers:systematic-debugging`.
- **graphify:** ogni brief riporta `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"`. Dopo le modifiche al codice: `PYTHONHASHSEED=0 graphify update .`.
- **Comandi di verifica** (da `scores-worker/`): `pnpm install --frozen-lockfile`, `pnpm test`, `pnpm check`. Atteso: tutto verde.
- **Confini:**
  - il codice Rust dell'app non cambia nella M8d1. I lettori Rust delle fixture e della tabella arrivano con la M8d2;
  - la sola eccezione è il file di dati `reference-scores.json`, che nessun codice Rust legge ancora.

## Decisioni del piano

Precisano il §7.6 dove la spec lascia una scelta.

| # | Decisione | Perché |
|---|---|---|
| DW1 | **Progetto.**<br>• `scores-worker/package.json`: `name` `oma-scores-worker`, `private`, `type` `module`, `packageManager` `pnpm@10.15.0`.<br>• Script:<br>&nbsp;&nbsp;– `test` = `vitest run`;<br>&nbsp;&nbsp;– `check` = `tsc --noEmit`;<br>&nbsp;&nbsp;– `author-table` = `node scripts/author-table.ts`;<br>&nbsp;&nbsp;– `recompute` = `node scripts/recompute.ts`;<br>&nbsp;&nbsp;– `deploy` = `wrangler deploy`.<br>• `tsconfig.json`: `strict`, `module`/`moduleResolution` `bundler`, `resolveJsonModule`, `types` `["@cloudflare/workers-types", "@cloudflare/vitest-plugin/types"]` (o il nome che il pacchetto documenta per i tipi di `cloudflare:test`), `allowImportingTsExtensions` e `noEmit` (per gli script `.ts` eseguiti da Node).<br>• Gli import relativi hanno sempre l'estensione `.ts`. Si usa solo sintassi che Node sa cancellare: niente `enum`, `namespace` o parameter properties. | Stesso stile dell'app; gli script girano con Node 22.18 senza strumenti in più. |
| DW2 | **`wrangler.toml`:**<br>`name = "oma-scores"`, `main = "src/index.ts"`, `compatibility_date = "2026-10-01"`, `workers_dev = false`;<br>`routes = [{ pattern = "scores.example.invalid", custom_domain = true }]`: il **segnaposto** del sottodominio dell'utente, unico punto da cambiare;<br>`[observability] enabled = false`;<br>`[[d1_databases]]`: `binding = "DB"`, `database_name = "oma-scores"`, `database_id = "00000000-0000-0000-0000-000000000000"` (segnaposto: l'id lo stampa `wrangler d1 create`), `migrations_dir = "migrations"`;<br>`[[ratelimits]]`: `name = "SUBMIT_LIMIT"`, `namespace_id = "1001"`, `simple = { limit = 5, period = 60 }`;<br>`[triggers] crons = ["17 3 * * *"]`.<br>Ogni riga da cambiare porta un commento `# OMA:` che dice cosa metterci. | Un URL proprio permette di cambiare host senza aggiornare l'app (§7.6). 5 invii al minuto per IP bastano a un uso umano. |
| DW3 | **Schema D1** (`migrations/0001_init.sql`):<br>• `entries(id INTEGER PRIMARY KEY, submission TEXT NOT NULL, day TEXT NOT NULL, board TEXT NOT NULL, score_version TEXT NOT NULL, model_key TEXT NOT NULL, model TEXT NOT NULL, value REAL NOT NULL, overclock INTEGER NOT NULL, app_version TEXT NOT NULL, os_build TEXT NOT NULL, ram_gb INTEGER NOT NULL, flags TEXT NOT NULL)`, con gli indici `entries_group(board, score_version, model_key, value)` e `entries_day(day)`;<br>• `hidden_models(model_key TEXT PRIMARY KEY)`;<br>• `published(id INTEGER PRIMARY KEY CHECK (id = 1), body TEXT NOT NULL, etag TEXT NOT NULL)`, con la riga iniziale `(1, '{"format":1,"generatedAt":"2026-10-08T00:00:00Z","rows":[]}', '"empty"')`;<br>• `daily(day TEXT PRIMARY KEY, n INTEGER NOT NULL)`.<br>Una riga di `entries` per ogni valore della classifica: un invio della CPU dà due righe con lo stesso `submission`. | La mediana si calcola in SQL per gruppo (DW8). `submission` (un `crypto.randomUUID()` del server) serve solo a cancellare un invio intero. |
| DW4 | **Categorie della classifica** (`board`), dalla categoria e dai punteggi dell'invio:<br>• `cpu`: `scores.single` → `cpu-single`, `scores.multi` → `cpu-multi`;<br>• `gpu`: `scores.compute` → `gpu-compute`, `scores.graphics` → `gpu-graphics`;<br>• `disk`: `scores.points` → `disk`.<br>`readMBs` e `writeMBs` si accettano ma non si salvano. Versioni note: `cpu` → `cpu-1`, `gpu` → `gpu-1`, `disk` → `disk-1`. | Le cinque categorie del §7.6. Un invio B2 del disco non ha punti, quindi non è condivisibile. |
| DW5 | **Invio** (§8.5 più `valid` e `overclock`):<br>`{ format: 1, appVersion, category, scoreVersion, valid, overclock, scores, kernels, hardware: { model, ramGB, osBuild }, flags }`.<br>**Schema (`bad_schema` se non torna):**<br>• il corpo è un oggetto;<br>• `appVersion` `^\d+\.\d+\.\d+$`, al massimo 16 caratteri;<br>• `category` è uno fra `cpu`, `gpu` e `disk`;<br>• `scoreVersion` è una stringa;<br>• `valid` e `overclock` sono booleani;<br>• `scores` è un oggetto con le chiavi di DW4 come numeri (`null` o assente → `bad_schema`);<br>• `kernels` è un array di al massimo 32 elementi (il contenuto non si legge);<br>• `hardware.model` è una stringa senza caratteri di controllo (U+0000–U+001F, U+007F), che dopo la normalizzazione è lunga da 1 a 128 caratteri;<br>• `hardware.ramGB` è un intero da 1 a 4096;<br>• `hardware.osBuild` `^\d{4,6}(\.\d{1,6})?$`;<br>• `flags` è un array di al massimo 16 stringhe `^[a-z0-9_]{1,32}$`.<br>I campi sconosciuti si ignorano.<br>**Ordine dei controlli:** `bad_schema` → `bad_format` (`format` ≠ 1) → `unknown_version` → `not_valid` (`valid` ≠ `true`) → `bad_value` (un punteggio non finito, ≤ 0 o > `VALUE_CAP`). | Il §8.5 non dice come si segnala un punteggio non valido: `valid` lo rende esplicito, e il server lo ricontrolla. |
| DW6 | **Normalizzazione dei modelli** (`normalizeModel(raw)`):<br>• toglie `(R)`, `(TM)` (senza distinguere maiuscole), `®` e `™`;<br>• riduce ogni sequenza di spazi bianchi a uno spazio e taglia gli estremi.<br>`display` è il risultato, `key` è `display.toLowerCase()`. | «AMD Radeon(TM) Graphics» e «AMD Radeon Graphics» sono lo stesso modello. |
| DW7 | **Plausibilità** (`plausible`):<br>• le righe di confronto sono quelle dell'autore unite alla tabella pubblicata, della stessa categoria e versione;<br>• se c'è il modello (stessa `key`), il riferimento è la mediana dei suoi valori; altrimenti la mediana di tutte le righe della categoria; se non c'è nessuna riga, l'invio è plausibile (il tetto `VALUE_CAP` l'ha già controllato);<br>• l'invio è plausibile se `0,2 × rif ≤ valore ≤ 5 × rif`, estremi compresi.<br>Se `published.body` non si legge come tabella, si usano solo le righe dell'autore. | Si confronta con dati già pubblici, con una sola lettura di D1 per invio. Le righe dell'autore danno un riferimento dal primo giorno. |
| DW8 | **Aggregazione tutta in SQL** (`AGGREGATE_SQL: string[]` in `src/aggregate.ts`), senza parametri e con l'ora di SQLite (`'now'`):<br>1. `DELETE FROM entries WHERE day < date('now', '-24 months')`;<br>2. `DELETE FROM daily WHERE day < date('now', '-7 days')`;<br>3. `INSERT OR REPLACE INTO published (id, body, etag) SELECT 1, json_object('format', 1, 'generatedAt', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 'rows', json((SELECT json_group_array(json_object('category', board, 'scoreVersion', score_version, 'model', model, 'value', value, 'n', n, 'source', 'community')) FROM (<mediane> ORDER BY board, score_version, model_key)))), '"' \|\| strftime('%Y%m%d%H%M%S', 'now') \|\| '"'`.<br>Le mediane: `WITH ranked AS (SELECT board, score_version, model_key, MIN(model) OVER (PARTITION BY board, score_version, model_key) AS model, value, ROW_NUMBER() OVER w AS rn, COUNT(*) OVER (PARTITION BY board, score_version, model_key) AS n FROM entries WHERE overclock = 0 AND model_key NOT IN (SELECT model_key FROM hidden_models) WINDOW w AS (PARTITION BY board, score_version, model_key ORDER BY value)) SELECT board, score_version, model_key, MIN(model) AS model, CAST(ROUND(AVG(value)) AS INTEGER) AS value, MAX(n) AS n FROM ranked WHERE n >= 3 AND rn IN ((n + 1) / 2, (n + 2) / 2) GROUP BY board, score_version, model_key`.<br>Il cron le esegue con `env.DB.batch`; `scripts/recompute.ts` le scrive in `.wrangler/tmp/aggregate.sql` e lancia `wrangler d1 execute oma-scores --remote --file …` (comando dell'utente). | Il lavoro pesa su D1 e non sui 10 ms di CPU del Worker. Il cron e il ricalcolo a mano usano la stessa fonte, senza un endpoint d'amministrazione né segreti. I valori sono punti, quindi interi. |
| DW9 | **Tetto giornaliero:** `DAILY_CAP = 2000` invii accettati al giorno (UTC), contati con `INSERT INTO daily (day, n) VALUES (?, 1) ON CONFLICT (day) DO UPDATE SET n = n + 1 RETURNING n` subito prima dell'inserimento. Oltre il tetto la risposta è `503 daily_cap`, senza inserire niente. | Il rate limit vale per una sola sede: il tetto protegge le 100.000 scritture al giorno di D1 da uno script con molti IP. |
| DW10 | **Risposte** (`application/json; charset=utf-8`):<br>• `201 {"ok":true}`;<br>• errori `{"error":"<codice>"}`:<br>&nbsp;&nbsp;– 400: `bad_json`, `bad_schema`, `bad_format`, `unknown_version`, `not_valid`, `bad_value`, `implausible`;<br>&nbsp;&nbsp;– 413: `body_too_large` (più di `MAX_BODY_BYTES` = 16384 byte, dall'header `Content-Length` o dal corpo letto);<br>&nbsp;&nbsp;– 429: `rate_limited`;<br>&nbsp;&nbsp;– 503: `daily_cap`;<br>&nbsp;&nbsp;– 404: `not_found`;<br>&nbsp;&nbsp;– 405: `method_not_allowed`.<br>**Ordine dell'invio:** metodo e percorso → dimensione → rate limit (chiave `CF-Connecting-IP`, `"unknown"` se manca) → `JSON.parse` → validazione → plausibilità → tetto giornaliero → inserimento.<br>**`GET /v1/reference-scores.json`:**<br>• `200` con `body`, `ETag` ed è `Cache-Control: public, max-age=3600`;<br>• `304` senza corpo se `If-None-Match` è uguale all'`etag`. | Codici stabili che l'app traduce (M8d2). Il rate limit viene prima del parsing, così lo spam costa poca CPU. |
| DW11 | **Tabella dell'autore** (`crates/oma-core/src/scores/reference-scores.json`): `{ "format": 1, "generatedAt", "license": "CC0-1.0", "rows": [...] }`, con righe `source: "author"` ordinate per categoria, versione e `key`.<br>La genera `pnpm author-table [cartella]`, per default `%LOCALAPPDATA%\OpenMonitorAdvanced\performance\scores`, dai file che hanno:<br>• `valid: true`, `provisional: false` e una versione nota;<br>• per il disco, `diskProfile: "b1"` e `points`.<br>Il valore è la mediana arrotondata per `board`, versione e `key`; `n` è il numero di misure; il nome mostrato è il `display` della prima misura. | Le misure dell'autore, non numeri a mano. Le misure provvisorie della CPU, fatte prima della taratura B11, si escludono: le righe della CPU arrivano in W7. |
| DW12 | **Fixture comuni** (`testdata/scores/`, con un `README.md` che dice chi le legge):<br>• `normalize.json`: `[{ input, display, key }]`;<br>• `submissions.json`: `{ valid: [{ name, body }], invalid: [{ name, body, error }] }`;<br>• `aggregate.json`: `{ entries: [{ board, scoreVersion, model, value, overclock }], hidden: [key], expected: <tabella del §8.4 senza generatedAt> }`.<br>Gli invii validi sono anche l'esempio che l'app dovrà produrre (M8d2). | Le regole restano uguali fra Rust e TypeScript (§7.6). |
| DW13 | **CI:** un job `scores-worker` su `ubuntu-latest` in `.github/workflows/ci.yml`, con:<br>• `pnpm/action-setup` e `actions/setup-node` agli stessi SHA di `.github/actions/setup-toolchain`, Node 22, cache su `scores-worker/pnpm-lock.yaml`;<br>• `pnpm install --frozen-lockfile`, `pnpm check`, `pnpm test`;<br>• un passo che fallisce se `grep -rn 'console\.' src` trova qualcosa. | La CI non deve mai toccare Cloudflare (deploy a mano, §7.6). |

## Review Focus

1. **Corpo malformato o ostile**: un array, `null`, una stringa, un JSON annidato, campi enormi, oppure un corpo a pezzi senza `Content-Length` oltre i 16 KB.
   - **Atteso:** un codice 400 o 413, mai un'eccezione né un 500.
   - **Test:**
     - W2: `rejects_non_object_bodies` (array, `null`, numero, stringa → `bad_schema`), `too_many_kernels_is_bad_schema`;
     - W4: `chunked_body_over_limit_is_413`, `invalid_json_is_bad_json`.
2. **Lo stesso modello scritto in modo diverso** da quello dell'autore (maiuscole, spazi, `(TM)`).
   - **Atteso:** la plausibilità usa la mediana di quel modello, non quella della categoria.
   - **Test:** W3, `plausibility_matches_the_normalized_model`.
3. **Soglia dei 3 invii raggiunta solo contando quelli in overclock o nascosti, e mediana con un numero pari di invii.**
   - **Atteso:** il modello resta fuori, e la mediana pari è la media dei due centrali, arrotondata.
   - **Test:** W5, il caso `aggregate.json` con 2 invii normali e 1 in overclock, il caso pari 1500/1501 → 1501, e un modello nascosto.
4. **`published` guasto** dopo un comando di moderazione sbagliato.
   - **Atteso:** l'invio funziona con le sole righe dell'autore; il GET restituisce quello che c'è.
   - **Test:** W4, `broken_published_falls_back_to_author_rows`.
5. **L'IP che finisce dove non deve.**
   - **Atteso:** nessuna colonna, risposta o log con l'IP.
   - **Test:**
     - W4: `stored_row_has_no_ip` (dopo un invio con `CF-Connecting-IP: 203.0.113.7`, nessun campo di `entries` contiene `203.0.113.7`, e la risposta non lo contiene);
     - W1: il passo della CI senza `console.`.

## Task

### Task W1: impalcatura, schema, router e CI

**Files:**
- Create:
  - `scores-worker/package.json`, `pnpm-lock.yaml`, `tsconfig.json`, `wrangler.toml`, `vitest.config.ts`, `.gitignore` (`node_modules/`, `.wrangler/`);
  - `scores-worker/migrations/0001_init.sql`;
  - `scores-worker/src/index.ts`, `src/env.ts`;
  - `scores-worker/test/apply-migrations.ts`, `test/index.test.ts`.
- Modify: `.github/workflows/ci.yml`.

**Interfaces:**
- Produces:
  - `interface Env { DB: D1Database; SUBMIT_LIMIT: RateLimit; }` in `src/env.ts`;
  - `export default { fetch(request, env, ctx), scheduled(controller, env, ctx) }` in `src/index.ts`;
  - `json(status: number, body: unknown, headers?: HeadersInit): Response` in `src/index.ts`;
  - i codici `not_found` e `method_not_allowed` (DW10).
  - `scheduled` è vuoto fino a W5; `POST /v1/submit` risponde `501` fino a W4.

- [ ] **Step 1: configurare** il progetto come DW1–DW3. `vitest.config.ts` segue l'esempio D1 del pacchetto: `cloudflareTest({ wrangler: { configPath: "./wrangler.toml" }, miniflare: { bindings: { TEST_MIGRATIONS: await readD1Migrations("./migrations") } } })` e `setupFiles: ["./test/apply-migrations.ts"]`. Si verificano i nomi esatti delle opzioni con context7 (`/cloudflare/workers-sdk`, «vitest-plugin d1 example»).
- [ ] **Step 2: test che falliscono** (`test/index.test.ts`, con `SELF.fetch` o con l'export di default):
  - `unknown_path_is_404`: `GET /x` → 404 `{"error":"not_found"}`;
  - `wrong_method_is_405`: `PUT /v1/reference-scores.json` → 405;
  - `get_returns_the_seeded_table`: `GET /v1/reference-scores.json` → 200, il corpo è quello della riga iniziale di DW3, `ETag: "empty"`, `Cache-Control: public, max-age=3600`;
  - `matching_etag_is_304`: con `If-None-Match: "empty"` → 304 senza corpo.
- [ ] **Step 3:** `pnpm test`. Atteso: FAIL.
- [ ] **Step 4:** implementare il router, il GET e `json`.
- [ ] **Step 5:**
  - `pnpm test` e `pnpm check`;
  - `pnpm exec wrangler deploy --dry-run --outdir .wrangler/dry`: compila senza toccare l'account; se chiede il login, si salta e lo si scrive nel report;
  - aggiungere il job di DW13 e validarlo con `actionlint` se è installato (`actionlint .github/workflows/ci.yml`), altrimenti con la revisione.

  Atteso: PASS.
- [ ] **Step 6: commit** `feat(scores-worker): scaffold the leaderboard Worker with D1 schema and CI`.

### Task W2: regole condivise e fixture

**Files:**
- Create:
  - `scores-worker/src/rules.ts`;
  - `scores-worker/test/rules.test.ts`;
  - `testdata/scores/README.md`, `normalize.json`, `submissions.json`.

**Interfaces:**
- Produces (in `src/rules.ts`):
  - `export const MAX_BODY_BYTES = 16384`, `VALUE_CAP = 100000`, `PLAUSIBLE_MIN = 0.2`, `PLAUSIBLE_MAX = 5`, `MIN_ENTRIES = 3`, `DAILY_CAP = 2000`, `MODEL_MAX = 128`;
  - `export type Board = "cpu-single" | "cpu-multi" | "gpu-compute" | "gpu-graphics" | "disk"`;
  - `export type ErrorCode` con i codici di DW10;
  - `export function normalizeModel(raw: string): { display: string; key: string }` (DW6);
  - `export interface Submission { category: "cpu" | "gpu" | "disk"; scoreVersion: string; overclock: boolean; appVersion: string; osBuild: string; ramGB: number; flags: string[]; model: { display: string; key: string }; values: { board: Board; value: number }[] }`;
  - `export function validateSubmission(body: unknown): { ok: true; value: Submission } | { ok: false; error: ErrorCode }` (DW4, DW5).

- [ ] **Step 1: fixture.**
  - `normalize.json` ha almeno questi casi:
    - `"AMD Radeon(TM) Graphics"` → `"AMD Radeon Graphics"`;
    - `"  Intel(R) Core(TM) i9-13900K  "` → `"Intel Core i9-13900K"`;
    - `"Intel® Core™ Ultra 9 285K"` → `"Intel Core Ultra 9 285K"`;
    - `"NVIDIA  GeForce\tRTX 4080"` → `"NVIDIA GeForce RTX 4080"`;
    - `"AMD Ryzen 7 7800X3D 8-Core Processor"` invariato.

    Ogni caso ha anche la sua `key`.
  - `submissions.json` ha:
    - un invio valido per categoria;
    - uno valido con un campo sconosciuto;
    - un caso non valido per ogni codice da `bad_schema` a `bad_value`, compresi questi:
      - `hardware.model` con `\u0000`;
      - `model` di 129 caratteri dopo la normalizzazione;
      - `ramGB` 0 e 4097;
      - `osBuild` `"26300.abc"`;
      - `flags` con `"Bad-Flag"`;
      - `scores.multi` a `null`;
      - `points` a 0 e a 100001;
      - `valid: false`;
      - `scoreVersion` `"cpu-2"`;
      - `format: 2`;
      - un invio della CPU con `scoreVersion` `"gpu-1"` (`unknown_version`).
- [ ] **Step 2: test che falliscono** (`test/rules.test.ts`, che importa i JSON di `../../testdata/scores/`):
  - `normalize_matches_the_fixture`;
  - `valid_submissions_pass`: ogni `valid[i]` dà `ok: true`, con le categorie di DW4 nei `values`;
  - `invalid_submissions_give_their_code`;
  - `rejects_non_object_bodies`: `[]`, `null`, `1`, `"x"` → `bad_schema`;
  - `too_many_kernels_is_bad_schema`: 33 elementi;
  - `disk_values_are_points_only`: un invio del disco con `readMBs` dà un solo valore, `disk`.
- [ ] **Step 3:** `pnpm test`. Atteso: FAIL.
- [ ] **Step 4:** implementare `src/rules.ts` senza dipendenze.
- [ ] **Step 5:** `pnpm test` e `pnpm check`. Atteso: PASS.
- [ ] **Step 6: commit** `feat(scores-worker): submission rules with shared fixtures`.

### Task W3: tabella, righe dell'autore e plausibilità

**Files:**
- Create:
  - `scores-worker/src/table.ts`, `src/author.ts`;
  - `scores-worker/scripts/author-table.ts`;
  - `scores-worker/test/table.test.ts`;
  - `crates/oma-core/src/scores/reference-scores.json`.

**Interfaces:**
- Consumes: `normalizeModel`, `Board`, i limiti di `src/rules.ts` (W2).
- Produces (in `src/table.ts`):
  - `export interface TableRow { category: Board; scoreVersion: string; model: string; value: number; n: number; source: "author" | "community" }`;
  - `export function parseTable(text: string): TableRow[] | null` (`null` se non è una tabella del §8.4 con `format: 1`; si scartano le righe con valori non finiti o ≤ 0);
  - `export function median(values: number[]): number`;
  - `export function plausible(board: Board, scoreVersion: string, key: string, value: number, rows: TableRow[]): boolean` (DW7);
  - `export function authorRows(files: unknown[]): TableRow[]` (DW11; i file illeggibili o che non rispettano DW11 si saltano).
- Produces (in `src/author.ts`, a parte, così lo script non importa il file che genera): `export const AUTHOR_ROWS: TableRow[]`, cioè le righe di `reference-scores.json` importato.

- [ ] **Step 1: test che falliscono:**
  - `median_of_odd_and_even`;
  - `parse_table_rejects_garbage`: testo non JSON, `format: 2` e `rows` non array → `null`;
  - `plausibility_uses_the_model_median`: con il modello a 1500, 300 e 7500 passano, 299 e 7501 no;
  - `plausibility_falls_back_to_the_category`;
  - `no_rows_is_plausible`;
  - `plausibility_matches_the_normalized_model`: una riga `"AMD Radeon(TM) Graphics"` vale per la chiave `"amd radeon graphics"`;
  - `author_rows_skip_provisional_and_b2`: con file finti, quelli `provisional` e quelli `b2` non entrano, e la mediana di 995, 996 e 990 è 995.
- [ ] **Step 2:** `pnpm test`. Atteso: FAIL.
- [ ] **Step 3:** implementare `src/table.ts`, poi lo script. Lo script legge con `node:fs` i `*.json` della cartella, chiama `authorRows` e scrive la tabella di DW11 con 2 spazi e LF.
- [ ] **Step 4:** generare la tabella dalla cartella dell'utente, solo in lettura: `pnpm author-table`. Ci si aspettano righe GPU (RTX 4080, AMD Radeon Graphics) e disco (Fanxiang S880 2TB e gli altri dischi con misure B1 non provvisorie), e **nessuna riga CPU** (W7). Se la cartella non c'è, si scrive una tabella con `rows: []` e lo si dice nel report.
- [ ] **Step 5:** `pnpm test` e `pnpm check`. Atteso: PASS.
- [ ] **Step 6: commit** `feat(scores-worker): author reference table and plausibility check`.

### Task W4: `POST /v1/submit`

**Files:**
- Create:
  - `scores-worker/src/submit.ts`;
  - `scores-worker/test/submit.test.ts`.
- Modify: `scores-worker/src/index.ts`.

**Interfaces:**
- Consumes:
  - `validateSubmission` e i limiti (W2);
  - `parseTable`, `plausible` (`src/table.ts`) e `AUTHOR_ROWS` (`src/author.ts`) (W3);
  - `json`, `Env` (W1).
- Produces: `export async function handleSubmit(request: Request, env: Env, today: string): Promise<Response>`. `today` è `YYYY-MM-DD` in UTC; `index.ts` lo calcola da `new Date()`.

- [ ] **Step 1: test che falliscono** (con D1 vero del pool; il rate limit è finto, `env` con `SUBMIT_LIMIT: { limit: async () => ({ success }) }`):
  - `valid_cpu_submission_is_201_with_two_rows`: due righe in `entries`, stesso `submission`, `day` uguale a `today`, `model` normalizzato;
  - `stored_row_has_no_ip` (Review Focus 5);
  - `content_length_over_limit_is_413`;
  - `chunked_body_over_limit_is_413`: corpo da uno stream senza `Content-Length`, di 16385 byte;
  - `invalid_json_is_bad_json`;
  - `rate_limited_is_429_before_parsing`: con `success: false` e un corpo non JSON → 429 (non `bad_json`);
  - `implausible_is_400`: con una riga dell'autore iniettata nella tabella pubblicata;
  - `broken_published_falls_back_to_author_rows`: `published.body` = `"x"` → l'invio valido dà 201;
  - `daily_cap_is_503_and_inserts_nothing`: `daily` per `today` già a `DAILY_CAP` → 503, e `entries` non cambia;
  - `limiter_key_is_the_client_ip`: la chiave passata al limitatore è il valore di `CF-Connecting-IP`, `"unknown"` senza header.
- [ ] **Step 2:** `pnpm test`. Atteso: FAIL.
- [ ] **Step 3:** implementare l'ordine di DW10.
  - **Il tetto** è l'`UPSERT … RETURNING n` di DW9, eseguito da solo. Se `n > DAILY_CAP` la risposta è 503: il rifiuto resta contato, cosa innocua, perché oltre il tetto non entra più niente.
  - **Le righe di un invio** si inseriscono poi in un solo `env.DB.batch`, così un invio non si salva a metà.
- [ ] **Step 4:** `pnpm test` e `pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(scores-worker): anonymous score submission endpoint`.
- [ ] **Step 6:** revisione `security-review` sul diff di W4.

### Task W5: aggregazione giornaliera e ricalcolo a mano

**Files:**
- Create:
  - `scores-worker/src/aggregate.ts`;
  - `scores-worker/scripts/recompute.ts`;
  - `scores-worker/test/aggregate.test.ts`;
  - `testdata/scores/aggregate.json`.
- Modify: `scores-worker/src/index.ts` (`scheduled` → `ctx.waitUntil(env.DB.batch(AGGREGATE_SQL.map(s => env.DB.prepare(s))))`).

**Interfaces:**
- Consumes: `Env` (W1); `parseTable` (W3, nei test).
- Produces: `export const AGGREGATE_SQL: string[]` (DW8).

- [ ] **Step 1: fixture `aggregate.json`.** Contiene:
  - un modello CPU con 3 invii normali (1400, 1500, 1600 → 1500);
  - lo stesso modello in `cpu-multi` con 4 invii (1500, 1501, 1400, 1700 → 1501);
  - un modello con 2 invii normali e 1 in overclock (escluso);
  - un modello con 3 invii ma in `hidden` (escluso);
  - un modello con varianti di maiuscole nello stesso gruppo (il `model` pubblicato è il minimo).
- [ ] **Step 2: test che falliscono:**
  - `aggregate_matches_the_fixture`: si inseriscono le `entries` (con `model_key` da `normalizeModel`), si esegue `AGGREGATE_SQL`, e `parseTable(published.body)` coincide con `expected`; `generatedAt` rispetta `^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$`, e l'`etag` è cambiato;
  - `old_entries_are_deleted`: una riga con `day` `2000-01-01` sparisce, una di oggi resta;
  - `empty_entries_publish_an_empty_table`: `rows: []`;
  - `scheduled_runs_the_aggregation`: con `createScheduledController` e `waitOnExecutionContext`, `published` cambia;
  - `aggregate_file_joins_the_statements`: `aggregateFile(): string` (in `src/aggregate.ts`, non nello script, che importa `node:child_process`) unisce `AGGREGATE_SQL` con `;\n` e finisce con `;\n`.
- [ ] **Step 3:** `pnpm test`. Atteso: FAIL.
- [ ] **Step 4:** implementare DW8. Lo script scrive `.wrangler/tmp/aggregate.sql` e lancia `wrangler d1 execute oma-scores --remote --file .wrangler/tmp/aggregate.sql` con `node:child_process`. Il comando lo lancia l'utente: lo script stampa che tocca il database remoto prima di eseguirlo.
- [ ] **Step 5:** `pnpm test` e `pnpm check`. Atteso: PASS.
- [ ] **Step 6: commit** `feat(scores-worker): daily aggregation of community medians`.
- [ ] **Step 7:** revisione `security-review` sul diff di W5.

### Task W6: documenti

**Files:**
- Create:
  - `docs/benchmark-scoring.md`;
  - `scores-worker/README.md` (in italiano: comandi, segnaposti, deploy).
- Modify:
  - `README.md` (la nota sulla privacy della condivisione, con un link);
  - `CLAUDE.md` (la voce `scores-worker/` in «Struttura», i comandi del Worker in «Comandi», lo stato della M8d1);
  - `docs/follow-ups.md` (la sezione M8d1).

- [ ] **Step 1: `docs/benchmark-scoring.md`**, in italiano, con queste sezioni:
  - **Macchine base e formule:**
    - `cpu-1`, `gpu-1` e `disk-1`, con i file `*-baseline.json` e le macchine di taratura (7800X3D, RTX 4080, Fanxiang S880);
    - la scala a 1500 per CPU e GPU, i 1000 × la media geometrica per il disco.

    Si riprendono dai piani M8a2, M8b2 e M8c, senza inventare.
  - **Le categorie della classifica** (DW4) e la regola «solo punteggi della stessa versione».
  - **Formato dell'invio** (DW5) e **della tabella** (§8.4, DW11).
  - **Il Worker:** gli endpoint, i codici di DW10, il rate limit, il tetto giornaliero e l'aggregazione (DW8, la soglia di 3 invii, overclock esclusi, 24 mesi).
  - **Moderazione**, con i comandi esatti di `wrangler d1 execute oma-scores --remote --command "…"`:
    - vedere gli ultimi invii di un modello;
    - cancellare un invio (`DELETE FROM entries WHERE submission = '…'`);
    - nascondere un modello e togliere l'esclusione (`hidden_models`);
    - ricalcolare la tabella (`pnpm recompute`).
  - **Informativa sulla privacy della condivisione:**
    - titolare: l'autore del progetto, con contatto tramite le issue del repository;
    - dati: quelli di DW5, senza id né IP salvati;
    - finalità: la classifica pubblica;
    - base giuridica: il consenso, con l'invio dopo l'anteprima; per l'IP visto dal rate limit, il legittimo interesse;
    - conservazione: 24 mesi;
    - responsabile del trattamento: Cloudflare (DPA nei termini del servizio; dati di D1 in UE);
    - diritti: siccome un invio è anonimo, l'autore non può più riconoscerlo, quindi non può cancellarlo su richiesta (art. 11 del GDPR);
    - il download della tabella, con IP e User-Agent visti da Cloudflare.

    Si verifica con WebFetch la certificazione DPF di Cloudflare (`https://www.dataprivacyframework.gov/list`); se la pagina non si legge, si scrive «da verificare» e lo si dice nel report.
  - **Deploy**, i comandi dell'utente:
    - `wrangler login`;
    - `wrangler d1 create oma-scores --jurisdiction eu`;
    - i due segnaposti di DW2;
    - `wrangler d1 migrations apply oma-scores --remote`;
    - `wrangler deploy`;
    - la regola di rate limit della zona come ripiego, se il binding non funziona nel piano gratuito.
- [ ] **Step 2:** il README e `CLAUDE.md`. In `docs/follow-ups.md`, la sezione «Open: leaderboard server (M8d1)» con i segnaposti da sostituire, le righe CPU da misurare e le verifiche di W7.
- [ ] **Step 3:** `PYTHONHASHSEED=0 graphify update .`, poi la verifica completa del Worker. Atteso: PASS.
- [ ] **Step 4: commit** `docs: leaderboard server, scoring and privacy notice`.

### Task W7: deploy e prove dal vivo con l'utente

Le esegue il controller con l'utente. I comandi su Cloudflare li lancia l'utente, a uno a uno (`! <comando>` nel prompt). I `curl` verso il Worker pubblicato li può lanciare il controller dopo il via dell'utente, perché sono dati di prova che si cancellano alla fine.

- [ ] **Step 1: righe CPU dell'autore.** Si chiede il permesso prima (carico pesante su tutta la CPU per qualche minuto). L'utente esegue 3 benchmark della CPU con la build release; poi `pnpm author-table`, una revisione delle righe con l'utente (quali dischi tenere) e il commit `chore(scores-worker): author rows for the CPU`.
- [ ] **Step 2: database e indirizzo.**
  - L'utente esegue `pnpm exec wrangler login` e `pnpm exec wrangler d1 create oma-scores --jurisdiction eu`.
  - L'utente sceglie il sottodominio. Il controller mette l'id e il sottodominio in `wrangler.toml` e fa il commit `chore(scores-worker): set the database id and domain`.
  - L'utente esegue `pnpm exec wrangler d1 migrations apply oma-scores --remote` e `pnpm deploy`.
- [ ] **Step 3: prove con `curl`:**
  - `GET` → 200 con la tabella vuota e `ETag`; di nuovo con `If-None-Match` → 304;
  - `POST` dell'invio valido della CPU di `submissions.json`, con il modello `OMA Test CPU` → 201;
  - `POST` di un invio non valido → 400 con il codice;
  - 6 `POST` di fila → almeno un 429. Se non arriva, il binding non funziona nel piano gratuito: si aggiunge la regola di rate limit della zona (`docs/benchmark-scoring.md`) e lo si scrive nei follow-up.
- [ ] **Step 4: aggregazione e moderazione:**
  - 3 invii in tutto per `OMA Test CPU`, poi `pnpm recompute`: il `GET` mostra la riga con `n: 3`;
  - `hidden_models` con la chiave di prova e `pnpm recompute`: la riga sparisce;
  - cancellazione di tutte le righe di prova (`DELETE FROM entries WHERE model_key = 'oma test cpu'`, `DELETE FROM hidden_models …`, `DELETE FROM daily`) e `pnpm recompute`: la tabella torna vuota.
- [ ] **Step 5: privacy.** Nel pannello di Cloudflare, l'utente controlla che i Workers Logs siano spenti e che il database sia nella giurisdizione EU.
- [ ] **Step 6: chiusura.**
  - Gli esiti vanno in `docs/follow-ups.md` e lo stato in `CLAUDE.md`, con il commit `docs: record the M8d1 live checks`.
  - Poi la revisione dell'intero branch e `superpowers:finishing-a-development-branch` (il merge in locale solo su richiesta dell'utente).
