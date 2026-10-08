# M8 — Prestazioni: benchmark, stress test e classifica — design di dettaglio

Data: 2026-10-06. Stato: approvato a sezioni nel brainstorming, da rivedere per intero dall'utente prima dei piani.

Riferimenti:

- spec generale `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`, budget `docs/perf-budget.md`;
- pipe, processo figlio e firma seguono la M7 (`docs/superpowers/specs/2026-10-04-m7-manutenzione-overlay-design.md`) e la M6a;
- client HTTP e nota sulla privacy della M6c (`docs/superpowers/specs/2026-10-04-m6c-report-aggiornamenti-design.md`);
- ricerca del brainstorming (con le fonti): `docs/superpowers/references/m8/research-cpu.md`, `research-gpu.md`, `research-disk.md`, `research-leaderboard.md`.

## 1. Intento e confini

L'utente vuole una sezione nuova dell'app dedicata alle prestazioni dell'hardware, con due funzioni.

- **Benchmark.** Misura CPU, GPU e disco e dà un punteggio che colloca l'hardware in una classifica. Ogni misura si vede su due contagiri in stile automobilistico, ispirati agli screenshot portati dall'utente ma nello stile Synthwave dell'app.
- **Stress test.** Si sceglie il componente, poi l'obiettivo: una verifica normale oppure la stabilità di un overclock. Il catalogo delle modalità è ricco, e ogni modalità verifica i propri risultati e segnala gli errori di calcolo. Il test si può fermare in ogni momento. La sessione si salva sempre, finita o interrotta, e resta in una cronologia.

Successo: chi fa overclock o undervolt capisce in pochi clic se il sistema è stabile e quale core cede. Chiunque vede quanto va il proprio hardware rispetto ad altri. Chi non conosce i termini tecnici capisce cosa sta facendo e che tipo di test sta girando.

### 1.1 Decisioni del brainstorming

