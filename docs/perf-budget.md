# Performance budget

## Grafici fluidi — protocollo release e verifica in corso

Stato al 2026-09-27: il gate anticipato del compositor (Task 5) è stato
superato sulla release del commit `f5e54b5` (vedi "Gate anticipato del
compositor" sotto) e la verifica visiva del Task 7 è stata completata
dall'utente sulla release `474da9f` (vedi "Verifica visiva del Task 7"
sotto). Le misure lunghe del Task 7 — storico pieno, tracce di almeno 60 s,
almeno un'ora visibile e tray — non sono state eseguite in questo giro per
decisione dell'utente, perciò i criteri complessivi non sono dichiarati
**soddisfatti**. Le misure
preliminari qui sotto si riferiscono al primo design, a ridisegno continuo,
poi sostituito dall'architettura a compositor verificata nel gate anticipato:
sono conservate come storico e non descrivono lo stato attuale. Dopo la
verifica del Task 7, aggiornare questa sezione con l'esito completo sulla
build finale.

### Misure diagnostiche preliminari

*(Design superato: le misure di questa sottosezione riguardano il primo
design a ridisegno continuo delle serie, prima della riscrittura a
compositor del Task 5. Sono conservate come storico, non come stato
attuale — vedi "Gate anticipato del compositor" più sotto per le misure
sull'architettura corrente.)*

Macchina: Windows 11 Pro 10.0.26200, AMD Ryzen 7 7800X3D (16 processori
logici), 32 GB RAM, NVIDIA RTX 4080 (driver 32.0.16.1714), 2560×1440 a
**164 Hz**; runtime Microsoft Edge WebView2 154.0.4258.37, servizio `oma-service`
connesso e moduli `atiadlxx.dll`, `nvapi64.dll`, `nvml.dll` presenti. L'utente
ha scelto di mantenere il display a 164 Hz. Le righe CPU/memoria sotto usano
30 s di warm-up e 60 s di campionamento della release visibile, senza DevTools
né tracing; `TotalAppCpuValid=True` e 7 processi app+WebView2 in ogni riga.
Sono prove con storico breve, non la verifica finale con storico pieno e dopo
un'ora visibile. La baseline è il precedente eseguibile di `main`, SHA-256
`AE3FB06899E7F8232E6550D206662C8E00B5D2F1C2E56A84BB81F73A05FF9778`.

| Build / vista | CPU app+WebView2 % | Memoria privata MB | Esito del budget |
|---|---:|---:|---|
| Baseline, Avanzata GPU, 1 min, 8 serie verificate | 0,19 | 127,7 | entro i limiti |
| Grafici fluidi, stessa vista | 1,22 | 249,9 | CPU e memoria oltre limite |
| Grafici fluidi, Avanzata GPU, 1 h, 8 serie, storico breve | 1,26 | 251,9 | CPU e memoria oltre limite |
| Baseline, Semplificata | 0,23 | 115,8 | entro i limiti |
| Grafici fluidi, Semplificata | 1,33 | 193,9 | CPU oltre limite |

Una traccia WebView2 separata di 10 s della vista Avanzata con storico breve
ha registrato 618 eventi `DrawFrame` in 9,999 s: circa 61,8/s in media,
intervallo mediano 17,974 ms (circa 55,6 FPS dalla mediana) e p95 19,592 ms,
senza perdita di eventi. Il monitor a 164 Hz quantizza gli intervalli; la
media e la mediana descrivono aspetti diversi della stessa prova. L'utente
ha osservato scorrimento fluido e glow corretto, ma anche un salto del bordo
sinistro al taglio dello storico; il fix `5287167` conserva un campione fuori
vista ed è coperto dai test, ancora da verificare nella release.

Una prova **non rappresentativa della resa finale** con movimento ridotto e
traslazione CSS del canvas uPlot già disegnato ha dato 0,98% CPU e 150,5 MB.
Dimostra che evitare il ridisegno completo può ridurre la memoria, ma la CPU
è troppo vicina all'1% per dichiarare il budget raggiunto. Inoltre la prova
muoveva anche assi ed etichette. La prova continua di un'ora è stata interrotta
quando l'utente ha cambiato la scala da 1 h a 1 min per esaminare la resa;
nessun risultato di soak o storico pieno viene dichiarato.

### Gate anticipato del compositor (Task 5)

Stessa macchina delle misure preliminari sopra: Windows 11 Pro 10.0.26200,
AMD Ryzen 7 7800X3D (16 processori logici), 32 GB RAM, NVIDIA RTX 4080,
display 2560×1440 a **164 Hz** — esplicitamente **non** una prova a 60 Hz —
e runtime WebView2 già indicato sopra. Servizio `oma-service` connesso e
`TotalAppCpuValid=True` in ogni riga. Storico **breve** in entrambe le
misure, non lo storico pieno del Task 7. Viste: Avanzata sulla pagina GPU
`gpu/pci-0000:01:00.0`, finestra 1 min, 8 serie verificate; Semplificata.

| Misura / vista | CPU app+WebView2 % | Memoria privata MB | Esito del budget |
|---|---:|---:|---|
| 037c040, Avanzata GPU, 1 min, 8 serie, storico breve | 1,20 | 163,5 | CPU oltre limite |
| 037c040, Semplificata | 1,50 | 206,6 | CPU e memoria oltre limite |
| f5e54b5, Avanzata GPU, 1 min, 8 serie, storico breve | 0,97 | 154,5 | entro i limiti (margine CPU minimo) |
| f5e54b5, Semplificata | 0,81 | 121,4 | entro i limiti |

**Prima misura del compositor**, 2026-09-27 21:10–21:19, release dal commit
`037c040`, `oma-app.exe` SHA-256
`CD237BC406DFC162E0C10517AE307C2B3609003C3B27B1B904FC02A216E4D7EA`. Le
finestre non sono state osservate dall'utente in questa misura (esecuzioni
automatiche, nessun puntatore sui grafici); le tracce mostrano comunque
`requestAnimationFrame` a 164 Hz, quindi le finestre non erano nascoste.
`TotalAppPercentCpu` valido, 7 processi app+WebView2 in entrambe le righe.
Traccia WebView2 di 60 s: Avanzata 3722 eventi `DrawFrame` (62/s),
intervallo mediano 18,058 ms (≈55,4 FPS), p95 18,445 ms; Semplificata 7241
eventi `DrawFrame` (≈120/s), mediana 6,206 ms, p95 12,225 ms.

