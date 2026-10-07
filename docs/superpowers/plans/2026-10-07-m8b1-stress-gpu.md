# M8b1 — Stress test della GPU: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** aggiungere lo stress test della GPU alla vista Prestazioni:
- una voce per ogni GPU nella procedura guidata, con i profili «Verifica normale» e «Stabilità overclock» del §5.4;
- carichi D3D11 verificati in `oma-load.exe`, con taratura degli invii, device perso e invio bloccato;
- stop termico sulla temperatura della GPU (`gpuStopC`), stabilità della velocità ≥ 97%, contatore dei replay PCIe;
- verdetti, cronologia e tooltip come per CPU e RAM.

**Architecture:**
- **`oma-ipc::load` v4 (G1):** la GPU bersaglio nel piano, i kernel `s1`–`s6` (senza `s3`, DG1) e i modi `ramp`, `alternate`, `pause_resume`, l'errore `device_lost`, il livello di carico.
- **`oma-core` (puro):** catalogo, profili e piani della GPU (G2); esiti, sensori della GPU, stabilità, replay PCIe e sessione (G3).
- **`oma-win` (G4):** elenco delle GPU adatte allo stress (LUID, `device_id` dello schema) e lettura del contatore dei replay PCIe da NVML.
- **`oma-load`:** shader HLSL compilati al build con `fxc` e fondamenta D3D11 (G5); motore delle fasi GPU con S1, S2, S7, S8 e S9 (G6); S4, la verifica della VRAM (G7); S5 e S6, il carico grafico e la scansione degli artefatti (G8).
- **App:** impostazione `gpuStopC` (G9); runner, comandi, sensori e tray per la GPU (G10).
- **UI:** procedura guidata e Personalizza (G11); durante il test, risultato, cronologia e glossario (G12).
- **Chiusura:** licenze, documenti e misure (G13); prove dal vivo con l'utente (G14).

**Tech Stack:** Rust 1.90 (workspace `rust-version` 1.85, `oma-load` 1.89), crate `windows` 0.62 (D3D11, DXGI), HLSL `cs_5_0`/`vs_5_0`/`ps_5_0` compilato con `fxc.exe` del Windows SDK, Tauri 2.11 + Svelte 5 + TypeScript 6 + Vitest.

