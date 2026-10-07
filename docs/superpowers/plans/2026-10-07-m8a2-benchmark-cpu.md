# M8a2 — Benchmark della CPU, contagiri e pagina di punteggio: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** aggiungere alla vista Prestazioni il gruppo «Punteggio» con il benchmark della CPU, che comprende:
- sei carichi verificati a lavoro fisso, in single e in multi core;
- punti su una scala fissa;
- due contagiri in stile C;
- l'elenco delle proprie misure.

**Architecture:**
- **`oma-ipc::load` passa alla versione 2** (B1), con tre aggiunte:
  - fasi a lavoro fisso (`iterations`) con pausa iniziale;
  - una dimensione dei dati `Fixed`, uguale su ogni macchina;
  - tre carichi del benchmark (`hash`, `compress`, `sort`).
- **`oma-load`** esegue le fasi a lavoro fisso e ne misura il tempo di lavoro puro (B2), con le dimensioni fisse dei kernel e i tre carichi nuovi (B3).
- **`oma-core::scores` (puro):** carichi, piano, punteggio, scala dei contagiri e file dei punteggi (B4), più il controller del benchmark, una macchina a stati come `RunController` (B5).
- **`oma-win`:** alimentazione a batteria e quota di CPU degli altri processi (B6).
- **App (Rust):** archivio dei punteggi, avvio del benchmark nel runner esistente, comandi, evento, tray e toast (B7).
- **UI (Svelte):** font e contagiri (B8), pagina «CPU» del gruppo Punteggio (B9).
- **Chiusura:** licenze e documenti (B10), poi la taratura sul 7800X3D e le prove dal vivo con l'utente (B11).

**Tech Stack:** Rust 1.90 (workspace `rust-version` 1.85, `oma-load` 1.89), crate `windows` 0.62, Tauri 2.11, Svelte 5, TypeScript 6, Vitest, PowerShell 7.

**Spec:** `docs/superpowers/specs/2026-10-06-m8-prestazioni-design.md`. Le sezioni usate sono:
- §3.1 (gruppo Punteggio), §3.2, §3.3, §3.9 (toast del benchmark);
- §4.6 (aggiornato insieme a questo piano: scala fissa, DB1);
- §8.2, §11, §12 (font), §13.

Il piano precedente, da imitare, è `docs/superpowers/plans/2026-10-06-m8a1-stress-cpu-ram.md`. Le sue decisioni DA1–DA20 restano valide dove questo piano non le cambia.

**Branch:** `feat/m8a2-cpu-bench` da `main`. Alla fine si fa il merge in `main` in locale; push e release solo su richiesta dell'utente.

**Esecuzione:**
- **Subagent-driven:** un implementer e una revisione per task, poi la revisione dell'intero branch.
- **Revisioni in più:**
  - `ffi-safety-reviewer` dopo B6;
  - `protocol-parity-reviewer` non serve, perché il protocollo `load` non ha una controparte .NET;
  - `security-review` dopo B7, per gli id dei file dei punteggi;
  - `frontend-design:frontend-design` prima di B8 e B9.
- **Prove dal vivo:** B11, con l'utente.

## Global Constraints

