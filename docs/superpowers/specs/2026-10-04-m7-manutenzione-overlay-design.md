# M7 — Manutenzione e overlay in-game: design di dettaglio

- **Data:** 2026-10-04
- **Stato:** approvato a sezioni nel brainstorming del 2026-10-04, in attesa della revisione del testo scritto
- **Spec principale:** `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md` (§1: l'overlay in-game era «fuori dalla v1, un sotto-progetto con una propria spec»; questa è quella spec)
- **Ricerche (con le fonti):** `docs/superpowers/references/m7/`
  - `r1-overlay-rendering.md`: tecniche di disegno, anti-cheat, licenze dei componenti;
  - `r2-fps-framegen.md`: PresentMon, ETW, frame generation, definizioni delle metriche;
  - `r3-overlay-features.md`: panorama delle funzioni (RTSS OverlayEditor, PresentMon, Steam, MangoHud…);
  - `r4-presentmon-options.md`: console di PresentMon contro servizio con API, letto sui sorgenti della 2.6.0.

## 1. Intento e confini

La M7 ha due parti:
- **una parte di manutenzione (M7a).** Aggiornare sopra una versione installata senza domande, chiudere il debito tecnico che ha un effetto visibile o di robustezza, e dividere i file diventati troppo grandi;
- **la prima funzione nuova dopo la M6: l'overlay in-game.** Mostra sopra i giochi DirectX 9/10/11/12, OpenGL e Vulkan i valori dei sensori, FPS, frametime e le metriche derivate, come testo o come grafico. Si compone in un editor a griglia e si salva in profili.

Il criterio guida è **non mettere mai a rischio l'account dell'utente**: niente entra nel processo del gioco. Gli FPS si misurano dall'esterno con ETW, attraverso PresentMon. L'overlay è una finestra separata che Windows compone sopra il gioco.

### 1.1 Decisioni del brainstorming

| # | Decisione |
|---|---|
| D1 | **Aggiornamento sopra una versione installata:** l'installer chiude l'app da solo, senza chiedere, e a fine installazione la riapre, con i diritti dell'utente, se era aperta. Supera la decisione del 2026-10-02 («l'installer chiede»). |
| D2 | **Manutenzione mirata:** split dei file grandi senza cambi di comportamento, più le voci di debito con effetto visibile o di robustezza (§2.2). Le altre voci minori restano in `docs/follow-ups.md`. |
| D3 | **Sicurezza prima di tutto:** nessuna iniezione di DLL, nessun hook, nessun layer Vulkan implicito. I giochi con anti-cheat sono coperti solo con tecniche esterne. |
| D4 | **Overlay = processo nativo separato `oma-overlay.exe`** (Rust, Direct2D/DirectWrite + DirectComposition): finestra in primo piano, trasparente ai clic. Scartate la finestra WebView2 (fuori budget mentre si gioca) e RTSS come motore (iniettato, bloccato da Vanguard, BattlEye e FACEIT, dipendenza esterna). |
| D5 | **FPS e frametime da PresentMon:** la console `PresentMon-2.6.0-x64.exe`, firmata Intel e con licenza MIT, va inclusa nel setup e la avvia `oma-service`, che ha già i diritti per ETW. Il servizio PresentMon con la sua API è stato valutato (`r4`) e scartato: il doppio del codice, una telemetria che non si spegne, e una pipe di controllo aperta a Everyone in un processo figlio LocalSystem. Si torna a valutarlo solo se lo spike mostra che la console sfora il budget (§4.7). |
| D6 | **Editor in una finestra Tauri** (Svelte), con un'**anteprima reale** disegnata da `oma-overlay`. |
| D7 | **La frequenza dei grafici dell'overlay è un'impostazione separata** da quella dei grafici della UI: 15, 30 o 60 FPS, predefinito 30. |
| D8 | **Funzioni oltre il nucleo, tutte in questa spec:** profilo automatico per gioco, cattura del benchmark, soglie e visibilità condizionale, latenza (Reflex) e collo di bottiglia CPU/GPU. |
| D9 | **Latenza Reflex (PCL) e tracciamento GPU sono opzioni esplicite**, spente di predefinito. Cambiarle riavvia PresentMon, e a gioco avviato un anti-cheat tipo EA Javelin può rifiutare la nuova sessione. |
| D10 | **Nascondi alla cattura** (OBS, screenshot): un'opzione, **spenta** di predefinito. |
| D11 | **Versioni:** la M7a esce da sola come **0.4.1**, la M7 completa come **0.5.0**. |
| D12 | **Spike all'inizio della M7b** su questo PC (RTX 4080 + iGPU AMD) con giochi che l'utente possiede: DLSS FG, FSR 3/4 FG e NVIDIA Smooth Motion. Lossless Scaling non è disponibile. |

### 1.2 Scomposizione in piani

Una spec, quattro piani, ciascuno su un proprio branch `feat/m7x-…` con merge in `main` in locale:

| Piano | Contenuto | Sezioni |
|---|---|---|
| **M7a** manutenzione | aggiornamento sopra versione installata, debito mirato, split, release 0.4.1 | §2 |
| **M7b** motore dei frame | spike, PresentMon nel servizio, protocollo v4, scelta del bersaglio, metriche in `oma-core` | §3, §4 |
| **M7c** overlay | `oma-overlay.exe`, modello del layout, rendering dei blocchi, posizionamento, impostazioni, scorciatoie, tray, profilo automatico, nascondi alla cattura, elenco dei giochi esclusi | §5, §6 |
| **M7d** editor, profili e benchmark | finestra editor, anteprima reale, gestione dei profili, modelli pronti, import/export, cattura del benchmark e storico, release 0.5.0 | §6.5, §6.6, §7, §8 |

L'ordine è vincolante: la M7c consuma le metriche della M7b, e la M7d consuma il modello del layout e il processo della M7c.

### 1.3 Fuori da questa spec

- **Tecniche che entrano nel processo del gioco:** iniezione di DLL, hook di `Present`/`SwapBuffers`/`vkQueuePresentKHR`, layer Vulkan implicito. I giochi Vulkan sono coperti dalla finestra overlay come gli altri.
- **`uiAccess` per il fullscreen esclusivo:** richiede un eseguibile firmato in Program Files, quindi arriva dopo l'ammissione a SignPath. Sarà una spec a parte.
- **Uscita verso RTSS e widget della Xbox Game Bar.**
- **Confronto fra sessioni di benchmark, espressioni e formule sui valori, limitatore di FPS.**
- **Metriche dei frame come sensori dell'engine** (storico, regole, log CSV dei sensori): in questa spec vivono solo nell'overlay, nell'editor e nel benchmark.

## 2. M7a — Manutenzione

### 2.1 Aggiornamento sopra una versione installata

**Oggi.** Il messaggio «il prodotto è ancora in esecuzione» lo mostra il **disinstallatore della versione vecchia**: il nuovo setup lo lancia (`reinst_uninstall` in `installer.nsi`, con `/UPDATE`) mentre l'app è nel tray. Poi `CheckIfAppIsRunning` della sezione `-Install` lo chiederebbe di nuovo.