Il profiling ha individuato le cause e le correzioni applicate: l'orologio
dei frame dei grafici dava a ogni sottoscrittore una fase propria, così le
quattro sparkline si ridisegnavano su vsync diversi (~120 frame/s in
Semplificata) — corretto con un'unica fase condivisa per frequenza (commit
`a2bc0cc`); i frame fra un campione e l'altro ridisegnavano comunque
qualcosa (la larghezza del marcatore in Avanzata, la geometria SVG in
Semplificata, la proprietà custom dell'offset del cursore scritta a ogni
frame) — corretto limitando i frame intermedi a un solo cambio di
`transform` su livelli già disegnati, con il segmento mantenuto disegnato
nel livello traslato e l'offset del cursore aggiornato solo durante l'hover
(`933c5c0`); le etichette dell'asse X con i secondi sotto 1 min e la
spaziatura delle etichette misurata (`709bed8`, `f5e54b5`). Il thread
principale del renderer costava solo circa 0,18–0,24 % della macchina: la
maggior parte del costo è del compositor/GPU/browser e scala con i frame
disegnati, non con il lavoro JavaScript.

**Seconda misura**, 2026-09-27 22:29–22:37, release dal commit `f5e54b5`,
SHA-256 `C194295C0F3E15B6431B1982EC2FDBC07297CE56BCAA4ECD607BFE980419D4AA`.
Utente presente, finestre visibili, nessun puntatore sui grafici, viste
confermate a vista. Carico in parallelo: OpenCode con un modello llama
locale caricato in VRAM (15,6/16 GB) ma inattivo (nessuna inferenza in
corso). Protocollo: 30 s di warm-up + 60 s di campionamento pulito per
vista, poi una traccia WebView2 separata di 60 s (la corsa di tracing non è
stata usata per CPU/memoria). Avanzata: 0,97 % (valido, 7 processi), 154,5
MB → entro i limiti con margine CPU minimo; traccia 3637 `DrawFrame` in
60,012 s (60,6/s), mediana 18,103 ms (≈55,2 FPS), p95 18,642 ms.
Semplificata: 0,81 %, 121,4 MB → entro i limiti; traccia 3758 `DrawFrame` in
59,982 s (62,7/s), mediana 18,092 ms (≈55,3 FPS), p95 18,531 ms.

