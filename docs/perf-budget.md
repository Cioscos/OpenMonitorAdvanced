# Performance budget

## Grafici fluidi — protocollo release e risultati in attesa

Stato al 2026-09-27: la build release WebView2 è stata compilata, ma la verifica
visiva e le misure dal vivo non sono ancora state eseguite. Nessun valore FPS,
CPU o memoria di questa build è dichiarato come risultato o come criterio
soddisfatto. Build da verificare: commit `aee6305ce273bf04a425f4bf8e355360f18489d9`,
`target/release/oma-app.exe` SHA-256
`B169955CD41FD4BF46774D0D55578688AFCEF0A15C22EF8C39DBEFE3E4EBB9B0`.
Se il codice cambia, ricompilare e registrare il nuovo commit e hash.

### Procedura riproducibile

1. Registrare data e ora, Windows e build WebView2, CPU, GPU e driver, RAM,
   display e frequenza di aggiornamento, alimentazione, eventuale carico in
   parallelo, servizio connesso, modalità sicura e moduli vendor. Chiudere le
   altre istanze di `oma-app.exe`; verificare che non vi siano crash marker.
   Eseguire la release, non `pnpm dev`. L'utente deve gestire la finestra e
   confermare che resti visibile; non usare clic sintetici o UI Automation.
2. Con l'app chiusa, predisporre Avanzata con
   `scripts/seed-advanced-view.ps1 -Section 'gpu/pci-0000:01:00.0' -Window 3600 -Series $series`,
   dove `$series` contiene gli otto ID della misura M3 riportati sotto:
   `load/core`, `load/3d`, `load/copy`, `load/video-decode`,
   `load/video-encode`, `fan/percent`, `percent/power-limit`,
   `temperature/core`, ciascuno prefissato da `gpu/pci-0000:01:00.0/`.
   Verificare poi con `-CheckOnly`. Lo script apre e chiude l'app: usarlo soltanto nella
   sessione concordata con l'utente. In Semplificata selezionare la vista con
   l'utente; registrare il riquadro osservato. Verificare a vista curve,
   punto bianco e glow rispetto allo screenshot, dati assenti, nascondi/riprendi,
   movimento ridotto e finestre Avanzata 1/5/30/60 min con 8 serie.
3. Per ciascuna vista, riempire lo storico con
   `scripts/measure-footprint.ps1 -FillHistoryMinutes 61 -SampleSeconds 60 -Service`
   (omettere `-Service` se il servizio non è installato, annotandolo). Lo
   script misura prima la tray e poi apre la finestra per almeno 15 s di
   warm-up e 60 s di campionamento; chiude il processo alla fine. Confermare
   a vista la pagina effettivamente aperta. Per la vista Semplificata, i 61
   minuti superano la sua finestra di 5 minuti; per Avanzata riempiono 1 h.
4. In una nuova esecuzione per ciascuna vista, lasciare la finestra
   continuamente visibile per almeno un'ora:
   `scripts/measure-footprint.ps1 -WarmupSeconds 3660 -SampleSeconds 60 -Service`.
   L'utente conferma che non è stata ridotta a icona o chiusa. Fare una
   misura tray separata con `-Minimized -SampleSeconds 60` se la coppia del
   punto 3 non è valida. Registrare output grezzo e durata effettiva.