**Dopo.** Prima di lanciare il disinstallatore vecchio, e comunque prima di `CheckIfAppIsRunning`, il nuovo setup:
1. **Ricorda se l'app era in esecuzione** nella sessione dell'utente che installa (variabile `OmaAppWasRunning`).
2. **Chiude tutte le istanze, di ogni sessione**, senza chiedere, anche con `/S` e `/P`:
   - **chiusura ordinata:** esegue `"$INSTDIR\<exe>" --quit` con `nsis_tauri_utils::RunAsUser`. La seconda istanza inoltra `--quit` alla prima tramite il plugin single-instance, e la prima esce in modo pulito: chiude la sessione del log CSV, il link al servizio e le icone. Poi il setup aspetta fino a 10 s che il processo termini;
   - **solo se la versione installata è almeno 0.4.1**, letta da `DisplayVersion` della chiave di disinstallazione. La 0.4.0 tratterebbe `--quit` come un normale secondo avvio e mostrerebbe la finestra principale (`opens_window` in `main.rs`), quindi con la 0.4.0 si passa subito alla chiusura forzata;
   - **chiusura forzata:** se dopo 10 s qualche istanza è ancora viva, o se la versione installata è la 0.4.0, `nsis_tauri_utils::KillProcess`. Vale anche per le istanze di altri utenti.
3. **Ferma il servizio** come oggi (`OmaStopService`).

A questo punto il disinstallatore vecchio non trova nulla in esecuzione e non chiede niente. Il comando `--quit` entra nell'app con la 0.4.1: dalla 0.4.1 in poi la chiusura è sempre ordinata.

**Fine installazione.** Se `OmaAppWasRunning = 1`, `.onInstSuccess` riapre l'app con `RunAsUser`, cioè senza elevazione, anche in modalità silenziosa e passiva. La casella «Avvia» della pagina finale non deve produrre un secondo avvio; il plugin single-instance renderebbe comunque innocuo un doppio lancio. Il servizio riparte come oggi.

**Disinstallazione da sola**, dal Pannello di controllo: il disinstallatore nuovo **continua a chiedere**, perché lì la domanda ha senso. Con `/UPDATE`, cioè dentro un aggiornamento, chiude senza chiedere come al passo 2. Questo vale per gli aggiornamenti dalla 0.4.1 in poi.

**Dove:**
- `app/src-tauri/nsis/installer.nsi`, solo sulle righe marcate `; OMA`;
- `app/src-tauri/nsis/oma.nsh`, con una nuova macro `OMA_CLOSE_APP` e le stringhe in inglese e italiano;
- `app/src-tauri/src/main.rs`, per `--quit` nel callback single-instance e all'avvio;
- `Italian.nsh` resta invariato.

Il disinstallatore vecchio della 0.4.0 resta quello che è, e non lo si può cambiare. La chiusura preventiva è l'unico modo di evitarne il messaggio.

### 2.2 Debito mirato

Voci prese da `docs/follow-ups.md`. Ognuna con un test che la fissa dove è possibile; al merge si spostano in «Closed in M7a».

| Voce | Dove |
|---|---|
| Test temporizzati instabili: `log::session` (timeout di 100–200 ms) e `reads_disk_temperatures_on_this_machine` (limite di 200 ms sotto carico). Attendere una condizione invece di dormire; il test hardware resta da eseguire solo all'utente. | `app/src-tauri/src/log/session/tests.rs`, `crates/oma-win/` |
| `shell_open`: `ShellExecuteExW` con `SEE_MASK_NOASYNC \| SEE_MASK_FLAG_NO_UI`, join del thread con timeout, controllo che la cartella esista prima di aprirla (errore `folderMissing`). | `app/src-tauri/src/` (comandi shell) |
| `bad_request` e log: nomi di modulo e `type` sconosciuti troncati a 64 caratteri, mai oltre `MaxFrameBytes`. | `service/…/Protocol/`, `Sensors/` |
| Nomi di LibreHardwareMonitor con spazi finali e NUL: si ripuliscono da spazi e caratteri di controllo prima del log e dello schema. | `service/…/Sensors/SchemaBuilder.cs` e log |
| Valore HKCU Run: all'avvio si confronta il percorso salvato con quello dell'eseguibile attuale e lo si ripara; il limite del disinstallatore perMachine (Run di un utente standard) va documentato nel README. | `app/src-tauri/src/autostart.rs`, `README.md` |
| Link al servizio: coda dei comandi limitata (capacità fissa; un comando oltre la capacità risponde con un errore); `NotInstalled`/`Stopped` visibili anche dopo `Incompatible`/`PidMismatch`. | `crates/oma-win/src/svc/link.rs` |
| Grafico vuoto quando tutta la serie è sospesa (standby): mostra l'asse Y e la scritta «In standby» invece di un'area bianca. | `app/src/` (grafici) |
| `used_pct` duplicato in memoria e dischi: un solo helper. | `crates/oma-win/src/memory.rs`, `storage.rs` |
| PDH: conteggio restituito dall'API controllato prima di `from_raw_parts`; `szName` nullo gestito. | `crates/oma-win/src/pdh.rs` |
| `THIRD_PARTY_LICENSES.txt`: aggiungere i `THIRD-PARTY-NOTICES` degli altri pacchetti NuGet Microsoft della pubblicazione del servizio. | `scripts/generate-licenses.ps1` |

### 2.3 Split dei file grandi

Spostamenti meccanici, **senza cambi di comportamento né di API pubblica**. I test esistenti passano invariati prima e dopo; ogni split è un commit a sé, `refactor:`.

| File (righe) | Divisione |
|---|---|
| `crates/oma-win/src/storage.rs` (≈3100) | `storage/gate.rs` (`DiskGate`), `storage/temperatures.rs` (`DiskTemperatures`), `storage/identity_cache.rs` (classe del disco e cache), `storage/mod.rs` (provider) |
| `crates/oma-win/src/svc/link.rs` (≈4300, test compresi) | stato e transizioni, coda dei comandi, I/O della pipe, test in un file a parte |
| `service/…/Sensors/SensorHub.cs` (≈1900) | `GateEpisode.cs`, lettura e stato dei dischi (`DriveStates.cs`), hub |
| `crates/oma-core/src/rules/health.rs` (≈2000) | formattatore delle chiavi di visualizzazione in un suo modulo |
| `service/…Tests/Sensors/SensorHubTests.cs` (≈4400) | seguendo gli split del codice: gate, dischi, hub |

I nomi esatti dei file li fissa il piano, dopo aver letto il codice con graphify.

### 2.4 Release 0.4.1

- `bump-version.ps1 0.4.1`, note di rilascio con «Aggiornamento senza conferme» e le correzioni visibili.
- Tag e push **solo su richiesta dell'utente**.
- L'effetto si vede aggiornando **dalla 0.4.0 alla 0.4.1**, con la chiusura forzata della 0.4.0, e da lì in poi con la chiusura ordinata.

## 3. Overlay: architettura

### 3.1 Processi e responsabilità

