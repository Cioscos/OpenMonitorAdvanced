# M8d2 — Classifica nell'app: tabella, Classifica, ▲ di un modello, condivisione ed esportazione: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** la parte dell'app della classifica:
- la tabella dell'autore inclusa e quella della community scaricata dal Worker della M8d1;
- la pagina Classifica;
- la terza voce del riferimento ▲, «Modello della tabella…»;
- «Condividi», con anteprima e invio anonimo;
- «Esporta JSON».

**Architecture:**
- **Regole pure in Rust:** stanno in `oma-core::scores`:
  - `board.rs`: normalizzazione, validazione, tabella, plausibilità e mediana, porting fedele di `scores-worker/src/rules.ts` e `table.ts`;
  - `share.rs`: l'oggetto del §8.5.

  Le fixture comuni di `testdata/scores/` le leggono ora anche i test Rust.
- **Rete:** passa da `oma-win::http`, che riceve `post` e la lettura dell'`ETag`. Il servizio della tabella sta in `app/src-tauri/src/performance/board.rs`, dietro un trait di trasporto che i test sostituiscono.
- **Interfaccia:** Svelte, con le funzioni pure in `app/src/lib/performance/board.ts`, la pagina `BoardPage.svelte` e il dialogo `ShareDialog.svelte`.
- **Ordine dei task:**
  - Z1: regole condivise in Rust;
  - Z2: HTTP e dati della macchina;
  - Z3: oggetto dell'invio e segno «condiviso»;
  - Z4: impostazione `communityTable`;
  - Z5: servizio della tabella;
  - Z6: comandi di condivisione ed esportazione;
  - Z7: pagina Classifica;
  - Z8: ▲ della tabella e dialogo di condivisione;
  - Z9: documenti e prove dal vivo con l'utente.

**Tech Stack:** quelli già in uso:
- Rust 1.90 con `windows` 0.62 (WinHTTP, `SystemInformation`) e `serde_json`;
- Tauri 2.11 con `tauri-plugin-dialog` 2.7;
- Svelte 5 e TypeScript 6 con Vitest;
- per il Worker, solo i test Vitest di Z1.

Nessuna dipendenza nuova.