5. Nella shell che avvia la misura impostare
   `$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9223'`,
   collegare DevTools alla WebView2 e rimuovere la variabile dopo la misura.
   Durante ogni finestra di 60 s, raccogliere ed esportare una traccia
   Performance della WebView2 con callback `requestAnimationFrame`, disegno
   e frame presentati. Annotare percorso della traccia e conteggio frame.
   Escludere warm-up, intervalli nascosti e movimento ridotto dal calcolo FPS.
   Dai timestamp dei frame presentati calcolare gli intervalli consecutivi:
   FPS mediano = `1000 / mediana(intervalli_ms)` e p95 del tempo frame =
   95° percentile degli intervalli in ms; annotare anche callback, disegno e
   frame lunghi. Non confondere la cadenza dei campioni sensore con gli FPS.
   Associare CPU e memoria di `measure-footprint.ps1` alla stessa finestra.
   Per il budget CPU usare **`TotalAppPercentCpu`**, somma di `oma-app.exe`
   e dei discendenti WebView2 sullo stesso intervallo, soltanto quando
   `TotalAppCpuValid` è `True`. Se la topologia cambia tra i due rilievi, un
   contatore è nullo/mancante o la finestra non ha WebView2, la misura CPU è
   invalida: ripeterla, senza interpretare `null` come zero. Lo script vede
   la topologia solo all’inizio e alla fine: un processo nato e terminato
   durante l’intervallo può sfuggire alla somma. Annotare questo limite;
   se un tracciamento dei processi rivela ricambi nell’intervallo, invalidare
   la misura e ripeterla su un intervallo stabile. `CorePercentCpu` resta
   il solo processo host per continuità con le misure precedenti.
   `TotalPrivateMB` comprende app e processi WebView2, mentre tray usa
   `AppPrivateMB`. Conservare trace, screenshot e output grezzo con la build.

| Vista / stato | Data, hardware, refresh, build | FPS mediano | p95 frame ms | CPU app+WebView2 % | Memoria MB | Traccia / output | Esito |
|---|---|---:|---:|---:|---:|---|---|
| Semplificata, storico pieno, ≥60 s | in attesa | — | — | — | — | in attesa | non valutato |
| Avanzata, 1 h, 8 serie, storico pieno, ≥60 s | in attesa | — | — | — | — | in attesa | non valutato |
| Semplificata, dopo ≥1 h visibile | in attesa | — | — | — | — | in attesa | non valutato |
| Avanzata, 1 h, 8 serie, dopo ≥1 h visibile | in attesa | — | — | — | — | in attesa | non valutato |
| Tray | in attesa | n/a | n/a | — | — | in attesa | non valutato |

Criteri su display a 60 Hz e finestra visibile con movimento normale:
FPS mediano ≥55, p95 frame ≤20 ms, CPU app a riposo <1% della macchina,
finestra <200 MB complessivi e tray <30 MB. Riportare esplicitamente il
refresh reale se diverso da 60 Hz. Se una misura fallisce, profilare,
correggere e ripetere quella misura; un cambio di renderer richiede prima
una revisione del design approvato.

Budget (spec §1.2), measured with `scripts/measure-footprint.ps1` on a release build.
Memory = private working set (Task Manager "Memory" column); CPU = share of all logical processors.
Le righe M1–M4 qui sotto riportano il vecchio `CorePercentCpu`, cioè solo
`oma-app.exe`; non sono evidenza di CPU complessiva della WebView2. Le nuove
misure dei grafici fluidi useranno `TotalAppPercentCpu` con validità esplicita.

| Milestone | Machine | Mode | App CPU % | App private MB | WebView2 procs | Total private MB | Budget met |
|---|---|---|---|---|---|---|---|
| M1 | AMD Ryzen 7 7800X3D, 32 GB RAM, Windows 11 Pro 10.0.26200 | window | 0 | 15.0 | 6 | 112.5 | yes |
| M1 | same machine | tray | 0.01 | 12.3 | 0 | 12.3 | yes |
| M2 | same machine, NVIDIA GeForce RTX 4080 (driver 32.0.16.1714) + AMD Radeon(TM) Graphics iGPU Raphael (driver 32.0.21045.5002), build `d678c21` | window | 0.05 | 18.2 | 6 | 113.3 | yes |
| M2 | same machine, same drivers, build `d678c21` | tray | 0.05 | 16.7 | 0 | 16.7 | yes |
| M3 | same machine, NVIDIA GeForce RTX 4080 (driver 32.0.16.1714) + AMD Radeon(TM) Graphics iGPU Raphael (driver 32.0.21045.5002), build `21896f5` | window (Advanced view, GPU page, 1 h chart, 8 series) | 0.1 | 18.9 | 6 | 131.6 | yes |
| M3 | same machine, same drivers, build `21896f5` | tray | 0.04 | 16.4 | 0 | 16.4 | yes |
| M3 | same machine, same drivers, build `21896f5` | window, after 61 min in the tray (full 1 h history; Advanced view as above) | 0.06 | 23.0 | 6 | 139.6 | yes |
| M3 | same machine, same drivers, build `21896f5` | tray, after 61 min (full 1 h history) | 0.04 | 20.6 | 0 | 20.6 | yes |
| M3 | same machine, same drivers, build `21896f5` | window, continuously visible for 61 min (Advanced GPU, 1 h, 8 series; raw live tail) | 0.06 | 23.9 | 6 | 149.9 | yes |
| M4 | same machine, PawnIO 2.2.0, Windows 11 Pro 10.0.26200, service installed | window (`oma-service` connected, Task 15 live verification) | 0.03 | 19.5 | 6 | 141.2 | yes |
| M4 | same machine, same conditions | tray | 0.07 | 17.3 | 0 | 17.3 | yes |