Valgono tutti i vincoli globali del piano M8a1, in particolare questi:
- lingua (codice in inglese, documenti in italiano);
- LF;
- ogni commit termina con `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
- nessun input sintetico, nessun installer eseguito, nessun test Pester `Integration`, nessuna ricerca a tutto il disco;
- protocollo con chiavi sempre presenti ed enumerati `snake_case`;
- graphify nei brief;
- nessun crate o pacchetto npm nuovo.

In più:
- **Carico della CPU:**
  - un agente non avvia mai il benchmark vero né `oma-load.exe` a tutti i thread;
  - le misure dei tempi d'iterazione (B3) usano **un solo thread**, al massimo 3 s per carico;
  - la taratura e i benchmark completi si fanno solo in B11, chiesti all'utente.
- **Nomi fissi:**
  - cartella `%LOCALAPPDATA%\OpenMonitorAdvanced\performance\scores\`;
  - file `AAAAMMGG-HHMMSS-<uuid>.json`;
  - versione del punteggio `cpu-1`;
  - evento Tauri `performance-bench`;
  - pagina della vista `score-cpu`;
  - chiavi i18n `performance.score.*` e `glossary.bench.<id>`.
- **Font:** solo file locali in `app/src/assets/fonts/`, con `@font-face` e niente Google Fonts. La CSP non cambia.
- **Budget (§11):** a riposo non cambia niente. Durante il benchmark la finestra resta sotto i 200 MB, e i contagiri si animano solo con la pagina visibile e l'ago in movimento.

## Decisioni del piano

| # | Decisione | Perché |
|---|---|---|
| DB1 | **Scala fissa, senza «macchina base» (decisione dell'utente, 2026-10-07).**<br>• Ogni carico ha due velocità di riferimento, `single` e `multi`, nel file `crates/oma-core/src/scores/cpu-1-baseline.json`.<br>• Punteggio = `round(1500 × media geometrica(velocità / riferimento))`, separato per single e multi.<br>• I riferimenti si tarano una volta sul Ryzen 7 7800X3D dell'autore con il BIOS di fabbrica (B11), quindi su quella CPU il punteggio vale 1500.<br>• L'interfaccia non nomina la CPU di taratura: il tooltip dice «punti su una scala fissa».<br>• Fino alla taratura il file ha valori provvisori (B4), con `"provisional": true`.<br>Sostituisce «macchina base = 1000» del §4.6. | Una CPU medio-alta a 1000 farebbe sembrare «sotto la media» quasi tutte le altre. Con 1500 una CPU recente a 6 core cade intorno a 1000. |
| DB2 | **Dimensioni fisse, uguali su ogni macchina** (`DataSize::Fixed`).<br>• **K2:** FFT di 4096 complessi f64.<br>• **K5:** NTT di 2^15 punti.<br>• **K7:** GEMM con n = 256.<br>• **`hash`:** SHA-256 e CRC32C dello stesso buffer da 1 MiB.<br>• **`compress`:** il LZ di K8, compressione e decompressione di 4 blocchi da 256 KiB.<br>• **`sort`:** il quicksort di K8 su 262 144 `u32`.<br>I dati vengono dal seme, come nello stress test. | Nello stress test le dimensioni seguono la cache: un benchmark confrontabile deve dare lo stesso lavoro a ogni CPU. |
| DB3 | **I sei carichi** (§4.6), con id, unità e lavoro per iterazione:<br>• interi: `ntt` (K5, Mop/s, farfalle), `hash` (MB/s del buffer), `compress` (MB/s dei dati in ingresso), `sort` (Melem/s);<br>• virgola mobile: `fft` (K2, GFLOP/s, 5·N·log₂N per trasformata), `gemm` (K7, GFLOP/s, 2·n³ per prodotto).<br>Il lavoro esatto di un'iterazione lo ricava l'implementer di B3 dal codice del kernel; lo scrive come costante `WORK_PER_ITERATION` in `oma-core::scores::workloads`, con un commento che lo spiega.<br>`hash` usa SHA-256 e CRC32C già verificati in K8, non xxHash. | Si riusa codice già verificato con i vettori noti. Il §4.6 cita xxHash solo come esempio. |
| DB4 | **Svolgimento** (§4.6):<br>• prima i sei carichi in single, poi i sei in multi;<br>• per ogni carico, una fase di riscaldamento e tre ripetizioni, tutte a lavoro fisso;<br>• la fase di riscaldamento di ogni carico ha `pause_before_ms = 2000`;<br>• **single:** `Placement::OnePerCore` con `cores = [core_order(topology)[0]]`, cioè il primo core della classe `EfficiencyClass` più alta;<br>• **multi:** `Placement::AllLogical`, una copia indipendente per processore logico, ognuna con le stesse `iterations`;<br>• set d'istruzioni: il migliore della CPU (`detected_isa()[0]`), salvato nel punteggio;<br>• `stop_on_error = true` in ogni fase.<br>48 fasi in tutto. | Scaletta del §4.6, resa un piano di fasi che `oma-load` sa già eseguire. |
| DB5 | **Lavoro fisso e tempo** (B2):<br>• `iterations` è il numero di iterazioni di ogni thread;<br>• il tempo di lavoro `work_ms` va dall'apertura del cancello all'ultimo thread finito, quindi esclude il calcolo del riferimento e la pausa;<br>• `duration_s` è il tetto, 30 s per fase: arrivati al tetto, la fase finisce con i thread che si fermano e `skipped = None`;<br>• velocità della ripetizione = `checks` / `work_ms`, in iterazioni al secondo di tutti i thread.<br>Le iterazioni per carico (`ITERATIONS`, in `workloads`) si scelgono in B4 perché una ripetizione single duri circa 1 s su questo PC, misurata con un thread in B3.<br>Totale atteso sul 7800X3D: circa 2 minuti. | Con il tetto, una CPU molto lenta finisce lo stesso e la velocità resta giusta: diventa una misura a tempo fisso solo per lei. |
| DB6 | **Punteggio, mediana e scala:**<br>• per ogni carico e modo si tiene la mediana delle tre ripetizioni;<br>• scala = media geometrica sui sei carichi di `multi / (single × processori logici)`, mostrata in percentuale;<br>• contagiri (§3.3):<br>&nbsp;&nbsp;– fondo scala = il primo numero della serie 1-2-2,5-5 × 10ⁿ maggiore o uguale a 1,1 × max(record, riferimento ▲, 1500);<br>&nbsp;&nbsp;– durante la misura il fondo scala può solo crescere: se un valore lo supera, si ricalcola con il valore al posto del massimo;<br>• ago dal vivo = 1500 × `rate` / riferimento del carico e del modo in corso, con `rate` di `Progress`.<br>`round` è l'arrotondamento al più vicino. | §3.3 e §4.6, con 1500 al posto della stima della macchina base. |
| DB7 | **Validità (§4.6):**<br>• **non valida** (`valid = false`): un `Error` di calcolo o un thread bloccato, con il messaggio della spec e il flag `compute_error`; il punteggio si salva lo stesso, con i carichi fatti fino a lì;<br>• **valida con avviso**, flag in `flags`:<br>&nbsp;&nbsp;– `battery`: il PC va a batteria all'inizio o in un campione;<br>&nbsp;&nbsp;– `thermal_throttle`: un campione con la temperatura ≥ Tjmax − 2, oppure ≥ 93 °C senza Tjmax. Non c'è un sensore di throttling della CPU, quindi si usa la temperatura;<br>&nbsp;&nbsp;– `busy_system`: altri processi sopra il 10% della CPU totale, misurato all'avvio e poi ogni 5 s;<br>&nbsp;&nbsp;– `virtual_machine`: `Topology.hypervisor`;<br>&nbsp;&nbsp;– `no_sensors`: servizio assente, quindi niente controllo termico.<br>• **Fermato** dall'utente, dalla tray o all'uscita: non si salva niente.<br>• **Non avviato** (`oma-load` assente o incompatibile): nessun file, e la pagina mostra l'errore. | Una misura fermata a metà non è una misura. Una misura con un errore va mostrata, perché l'errore è un'informazione. |
| DB8 | **Un solo lavoro alla volta.** Benchmark e stress test usano lo stesso `oma-load` e lo stesso slot `active` del runner: avviarne uno mentre l'altro gira dà `StartError::Busy` (`busy`). | Due carichi insieme falserebbero entrambi. |
| DB9 | **Tray, finestra e uscita:**<br>• durante il benchmark l'icona ha il punto di DA17 e il tooltip «Benchmark della CPU in corso»;<br>• «Ferma il test» ferma anche il benchmark; «Apri il test in corso» apre `score-cpu`;<br>• chiudere la finestra lascia andare il benchmark;<br>• il toast finale arriva solo se la finestra non è visibile (§3.9), e un clic apre `score-cpu`;<br>• «Esci» e `--quit` fermano il benchmark senza chiedere, senza salvare niente. | Il benchmark dura 2 minuti: non vale una domanda di conferma. |
| DB10 | **Rinviati alla M8d:**<br>• «Esporta JSON» e «Condividi su GitHub», che usano il formato del §8.5;<br>• il riferimento ▲ «modello della tabella».<br>Nella M8a2 il menu del riferimento ha «Il mio record» (predefinito) e «L'ultima misura». | Esportazione e condivisione hanno senso con la classifica. |
| DB11 | **Il file del punteggio** (§8.2), JSON camelCase con `format: 1`:<br>• `id`, `at`, `category: "cpu"`, `scoreVersion: "cpu-1"`, `provisional`, `isa`;<br>• `scores: { single, multi }`, entrambi `Option<u32>`;<br>• `kernels: [{ id, unit, single, multi }]`, con le velocità in unità vere, `Option<f64>`;<br>• `device: { model, cores, logical }`;<br>• `flags`, `valid`, `scaling`;<br>• `samples`: uno ogni 5 s, `{ tMs, tempC, powerW, clockMhz }`;<br>• `appVersion`, `loadVersion`.<br>Si scrive in modo atomico, con il file temporaneo dello store. Si legge con tolleranza: i file illeggibili o di un formato futuro si saltano con una riga nel log, e se ne tengono al massimo 500. | Lo stesso trattamento delle sessioni di stress. |

## Review Focus

1. **Sessioni di stress salvate dalla M8a1** (`plan.phases` senza `iterations` né `pause_before_ms`). Atteso: la cronologia le legge ancora.

   Test: B1, `v1_phase_without_bench_fields_still_parses` (con `#[serde(default)]` sui campi nuovi in lettura; la scrittura li mette sempre).
2. **CPU molto lenta o macchina virtuale con pochi core.** Atteso: ogni fase finisce al tetto di 30 s con una velocità giusta, il benchmark non si blocca e il punteggio esce.

   Test: B2, `fixed_work_phase_stops_at_the_cap_and_reports_work_ms`; B5, `capped_reps_still_score`.