**Spec:** `docs/superpowers/specs/2026-10-06-m8-prestazioni-design.md`:
- **il §7.6 prima di tutto:** dove contraddice i §7.1–7.5, vale il §7.6;
- **le altre:** §7.1 (tabella), §7.2 (Classifica), §7.5 (download), §8.4 (formato della tabella), §8.5 (oggetto dell'invio), §13.

Il contratto del server è in `docs/benchmark-scoring.md` («Formato dell'invio», «Formato della tabella», «Il Worker»). Le note di parità sono in `docs/follow-ups.md`, «Open: leaderboard server (M8d1)».

## Global Constraints

- **Lingua e formato:**
  - codice, commenti e commit in inglese (conventional commits);
  - documentazione in italiano con gli accenti corretti;
  - fine riga LF;
  - ogni commit termina con le due righe:
    - `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
    - `Claude-Session: https://claude.ai/code/session_01B3yzfCRMU6TL8scsVZQ32s`.
- **Divieti per gli agenti:**
  - mai una richiesta a `scores.cischi.dev` dai test automatici: `POST` mai, `GET` solo nei test `#[ignore = "requires network"]`;
  - mai comandi Cloudflare (`wrangler`, `pnpm run deploy`, `pnpm recompute`);
  - mai push, merge in `main` o release;
  - mai ricerche a tutto il disco;
  - mai comandi elevati;
  - mai clic sintetici o UI Automation sul desktop.
- **Privacy (§7.6):**
  - l'app manda dati solo dopo l'anteprima e «Invia»;
  - il download parte solo con `communityTable` acceso e mentre si usa la vista Prestazioni (apertura della Classifica, fine di un benchmark, «Aggiorna ora»);
  - nessun identificatore nell'invio né nell'esportazione: niente id del punteggio, `deviceId`, seriali, nome del PC o dell'utente.
- **Sicurezza dell'interfaccia:** i nomi dei modelli (scaricati o locali) si mostrano solo come testo (`{…}` di Svelte), mai con `{@html}`.
- **Tooltip:** ogni termine tecnico nuovo ha la sua voce `glossary.*` (in `en.json` e `it.json`) e passa da `Term`. Sono:
  - `board` (classifica);
  - `percentile`;
  - `sourceAuthor` e `sourceCommunity`;
  - `scoreVersion`;
  - `overclock`;
  - `anonymousShare`.
- **Indirizzo del Worker:** `https://scores.cischi.dev`, in un solo punto dell'app (DZ1).
- **TDD e debug:** prima il test che fallisce (`superpowers:test-driven-development`); davanti a un test che fallisce senza un motivo chiaro, `superpowers:systematic-debugging`.
- **graphify:** ogni brief riporta `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"`. Dopo le modifiche al codice: `PYTHONHASHSEED=0 graphify update .`.
- **Comandi di verifica:**
  - `cargo fmt --all --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo test --workspace`;
  - `cd app && pnpm test && pnpm check`;
  - con Z1, anche `cd scores-worker && pnpm test && pnpm check`.

  Atteso: tutto verde. Il test `updates::tests` di `oma-app` è già noto come instabile nell'esecuzione completa: se fallisce lì e passa da solo, lo si scrive nel report e si va avanti.
- **FFI:** un commento `// SAFETY:` su ogni blocco `unsafe`. Dopo Z2 lo controlla l'agente `ffi-safety-reviewer`.

## Decisioni del piano

Precisano il §7.6 dove la spec lascia una scelta.

| # | Decisione | Perché |
|---|---|---|
| DZ1 | **Indirizzo:** in `crates/oma-core/src/scores/board.rs`, `pub const TABLE_URL: &str = "https://scores.cischi.dev/v1/reference-scores.json";` e `pub const SUBMIT_URL: &str = "https://scores.cischi.dev/v1/submit";`, una accanto all'altra, con il commento `// OMA: the Worker's host, as in scores-worker/wrangler.toml routes`. | Il §7.6 vuole un solo punto per parte. |
| DZ2 | **Porting delle regole** (`board.rs`), con gli stessi nomi del TypeScript in snake_case:<br>• `MAX_SUBMIT_BYTES = 16384`, `VALUE_CAP = 100000.0`, `PLAUSIBLE_MIN = 0.2`, `PLAUSIBLE_MAX = 5.0`, `MODEL_MAX = 128`, `MAX_TABLE_BYTES = 1 << 20`.<br>• **Spazi bianchi:** quelli di `\s` e di `trim` di JavaScript, scritti a mano: U+0009–U+000D, U+0020, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000, U+FEFF. Non si usa `char::is_whitespace`, che comprende U+0085 e non U+FEFF.<br>• **Marchi:** `(R)` e `(TM)` si tolgono senza distinguere le maiuscole ASCII, in una passata da sinistra come `replace(/…/gi)`.<br>• **Minuscole:** `str::to_lowercase`.<br>• **Lunghezza:** `MODEL_MAX` conta `encode_utf16().count()`.<br>• **Caratteri vietati nel nome grezzo:** U+0000–U+001F, U+007F–U+009F e la categoria `Cf` da una tabella `CF_RANGES: &[(u32, u32)]`, uguale alla fixture `format-chars.json` (Z1).<br>• **`ramGB`:** basta che sia un numero intero, anche scritto `32.0` (`Number.isInteger` di JavaScript): `as_f64()` con `fract() == 0`.<br>• **Formati:** `osBuild`, `appVersion` e `flags` si controllano a mano con cifre e lettere ASCII, senza il crate `regex`.<br>• **`normalize_model`** si applica una volta sola, come nel server: non è idempotente (`((R)R)`). | Le fixture comuni fissano i comportamenti di JavaScript, e Rust li deve imitare anche dove le librerie standard differiscono (nota (e) della M8d1). |
| DZ3 | **Tabella nell'app:**<br>• righe dell'autore da `include_str!("reference-scores.json")`, lette una volta (`OnceLock`);<br>• righe della community dalla copia scaricata, di cui si tengono **solo** le righe con `source: "community"`.<br>`parse_table` scarta le righe non valide come `parseTable` del Worker; in più rifiuta un `n` che non è un intero ≥ 0 e un file più grande di `MAX_TABLE_BYTES`. | Il Worker pubblica solo righe della community: una riga «autore» scaricata sarebbe falsa. |
| DZ4 | **Oggetto dell'invio** (`share.rs`), nell'ordine dei campi:<br>`format` 1, `appVersion` (quella del file del punteggio), `category`, `scoreVersion`, `valid`, `overclock` (solo nell'invio), `scores`, `kernels` (i `KernelRate` del file così come sono), `hardware { model, ramGB, osBuild }`, `flags`.<br>**`scores`:**<br>• CPU: `{ single, multi }`;<br>• GPU: `{ compute, graphics }`;<br>• disco: `{ points, readMBs, writeMBs }`.<br>**`hardware.model`:** il modello grezzo del file, perché lo normalizza il server.<br>**Testo inviato:** `serde_json::to_vec_pretty` di una struct con `#[derive(Serialize)]` (l'ordine dei campi è quello della struct). Il testo dell'anteprima e i byte inviati sono identici.<br>L'esportazione è lo stesso oggetto senza `overclock` (§8.5). | Quello che l'utente vede è esattamente quello che parte. |
| DZ5 | **Dati della macchina:**<br>• `os_build()`: il `dwBuildNumber` di `RtlGetVersion` come testo (`"26300"`);<br>• `installed_ram_gb()`: `GetPhysicallyInstalledSystemMemory` in KiB arrotondato ai GiB; se la chiamata fallisce, `ullTotalPhys` di `GlobalMemoryStatusEx` arrotondato per eccesso ai GiB; sempre fra 1 e 4096.<br>Si leggono al momento dell'anteprima, dell'invio e dell'esportazione: il file del punteggio non li contiene. | `ullTotalPhys` è un po' meno della RAM installata (31,2 GiB su 32): darebbe un taglio che non esiste. |
| DZ6 | **HTTP** (`oma-win::http`):<br>• `HttpResponse` riceve `etag: Option<String>`, da `WINHTTP_QUERY_ETAG`: `None` se manca o se è più lungo di 256 caratteri;<br>• `post(url, user_agent, headers, body: &[u8], deadline, max_body)`;<br>• `get` mantiene la firma, ed entrambe passano da una funzione privata `send(verb, …)`.<br>Il chiamante passa `Content-Type: application/json`. Le righe di log `update check:` diventano `http:`. | Una sola implementazione di WinHTTP, già provata dalla M6c. |
| DZ7 | **File del download**, nella radice delle Prestazioni (`%LOCALAPPDATA%\OpenMonitorAdvanced\performance\`):<br>• `reference-scores.json`: i byte ricevuti con il `200`, scritti con `crate::overlay::store::write_file` (scrittura atomica);<br>• `reference-scores.state.json`: `{ "format": 1, "etag": string\|null, "checkedAtMs": number\|null, "failedAtMs": number\|null, "error": string\|null }`. | Il percorso della tabella è quello del §7.5; lo stato sta a parte, così la tabella resta il JSON del server. |
| DZ8 | **Quando si scarica** (`fetch_due(state, now_ms, manual) -> bool`, pura):<br>• `manual` → sì;<br>• `failedAtMs` negli ultimi 6 h → no;<br>• `checkedAtMs` nelle ultime 24 h → no;<br>• altrimenti sì.<br>Un istante nel futuro (orologio spostato) non blocca: conta come scaduto.<br>**`If-None-Match`:** si manda solo se la copia salvata si legge bene.<br>**Richiesta:** scadenza 10 s, corpo al massimo `MAX_TABLE_BYTES`, User-Agent `oma_core::updates::user_agent(<versione dell'app>)`.<br>**Esiti:**<br>• `200` valido: tabella, poi stato `{ etag, checkedAtMs: ora, failedAtMs: null, error: null }`;<br>• `304`: `checkedAtMs` = ora;<br>• `304` senza copia, un altro stato o un corpo non valido: errore `http` o `invalid`;<br>• errore di trasporto: la categoria di `CheckError`.<br>Dopo un errore `failedAtMs` = ora, e la tabella buona resta.<br>**Una richiesta alla volta:** un `Mutex` intorno a `refresh`; chi aspetta trova lo stato aggiornato e non rifà la richiesta. | Il §7.5 con le correzioni del §7.6. |
| DZ9 | **Impostazione spenta** (`performance.communityTable: false`): nessuna richiesta, neanche con «Aggiorna ora» (il pulsante è disattivato, con il motivo); la copia già scaricata si continua a usare. | Spenta vuol dire niente rete; i dati già sul disco sono locali. |
| DZ10 | **Comandi Tauri:**<br>• `performance_board() -> BoardTable` (solo locale);<br>• `performance_board_refresh(manual: bool) -> BoardTable`;<br>• `performance_share_preview(id: String, overclock: bool) -> Result<String, String>`;<br>• `performance_share_send(id: String, overclock: bool) -> Result<(), String>`;<br>• `performance_score_export(id: String) -> Result<Option<String>, String>`.<br>`BoardTable { rows: Vec<BoardRow>, communityAt: Option<String>, checkedAtMs: Option<u64>, error: Option<String>, enabled: bool, versions: { cpu, gpu, disk } }`.<br>`BoardRow { board, scoreVersion, model, key, value, n, source }`, con `key` = `normalize_model(model).key` calcolata in Rust. Le righe sono ordinate per `board`, poi per `value` decrescente. | L'interfaccia non normalizza da sé: la regola sta in un solo posto per parte. |
| DZ11 | **Invio** (`performance_share_send`):<br>1. carica il file;<br>2. lo rifiuta con `provisional` se è provvisorio, con `shared` se è già condiviso;<br>3. costruisce i byte (DZ4) e li valida con `validate_submission`, dando il codice di DZ12;<br>4. controlla la plausibilità di ogni valore con la tabella unita (autore e community) → `implausible`, senza inviare;<br>5. fa il `POST` a `SUBMIT_URL` (scadenza 10 s, risposta al massimo 4096 byte).<br>**Esito** (`submit_outcome(status, body) -> Result<(), String>`):<br>• `201` → ok;<br>• `400`, `413`, `429` e `503` con un `{"error": <codice noto>}` → quel codice;<br>• altrimenti `http`.<br>**Se riesce:** `PerformanceStore::mark_shared(id)` riscrive il file con `shared: true`. Se la scrittura fallisce resta un `warn!` nel log, e l'esito è comunque ok. | Lo stesso controllo del server, prima di spendere una richiesta e il rate limit. |
| DZ12 | **Codici d'errore** tradotti dall'interfaccia con `performance.share.error.<codice>`:<br>• del server: `bad_json`, `bad_schema`, `bad_format`, `unknown_version`, `not_valid`, `bad_value`, `implausible`, `body_too_large`, `rate_limited`, `daily_cap`;<br>• del trasporto: `offline`, `timeout`, `tls`, `http`, `invalid`;<br>• dell'app: `provisional`, `shared`, `not_found`.<br>Un codice sconosciuto si mostra con `performance.share.error.unknown`. | Messaggi chiari per ogni caso. |
| DZ13 | **Classifica** (`app/src/lib/performance/board.ts`, pura):<br>• le cinque categorie `BOARDS = ['cpu-single', 'cpu-multi', 'gpu-compute', 'gpu-graphics', 'disk']`, nell'ordine;<br>• **righe proprie:** il miglior punteggio `valid`, non provvisorio, della versione corrente, uno per modello (CPU: `single`/`multi`; GPU: `compute`/`graphics`, uno per GPU; disco: `points`, uno per disco, solo B1);<br>• **vicini:** `NEIGHBOURS = 10` righe in tutto (tabella e proprie, per valore decrescente), con la finestra centrata sulla migliore riga propria (`start = clamp(i − 5, 0, len − 10)`); senza punteggi propri, le prime 10;<br>• **percentile:** `Math.floor(100 × righe della tabella con valore < il migliore proprio / righe della tabella)`, mostrato solo con almeno `PERCENTILE_MIN_ROWS = 10` righe della tabella;<br>• si confrontano solo righe con `scoreVersion` uguale a `versions[categoria]`. | Il §7.2 dice 8–12 modelli: 10 è il centro. |
| DZ14 | **Riferimento ▲ «Modello della tabella…»**:<br>• terza voce del menu, per CPU, GPU e disco;<br>• una seconda tendina elenca le righe della categoria del primo contagiri (per il disco, della categoria `disk`), con l'etichetta `<modello> · <fonte>`;<br>• CPU e GPU: il ▲ di ogni contagiri è la riga con la stessa `key` e la stessa `source` nella sua categoria, `null` se manca;<br>• **disco:** i contagiri sono in MB/s e la tabella in punti, quindi il ▲ del modello va su una **barra dei punti** (`PointsBar.svelte`) sotto i contagiri, al posto della riga «Punti: N». La barra mostra i punti della misura mostrata (`–` per una misura B2) e il ▲ del riferimento scelto: il record dei punti, i punti dell'ultima misura o la riga della tabella. Con «Modello della tabella…» i contagiri del disco tengono il ▲ del record. Fondo scala: il primo multiplo di 500 sopra `max(punti, ▲, 1000)`;<br>• la scelta resta in memoria, come gli altri riferimenti (DB10). | Il disco ha un confronto con un modello senza toccare il server: i MB/s nella tabella (categorie `disk-read` e `disk-write`) restano una possibile aggiunta futura, con migrazione e deploy del Worker (decisione dell'utente del 2026-10-09). |
| DZ15 | **Pagina:** `PerformancePage` riceve `'board'`. La voce di menu «Classifica» è l'ultima del gruppo «Punteggio».<br>All'apertura la pagina chiama `refresh(false)`; alla fine di un benchmark salvato, `benchStore` chiama `boardStore.refresh(false)` se il negozio della tabella è collegato. | Il download parte solo mentre la vista Prestazioni è in uso (§7.5). |
| DZ16 | **Condividi ed Esporta**, sotto il dettaglio della misura mostrata in `ScorePage`:<br>• **«Condividi»** compare se `valid`, non provvisoria e, per il disco, con `points`. Se il punteggio è già condiviso, il pulsante è disattivato con la scritta «Condiviso».<br>• **«Esporta JSON»** c'è per ogni misura salvata. Il file si chiama `oma-score-<categoria>-AAAAMMGG-HHMMSS.json`, con l'ora locale del punteggio, e il dialogo parte nella cartella Documenti, come per lo stress. | La misura mostrata è quella appena fatta o l'ultima del dispositivo. |
| DZ17 | **Testi fissati** (`it.json`; `en.json` con lo stesso senso):<br>• **nota dell'anteprima** (`performance.share.note`): «L'invio è anonimo: nessun account e nessun indirizzo IP salvato. Diventa pubblico solo come mediana di almeno 3 invii. Dopo l'invio non si può più riconoscere né cancellare.»;<br>• **nota dell'impostazione** (`settings.performance.communityTable.note`): «La tabella si scarica da scores.cischi.dev, tramite Cloudflare, al massimo una volta al giorno e solo mentre usi la vista Prestazioni. La richiesta manda l'indirizzo IP e uno User-Agent con la versione dell'app, nient'altro.»;<br>• **didascalia della Classifica:** «Versione {version} · {count} modelli · tabella aggiornata il {date}» oppure «… · tabella inclusa nell'app» (senza una copia scaricata);<br>• **percentile:** «Più veloce del {pct}% dei modelli in tabella». | Le note del §7.6. |

## Review Focus

1. **Copia salvata sparita o guasta, con l'`ETag` ancora nello stato.**
   - **Atteso:** la richiesta parte senza `If-None-Match` e ripristina la tabella. Con l'`If-None-Match` il server risponderebbe `304` per sempre, e la tabella non tornerebbe più.
   - **Test:** Z5, `etag_is_sent_only_with_a_readable_cache`.
2. **Orologio spostato in avanti e poi corretto** (`checkedAtMs` nel futuro).
   - **Atteso:** il download non resta bloccato per giorni.
   - **Test:** Z5, `future_timestamps_make_the_fetch_due`.
3. **Tabella scaricata ostile o guasta:**
   - 2 MB;
   - `format: 2`;
   - `value` `NaN` (come testo) o negativo;
   - righe `source: "author"`;
   - un modello `<img src=x onerror=alert(1)>`.
   - **Atteso:** la copia buona resta; le righe «autore» scaricate si scartano; il nome si vede come testo.
   - **Test:**
     - Z5: `bad_download_keeps_the_good_copy`, `downloaded_author_rows_are_dropped`;
     - Z7: `model_names_render_as_text`.
4. **Nomi di modello Unicode** (NBSP, U+0085, U+FEFF, caratteri fuori dal piano base, sigma finale, `Cf`).
   - **Atteso:** l'app e il server danno la stessa chiave e lo stesso esito.
   - **Test:** Z1, i casi nuovi di `normalize.json`, `submissions.json` e `format-chars.json`, letti da `cargo test` e da Vitest.
5. **Invio che fallisce** (rete assente, `429`, `503`) **o doppio clic su «Invia».**
   - **Atteso:** il punteggio non risulta condiviso, il motivo si legge tradotto, il pulsante torna attivo, e parte un solo `POST`.
   - **Test:**
     - Z6: `failed_send_is_not_marked_shared`;
     - Z8: `share_error_is_translated_and_retry_is_possible`, `send_button_is_disabled_while_sending`.

## Task

### Task Z1: regole condivise in Rust e fixture estese

**Files:**
- Create:
  - `crates/oma-core/src/scores/board.rs`;
  - `testdata/scores/format-chars.json`.
- Modify:
  - `crates/oma-core/src/scores/mod.rs`;
  - `testdata/scores/normalize.json`, `submissions.json`, `README.md`;
  - `scores-worker/test/rules.test.ts`.

**Interfaces:**
- Produces (`oma_core::scores`):
  - `TABLE_URL`, `SUBMIT_URL` (DZ1) e le costanti di DZ2;
  - `enum Board { CpuSingle, CpuMulti, GpuCompute, GpuGraphics, Disk }`, serde `kebab-case`;
  - `enum Source { Author, Community }`, serde `lowercase`;
  - `enum ErrorCode`, serde `snake_case`, con `as_str()` per i codici del server di DZ12;
  - `struct Model { display: String, key: String }`, `fn normalize_model(raw: &str) -> Model`;
  - `struct Submission { category: String, score_version: String, overclock: bool, app_version: String, os_build: String, ram_gb: u32, flags: Vec<String>, model: Model, values: Vec<(Board, f64)> }`;
  - `fn validate_submission(body: &serde_json::Value) -> Result<Submission, ErrorCode>`;
  - `struct TableRow { category: Board, score_version: String, model: String, value: f64, n: u32, source: Source }`, serde camelCase;
  - `struct Table { generated_at: Option<String>, rows: Vec<TableRow> }`, `fn parse_table(bytes: &[u8]) -> Option<Table>`;
  - `fn median(values: &[f64]) -> f64`;
  - `fn plausible(board: Board, version: &str, key: &str, value: f64, rows: &[TableRow]) -> bool`;
  - `fn author_rows() -> &'static [TableRow]`;
  - `fn known_version(category: &str) -> Option<&'static str>`, da `SCORE_VERSION`, `GPU_SCORE_VERSION` e `DISK_SCORE_VERSION`.

- [ ] **Step 1: fixture nuove.**
  - In `normalize.json`, almeno questi casi:
    - `"Intel Core﻿i5"` → `"Intel Core i5"`;
    - `"A\u0085B"` invariato (U+0085 non è uno spazio per JavaScript);
    - `"ΣΊΣΥΦΟΣ GPU"` → la chiave `"σίσυφος gpu"`;
    - `"((R)R) X"` → `"(R) X"`, che documenta che la funzione non è idempotente.
  - In `submissions.json`:
    - nei non validi, `model_astral_65`: 65 volte `😀` (130 unità UTF-16) → `bad_schema`;
    - nei validi, `ram_integral_float`: `"ramGB": 32.0`, scritto così a mano.
  - `format-chars.json`: `{ "cf": [[start, end], …] }`, gli intervalli dei code point `\p{Cf}`. Si generano una volta con uno script Node usa e getta che scorre da 0 a 0x10FFFF; il test di Vitest che segue li confronta con il runtime dei Worker.
  - Il README elenca il file nuovo e dice che `cargo test` legge `normalize.json`, `submissions.json` e `format-chars.json`.
- [ ] **Step 2: test Vitest** in `rules.test.ts`:
  - `format_chars_fixture_matches_the_runtime`: gli intervalli di `\p{Cf}` calcolati nel runtime sono uguali a `cf`.

  Poi `cd scores-worker && pnpm test && pnpm check`. Atteso: PASS. I comportamenti nuovi sono quelli di JavaScript; se un caso fallisce, si corregge la fixture, non il Worker.
- [ ] **Step 3: test Rust che falliscono** (in `board.rs`, con `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/scores/…"))`):
  - `normalize_matches_the_fixture`;
  - `valid_submissions_pass`: con i `values` di DW4 della M8d1 (CPU 2, GPU 2, disco 1 solo `disk`);
  - `invalid_submissions_give_their_code`;
  - `cf_table_matches_the_fixture`: `CF_RANGES` è uguale a `cf`;
  - `rejects_non_object_bodies`: `[]`, `null`, `1`, `"x"` → `BadSchema`;
  - `too_many_kernels_is_bad_schema`;
  - `parse_table_skips_bad_rows_and_rejects_bad_files`: `format: 2` → `None`; più di 1 MiB → `None`; una riga con `value: -1` o `n: 1.5` saltata;
  - `plausibility_matches_the_normalized_model`: con la riga dell'autore `AMD Radeon Graphics` 26, la chiave di `"AMD Radeon(TM) Graphics"` con 140 → `false` (oltre 5 × 26 = 130), con 100 → `true`;
  - `unknown_model_uses_the_category_band`: righe 10 e 1000 → 2 e 5000 plausibili, 1,9 e 5001 no (valori scelti perché 0,2 × 10 e 5 × 1000 sono esatti in virgola mobile);
  - `empty_category_is_plausible`;
  - `author_rows_are_all_author`: `author_rows()` non è vuoto e ha solo `Source::Author`.
- [ ] **Step 4:** `cargo test -p oma-core scores::board`. Atteso: FAIL.
- [ ] **Step 5:** implementare `board.rs` come DZ2 e DZ3, senza dipendenze nuove.
- [ ] **Step 6:** `cargo test -p oma-core`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`. Atteso: PASS.
- [ ] **Step 7: commit** `feat(scores): port the leaderboard rules to oma-core with shared fixtures`.

### Task Z2: HTTP POST, ETag e dati della macchina

**Files:**
- Modify:
  - `crates/oma-win/src/http.rs`;
  - `crates/oma-win/src/os_version.rs`;
  - `crates/oma-win/src/memory.rs`;
  - `crates/oma-win/Cargo.toml`, solo se manca una feature di `windows`.

**Interfaces:**
- Produces:
  - `HttpResponse { status: u16, body: Vec<u8>, etag: Option<String> }`;
  - `pub fn post(url: &str, user_agent: &str, headers: &[(&str, &str)], body: &[u8], deadline: Duration, max_body: usize) -> Result<HttpResponse, CheckError>`;
  - `pub fn os_build() -> Option<String>`;
  - `pub fn installed_ram_gb() -> Option<u32>`;
  - le funzioni pure `ram_gb_from_kib(kib: u64) -> u32` e `ram_gb_from_bytes(bytes: u64) -> u32`, tutte e due fra 1 e 4096.

- [ ] **Step 1: test che falliscono:**
  - `post_rejects_non_https_without_connecting` e `post_rejects_header_line_breaks_without_connecting`, come quelli di `get`;
  - `ram_gb_rounds_to_the_installed_size`: `ram_gb_from_kib(33_554_432)` → 32, `16_777_216` → 16, `33_520_000` → 32, `0` → 1;
  - `ram_gb_from_bytes_rounds_up`: 31,2 GiB → 32, `0` → 1, oltre 4096 GiB → 4096;
  - `os_build_is_digits` (legge questo sistema): da 4 a 6 cifre;
  - un test `#[ignore = "requires network"]`, `table_get_returns_an_etag_and_304`: `GET` di `oma_core::scores::TABLE_URL` → `200` con `etag` `Some`; di nuovo con `If-None-Match` → `304`.
- [ ] **Step 2:** `cargo test -p oma-win`. Atteso: FAIL.
- [ ] **Step 3:** implementare DZ5 e DZ6. Il corpo del `POST` va in `WinHttpSendRequest` (`lpOptional` e `dwTotalLength`). `ERROR_WINHTTP_HEADER_NOT_FOUND` vuol dire `etag: None`.
- [ ] **Step 4:** `cargo test -p oma-win` e `cargo test -p oma-app updates`, poi clippy e fmt. Atteso: PASS. Il test di rete ignorato si lancia una volta a mano (`cargo test -p oma-win table_get -- --ignored`), con il risultato nel report.
- [ ] **Step 5: commit** `feat(http): add POST and ETag to the WinHTTP client, read OS build and installed RAM`.

### Task Z3: oggetto dell'invio e segno «condiviso»

**Files:**
- Create: `crates/oma-core/src/scores/share.rs`.
- Modify:
  - `crates/oma-core/src/scores/file.rs`, `mod.rs`;
  - `app/src-tauri/src/performance/store.rs`;
  - `app/src/lib/types.ts`.

**Interfaces:**
- Consumes: `validate_submission`, `ErrorCode` e `MAX_SUBMIT_BYTES` (Z1).
- Produces:
  - `ScoreFile.shared: bool`, `#[serde(default)]`;
  - `ScoreSummary.shared: bool` e `ScoreSummary.score_version: String`, `#[serde(default)]`;
  - `struct HostFacts { ram_gb: u32, os_build: String }`;
  - `fn export_bytes(file: &ScoreFile, facts: &HostFacts) -> Vec<u8>`;
  - `fn submission_bytes(file: &ScoreFile, facts: &HostFacts, overclock: bool) -> Result<Vec<u8>, ErrorCode>`: valida e controlla `MAX_SUBMIT_BYTES` (`BodyTooLarge`);
  - `fn share_block(file: &ScoreFile) -> Option<&'static str>`: `provisional` o `shared`;
  - `PerformanceStore::mark_shared(&self, id: &str) -> io::Result<bool>`, `false` se il punteggio non c'è;
  - in TypeScript, `shared` e `scoreVersion` su `ScoreFile` e `ScoreSummary`.

- [ ] **Step 1: test che falliscono:**
  - `cpu_submission_passes_the_shared_rules`: un `ScoreFile` della CPU con `single` 1491 e `multi` 1495 → i byte passano `validate_submission`, `hardware.ramGB` e `osBuild` sono quelli di `HostFacts`, e non c'è nessun `id`, `deviceId` né `samples`;
  - `disk_submission_has_points_and_throughput`;
  - `b2_disk_score_is_bad_schema`: senza `points`;
  - `export_has_no_overclock`;
  - `submission_field_order_is_fixed`: il testo comincia con `{\n  "format": 1,\n  "appVersion"`;
  - `share_block_reports_provisional_and_shared`;
  - `old_score_files_read_as_not_shared`: il JSON di un file M8c senza `shared` → `false`;
  - `mark_shared_rewrites_the_file` (in `store.rs`), con `false` per un id che non c'è.
- [ ] **Step 2:** `cargo test -p oma-core scores::share` e `cargo test -p oma-app store`. Atteso: FAIL.
- [ ] **Step 3:** implementare DZ4 e `mark_shared` (`load_score`, `shared = true`, `save_score`), e aggiornare `summary()`.
- [ ] **Step 4:** cargo test, clippy, fmt, `cd app && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(scores): build the share and export object, keep a local shared mark`.

### Task Z4: impostazione `communityTable`

**Files:**
- Modify:
  - `crates/oma-core/src/settings/performance.rs`, `decode.rs`, `patch.rs`, `mod.rs` (test);
  - `app/src/lib/types.ts`, `app/src/lib/backend/mockSettings.ts`;
  - `app/src/components/settings/PerformanceSection.svelte`, `PerformanceSection.test.ts`;
  - `app/src/lib/i18n/en.json`, `it.json`.

**Interfaces:**
- Produces:
  - `PerformanceSettings.community_table: bool`, predefinito `true`, chiave JSON `communityTable`;
  - la patch `{ performance: { communityTable: boolean } }`.

- [ ] **Step 1: test che falliscono:**
  - `performance_defaults_match_the_spec` aggiornato con `"communityTable": true`;
  - un caso di decode con il tipo sbagliato, che dà la diagnostica `performance.communityTable`;
  - un caso di patch;
  - in Vitest, `community_table_toggle_sends_the_patch_and_shows_the_note`: il testo di DZ17 è visibile e il clic manda `{ performance: { communityTable: false } }`.
- [ ] **Step 2:** `cargo test -p oma-core settings`, `cd app && pnpm test PerformanceSection`. Atteso: FAIL.
- [ ] **Step 3:** implementare, seguendo `riskNoticeSeen` per la parte Rust e il `Toggle` di `thermalStop` per l'interfaccia. Il gruppo nuovo si chiama «Classifica» (`settings.performance.group.board`), con l'etichetta `settings.performance.communityTable` e il `Term` `board`.
- [ ] **Step 4:** comandi di verifica. Atteso: PASS.
- [ ] **Step 5: commit** `feat(settings): add the community table switch with its privacy note`.

### Task Z5: servizio della tabella e comandi

**Files:**
- Create: `app/src-tauri/src/performance/board.rs`.
- Modify:
  - `app/src-tauri/src/performance/mod.rs`, `commands.rs`;
  - `app/src-tauri/src/main.rs` (stato gestito e registrazione dei comandi);
  - `app/src/lib/types.ts`;
  - `app/src/lib/backend/backend.ts`, `tauri.ts`, `tauri.test.ts`, `mock.ts`, `mockPerformance.ts`.

**Interfaces:**
- Consumes:
  - `parse_table`, `author_rows`, `normalize_model`, `TABLE_URL` (Z1);
  - `oma_win::http::get` con `etag` (Z2);
  - `settings.performance.community_table` (Z4).
- Produces:
  - `trait BoardTransport: Send + Sync { fn get_table(&self, user_agent: &str, etag: Option<&str>) -> Result<Reply, CheckError>; fn submit(&self, user_agent: &str, body: &[u8]) -> Result<Reply, CheckError>; }`, con `struct Reply { status: u16, body: Vec<u8>, etag: Option<String> }`, e `WinHttpBoard`, l'implementazione vera (`#[cfg(windows)]`, `Offline` altrove);
  - `struct BoardService`, con `new(root: PathBuf, transport: Arc<dyn BoardTransport>, app_version: String)`, `table(&self, enabled: bool) -> BoardTable`, `refresh(&self, enabled: bool, manual: bool, now_ms: u64) -> BoardTable` e `merged_rows(&self) -> Vec<TableRow>` (per la plausibilità in Z6);
  - `fn fetch_due(state: &FetchState, now_ms: u64, manual: bool) -> bool` (DZ8);
  - i comandi `performance_board` e `performance_board_refresh` (DZ10);
  - in TypeScript: `Board`, `BoardRow`, `BoardTable`, e in `Backend` `performanceBoard(): Promise<BoardTable>` e `performanceBoardRefresh(manual: boolean): Promise<BoardTable>`. Il backend finto ha una tabella con almeno 12 righe `cpu-single` (autore e community), così `pnpm dev` mostra il percentile.

- [ ] **Step 1: test che falliscono** (in `board.rs`, con un trasporto finto che conta le chiamate e registra l'`etag`, in una cartella temporanea):
  - `first_refresh_downloads_and_saves`;
  - `refresh_within_24h_does_not_request`;
  - `failure_waits_6h_and_keeps_the_good_copy`;
  - `manual_refresh_always_requests`;
  - `disabled_never_requests_but_uses_the_saved_copy`;
  - `not_modified_updates_checked_at`;
  - `etag_is_sent_only_with_a_readable_cache` (Review Focus 1);
  - `future_timestamps_make_the_fetch_due` (Review Focus 2);
  - `bad_download_keeps_the_good_copy`: 2 MB, `format: 2`, testo non JSON (Review Focus 3);
  - `downloaded_author_rows_are_dropped` (Review Focus 3);
  - `rows_carry_the_normalized_key_and_are_sorted`;
  - `concurrent_refreshes_make_one_request`: due thread → una richiesta.
- [ ] **Step 2:** `cargo test -p oma-app performance::board`. Atteso: FAIL.
- [ ] **Step 3:** implementare DZ7–DZ10. I comandi girano sul pool bloccante, come `performance_system`, e `communityTable` si legge da `SettingsStore` a ogni chiamata.
- [ ] **Step 4: test TypeScript:** in `tauri.test.ts`, la mappatura dei due comandi (`performance_board`; `performance_board_refresh` con `{ manual }`). Poi `pnpm test && pnpm check`.
- [ ] **Step 5:** comandi di verifica. Atteso: PASS.
- [ ] **Step 6: commit** `feat(performance): download and merge the community leaderboard table`.

### Task Z6: comandi di condivisione ed esportazione

**Files:**
- Modify:
  - `app/src-tauri/src/performance/board.rs`, `commands.rs`;
  - `app/src-tauri/src/main.rs`;
  - `crates/oma-core/src/scores/board.rs` (`submit_outcome`);
  - `app/src/lib/backend/backend.ts`, `tauri.ts`, `tauri.test.ts`, `mock.ts`, `mockPerformance.ts`.

**Interfaces:**
- Consumes:
  - `submission_bytes`, `export_bytes`, `share_block` e `mark_shared` (Z3);
  - `BoardService` e `BoardTransport::submit` (Z5);
  - `os_build` e `installed_ram_gb` (Z2).
- Produces:
  - `fn submit_outcome(status: u16, body: &[u8]) -> Result<(), String>` in `oma_core::scores`;
  - `BoardService::share(&self, file: &ScoreFile, facts: &HostFacts, overclock: bool) -> Result<(), String>` (DZ11);
  - i comandi `performance_share_preview`, `performance_share_send` e `performance_score_export` (DZ10, DZ16);
  - in `Backend`: `performanceSharePreview(id: string, overclock: boolean): Promise<string>`, `performanceShareSend(id: string, overclock: boolean): Promise<void>` (rifiuta con il codice di DZ12) e `performanceScoreExport(id: string): Promise<string | null>`.
  - Il backend finto accetta l'invio e segna il punteggio come condiviso; con un modello che contiene `fail` rifiuta con `rate_limited`, per provare il messaggio in `pnpm dev`.

- [ ] **Step 1: test che falliscono:**
  - `submit_outcome_maps_the_worker_answers`: `201`; `400 {"error":"implausible"}` → `implausible`; `429` → `rate_limited`; `503` → `daily_cap`; `400 {"error":"strange"}` → `http`; `500` → `http`;
  - `share_posts_the_previewed_bytes`: il trasporto finto riceve gli stessi byte di `submission_bytes`;
  - `implausible_share_is_refused_without_a_request`: 50000 punti su `cpu-single` per il 7800X3D;
  - `failed_send_is_not_marked_shared` (Review Focus 5): trasporto `Offline` → `offline`, il file resta `shared: false`;
  - `successful_send_marks_the_score_shared`;
  - `provisional_and_shared_scores_are_refused`;
  - `score_export_file_name_uses_local_time` → `oma-score-cpu-20261009-093000.json`.
- [ ] **Step 2:** `cargo test -p oma-core scores::board` e `cargo test -p oma-app performance`. Atteso: FAIL.
- [ ] **Step 3:** implementare. `HostFacts` si legge con `os_build()` e `installed_ram_gb()` al momento della chiamata; se mancano, l'anteprima e l'invio falliscono con `invalid`. L'esportazione segue `export` dello stress (`documents_dir`, `write_report`, il dialogo sulla finestra principale).
- [ ] **Step 4:** in `tauri.test.ts`, la mappatura dei tre comandi. Poi i comandi di verifica. Atteso: PASS.
- [ ] **Step 5: commit** `feat(performance): share a score anonymously and export it as JSON`.

### Task Z7: pagina Classifica

Usare la skill `frontend-design:frontend-design` per la pagina, con lo stile delle altre pagine delle Prestazioni: tabelle `.panel` e barre con i colori della palette Synthwave.

**Files:**
- Create:
  - `app/src/lib/performance/board.ts`, `board.test.ts`;
  - `app/src/lib/performance/board.svelte.ts`;
  - `app/src/components/performance/BoardPage.svelte`, `BoardPage.test.ts`.
- Modify:
  - `app/src/lib/view.ts`;
  - `app/src/components/performance/PerformanceView.svelte`, `PerformanceView.test.ts`;
  - `app/src/lib/performance/bench.svelte.ts`;
  - `app/src/lib/performance/glossary.ts`;
  - `app/src/lib/i18n/en.json`, `it.json`.

**Interfaces:**
- Consumes: `BoardTable`, `BoardRow`, `performanceBoard` e `performanceBoardRefresh` (Z5); `ScoreSummary.scoreVersion` (Z3).
- Produces:
  - `BOARDS`, `NEIGHBOURS = 10`, `PERCENTILE_MIN_ROWS = 10`;
  - `ownRows(scores: ScoreSummary[], board: Board, version: string): OwnRow[]`, con `OwnRow { model: string; value: number; scoreId: string }`;
  - `boardView(table: BoardTable, own: OwnRow[], board: Board): { rows: ViewRow[]; percentile: number | null; models: number }`, con `ViewRow = { kind: 'table'; row: BoardRow } | { kind: 'own'; own: OwnRow }`;
  - `boardStore`, con `table`, `loading`, `error`, `connect(backend): Promise<Unsubscribe>`, `refresh(manual: boolean): Promise<void>` e `rowsFor(board: Board): BoardRow[]` (per Z8).

- [ ] **Step 1: test che falliscono** (`board.test.ts`):
  - `own_rows_take_the_best_valid_current_score_per_model`: un punteggio provvisorio, uno non valido e uno `cpu-0` restano fuori;
  - `neighbours_center_on_the_best_own_row`: 30 righe e la propria al 15° posto → 10 righe, con la propria al 6° posto;
  - `neighbours_clamp_at_the_edges`;
  - `without_own_scores_the_top_ten_show`;
  - `percentile_needs_ten_rows`: 9 righe → `null`; 10 righe e 7 sotto → 70;
  - `other_versions_are_not_compared`;
  - `disk_own_rows_use_points_of_b1_only`.

  In `BoardPage.test.ts`:
  - `tabs_switch_the_category`;
  - `caption_says_bundled_or_updated`;
  - `refresh_now_is_disabled_when_the_setting_is_off`;
  - `opening_the_page_refreshes_once`;
  - `source_badges_have_tooltips`;
  - `model_names_render_as_text` (Review Focus 3): il nome `<img src=x onerror=alert(1)>` compare come testo e non c'è nessun `img`.

  In `PerformanceView.test.ts`: `board_entry_opens_the_board_page`.
- [ ] **Step 2:** `cd app && pnpm test board BoardPage PerformanceView`. Atteso: FAIL.
- [ ] **Step 3: implementare** DZ13, DZ15 e DZ17.
  - **Riga della categoria:** il modello, la barra (larghezza in proporzione al valore più alto della finestra), il valore, il badge della fonte con il `Term`, e `n` in un tooltip.
  - **Riga propria:** è evidenziata e porta «Il tuo punteggio».
  - **Sotto la tabella:** il percentile, poi la didascalia e «Aggiorna ora».
  - **Dopo un errore:** «Non è stato possibile aggiornare la tabella: {motivo}», con i motivi `performance.board.error.<categoria>`.
  - **Voci nuove del glossario:** quelle dei Global Constraints.
- [ ] **Step 4:** `pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): leaderboard page with neighbours, percentile and sources`.

### Task Z8: ▲ «Modello della tabella…» e dialogo di condivisione

Usare la skill `frontend-design:frontend-design`. Il dialogo segue `QuitDialog.svelte`: modale, `Esc` chiude, fuoco al primo controllo e di nuovo sul pulsante di partenza alla chiusura.

**Files:**
- Create:
  - `app/src/components/performance/ShareDialog.svelte`, `ShareDialog.test.ts`;
  - `app/src/components/performance/PointsBar.svelte`, `PointsBar.test.ts`.
- Modify:
  - `app/src/components/performance/ScorePage.svelte`, `ScorePage.test.ts`;
  - `app/src/lib/performance/bench.svelte.ts` (aggiornare i punteggi dopo un invio);
  - `app/src/lib/i18n/en.json`, `it.json`.

**Interfaces:**
- Consumes:
  - `boardStore.rowsFor` (Z7);
  - `performanceSharePreview`, `performanceShareSend` e `performanceScoreExport` (Z6);
  - `ScoreFile.shared` (Z3).
- Produces:
  - `ShareDialog` con le props `{ backend: Backend; scoreId: string; onClose: (shared: boolean) => void }`;
  - `PointsBar` con le props `{ value: number | null; reference: number | null; referenceLabel: string | null }`;
  - `benchStore.pointsFor(target: ScoreTarget): { record: number | null; last: number | null }`, sui punteggi del disco confrontabili (validi, B1, dello stesso disco).

- [ ] **Step 1: test che falliscono.**

  In `ScorePage.test.ts`:
  - `table_reference_lists_rows_of_the_first_gauge`;
  - `table_reference_sets_both_marks_from_the_same_model_and_source`, con il ▲ assente se la seconda categoria non ha la riga;
  - `disk_table_reference_moves_the_points_mark`: con «Modello della tabella…» e la riga `Fanxiang S880 2TB` 995, la barra dei punti ha il ▲ a 995 e i contagiri tengono il ▲ del record;
  - `disk_points_bar_follows_record_and_last`;
  - `points_bar_shows_a_dash_for_b2`;
  - `share_shows_only_for_valid_final_scores`: nascosto per un punteggio provvisorio, uno non valido e un disco B2; per uno condiviso è disattivato con «Condiviso»;
  - `export_saves_and_names_the_file`.

  In `PointsBar.test.ts`:
  - `full_scale_is_the_next_500_above_the_marks`: punti 1012 e ▲ 995 → 1500; punti 300 e ▲ nessuno → 1000;
  - `mark_has_its_label_for_screen_readers`.

  In `ShareDialog.test.ts`:
  - `preview_shows_the_exact_json_and_the_note`;
  - `overclock_box_reloads_the_preview`;
  - `send_success_closes_and_marks_shared`;
  - `share_error_is_translated_and_retry_is_possible` (Review Focus 5);
  - `send_button_is_disabled_while_sending` (Review Focus 5): due clic → una chiamata;
  - `unknown_error_code_uses_the_fallback`.
- [ ] **Step 2:** `cd app && pnpm test ScorePage ShareDialog PointsBar`. Atteso: FAIL.
- [ ] **Step 3: implementare** DZ12, DZ14, DZ16 e DZ17.
  - **Anteprima:** `<pre>` con il testo, la casella «Hardware in overclock» (`Term` `overclock`), la nota (`Term` `anonymousShare`), «Invia» e «Annulla».
  - **Dopo un invio riuscito:** `benchStore.refresh()` e un nuovo caricamento del dettaglio.
- [ ] **Step 4:** `pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): table model reference, share preview and JSON export on the score page`.

### Task Z9: documenti e prove dal vivo con l'utente

**Files:**
- Modify:
  - `docs/benchmark-scoring.md`: la parte dell'app (download, quando e dove si salva, ripiego, condivisione, esportazione, codici d'errore); l'informativa, che ora indica `scores.cischi.dev` per il download;
  - `README.md`: la sezione «Sharing scores» al presente, più il download e l'impostazione per spegnerlo;
  - `docs/follow-ups.md`: una sezione «Open: leaderboard app (M8d2)», e la nota «Sottodominio nell'app» chiusa;
  - `CLAUDE.md`: lo stato della M8d2, i moduli nuovi (`scores::board`, `scores::share`, `performance/board.rs`, `BoardPage`, `ShareDialog`).

- [ ] **Step 1: documenti e commit** `docs: leaderboard in the app (M8d2)`.
- [ ] **Step 2: prove dal vivo.** Le fa l'utente, un passo alla volta; il controller avvia l'app (`cargo build -p oma-overlay -p oma-load`, poi `cd app && pnpm tauri dev`) e legge file e log, senza input sintetico.
  - **Z9.1 Classifica e download:** si apre la Classifica.
    - `reference-scores.json` e `reference-scores.state.json` compaiono nella cartella delle Prestazioni, con un `etag`.
    - La didascalia dice «tabella aggiornata il …».
    - Le righe dell'autore hanno il badge «autore».
  - **Z9.2 «Aggiorna ora»:** `checkedAtMs` cambia; l'`etag` resta lo stesso (`304`).
  - **Z9.3 Impostazione spenta:**
    - «Aggiorna ora» è disattivato;
    - riaprendo la Classifica, lo stato non cambia;
    - la tabella già scaricata si vede ancora.
  - **Z9.4 Senza rete:** l'utente stacca la rete e preme «Aggiorna ora». Si vede il messaggio `offline`, e le righe restano.
  - **Z9.5 ▲ «Modello della tabella…»:** sulle pagine della CPU e della RTX 4080, il ▲ va al valore della riga scelta; sulla pagina del disco, il ▲ va sulla barra dei punti (Fanxiang S880 2TB) e i contagiri tengono quello del record.
  - **Z9.6 Esporta JSON:** il file salvato è identico all'anteprima, senza `overclock`.
  - **Z9.7 Condividi:** su un punteggio della CPU valido. Prima si chiede all'utente se tenere l'invio vero o cancellarlo dopo; la cancellazione da D1 la fa il controller solo con un nuovo via dell'utente.
    - L'anteprima e la nota sono quelle previste.
    - «Invia» dà l'esito positivo, e il pulsante diventa «Condiviso», anche dopo un riavvio dell'app.
    - Prima, con la rete staccata, «Invia» deve dare `offline` e il punteggio non deve risultare condiviso.
  - **Z9.8 Tooltip:** ci sono su tutti i termini nuovi.
  - **Z9.9 Impronta:** `scripts/measure-footprint.ps1` con la Classifica aperta: finestra sotto 200 MB.
- [ ] **Step 3: chiusura.**
  - Gli esiti vanno in `docs/follow-ups.md` e lo stato in `CLAUDE.md`, con il commit `docs: record the M8d2 live checks`.
  - Poi la revisione dell'intero branch e `superpowers:finishing-a-development-branch` (il merge in locale solo su richiesta dell'utente).

**Restano fuori dalla M8d2** (lato server, voci già in `docs/follow-ups.md`):
- `HEAD` che risponde `405`;
- `MIN(model)` al posto della grafia più frequente;
- le note di revisione (a), (b) e (d).

Toccano il Worker e un nuovo deploy, quindi si fanno a parte.