Budget: app CPU < 1 % at idle; tray < 30 MB; window open < 200 MB in total.

| Milestone | Machine | Mode | Service CPU % (of machine) | Service Private Bytes |
|---|---|---|---|---|
| M4 | AMD Ryzen 7 7800X3D, B650, 2× DDR5, 1 SATA HDD, 1 SATA SSD, 2 NVMe, RTX 4080 + AMD iGPU, Windows 11 Pro 10.0.26200, PawnIO 2.2.0 | window | 0.03 | 51.6 MB |
| M4 | same machine, same conditions | tray | 0.04 | 52.7 MB |

Budget: service CPU < 1 % of the machine; service private bytes < 80 MB — both met.

## M4 measurement details

Measured 2026-09-27 on the development machine (AMD Ryzen 7 7800X3D, B650,
2× DDR5, 1 SATA HDD, 1 SATA SSD, 2 NVMe, RTX 4080 + AMD iGPU, Windows 11 Pro
10.0.26200, PawnIO 2.2.0), with `scripts/measure-footprint.ps1 -Exe
'C:\Program Files\OpenMonitor Advanced\oma-app.exe' -Service`, 15 s warm-up +
30 s sample per run, `oma-service` connected to the app. The service CPU
figure is computed from raw performance counters over the sample window
(fix `2ecd011`), not the integer-formatted counter, which had earlier shown
a spurious 0 % reading that was discarded.

## M2 measurement details

