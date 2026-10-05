# M7b — Esito dello spike sul motore dei frame

- **Data:** 2026-10-04, su questo PC: RTX 4080 con driver 617.14, iGPU AMD, Windows 11 26300, 16 thread logici.
- **Giochi:** Control Resonant (`CONTROLResonant.exe`, DX12, Reflex) e God of War 2018 (`GoW.exe`, DX11, Reflex), in finestra senza bordi, avviati dall'utente.
- **PresentMon:** console `PresentMon-2.6.0-x64.exe`.
  - Firma: Authenticode `Valid`, `CN=Intel Corporation`, timestamp Sectigo.
  - Dimensione: 980.320 byte.
  - SHA-256: `B2A706BC6AD475749E3B7E3409263AA1E6906D45BDCF993F6DBC0F660188F1AF`.
- **Strumenti:** da buttare, in `target/spike/m7b/` e non tracciati:
  - `capture.ps1`;
  - `analyze.py`;
  - la sonda .NET `flush-probe`, che riproduce il §4.1 della spec;
  - la sonda Rust `overlay-probe`, finestra DirectComposition trasparente ai clic.
- **Fixture:** in `testdata/presentmon/`, origine e valori in `testdata/presentmon/README.md`.

## In breve

| # | Domanda | Risposta |
|---|---|---|
| S1 | Come appaiono FG e Reflex nel CSV? | **DLSS FG, FSR FG e Smooth Motion** si distinguono tutti con `--track_pc_latency --write_frame_id`: `PCLFrameId` è 0 sui frame generati. Senza PCL, DLSS FG e Smooth Motion si riconoscono dall'alternanza dei `MsBetweenPresents`, FSR FG no. `FrameType` vale sempre `Application`: nessuna delle tre usa il provider Intel. |
| S2 | Quanto costa PresentMon sempre acceso? | Dallo 0,006% allo 0,05% della CPU totale, con 5–6,5 MB privati: due ordini di grandezza sotto la soglia dell'1%. **D5 confermata.** |
| S3 | PresentMon gira come figlio di LocalSystem? | Sì, nella sessione 0. Funzionano lo svuotamento dei buffer, la chiusura con il Job Object e l'arresto per nome dopo una caduta. Ritardo tipico 250 ms, con buchi occasionali di circa 2 s (§S3). |
| S4 | La finestra sopra il gioco rompe il flip indipendente? | No, su questo PC: il gioco resta in `Hardware Composed: Independent Flip` (MPO) con la finestra visibile, vuota o nascosta, e G-Sync resta acceso. |
| S5 | `--stop_existing_session` tocca altre sessioni? | No: una seconda sessione con un altro nome continua a registrare senza interruzioni. |

## S1 — Colonne, frame generation e Reflex

### Argomenti e colonne