Verifica visiva dell'utente: scorrimento fluido in entrambe le viste; punto
bianco intero al bordo destro; segmento mantenuto con lo stesso glow della
sua linea; percorso della sparkline, segmento mantenuto e punto uniti senza
discontinuità; etichette X in formato HH:MM:SS alla finestra di 1 min.
L'utente ha segnalato le tacche dell'asse Y secondario (°C) non allineate
alla griglia: difetto preesistente su `main`, corretto successivamente nei
commit `3423f87` e `44893ce` (cambio dell'asse statico, non rimisurato qui;
il Task 7 misurerà la build finale). Confronto con la baseline di `main`
(grafici statici): 0,19 % / 127,7 MB in Avanzata e 0,23 % / 115,8 MB in
Semplificata. Vale anche qui la nota sulla quantizzazione del refresh già
riportata sopra: a 164 Hz l'intervallo mediano di circa 18 ms corrisponde a
~55 FPS mentre la frequenza media dei `DrawFrame` è di circa 61/s — sono due
aspetti diversi della stessa prova, non un errore.

**Frequenza dei frame come compromesso di leggerezza.** Tutte le misure di
questa sottosezione sono al limite di 60 FPS; l'orologio condiviso dei
grafici è già progettato anche per 30 e 15 FPS. Quando sarà implementata la
schermata delle impostazioni, l'utente potrà scegliere 60, 30 o 15 FPS per i
grafici: frequenze più basse riducono la CPU perché il costo scala con i
frame disegnati. Poiché la leggerezza è uno dei punti di forza principali
dell'app, l'opzione 60 FPS nelle impostazioni dovrà indicare accanto a sé
che aumenta leggermente l'uso di CPU dell'app (su questa macchina circa
+0,6–0,8 punti percentuali rispetto ai grafici statici, dai numeri sopra).

### Verifica visiva del Task 7 (release `474da9f`)

Il 2026-09-27 l'utente ha verificato a vista la release del commit
`474da9f`, `oma-app.exe` SHA-256
`8E5377EF4AF4DA5199C3708D21B8F09E16FAE29B8AAFEF1143F9BFD6F7D90669`, con
esito positivo su tutti i punti: in Avanzata l'asse X alle finestre di 1, 5,
30 e 60 min; punti bianchi, tratti mantenuti e asse °C senza tacche; la
transizione Y; cursore, legenda e serie nascoste. In Semplificata le
sparkline, il ridimensionamento della finestra e il ritorno dalla tray senza
recupero accelerato del tempo trascorso. Il movimento ridotto non è stato
provato dal vivo ed è coperto dai test unitari.

Per decisione dell'utente, in questo giro **non** sono state eseguite le
misure lunghe: riempimento dello storico di 61 min, almeno un'ora visibile
per ciascuna vista, tracce di almeno 60 s con storico pieno e tray. Le righe
della tabella di accettazione qui sotto restano quindi senza esito e i
criteri complessivi non sono dichiarati soddisfatti.