| # | Decisione |
|---|---|
| D1 | Divisione per componente: **M8a** fondamenta, CPU e RAM; **M8b** GPU; **M8c** disco; **M8d** classifica e condivisione. Benchmark e stress di un componente usano gli stessi carichi. |
| D2 | Nome della vista: **«Prestazioni»**, con i gruppi **«Punteggio»** e **«Stress test»**. Il «Benchmark» dei giochi della M7d mantiene il suo nome. |
| D3 | **Stop termico** automatico con soglia modificabile, disattivabile solo con una conferma. Senza servizio lo stress della CPU parte lo stesso, con un avviso che manca la protezione termica della CPU. |
| D4 | Il test d'overclock della CPU copre tutti i core insieme, un core alla volta, RAM e controller di memoria, e il carico variabile. La RAM entra quindi nella M8a. |
| D5 | Anche la verifica normale controlla gli errori di calcolo, con gli stessi carichi verificati. I due profili cambiano scaletta, durata, ciclo per core e severità. |
| D6 | Contagiri in **stile C, «Ibrido con riferimento»** (mockup `gauges.html` del brainstorming): ghiera metallica, quadrante in carbonio tinto viola, ago rosa, display a matrice di punti ciano, arco acceso sotto le tacche, segno ▲ di riferimento sulla ghiera. |
| D7 | Il ▲ indica un riferimento scelto da un menu: il proprio record (predefinito), l'ultima misura o un modello della tabella. |
| D8 | Stress test configurato con una **procedura guidata a quattro passi**: componente, obiettivo, durata, riepilogo con «Personalizza». |
| D9 | Le schermate «durante il test» e «risultato» seguono il mockup `stress-run.html`, approvato senza modifiche. |
| D10 | **Requisito (importante per l'utente):** ogni termine tecnico delle pagine ha un tooltip in parole semplici, e ogni modalità spiega cosa fa e che tipo di test è. |
| D11 | Se si chiude la finestra, il test continua: la tray lo segnala, il menu ha «Ferma il test» e alla fine arriva un toast con il verdetto. «Esci» chiede conferma e salva la sessione come interrotta. |
| D12 | I carichi girano in un **processo ausiliario `oma-load.exe`** (approccio 1). Sono stati scartati i thread dentro l'app e il servizio. |
| D13 | I punteggi di CPU e GPU sono in **punti** (per la CPU una scala fissa con il 7800X3D di taratura a 1500, §4.6; per la GPU una scala fissa con la RTX 4080 di taratura a 1500, §5.2), con le velocità vere di ogni carico in una tabella di dettaglio. I contagiri del **disco** sono in **MB/s**, e i punti del disco servono alla classifica. |
| D14 | La classifica parte da una tabella di riferimento inclusa nell'app. La community la alimenta con **issue GitHub precompilate** dall'app. Un'Action valida e aggrega le issue e pubblica la tabella su GitHub Pages. L'app la **scarica da sola** (al massimo una volta al giorno, attivo di default, si può spegnere). **Rivista nel §7.6:** al posto di issue, Action e Pages, un invio anonimo a un Cloudflare Worker. |
| D15 | Codice di terzi da adattare, non solo da studiare: **FIRESTARTER** (GPL-3.0-or-later), **OpenDCDiag** (Apache-2.0), **memtest_vulkan** (Zlib) (§12). |

### 1.2 Scomposizione in piani

| Piano | Contenuto |
|---|---|
| **M8a** | Fondamenta: vista Prestazioni, `oma-load.exe` e protocollo, cronologia, diario dei crash, WHEA, stop termico, tray, tooltip, impostazioni. Stress di CPU e RAM, benchmark della CPU e contagiri. Se il piano supera la dimensione abituale si divide in **M8a1** (fondamenta e stress di CPU e RAM) e **M8a2** (benchmark della CPU e contagiri). |
| **M8b** | GPU: spike iniziale (§5.6), benchmark Calcolo e Grafica, stress S1–S9. |
| **M8c** | Disco: benchmark Lettura e Scrittura, stress N1–N4 e V1–V4. Un piano unico (§6.4). |
| **M8d** | Classifica: tabella di riferimento, pagina Classifica, esportazione, condivisione anonima, download automatico. Due piani: **M8d1** il server (Cloudflare Worker), **M8d2** l'app (§7.6). |

Ogni piano ha il suo branch `feat/m8x-…`. La release la decide l'utente alla fine di ogni sotto-milestone.

### 1.3 Fuori da questa spec

- K6 (trasformata mista intero/FP in stile VT3) e la variante AVX-512 IFMA di K5;
- le modalità GPU che richiedono D3D12: code concorrenti (S10), FP16/matrici (S11), ray tracing (S12);
- un backend online proprio (Cloudflare o simili) e la classifica per utente;
- lettura diretta di MCA/MSR, test di tutta la RAM fisica, pagine grandi garantite, priorità real-time;
- controllo di tensioni e frequenze: l'app misura e non regola;
- una modalità «sostenuta» del benchmark (10 min): il comportamento termico lo mostra lo stress test;
- Linux.

## 2. Architettura

### 2.1 Processi e responsabilità

- **`oma-app`** orchestra:
  - costruisce il piano delle fasi con codice puro (`oma-core::load`) e lo manda al processo dei carichi;
  - legge i sensori dal proprio scheduler per lo stop termico e per il riepilogo;
  - scrive il diario dei crash e la cronologia;
  - aggiorna la tray e manda i toast.
- **`oma-load.exe`** (crate `crates/oma-load`) esegue:
  - si avvia solo per un benchmark o uno stress test ed esce alla fine;
  - riceve il piano, rileva la topologia (§4.4) e i set d'istruzioni, ed esegue le fasi;
  - ogni secondo manda progressi, misure ed esito delle verifiche, e segnala subito ogni errore;
  - fa da sentinella sui propri thread: un thread che non avanza da 10 s viene segnalato come «bloccato sul core N».
- **Divisione del codice:**
  - i kernel sono portabili e testati con `cargo test` (moduli senza codice Windows);
  - il codice Windows del processo sta dentro `oma-load`: affinità, CPU Sets, EcoQoS, D3D11 nella M8b, I/O senza buffer nella M8c. È lo stesso schema di `oma-overlay`;
  - la lettura del registro eventi (WHEA, BugCheck, Kernel-Power) sta in `oma-win`, perché la usa l'app.
- **Priorità.** I thread di carico girano a `THREAD_PRIORITY_BELOW_NORMAL`: il desktop resta reattivo e il carico resta pieno. L'EcoQoS è spento sui thread di carico (`ThreadPowerThrottling` con `StateMask = 0`).
- **Nome neutro.** `oma-load.exe` non richiama nessuno strumento noto, così il driver non riconosce il test e non lo limita (ricerca GPU, §1: è già successo con FurMark e OCCT).

### 2.2 Pipe e protocollo (`oma-ipc::load`)

- **Pipe.** È quella dell'overlay:
  - nome `\\.\pipe\OpenMonitorAdvanced-Load-<uuid v4>`;
  - creata dall'app con `FILE_FLAG_FIRST_PIPE_INSTANCE`, DACL solo per l'utente corrente e `PIPE_REJECT_REMOTE_CLIENTS`;
  - all'avvio l'app controlla che il PID del client sia quello del figlio appena avviato.
- **Job Object.** Il figlio sta in un Job Object con `KILL_ON_JOB_CLOSE`, così muore se l'app cade.
- **Regole del protocollo.** Sono quelle del protocollo del servizio: chiavi sempre presenti (`nil` per i valori assenti), enumerati come stringhe, nessun `deny_unknown_fields`, `LoadMessage::validate` dal lato ricevente. MessagePack con framing. Versione propria, `LOAD_PROTOCOL_VERSION = 1`, con fixture in `protocol/fixtures/load/`.
- **Messaggi dall'app:** `Hello`, `Run { plan }`, `Stop`.
- **Messaggi dal processo:**
  - `Hello`, con versione e set d'istruzioni;
  - `Topology`, con core logici e fisici, `EfficiencyClass`, `LastLevelCacheIndex` e cache;
  - `Progress` (1 Hz): fase, tempo, velocità del carico in corso, numero di verifiche, stato dei core;
  - `Error`: kernel, core logico e fisico, iterazione, valore atteso e ottenuto (hash), seme;
  - `PhaseDone`;
  - `Finished`: esito e riepilogo del processo.
- **Codici d'uscita:**
  - 0: fine regolare;
  - 1: argomenti;
  - 2: connessione;
  - 3: `Hello` incompatibile;
  - 4: device GPU perso (M8b);
  - 5: errore d'I/O non recuperabile (M8c).

  Ogni altro codice, come `STATUS_ILLEGAL_INSTRUCTION` o `STATUS_ACCESS_VIOLATION`, vale «processo chiuso male».
- **Percorso dell'eseguibile.** In sviluppo l'app cerca `oma-load.exe` accanto al proprio eseguibile (`target\debug\`); installato sta in `$INSTDIR\oma-load.exe`.

### 2.3 Esiti

| Esito | Quando | Verdetto mostrato |
|---|---|---|
| `passed` | Fine del piano, zero errori, nessun evento WHEA. | «Superato» |
| `marginal` | Fine del piano, zero errori di calcolo, ma eventi WHEA corretti (ID 19 o 17) durante il test. | «Superato con avvisi: errori corretti dall'hardware» |
| `errors` | Uno o più errori di calcolo o di dati. | «Errori trovati» o «Instabile · core N» |
| `crashed` | Il processo si è chiuso con un codice anomalo. | «Instabile: il test si è chiuso» |
| `hung` | Pipe muta da più di 5 s, oppure un thread bloccato. | «Instabile: il test si è bloccato» |
| `device_lost` | Device GPU perso (M8b), con `GetDeviceRemovedReason`. | «Instabile: la GPU si è azzerata» |
| `system_crash` | Diario aperto al riavvio (§2.4). | «Interrotto da un crash del sistema durante …» |
| `stopped_user` | «Ferma e salva», «Ferma il test» dalla tray, oppure «Esci» confermato. | «Fermato da te» |
| `stopped_thermal` | Lo stop termico è scattato (§2.6). | «Fermato: temperatura a N °C» |
| `stopped_disk_full` | Spazio esaurito durante un test del disco (M8c, §6.4). | «Fermato: spazio su disco esaurito» |
| `suspended` | Il PC è andato in sospensione durante il test. | «Interrotto dalla sospensione» |
| `failed_to_start` | `oma-load.exe` assente o incompatibile, disco pieno, nessuna GPU adatta. | «Non avviato: …», con la causa |

`crashed`, `hung`, `device_lost` e `system_crash` contano come instabilità. Nel benchmark ogni esito diverso da `passed` o `marginal` rende la misura non valida.

### 2.4 Diario dei crash

- **Cosa contiene.** `journal.json` in `%LOCALAPPDATA%\OpenMonitorAdvanced\performance\`: id della sessione, descrizione del piano, fase, kernel, core in prova, istante dell'ultimo aggiornamento, `cleanEnd: false`.
- **Quando si scrive.** L'app lo riscrive con file temporaneo, `FlushFileBuffers` e rinomina atomica a ogni cambio di fase o di core, e ogni 30 s. A fine sessione lo cancella.
- **All'avvio.** Se il diario esiste, la sessione indicata diventa `system_crash`. L'app legge gli eventi del registro di Sistema fra l'ultimo aggiornamento del diario e l'avvio: WHEA-Logger 18 e 19, BugCheck 1001, Kernel-Power 41. Li aggiunge al dettaglio («crash di sistema durante il core 5, fase FFT piccole AVX2, dopo 2 min 14 s; evento WHEA 18 alle 21:47»). La sessione si salva con i dati che c'erano nel diario e nell'ultimo salvataggio intermedio.
- **Salvataggio intermedio.** La sessione stessa (§8.1) si riscrive ogni 60 s, così dopo un crash restano i campioni fino a quel punto.

### 2.5 WHEA dal vivo

- **Come si legge.** `oma-win` interroga il canale System con `EvtQuery` e la query XPath sul provider `Microsoft-Windows-WHEA-Logger`, senza privilegi: l'SDDL predefinito del registro System dà lettura agli Interactive Users (ricerca CPU §2.5).
- **Quando.** Si fotografa l'ultimo record all'avvio del test. Poi si interroga ogni 5 s e una volta alla fine, perché i record possono arrivare in ritardo.
- **Cosa si conta.** Gli eventi si contano per ID e, per l'ID 19, per APIC ID. L'APIC ID indica il processore logico che ha segnalato l'errore, non per forza il colpevole, e il tooltip lo dice.
- **Registro non leggibile** (criterio di gruppo restrittivo): l'avviso «Errori hardware non leggibili» compare nel riepilogo, e il test continua.

### 2.6 Sicurezza

- **Stop termico.**
  - **Soglie:**
    - CPU: Tjmax − 5 °C se il servizio pubblica il Tjmax, altrimenti 95 °C;
    - GPU: 90 °C sul core;
    - disco NVMe: la soglia d'allarme (WCTEMP) se è nota, altrimenti 70 °C.
  - **Come scatta.** La temperatura si confronta con la soglia a ogni campione dei sensori. Lo stop scatta dopo 2 campioni consecutivi sopra la soglia: l'app manda `Stop` e salva `stopped_thermal`.
  - **Sensore assente.** Se il sensore manca per più di 10 s, compare l'avviso «Temperatura non disponibile» senza fermare il test.
  - **Senza servizio.** Lo stress della CPU parte con un avviso nel riepilogo e nel diario degli eventi. Se il servizio cade durante il test, il test continua e compare lo stesso avviso.
- **Un solo test alla volta.** Benchmark e stress si escludono a vicenda; i pulsanti di avvio sono disattivati mentre un test gira.
- **Avviso sui rischi alla prima volta:** calore e consumi, e il consiglio di salvare il lavoro aperto prima di un test d'overclock, perché un crash del sistema lo perde. Si può non far più comparire.
- **Sospensione.** Durante un test l'app chiama `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`. Se il PC va comunque in sospensione (coperchio chiuso, pulsante), alla ripresa la sessione si chiude come `suspended`.
- **Tray e uscita.** La tray segnala il test in corso e offre «Ferma il test» (D11). «Esci» chiede conferma; `--quit` ferma il test e salva la sessione come `stopped_user`, senza chiedere.

## 3. Vista Prestazioni e interfaccia comune

### 3.1 Struttura

- **Selettore.** È la terza voce del selettore della barra superiore (Semplice, Avanzata, Prestazioni). All'avvio non diventa mai la vista predefinita: `view.last` ricorda solo Semplice e Avanzata, come per le impostazioni.
- **Barra laterale:**
  - gruppo **Punteggio**: CPU; GPU (una voce per scheda, M8b); Disco (M8c); Classifica (M8d);
  - gruppo **Stress test**: «Nuovo test», che diventa «In corso ●» mentre un test gira, e «Cronologia».

  Le voci delle sotto-milestone non ancora fatte non compaiono.

### 3.2 Pagina di punteggio (CPU, GPU, Disco)

- **In alto:** titolo («CPU Benchmark»), i due contagiri, la barra a segmenti (un segmento per carico e ripetizione) e il pulsante Avvia.
- **Menu del riferimento ▲** (D7): il mio record (predefinito), l'ultima misura, oppure un modello della tabella (M8d).
- **Sotto i contagiri:**
  - la tabella di dettaglio con la velocità di ogni carico, nella sua unità vera e con un tooltip;
  - gli avvisi di validità;
  - le azioni «Esporta JSON» e «Condividi su GitHub» (M8d).
- **«Le tue misure».** Elenco delle misure di quel componente, con data, punteggi, avvisi ed eliminazione.

### 3.3 Contagiri (stile C)

- **Disegno.** SVG in un componente Svelte (`Gauge.svelte`), con la geometria in una funzione pura (`app/src/lib/performance/gauge.ts`):
  - arco di 270°, da 135° a 405°, con lo zero in basso a sinistra;
  - 50 tacche (maggiori ogni 10, medie ogni 5) ed etichette sulle tacche maggiori;
  - arco acceso dal ciano (`--accent-2`) al rosa (`--accent`) fino al valore;
  - ago rosa e ▲ bianco sulla ghiera;
  - display a matrice di punti ciano con le cifre «fantasma».

  I colori vengono dai token di `theme.css`.
- **Font.** Orbitron per titolo ed etichette, Share Tech Mono per il display. Entrambi sono OFL e si includono nell'app come file locali: niente Google Fonts, la CSP non cambia.
- **Scala.** Si sceglie prima della misura: il primo «numero tondo» della serie 1-2-2,5-5 × 10ⁿ sopra 1,1 × max(record, riferimento, stima). La stima è 1500 per la CPU (§4.6); per la GPU è 1500 su una scheda dedicata e 20 su una integrata, e vale solo finché non ci sono né record né riferimento (decisione del piano M8b2: con 1500 fisso una iGPU, che fa pochi punti, vedrebbe l'ago sempre a zero); per il disco è il massimo teorico del tipo di bus. Durante la misura la scala può solo crescere.
- **Movimento.**
  - **Durante la misura:** l'ago segue la velocità dal vivo del carico in corso. Per CPU e GPU la velocità è già trasformata in punti dal rapporto con la velocità di riferimento (la scala fissa, §4.6 e §5.2); per il disco è in MB/s.
  - **Alla fine:** l'ago si ferma sul risultato.
  - **Animazione:** una media esponenziale breve via `requestAnimationFrame`, solo mentre la pagina è visibile e l'ago non è fermo. Con `prefers-reduced-motion` l'ago salta al valore.

### 3.4 Procedura guidata dello stress test

1. **Componente.** CPU (modello), RAM (quantità), una voce per GPU (M8b) e Disco (M8c). Le voci non disponibili mostrano il motivo, per esempio «nessuna GPU adatta».
2. **Obiettivo.** Due riquadri grandi, spiegati a parole:
   - «Verifica normale»: voglio sapere se il PC regge carichi lunghi senza surriscaldarsi o sbagliare calcoli;
   - «Stabilità overclock»: ho cambiato frequenze, tensioni o Curve Optimizer e voglio trovare gli errori.
3. **Durata.** Le durate preimpostate del profilo (§4.5, §5.4, §6.3).
4. **Riepilogo.**
   - Mostra le fasi previste, ciascuna con il tooltip e la durata.
   - Mostra gli avvisi: senza servizio, set d'istruzioni rilevato, quota di RAM, dati che verranno scritti sul disco.
   - **«Personalizza»** permette di scegliere:
     - quali modalità includere e con quale durata ciascuna;
     - il set d'istruzioni (automatico, AVX-512, AVX2, SSE2);
     - i thread (tutti, oppure uno per core);
     - «Fermati al primo errore».
   - Il pulsante è **Avvia**.

La cronologia offre «Ripeti il test», che riparte dalla stessa configurazione senza rifare i passi.

### 3.5 Durante il test e risultato

Seguono il mockup approvato (D9).

**Durante il test:**
- intestazione con obiettivo e componente, stato a pillola, tempo trascorso e totale, «Ferma e salva»;
- barra delle fasi con le etichette;
- cinque riquadri: temperatura (massimo e soglia di stop), potenza, clock, errori di calcolo (con il numero di verifiche), errori WHEA;
- grafico di temperatura e potenza degli ultimi 10 minuti, con i componenti di grafico esistenti;
- griglia dei core nelle fasi «un core alla volta»;
- diario degli eventi.

**Risultato:**
- verdetto con fase, kernel, core, tempo, clock e temperatura al momento dell'errore, più un consiglio a parole («con Curve Optimizer, di solito si riduce l'offset di quel core»);
- azioni «Riprova solo il core N» (un piano con il solo ciclo su quel core), «Ripeti il test» ed «Esporta (JSON)»;
- riepilogo della sessione e stato di ogni core.

### 3.6 Cronologia

- **Lista:** data, componente, obiettivo, durata, verdetto. Si filtra per componente; ogni voce apre il dettaglio e si elimina con conferma.
- **Limite:** al massimo **500 file per tipo** (sessioni e punteggi). Oltre si eliminano i più vecchi, all'avvio e dopo ogni salvataggio.

### 3.7 Tooltip e glossario (D10)

- **Componente.** Un componente unico `Term.svelte`: il termine appare sottolineato a puntini, si raggiunge con Tab (`tabindex="0"`) e mostra la spiegazione al passaggio del mouse e con il focus. È legato con `aria-describedby`, come i badge della fonte della vista Avanzata.
- **Testi.** Stanno in `glossary.<termine>` in `en.json` e `it.json`, con le stesse chiavi.
- **Cosa coprono:**
  - **modalità:** ogni modalità (K1–K10, S1–S9, B1–B2, N1–N4, V1–V4) ha `glossary.mode.<id>`, con cosa fa, cosa sollecita e che tipo di test è;
  - **set d'istruzioni** e **termini tecnici** delle pagine, per esempio: FFT, NTT, Linpack, FMA, AVX-512, AVX2, SSE2, SMT, CCD, core P/E, Tjmax, WHEA, Curve Optimizer, PBO, EXPO/XMP, VRM, TDR, VRAM, cache SLC, IOPS, QD/coda, SEQ1M/RND4K, MB/s, TFLOPS, GB/s, punti, percentile, versione del punteggio.
- **Test (Vitest).** Fallisce se:
  - una modalità del catalogo o un set d'istruzioni non ha la sua voce;
  - un termine marcato nelle pagine non ha la chiave;
  - le chiavi di `en` e `it` non coincidono.

### 3.8 Impostazioni (sezione `performance` di `settings.json`, pagina Impostazioni › Prestazioni)

| Chiave | Predefinito | Note |
|---|---|---|
| `thermalStop` | `true` | spegnerlo chiede conferma |
| `cpuStopC` | `null` (= Tjmax − 5 o 95) | da 60 a 110 |
| `gpuStopC` | `90` | da 60 a 110 (M8b) |
| `diskStopC` | `null` (= WCTEMP o 70) | da 40 a 90 (M8c) |
| `stopOnFirstError` | `null` (= quello del profilo) | |
| `ramSharePercent` | `70` | da 10 a 90; restano liberi almeno 2 GB |
| `communityTable` | `true` | download della classifica (M8d) |
| `diskFolder` | `null` | ultima cartella del test del disco (M8c, §6.4) |
| `riskNoticeSeen` | `false` | |

La lettura e la scrittura passano per il modulo delle impostazioni esistente (`oma-core::settings`), con patch e valori fuori intervallo riportati nei limiti.

### 3.9 Tray

- **Icona.** Durante un test l'icona porta un segno, con un tooltip «Stress test in corso: …».
- **Menu:**
  - «Ferma il test»;
  - «Apri il test in corso», che porta alla vista Prestazioni.
- **Toast.** Alla fine arriva un toast con il verdetto; un clic apre il risultato. Per il benchmark il toast arriva solo se la finestra non è visibile.

## 4. M8a — CPU e RAM

### 4.1 Catalogo dei kernel

Tutti i kernel sono scritti in Rust con `core::arch`, scelgono a runtime il set d'istruzioni (AVX-512 → AVX2+FMA → SSE2, con `is_x86_feature_detected!`) e **non generano codice a runtime**. Gli operandi restano limitati e non banali, perché i valori influiscono sulla potenza.

| Id | Nome nell'interfaccia | Carico | Dati | Sollecita | Verifica |
|---|---|---|---|---|---|
| K1 | Carico massimo (FMA) | catene FMA con una quota di load/store in L1/L2, a gruppi d'istruzioni come FIRESTARTER | L1/L2 | potenza, VRM, raffreddamento, tenuta del boost | hash CRC32 dei registri vettoriali per iterazione, confrontato con i thread vicini ad anello; con un solo thread, contro il riferimento iniziale |
| K2 | FFT piccole · core e cache | FFT complessa in doppia precisione, avanti e indietro, in place; variante «minima» in L1 | 50–75% della L2 per thread | unità FP, L1/L2 | risultato identico bit per bit fra thread e iterazioni, controllo della somma (in stile SUMINP/SUMOUT) e andata e ritorno |
| K3 | FFT grandi · memoria | come K2 | almeno 4 × L3 totale, 256 MB–1 GB per thread nella quota di RAM | controller di memoria, Infinity Fabric, uncore | come K2 |
| K4 | Misto (blend) | dimensioni di K2 a rotazione da L1 a RAM | L1 → RAM | passaggi fra cache e memoria | come K2 |
| K5 | Interi esatti (NTT) | NTT modulare a 64 bit (primo vicino a 2⁶³), moltiplicazioni `mulx` | varianti L2, L3 e RAM | moltiplicatore intero, ALU | andata e ritorno esatti più checksum modulo 2⁶¹ − 1; nessuna tolleranza |
| K7 | Linpack | GEMM a blocchi | adatto a L2/L3 o RAM | FP sostenuto, banda | controllo di Freivalds su matrici a interi piccoli (esatto in f64) e hash di C confrontato fra thread |
| K8 | Crittografia e compressione | catene AES-NI, SHA-NI, CRC32C e CLMUL, compressione e decompressione, ordinamento e codice ricco di salti | L1–L3 | unità a funzione fissa, predittore dei salti, front-end | vettori a risposta nota, andata e ritorno, hash fra thread |
| K9 | Scambio fra core | righe da 64 byte con checksum passate fra core e CCD fissati | L1–L3 | coerenza delle cache, collegamento fra CCD | checksum e numeri di sequenza |
| K10 | Pattern di memoria (RAM) | moving inversions, modulo-20, pattern casuali con seme, indirizzo-nell'indirizzo, CRC durante la copia (in stile stressapptest) | quota di RAM (§3.8) | DRAM, IMC, timing, EXPO/XMP | rilettura e confronto, CRC |

Il tooltip di K10 dice che lavora in modalità utente, cioè sulla memoria che Windows concede al processo e con pagine da 4 KB, e che non sostituisce MemTest86, TestMem5 o Karhu.

### 4.2 Verifica comune

- **Determinismo.** I dati di ogni fase vengono da un seme fisso. Il seme si salva nella sessione e si stampa nell'errore, così l'errore si può riprodurre.
- **Riferimento.** Il risultato di riferimento lo calcolano **tre core diversi** all'inizio della fase, e i tre devono coincidere. Altrimenti la fase fallisce subito con «riferimento discorde», che è già un errore. Ogni set d'istruzioni ha il proprio riferimento.
- **Confronto esatto.** Hash a 64 bit o confronto bit per bit, mai una tolleranza in virgola mobile: niente falsi positivi da arrotondamento.
- **Errore.** Registra kernel, set d'istruzioni, core logico e fisico, iterazione, hash atteso e ottenuto, seme, e il clock e la temperatura dell'ultimo campione.
- **Struttura.** Riferimento, confronto, seme riproducibile e test a risposta nota riprendono la struttura di OpenDCDiag (§12).

### 4.3 Modi di carico

- **Costante:** 100% di occupazione.
- **Variabile:**
  - onda quadra con periodi casuali da 10 a 500 ms di lavoro e altrettanto di pausa, con seme;
  - salti fra un kernel e l'altro (K1 → K5 → pausa).

  Sollecita i transitori di tensione, i passaggi di boost e gli stati di sospensione dei core (C-state). Le pause usano un'attesa mista fra sospensione e attesa attiva, con un timer ad alta risoluzione.
- **Leggero:** uno o due thread SSE2 o scalari con pause periodiche, per il boost massimo a basso carico, tipico dell'instabilità con Curve Optimizer.
- **Un core alla volta:**
  - un thread per core fisico, con il fratello SMT a riposo, perché un core da solo raggiunge il boost più alto; l'opzione «entrambi i thread» è in Personalizza;
  - prima i core P, poi gli E; i core parcheggiati non si saltano: `Parked` è solo lo stato di riposo del momento e l'affinità rigida li risveglia (revisione finale della M8a1);
  - tempo per core secondo il profilo;
  - un core che sbaglia si segna e si passa al successivo; i risultati si raggruppano per CCD (`LastLevelCacheIndex`).
- **Tutti i core:** un thread per processore logico.

### 4.4 Topologia e affinità

- **Topologia.** `GetSystemCpuSetInformation` dà `CoreIndex` (core fisico), `EfficiencyClass` (P/E), `LastLevelCacheIndex` (CCD o dominio L3), `Group` e `Parked`. `GetLogicalProcessorInformationEx(RelationCache)` dà le dimensioni delle cache.
- **Affinità rigida** con `SetThreadGroupAffinity`, valida con più gruppi di processori. Su Windows 10 serve impostarla esplicitamente oltre i 64 processori logici.
- **Numero mostrato.** Il «core N» mostrato è l'indice progressivo dei core fisici in ordine di `CoreIndex`. La corrispondenza con la numerazione di Ryzen Master o del BIOS è un punto da fissare nel piano (§15).

### 4.5 Profili

| Profilo | Durate | Scaletta | Severità |
|---|---|---|---|
| CPU · Verifica normale | Rapido 5 min · Standard 30 min · Lungo 1 h | Rapido: K2 + K5 + K8. Standard e Lungo: K1 → K2 → K7 → K8 → K3, a tutti i core, carico costante, con le fasi in scala | non si ferma al primo errore: conta gli errori per core e per kernel; un solo errore dà `errors` |
| CPU · Stabilità overclock | Standard 1 h · Lungo 2 h · Notte 8 h | (1) tutti i core: K2, K5, K7, K3; (2) un core alla volta: K2 AVX2, K5, carico leggero variabile; (3) carico variabile a tutti i core; (4) K4 e K9; (5) fasi AVX-512, se presenti. La scaletta si ripete fino alla durata | si ferma al primo errore nelle fasi a tutti i core; nel ciclo per core segna il core e prosegue; WHEA corretti → `marginal` |
| RAM · Verifica normale | 15 · 30 · 60 min | K10 a rotazione dei pattern + K3 | come la CPU normale |
| RAM · Stabilità overclock | 1 h · 2 h · 8 h | K10 con tutti i pattern + K3 + K4 | si ferma al primo errore |

- **Tempo per core** nel ciclo d'overclock: 3 min per core con Standard, 5 con Lungo, 10 con Notte, ridotto se i core sono tanti. Il piano calcola la scaletta in modo che il totale rispetti la durata scelta.
- **«Fermati al primo errore».** Sostituisce la severità del profilo, se l'utente lo imposta.

### 4.6 Benchmark della CPU

- **Carichi.** Sei, a lavoro fisso:
  - interi: NTT (K5), hash (xxHash/CRC32C), compressione e decompressione, ordinamento;
  - virgola mobile: FFT (K2) e GEMM (K7).

  Ciascuno verifica il proprio risultato.
- **Single core.** Il carico gira su un thread fissato al primo core della classe `EfficiencyClass` più alta.
- **Multi core.** Una copia indipendente del carico per ogni processore logico. Il **fattore di scala**, cioè multi diviso per (single × thread), compare sotto i contagiri.
  - **Velocità per thread** (decisione dell'utente del 2026-10-07): ogni thread misura il tempo delle sue iterazioni, e la velocità multi è la somma delle velocità dei thread. Un thread che ha finito continua a lavorare, senza contare, finché non hanno finito tutti: così ogni velocità è misurata a CPU piena. Sulle CPU ibride i core veloci non restano fermi ad aspettare quelli lenti.
- **Svolgimento.** Un giro di riscaldamento da scartare, poi 3 ripetizioni per carico, di cui si tiene la mediana. Fra un carico e l'altro ci sono 2 s di pausa. In tutto circa 2 minuti.
- **Punteggio.** Media geometrica, sui sei carichi, del rapporto fra la velocità misurata e una velocità di riferimento fissa, × 1500. Single e multi sono separati, e non c'è un punteggio combinato.
  - **Scala fissa** (decisione dell'utente del 2026-10-07, che sostituisce la «macchina base = 1000»): le velocità di riferimento si tarano una volta sul Ryzen 7 7800X3D dell'autore con le impostazioni di fabbrica, che vale quindi 1500 in single e 1500 in multi. L'interfaccia non nomina la CPU di taratura e parla di «punti su una scala fissa».
  - **Dimensioni dei dati fisse**, uguali su ogni macchina, e non legate alla cache come nello stress test.
  - **Versione:** `cpu-1`. Ogni cambio dei carichi, della loro dimensione o delle velocità di riferimento cambia la versione.
- **Validità.**
  - **Non valida:** una verifica fallita rende la misura non valida, con il messaggio «Errore di calcolo durante il benchmark: prova lo stress test».
  - **Valida con avviso** quando la misura è fatta:
    - con il portatile a batteria;
    - con throttling termico (dai sensori);
    - con altri processi sopra il 10% di CPU (misurato da PDH all'inizio e durante);
    - in macchina virtuale (bit «hypervisor present» di CPUID).

## 5. M8b — GPU

### 5.1 Vincoli

- **API.** Solo D3D11 (compute e grafica), senza CUDA, Vulkan o header proprietari.
- **Scelta della GPU.** Una GPU per test, scelta per LUID con `EnumAdapterByGpuPreference`; si esclude il «Microsoft Basic Render Driver». Il carico non apre finestre.
- **TDR.** Ogni invio di lavoro si calibra da solo a 30–50 ms, così il timeout predefinito di 2 s non scatta mai.
- **Device perso.** `DXGI_ERROR_DEVICE_REMOVED`, `DEVICE_HUNG` e `DEVICE_RESET`, con `GetDeviceRemovedReason`, danno `device_lost`. Un invio che non torna entro 1 s dà `hung`.
- **VRAM.** Si dimensiona dal budget di `IDXGIAdapter3::QueryVideoMemoryInfo`: 95% su una GPU dedicata e 90% su una integrata, meno 400 MB. Si alloca a pezzi da 256 MB–1 GB, perché D3D11 garantisce una sola risorsa fino a min(max(128 MB, 25% della VRAM), 2 GB). Se l'allocazione fallisce, si riprova con meno memoria.
- **Stop termico.** Le temperature della GPU arrivano già senza servizio (NVML/ADL/IGCL).

### 5.2 Benchmark: Calcolo e Grafica

- **Calcolo.** Media geometrica, contro le velocità di riferimento, di:
  - catene FMA in FP32 (TFLOPS);
  - hash su interi INT32 (TIOPS);
  - banda di memoria con copia float4 su almeno 1 GB (GB/s).
- **Grafica.** Scena fuori schermo deterministica: riempimento (Gpixel/s), texture (Gtexel/s) e overdraw.
- **Punteggio.** Scala fissa, come per la CPU (decisione dell'utente del 2026-10-07, che sostituisce «RTX 4080 = 1000»): punti = 1500 × media geometrica del rapporto fra la velocità misurata e quella di riferimento, separati per Calcolo e Grafica. Le velocità di riferimento si tarano una volta sulla RTX 4080 dell'autore con le impostazioni di fabbrica, che vale quindi 1500. Versione `gpu-1`.
- **Svolgimento.** 3–5 s di riscaldamento, almeno 5 finestre da 1 s misurate con le timestamp query, poi mediana e dispersione.
- **Avvisi:** throttling; un altro processo che usa la GPU (dalla tabella dei processi GPU esistente).

### 5.3 Catalogo dello stress

| Id | Nome | Cosa fa | Verifica |
|---|---|---|---|
| S1 | Calcolo FP32 | catene FMA | in overclock usa dati a interi piccoli, così FP32 è esatto: confronto con il riferimento della CPU e con una copia ridondante |
| S2 | Hash su interi | catene imul/xor/rotl | riferimento della CPU su un campione di thread e doppia esecuzione |
| S3 | Flusso di memoria | lettura e scrittura float4 su ≥ 1 GB | un calo sostenuto della banda segnala gli errori di collegamento corretti in silenzio da GDDR6/6X |
| S4 | Verifica VRAM | pattern derivato dall'indirizzo e ruotato, scritto una volta e riletto in ordine sparso, più passi classici (walking ones, moving inversions), in stile memtest_vulkan | confronto con statistiche dei bit sbagliati |
| S5 | Carico grafico | pelliccia e overdraw fuori schermo | in overclock, checksum calcolato dalla GPU |
| S6 | Scansione artefatti | scena deterministica | hash del fotogramma contro un fotogramma di riferimento della stessa GPU; differenza dei pixel in caso di errore |
| S7 | Rampa adattiva | carico dal 20 al 100% a passi del 5% | riporta il livello di carico e il clock al primo errore |
| S8 | Carico alternato | dal 100 a circa il 15% con periodi da 10 a 500 ms | quelle dei kernel sottostanti |
| S9 | Pausa e ripartenza | carico fermo per 10–15 s, poi di nuovo pieno | quelle dei kernel sottostanti |

I valori in virgola mobile non si confrontano mai fra GPU diverse, perché i driver non sono bit-exact fra vendor.

### 5.4 Profili

| Profilo | Durate | Scaletta | Supera se |
|---|---|---|---|
| GPU · Verifica normale | 5 · 15 · 30 min | S5 + S1, poi S7 | nessun device perso, verifiche superate, stabilità ≥ 97% (finestra peggiore / migliore del throughput, come lo stress test di 3DMark) |
| GPU · Stabilità overclock | 30 min · 1 h · 2 h | un giro di S4, S3, S2, S1 esatto, S6, S7, S8, S9 (S3 aggiunto dall'utente il 2026-10-07) | zero discrepanze, zero device persi, nessun invio bloccato, stabilità ≥ 97% escluso il throttling |

### 5.5 Messaggi in più nel protocollo

`Run` porta il LUID della GPU. `Progress` porta il throughput della finestra e il numero di discrepanze. La versione del protocollo sale a 2 se i campi cambiano.

### 5.6 Spike prima del piano

Codice da buttare, sulla RTX 4080 e sulla iGPU AMD. Serve a rispondere a:

1. reattività e granularità della preemption con invii da 30–50 ms;
2. esattezza bit per bit di FP32 su interi piccoli e degli hash interi, fra le due GPU;
3. determinismo dell'hash del fotogramma di S6 sulla stessa GPU;
4. come compilare gli shader: bytecode precompilato con `fxc` o `dxc` al build, oppure a runtime con `D3DCompile`;
5. contatore dei replay PCIe da NVML come segnale aggiuntivo;
6. effetto del nome del processo sulla limitazione del driver;
7. costo del processo sulla CPU durante lo stress della GPU.

Esiti e scelte (2026-10-07): `docs/superpowers/references/m8/spike-gpu.md`. Correggono il §5.1 su due punti: l'invio si tara a 20 ms sulle GPU integrate, e la VRAM delle integrate ha un tetto, perché il loro budget è la RAM condivisa.

## 6. M8c — Disco

### 6.1 Vincoli

- **Dove.** Il test usa un file in una cartella scelta dall'utente. La predefinita è `%LOCALAPPDATA%\Temp` del disco di sistema. La pagina mostra a quale disco fisico corrisponde la cartella, attraverso il volume e la tabella dei dischi della M6b.
- **I/O.** Senza cache (`FILE_FLAG_NO_BUFFERING`) e con `FILE_FLAG_OVERLAPPED`, con un IOCP. Buffer allineati con `VirtualAlloc`; offset e dimensioni multipli del settore fisico.
- **File di test.**
  - Si crea con `CREATE_NEW` e si **riempie tutto prima delle letture**: oltre la lunghezza valida dei dati Windows restituisce zeri senza toccare il disco, e `SetFileValidData` richiede privilegi d'amministratore.
  - Si toglie la compressione NTFS.
  - Ha `FILE_FLAG_DELETE_ON_CLOSE` e un file accanto (`.oma-test.json`) con PID e istante d'avvio. All'avvio l'app elimina i file di test orfani delle cartelle usate di recente.
  - Lo svuotamento è `FlushFileBuffers`.
- **Dati.** Casuali di default. «Dati comprimibili» (zeri) è un'opzione di Personalizza.
- **Spazio libero.** Ne resta sempre almeno max(1 GiB, 5% del volume).
- **Dischi speciali.**
  - Dischi di rete: rifiutati.
  - Rimovibili e cartelle di OneDrive o Dropbox: avviso.
  - HDD in standby: il test parte solo con il consenso, chiesto attraverso il gate dei dischi esistente (`storage_gate`).
- **Antivirus.** Defender o l'accesso controllato alle cartelle possono rallentare o bloccare le scritture: si spiega nel messaggio d'errore.

### 6.2 Benchmark: Lettura e Scrittura (MB/s)

- **Prove (B1, predefinito):** SEQ1M Q8T1, SEQ1M Q1T1, RND4K Q32T1 e RND4K Q1T1, ognuna in lettura e in scrittura.
- **Profilo NVMe (B2), in Personalizza:** SEQ1M Q8T1, SEQ128K Q32T1, RND4K Q32T16 e RND4K Q1T1.
- **Svolgimento.** File da 1 GiB, 5 s per prova, 5 s di intervallo, un giro di riscaldamento e 3 misure, di cui si tiene la migliore, come CrystalDiskMark.
- **Tetto di scrittura.** Le prove di scrittura hanno anche un tetto in byte: in tutto al massimo 40 GiB per esecuzione, divisi per prova come nel §6.4.
- **Contagiri.** In MB/s (D13): l'ago segue la prova in corso, e alla fine si ferma su SEQ1M Q8T1.
- **Tabella di dettaglio.** MB/s, IOPS e latenza media e p99 di ogni prova.
- **Punti per la classifica.** Media geometrica delle 4 prove (lettura e scrittura) contro il disco NVMe di sistema dell'autore = 1000. Versione `disk-1`.

### 6.3 Stress e stabilità

Prima di partire la pagina mostra la stima dei dati che verranno scritti. Con il servizio attivo, il riepilogo riporta l'usura misurata dai Data Units Written dello SMART, prima e dopo il test.

| Id | Nome | Cosa fa | Limiti predefiniti |
|---|---|---|---|
| N1 | Misto 70/30 | letture e scritture casuali e sequenziali | 10 min, al massimo 200 GiB scritti |
| N2 | Scrittura sostenuta | SEQ1M Q1 campionata a 2 Hz: un calo unico indica la fine della cache SLC, un andamento a denti di sega con temperatura alta indica throttling | al massimo 100 GiB |
| N3 | Lettura prolungata | letture continue per il calore; nessuna usura dopo il riempimento | 10 min |
| N4 | IOPS casuali | RND4K ad alta coda | 10 min, al massimo 50 GiB scritti |
| V1 | Riempi e verifica | riempimento, svuotamento, rilettura in ordine inverso; 3 cicli | 8 GiB per ciclo |
| V2 | Sovrascritture casuali | sovrascritture con numero di generazione e passate periodiche di verifica | velocità limitata, 30 min |
| V3 | Capacità reale | file da 1 GiB fino allo spazio libero, poi verifica di tutti, in stile h2testw; per chiavette e schede | lo spazio libero meno la riserva |
| V4 | Scritture sincrone | scritture con write-through e svuotamento, poi verifica | 2 GiB |

**Formato del blocco da 4 KiB:** intestazione con magic, id della sessione, indice del blocco, generazione e checksum xxh3, poi dati pseudo-casuali con seme.
- **Verifica.** Si controlla il checksum e, solo se non torna, si rigenerano i dati per classificare l'errore: bit sbagliati, blocco nel posto sbagliato (indice diverso), blocco vecchio (generazione diversa) o zeri.
- **Errore passeggero o persistente.** Ogni errore si rilegge una volta.
- **Ordine.** La verifica avviene dopo lo svuotamento, senza cache e in un ordine diverso da quello della scrittura.

**Profili:**

| Profilo | Durate | Scaletta |
|---|---|---|
| Disco · Verifica normale | Rapido 10 min · Standard 30 min · Lungo 1 h | N1, N3, N2 (ridotta), N4, con le fasi in scala e i tetti in byte di N1–N4 |
| Disco · Stabilità | Standard (V1 × 3 cicli, V2 30 min, V4) · Lungo (V1 × 6 cicli, V2 2 h, V4) | V1, V2, V4; V3 al posto di V1 se il disco è rimovibile |

**Stop termico.** Alla soglia di §2.6.

### 6.4 Decisioni del brainstorming della M8c (2026-10-08)

Precisano i §6.1–6.3; dove li contraddicono, vale questo paragrafo.

- **Un piano unico.** La M8c non si divide: benchmark e stress stanno in un solo piano e in un solo branch (`feat/m8c-disk`), decisione dell'utente.
- **Architettura.**
  - **`oma-load`:** il motore d'I/O sta nel modulo `disk/`, come la GPU sta in `gpu/`:
    - un IOCP per thread, con `GetQueuedCompletionStatusEx`;
    - buffer allocati con `VirtualAlloc`;
    - latenze misurate con QPC in un istogramma logaritmico (media e p99).
  - **`oma-core`:** contiene la parte pura:
    - piani e profili (B1, B2, N, V);
    - punteggio `disk-1` e controller del benchmark;
    - formato dei blocchi da 4 KiB e classificazione degli errori, condivisi da `oma-load` e dai test.
- **Protocollo load v6.** I campi nuovi sono tutti facoltativi in lettura:
  - `Plan.disk`: un `DiskTarget` con cartella, dimensione del file, settore fisico e tipo di dati (casuali o comprimibili);
  - per ogni fase, un `DiskJob` con:
    - dimensione del blocco, coda e thread;
    - percentuale di letture, accesso sequenziale o casuale;
    - tetto in byte e tipo di verifica;
  - i kernel `disk_bench`, `n1`–`n4` e `v1`–`v4`;
  - `Progress.disk` con `readBps`, `writeBps`, `writtenBytes` e `iops`;
  - `PhaseDone.disk` con byte, numero di I/O, latenza media e p99, separati per lettura e scrittura;
  - gli `ErrorKind` del disco: `bit_flip`, `misplaced`, `stale`, `zeros` e `io_error`. Ognuno è marcato come passeggero o persistente; l'indice del blocco prende il posto dell'iterazione;
  - le notice `disk_full` e `access_denied`, che ricorda l'accesso controllato alle cartelle di Defender;
  - il codice d'uscita 5 (§2.2).
- **Bersaglio.**
  - **Barra laterale:** una sola voce «Disco» sotto Punteggio.
  - **Scelta del volume:** in cima alla pagina c'è il menu dei volumi locali, per esempio «C: · Samsung 990 Pro · NVMe». Il disco fisico viene dalla tabella dei dischi della M6b. L'ultima voce del menu è «Scegli cartella…».
  - **Cartella usata:**
    - sul volume di sistema è `%LOCALAPPDATA%\Temp`;
    - sugli altri volumi è la radice;
    - se la radice non è scrivibile, l'app chiede di scegliere una cartella.
  - **Misure e record:**
    - «Le tue misure» elenca tutti i dischi, ciascuno con il suo modello;
    - il record del riferimento ▲ è per disco fisico, identificato con la chiave dei dischi della M6b.
  - **Procedura guidata dello stress:** al passo 1 la voce Disco mostra lo stesso menu.
  - **Dischi rifiutati o con avviso:**
    - rifiutati: i dischi di rete;
    - con avviso: i dischi rimovibili, le cartelle sincronizzate (OneDrive, Dropbox, Google Drive) e i dischi virtuali;
    - con consenso: un HDD in standby (§6.1).
- **File orfani (punto aperto del §15).** Con `FILE_FLAG_DELETE_ON_CLOSE` il file sparisce anche quando `oma-load` o l'app cadono, perché Windows chiude gli handle. Resta solo dopo un crash del sistema o una mancanza di corrente.
  - **Dove si registra la cartella:**
    - il diario (`journal.json`, §8.3) ha il campo `diskFolder`;
    - le impostazioni hanno `performance.diskFolder`, l'ultima cartella usata.
  - **Pulizia all'avvio:** l'app guarda solo la cartella del diario aperto e l'ultima cartella usata.
  - **Cosa cancella:** solo i file `oma-test-*.bin` che hanno accanto il loro `.oma-test.json` e il cui PID non è più vivo, oppure è vivo ma con un istante d'avvio diverso.
- **Benchmark.**
  - **Svolgimento.**
    - Il file da 1 GiB si riempie una volta, prima di tutto; il riempimento non conta nel tetto.
    - Le prove girano prima tutte in lettura, poi tutte in scrittura, così le letture non risentono della pulizia interna del disco dopo le scritture.
  - **Tetto di 40 GiB diviso per prova** (decisione dell'utente). Una misura finisce dopo 5 s o al suo tetto in byte, quello che arriva prima.

    | Prove di scrittura | Riscaldamento | Ognuna delle 3 misure | Totale |
    |---|---|---|---|
    | SEQ1M Q8T1, SEQ1M Q1T1 (in B2: SEQ1M Q8T1, SEQ128K Q32T1) | 1 GiB | 4 GiB | 13 GiB per prova, 26 GiB |
    | RND4K Q32T1, RND4K Q1T1 (in B2: RND4K Q32T16, RND4K Q1T1) | 1 GiB | 2 GiB | 7 GiB per prova, 14 GiB |

    Sui dischi SATA e sugli HDD il tetto non scatta.
  - **Misure.** MB/s si intende come 10⁶ B/s, come in CrystalDiskMark; si misura dal primo invio all'ultimo completamento.
  - **Scala del contagiri.** La stima iniziale dipende dal tipo di disco:
    - NVMe: 8000 MB/s;
    - SSD SATA: 600 MB/s;
    - HDD: 300 MB/s;
    - USB: 1000 MB/s.

    Poi vale la regola del §3.3: si sceglie il numero tondo sopra la stima e la scala può solo crescere.
  - **Punti `disk-1`.**
    - Il calcolo: 1000 × la media geometrica degli 8 rapporti (le 4 prove di B1, in lettura e in scrittura) contro le velocità di riferimento.
    - La taratura: le velocità di riferimento si tarano sul disco NVMe di sistema dell'autore con l'esempio `calibrate_disk`, come per `cpu-1`.
    - B2 dà solo MB/s, senza punti.
  - **Avvisi di validità:**
    - PC a batteria;
    - altro I/O sullo stesso disco oltre il 5% durante le misure (dai contatori PDH già letti dall'app);
    - dati comprimibili;
    - disco virtuale;
    - disco rimovibile;
    - temperatura oltre la soglia.

    Una misura con dati comprimibili non vale per la classifica.
- **Stress.**
  - **Intestazione del blocco:** 64 byte.

    | Byte | Campo |
    |---|---|
    | 0..8 | magic |
    | 8..16 | id della sessione |
    | 16..24 | indice del blocco |
    | 24..28 | generazione |
    | 28..32 | versione |
    | 32..40 | xxh3 dell'intestazione (byte 0..32) e dei dati |

    I dati che seguono si generano dal seme (sessione, indice, generazione).
  - **Hash.** xxh3 viene da una crate scelta nel piano, con licenza verificata e aggiunta all'elenco `accepted` di `about.toml` (§12).
  - **N2.**
    - **Fine della cache SLC:** la velocità resta sotto il 60% di quella iniziale per almeno 3 s.
    - **Cosa riporta:** la dimensione della cache SLC e la velocità che segue.
    - **Sospetto termico:** se nei 10 s prima del calo la temperatura era oltre la soglia d'avviso, il calo è marcato come «sospetto termico».
  - **Pagina durante il test.** Al posto della griglia dei core mostra i blocchi verificati e gli errori, con la loro posizione nel file. Il grafico mostra MB/s e temperatura.
  - **Riepilogo.** Mostra i dati scritti misurati dallo SMART, cioè la differenza del sensore `data/host-written` del servizio prima e dopo il test.
  - **Errori iniettati.** Solo nelle build di debug, `OMA_LOAD_INJECT='v1'` corrompe un blocco dopo la lettura.
- **Esiti.**
  - Un errore di dati, o un errore d'I/O persistente, dà `errors`, con il tipo dell'errore.
  - Un errore d'I/O non recuperabile, come un disco scollegato, chiude `oma-load` con il codice 5 e dà `errors`, con la causa.
  - Lo spazio esaurito durante il test dà un esito nuovo, `stopped_disk_full`, con il verdetto «Fermato: spazio su disco esaurito». Il test si ferma, il file si cancella e la sessione si salva.
- **Verifica dal vivo in più.** Tre benchmark di fila sul disco NVMe di sistema. La dispersione di SEQ1M Q8T1 in scrittura deve restare entro il 5%; se la supera, il tetto si alza.

## 7. M8d — Classifica e condivisione

### 7.1 Tabella di riferimento

- **File nel repository.** `reference-scores.json` (CC0) contiene le righe misurate dall'autore: 7800X3D, RTX 4080, iGPU AMD e i dischi della macchina. Si aggiungono le macchine di amici, se ce ne sono.
- **Una riga:** categoria (`cpu-single`, `cpu-multi`, `gpu-compute`, `gpu-graphics`, `disk`), versione del punteggio, modello, valore (mediana), numero di misure, fonte (`author` oppure `community`).
- **Documentazione.** `docs/benchmark-scoring.md` documenta:
  - le macchine base, le formule e le versioni;
  - le regole di aggregazione;
  - il formato dell'esportazione;
  - la privacy della condivisione.

### 7.2 Pagina Classifica

- **Contenuto.**
  - Una scheda per categoria.
  - Una lista a barre ordinata di 8–12 modelli vicini al proprio punteggio, con la propria riga evidenziata: il miglior punteggio valido della versione corrente.
  - Sotto, una didascalia: versione del punteggio, numero di modelli e fonte («tabella aggiornata il …» oppure «tabella inclusa nell'app»).
  - Il pulsante «Aggiorna ora».
- **Percentile.** Compare solo con almeno 10 righe nella categoria: «più veloce del 70% dei modelli in tabella».
- **Confronti.** Si confrontano solo punteggi della stessa versione.

### 7.3 Condivisione con un'issue GitHub precompilata

1. Sotto ogni punteggio valido, il pulsante «Condividi su GitHub» apre un'**anteprima**. L'anteprima mostra il JSON esatto e l'avviso: la issue è pubblica e permanente e mostra il tuo nome utente GitHub; per rimuoverla serve chiedere al maintainer.
2. «Apri GitHub» apre il browser predefinito su `https://github.com/Cioscos/OpenMonitorAdvanced/issues/new?template=benchmark-result.yml&title=…&result=…`. Campi e titolo arrivano precompilati dall'URL, come permettono i moduli delle issue. L'utente, collegato a GitHub, preme soltanto «Create».
3. Se l'URL supera 8000 caratteri, l'app copia il JSON negli appunti e apre la issue con il campo vuoto e l'istruzione di incollarlo.

**Modello della issue.** Il file `.github/ISSUE_TEMPLATE/benchmark-result.yml` contiene:
- il campo `result`: un'area di testo con resa JSON, obbligatoria;
- la casella «Hardware in overclock»;
- il campo note, facoltativo;
- la casella obbligatoria «I dati diventano pubblici».

Il modello applica l'etichetta `benchmark-result`.

**Contenuto (§8.5).** Il JSON contiene:
- categoria, versione del punteggio, punteggi, velocità di ogni carico;
- modello di CPU, GPU o disco, quantità di RAM, build di Windows, versione dell'app, avvisi.

Non contiene campioni, seriali, GUID, nome del PC o nome utente.

### 7.4 Validazione e aggregazione (GitHub Actions)

- **Codice.** La logica sta in `oma-core::scores`, la stessa che usa l'app, e una piccola CLI del workspace (`oma-scores`) la espone alle Action.
- **`benchmark-validate.yml`** parte a ogni issue aperta o modificata con l'etichetta `benchmark-result`.
  - Estrae il JSON dal corpo della issue.
  - Controlla schema, versione e plausibilità: valori finiti e positivi, dentro un fattore 0,2–5 della mediana del modello se esiste, altrimenti della categoria.
  - Commenta l'esito e mette l'etichetta `valid` oppure `invalid`.
- **`benchmark-aggregate.yml`** parte quando un'etichetta cambia, ogni giorno e a mano.
  - Raccoglie le issue con `valid` e senza `rejected`.
  - Tiene **un risultato per utente GitHub e per modello**, il più recente, ed esclude quelli in overclock.
  - Calcola la mediana per modello e categoria. Entrano solo i modelli con **almeno 3 utenti diversi**.
  - Unisce le righe `author` del repository.
  - Pubblica il risultato su **GitHub Pages** in `https://cioscos.github.io/OpenMonitorAdvanced/scores/v1/reference-scores.json`, con `actions/deploy-pages`. Non usa `raw.githubusercontent.com`, perché da maggio 2025 risponde spesso 429.
- **Moderazione.** L'autore modera con le etichette: aggiunge `rejected` o toglie `valid`, e al giro successivo la riga si aggiorna. Le Action non si fidano del contenuto delle issue: lo trattano come dati, mai come comandi o codice.
- **Pages.** Va abilitato una volta dall'utente, con sorgente «GitHub Actions».

### 7.5 Download automatico nell'app

- **Client.** Usa `oma-win::http` (WinHTTP, solo HTTPS, store di Windows, TLS 1.2/1.3) con `If-None-Match`, e tiene l'ETag.
- **Quando.** Al massimo una volta ogni 24 h, e solo mentre si usa la vista Prestazioni: all'apertura della Classifica o alla fine di un benchmark. In più c'è «Aggiorna ora». Dopo un errore si ritenta non prima di 6 h.
- **Controlli sul file.** Al massimo 1 MB, formato `v1`, schema valido, valori finiti e positivi. Un file malformato non sostituisce mai quello buono.
- **Dove si salva.** `%LOCALAPPDATA%\OpenMonitorAdvanced\performance\reference-scores.json`, con scrittura atomica.
- **Ripiego.** Senza rete o con un file non valido si usa l'ultima copia buona, altrimenti la tabella inclusa nell'installer.
- **Privacy.** Attivo di default (`communityTable`), si spegne nelle Impostazioni › Prestazioni. Accanto alla casella e in `docs/benchmark-scoring.md` c'è la nota, nello stile di quella della M6c: la richiesta va a `cioscos.github.io` e manda l'indirizzo IP e uno User-Agent con la versione dell'app, nient'altro. Si applica la privacy di GitHub.

### 7.6 Decisioni del brainstorming della M8d (2026-10-08)

Precisano i §7.1–7.5; dove li contraddicono, vale questo paragrafo. Per decisione dell'utente la condivisione non deve richiedere un account, quindi **un Cloudflare Worker anonimo sostituisce le issue GitHub, le Action e GitHub Pages** (§7.3, §7.4 e il server del §7.5). La ricerca sul GDPR è in `docs/superpowers/references/m8/research-gdpr.md` (non è un parere legale).

- **Due piani.**
  - **M8d1, il server:** il Worker, il database D1, l'aggregazione, le fixture comuni e la documentazione della moderazione. Branch `feat/m8d1-…`.
  - **M8d2, l'app:** la tabella inclusa, il download, la pagina Classifica, il riferimento ▲ «modello della tabella», «Esporta JSON» e «Condividi». Parte quando il Worker è pubblicato, così si prova dal vivo.
- **Anonimato (l'opzione «a» della ricerca).**
  - **Cosa non si salva:** nessun account, nessun id d'installazione, nessun indirizzo IP né nel database né nei log, e i log del Worker sono spenti.
  - **La data:** si salva il giorno dell'invio, non l'ora.
  - **Cosa diventa pubblico:** solo le mediane di almeno 3 invii; le righe grezze restano in D1. È il controllo di unicità: una mediana di almeno 3 invii non indica una persona.
  - **L'IP:** serve solo al rate limit, in memoria. La base giuridica è il legittimo interesse contro gli abusi.
  - **Il prezzo:** gli invii ripetuti della stessa persona contano più volte. Lo limitano il rate limit, il filtro di plausibilità, la soglia dei 3 invii, la moderazione e il segno «condiviso» locale dell'app.
- **Il Worker.**
  - **Dove sta:** la cartella `scores-worker/` del repository, con TypeScript, `wrangler.toml`, test Vitest con il runtime dei Worker simulato e un lockfile proprio.
  - **Costo:** gratis nel piano Workers Free. Il piano ricontrolla i limiti di richieste, CPU e D1, e se il piano gratuito offre il binding di rate limit e i cron.
  - **Indirizzo:** un sottodominio del dominio dell'utente su Cloudflare, scelto prima del deploy. Nel codice è un segnaposto in un solo punto per parte: la costante dell'app e `wrangler.toml`. Un URL proprio permette di cambiare host senza aggiornare l'app.
  - **Deploy:** lo fa l'utente a mano, con `wrangler login` e `wrangler deploy`. Nei segreti di GitHub non c'è nessun token Cloudflare; la CI esegue solo i test e il controllo dei tipi del Worker.
  - **`POST /v1/submit`:**
    - **Corpo:** l'oggetto del §8.5 più `overclock` (booleano), al massimo 16 KB.
    - **Controlli:** schema, `format: 1`, versione del punteggio nota (`cpu-1`, `gpu-1`, `disk-1`), `valid: true`, valori finiti e positivi.
    - **Plausibilità:** il valore sta dentro un fattore 0,2–5 della mediana del modello. Se il modello non ha una mediana, vale quella della categoria; se la categoria è vuota, vale un tetto per categoria fissato nel piano.
    - **Risposte:**
      - `201` se l'invio è accettato;
      - `400` con un codice d'errore che l'app traduce;
      - `413` se il corpo è troppo grande;
      - `429` se le richieste sono troppe.
    - **Riga salvata:** categoria, versione, modello normalizzato, punteggi, versione dell'app, build di Windows, RAM in GB, avvisi, `overclock` e il giorno.
  - **Rate limit:** per IP. Si usa il binding di rate limit dei Worker se il piano gratuito lo offre, altrimenti la regola di rate limit gratuita della zona. I numeri li fissa il piano.
  - **Aggregazione:** un cron giornaliero del Worker.
    - Esclude gli invii in overclock e i modelli della tabella `hidden_models`.
    - Calcola la mediana per categoria (`cpu-single`, `cpu-multi`, `gpu-compute`, `gpu-graphics`, `disk`), versione e modello. Un modello entra solo con almeno 3 invii.
    - Scrive il JSON del §8.4 con `source: "community"`.
    - Cancella gli invii più vecchi di 24 mesi.
  - **`GET /v1/reference-scores.json`:** serve il JSON già calcolato, con ETag e `Cache-Control` di un'ora.
  - **Moderazione:**
    - gli invii entrano da soli, e l'autore interviene dopo;
    - gli interventi sono comandi `wrangler d1 execute` documentati in `docs/benchmark-scoring.md`: cancellare un invio, nascondere un modello, ricalcolare la tabella;
    - non c'è un pannello web.
- **Regole condivise.**
  - **Cosa coprono:** la normalizzazione dei modelli (spazi e maiuscole), la validazione, la plausibilità e la mediana.
  - **Dove stanno:** in Rust (`oma-core::scores`, per l'app) e in TypeScript (il Worker).
  - **Come restano uguali:** con le fixture di `testdata/scores/`, lette da `cargo test` e da Vitest: invii validi e non validi con il codice atteso, modelli da normalizzare, e un'aggregazione con il risultato atteso.
  - **La CLI `oma-scores` del §7.4** non serve più.
- **Righe dell'autore.** Restano separate da quelle della community.
  - **Nell'app:** `reference-scores.json` (CC0) è incluso con le righe dell'autore, cioè le mediane delle sue misure valide: 7800X3D, RTX 4080, iGPU AMD e Fanxiang S880. È sempre disponibile, anche senza rete.
  - **Nel Worker:** si pubblicano solo le righe della community. L'app unisce le due tabelle e mostra la fonte di ogni riga.
- **App.**
  - **Download:** come il §7.5, ma verso l'URL del Worker.
    - **Frequenza:** al massimo una volta ogni 24 h; dopo un errore, nuovo tentativo dopo 6 h.
    - **Controlli:** ETag, al massimo 1 MB, scrittura atomica.
    - **Ripiego:** l'ultima copia buona, poi le sole righe dell'autore.
    - **Privacy:** la nota accanto a `communityTable` dice che la richiesta va al dominio del Worker, tramite Cloudflare, con l'indirizzo IP e uno User-Agent con la versione dell'app.
  - **«Condividi»:** c'è solo sotto un punteggio valido e non provvisorio.
    - **Anteprima:** mostra il JSON esatto, la casella «Hardware in overclock» e la nota: l'invio è anonimo, senza account e senza IP salvato; diventa pubblico solo come mediana di almeno 3 invii; dopo l'invio non si può più riconoscere né cancellare.
    - **«Invia»:** fa un POST con una funzione `post` nuova in `oma-win::http`.
    - **Esito:** se l'invio riesce, l'app segna il punteggio come «condiviso» solo in locale e disattiva il pulsante. Dopo un errore mostra il motivo tradotto e non lo segna.
  - **Pagina Classifica:** come il §7.2, con un badge della fonte su ogni riga («autore» o «community», con il tooltip).
  - **Riferimento ▲:** la terza voce, «Modello della tabella…», sceglie un modello della stessa categoria e versione.
  - **«Esporta JSON»:** salva l'oggetto del §8.5 con un dialogo di salvataggio.
- **Privacy (al posto del §10 per la condivisione).**
  - **Quando partono i dati:** l'app li manda solo dopo l'anteprima e «Invia».
  - **Informativa:** sta nel README e in `docs/benchmark-scoring.md`. Contiene titolare, dati, finalità, base giuridica, conservazione di 24 mesi, Cloudflare come responsabile del trattamento, e il motivo per cui un invio non si può cancellare.
  - **Punti aperti della ricerca, da verificare nel piano:** i log degli IP nei Worker, la residenza dei dati di D1 e la certificazione DPF di Cloudflare.
- **Verifiche dal vivo.**
  - **M8d1:**
    - `wrangler login` e deploy;
    - un invio con `curl`;
    - il cron lanciato a mano;
    - la tabella scaricata;
    - un comando di moderazione;
    - la cancellazione degli invii di prova.
  - **M8d2:**
    - la condivisione dall'app;
    - il download e la Classifica;
    - l'impostazione spenta;
    - «Aggiorna ora»;
    - l'uso senza rete.

## 8. Formati dei file

Tutti i file sono JSON con `format: 1`, scritti in modo atomico (file temporaneo proprio e rinomina) e letti con tolleranza per i campi sconosciuti. Stanno in `%LOCALAPPDATA%\OpenMonitorAdvanced\performance\`.

### 8.1 Sessione di stress (`stress\<AAAAMMGG-HHMMSS>-<id>.json`)

- **Intestazione:**
  - `id` (uuid);
  - `startedAt`, `endedAt` (UTC, ISO 8601);
  - `component` (`cpu`, `ram`, `gpu`, `disk`) e il modello del dispositivo;
  - `objective` (`normal`, `overclock`) e `preset`;
  - il `plan` completo (fasi, kernel, set d'istruzioni, modi di carico, durate, seme), `stopOnFirstError`;
  - `outcome` (§2.3) e `outcomeDetail`.
- **Risultati:**
  - `phases`: esito, durata, numero di verifiche e di errori per fase;
  - `cores`: stato per core (`passed`, `failed`, `untested`), con il primo errore;
  - `errors`: al massimo 200 errori, gli altri contati;
  - `whea`: conteggi per ID e per APIC ID;
  - `stats`: massimo e media di temperatura, potenza e clock;
  - `samples`: uno ogni 5 s, con tempo, temperatura, potenza e clock;
  - `events`: il diario degli eventi della sessione.
- **Versioni:** `appVersion`, `loadVersion`.

### 8.2 Punteggio (`scores\<AAAAMMGG-HHMMSS>-<id>.json`)

- **Identità:** `id`, `at`, `category` (`cpu`, `gpu`, `disk`), `scoreVersion`.
- **Punteggi** (`scores`): `single` e `multi` per la CPU, `compute` e `graphics` per la GPU, `readMBs`, `writeMBs` e `points` per il disco.
- **Dettaglio:**
  - `kernels`: nome, unità e velocità di ogni carico;
  - `device`: modello e proprietà della lista bianca della M6c;
  - `flags`: gli avvisi di validità;
  - `valid`;
  - `scaling` (solo CPU);
  - `samples`: diradati.

### 8.3 Diario (`journal.json`)

`sessionId`, `planSummary`, `phaseIndex`, `kernel`, `core`, `updatedAt`, `cleanEnd`; per i test del disco anche `diskFolder` (M8c, §6.4).

### 8.4 Tabella di riferimento

`format`, `generatedAt`, `rows[]` con `category`, `scoreVersion`, `model`, `value`, `n`, `source`.

### 8.5 Esportazione e condivisione

È lo stesso oggetto: `format`, `appVersion`, `category`, `scoreVersion`, `scores`, `kernels`, `hardware` (modello di CPU/GPU/disco, `ramGB`, `osBuild`), `flags`. Il campo `overclock` viene dalla casella della issue. Nessun identificatore.

## 9. Errori e casi limite

- **`oma-load.exe` assente, non avviabile o con un `Hello` incompatibile:** `failed_to_start` con il motivo e l'invito a reinstallare.
- **Set d'istruzioni.** Se la CPU non ha AVX2, i profili usano SSE2. Una scelta manuale di AVX-512 senza supporto non compare nel menu.
- **Quota di RAM.** Se la memoria richiesta non è più disponibile all'avvio, si riduce fino a 512 MB per thread e lo si scrive nel diario. Sotto quel limite la fase non parte.
- **Cartella del disco** non scrivibile, piena o di rete: errore prima dell'avvio, con la causa.
- **Nessuna GPU adatta**, oppure GPU sparita durante il test (driver aggiornato): `failed_to_start` oppure `device_lost`.
- **Servizio** che si collega o cade durante il test: avviso nel diario, il test continua. Gli avvisi termici seguono §2.6.
- **Overlay e cattura frame** possono restare accesi durante un test.
- **Orologio** cambiato durante un test: le durate si misurano con un orologio monotono, e gli istanti in UTC servono solo a mostrare l'ora.
- **File della cronologia** illeggibili o di una versione futura: si saltano, con una riga nel log dell'app.
- **Due finestre**, per esempio editor dell'overlay e app: un test si avvia solo dalla vista Prestazioni della finestra principale.

## 10. Sicurezza e privacy

- **Nessun privilegio amministrativo.** L'avvio di `oma-load` avviene senza elevazione.
- **Pipe** (§2.2): solo per l'utente corrente, PID verificato, nessun client remoto. `oma-load` accetta solo un piano da una pipe che non ha creato lui e lo valida (limiti di durata, thread, memoria e dimensioni) prima di eseguirlo.
- **Nessun dato esce senza un'azione dell'utente**, tranne il download della tabella (§7.5), che si spegne e lo dichiara.
- **Condivisione.** Passa dal browser dell'utente: l'app non usa token GitHub e non invia nulla da sé.
- **GitHub Actions.**
  - Permessi minimi: `issues: write` per la validazione; `pages: write` e `id-token: write` per la pubblicazione.
  - Il corpo delle issue non entra mai in un'espressione `${{ }}` di uno script: passa da file o variabili d'ambiente, contro l'iniezione nei workflow.

## 11. Prestazioni

- **A riposo** il budget non cambia: senza test `oma-load` non esiste e la vista Prestazioni non fa lavoro periodico.
- **Durante un test** la finestra resta sotto i 200 MB: campioni diradati, grafico degli ultimi 10 minuti, contagiri animati solo se visibili.
- **Memoria di `oma-load`:**
  - CPU: limitata dalle cache (pochi MB per thread), tranne K3 e K10, che usano la quota di RAM;
  - GPU: la VRAM del budget e pochi MB di RAM;
  - disco: i buffer delle code.
- **Misure.** `scripts/measure-footprint.ps1` misura a riposo dopo ogni sotto-milestone, come sempre. Le misure sotto carico si chiedono all'utente prima di farle.

## 12. Licenze e codice di terzi

| Progetto | Licenza | Uso |
|---|---|---|
| FIRESTARTER (TU Dresden) | GPL-3.0-or-later | adattamento in K1: gruppi d'istruzioni, rapporto fra registri e load/store, confronto ad anello degli hash dei registri. Senza la generazione di codice a runtime: kernel precompilati con intrinseci |
| OpenDCDiag (Intel) | Apache-2.0 | adattamento della struttura di verifica: riferimento e confronto, seme riproducibile, test a risposta nota (K2–K8) |
| memtest_vulkan | Zlib | porting in HLSL dell'algoritmo di S4 (pattern ruotato derivato dall'indirizzo, rilettura continua, statistiche dei bit) |
| stress-ng, stressapptest, gpu-burn, f3, DiskSpd, CrystalDiskMark | GPL-2.0-or-later, Apache-2.0, BSD-2, GPL-3.0, MIT, MIT | idee e metodi: CRC durante la copia, confronto fra copie ridondanti, formato dei blocchi, parametri delle prove |
| Prime95, y-cruncher, OCCT, CoreCycler, FurMark, AIDA64, 3DMark, Cinebench, Geekbench, PassMark, memtester | proprietarie, CC BY-NC-SA, GPL-2.0-only | solo le idee pubblicate, nessun codice |
| Orbitron, Share Tech Mono | OFL-1.1 | font inclusi |

- **File adattati.** I file che adattano codice di FIRESTARTER, OpenDCDiag o memtest_vulkan portano in testa un commento con il progetto d'origine, il copyright originale e la licenza, come le tre licenze richiedono. Non è un tag SPDX: la regola del progetto vieta i tag SPDX di terzi, non le note di copyright richieste.
- **Attribuzioni.** Le attribuzioni complete vanno in `THIRD_PARTY_NOTICES.md`; i testi di Apache-2.0, Zlib e OFL in `THIRD_PARTY_LICENSES.txt`, con le voci aggiunte a mano accanto a quelle di `cargo-about`. Il controllo in CI deve accettarle.
- **Nessuna dipendenza nuova per i kernel.** Se un piano ne propone una (per esempio un crate di compressione per K8), ne verifica la licenza e la mette nella lista di `cargo-about`.

## 13. Verifiche

### 13.1 Test automatici (TDD)

- **`oma-core::load`:**
  - costruzione dei piani per ogni profilo e durata, con il totale che rispetta la durata e il tempo per core;
  - calcolo dei verdetti per ogni esito;
  - recupero dal diario;
  - scelta delle soglie termiche e logica dei 2 campioni;
  - validazione dei piani ricevuti.
- **`oma-core::scores`:**
  - punteggi (media geometrica, versione) e scala dei contagiri;
  - formati di sessione, punteggio, tabella ed esportazione;
  - validazione e plausibilità;
  - aggregazione: un risultato per utente, almeno 3 utenti, overclock esclusi, mediana;
  - estrazione del JSON dal corpo di una issue (fixture di issue vere e malformate).
- **Kernel (`oma-load`, parte portabile):**
  - ogni kernel su dimensioni piccole, per ogni set d'istruzioni presente sulla macchina; gli altri si saltano dichiarandolo;
  - determinismo con lo stesso seme;
  - verifiche: Freivalds, andata e ritorno di NTT e FFT, vettori noti di K8, pattern di K10.
- **Errori iniettati.** Solo nelle build di debug, `oma-load --inject-fault <kernel>[:<core>]` fa sbagliare un bit di proposito. I test d'integrazione controllano che l'errore arrivi all'app come `Error` e diventi il verdetto giusto, compreso «Instabile · core N». Nella M8c lo stesso meccanismo corrompe un blocco del file.
- **Protocollo** `oma-ipc::load`: fixture rigenerate solo con `OMA_WRITE_FIXTURES=1`, più `validate`.
- **`oma-win`:**
  - parsing degli eventi WHEA, BugCheck e Kernel-Power da XML di esempio (puro);
  - un test `#[ignore = "requires real Windows hardware"]` che interroga il registro vero.
- **Vitest:**
  - procedura guidata (passi, Personalizza, avvisi);
  - geometria dei contagiri (angoli, tacche, scala);
  - pagine durante il test e risultato con dati finti;
  - cronologia;
  - Classifica (vicini, percentile, versione);
  - anteprima della condivisione e costruzione dell'URL della issue (limite, ripiego sugli appunti);
  - test del glossario (§3.7).
- **Action.** Validazione e aggregazione provate con la CLI su issue finte; il workflow con `act` o in un repository di prova (da fissare nel piano).

### 13.2 Verifiche dal vivo, con l'utente

Le fa l'utente, con un elenco a passi come nelle milestone precedenti. Prima di ogni carico pesante si chiede (memoria «no heavy load without warning»), e non si usa input sintetico sul desktop. Per ogni sotto-milestone l'elenco comprende almeno:

- avvio e fine regolare di ogni profilo nella durata più breve;
- errore iniettato visibile come «Instabile · core N»;
- «Ferma e salva»;
- «Ferma il test» dalla tray;
- chiusura della finestra con il test che continua e il toast finale;
- stop termico con una soglia bassa impostata apposta;
- diario dopo un riavvio forzato, che l'utente fa solo se vuole;
- WHEA leggibili;
- tooltip presenti in tutte le pagine;
- contagiri fluidi e budget della finestra.

Per la M8d valgono le verifiche del §7.6, che sostituiscono quelle con le issue e con Pages.

## 14. Limiti dichiarati

- **RAM.** I test della RAM coprono solo la memoria che Windows concede al processo: non sostituiscono MemTest86 o TestMem5.
- **Errori fatali e crash del sistema.** I WHEA fatali e i crash si vedono solo dopo il riavvio. Il «core N» del diario dice quale core era in prova, non prova che sia il colpevole.
- **Errori silenziosi della VRAM.** S3 li rileva solo come calo della banda: un overclock della memoria che ne produce pochi può passare.
- **Classifica.** All'inizio è povera: poche righe, quasi tutte dell'autore. La community la arricchisce solo se partecipa, e serve un account GitHub. I punteggi della community si possono falsificare: la mediana su almeno 3 utenti e la moderazione limitano il danno, ma non lo azzerano.
- **Benchmark del disco.** Misura il file in una cartella, attraverso il file system, non il volume grezzo.

## 15. Punti che i piani devono fissare

- **Ordine dei task della M8a:**
  - protocollo e `oma-load` vuoto;
  - topologia e affinità;
  - kernel con verifica, uno per task;
  - modi di carico e profili;
  - app: host, controller, diario, WHEA, stop termico, cronologia;
  - interfaccia: vista, procedura guidata, durante e risultato, cronologia, tooltip;
  - benchmark e contagiri;
  - tray e impostazioni;
  - installer e firma (quinto file firmato, `sign-shim.ps1 -Mode register-payload`, risorsa di versione con `tauri-winres`, cancellazione nel disinstallatore);
  - documentazione e verifiche dal vivo.

  Decidere se dividere la M8a in M8a1 e M8a2.
- **Dimensioni e conteggi:** dimensioni esatte di ogni kernel per cache e set d'istruzioni, lavoro fisso dei carichi del benchmark e numero di ripetizioni; corrispondenza fra «core N» e la numerazione di Ryzen Master e del BIOS (`CoreIndex`, APIC ID).
- **Stop termico:** sensori e id da usare (Tctl/Tdie, package, hot spot GPU, temperatura NVMe) e come trovare il Tjmax.
- **Chi tiene la cronologia:** i moduli Rust che leggono e scrivono la cronologia, e i comandi Tauri e gli eventi verso la UI (`performance-*`).
- **M8b:** gli esiti dello spike (§5.6) e la scelta della compilazione degli shader.
- **M8c:** quali cartelle «usate di recente» si puliscono all'avvio e dove si registra che sono state usate. Deciso nel §6.4: la cartella del diario aperto e l'ultima cartella usata.
- **M8d:** con il Worker del §7.6 cadono la CLI `oma-scores`, le prove delle Action, il modello della issue e Pages. Restano da fissare:
  - i limiti attuali del piano gratuito;
  - i numeri del rate limit e i tetti per categoria;
  - lo schema di D1;
  - l'informativa nel README.
- **Versione e release:** quale numero di versione esce con ogni sotto-milestone (lo decide l'utente).
