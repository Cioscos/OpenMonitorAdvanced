# M7b — Motore dei frame: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** misurare FPS e frametime dei giochi dall'esterno, con PresentMon avviato da `oma-service`, e portarli all'app con il protocollo v4. L'app sceglie il gioco da seguire e calcola le metriche in `oma-core::frames`. Prima di tutto c'è uno spike che fissa i dati mancanti.

**Architecture:**
- **Parte A, lo spike (§4.7 della spec):** cinque domande (S1–S5) con strumenti da buttare in `target/spike/m7b/`, eseguite su questo PC con l'utente che avvia i giochi. Il risultato è un documento di esito, delle fixture CSV e le decisioni sulla spec.
- **Parte B, l'implementazione** (scritta a spike chiuso, come chiede il §4.7):
  - metriche pure in `oma-core::frames` (B1, B2);
  - protocollo v4 in Rust e .NET (B3, B4);
  - nel servizio: parser, processo PresentMon, ciclo di vita e distribuzione ai client (B5–B8);
  - nell'app: collegamento v4, primo piano, scelta del bersaglio e diagnostica dei frame con `OMA_FRAMES_DEBUG` (B9–B11);
  - installer, licenze, misure e documenti (B12).

**Tech Stack:**
- PresentMon 2.6.0 (console, firmata Intel, MIT), ETW (`ControlTraceW`), Job Object;
- Rust 1.90 (`oma-core`, `oma-ipc`, `oma-win` con `windows` 0.62, `oma-app`), .NET 10 + xUnit v3 per il servizio, NSIS e PowerShell 7 + Pester 5.7.1;
- per lo spike: WPR, tracerpt e Python 3 (solo libreria standard).

**Spec:** `docs/superpowers/specs/2026-10-04-m7-manutenzione-overlay-design.md`, §3 e §4 (anche §9, §10, §11 e §15 per la M7b). Ricerche: `docs/superpowers/references/m7/r2-fps-framegen.md` e `r4-presentmon-options.md`.

**Branch:** `feat/m7b-motore-frame`, da `main`; merge in `main` in locale alla fine della Parte B. Push solo su richiesta dell'utente. La M7b non fa release: la 0.5.0 arriva con la M7d (D11).

**Esecuzione:**
- **Parte A in linea, nella sessione principale:** quasi ogni task ha passi dal vivo con l'utente, e gli strumenti non si committano. Ogni sonda si rilegge prima di passarla all'utente.
- **Parte B subagent-driven:** un implementer e una revisione per task, le revisioni `protocol-parity-reviewer` (dopo B4) e `ffi-safety-reviewer` (B6, B10), poi la revisione dell'intero branch. Le prove dal vivo di B12 si fanno con l'utente.

## Global Constraints

- **Lingua e formato:** codice, commenti e messaggi di commit in inglese (conventional commits); documentazione in italiano con gli accenti corretti. Fine riga LF ovunque. Ogni commit termina con `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.
- **Strumenti dello spike:** vivono solo in `target/spike/m7b/` (ignorata da git) e non entrano in nessun commit. Le uniche cose committate dalla Parte A sono:
  - `docs/superpowers/references/m7/spike-findings.md`;
  - le fixture anonime in `testdata/presentmon/`;
  - la sezione «Decisioni dello spike» di questo piano.
- **Privilegi:** un agente non esegue mai comandi elevati, catture dal vivo con PresentMon (sessioni ETW), WPR, operazioni di `schtasks` o installer: li esegue l'utente in un PowerShell amministratore, con i comandi esatti scritti nel passo. Gli agenti preparano, compilano e analizzano; PresentMon lo eseguono solo senza sessione ETW (`--help`, analisi di una ETL con `--etl_file`).
- **Input sintetico:** mai clic sintetici, UI Automation o tasti inviati al desktop o ai giochi; giochi, finestre e tray li usa l'utente.
- **Carico:** niente loop ripetuti né generatori di carico. Ogni misura dura al massimo 90 s ed è avviata dall'utente.
- **Nomi delle sessioni ETW:**
  - quella del prodotto è `OpenMonitorAdvanced-Frames`;
  - quelle dello spike sono `OMA-Spike-A`, `OMA-Spike-B` e `OMA-S5-Other`;
  - mai `PresentMon` né `PMService`, che `--stop_existing_session` toglierebbe ad altri strumenti.
- **PresentMon:**
  - si usa solo `PresentMon-2.6.0-x64.exe` dalla release ufficiale `v2.6.0` di GameTechDev;
  - la firma Authenticode deve risultare `Valid`, con `CN=Intel Corporation`, e la dimensione 980.320 byte;
  - lo SHA-256 si annota nell'esito.
- **Privacy delle fixture:**
  - solo le righe del PID del gioco, con l'intestazione completa;
  - nessun nome di altri processi, nessun percorso;
  - nessuna ETL nel repository.

## Review Focus

Rischi del lavoro dal vivo che nessun test copre, ciascuno legato al passo che lo chiude:

1. **Una sessione ETW resta aperta dopo una cattura interrotta**, consumando buffer non paginati e uno dei 64 slot. `capture.ps1` la ferma in `finally`, e A10 controlla `logman query -ets` (Task A2 passo 1, Task A10 passo 3).
2. **Un PresentMon orfano resta in esecuzione dopo la caduta della sonda.** Il Job Object con `KILL_ON_JOB_CLOSE` si verifica apposta con `--failfast-after` (Task A7 passo 3).
3. **L'operazione pianificata come SYSTEM non viene tolta.** Il passo finale di A7 la elimina, e A10 lo verifica con `schtasks /Query` (Task A7 passo 4, Task A10 passo 3).
4. **Una fixture contiene i nomi di altri processi** (app dell'utente) o righe non del gioco. `analyze.py --trim-fixture` tiene un solo PID, e A10 lo controlla con un `grep` delle colonne `Application` e `ProcessID` (Task A10 passo 1).
5. **Una conclusione nasce da una sola cattura poco rappresentativa** (menu, schermata di caricamento). Ogni cattura è di 30 s in gioco stabile, e l'analisi scarta i primi 2 s. Un esito ambiguo si ripete una volta sola, su richiesta all'utente (Task A6 passo 3).

---

# Parte A — Spike

### Task A1: preparazione e PresentMon verificato

**Files:**
- Create: `target/spike/m7b/bin/PresentMon-2.6.0-x64.exe`, `target/spike/m7b/help.txt`, `target/spike/m7b/notes.md` (non tracciati)

- [x] **Step 1 (agente): committare questo piano su `main` e creare il branch**

```bash
git add docs/superpowers/plans/2026-10-04-m7b-motore-frame.md
git commit -m "docs: add the M7b frame engine plan (spike first)"
git switch -c feat/m7b-motore-frame
```

- [x] **Step 2 (agente): scaricare la console**

Sorgente: `https://github.com/GameTechDev/PresentMon/releases/download/v2.6.0/PresentMon-2.6.0-x64.exe`, salvata in `target/spike/m7b/bin/`.

- [x] **Step 3 (agente): verificare l'eseguibile**

Comando: `Get-AuthenticodeSignature` e `Get-FileHash -Algorithm SHA256`.
Atteso:
- `Status = Valid`;
- `SignerCertificate.Subject` contiene `CN=Intel Corporation`;
- `Length = 980320`.

Hash, soggetto e data vanno in `notes.md`. Se uno dei tre controlli non torna, ci si ferma e si avvisa l'utente.

- [x] **Step 4 (agente): salvare l'aiuto e verificare le opzioni**

Comando: `PresentMon-2.6.0-x64.exe --help > target/spike/m7b/help.txt`.
Atteso: compaiono tutte queste opzioni:

```
--output_stdout --output_file --no_console_stats --qpc_time --track_frame_type
--track_pc_latency --no_track_gpu --no_track_input --write_frame_id --v2_metrics
--session_name --stop_existing_session --terminate_existing_session --timed
--terminate_after_timed --etl_file
```

Le opzioni che mancano vanno in `notes.md`, e i passi che le usano si adattano.

### Task A2: `capture.ps1` e profilo WPR

**Files:**
- Create: `target/spike/m7b/capture.ps1`, `target/spike/m7b/m7b.wprp` (non tracciati)

- [x] **Step 1 (agente): scrivere `capture.ps1`**

L'intestazione è `#Requires -RunAsAdministrator`. Parametri:
- `-Name` obbligatorio, con convalida `^[a-z0-9-]{1,32}$`;
- `-Seconds` con valore predefinito 30 (al massimo 90);
- gli switch `-Pcl`, `-NoGpu` e `-Wpr`.

Comportamento:
1. **Avvio di due PresentMon in parallelo**, ciascuno con `Start-Process -PassThru`, le stesse opzioni comuni e `--timed <Seconds> --terminate_after_timed`:
   - A: `--session_name OMA-Spike-A --output_file <dir>\<Name>-v1.csv`;
   - B: `--session_name OMA-Spike-B --output_file <dir>\<Name>-v2.csv --v2_metrics`.

   Opzioni comuni:

   `--stop_existing_session --no_console_stats --qpc_time --track_frame_type --write_frame_id --no_track_input`

   più `--track_pc_latency` con `-Pcl` e `--no_track_gpu` con `-NoGpu`. `<dir>` è `target\spike\m7b\captures\`.
2. **Con `-Wpr`:** prima dell'avvio `wpr -start <dir>\..\m7b.wprp!M7b -filemode`; dopo la fine dei due processi `wpr -stop <dir>\<Name>.etl`.
3. **Chiusura in `finally`:**
   - termina i PresentMon ancora vivi;
   - esegue `logman stop OMA-Spike-A -ets` e `logman stop OMA-Spike-B -ets`, ignorando «non trovato»;
   - con `-Wpr`, esegue `wpr -cancel` se la registrazione è ancora attiva.
4. **Stampa finale:** dimensione e numero di righe dei due CSV, poi l'esito di `logman query -ets` filtrato su `OMA-Spike`, che deve risultare vuoto.

- [x] **Step 2 (agente): scrivere `m7b.wprp`**

È un profilo WPR personalizzato chiamato `M7b`, in modalità file, con livello di dettaglio `Verbose` e soli provider utente, a livello 5 e con tutte le keyword:
- `Microsoft-Windows-DxgKrnl` `802EC45A-1E99-4B83-9920-87C98277BA9D`;
- `Microsoft-Windows-DXGI` `CA11C036-0102-4A2D-A6AD-F03CFED5D3C9`;
- `Microsoft-Windows-Dwm-Core`;
- `Microsoft-Windows-Kernel-Process`;
- Intel-PresentMon `ECAA4712-4644-442F-B94C-A32F6CF8A499`;
- NVIDIA Display Driver `AE4F8626-8265-40D1-A70B-11B64240E8E9`;
- NVIDIA PCL Stats `0D216F06-82A6-4D49-BC4F-8F38AE56EFAB`.

I GUID di Dwm-Core e Kernel-Process si prendono da `logman query providers "<nome>"`, non a memoria. Nessun provider del kernel, per non registrare l'attività di tutto il sistema.

- [x] **Step 3 (agente): verificare la sintassi senza eseguire**

Comandi:
- `pwsh -NoProfile -Command "[System.Management.Automation.Language.Parser]::ParseFile('target/spike/m7b/capture.ps1',[ref]$null,[ref]$e) | Out-Null; $e"`;
- `[xml](Get-Content target/spike/m7b/m7b.wprp)`.

Atteso: nessun errore di parsing, XML valido.

### Task A3: `analyze.py`

**Files:**
- Create: `target/spike/m7b/analyze.py` (non tracciato)

- [x] **Step 1 (agente): scrivere l'analizzatore**

Comando: `python analyze.py <csv> [--app <nome.exe>] [--skip 2] [--trim-fixture <out.csv> --seconds 10]`. Usa solo la libreria standard (`csv`, `statistics`), legge le colonne per nome e funziona con i CSV v1 e v2. Stampa:
- l'elenco delle colonne dell'intestazione;
- per ogni coppia (PID, swapchain) con almeno 30 righe:
  - righe, righe mostrate e conteggi di `FrameType` e `PresentMode`;
  - FPS mostrati: `1000·N / Σ MsBetweenDisplayChange` sulle righe con valore numerico;
  - FPS presentati: righe al secondo, dal campo del tempo QPC;
- se c'è una colonna con l'id del frame:
  - il suo nome;
  - gli id distinti al secondo, e il rapporto tra righe mostrate e id distinti;
- la percentuale di righe con `MsPCLatency` numerico;
- **alternanza:** su finestre di 2 s, la quota di coppie consecutive di `MsBetweenPresents` con segno alterno delle differenze e il rapporto `mediana(lunghi) / mediana(brevi)`;
- **collo di bottiglia:** se ci sono una colonna GPU busy e una del frametime dell'app, la quota di frame con `busy ≥ 0,9 × ft_app`;
- **righe scartate**, con il motivo: valore non numerico, `NA`, colonna mancante.

`--app` sceglie il processo con più righe tra quelli con quel nome. `--skip` scarta i primi N secondi. `--trim-fixture` scrive l'intestazione completa e le sole righe del PID scelto per `--seconds` secondi, dopo lo skip.

- [x] **Step 2 (agente): provarlo su un CSV costruito a mano**

Il CSV ha 4 righe di 2 PID, una con `NA`, salvato in `target/spike/m7b/selftest.csv`.
Atteso: i conteggi corrispondono a quelli scritti nel file, e la riga `NA` risulta scartata con il motivo.

### Task A4: sonda del servizio `flush-probe` (S2, S3, S5)

**Files:**
- Create: `target/spike/m7b/flush-probe/flush-probe.csproj`, `target/spike/m7b/flush-probe/Program.cs` (non tracciati)

Riproduce il §4.1 della spec quanto basta per misurarlo, nel linguaggio di `FrameCapture` (.NET).

- [x] **Step 1 (agente): scrivere la sonda**

Console `net10.0`, senza pacchetti NuGet, P/Invoke scritti a mano su `advapi32` e `kernel32`. Argomenti:
- `--presentmon <path>` e `--out <log>`;
- `--seconds 60`, al massimo 90;
- `--flush-ms 0|100`;
- `--pcl`, `--gpu` e `--failfast-after <s>`.

Sequenza:
1. **Identità:** nel log scrive l'identità (`WindowsIdentity.GetCurrent().Name`), l'id di sessione del processo e la versione di Windows.
2. **Pulizia iniziale:** `ControlTraceW(0, "OpenMonitorAdvanced-Frames", props, EVENT_TRACE_CONTROL_STOP)` e il codice restituito: 0 = fermata una sessione rimasta, 4201 = nessuna.
3. **Avvio di PresentMon** in un Job Object con `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, lo standard output letto in modo asincrono e lo standard error nel log (righe troncate a 512 caratteri). Argomenti fissi del §4.1:

   `--output_stdout --no_console_stats --qpc_time --track_frame_type --session_name OpenMonitorAdvanced-Frames --stop_existing_session --no_track_input`

   più `--track_pc_latency` con `--pcl`; `--no_track_gpu` si aggiunge se **non** c'è `--gpu`.