Le correzioni della revisione finale del branch (ridimensionamento senza
ricostruzione e serie nascoste conservate, punto intero delle sparkline,
backing store del canvas riusato, font delle etichette X, punti durante la
transizione Y, niente frame a vuoto, misura della larghezza delle sparkline)
sono successive a questa verifica visiva e non sono state rimisurate.

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
| Semplificata, storico pieno, ≥60 s | non eseguito in questo giro | — | — | — | — | in attesa | non valutato |
| Avanzata, 1 h, 8 serie, storico pieno, ≥60 s | non eseguito in questo giro | — | — | — | — | in attesa | non valutato |
| Semplificata, dopo ≥1 h visibile | non eseguito in questo giro | — | — | — | — | in attesa | non valutato |
| Avanzata, 1 h, 8 serie, dopo ≥1 h visibile | non eseguito in questo giro | — | — | — | — | in attesa | non valutato |
| Tray | non eseguito in questo giro | n/a | n/a | — | — | in attesa | non valutato |

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
| M5a | same machine, PawnIO 2.2.0, release build of the branch, service installed, dynamic tray icon active | window (Advanced view, `oma-service` connected) | 0.96 (7 processes; core 0.04) | 20.5 | 6 | 186.6 | yes |
| M5a | same machine, same conditions | tray | 0.04 | 17.3 | 0 | 17.3 | yes |
| M5b | same machine, Windows 11 Pro 10.0.26300, release build of the branch, service installed, a critical custom rule active (tray icon red) | window (Settings › Rules and alerts) | 0.04 (7 processes; core 0.01) | 20.2 | 6 | 122.6 | yes |
| M5b | same machine, same conditions | window (a view with charts, first run) | 0.95 (7 processes; core 0.03) | 20.1 | 6 | 123.0 | yes |
| M5b | same machine, same conditions | tray (red icon) | 0.003 | 17.8 | 0 | 17.8 | yes |
| M5c | same machine, Windows 11 Pro 10.0.26300, PawnIO 2.2.0, release build of the branch, service installed, all service modules on, CSV log recording every 1 s | tray | 0.04 | 19.1 | 0 | 19.1 | yes |
| M5c | same machine, same conditions | window (Advanced view, log recording, blinking dot) | 0.91 (7 processes; core 0.06) | 20.7 | 6 | 184.8 | yes |
| M5c | same machine, same conditions | window (Settings › CSV log, log recording) | 0.15 (7 processes; core 0.06) | 21.1 | 6 | 130.0 | yes |
| M5c | same machine, same conditions, control run: the same Advanced page, log not recording | window (Advanced view, no log) | 1.01 (7 processes; core 0.08) | 20.7 | 6 | 180.1 | at the limit (see the details) |
| M6b | same machine, Windows 11 Pro 10.0.26300, PawnIO 2.2.0, release build `4608bed`, service (protocol v3) connected, SATA HDD in standby | window (Advanced view, HDD page) | 0.92 (7 processes; core 0.05) | 20.2 | 6 | 166.9 | yes |
| M6b | same machine, same conditions | tray | 0.05 | 18.2 | 0 | 18.2 | yes |
| M6c | same machine, Windows 11 Pro 10.0.26300, PawnIO 2.2.0, release build `54da8a7`, service connected, automatic update check on | window | 0.84 (7 processes; core 0.04) | 20.7 | 6 | 129 | yes |
| M6c | same machine, same conditions | tray | 0.04 | 18.0 | 0 | 18.0 | yes |

Budget: app CPU < 1 % at idle; tray < 30 MB; window open < 200 MB in total.

| Milestone | Machine | Mode | Service CPU % (of machine) | Service Private Bytes |
|---|---|---|---|---|
| M4 | AMD Ryzen 7 7800X3D, B650, 2× DDR5, 1 SATA HDD, 1 SATA SSD, 2 NVMe, RTX 4080 + AMD iGPU, Windows 11 Pro 10.0.26200, PawnIO 2.2.0 | window | 0.03 | 51.6 MB |
| M4 | same machine, same conditions | tray | 0.04 | 52.7 MB |
| M5a | same machine, release build of the branch | window | 0.01 | 63.2 MB |
| M5a | same machine, same conditions | tray | 0.01 | 62.9 MB |
| M5b | same machine, release build of the branch (TjMax, Tdie, NVMe critical warning) | window (Settings › Rules and alerts) | 0.09 | 59.2 MB |
| M5b | same machine, same conditions | tray | 0.1 | 62.4 MB |
| M5c | same machine, release build of the branch, log recording every 1 s | tray | 0.03 | 59.2 MB |
| M5c | same machine, same conditions | window (Advanced view, log recording) | 0.03 | 63.2 MB |
| M5c | same machine, same conditions | window (Settings › CSV log, log recording) | 0.03 | 66.6 MB |
| M5c | same machine, same conditions, control run | window (Advanced view, no log) | 0.02 | 62.6 MB |
| M6b | same machine, release build `4608bed` (protocol v3, disk activity gate), SATA HDD in standby | window (Advanced view, HDD page) | 0.09 | 57.1 MB |
| M6b | same machine, same conditions | tray | 0.03 | 62.5 MB |
| M6c | same machine, release build `54da8a7` (automatic update check on) | window | 0.01 | 63.7 MB |
| M6c | same machine, same conditions | tray | 0.01 | 62.8 MB |

