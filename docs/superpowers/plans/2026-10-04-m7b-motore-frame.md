# M7b — Motore dei frame: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** misurare FPS e frametime dei giochi dall'esterno, con PresentMon avviato da `oma-service`, e portarli all'app con il protocollo v4. L'app sceglie il gioco da seguire e calcola le metriche in `oma-core::frames`. Prima di tutto c'è uno spike che fissa i dati mancanti.

**Architecture:**
- **Parte A, lo spike (§4.7 della spec):** cinque domande (S1–S5) con strumenti da buttare in `target/spike/m7b/`, eseguite su questo PC con l'utente che avvia i giochi. Il risultato è un documento di esito, delle fixture CSV e le decisioni sulla spec.
- **Parte B, l'implementazione:**
  - `FrameCapture` nel servizio;
  - il protocollo v4;
  - `overlay::Target` nell'app;
  - le metriche in `oma-core::frames`.

  La spec vieta di scriverla prima dello spike (§4.7: «Prima di scrivere il resto del piano M7b…»). Qui c'è la mappa dei task, con la risposta dello spike da cui dipende ciascuno; i passi si scrivono in questo file a spike chiuso.

**Tech Stack:**
- PresentMon 2.6.0 (console, firmata Intel, MIT), ETW (`ControlTraceW`), WPR e tracerpt;
- .NET 10 per la sonda del servizio, Rust 1.90 con `windows` 0.62 per la sonda della finestra;
- Python 3 (solo libreria standard) per l'analisi dei CSV.

**Spec:** `docs/superpowers/specs/2026-10-04-m7-manutenzione-overlay-design.md`, §3 e §4 (anche §9, §10, §11 e §15 per la M7b). Ricerche: `docs/superpowers/references/m7/r2-fps-framegen.md` e `r4-presentmon-options.md`.

**Branch:** `feat/m7b-motore-frame`, da `main`; merge in `main` in locale alla fine della Parte B. Push solo su richiesta dell'utente. La M7b non fa release: la 0.5.0 arriva con la M7d (D11).

**Esecuzione:**
- **Parte A in linea, nella sessione principale:** quasi ogni task ha passi dal vivo con l'utente, e gli strumenti non si committano. Ogni sonda si rilegge prima di passarla all'utente.
- **Parte B subagent-driven:** i task si scrivono dopo lo spike.

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

# Parte B — Implementazione (mappa; i passi si scrivono dopo lo spike)

