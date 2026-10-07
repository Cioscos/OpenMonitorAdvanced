# M8b2 — Benchmark della GPU, S3 e pagina di punteggio della GPU: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** aggiungere al gruppo «Punteggio» della vista Prestazioni il benchmark della GPU, che comprende:
- sei carichi D3D11 misurati con le timestamp query: tre di Calcolo (FMA FP32, interi, banda della memoria) e tre di Grafica (riempimento, texture, sovrapposizione);
- due punteggi, Calcolo e Grafica, su una scala fissa con la RTX 4080 a 1500;
- una pagina per ogni GPU, con i contagiri, il dettaglio dei carichi e le proprie misure;
- S3 (flusso di memoria) nello stress test, dentro il profilo «Stabilità overclock».

**Architecture:**
- **`oma-ipc::load` passa alla versione 5 (H1):**
  - i kernel `s3`, `fill`, `texture` e `overdraw`;
  - le fasi del benchmark della GPU (`Phase.windows`) e le velocità delle finestre (`PhaseDone.rates`);
  - l'impronta degli shader in `Hello`.
- **`oma-load`:**
  - misura le finestre da 1 s con le timestamp query (H2);
  - S3, il flusso di memoria (H3);
  - i tre carichi grafici del benchmark (H4).
- **`oma-core` (puro):**
  - S3 nel catalogo, nel profilo d'overclock e nella stabilità, più il flag termico della GPU (H5);
  - carichi, piano, punteggio e file del benchmark della GPU (H6);
  - il controller del benchmark della GPU, con la parte comune estratta da quello della CPU (H7).
- **`oma-win` (H8):** i processi discendenti di un PID, così l'avviso «altri programmi sulla GPU» non conta i nostri.
- **App (H9):** runner, comandi, tray e toast del benchmark della GPU.
- **UI (H10):** la pagina di punteggio generalizzata a CPU e GPU, la voce per ogni GPU nella barra laterale, i testi e il glossario.
- **Chiusura:** documenti (H11) e prove dal vivo con la taratura (H12).

**Tech Stack:** Rust 1.90 (workspace `rust-version` 1.85, `oma-load` 1.89), crate `windows` 0.62 (D3D11, DXGI, ToolHelp), HLSL `cs_5_0`/`vs_5_0`/`ps_5_0` compilato con `fxc.exe`, Tauri 2.11, Svelte 5, TypeScript 6, Vitest.