**Spec:** `docs/superpowers/specs/2026-10-06-m8-prestazioni-design.md`:
- §2 (con `device_lost` del §2.3 e il codice d'uscita 4 del §2.2), §2.6 per la soglia della GPU, §3.4–§3.9;
- §5.1, §5.3, §5.4, §5.5; §8.1; §9; §11; §12 per memtest_vulkan; §13.

Esiti dello spike, che correggono il §5.1: `docs/superpowers/references/m8/spike-gpu.md`. Ricerca con le fonti: `docs/superpowers/references/m8/research-gpu.md` (§3, §4, §8).

**Branch:** `feat/m8b1-gpu-stress` da `main` (G1 passo 1). Alla fine il merge in `main` si fa in locale, solo dopo le prove dal vivo o su richiesta dell'utente. Push e release solo su richiesta dell'utente.

**Esecuzione:**
- **Subagent-driven:** un implementer e una revisione per task, poi la revisione dell'intero branch.
- **Revisioni in più:**
  - `ffi-safety-reviewer` dopo G4, G5, G6, G7 e G8;
  - `security-review` dopo G1 e G10: piano ricevuto, LUID e `device_id` dalla UI.
- **Prove dal vivo:** G14, con l'utente.

## Global Constraints

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
  - un agente non avvia mai uno stress test vero, né `oma-load.exe` con un piano della GPU o della CPU intero;
  - i test che usano la GPU vera sono `#[ignore = "requires real Windows hardware"]`;
  - un agente li può eseguire solo con carichi minimi: ogni test usa al massimo 2 s di GPU, e una verifica completa al massimo 30 s in tutto;
  - i test automatici della CPU restano quelli della M8a1: al massimo 2 thread e 3 s per prova;
  - la suite completa si esegue una volta per verifica, non in un ciclo.
- **Risparmio (preferenza dell'utente):**
  - senza un test, `oma-load` non esiste e la vista Prestazioni non fa lavoro periodico;
  - l'elenco delle GPU si legge solo quando la UI lo chiede (`performance_system`), mai in un timer;
  - durante un test della GPU `oma-load` aspetta gli invii con `sleep(1)`, mai con l'attesa attiva (spike, Q7).
- **TDD e debug:** prima il test che fallisce (`superpowers:test-driven-development`); davanti a un test che fallisce senza un motivo chiaro, `superpowers:systematic-debugging`.
- **graphify:** ogni brief riporta `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"` per orientarsi. Dopo le modifiche al codice: `PYTHONHASHSEED=0 graphify update .`.
- **FFI Rust:**
  - un commento `// SAFETY:` per ogni blocco `unsafe`;
  - un assert di dimensione a compile time per ogni struct FFI scritta a mano (le struct del crate `windows` non ne hanno bisogno);
  - helper puri testati senza hardware;
  - le DLL dei vendor (NVML) si caricano solo da System32 con `dynlib::Library`, già usato in `oma-win`.
- **Protocollo `oma-ipc::load`:**
  - mai `skip_serializing_if`: chiavi sempre presenti, `nil` per gli assenti;
  - enumerati come stringhe `snake_case`;
  - niente `deny_unknown_fields`;
  - i campi nuovi si leggono con `#[serde(default)]`, così le sessioni salvate con le versioni precedenti si leggono ancora;
  - il ricevente chiama `LoadMessage::validate`;
  - fixture in `protocol/fixtures/load/`, rigenerate solo con `OMA_WRITE_FIXTURES=1`, a thread singolo.
- **Dipendenze:**
  - nessun nuovo crate, nessun pacchetto npm nuovo;
  - feature nuove del crate `windows`, solo in `oma-load`: `Win32_Graphics_Direct3D`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`;
  - nessuna dipendenza da `d3dcompiler_47.dll` a runtime: gli shader arrivano compilati dal build (DG3);
  - `pwsh scripts/generate-licenses.ps1 -Check` deve passare.
- **Codice di terzi (§12):** il file HLSL e il modulo Rust di S4 portano in testa la nota d'origine di memtest_vulkan dettata in G7; nessun tag SPDX di terzi.
- **Nomi fissi:**
  - stesso eseguibile `oma-load.exe`, stessa pipe, stesse cartelle e stesso diario della M8a1;
  - chiave `gpuStopC` nella sezione `performance` di `settings.json`;
  - chiavi `glossary.mode.<id>` per `s1`, `s2`, `s4`, `s5`, `s6`, `ramp`, `alternate`, `pauseResume`.
- **i18n:**
  - stesse chiavi in `app/src/lib/i18n/en.json` e `it.json`;
  - ogni chiave letta da Rust compare in `RUST_KEYS` (`app/src-tauri/src/i18n.rs`);
  - i testi italiani esatti sono nelle tabelle T1–T3; l'inglese lo traduce l'implementer e la revisione ne controlla il senso.
- **Budget (§11):**
  - a riposo non cambia niente;
  - durante un test la finestra resta sotto i 200 MB;
  - `oma-load` usa pochi MB di RAM; la VRAM solo nei limiti di DG6.

## Decisioni del piano

Fissano i punti del §15 per la M8b e le scelte che la spec lascia aperte. Il revisore le tratta come requisiti.

| # | Decisione | Perché |
|---|---|---|
| DG1 | **La M8b si divide.**<br>• **M8b1** (questo piano): fondamenta GPU in `oma-load` e stress test con i profili del §5.4.<br>• **M8b2** (piano a parte): benchmark Calcolo e Grafica (§5.2), pagina di punteggio della GPU, contagiri e S3.<br>**S3** (flusso di memoria) va nella M8b2, perché nessun profilo del §5.4 lo usa e il benchmark gli serve per la banda. Il protocollo v4 non ha quindi `s3`: arriverà con la versione del benchmark.<br>La barra laterale non cambia: la voce «GPU» del gruppo Punteggio arriva con la M8b2. | Lo stesso taglio della M8a, approvato dall'utente il 2026-10-07. |
| DG2 | **Protocollo v4.**<br>• `Plan.gpu: Option<GpuTarget { luid: u64, integrated: bool }>`. Un piano con `gpu` contiene solo kernel della GPU, e uno senza solo kernel della CPU.<br>• `KernelId`: `s1`, `s2`, `s4`, `s5`, `s6`, con `KernelId::is_gpu()`.<br>• `LoadMode`: `ramp` (S7), `alternate` (S8), `pause_resume` (S9), solo per i kernel della GPU. `steady` vale per tutti; `variable` e `light` solo per la CPU.<br>• `ErrorKind::DeviceLost`: `actual` porta l'HRESULT di `GetDeviceRemovedReason` come `u32`.<br>• `Progress.load_percent: Option<u8>` e `ComputeError.load_percent: Option<u8>`: il livello di carico di S7 e S8.<br>• Codici di `Notice` della GPU: `gpu_missing`, `vram_allocated` (byte), `vram_reduced` (byte), `vram_bits` (maschera dei bit sbagliati), `artifact_tiles` (riquadri diversi).<br>• Codice d'uscita `EXIT_DEVICE_LOST = 4`. | Il §5.5 chiede il LUID nel piano e il throughput nel `Progress`. Il throughput è già `Progress.rate` (DG7). |
| DG3 | **Shader compilati al build** (spike, Q4).<br>• Sorgenti in `crates/oma-load/shaders/*.hlsl`.<br>• `build.rs` li compila solo se `CARGO_CFG_TARGET_OS` è `windows`, con `fxc.exe /nologo /O3 /Gis /T <profilo> /E <entrata> /Fo $OUT_DIR/<nome>.cso`.<br>• `fxc.exe` si cerca così: prima la variabile `OMA_FXC`; poi la versione più alta di `%ProgramFiles(x86)%\Windows Kits\10\bin\10.*\x64\fxc.exe`. Se non lo trova, il build fallisce con «fxc.exe not found: install the Windows 10/11 SDK or set OMA_FXC».<br>• `cargo:rerun-if-changed` su ogni `.hlsl` e `cargo:rerun-if-env-changed=OMA_FXC`.<br>• Il codice li include con `include_bytes!(concat!(env!("OUT_DIR"), "/<nome>.cso"))`. | Bytecode uguale per tutti, cosa che conta per la classifica della M8b2. Il Windows SDK c'è già dove si compila Rust con MSVC, anche sui runner `windows-latest`. `/Gis` (IEEE strictness) impedisce di riassociare le catene di `mad`. |
| DG4 | **Invii e tempi.**<br>• Ogni invio si tara a **40 ms** sulle GPU dedicate e a **20 ms** sulle integrate (`GpuTarget::integrated`), con le timestamp query, all'inizio di ogni fase.<br>• Restano in volo al massimo 2 invii; l'attesa del precedente usa una event query e `sleep(1)`.<br>• Un invio che non finisce entro **1 s** dà `ErrorKind::Hung`.<br>• `DXGI_ERROR_DEVICE_REMOVED`, `DEVICE_HUNG`, `DEVICE_RESET` e `DRIVER_INTERNAL_ERROR`, da qualsiasi chiamata, danno `ErrorKind::DeviceLost`.<br>• Le verifiche si leggono una volta al secondo, da un buffer di contatori di 32 byte. | Spike, Q1 e Q7. |
| DG5 | **Verifica dei carichi di calcolo (S1, S2).**<br>• Ogni invio ripete lo stesso lavoro con lo stesso seme di fase, così le uscite sono sempre identiche.<br>• Il primo invio della fase scrive l'uscita di riferimento («golden»). La CPU ricalcola 1 thread ogni 97 (stesse formule dello shader, in `gpu::reference`) con un numero di passi ridotto a 1000 in un invio di prova: se un solo thread differisce, l'errore è `ReferenceInvalid` e la fase si salta.<br>• Ogni invio successivo è seguito da uno shader di confronto che conta le parole diverse dal golden (`InterlockedAdd`) e registra il primo indice (`InterlockedMin`).<br>• Un errore riporta: `iteration` = numero dell'invio, `expected` e `actual` = la parola al primo indice diverso.<br>• **S1** usa le catene FMA esatte dello spike (`x ← mad(x, −1, c)`, interi sotto 2²⁴). **S2** usa l'hash su interi dello spike.<br>• Iniezione d'errori (DA18 della M8a1): `--inject-fault s1` (o `s2`, `s4`, `s6`) capovolge il bit 0 della prima parola del golden dopo l'invio 3, solo con `debug_assertions`. | Spike, Q2: esatti bit per bit fra NVIDIA, AMD e CPU. Il confronto sulla GPU evita di rileggere 16 MB a ogni invio. |
| DG6 | **VRAM (S4).**<br>• Obiettivo, da `QueryVideoMemoryInfo` (nodo 0, segmento locale):<br>&nbsp;&nbsp;– GPU dedicata: 95% del budget − 400 MiB;<br>&nbsp;&nbsp;– GPU integrata: il minimo fra 90% del budget − 400 MiB, 25% della RAM disponibile e 4 GiB.<br>• Pezzi da `clamp(dedicata / 4, 256 MiB, 512 MiB)` (512 MiB = 2²⁷ parole, il limite di una vista D3D11), arrotondati a 4 KiB; un pezzo rifiutato si dimezza fino a 64 MiB prima di contarlo come mancanza.<br>• Se un'allocazione fallisce, si tiene quello che c'è: con meno dell'obiettivo arriva `Notice { code: "vram_reduced", value: byte }`; sotto i 256 MiB la fase si salta con `skipped: Some("vram")`.<br>• `Notice { code: "vram_allocated", value: byte }` all'inizio della fase.<br>• Il calcolo è una funzione pura, `gpu::sizing::vram_target(budget, integrated, available_ram) -> u64`. | Spike, «VRAM della iGPU»: il budget della integrata è la RAM condivisa. |
| DG7 | **Velocità e stabilità.**<br>• Per le fasi della GPU, `Progress.rate` = invii completati al secondo. Ogni invio ha un lavoro fisso, tarato all'inizio della fase, quindi un calo del rate è un calo del throughput.<br>• Il `RunController` raccoglie i rate delle fasi `steady` dei kernel `s1`, `s2` e `s5` in finestre da 10 s. Scarta i primi 30 s di ogni fase.<br>• Nell'obiettivo `overclock` scarta anche le finestre con il throttling acceso, cioè con `SensorSample::throttling == Some(true)` in un campione dentro la finestra.<br>• Stabilità = min / max delle finestre di una fase; quella della sessione è la minima fra le fasi, in `Session.stability: Option<f64>` (0–1).<br>• Senza almeno 2 finestre, la stabilità è `None` e non conta. | Il §5.4 chiede «finestra peggiore / migliore del throughput». Con invii da 40 ms ne arrivano 25 al secondo: in finestre da 10 s la granularità è dello 0,4%. |
| DG8 | **Esiti.**<br>• Esiti nuovi:<br>&nbsp;&nbsp;– `device_lost`: un `Error` di tipo `device_lost`, oppure l'uscita con il codice 4;<br>&nbsp;&nbsp;– `low_stability`: piano completato con stabilità < 0,97.<br>• Precedenza: `failed_to_start` > `system_crash` > `crashed` > `device_lost` > `hung` > `errors` > `stopped_thermal` > `suspended` > `stopped_user` > `low_stability` > `marginal` > `passed`.<br>• `device_lost` e `low_stability` contano come instabilità.<br>• Un `Notice { code: "gpu_missing" }` prima della prima fase dà `failed_to_start` con il motivo `performance.start.no_gpu`. | §2.3 e §5.4. |
| DG9 | **Profili della GPU (§5.4).** Durate in secondi; ogni fase si arrotonda al secondo e l'ultima prende il resto.<br>• **GPU · Verifica normale:** Rapido 300, Standard 900, Lungo 1800. Fasi:<br>&nbsp;&nbsp;– `s5` con `alt_kernel: s1`, `steady`, 70%;<br>&nbsp;&nbsp;– `s1`, `ramp`, 30%.<br>&nbsp;&nbsp;Senza `stop_on_error`.<br>• **GPU · Stabilità overclock:** Standard 1800, Lungo 3600, Notte 7200. Un giro:<br>&nbsp;&nbsp;– `s4` 20%, `s2` 10%, `s1` 10% (tutti `steady`);<br>&nbsp;&nbsp;– `s6` 15% (`steady`);<br>&nbsp;&nbsp;– `s1` `ramp` 20%, `s1` `alternate` 10%, `s1` `pause_resume` 15%.<br>&nbsp;&nbsp;Con `stop_on_error`.<br>• I campi delle fasi della GPU: `isa: sse2`, `size: auto`, `placement: all_logical`, `per_core_s: None`, `both_smt: false`, `cores: None`, `patterns` vuoto, `iterations: None`, `pause_before_ms: 0`. Il motore della GPU li ignora.<br>• **Personalizza** per la GPU: modalità incluse e minuti, «Fermati al primo errore». Set d'istruzioni e thread non compaiono. «Riprova solo il core N» non esiste per la GPU. | La scaletta del §5.4 in numeri. S5 + S1 alternano gli invii su una sola coda: è il «S5 + S1» della spec. |
| DG10 | **Modi di carico della GPU** (funzioni pure in `gpu::pace`).<br>• **`ramp` (S7):** livelli dal 20 al 100% a passi del 5% (17 livelli), ciascuno per 1/17 della fase.<br>• **`alternate` (S8):** periodi al 100% e al 15% che si alternano; la durata di ogni periodo è casuale, da 10 a 500 ms, dal seme della fase.<br>• **`pause_resume` (S9):** 60 s di carico pieno, poi 12 s fermo, poi di nuovo, fino alla fine della fase.<br>• Un livello L < 100% si ottiene con una pausa dopo ogni invio: `pausa = durata dell'invio × (100 − L) / L`.<br>• `Progress.load_percent` porta il livello per `ramp` e `alternate`, `None` per gli altri. | §5.3 e ricerca §8.1. |
| DG11 | **Scene grafiche.**<br>• **S5 (carico grafico):** quad ruotati e istanziati a 1920×1080 fuori schermo, con fusione alfa e 32 letture di texture per pixel (la «pelliccia»). Il numero di istanze si tara sull'invio. Non ha verifica: contano device perso, invio bloccato e stabilità.<br>• **S6 (scansione artefatti):** la scena dello spike (8 letture di texture per pixel), con istanze tarate una volta all'inizio della fase e poi fisse. Uno shader di calcolo riduce il render target a un hash FNV-1a a 32 bit per riquadro di 16×16 pixel (120×68 riquadri). Il primo fotogramma è il riferimento. Ogni fotogramma successivo si confronta riquadro per riquadro sulla GPU; un fotogramma diverso dà un errore (`expected` e `actual` = hash del primo riquadro diverso) e `Notice { code: "artifact_tiles", value: n }`.<br>• Il rasterizzatore ha `CullMode = NONE`. | Spike, Q3: hash identico con gli stessi parametri. Lo spike ha mostrato che la scena va tarata (6,7 s a fotogramma sulla iGPU senza taratura). |
| DG12 | **Sensori della GPU (§2.6).** `resolve_gpu_sensors(schema, device_id)`:<br>• **temperatura:** la prima presente fra `<device_id>/temperature/core` e `<device_id>/temperature/hotspot`;<br>• **potenza:** `<device_id>/power/board`;<br>• **clock:** `<device_id>/clock/core`;<br>• **throttling:** `<device_id>/flag/throttle-power` e `<device_id>/flag/throttle-thermal`; vale `Some(true)` se uno dei due vale 1.<br>Il servizio non serve: lo stress della GPU non mostra mai l'avviso `noService`. Soglia: `gpuStopC` (predefinito 90, da 60 a 110). | Gli id escono da `oma-win::gpu::field`. Le temperature della GPU arrivano senza servizio (§5.1). |
| DG13 | **Scelta della GPU.**<br>• `oma_win::gpu::stress_adapters() -> Vec<StressAdapter>` elenca le GPU hardware con DXGI. Il `device_id` si calcola come nel provider, così combacia con lo schema dei sensori.<br>• La UI sceglie per `device_id` (`StartRequest.gpu: Option<String>`), che resta uguale fra un riavvio e l'altro; l'app ricava il LUID all'avvio del test.<br>• Un `device_id` sconosciuto dà l'errore `build:no_gpu`.<br>• `Session.device` è il nome della GPU. | Il LUID cambia a ogni avvio di Windows: «Ripeti il test» deve funzionare anche dopo un riavvio. |
| DG14 | **Replay PCIe (spike, Q5).**<br>• `oma_win::gpu::pcie_replay_count(device_id) -> Option<u32>`: solo NVIDIA, con `nvmlDeviceGetHandleByPciBusId_v2` e `nvmlDeviceGetPcieReplayCounter`.<br>• Il runner lo legge all'avvio e ogni 5 s, e lo passa a `RunController::on_pcie_replay`.<br>• Il primo valore è la base. Un valore più alto aggiunge una volta l'avviso `pcieReplay` e un evento nel diario. È solo un avviso, mai un errore. | Il contatore risponde sulla 4080 (spike). |
| DG15 | **Nessun avviso «altro processo sulla GPU» nello stress.** Arriva con il benchmark (M8b2, §5.2). | Il §5.2 lo chiede solo per il benchmark. |

## Review Focus

1. **GPU che sparisce o driver che si azzera a metà test** (aggiornamento del driver, overclock instabile). Atteso: verdetto «Instabile: la GPU si è azzerata» con il codice, `oma-load` esce con 4, nessuna attesa infinita.

   Test: G5, `removed_hresults_map_to_device_lost`; G3, `device_lost_error_then_exit_4_is_device_lost`; G6, `device_lost_finishes_and_exits_with_4` (con un `Submitter` finto).
2. **GPU integrata, che ha come budget la RAM condivisa.** Atteso: la VRAM usata da S4 resta sotto il 25% della RAM libera e sotto i 4 GiB; gli invii sono da 20 ms.

   Test: G5, `integrated_vram_target_is_capped`, `submit_target_is_20_ms_on_integrated`.
3. **«Ripeti il test» dopo un riavvio o con la GPU tolta.** Atteso: il LUID nuovo si ricava dal `device_id`; senza la GPU, un errore chiaro «nessuna GPU adatta», mai un crash.

   Test: G10, `gpu_request_resolves_luid_from_device_id`, `unknown_gpu_device_is_no_gpu`; G6, `unknown_luid_sends_gpu_missing`.
4. **GPU senza sensore di temperatura** (iGPU AMD senza ADL, driver senza NVML). Atteso: l'avviso «Temperatura non disponibile», il test continua, nessun avviso sul servizio.

   Test: G3, `gpu_without_temperature_warns_and_continues`; G10, `gpu_test_never_warns_no_service`.
5. **Throttling e riscaldamento che falsano la stabilità.** Atteso: i primi 30 s di ogni fase non contano; in overclock non contano le finestre con il throttling acceso; senza finestre utili, nessun verdetto di stabilità.

   Test: G3, `warmup_is_excluded_from_stability`, `throttled_windows_are_excluded_in_overclock`, `too_few_windows_give_no_stability`.

## Tabelle dei testi (italiano esatto)

### T1. Glossario: modalità della GPU (`glossary.mode.<id>`, con `glossary.mode.<id>.name`)

| Chiave | Nome | Spiegazione |
|---|---|---|
| `mode.s1` | Calcolo FP32 (FMA) | Catene di moltiplicazioni e somme in virgola mobile su tutte le unità di calcolo della GPU. I numeri sono interi piccoli, così il risultato è esatto e si confronta bit per bit con quello calcolato dalla CPU. |
| `mode.s2` | Hash su interi | Calcoli su interi (moltiplicazioni, rotazioni, XOR) ripetuti su milioni di thread. Il risultato è esatto e si confronta con quello calcolato dalla CPU. |
| `mode.s4` | Verifica della VRAM | Riempie quasi tutta la memoria video con schemi di bit derivati dall'indirizzo e li rilegge in ordine sparso, come memtest_vulkan. Trova celle di memoria sbagliate, per esempio con un overclock della VRAM. |
| `mode.s5` | Carico grafico | Disegna fuori schermo molti strati trasparenti con texture, come la «pelliccia» di FurMark: fa lavorare la parte grafica della GPU e la fa consumare e scaldare molto. |
| `mode.s6` | Scansione artefatti | Disegna sempre la stessa scena e confronta ogni fotogramma con il primo, riquadro per riquadro. Un riquadro diverso è un artefatto: un errore che a schermo vedresti come puntini o righe. |
| `mode.ramp` | Rampa adattiva | Il carico sale dal 20 al 100% a passi del 5%. Se compare un errore, il test dice a che livello di carico e a che frequenza: molte instabilità compaiono a carico parziale. |
| `mode.alternate` | Carico alternato | Il carico salta di continuo fra il 100% e circa il 15%, con periodi da 10 a 500 ms. Prova l'alimentazione della GPU e i cambi rapidi di frequenza. |
| `mode.pauseResume` | Pausa e ripartenza | Ogni minuto il carico si ferma per 12 secondi e poi riparte al massimo. Prova il passaggio dalla frequenza bassa a quella alta, dove un overclock spesso cede. |

### T2. Glossario: termini della GPU (`glossary.<termine>`)

| Chiave | Termine | Spiegazione |
|---|---|---|
| `tdr` | TDR | Il controllo di Windows che azzera la GPU se resta bloccata per più di 2 secondi. Il test manda lavoro a piccoli pezzi, così non scatta mai da solo: se scatta, la GPU si è bloccata davvero. |
| `vram` | VRAM | La memoria video della scheda. Nelle GPU integrate è una parte della RAM di sistema. |
| `deviceLost` | GPU azzerata | Windows o il driver hanno azzerato la GPU durante il test: con un overclock di solito vuol dire che la GPU non regge. |
| `stability` | Stabilità della velocità | Il rapporto fra il tratto più lento e quello più veloce del test. Sotto il 97% la GPU rallenta più del normale, per esempio per errori corretti in silenzio dalla memoria. |
| `pcieReplay` | Replay PCIe | Pacchetti che la GPU ha dovuto rispedire sul collegamento PCIe perché arrivati sbagliati. Pochi non sono un problema; se crescono durante il test, il collegamento è al limite. |
| `artifact` | Artefatto | Un errore nell'immagine disegnata dalla GPU: puntini, righe o riquadri sbagliati. |
| `loadLevel` | Livello di carico | Quanta parte del tempo la GPU lavora: al 50% lavora metà del tempo e riposa l'altra metà. |

### T3. Altri testi fissi

| Chiave | Testo |
|---|---|
| `performance.outcome.device_lost` | Instabile: la GPU si è azzerata |
| `performance.outcome.low_stability` | Instabile: velocità non costante ({stability} %) |
| `performance.start.no_gpu` | nessuna GPU adatta |
| `performance.wizard.gpu` | GPU |
| `performance.wizard.gpu.detail` | {vram} di VRAM |
| `performance.wizard.gpu.integrated` | Integrata: usa la RAM di sistema |
| `performance.wizard.gpu.none` | Nessuna GPU adatta |
| `performance.warn.gpuShared` | La GPU integrata usa la RAM di sistema: la verifica della VRAM ne userà al massimo 4 GB. |
| `performance.warn.pcieReplay` | La GPU ha rispedito dati sul collegamento PCIe durante il test. |
| `performance.warn.vramReduced` | Meno VRAM del previsto: la verifica copre {size}. |
| `performance.result.loadLevel` | Al {level} % di carico |
| `performance.result.deviceLostCode` | Codice del driver: {code} |
| `performance.result.stability` | Stabilità della velocità: {stability} % |
| `settings.performance.gpuStopC` | Temperatura massima della GPU |
| `settings.performance.gpuStopC.hint` | Il test si ferma se il core della GPU supera questa temperatura per due letture di fila. |

---

### Task G1: protocollo `oma-ipc::load`, versione 4

**Files:**
- Modify:
  - `crates/oma-ipc/src/load.rs`;
  - `crates/oma-ipc/tests/load_fixtures.rs` (nuova fixture `run_gpu`);
  - `protocol/fixtures/load/*.msgpack`, rigenerate;
  - `protocol/fixtures/README.md`.

**Interfaces:**
- Consumes: il protocollo v3 attuale (`LOAD_PROTOCOL_VERSION = 3`, `check_phase`, `KernelId::is_bench_only`).
- Produces (DG2):
  - `LOAD_PROTOCOL_VERSION: u32 = 4`;
  - `GpuTarget { luid: u64, integrated: bool }` e `Plan.gpu: Option<GpuTarget>` con `#[serde(default)]`;
  - `KernelId::{S1, S2, S4, S5, S6}` (`"s1"`…), `KernelId::is_gpu(&self) -> bool`;
  - `LoadMode::{Ramp, Alternate, PauseResume}` (`"ramp"`, `"alternate"`, `"pause_resume"`), `LoadMode::is_gpu_only(&self) -> bool`;
  - `ErrorKind::DeviceLost` (`"device_lost"`);
  - `Progress.load_percent: Option<u8>` e `ComputeError.load_percent: Option<u8>`, con `#[serde(default)]`.
  - **Regole nuove di `validate` per `Run`:**
    - con `gpu` presente, ogni fase ha un kernel `is_gpu()` e un `alt_kernel` assente o `is_gpu()`; senza `gpu`, nessun kernel `is_gpu()`;
    - `ramp`, `alternate` e `pause_resume` solo con kernel `is_gpu()`;
    - `variable` e `light` mai con kernel `is_gpu()`;
    - `iterations` mai con kernel `is_gpu()`;
    - `luid` diverso da 0.
  - `Progress.load_percent` e `ComputeError.load_percent`, se presenti, fra 1 e 100.

- [ ] **Step 1: creare il branch** con `git switch -c feat/m8b1-gpu-stress` da `main`.
- [ ] **Step 2: test che falliscono** (`load.rs`, modulo di test):
  - `gpu_kernels_and_modes_are_snake_case`: `KernelId::S4` è `"s4"`, `LoadMode::PauseResume` è `"pause_resume"`, `ErrorKind::DeviceLost` è `"device_lost"`;
  - `gpu_plan_needs_gpu_kernels`: un piano con `gpu` e una fase `k1` è respinto, come uno senza `gpu` con una fase `s1`;
  - `gpu_alt_kernel_must_be_gpu`: `s5` con `alt_kernel: k5` è respinto;
  - `gpu_modes_only_on_gpu_kernels`: `k2` con `ramp` è respinto, come `s1` con `variable`;
  - `gpu_phases_refuse_iterations`;
  - `zero_luid_is_rejected`;
  - `load_percent_out_of_range_is_rejected`: 0 e 101 in `Progress` e in `ComputeError`;
  - `v3_run_without_gpu_still_decodes`: un `Plan` serializzato senza la chiave `gpu` si legge con `gpu: None`;
  - `hello_compatibility`: versione 4 sì, 3 no.

  In `tests/load_fixtures.rs`, la fixture nuova `run_gpu`: piano con `gpu: Some(GpuTarget { luid: 0x17e99, integrated: false })` e due fasi (`s5` con `alt_kernel: s1`, `steady`; `s1`, `ramp`).
- [ ] **Step 3:** `cargo test -p oma-ipc load`. Atteso: FAIL.
- [ ] **Step 4:** implementare, poi rigenerare le fixture con `$env:OMA_WRITE_FIXTURES='1'; cargo test -p oma-ipc --test load_fixtures -- --test-threads=1`.
- [ ] **Step 5:** `cargo test -p oma-ipc` senza la variabile, `cargo clippy -p oma-ipc --all-targets -- -D warnings`, poi `cargo build --workspace` per i match diventati non esaustivi. Per i `match` su `KernelId` e `LoadMode` fuori da `oma-ipc` basta, per ora, il ramo che li rifiuta (per esempio `factory` che restituisce `None`); le implementazioni arrivano nei task successivi. Atteso: PASS.
- [ ] **Step 6: commit** `feat(ipc): load protocol v4 with GPU target, kernels and modes`.

### Task G2: `oma-core::load`, catalogo, profili e piani della GPU

**Files:**
- Modify:
  - `crates/oma-core/src/load/plan.rs`;
  - `crates/oma-core/src/load/catalog.rs`;
  - `crates/oma-core/src/load/mod.rs`;
  - `testdata/performance/catalog.json`, rigenerato.

**Interfaces:**
- Consumes: G1 (`GpuTarget`, i kernel e i modi della GPU).
- Produces:
  - `Component::Gpu` (`"gpu"`);
  - `StartRequest.gpu: Option<String>` (`device_id`, DG13), con `#[serde(default)]`;
  - `BuildInput.gpu: Option<GpuTarget>`;
  - `BuildError::NoGpu` (wire `no_gpu`);
  - `presets(Component::Gpu, Objective::Normal)` = Quick 300, Standard 900, Long 1800; `presets(Component::Gpu, Objective::Overclock)` = Standard 1800, Long 3600, Night 7200 (DG9);
  - `build_plan` per la GPU secondo DG9: `Plan.gpu = input.gpu`, `ram_bytes: 0`;
  - **Personalizza (DG9):** per la GPU, `Custom.isa` e `Custom.threads` si ignorano; `RetryCore` con la GPU dà `BuildError::NoCores`;
  - **catalogo:** `components` con `"gpu"`; un elenco `gpuKernels` (`s1`, `s2`, `s4`, `s5`, `s6`) accanto a `kernels`; `modes` con `ramp`, `alternate`, `pauseResume`; i preset `"gpu.normal"` e `"gpu.overclock"`.

- [ ] **Step 1: test che falliscono** (`plan.rs`, `catalog.rs`):
  - `gpu_presets_match_the_spec`: i valori di DG9;
  - `gpu_normal_plan_is_s5_s1_then_ramp`: Standard (900) dà due fasi, `s5`+`s1` `steady` di 630 s e `s1` `ramp` di 270 s, senza `stop_on_error`, con `plan.gpu` uguale all'input;
  - `gpu_overclock_round_has_the_seven_phases`: Standard (1800) dà `s4` 360, `s2` 180, `s1` 180, `s6` 270, `s1` `ramp` 360, `s1` `alternate` 180, `s1` `pause_resume` 270, tutte con `stop_on_error`;
  - `gpu_phase_totals_equal_the_preset`, per ogni preset;
  - `gpu_without_target_is_no_gpu`;
  - `gpu_custom_ignores_isa_and_threads`: un `Custom` con `isa: Some(Avx512)` e `threads: OnePerCore` lascia `isa: sse2` e `placement: all_logical`;
  - `gpu_custom_minutes_rescale_the_phase`: `s4` a 10 minuti dà una fase di 600 s, e il totale è la somma;
  - `gpu_retry_core_is_refused`;
  - `cpu_plans_have_no_gpu`: i piani di CPU e RAM hanno `gpu: None`;
  - `catalog_lists_gpu_entries`: `components` contiene `"gpu"`, `gpuKernels` ha 5 voci, `presets` ha `"gpu.normal"`.
- [ ] **Step 2:** `cargo test -p oma-core load`. Atteso: FAIL.
- [ ] **Step 3:** implementare, poi rigenerare il catalogo con `$env:OMA_WRITE_FIXTURES='1'; cargo test -p oma-core catalog_fixture_matches`.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): GPU stress profiles and plans`.

### Task G3: `oma-core::load`, esiti, sensori, stabilità, replay PCIe e sessione

**Files:**
- Modify:
  - `crates/oma-core/src/load/outcome.rs`;
  - `crates/oma-core/src/load/sensors.rs`;
  - `crates/oma-core/src/load/thermal.rs`;
  - `crates/oma-core/src/load/run.rs`;
  - `crates/oma-core/src/load/session.rs`;
  - `crates/oma-core/src/load/mod.rs`.
- Create: `crates/oma-core/src/load/stability.rs`.

**Interfaces:**
- Consumes: G1, G2.
- Produces:
  - `Outcome::{DeviceLost, LowStability}` (`"device_lost"`, `"low_stability"`), con la precedenza di DG8;
  - `OutcomeFacts.device_lost: Option<u32>` (l'HRESULT) e `OutcomeFacts.stability: Option<f64>`;
  - `decide` dà la chiave `performance.outcome.device_lost` e `performance.outcome.low_stability` con il parametro `stability` (percentuale con una cifra decimale, come stringa);
  - `GpuSensorIds { temp, power, clock, throttle: Vec<usize> }` e `resolve_gpu_sensors(schema: &Schema, device_id: &str) -> GpuSensorIds` (DG12);
  - `read_gpu_sample(ids: &GpuSensorIds, snapshot: &Snapshot, quality: &[Quality]) -> SensorSample`;
  - `SensorSample.throttling: Option<bool>`, `None` per la CPU;
  - `gpu_stop_threshold(setting: u32) -> f64`;
  - `stability::StabilityMeter`: `new(objective: Objective)`, `phase_started(phase: u32, counts: bool, mono_ms)`, `rate(rate: f64, mono_ms)`, `throttling(on: bool, mono_ms)`, `result() -> Option<f64>`. Costanti: `WINDOW_MS = 10_000`, `WARMUP_MS = 30_000`, `MIN_STABILITY = 0.97`;
  - `RunController`:
    - con `Component::Gpu` crea uno `StabilityMeter` e gli passa i `Progress.rate` delle fasi che contano (DG7) e il throttling dei campioni;
    - `Error` di tipo `device_lost` imposta `device_lost`, e l'uscita con codice 4 lo imposta se manca;
    - `Notice { code: "gpu_missing" }` prima della prima fase dà `failed_to_start` con `performance.start.no_gpu`;
    - `Notice` con `vram_reduced` aggiunge l'avviso `vramReduced`;
    - `on_pcie_replay(count: u32, clock: Clock) -> Vec<Action>` (DG14): avviso `pcieReplay` ed evento una volta sola;
    - `ComputeError.load_percent` finisce in `ErrorRecord.load_percent: Option<u8>` (`#[serde(default)]`);
  - `Session.stability: Option<f64>` e `Session.gpu_device_id: Option<String>`, con `#[serde(default)]`; `RunStatus.load_percent: Option<u8>` e `RunStatus.stability: Option<f64>`.

- [ ] **Step 1: test che falliscono:**
  - `outcome.rs`: `device_lost_beats_hung_and_errors`, `crashed_beats_device_lost`, `low_stability_only_when_completed`, `stopped_user_beats_low_stability`, `low_stability_key_carries_the_percent` (0,953 dà `"95.3"`);
  - `sensors.rs`: `gpu_sensor_ids_follow_dg12` (schema con `temperature/core` e `temperature/hotspot`: vince `core`), `gpu_temperature_falls_back_to_hotspot`, `throttling_is_true_when_any_flag_is_one`, `cpu_samples_have_no_throttling`;
  - `stability.rs`: `steady_rates_give_stability_one`, `a_slow_window_lowers_stability` (finestre 100, 100, 95 danno 0,95), `warmup_is_excluded_from_stability`, `throttled_windows_are_excluded_in_overclock`, `throttled_windows_count_in_normal`, `too_few_windows_give_no_stability`, `phases_that_do_not_count_are_ignored`, `session_stability_is_the_worst_phase`;
  - `run.rs`: `device_lost_error_then_exit_4_is_device_lost`, `exit_4_alone_is_device_lost`, `gpu_missing_notice_fails_to_start`, `gpu_without_temperature_warns_and_continues`, `completed_gpu_run_with_low_stability_is_low_stability`, `pcie_replay_increase_warns_once`, `error_keeps_the_load_percent`;
  - `session.rs`: `sessions_without_stability_still_parse`.
- [ ] **Step 2:** `cargo test -p oma-core load`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): GPU outcomes, sensors, throughput stability and PCIe replay`.

### Task G4: `oma-win`, GPU per lo stress e replay PCIe

**Files:**
- Modify:
  - `crates/oma-win/src/gpu/mod.rs` (funzioni pubbliche);
  - `crates/oma-win/src/gpu/nvml.rs` (simboli facoltativi `nvmlDeviceGetHandleByPciBusId_v2` e `nvmlDeviceGetPcieReplayCounter`).

**Interfaces:**
- Consumes: `enumerate()`, `Adapter::device_id(ordinal)` e il conteggio degli ordinali del provider (`gpu/mod.rs`, intorno alla riga 347).
- Produces:
  - `pub struct StressAdapter { pub luid: u64, pub device_id: String, pub name: String, pub vendor_id: u32, pub integrated: bool, pub dedicated_bytes: u64 }`, `Serialize` in camelCase;
  - `pub fn stress_adapters() -> Vec<StressAdapter>`: solo GPU hardware, nell'ordine di `enumerate()`, con lo stesso calcolo del `device_id` del provider. Una funzione pura `assign_device_ids(&[Adapter]) -> Vec<String>` la usano sia il provider sia `stress_adapters`;
  - `pub fn pcie_replay_count(device_id: &str) -> Option<u32>`: `None` senza NVML, senza indirizzo PCI o per una GPU non NVIDIA. NVML si carica una volta e non si scarica mai (D1).

- [ ] **Step 1: test che falliscono:**
  - `device_ids_match_the_provider`: per una lista di `Adapter` finti, con e senza indirizzo PCI, `assign_device_ids` dà gli stessi id del provider;
  - `pci_bus_id_string_for_nvml`: un indirizzo PCI diventa la stringa `"00000000:01:00.0"` attesa da NVML;
  - `stress_adapters_lists_real_gpus` (`#[ignore = "requires real Windows hardware"]`): almeno una voce, LUID diversi da 0, nessun «Microsoft Basic Render Driver»;
  - `pcie_replay_counter_reads_on_nvidia` (`#[ignore = "requires real Windows hardware"]`): `Some` per la RTX 4080, se c'è.
- [ ] **Step 2:** `cargo test -p oma-win gpu`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-win`, poi `cargo test -p oma-win gpu -- --include-ignored` (nessun carico), clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(win): list GPUs for the stress test and read the NVML PCIe replay counter`.

### Task G5: `oma-load`, shader al build e fondamenta D3D11

**Files:**
- Create:
  - `crates/oma-load/shaders/s1_fma.hlsl`, `s2_hash.hlsl`, `compare.hlsl` (confronto con il golden, DG5), `probe.hlsl` (scrive una costante, per i test);
  - `crates/oma-load/src/gpu/mod.rs`;
  - `crates/oma-load/src/gpu/sizing.rs`, `crates/oma-load/src/gpu/pace.rs`, `crates/oma-load/src/gpu/reference.rs` (portabili);
  - `crates/oma-load/src/gpu/shaders.rs`, `crates/oma-load/src/gpu/device.rs`, `crates/oma-load/src/gpu/submit.rs` (`cfg(windows)`).
- Modify:
  - `crates/oma-load/build.rs` (DG3);
  - `crates/oma-load/Cargo.toml` (le quattro feature del crate `windows`);
  - `crates/oma-load/src/lib.rs`.

**Interfaces:**
- Consumes: G1. Gli shader S1 e S2 sono quelli dello spike (`docs/superpowers/references/m8/spike-gpu/fma.hlsl` e `hash.hlsl`, con i riferimenti della CPU (`cpu_fma`, `cpu_hash`) in `main.rs.txt`), con il seme e il numero di passi nel constant buffer.
- Produces:
  - **`sizing`:** `vram_target(budget: u64, integrated: bool, available_ram: u64) -> u64` e `chunk_bytes(dedicated: u64) -> u64` (DG6); `submit_target_ms(integrated: bool) -> f64` (DG4).
  - **`pace`** (DG10):
    - `ramp_level(elapsed_ms: u64, phase_ms: u64) -> u8`;
    - `Alternate::new(seed: u64)`, con `level_at(elapsed_ms: u64) -> u8` che vale 100 o 15;
    - `pause_resume_active(elapsed_ms: u64) -> bool`;
    - `idle_after_ms(level: u8, submit_ms: f64) -> f64`.
  - **`reference`:** `fma_thread(id: u32, seed: u32, steps: u32) -> [u32; 4]` e `hash_thread(id: u32, seed: u32, steps: u32) -> [u32; 4]`, uguali bit per bit agli shader.
  - **`shaders`:** `S1_FMA`, `S2_HASH`, `COMPARE`, `PROBE: &[u8]`.
  - **`device`:**
    - `GpuError::{NotFound, Create(i32), Lost(u32), Hung, OutOfMemory}`;
    - `fn map_hresult(hr: i32, removed_reason: impl FnOnce() -> u32) -> GpuError`, pura;
    - `GpuDevice::open(luid: u64) -> Result<GpuDevice, GpuError>`, con `D3D_DRIVER_TYPE_UNKNOWN` e il livello 11_0, escludendo gli adattatori software;
    - `GpuDevice::video_memory() -> Result<(u64, u64), GpuError>` (budget, uso) da `IDXGIAdapter3::QueryVideoMemoryInfo`;
    - `GpuDevice::dedicated_bytes() -> u64`.
  - **`submit`:**
    - `trait Submit { fn submit(&mut self, work: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<(), GpuError>; fn finish(&mut self) -> Result<(), GpuError>; fn gpu_ms(&mut self, work: &mut dyn FnMut(&ID3D11DeviceContext)) -> Result<f64, GpuError>; }`;
    - `Submitter` la implementa con al massimo 2 invii in volo, event query, `sleep(1)` e il limite di 1 s (DG4);
    - `fn calibrate(sub: &mut dyn Submit, target_ms: f64, run: &mut dyn FnMut(&ID3D11DeviceContext, u32)) -> Result<u32, GpuError>`: raddoppia il parametro da 64 finché un invio supera un quarto dell'obiettivo, poi scala in proporzione.

- [ ] **Step 1: test che falliscono:**
  - `sizing.rs`: `dedicated_vram_target_is_95_percent_minus_400_mib` (budget 15 280 MiB dà 14 116 MiB), `integrated_vram_target_is_capped` (budget 15 647 MiB e 30 GiB di RAM libera danno 4 GiB; con 8 GiB liberi danno 2 GiB), `tiny_budget_gives_zero`, `chunks_are_between_256_and_512_mib`, `submit_target_is_20_ms_on_integrated`;
  - `pace.rs`: `ramp_has_17_levels_from_20_to_100`, `alternate_periods_are_10_to_500_ms_and_seeded`, `pause_resume_is_60_on_12_off`, `idle_keeps_the_duty_cycle` (livello 25 con invii da 40 ms dà 120 ms);
  - `reference.rs`: `fma_reference_stays_integer` (con `steps` pari ritorna i valori iniziali), `hash_reference_known_vector` (un valore fissato calcolato una volta e scritto nel test);
  - `device.rs`: `removed_hresults_map_to_device_lost` (le quattro HRESULT di DG4 danno `Lost` con la ragione), `other_hresults_are_create_errors`;
  - `gpu_matches_the_cpu_reference` (`#[ignore = "requires real Windows hardware"]`, meno di 2 s): S1 e S2 con 1000 passi su ogni GPU dell'elenco danno, sui thread campione, gli stessi valori di `reference`;
  - `probe_round_trips` (`#[ignore = "requires real Windows hardware"]`): `PROBE` scrive la costante e `Submitter` la rilegge.
- [ ] **Step 2:** `cargo test -p oma-load gpu`. Atteso: FAIL.
- [ ] **Step 3:** implementare `build.rs` (DG3), i moduli e le feature.
- [ ] **Step 4:**
  - `cargo build -p oma-load`;
  - `cargo test -p oma-load`;
  - `cargo test -p oma-load gpu -- --include-ignored`, che rientra nel limite di carico dei Global Constraints;
  - clippy.

  Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): build-time HLSL with fxc and D3D11 submission foundations`.

### Task G6: `oma-load`, motore delle fasi della GPU con S1, S2, S7, S8 e S9

**Files:**
- Create:
  - `crates/oma-load/src/gpu/engine.rs`;
  - `crates/oma-load/src/gpu/compute.rs` (carichi S1 e S2 con il golden, DG5);
  - `crates/oma-load/src/gpu/tests.rs`.
- Modify:
  - `crates/oma-load/src/link.rs` (instradamento dei piani con `gpu`, `EXIT_DEVICE_LOST = 4`);
  - `crates/oma-load/src/args.rs` (`--inject-fault s1|s2|s4|s6`);
  - `crates/oma-load/src/gpu/mod.rs`.

**Interfaces:**
- Consumes: G1, G5.
- Produces:
  - `trait GpuWorkload { fn prepare(&mut self, sub: &mut dyn Submit, target_ms: f64) -> Result<(), GpuError>; fn submit(&mut self, sub: &mut dyn Submit) -> Result<(), GpuError>; fn check(&mut self, sub: &mut dyn Submit) -> Result<GpuCheck, GpuError>; }`;
  - `GpuCheck { checks: u64, mismatches: Vec<GpuMismatch { iteration: u64, expected: u64, actual: u64 }>, notices: Vec<(String, u64)> }`;
  - `fn workload(kernel: KernelId, dev: &GpuDevice, ctx: &PhaseCtx) -> Result<Box<dyn GpuWorkload>, GpuError>`. G7 e G8 aggiungono `s4`, `s5` e `s6`; fino ad allora la fase si salta con `skipped: Some("unsupported")`;
  - `PhaseCtx { seed: u64, integrated: bool, inject: Option<Inject>, budget: VramBudget }`;
  - `pub fn run_gpu(plan: &Plan, out: &(dyn Fn(LoadMessage) + Sync), stop: &AtomicBool, inject: Option<Inject>) -> GpuRunEnd`, con `GpuRunEnd { finished: Finished, exit_code: i32 }`. `run_gpu_with` accetta un `open` finto per i test.
  - **Comportamento:**
    - per fase: `workload`, poi `prepare` (taratura sull'obiettivo di DG4), poi il ciclo d'invio secondo il modo (DG10), con `alt_kernel` che alterna gli invii;
    - una volta al secondo: `check`, poi `Progress` con `rate` (DG7) e `load_percent`;
    - `stop_on_error` ferma al primo errore con `FinishReason::FirstError`;
    - `GpuError::Lost` → `Error { kind: device_lost }`, `Finished { reason: failed }`, codice d'uscita 4;
    - `GpuError::Hung` → `Error { kind: hung }`, `Finished { reason: failed }`, codice 0;
    - `GpuError::NotFound` → `Notice { code: "gpu_missing" }`, `Finished { reason: failed }`, codice 0;
    - lo `stop` si controlla almeno ogni 100 ms, anche durante le pause dei modi.
  - **`link`:** un piano con `gpu` va a `run_gpu` su un thread; lo stesso `Stop` e la stessa regola su un secondo `Run`.

- [ ] **Step 1: test che falliscono.** I test puri usano un `Submit` e un `GpuWorkload` finti, al massimo 3 s ciascuno:
  - `phases_run_in_order_with_progress_every_second`;
  - `rate_counts_completed_submissions`;
  - `ramp_reports_the_load_level`;
  - `alternate_switches_between_100_and_15`;
  - `pause_resume_stops_submitting_during_the_pause`;
  - `alt_kernel_alternates_submissions`;
  - `mismatch_becomes_an_error_with_iteration`;
  - `stop_on_error_finishes_with_first_error`;
  - `stop_flag_finishes_within_200_ms`;
  - `device_lost_finishes_and_exits_with_4`;
  - `hung_submission_is_hung`;
  - `unknown_luid_sends_gpu_missing`;
  - `missing_workload_skips_the_phase`;
  - `injected_fault_hits_s1` (`args`: `--inject-fault s1` si legge solo in debug);
  - `gpu_plan_runs_end_to_end` (`#[ignore = "requires real Windows hardware"]`): un piano di 2 s con `s1` `steady` sulla prima GPU finisce `completed` con almeno un controllo e zero errori.
- [ ] **Step 2:** `cargo test -p oma-load gpu`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, poi `cargo test -p oma-load gpu -- --include-ignored` (rientra nel limite di carico), clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): GPU phase engine with verified FMA and integer hash loads`.

### Task G7: `oma-load`, S4, la verifica della VRAM (adattamento di memtest_vulkan)

**Files:**
- Create:
  - `crates/oma-load/shaders/s4_vram.hlsl` (scrittura, verifica e passi classici);
  - `crates/oma-load/src/gpu/vram.rs`.
- Modify: `crates/oma-load/build.rs`, `crates/oma-load/src/gpu/shaders.rs`, `crates/oma-load/src/gpu/engine.rs` (`workload` per `s4`).

**Interfaces:**
- Consumes: G5, G6.
- Produces:
  - `VramWorkload`, che implementa `GpuWorkload`:
    - in `prepare`: obiettivo (DG6), pezzi e `Notice` `vram_allocated` / `vram_reduced`, oppure la fase saltata con `skipped: Some("vram")`;
    - ogni giro: scrittura del pattern derivato dall'indirizzo e ruotato con la chiave del giro, poi la rilettura in ordine ruotato. Ogni 4 giri si fa un passo classico, a turno «walking ones», «moving inversions» (`0x00000000`/`0xFFFFFFFF` e `0x55555555`/`0xAAAAAAAA`) e «modulo 20»;
    - ogni invio copre una finestra di un pezzo, tarata su DG4;
    - le statistiche sulla GPU in un buffer: numero di parole sbagliate, primo indice, valore atteso e letto della prima, OR dei bit sbagliati;
    - un giro con errori dà un `GpuMismatch` (`iteration` = giro), più `vram_bits` e il numero di parole nel `Notice`;
  - `fn vram_word(index: u64, key: u32) -> u32`, pura e uguale bit per bit allo shader.
  - **Nota d'origine** in testa a `s4_vram.hlsl` e a `vram.rs`. L'implementer legge con WebFetch il repository `GpuZelenograd/memtest_vulkan` all'ultimo commit di `main`. Ne fissa il commit, copia la riga di copyright dal file `LICENSE` e porta in HLSL la formula del pattern e l'ordine di rilettura:

```text
// Adapted from memtest_vulkan (https://github.com/GpuZelenograd/memtest_vulkan),
// commit <hash>: address-derived rotated pattern, write once and re-read in
// rotated order, bit-error statistics.
// Original work: <riga di copyright di LICENSE>, licensed under the zlib
// License (see THIRD_PARTY_LICENSES.txt).
// Modified for OpenMonitor Advanced: ported to HLSL cs_5_0 and D3D11, sized
// from the DXGI video memory budget, classic passes added.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.
```

- [ ] **Step 1: test che falliscono:**
  - `vram_word_is_deterministic_and_key_dependent`;
  - `vram_word_differs_between_neighbours`, su 1 milione di indici consecutivi;
  - `classic_pass_order_rotates`;
  - `allocation_shortfall_reports_vram_reduced`, con un allocatore finto;
  - `under_256_mib_skips_the_phase`;
  - `vram_detects_a_flipped_word` (`#[ignore = "requires real Windows hardware"]`): su un pezzo da 64 MiB, un bit capovolto con un hook `#[cfg(test)]` fra scrittura e rilettura dà un `GpuMismatch` con l'indice giusto, in meno di 2 s.
- [ ] **Step 2:** `cargo test -p oma-load vram`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, poi `cargo test -p oma-load vram -- --include-ignored`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): S4 VRAM verification adapted from memtest_vulkan`.

### Task G8: `oma-load`, S5 e S6, carico grafico e scansione degli artefatti

**Files:**
- Create:
  - `crates/oma-load/shaders/scene.hlsl` (VS e PS di S5 e S6, con il numero di letture di texture nel constant buffer);
  - `crates/oma-load/shaders/tile_hash.hlsl` (hash per riquadro e confronto, DG11);
  - `crates/oma-load/src/gpu/graphics.rs`.
- Modify: `crates/oma-load/build.rs`, `crates/oma-load/src/gpu/shaders.rs`, `crates/oma-load/src/gpu/engine.rs` (`workload` per `s5` e `s6`).

**Interfaces:**
- Consumes: G5, G6. La scena dello spike (`docs/superpowers/references/m8/spike-gpu/scene.hlsl`), con `CullMode = NONE`.
- Produces:
  - `GraphicsWorkload { kind: Fur | Artifact }`, che implementa `GpuWorkload` secondo DG11;
  - `TILE: u32 = 16`, `WIDTH: u32 = 1920`, `HEIGHT: u32 = 1080`;
  - `fn tile_count(width: u32, height: u32) -> (u32, u32)`, che dà `(120, 68)`;
  - `fn fnv1a32(bytes: &[u8]) -> u32`, pura, uguale allo shader `tile_hash` su un riquadro dato.

- [ ] **Step 1: test che falliscono:**
  - `tile_grid_covers_1080p`;
  - `cpu_tile_hash_matches_known_vector`;
  - `artifact_scene_is_deterministic` (`#[ignore = "requires real Windows hardware"]`): 5 fotogrammi con le istanze fisse danno zero riquadri diversi;
  - `artifact_scan_detects_a_changed_frame` (`#[ignore = "requires real Windows hardware"]`): un parametro cambiato con un hook `#[cfg(test)]` dà un `GpuMismatch` e `artifact_tiles` > 0;
  - `fur_load_calibrates_near_the_target` (`#[ignore = "requires real Windows hardware"]`): l'invio tarato dura fra il 50 e il 200% dell'obiettivo.

  I tre test hardware, insieme, stanno sotto i 2 s di GPU ciascuno.
- [ ] **Step 2:** `cargo test -p oma-load graphics`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, poi `cargo test -p oma-load graphics -- --include-ignored`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): S5 graphics load and S6 artifact scan`.

### Task G9: impostazione `gpuStopC`

**Files:**
- Modify:
  - `crates/oma-core/src/settings/performance.rs`, `crates/oma-core/src/settings/decode.rs`, `crates/oma-core/src/settings/mod.rs`;
  - `app/src/lib/types.ts` (`PerformanceSettings`);
  - `app/src/lib/backend/mockSettings.ts`;
  - `app/src/components/settings/PerformanceSection.svelte` e il suo test;
  - `app/src/lib/i18n/en.json`, `it.json` (T3: `settings.performance.gpuStopC` e `.hint`).

**Interfaces:**
- Consumes: lo schema di `cpuStopC`.
- Produces:
  - `PerformanceSettings.gpu_stop_c: u32`, predefinito 90, in JSON `gpuStopC`;
  - `GPU_STOP_C: RangeInclusive<u32> = 60..=110`;
  - un valore fuori intervallo o non numerico torna a 90, con una diagnostica;
  - `encode()` scrive sei chiavi;
  - nella pagina Impostazioni › Prestazioni, un campo numerico con le unità °C, sotto quello della CPU.

- [ ] **Step 1: test che falliscono:**
  - Rust: `gpu_stop_defaults_to_90`, `gpu_stop_out_of_range_falls_back_to_90`, il test con l'elenco completo delle chiavi aggiornato;
  - Vitest: la sezione mostra il campo e una patch con `gpuStopC: 75` arriva al backend.
- [ ] **Step 2:** `cargo test -p oma-core settings`; `cd app && pnpm test PerformanceSection`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-core`, `cd app && pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(settings): GPU thermal stop threshold`.

### Task G10: app, runner, comandi, sensori e tray per la GPU

**Files:**
- Modify:
  - `app/src-tauri/src/performance/runner.rs` (con il banco di prova dei test);
  - `app/src-tauri/src/performance/commands.rs`;
  - `app/src-tauri/src/i18n.rs` (`RUST_KEYS`: `performance.start.no_gpu`, `performance.outcome.device_lost`, `performance.outcome.low_stability`);
  - `app/src/lib/i18n/en.json`, `it.json` (le stesse chiavi, T3).

**Interfaces:**
- Consumes: G2, G3, G4, G9.
- Produces:
  - **`Machine`:** in più `gpus() -> Vec<StressAdapter>` e `pcie_replay(device_id: &str) -> Option<u32>`; `WinMachine` usa G4, `FakeMachine` una lista data;
  - **`SystemInfo.gpus: Vec<GpuChoice { deviceId, name, integrated, dedicatedBytes }>`**, in camelCase;
  - **`start` con `Component::Gpu`:**
    - risolve `request.gpu` fra `machine.gpus()`; se manca o non c'è, restituisce l'errore `build:no_gpu`;
    - costruisce il piano con `BuildInput.gpu = Some(GpuTarget { luid, integrated })`;
    - `RunConfig.threshold_c = gpu_stop_threshold(perf.gpu_stop_c)`, `cores` vuoto;
    - `Session.device` = nome della GPU, `Session.gpu_device_id` = `device_id`;
  - **`on_tick`:** per la GPU, `resolve_gpu_sensors` e `read_gpu_sample` al posto di quelli della CPU. `service_available` vale sempre `true` per la GPU (DG12);
  - **replay PCIe:** letto all'avvio e ogni 5 s sul thread del runner, poi passato a `on_pcie_replay` (DG14);
  - **`preview`** per la GPU, con la stessa risoluzione del `device_id`;
  - **tray e toast:** il tooltip e il toast usano il nome del componente «GPU», con le chiavi esistenti dei componenti.

- [ ] **Step 1: test che falliscono** (banco di prova del runner):
  - `system_lists_gpus`;
  - `gpu_request_resolves_luid_from_device_id`;
  - `unknown_gpu_device_is_no_gpu`;
  - `gpu_start_uses_the_gpu_threshold`;
  - `gpu_samples_come_from_gpu_sensors`;
  - `gpu_test_never_warns_no_service`;
  - `pcie_replay_is_polled_every_five_seconds`;
  - `gpu_session_records_the_device`.
- [ ] **Step 2:** `cargo test -p oma-app performance`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-app`, clippy, `cargo test -p oma-app i18n`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): run GPU stress tests with GPU sensors and PCIe replay`.

### Task G11: UI, procedura guidata e Personalizza per la GPU

**Files:**
- Modify:
  - `app/src/lib/types.ts` (`StressComponent` con `'gpu'`, `StartRequest.gpu`, `SystemInfo.gpus`, `GpuChoice`);
  - `app/src/components/performance/StressWizard.svelte` e il suo test;
  - `app/src/components/performance/WizardCustomize.svelte` e il suo test;
  - `app/src/lib/backend/mockPerformance.ts` (due GPU finte: una dedicata da 16 GB e una integrata);
  - `app/src/lib/i18n/en.json`, `it.json` (T3: `performance.wizard.gpu*`, `performance.warn.gpuShared`).

**Interfaces:**
- Consumes: G2 (catalogo), G10 (`SystemInfo.gpus`, `build:no_gpu`).
- Produces:
  - nel passo «Componente», una voce per GPU dopo CPU e RAM, con il nome e `{vram} di VRAM` (o «Integrata: usa la RAM di sistema»). Senza GPU c'è una sola voce disattivata, «Nessuna GPU adatta»;
  - lo stato della procedura tiene il `deviceId` scelto, che va in `StartRequest.gpu`;
  - le durate vengono da `catalog.presets['gpu.<obiettivo>']`;
  - il riepilogo mostra `performance.warn.gpuShared` per una GPU integrata e non mostra né set d'istruzioni né quota di RAM;
  - Personalizza per la GPU nasconde set d'istruzioni e thread (DG9);
  - «Ripeti il test» dalla cronologia riparte con lo stesso `deviceId`.

- [ ] **Step 1: test che falliscono** (Vitest):
  - `lists one tile per GPU`;
  - `shows the no-GPU tile when none is available`;
  - `GPU choice sends its deviceId`;
  - `GPU presets come from gpu.<objective>`;
  - `integrated GPU summary warns about shared memory`;
  - `customize hides isa and threads for the GPU`.
- [ ] **Step 2:** `cd app && pnpm test StressWizard WizardCustomize`. Atteso: FAIL.
- [ ] **Step 3:** implementare, con lo stile dei riquadri esistenti.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): choose a GPU in the stress test wizard`.

### Task G12: UI, durante il test, risultato, cronologia e glossario della GPU

**Files:**
- Modify:
  - `app/src/components/performance/StressRun.svelte`, `StressResult.svelte`, `StressHistory.svelte` e i loro test;
  - `app/src/lib/performance/format.ts` (nomi delle HRESULT di DG4, percentuali);
  - `app/src/lib/performance/glossary.ts` e `glossary.test.ts`;
  - `app/src/lib/types.ts` (`RunStatus.loadPercent`, `RunStatus.stability`, `Session.stability`, `ErrorRecord.loadPercent`);
  - `app/src/lib/backend/mockPerformance.ts` (uno scenario GPU dal vivo, uno `device_lost` e uno `low_stability`);
  - `app/src/lib/i18n/en.json`, `it.json` (T1, T2, T3).

**Interfaces:**
- Consumes: G3 (`RunStatus`, `Session`), G11.
- Produces:
  - **Durante il test:**
    - per la GPU niente griglia dei core;
    - il riquadro del clock mostra quello della GPU;
    - durante `ramp` e `alternate`, il livello di carico accanto alla fase («Al {level} % di carico», con `Term` `loadLevel`);
  - **Risultato:**
    - verdetti `device_lost` e `low_stability` (T3);
    - con `device_lost`, il «Codice del driver» con il nome dell'HRESULT (per esempio `DXGI_ERROR_DEVICE_HUNG (0x887A0006)`);
    - per un errore con `loadPercent`, il livello di carico e il clock a quel momento;
    - la stabilità della velocità nel riepilogo (`Term` `stability`);
    - niente «Riprova solo il core N» per la GPU;
  - **Cronologia:** il filtro per componente ha «GPU»;
  - **Glossario:** `MODE_TERMS` legge anche `gpuKernels` e i modi nuovi del catalogo. Il conteggio di `glossary.test.ts` passa da 22 a 30. Ogni termine di T2 è marcato con `Term` dove compare.

- [ ] **Step 1: test che falliscono** (Vitest):
  - `gpu run hides the core grid`;
  - `ramp shows the load level`;
  - `device lost result shows the driver code`;
  - `low stability result shows the percent`;
  - `gpu result has no retry-core action`;
  - `history filters by GPU`;
  - il test del glossario con 30 voci e le chiavi T1/T2 in `en` e `it`.
- [ ] **Step 2:** `cd app && pnpm test performance`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): GPU stress run, result, history and glossary`.

### Task G13: licenze, documenti, misure e grafo

**Files:**
- Modify:
  - `THIRD_PARTY_NOTICES.md`: sezione «memtest_vulkan», nello stile di quelle di FIRESTARTER e OpenDCDiag, con il commit di G7, il copyright e la licenza zlib;
  - `scripts/lib/OmaLicenses.psm1` e `scripts/generate-licenses.ps1`: la voce memtest_vulkan nell'ecosistema `Adapted`, con il testo `scripts/licenses/Zlib.txt` (nuovo, copiato dal `LICENSE` del commit fissato), più il test di `Licenses.Tests.ps1`;
  - `THIRD_PARTY_LICENSES.txt`, rigenerato;
  - `CLAUDE.md`:
    - `oma-load` con `gpu/` e `shaders/`, e il Windows SDK (`fxc.exe` o `OMA_FXC`) fra i requisiti di build;
    - il protocollo v4;
    - lo stato della M8b1;
  - `README.md` e `README.it.md`: lo stress test della GPU;
  - `docs/perf-budget.md`: sezione «M8b1», con la misura a riposo;
  - `docs/follow-ups.md`: M8b2 (benchmark, S3, avviso degli altri processi), prove dal vivo G14.

**Interfaces:**
- Consumes: tutto il branch.
- Produces: documentazione e licenze allineate.

- [ ] **Step 1:** `pwsh scripts/measure-footprint.ps1` a riposo, senza test, e i valori in `docs/perf-budget.md`. Atteso: nucleo < 1% CPU, tray < 30 MB, finestra < 200 MB, come prima.
- [ ] **Step 2:** aggiornare documenti e licenze.
- [ ] **Step 3:**
  - `Import-Module Pester -RequiredVersion 5.7.1; Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI`;
  - `pwsh scripts/generate-licenses.ps1`, poi `pwsh scripts/generate-licenses.ps1 -Check`;
  - `cargo fmt --all --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo test --workspace`;
  - `cd app && pnpm test && pnpm check && pnpm build`;
  - `pwsh scripts/check-version.ps1`;
  - `PYTHONHASHSEED=0 graphify update .`.

  Atteso: PASS.
- [ ] **Step 4: commit** `docs: M8b1 GPU stress test documentation, licences and footprint`.

### Task G14: prove dal vivo con l'utente

Le fa l'utente, un blocco alla volta (memoria «user admin shell»: un comando per blocco). L'agente prepara i comandi, chiede prima di ogni carico e non usa mai input sintetico.

Prima delle prove:
- `cargo build -p oma-load` e `cargo build -p oma-overlay`;
- poi `cd app && pnpm tauri dev`.

| # | Prova | Atteso |
|---|---|---|
| Q1 | GPU (RTX 4080) · Verifica normale · Rapido (5 min) | Fine regolare, «Superato», stabilità ≥ 97% nel riepilogo, sessione nella cronologia, toast finale. |
| Q2 | iGPU AMD · Verifica normale · Rapido | Fine regolare; desktop fluido durante il test; avviso sulla RAM condivisa nel riepilogo. |
| Q3 | RTX 4080 · Stabilità overclock · Standard (30 min) | Le sette fasi, `vram_allocated` vicino al 95% del budget meno 400 MB, «Superato». |
| Q4 | `$env:OMA_LOAD_INJECT='s1'`, poi RTX 4080 · Stabilità overclock, fermato dopo l'errore | «Errori trovati», con la fase e il livello di carico se l'errore cade in S7. |
| Q5 | «Ferma e salva» e «Ferma il test» dal tray | «Fermato da te», sessione salvata. |
| Q6 | Stop termico con `gpuStopC` = 60 | «Fermato: temperatura a N °C» entro due letture sopra la soglia. |
| Q7 | Chiusura della finestra durante un test | Il test continua nella tray, poi il toast finale. |
| Q8 | Tooltip | Ogni termine della GPU (T1, T2) mostra la spiegazione con il mouse e con Tab. |
| Q9 | Budget durante Q1 | `scripts/measure-footprint.ps1`: finestra < 200 MB; `oma-load` annotato con la sua VRAM. |
| Q10 | «Ripeti il test» dopo un riavvio di Windows, solo se l'utente vuole | Il test riparte sulla stessa GPU. |

- [ ] **Step 1:** preparare l'elenco con i comandi esatti e chiedere il via per ogni prova.
- [ ] **Step 2:** registrare gli esiti in `docs/follow-ups.md` e nella memoria del progetto (`m8b1-followups.md`).
- [ ] **Step 3: commit** `docs: record the M8b1 live checks`.
- [ ] **Step 4:** `superpowers:finishing-a-development-branch`: merge in `main` in locale, nessun push senza richiesta.