Budget: service CPU < 1 % of the machine; service private bytes < 80 MB — both met.

## M7 — motore dei frame e overlay (spec M7 §11)

Limiti del §11 della spec M7 (invariati dopo lo spike, SD9). Si misurano con
`scripts/measure-footprint.ps1 -Service`, che dalla M7b riporta anche il
processo PresentMon figlio del servizio (`PresentMonCpuPercent`,
`PresentMonPrivateBytesMB`, oppure `PresentMonValid = False` con il motivo
`not running` quando il servizio non lo ha avviato). Il lavoro in più del
servizio è la differenza fra `ServiceCorePercentCpu` con il motore acceso e
quella con il motore spento. Dalla M7c lo script riporta anche, in ogni modo
e senza `-Service`, il processo `oma-overlay.exe` figlio dell'app
(`OverlayCpuPercent` e `OverlayPrivateBytesMB`, dai contatori grezzi come
PresentMon, oppure `OverlayValid = False` con il motivo, `not running` quando
l'overlay è spento o nascosto senza processo). `oma-overlay` non entra in
`TotalAppPercentCpu` (solo l'app e WebView2).

| Stato | Limite | Punto di partenza (spike M7b, SD9) |
|---|---|---|
| Overlay spento | nessun processo in più, costo invariato | PresentMon non gira |
| Overlay acceso, nessun gioco | PresentMon + `oma-overlay` < 0,5% della CPU totale; `oma-overlay` < 40 MB privati | PresentMon 0,006–0,023% sul desktop, 5–6,5 MB privati |
| Overlay visibile in gioco, profilo «Gaming», grafici a 30 FPS | `oma-overlay` + PresentMon + il lavoro in più di app e servizio < 1% della CPU totale; `oma-overlay` < 40 MB | PresentMon fino allo 0,05% (GoW a 148 righe/s, come SYSTEM), 5,5 MB; lettura del CSV e svuotamento stimati dallo 0,04% allo 0,1% (sonda dello spike) |
| Editor aperto | come la finestra principale (< 200 MB con WebView2) | — |

Misure della M7c, da compilare nel task C20 (release, stessa macchina):

| Stato | `oma-overlay` CPU % | `oma-overlay` MB privati | PresentMon CPU % | Lavoro in più di app e servizio | Esito |
|---|---:|---:|---:|---:|---|
| Overlay spento | da misurare in C20 (atteso: nessun processo) | — | — | da misurare in C20 | da misurare in C20 |
| Overlay acceso, nessun gioco | da misurare in C20 | da misurare in C20 | da misurare in C20 | — | da misurare in C20 |
| Overlay visibile in gioco, «Gaming», grafici a 30 FPS | da misurare in C20 | da misurare in C20 | da misurare in C20 | da misurare in C20 | da misurare in C20 |

Le cifre dello spike vengono da `flush-probe` (60 s, 16 processori logici;
dettagli in `docs/superpowers/references/m7/spike-findings.md`, S2), non dal
servizio. La misura sul servizio installato, con il motore acceso e senza
gioco (PresentMon più il lavoro del servizio sotto lo 0,5%), è la verifica dal
vivo V6 della M7b, eseguita il 2026-10-05 (`docs/follow-ups.md`): servizio più
PresentMon circa 0,03%. `oma-overlay` arriva con la M7c; le sue misure sono
nella tabella sopra.

## M6c measurement details

Measured 2026-10-04 on the development machine (16 logical processors,
Windows 11 Pro 10.0.26300, PawnIO 2.2.0) with the release build of commit
`54da8a7` (`target\release\oma-app.exe`, built by `pnpm tauri build --bundles
nsis`), `scripts/measure-footprint.ps1 -Service` (`-Minimized` for the tray
run) with the script's defaults (15 s warm-up, 30 s sample), the automatic
update check enabled, the service connected. Raw output in
`.superpowers/sdd/2026-10-04-m6c-report-aggiornamenti/u7-{window,tray}.log`
(not tracked).

```
Mode              : window
CorePercentCpu    : 0.04
TotalAppPercentCpu: 0.84 (7 processes)
AppPrivateMB      : 20.7
WebView2Processes : 6
TotalPrivateMB    : 129
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
Service CPU       : 0.01 %
Service Private   : 63.7 MB

Mode              : tray
CorePercentCpu    : 0.04
TotalAppPercentCpu: 0.04
AppPrivateMB      : 18.0
WebView2Processes : 0
TotalPrivateMB    : 18.0
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
Service CPU       : 0.01 %
Service Private   : 62.8 MB
```

All budget items are met. The update check (one timer, one HTTPS request per
day at most) and the gear dot add no visible cost: the core stays at 0.04 % and
the tray at 18.0 MB. The service is at 0.01 % and under 64 MB. No limit was
widened.

## M6b measurement details

Measured 2026-10-04 at about 02:30 on the development machine (16 logical
processors, Windows 11 Pro 10.0.26300, PawnIO 2.2.0) with the release build of
the branch (`target\release\oma-app.exe` of `4608bed`, the same commit as the
installer the user ran for the live checks), `scripts/measure-footprint.ps1
-Service` (`-Minimized` for the tray run) with the script's defaults (15 s
warm-up, 30 s sample), the branch's `oma-service` (version still 0.3.0, protocol v3)
connected, no history fill. The SATA HDD was in standby (turned off by
Windows) during both runs; the view saved in the profile was the Advanced view
on the HDD page. The app exited by itself after each run. Raw output in
`target/spike/m6b/task15-footprint-{window,tray}.log` (not tracked).

```
Mode              : window (Advanced view, HDD page)
CorePercentCpu    : 0.05
TotalAppPercentCpu: 0.92 (7 processes)
AppPrivateMB      : 20.2
WebView2Processes : 6
TotalPrivateMB    : 166.9
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
Service CPU       : 0.09 %
Service Private   : 57.1 MB

Mode              : tray
CorePercentCpu    : 0.05
TotalAppPercentCpu: 0.05
AppPrivateMB      : 18.2
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
Service CPU       : 0.03 %
Service Private   : 62.5 MB
```

All budget items are met. The window run is in line with M5c (0.91 to 1.01 %
on a page with charts): the per-sensor quality and the disk state tags add
no visible cost (no control run without them), and the core stays at 0.05 %.
The service's disk activity gate (counters read with access 0, no command to
a resting disk) leaves the service at 0.03 to 0.09 % and under 63 MB, in line
with M5c (0.02 to 0.03 %, 59.2 to 66.6 MB) and well inside its budget. No limit
was widened.