**Spec:** `docs/superpowers/specs/2026-10-06-m8-prestazioni-design.md`. Le sezioni usate sono:
- §3.1 (voce GPU del gruppo Punteggio), §3.2, §3.3 (aggiornato insieme a questo piano: stima della scala per la GPU), §3.7, §3.9;
- §5.2 (aggiornato: scala fissa a 1500), §5.3 (S3), §5.4 (aggiornato: S3 nel giro d'overclock);
- §8.2, §11, §13.

Esiti dello spike: `docs/superpowers/references/m8/spike-gpu.md`. Piani precedenti, da imitare:
- `docs/superpowers/plans/2026-10-07-m8a2-benchmark-cpu.md`: le sue decisioni DB1–DB12 restano valide dove questo piano non le cambia;
- `docs/superpowers/plans/2026-10-07-m8b1-stress-gpu.md`: le sue decisioni DG1–DG15 restano valide dove questo piano non le cambia.

**Branch:** `feat/m8b2-gpu-bench` da `main`. Alla fine il merge in `main` si fa in locale, dopo le prove dal vivo o su richiesta dell'utente. Push e release solo su richiesta dell'utente.

**Esecuzione:**
- **Subagent-driven:** un implementer e una revisione per task, poi la revisione dell'intero branch.
- **Revisioni in più:**
  - `ffi-safety-reviewer` dopo H2, H3, H4 e H8;
  - `security-review` dopo H9 (il `device_id` dalla UI e dalla stringa del toast);
  - `frontend-design:frontend-design` prima di H10.
- **Prove dal vivo:** H12, con l'utente.

## Global Constraints

Valgono tutti i vincoli globali dei piani M8b1 e M8a2, in particolare questi:
- **Lingua e formato:**
  - codice, commenti e messaggi di commit in inglese (conventional commits);
  - documentazione e prosa in italiano con gli accenti corretti;
  - fine riga LF ovunque;
  - ogni commit termina con le due righe:
    - `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
    - `Claude-Session: https://claude.ai/code/session_01B3yzfCRMU6TL8scsVZQ32s`.
- **Divieti per gli agenti:**
  - mai clic sintetici, UI Automation o tasti inviati al desktop, al tray o alle finestre dell'app;
  - mai installer eseguiti, mai test Pester `Integration`, mai comandi elevati;
  - mai ricerche a tutto il disco;
  - mai push, merge in `main` o release.
- **Carico (memoria «no heavy load without warning»):**
  - un agente non avvia mai il benchmark vero, né uno stress test, né `oma-load.exe` con un piano intero;
  - i test sulla GPU vera sono `#[ignore = "requires real Windows hardware"]`, con al massimo 2 s di GPU ciascuno e 30 s in tutto per verifica;
  - la suite completa si esegue una volta per verifica, non in un ciclo.
- **Risparmio:**
  - senza un test, `oma-load` non esiste e la vista Prestazioni non fa lavoro periodico;
  - l'elenco delle GPU per la barra laterale si legge una volta, quando si apre la vista (`performance_system`), mai in un timer;
  - la quota degli altri processi sulla GPU si legge solo durante il benchmark, ogni 5 s.
- **TDD e debug:** prima il test che fallisce (`superpowers:test-driven-development`); davanti a un test che fallisce senza un motivo chiaro, `superpowers:systematic-debugging`.
- **graphify:** ogni brief riporta `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"`. Dopo le modifiche al codice: `PYTHONHASHSEED=0 graphify update .`.
- **FFI Rust:** un `// SAFETY:` per ogni blocco `unsafe`; un assert di dimensione per ogni struct FFI scritta a mano; helper puri testati senza hardware.
- **Protocollo `oma-ipc::load`:**
  - chiavi sempre presenti, mai `skip_serializing_if`, enumerati `snake_case`, niente `deny_unknown_fields`;
  - i campi nuovi si leggono con `#[serde(default)]`;
  - il ricevente chiama `LoadMessage::validate`;
  - fixture rigenerate solo con `OMA_WRITE_FIXTURES=1`, a thread singolo.
- **Dipendenze:** nessun crate e nessun pacchetto npm nuovo. Nessuna feature nuova del crate `windows` (ToolHelp c'è già in `oma-win`).
- **Compatibilità dei file:** le sessioni di stress e i punteggi della CPU salvati dalla M8a1, dalla M8a2 e dalla M8b1 si leggono ancora; il formato dei punteggi resta `format: 1`.
- **Nomi fissi:**
  - versione del punteggio `gpu-1`, file `crates/oma-core/src/scores/gpu-1-baseline.json`;
  - pagina della vista `score-gpu:<deviceId>`;
  - comando `performance_gpu_bench_start`, evento `performance-bench` (lo stesso della CPU);
  - chiavi i18n `performance.score.*`, `glossary.gpuBench.<id>`, `glossary.mode.s3`.
- **i18n:** stesse chiavi in `en.json` e `it.json`; ogni chiave letta da Rust compare in `RUST_KEYS`; i testi italiani esatti sono nelle tabelle T1–T3, l'inglese lo traduce l'implementer.
- **Budget (§11):** a riposo non cambia niente; durante il benchmark la finestra resta sotto i 200 MB e i contagiri si animano solo con la pagina visibile.

## Decisioni del piano

Il revisore le tratta come requisiti.

| # | Decisione | Perché |
|---|---|---|
| DH1 | **Scala fissa con la RTX 4080 a 1500 (decisione dell'utente, 2026-10-07).**<br>• Punti = `round(1500 × media geometrica(velocità / riferimento))`, sui tre carichi di Calcolo e, a parte, sui tre di Grafica.<br>• I riferimenti stanno in `gpu-1-baseline.json`, con `"provisional": true` fino alla taratura (H12). I valori provvisori sono le velocità misurate in H4 su questo PC.<br>• L'interfaccia non nomina la GPU di taratura: i tooltip parlano di «punti su una scala fissa», come per la CPU.<br>• La taratura si può fare con la build di sviluppo: le finestre si misurano col tempo della GPU, e gli shader sono gli stessi in debug e in release.<br>Sostituisce «RTX 4080 = 1000» di D13 e del §5.2. | Le pagine della CPU e della GPU si leggono allo stesso modo. |
| DH2 | **I sei carichi:**<br>• **Calcolo:**<br>&nbsp;&nbsp;– `fma` (kernel `s1`, TFLOPS): 128 FLOP per passo e thread (16 `mad` su `float4`);<br>&nbsp;&nbsp;– `int_hash` (kernel `s2`, TIOPS): le operazioni intere per passo di `s2_hash.hlsl`, contate dall'implementer di H2 (ogni operatore intero conta 1; una rotazione scritta con due shift e un OR conta 3), con un commento che le elenca;<br>&nbsp;&nbsp;– `bandwidth` (kernel `s3`, GB/s): 32 byte per `float4` copiato, 16 letti e 16 scritti, come la «copy» di STREAM.<br>• **Grafica**, su un render target RGBA8 da 1920×1080 fuori schermo, con quad grandi quanto tutto il target e istanziati:<br>&nbsp;&nbsp;– `fill` (kernel `fill`, Gpixel/s): quad opachi a colore costante, senza texture e senza fusione;<br>&nbsp;&nbsp;– `texture` (kernel `texture`, Gtexel/s): quad opachi con 8 letture bilineari indipendenti per pixel da una texture RGBA8 da 1024×1024; texel = pixel × 8;<br>&nbsp;&nbsp;– `overdraw` (kernel `overdraw`, Gpixel/s): quad a colore costante con fusione alfa (`SRC_ALPHA`, `INV_SRC_ALPHA`) (RGBA16F per `overdraw`, decisione dell'utente del 2026-10-07).<br>• Pixel di un invio = istanze × 1920 × 1080. I carichi grafici non hanno verifica, come S5: contano device perso e invio bloccato. | Un quad grande quanto il target dà un numero esatto di pixel, uguale su ogni GPU. Le tre prove grafiche sono quelle del §5.2. |
| DH3 | **Protocollo v5.**<br>• `KernelId::{S3, Fill, Texture, Overdraw}` (`"s3"`, `"fill"`, `"texture"`, `"overdraw"`), tutti `is_gpu()`; `is_gpu_bench_only()` vale per `fill`, `texture` e `overdraw`.<br>• `Phase.windows: Option<u8>`: le finestre da 1 s misurate in una fase del benchmark della GPU.<br>• `pub const GPU_BENCH_WARMUP_S: u32 = 4`.<br>• `PhaseDone.rates: Vec<f64>`: una velocità per finestra misurata, in unità di base al secondo (FLOP, operazioni, byte, pixel o texel); vuoto per le altre fasi.<br>• `LoadHello.shader_digest: Option<String>`: l'impronta degli shader (DH8).<br>• **`validate`:**<br>&nbsp;&nbsp;– `windows` da 1 a 30, solo con un kernel `is_gpu()`, `steady`, senza `alt_kernel`, con `duration_s >= GPU_BENCH_WARMUP_S + windows`;<br>&nbsp;&nbsp;– i kernel `is_gpu_bench_only()` richiedono `windows`;<br>&nbsp;&nbsp;– `rates` al massimo 64 voci, tutte finite e ≥ 0. | Le misure del §5.2: riscaldamento da 3 a 5 s, almeno 5 finestre da 1 s, mediana e dispersione. |
| DH4 | **Le fasi del benchmark in `oma-load`:**<br>• dopo `prepare`, gli invii vanno come in `steady`, ma ogni secondo è una finestra misurata col tempo della GPU: `Submit::window_begin` prima del primo invio della finestra, `Submit::window_end` alla fine (attende la GPU e dà i millisecondi, oppure `None` se le timestamp sono disgiunte);<br>• velocità della finestra = lavoro degli invii della finestra × 1000 / millisecondi della GPU, con il lavoro da `GpuWorkload::work_per_submission`;<br>• le prime `GPU_BENCH_WARMUP_S` finestre sono il riscaldamento; le `windows` successive vanno in `PhaseDone.rates`; una finestra disgiunta o con 0 ms non conta e non si conta;<br>• la fase finisce quando ha le sue `windows` finestre, oppure a `duration_s` (dall'inizio della fase, taratura compresa), a uno stop o al primo errore;<br>• `Progress.rate` in queste fasi è la velocità dell'ultima finestra in unità di base al secondo, riscaldamento compreso, così l'ago si muove dal primo secondo; `None` prima della prima finestra;<br>• un carico con `work_per_submission() == 0` in una fase con `windows` salta la fase con `skipped: Some("unsupported")`. | Una coppia di timestamp per finestra misura tutti gli invii in volo, senza fermare la coda. |
| DH5 | **S3, il flusso di memoria** (`gpu/stream.rs`, `shaders/s3_stream.hlsl`):<br>• un insieme fisso di 1 GiB (`STREAM_BYTES`), metà sorgenti e metà destinazioni, in pezzi da `chunk_bytes(dedicated)` (DG6);<br>• il totale si limita a `vram_target(...)` (DG6), arrotondato per difetto a un numero pari di pezzi; con meno di 1 GiB arriva `Notice { code: "vram_reduced", value: byte }`; sotto i 256 MiB la fase si salta con `skipped: Some("vram")`;<br>• le sorgenti si riempiono una volta dal seme; ogni invio copia pezzi interi a turno, così in una fase ogni byte dell'insieme si legge e si scrive; il parametro della taratura (DG4) è il numero di pezzi copiati per invio;<br>• nessuna verifica dei dati (§5.3): nello stress conta la stabilità della banda.<br>**Nello stress:**<br>• S3 entra nel giro di «Stabilità overclock» (decisione dell'utente, 2026-10-07): `s4` 15%, `s3` 10%, `s2` 10%, `s1` 10%, `s6` 15%, `s1` `ramp` 20%, `s1` `alternate` 10%, `s1` `pause_resume` 10%;<br>• i rate delle fasi `steady` di `s3` contano nella stabilità, come quelli di `s1`, `s2` e `s5` (DG7). | Il §5.3 verifica S3 con il calo della banda. Il 10% di 30 minuti (180 s) dà almeno 2 finestre da 60 s utili. |
| DH6 | **Piano del benchmark** (`gpu_bench_plan`):<br>• sei fasi, nell'ordine `fma`, `int_hash`, `bandwidth`, `fill`, `texture`, `overdraw`;<br>• ognuna con `windows: Some(5)` (`GPU_WINDOWS`), `duration_s = 30` (`GPU_CAP_S`), `steady`, `stop_on_error = true`, `Plan.gpu = Some(target)`, `ram_bytes = 0`;<br>• i campi della CPU con i valori neutri di DG9;<br>• `BenchStep { kernel, mode: Compute \| Graphics, rep: 1 }`, uno per fase.<br>Durata attesa: circa un minuto. | Come il §5.2: 4 s di riscaldamento e 5 finestre. |
| DH7 | **Punteggio, mediana e dispersione:**<br>• per ogni carico, la mediana delle finestre e la dispersione = (massimo − minimo) / mediana, contando solo i valori finiti e > 0;<br>• un gruppo senza tutti e tre i suoi carichi non ha punteggio (`None`);<br>• ago dal vivo = 1500 × (`rate` / unità) / riferimento del carico in corso;<br>• **contagiri della GPU** (§3.3 aggiornato): fondo scala = il primo numero della serie 1-2-2,5-5 × 10ⁿ ≥ 1,1 × max(record, riferimento ▲, valore, picco); senza record né riferimento, la base è la stima: 1500 su una GPU dedicata, 20 su una integrata. Durante la misura il fondo scala può solo crescere. | Con 1500 fisso nel fondo scala, una iGPU da 5 punti avrebbe l'ago sempre a zero. |
| DH8 | **Impronta degli shader** (voce aperta della M8b1: il bytecode dipende dalla versione di `fxc.exe`):<br>• `build.rs` di `oma-load` calcola FNV-1a a 64 bit sul bytecode compilato di `s1_fma`, `s2_hash`, `s3_stream` e `bench_gfx`, in quest'ordine, e lo esporta come `OMA_SHADER_DIGEST` (16 cifre esadecimali minuscole);<br>• `oma-load` lo manda in `LoadHello.shader_digest`, e il punteggio lo salva in `shaderDigest`.<br>La classifica (M8d) potrà così separare i build con un compilatore diverso. | Costa poche righe e chiude la voce aperta. |
| DH9 | **Validità** (§5.2, §2.3):<br>• **non valida** (`valid = false`), salvata con i carichi fatti fino a lì:<br>&nbsp;&nbsp;– una discrepanza di S1 o S2, oppure `Finished { reason: FirstError }`: flag `compute_error`;<br>&nbsp;&nbsp;– `Error { kind: device_lost }`: flag `device_lost`;<br>&nbsp;&nbsp;– `Error { kind: hung }`: flag `hung`;<br>• **valida con avviso:**<br>&nbsp;&nbsp;– `throttling`: un campione con il flag termico della GPU acceso (`<device_id>/flag/throttle-thermal` = 1). Il limite di potenza non conta: GPU Boost lavora al limite di potenza per costruzione;<br>&nbsp;&nbsp;– `busy_gpu`: un altro processo sopra il 10% del motore più usato della GPU, all'avvio o in una lettura ogni 5 s (DH10);<br>&nbsp;&nbsp;– `battery`: il PC va a batteria all'avvio o in una lettura ogni 5 s;<br>&nbsp;&nbsp;– `vram_reduced`: S3 ha avuto meno di 1 GiB;<br>• `ReferenceInvalid` fa fallire il solo passo, come nella CPU;<br>• fermato dall'utente, dalla tray o all'uscita: niente salvato; uscita di `oma-load` prima di `Finished`: niente salvato, stato `failed`. | Lo stesso trattamento del benchmark della CPU (DB7). |
| DH10 | **«Altri programmi sulla GPU».**<br>• `Machine::gpu_busy_share(device_id) -> Option<f64>`: il massimo `load_percent / 100` fra i processi di quella GPU nella `GpuProcessTable` esistente;<br>• si escludono i PID 0 e 4, `dwm.exe` (il compositore, che lavora per la nostra finestra) e tutto l'albero dei processi dell'app (il suo PID e i discendenti: `oma-load.exe`, WebView2), da un'istantanea ToolHelp;<br>• `None` se la tabella è vuota o vecchia: nessun avviso. | Senza l'esclusione, la finestra stessa dell'app farebbe scattare l'avviso, come succede per la CPU (voce aperta della M8a2). |
| DH11 | **File del punteggio** (§8.2), sempre `format: 1` e camelCase. Per la GPU:<br>• `category: "gpu"`, `scoreVersion: "gpu-1"`, `isa: null`, `scaling: null`;<br>• `scores: { single: null, multi: null, compute, graphics }`;<br>• `kernels`: una voce per carico, con `value` (mediana, unità vere) e `spread`, e `single`/`multi` a `null`;<br>• `device: { model, cores: 0, logical: 0, deviceId, vendorId, dedicatedBytes, integrated }`;<br>• `shaderDigest`.<br>I campi nuovi si leggono con `#[serde(default)]`, così i punteggi della CPU della M8a2 si leggono ancora; la CPU li scrive a `null` (`compute`, `graphics`, `value`, `spread`, `deviceId`…) e scrive `isa` come prima. `ScoreSummary` ha in più `category`, `compute`, `graphics` e `deviceId`. | Un solo formato e un solo archivio, con il tetto di 500 file comune (§3.6). |
| DH12 | **App e UI:**<br>• un solo lavoro alla volta (DB8): il benchmark della GPU usa lo slot `active` di stress e benchmark della CPU;<br>• `BenchStatus` ha in più `category` (`"cpu"` o `"gpu"`), `deviceId`, `compute` e `graphics`;<br>• la pagina `score-gpu:<deviceId>` mostra lo stato e le misure di quella GPU soltanto; la pagina CPU mostra solo `category == "cpu"`;<br>• la barra laterale ha «Punteggio › CPU» e poi una voce per ogni GPU di `SystemInfo.gpus`, con il nome, e il punto sulla voce del benchmark in corso;<br>• una pagina per un `deviceId` che non c'è più mostra `performance.score.gpu.missing`, con «Avvia» disattivato, e la cronologia di quella GPU;<br>• tray: il tooltip `tray.gpuBenchRunning`; «Apri il test in corso» e il clic sul toast aprono `score-gpu:<deviceId>` (`{"open":"score-gpu","device":"<deviceId>"}`); il toast arriva solo con la finestra non visibile (DB9). | §3.1 («una voce per scheda»), §3.9. |
| DH13 | **Rinviati alla M8d:** «Esporta JSON», «Condividi su GitHub» e il riferimento ▲ «modello della tabella», come per la CPU (DB10). | Hanno senso con la classifica. |

## Review Focus

1. **Punteggi e sessioni salvati dalle milestone precedenti** (punteggi della CPU senza `compute`, `value`, `deviceId`, `shaderDigest`; sessioni con la v4). Atteso: si leggono ancora, e la pagina CPU non mostra mai un punteggio della GPU, né il suo record.

   Test: H6, `cpu_score_files_from_m8a2_still_parse`; H10, `cpu page ignores gpu scores and status`.
2. **Timestamp disgiunte o tempo della GPU nullo** (cambio di clock, GPU in risparmio). Atteso: la finestra si scarta, nessun `NaN` né infinito nelle velocità, nei punti o nell'ago.

   Test: H2, `disjoint_window_is_not_reported`, `zero_ms_window_is_not_reported`; H6, `median_spread_ignores_zero_and_non_finite`.
3. **GPU con poca memoria libera per S3** (iGPU con poca RAM disponibile). Atteso: la copia usa quello che c'è, con l'avviso; sotto i 256 MiB il carico si salta e il Calcolo resta senza punteggio, senza bloccare la Grafica.

   Test: H3, `stream_set_is_capped_by_the_vram_target`, `under_256_mib_skips_the_stream`; H7, `vram_reduced_notice_flags_the_score`, `skipped_load_leaves_its_group_without_points`.
4. **Altri processi sulla GPU, e i nostri.** Atteso: l'avviso solo per un altro programma sopra il 10%; mai per `oma-load`, WebView2 dell'app o `dwm.exe`.

   Test: H8, `descendants_include_grandchildren`, `descendants_survive_a_parent_cycle`; H9, `busy_share_excludes_our_tree_dwm_and_system`.
5. **GPU che si azzera o sparisce durante il benchmark, oppure pagina di una GPU tolta.** Atteso: misura salvata come non valida con «La GPU si è azzerata…», nessuna attesa infinita; la pagina di una GPU che non c'è più non si avvia e non va in errore.

   Test: H7, `device_lost_saves_an_invalid_score`; H9, `unknown_gpu_bench_is_no_gpu`; H10, `missing gpu page disables start`.

## Tabelle dei testi (italiano esatto)

### T1. Pagina, contagiri, misure, tray e toast

| Chiave | Testo |
|---|---|
| `performance.score.gpu.title` | GPU Benchmark |
| `performance.score.compute` | Calcolo |
| `performance.score.graphics` | Grafica |
| `performance.score.gpu.duration` | Circa un minuto. Chiudi giochi e programmi che usano la GPU prima di iniziare. |
| `performance.score.gpu.phase` | {kernel} · {group} |
| `performance.score.gpu.missing` | Questa GPU non c'è più: collegala o scegline un'altra. |
| `performance.score.col.value` | Velocità |
| `performance.score.col.spread` | Dispersione |
| `performance.score.flag.throttling` | La GPU ha rallentato per la temperatura: il punteggio può essere più basso. |
| `performance.score.flag.busy_gpu` | Altri programmi usavano la GPU durante la misura. |
| `performance.score.flag.vram_reduced` | Poca memoria video libera: la banda è misurata su meno di 1 GB. |
| `performance.score.invalid.device_lost` | La GPU si è azzerata durante il benchmark: prova lo stress test. |
| `performance.score.invalid.hung` | La GPU si è bloccata durante il benchmark: prova lo stress test. |
| `performance.toast.gpuBenchDone` | Benchmark della GPU finito: {compute} calcolo, {graphics} grafica. |
| `performance.toast.gpuBenchInvalid` | Benchmark della GPU non valido. |
| `tray.gpuBenchRunning` | Benchmark della GPU in corso |

Le chiavi esistenti si riusano: `performance.score.invalid` (errore di calcolo), `performance.score.flag.battery`, `performance.score.start`, `.stop`, `.points`, `.reference*`, `.detail`, `.col.kernel`, `.history*`, `.delete`, `.provisional`, `.error.*`.

### T2. Glossario (`glossary.<termine>`, con `.name`)

| Chiave | `.name` | Spiegazione |
|---|---|---|
| `computeScore` | Calcolo | Quanto calcola la GPU: operazioni in virgola mobile, operazioni su interi e velocità della memoria video. Conta per l'intelligenza artificiale, il rendering e i programmi di calcolo. |
| `graphicsScore` | Grafica | Quanto disegna la GPU: pixel riempiti, texture lette e strati trasparenti sovrapposti. Conta per i giochi. |
| `spread` | dispersione | Quanto cambiano fra loro i cinque secondi misurati. Sotto il 2% la misura è stabile; sopra, qualcosa ha disturbato la GPU. |
| `tflops` | TFLOPS | Migliaia di miliardi di operazioni in virgola mobile al secondo. TIOPS è lo stesso per le operazioni su interi. |
| `gbps` | GB/s | Miliardi di byte al secondo: quanti dati la GPU legge e scrive nella sua memoria. |

`glossary.gpuBench.<id>`, con `.name`:

| Id | `.name` | Spiegazione |
|---|---|---|
| `fma` | FMA FP32 | Catene di moltiplicazioni e somme in virgola mobile a 32 bit su tutte le unità di calcolo: misura la potenza di calcolo. |
| `int_hash` | Interi | Calcoli su interi (moltiplicazioni, rotazioni, XOR) su milioni di thread: misura le unità di calcolo intere. |
| `bandwidth` | Banda della memoria | Copia 1 GB di dati nella memoria video e misura quanti byte al secondo legge e scrive. |
| `fill` | Riempimento | Disegna rettangoli pieni grandi quanto l'immagine: misura quanti miliardi di pixel al secondo la GPU scrive. |
| `texture` | Texture | Legge 8 punti di una texture per ogni pixel: misura quanti miliardi di texel al secondo la GPU legge. |
| `overdraw` | Sovrapposizione | Disegna strati trasparenti uno sopra l'altro, fondendoli con quelli sotto: è il lavoro di fumo, vetri ed effetti nei giochi. |

### T3. Stress test

| Chiave | Testo |
|---|---|
| `glossary.mode.s3.name` | Flusso di memoria |
| `glossary.mode.s3` | Copia di continuo 1 GB di dati nella memoria video e ne misura la velocità. Un overclock della VRAM che sbaglia viene corretto in silenzio dalla scheda, ma la correzione rallenta la copia: un calo della velocità lo rivela. |

---

## Task

### Task H1: protocollo `oma-ipc::load`, versione 5

**Files:**
- Modify:
  - `crates/oma-ipc/src/load.rs`;
  - `crates/oma-ipc/tests/load_fixtures.rs` (fixture nuova `run_gpu_bench`);
  - `protocol/fixtures/load/*.msgpack`, rigenerate, e `protocol/fixtures/README.md`;
  - i costruttori di `Phase`, `PhaseDone` e `LoadHello` nel resto del workspace, con i valori neutri (`windows: None`, `rates: vec![]`, `shader_digest: None`).

**Interfaces:**
- Consumes: il protocollo v4 (`check_phase`, `KernelId::is_gpu`).
- Produces (DH3):
  - `LOAD_PROTOCOL_VERSION = 5`, `GPU_BENCH_WARMUP_S: u32 = 4`;
  - `KernelId::{S3, Fill, Texture, Overdraw}`, `KernelId::is_gpu_bench_only(self) -> bool`;
  - `Phase.windows: Option<u8>`, `PhaseDone.rates: Vec<f64>`, `LoadHello.shader_digest: Option<String>`, tutti con `#[serde(default)]`;
  - le regole nuove di `validate` di DH3.

- [ ] **Step 1: creare il branch** con `git switch -c feat/m8b2-gpu-bench` da `main`.
- [ ] **Step 2: test che falliscono** (`load.rs`, modulo di test):
  - `bench_gpu_kernels_are_snake_case`: `KernelId::S3` è `"s3"`, `KernelId::Overdraw` è `"overdraw"`;
  - `gpu_bench_phase_round_trips`;
  - `windows_out_of_range_is_rejected`: 0 e 31;
  - `windows_need_a_steady_gpu_phase_without_alt_kernel`: respinti con `k2`, con `ramp` e con `alt_kernel: s1`;
  - `windows_need_the_warmup`: `windows: 5` con `duration_s: 8` respinto, con `duration_s: 9` accettato;
  - `bench_only_gpu_kernels_need_windows`: `fill` senza `windows` respinto;
  - `rates_must_be_finite_and_few`: un `NaN`, un −1 e 65 voci respinti;
  - `v4_messages_still_decode`: `Phase`, `PhaseDone` e `LoadHello` senza le chiavi nuove si leggono con `None` e `vec![]`;
  - `hello_compatibility`: versione 5 sì, 4 no.

  In `tests/load_fixtures.rs`, la fixture `run_gpu_bench`: il piano di DH6 con `luid: 0x17e99`.
- [ ] **Step 3:** `cargo test -p oma-ipc load`. Atteso: FAIL.
- [ ] **Step 4:** implementare, poi rigenerare le fixture con `$env:OMA_WRITE_FIXTURES='1'; cargo test -p oma-ipc --test load_fixtures -- --test-threads=1`.
- [ ] **Step 5:** `cargo test -p oma-ipc` senza la variabile, `cargo build --workspace` (i `match` diventati non esaustivi danno, per ora, il ramo che rifiuta: `workload` restituisce `Ok(None)`), clippy. Atteso: PASS.
- [ ] **Step 6: commit** `feat(ipc): load protocol v5 with GPU bench phases, window rates and shader digest`.

### Task H2: `oma-load`, finestre misurate e fasi del benchmark della GPU

**Files:**
- Modify:
  - `crates/oma-load/src/gpu/submit.rs` (`window_begin`, `window_end`);
  - `crates/oma-load/src/gpu/engine.rs` (fasi con `windows`, `work_per_submission`);
  - `crates/oma-load/src/gpu/compute.rs` (lavoro per invio di S1 e S2, DH2);
  - `crates/oma-load/src/gpu/tests.rs` (il `Submit` finto con le finestre);
  - `crates/oma-load/build.rs` (DH8) e `crates/oma-load/src/link.rs` (`shader_digest` in `Hello`).

**Interfaces:**
- Consumes: H1; `Submit`, `GpuWorkload`, `run_gpu_with` e `Hooks` di oggi.
- Produces:
  - **`Submit`:** `fn window_begin(&mut self) -> Result<(), GpuError>` e `fn window_end(&mut self) -> Result<Option<f64>, GpuError>` (DH4). `Submitter` usa una query `TIMESTAMP_DISJOINT` e due `TIMESTAMP` proprie della finestra, separate da quelle di `gpu_ms`; `window_end` attende con le stesse regole di `wait_for` (1 s, `Hung` o `Lost`).
  - **`GpuWorkload::work_per_submission(&self) -> f64`**, con il valore predefinito `0.0`; S1 = 128 × passi × `THREADS`; S2 = operazioni per passo × passi × `THREADS`.
  - **Motore:** le fasi con `windows` secondo DH4, `PhaseDone.rates` e `Progress.rate` in unità di base al secondo.
  - **`build.rs`:** `cargo:rustc-env=OMA_SHADER_DIGEST=<16 cifre>` (DH8; `s3_stream` e `bench_gfx` entrano nell'impronta con H3 e H4, che aggiungono i loro shader all'elenco); `fn fnv1a64(bytes: &[u8]) -> u64` pura nel file.
  - **`link`:** `LoadHello.shader_digest = Some(env!("OMA_SHADER_DIGEST").to_owned())`.

- [ ] **Step 1: test che falliscono.** I test puri usano il `Submit` e il `GpuWorkload` finti, al massimo 3 s ciascuno:
  - `bench_phase_reports_its_windows`: 5 finestre con un lavoro fisso per invio e 10 ms di GPU per invio danno 5 `rates` uguali al valore atteso;
  - `warmup_windows_are_not_reported`: con `GPU_BENCH_WARMUP_S` = 4, i primi 4 secondi non finiscono in `rates`;
  - `bench_phase_ends_after_its_windows`: la fase finisce prima di `duration_s`;
  - `bench_phase_stops_at_the_cap`: con finestre tutte disgiunte, la fase finisce a `duration_s` con `rates` vuoto;
  - `disjoint_window_is_not_reported` e `zero_ms_window_is_not_reported`;
  - `progress_rate_is_the_last_window_in_units`;
  - `bench_phase_without_work_is_unsupported`;
  - `timed_phases_have_no_rates`: una fase senza `windows` ha `rates` vuoto e `Progress.rate` in invii al secondo, come prima;
  - `fma_work_counts_128_flop_per_step` (`compute.rs`);
  - `hello_carries_the_shader_digest`: 16 cifre esadecimali minuscole;
  - `bench_window_times_the_gpu` (`#[ignore = "requires real Windows hardware"]`, meno di 2 s): una fase `s1` con `windows: 1` sulla prima GPU dà una velocità finita e > 0.
- [ ] **Step 2:** `cargo test -p oma-load gpu`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, poi `cargo test -p oma-load gpu -- --include-ignored`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): GPU bench phases timed with per-window timestamp queries`.

### Task H3: `oma-load`, S3, il flusso di memoria

**Files:**
- Create: `crates/oma-load/shaders/s3_stream.hlsl`, `crates/oma-load/src/gpu/stream.rs`.
- Modify: `crates/oma-load/build.rs` (shader e impronta), `crates/oma-load/src/gpu/shaders.rs`, `crates/oma-load/src/gpu/engine.rs` (`workload` per `s3`), `crates/oma-load/src/gpu/mod.rs`.

**Interfaces:**
- Consumes: H2; `sizing::{vram_target, chunk_bytes}`, `VramBudget`, `calibrate`.
- Produces:
  - `pub const STREAM_BYTES: u64 = 1 << 30`;
  - `pub fn stream_set(target: u64, chunk: u64) -> Option<(u64, u32)>`, pura: la dimensione dell'insieme e il numero di pezzi per DH5, `None` sotto i 256 MiB;
  - `StreamLoad`, che implementa `GpuWorkload` secondo DH5, con `work_per_submission` = elementi `float4` copiati × 32;
  - le notice `vram_allocated` e, con meno di 1 GiB, `vram_reduced`.

- [ ] **Step 1: test che falliscono:**
  - `stream_set_is_1_gib_when_it_fits`: obiettivo 14 GiB e pezzi da 512 MiB danno (1 GiB, 2);
  - `stream_set_is_capped_by_the_vram_target`: obiettivo 700 MiB e pezzi da 256 MiB danno (512 MiB, 2);
  - `under_256_mib_skips_the_stream`;
  - `stream_copies_every_piece` (`#[ignore = "requires real Windows hardware"]`, meno di 2 s): dopo un numero di invii pari ai pezzi, ogni destinazione è uguale alla sua sorgente (letta all'indietro in un campione di 4096 parole);
  - `stream_bandwidth_is_plausible` (`#[ignore = "requires real Windows hardware"]`, meno di 2 s): la velocità di una finestra sta fra 1 GB/s e 5000 GB/s.
- [ ] **Step 2:** `cargo test -p oma-load stream`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, poi `cargo test -p oma-load stream -- --include-ignored`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): S3 memory stream load`.

### Task H4: `oma-load`, i carichi grafici del benchmark

**Files:**
- Create: `crates/oma-load/shaders/bench_gfx.hlsl` (VS del quad a tutto schermo per istanza, PS con il modo nel constant buffer), `crates/oma-load/src/gpu/bench_gfx.rs`.
- Modify: `crates/oma-load/build.rs`, `crates/oma-load/src/gpu/shaders.rs`, `crates/oma-load/src/gpu/engine.rs` (`workload` per `fill`, `texture` e `overdraw`), `crates/oma-load/src/gpu/mod.rs`.

**Interfaces:**
- Consumes: H2; `graphics::{WIDTH, HEIGHT}`.
- Produces:
  - `enum BenchGfxKind { Fill, Texture, Overdraw }` e `BenchGfxLoad`, che implementa `GpuWorkload` secondo DH2: le istanze tarate sull'invio (DG4), stato della pipeline legato a ogni invio e sciolto dopo;
  - `pub const TEXTURE_READS: u32 = 8`;
  - `pub fn gfx_work(kind: BenchGfxKind, instances: u32) -> f64`, pura: pixel o texel di un invio;
  - il test ignorato `gpu_bench_rates`, che stampa con `--nocapture` la velocità di una finestra per ciascuno dei sei carichi del benchmark sulla prima GPU, in unità vere (al massimo 2 s per carico). Servono a H6 per i riferimenti provvisori.

- [ ] **Step 1: test che falliscono:**
  - `gfx_work_counts_full_screen_pixels`: 10 istanze di `fill` danno 20 736 000 pixel; 10 di `texture` danno 165 888 000 texel;
  - `bench_gfx_calibrates_near_the_target` (`#[ignore = "requires real Windows hardware"]`): per i tre modi, l'invio tarato dura fra il 50 e il 200% dell'obiettivo;
  - `overdraw_is_slower_than_fill` (`#[ignore = "requires real Windows hardware"]`): sulla stessa GPU la velocità di `overdraw` è minore o uguale a quella di `fill`.