| Task | Contenuto | Spec | Dipende da |
|---|---|---|---|
| B1 | `build-installer-payload.ps1`: scaricare PresentMon, SHA-256 fissato in `app/src-tauri/nsis/presentmon.sha256` e firma Intel verificata, con i test Pester senza `Integration`; installazione in `$INSTDIR\service\presentmon\`; arresto della sessione per nome nel disinstallatore; `THIRD_PARTY_NOTICES.md` e `generate-licenses.ps1` | §4.1, §10, §12 | A1 (hash) |
| B2 | protocollo v4 in `crates/oma-ipc`: `FramesConfigure`, `FramesTarget`, `FramesStatus`, `PresentingProcesses`, `FrameBatch`, `Frame`; `PROTOCOL_VERSION = 4`; fixture rigenerate | §4.2 | S1 (campi) |
| B3 | protocollo v4 nel servizio (.NET), revisione `protocol-parity-reviewer` | §4.2 | B2 |
| B4 | parser CSV del servizio: per nome di colonna, righe al massimo di 4 KiB, scarti contati, `Failed("columns")`; test sulle fixture di `testdata/presentmon/` | §4.1 | S1 (colonne, v1/v2) |
| B5 | `FrameCapture` dietro `IFrameSource`: Job Object, hash prima dell'avvio, svuotamento ogni 100 ms, arresto per nome, ritentativi da 1 s a 60 s, `Failed("crashing")` dopo 5 cadute in 10 minuti, `Denied`, `Tampered`, `Missing`, stop dopo 30 s di disconnessione; test con un finto PresentMon | §4.1, §9 | S2, S3, S5 |
| B6 | filtro del PID e validazione (non 0 né 4), riepilogo a 1 Hz (al massimo 32 voci), lotti a 10 Hz (al massimo 512 frame, `dropped`), collegamento alla pipe | §4.1, §4.2, §10 | B4, B5 |
| B7 | `oma-core::frames`: FPS mostrati, renderizzati e presentati, frametime, low integrali e percentili, stutter, moltiplicatore FG, latenze, collo di bottiglia; casi calcolati a mano e fixture | §4.4 | S1 (soglia del collo di bottiglia) |
| B8 | `oma-core::frames`: cascata del §4.5, euristica «FG?», scelta della swapchain, `synthetic(seed, profile)` deterministico | §4.5, §4.6 | S1 (DLSS con PCL, FSR, Smooth Motion) |
| B9 | app, `overlay::Target`: `SetWinEventHook` per il primo piano e `LOCATIONCHANGE`, PID della `CoreWindow` per UWP, esclusioni, tolleranza di 3 s, soglia di 10 FPS; test con eventi finti; revisione `ffi-safety-reviewer` | §4.3 | B2 |
| B10 | app, client dei frame: `FramesConfigure` e `FramesTarget`, stato e metriche. Senza l'interfaccia dell'overlay, che arriva con la M7c, si attiva con la variabile d'ambiente `OMA_FRAMES_DEBUG=1` e scrive le metriche nel log a 1 Hz | §4.2, §9 | B6, B7, B8, B9 |
| B11 | `measure-footprint.ps1` esteso a PresentMon; `docs/perf-budget.md`; `CLAUDE.md` (protocollo v4, SHA-256 di PresentMon, moduli nuovi); README («Known limits», nota su PCL); `CODE_SIGNING.md`; `docs/follow-ups.md`; poi le prove dal vivo del §13.2 per la M7b | §11, §12, §13.2 | tutti |

**Punto aperto, da confermare con la Parte B:** B10 propone una variabile d'ambiente invece di un'interfaccia, per non anticipare le impostazioni della M7c (§5.7). In questo modo le prove dal vivo della M7b (metriche con e senza FG, confrontate con l'overlay di Steam o con FrameView) si leggono dal log.

## Decisioni dello spike

Spike eseguito il 2026-10-04 (esito: `docs/superpowers/references/m7/spike-findings.md`; fixture: `testdata/presentmon/`). Nessuna porta di decisione ha chiesto di fermarsi: D5 (console di PresentMon) resta. Correzioni alla spec, nella forma «§: prima → dopo, perché»:

| # | Correzione |
|---|---|
| SD1 | §4.1 argomenti: «elenco da fissare» → `--output_stdout --no_console_stats --qpc_time --track_frame_type --write_frame_id --session_name OpenMonitorAdvanced-Frames --stop_existing_session --no_track_input`, più `--track_pc_latency` con `trackPcLatency` e `--no_track_gpu` senza `trackGpu`; colonne predefinite (né `--v1_metrics` né `--v2_metrics`). `--write_frame_id` è nascosta nell'aiuto ma senza di essa `PCLFrameId` non esiste; `--v2_metrics` non ha `MsBetweenPresents`, `MsBetweenDisplayChange`, `MsUntilDisplayed`. |
| SD2 | §4.1 parser: colonne obbligatorie `Application, ProcessID, SwapChainAddress, PresentMode, TimeInQPC, MsBetweenPresents, MsBetweenDisplayChange, MsUntilDisplayed, MsBetweenAppStart`; facoltative `FrameType`, `MsPCLatency`, `PCLFrameId`, `MsGPUBusy` (assenti → campo nil). `NA` = nil; BOM e CR finale si tollerano. Il valore di `SwapChainAddress` è esadecimale con `0x`. |
| SD3 | §4.2 `Frame`, derivazione dei campi: `qpc`←`TimeInQPC`; `swapchain`←`SwapChainAddress`; `frame_type`←`FrameType` (`Application`→`app`, `Intel XeSS-FG`→`generated_intel_xefg`, `AMD AFMF`→`generated_amd_afmf`, altro testo→`generated_other`, colonna assente→`unknown`); `displayed`←`MsBetweenDisplayChange` numerico; `ms_between_presents`←`MsBetweenPresents`; `ms_between_display_change`←`MsBetweenDisplayChange`; `ms_until_displayed`←`MsUntilDisplayed`; `ms_app_frametime`←`MsBetweenAppStart`; `ms_pc_latency`←`MsPCLatency`; `ms_gpu_busy`←`MsGPUBusy`; `pcl_frame_id`←`PCLFrameId` (0 → nil). Nessun campo in più. |
| SD4 | §4.5 punto 2: «PCL solo per DLSS FG» → **vale per DLSS FG, FSR FG e Smooth Motion in ogni gioco con Reflex**; gli FPS renderizzati sono `(ultimo − primo pcl_frame_id) / tempo fra le due righe` nella finestra, non il conteggio degli id (PresentMon lascia senza id circa il 5% dei frame dell'app). L'etichetta dell'origine resta «Reflex». |
| SD5 | §4.5 punto 3, euristica «FG?»: «rapporto ≥ 1,8 per 2 s» → alternanza ≥ 0,9 **e** rapporto ≥ 1,8 su 2 s. Scatta per DLSS FG e Smooth Motion senza PCL (1,00 / 44–58), non scatta senza FG (≤ 0,79 / ≤ 1,27) né con FSR FG senza PCL (presentazioni regolari). |
| SD6 | §4.4 collo di bottiglia: «la maggior parte dei frame» → `gpu` se almeno il **75%** dei frame dell'app nella finestra ha `ms_gpu_busy ≥ 0,9 × ms_app_frametime`, altrimenti `cpu`; con meno di 30 frame dell'app `unknown`. Con FG i frame dell'app sono quelli con `pcl_frame_id`; con FG sospetta («FG?») e senza PCL, `unknown`. Misure: limite GPU 92–99%, limite CPU 40–54%. |
| SD7 | §4.1 svuotamento: resta ogni 100 ms (50 ms non migliora). §4.7/§11 porta dei 300 ms → **ritardo tipico 200–400 ms dalla presentazione all'arrivo, con buchi occasionali fino a circa 2,3 s** in cui i frame arrivano in ritardo ma tutti; la causa non è isolata. Il servizio conta i buchi oltre 1 s e li scrive nel log a livello DEBUG; l'overlay (M7c) deve disegnare per tempo del dato, non d'arrivo. |
| SD8 | §4.1 buffer ETW (§15): non configurabili con la console; PresentMon usa 64 KB × 256 (max 1024), `FlushTimer` 1 s. Nessun evento perso nelle prove. |
| SD9 | §11: costo di PresentMon misurato 0,006–0,05% della CPU totale, 5–6,5 MB privati; il lavoro di lettura nel servizio è stimato da 0,04% a 0,1% (sonda). Limiti del §11 invariati. |
| SD10 | §3.2 e §14 G-Sync: su RTX 4080 la finestra trasparente ai clic non toglie il flip indipendente (MPO) né G-Sync, visibile, vuota o nascosta. La regola «finestra nascosta» resta per l'hardware senza MPO; il limite del §14 diventa «può succedere su hardware senza piani MPO liberi». |
| SD11 | §4.3 esclusioni: confermate `dwm.exe` (presenta come `Hardware: Legacy Flip` quando esiste una finestra sopra il gioco) e i nostri processi (la finestra dell'overlay presenta come ogni altra). |
| SD12 | §14 limiti, FG: «FSR 3/4 FG e Smooth Motion non distinguibili» → **non distinguibili solo senza Reflex o con PCL spento**; con PCL spento DLSS FG e Smooth Motion mostrano «FG?», FSR FG solo gli FPS mostrati. |