## M5c measurement details

This is the final M5 measurement (M5a, M5b and M5c together).

Measured 2026-09-30 on the development machine (16 logical processors,
Windows 11 Pro 10.0.26300, PawnIO 2.2.0) with the release build of the branch
(`target\release\oma-app.exe`), `scripts/measure-footprint.ps1 -Service
-WarmupSeconds 60` (`-Minimized` for the tray run), `oma-service` connected,
all service modules on, no history fill. During the 60 s warm-up the user
started the log, with the hotkey or the REC button, at the default interval
(every tick, 1 s); 85 rows were logged during the tray run. The window runs
used the Advanced view and Settings › CSV log as listed in the table.

```
Mode              : tray (log recording)
CorePercentCpu    : 0.04
TotalAppPercentCpu: 0.04
AppPrivateMB      : 19.1
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
Service CPU       : 0.03 %
Service Private   : 59.2 MB

Mode              : window (Advanced view, log recording, blinking dot)
CorePercentCpu    : 0.06
TotalAppPercentCpu: 0.91 (7 processes)
AppPrivateMB      : 20.7
WebView2Processes : 6
TotalPrivateMB    : 184.8
Service CPU       : 0.03 %
Service Private   : 63.2 MB

Mode              : window (Settings › CSV log, log recording)
CorePercentCpu    : 0.06
TotalAppPercentCpu: 0.15 (7 processes)
AppPrivateMB      : 21.1
WebView2Processes : 6
TotalPrivateMB    : 130.0
Service CPU       : 0.03 %
Service Private   : 66.6 MB

Mode              : window (Advanced view, log NOT recording: control run)
CorePercentCpu    : 0.08
TotalAppPercentCpu: 1.01 (7 processes)
AppPrivateMB      : 20.7
WebView2Processes : 6
TotalPrivateMB    : 180.1
Service CPU       : 0.02 %
Service Private   : 62.6 MB
```