- [ ] **Step 2:** `cargo test -p oma-load bench_gfx`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, `cargo test -p oma-load bench_gfx -- --include-ignored`, clippy, poi una volta `cargo test -p oma-load gpu_bench_rates -- --ignored --nocapture`. Annotare le sei velocità nel messaggio di commit. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): fill, texture and overdraw GPU bench loads`.

### Task H5: `oma-core::load`, S3 nello stress e flag termico della GPU

**Files:**
- Modify:
  - `crates/oma-core/src/load/plan.rs` (giro d'overclock, DH5);
  - `crates/oma-core/src/load/catalog.rs` e `testdata/performance/catalog.json`, rigenerato;
  - `crates/oma-core/src/load/run.rs` (`s3` fra i kernel della stabilità);
  - `crates/oma-core/src/load/sensors.rs` (flag termico).

**Interfaces:**
- Consumes: H1.
- Produces:
  - `gpu_phases(Overclock, d)` con le otto fasi di DH5;
  - `gpuKernels` del catalogo con `s3` (sei voci);
  - i rate di `s3` `steady` nello `StabilityMeter`;
  - `GpuSensorIds.thermal: Option<usize>` (`<device_id>/flag/throttle-thermal`) e `SensorSample.thermal_throttling: Option<bool>`, `None` per la CPU; `throttling` resta com'è (potenza o temperatura, per lo stress).

- [ ] **Step 1: test che falliscono:**
  - `gpu_overclock_round_has_the_eight_phases`: Standard (1800) dà `s4` 270, `s3` 180, `s2` 180, `s1` 180, `s6` 270, `s1` `ramp` 360, `s1` `alternate` 180, `s1` `pause_resume` 180, tutte con `stop_on_error`;
  - `gpu_phase_totals_equal_the_preset`, aggiornato;
  - `s3_rates_count_for_stability` (`run.rs`);
  - `catalog_lists_gpu_entries`: `gpuKernels` ha 6 voci;
  - `thermal_flag_is_read_apart_from_power` (`sensors.rs`): con `throttle-power` = 1 e `throttle-thermal` = 0, `throttling == Some(true)` e `thermal_throttling == Some(false)`.

  Il test `gpu_overclock_round_has_the_seven_phases` di oggi lascia il posto al primo.
- [ ] **Step 2:** `cargo test -p oma-core load`. Atteso: FAIL.
- [ ] **Step 3:** implementare, poi rigenerare il catalogo con `$env:OMA_WRITE_FIXTURES='1'; cargo test -p oma-core catalog_fixture_matches`.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): S3 in the GPU overclock round and the GPU thermal flag`.