```
 gioco ──ETW──► PresentMon.exe ──CSV su stdout──► oma-service (LocalSystem)
                (figlio, Job Object)               FrameCapture: parser, filtro PID, lotti a 10 Hz,
                                                   riepilogo dei processi che presentano (1 Hz)
                                                         │ pipe esistente, protocollo v4
                                                         ▼
                                     oma-app (utente): engine dei sensori; oma-core::frames (metriche);
                                     overlay::Target (primo piano), profili, scorciatoie, tray
                                                         │ pipe privata app→overlay (oma-ipc, framing esistente)
                                                         ▼
                                     oma-overlay.exe (utente): finestra trasparente ai clic,
                                     D2D/DirectWrite + DirectComposition
```

| Componente | Responsabilità | Non fa |
|---|---|---|
| `oma-service`, `FrameCapture` | avvia e sorveglia PresentMon, legge il CSV, filtra il PID bersaglio, invia i frame grezzi e il riepilogo dei processi | statistiche, scelte sul bersaglio |
| `crates/oma-core`, `frames` (nuovo modulo) | metriche pure: FPS, frametime, low, stutter, moltiplicatore FG, latenza, collo di bottiglia, scelta della swapchain, generatore di frame sintetici | I/O, Windows |
| `crates/oma-core`, `overlay` (nuovo modulo) | modello del layout e dei profili (serde), validazione, geometria della griglia, valutazione di soglie e condizioni | disegno |
| `app/src-tauri`, `overlay` (nuovo modulo) | primo piano e bersaglio, profilo attivo, associazioni gioco→profilo, ciclo di vita di `oma-overlay`, inoltro di valori e metriche, scorciatoie, tray, benchmark | disegno |
| `crates/oma-overlay` (nuovo crate binario) | finestra, posizionamento, disegno dei blocchi, anteprima | logica dei dati |
| `app/src`, editor (nuova finestra Svelte) | composizione dei profili | — |