3. **Un solo core logico, oppure topologia senza classi di efficienza.** Atteso: il single usa il core 0, la scala vale 100% e non c'è nessuna divisione per zero.

   Test: B4, `bench_plan_on_a_single_logical_cpu`, `scaling_with_one_logical_is_one`.
4. **Velocità nulle o assurde da un messaggio** (`rate` 0, `work_ms` 0, `checks` 0). Atteso: nessun `NaN` né infinito nel punteggio, la ripetizione si scarta e il carico conta come non misurato.

   Test: B4, `zero_work_ms_rep_is_ignored`, `score_without_all_six_kernels_is_none`.
5. **File dei punteggi ostili:** un id con `..\`, un JSON troncato, un `format: 2`. Atteso: rifiuto o salto con una riga nel log, e nessun file toccato fuori dalla cartella.

   Test: B7, `score_ids_outside_the_uuid_form_are_rejected`, `corrupt_and_future_scores_are_skipped`.

## Tabelle dei testi (italiano esatto)

### T1. Pagina, contagiri e misure (`performance.score.*`, `performance.nav.*`)

| Chiave | Testo |
|---|---|
| `performance.nav.score` | Punteggio |
| `performance.nav.scoreCpu` | CPU |
| `performance.score.title` | CPU Benchmark |
| `performance.score.start` | Avvia |
| `performance.score.stop` | Ferma |
| `performance.score.single` | Single core |
| `performance.score.multi` | Multi core |
| `performance.score.points` | punti |
| `performance.score.scaling` | Scala multi core: {pct}% |
| `performance.score.duration` | Circa 2 minuti. Chiudi le applicazioni pesanti prima di iniziare. |
| `performance.score.reference` | Riferimento ▲ |
| `performance.score.reference.record` | Il mio record |
| `performance.score.reference.last` | L'ultima misura |
| `performance.score.detail` | Dettaglio dei carichi |
| `performance.score.col.kernel` | Carico |
| `performance.score.history` | Le tue misure |
| `performance.score.history.empty` | Nessuna misura ancora. |
| `performance.score.delete` | Elimina |
| `performance.score.invalid` | Errore di calcolo durante il benchmark: prova lo stress test. |
| `performance.score.flag.battery` | Misura fatta a batteria: il punteggio può essere più basso. |
| `performance.score.flag.thermal_throttle` | La CPU ha raggiunto la temperatura massima: il punteggio può essere più basso. |
| `performance.score.flag.busy_system` | Altri programmi usavano più del 10% della CPU durante la misura. |
| `performance.score.flag.virtual_machine` | Misura fatta in una macchina virtuale. |
| `performance.score.flag.no_sensors` | Servizio non attivo: temperatura non controllata durante la misura. |
| `performance.score.provisional` | Scala non ancora tarata: i punti cambieranno. |
| `performance.score.error.busy` | È già in corso uno stress test o un benchmark. |
| `performance.score.error.start` | Il benchmark non è partito: {reason}. Prova a reinstallare l'app. |
| `performance.score.phase` | {kernel} · {mode} · {rep} |
| `performance.score.warmup` | riscaldamento |
| `performance.score.rep` | ripetizione {n} di 3 |
| `performance.toast.benchDone` | Benchmark finito: {single} single core, {multi} multi core. |
| `performance.toast.benchInvalid` | Benchmark non valido: errore di calcolo. |
| `tray.benchRunning` | Benchmark della CPU in corso |

### T2. Glossario (`glossary.<termine>`, con `.name`)

| Chiave | `.name` | Spiegazione |
|---|---|---|
| `benchPoints` | punti | Punti su una scala fissa, uguale per tutte le CPU: il doppio dei punti vuol dire il doppio del lavoro fatto nello stesso tempo. |
| `singleCore` | single core | Un solo carico su un solo core, il più veloce: dice quanto è rapido un core da solo, come nei giochi e nei programmi che usano pochi thread. |
| `multiCore` | multi core | Una copia del carico su ogni thread della CPU, tutte insieme: dice quanto lavoro fa la CPU intera. |
| `scaling` | scala multi core | Quanto del lavoro teorico arriva davvero quando lavorano tutti i thread: 100% vorrebbe dire che ogni thread va veloce come da solo. Lo abbassano gli SMT, il calore e la memoria condivisa. |
| `referenceMark` | riferimento ▲ | Il segno sulla ghiera: mostra dove arriva la misura scelta nel menu, per confrontarla con quella di adesso. |
| `warmup` | riscaldamento | Un primo giro che non conta: porta la CPU al regime e riempie le cache. |
| `median` | mediana | Delle tre ripetizioni si tiene quella centrale: un disturbo isolato non cambia il risultato. |

`glossary.bench.<id>` per i sei carichi, con `.name`:

| Id | `.name` | Spiegazione |
|---|---|---|
| `ntt` | NTT (interi) | Trasformata con aritmetica intera esatta a 64 bit: misura il moltiplicatore intero. |
| `hash` | Hash | Calcola le impronte SHA-256 e CRC32C di un blocco di dati: misura le istruzioni dedicate di crittografia e controllo. |
| `compress` | Compressione | Comprime e decomprime testo: misura la gestione dei salti e della memoria vicina, come quando si apre un archivio. |
| `sort` | Ordinamento | Mette in ordine un milione di byte di numeri: misura i salti difficili da prevedere. |
| `fft` | FFT (virgola mobile) | Trasformata di Fourier in doppia precisione: misura le unità di calcolo in virgola mobile, come nell'audio e nella simulazione. |
| `gemm` | Prodotto di matrici | Moltiplica matrici in doppia precisione: misura la potenza di calcolo vettoriale sostenuta. |

## Task

### Task B1: protocollo `oma-ipc::load` versione 2

**Files:**
- Modify:
  - `crates/oma-ipc/src/load.rs`;
  - `crates/oma-ipc/tests/load_fixtures.rs`;
  - `protocol/fixtures/load/*.msgpack` (rigenerate);
  - i costruttori di `Phase` in `crates/oma-core/src/load/plan.rs` e nei test, che mettono i valori neutri.

**Interfaces:**
- Produces:
  - `LOAD_PROTOCOL_VERSION = 2`;
  - `Phase.iterations: Option<u64>` e `Phase.pause_before_ms: u32`, entrambi con `#[serde(default)]`;
  - `DataSize::Fixed`;
  - `KernelId::{Hash, Compress, Sort}`, in serializzazione `"hash"`, `"compress"`, `"sort"`;
  - `PhaseDone.work_ms: Option<u64>`, con `#[serde(default)]`.
  - **`validate`:**
    - `iterations` va da 1 a 10⁹;
    - con `iterations` impostato servono `mode == Steady` e `placement != CoreCycle`;
    - `pause_before_ms` è al massimo 10 000;
    - `Hash`, `Compress` e `Sort` sono validi solo con `iterations`;
    - `Fixed` è valido solo per K2, K5, K7 e i tre carichi nuovi.

- [ ] **Step 1: test che falliscono:**
  - `bench_phase_round_trips`;
  - `iterations_out_of_range_is_rejected`;
  - `iterations_need_steady_and_no_core_cycle`;
  - `bench_kernels_need_iterations`;
  - `fixed_size_only_for_bench_kernels`;
  - `v1_phase_without_bench_fields_still_parses`: un JSON di `Phase` senza i campi nuovi dà `iterations: None` e `pause_before_ms: 0`;
  - `phase_done_without_work_ms_parses`.
- [ ] **Step 2:** `cargo test -p oma-ipc load`. Atteso: FAIL.
- [ ] **Step 3:** implementare, poi rigenerare le fixture con `OMA_WRITE_FIXTURES=1 cargo test -p oma-ipc --test load_fixtures -- --test-threads=1`.
- [ ] **Step 4:** `cargo test --workspace` e clippy. Atteso: PASS, con `oma-load` che compila (i match sui `KernelId` nuovi danno `Unsupported` fino a B3).
- [ ] **Step 5: commit** `feat(ipc): load protocol v2 with fixed-work bench phases`.

### Task B2: `oma-load`, fasi a lavoro fisso

**Files:**
- Modify:
  - `crates/oma-load/src/engine/mod.rs` (`attempt`, `monitor`, `worker`, `step`);
  - `crates/oma-load/src/engine/modes.rs`;
  - `crates/oma-load/src/engine/tests.rs`.

**Interfaces:**
- Consumes: B1.
- Produces:
  - prima di una fase con `pause_before_ms > 0`, il motore dorme quel tempo, senza thread di carico e senza battiti contati come blocco: la sentinella non segnala niente;
  - con `iterations = Some(n)` ogni worker esegue esattamente n `step` in `Steady`, poi si ferma e lo segnala; `monitor` finisce quando tutti hanno finito, oppure al tetto `duration_s`, a uno stop o al primo errore;
  - `PhaseDone.work_ms = Some(ms dall'apertura del cancello all'ultimo worker finito o al tetto)` per queste fasi, `None` per le altre;
  - `checks` della fase = iterazioni fatte da tutti i worker;
  - `Progress.rate` resta com'è: è la velocità dal vivo dell'ago.

- [ ] **Step 1: test che falliscono** (factory finta, 2 thread al massimo):
  - `fixed_work_phase_runs_exact_iterations_per_worker`: 2 worker con `iterations = 50` danno `checks == 100`;
  - `fixed_work_phase_stops_at_the_cap_and_reports_work_ms`: con un kernel finto lento e un tetto di 1 s, `work_ms` è circa 1000 e `checks < iterations × worker`;
  - `work_ms_excludes_the_reference_and_the_pause`: con un riferimento finto da 300 ms e `pause_before_ms = 200`, `work_ms` è minore della durata della fase meno 400;
  - `pause_is_not_a_hang`;
  - `timed_phase_has_no_work_ms`;
  - `fixed_work_error_stops_the_phase`.
- [ ] **Step 2:** `cargo test -p oma-load engine`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load` e clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): fixed-work phases with pause and work time`.

### Task B3: `oma-load`, dimensioni fisse e carichi del benchmark

**Files:**
- Create: `crates/oma-load/src/kernels/bench.rs` (factory di `Hash`, `Compress` e `Sort`, che riusano `k8::sha256`, `crc32c_u64`, `k8::lz` e `k8::sort`).
- Modify:
  - `crates/oma-load/src/kernels/k2.rs`, `k5.rs`, `k7.rs` (`DataSize::Fixed`, DB2);
  - `crates/oma-load/src/kernels/mod.rs`;
  - `crates/oma-load/src/kernel.rs` (`factory`).

**Interfaces:**
- Consumes: B1, B2.
- Produces:
  - `pub const FIXED_FFT_N: usize = 4096`, `FIXED_NTT_N: usize = 1 << 15`, `FIXED_GEMM_N: usize = 256`, `BENCH_BYTES: usize = 1 << 20`, `BENCH_SORT_LEN: usize = 262_144`;
  - `factory(KernelId::Hash | Compress | Sort)` restituisce un `KernelFactory` con riferimento e verifica come K8;
  - ogni iterazione fa lo stesso lavoro, e il digest dipende solo dal seme e dall'ISA.

- [ ] **Step 1: test che falliscono:**
  - `fixed_size_ignores_the_budget`: K2, K5 e K7 con due `ThreadBudget` diversi danno lo stesso digest;
  - `hash_kernel_matches_known_vectors` (SHA-256 di `"abc"`, CRC32C di `"123456789"`, dai vettori di K8);
  - `compress_kernel_round_trips`;
  - `sort_kernel_sorts_and_keeps_the_sum`;
  - `bench_kernels_are_deterministic`;
  - `bench_kernel_bit_flip_is_a_mismatch`.
- [ ] **Step 2:** `cargo test -p oma-load bench`. Atteso: FAIL.
- [ ] **Step 3:** implementare. Aggiungere il test ignorato `bench_iteration_times`, che misura un'iterazione di ciascuno dei sei carichi (dimensione `Fixed`, migliore ISA, **un thread**) e la stampa in ms.
- [ ] **Step 4:** `cargo test -p oma-load` e clippy, poi una volta `cargo test -p oma-load --release bench_iteration_times -- --ignored --nocapture`. Annotare i sei tempi nel messaggio di commit.

  Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): fixed data sizes and the hash, compress and sort bench kernels`.

### Task B4: `oma-core::scores`, carichi, piano, punteggio e file

**Files:**
- Create:
  - `crates/oma-core/src/scores/mod.rs`, `workloads.rs`, `plan.rs`, `score.rs`, `gauge.rs`, `file.rs`;
  - `crates/oma-core/src/scores/cpu-1-baseline.json`;
  - `crates/oma-core/examples/calibrate_cpu.rs`.
- Modify: `crates/oma-core/src/lib.rs`.

**Interfaces:**
- Consumes: B1; i tempi di B3.
- Produces:
  - **`workloads`:**
    - `enum BenchKernel { Ntt, Hash, Compress, Sort, Fft, Gemm }` (serde `snake_case`), in quest'ordine;
    - `struct Workload { id: BenchKernel, kernel: KernelId, unit: &'static str, work_per_iteration: f64, iterations: u64 }`;
    - `pub const WORKLOADS: [Workload; 6]`.
    - Le unità sono `"Mop/s"`, `"MB/s"`, `"MB/s"`, `"Melem/s"`, `"GFLOP/s"`, `"GFLOP/s"`. `work_per_iteration` è nell'unità × 10⁶ o 10⁹.
    - `iterations` si sceglie per circa 1 s a ripetizione single su questo PC, dai tempi di B3.
  - **`plan`:**
    - `enum BenchMode { Single, Multi }`;
    - `struct BenchStep { kernel: BenchKernel, mode: BenchMode, rep: u8 }`, dove `rep` 0 è il riscaldamento e 1–3 le ripetizioni;
    - `pub fn bench_plan(topology: &Topology, isa: Isa, seed: u64) -> (Plan, Vec<BenchStep>)`, con la scaletta di DB4 e DB5 e `ram_bytes = 0`.
  - **`score`:**
    - `struct Baseline { version: String, provisional: bool, single: BTreeMap<BenchKernel, f64>, multi: BTreeMap<BenchKernel, f64> }`, con le velocità in unità vere;
    - `pub fn cpu_baseline() -> &'static Baseline`, letto con `include_str!`;
    - `pub fn rate(w: &Workload, checks: u64, work_ms: u64) -> Option<f64>`, nell'unità vera, `None` con `work_ms == 0` o `checks == 0`;
    - `pub fn median3(v: &[f64]) -> Option<f64>`, con le ripetizioni valide (1–3) e la mediana o la media delle due centrali;
    - `pub fn points(rates: &BTreeMap<BenchKernel, f64>, reference: &BTreeMap<BenchKernel, f64>) -> Option<u32>`: `None` se manca uno dei sei carichi;
    - `pub fn scaling(single: &BTreeMap<..>, multi: &BTreeMap<..>, logical: u32) -> Option<f64>`;
    - `pub const SCALE_POINTS: f64 = 1500.0`.
  - **`gauge`:** `pub fn full_scale(values: &[f64]) -> f64`, che contiene già 1500, con la serie di DB6.
  - **`file`:**
    - `struct ScoreFile` con i campi di DB11 (camelCase), più `ScoreSummary { id, at, single, multi, valid, flags, provisional }`;
    - `parse_score(bytes) -> Result<ScoreFile, ...>`, che rifiuta `format != 1`;
    - `score_file_name(at_utc, id)`, che riusa la forma di `session_file_name`.
  - **Baseline provvisoria:**
    - single = la velocità di B3 per carico;
    - multi = single × processori logici di questo PC × 0,75;
    - `"provisional": true`, `"version": "cpu-1"`.
  - **`calibrate_cpu`:** `cargo run -p oma-core --example calibrate_cpu -- <score.json>` riscrive `cpu-1-baseline.json` con le velocità mediane del file, arrotondate a 4 cifre significative, e `provisional: false`. Il controllo sta in `oma_core::scores::calibration_from`: rifiuta un file non valido, con flag, di un'altra versione del punteggio o senza tutti i sei carichi nei due modi; prima di scrivere stampa modello, core, processori logici, set d'istruzioni e mediane, da confrontare con il 7800X3D al BIOS di fabbrica.

- [ ] **Step 1: test che falliscono:**
  - `bench_plan_has_48_phases_single_then_multi`;
  - `single_phases_pin_the_first_core_of_the_best_class`;
  - `each_warmup_pauses_two_seconds`;
  - `bench_plan_on_a_single_logical_cpu`;
  - `bench_plan_validates` (con `LoadMessage::Run(...).validate()`);
  - `rate_converts_to_true_units`;
  - `zero_work_ms_rep_is_ignored`;
  - `median_of_three_and_of_two`;
  - `points_at_the_baseline_are_1500`;
  - `doubling_every_rate_doubles_the_points`;
  - `score_without_all_six_kernels_is_none`;
  - `scaling_with_one_logical_is_one`;
  - `full_scale_series`: 1500 → 2000; 1900 → 2500 (perché 1,1 × 1900 = 2090); 4600 → 5000; 120 con 1500 → 2000; 50 000 → 100 000;
  - `score_file_round_trips_and_rejects_a_future_format`;
  - `baseline_parses_and_has_six_kernels_per_mode`.
- [ ] **Step 2:** `cargo test -p oma-core scores`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-core` e clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): CPU benchmark workloads, plan, fixed-scale score and score file`.

### Task B5: `oma-core::scores::bench`, il controller del benchmark

**Files:**
- Create: `crates/oma-core/src/scores/bench.rs`.

**Interfaces:**
- Consumes:
  - B4;
  - `Clock`, `SensorSample` e `Action` di `oma-core::load::run`, con lo stesso modello: ogni `on_*` restituisce `Vec<Action>`.
  - Per non allargare `Action` si usa un enum proprio, `BenchAction { SendStop, Kill, Save(ScoreFile), Finished(BenchEnd) }`, con `enum BenchEnd { Saved(String), Stopped, Failed(String) }`.
- Produces:
  - `BenchController::new(steps: Vec<BenchStep>, ctx: BenchContext, now: Clock)`, con `BenchContext { id, at, isa, device, logical, tjmax_c: Option<f64>, service_available, on_battery: Option<bool>, hypervisor, baseline: &'static Baseline, app_version }`;
  - i metodi `on_load(&LoadMessage, Clock)`, `on_sample(&SensorSample, Clock)`, `on_busy_share(f64)` (la quota degli altri processi, 0–1), `on_battery(bool)`, `on_user_stop(Clock)`, `on_exit(Option<i32>, Clock)` e `status() -> BenchStatus`.
  - **`BenchStatus`** (serde camelCase): `state` (`starting`, `running`, `stopping`, `done`, `stopped`, `failed`), `step: Option<usize>`, `steps: Vec<BenchStep>`, `segments: Vec<SegmentState>` (`pending`, `running`, `done`, `failed`, una per passo), `livePoints: Option<f64>` (DB6), `single: Option<u32>`, `multi: Option<u32>`, `flags: Vec<String>`, `scoreId: Option<String>`, `error: Option<String>`.
  - **Regole:**
    - `PhaseDone` con `work_ms` registra la velocità della ripetizione; i riscaldamenti non contano;
    - `single` e `multi` di `status()` sono i punti appena i sei carichi di quel modo sono finiti;
    - un `Error` o `Finished { reason: FirstError }` porta a `valid = false` con `compute_error`: si salva subito e si chiude con `Saved`;
    - `Finished { reason: Completed }` salva con i flag di DB7 e chiude con `Saved(id)`;
    - fermarsi chiude con `Stopped` senza `Save`;
    - un'uscita di `oma-load` prima di `Finished` chiude con `Failed("exited")` senza `Save`;
    - un campione ogni 5 s finisce in `samples`.

- [ ] **Step 1: test che falliscono** (messaggi scritti a mano, orologio finto):
  - `full_run_saves_both_scores`;
  - `warmup_reps_do_not_count`;
  - `capped_reps_still_score`;
  - `live_points_follow_progress_rate`;
  - `error_saves_an_invalid_score`;
  - `user_stop_saves_nothing`;
  - `early_exit_fails_without_saving`;
  - `hot_sample_flags_thermal_throttle`, con Tjmax 89 e un campione a 87;
  - `busy_share_above_ten_percent_flags`;
  - `battery_and_hypervisor_and_no_service_flags`;
  - `segments_follow_the_steps`.
- [ ] **Step 2:** `cargo test -p oma-core scores::bench`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-core` e clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): benchmark controller`.

### Task B6: `oma-win`, batteria e CPU degli altri processi

**Files:**
- Create: `crates/oma-win/src/proc_cpu.rs`.
- Modify: `crates/oma-win/src/power.rs`, `crates/oma-win/src/lib.rs`.

**Interfaces:**
- Produces:
  - `pub fn on_battery() -> Option<bool>`, da `GetSystemPowerStatus`: `ACLineStatus == 0` dà `Some(true)`, 1 dà `Some(false)`, 255 o un errore danno `None`; con l'helper puro `battery_from_line_status(u8) -> Option<bool>`;
  - `pub struct OtherCpu`, con `OtherCpu::open() -> io::Result<Self>` (query PDH `\Process(*)\% Processor Time` con `add_english`) e `sample(&mut self, logical: u32) -> Option<f64>`, la quota degli altri processi fra 0 e 1;
  - l'helper puro `others_share(rows: &[(String, f64)], logical: u32) -> f64`, che esclude `_Total`, `Idle` e le istanze che iniziano con `oma-` (anche con suffisso `#n`) e divide per 100 × `logical`.

- [ ] **Step 1: test che falliscono:**
  - `battery_from_line_status_values`;
  - `others_share_excludes_idle_total_and_our_processes`;
  - `others_share_with_instance_suffixes`;
  - `other_cpu_reads_this_machine` (`#[ignore = "requires real Windows hardware"]`).
- [ ] **Step 2:** `cargo test -p oma-win proc_cpu power`. Atteso: FAIL.
- [ ] **Step 3:** implementare, con `// SAFETY:` sulla chiamata FFI.
- [ ] **Step 4:** `cargo test -p oma-win`, poi `cargo test -p oma-win -- --include-ignored other_cpu`, e clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(win): battery status and other processes' CPU share`.

### Task B7: app, archivio dei punteggi, runner, comandi, tray e toast

**Files:**
- Create: `app/src-tauri/src/performance/bench.rs` (il ciclo del benchmark sul thread `oma-perf-runner`).
- Modify:
  - `app/src-tauri/src/performance/store.rs` (sottocartella `scores\`);
  - `runner.rs` (lo slot `active` condiviso, DB8);
  - `commands.rs` e `main.rs`;
  - `capabilities/default.json`;
  - il tray (DB9) e `notifier.rs` (`LaunchTarget` verso `score-cpu`);
  - `app/src-tauri/src/i18n.rs` (`RUST_KEYS`).

**Interfaces:**
- Consumes: B4, B5, B6; `Launcher`, `Machine`, `ToastSink` e `KeepAwake` di oggi.
- Produces:
  - **`PerformanceStore`:** `save_score(&ScoreFile)`, `list_scores() -> Vec<ScoreSummary>` (dal più recente), `load_score(id) -> Option<ScoreFile>`, `delete_score(id)`; id solo in forma uuid; al massimo 500 file, i più vecchi si cancellano;
  - **`PerformanceRunner::start_bench() -> Result<String, StartError>`:**
    - mette in fila il piano di B4, `KeepAwake` e l'host;
    - il ciclo ogni 250 ms dà al `BenchController` i messaggi, i campioni del sampler, `OtherCpu::sample` all'avvio e ogni 5 s, e `on_battery` all'avvio e ogni 5 s;
    - `BenchAction::Save` chiama `save_score`;
  - **`stop_bench()`** e **`bench_status() -> Option<BenchStatus>`**;
  - **comandi:** `performance_bench_start -> String`, `performance_bench_stop`, `performance_bench_status -> Option<BenchStatus>`, `performance_scores -> Vec<ScoreSummary>`, `performance_score(id) -> Option<ScoreFile>`, `performance_score_delete(id)`, `performance_baseline -> { provisional: bool }`;
  - **evento** `performance-bench` con `BenchStatus`, al cambio di stato e a 2 Hz durante il benchmark, solo con una finestra aperta;
  - **tray e toast:** come DB9; testi di T1 (`tray.benchRunning`, `performance.toast.benchDone`, `performance.toast.benchInvalid`).

- [ ] **Step 1: test che falliscono** (con `Launcher` e `Machine` finti, come i test del runner di oggi):
  - `bench_runs_to_a_saved_score`;
  - `bench_while_stress_runs_is_busy`, e il contrario;
  - `bench_stop_saves_nothing`;
  - `quit_stops_the_bench_without_asking`;
  - `bench_toast_only_when_the_window_is_hidden`;
  - `score_ids_outside_the_uuid_form_are_rejected`;
  - `corrupt_and_future_scores_are_skipped`;
  - `scores_are_pruned_to_500`.
- [ ] **Step 2:** `cargo test -p oma-app performance`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test --workspace` e clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): CPU benchmark runner, score store, commands and tray`.

### Task B8: UI, font e contagiri

Prima: `frontend-design:frontend-design`, con il mockup approvato `.superpowers/brainstorm/1446-1791244410/content/gauges.html` (stile C, «Ibrido con riferimento», D6) e i token di `app/src/styles/theme.css`.

**Files:**
- Create:
  - `app/src/assets/fonts/Orbitron[wght].ttf` e `ShareTechMono-Regular.ttf`, dal repository `google/fonts` a un commit fissato, con il commit e lo SHA-256 nel commento di `fonts.css`;
  - `app/src/styles/fonts.css`;
  - `app/src/lib/performance/gauge.ts` e il suo test;
  - `app/src/components/performance/Gauge.svelte` e il suo test.
- Modify: `app/src/main.ts` (import di `fonts.css`).

**Interfaces:**
- Produces:
  - **`gauge.ts`:**
    - `angleFor(value: number, max: number): number` dà gradi da 135 a 405, con limiti a 0 e max;
    - `ticks(max: number): { angle: number; kind: 'major' | 'mid' | 'minor'; label?: string }[]` dà 51 tacche (da 0 a 50), maggiori ogni 10 e medie ogni 5, con le etichette sulle maggiori;
    - `fullScale(values: number[]): number`, la stessa serie di `oma-core::scores::gauge` (DB6), con 1500 compreso;
    - `smooth(current: number, target: number, dtMs: number): number`, una media esponenziale con costante di tempo di 150 ms;
  - **`Gauge.svelte`:**
    - props `{ value: number | null; max: number; reference: number | null; label: string; unit: string; moving: boolean }`;
    - SVG con ghiera, quadrante in carbonio viola, arco acceso da `--accent-2` a `--accent`, ago rosa e ▲ bianco al riferimento;
    - display a matrice di punti con le cifre «fantasma» `8888`, in Share Tech Mono; etichette in Orbitron;
    - `requestAnimationFrame` solo se `moving`, con `document.visibilityState === 'visible'` e l'ago lontano dal valore; con `prefers-reduced-motion` l'ago salta al valore;
    - `role="meter"` con `aria-valuenow`, `aria-valuemax` e `aria-label`.

- [ ] **Step 1: test che falliscono:**
  - `gauge.test.ts`: `angle_bounds` (0 → 135, max → 405, oltre max → 405); `ticks_count_and_kinds` (51 tacche, 6 maggiori con etichetta); `full_scale_matches_the_rust_series` (gli stessi casi di `full_scale_series` di B4); `smooth_converges_without_overshoot`;
  - `Gauge.test.ts`: `meter_has_value_and_label`; `reference_mark_only_with_a_reference`; `reduced_motion_jumps_to_the_value`; `no_animation_frame_when_not_moving`.
- [ ] **Step 2:** `cd app && pnpm test`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS, e i due font nel bundle (`dist/assets`).
- [ ] **Step 5: commit** `feat(ui): Orbitron and Share Tech Mono fonts and the gauge component`.

### Task B9: UI, pagina «CPU» del gruppo Punteggio

**Files:**
- Create:
  - `app/src/components/performance/CpuScore.svelte` e il suo test;
  - `app/src/lib/performance/bench.svelte.ts` e il suo test.
- Modify:
  - `PerformanceView.svelte` (gruppo «Punteggio» sopra «Stress test»);
  - `app/src/lib/view.ts` (`PerformancePage` con `'score-cpu'`, e la navigazione dal toast);
  - `app/src/lib/types.ts`;
  - `app/src/lib/backend/backend.ts`, `tauri.ts`, `mockPerformance.ts` e `app/src/test/fake-backend.ts`;
  - `app/src/lib/performance/glossary.ts` (`TERMS` e `BENCH_TERMS`);
  - `en.json` e `it.json`.

**Interfaces:**
- Consumes: i comandi e l'evento di B7; `Gauge` di B8; T1 e T2.
- Produces:
  - **`Backend`:** `performanceBenchStart`, `performanceBenchStop`, `performanceBenchStatus`, `performanceScores`, `performanceScore`, `performanceScoreDelete`, `performanceBaseline`, `onPerformanceBench`. Il mock simula 20 s di benchmark, e con `?bench=error` una misura non valida.
  - **`BenchStore`:**
    - `status` e `scores`, in `$state.raw`;
    - `connect(backend)`: prima si iscrive, poi legge;
    - `start()`, `stop()`, `refresh()`, `remove(id)`;
    - `record`: i massimi `single` e `multi` delle misure valide.
  - **`CpuScore.svelte`:**
    - in alto il titolo, due `Gauge` (single e multi), la barra a segmenti di `status.segments`, «Avvia» o «Ferma» e il testo `performance.score.duration`;
    - il menu del riferimento ▲ (DB10), salvato solo in memoria;
    - sotto: la scala in percentuale, la tabella di dettaglio con le velocità di ogni carico nella sua unità, i flag di validità, l'avviso `provisional` e l'errore;
    - «Le tue misure»: data, single, multi, flag, eliminazione;
    - ogni termine tecnico è un `Term`: `benchPoints`, `singleCore`, `multiCore`, `scaling`, `referenceMark`, `warmup`, `median` e `bench.<id>` nella tabella.
  - **La barra laterale** mostra «Punteggio › CPU» con un punto mentre il benchmark gira.
  - **Durante lo stress test** «Avvia» è disattivato, con `performance.score.error.busy` come titolo.

- [ ] **Step 1: test che falliscono:**
  - `bench.test.ts`: `connect_subscribes_before_reading`; `record_ignores_invalid_scores`;
  - `CpuScore.test.ts`:
    - `start_runs_and_shows_both_scores`;
    - `live_gauge_moves_with_status`;
    - `reference_menu_switches_between_record_and_last`;
    - `invalid_score_shows_the_message`;
    - `flags_show_their_text`;
    - `start_is_disabled_while_a_stress_test_runs`;
    - `history_lists_and_deletes`;
  - `glossary.test.ts`: `every_bench_kernel_has_an_entry`. Il test esistente `every_term_used_in_performance_pages_has_a_key` copre i `Term` nuovi da solo.
- [ ] **Step 2:** `cd app && pnpm test`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): CPU score page with gauges and measurements`.

### Task B10: licenze e documenti

**Files:**
- Modify:
  - `scripts/generate-licenses.ps1` (voce «Fonts» con Orbitron e Share Tech Mono, OFL-1.1, copyright dai rispettivi `OFL.txt`);
  - `scripts/licenses/` (i due `OFL.txt` copiati dal commit fissato);
  - `about.toml` (`OFL-1.1` fra le licenze accettate, se serve);
  - `THIRD_PARTY_LICENSES.txt` (rigenerato) e `THIRD_PARTY_NOTICES.md`;
  - `CLAUDE.md`: struttura (`oma-core::scores`, `performance/bench.rs`, `Gauge.svelte`, `CpuScore.svelte`, i font) e stato della M8a2;
  - `README.md` e `README.it.md`: il benchmark della CPU;
  - `docs/perf-budget.md`: sezione «M8a2» con la misura a riposo;
  - `docs/follow-ups.md`: le voci aperte (esportazione e condivisione alla M8d; il benchmark sui portatili per il flag `battery`).

- [ ] **Step 1:** `pwsh scripts/generate-licenses.ps1`, poi `pwsh scripts/generate-licenses.ps1 -Check`. Atteso: PASS.
- [ ] **Step 2:** `pwsh scripts/measure-footprint.ps1` a riposo. Atteso: nucleo < 1% CPU, tray < 30 MB, finestra < 200 MB.
- [ ] **Step 3:** aggiornare i documenti.
- [ ] **Step 4: verifiche:**
  - `cargo fmt --all --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo test --workspace`;
  - `cd app && pnpm test && pnpm check && pnpm build`;
  - `PYTHONHASHSEED=0 graphify update .`.

  Atteso: PASS.
- [ ] **Step 5: commit** `docs: M8a2 CPU benchmark documentation, font licences and footprint`.

### Task B11: taratura e prove dal vivo con l'utente

Le fa l'utente, un blocco alla volta (memoria «user admin shell»). Ogni benchmark completo è un carico a tutti i core: si chiede prima. Niente input sintetico. Prima: `cargo build -p oma-load --release`, poi `cd app && pnpm tauri dev`.

| # | Prova | Atteso |
|---|---|---|
| Q1 | **Taratura.** L'utente conferma il BIOS di fabbrica (PBO e Curve Optimizer spenti; EXPO annotato com'è), chiude le applicazioni pesanti e avvia il benchmark. Poi `cargo run -p oma-core --example calibrate_cpu -- <file del punteggio>`, commit del nuovo `cpu-1-baseline.json` e nuova build. | Misura valida senza flag; il file di riferimento ha `provisional: false`. |
| Q2 | Secondo benchmark, dopo la taratura (le misure provvisorie si cancellano dalla pagina) | Single e multi a 1500 ± 3%; durata intorno ai 2 minuti; contagiri fluidi; ▲ sul record. |
| Q3 | Menu del riferimento: «L'ultima misura» | Il ▲ si sposta sull'ultima misura. |
| Q4 | `$env:OMA_LOAD_INJECT='k5'` e benchmark | «Errore di calcolo durante il benchmark: prova lo stress test», misura non valida nell'elenco. |
| Q5 | «Ferma» a metà | Nessuna misura nuova. |
| Q6 | Finestra chiusa durante il benchmark | Il benchmark continua, arriva il toast finale, e il clic apre la pagina CPU. |
| Q7 | «Ferma il test» dal tray durante il benchmark | Si ferma, senza misura. |
| Q8 | Benchmark durante uno stress test, e il contrario | «È già in corso uno stress test o un benchmark.» |
| Q9 | Due cicli PowerShell a vuoto aperti dall'utente (`while($true){}` in due finestre), poi il benchmark | Flag «Altri programmi usavano più del 10%…». |
| Q10 | Senza servizio | Flag «Servizio non attivo…». |
| Q11 | Tooltip e font | Ogni termine tecnico della pagina ha la spiegazione con il mouse e con Tab; Orbitron e Share Tech Mono visibili. |
| Q12 | Budget durante il benchmark | `scripts/measure-footprint.ps1`: finestra < 200 MB. |

- [ ] **Step 1:** preparare i comandi esatti e chiedere il via per Q1.
- [ ] **Step 2:** dopo Q1, commit `feat(core): calibrate the cpu-1 scale on the reference machine`.
- [ ] **Step 3:** registrare gli esiti in `docs/follow-ups.md` e nella memoria del progetto (`m8a2-followups.md`), poi commit `docs: record the M8a2 live checks`.
- [ ] **Step 4:** `superpowers:finishing-a-development-branch`: revisione dell'intero branch e merge in `main` in locale, senza push se l'utente non lo chiede.

## Aggiunta del 2026-10-07: velocità per thread (decisione dell'utente)

Dopo la revisione finale l'utente ha scelto la «soluzione C» per il multi core, che sostituisce la regola di DB5 «velocità = `checks` / `work_ms` dell'ultimo thread».

**DB12.** Ogni thread ha un suo tempo:
- un thread misura il tempo delle sue n iterazioni, dall'apertura del cancello;
- dopo, continua a lavorare senza contare (lavoro di riempimento, verificato come le altre iterazioni) finché non hanno finito tutti, oppure fino al tetto, a uno stop o a un errore;
- la velocità della ripetizione è Σ (iterazioni del thread / tempo del thread);
- in single c'è un solo thread, quindi la velocità resta quella di prima;
- `Progress.rate` conta anche le iterazioni di riempimento: è il lavoro della CPU intera a carico pieno;
- il file del punteggio non cambia (`format: 1`): le velocità per thread non si salvano, e il dettaglio per core resta alla M8d.

Con l'occasione si chiude la voce aperta della revisione finale sull'ago: a fine ripetizione l'ago prende la velocità di quella ripetizione, invece di tornare a vuoto.

### Task B12: protocollo `load` v3 e tempi per thread nel motore

**Files:**
- Modify:
  - `crates/oma-ipc/src/load.rs`, `crates/oma-ipc/tests/load_fixtures.rs`, `protocol/fixtures/load/*.msgpack` (rigenerate);
  - `crates/oma-load/src/engine/mod.rs`, `modes.rs`, `tests.rs`;
  - i costruttori di `PhaseDone` nel resto del workspace (valore neutro: `workers: vec![]`).

**Interfaces:**
- Produces:
  - `LOAD_PROTOCOL_VERSION = 3`;
  - `pub struct WorkerDone { pub logical: u32, pub iterations: u64, pub work_ms: u64 }`;
  - `PhaseDone.workers: Vec<WorkerDone>`, con `#[serde(default)]` (le sessioni salvate prima lo leggono vuoto) e sempre scritto; vuoto per le fasi a tempo e per quelle saltate;
  - `validate`: al massimo 1024 voci in `workers`.
  - **Motore:**
    - in una fase a lavoro fisso, ogni worker registra le iterazioni contate e il proprio `work_ms` quando arriva a n;
    - poi continua con passi di riempimento, verificati ma non contati in `checks`;
    - al tetto, a uno stop o a un errore, un worker che non ha finito riporta le iterazioni fatte e il tempo fino a lì;
    - `PhaseDone.work_ms` resta il tempo dell'ultimo thread;
    - `Progress.rate` conta tutti i passi, anche quelli di riempimento.

- [ ] **Step 1: test che falliscono:**
  - oma-ipc: `phase_done_workers_round_trip`, `phase_done_without_workers_parses`, `too_many_workers_are_rejected`;
  - motore (factory finta, al massimo 2 thread):
    - `per_worker_times_with_a_slow_worker`: il worker lento ha `work_ms` maggiore, ed entrambi hanno `iterations == n`;
    - `finished_worker_keeps_loading_until_all_done`: il worker veloce fa più di n passi in tutto, e `checks == n × worker`;
    - `capped_worker_reports_partial_iterations`;
    - `filler_error_is_still_an_error`;
    - `timed_phase_has_no_workers`.
- [ ] **Step 2:** `cargo test -p oma-ipc load` e `cargo test -p oma-load --lib engine`. Atteso: FAIL.
- [ ] **Step 3:** implementare, poi rigenerare le fixture con `OMA_WRITE_FIXTURES=1 cargo test -p oma-ipc --test load_fixtures -- --test-threads=1`.
- [ ] **Step 4:** `cargo test --workspace` e clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): per-thread work time with filler load (protocol v3)`.

### Task B13: velocità per thread nel punteggio e ago a fine ripetizione

**Files:**
- Modify:
  - `crates/oma-core/src/scores/score.rs`, `bench.rs`;
  - `app/src/lib/backend/mockPerformance.ts`, se serve;
  - `CLAUDE.md` (protocollo `load` v3, DB12) e `README.md`/`README.it.md`, se descrivono il multi core.

**Interfaces:**
- Consumes: B12.
- Produces:
  - `pub fn rate_from_workers(w: &Workload, workers: &[WorkerDone]) -> Option<f64>`: `per_second_to_units(w, Σ iterations × 1000 / work_ms)`, che salta le voci con `work_ms == 0` o `iterations == 0`, e dà `None` se non ne resta nessuna;
  - `BenchController`:
    - usa `rate_from_workers` quando `workers` non è vuoto, altrimenti `rate(checks, work_ms)`;
    - in `phase_done` mette `live_points` alla velocità della ripetizione appena finita, riscaldamento compreso, invece di `None`;
    - accetta un `Progress` con `rate > 0` senza il limite di 1 s.

- [ ] **Step 1: test che falliscono:**
  - `multi_rate_sums_thread_rates`: 8 thread da 1000 iterazioni in 1000 ms e 16 da 1000 in 2000 ms danno 16 000 it/s, convertiti nell'unità del carico;
  - `single_worker_rate_equals_checks_over_work_ms`;
  - `zero_time_workers_are_ignored`;
  - `needle_holds_the_rep_rate_after_phase_done`;
  - `needle_ignores_zero_rate_progress`.
- [ ] **Step 2:** `cargo test -p oma-core scores`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test --workspace`, clippy, `cd app && pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): multi-core rate as the sum of per-thread rates; needle holds the rep rate`.