### Task H6: `oma-core::scores`, carichi, piano, punteggio e file della GPU

**Files:**
- Create:
  - `crates/oma-core/src/scores/gpu.rs` (carichi, piano, riferimenti, punti, mediana e dispersione);
  - `crates/oma-core/src/scores/gpu-1-baseline.json`;
  - `crates/oma-core/examples/calibrate_gpu.rs`.
- Modify:
  - `crates/oma-core/src/scores/workloads.rs`, `plan.rs`, `file.rs`, `score.rs`, `mod.rs`;
  - `crates/oma-core/src/scores/bench.rs` e i suoi test, per i campi nuovi del file (la CPU li scrive a `null`).

**Interfaces:**
- Consumes: H1; le velocità di H4.
- Produces:
  - `BenchKernel` con in più `Fma`, `IntHash`, `Bandwidth`, `Fill`, `Texture`, `Overdraw` (`"fma"`, `"int_hash"`…); `BenchMode` con in più `Compute` e `Graphics`. `WORKLOADS` resta quello dei sei carichi della CPU;
  - `struct GpuLoad { id: BenchKernel, kernel: KernelId, mode: BenchMode, unit: &'static str, per_unit: f64 }` e `pub const GPU_LOADS: [GpuLoad; 6]`, nell'ordine di DH6, con le unità `"TFLOPS"`, `"TIOPS"`, `"GB/s"`, `"Gpixel/s"`, `"Gtexel/s"`, `"Gpixel/s"` e `per_unit` 10¹², 10¹², 10⁹, 10⁹, 10⁹, 10⁹;
  - `pub const GPU_WINDOWS: u8 = 5`, `pub const GPU_CAP_S: u32 = 30`, `pub const GPU_SCORE_VERSION: &str = "gpu-1"`;
  - `pub fn gpu_bench_plan(target: GpuTarget, seed: u64) -> (Plan, Vec<BenchStep>)` (DH6);
  - `struct GpuBaseline { version: String, provisional: bool, compute: BTreeMap<BenchKernel, f64>, graphics: BTreeMap<BenchKernel, f64> }`, `pub fn gpu_baseline() -> &'static GpuBaseline` (con `include_str!`), che rifiuta un file senza i tre carichi per gruppo o di un'altra versione;
  - `pub fn median_spread(rates: &[f64]) -> Option<(f64, f64)>` (DH7);
  - `pub fn gpu_points(rates: &BTreeMap<BenchKernel, f64>, mode: BenchMode, baseline: &GpuBaseline) -> Option<u32>`, che riusa la media geometrica di `score.rs`;
  - `pub fn gpu_calibration_from(score: &ScoreFile) -> Result<GpuBaseline, BaselineError>`: rifiuta un file non valido, con flag, di un'altra versione o senza i sei carichi; l'esempio `cargo run -p oma-core --example calibrate_gpu -- <score.json>` stampa modello, `deviceId`, VRAM e mediane, poi riscrive `gpu-1-baseline.json` con 4 cifre significative e `provisional: false`;
  - **file (DH11):** `ScoreFile.isa: Option<Isa>`, `ScoreFile.shader_digest: Option<String>`, `Scores.{compute, graphics}`, `KernelRate.{value, spread}`, `Device.{device_id, vendor_id, dedicated_bytes, integrated}` con `#[serde(rename_all = "camelCase")]`, `ScoreSummary.{category, compute, graphics, device_id}`;
  - **riferimenti provvisori:** le sei velocità di H4, con `"provisional": true`.