4. **Per ogni riga del CSV:**
   - ritardo = `QueryPerformanceCounter()` meno `TimeInQPC` (colonna cercata per nome), in ms;
   - contatore delle righe al secondo.
5. **Svuotamento dei buffer:** se `--flush-ms` è maggiore di 0, un timer chiama `ControlTraceW(…, EVENT_TRACE_CONTROL_FLUSH)` a quell'intervallo e conta gli errori.
6. **Stato della sessione,** ogni 10 s e alla fine, con `ControlTraceW(…, EVENT_TRACE_CONTROL_QUERY)`: `BufferSize`, `MinimumBuffers`, `MaximumBuffers`, `NumberOfBuffers`, `EventsLost`, `RealTimeBuffersLost` e `BuffersWritten`.
7. **Fine:**
   - CPU del processo PresentMon e della sonda tra inizio e fine, come percentuale della CPU totale (`TotalProcessorTime / tempo / ProcessorCount`);
   - picco della memoria privata di PresentMon;
   - percentili 50, 95 e massimo del ritardo;
   - righe totali e scartate.
8. **Chiusura:** chiude l'handle del Job Object, verifica che PresentMon sia uscito, chiama `ControlTraceW(…, STOP)` e poi `QUERY`, che deve dare 4201, con il codice nel log.
9. **Caduta simulata:** con `--failfast-after N`, dopo N secondi `Environment.FailFast("spike")`, senza i passi 7 e 8.

`EVENT_TRACE_PROPERTIES` ha il nome della sessione nello stesso buffer, a `LoggerNameOffset`. La dimensione della struct si controlla con `Marshal.SizeOf` all'avvio e si confronta con 120 byte (x64); se non coincide, la sonda esce con un errore.

- [x] **Step 2 (agente): compilare**

Comando: `dotnet build target/spike/m7b/flush-probe -c Release`.
Atteso: compilazione riuscita, 0 avvisi. L'agente non la esegue: senza privilegi otterrebbe solo accesso negato.

### Task A5: sonda della finestra `overlay-probe` (S4)

**Files:**
- Create: `target/spike/m7b/overlay-probe/Cargo.toml` (con `[workspace]` proprio), `target/spike/m7b/overlay-probe/src/main.rs` (non tracciati)

- [x] **Step 1 (agente): scrivere la sonda**

Rust con `windows = "0.62"`; le feature servono per D3D11, DXGI, Direct2D, DirectWrite, DirectComposition, WindowsAndMessaging e Gdi. Argomenti:
- `--x 16 --y 16 --w 320 --h 120` e `--fps 30`;
- `--delay 10`, `--seconds 90`, `--hide-at 30` e `--show-at 60`;
- `--empty` e `--log <file>`.

Finestra:
- **stile:** `WS_POPUP`, con stili estesi `WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`, più `SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA)`;
- **visualizzazione:** `ShowWindow(SW_SHOWNOACTIVATE)`.

Disegno:
- dispositivo D3D11 con `D3D11_CREATE_DEVICE_BGRA_SUPPORT`;
- swapchain da `IDXGIFactory2::CreateSwapChainForComposition`, `B8G8R8A8_UNORM`, `DXGI_ALPHA_MODE_PREMULTIPLIED`, `FLIP_SEQUENTIAL`, 2 buffer;
- `ID2D1DeviceContext` sul back buffer;
- `DCompositionCreateDevice`, `CreateTargetForHwnd(hwnd, topmost = true)` e una visual con la swapchain come contenuto, poi `Commit`.

Ogni frame, alla cadenza di `--fps`, disegna un rettangolo scuro al 60% di opacità e il testo DirectWrite `OMA probe  frame <n>` (Segoe UI, 18 px). Con `--empty` cancella a trasparente e basta.

Fasi:
- attesa di `--delay` s, per dare all'utente il tempo di tornare al gioco;
- visibile fino a `--hide-at`;
- `ShowWindow(SW_HIDE)` fino a `--show-at`;
- di nuovo visibile fino a `--seconds`.

A ogni cambio di fase scrive `QueryPerformanceCounter`, ora locale e fase su stdout e nel log. Esce da solo a fine durata, e anche su `WM_CLOSE`. Non registra scorciatoie e non legge input.

Ogni blocco `unsafe` ha il suo commento `// SAFETY:` anche nella sonda, perché la M7c ne riprenderà il codice.

- [x] **Step 2 (agente): compilare**

Comando: `cargo build --release --manifest-path target/spike/m7b/overlay-probe/Cargo.toml`.
Atteso: compilazione riuscita. La prima esecuzione è dell'utente, in A8.

### Task A6: S1, catture con e senza frame generation (utente + agente)

**Files:**
- Create: `target/spike/m7b/captures/*.csv`, `*.etl` (non tracciati)

- [x] **Step 1 (agente → utente): scegliere i giochi**

Si chiede all'utente quale gioco usa per ciascun caso:

| Caso | Requisito | Comando (PowerShell amministratore, dalla radice del repository) |
|---|---|---|
| C1 senza FG | DX12 o DX11, borderless | `pwsh -File target\spike\m7b\capture.ps1 -Name nofg` |
| C2 DLSS FG | gioco con DLSS FG e Reflex | `… -Name dlssfg` |
| C3 DLSS FG con PCL | lo stesso gioco di C2 | `… -Name dlssfg-pcl -Pcl -Wpr` |
| C4 FSR 3/4 FG | gioco con FSR FG | `… -Name fsrfg -Wpr` |
| C5 Smooth Motion | DX11/DX12, Smooth Motion attivo nell'app NVIDIA | `… -Name smooth -Wpr` |
| C6 collo di bottiglia | il gioco di C1: qualità massima, poi risoluzione minima | `… -Name gpubound` e `… -Name cpubound` |