The tray, the Settings page and the service are within budget. The total app
CPU on a page with charts is at the 1 % line: the control run on the same
Advanced page without recording reads 1.01 %, the same level as M5a (0.96 %)
and M5b (0.95 %), so it is the WebView2 chart rendering and not the log (0.91 %
with the log recording and the dot blinking). The core stays under 0.1 %
in every run (0.04 to 0.08 %). The log costs nothing measurable: 0.04 % in the
tray with a row written every second, and the service is unaffected (0.02 to
0.03 %).

Memory on the Advanced page (180.1 MB without the log, 184.8 MB with it) matches
M5a's Advanced view (186.6 MB); the 123 MB of M5b was a lighter page. All
memory figures stay under the 200 MB window budget. No limit was widened.

## M5b measurement details

Measured 2026-09-30 on the development machine (16 logical processors,
Windows 11 Pro 10.0.26300, PawnIO 2.2.0) with the release build of the branch
(`target\release\oma-app.exe`, the same binary as the installer built the
same day), `scripts/measure-footprint.ps1 -Service`, `oma-service` connected,
no history fill. A custom rule "RAM used above 10 %" (critical, notification
off) was active throughout, so the tray icon was red and the tooltip started
with the verdict. The window runs used `-WarmupSeconds 45` and `60`, so the
user could open the page to measure before sampling started.

```
Mode              : tray (red icon)
CorePercentCpu    : 0
TotalAppPercentCpu: 0.003
AppPrivateMB      : 17.8
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
Service CPU       : 0.1 %
Service Private   : 62.4 MB

Mode              : window (Settings › Rules and alerts)
CorePercentCpu    : 0.01
TotalAppPercentCpu: 0.04 (7 processes)
AppPrivateMB      : 20.2
WebView2Processes : 6
TotalPrivateMB    : 122.6
Service CPU       : 0.09 %
Service Private   : 59.2 MB
```

All budget items are met. In a first window run the window stayed on a view
with charts instead of Settings (confirmed by the user): 0.95 % across the seven
processes, 123.0 MB, in line with the Advanced view at M5a (0.96 %). The rules
page itself, which polls the rule status every second, costs almost nothing
(0.04 %); the WebView2 chart rendering remains the item closest to the 1 %
limit.

The rules engine is evaluated inside the core tick. Task 7 measured `evaluate`
on this machine in a release build (`crates/oma-core/tests/rules_alloc.rs`: a
2000-sensor schema, the 12 default rules plus 200 custom ones): 2.1 µs per
evaluation with every rule `ok` and 16.4 µs with more than 200 alerts active,
with 0 heap allocations in both steady states.

## M5a measurement details

Measured 2026-09-30 on the development machine (16 logical processors,
Windows 11 Pro 10.0.26200, PawnIO 2.2.0) with the release build installed by
the NSIS setup, `scripts/measure-footprint.ps1 -Service`, `oma-service`
connected, no history fill. In the tray run the dynamic tray icon (number or
bar, tooltip) was active.

```
Mode              : tray
CorePercentCpu    : 0.04
TotalAppPercentCpu: 0.04
AppPrivateMB      : 17.3
VendorModules     : atiadlxx.dll, nvapi64.dll, nvml.dll
Service CPU       : 0.01 %
Service Private   : 62.9 MB

Mode              : window (Advanced view)
CorePercentCpu    : 0.04
TotalAppPercentCpu: 0.96 (7 processes)
AppPrivateMB      : 20.5
WebView2Processes : 6
TotalPrivateMB    : 186.6
Service CPU       : 0.01 %
Service Private   : 63.2 MB
```

All budget items are met (app CPU < 1 %; tray < 30 MB; window < 200 MB in
total; service CPU < 1 %, service private bytes < 80 MB). The window row is
the closest to the limit: 0.96 % of the machine across the seven processes
and 186.6 MB in total, up from 141.2 MB at M4. The Settings view was not measured
separately: the Advanced view is the heavier one. The service private bytes grew by about
10 MB over M4 (51.6 to 63.2 MB) but stay under the 80 MB budget.

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