- [ ] **Step 1: test che falliscono:**
  - `gpu_bench_plan_has_six_phases_compute_then_graphics`;
  - `gpu_bench_plan_validates` (con `LoadMessage::Run(...).validate()`);
  - `median_spread_of_five_windows`: `[10.0, 11.0, 9.0, 10.0, 12.0]` dà `(10.0, 0.3)`;
  - `median_spread_ignores_zero_and_non_finite`;
  - `gpu_points_at_the_baseline_are_1500`;
  - `doubling_the_graphics_rates_doubles_the_graphics_points`;
  - `group_without_all_three_loads_has_no_points`;
  - `gpu_baseline_parses_and_has_three_loads_per_group`;
  - `gpu_calibration_refuses_an_invalid_flagged_or_partial_run`;
  - `cpu_score_files_from_m8a2_still_parse`: un JSON scritto a mano nel formato della M8a2, senza i campi nuovi, si legge con `compute: None`, `value: None`, `device_id: None`, `shader_digest: None`;
  - `gpu_score_file_round_trips`.
- [ ] **Step 2:** `cargo test -p oma-core scores`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): GPU benchmark loads, plan, fixed-scale points and score file fields`.

### Task H7: `oma-core::scores`, il controller del benchmark della GPU

**Files:**
- Create:
  - `crates/oma-core/src/scores/lifecycle.rs`: la parte comune ai due controller, estratta da `bench.rs`;
  - `crates/oma-core/src/scores/gpu_bench.rs`.
- Modify: `crates/oma-core/src/scores/bench.rs`, `crates/oma-core/src/scores/mod.rs`.

**Interfaces:**
- Consumes: H5 (`SensorSample.thermal_throttling`), H6; `BenchAction`, `BenchEnd`, `BenchState`, `SegmentState` di oggi.
- Produces:
  - **`lifecycle`:** una struct privata del modulo `scores` con stato, orologio, scadenza dello stop, ultimo messaggio, tetto del piano, flag, campioni ogni 5 s, versione di `oma-load`, id e errore. Ci passano `on_user_stop`, `on_clock`, `on_exit` e il campionamento. Il comportamento della CPU non cambia: i test di `bench.rs` passano senza modifiche, a parte i campi nuovi di `BenchStatus`.
  - **`BenchStatus`:** in più `category: String`, `device_id: Option<String>`, `compute: Option<u32>`, `graphics: Option<u32>`; la CPU dà `"cpu"` e `None`.
  - **`GpuBenchController`:** `new(steps: Vec<BenchStep>, ctx: GpuBenchContext, now: Clock)`, con `GpuBenchContext { id, at, device: Device, device_id: String, integrated: bool, on_battery: Option<bool>, baseline: &'static GpuBaseline, app_version: String }`; gli stessi metodi del controller della CPU (`on_load`, `on_sample`, `on_battery`, `on_user_stop`, `on_clock`, `on_exit`, `status`, `is_finished`) e in più `on_busy_gpu(share: f64)`.
  - **Regole:**
    - `PhaseDone` con `rates` dà mediana e dispersione del carico (DH7); senza velocità utili, o con `skipped`, il passo è `failed`;
    - `Progress.rate > 0` muove l'ago (DH7); a fine fase l'ago tiene la mediana del carico se il passo dopo è dello stesso gruppo;
    - `compute` e `graphics` di `status()` appaiono appena i passi del gruppo sono finiti;
    - validità e flag di DH9, nell'ordine `battery`, `throttling`, `busy_gpu`, `vram_reduced`, `compute_error`, `device_lost`, `hung`;
    - `Hello.shader_digest` finisce in `shaderDigest` del file (DH8);
    - `Finished { reason: Completed }` salva il file di DH11 e chiude con `Saved(id)`.

- [ ] **Step 1: test che falliscono** (`gpu_bench.rs`, messaggi scritti a mano, orologio finto):
  - `full_gpu_run_saves_both_scores`;
  - `live_points_follow_progress_rate`;
  - `skipped_load_leaves_its_group_without_points`: `bandwidth` saltato lascia `compute: None` e `graphics` presente;
  - `mismatch_saves_an_invalid_score`;
  - `device_lost_saves_an_invalid_score` e `hung_saves_an_invalid_score`;
  - `vram_reduced_notice_flags_the_score`;
  - `thermal_sample_flags_throttling_but_power_does_not`;
  - `busy_gpu_above_ten_percent_flags`;
  - `user_stop_saves_nothing` e `early_exit_fails_without_saving`;
  - `status_carries_category_and_device`.
- [ ] **Step 2:** `cargo test -p oma-core scores`. Atteso: FAIL.
- [ ] **Step 3:** estrarre `lifecycle` (i test di `bench.rs` devono restare verdi), poi implementare il controller.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): GPU benchmark controller sharing the CPU one's lifecycle`.