Il codice Windows nuovo (DirectWrite per l'elenco dei font, `SetWinEventHook`, processi) segue le regole FFI del progetto: un commento `// SAFETY:` per ogni blocco `unsafe`, assert di dimensione per le struct, revisione `ffi-safety-reviewer`. La finestra e il disegno usano il crate `windows`, già nel workspace.

### 3.2 Ciclo di vita

| Stato dell'impostazione | PresentMon | `oma-overlay` | Costo atteso |
|---|---|---|---|
| Overlay spento (predefinito) | non in esecuzione | non in esecuzione | zero |
| Overlay acceso, nessun gioco | in esecuzione, prima del gioco, per gli anti-cheat tipo EA | in esecuzione, finestra **nascosta** | da misurare nello spike (§4.7) |
| Overlay acceso, gioco in primo piano | in esecuzione | finestra visibile sopra il gioco | §11 |
| Overlay nascosto con la scorciatoia | in esecuzione | finestra **nascosta** (`SW_HIDE`), non solo vuota | come «nessun gioco» |
| Anteprima dall'editor | come sopra | avviato anche con l'overlay spento, solo la finestra di anteprima | — |

La finestra nascosta, e non solo trasparente, serve a lasciare intatti il flip indipendente e G-Sync/FreeSync quando l'overlay non si vede.

## 4. M7b — Motore dei frame

### 4.1 PresentMon nel servizio (`FrameCapture`)

- **Componente:** `PresentMon-2.6.0-x64.exe`, firmato Intel. Lo scarica `build-installer-payload.ps1`, che lo accetta solo con lo SHA-256 fissato in `app/src-tauri/nsis/presentmon.sha256` e con la firma Authenticode di Intel verificata, come per PawnIO. Si installa in `$INSTDIR\service\presentmon\`, una cartella che nessun utente può scrivere.
- **Avvio:** quando l'app chiede `FramesConfigure { enabled: true, … }`. Prima di ogni avvio il servizio ricalcola lo SHA-256 dell'eseguibile; se non coincide, rifiuta (stato `Tampered`).
- **Argomenti fissi**, senza parti fornite dall'utente: `--output_stdout --no_console_stats --qpc_time --track_frame_type --session_name OpenMonitorAdvanced-Frames --stop_existing_session --no_track_input`, più `--track_pc_latency` se `trackPcLatency` e `--no_track_gpu` se **non** `trackGpu`. L'elenco definitivo, e la scelta delle colonne v1 o `--v2_metrics`, li fissa lo spike.
- **Processo:** figlio in un Job Object con `KILL_ON_JOB_CLOSE`, standard output letto in modo asincrono, standard error nel log (righe troncate).
- **Svuotamento dei buffer ETW:** PresentMon non lo fa da sé, e senza svuotamento i dati arrivano circa una volta al secondo. Il servizio chiama `ControlTraceW(…, "OpenMonitorAdvanced-Frames", …, EVENT_TRACE_CONTROL_FLUSH)` ogni 100 ms mentre PresentMon gira.
- **Parser:**
  - legge per **nome di colonna** dall'intestazione, quindi l'ordine delle colonne non conta;
  - ogni riga ha una lunghezza massima di 4 KiB; le righe oltre e i valori non numerici si scartano e si contano;
  - una colonna obbligatoria mancante porta allo stato `Failed("columns")`.
- **Filtro:** solo le righe del PID bersaglio diventano frame. Per **tutti** i PID si tiene un riepilogo di 1 s: frame mostrati, `PresentMode`, nome del processo, swapchain.
- **Arresto:** `FramesConfigure { enabled: false }`, oppure l'app che si disconnette per più di 30 s, oppure l'arresto del servizio. Il Job Object chiude PresentMon, poi il servizio chiama `ControlTraceW(…, STOP)` sulla **sola** sessione con il nostro nome. All'avvio del servizio la nostra sessione, rimasta da un crash, si ferma per nome.
- **Cadute:** riavvio dopo 1, 2, 4, 8… s, fino a 60 s; dopo 5 cadute in 10 minuti, stato `Failed("crashing")` fino a un nuovo `FramesConfigure`.
- **Sessione negata** (`StartTrace` → accesso negato, il caso di EA Javelin): stato `Denied`. L'app suggerisce «attiva l'overlay prima di avviare il gioco» (§9).

La sorgente dei frame sta dietro un'interfaccia (`IFrameSource`), così che la via del servizio PresentMon (`r4`) si possa aggiungere senza toccare il protocollo.

### 4.2 Protocollo v4

`PROTOCOL_VERSION = 4`. Valgono le regole esistenti: chiavi sempre presenti, `nil` per i valori assenti, fixture in `protocol/fixtures/` rigenerate con `OMA_WRITE_FIXTURES=1`, revisione `protocol-parity-reviewer`. Un'app 0.5.0 con un servizio 0.4.x, o il contrario, dà `Incompatible`, come oggi.

**Dall'app al servizio:**
- `FramesConfigure { enabled: bool, track_pc_latency: bool, track_gpu: bool }`: avvia, riavvia o ferma PresentMon. È idempotente: con gli stessi valori non riavvia.
- `FramesTarget { pid: u32 | nil }`: il processo da seguire. Con `nil` si segue solo il riepilogo.

**Dal servizio all'app:**
- `FramesStatus { state, detail: str | nil, presentmon_version: str | nil }`, dove `state` è uno fra `Off`, `Starting`, `Running`, `Denied`, `Tampered`, `Missing`, `Failed`. Si invia a ogni cambio.
- `PresentingProcesses { at_qpc, processes: [ { pid, name, displayed_fps, present_mode, swapchains } ] }`: 1 Hz, solo i processi con almeno un frame nell'ultimo secondo, al massimo 32 voci.
- `FrameBatch { pid, frames: [ Frame ] }`: 10 Hz mentre c'è un bersaglio che presenta; al massimo 512 frame per lotto, quelli in eccesso si contano in `dropped`.
- `Frame`:
  - `qpc` (inizio della presentazione), `swapchain`;
  - `frame_type`: `app`, `generated_intel_xefg`, `generated_amd_afmf`, `generated_other` o `unknown`;
  - `displayed: bool`, `ms_between_presents`, `ms_between_display_change` (nil se non mostrato), `ms_until_displayed`;
  - `ms_app_frametime` (nil se assente);
  - `ms_pc_latency` (nil senza PCL), `ms_gpu_busy` (nil senza `trackGpu`);
  - `pcl_frame_id` (nil senza PCL).

I campi esatti, e da quali colonne del CSV derivano, li fissa lo spike.

### 4.3 Scelta del bersaglio (app, `overlay::Target`)

- **Primo piano:** `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` senza thread di polling. Ricava il PID con `GetWindowThreadProcessId`; per `ApplicationFrameHost.exe` (giochi UWP) prende il PID della `CoreWindow` figlia.
- **È un gioco se** il PID in primo piano compare in `PresentingProcesses` con almeno 10 FPS mostrati, **e** non è escluso:
  - i nostri processi;
  - `dwm.exe` e i processi di sistema noti;
  - l'elenco «non mostrare l'overlay in questo gioco» (§5.6).
- **Tolleranza:** se il bersaglio perde il primo piano o smette di presentare, resta tale per 3 s, per reggere alt-tab e schermate di caricamento.
- **Finestra del gioco:** la finestra principale in primo piano del PID; la sua posizione si segue con `EVENT_OBJECT_LOCATIONCHANGE`, filtrato su quella finestra.
- **Swapchain:** fra quelle del bersaglio si usa quella con più frame mostrati nell'ultimo secondo (`oma-core::frames`).

### 4.4 Metriche (`oma-core::frames`)

Funzioni pure su una finestra scorrevole di frame. Sia `ft` il frametime in ms e `T` la somma degli `ft` nella finestra.

| Metrica | Definizione |
|---|---|
| **FPS mostrati** | `1000 · N / T` sui frame mostrati, con `ft = ms_between_display_change`; i frame generati sono compresi e quelli scartati esclusi. Mai la media di `1000/ft_i`. |
| **FPS renderizzati** | `1000 · N_app / T` sui soli frame dell'app, secondo la cascata di §4.5; nil se l'origine è `unknown`. |
| **FPS presentati** | chiamate a `Present` al secondo; solo diagnostica, non è un blocco predefinito. |
| **Frametime mostrato / dell'app** | per frame: `ms_between_display_change` e `ms_app_frametime`. |
| **1% e 0.1% low** | due definizioni, scelte per blocco: **integrale** (predefinita, come Afterburner e CapFrameX: si sommano i frametime più lunghi fino all'1%, o allo 0.1%, di `T`, e si prende `1000/ft` dove la somma passa la soglia) e **percentile** (`1000 / P99(ft)`, `1000 / P99.9(ft)`). Si calcolano sui frame mostrati; l'etichetta indica sempre la definizione. |
| **Moltiplicatore FG** | FPS mostrati ÷ FPS renderizzati nella stessa finestra; nil se uno dei due manca. |
| **Stutter** | un frame con `ft > 2,5 × mediana scorrevole di 2 s` **e** `ft − mediana > 8 ms`. Si riporta come conteggio e come percentuale del tempo. |
| **Latenza PC** | media di `ms_pc_latency` (solo con PCL e giochi con Reflex). |
| **Latenza di visualizzazione** | media di `ms_until_displayed`. |
| **Collo di bottiglia** (solo con `trackGpu`) | `gpu` se `ms_gpu_busy ≥ 0,9 × ft_app` per la maggior parte dei frame della finestra, altrimenti `cpu`; con meno di 30 frame `unknown`. La soglia la conferma lo spike. |

La finestra predefinita è 1 s per gli FPS e 10 s per low e stutter. Ogni blocco può cambiarla (§6.3).

### 4.5 Frame generation: FPS renderizzati e FPS mostrati

Gli FPS renderizzati si ricavano da questa cascata. L'origine si mostra, se il blocco lo chiede, come piccola etichetta («XeSS-FG», «AFMF», «Reflex», «FG?»).

1. **Tipo di frame dal driver** (Intel XeSS-FG, AMD AFMF): i frame `app` sono quelli renderizzati. **Esatto.**
2. **Marcatori Reflex/PCL** (DLSS FG/MFG e ogni gioco con Reflex), solo con `trackPcLatency`: gli FPS renderizzati sono gli `pcl_frame_id` distinti al secondo. **Quasi esatto; lo spike verifica che funzioni con DLSS FG.**
3. **Nessuna prova** (FSR 3/4 FG, Smooth Motion finché lo spike non dice altro, DLSS senza PCL): FPS renderizzati nil, si mostrano solo gli FPS mostrati.
   - **Euristica «FG?»:** se gli intervalli fra le presentazioni alternano in modo regolare brevi e lunghi (rapporto ≥ 1,8 per 2 s), un blocco di FPS renderizzati mostra «FG?» invece di un numero.
   - Un numero ricavato da un'euristica non si presenta **mai** come FPS renderizzati.

I frame `generated_*` restano nei frametime mostrati. Low e stutter si calcolano sui frame mostrati; il frametime dell'app ha un suo blocco.

### 4.6 Generatore di frame sintetici

`oma-core::frames::synthetic(seed, profile)` produce lotti deterministici con:
- FPS di base, frame generation ×2 o ×3, jitter e stutter occasionali;
- latenza e GPU busy.

Lo usano l'editor e l'anteprima quando non c'è un bersaglio, e i test.

### 4.7 Spike (codice da buttare, solo risposte)

Prima di scrivere il resto del piano M7b, su questo PC e con l'utente che avvia i giochi:

| # | Domanda | Come | Porta di decisione |
|---|---|---|---|
| S1 | Come appaiono nel CSV e nell'ETL un gioco senza FG, con DLSS FG, con FSR FG e con Smooth Motion? | PresentMon con `--track_frame_type --track_pc_latency`, più WPR con i provider di `r2` §1.2; catture salvate come fixture (senza dati personali) | colonne e campi del §4.2; la cascata del §4.5 vale per DLSS? |
| S2 | Quanto costa PresentMon sempre acceso, senza gioco e con un gioco? | `measure-footprint.ps1` e i contatori del processo | > 1% della CPU senza gioco ⇒ si valuta la via del servizio PresentMon (D5) |
| S3 | PresentMon gira come figlio del servizio (LocalSystem, sessione 0) e il FLUSH da `ControlTraceW` funziona? | prototipo in una console avviata come LocalSystem (`psexec -s` o un servizio di prova) | se no, si rivede §4.1 |
| S4 | Cosa fa una finestra in primo piano, trasparente ai clic, sopra un gioco borderless? | prototipo D2D + DirectComposition; si osserva il `PresentMode` del gioco (flip indipendente → composto?) e la frequenza G-Sync mostrata dal monitor | se rompe sempre il flip indipendente: lo si scrive nei limiti e la finestra resta nascosta finché l'utente non la chiama |
| S5 | `--stop_existing_session` tocca solo la nostra sessione? Convivenza con CapFrameX o FrameView, se l'utente li ha. | prova diretta | — |

I risultati vanno in `docs/superpowers/references/m7/spike-findings.md`; le correzioni alla spec si annotano nel piano come decisioni.

## 5. M7c — `oma-overlay.exe`

### 5.1 Finestra

- **Stili:** `WS_POPUP` con `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`, quindi niente barra delle applicazioni, niente fuoco e i clic che passano al gioco.
- **Disegno:** contenuto composto con DirectComposition su una swapchain `DXGI_ALPHA_MODE_PREMULTIPLIED` (D3D11 + Direct2D), senza `UpdateLayeredWindow`.
- **DPI:** Per-Monitor v2.
- **Nascondi alla cattura:** con `hideFromCapture`, `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`; dove non è supportato (Windows 10 prima della 2004), `WDA_MONITOR` e una riga nel log.
- **Visibilità:** `SW_HIDE` quando non c'è un bersaglio, quando l'utente nasconde l'overlay, quando il gioco è nell'elenco escluso e quando l'overlay è spento.

### 5.2 Disegno

- **Testo:** DirectWrite con i font di sistema; contorno e ombra con un renderer di testo personalizzato o con geometria. Nessun font incluso nel pacchetto.
- **Grafici:** geometrie Direct2D ricostruite solo quando cambiano i dati; i colori sono brush in cache.
- **Cadenza:**
  - grafici e frametime alla frequenza `overlay.chartFps` (15/30/60, predefinito 30);
  - testi a `overlay.textHz` (2 o 4 Hz, predefinito 2);
  - senza cambiamenti non si presenta nulla.
- **HDR:** il contenuto è SDR e lo compone DWM. In un gioco HDR il bianco segue il cursore «luminosità del contenuto SDR» di Windows; è un limite dichiarato (§14).

### 5.3 Posizionamento

- **`overlay.attach`:**
  - `window` (predefinito): l'area client della finestra del gioco;
  - `monitor`: l'intero monitor che la contiene (`MonitorFromWindow`).
- In borderless le due coincidono.
- Il profilo si ancora a uno dei 9 punti dell'area, con lo scostamento in celle (§6.1).
- Con un cambio di posizione o di dimensione della finestra del gioco la finestra overlay si riallinea al tick successivo dell'evento.

### 5.4 Pipe fra app e overlay

- **Creazione:** l'app crea `\\.\pipe\OpenMonitorAdvanced-Overlay-<uuid casuale>` con `FILE_FLAG_FIRST_PIPE_INSTANCE`, un DACL che concede l'accesso solo al SID dell'utente corrente e `PIPE_REJECT_REMOTE_CLIENTS`.
- **Collegamento:** l'app avvia `oma-overlay.exe --pipe <nome>`, e alla connessione controlla che il PID del client sia quello del figlio appena avviato (`GetNamedPipeClientProcessId`).
- **Messaggi:** framing e MessagePack di `oma-ipc`, con tipi propri in un modulo `oma-ipc::overlay` (versione a parte, `OVERLAY_PROTOCOL_VERSION = 1`, con lo stesso controllo di compatibilità):

| Messaggio | Contenuto |
|---|---|
| `Hello` | versione, in entrambe le direzioni |
| `SetProfile` | il profilo completo (§6.5) e le impostazioni di disegno (`chartFps`, `textHz`, `hideFromCapture`, `attach`) |
| `SetPlacement` | rettangolo in pixel fisici e DPI del monitor, oppure `Hidden` |
| `Values` | a ogni tick dell'engine, solo i sensori usati dal profilo attivo: id, valore, qualità |
| `FrameMetrics` | a `textHz`: le metriche del §4.4 già calcolate |
| `FrameTimes` | a 10 Hz: i frametime mostrati e dell'app del lotto, per i grafici del frametime |
| `Preview` | apre o chiude la finestra di anteprima (§7.4) |
| `Benchmark` | stato del ● REC e riepilogo finale (§8) |
| `Fonts` (risposta) | elenco delle famiglie DirectWrite (per l'editor, anche se l'overlay è spento) |

- **Ciclo di vita:**
  - l'overlay esce quando la pipe si chiude;
  - l'app lo riavvia dopo 1, 2, 4… s, fino a 60 s; dopo 5 cadute in 10 minuti lo stato diventa `Failed` fino a un nuovo «attiva».

### 5.5 Scorciatoie e tray

- **Scorciatoie globali**, con l'infrastruttura della M5c (`hotkeys.rs`, forma canonica `Ctrl+Alt+Shift+X`, stato `HotkeyStatus`): `overlay.hotkeyToggle` (mostra/nascondi), `overlay.hotkeyNextProfile`, `overlay.hotkeyBenchmark`. Sono **non impostate** di predefinito; i conflitti si segnalano come per il log.
- **Tray:** voci «Mostra/nascondi overlay» (spunta) ed «Editor overlay».

### 5.6 Profilo automatico e giochi esclusi

- **`overlay.gameProfiles`:** mappa dal nome dell'eseguibile, senza distinzione fra maiuscole e minuscole e senza percorso, all'id del profilo.
  - Al cambio di bersaglio si attiva il profilo associato, altrimenti `overlay.defaultProfile`.
  - `hotkeyNextProfile` cambia profilo fino al prossimo cambio di bersaglio.
- **`overlay.blockedGames`:** elenco di eseguibili sopra i quali l'overlay non si mostra mai. PresentMon continua comunque a misurarli, perché è passivo.
- **«Associa al gioco attuale» / «Escludi il gioco attuale»:** pulsanti nelle impostazioni, attivi quando c'è un bersaglio.

### 5.7 Impostazioni dell'overlay (sezione `overlay` di `settings.json`)

| Chiave | Tipo | Predefinito |
|---|---|---|
| `enabled` | bool | `false` |
| `chartFps` | 15 \| 30 \| 60 | 30 |
| `textHz` | 2 \| 4 | 2 |
| `hideFromCapture` | bool | `false` |
| `attach` | `window` \| `monitor` | `window` |
| `trackPcLatency` | bool | `false` |
| `trackGpu` | bool | `false` |
| `defaultProfile` | id | modello «Gaming» |
| `gameProfiles` | mappa exe → id | `{}` |
| `blockedGames` | elenco di exe | `[]` |
| `hotkeyToggle`, `hotkeyNextProfile`, `hotkeyBenchmark` | scorciatoia \| null | `null` |

La pagina va in **Impostazioni › Overlay**, con lo stato del motore (§9), il pulsante «Apri editor», le spiegazioni di `trackPcLatency` (il gioco riceve messaggi «ping» innocui) e di `trackGpu` (più eventi, più costo), e il cambio di ciascuno che riavvia PresentMon.

## 6. Modello del layout e profili (`oma-core::overlay`)

### 6.1 Tela

- **Unità:** la cella, 8 px logici a scala 100%. Le coordinate dei blocchi sono intere, in celle.
- **Profilo:**
  - `anchor`, uno fra 9: `top-left`, `top`, `top-right`, `left`, `center`, `right`, `bottom-left`, `bottom`, `bottom-right`;
  - `offset {x, y}` in celle;
  - `scale` da 0,5 a 3,0, continua, moltiplicata per il DPI del monitor;
  - `panel {color, opacity, radius, padding}` come sfondo comune.
- **Ingombro:** il rettangolo che contiene tutti i blocchi, ancorato all'area §5.3.

### 6.2 Blocchi

Ogni blocco ha `id`, `rect {x, y, w, h}` (celle), `z`, `source`, `stat`, `kind`, `style`, `thresholds`, `visibleIf`, `panel` (opzionale, sostituisce quello del profilo).

**`source`:**
- `{ sensor: "<device_id>/<kind>/<name>" }`, un sensore dello schema dell'app;
- `{ frames: "<metrica>" }`, con la metrica fra `fps-displayed`, `fps-rendered`, `fps-presented`, `frametime-displayed`, `frametime-app`, `low-1`, `low-01`, `fg-multiplier`, `stutter`, `latency-pc`, `latency-display`, `bound`;
- `{ text: "<testo>" }`, un testo fisso.

**`stat`:** `current` (predefinito), `min`, `avg` o `max` su `window` secondi (da 1 a 300). Per i sensori la finestra si tiene nell'overlay, con un anello per sorgente alla frequenza del tick. Per `low-*` la finestra è quella del calcolo (§4.4) e `definition` vale `integral` o `percentile`.

**`kind`:**
1. **`text`:** etichetta, valore e unità, con uno stile per ciascuna delle tre parti.
2. **`graph`:** `line`, `area`, `bars` (valori nel tempo) o `frametime` (una barra per frame, solo con le sorgenti `frametime-*`); facoltativamente il valore sovrapposto.
3. **`meter`:** barra orizzontale o verticale, con `min` e `max` fissi o presi dal sensore (per esempio il 100%).
4. **`sparkline`:** valore con un mini grafico a linea accanto.
5. **`gauge`:** arco da `min` a `max`, con il valore al centro.

### 6.3 Proprietà (`style`)

- **Testo:** `font` (una famiglia DirectWrite), `size` (pt), `weight`, `italic`, `color`, `outline {width, color}`, `shadow {dx, dy, color}`, `align`, `decimals`, `unit` (`auto` o un'unità fissa compatibile, per esempio MB/GB, MHz/GHz), `label` (testo; se assente si usa l'etichetta tradotta del sensore).
- **Grafici:** `range` (da 5 a 300 s), `y {mode: auto | fixed, min, max}`, `line {color, width}`, `fill {color, alpha}`, `gridLines`, `showMinAvgMax`.
- Ogni proprietà ha un valore predefinito, e un profilo salva solo ciò che è diverso dal predefinito.

### 6.4 Soglie e visibilità condizionale

- **`thresholds`:** un elenco ordinato di `{ op: > | >= | < | <=, value, color, target: value | graph | panel }`. Vince la **prima** regola vera; ogni `target` si valuta da sé. Il valore confrontato è quello mostrato dal blocco, dopo `stat` e prima dell'arrotondamento.
- **`visibleIf`:** `{ source, stat, op, value }`, oppure `{ fg: active }` (moltiplicatore > 1,2 o origine «FG?»), oppure assente. Una sorgente assente rende falsa la condizione e il blocco resta nascosto.
- La valutazione è pura, in `oma-core::overlay`, ed è testata da sé.

### 6.5 File dei profili

- **Percorso:** `%APPDATA%\OpenMonitorAdvanced\overlay\profiles\<id>.json`, nella cartella dati dell'app accanto a `settings.json`. L'id è un UUID; il nome visibile sta nel file.
- **Formato:** `{ "format": 1, "name": …, "anchor": …, "offset": …, "scale": …, "panel": …, "blocks": [ … ] }`. Le chiavi sconosciute si rifiutano (`deny_unknown_fields`); i limiti sono 256 blocchi e 64 KiB per testo.
- **Scrittura atomica:** temporaneo più rinomina, come per `settings.json`.
- **Profilo non valido:** il file resta intatto, compare una diagnostica e si usa il modello predefinito.
- **Import ed export:** un file `.omaoverlay.json`. All'import si assegna un nuovo id e, se il nome esiste già, si aggiunge « (2)». Un sensore che non esiste su questo PC resta nel profilo e il blocco mostra «sensore assente».

### 6.6 Modelli pronti

Sono in sola lettura dentro l'app e si duplicano per modificarli:

| Modello | Contenuto |
|---|---|
| **Minimo FPS** | FPS mostrati in un angolo |
| **Gaming** | FPS mostrati e renderizzati, frametime a grafico, 1% low, carico e temperatura di GPU e CPU, VRAM |
| **Completo** | Gaming più RAM, clock, potenza GPU, latenza, collo di bottiglia, moltiplicatore FG |
| **Barra orizzontale** | una riga compatta in alto |

I sensori dei modelli si legano per **ruolo** (la prima GPU dedicata, la CPU) al momento della duplicazione, con le stesse regole del tray e della vista Semplificata.

## 7. M7d — Editor

### 7.1 Finestra

- **Apertura:** finestra Tauri `overlay-editor`, una sola istanza, da Impostazioni › Overlay e dal tray. Dimensione minima 1100×700; la posizione si ricorda.
- **Barra in alto:**
  - selettore del profilo; salva, salva come, rinomina, duplica, elimina, importa, esporta;
  - ancoraggio, scostamento, scala e pannello del profilo;
  - risoluzione simulata: monitor attuale, 1920×1080, 2560×1440 o 3840×2160.
- **Sinistra, la tavolozza:** `SensorTree` con la ricerca, il gruppo «Frame» (metriche del §6.2) e «Testo».
- **Centro, la tela:** l'area simulata in scala, con la griglia e l'ingombro del profilo.
- **Destra, le proprietà** del blocco selezionato. Con più blocchi selezionati si mostrano solo le proprietà comuni, e una modifica si applica a tutti.

### 7.2 Interazioni

- **Creare un blocco:** si trascina una sorgente dalla tavolozza sulla tela. Il tipo predefinito è `text`, `graph` per le sorgenti `frametime-*`.
- **Spostare e ridimensionare:** con il mouse, con l'aggancio alla griglia; le frecce spostano di una cella, Maiusc+frecce ridimensionano.
- **Modificare:** selezione multipla con Maiusc o Ctrl; copia, incolla, duplica (Ctrl+C/V/D), elimina (Canc), porta avanti e indietro.
- **Annulla e ripeti:** Ctrl+Z e Ctrl+Y, con una storia di 100 passi per sessione dell'editor.
- **Uscita con modifiche non salvate:** l'editor chiede salva, scarta o annulla.
- **Accessibilità:** tutto è raggiungibile da tastiera, con focus visibile.

### 7.3 Dati nell'editor

- **Valori:** la tela mostra i valori reali dei sensori dall'engine. Senza un bersaglio, i frame vengono dal generatore sintetico (§4.6), così grafici, moltiplicatore e soglie si vedono in movimento.
- **Disegno della tela:** Canvas2D con la geometria di `oma-core::overlay`, quindi fedele, ma non al pixel per i font. Lo dice una riga sotto la tela, che rimanda all'anteprima.
- **Font:** l'elenco arriva da `oma-overlay` (`Fonts`), avviato se serve, o da un comando DirectWrite in `oma-win`. Il piano sceglie una delle due vie.

### 7.4 Anteprima reale

- **«Anteprima»:** `oma-overlay` apre una normale finestra «Anteprima overlay» (non in primo piano, non trasparente ai clic, sfondo scuro) e ci disegna il profilo **in corso di modifica**, con gli stessi dati della tela.
- **Aggiornamento:** a ogni modifica, con un ritardo di 100 ms.
- **Fedeltà:** è il risultato esatto che si vedrà in gioco.
- **«Usa ora»:** salva e attiva il profilo.

## 8. M7d — Cattura del benchmark

- **Avvio e arresto:** con `hotkeyBenchmark` o con il pulsante in Impostazioni › Benchmark. Serve un bersaglio, altrimenti compare l'avviso «nessun gioco in primo piano».
- **File:** nella cartella dei log della M5c (`log.folder`), sottocartella `benchmarks\`:
  - `<exe>-<AAAAMMGG-hhmmss>.csv`: una riga per frame, con `qpc_ms` (relativo all'inizio), `frametime_displayed_ms`, `frametime_app_ms`, `frame_type`, `displayed`, `pc_latency_ms`, `gpu_busy_ms`. Valgono le regole di sicurezza del registratore M5c: percorsi controllati, formula guard, scrittura con buffer;
  - un `.json` accanto con il riepilogo.
- **Fine della cattura:** con la scorciatoia o il pulsante; dopo 10 s senza bersaglio; dopo 60 minuti.
- **Riepilogo:**
  - gioco, inizio e durata;
  - frame totali, mostrati e generati;
  - FPS medi mostrati e renderizzati (con l'origine);
  - 1% e 0.1% low in **entrambe** le definizioni;
  - frametime minimo e massimo;
  - stutter (conteggio e % del tempo);
  - moltiplicatore FG medio;
  - latenze medie, se disponibili.

  Lo calcola `oma-core::frames` sull'intera sessione.
- **In overlay:**
  - durante la cattura, `● REC mm:ss` in un angolo fisso, che non fa parte dei profili;
  - alla fine, per 10 s, un riquadro di riepilogo con FPS medi, 1% e 0.1% low, stutter.
- **Storico:** l'elenco delle sessioni, letto dai `.json` della cartella, dalla più recente; per ognuna il riepilogo, «Apri CSV», «Apri cartella» ed «Elimina» (con conferma).

## 9. Errori e casi limite

| Situazione | Cosa vede l'utente | Comportamento |
|---|---|---|
| Servizio assente, fermo o incompatibile | blocchi dei frame «—», stato «FPS non disponibili: servizio non attivo» | i blocchi dei sensori funzionano con le sorgenti dell'app |
| PresentMon mancante | «Componente FPS mancante: reinstalla» | stato `Missing` |
| PresentMon con hash diverso | «Componente FPS alterato» | stato `Tampered`, non si avvia |
| Sessione ETW negata | «Il sistema ha rifiutato la misura degli FPS. Attiva l'overlay prima di avviare il gioco.» | stato `Denied`, nuovo tentativo solo con un nuovo `FramesConfigure` |
| PresentMon che cade spesso | «Misura degli FPS interrotta» | §4.1 |
| Gioco in fullscreen esclusivo (`PresentMode` = `Hardware: Legacy Flip` o equivalente) | una notifica per gioco: «L'overlay non è visibile in schermo intero esclusivo: usa la modalità senza bordi» | la misura continua; il benchmark funziona |
| Gioco nell'elenco escluso | nessun overlay | misura attiva, benchmark possibile |
| `oma-overlay` che cade spesso | «Overlay interrotto» | §5.4 |
| Profilo non valido o mancante | diagnostica in Impostazioni › Overlay | modello predefinito |
| Sensore del profilo assente | «sensore assente» nel blocco | il profilo non cambia |
| Frame generation senza prove | FPS mostrati; FPS renderizzati «—» o «FG?» | §4.5 |
| Scorciatoia in conflitto | stato come per il log M5c | — |
| Due monitor con DPI diversi, gioco spostato | l'overlay segue il gioco e si ridisegna al DPI nuovo | `SetPlacement` |
| Gioco UWP | come gli altri | PID della `CoreWindow` |
| Più utenti interattivi | ognuno ha la sua app e il suo overlay; il servizio serve un client solo, come oggi | limite dichiarato |

## 10. Sicurezza e privacy

- **Il gioco non si tocca:** nessun handle sul processo del gioco, nessuna lettura della sua memoria, nessun input sintetico. Il nome del processo arriva dal CSV di PresentMon e, nell'app, da `QueryFullProcessImageNameW` con `PROCESS_QUERY_LIMITED_INFORMATION`.
- **PresentMon:**
  - SHA-256 fissato e firma Intel verificata in fase di build;
  - hash ricontrollato dal servizio prima di ogni avvio;
  - cartella non scrivibile dagli utenti, argomenti fissi;
  - CSV con limiti di riga e di lotto (§4.1, §4.2).
- **Protocollo v4:** il servizio valida il PID (diverso da 0 e 4, al massimo un bersaglio) e non usa mai il PID per aprire processi.
- **Pipe dell'overlay:** nome casuale, prima istanza, DACL solo per l'utente corrente, niente client remoti, controllo del PID del figlio (§5.4).
- **Profili importati:** sono dati, mai codice; vale lo schema rigido del §6.5.
- **Privacy:** nessun dato esce dal PC. Benchmark e profili restano locali.

## 11. Prestazioni

Si aggiungono a `docs/perf-budget.md` e si misurano con `measure-footprint.ps1`, esteso a `oma-overlay` e a PresentMon:

| Stato | Limite |
|---|---|
| Overlay spento | nessun processo in più, costo invariato |
| Overlay acceso, nessun gioco | PresentMon + `oma-overlay` < 0,5% della CPU totale; `oma-overlay` < 40 MB privati |
| Overlay visibile in gioco, profilo «Gaming», grafici a 30 FPS | `oma-overlay` + PresentMon + il lavoro in più di app e servizio < 1% della CPU totale; `oma-overlay` < 40 MB |
| Editor aperto | come la finestra principale (< 200 MB con WebView2) |

Lo spike fissa i numeri di partenza. Se un limite non si rispetta, il piano se ne occupa prima del merge.

## 12. Licenze e documentazione

- **`THIRD_PARTY_NOTICES.md`:** PresentMon (MIT) e i componenti compilati nell'eseguibile (Boost, BSL-1.0; cereal e CLI11, BSD-3-Clause; concurrentqueue), secondo `r4` §8. `generate-licenses.ps1` include i loro testi in `THIRD_PARTY_LICENSES.txt`.
- **Nessun header proprietario:** le costanti PCL non servono, perché le gestisce PresentMon.
- **Font:** solo quelli di sistema, nessun file incluso.
- **README:**
  - la sezione «Overlay in-game», con cosa misura, come attivarlo e scorciatoie;
  - i «Known limits»: fullscreen esclusivo, frame generation non distinguibile per FSR e Smooth Motion (salvo esito dello spike), anti-cheat che bloccano anche le finestre, G-Sync con overlay visibile senza MPO, HDR;
  - la nota di privacy su PCL.
- **`CODE_SIGNING.md`:** PresentMon è un binario firmato Intel che ridistribuiamo; i nuovi eseguibili `oma-overlay.exe` sono nella stessa politica di firma dell'app.
- **`CLAUDE.md`:**
  - i nuovi crate e moduli;
  - il protocollo **v4**;
  - la pipe dell'overlay;
  - lo SHA-256 di PresentMon accanto a quello di PawnIO;
  - i nuovi comandi, se ci sono.
- **`docs/follow-ups.md`:** voci chiuse e aperte a ogni fine piano.

## 13. Verifiche

### 13.1 Test automatici (TDD)

| Area | Test |
|---|---|
| M7a | test della chiusura con `--quit` (logica pura nell'app); test esistenti invariati dopo gli split; test di ogni voce del §2.2 |
| `oma-core::frames` | metriche sulle fixture CSV dello spike (senza FG, DLSS FG, FSR FG, Smooth Motion): FPS mostrati e renderizzati, le due definizioni dei low con casi calcolati a mano, stutter, moltiplicatore, cascata ed euristica «FG?», swapchain, generatore sintetico deterministico |
| `oma-core::overlay` | round trip dei profili, rifiuto di chiavi sconosciute e dei limiti, geometria (ancore, scala, ingombro), soglie (prima vera, per target), `visibleIf`, legame per ruolo dei modelli |
| Servizio | parser CSV per nome di colonna (colonne mancanti, righe lunghe, valori non numerici); ciclo di vita con un finto PresentMon (avvio, caduta e backoff, `Denied`, `Tampered`, arresto per disconnessione); filtro PID e riepilogo; limiti dei lotti |
| Protocollo v4 | fixture condivise Rust/.NET, `nil` sempre presenti; `protocol-parity-reviewer` |
| App | scelta del bersaglio (tolleranza, esclusioni, UWP) con eventi finti; profilo automatico; ciclo di vita di `oma-overlay` (riavvio, controllo del PID); benchmark (file, fine automatica, riepilogo) |
| `oma-overlay` | helper puri di layout, testo e cadenza; test hardware `#[ignore]` per DirectWrite e DirectComposition |
| Editor (Vitest) | annulla/ripeti, aggancio, selezione multipla, proprietà comuni, import ed export, conferma all'uscita; chiavi i18n identiche in `en.json` e `it.json` |
| Script | Pester per la verifica di SHA-256 e firma di PresentMon in `build-installer-payload.ps1` (senza il tag `Integration`) |

### 13.2 Verifiche dal vivo, con l'utente

**Regole:**
- mai clic sintetici: le azioni su tray, finestre e giochi le fa l'utente;
- mai installer eseguiti su questo PC da un agente: le prove dell'installer le fa l'utente, scaricando o costruendo il setup.

**Prove:**
- **M7a:**
  - aggiornamento dalla 0.4.0 installata alla 0.4.1 con l'app nel tray: nessuna domanda, app riaperta;
  - stessa prova con l'app chiusa: l'app non si apre;
  - disinstallazione da Pannello di controllo con l'app aperta: la domanda c'è.
- **M7b:** lo spike (§4.7), poi le metriche in un gioco senza FG, con DLSS FG (con e senza `trackPcLatency`), con FSR FG e con Smooth Motion, confrontate con l'overlay di Steam o con FrameView dove possibile.
- **M7c:**
  - overlay sopra un gioco borderless in DX11, DX12 e Vulkan e, se l'utente ne ha uno, OpenGL;
  - mostra e nascondi; profilo automatico;
  - «nascondi alla cattura» con OBS;
  - avviso di fullscreen esclusivo;
  - spostamento fra monitor;
  - misura del budget.
- **M7d:** editor completo (creazione, proprietà, soglie, condizioni, annulla), anteprima reale, import ed export, benchmark con riepilogo e storico.

## 14. Limiti dichiarati

- **Fullscreen esclusivo vero:** l'overlay non si vede finché non arriva `uiAccess`, dopo la firma. La misura e il benchmark funzionano.
- **FSR 3/4 FG e Smooth Motion:** gli FPS renderizzati non sono distinguibili via ETW (salvo esito diverso dello spike); DLSS FG solo con `trackPcLatency`.
- **G-Sync/FreeSync e latenza:** mentre l'overlay è visibile, Windows può comporre il gioco invece di usare il flip indipendente, a meno che la scheda usi un piano MPO. Nascondere l'overlay ripristina il flip indipendente.
- **Anti-cheat che bloccano anche le finestre esterne** (caso noto: Battlefield 6 con FrameView 2.0): si usa l'elenco dei giochi esclusi.
- **HDR:** il contenuto dell'overlay è SDR composto da DWM.
- **Più utenti interattivi:** il servizio segue un client solo.
- **Disinstallazione della 0.4.0 durante l'aggiornamento:** la sua istanza in esecuzione si chiude in modo forzato, perché non conosce `--quit`.

## 15. Punti che i piani devono fissare

- **M7a:**
  - come trovare le istanze di tutte le sessioni e chiuderle (`nsis_tauri_utils` o `taskkill /F /IM`), e il timeout esatto;
  - la gestione della casella «Avvia» della pagina finale;
  - i nomi dei file degli split, dopo una lettura con graphify.
- **M7b:**
  - esito dello spike: colonne del CSV, argomenti definitivi, soglia del collo di bottiglia, PCL con DLSS, Smooth Motion;
  - dimensione dei buffer ETW;
  - nomi esatti dei tipi del protocollo.
- **M7c:**
  - come disegnare contorno e ombra del testo (renderer DirectWrite personalizzato o geometria);
  - crate `windows` e feature necessarie;
  - percorso d'installazione di `oma-overlay.exe` e sua presenza nel bundle Tauri (risorsa o sidecar);
  - firma nel flusso SignPath.
- **M7d:**
  - la sorgente dell'elenco dei font (§7.3);
  - la libreria di trascinamento (o eventi pointer a mano);
  - il disegno della tela in Canvas2D o SVG.
- **Tutti:** le stringhe esatte in inglese e italiano, e le chiavi i18n nuove.