Measured 2026-09-25 on the development machine, release build `d678c21`
(`cd app && pnpm tauri build --no-bundle`), 15 s warm-up + 30 s sample per run
(the script's defaults), no crash marker present, no other `oma-app.exe`
instance running. In both modes `VendorModules : atiadlxx.dll, nvapi64.dll,
nvml.dll` — all three vendor libraries were loaded, so the measurement is not
a safe-mode result.

Raw output:

```
Mode              : window
CorePercentCpu    : 0.05
AppPrivateMB      : 18.2
WebView2Processes : 6
TotalPrivateMB    : 113.3
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll

Mode              : tray
CorePercentCpu    : 0.05
AppPrivateMB      : 16.7
WebView2Processes : 0
TotalPrivateMB    : 16.7
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
```

All budget items are met on both modes. `AppPrivateMB` grew by about 3-4 MB
over the M1 baseline (window: 15.0 → 18.2; tray: 12.3 → 16.7), consistent
with the private working set added by the three vendor libraries and their
NVML/NVAPI/ADL handles; this run does not isolate that delta further because
it stayed comfortably under budget, so a safe-mode comparison and a
`VirtualUnlock` before/after measurement were not repeated on this build.

Note: private working set is not the same as committed memory.
`VirtualUnlock` (Task 7, on the `nvml.dll` `.data` section) can remove pages
from the working set without freeing the library's underlying allocations, so
a low `AppPrivateMB`/`TotalPrivateMB` here does not by itself mean the vendor
libraries hold no committed memory — only that it is not currently resident.
The finer-grained figures from the spike recorded in the M2 plan
(`docs/superpowers/plans/2026-09-25-m2-gpu.md`), e.g. the ~19 MB NVML `.data`
section before/after `VirtualUnlock` or the isolated per-library vendor delta,
are prior spike evidence, not reproduced against this final build; see the
plan for those references.

## M3 measurement details

Measured 2026-09-25 on the development machine, release build `21896f5`
(`cd app && pnpm tauri build --no-bundle`), same GPU drivers as M2
(NVIDIA GeForce RTX 4080, driver 32.0.16.1714; AMD Radeon(TM) Graphics iGPU
Raphael, driver 32.0.21045.5002). Protocol: 15 s warm-up + 30 s sample per
run (the script's defaults), except for the continuously-visible row, which
uses `-WarmupSeconds 3660` to delay the sample while the window stays open;
the full-history rows measure after the app has run 61 minutes in the tray,
so the one-hour ring buffer is full. No crash marker present, no other
`oma-app.exe` instance running (verified before each run).

The Advanced view was prepared with `scripts/seed-advanced-view.ps1`,
section `gpu/pci-0000:01:00.0`, window `3600`, and the 8 series listed in
the M3 plan (`load/core`, `load/3d`, `load/copy`, `load/video-decode`,
`load/video-encode`, `fan/percent`, `percent/power-limit`,
`temperature/core` of the RTX 4080). The seeded state was checked again
after the full-history run with `-CheckOnly`:

```
view         : advanced
section      : gpu/pci-0000:01:00.0
window       : 3600
series       : ["gpu/pci-0000:01:00.0/load/core","gpu/pci-0000:01:00.0/load/3d","gpu/pci-0000:01:00.0/load/copy","gpu/pci-0000:01:00.0/load/video-decode","gpu/pci-0000:01:00.0/load/video-encode","gpu/pci-0000:01:00.0/fan/percent","gpu/pci-0000:01:00.0/percent/power-limit","gpu/pci-0000:01:00.0/temperature/core"]
charts       : 1
legendSeries : 9
expectedText : True

Seeded: the next start opens the Advanced view on the requested page.
```

Raw output of the four `measure-footprint.ps1` runs, plus the
continuously-visible run:

```
Mode              : window
HistoryMinutes    : 0
CorePercentCpu    : 0.1
AppPrivateMB      : 18.9
WebView2Processes : 6
TotalPrivateMB    : 131.6
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll

Mode              : tray
HistoryMinutes    : 0
CorePercentCpu    : 0.04
AppPrivateMB      : 16.4
WebView2Processes : 0
TotalPrivateMB    : 16.4
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll

Mode              : tray
HistoryMinutes    : 61
CorePercentCpu    : 0.04
AppPrivateMB      : 20.6
WebView2Processes : 0
TotalPrivateMB    : 20.6
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll

Mode              : window
HistoryMinutes    : 61
CorePercentCpu    : 0.06
AppPrivateMB      : 23.0
WebView2Processes : 6
TotalPrivateMB    : 139.6
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll

Mode              : window
HistoryMinutes    : 0
CorePercentCpu    : 0.06
AppPrivateMB      : 23.9
WebView2Processes : 6
TotalPrivateMB    : 149.9
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
```

The last block is the continuously-visible run (`-WarmupSeconds 3660
-SampleSeconds 30`, no `-FillHistoryMinutes`): `HistoryMinutes : 0` only
means the preliminary tray fill was not used here, not that the history was
empty — the window stayed open and visible for the full 3660 s warm-up
before the 30 s sample, so the chart's live, non-decimated tail (up to
about 3600 points per series) had time to build up. The user confirmed the
window stayed visible (never minimised or closed) for the whole 61-minute
run; the window was on a second screen while a game ran on the other
screen, so app CPU for that row was measured with a GPU-heavy process
present, not an idle system.

All five M3 rows meet the budget (app CPU < 1 %; tray < 30 MB; window
< 200 MB total) on all counts, and `VendorModules` lists all three vendor
libraries on every row, so none of the measurements were taken in safe
mode. Two structural choices keep the window under the 200 MB budget: the
30-minute and 1-hour chart windows are decimated to at most 900 points per
series (§4.2), and the chart is capped at 8 series and 2 units at once
(§7.3, D4). The 148 MB uPlot spike figure referenced in the plan (8 series ×
3600 non-decimated points, an M2 build) is prior evidence, not reproduced
against this final build — the closest measurement here, the
continuously-visible row, reached 149.9 MB total with the smaller live-tail
queue from Task 11 rather than a full non-decimated spike.