### Task H8: `oma-win`, l'albero dei processi

**Files:**
- Create: `crates/oma-win/src/process_tree.rs`.
- Modify: `crates/oma-win/src/lib.rs`.

**Interfaces:**
- Produces:
  - `pub fn descendants_of(pairs: &[(u32, u32)], root: u32) -> HashSet<u32>`, pura: dato l'elenco `(pid, pid del padre)`, il `root` e tutti i suoi discendenti, a qualsiasi profondità; regge i cicli (un PID riusato che risulta padre di un suo antenato);
  - `pub fn descendants(root: u32) -> io::Result<HashSet<u32>>`: l'istantanea `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)`, `Process32FirstW` / `Process32NextW`, con l'handle chiuso da una guardia.

- [ ] **Step 1: test che falliscono:**
  - `descendants_include_grandchildren`;
  - `descendants_survive_a_parent_cycle`;
  - `unrelated_processes_are_left_out`;
  - `this_process_is_its_own_root` (`#[ignore = "requires real Windows hardware"]`): `descendants(std::process::id())` contiene il PID corrente.
- [ ] **Step 2:** `cargo test -p oma-win process_tree`. Atteso: FAIL.
- [ ] **Step 3:** implementare, con `// SAFETY:` sulle chiamate FFI.
- [ ] **Step 4:** `cargo test -p oma-win`, poi `cargo test -p oma-win process_tree -- --include-ignored`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(win): descendants of a process from a ToolHelp snapshot`.

### Task H9: app, runner, comandi, tray e toast del benchmark della GPU

**Files:**
- Modify:
  - `app/src-tauri/src/performance/bench.rs` (avvio e ciclo del benchmark della GPU, accanto a quelli della CPU);
  - `app/src-tauri/src/performance/runner.rs` (`Machine::gpu_busy_share`, `WinMachine` con la `GpuProcessTable`, il banco di prova dei test);
  - `app/src-tauri/src/performance/commands.rs`, `app/src-tauri/src/main.rs`, `app/src-tauri/capabilities/default.json`;
  - `app/src-tauri/src/notifier.rs` (`LaunchTarget::ScoreGpu(String)`, `launch_for_score_gpu`), il tray;
  - `app/src-tauri/src/i18n.rs` (`RUST_KEYS`: `tray.gpuBenchRunning`, `performance.toast.gpuBenchDone`, `performance.toast.gpuBenchInvalid`), `en.json` e `it.json` (T1).

**Interfaces:**
- Consumes: H6, H7, H8; `Machine::gpus()`, `resolve_gpu_sensors`, `read_gpu_sample`.
- Produces:
  - `Machine::gpu_busy_share(&self, device_id: &str) -> Option<f64>` (DH10); `WinMachine::new(processes: GpuProcessTable)`; in `FakeMachine` un valore dato;
  - `PerformanceRunner::start_gpu_bench(&self, device_id: &str) -> Result<String, StartError>`: risolve il `device_id` fra `machine.gpus()` (sconosciuto: `StartError::Plan(BuildError::NoGpu)`, che `wire()` dà già come `build:no_gpu`), costruisce il piano con `gpu_bench_plan`, campiona i sensori della GPU, legge `gpu_busy_share` e `on_battery` all'avvio e ogni 5 s; `stop_bench` e `bench_status` valgono per entrambi i benchmark;
  - comandi: `performance_gpu_bench_start(device_id: String) -> Result<String, String>` (`busy` o `build:no_gpu`); `performance_baseline` dà `{ provisional, gpuProvisional }`;
  - tray e toast secondo DH12, con `launch_for_score_gpu(device_id)` = `{"open":"score-gpu","device":"<device_id>"}`; `launch_target` accetta il `device` solo con `"open":"score-gpu"` e solo nella forma dei `device_id` della GPU.

- [ ] **Step 1: test che falliscono** (banco di prova del runner, con `Launcher` e `Machine` finti):
  - `gpu_bench_runs_to_a_saved_score`;
  - `unknown_gpu_bench_is_no_gpu`;
  - `gpu_bench_while_stress_runs_is_busy`, e il contrario;
  - `gpu_bench_samples_come_from_gpu_sensors`;
  - `busy_share_is_polled_every_five_seconds`;
  - `busy_share_excludes_our_tree_dwm_and_system`, sull'helper puro che filtra le righe della tabella con un insieme di PID;
  - `gpu_bench_toast_opens_the_gpu_page`, con `launch_target` che restituisce `ScoreGpu`;
  - `launch_target_refuses_a_malformed_device`.
- [ ] **Step 2:** `cargo test -p oma-app performance`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test --workspace`, clippy, `cargo test -p oma-app i18n`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): GPU benchmark runner, commands, tray and toast`.

### Task H10: UI, pagina di punteggio della GPU

Prima: `frontend-design:frontend-design`, con la pagina CPU di oggi come modello (stessi contagiri, stessa disposizione).

**Files:**
- Create: `app/src/components/performance/ScorePage.svelte` (da `CpuScore.svelte`, con `git mv`) e il suo test (da `CpuScore.test.ts`).
- Modify:
  - `app/src/components/performance/PerformanceView.svelte` (voci per GPU, DH12);
  - `app/src/lib/view.ts` (`PerformancePage` con `` `score-gpu:${string}` ``, e il clic sul toast);
  - `app/src/lib/performance/bench.svelte.ts` e il suo test;
  - `app/src/lib/performance/gauge.ts` e il suo test;
  - `app/src/lib/performance/glossary.ts` e il suo test;
  - `app/src/lib/types.ts`;
  - `app/src/lib/backend/backend.ts`, `tauri.ts`, `mockPerformance.ts`, `app/src/test/fake-backend.ts`;
  - `en.json` e `it.json` (T1, T2, T3).

**Interfaces:**
- Consumes: i comandi e l'evento di H9; `Gauge.svelte`.
- Produces:
  - **`gauge.ts`:** `gpuFullScale(values: number[], integrated: boolean): number` (DH7), con la serie comune a `fullScale`;
  - **`Backend`:** `performanceGpuBenchStart(deviceId)`; `performanceBaseline()` con `gpuProvisional`. Il mock simula 12 s di benchmark della GPU, e con `?bench=error` una misura non valida;
  - **`BenchStore`:** `start(target)`, con `type ScoreTarget = { category: 'cpu' } | { category: 'gpu'; deviceId: string }`; `scoresFor(target)`, `recordFor(target)`, `lastFor(target)` e `statusFor(target)`, che filtrano per `category` e `deviceId`;
  - **`ScorePage.svelte`:** prop `target: ScoreTarget & { name?: string; integrated?: boolean }`:
    - CPU: come oggi;
    - GPU: titolo `performance.score.gpu.title` con il nome della GPU sotto; contagiri Calcolo e Grafica (`Term` `computeScore` e `graphicsScore`); la fase con `performance.score.gpu.phase`; la tabella con Velocità e Dispersione (`Term` `spread`, e le unità con `Term` `tflops` o `gbps` dove servono); i flag di T1; il messaggio di non validità del flag (`invalid`, `invalid.device_lost`, `invalid.hung`); `performance.score.gpu.missing` per una GPU che non c'è più;
  - **`PerformanceView`:** legge `performanceSystem()` una volta, all'apertura della vista, per le voci delle GPU;
  - **glossario:** `GPU_BENCH_TERMS` (`gpuBench.<id>` dei sei carichi) e i termini di T2 in `TERMS`.

- [ ] **Step 1: test che falliscono:**
  - `gauge.test.ts`: `gpu_full_scale_without_record_uses_the_estimate` (([], false) → 2000; ([], true) → 25); `gpu_full_scale_follows_the_record` (([150], false) → 200; ([5.2], true) → 10; ([1500], false) → 2000);
  - `bench.test.ts`: `scores_and_record_are_per_target`;
  - `ScorePage.test.ts`:
    - i test di `CpuScore.test.ts`, con `target={{ category: 'cpu' }}`;
    - `cpu page ignores gpu scores and status`;
    - `gpu page runs and shows compute and graphics`;
    - `gpu detail shows value and spread`;
    - `gpu flags and invalid reasons show their text`;
    - `missing gpu page disables start`;
  - `PerformanceView.test.ts`: `sidebar lists one score entry per gpu`;
  - `glossary.test.ts`: `every_gpu_bench_load_has_an_entry`; il conteggio delle modalità sale di 1 (`mode.s3`).
- [ ] **Step 2:** `cd app && pnpm test`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): GPU score pages with compute and graphics gauges`.