Si annotano anche la versione del driver NVIDIA (`nvidia-smi --query-gpu=driver_version --format=csv,noheader`, eseguibile dall'agente) e, per ogni caso, il numero di FPS mostrato dal gioco o dall'overlay di Steam, se c'è.

Si evitano per primi i giochi con EA Javelin. AFMF sull'iGPU e Lossless Scaling restano fuori dallo spike (D12).

- [x] **Step 2 (utente): eseguire le catture**

Per ogni caso:
- gioco in borderless, in una scena di gioco stabile (non un menu);
- avvio del comando dalla tabella;
- 30 s senza uscire dal gioco.

L'utente riporta l'esito stampato dallo script, cioè righe dei CSV e sessioni rimaste.

- [x] **Step 3 (agente): analizzare**

Comando: `python target/spike/m7b/analyze.py <csv> --app <exe> --skip 2`, per ogni CSV (v1 e v2).

Per le ETL:
- `tracerpt <etl> -o <xml> -of XML`, per contare gli eventi PCL per `Marker`, con attenzione a 20, 21 e 22, e gli eventi `PresentFrameType` e `FlipFrameType` di Intel-PresentMon;
- `PresentMon-2.6.0-x64.exe --etl_file <etl> --output_file <etl>.csv --track_frame_type --track_pc_latency --write_frame_id`, per confrontare il risultato con la cattura dal vivo.

Se tracerpt non decodifica il provider PCL (TraceLogging), si aggiunge alla sonda di A4 un comando `--dump-etl <etl>` con `OpenTraceW` e `TdhGetEventInformation`, limitato ai provider PCL e Intel-PresentMon.

Un esito ambiguo, per esempio poche righe del gioco o una scena di caricamento, si ripete una volta sola, chiedendolo all'utente.

- [x] **Step 4 (agente): scrivere le risposte di S1 in `notes.md`**

Le risposte coprono:
1. **Mappa delle colonne:** campo `Frame` del §4.2 → colonna v1 e colonna v2, con la scelta tra v1 e `--v2_metrics`.
2. **Valori di `FrameType` e `PresentMode`** osservati per caso.
3. **DLSS FG:** con PCL, gli id distinti al secondo valgono circa FPS mostrati ÷ 2? Senza PCL c'è un segno qualunque? Da qui la cascata del §4.5, punto 2: vale o no.
4. **FSR FG:** il rapporto di alternanza misurato, da confrontare con la soglia 1,8 del §4.5.
5. **Smooth Motion:** la firma vista, cioè swapchain per PID, un secondo processo, `FrameType`, i marcatori.
6. **Collo di bottiglia:** quota di frame con `busy ≥ 0,9 × ft_app` in C6 `gpubound` e in `cpubound`. Si conferma la soglia del §4.4, o si propone quella che separa i due casi.

### Task A7: S2 e S3, costo e PresentMon come figlio di LocalSystem (utente + agente)

- [x] **Step 1 (utente, PowerShell amministratore, nessun gioco aperto): misure da amministratore**

Ogni comando dura 60 s; `$p` è il percorso assoluto di `target\spike\m7b`:

```powershell
$p = "$PWD\target\spike\m7b"
$probe = "$p\flush-probe\bin\Release\net10.0-windows\flush-probe.exe"
$pm = "$p\bin\PresentMon-2.6.0-x64.exe"
& $probe --presentmon $pm --seconds 60 --flush-ms 0   --out "$p\s2-admin-noflush.log"
& $probe --presentmon $pm --seconds 60 --flush-ms 100 --out "$p\s2-admin-flush.log"
& $probe --presentmon $pm --seconds 60 --flush-ms 100 --gpu --out "$p\s2-admin-gpu.log"
& $probe --presentmon $pm --seconds 60 --flush-ms 100 --pcl --out "$p\s2-admin-pcl.log"
```

- [x] **Step 2 (utente, PowerShell amministratore): la stessa sonda come SYSTEM**

Si usa un'operazione pianificata, senza gioco e poi con il gioco di C1 in primo piano:

```powershell
$tr = "`"$probe`" --presentmon `"$pm`" --seconds 60 --flush-ms 100 --out `"$p\s3-system.log`""
schtasks /Create /TN OMA-M7b-Spike /RU SYSTEM /SC ONCE /ST 23:59 /TR $tr /F
schtasks /Run /TN OMA-M7b-Spike        # attendere 70 s
# con il gioco di C1 aperto: stessa operazione con --out s3-system-game.log
```

- [x] **Step 3 (utente): caduta simulata e pulizia al riavvio**

Si ricrea l'operazione con `--failfast-after 10 --out "$p\s3-crash.log"` e la si esegue. Dopo 20 s:
- `logman query OpenMonitorAdvanced-Frames -ets`: atteso, la sessione esiste ancora;
- `Get-Process PresentMon-2.6.0-x64 -ErrorAction SilentlyContinue`: atteso, nessun processo.

Poi si riesegue la sonda normale (`--out "$p\s3-after-crash.log"`): atteso, il primo `STOP` del log restituisce 0.

- [x] **Step 4 (utente): togliere l'operazione**

Comando: `schtasks /Delete /TN OMA-M7b-Spike /F`.

- [x] **Step 5 (agente): leggere i log e decidere**

Si leggono i log e si annotano in `notes.md`:
- **identità:** `NT AUTHORITY\SYSTEM` e sessione 0 nei log `s3-*`;
- **righe:** arrivano da SYSTEM come da amministratore;
- **ritardo:** p50, p95 e massimo, con e senza svuotamento dei buffer;
- **buffer:** valori dei buffer ETW (risposta al §15 «dimensione dei buffer ETW»: con la console li fissa PresentMon, quindi si annotano e basta), `EventsLost`;
- **costo:** CPU e memoria di PresentMon per ogni variante, senza gioco e con il gioco.

Porte di decisione (spec §4.7):

| Misura | Soglia | Conseguenza |
|---|---|---|
| CPU di PresentMon senza gioco, base | > 1% della CPU totale | ci si ferma e si discute con l'utente la via del servizio PresentMon (D5) prima della Parte B |
| CPU di PresentMon senza gioco, base | > 0,5% (§11) | si annota: il budget della M7c va rivisto |
| ritardo p95 con svuotamento | > 300 ms | si rivede il §4.1, per esempio con un intervallo diverso |
| righe da SYSTEM | nessuna, oppure `StartTrace` negato | ci si ferma: il §4.1 va rivisto con l'utente |
| caduta | sessione non trovata al riavvio, o PresentMon orfano | si corregge il ciclo di vita del §4.1 nella Parte B |

### Task A8: S4, finestra sopra un gioco borderless (utente + agente)

- [x] **Step 1 (utente): preparare**

Si apre il gioco di C1 in borderless, con G-Sync attivo e il suo indicatore acceso (Pannello di controllo NVIDIA › Visualizza › «Indicatore G-SYNC/G-SYNC Compatible»), oppure con l'OSD del monitor che mostra la frequenza.

- [x] **Step 2 (utente): due finestre di PowerShell**

Nella finestra amministratore:

`pwsh -File target\spike\m7b\capture.ps1 -Name overlay -Seconds 90 -NoGpu`

Subito dopo, in una finestra normale (non elevata):

`target\spike\m7b\overlay-probe\target\release\overlay-probe.exe --delay 10 --seconds 90 --hide-at 30 --show-at 60 --log target\spike\m7b\s4.log`

Poi si torna al gioco entro 10 s.

L'utente annota per ogni fase (0–30 visibile, 30–60 nascosta, 60–90 visibile):
- se il riquadro si vede;
- se un clic sul riquadro arriva al gioco;
- l'indicatore G-Sync o la frequenza del monitor.

- [x] **Step 3 (utente): ripetere con la finestra vuota**

Si ripete con `--empty` (finestra visibile ma vuota) e `-Name overlay-empty`.

- [x] **Step 4 (utente, facoltativo): fullscreen esclusivo**

Se il gioco ha il fullscreen esclusivo, si ripete lì una sola fase visibile. Atteso: riquadro non visibile (§14).

- [x] **Step 5 (agente): correlare e decidere**

Si allineano i QPC delle fasi di `s4.log` con `TimeInQPC` di `overlay-v1.csv` e si riportano i conteggi di `PresentMode` del gioco per fase.

| Esito | Decisione |
|---|---|
| visibile → `Composed: Flip`, nascosta → di nuovo `Hardware: Independent Flip` (o `Hardware Composed`) | il design del §3.2 è confermato |
| anche visibile resta `Hardware…` | MPO: si annota, nessun cambio |
| la finestra nascosta **non** riporta il flip indipendente | ci si ferma e si discute con l'utente: il §3.2 e il §14 vanno rivisti prima della M7c |
| la finestra vuota (`--empty`) vale quanto quella nascosta | si annota: «vuota» basterebbe, ma il §3.2 resta «nascosta» |

### Task A9: S5, convivenza delle sessioni (utente + agente)

- [x] **Step 1 (utente, PowerShell amministratore): la sessione di un altro strumento**

Nella stessa finestra di A7, oppure dopo aver ridefinito `$p`, `$probe` e `$pm` come nel suo passo 1:

```powershell
Start-Process $pm -ArgumentList '--session_name OMA-S5-Other --output_file',"$p\s5-other.csv",'--timed 40 --terminate_after_timed --no_console_stats'
Start-Sleep 5
logman query -ets > "$p\s5-before.txt"
& $probe --presentmon $pm --seconds 20 --flush-ms 100 --out "$p\s5.log"
logman query -ets > "$p\s5-after.txt"
```

- [x] **Step 2 (utente, facoltativo): CapFrameX o FrameView**

Se l'utente ha CapFrameX o FrameView, lo avvia in cattura durante il passo 1 e riporta se la cattura si interrompe.

- [x] **Step 3 (agente): controllare**

Atteso:
- `OMA-S5-Other` compare in `s5-before.txt`;
- `s5-other.csv` ha righe lungo tutti i 40 s, senza un buco durante i 20 s della sonda;
- in `s5-after.txt` non c'è `OpenMonitorAdvanced-Frames`.

Il risultato va in `notes.md`.

### Task A10: esito, fixture e decisioni

**Files:**
- Create: `docs/superpowers/references/m7/spike-findings.md`
- Create: `testdata/presentmon/README.md`, `testdata/presentmon/<caso>-v1.csv` e `-v2.csv` per C1–C5 (e C6, se la soglia lo richiede)
- Modify: questo piano (sezione «Decisioni dello spike»)

- [x] **Step 1 (agente): preparare le fixture**

Per ogni caso:

`python target/spike/m7b/analyze.py <csv> --app <exe> --skip 2 --trim-fixture testdata/presentmon/<caso>-v1.csv --seconds 10`

(lo stesso per v2).

Verifica: in ogni fixture `Application` ha un solo valore (il gioco) e `ProcessID` uno solo. Nessun percorso (`grep -c '\\' file` = 0). Ogni file sotto 1 MiB.

`README.md` riporta per ogni file:
- caso;
- gioco e API;
- GPU, driver NVIDIA e versione di PresentMon con lo SHA-256;
- data;
- argomenti;
- secondi tenuti.

- [x] **Step 2 (agente): scrivere l'esito**

`spike-findings.md` ha una sezione per S1–S5 con le misure, le risposte e le porte di decisione del §4.7 con il loro esito. In più:
- gli argomenti definitivi di PresentMon;
- la mappa delle colonne del §4.2;
- la soglia del collo di bottiglia;
- la scelta fra v1 e v2;
- la risposta su PCL con DLSS e su Smooth Motion.

Questi sono i punti del §15 per la M7b.

- [x] **Step 3 (agente): verificare la pulizia**

Comandi, eseguibili senza privilegi; se `logman` li richiede, si chiede all'utente:
- `schtasks /Query /TN OMA-M7b-Spike`: atteso, non trovato;
- `logman query -ets`: nessuna sessione `OMA-Spike-*`, `OMA-S5-Other` o `OpenMonitorAdvanced-Frames`;
- `Get-Process PresentMon* -ErrorAction SilentlyContinue`: nessuno.

- [x] **Step 4 (agente): compilare «Decisioni dello spike»**

Si compila la sezione in fondo a questo piano: ogni correzione alla spec in una riga, nella forma «§x.y: prima → dopo, perché».

Se una porta di decisione ha chiesto di fermarsi (A7 o A8), ci si ferma qui e si discute con l'utente prima di scrivere la Parte B.

- [x] **Step 5 (agente): commit**

```bash
git add docs/superpowers/references/m7/spike-findings.md testdata/presentmon docs/superpowers/plans/2026-10-04-m7b-motore-frame.md
git commit -m "docs: record the M7b spike findings and PresentMon fixtures"
```

- [x] **Step 6 (agente): scrivere la Parte B**

Si scrivono in questo file i passi dei task B1–B11, a partire dalla mappa qui sotto, con le risposte dello spike. Poi si chiede all'utente di rivedere la Parte B prima di eseguirla.

---

# Parte B — Implementazione

Scritta dopo lo spike: valgono le decisioni SD1–SD12 in fondo al piano, che correggono la spec.

## Vincoli aggiuntivi della Parte B

- **TDD e debug:** si scrive prima il test che fallisce (`superpowers:test-driven-development`); davanti a un test che fallisce senza un motivo chiaro si usa `superpowers:systematic-debugging`. Ogni brief per un subagent riporta i comandi `graphify query`, `graphify explain` e `graphify path` per orientarsi; dopo le modifiche al codice si esegue `PYTHONHASHSEED=0 graphify update .`.
- **FFI Rust:** un commento `// SAFETY:` per ogni blocco `unsafe`; le DLL solo da System32; revisione `ffi-safety-reviewer` per B10.
- **P/Invoke .NET:** `[DllImport(..., SetLastError = true)]` con `CharSet.Unicode`, come in `Pipe/PipeNative.cs`; un commento `// SAFETY:` per blocco; la dimensione di ogni struct si verifica con un test (`Marshal.SizeOf`, valori x64), come in `Tests/Setup/NativeMethodsStructLayoutTests.cs`.
- **Protocollo:**
  - mai `skip_serializing_if`: ogni campo si scrive sempre, e un valore assente è `nil`;
  - le fixture si rigenerano solo con `$env:OMA_WRITE_FIXTURES='1'; cargo test -p oma-ipc --test fixtures -- --test-threads=1`, poi si riesegue il test senza la variabile;
  - dopo B3 e B4 si fa la revisione `protocol-parity-reviewer`;
  - i valori enumerati viaggiano come stringhe, come gli altri del protocollo.
- **Divieti per gli agenti:**
  - mai eseguire PresentMon in cattura, nemmeno nei test: i test usano finti processi (`cmd.exe`) e finte sessioni ETW;
  - mai test Pester `Integration`, mai installer;
  - niente loop di test ripetuti né generatori di carico: una suite completa per verifica va bene.
- **Fixture CSV:** si leggono da `testdata/presentmon/` (README con i valori attesi). Il servizio le collega nel progetto di test come `<None Include="..\..\testdata\presentmon\*.csv" Link="Fixtures\PresentMon\%(Filename)%(Extension)" CopyToOutputDirectory="PreserveNewest" />`; i test Rust le leggono da `env!("CARGO_MANIFEST_DIR")/../../testdata/presentmon/`.
- **Nomi fissi:** sessione ETW `OpenMonitorAdvanced-Frames`; eseguibile `PresentMon-2.6.0-x64.exe` in `$INSTDIR\service\presentmon\`; versione riportata `2.6.0`.
- **Esclusioni:** `LOCATIONCHANGE` e la posizione della finestra del gioco passano alla M7c, che li usa (§4.3, ultimo punto). Ruling: in M7b non servono a niente, e senza di loro non c'è un hook che gira per ogni movimento di finestra.

## Review Focus (Parte B)

1. **Alt-tab e schermate di caricamento:** il bersaglio non deve cambiare a ogni perdita di primo piano. Resta per 3 s, poi cade (B11, `target_survives_a_short_alt_tab` e `target_drops_after_three_seconds`).
2. **PresentMon che cade in continuazione:** ritentativi da 1 s fino a 60 s, poi `failed`/`crashing`, senza un ciclo che consuma CPU (B7, `FiveCrashesInTenMinutesFail` e `BackoffDoublesUpToSixtySeconds`).
3. **Servizio riavviato mentre l'app è aperta:** dopo la riconnessione l'app rimanda da sola `FramesConfigure` e `FramesTarget` (B9, `frames_configuration_is_resent_after_reconnect`).
4. **Righe CSV sporche:** BOM, CRLF, `NA`, righe oltre i 4 KiB, campi non numerici, colonne in ordine diverso. Si contano e si scartano senza fermare la lettura (B5, test del parser).
5. **Raffica dopo un buco di 2 s:** a 240 FPS sono circa 480 frame in un colpo. Il lotto si ferma a 512 frame, gli altri vanno in `dropped`, e le metriche dell'app usano il tempo del frame, non quello d'arrivo (B5, `BatchCapsAtFiveHundredTwelveAndCountsDropped`; B1, `window_uses_frame_time_not_arrival`).

---

### Task B1: `oma-core::frames`, tipi, finestra e metriche

**Files:**
- Create: `crates/oma-core/src/frames/mod.rs`, `crates/oma-core/src/frames/metrics.rs`
- Modify: `crates/oma-core/src/lib.rs` (`pub mod frames;`)
- Test: test inline `#[cfg(test)] mod tests` nei due file

**Interfaces:**
- Produces (`oma_core::frames`):
  - `pub enum FrameKind { App, GeneratedIntelXefg, GeneratedAmdAfmf, GeneratedOther, Unknown }` con `pub fn from_wire(s: &str) -> FrameKind` (`"app"`, `"generated_intel_xefg"`, `"generated_amd_afmf"`, `"generated_other"`, ogni altro valore → `Unknown`) e `pub fn is_generated(self) -> bool`.
  - `pub struct FrameSample { pub t_s: f64, pub swapchain: u64, pub kind: FrameKind, pub displayed: bool, pub ms_between_presents: f64, pub ms_between_display_change: Option<f64>, pub ms_until_displayed: Option<f64>, pub ms_app_frametime: Option<f64>, pub ms_pc_latency: Option<f64>, pub ms_gpu_busy: Option<f64>, pub pcl_frame_id: Option<u64> }`, dove `t_s` è l'inizio della presentazione in secondi (QPC ÷ frequenza).
  - `pub struct FrameWindow` con `new(max_age_s: f64)`, `push(&mut self, f: FrameSample)`, `last(&self, seconds: f64) -> Vec<FrameSample>` (frame con `t_s ≥ t_ultimo − seconds`), `clear(&mut self)`. Tiene i frame ordinati per `t_s` (un frame più vecchio dell'ultimo si inserisce al suo posto) e scarta quelli più vecchi di `max_age_s` rispetto all'ultimo.
  - `pub const FPS_WINDOW_S: f64 = 1.0; pub const LOWS_WINDOW_S: f64 = 10.0;`
  - In `metrics.rs`:
    - `pub fn displayed_fps(frames: &[FrameSample]) -> Option<f64>`: `1000·N / Σ ms_between_display_change` sui frame con `displayed` e il valore presente; `None` se N = 0.
    - `pub fn presented_fps(frames: &[FrameSample]) -> Option<f64>`: `1000·N / Σ ms_between_presents`.
    - `pub enum LowDefinition { Integral, Percentile }`.
    - `pub struct Lows { pub one_percent: f64, pub point_one_percent: f64 }`.
    - `pub fn lows(frametimes_ms: &[f64], def: LowDefinition) -> Option<Lows>`: `None` con meno di 2 valori.
    - `pub struct Stutter { pub count: u32, pub time_percent: f64 }`.
    - `pub fn stutter(frames: &[FrameSample]) -> Stutter`.
    - `pub fn mean_pc_latency(frames: &[FrameSample]) -> Option<f64>` e `pub fn mean_display_latency(frames: &[FrameSample]) -> Option<f64>`, medie sui frame mostrati che hanno il valore.
    - `pub enum Bottleneck { Gpu, Cpu, Unknown }`.
    - `pub fn bottleneck(frames: &[FrameSample], fg_suspected: bool) -> Bottleneck`.

**Definizioni da rispettare** (spec §4.4 con SD6):
- **Low integrali:** si ordinano i frametime dal più lungo; si sommano finché la somma raggiunge `p · T`, con `T` = somma di tutti i frametime e `p` = 0,01 o 0,001; il risultato è `1000 / ft` del frame in cui la somma passa la soglia.
- **Low percentili:** `1000 / P(ft)` con il percentile per rango più vicino (`ceil(q·n)`-esimo valore ordinato in modo crescente, `q` = 0,99 o 0,999).
- **Stutter:** frame mostrato con `ft > 2,5 × mediana` **e** `ft − mediana > 8 ms`, dove `ft` = `ms_between_display_change` e la mediana è quella dei frame mostrati nei 2 s che precedono il frame; senza almeno 10 frame nei 2 s precedenti il frame non si valuta. `time_percent` = somma dei frametime degli stutter ÷ `T` × 100.
- **Collo di bottiglia:**
  - con `fg_suspected` → `Unknown`;
  - frame dell'app = quelli con `kind == App` e, se almeno un frame della finestra ha `pcl_frame_id`, solo quelli con `pcl_frame_id`;
  - servono `ms_gpu_busy` e `ms_app_frametime`; con meno di 30 frame validi → `Unknown`;
  - `Gpu` se almeno il 75% ha `ms_gpu_busy ≥ 0,9 × ms_app_frametime`, altrimenti `Cpu`.

- [ ] **Step 1: test che falliscono.** In `metrics.rs`, con frame costruiti da un helper `fn f(t_s: f64, ft: f64) -> FrameSample` (mostrato, `App`, tutti i ms uguali a `ft`):
  - `displayed_fps_is_count_over_total_time`: frametime `[10, 10, 20, 20]` → `1000·4/60` = 66,666…;
  - `displayed_fps_ignores_frames_not_displayed`: un frame con `displayed = false` non conta;
  - `integral_lows_match_a_hand_computed_case`: 1000 frametime, 990 da 10 ms, 9 da 20 ms e uno da 100 ms → `T` = 10180; 1% di `T` = 101,8: 100 non basta, 100 + 20 = 120 sì → `one_percent` = 50,0; 0,1% = 10,18: basta il primo → `point_one_percent` = 10,0;
  - `percentile_lows_use_nearest_rank`: 1000 frametime, 990 da 10 ms e 10 da 40 ms → P99 = valore crescente numero `ceil(0,99·1000)` = 990 = 10 ms → `one_percent` = 100,0; P99,9 = valore numero 999 = 40 ms → `point_one_percent` = 25,0;
  - `stutter_needs_both_conditions`: 200 frame da 10 ms a 100 FPS, poi uno da 30 ms (2,5 × 10 = 25 e 30 − 10 = 20 > 8: stutter) e uno da 24 ms (non stutter); `count` = 1;
  - `stutter_ignores_small_absolute_spikes`: frame da 2 ms, poi uno da 6 ms (3× ma solo 4 ms in più): `count` = 0;
  - `bottleneck_is_gpu_at_seventy_five_percent` (75 su 100 sopra soglia → `Gpu`), `bottleneck_is_cpu_below` (74 → `Cpu`), `bottleneck_unknown_below_thirty_frames`, `bottleneck_unknown_when_fg_is_suspected`, `bottleneck_uses_only_pcl_frames_when_present` (frame senza `pcl_frame_id` con GPU busy alto ignorati).
  - In `mod.rs`: `window_drops_frames_older_than_max_age`, `window_last_returns_the_trailing_seconds`, `window_uses_frame_time_not_arrival` (un frame con `t_s` più vecchio dell'ultimo, inserito dopo, finisce al suo posto e conta in `last`).
- [ ] **Step 2:** `cargo test -p oma-core frames`: atteso FAIL (modulo assente).
- [ ] **Step 3:** implementare i tipi e le funzioni sopra.
- [ ] **Step 4:** `cargo test -p oma-core frames`: atteso PASS. Poi `cargo clippy -p oma-core --all-targets -- -D warnings`.
- [ ] **Step 5: commit** `feat(core): add frame samples, rolling window and frame metrics`.

### Task B2: `oma-core::frames`, frame generation, swapchain, generatore sintetico, fixture

**Files:**
- Create: `crates/oma-core/src/frames/generation.rs`, `crates/oma-core/src/frames/swapchain.rs`, `crates/oma-core/src/frames/synthetic.rs`
- Create: `crates/oma-core/tests/frames_fixtures.rs`
- Modify: `crates/oma-core/src/frames/mod.rs` (moduli e re-export)

**Interfaces:**
- Consumes: B1 (`FrameSample`, `FrameKind`, `displayed_fps`, `bottleneck`, `Bottleneck`).
- Produces:
  - `pub enum RenderedSource { FrameType, Reflex }` e `pub fn source_label(source: RenderedSource, frames: &[FrameSample]) -> &'static str`: `Reflex` → `"Reflex"`; `FrameType` → `"XeSS-FG"` se prevalgono i frame `GeneratedIntelXefg`, `"AFMF"` se prevalgono `GeneratedAmdAfmf`, altrimenti `"FG"`.
  - `pub enum Rendered { Fps { fps: f64, source: RenderedSource }, FgSuspected, Unavailable }`.
  - `pub fn rendered_fps(frames: &[FrameSample]) -> Rendered`, con la cascata del §4.5 e SD4/SD5:
    1. se almeno un frame ha `kind.is_generated()`: `1000 · N_app / T`, con `N_app` = frame mostrati con `kind == App` e `T` = somma dei `ms_between_display_change` dei frame mostrati → `Fps { source: FrameType }`;
    2. altrimenti, se almeno 2 frame hanno `pcl_frame_id`: `(id_ultimo − id_primo) / (t_ultimo − t_primo)` sui soli frame con id → `Fps { source: Reflex }`;
    3. altrimenti `FgSuspected` se `fg_suspected(frames)`, se no `Unavailable`.
  - `pub fn fg_suspected(frames: &[FrameSample]) -> bool`: su finestre consecutive di 2 s (per `t_s`) con almeno 8 frame, vero se la mediana delle finestre ha alternanza ≥ 0,9 **e** rapporto ≥ 1,8. L'alternanza è la quota di coppie consecutive di differenze dei `ms_between_presents` con segno opposto; il rapporto è `mediana(valori sopra la mediana) / mediana(valori fino alla mediana)`.
  - `pub fn fg_multiplier(displayed: Option<f64>, rendered: &Rendered) -> Option<f64>`.
  - `pub fn pick_swapchain(frames: &[FrameSample]) -> Option<u64>`: la swapchain con più frame mostrati nell'ultimo secondo (rispetto all'ultimo `t_s`); a parità, quella del frame più recente.
  - `pub struct SyntheticProfile { pub base_fps: f64, pub fg_factor: u8, pub jitter_ms: f64, pub stutter_every: Option<u32>, pub pcl: bool, pub gpu_busy_ratio: Option<f64> }` e `pub fn synthetic(seed: u64, profile: &SyntheticProfile, duration_s: f64) -> Vec<FrameSample>`.
    - Il generatore è deterministico: un xorshift64* con il seme, nessuna dipendenza nuova.
    - Ogni frame dell'app è seguito da `fg_factor − 1` frame `GeneratedOther` (o, se `pcl`, frame `App` senza `pcl_frame_id`, come DLSS FG nelle fixture), con `ms_between_presents` brevi (0,25 ms) per i generati.
    - `ms_between_display_change` = frametime dell'app ÷ `fg_factor`, più il jitter.
    - Con `stutter_every = Some(n)`, ogni n-esimo frame dell'app dura 4×.
    - `gpu_busy_ratio` imposta `ms_gpu_busy = ratio × ms_app_frametime` sui frame dell'app.

- [ ] **Step 1: test unitari che falliscono.**
  - `generation.rs`: `frame_type_cascade_counts_app_frames` (frame alternati `App`/`GeneratedIntelXefg` → renderizzati = metà dei mostrati, `source == FrameType`); `reflex_rate_uses_the_id_range_not_the_row_count` (id 100, 101, 103, 104 su 0,04 s → 100 FPS anche se le righe con id sono 4); `fg_suspected_fires_on_alternating_presents` (0,25/13 ms ripetuti per 3 s); `fg_suspected_stays_quiet_on_jitter` (valori casuali fra 9 e 11 ms); `no_evidence_gives_unavailable`; `multiplier_needs_both_values`.
  - `swapchain.rs`: `picks_the_swapchain_with_most_displayed_frames`, `tie_goes_to_the_most_recent`.
  - `synthetic.rs`: `same_seed_same_frames`, `different_seed_different_jitter`, `fg_factor_two_doubles_displayed_over_rendered` (60 FPS di base, ×2 → `displayed_fps` fra 118 e 122 e `rendered_fps` `Fps` fra 59 e 61 con `pcl: true`), `stutter_every_shows_up_in_stutter_count`.
- [ ] **Step 2: test delle fixture che falliscono** (`crates/oma-core/tests/frames_fixtures.rs`). Un helper di test legge un CSV di `testdata/presentmon/` per nome di colonna (split su `,`, colonne di SD3, `NA` → `None`, `PCLFrameId` 0 → `None`, `FrameType` mappato come in SD3, `t_s = TimeInQPC / 1e7`) e restituisce `Vec<FrameSample>` su tutti i 10 s. Valori attesi (dal README delle fixture):
  - `displayed_fps` entro ±0,5: `nofg` 74,2; `nofg-pcl` 97,0; `cpubound` 156,4; `dlssfg` 130,1; `dlssfg-pcl` 130,5; `fsrfg` 125,8; `fsrfg-pcl` 106,3; `smooth` 157,0; `smooth-pcl` 157,0;
  - `rendered_fps` = `Fps { source: Reflex }` entro ±1,0: `nofg-pcl` 96,9; `dlssfg-pcl` 65,3; `fsrfg-pcl` 53,1; `smooth-pcl` 78,5;
  - `FgSuspected` per `dlssfg` e `smooth`; `Unavailable` per `nofg`, `cpubound`, `fsrfg`;
  - `fg_multiplier` fra 1,9 e 2,1 per `dlssfg-pcl`, `fsrfg-pcl`, `smooth-pcl`;
  - `bottleneck(…, false)`: `Gpu` per `nofg` e `nofg-pcl`, `Cpu` per `cpubound`; `bottleneck(…, true)` → `Unknown` per `dlssfg`;
  - `pick_swapchain` = l'unico indirizzo del file.
- [ ] **Step 3:** `cargo test -p oma-core frames` e `cargo test -p oma-core --test frames_fixtures`: atteso FAIL.
- [ ] **Step 4:** implementare.
- [ ] **Step 5:** stessi comandi, atteso PASS; `cargo clippy -p oma-core --all-targets -- -D warnings`.
- [ ] **Step 6: commit** `feat(core): add frame generation cascade, swapchain choice and synthetic frames`.

### Task B3: protocollo v4 in `oma-ipc`

**Files:**
- Modify: `crates/oma-ipc/src/lib.rs`, `crates/oma-ipc/src/message.rs`, `crates/oma-ipc/src/frame.rs`, `crates/oma-ipc/tests/fixtures.rs`, `protocol/fixtures/README.md`
- Create: `protocol/fixtures/frames_configure.msgpack`, `frames_target.msgpack`, `frames_target_none.msgpack`, `frames_status.msgpack`, `presenting_processes.msgpack`, `frame_batch.msgpack` (generati)

**Interfaces:**
- Produces (in `message.rs`, stessi derive e stile delle struct esistenti):
  - `pub struct FramesConfigure { pub enabled: bool, pub track_pc_latency: bool, pub track_gpu: bool }`
  - `pub struct FramesTarget { pub pid: Option<u32> }`
  - `pub struct FramesStatus { pub state: String, pub detail: Option<String>, pub presentmon_version: Option<String> }`
  - `pub struct PresentingProcess { pub pid: u32, pub name: String, pub displayed_fps: f64, pub present_mode: String, pub swapchains: u32 }`
  - `pub struct PresentingProcesses { pub at_qpc: u64, pub processes: Vec<PresentingProcess> }`
  - `pub struct WireFrame { pub qpc: u64, pub swapchain: u64, pub frame_type: String, pub displayed: bool, pub ms_between_presents: f64, pub ms_between_display_change: Option<f64>, pub ms_until_displayed: Option<f64>, pub ms_app_frametime: Option<f64>, pub ms_pc_latency: Option<f64>, pub ms_gpu_busy: Option<f64>, pub pcl_frame_id: Option<u64> }`
  - `pub struct FrameBatch { pub pid: u32, pub frames: Vec<WireFrame>, pub dropped: u32 }`
  - nuove varianti di `Message`, in coda: `FramesConfigure(FramesConfigure)`, `FramesTarget(FramesTarget)`, `FramesStatus(FramesStatus)`, `PresentingProcesses(PresentingProcesses)`, `FrameBatch(FrameBatch)` (tag `frames_configure`, `frames_target`, `frames_status`, `presenting_processes`, `frame_batch`).
  - In `lib.rs`: `PROTOCOL_VERSION = 4`; `pub const MAX_FRAMES_PER_BATCH: usize = 512; pub const MAX_PRESENTING_PROCESSES: usize = 32;` e le costanti di stato `pub mod frames_state { pub const OFF: &str = "off"; STARTING "starting"; RUNNING "running"; DENIED "denied"; TAMPERED "tampered"; MISSING "missing"; FAILED "failed"; }`.
- **Controlli in `decode_payload`**, accanto a quelli dello snapshot:
  - `FrameBatch` con più di 512 frame, o `PresentingProcesses` con più di 32 voci → `IpcError::Decode`;
  - un `f64` facoltativo non finito diventa `None`;
  - un `f64` obbligatorio non finito (`ms_between_presents`, `displayed_fps`) → `IpcError::Decode`.

**Contenuto delle fixture:**
- `frames_configure`: `{enabled: true, track_pc_latency: true, track_gpu: false}`.
- `frames_target`: `{pid: 25848}`; `frames_target_none`: `{pid: nil}`.
- `frames_status`: `{state: "running", detail: nil, presentmon_version: "2.6.0"}`.
- `presenting_processes`: `at_qpc` 380058775270 e due voci:
  - `{25848, "CONTROLResonant.exe", 61.5, "Hardware Composed: Independent Flip", 1}`;
  - `{1852, "dwm.exe", 20.0, "Hardware: Legacy Flip", 1}`.
- `frame_batch`: `pid` 25848, `dropped` 3 e due frame tratti dalle prime due righe di `testdata/presentmon/dlssfg-pcl.csv` (uno con `pcl_frame_id` nil, uno con id), con `frame_type` `"app"` e `ms_gpu_busy` presente.

- [ ] **Step 1: test che falliscono.**
  - In `tests/fixtures.rs`: aggiungere i nomi a `NAMES` e i messaggi di riferimento; aggiornare `protocol_constants_are_v3` in `protocol_constants_are_v4` (`PROTOCOL_VERSION == 4`, `MAX_FRAMES_PER_BATCH == 512`, `MAX_PRESENTING_PROCESSES == 32`); `frames_target_none_keeps_the_pid_key` (il payload contiene la chiave `pid` con `nil`).
  - In `frame.rs`: `a_batch_over_512_frames_is_rejected`, `thirty_three_processes_are_rejected`, `non_finite_optional_frame_values_become_none`, `non_finite_required_frame_values_are_rejected`.
- [ ] **Step 2:** `cargo test -p oma-ipc`: atteso FAIL.
- [ ] **Step 3:** implementare; rigenerare le fixture (comando nei vincoli), poi `cargo test -p oma-ipc` senza variabile.
- [ ] **Step 4:** aggiornare `protocol/fixtures/README.md`: contenuto logico v4, tabella delle dimensioni.
- [ ] **Step 5:** `cargo test --workspace` (il resto del workspace compila con le varianti nuove: il ramo `other =>` della macchina del collegamento le tratta ancora come inattese, ci pensa B9); `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 6: commit** `feat(ipc): add protocol v4 frame messages`.

### Task B4: protocollo v4 nel servizio

**Files:**
- Modify: `service/OpenMonitorAdvanced.Service/Protocol/Messages.cs`, `ProtocolConstants.cs`, `MessageCodec.cs`
- Modify: `service/OpenMonitorAdvanced.Service.Tests/Protocol/CodecTests.cs`

**Interfaces:**
- Consumes: B3 (fixture e forma dei messaggi).
- Produces (record in `Messages.cs`):
  - `FramesConfigureMessage(bool Enabled, bool TrackPcLatency, bool TrackGpu)`;
  - `FramesTargetMessage(uint? Pid)`;
  - `FramesStatusMessage(string State, string? Detail, string? PresentMonVersion)`;
  - `PresentingProcess(uint Pid, string Name, double DisplayedFps, string PresentMode, uint Swapchains)` e `PresentingProcessesMessage(ulong AtQpc, IReadOnlyList<PresentingProcess> Processes)`;
  - `WireFrame(ulong Qpc, ulong Swapchain, string FrameType, bool Displayed, double MsBetweenPresents, double? MsBetweenDisplayChange, double? MsUntilDisplayed, double? MsAppFrametime, double? MsPcLatency, double? MsGpuBusy, ulong? PclFrameId)`;
  - `FrameBatchMessage(uint Pid, IReadOnlyList<WireFrame> Frames, uint Dropped)`.
  - In `ProtocolConstants.cs`: `Version = 4`, `MaxFramesPerBatch = 512`, `MaxPresentingProcesses = 32`, e una classe `FramesStates` con le sette stringhe di B3.
- Il codec scrive le chiavi nell'ordine delle struct Rust (`to_vec_named` scrive in ordine di dichiarazione), con gli stessi limiti di decodifica di B3.

- [ ] **Step 1: test che falliscono.** Aggiungere i sei nomi a `FixtureNames` (le teorie esistenti `FixtureIsEncodedByteForByte` e `FixtureDecodesToTheReference` li coprono), i messaggi di riferimento identici a quelli di B3, e `ABatchOverFiveHundredTwelveFramesIsRejected`, `ThirtyThreeProcessesAreRejected`, `ProtocolVersionIsFour`.
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx`: atteso FAIL.
- [ ] **Step 3:** implementare scrittura e lettura.
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx`: atteso PASS; `pwsh scripts/check-trim-warnings.ps1` invariato.
- [ ] **Step 5: commit** `feat(service): add protocol v4 frame messages`. Poi la revisione `protocol-parity-reviewer` su B3 e B4 insieme.

### Task B5: parser CSV e aggregatore del servizio

**Files:**
- Create: `service/OpenMonitorAdvanced.Service/Frames/PresentMonCsv.cs`, `service/OpenMonitorAdvanced.Service/Frames/FrameAggregator.cs`
- Create: `service/OpenMonitorAdvanced.Service.Tests/Frames/PresentMonCsvTests.cs`, `FrameAggregatorTests.cs`
- Modify: `service/OpenMonitorAdvanced.Service.Tests/OpenMonitorAdvanced.Service.Tests.csproj` (collegamento delle fixture CSV, vedi vincoli)

**Interfaces:**
- Produces (namespace `OpenMonitorAdvanced.Service.Frames`):
  - `internal sealed record PresentMonRow(uint Pid, string Name, string PresentMode, WireFrame Frame)`.
  - `internal sealed class PresentMonCsv`:
    - `bool TryReadHeader(string line, out string? missingColumn)`: toglie BOM e `\r`, mappa le colonne per nome; se manca una colonna obbligatoria di SD2, restituisce `false` con il nome;
    - `PresentMonRow? ParseRow(string line)`: `null` per una riga scartata, contata in `long Rejected`;
    - una riga si scarta se è più lunga di 4096 caratteri, se ha un numero di campi diverso dall'intestazione, o se un campo obbligatorio non è numerico;
    - `NA` o un valore vuoto in un campo facoltativo diventa `null`;
    - `SwapChainAddress` si legge come esadecimale con `0x`;
    - la mappa dei campi è quella di SD3.
  - `internal sealed class FrameAggregator(int maxFramesPerBatch = 512, int maxProcesses = 32)`:
    - `void Add(PresentMonRow row, long arrivalTicks)`;
    - `void SetTargets(IReadOnlyCollection<uint> pids)`: solo le righe di questi PID diventano frame;
    - `FrameBatchMessage? TakeBatch(uint pid)`: `null` se non ci sono frame nuovi; oltre 512 frame si tengono i primi 512 e il resto va in `Dropped`, che torna a 0 dopo la consegna;
    - `PresentingProcessesMessage TakeSummary(ulong atQpc)`: per ogni PID con almeno un frame mostrato nell'ultimo secondo (secondo il `TimeInQPC` dell'ultima riga vista):
      - FPS mostrati con la formula di B1;
      - `PresentMode` più frequente;
      - numero di swapchain distinte;
      - ordinati per FPS mostrati decrescenti, al massimo 32 voci.
    - `int Stalls { get; }`: arrivi distanti più di 1 s mentre arrivano righe (SD7).

- [ ] **Step 1: test che falliscono** (`PresentMonCsvTests`):
  - `ReadsColumnsByNameInAnyOrder` (intestazione permutata);
  - `ReportsTheMissingRequiredColumn` (senza `MsBetweenDisplayChange` → `false` e il nome);
  - `ToleratesBomAndCarriageReturn`;
  - `NaBecomesNullInOptionalFields`;
  - `RejectsLongLinesAndNonNumericValues` (`Rejected` == 2);
  - `ZeroPclFrameIdIsNull`;
  - `ParsesEveryFixtureWithoutRejects` (teoria sui nove file di `Fixtures\PresentMon`, righe == righe del README, `Rejected` == 0).
- [ ] **Step 2: test che falliscono** (`FrameAggregatorTests`):
  - `OnlyTargetRowsBecomeFrames`;
  - `BatchCapsAtFiveHundredTwelveAndCountsDropped` (600 righe → 512 frame, `Dropped` 88; il lotto dopo ha `Dropped` 0);
  - `SummaryKeepsProcessesSeenInTheLastSecond`;
  - `SummaryIsCappedAtThirtyTwoSortedByFps`;
  - `SummaryUsesTheMostFrequentPresentMode`;
  - `SummaryMatchesTheFixtureFps` (`nofg.csv` → 74,2 ± 0,5 sull'ultimo secondo, calcolato a parte nel test);
  - `CountsArrivalStallsOverOneSecond`.
- [ ] **Step 3:** `dotnet test service/OpenMonitorAdvanced.slnx --filter "FullyQualifiedName~Frames"`: atteso FAIL.
- [ ] **Step 4:** implementare.
- [ ] **Step 5:** stesso comando, atteso PASS.
- [ ] **Step 6: commit** `feat(service): parse PresentMon CSV and aggregate frames per process`.

### Task B6: PresentMon come processo figlio e controllo della sessione ETW

**Files:**
- Create: `service/OpenMonitorAdvanced.Service/Frames/FramesNative.cs`, `PresentMonProcess.cs`, `EtwSessionControl.cs`, `PresentMonPin.cs`
- Create: `service/OpenMonitorAdvanced.Service.Tests/Frames/FramesNativeLayoutTests.cs`, `PresentMonProcessTests.cs`, `PresentMonPinTests.cs`, `EtwSessionControlTests.cs`
- Modify: il csproj dei test (collega `app\src-tauri\nsis\presentmon.sha256` come `Fixtures\presentmon.sha256`)
- Create: `app/src-tauri/nsis/presentmon.sha256` (una riga: `B2A706BC6AD475749E3B7E3409263AA1E6906D45BDCF993F6DBC0F660188F1AF`)

**Interfaces:**
- Produces:
  - **Interfacce per i test:**
    - `internal interface IFrameSource { IPresentMonRun Start(IReadOnlyList<string> arguments); }`;
    - `internal interface IPresentMonRun : IDisposable { ChannelReader<string> StdoutLines { get; } Task<PresentMonExit> Exited { get; } }`, con `internal sealed record PresentMonExit(int ExitCode, IReadOnlyList<string> StderrTail)`, che tiene le ultime 20 righe, ciascuna troncata a 512 caratteri.
  - **`PresentMonProcess : IFrameSource`:**
    - `Start` avvia l'eseguibile con `UseShellExecute = false`, standard output e standard error rediretti, `CreateNoWindow = true`;
    - lo assegna subito a un Job Object con `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`;
    - legge le righe in un `Channel` limitato a 8192 righe (`BoundedChannelFullMode.DropOldest`);
    - `Dispose` chiude il Job Object (e quindi il processo) e attende l'uscita al massimo 5 s.
    - Il costruttore riceve il percorso dell'eseguibile.
  - **`internal interface IEtwSession { uint Stop(string name); uint Flush(string name); }`** con `EtwSessionControl : IEtwSession` (`ControlTraceW` con `EVENT_TRACE_CONTROL_STOP` = 1 e `EVENT_TRACE_CONTROL_FLUSH` = 3, buffer delle proprietà = struct + 2 × 1024 caratteri, come nella sonda dello spike).
  - **`FramesNative`:** `EVENT_TRACE_PROPERTIES` (120 byte x64), `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` (144 byte), `CreateJobObjectW`, `SetInformationJobObject`, `AssignProcessToJobObject`, `ControlTraceW`; il Job Object si avvolge in un `SafeHandle`.
  - **`internal static class PresentMonPin`:**
    - `public const string Sha256`, uguale a `presentmon.sha256`;
    - `public const string FileName = "PresentMon-2.6.0-x64.exe"`;
    - `public const string Version = "2.6.0"`;
    - `static string? HashOf(string path)`: `null` se il file manca.

- [ ] **Step 1: test che falliscono.**
  - `FramesNativeLayoutTests`: `EventTracePropertiesIsOneHundredTwentyBytes`, `JobExtendedLimitInformationIsOneHundredFortyFourBytes`.
  - `PresentMonPinTests`: `PinMatchesTheInstallerFile` (la costante è uguale alla riga del file collegato).
  - `PresentMonProcessTests` (con `cmd.exe` di System32 al posto di PresentMon):
    - `LinesFromStdoutArrive` (`/c echo a& echo b`);
    - `ExitCodeAndStderrAreReported` (`/c echo boom 1>&2& exit 6` → `ExitCode` 6, `StderrTail` contiene `boom`);
    - `DisposeKillsTheChildThroughTheJob` (`/c ping -n 30 127.0.0.1 >nul`, poi `Dispose`: il processo esce entro 5 s).
  - `EtwSessionControlTests`: `StoppingAMissingSessionReportsNotFound` (`Stop("OpenMonitorAdvanced-Test-Missing")` restituisce 4201: verificato senza privilegi nello spike).
- [ ] **Step 2:** `dotnet test … --filter "FullyQualifiedName~Frames"`: atteso FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** atteso PASS; `pwsh scripts/check-trim-warnings.ps1` senza avvisi nuovi.
- [ ] **Step 5: commit** `feat(service): run PresentMon in a kill-on-close job and control the ETW session`. Poi la revisione `ffi-safety-reviewer` sui file `Frames/*Native*` e `PresentMonProcess.cs`.

### Task B7: `FrameCapture`, il ciclo di vita

**Files:**
- Create: `service/OpenMonitorAdvanced.Service/Frames/FrameCapture.cs`
- Create: `service/OpenMonitorAdvanced.Service.Tests/Frames/FrameCaptureTests.cs`, `FramesFakes.cs`

**Interfaces:**
- Consumes: B5 (`PresentMonCsv`, `FrameAggregator`), B6 (`IFrameSource`, `IPresentMonRun`, `IEtwSession`, `PresentMonPin`).
- Produces:
  - `internal sealed record FramesOptions(bool TrackPcLatency, bool TrackGpu)`.
  - `internal sealed class FrameCapture : IDisposable`. Costruttore: `(IFrameSource source, IEtwSession etw, Func<string?> currentHash, TimeProvider time, ILogger<FrameCapture> log)`.
    - `currentHash` restituisce lo SHA-256 dell'eseguibile, o `null` se il file non c'è.
    - **Pulizia iniziale:** alla costruzione chiama `etw.Stop("OpenMonitorAdvanced-Frames")`, per una sessione rimasta da una caduta.
  - `void Configure(FramesOptions? options)`:
    - `null` = spento;
    - con le stesse opzioni non riavvia;
    - con opzioni diverse riavvia;
    - dopo `denied`, `tampered`, `missing` o `failed` riprova.
  - `event Action<FramesStatusMessage> StatusChanged` (si invia a ogni cambio) e `FramesStatusMessage Status { get; }`.
  - `event Action<PresentMonRow, long> RowParsed`: per l'aggregatore, sul thread di lettura.
  - **Argomenti:** quelli di SD1, costruiti da `internal static IReadOnlyList<string> Arguments(FramesOptions o)`.
- **Ciclo di vita** (`state` con le costanti di B3):
  1. **Controllo dell'eseguibile:** se il file manca → `missing`; se l'hash è diverso → `tampered`, `detail` nil. In entrambi i casi non si avvia.
  2. **Avvio:** `starting`; avvio di PresentMon e lettura dell'intestazione. Con l'intestazione valida → `running`, e un timer chiama `etw.Flush` ogni 100 ms.
  3. **Intestazione senza una colonna obbligatoria:** `failed`, `detail` = `"columns"`; PresentMon si chiude e non si riprova.
  4. **Uscita prima dell'intestazione** con `StderrTail` che contiene `access denied` → `denied`, senza nuovi tentativi.
  5. **Ogni altra uscita è una caduta:**
     - `etw.Stop`, poi un nuovo tentativo dopo 1, 2, 4, 8… s, al massimo 60 s; lo stato resta `starting`;
     - una corsa in `running` per più di 60 s riporta l'attesa a 1 s;
     - alla quinta caduta in 10 minuti → `failed`, `detail` = `"crashing"`, fino al prossimo `Configure`.
  6. **Spegnimento** (`Configure(null)` o `Dispose`): `Dispose` della corsa (cioè del Job Object), poi `etw.Stop`, poi `off`.

  `presentmon_version` vale `"2.6.0"` in `starting` e `running`, nil negli altri stati.

- [ ] **Step 1: test che falliscono** (finti in `FramesFakes.cs`: `FakeFrameSource`, che registra gli argomenti e restituisce un `FakeRun` con un `Channel` e un `TaskCompletionSource<PresentMonExit>` pilotati dal test; `FakeEtw`, che conta `Stop` e `Flush`; e `FakeTimeProvider`):
  - `StopsALeftoverSessionAtConstruction`;
  - `ArgumentsMatchTheSpikeDecision` (senza opzioni, con PCL, con GPU: confronto con l'elenco esatto di SD1);
  - `MissingExecutableReportsMissing`;
  - `WrongHashReportsTamperedAndNeverStarts`;
  - `ValidHeaderMeansRunningAndFlushesEveryHundredMilliseconds` (`Advance(1 s)` → 10 flush);
  - `MissingColumnFailsWithColumns`;
  - `AccessDeniedBeforeHeaderMeansDenied` (stderr `error: failed to start trace session: access denied.`, codice 6);
  - `CrashRestartsWithBackoff` e `BackoffDoublesUpToSixtySeconds`;
  - `FiveCrashesInTenMinutesFail`;
  - `SameOptionsDoNotRestart` e `DifferentOptionsRestart`;
  - `ConfigureNullStopsTheJobAndTheSession`;
  - `RowsAreParsedAndRaised`.
- [ ] **Step 2:** `dotnet test … --filter "FullyQualifiedName~FrameCapture"`: atteso FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** atteso PASS.
- [ ] **Step 5: commit** `feat(service): add the PresentMon capture lifecycle`.

### Task B8: `FramesHub`, richieste dei client e collegamento alla pipe

**Files:**
- Create: `service/OpenMonitorAdvanced.Service/Frames/FrameRequests.cs`, `FramesHub.cs`
- Modify: `service/OpenMonitorAdvanced.Service/Pipe/ClientSession.cs`, `Pipe/PipeListener.cs`, `ServiceHost.cs`
- Create: `service/OpenMonitorAdvanced.Service.Tests/Frames/FrameRequestsTests.cs`, `FramesHubTests.cs`
- Modify: `service/OpenMonitorAdvanced.Service.Tests/Pipe/` (test di sessione con il listener esistente, `ListenerHarness`)

**Interfaces:**
- Consumes: B4, B5, B7.
- Produces:
  - **`internal sealed class FrameRequests(TimeProvider time)`.** Il servizio accetta fino a 8 client (`MaxClients`); le richieste si combinano, invece di valere «un client solo» come diceva la spec.
    - `void Configure(int session, FramesConfigureMessage m)` e `void Target(int session, uint? pid)`;
    - `void Disconnected(int session)`: la richiesta resta valida per 30 s (§4.1), poi scade;
    - `FramesOptions? Effective`: `null` se nessuna sessione valida ha `Enabled`; altrimenti PCL e GPU in OR fra le sessioni attive;
    - `IReadOnlyCollection<uint> Targets`;
    - `TimeSpan? NextExpiry`.
    - Un PID 0 o 4 non è valido: vale come `nil`, con un avviso nel log.
  - **`internal sealed class FramesHub : IDisposable`** (singleton in DI). Costruttore: `(FrameCapture capture, FrameAggregator aggregator, FrameRequests requests, TimeProvider time, ILogger<FramesHub> log)`.
    - Ogni 100 ms consegna a ogni sessione iscritta il `FrameBatchMessage` del suo bersaglio;
    - ogni secondo consegna a tutte le sessioni iscritte il `PresentingProcessesMessage`;
    - a ogni cambio di stato consegna il `FramesStatusMessage`.
    - Una sessione si iscrive con il primo `FramesConfigure` ricevuto: `IDisposable Subscribe(int session, Func<IMessage, bool> deliver)`, dove `deliver` restituisce `false` quando la coda della sessione è piena.
    - `void OnConfigure(int session, FramesConfigureMessage m)`, `void OnTarget(int session, FramesTargetMessage m)` e `void OnDisconnected(int session)` aggiornano le richieste e chiamano `capture.Configure(requests.Effective)` e `aggregator.SetTargets(requests.Targets)`.
    - Un `FrameBatchMessage` non consegnato si somma al `Dropped` del lotto dopo; un riepilogo non consegnato si perde.
    - I buchi oltre 1 s (`aggregator.Stalls`) si scrivono nel log a livello Debug, al massimo una riga al minuto.
  - **`ClientSession`:** `ReadLoopAsync`, dopo il `Subscribe`, accetta anche `FramesConfigureMessage` e `FramesTargetMessage` e li passa al hub; ogni altro messaggio dà ancora `QueueError`.
    - Le consegne del hub entrano nella stessa coda `Outgoing`, al massimo 4 messaggi dei frame in attesa per sessione (oltre → `deliver` restituisce `false`).
    - La regola esistente `MaxQueuedUpdates = 2` per gli aggiornamenti dei sensori non cambia.
    - Alla fine della sessione si chiama `OnDisconnected`.
  - **DI** (`ServiceHost.Build`): `FrameCapture` con `PresentMonProcess(Path.Combine(ServiceDirectory, "presentmon", PresentMonPin.FileName))`, `EtwSessionControl`, `() => PresentMonPin.HashOf(…)` e `TimeProvider.System`; poi `FrameAggregator`, `FrameRequests`, `FramesHub`. Il hub si chiude insieme al feed, nell'hosted service di chiusura esistente.

- [ ] **Step 1: test che falliscono.**
  - `FrameRequestsTests`:
    - `NoSessionMeansOff`;
    - `OptionsAreOredAcrossSessions`;
    - `DisconnectKeepsTheRequestForThirtySeconds`;
    - `ReconnectWithinGraceKeepsCaptureRunning`;
    - `PidZeroAndFourAreIgnored`;
    - `TargetsAreTheDistinctValidPids`.
  - `FramesHubTests` (con `FrameCapture` su finti di B7):
    - `BatchesAreDeliveredEveryHundredMilliseconds`;
    - `EachSessionGetsOnlyItsTarget`;
    - `SummaryGoesToEverySubscriber`;
    - `UndeliveredBatchAddsToDropped`;
    - `StatusChangesAreBroadcast`.
  - Test della sessione sulla pipe:
    - `FramesConfigureAfterSubscribeIsAccepted`;
    - `FramesConfigureBeforeSubscribeGetsAnError`.
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx`: atteso FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx` tutto verde; `pwsh scripts/check-trim-warnings.ps1`.
- [ ] **Step 5: commit** `feat(service): serve frame data to subscribed clients`.

### Task B9: il collegamento dell'app parla v4 (`oma-win::svc`)

**Files:**
- Modify: `crates/oma-win/src/svc/link/machine.rs` (+ `machine_tests.rs`), `crates/oma-win/src/svc/link/mod.rs`, `crates/oma-win/src/svc/link/transport.rs`, `crates/oma-win/src/svc/mod.rs`
- Create: `crates/oma-win/src/svc/frames_feed.rs`

**Interfaces:**
- Consumes: B3.
- Produces:
  - `LinkCommand::ConfigureFrames(oma_ipc::FramesConfigure)` e `LinkCommand::SetFramesTarget(Option<u32>)`.
  - **`Machine`:**
    - ricorda l'ultima configurazione e l'ultimo bersaglio;
    - quando il collegamento arriva in streaming (dopo il primo `Snapshot`), invia `FramesConfigure` se `enabled`, poi `FramesTarget`;
    - in streaming, un comando che cambia qualcosa si invia subito; uno uguale all'ultimo inviato non si invia;
    - dopo una riconnessione si invia di nuovo tutto;
    - in `on_message`, i rami `FramesStatus`, `PresentingProcesses` e `FrameBatch` producono `Effect::Frames(FramesEvent)` invece di chiudere il collegamento.
  - `pub enum FramesEvent { Status(FramesStatus), Processes(PresentingProcesses), Batch(FrameBatch), Disconnected }`, con `Disconnected` emesso quando il collegamento perde il servizio.
  - **`pub struct FramesFeed`** (clonabile, `Arc<Mutex<…>>`, come `SvcFeed`):
    - `fn apply(&self, e: FramesEvent)`;
    - `pub fn drain(&self) -> FramesUpdate`, con `pub struct FramesUpdate { pub status: Option<FramesStatus>, pub processes: Option<PresentingProcesses>, pub batches: Vec<FrameBatch>, pub connected: bool }`;
    - tiene al massimo 64 lotti (6,4 s); oltre, scarta i più vecchi;
    - `Disconnected` azzera lo stato.
  - **`ServiceLink::spawn`** riceve anche un `FramesFeed`, e `Driver::apply` gli passa gli eventi.
  - **`LinkSink::message`:** con la coda piena scarta `FrameBatch` e `PresentingProcesses`, come lo `Snapshot`; `FramesStatus` aspetta come gli altri messaggi.
  - **`pub fn qpc_frequency() -> u64`** in `crates/oma-win/src/` (nuovo `qpc.rs`, `QueryPerformanceFrequency`, feature `Win32_System_Performance` se non c'è già).

- [ ] **Step 1: test che falliscono** (`machine_tests.rs`):
  - `frames_configuration_is_sent_after_the_first_snapshot`;
  - `frames_target_follows_configuration`;
  - `same_frames_command_is_not_resent`;
  - `frames_configuration_is_resent_after_reconnect`;
  - `frame_messages_become_frames_effects_not_a_disconnect`;
  - `disabled_frames_send_nothing_at_start`.

  Test di `frames_feed.rs`: `drain_returns_and_clears_batches`, `feed_keeps_at_most_sixty_four_batches`, `disconnect_clears_status`.

  Test di `transport.rs`: `full_queue_drops_frame_batches`.
- [ ] **Step 2:** `cargo test -p oma-win svc`: atteso FAIL.
- [ ] **Step 3:** implementare; aggiornare il chiamante in `app/src-tauri/src/service.rs` (`spawn_link` crea e conserva un `FramesFeed`).
- [ ] **Step 4:** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 5: commit** `feat(win): carry protocol v4 frame messages over the service link`.

### Task B10: finestra in primo piano (`oma-win::foreground`)

**Files:**
- Create: `crates/oma-win/src/foreground.rs`
- Modify: `crates/oma-win/src/lib.rs`, `crates/oma-win/Cargo.toml` (feature `Win32_UI_Accessibility`)

**Interfaces:**
- Produces:
  - `pub struct ForegroundWatcher` con `pub fn spawn(sink: Box<dyn Fn(u32) + Send>) -> std::io::Result<ForegroundWatcher>`.
    - Il thread `oma-foreground` installa `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, callback, 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)` e fa girare un ciclo `GetMessageW`.
    - Il sink riceve il PID del primo piano, risolto con `foreground_pid`.
    - All'avvio si invia subito il primo piano corrente (`GetForegroundWindow`).
    - `Drop` posta `WM_QUIT` al thread (`PostThreadMessageW`), lo attende e chiama `UnhookWinEvent`.
  - **Funzioni pure, testabili:**
    - `pub(crate) fn foreground_pid(window_pid: u32, window_class: &str, core_window_pid: Option<u32>) -> u32`: se la classe è `ApplicationFrameWindow` e c'è una `Windows.UI.Core.CoreWindow` figlia con un PID, si usa quello (giochi UWP); altrimenti il PID della finestra.
    - Il codice FFI chiama `GetClassNameW`, `FindWindowExW(hwnd, None, "Windows.UI.Core.CoreWindow", None)` e `GetWindowThreadProcessId`. Nessun handle verso processi (§10).
  - **Callback:** non chiama il sink dentro l'hook se non con dati già copiati, e non fa operazioni bloccanti.

- [ ] **Step 1: test che falliscono:**
  - `uwp_frame_host_resolves_to_the_core_window_pid`;
  - `a_normal_window_keeps_its_pid`;
  - `frame_host_without_core_window_keeps_its_pid`;
  - test hardware `#[ignore = "requires real Windows hardware"]` `watcher_reports_the_current_foreground_at_start`.
- [ ] **Step 2:** `cargo test -p oma-win foreground`: atteso FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** atteso PASS; `cargo clippy -p oma-win --all-targets -- -D warnings`.
- [ ] **Step 5: commit** `feat(win): watch the foreground window with a WinEvent hook`. Poi la revisione `ffi-safety-reviewer`.

### Task B11: app, scelta del bersaglio e diagnostica dei frame

**Files:**
- Create: `app/src-tauri/src/overlay/mod.rs`, `app/src-tauri/src/overlay/target.rs`, `app/src-tauri/src/overlay/frames.rs`
- Modify: `app/src-tauri/src/main.rs` (`mod overlay;`, avvio in `.setup()`), `app/src-tauri/src/service.rs` (accesso a `FramesFeed` e invio dei comandi)

**Interfaces:**
- Consumes: B1, B2 (`oma_core::frames`), B9 (`FramesFeed`, `LinkCommand`, `qpc_frequency`), B10 (`ForegroundWatcher`).
- Produces:
  - **`overlay::target`** (puro):
    - `pub struct ProcessInfo { pub pid: u32, pub name: String, pub displayed_fps: f64 }`;
    - `pub struct TargetPicker` con `new(own_pids: Vec<u32>, excluded_names: Vec<String>)`, `on_foreground(&mut self, pid: u32, now_ms: u64)`, `on_processes(&mut self, procs: &[ProcessInfo], now_ms: u64)`, `current(&self) -> Option<&ProcessInfo>` e `tick(&mut self, now_ms: u64) -> Option<Option<u32>>` (`Some` solo quando il bersaglio cambia);
    - `pub const MIN_GAME_FPS: f64 = 10.0; pub const TARGET_GRACE_MS: u64 = 3_000;`;
    - `pub const SYSTEM_EXCLUDED: &[&str]`: `dwm.exe`, `explorer.exe`, `applicationframehost.exe`, `shellexperiencehost.exe`, `startmenuexperiencehost.exe`, `searchhost.exe`, `textinputhost.exe`, `lockapp.exe`, `csrss.exe`, `msedgewebview2.exe`, `oma-app.exe`, `oma-overlay.exe`, `oma-service.exe`, `presentmon-2.6.0-x64.exe`; il confronto non distingue maiuscole e minuscole.
    - **Regola:** il candidato è il PID in primo piano, se compare nel riepilogo con almeno 10 FPS mostrati e non è escluso. Se c'è un candidato, il bersaglio diventa subito quello; se non c'è, il bersaglio corrente resta finché sono passati meno di 3 s dall'ultima volta in cui era candidato, poi cade a `None`.
  - **`overlay::frames`:**
    - `pub fn start_if_requested(link: …, feed: FramesFeed) -> Option<FramesDiagnostics>`. Legge `OMA_FRAMES_DEBUG`: `1` = solo FPS; `pcl` = con `track_pc_latency`; `all` = con PCL e GPU; ogni altro valore, o la variabile assente, non fa nulla.
    - Se attiva:
      - invia `ConfigureFrames`;
      - avvia `ForegroundWatcher`;
      - fa girare il thread `oma-frames` a 4 Hz, che svuota il feed, aggiorna `TargetPicker`, invia `SetFramesTarget` ai cambi e converte i lotti in `FrameSample` (`t_s = qpc / qpc_frequency()`) dentro una `FrameWindow` da 10 s;
      - una volta al secondo scrive con `tracing::info!` una riga `frames:` con:
        - bersaglio (nome, PID), stato del servizio;
        - FPS mostrati (1 s), FPS renderizzati con l'origine o `FG?`, moltiplicatore;
        - low 1% e 0,1% integrali (10 s), frametime medio mostrato;
        - stutter (conteggio, %), latenza PC e di visualizzazione;
        - collo di bottiglia;
        - `dropped` cumulati.
    - Al cambio di bersaglio la finestra si svuota; alla chiusura dell'app il thread si ferma e il watcher si chiude.

- [ ] **Step 1: test che falliscono** (`target.rs`):
  - `foreground_game_becomes_target`;
  - `slow_presenter_is_not_a_game` (9 FPS);
  - `excluded_names_are_never_targets` (`DWM.EXE` maiuscolo);
  - `own_pids_are_never_targets`;
  - `target_survives_a_short_alt_tab` (primo piano altrove per 2 s);
  - `target_drops_after_three_seconds`;
  - `switching_to_another_game_is_immediate`;
  - `tick_reports_only_changes`.

  `frames.rs`: `env_value_maps_to_options` (`1`, `pcl`, `all`, `0`, assente).
- [ ] **Step 2:** `cargo test -p oma-app overlay`: atteso FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`.
- [ ] **Step 5: commit** `feat(app): pick the game to follow and log frame metrics on request`.

### Task B12: installer, licenze, misure e documentazione

**Files:**
- Modify: `scripts/build-installer-payload.ps1`, `scripts/tests/BuildInstallerPayload.Tests.ps1`
- Create: `scripts/lib/OmaPresentMonPins.psm1` (sul modello di `OmaPawnIoPins.psm1`)
- Modify: `app/src-tauri/nsis/oma.nsh`, `scripts/lib/OmaSigning.psm1` (`Test-OmaPayload`), `THIRD_PARTY_NOTICES.md`, `scripts/generate-licenses.ps1` (+ `scripts/licenses/`), `THIRD_PARTY_LICENSES.txt` (rigenerato)
- Modify: `scripts/measure-footprint.ps1`, `docs/perf-budget.md`, `CLAUDE.md`, `README.md`, `CODE_SIGNING.md`, `docs/follow-ups.md`, la spec M7 (§4.1, §4.2, §4.4, §4.5, §11, §14 secondo SD1–SD12)

**Interfaces:**
- **Pin di PresentMon:**
  - `Get-OmaPresentMonPins -RepoRoot` restituisce `{Sha256, SignerSubject}`, con `SignerSubject` = `CN=Intel Corporation, O=Intel Corporation, S=California, C=US`;
  - `Test-OmaPresentMonExe -Path -Pins [-SignatureProvider]` restituisce il primo problema o `$null` (hash, firma `Valid`, soggetto).
- **`build-installer-payload.ps1`**, passo nuovo dopo quello di PawnIO:
  - parametri nuovi `-PresentMonSource`, con valore predefinito l'URL della release `v2.6.0`, e `-PresentMonOnly`;
  - stessa logica di cache, download in `*.partial`, verifica e `Move-Item` verso `target/installer-payload/presentmon/PresentMon-2.6.0-x64.exe`.
- **`oma.nsh`:**
  - nella sezione dei sensori, dopo il servizio: `SetOutPath "$INSTDIR\service\presentmon"` + `File "${OMA_PAYLOAD}\presentmon\PresentMon-2.6.0-x64.exe"`;
  - `!error` a compile time se il file manca, e `!system` con il controllo dell'hash contro `presentmon.sha256`, come per PawnIO;
  - `OmaProtectServiceDir` copre anche la sottocartella;
  - la deselezione e il disinstallatore cancellano l'eseguibile e la cartella;
  - il disinstallatore ferma la sessione con `logman stop OpenMonitorAdvanced-Frames -ets`, ignorando l'esito, dopo l'arresto del servizio.
- **`Test-OmaPayload`:** accetta esattamente un `PresentMon-2.6.0-x64.exe` firmato Intel nell'installer.

- [ ] **Step 1: test Pester che falliscono** (senza tag):
  - `PresentMon is staged when the hash and the Intel signature match`;
  - `a PresentMon with a different hash is rejected`;
  - `an unsigned PresentMon is rejected`;
  - `a cached PresentMon is reused`.

  Si usano `-PresentMonOnly -PresentMonSource <file locale>` e `-SignatureProvider`, come per PawnIO. Si aggiungono anche i casi di `Test-OmaPayload` sul modello di quelli di PawnIO.
- [ ] **Step 2:** `Import-Module Pester -RequiredVersion 5.7.1; Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI`: atteso FAIL sui test nuovi.
- [ ] **Step 3:** implementare script, `oma.nsh` e verifica.
- [ ] **Step 4: licenze.**
  - Testi da includere:
    - testo MIT di PresentMon (`LICENSE.txt` del tag `v2.6.0`, «Copyright 2017-2024 Intel Corporation») in `scripts/licenses/PresentMon-2.6.0-LICENSE.txt`, come voce aggiunta a mano accanto a quella del runtime .NET;
    - i componenti di terze parti compilati nella console, se le stringhe dell'eseguibile li mostrano (`Select-String -Path <exe> -Pattern 'boost|cereal|CLI11|moodycamel' -Encoding ascii`). Per ciascuno trovato, il testo della sua licenza (BSL-1.0, BSD-3-Clause, BSD-2-Clause) nella stessa forma.
  - `THIRD_PARTY_NOTICES.md` riceve una sezione `## PresentMon`, sul modello di `## PawnIO`: versione, URL, firmatario, pin, licenza, componenti, «nessun sorgente incluso».
  - Rigenerare con `pwsh scripts/generate-licenses.ps1`, poi `-Check`.
- [ ] **Step 5: misure.** `measure-footprint.ps1 -Service` misura anche i processi figli del servizio con nome `PresentMon-2.6.0-x64.exe` (stesso metodo di `Measure-ServiceSample`, PID dal genitore). Il risultato va nei campi `PresentMonCpuPercent`, `PresentMonPrivateBytesMB` e `PresentMonValid`, oppure `PresentMonValid = $false` con il motivo `"not running"`. In `docs/perf-budget.md` vanno le righe del §11 con i numeri di SD9.
- [ ] **Step 6: documentazione.**
  - **`CLAUDE.md`:**
    - protocollo **v4** e messaggi dei frame;
    - moduli nuovi (`oma-core::frames`, `oma-win::foreground`, `svc/frames_feed`, `overlay/` dell'app, `Frames/` del servizio);
    - `presentmon.sha256` accanto a `pawnio.sha256`;
    - `testdata/presentmon/`;
    - la variabile `OMA_FRAMES_DEBUG`;
    - stato della milestone M7b.
  - **`README.md`:** i «Known limits» di §14 come corretti da SD10 e SD12, e la nota di privacy su PCL (§1.5 di `r2`: il gioco riceve un ping e, se configurato, tasti F13–F15).
  - **`CODE_SIGNING.md`:** PresentMon è un binario Intel firmato che ridistribuiamo.
  - **Spec M7:** le correzioni SD1–SD12 nei paragrafi indicati.
  - **`docs/follow-ups.md`:** sezione «Manual checks after M7b», con le prove dello Step 8.
- [ ] **Step 7: verifiche complete.**
  - `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`;
  - `dotnet test service/OpenMonitorAdvanced.slnx`;
  - `cd app && pnpm test && pnpm check && pnpm build`;
  - i test Pester (senza `Integration`);
  - `pwsh scripts/generate-licenses.ps1 -Check`;
  - `pwsh scripts/check-version.ps1`.

  Commit `build: ship PresentMon with the service and document the frame engine`. Poi `PYTHONHASHSEED=0 graphify update .`.
- [ ] **Step 8: prove dal vivo con l'utente** (§13.2, mai eseguite da un agente).
  - L'agente costruisce il setup (`pwsh scripts/build-installer-payload.ps1`, poi `cd app && pnpm tauri build --bundles nsis`) e stampa il percorso.
  - L'utente lo installa, chiude l'app dal tray e la riapre da un PowerShell con `$env:OMA_FRAMES_DEBUG='pcl'` (o `1`, o `all`).
  - Poi gioca, mentre l'agente legge le righe `frames:` in `%LOCALAPPDATA%\OpenMonitorAdvanced\logs`.

  Le prove:
  - **V1:** un gioco senza FG: FPS mostrati confrontati con l'overlay di Steam, entro il 5%;
  - **V2:** DLSS FG con e senza PCL (`pcl` contro `1`): con PCL «Reflex» e ×2; senza PCL «FG?»;
  - **V3:** FSR FG con PCL (×2) e senza PCL (solo FPS mostrati);
  - **V4:** Smooth Motion con e senza PCL;
  - **V5:** alt-tab fuori dal gioco e ritorno: il bersaglio regge 3 s e poi cade; tornando, riprende;
  - **V6:** `measure-footprint.ps1 -Service` con l'overlay acceso e senza gioco: PresentMon più il lavoro del servizio sotto lo 0,5% della CPU totale (§11);
  - **V7:** riavvio del servizio da `services.msc` con l'app aperta: dopo la riconnessione le righe `frames:` riprendono da sole.

  Gli esiti vanno nel piano, sezione «Esito dell'esecuzione», e in `docs/follow-ups.md`.

## Decisioni dello spike

Spike eseguito il 2026-10-04 (esito: `docs/superpowers/references/m7/spike-findings.md`; fixture: `testdata/presentmon/`). Nessuna porta di decisione ha chiesto di fermarsi: D5 (console di PresentMon) resta. Correzioni alla spec, nella forma «§: prima → dopo, perché»:

| # | Correzione |
|---|---|
| SD1 | §4.1 argomenti: «elenco da fissare» → `--output_stdout --no_console_stats --qpc_time --track_frame_type --write_frame_id --session_name OpenMonitorAdvanced-Frames --stop_existing_session --no_track_input`, più `--track_pc_latency` con `trackPcLatency` e `--no_track_gpu` senza `trackGpu`; colonne predefinite (né `--v1_metrics` né `--v2_metrics`). `--write_frame_id` è nascosta nell'aiuto ma senza di essa `PCLFrameId` non esiste; `--v2_metrics` non ha `MsBetweenPresents`, `MsBetweenDisplayChange`, `MsUntilDisplayed`. |
| SD2 | §4.1 parser: colonne obbligatorie `Application, ProcessID, SwapChainAddress, PresentMode, TimeInQPC, MsBetweenPresents, MsBetweenDisplayChange, MsUntilDisplayed, MsBetweenAppStart`; facoltative `FrameType`, `MsPCLatency`, `PCLFrameId`, `MsGPUBusy` (assenti → campo nil). `NA` = nil; BOM e CR finale si tollerano. Il valore di `SwapChainAddress` è esadecimale con `0x`. |
| SD3 | §4.2 `Frame`, derivazione dei campi: `qpc`←`TimeInQPC`; `swapchain`←`SwapChainAddress`; `frame_type`←`FrameType` (`Application`→`app`, `Intel XeSS-FG`→`generated_intel_xefg`, `AMD AFMF`→`generated_amd_afmf`, `Unknown`, `NA` o vuoto→`unknown`, altro testo→`generated_other`, colonna assente→`unknown`); `displayed`←`MsBetweenDisplayChange` numerico; `ms_between_presents`←`MsBetweenPresents`; `ms_between_display_change`←`MsBetweenDisplayChange`; `ms_until_displayed`←`MsUntilDisplayed`; `ms_app_frametime`←`MsBetweenAppStart`; `ms_pc_latency`←`MsPCLatency`; `ms_gpu_busy`←`MsGPUBusy`; `pcl_frame_id`←`PCLFrameId` (0 → nil). Nessun campo in più. |
| SD4 | §4.5 punto 2: «PCL solo per DLSS FG» → **vale per DLSS FG, FSR FG e Smooth Motion in ogni gioco con Reflex**; gli FPS renderizzati sono `(ultimo − primo pcl_frame_id) / tempo fra le due righe` nella finestra, non il conteggio degli id (PresentMon lascia senza id circa il 5% dei frame dell'app). L'etichetta dell'origine resta «Reflex». |
| SD5 | §4.5 punto 3, euristica «FG?»: «rapporto ≥ 1,8 per 2 s» → alternanza ≥ 0,9 **e** rapporto ≥ 1,8 su 2 s. Scatta per DLSS FG e Smooth Motion senza PCL (1,00 / 44–58), non scatta senza FG (≤ 0,79 / ≤ 1,27) né con FSR FG senza PCL (presentazioni regolari). |
| SD6 | §4.4 collo di bottiglia: «la maggior parte dei frame» → `gpu` se almeno il **75%** dei frame dell'app nella finestra ha `ms_gpu_busy ≥ 0,9 × ms_app_frametime`, altrimenti `cpu`; con meno di 30 frame dell'app `unknown`. Con FG i frame dell'app sono quelli con `pcl_frame_id`; con FG sospetta («FG?») e senza PCL, `unknown`. Misure: limite GPU 92–99%, limite CPU 40–54%. |
| SD7 | §4.1 svuotamento: resta ogni 100 ms (50 ms non migliora). §4.7/§11 porta dei 300 ms → **ritardo tipico 200–400 ms dalla presentazione all'arrivo, con buchi occasionali fino a circa 2,3 s** in cui i frame arrivano in ritardo ma tutti; la causa non è isolata. Il servizio conta i buchi oltre 1 s e li scrive nel log a livello DEBUG; l'overlay (M7c) deve disegnare per tempo del dato, non d'arrivo. |
| SD8 | §4.1 buffer ETW (§15): non configurabili con la console; PresentMon usa 64 KB × 256 (max 1024), `FlushTimer` 1 s. Nessun evento perso nelle prove. |
| SD9 | §11: costo di PresentMon misurato 0,006–0,05% della CPU totale, 5–6,5 MB privati; il lavoro di lettura nel servizio è stimato da 0,04% a 0,1% (sonda). Limiti del §11 invariati. |
| SD10 | §3.2 e §14 G-Sync: su RTX 4080 la finestra trasparente ai clic non toglie il flip indipendente (MPO) né G-Sync, visibile, vuota o nascosta. La regola «finestra nascosta» resta per l'hardware senza MPO; il limite del §14 diventa «può succedere su hardware senza piani MPO liberi». |
| SD11 | §4.3 esclusioni: confermate `dwm.exe` (presenta come `Hardware: Legacy Flip` quando esiste una finestra sopra il gioco) e i nostri processi (la finestra dell'overlay presenta come ogni altra). |
| SD12 | §14 limiti, FG: «FSR 3/4 FG e Smooth Motion non distinguibili» → **non distinguibili solo senza Reflex o con PCL spento**; con PCL spento DLSS FG e Smooth Motion mostrano «FG?», FSR FG solo gli FPS mostrati. |