- **Insieme di colonne:** si usa quello predefinito della 2.6.0, senza `--v1_metrics` né `--v2_metrics`. Con `--v2_metrics` mancano `MsBetweenPresents`, `MsBetweenDisplayChange` e `MsUntilDisplayed`, al loro posto ci sono `FrameTime`, `DisplayedTime` e `DisplayLatency`.
- **Intestazione predefinita** (23 colonne): `Application, ProcessID, SwapChainAddress, PresentRuntime, SyncInterval, PresentFlags, AllowsTearing, PresentMode, FrameType, TimeInQPC, MsBetweenSimulationStart, MsBetweenPresents, MsBetweenDisplayChange, MsInPresentAPI, MsRenderPresentLatency, MsUntilDisplayed, CPUStartQPC, MsBetweenAppStart, MsCPUBusy, MsCPUWait, MsAnimationError, AnimationTime, MsFlipDelay`.
- **Opzioni che aggiungono colonne:**
  - **GPU tracciata** (senza `--no_track_gpu`): `MsGPULatency, MsGPUTime, MsGPUBusy, MsGPUWait` dopo `MsCPUWait`.
  - **`--track_pc_latency`:** `MsPCLatency` dopo `MsUntilDisplayed`.
  - **`--write_frame_id`:** `FrameId` in fondo, e con PCL anche `PCLFrameId`. L'opzione **non compare nell'aiuto** della 2.6.0, ma è accettata (un'opzione sconosciuta dà `unrecognized option`). **Senza `--write_frame_id`, `PCLFrameId` non c'è**, nemmeno con PCL attivo.
- **Formato del testo:**
  - `--output_file` scrive un BOM UTF-8 e righe CRLF;
  - su `--output_stdout` le righe arrivano una per una;
  - `NA` indica un valore assente, per esempio `MsAnimationError` sui frame generati.

### Casi misurati

I valori sono sulla cattura intera da 30 s, meno i primi 2 s. «Rapporto» è `mediana(intervalli lunghi) / mediana(intervalli brevi)` dei `MsBetweenPresents` su finestre di 2 s; «alternanza» è la quota di coppie consecutive di differenze con segno opposto.

| Caso | FPS mostrati | FPS renderizzati (PCL) | Alternanza / rapporto | `PresentMode` | Note |
|---|---|---|---|---|---|
| Control, senza FG | 74,4 | — | 0,74 / 1,06 | Hardware Composed: Independent Flip | GPU busy ≥ 0,9 × frametime sul 98% dei frame |
| Control, senza FG, PCL | 101,5 | 99,7 | 0,78 / 1,13 | come sopra | `PCLFrameId` = 0 sul 5% delle righe, sparse |
| Control, limite CPU | 156,1 | — | 0,53 / 1,27 | come sopra | GPU busy ≥ 0,9 × frametime sul 40% |
| Control, DLSS FG | 135,1 | — | **1,00 / 57** | Hardware: Independent Flip | righe generate: `MsBetweenPresents` 0,25 ms, `MsCPUBusy` 0,08 ms |
| Control, DLSS FG, PCL | 125,3 | **61,3 (×2,04)** | 1,00 / 58 | come sopra | `PCLFrameId` solo sulle righe dell'app |
| Control, FSR FG | 113,4 | — | 0,72 / 1,05 | Hardware Composed: Independent Flip | **presentazioni regolari**: l'FG non si vede dai tempi |
| Control, FSR FG, PCL | 103,8 | **51,4 (×2,02)** | 0,71 / 1,04 | come sopra | `PCLFrameId` a righe alterne |
| GoW, senza FG | 122,1 | — | 0,74 / 1,03 | Hardware Composed: Independent Flip | GPU busy ≥ 0,9 × frametime sul 99% |
| GoW, Smooth Motion | 157,0 | — | **1,00 / 44** | Hardware: Independent Flip | stessa firma di DLSS FG |
| GoW, Smooth Motion, PCL | 157,0 | **78,6 (×2,0)** | 1,00 / 45 | come sopra | `PCLFrameId` a righe alterne |

Osservazioni:
- **Nessun processo in più:** con DLSS FG, FSR FG e Smooth Motion i frame generati sono presentazioni dello **stesso PID**, su **un'unica swapchain**. Accendere o spegnere l'FG crea una swapchain nuova, con un altro indirizzo.
- **Tracce WPR:**
  - Control con DLSS FG: il provider PCL emette i marcatori 0–5, 9, 10 e 22 (`NUM_PRESENTS_IN_BATCH`) una volta per frame dell'app, e 11/12 due volte per frame (due presentazioni per frame);
  - Control con FSR FG e GoW: solo 0–5 e il ping 8;
  - in nessuna traccia c'è un evento del provider Intel-PresentMon.
- **FSR FG senza PCL:** la `MsGPUBusy` alterna in modo regolare (9,4 e 8,2 ms, alternanza 0,92). È un indizio su un solo caso: non si usa.

### Conseguenze per la spec

- **FPS renderizzati con PCL:** si calcolano dall'avanzare degli id, cioè `(ultimo PCLFrameId − primo PCLFrameId) / tempo fra le due righe`, e non contando le righe con un id. PresentMon lascia senza id circa il 5% dei frame dell'app (passi di 2 negli id), e il conteggio delle righe darebbe un valore più basso del 5%.
- **Euristica «FG?»:** si attiva quando, in 2 s, l'alternanza è almeno 0,9 **e** il rapporto almeno 1,8. Senza FG l'alternanza misurata va da 0,53 a 0,79 e il rapporto non supera 1,27; con DLSS FG e Smooth Motion valgono 1,00 e 44–58. FSR FG senza PCL non la attiva, e resta un limite dichiarato.

## S2 — Costo

| Prova (`flush-probe`, 60 s) | CPU di PresentMon (% del totale) | Memoria privata |
|---|---|---|
| desktop, senza svuotamento | 0,008% | 6,5 MB |
| desktop, svuotamento ogni 100 ms | 0,006% | 5,4 MB |
| desktop, GPU tracciata | 0,023% | 6,0 MB |
| desktop, PCL | 0,019% | 5,5 MB |
| desktop, come SYSTEM | 0,011% | 6,0 MB |
| Control, come SYSTEM | 0,037% | 5,3 MB |
| GoW a 148 righe/s, come SYSTEM | 0,050% | 5,5 MB |

La sonda stessa, cioè lettura del CSV e svuotamento, costa dallo 0,04% allo 0,1%: è il riferimento per il lavoro in più di `oma-service`.

**Buffer ETW:** la console li fissa da sola e non si possono configurare. I valori sono `BufferSize` 64 KB, `MinimumBuffers` 256, `MaximumBuffers` 1024, `FlushTimer` 1 s. In tutte le prove `EventsLost` e `RealTimeBuffersLost` sono 0.

**Provider attivati** (da `logman query`):
- Intel-PresentMon, D3D9, Kernel-Process, DxgKrnl (Base, Present), Dwm-Core, DXGI e Win32k;
- NVIDIA Display Driver;
- due GUID senza nome: `8C9DD1AD-E6E5-4B07-B455-684A9D879900` e `65CD4C8A-0848-4583-92A0-31C0FBAF00C0`.

## S3 — Figlio di LocalSystem, svuotamento, cadute

- **Esecuzione come SYSTEM:** la sonda è stata avviata da un'operazione pianificata `/RU SYSTEM`, con identità `NT AUTHORITY\SYSTEM` nella sessione 0. PresentMon parte, le righe arrivano e `ControlTraceW(FLUSH)` non dà errori.
- **Caduta:** con `Environment.FailFast` dopo 10 s, il Job Object chiude PresentMon (nessun processo orfano), e la sessione `OpenMonitorAdvanced-Frames` resta aperta (`logman`: in esecuzione). All'avvio successivo `ControlTraceW(STOP)` per nome restituisce 0 e la ferma; dopo l'arresto `QUERY` dà 4201.
- **Ritardo** = istante di arrivo della riga meno `TimeInQPC`, quindi compreso il tempo fino alla visualizzazione:
  - **desktop:** non è rappresentativo, perché le finestre presentano raramente e PresentMon tiene la riga finché non arriva la presentazione successiva (p50 0,3–2,2 s);
  - **in gioco, come SYSTEM, svuotamento ogni 100 ms:** p50 245–265 ms, p95 362–714 ms;
  - **svuotamento ogni 50 ms:** p50 209 ms contro 282 nella stessa scena, nel rumore delle misure. Si resta a 100 ms.
- **Buchi di circa 2 s:** ogni tanto nessuna riga arriva per circa 2 s, poi arrivano tutte insieme. Nessun dato va perso: i frame arrivano in ritardo.
  - **Come amministratore** nella console dell'utente capitano ogni 10 s, in corrispondenza delle righe che la sonda scrive nella console. Non dipendono dalla `QUERY` periodica: la prova senza `QUERY` li mostra uguali.
  - **Come SYSTEM** (senza console) ce n'è uno all'avvio e uno in 60 s.
  - La causa non è stata isolata. L'ipotesi è PresentMon che trattiene l'uscita in ordine mentre aspetta la conclusione di una presentazione di un altro processo (per esempio la console, il cui processo compare nel CSV).

## S4 — Finestra trasparente ai clic sopra il gioco

- **La sonda** (topmost, `WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`, swapchain per la composizione, Direct2D e DirectWrite a 30 FPS) si vede sopra il gioco senza bordi, e i clic la attraversano.
- **Control, 90 s per prova**, sia con la finestra disegnata sia vuota (`--empty`):

  | Fase | `PresentMode` del gioco | FPS |
  |---|---|---|
  | prima della finestra | Hardware Composed: Independent Flip | 61–64 |
  | visibile | Hardware Composed: Independent Flip | 60–61 |
  | nascosta | Hardware Composed: Independent Flip | 61–62 |
  | di nuovo visibile | Hardware Composed: Independent Flip | 61 |

  L'indicatore G-SYNC non si è mai spento.
- **Presentazioni in più:** mentre la finestra esiste, presentano anche `dwm.exe` (`Hardware: Legacy Flip`, circa 20 al secondo) e la finestra stessa (`Composed: Flip`, 30 al secondo).
- **Non provati:** il fullscreen esclusivo e un hardware senza piani MPO liberi.

## S5 — Convivenza

Una seconda istanza di PresentMon, `OMA-S5-Other`, registrava da 5 s quando è partita la nostra con `--stop_existing_session`. Risultati:
- `logman` la mostra in esecuzione prima e dopo la nostra;
- il suo CSV copre tutti i 40 s, senza buchi oltre gli 0,5 s normali delle presentazioni del desktop;
- la nostra sessione sparisce alla fine.

Su questo PC non ci sono né CapFrameX né FrameView.