### Task H11: documenti e grafo

**Files:**
- Modify:
  - `CLAUDE.md`: struttura (`oma-core::scores` con la GPU, `gpu/stream.rs`, `gpu/bench_gfx.rs`, `process_tree`, `ScorePage.svelte`), protocollo `load` v5, stato della M8b2;
  - `README.md` e `README.it.md`: il benchmark della GPU;
  - `docs/perf-budget.md`: sezione «M8b2», con la nota che la misura con la build release si fa con la B11;
  - `docs/follow-ups.md`: chiudere le voci della M8b1 risolte (S3, avviso degli altri processi, impronta degli shader) e aprire quelle della M8b2.

- [ ] **Step 1:** aggiornare i documenti.
- [ ] **Step 2: verifiche:**
  - `pwsh scripts/generate-licenses.ps1 -Check`;
  - `cargo fmt --all --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo test --workspace`;
  - `cd app && pnpm test && pnpm check && pnpm build`;
  - `PYTHONHASHSEED=0 graphify update .`.

  Atteso: PASS.
- [ ] **Step 3: commit** `docs: M8b2 GPU benchmark documentation`.

### Task H12: taratura e prove dal vivo con l'utente

Le fa l'utente, un blocco alla volta (memoria «user admin shell»). Ogni benchmark e ogni stress test sono un carico pesante sulla GPU: si chiede prima. Niente input sintetico. Prima: `cargo build -p oma-load` e `cargo build -p oma-overlay`, poi `cd app; pnpm tauri dev`.

| # | Prova | Atteso |
|---|---|---|
| R1 | **Taratura.** L'utente conferma la RTX 4080 senza overclock, chiude giochi e programmi sulla GPU e avvia il benchmark dalla pagina della 4080. Poi `cargo run -p oma-core --example calibrate_gpu -- <file del punteggio>`, commit del nuovo `gpu-1-baseline.json` e nuova build di `oma-load` e dell'app. Se l'utente preferisce, la taratura si rimanda alla fine della M8 insieme alla B11. | Misura valida senza flag; `provisional: false`. |
| R2 | Secondo benchmark sulla 4080, dopo la taratura (le misure provvisorie si cancellano dalla pagina) | Calcolo e Grafica a 1500 ± 3%; dispersione sotto il 2%; circa un minuto; contagiri fluidi; ▲ sul record. |
| R3 | Benchmark sulla iGPU AMD | Misura valida, punti sotto 10, fondo scala adatto (10 o 25), desktop fluido. |
| R4 | `$env:OMA_LOAD_INJECT='s1'` e benchmark sulla 4080 | «Errore di calcolo durante il benchmark: prova lo stress test», misura non valida nell'elenco. |
| R5 | «Ferma» a metà, poi «Ferma il test» dalla tray | Nessuna misura nuova in entrambi i casi. |
| R6 | Finestra chiusa durante il benchmark | Il benchmark continua, arriva il toast, e il clic apre la pagina della GPU giusta. |
| R7 | Benchmark durante uno stress test, e il contrario | «È già in corso uno stress test o un benchmark.» |
| R8 | Un video o un gioco aperto sulla 4080 durante il benchmark; poi un benchmark con la sola finestra dell'app | Flag «Altri programmi usavano la GPU…» solo la prima volta. |
| R9 | Stress «Stabilità overclock» con Personalizza: solo S3, 3 minuti | «Superato», stabilità della velocità nel riepilogo. |
| R10 | Tooltip | Ogni termine della pagina della GPU (T2) e `mode.s3` mostra la spiegazione con il mouse e con Tab. |
| R11 | Budget durante il benchmark | Annotato in sviluppo; la misura con la build release resta con la B11. |

- [ ] **Step 1:** preparare i comandi esatti e chiedere il via per R1.
- [ ] **Step 2:** dopo R1, commit `feat(core): calibrate the gpu-1 scale on the reference GPU`.
- [ ] **Step 3:** registrare gli esiti in `docs/follow-ups.md` e nella memoria del progetto (`m8b2-followups.md`), poi commit `docs: record the M8b2 live checks`.
- [ ] **Step 4:** `superpowers:finishing-a-development-branch`: revisione dell'intero branch e merge in `main` in locale, senza push se l'utente non lo chiede.
