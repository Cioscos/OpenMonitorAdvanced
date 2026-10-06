# M8a1 — Fondamenta delle Prestazioni e stress test di CPU e RAM: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** aggiungere la vista «Prestazioni» con lo stress test di CPU e RAM:
- procedura guidata, schermate «durante il test» e «risultato», cronologia, tooltip su ogni termine tecnico;
- carichi verificati in un processo ausiliario `oma-load.exe`;
- stop termico, WHEA dal vivo, diario dei crash, tray e impostazioni.

**Architecture:**
- **`oma-ipc::load`:** protocollo app↔`oma-load`, versione 1, sul framing esistente; contiene anche il piano delle fasi (A1).
- **`oma-core::load` (puro):**
  - catalogo, profili e costruzione dei piani (A2);
  - esiti, soglie termiche, formati di sessione e diario, cronologia (A3);
  - il controller della sessione, una macchina a stati guidata da messaggi, campioni e orologio (A4).
- **`oma-win`:**
  - pipe privata resa generica, Job Object, topologia della CPU, memoria libera, sospensione impedita (A5);
  - lettura del registro eventi (WHEA, BugCheck, Kernel-Power) (A6).
- **`crates/oma-load` (nuovo binario):**
  - collegamento, affinità e priorità (A7);
  - motore delle fasi con la verifica in stile OpenDCDiag (A8);
  - i kernel K1–K10, uno per task (A9–A15);
  - prova da capo a fondo (A16).
- **App (Rust):** host del processo (A17), impostazioni (A18), archivio e ripresa dopo un crash (A19), runner, comandi, eventi e toast (A20), tray e uscita (A21).
- **UI (Svelte):** fondamenta della vista e `Term.svelte` con il glossario (A22), procedura guidata (A23), durante il test e risultato (A24), cronologia (A25).
- **Chiusura:** installer, firma e licenze (A26), documenti e misure (A27), prove dal vivo con l'utente (A28).

**Tech Stack:** Rust 1.90 (workspace `rust-version` 1.85) con `core::arch` (AVX-512, AVX2+FMA, SSE2, AES-NI, SHA-NI, SSE4.2, PCLMULQDQ), crate `windows` 0.62, Tauri 2.11 (con `tauri-plugin-dialog` 2.7 già presente) + Svelte 5 + TypeScript 6 + Vitest, NSIS, PowerShell 7 + Pester 5.7.1.

**Spec:** `docs/superpowers/specs/2026-10-06-m8-prestazioni-design.md`:
- §1, §2, §3 (tranne §3.2 e §3.3), §4.1–§4.5;
- dal §8: §8.1 e §8.3;
- §9, §10, §11, §12 per FIRESTARTER e OpenDCDiag, §13, §14 e §15 per la M8a.

La ricerca con le fonti sta in `docs/superpowers/references/m8/research-cpu.md`.

**Branch:** `feat/m8a1-stress-cpu` da `main`, dopo il merge in locale di `docs/m8-spec` (A1 passo 1). Alla fine si fa il merge in `main` in locale. Push e release solo su richiesta dell'utente.

**Esecuzione:**
- **Subagent-driven:** un implementer e una revisione per task, poi la revisione dell'intero branch.
- **Revisioni in più:**
  - `ffi-safety-reviewer` dopo A5, A6, A7, A9 e A15;
  - `security-review` dopo A5, A17 e A20: pipe, piano ricevuto, id dei file della cronologia;
  - `frontend-design:frontend-design` prima di A22–A25.
- **Prove dal vivo:** A28, con l'utente.

## Global Constraints

- **Lingua e formato:**
  - codice, commenti e messaggi di commit in inglese (conventional commits);
  - documentazione e prosa in italiano con gli accenti corretti;
  - fine riga LF ovunque;
  - ogni commit termina con `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.
- **Divieti per gli agenti:**
  - mai clic sintetici, UI Automation o tasti inviati al desktop, al tray o alle finestre dell'app;
  - mai installer eseguiti, mai test Pester `Integration`, mai comandi elevati;
  - mai ricerche a tutto il disco.
- **Carico della CPU (memoria «no heavy load without warning»):**
  - un agente non avvia mai uno stress test vero né `oma-load.exe` a tutti i thread;
  - i test automatici usano al massimo 2 thread, dimensioni piccole e al massimo 3 s per prova;
  - la suite completa si esegue una volta per verifica, non in un ciclo;
  - i test a tutti i core si fanno solo nelle prove dal vivo, chiesti all'utente.
- **Risparmio (preferenza dell'utente):**
  - senza un test, `oma-load` non esiste e la vista Prestazioni non fa lavoro periodico;
  - gli eventi verso la UI partono solo con una finestra aperta;
  - la sessione tiene un campione ogni 5 s.
- **TDD e debug:** prima il test che fallisce (`superpowers:test-driven-development`); davanti a un test che fallisce senza un motivo chiaro, `superpowers:systematic-debugging`.
- **graphify:** ogni brief riporta `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"` per orientarsi. Dopo le modifiche al codice: `PYTHONHASHSEED=0 graphify update .`.
- **FFI Rust:**
  - un commento `// SAFETY:` per ogni blocco `unsafe`, compresi gli intrinseci `#[target_feature]`;
  - un assert di dimensione a compile time per ogni struct FFI scritta a mano;
  - helper puri testati senza hardware;
  - i test che chiedono hardware reale o il registro eventi vero sono `#[ignore = "requires real Windows hardware"]`.
- **Kernel:**
  - nessun codice generato a runtime;
  - il codice generato al build (A9) va bene;
  - ogni set d'istruzioni si sceglie con `is_x86_feature_detected!`, e un kernel non chiama mai un percorso che la CPU non ha;
  - un test di kernel salta un set assente sulla macchina e lo dichiara con `eprintln!("skipped: <isa> not available")`.
- **Protocollo `oma-ipc::load`:**
  - mai `skip_serializing_if`: chiavi sempre presenti, `nil` per gli assenti;
  - enumerati come stringhe `snake_case`;
  - niente `deny_unknown_fields`;
  - il ricevente chiama `LoadMessage::validate`;
  - fixture in `protocol/fixtures/load/`, rigenerate solo con `OMA_WRITE_FIXTURES=1`, a thread singolo.
- **Dipendenze:**
  - nessun nuovo crate a runtime, nessun pacchetto npm nuovo;
  - feature nuove del crate `windows`: `Win32_System_JobObjects` e `Win32_System_EventLog` in `oma-win`;
  - `oma-load` usa un sottoinsieme delle feature già usate da `oma-win`, scritto in A7;
  - dipendenza di build `tauri-winres` 0.3 per `oma-load`, come `oma-overlay`;
  - `pwsh scripts/generate-licenses.ps1 -Check` deve passare.
- **Codice di terzi (§12):**
  - i file che adattano FIRESTARTER o OpenDCDiag portano in testa la nota d'origine dettata in A8 e A9, con il copyright copiato dai file a monte al commit fissato;
  - nessun tag SPDX di terzi.
- **Nomi fissi:**
  - eseguibile `oma-load.exe`, installato in `$INSTDIR\oma-load.exe`;
  - pipe `\\.\pipe\OpenMonitorAdvanced-Load-<uuid v4>`;
  - cartelle `%LOCALAPPDATA%\OpenMonitorAdvanced\performance\` e `...\performance\stress\`;
  - diario `...\performance\journal.json`;
  - eventi Tauri `performance-status` e `performance-quit`;
  - sezione `performance` di `settings.json`;
  - chiavi `glossary.<termine>` e `glossary.mode.<id>`.
- **i18n:**
  - stesse chiavi in `app/src/lib/i18n/en.json` e `it.json`;
  - ogni chiave letta da Rust compare in `RUST_KEYS` (`app/src-tauri/src/i18n.rs`);
  - i testi italiani esatti sono nelle tabelle T1–T4 del piano; l'inglese lo traduce l'implementer e la revisione ne controlla il senso.
- **Budget (§11):**
  - a riposo non cambia niente;
  - durante un test la finestra resta sotto i 200 MB;
  - `oma-load` usa pochi MB per thread, tranne K3 e K10 (quota di RAM).

## Decisioni del piano

Fissano i punti del §15 per la M8a e le scelte che la spec lascia aperte. Il revisore le tratta come requisiti.

| # | Decisione | Perché |
|---|---|---|
| DA1 | **La M8a si divide.**<br>• **M8a1** (questo piano): tutto il §1.2 della M8a tranne il benchmark.<br>• **M8a2** (piano a parte, dopo la M8a1): benchmark della CPU (§4.6), contagiri (§3.3), pagina di punteggio (§3.2), file dei punteggi (§8.2), font Orbitron e Share Tech Mono, `oma-core::scores`.<br>Nella M8a1 la barra laterale mostra solo il gruppo «Stress test».<br>Delle impostazioni del §3.8 la M8a1 fa solo quelle della CPU e della RAM: `gpuStopC`, `diskStopC` e `communityTable` arrivano con M8b, M8c e M8d. | Insieme supera di molto i piani precedenti (500–1200 righe). Lo stress test sta in piedi da solo, e il benchmark riusa i kernel. Le chiavi in più si aggiungono senza rompere niente, grazie alla lettura tollerante delle impostazioni. |
| DA2 | **Dove stanno i tipi.**<br>• Il piano delle fasi (`Plan`, `Phase` e gli enumerati) è un tipo del protocollo in `oma-ipc::load`, validato lì.<br>• `oma-core` prende la dipendenza da `oma-ipc` e costruisce i piani; `oma-ipc` resta senza `oma-core`.<br>• La sessione salvata (§8.1) contiene il `Plan` così com'è. | Un solo tipo per il piano dall'app al processo e nel file. |
| DA3 | **La topologia si legge in `oma-win::topology`**, che usano sia l'app sia `oma-load`:<br>• l'app la usa per la procedura guidata e per il piano (numero di core, P/E, cache);<br>• `oma-load` aggiunge l'APIC ID di ogni processore logico, eseguendo CPUID foglia 0x0B (EDX) su un thread fissato a quel processore, e manda il messaggio `Topology`.<br>Affinità, priorità ed EcoQoS restano in `oma-load` (§2.1). | L'app ha bisogno della topologia prima di avviare il processo. L'APIC ID collega i WHEA 19 a «core N». |
| DA4 | **«Core N» (§4.4, §15).**<br>• Numerato da 0, nell'ordine crescente dei `CoreIndex` distinti di `GetSystemCpuSetInformation`.<br>• È la numerazione del Curve Optimizer nel BIOS (Core 0…) e di CoreCycler.<br>• Ryzen Master conta da 1 (C01 = core 0), e il tooltip `glossary.coreNumber` lo dice.<br>• Con core disattivati di fabbrica la numerazione del BIOS può saltare: lo dice il tooltip e la prova P11 lo confronta sul PC dell'utente. | La numerazione da 0 è quella che serve per correggere il Curve Optimizer. |
| DA5 | **Sensori dello stop termico e del riepilogo (§15).** Sono tutti del dispositivo `cpu/0`, con un campione a ogni tick del sampler (predefinito 1 s).<br>• **Temperatura:** il primo presente fra `cpu/0/temperature/tdie`, `.../tctl`, `.../package`, `.../core-max`.<br>• **Potenza:** `cpu/0/power/package`.<br>• **Clock:** il primo fra `cpu/0/clock/average-effective`, `.../average`, `.../effective`.<br>• **Clock del core N:** `cpu/0/clock/core-<N+1>-effective`, altrimenti `cpu/0/clock/core-<N+1>`, perché LHM numera da 1.<br>• **Tjmax:** la proprietà `tjMaxC` del dispositivo `cpu/0`, già pubblicata dal servizio.<br>• **Soglia:** `cpuStopC` se impostato, altrimenti Tjmax − 5, altrimenti 95.<br>Un valore vale solo con qualità `Fresh` o `Held`. | La regola integrata `cpu-temp` usa gli stessi nomi; `tjMaxC` c'è già (tabella AMD, parametro LHM per Intel). |
| DA6 | **K1 senza confronto ad anello.**<br>• Si adattano da FIRESTARTER i gruppi d'istruzioni (DA8) e l'hash CRC32 dei registri vettoriali.<br>• Gli accumulatori ripartono dallo stesso stato a ogni blocco di 16 384 iterazioni, così l'hash di ogni blocco è identico per tutti i thread e uguale al riferimento calcolato su tre core.<br>• Il confronto coi vicini ad anello del §4.1 diventa il confronto con questo riferimento comune.<br>Supera la riga K1 della tabella del §4.1. | L'esito è lo stesso, ma l'errore si attribuisce al core giusto anche con un solo thread. Il ripartire dallo stesso stato impedisce agli accumulatori di arrivare a ±inf, il difetto di FIRESTARTER 1. |
| DA7 | **Verifica (§4.2).**<br>• Ogni fase calcola il riferimento su tre processori logici di **tre core fisici diversi**: primo, centrale e ultimo nell'ordine di DA4; con meno core, quelli che ci sono.<br>• I tre risultati devono coincidere, altrimenti c'è un `Error` di tipo `reference_disagreement`.<br>• Ogni iterazione confronta un digest a 64 bit con il riferimento, senza tolleranza.<br>• Le prove con tolleranza (somma in stile SUMINP/SUMOUT, andata e ritorno della FFT) valgono solo sul riferimento, dentro il calcolo del riferimento: segnalano un difetto dell'implementazione (`reference_invalid`), mai un errore di un core.<br>• K9 e K10 si verificano da soli (checksum e pattern noti), senza riferimento. | È il modello di OpenDCDiag (riferimento e confronto). Nessun falso positivo da arrotondamento. |
| DA8 | **Gruppi d'istruzioni di K1**, da FIRESTARTER v2.2 (commit `927ae17e55f3f90f7575f6a68630a366fde9c94e`), con 1536 righe svolte. Le voci L3 e RAM si tolgono, perché K1 resta in L1/L2 (§4.1).<br>• **AVX-512**, da `SkylakeSPConfig.hpp`: `REG:140,L1_L:40,L2_L:70,L2_S:4`.<br>• **AVX2+FMA**, da `HaswellConfig.hpp`: `REG:40,L1_LS:90,L2_LS:9`.<br>• **SSE2**: gli stessi gruppi di AVX2, con `mulpd` + `addpd` al posto di FMA.<br>Il codice svolto lo genera `crates/oma-load/build.rs` da questi testi (A9): è codice generato al build, non a runtime. | FIRESTARTER genera il codice con asmjit a runtime, e la spec lo vieta. Il build.rs riproduce lo stesso flusso d'istruzioni. |
| DA9 | **Dimensioni dei dati (§15).** «Per thread» si calcola sui thread della fase:<br>• `l2_thread` = L2 del core diviso i thread della fase su quel core;<br>• `l3_share` = L3 del dominio diviso i thread della fase su quel dominio.<br>Le potenze di 2 sono le più grandi che rispettano la condizione, con il minimo indicato.<br>• **K1:** zona L1 = L1d/2, zona L2 = `l2_thread`/2.<br>• **K2:** `l1` ha N complessi f64 con 16·N ≤ L1d/2 (min 64); `l2` ha 16·N ≤ 0,6·`l2_thread` (min 1024).<br>• **K3:** per thread min(1 GiB, quota/thread), almeno 256 MiB; N con 16·N ≤ quel valore.<br>• **K4:** N da quello di K2 `l1` a quello di K3 raddoppiando, 20 s per passo, poi da capo.<br>• **K5:** `l2` ha 8·N ≤ 0,5·`l2_thread`; `l3` ha 8·N ≤ 0,5·`l3_share`; `ram` ha 8·N = 128 MiB per thread, nella quota.<br>• **K7:** n multiplo di 8; `l2` con 24·n² ≤ 0,75·`l2_thread`; `l3` con 24·n² ≤ 0,75·`l3_share`; `ram` n = 2048, oppure 1024 se la quota non basta.<br>• **K8:** buffer da 64 KiB per AES, SHA-256, CRC32C e CLMUL; compressione su 256 KiB; ordinamento di 65 536 `u32`.<br>• **K9:** anello di 64 righe da 64 byte per coppia di thread.<br>• **K10:** la quota divisa per thread, in blocchi da al massimo 1 GiB.<br>Ogni iterazione deve durare al massimo 2 s su una CPU lenta; i kernel più lunghi aggiornano il battito a metà del lavoro. | Sono i valori del §4.1 e della ricerca (§2.3), fissati in numeri. |
| DA10 | **Quota di RAM e riduzione (§3.8, §9).**<br>• Quota = min(disponibile × `ramSharePercent`/100, disponibile − 2 GiB), dalla memoria disponibile all'avvio; si mostra nel riepilogo.<br>• Se l'allocazione fallisce, K3, K4 e K10 dimezzano la memoria per thread fino a 256 MiB e lo scrivono con `Notice { code: "ram_reduced" }`.<br>• Sotto i 256 MiB per thread, K3 e K4 passano a un thread per core fisico. Se ancora non basta, la fase si salta con `Notice { code: "ram_insufficient" }`.<br>Il «512 MB» del §9 diventa 256 MiB. | Il §4.1 fissa la soglia a 256 MB. Con 512 MB, una CPU da 32 thread e 16 GB di RAM salterebbe sempre K3. |
| DA11 | **Profili e scalette (§4.5).** Le durate sono in secondi e il totale è sempre uguale alla durata scelta.<br>• **CPU · Verifica normale**, al miglior set d'istruzioni, `all_logical` e `steady`, senza fermarsi agli errori:<br>&nbsp;&nbsp;– Rapido (300): K2 `l2` 120, K5 `l2` 90, K8 90;<br>&nbsp;&nbsp;– Standard (1800): K1 600, K2 `l2` 360, K7 `l3` 300, K8 300, K3 240;<br>&nbsp;&nbsp;– Lungo (3600): gli stessi × 2.<br>• **CPU · Stabilità overclock**: un giro composto da<br>&nbsp;&nbsp;– (1) a tutti i core, AVX2: K2 `l2`, K5 `l3`, K7 `l3`, K3, 300 s ciascuno, con `stop_on_error`;<br>&nbsp;&nbsp;– (2) un core alla volta: K2 `l2` AVX2, K5 `l2`, poi K2 SSE2 `light`, tre fasi `core_cycle` di t/3 per core ciascuna, senza fermarsi;<br>&nbsp;&nbsp;– (3) K1 `variable` con `alt_kernel` K5, 300 s;<br>&nbsp;&nbsp;– (4) K4 300 s e K9 300 s;<br>&nbsp;&nbsp;– (5) solo con AVX-512: K2 `l2` e K1 in AVX-512, 300 s ciascuno.<br>&nbsp;&nbsp;Tempo t per core: 180 s (Standard 3600), 300 (Lungo 7200), 600 (Notte 28 800).<br>&nbsp;&nbsp;Adattamento (A2): se il giro supera la durata, t scende fino a 60 s, multiplo di 3; poi le fasi a tutti i core scendono in proporzione fino a 120 s, poi fino a 60.<br>&nbsp;&nbsp;Il giro si ripete finché resta tempo; l'ultima fase si accorcia al resto; un resto sotto i 60 s si aggiunge alla fase precedente.<br>• **RAM · Verifica normale** (900, 1800, 3600): K10 con `moving_inversions`, `random` e `crc_copy` per il 70%, poi K3 per il 30%, senza fermarsi.<br>• **RAM · Stabilità overclock** (3600, 7200, 28 800): K10 con tutti i pattern 60%, K3 25%, K4 15%, con `stop_on_error`.<br>• I core già segnati come sbagliati si saltano nelle fasi `core_cycle` successive. | Fissa il «con le fasi in scala» e il «ridotto se i core sono tanti» della spec in un algoritmo verificabile. |
| DA12 | **«Personalizza» (§3.4)** è un `Custom` che si applica al piano costruito:<br>• per ogni kernel: incluso o no, e minuti totali (le sue fasi si scalano in proporzione);<br>• set d'istruzioni forzato: le fasi `light` restano SSE2;<br>• thread `all_logical` o `one_per_core`, solo per le fasi a tutti i core;<br>• «Fermati al primo errore» sì o no.<br>La durata totale diventa la somma; il massimo è 24 h. | Il §3.4 parla di modalità e durata ciascuna, non di fasi. |
| DA13 | **Esiti (§2.3): precedenza** quando ne valgono più d'uno:<br>`failed_to_start` > `system_crash` > `crashed` > `hung` > `errors` > `stopped_thermal` > `suspended` > `stopped_user` > `marginal` > `passed`.<br>«Instabile · core N» si usa quando tutti gli errori sono sullo stesso core; altrimenti «Errori trovati». | Un test fermato a mano dopo un errore deve dire «Errori trovati». |
| DA14 | **Crash di sistema o dell'app (§2.4).** All'avvio, un diario presente diventa:<br>• `system_crash` se l'avvio del sistema (ora attuale − `GetTickCount64`) è successivo a `updatedAt`;<br>• altrimenti `crashed` con il dettaglio `app_closed`, perché si è chiusa solo l'app.<br>Gli eventi del registro si cercano da `updatedAt` − 60 s all'avvio dell'app. Un toast dice che il test precedente si è interrotto e apre il risultato. | Distingue un BSOD da un'app chiusa dal Task Manager. |
| DA15 | **Sospensione (§2.6).**<br>• `oma_win::power::asleep_ms()` = `GetTickCount64()` − `QueryUnbiasedInterruptTime()`/10 000. Il valore cresce solo mentre il PC dorme, e non cambia se l'utente sposta l'ora (§9).<br>• Il runner dell'app gira ogni 250 ms. Se fra due giri `asleep_ms` cresce di più di 1000 ms, la sessione si chiude come `suspended`, prima di ogni controllo sulla pipe muta.<br>• La sentinella di `oma-load` azzera la base dei battiti quando `asleep_ms` cresce.<br>• `SetThreadExecutionState` si chiama sul thread del runner, perché vale per il thread che la chiama, e si ripristina quando la sessione finisce. | Dopo una sospensione la pipe torna a parlare: senza questo controllo la sessione diventerebbe `hung`. Un salto dell'orologio di sistema non basta, perché scambierebbe un cambio d'ora per una sospensione. |
| DA16 | **Chiusura durante un test (D11).**<br>• Chiudere la finestra principale lascia l'app nella tray anche con `tray.closeToTray` spento.<br>• «Esci» dal tray, con un test in corso, apre la finestra e chiede «Fermare il test e uscire?». Si chiede prima dell'editor con modifiche: dopo la conferma il flusso dell'editor resta quello di oggi.<br>• `--quit` e la fine dell'app (`RunEvent::Exit`) fermano il test e salvano `stopped_user` senza chiedere: `Stop`, attesa di `Finished` per al massimo 2 s, poi il Job chiude il processo. | §2.6 e D11. L'installer deve poter chiudere l'app. |
| DA17 | **Tray (§3.9).**<br>• Durante un test l'icona porta un punto `--warn` in alto a destra; il punto rosso della registrazione resta in basso.<br>• Il tooltip inizia con «Stress test in corso: <componente> · <obiettivo>».<br>• Il menu ha «Ferma il test» e «Apri il test in corso» solo mentre un test gira.<br>• Il toast finale arriva sempre per lo stress test, e un clic apre il risultato (`LaunchTarget::Performance`). | La spec non fissa il segno. |
| DA18 | **Iniezione d'errori (§13.1).**<br>• Solo con `debug_assertions`, `oma-load --inject-fault <kernel>[:<core>]` capovolge il bit 0 del digest dell'iterazione 3 del primo thread su quel core, oppure del primo thread se manca il core.<br>• L'app in debug passa l'argomento letto dalla variabile d'ambiente `OMA_LOAD_INJECT`, per le prove dal vivo.<br>• Nelle build di release l'argomento non esiste: dà `EXIT_USAGE`. | L'utente può vedere «Instabile · core N» senza un overclock instabile. |
| DA19 | **Catalogo condiviso con la UI.**<br>• `testdata/performance/catalog.json` elenca id dei kernel, set d'istruzioni, modi di carico, pattern della RAM, componenti, obiettivi e preset.<br>• Lo scrive e lo controlla un test di `oma-core` (`OMA_WRITE_FIXTURES=1` per rigenerarlo).<br>• Il test del glossario di Vitest lo legge (§3.7). | Lo stesso schema della fixture di geometria della M7d. |
| DA20 | **Comandi ed eventi (§15).** Tutti in `app/src-tauri/src/performance/commands.rs` e in `default.json`; l'editor non li ha.<br>• `performance_system`, `performance_preview`, `performance_start`, `performance_stop`, `performance_status`;<br>• `performance_history`, `performance_session`, `performance_delete`, `performance_export`;<br>• `performance_quit_confirmed`.<br>Eventi: `performance-status` (al cambio di stato e a 1 Hz durante un test, solo con una finestra aperta) e `performance-quit`. | Un test si avvia solo dalla finestra principale (§9). |

## Review Focus

1. **App chiusa male durante un test**, dal Task Manager o con un crash di `oma-app`. Atteso:
   - `oma-load` muore subito, per il Job Object e per la pipe chiusa;
   - al riavvio la sessione è `crashed` con `app_closed`, non `system_crash`;
   - il diario sparisce.

   Test: A5, `killing_the_job_kills_the_child`; A19, `journal_older_than_boot_is_system_crash`, `journal_newer_than_boot_is_app_closed`.
2. **PC in sospensione durante un test** (coperchio, pulsante). Atteso: la sessione è `suspended`, mai `hung`; `oma-load` non segnala thread bloccati alla ripresa.

   Test: A4, `sleep_ends_as_suspended_before_the_pipe_check`, `wall_clock_change_is_not_a_suspend`; A8, `sentinel_ignores_a_suspend_gap`.
3. **Topologie insolite:**
   - più di 64 processori logici in due gruppi;
   - CPU ibride P/E con core parcheggiati;
   - un solo core;
   - SMT spento;
   - L3 assente.

   Atteso: piani validi, «core N» stabile, riferimento con i core che ci sono, nessun panic.

   Test: A2, `plan_for_128_logical_in_two_groups`, `core_cycle_orders_p_before_e_and_skips_parked`, `single_core_machine_builds_every_profile`; A8, `reference_with_one_core_uses_it_alone`.
4. **Poca RAM o RAM che sparisce all'avvio.** Atteso: la quota lascia sempre 2 GiB, la fase si riduce e poi si salta con un avviso nel diario, nessun errore di memoria esaurita.

   Test: A2, `ram_budget_leaves_two_gib_free`; A15, `allocation_failure_halves_then_skips`.
5. **Dati ostili o rotti:**
   - un piano enorme dalla pipe;
   - un messaggio con `NaN`;
   - un `journal.json` troncato;
   - una sessione di una versione futura;
   - un id della cronologia con `..\` o `/`.

   Atteso: rifiuto o salto con una riga nel log, nessun file toccato fuori dalla cartella, nessun ciclo all'avvio.

   Test: A1, `oversized_plan_is_rejected`, `non_finite_values_are_rejected`; A19, `corrupt_journal_is_removed_and_logged`, `future_format_sessions_are_skipped`, `ids_outside_the_session_form_are_rejected`.

## Tabelle dei testi (italiano esatto)

### T1. Glossario: modalità (`glossary.mode.<id>`) e set d'istruzioni (`glossary.isa.<id>`)

Ogni voce ha anche un titolo `glossary.mode.<id>.name`, che è il nome mostrato nell'interfaccia.

| Chiave | Nome | Spiegazione |
|---|---|---|
| `mode.k1` | Carico massimo (FMA) | Fa lavorare al massimo le unità di calcolo con istruzioni FMA, come FIRESTARTER: è il carico che fa consumare e scaldare di più la CPU. Prova raffreddamento, alimentazione (VRM) e tenuta del boost. Ogni blocco di calcoli si confronta con un riferimento. |
| `mode.k2` | FFT piccole · core e cache | Trasformate di Fourier (FFT) su dati che stanno nella cache del core. Sollecitano le unità di calcolo e le cache L1/L2: è il test classico per trovare un core instabile. Il risultato deve essere identico bit per bit al riferimento. |
| `mode.k3` | FFT grandi · memoria | Le stesse FFT su dati molto più grandi della cache, quindi in RAM. Sollecitano il controller di memoria, la RAM e i collegamenti interni della CPU (per esempio Infinity Fabric). Usano la quota di RAM delle impostazioni. |
| `mode.k4` | Misto (blend) | Alterna FFT di dimensioni diverse, dalla cache alla RAM, come il «Blend» di Prime95: prova i passaggi fra cache e memoria. |
| `mode.k5` | Interi esatti (NTT) | Calcoli esatti su interi a 64 bit con la NTT, la trasformata che serve a moltiplicare numeri enormi (come in y-cruncher). Sollecita il moltiplicatore intero; non ammette tolleranze: un solo bit sbagliato è un errore. |
| `mode.k7` | Linpack | Moltiplicazioni di grandi matrici, il calcolo di Linpack: il carico in virgola mobile più continuo, che scalda molto. Il risultato si controlla in modo esatto con il metodo di Freivalds. |
| `mode.k8` | Crittografia e compressione | Crittografia (AES, SHA-256), codici di controllo (CRC32C), compressione e ordinamento: provano le unità specializzate della CPU e la previsione dei salti, che gli altri test usano poco. I risultati si confrontano con valori noti. |
| `mode.k9` | Scambio fra core | Due core alla volta si passano blocchi di dati con un codice di controllo: prova la coerenza delle cache e il collegamento fra i core e fra i CCD. |
| `mode.k10` | Pattern di memoria (RAM) | Scrive e rilegge schemi di bit nella RAM: inversioni, modulo 20, dati casuali, indirizzo nell'indirizzo, copia con CRC. Lavora sulla memoria che Windows concede al programma, con pagine da 4 KB: non sostituisce MemTest86, TestMem5 o Karhu. |
| `mode.steady` | Carico costante | Tutti i thread lavorano al 100% senza pause: prova calore e consumi. |
| `mode.variable` | Carico variabile | Lavoro e pause brevi scelti a caso (da 10 a 500 ms), con salti fra un test e l'altro. Prova i cambi rapidi di tensione e frequenza, dove un undervolt o un Curve Optimizer spesso cedono. |
| `mode.light` | Carico leggero | Un thread con pause, che lascia salire il boost al massimo. Molte instabilità del Curve Optimizer compaiono proprio a basso carico. |
| `mode.coreCycle` | Un core alla volta | Il test gira su un solo core fisico per qualche minuto, poi passa al successivo. Così si scopre quale core sbaglia. |
| `mode.allCore` | Tutti i core | Un thread per ogni processore logico: il carico più alto per calore e consumi. |
| `mode.pattern.moving_inversions` | Inversioni | Scrive un valore, lo rilegge e lo sostituisce con il suo opposto, avanti e indietro in tutta la memoria. |
| `mode.pattern.modulo20` | Modulo 20 | Scrive un valore ogni 20 posizioni e altro altrove: un test che le cache non riescono a nascondere. |
| `mode.pattern.random` | Dati casuali | Riempie la memoria di dati pseudo-casuali da un seme e li rilegge. |
| `mode.pattern.address` | Indirizzo nell'indirizzo | Ogni posizione contiene il proprio indirizzo: trova le righe e le colonne scambiate. |
| `mode.pattern.crc_copy` | Copia con CRC | Copia blocchi da una zona all'altra calcolando il CRC durante la copia, come stressapptest: prova la memoria mentre i dati viaggiano. |
| `isa.avx512` | AVX-512 | Istruzioni vettoriali a 512 bit (Zen 4 e Zen 5, alcune CPU Intel). È un carico a parte: un overclock stabile in AVX2 può non esserlo in AVX-512. |
| `isa.avx2` | AVX2 | Istruzioni vettoriali a 256 bit con FMA, presenti in quasi tutte le CPU dal 2013. È il set più usato dai programmi. |
| `isa.sse2` | SSE2 | Istruzioni vettoriali a 128 bit, presenti in tutte le CPU a 64 bit. Scaldano meno e lasciano salire il boost più in alto. |

### T2. Glossario: termini (`glossary.<termine>`)

| Chiave | Termine | Spiegazione |
|---|---|---|
| `fft` | FFT | Trasformata di Fourier veloce: un calcolo che scompone i dati in frequenze. Muove molti dati e usa a fondo le unità in virgola mobile. |
| `ntt` | NTT | La versione della FFT sui numeri interi, esatta: serve a moltiplicare numeri enormi. |
| `linpack` | Linpack | Il test classico dei supercomputer: risolve grandi sistemi di equazioni con moltiplicazioni di matrici. |
| `fma` | FMA | Un'istruzione che fa una moltiplicazione e una somma insieme: è il calcolo che consuma di più. |
| `smt` | SMT / Hyper-Threading | Ogni core fisico esegue due thread. «Un core alla volta» di solito ne usa uno solo, perché un core da solo sale al boost più alto. |
| `ccd` | CCD | Un blocco di core di una CPU AMD con la sua cache L3. Le CPU con più CCD hanno impostazioni di overclock separate per ognuno. |
| `coreType` | Core P ed E | Nelle CPU Intel ibride i core P (prestazioni) sono veloci, i core E (efficienza) consumano poco. Il test prova prima i core P. |
| `coreNumber` | Numero del core | «Core 0» è il primo core fisico, come nel Curve Optimizer del BIOS. Ryzen Master conta da 1 (C01 = core 0). Con core disattivati di fabbrica la numerazione del BIOS può saltare un numero. |
| `apicId` | APIC ID | Il numero con cui l'hardware identifica un processore logico. Un errore WHEA dice quale processore l'ha segnalato, non per forza quale l'ha causato. |
| `tjmax` | Tjmax | La temperatura massima ammessa dalla CPU. Lo stop termico si ferma 5 °C sotto, se Tjmax è noto. |
| `thermalStop` | Stop termico | Il test si ferma da solo se la temperatura supera la soglia per due letture di fila. |
| `throttling` | Throttling | La CPU abbassa da sola la frequenza perché è troppo calda o consuma troppo. |
| `whea` | WHEA | Gli errori hardware che Windows registra. Quelli «corretti» li ha già sistemati l'hardware, ma dicono che il sistema è al limite. |
| `curveOptimizer` | Curve Optimizer | L'impostazione AMD che abbassa la tensione di ogni core. Un valore troppo basso dà errori di calcolo, spesso a basso carico. |
| `pbo` | PBO | Precision Boost Overdrive: l'overclock automatico di AMD, che alza limiti di potenza e boost. |
| `expoXmp` | EXPO / XMP | Profili della RAM che ne alzano frequenza e timing. Sono un overclock della memoria e del controller. |
| `vrm` | VRM | Il circuito della scheda madre che alimenta la CPU. Sotto carico pieno si scalda anche lui. |
| `cState` | C-state | Gli stati di riposo dei core. Entrare e uscire da questi stati fa variare la tensione di colpo. |
| `boost` | Boost | La frequenza in più che la CPU raggiunge quando ha margine di calore e consumi. |
| `imc` | Controller di memoria | La parte della CPU che parla con la RAM. Con EXPO/XMP lavora più veloce del normale. |
| `cache` | Cache L1, L2, L3 | Memorie piccole e velocissime dentro la CPU. L1 e L2 sono di ogni core, la L3 è condivisa. |
| `check` | Verifica | Ogni volta che un calcolo finisce, il risultato si confronta con quello giusto. Il numero dice quanti confronti sono stati fatti. |
| `reference` | Riferimento | Il risultato giusto, calcolato all'inizio su tre core diversi che devono essere d'accordo. |
| `seed` | Seme | Il numero da cui nascono i dati del test. Con lo stesso seme il test si ripete identico. |
| `iteration` | Iterazione | Un giro completo del calcolo di una fase. |
| `ramShare` | Quota di RAM | La parte della memoria libera usata dai test della RAM. Restano sempre almeno 2 GB per Windows. |

### T3. Esiti e verdetti (`performance.outcome.<id>`)

| Esito | Testo |
|---|---|
| `passed` | Superato |
| `marginal` | Superato con avvisi: errori corretti dall'hardware |
| `errors` | Errori trovati |
| `errors_core` | Instabile · core {core} |
| `crashed` | Instabile: il test si è chiuso |
| `crashed_app` | Interrotto: l'app si è chiusa durante il test |
| `hung` | Instabile: il test si è bloccato |
| `system_crash` | Interrotto da un crash del sistema durante {phase} |
| `stopped_user` | Fermato da te |
| `stopped_thermal` | Fermato: temperatura a {temp} °C |
| `suspended` | Interrotto dalla sospensione |
| `failed_to_start` | Non avviato: {reason} |

### T4. Altri testi fissi

| Chiave | Testo |
|---|---|
| `view.performance` | Prestazioni |
| `performance.nav.stress` | Stress test |
| `performance.nav.new` | Nuovo test |
| `performance.nav.running` | In corso ● |
| `performance.nav.history` | Cronologia |
| `performance.objective.normal` | Verifica normale |
| `performance.objective.normal.hint` | Voglio sapere se il PC regge carichi lunghi senza surriscaldarsi o sbagliare calcoli. |
| `performance.objective.overclock` | Stabilità overclock |
| `performance.objective.overclock.hint` | Ho cambiato frequenze, tensioni o Curve Optimizer e voglio trovare gli errori. |
| `performance.risk.title` | Prima di iniziare |
| `performance.risk.body` | Uno stress test porta la CPU al massimo: aumentano calore, consumi e rumore delle ventole. Un test di overclock può far bloccare o riavviare il PC: salva il lavoro aperto prima di avviarlo. |
| `performance.risk.dontShow` | Non mostrare più |
| `performance.warn.noService` | Servizio non attivo: senza la temperatura della CPU lo stop termico non funziona. |
| `performance.warn.wheaUnreadable` | Errori hardware non leggibili: il registro di sistema non è accessibile. |
| `performance.warn.tempMissing` | Temperatura non disponibile |
| `performance.advice.core` | Con Curve Optimizer, di solito si riduce l'offset di quel core. |
| `performance.quit.title` | Un test è in corso |
| `performance.quit.body` | Fermare il test e uscire? La sessione si salva come interrotta. |
| `performance.quit.confirm` | Ferma ed esci |
| `tray.performance.stop` | Ferma il test |
| `tray.performance.open` | Apri il test in corso |
| `tray.performance.tooltip` | Stress test in corso: {component} · {objective} |
| `performance.toast.title` | Stress test finito |
| `performance.toast.recovered` | Il test precedente si è interrotto: apri il risultato. |
| `performance.closeToTray` | Il test continua nella tray. |

---

### Task A1: protocollo `oma-ipc::load`, versione 1

**Files:**
- Create:
  - `crates/oma-ipc/src/load.rs`;
  - `crates/oma-ipc/tests/load_fixtures.rs`;
  - `protocol/fixtures/load/*.msgpack`, uno per messaggio.
- Modify:
  - `crates/oma-ipc/src/lib.rs` (`pub mod load;`);
  - `protocol/fixtures/README.md` (la cartella `load/`).

**Interfaces:**
- Consumes:
  - `encode_frame_of`, `decode_payload_of` e `FrameDecoder::next_of` di `crates/oma-ipc/src/frame.rs`;
  - `IpcError::Decode`;
  - i helper `check_len` di `overlay.rs`, spostati in `pub(crate)` se servono.
- Produces (serde `snake_case` per gli enumerati, campi `snake_case`, derive `Debug, Clone, PartialEq, Serialize, Deserialize`):
  - **Costanti:**
    - `LOAD_PROTOCOL_VERSION: u32 = 1`;
    - `LOAD_PIPE_PREFIX: &str = r"\\.\pipe\OpenMonitorAdvanced-Load-"`;
    - `MAX_PHASES = 512`, `MAX_PLAN_SECONDS: u32 = 86_400`, `MAX_LOGICAL = 1024`;
    - `MAX_TEXT_BYTES = 256`, `MAX_RAM_BYTES: u64 = 1 << 40`.
  - **`LoadMessage`** con `#[serde(tag = "type", content = "body", rename_all = "snake_case")]`:
    - `Hello(LoadHello)`, in tutti e due i versi;
    - `Run(RunRequest)` e `Stop(StopRequest)`, dall'app;
    - `Topology(Topology)`, `Progress(Progress)`, `Error(ComputeError)`, `Notice(Notice)`, `PhaseDone(PhaseDone)` e `Finished(Finished)`, dal processo.
  - **`LoadHello { protocol_version: u32, version: String, isa: Vec<Isa> }`:** l'app manda `isa` vuoto.
  - **`StopRequest {}`.**
  - **`Isa`:** `avx512`, `avx2`, `sse2`.
  - **`KernelId`:** `k1`, `k2`, `k3`, `k4`, `k5`, `k7`, `k8`, `k9`, `k10`.
  - **`DataSize`:** `l1`, `l2`, `l3`, `ram`, `auto`.
  - **`LoadMode`:** `steady`, `variable`, `light`.
  - **`Placement`:** `all_logical`, `one_per_core`, `core_cycle`.
  - **`RamPattern`:** `moving_inversions`, `modulo20`, `random`, `address`, `crc_copy`.
  - **`Topology`:**
    - `logical: Vec<LogicalCpu>`, `caches: CacheSizes`;
    - `hypervisor: bool`, `vendor: String`, `brand: String`.
  - **`LogicalCpu`:**
    - `index: u32`, globale e progressivo;
    - `group: u16`, `number: u8`;
    - `core: u32` (DA4), `core_index: u32`;
    - `efficiency_class: u8`, `llc: u32`, `parked: bool`;
    - `apic_id: Option<u32>`.
  - **`CacheSizes { l1d_bytes: u64, l2_bytes: u64, l2_shared_by: u32, l3_bytes: u64, l3_total_bytes: u64 }`.**
  - **`RunRequest { plan: Plan }`.**
  - **`Plan { seed: u64, ram_bytes: u64, phases: Vec<Phase> }`.**
  - **`Phase`:**
    - `kernel: KernelId`, `alt_kernel: Option<KernelId>`;
    - `isa: Isa`, `size: DataSize`, `mode: LoadMode`, `placement: Placement`;
    - `duration_s: u32`, `per_core_s: Option<u32>`;
    - `both_smt: bool`, `cores: Option<Vec<u32>>`;
    - `patterns: Vec<RamPattern>`;
    - `stop_on_error: bool`.
  - **`impl Plan { pub fn total_seconds(&self) -> u64 }`.**
  - **`Progress`:**
    - `phase: u32`, `phase_elapsed_ms: u64`, `elapsed_ms: u64`;
    - `checks: u64`, `errors: u64`;
    - `current_core: Option<u32>`;
    - `cores: Vec<CoreProgress { core: u32, state: CoreState }>`, con `CoreState` = `untested`, `testing`, `passed` o `failed`;
    - `memory_bytes: u64`;
    - `rate: Option<f64>`: le iterazioni al secondo di tutti i thread nell'ultimo secondo (§2.2). Il benchmark della M8a2 le userà; la M8a1 le mostra solo nel diario degli eventi.
  - **`ComputeError`:**
    - `phase: u32`, `kernel: KernelId`, `isa: Isa`;
    - `kind: ErrorKind`, con `mismatch`, `reference_disagreement`, `reference_invalid` o `hung`;
    - `logical: Option<u32>`, `core: Option<u32>`;
    - `iteration: u64`, `expected: u64`, `actual: u64`, `seed: u64`.
  - **`Notice { phase: u32, code: String, value: Option<u64> }`.**
  - **`PhaseDone { phase: u32, checks: u64, errors: u64, duration_ms: u64, skipped: Option<String> }`.**
  - **`Finished { reason: FinishReason, checks: u64, errors: u64 }`:** `FinishReason` vale `completed`, `stopped`, `first_error` o `failed`.
  - **`impl LoadMessage { pub fn validate(&self) -> Result<(), IpcError> }`.** Respinge:
    - stringhe oltre `MAX_TEXT_BYTES`;
    - più di `MAX_PHASES` fasi, oppure nessuna fase;
    - `duration_s` = 0, oppure un totale oltre `MAX_PLAN_SECONDS`;
    - `per_core_s` fuori da 1–3600, oppure assente quando `placement` è `core_cycle`;
    - `ram_bytes` oltre `MAX_RAM_BYTES`;
    - `Progress::rate` non finito o negativo;
    - più di `MAX_LOGICAL` voci in `logical`, `cores` o `Progress::cores`;
    - `patterns` non vuoto per un kernel diverso da `k10`, oppure vuoto per `k10`.
  - **`pub fn load_compatible(hello: &LoadHello) -> bool`.**

- [ ] **Step 1: portare la spec in `main` e creare il branch**

```bash
git switch main
git merge --ff-only docs/m8-spec
git switch -c feat/m8a1-stress-cpu
```

Il piano è già committato sul branch `docs/m8-spec`, quindi il merge lo porta in `main` con la spec.

- [ ] **Step 2: test che falliscono** (`load.rs`, modulo di test):
  - `every_message_round_trips`: un esempio per variante attraverso `encode_frame_of` e `FrameDecoder::next_of::<LoadMessage>()`;
  - `absent_values_are_nil_keys`: `serde_json::to_value` di un `Phase` con `alt_kernel: None` contiene la chiave `alt_kernel` con `null`;
  - `enums_are_snake_case_strings`: `KernelId::K10` è `"k10"`, `Placement::CoreCycle` è `"core_cycle"`;
  - `oversized_plan_is_rejected`: 513 fasi, un totale di 86 401 s, `per_core_s` 0 o 3601, 1025 core;
  - `core_cycle_needs_per_core_seconds`;
  - `k10_needs_patterns_and_others_refuse_them`;
  - `long_text_is_rejected`: un `Notice::code` o un `LoadHello::version` di 257 byte;
  - `non_finite_values_are_rejected`: `Progress::rate` `NaN`, `+inf` o negativo;
  - `unknown_fields_are_ignored`;
  - `hello_compatibility`: versione 1 sì, 2 no.

  In `tests/load_fixtures.rs`, `load_fixtures_match_the_encoder_byte_for_byte` segue lo schema di `tests/fixtures.rs`:
  - nomi `hello`, `run`, `stop`, `topology`, `progress`, `error`, `notice`, `phase_done`, `finished`;
  - cartella `protocol/fixtures/load/`;
  - scrittura con `OMA_WRITE_FIXTURES=1`.
- [ ] **Step 3:** `cargo test -p oma-ipc load`. Atteso: FAIL.
- [ ] **Step 4:** implementare, poi generare le fixture con `$env:OMA_WRITE_FIXTURES='1'; cargo test -p oma-ipc --test load_fixtures -- --test-threads=1`.
- [ ] **Step 5:** `cargo test -p oma-ipc` senza la variabile, `cargo clippy -p oma-ipc --all-targets -- -D warnings`. Atteso: PASS.
- [ ] **Step 6: commit** `feat(ipc): load protocol v1 between the app and oma-load`.

### Task A2: `oma-core::load`, catalogo, profili e piani

**Files:**
- Create:
  - `crates/oma-core/src/load/mod.rs`;
  - `crates/oma-core/src/load/catalog.rs`;
  - `crates/oma-core/src/load/plan.rs`;
  - `testdata/performance/catalog.json`.
- Modify:
  - `crates/oma-core/Cargo.toml` (`oma-ipc.workspace = true`);
  - `crates/oma-core/src/lib.rs`.

**Interfaces:**
- Consumes: i tipi di A1.
- Produces (serde `camelCase` per i tipi dell'app, enumerati `camelCase`):
  - **Enumerati:**
    - `Component`: `cpu`, `ram`;
    - `Objective`: `normal`, `overclock`;
    - `Preset`: `quick`, `standard`, `long`, `night`;
    - `ThreadChoice`: `allLogical`, `onePerCore`.
  - **`pub fn presets(component: Component, objective: Objective) -> &'static [(Preset, u32)]`:** le durate di DA11:
    - CPU normale: `quick` 300, `standard` 1800, `long` 3600;
    - CPU overclock: `standard` 3600, `long` 7200, `night` 28 800;
    - RAM normale: `quick` 900, `standard` 1800, `long` 3600;
    - RAM overclock: `standard` 3600, `long` 7200, `night` 28 800.
  - **`StartRequest`:**
    - `component`, `objective`, `preset`;
    - `custom: Option<Custom>`;
    - `retryCore: Option<RetryCore { core: u32, kernel: KernelId }>`.
  - **`Custom`:**
    - `modes: Vec<ModeEdit { kernel: KernelId, enabled: bool, minutes: Option<u32> }>`;
    - `isa: Option<Isa>`, `threads: ThreadChoice`;
    - `bothSmt: bool`: nelle fasi `core_cycle`, entrambi i thread del core (§4.3);
    - `stopOnFirstError: Option<bool>`.
  - **`BuildInput<'a>`:**
    - `request: &'a StartRequest`, `topology: &'a Topology`;
    - `isa: &'a [Isa]`, cioè i set disponibili;
    - `ram_budget: u64`;
    - `stop_override: Option<bool>`, da `performance.stopOnFirstError`;
    - `seed: u64`.
  - **`pub fn build_plan(input: &BuildInput) -> Result<Plan, BuildError>`:**
    - DA11 per la scaletta, DA12 per `custom`;
    - il miglior set disponibile per «automatico»;
    - il valore effettivo di «fermati al primo errore» è `custom` se c'è, poi `stop_override`, poi il profilo, e si scrive in `stop_on_error` di ogni fase;
    - con `retryCore`: le tre fasi (2) di DA11 sul solo core indicato, 120 s ciascuna, più 120 s di `retryCore.kernel` su quel core se non è fra quelle.
  - **`BuildError`:**
    - `NoCores`;
    - `NoPhases`, quando Personalizza le toglie tutte;
    - `TooLong`, oltre 24 h;
    - `UnknownCore(u32)`;
    - `RamBudget`, quando la quota è sotto i 256 MiB e il piano usa K3, K4 o K10.
  - **`pub fn ram_budget(available: u64, percent: u32) -> u64`:** DA10.
  - **`pub fn core_order(topology: &Topology) -> Vec<u32>`:**
    - i core in ordine di prova: `efficiency_class` decrescente, poi `core`;
    - salta i core con tutti i processori logici `parked`.
  - **`pub fn catalog_json() -> serde_json::Value`:** il contenuto di `catalog.json` di DA19, con le chiavi `kernels`, `isa`, `modes` (`steady`, `variable`, `light`, `coreCycle`, `allCore`), `patterns`, `components`, `objectives` e `presets`.

- [ ] **Step 1: test che falliscono** (`plan.rs` e `catalog.rs`). La fixture di topologia `topo(cores, smt, groups, hybrid)` è un helper del test.
  - `every_profile_and_preset_sums_to_its_duration`: per CPU e RAM, i due obiettivi e ogni preset, su 1, 8 (SMT) e 24 core (ibrida 8P+16E), `plan.total_seconds()` è uguale alla durata e ogni fase dura almeno 60 s;
  - `cpu_normal_standard_matches_the_table`: kernel, dimensioni e durate di DA11 nell'ordine;
  - `oc_standard_8_cores_with_avx512`:
    - t = 111 s (`per_core_s` 37 per ognuna delle tre fasi `core_cycle`, 296 s ciascuna);
    - le fasi (5) presenti;
    - il resto di 12 s aggiunto alla fase precedente;
    - totale 3600;
  - `oc_without_avx512_has_no_phase_5`;
  - `oc_with_many_cores_shrinks_per_core_time_to_60`: 64 core, Standard;
  - `core_cycle_orders_p_before_e_and_skips_parked`;
  - `plan_for_128_logical_in_two_groups`: il piano è valido per `LoadMessage::validate`;
  - `single_core_machine_builds_every_profile`;
  - `custom_disables_and_rescales_modes`;
  - `custom_isa_keeps_light_on_sse2`;
  - `custom_both_smt_applies_to_core_cycle_only`;
  - `custom_removing_everything_is_no_phases`;
  - `stop_override_and_custom_precedence`;
  - `retry_core_builds_only_that_core`;
  - `ram_budget_leaves_two_gib_free`: 8 GiB disponibili e 70% danno 5,6 GiB; 3 GiB disponibili danno 1 GiB; 1 GiB dà 0;
  - `ram_profiles_fail_below_256_mib`;
  - `catalog_fixture_matches`: confronta `catalog_json()` con `testdata/performance/catalog.json` e lo riscrive con `OMA_WRITE_FIXTURES=1`.
- [ ] **Step 2:** `cargo test -p oma-core load`. Atteso: FAIL.
- [ ] **Step 3:** implementare e generare la fixture.
- [ ] **Step 4:** `cargo test -p oma-core`, `cargo clippy -p oma-core --all-targets -- -D warnings`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): stress test catalogue, profiles and plan builder`.

### Task A3: `oma-core::load`, esiti, soglie, sensori, sessione e diario

**Files:**
- Create:
  - `crates/oma-core/src/load/outcome.rs`;
  - `crates/oma-core/src/load/thermal.rs`;
  - `crates/oma-core/src/load/sensors.rs`;
  - `crates/oma-core/src/load/session.rs`.
- Modify: `crates/oma-core/src/load/mod.rs`

**Interfaces:**
- Consumes:
  - `Schema`, `Snapshot` e `Quality` di `crates/oma-core/src/model.rs` e `provider.rs`;
  - i tipi di A1 e A2.
- Produces:
  - **`Outcome`** (serde `snake_case`): `passed`, `marginal`, `errors`, `crashed`, `hung`, `system_crash`, `stopped_user`, `stopped_thermal`, `suspended`, `failed_to_start`.
  - **`OutcomeFacts`:**
    - `failed_to_start: Option<String>`, `system_crash: bool`, `app_closed: bool`;
    - `crashed: bool`, `hung: bool`;
    - `errors: u64`, `error_cores: BTreeSet<u32>`;
    - `thermal_stop: Option<f64>`, `suspended: bool`, `user_stop: bool`;
    - `whea_corrected: u64`, `completed: bool`.
  - **`pub fn decide(facts: &OutcomeFacts) -> (Outcome, VerdictKey)`:** la precedenza di DA13. `VerdictKey { key: &'static str, params: BTreeMap<String, String> }` dà la chiave di T3:
    - `errors_core` con un solo core in `error_cores`;
    - `crashed_app` con `app_closed`.
  - **`pub fn cpu_stop_threshold(setting: Option<u32>, tjmax_c: Option<f64>) -> f64`:** DA5.
  - **`ThermalGuard`:**
    - `new(threshold_c: f64)`;
    - `observe(&mut self, temp_c: Option<f64>, mono_ms: u64) -> ThermalEvent`, con `ThermalEvent` = `None`, `Trip(f64)` o `Missing`;
    - `Trip` dopo 2 campioni consecutivi sopra la soglia;
    - `Missing` una volta sola, dopo più di 10 000 ms senza valore; si riarma quando il valore torna.
  - **`CpuSensorIds { temp: Option<usize>, power: Option<usize>, clock: Option<usize>, core_clock: Vec<Option<usize>>, tjmax_c: Option<f64> }`.**
  - **`pub fn resolve_cpu_sensors(schema: &Schema, cores: usize) -> CpuSensorIds`:** DA5.
  - **`SensorSample { temp_c, power_w, clock_mhz: Option<f64>, core_clock_mhz: Vec<Option<f64>> }`.**
  - **`pub fn read_sample(ids: &CpuSensorIds, snapshot: &Snapshot, quality: &[Quality]) -> SensorSample`:** solo `Fresh` o `Held`.
  - **Formati (serde `camelCase`, `format: 1`, senza `deny_unknown_fields`), come §8.1 e §8.3:**
    - **`Session`:**
      - `format`, `id`, `startedAt`, `endedAt: Option<String>`;
      - `component`, `device`, `objective`, `preset`;
      - `request: StartRequest`, `plan: Plan`;
      - `outcome: Option<Outcome>`, `outcomeDetail: Option<OutcomeDetail>`;
      - `phases: Vec<PhaseResult>`, `cores: Vec<CoreResult>`;
      - `errors: Vec<ErrorRecord>`, al massimo 200, più `errorsDropped: u64`;
      - `whea: WheaCounts`, `stats: Stats`;
      - `samples: Vec<Sample>`, `events: Vec<SessionEvent>`;
      - `appVersion`, `loadVersion: Option<String>`.
    - **`OutcomeDetail { verdict: String, params: BTreeMap<String, String>, phase: Option<u32>, kernel: Option<KernelId>, core: Option<u32>, tempC: Option<f64>, clockMhz: Option<f64>, atMs: Option<u64> }`.**
    - **`PhaseResult { index, kernel, outcome: String, durationMs, checks, errors, skipped: Option<String> }`.**
    - **`CoreResult { core: u32, state: CoreState, firstError: Option<ErrorRecord> }`.**
    - **`ErrorRecord`:** i campi di `ComputeError` più `atMs`, `tempC` e `clockMhz`.
    - **`WheaCounts { byId: BTreeMap<u32, u64>, byApic: BTreeMap<u32, u64>, unreadable: bool, lastRecord: Option<u64> }`.**
    - **`Stats { tempMaxC, tempAvgC, powerMaxW, powerAvgW, clockMaxMhz, clockAvgMhz: Option<f64> }`.**
    - **`Sample { tMs: u64, tempC, powerW, clockMhz: Option<f64> }`.**
    - **`SessionEvent { atMs: u64, code: String, params: BTreeMap<String, String> }`:** la UI lo traduce con `performance.event.<code>`.
    - **`Journal { format, sessionId, planSummary: String, phaseIndex: u32, kernel: Option<KernelId>, core: Option<u32>, updatedAt: String, cleanEnd: bool }`.**
  - **`pub fn parse_session(bytes: &[u8]) -> Result<Session, FormatError>` e `parse_journal`:**
    - rifiutano `format` > 1 (`FormatError::Future`) e i JSON rotti;
    - accettano campi sconosciuti.
  - **`pub fn summary(session: &Session) -> SessionSummary`:** `id`, `startedAt`, `component`, `objective`, `preset`, `durationMs`, `outcome`, `verdict`, `params`.
  - **`pub fn session_file_name(started_utc: &str, id: &str) -> String`:** `AAAAMMGG-HHMMSS-<id>.json`.
  - **`pub fn is_session_id(id: &str) -> bool`:** solo un uuid minuscolo.
  - **`pub fn is_session_file_name(name: &str) -> bool`:** solo quella forma.
  - **`pub fn prune(names: &[String], keep: usize) -> Vec<String>`:** i nomi da eliminare, i più vecchi, oltre `keep = 500`.

- [ ] **Step 1: test che falliscono:**
  - `precedence_follows_the_plan_table`: una tabella con tutte le coppie di DA13;
  - `single_error_core_is_unstable_core_n`, `several_cores_is_errors_found`;
  - `stopped_after_errors_is_errors`;
  - `whea_corrected_without_errors_is_marginal`;
  - `threshold_uses_setting_then_tjmax_then_95`: (None, Some(89)) dà 84; (Some(70), Some(89)) dà 70; (None, None) dà 95;
  - `two_consecutive_samples_trip`: 90, 84, 90, 90 con la soglia a 85 scatta al quarto campione, non al primo né al terzo;
  - `missing_for_ten_seconds_warns_once`;
  - `resolves_amd_and_intel_names`, sulla fixture `crates/oma-core/tests/fixtures/this-machine-schema.json` (`tctl`, `average-effective`, `core-1-effective`, `tjMaxC`) e su uno schema finto con `package`;
  - `suspended_quality_is_missing`;
  - `session_round_trips_and_ignores_unknown_fields`;
  - `future_format_is_rejected`, `truncated_json_is_an_error`;
  - `ids_outside_the_session_form_are_rejected`: `..\x`, `a/b`, un uuid maiuscolo e un nome senza `.json` sono rifiutati;
  - `prune_keeps_the_newest_500`.
- [ ] **Step 2:** `cargo test -p oma-core load`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): stress test outcomes, thermal guard, sensor pick and session files`.

### Task A4: `oma-core::load::run`, il controller della sessione

**Files:**
- Create: `crates/oma-core/src/load/run.rs`
- Modify: `crates/oma-core/src/load/mod.rs`

**Interfaces:**
- Consumes: A1, A3.
- Produces:
  - **`Clock { mono_ms: u64, wall_ms: i64, asleep_ms: u64 }`:**
    - `wall_ms` è il tempo Unix in ms, solo per mostrare l'ora;
    - `asleep_ms` viene da `power::asleep_ms()` (DA15).
  - **`WheaEvent { record_id: u64, event_id: u32, apic_id: Option<u32>, time_utc: String }`.**
  - **`RunConfig`:**
    - `threshold_c: f64`, `thermal_stop: bool`;
    - `service_available: bool`;
    - `cores: Vec<u32>`;
    - `apic_to_core: BTreeMap<u32, u32>`, riempito da `Topology`.
  - **`RunController`:**
    - `new(session: Session, config: RunConfig, now: Clock)`;
    - `on_load(&mut self, msg: &LoadMessage, now: Clock) -> Vec<Action>`;
    - `on_sample(&mut self, sample: &SensorSample, service_available: bool, now: Clock) -> Vec<Action>`;
    - `on_whea(&mut self, result: Result<Vec<WheaEvent>, ()>, now: Clock) -> Vec<Action>`;
    - `on_clock(&mut self, now: Clock) -> Vec<Action>`;
    - `on_user_stop(&mut self, now: Clock) -> Vec<Action>`;
    - `on_exit(&mut self, code: Option<i32>, now: Clock) -> Vec<Action>`;
    - `status(&self) -> RunStatus`;
    - `session(&self) -> &Session`;
    - `is_finished(&self) -> bool`.
  - **`Action`:**
    - `SendStop`, `Kill`;
    - `WriteJournal(Journal)`, `SaveSession`, `DeleteJournal`;
    - `PollWhea { after_record: Option<u64> }`;
    - `Toast`;
    - `Finished(Outcome)`.
  - **`RunStatus`** (serde `camelCase`; payload di `performance-status`):
    - `state`: `idle`, `starting`, `running`, `stopping` o `finished`;
    - `sessionId`, `component`, `objective`, `preset`;
    - `elapsedMs`, `totalMs`, `phaseIndex`, `phases: Vec<PhaseInfo { kernel, mode, placement, durationS, isa }>`;
    - `tempC`, `tempMaxC`, `stopC: Option<f64>`, `powerW`, `clockMhz`;
    - `checks`, `errors`;
    - `wheaCorrected`, `wheaFatal`;
    - `cores: Vec<CoreProgress>`, `currentCore: Option<u32>`;
    - `events`, gli ultimi 200;
    - `warnings: Vec<String>`, fra `noService`, `tempMissing`, `wheaUnreadable` e `ramReduced`;
    - `outcome: Option<Outcome>`.
  - **Regole:**
    - **Orologio:**
      - una crescita di `asleep_ms` di più di 1000 fra due `on_clock` chiude con `suspended` (DA15), con `SendStop` e poi `Kill`;
      - un salto di `wall_ms` da solo non cambia niente;
      - la pipe muta per più di 5000 ms di `mono_ms` dà `hung` e `Kill`.
    - **Scritture:**
      - `WriteJournal` a ogni cambio di `phase` o di `current_core` e ogni 30 000 ms;
      - `SaveSession` ogni 60 000 ms e alla fine;
      - `PollWhea` ogni 5000 ms e una volta dopo `Finished`.
    - **Campioni:**
      - un campione nella sessione ogni 5000 ms; le statistiche si aggiornano a ogni campione;
      - `ThermalGuard` a ogni `on_sample` se `thermal_stop`; `Trip` dà `SendStop` e l'esito `stopped_thermal` con la temperatura;
      - senza servizio l'avviso è `noService`, e lo stop termico non è attivo; se il servizio torna, lo stop si riattiva.
    - **WHEA:**
      - gli ID 17 e 19 contano come corretti, il 18 come fatale;
      - `Err(())` dà l'avviso `wheaUnreadable` una volta sola.
    - **Errori:**
      - un `Error` di tipo `hung` dà `hung`;
      - gli altri si contano, aggiornano `cores` e diventano `ErrorRecord` con l'ultimo campione: clock del core (DA5) e temperatura.
    - **Fine:**
      - dopo `SendStop` si aspetta `Finished` o l'uscita per al massimo 3000 ms, poi `Kill`;
      - `on_exit` senza un `Finished` prima dà `crashed`;
      - l'esito finale viene da `decide`, e alla fine si emettono `SaveSession`, `DeleteJournal`, `Toast` e `Finished(outcome)`.

- [ ] **Step 1: test che falliscono** (con un `Clock` finto e messaggi costruiti a mano):
  - `normal_run_completes_as_passed`;
  - `error_on_one_core_is_unstable_core_n_with_clock_and_temp`;
  - `thermal_trip_stops_and_records_the_temperature`;
  - `no_service_runs_with_a_warning_and_no_thermal_stop`;
  - `user_stop_saves_stopped_user`;
  - `sleep_ends_as_suspended_before_the_pipe_check`;
  - `wall_clock_change_is_not_a_suspend`;
  - `silent_pipe_is_hung_after_five_seconds`;
  - `exit_without_finished_is_crashed`;
  - `stop_without_answer_kills_after_three_seconds`;
  - `journal_every_phase_change_and_every_thirty_seconds`;
  - `session_saved_every_sixty_seconds`;
  - `whea_19_with_apic_maps_to_a_core_and_gives_marginal`;
  - `whea_unreadable_warns_once`;
  - `errors_beyond_200_are_counted`;
  - `samples_every_five_seconds`.
- [ ] **Step 2:** `cargo test -p oma-core load::run`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): stress session controller`.

### Task A5: `oma-win`, pipe privata generica, Job Object, topologia, memoria e sospensione

**Files:**
- Create:
  - `crates/oma-win/src/private_pipe.rs`;
  - `crates/oma-win/src/load_pipe.rs`;
  - `crates/oma-win/src/job.rs`;
  - `crates/oma-win/src/topology.rs`;
  - `crates/oma-win/src/power.rs`.
- Modify:
  - `crates/oma-win/src/overlay_pipe.rs`, che diventa una sottile coperta su `private_pipe`;
  - `crates/oma-win/src/memory.rs`;
  - `crates/oma-win/src/lib.rs`;
  - `crates/oma-win/Cargo.toml` (`Win32_System_JobObjects`).

**Interfaces:**
- Consumes:
  - `PipeConn`, `PipeEvent`, `PipeReader` e `CloseReason` di `pipe_io.rs`;
  - `user_only_sddl`, `current_user_sid`, `LocalMem` e `random_uuid_v4` di `overlay_pipe.rs`.
- Produces:
  - **`private_pipe`** (codice spostato da `overlay_pipe`, senza cambiarne il comportamento):
    - `PrivatePipeServer::create(prefix: &'static str, name: &str) -> io::Result<Self>`;
    - `accept(&self, timeout: Duration) -> io::Result<u32>`;
    - `into_connection<M>(self) -> PrivateConnection<M>`;
    - `connect_private_client<M>(prefix: &'static str, name: &str) -> io::Result<PrivateConnection<M>>`;
    - `PrivateConnection<M: Serialize + DeserializeOwned + Send + 'static>` con `send(&self, msg: &M) -> io::Result<()>` e `start_reader(&self, deliver: impl Fn(PipeEvent<M>) -> bool + Send + 'static) -> PipeReader`.

    L'API pubblica di `overlay_pipe` (`OverlayPipeServer`, `OverlayConnection`, `connect_overlay_client`, `random_pipe_name`, `random_uuid_v4`) e i suoi test restano uguali.
  - **`load_pipe`:**
    - `random_load_pipe_name() -> io::Result<String>`;
    - `LoadPipeServer`, cioè `PrivatePipeServer` con `LOAD_PIPE_PREFIX`;
    - `LoadConnection = PrivateConnection<LoadMessage>`;
    - `connect_load_client(name) -> io::Result<LoadConnection>`.
  - **`job::KillOnCloseJob`:**
    - `new() -> io::Result<Self>`;
    - `assign(&self, child: &std::process::Child) -> io::Result<()>`;
    - chiude l'handle con `Drop`, che uccide i processi assegnati.
  - **`topology::read() -> io::Result<Topology>`:**
    - `GetSystemCpuSetInformation` per gruppo, numero, `CoreIndex`, `EfficiencyClass`, `LastLevelCacheIndex` e `Parked`;
    - `GetLogicalProcessorInformationEx(RelationCache)` per le cache;
    - CPUID per vendor, brand e il bit hypervisor (foglia 1, ECX bit 31);
    - `apic_id` resta `None` (DA3).
  - **`topology::parse_cpu_sets(buf: &[u8]) -> Vec<LogicalCpu>` e `parse_caches(buf: &[u8]) -> CacheSizes`:** helper puri sul buffer restituito, con i test sui buffer costruiti a mano. «Core N» segue DA4.
  - **`memory::memory_status() -> io::Result<(u64, u64)>`:** totale e disponibile, da `GlobalMemoryStatusEx`.
  - **`power::KeepAwake`:**
    - `KeepAwake::new()` chiama `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`;
    - `Drop` chiama `SetThreadExecutionState(ES_CONTINUOUS)`;
    - `!Send`, perché vale per il thread che la crea.
  - **`power::boot_time_unix_ms() -> i64`:** ora attuale − `GetTickCount64`.
  - **`power::asleep_ms() -> u64`:** DA15, cioè `GetTickCount64()` − `QueryUnbiasedInterruptTime()`/10 000, con saturazione a 0.

- [ ] **Step 1: test che falliscono:**
  - `load_pipe_round_trip_and_pid`: crea un server, collega un client, scambia `Hello` e controlla che `accept` restituisca il proprio PID;
  - `second_load_server_with_same_name_fails`;
  - `load_name_requires_the_prefix`;
  - `killing_the_job_kills_the_child`:
    - avvia `cmd.exe /c ping -n 30 127.0.0.1 >nul` con `CREATE_NO_WINDOW`;
    - assegna il processo e chiude il Job;
    - entro 2 s `try_wait` è `Some`;
  - `parse_cpu_sets_two_groups_and_smt`: buffer finto con 2 gruppi, SMT e un core parcheggiato; «core N» in ordine di `CoreIndex`;
  - `parse_cpu_sets_hybrid_efficiency_classes`;
  - `parse_caches_l1_l2_l3_and_missing_l3`;
  - `real_topology_has_cores` (`#[ignore = "requires real Windows hardware"]`);
  - `asleep_ms_does_not_grow_while_awake`: due letture a 100 ms di distanza differiscono di meno di 50 ms;
  - i test esistenti di `overlay_pipe` restano verdi.
- [ ] **Step 2:** `cargo test -p oma-win private_pipe load_pipe job topology`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:**
  - `cargo test -p oma-win`;
  - `cargo test -p oma-win -- --include-ignored real_topology_has_cores`;
  - `cargo clippy --workspace --all-targets -- -D warnings`.

  Atteso: PASS.
- [ ] **Step 5: commit** `feat(win): generic private pipe, kill-on-close job, CPU topology and keep-awake`.

### Task A6: `oma-win::eventlog`, WHEA, BugCheck e Kernel-Power

**Files:**
- Create: `crates/oma-win/src/eventlog.rs`
- Modify:
  - `crates/oma-win/src/lib.rs`;
  - `crates/oma-win/Cargo.toml` (`Win32_System_EventLog`).

**Interfaces:**
- Consumes: `WheaEvent` di A4.
- Produces:
  - **`SystemEvent`:**
    - `record_id: u64`;
    - `provider: EventProvider`, con `Whea`, `BugCheck` o `KernelPower`;
    - `event_id: u32`, `time_utc: String`;
    - `apic_id: Option<u32>`.

    `impl SystemEvent { pub fn whea(&self) -> Option<WheaEvent> }`.
  - **`pub fn parse_event_xml(xml: &str) -> Option<SystemEvent>`** (puro). Legge:
    - `Provider/@Name`;
    - `EventID`, `EventRecordID`, `TimeCreated/@SystemTime`;
    - per il WHEA 19, `EventData/Data[@Name='ApicId']`.

    Accetta virgolette semplici e doppie; un valore assente dà `None` nel campo, un XML che non è un evento dà `None`.
  - **`pub fn latest_record_id() -> io::Result<Option<u64>>`:** l'ultimo record WHEA del canale `System`.
  - **`pub fn whea_after(record: Option<u64>) -> io::Result<Vec<SystemEvent>>`:**
    - `EvtQuery(None, "System", xpath, EvtQueryChannelPath)`;
    - XPath `*[System[Provider[@Name='Microsoft-Windows-WHEA-Logger'] and EventRecordID > N]]`;
    - `EvtNext` a blocchi di 32 e `EvtRender(EvtRenderEventXml)`;
    - al massimo 1000 eventi.
  - **`pub fn crash_evidence(since_utc: &str) -> io::Result<Vec<SystemEvent>>`:**
    - WHEA 17, 18 e 19 di `Microsoft-Windows-WHEA-Logger`;
    - 1001 di `Microsoft-Windows-WER-SystemErrorReporting`, cioè BugCheck;
    - 41 di `Microsoft-Windows-Kernel-Power`;
    - con `TimeCreated[@SystemTime>='<since>']`, al massimo 100 eventi.
  - **Accesso negato:** `ERROR_ACCESS_DENIED` e un canale mancante danno `Err`, e il controller lo segna `unreadable`.

- [ ] **Step 1: test che falliscono** (XML di esempio nel modulo di test, scritti secondo lo schema degli eventi di Windows):
  - `parses_whea_19_with_apic_id`;
  - `parses_whea_18_without_apic`;
  - `parses_bugcheck_1001_and_kernel_power_41`;
  - `single_and_double_quotes`;
  - `garbage_is_none`;
  - `real_system_log_is_readable` (`#[ignore = "requires real Windows hardware"]`): `latest_record_id()` restituisce `Ok`.
- [ ] **Step 2:** `cargo test -p oma-win eventlog`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-win`, poi `cargo test -p oma-win -- --include-ignored real_system_log_is_readable`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(win): read WHEA, bugcheck and kernel-power events from the System log`.

### Task A7: `crates/oma-load`, binario, collegamento, affinità e topologia completa

**Files:**
- Create:
  - `crates/oma-load/Cargo.toml`, `crates/oma-load/build.rs`;
  - `crates/oma-load/src/main.rs`, `crates/oma-load/src/lib.rs`;
  - `crates/oma-load/src/args.rs`, `crates/oma-load/src/log.rs`, `crates/oma-load/src/link.rs`;
  - `crates/oma-load/src/sys/mod.rs`, `crates/oma-load/src/sys/affinity.rs`, `crates/oma-load/src/sys/cpuid.rs`.
- Modify:
  - `Cargo.toml` della radice (membro `crates/oma-load`);
  - `about.toml` (`[oma-load] accepted = ["GPL-3.0-or-later"]`).

**Interfaces:**
- Consumes: A1; A2 (`core_order`); A5 (`connect_load_client`, `topology::read`, `power::asleep_ms`); lo schema di `crates/oma-overlay` per `args`, `log`, `link` e `build.rs`. Dipendenze: `oma-core`, `oma-ipc`, `tracing`, `tracing-subscriber`, `tracing-appender`; `oma-win` solo con `cfg(windows)`.
- Produces:
  - **Manifesto:**
    - pacchetto `oma-load`, con versione, edizione, `rust-version` e licenza del workspace;
    - `description = "OpenMonitor Advanced load generator"`;
    - libreria `oma_load` (`src/lib.rs`, portabile) e binario `oma-load` (`src/main.rs`);
    - `build.rs` con la risorsa di versione di `oma-overlay` (`FileDescription` «OpenMonitor Advanced load generator», `OriginalFilename` `oma-load.exe`).
  - **`#![windows_subsystem = "windows"]` in `main.rs`.** Codici d'uscita in `link.rs`: `EXIT_OK = 0`, `EXIT_USAGE = 1`, `EXIT_CONNECT = 2`, `EXIT_INCOMPATIBLE = 3`.
  - **`args::parse_args(&[String]) -> Result<Args, ArgsError>`:**
    - `Args { pipe: String, inject: Option<Inject { kernel: KernelId, core: Option<u32> }> }`;
    - `--inject-fault` esiste solo con `cfg(debug_assertions)` (DA18).
  - **`log::init()`:** file giornaliero `oma-load` nella cartella dei log, sette file, come `oma-overlay`.
  - **`link`:** collegamento con `Hello`, poi `Topology`, poi l'attesa di `Run`.
    - Al `Hello`: `isa` dai set rilevati, `version` = `CARGO_PKG_VERSION`.
    - `Run` validato con `validate()`; uno invalido chiude con `EXIT_USAGE` e una riga nel log.
    - `Stop` durante la corsa ferma il motore (A8).
    - La pipe chiusa ferma tutto ed esce con `EXIT_OK`.
  - **`sys::affinity`:**
    - `pin_current_thread(cpu: &LogicalCpu) -> io::Result<()>` con `SetThreadGroupAffinity`;
    - `prepare_worker_thread()`: `THREAD_PRIORITY_BELOW_NORMAL` ed EcoQoS spento (`SetThreadInformation(ThreadPowerThrottling, ControlMask = THREAD_POWER_THROTTLING_EXECUTION_SPEED, StateMask = 0)`).
  - **`sys::cpuid::apic_id() -> u32`:** foglia 0x0B, EDX; ripiego sulla foglia 1, EBX[31:24].
  - **`pub fn full_topology() -> io::Result<Topology>`:** `topology::read()` più l'APIC ID di ogni processore logico, letto da un thread temporaneo fissato a turno.
  - **Feature `windows` di `oma-load`:** `Win32_Foundation`, `Win32_System_Threading`, `Win32_System_SystemInformation`, `Win32_System_Kernel`, `Win32_System_Memory`. Se ne serve un'altra, l'implementer la motiva nel report.

- [ ] **Step 1: test che falliscono:**
  - `args_require_a_load_pipe_name`;
  - `inject_fault_parses_kernel_and_core`;
  - `unknown_argument_is_usage`;
  - `cpuid_leaf_parsing`: decodifica pura di EBX ed EDX dati;
  - `pinning_reports_distinct_apic_ids` (`#[ignore = "requires real Windows hardware"]`): su due processori logici gli APIC ID sono diversi.
- [ ] **Step 2:** `cargo test -p oma-load`. Atteso: FAIL.
- [ ] **Step 3:** implementare; per ora `Run` risponde con `Finished { reason: failed }`, perché il motore arriva in A8.
- [ ] **Step 4:** `cargo build -p oma-load`, `cargo test -p oma-load`, clippy, `pwsh scripts/generate-licenses.ps1 -Check`.

  Atteso: PASS. Se il controllo delle licenze fallisce, `THIRD_PARTY_LICENSES.txt` va rigenerato e committato.
- [ ] **Step 5: commit** `feat(load): oma-load process with pipe link, affinity and APIC topology`.

### Task A8: `oma-load`, motore delle fasi e verifica (adattamento di OpenDCDiag)

**Files:**
- Create:
  - `crates/oma-load/src/rng.rs`;
  - `crates/oma-load/src/verify.rs`;
  - `crates/oma-load/src/kernel.rs`;
  - `crates/oma-load/src/engine/mod.rs`;
  - `crates/oma-load/src/engine/schedule.rs`;
  - `crates/oma-load/src/engine/modes.rs`;
  - `crates/oma-load/src/engine/sentinel.rs`.
- Modify: `crates/oma-load/src/lib.rs`, `crates/oma-load/src/link.rs`

**Interfaces:**
- Consumes: A1, A7.
- Produces:
  - **`rng`:** `SplitMix64::new(seed)`, `Xoshiro256ss::new(seed)` con `next_u64` e `fill_u64(&mut [u64])`, `phase_seed(plan_seed, phase) -> u64`.
  - **`verify`:**
    - `digest_words(&[u64]) -> u64`, con `h = (h ^ w).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(31)` a partire da `0xCBF2_9CE4_8422_2325`;
    - `digest_f64(&[f64])`, sui bit;
    - `reference_on(cpus: &[LogicalCpu], f: &(dyn Fn() -> Result<u64, String> + Sync)) -> Result<u64, RefError>`: esegue `f` su ogni CPU fissata (al massimo tre) e restituisce il valore comune. `RefError` vale `Disagree(Vec<u64>)` o `Invalid(String)`.
  - **`kernel`:**
    - `trait Kernel: Send { fn iterate(&mut self, beat: &AtomicU64) -> Check; }`, dove `Check` vale `Digest(u64)`, `Ok` o `Mismatch { expected, actual }`;
    - `trait KernelFactory: Sync { fn reference(&self, ctx: &WorkerCtx) -> Option<Result<u64, String>>; fn worker(&self, ctx: &WorkerCtx) -> Result<Box<dyn Kernel>, KernelError>; }`;
    - `WorkerCtx { isa, size, budget: ThreadBudget, seed, worker: u32, workers: u32, patterns: Vec<RamPattern>, shared: Arc<PhaseShared> }`;
    - `ThreadBudget { l1d, l2_thread, l3_share, ram_per_thread: u64 }`, calcolato da DA9;
    - `KernelError` vale `Unsupported`, `Memory(u64)` o `Insufficient`;
    - `pub fn factory(id: KernelId) -> Option<&'static dyn KernelFactory>`: restituisce `None` finché il kernel non c'è (A9–A15).
  - **`engine::run(plan: &Plan, topology: &Topology, out: &(dyn Fn(LoadMessage) + Sync), stop: &AtomicBool, inject: Option<Inject>) -> Finished`:**
    - per fase: riferimento (DA7) e lavoratori secondo `placement`;
    - `core_cycle` segue `core_order` di A2; un core che sbaglia si segna e il ciclo passa al successivo; i core già sbagliati si saltano;
    - `stop_on_error` ferma al primo errore con `FinishReason::FirstError`;
    - `Progress` a 1 Hz dal thread del motore;
    - `Notice` per le riduzioni di DA10;
    - `PhaseDone` alla fine di ogni fase, con `skipped: Some("unsupported")` per un kernel senza `factory`;
    - la fault injection di DA18.
  - **`engine::modes`:**
    - `steady`;
    - `variable`: busy e pausa casuali 10–500 ms dal seme, che alternano `kernel` e `alt_kernel` a ogni burst;
    - `light`: 1 thread, busy 200–2000 ms e pausa 50–500 ms;
    - la pausa è `thread::sleep` fino a 1 ms dalla scadenza, poi attesa attiva, e aggiorna il battito.
  - **`engine::sentinel`:**
    - un thread controlla i battiti ogni secondo; un lavoratore fermo da 10 s manda `Error { kind: hung }`, poi `Finished { reason: failed }`, poi `std::process::exit(EXIT_OK)`;
    - una crescita di `power::asleep_ms()` fra due giri azzera la base dei battiti (DA15).
  - **Nota d'origine in testa a `verify.rs` e `kernel.rs`**, con le righe di copyright controllate su `framework/sandstone.h` e `tests/examples/vector_add.c` al commit `9957c45b899e2ff7deb7bad94229246e2281d667` (WebFetch dei file raw a quel commit):

```text
// Adapted from OpenDCDiag (https://github.com/opendcdiag/opendcdiag),
// commit 9957c45b899e2ff7deb7bad94229246e2281d667: framework/sandstone.h and
// tests/examples/vector_add.c (golden value in init, recompute and compare
// in the loop, reproducible seed).
// Original work: Copyright 2022 Intel Corporation, licensed under the
// Apache License, Version 2.0 (see THIRD_PARTY_LICENSES.txt).
// Modified for OpenMonitor Advanced: rewritten in Rust, reference computed
// on three cores that must agree, 64-bit digests instead of memcmp.
// The modifications are part of OpenMonitor Advanced, GPL-3.0-or-later.
```

- [ ] **Step 1: test che falliscono.** I kernel finti del test sono un `CountKernel` deterministico e un `StallKernel` che si blocca.
  - `rng_is_deterministic_and_seeds_differ_per_phase`;
  - `digest_detects_a_single_bit`;
  - `reference_agrees_or_reports_disagreement`;
  - `reference_with_one_core_uses_it_alone`;
  - `budget_matches_the_plan_table`: L2 da 1 MiB condivisa da 2 thread dà `l2_thread` 512 KiB;
  - `core_cycle_marks_failed_core_and_moves_on`;
  - `stop_on_error_finishes_with_first_error`;
  - `stop_flag_finishes_with_stopped_within_one_second`;
  - `variable_mode_alternates_kernels`;
  - `injected_fault_hits_the_chosen_core`;
  - `sentinel_reports_a_stalled_worker`: soglia ridotta per il test con una costante `#[cfg(test)]`;
  - `sentinel_ignores_a_suspend_gap`;
  - `missing_factory_skips_the_phase`.

  Ogni test usa al massimo 2 thread e meno di 3 s.
- [ ] **Step 2:** `cargo test -p oma-load`. Atteso: FAIL.
- [ ] **Step 3:** implementare e collegare `link` al motore: `Run` avvia `engine::run` su un thread; `Stop` alza `stop`.
- [ ] **Step 4:** `cargo test -p oma-load`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): phase engine, load modes, sentinel and OpenDCDiag-style verification`.

### Regole comuni ai kernel (A9–A15)

Valgono per ogni task di kernel e il revisore le controlla tutte:
- un modulo per kernel in `crates/oma-load/src/kernels/`, registrato in `kernel::factory`;
- un percorso per set d'istruzioni con `#[target_feature(enable = "...")]`, scelto da `ctx.isa`; un set richiesto ma assente dà `KernelError::Unsupported`;
- dimensioni da `ctx.budget` secondo DA9;
- dati da `Xoshiro256ss::new(ctx.seed)`, limitati e mai nulli, denormali o infiniti;
- `reference()` usa lo stesso codice di `worker()` e lo esegue una volta;
- `iterate` aggiorna `beat` almeno ogni 250 ms di lavoro;
- i test usano dimensioni piccole, cioè un `ThreadBudget` di prova da 32 KiB / 256 KiB / 1 MiB, un solo thread e meno di 3 s;
- per ogni set disponibile sulla macchina, il test controlla:
  - lo stesso digest con lo stesso seme, e uno diverso con un seme diverso;
  - un `Mismatch` (o un digest diverso dal riferimento) dopo aver capovolto un bit dei dati a metà iterazione, attraverso un hook `#[cfg(test)]`;
  - la proprietà matematica del kernel, elencata nel task.

### Task A9: K1, carico massimo FMA (adattamento di FIRESTARTER)

**Files:**
- Create:
  - `crates/oma-load/src/kernels/mod.rs`;
  - `crates/oma-load/src/kernels/k1/mod.rs`;
  - `crates/oma-load/src/kernels/k1/groups.rs`;
  - `crates/oma-load/src/kernels/k1/crc.rs`.
- Modify:
  - `crates/oma-load/build.rs`, che genera `$OUT_DIR/k1_payload.rs`;
  - `crates/oma-load/src/kernel.rs` (registrazione).

**Interfaces:**
- Consumes: A8 (`Kernel`, `KernelFactory`, `WorkerCtx`, `verify`).
- Produces:
  - **`groups.rs`:** le stringhe di DA8 come costanti (`AVX512_GROUPS`, `AVX2_GROUPS`) e `LINES: usize = 1536`. Le legge il `build.rs` con `include!`.
  - **`build.rs`:**
    - interpreta le stringhe `ITEM:VAL` (VAL ≥ 1);
    - ripete la sequenza pesata fino a 1536 righe, nell'ordine di FIRESTARTER: ogni giro emette le voci nell'ordine della stringa, VAL volte ciascuna;
    - scrive tre funzioni `unsafe fn k1_block_avx512(st: &mut K1State, l1: *mut f64, l2: *mut f64)`, `k1_block_avx2(...)` e `k1_block_sse2(...)`, con `#[target_feature]`;
    - ogni riga è la traduzione in intrinseci dell'operazione di FIRESTARTER:
      - `REG`: due `fmadd` sugli accumulatori;
      - `L1_L`: `fmadd` con operando caricato da L1;
      - `L1_LS` e `L2_LS`: store e poi `fmadd` con load;
      - `L2_L` e `L2_S`: load e store nella zona L2.

      Gli indirizzi avanzano a passi di una riga di cache e tornano all'inizio della zona.
  - **`K1State`:** 10 accumulatori vettoriali, i tre operandi `a`, `b` e `c`, e il contatore.
    - Valori iniziali da FIRESTARTER `initMemory`: `0.25 + i × 8 × 1e-7`.
    - Gli accumulatori ripartono dallo stato iniziale a ogni blocco (DA6).
    - Un blocco esegue `k1_block_*` 16 384 volte; con SSE2, 4096 volte.
  - **`crc.rs`:** `crc32c_u64(acc: u32, x: u64) -> u32` con `_mm_crc32_u64` (SSE4.2), e un ripiego software a tabella se SSE4.2 manca. È l'hash dei registri di FIRESTARTER (`emitErrorDetectionCode`).
  - **`iterate`:** un blocco, poi `Check::Digest` dell'hash CRC32C di tutte le corsie di tutti gli accumulatori, esteso a 64 bit con la riga digest del blocco L1.
  - **Nota d'origine** in testa a `k1/mod.rs`, `groups.rs`, `crc.rs` e `build.rs`:
    - il blocco di licenza GPL di FIRESTARTER copiato senza modifiche, con l'anno e il titolare dei file a monte, cioè `FMAPayload.cpp`, `AVX512Payload.cpp`, `X86Payload.hpp`, `HaswellConfig.hpp` e `SkylakeSPConfig.hpp` al commit `927ae17e55f3f90f7575f6a68630a366fde9c94e`, letti con WebFetch dei file raw a quel commit;
    - una riga `Adapted from FIRESTARTER (https://github.com/tud-zih-energy/FIRESTARTER), commit 927ae17e55f3f90f7575f6a68630a366fde9c94e, <file>: <cosa>`;
    - `Modified for OpenMonitor Advanced: Rust intrinsics generated at build time instead of asmjit at run time; accumulators reset every block; L3/RAM items dropped.`

- [ ] **Step 1: test che falliscono:**
  - `groups_parse_and_fill_1536_lines`: test del parser usato dal `build.rs`, con le stringhe in un modulo condiviso `include!`;
  - `zero_value_group_is_rejected`;
  - `k1_digest_is_deterministic_per_isa`;
  - `k1_blocks_repeat_the_same_hash`: due blocchi consecutivi danno lo stesso digest;
  - `k1_accumulators_stay_finite`: dopo un blocco tutte le corsie sono finite e diverse da zero;
  - `k1_bit_flip_changes_the_digest`;
  - `crc32c_matches_the_software_table`: lo stesso valore da SSE4.2 e dalla tabella, e `crc32c("123456789") == 0xE306_9283`.
- [ ] **Step 2:** `cargo test -p oma-load k1`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, clippy, `cargo build -p oma-load --release`. Atteso: PASS. Il binario release si compila in meno di 3 minuti; se no, si riduce la ripetizione, non `LINES`.
- [ ] **Step 5: commit** `feat(load): K1 FMA power kernel adapted from FIRESTARTER`.

### Task A10: K2, K3 e K4, FFT in doppia precisione

**Files:**
- Create: `crates/oma-load/src/kernels/fft.rs`, `crates/oma-load/src/kernels/k2.rs`, `crates/oma-load/src/kernels/k4.rs`.
- Modify: `crates/oma-load/src/kernels/mod.rs`, `crates/oma-load/src/kernel.rs`.

**Interfaces:**
- Consumes: A8.
- Produces:
  - **`fft::Fft`:**
    - `new(n: usize, isa: Isa) -> Self`: radix-2 iterativa, potenza di 2, con i twiddle precalcolati;
    - `forward(&self, re: &mut [f64], im: &mut [f64])` e `inverse(...)`, in place, con normalizzazione 1/N nell'inversa;
    - farfalle in AVX-512, AVX2+FMA o SSE2 secondo `isa`.
  - **K2:**
    - `size` `l1` o `l2` (DA9);
    - `iterate`: copia l'ingresso nel buffer di lavoro, poi `forward`, digest, `inverse`, digest;
    - `Check::Digest` dei due digest combinati.
  - **K3:** la stessa factory di K2 con `size = ram` e la memoria per thread di DA9 e DA10, allocata con `Vec::try_reserve_exact`. Un errore dà `KernelError::Memory`, che il motore trasforma nella riduzione di DA10.
  - **K4:** un lavoratore che tiene la dimensione corrente, la cambia ogni 20 s e confronta con il riferimento di quella dimensione. I riferimenti si calcolano tutti all'inizio, una dimensione per volta.
  - **Riferimento (DA7):** prima del digest, `reference()` controlla che:
    - X₀ sia uguale a Σxₙ entro `1e-9 × N × max|x|`;
    - l'andata e ritorno sia uguale all'ingresso entro `1e-9 × max|x|`.

    Fuori tolleranza dà `Err("reference_invalid")`.

- [ ] **Step 1: test che falliscono:**
  - `fft_matches_a_naive_dft_for_n_16_and_64`;
  - `fft_round_trip_within_tolerance_for_each_isa`;
  - `fft_dc_sum_check`;
  - `k2_digest_is_deterministic_per_isa`;
  - `k2_bit_flip_changes_the_digest`;
  - `k3_allocation_failure_is_memory_error`, con una richiesta di `u64::MAX / 2`;
  - `k4_cycles_sizes_from_l1_up`.
- [ ] **Step 2:** `cargo test -p oma-load fft k2 k4`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): K2, K3 and K4 double-precision FFT kernels`.

### Task A11: K5, NTT modulare a 64 bit

**Files:**
- Create: `crates/oma-load/src/kernels/k5.rs`
- Modify: `crates/oma-load/src/kernels/mod.rs`, `crates/oma-load/src/kernel.rs`

**Interfaces:**
- Consumes: A8.
- Produces:
  - **Costanti:**
    - `P: u64 = 9_223_372_006_790_004_737` (`0x7FFF_FFF9_0000_0001` = 2147483641 × 2³² + 1, primo, minore di 2⁶³);
    - generatore `G = 3`;
    - radici di ordine 2^k per N ≤ 2³²;
    - `M61: u64 = (1 << 61) - 1`.
  - **`mul_mod(a, b) -> u64`:** prodotto a 128 bit (`u128`, che genera `mulx` con BMI2) e riduzione.
  - **`ntt_forward(&mut [u64])` e `ntt_inverse(&mut [u64])`:** in place, radix-2.
  - **`iterate`:**
    - copia l'ingresso, poi forward;
    - controlla che Σ Xₖ ≡ N·x₀ (mod P), altrimenti `Mismatch`;
    - inversa, e il risultato deve essere identico all'ingresso, altrimenti `Mismatch` con l'indice;
    - `Check::Digest` = Σ Xₖ·(k+1) mod `M61`.
  - **Dimensioni:** `l2`, `l3` e `ram` di DA9; `isa` non conta.

- [ ] **Step 1: test che falliscono:**
  - `p_is_prime_and_g_generates`: Miller–Rabin deterministico con le basi 2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, poi `pow(G, (P-1)/q) != 1` per q ∈ {2, 2699, 795659};
  - `ntt_matches_naive_for_n_8`;
  - `ntt_round_trip_is_exact`;
  - `sum_identity_holds`;
  - `k5_digest_is_deterministic`;
  - `k5_bit_flip_is_a_mismatch`.
- [ ] **Step 2:** `cargo test -p oma-load k5`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): K5 exact 64-bit NTT kernel`.

### Task A12: K7, GEMM con il controllo di Freivalds

**Files:**
- Create: `crates/oma-load/src/kernels/k7.rs`
- Modify: `crates/oma-load/src/kernels/mod.rs`, `crates/oma-load/src/kernel.rs`

**Interfaces:**
- Consumes: A8.
- Produces:
  - **Matrici:** A e B n×n in f64 con interi in [−8, 8] dal seme, e C = A·B con blocchi da 64.
  - **Micro-kernel:** FMA in AVX-512, AVX2+FMA, oppure `mulpd`+`addpd` in SSE2. Con questi interi ogni prodotto e ogni somma è esatto in f64 fino a n = 2048.
  - **`iterate`:**
    - calcola C;
    - controlla con Freivalds, su un vettore r di 0 e 1 dal seme dell'iterazione, che A·(B·r) sia uguale a C·r in modo esatto, altrimenti `Mismatch`;
    - `Check::Digest` di C.

    Il digest è lo stesso per ogni set d'istruzioni, ma il riferimento resta per set (DA7).
  - **Dimensioni:** DA9.

- [ ] **Step 1: test che falliscono:**
  - `gemm_matches_naive_for_n_16`;
  - `freivalds_accepts_a_correct_product`;
  - `freivalds_catches_a_single_wrong_element`;
  - `k7_digest_is_identical_across_isas`;
  - `k7_bit_flip_is_a_mismatch`.
- [ ] **Step 2:** `cargo test -p oma-load k7`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): K7 blocked GEMM kernel with Freivalds check`.

### Task A13: K8, crittografia, codici di controllo, compressione e ordinamento

**Files:**
- Create:
  - `crates/oma-load/src/kernels/k8/mod.rs`;
  - `crates/oma-load/src/kernels/k8/aes.rs`, `sha256.rs`, `clmul.rs`, `lz.rs`, `sort.rs`.
- Modify: `crates/oma-load/src/kernels/mod.rs`, `crates/oma-load/src/kernel.rs`

**Interfaces:**
- Consumes: A8; `crc32c_u64` di A9.
- Produces:
  - **`aes`:**
    - AES-128 con `_mm_aesenc_si128`, `_mm_aesenclast_si128` e `_mm_aeskeygenassist_si128`;
    - cifratura a catena di 64 KiB;
    - se `aes` manca, il passo si salta e `iterate` non lo conta.
  - **`sha256`:**
    - SHA-256 con SHA-NI (`_mm_sha256rnds2_epu32`, `_mm_sha256msg1_epu32`, `_mm_sha256msg2_epu32`) se `sha` c'è;
    - altrimenti un'implementazione scalare;
    - su 64 KiB.
  - **`clmul`:** moltiplicazione senza riporto con `_mm_clmulepi64_si128` su coppie del buffer, confrontata a campione con una moltiplicazione software.
  - **`lz`:** un compressore LZ77 proprio, deterministico.
    - Formato: coppie di letterali e riferimenti (offset 16 bit, lunghezza 8 bit), con una tabella hash di 4096 voci.
    - Comprime 256 KiB di dati comprimibili generati dal seme (parole da un dizionario di 256 parole) e li decomprime.
    - Il risultato deve essere identico, altrimenti `Mismatch`.
  - **`sort`:** quicksort proprio con mediana di tre, che ripiega su insertion sort sotto 16 elementi, su 65 536 `u32` dal seme. L'esito deve essere ordinato e con la stessa somma, altrimenti `Mismatch`.
  - **`iterate`:** i cinque passi più CRC32C dei 64 KiB, e `Check::Digest` che combina i risultati.
  - **Vettori noti, controllati in `reference()` e nei test:**
    - AES-128 FIPS-197 C.1: chiave `000102…0f`, testo `00112233445566778899aabbccddeeff`, cifrato `69c4e0d86a7b0430d8cdb78070b4c55a`;
    - SHA-256(`"abc"`) = `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad`;
    - CRC32C(`"123456789"`) = `0xE3069283`.

- [ ] **Step 1: test che falliscono:**
  - `aes_fips197_vector`;
  - `sha256_abc_vector_with_and_without_sha_ni`;
  - `clmul_matches_software`;
  - `lz_round_trip_and_compresses`: il risultato è più corto del 50%;
  - `lz_rejects_a_corrupted_stream_without_panic`;
  - `sort_sorts_and_keeps_the_sum`;
  - `k8_digest_is_deterministic`;
  - `k8_bit_flip_is_a_mismatch`.
- [ ] **Step 2:** `cargo test -p oma-load k8`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): K8 crypto, checksum, compression and sort kernel`.

### Task A14: K9, scambio fra core

**Files:**
- Create: `crates/oma-load/src/kernels/k9.rs`
- Modify: `crates/oma-load/src/kernels/mod.rs`, `crates/oma-load/src/kernel.rs`, `crates/oma-load/src/engine/schedule.rs`

**Interfaces:**
- Consumes: A8 (`PhaseShared`).
- Produces:
  - **Coppie:** il lavoratore i si accoppia con i + workers/2. Così, con l'ordine di DA4, una coppia attraversa i CCD quando ce n'è più d'uno.
  - **Anello per coppia:** 64 righe da 64 byte, ciascuna con:
    - 6 parole di dati;
    - un numero di sequenza;
    - un checksum `digest_words` delle 7 parole.
  - **Scambio:**
    - un lato scrive con `Release`, l'altro legge con `Acquire`, controlla sequenza e checksum, e i ruoli si scambiano a ogni giro di 64 righe;
    - una sequenza fuori ordine o un checksum sbagliato dà `Mismatch`;
    - `iterate` restituisce `Check::Ok` a ogni giro.
  - **Attesa:** si aspetta con `spin_loop` e si aggiorna il battito; uno stop arrivato durante l'attesa esce subito.
  - **Meno di 2 lavoratori:** la fase si salta con `Notice { code: "k9_needs_two_cores" }`.

- [ ] **Step 1: test che falliscono:**
  - `k9_two_threads_exchange_without_errors`: 2 thread per 1 s;
  - `k9_corrupted_line_is_a_mismatch`;
  - `k9_single_worker_is_skipped`;
  - `k9_stop_unblocks_a_waiting_side`.
- [ ] **Step 2:** `cargo test -p oma-load k9`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): K9 core-to-core exchange kernel`.

### Task A15: K10, pattern di memoria e allocazione della quota di RAM

**Files:**
- Create:
  - `crates/oma-load/src/kernels/k10.rs`;
  - `crates/oma-load/src/sys/memory.rs`.
- Modify: `crates/oma-load/src/kernels/mod.rs`, `crates/oma-load/src/kernel.rs`

**Interfaces:**
- Consumes: A8; `crc32c_u64` di A9.
- Produces:
  - **`sys::memory::Region`:**
    - `alloc(bytes) -> Result<Region, KernelError>` con `VirtualAlloc(MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE)`, in blocchi da al massimo 1 GiB;
    - `as_mut_slice() -> &mut [u64]`;
    - `Drop` chiama `VirtualFree(MEM_RELEASE)`.

    Usata anche da K3 al posto di `Vec` per la memoria sopra i 64 MiB.
  - **`k10`:** una passata per iterazione su un pattern alla volta, a rotazione fra quelli di `ctx.patterns`:
    - `moving_inversions`: scrive v; sale controllando v e scrivendo !v; scende controllando !v e scrivendo v; v viene dal seme;
    - `modulo20`: per ogni offset 0–19, scrive v nelle posizioni ≡ offset e !v nelle altre, poi controlla;
    - `random`: riempie dal seme, poi rilegge rigenerando dal seme;
    - `address`: ogni parola contiene il proprio indirizzo XOR il seme, scritto in un verso e controllato nell'altro;
    - `crc_copy`: copia blocchi da 1 MiB da una metà all'altra con CRC32C durante la copia, controlla il CRC della destinazione e lo confronta.

    Scritture in streaming (`_mm256_stream_si256` in AVX2, `_mm_stream_si128` in SSE2), poi `_mm_sfence` prima della rilettura. Il primo valore sbagliato dà `Mismatch` con atteso e ottenuto.
  - **Riduzione (DA10):** `worker()` prova la quota per thread, poi la metà, fino a 256 MiB, e lo segnala al motore con `KernelError::Memory(bytes_ottenuti)`, che manda `Notice { code: "ram_reduced", value }`. Sotto i 256 MiB dà `KernelError::Insufficient`.

- [ ] **Step 1: test che falliscono** (regioni da 8 MiB):
  - `each_pattern_passes_on_good_memory`;
  - `each_pattern_catches_a_flipped_bit`, attraverso l'hook `#[cfg(test)]` che capovolge un bit fra scrittura e lettura;
  - `crc_copy_detects_a_corrupted_destination`;
  - `region_alloc_and_free`;
  - `allocation_failure_halves_then_skips`, con un allocatore iniettato nel test che fallisce sopra una soglia.
- [ ] **Step 2:** `cargo test -p oma-load k10 memory`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-load`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(load): K10 RAM pattern kernel with budget reduction`.

### Task A16: `oma-load` da capo a fondo

**Files:**
- Create: `crates/oma-load/tests/e2e.rs`
- Modify: nessun file di produzione, salvo le correzioni che il test trova.

**Interfaces:**
- Consumes:
  - l'eseguibile `env!("CARGO_BIN_EXE_oma-load")`;
  - `LoadPipeServer`, `random_load_pipe_name` e `KillOnCloseJob` di A5;
  - i messaggi di A1.
- Produces: la prova che il processo vero rispetta il protocollo.

- [ ] **Step 1: test** (`#[cfg(windows)]`). Ogni prova crea la pipe, avvia il processo con `--pipe` e lo assegna a un Job. I piani durano al massimo 2 s, su 2 thread, con `cores` limitato ai primi 2 core.
  - `handshake_topology_and_short_plan_complete`: `Hello` con `isa` non vuoto, poi `Topology` con `apic_id` presenti, poi `Run` con K5 `l2` per 2 s, poi `Progress`, poi `PhaseDone`, poi `Finished { completed }`, e codice d'uscita 0;
  - `every_kernel_runs_one_second`: ogni `KernelId` per 1 s su 1 thread, al miglior set; nessun `Error`; K9 su 2 thread;
  - `injected_fault_reaches_the_app_as_error_on_core_1`: solo in debug, `--inject-fault k5:1`, poi un `Error { kind: mismatch, core: Some(1) }`;
  - `stop_finishes_within_one_second`;
  - `closing_the_pipe_exits_the_process`;
  - `invalid_plan_exits_with_usage`: 0 fasi, poi codice 1.
- [ ] **Step 2:** `cargo test -p oma-load --test e2e`. Atteso: PASS, dopo le correzioni. Durata totale sotto i 20 s.
- [ ] **Step 3:** `cargo test --workspace`, clippy. Atteso: PASS.
- [ ] **Step 4: commit** `test(load): end-to-end runs of oma-load over the private pipe`.

### Task A17: app, host di `oma-load`

**Files:**
- Create:
  - `app/src-tauri/src/performance/mod.rs`;
  - `app/src-tauri/src/performance/host.rs`.
- Modify: `app/src-tauri/src/main.rs` (`mod performance;`)

**Interfaces:**
- Consumes:
  - A5: `LoadPipeServer`, `random_load_pipe_name`, `KillOnCloseJob`, `LoadConnection`;
  - A1;
  - lo schema di `app/src-tauri/src/overlay/host.rs`: `check_client`, accettazione a fette, `Hello`.
- Produces:
  - **Costanti:** `LOAD_EXE: &str = "oma-load.exe"` e `fn load_exe(current_exe: &Path) -> PathBuf`, accanto all'eseguibile.
  - **`LoadHost::start(exe: &Path, inject: Option<String>, events: mpsc::Sender<HostEvent>) -> Result<LoadHost, StartFailure>`:**
    1. crea la pipe;
    2. avvia il processo con `Command` (`CREATE_NO_WINDOW`, stdio nulli) e gli argomenti `--pipe <nome>`, più `--inject-fault <v>` se `inject` c'è;
    3. lo assegna al Job;
    4. accetta entro 5 s a fette da 250 ms e controlla il PID;
    5. manda `Hello` e aspetta `Hello` e `Topology` entro 5 s.

    Il lettore della pipe inoltra `HostEvent::Message(LoadMessage)` e, alla chiusura, `HostEvent::Closed`. Un thread aspetta l'uscita del figlio e manda `HostEvent::Exited(Option<i32>)`.
  - **`StartFailure`:**
    - `Missing`, se l'eseguibile non c'è;
    - `Spawn(io::Error)`;
    - `Timeout`, senza connessione entro 5 s;
    - `ForeignClient`, con un PID diverso;
    - `Incompatible`, se il `Hello` è di un'altra versione;
    - `NoTopology`.

    Ognuna ha una chiave i18n `performance.start.<variante>` per il `reason` di `failed_to_start`.
  - **`LoadHost`:**
    - `send(&self, msg: &LoadMessage) -> io::Result<()>`;
    - `topology(&self) -> &Topology`, `hello(&self) -> &LoadHello`;
    - `kill(&mut self)`: chiude il Job;
    - `Drop` chiude la pipe e il Job.
  - **`inject`:** con `cfg(debug_assertions)` il runner legge `OMA_LOAD_INJECT`; in release vale sempre `None`.

- [ ] **Step 1: test che falliscono:**
  - `load_exe_is_next_to_the_app`;
  - `missing_exe_is_missing`;
  - `foreign_client_pid_is_rejected`: il `check_client` riusato;
  - `real_host_runs_a_two_second_plan` (`#[cfg(windows)]`). Usa l'`oma-load.exe` di `target\debug\`; se manca, il test si salta con `eprintln!`. Il piano è quello di A16 (K5, 2 s, 2 core).
- [ ] **Step 2:** `cargo build -p oma-load`, poi `cargo test -p oma-app performance::host`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-app`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): oma-load host with private pipe, PID check and kill-on-close job`.

### Task A18: impostazioni `performance`

**Files:**
- Create:
  - `crates/oma-core/src/settings/performance.rs`;
  - `app/src/components/settings/PerformanceSection.svelte` e il suo test.
- Modify:
  - `crates/oma-core/src/settings/mod.rs`, `decode.rs`, `patch.rs` (`SCHEMA`), e i test `defaults_match_the_spec` ed `everything_changed`;
  - `app/src/lib/types.ts`;
  - `app/src/lib/backend/mockSettings.ts`;
  - `app/src/components/settings/SettingsView.svelte` (sezione `performance`, chiave `settings.section.performance` «Prestazioni»);
  - `app/src/lib/view.ts` (`SettingsTarget.section` con `performance`);
  - `en.json` e `it.json`.

**Interfaces:**
- Consumes: lo schema di `settings/overlay.rs` e `settings/log.rs` (decodifica tollerante, `Diagnostic`, `PatchError`).
- Produces:
  - **`PerformanceSettings`:**
    - `thermal_stop: bool` = `true`;
    - `cpu_stop_c: Option<u32>` = `None`, valido 60–110;
    - `stop_on_first_error: Option<bool>` = `None`;
    - `ram_share_percent: u32` = 70, valido 10–90;
    - `risk_notice_seen: bool` = `false`.

    Codifica con le chiavi `thermalStop`, `cpuStopC`, `stopOnFirstError`, `ramSharePercent`, `riskNoticeSeen`, sempre presenti.
  - **`Settings.performance`** con la codifica, `Reader::performance` e la voce in `SCHEMA`: `cpuStopC` e `stopOnFirstError` sono `nullable()`.
  - **TS:** `interface PerformanceSettings` e `Settings.performance`; il mock valida gli stessi intervalli.
  - **`PerformanceSection.svelte`:**
    - interruttore «Stop termico», con una domanda di conferma quando si spegne;
    - soglia della CPU: «Automatica (Tjmax − 5 °C o 95 °C)» oppure un numero fra 60 e 110 °C;
    - «Fermati al primo errore»: un `Segmented` con «Come il profilo», «Sì», «No»;
    - quota di RAM, fra 10 e 90%, con la nota «Restano sempre almeno 2 GB per Windows»;
    - pulsante «Mostra di nuovo l'avviso sui rischi», che rimette `riskNoticeSeen = false`.

    I termini Tjmax, stop termico e quota di RAM qui sono testo semplice: `Term` arriva in A22, che li avvolge.

- [ ] **Step 1: test che falliscono:**
  - Rust:
    - `performance_defaults`;
    - `cpu_stop_out_of_range_is_corrected`: 120 dà la diagnostica e il predefinito;
    - `ram_share_snaps_into_range`;
    - `patch_rejects_out_of_range_with_settings_error_range`;
    - `null_cpu_stop_is_automatic`;
    - i test esistenti aggiornati.
  - Vitest:
    - `disabling_thermal_stop_asks_first`;
    - `automatic_threshold_sends_null`;
    - `reset_risk_notice`.
- [ ] **Step 2:** `cargo test -p oma-core settings`, `cd app && pnpm test PerformanceSection`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test --workspace`, `cd app && pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(settings): performance section with thermal stop and RAM share`.

### Task A19: app, archivio della cronologia e ripresa dopo un crash

**Files:**
- Create: `app/src-tauri/src/performance/store.rs`
- Modify: `app/src-tauri/src/overlay/store.rs`, dove `write_file` diventa `pub(crate)`.

**Interfaces:**
- Consumes:
  - A3 (`Session`, `Journal`, `parse_*`, `session_file_name`, `is_session_id`, `prune`);
  - A6 (`crash_evidence`), A5 (`boot_time_unix_ms`);
  - `write_file` di `overlay/store.rs`, cioè file temporaneo proprio, `sync_all` e rinomina.
- Produces:
  - **`PerformanceStore::new(root: PathBuf)`:** `root` = `%LOCALAPPDATA%\OpenMonitorAdvanced\performance`; la cartella `stress\` si crea al primo salvataggio.
  - **Sessioni:**
    - `save(&self, session: &Session) -> io::Result<()>`: scrittura atomica, poi `prune` a 500; `prune` gira anche una volta all'avvio dell'app (§3.6);
    - `list(&self) -> Vec<SessionSummary>`: dalla più recente; i file illeggibili o di una versione futura si saltano con un `warn!` per file, una volta per avvio;
    - `load(&self, id: &str) -> io::Result<Option<Session>>`;
    - `delete(&self, id: &str) -> io::Result<()>`.

    `load` e `delete` rifiutano gli id che non passano `is_session_id` con `ErrorKind::InvalidInput`, e trovano il file cercando `*-<id>.json` nella cartella, mai costruendo un percorso dall'id prima del controllo.
  - **Diario:** `write_journal(&self, j: &Journal)`, `delete_journal(&self)`, `read_journal(&self) -> Option<Result<Journal, FormatError>>`.
  - **`recover(&self, boot_ms: i64, evidence: impl Fn(&str) -> io::Result<Vec<SystemEvent>>, app_version: &str) -> Option<SessionSummary>`:**
    - con un diario presente carica la sessione (l'ultimo salvataggio intermedio) o, se manca, ne crea una minima dal diario;
    - applica DA14 (`system_crash`, oppure `crashed` con `app_closed`);
    - aggiunge gli eventi di `crash_evidence(updatedAt − 60 s)` come `SessionEvent` (`whea18`, `bugcheck`, `kernelPower41`) e come conteggi WHEA;
    - salva la sessione e cancella il diario;
    - un diario rotto si cancella con un `warn!` e dà `None`.

- [ ] **Step 1: test che falliscono** (cartella temporanea, eventi finti):
  - `save_list_load_delete_round_trip`;
  - `prune_keeps_500`;
  - `ids_outside_the_session_form_are_rejected`: `..\..\x`, `C:\x`, `a/b` e un uuid maiuscolo, senza toccare file;
  - `future_format_sessions_are_skipped`;
  - `journal_older_than_boot_is_system_crash`;
  - `journal_newer_than_boot_is_app_closed`;
  - `recovery_adds_whea_and_bugcheck_events`;
  - `corrupt_journal_is_removed_and_logged`;
  - `recovery_without_saved_session_builds_one_from_the_journal`.
- [ ] **Step 2:** `cargo test -p oma-app performance::store`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-app`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): stress session history, journal and crash recovery`.

### Task A20: app, runner, comandi, eventi e toast

**Files:**
- Create:
  - `app/src-tauri/src/performance/runner.rs`;
  - `app/src-tauri/src/performance/commands.rs`.
- Modify:
  - `app/src-tauri/src/main.rs` (stato gestito, `invoke_handler`, `on_tick` nel sampler, ripresa all'avvio, chiusura in `RunEvent::Exit`);
  - `app/src-tauri/src/notifier.rs` (`LaunchTarget::Performance(String)` e `launch_for_performance(id)`);
  - `app/src-tauri/src/window.rs` (`NavigationTarget.performance: Option<PerformanceNav { page: "run" | "result", sessionId: Option<String> }>`, `show_performance(app, nav)`);
  - `app/src-tauri/src/i18n.rs` (`RUST_KEYS`);
  - `app/src-tauri/capabilities/default.json`.

**Interfaces:**
- Consumes: A2–A4, A5 (`KeepAwake`, `memory_status`, `topology::read`), A6, A17–A19; `Engine::latest`/`schema`, `ServiceStatusTable`, `SystemToaster`.
- Produces:
  - **`PerformanceRunner`** (stato gestito `Arc<PerformanceRunner>`):
    - `start(&self, request: StartRequest) -> Result<String, StartError>`:
      - un test alla volta, altrimenti `StartError::Busy`;
      - costruisce la sessione e il piano (A2, con il seme da `random_uuid_v4`);
      - avvia il thread `oma-perf-runner`;
      - restituisce l'id;
      - un avvio fallito salva comunque una sessione `failed_to_start`.
    - `stop(&self)`;
    - `status(&self) -> RunStatus`: `idle` senza test;
    - `on_tick(&self, out: &TickOutput, schema: &Schema)`: non blocca e manda il campione al thread con `try_send`;
    - `is_running(&self) -> bool`;
    - `shutdown(&self, timeout: Duration)`: per DA16.
  - **Il thread del runner:**
    - crea `KeepAwake`;
    - avvia `LoadHost`;
    - fa `latest_record_id` all'inizio;
    - gira ogni 250 ms con `recv_timeout` ed esegue le `Action` di A4: pipe, Job, store, `whea_after` sul proprio thread, toast con `launch_for_performance`;
    - emette `performance-status` a ogni cambio di stato e ogni secondo, solo se `window::any_open`.
  - **Comandi** (DA20):
    - `performance_system() -> SystemInfo { cpuModel, logical, cores, isa: Vec<Isa>, ramTotal, ramBudget, serviceConnected, tjmaxC: Option<f64>, stopC: f64, hypervisor }`;
    - `performance_preview(request) -> Result<Plan, String>`;
    - `performance_start(request) -> Result<String, String>`, `#[tauri::command(async)]`;
    - `performance_stop()`, `performance_status() -> RunStatus`;
    - `performance_history() -> Vec<SessionSummary>`, `performance_session(id) -> Result<Option<Session>, String>`, `performance_delete(id) -> Result<(), String>`;
    - `performance_export(id) -> Result<Option<String>, String>`: finestra di salvataggio di `tauri-plugin-dialog`, come `report.rs`, con nome `oma-stress-<AAAAMMGG-HHMMSS>.json`; scrive la sessione intera;
    - `performance_quit_confirmed()`.

    Ogni comando ha la sua voce `allow-performance-*` in `default.json`.
  - **Avvio dell'app:** `store.recover(...)` su un thread dopo il setup; se restituisce una sessione, arriva il toast `performance.toast.recovered` che apre il risultato.
  - **Toast finale:** titolo `performance.toast.title`; testo = il verdetto di T3 tradotto da Rust con i parametri.

- [ ] **Step 1: test che falliscono.** Il runner riceve un `LoadHost` finto attraverso un trait `LoadLink` interno, che produce i messaggi di un copione.
  - `start_while_running_is_busy`;
  - `scripted_run_saves_a_passed_session_and_toasts`;
  - `scripted_error_on_core_2_saves_unstable_core_2`;
  - `start_failure_saves_failed_to_start_with_the_reason`;
  - `status_events_only_with_a_window_open`;
  - `on_tick_never_blocks_when_the_channel_is_full`;
  - `shutdown_stops_and_saves_stopped_user`;
  - `launch_target_performance_round_trips`;
  - `navigation_target_serializes_performance`;
  - `rust_keys_exist_in_both_catalogs`, già esistente, con le chiavi nuove.
- [ ] **Step 2:** `cargo test -p oma-app performance`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test --workspace`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): stress test runner, commands, status events and toasts`.

### Task A21: tray, chiusura della finestra e uscita durante un test

**Files:**
- Modify:
  - `app/src-tauri/src/tray.rs`, `app/src-tauri/src/tray_icon.rs`;
  - `app/src-tauri/src/window.rs` (`quit_action`);
  - `app/src-tauri/src/main.rs` (`keep_running_on_last_close`, collegamento del runner al tray);
  - `app/src/App.svelte`, più il dialogo `app/src/components/performance/QuitDialog.svelte` e il suo test.

**Interfaces:**
- Consumes: A20 (`is_running`, `stop`, un callback di stato), T4.
- Produces:
  - **Icona:**
    - `IconMarks { recording: bool, testing: bool }`;
    - `render(&IconContent, IconStyle, marks: IconMarks)` disegna il punto `--warn` in alto a destra quando `testing` (DA17);
    - la cache dell'icona usa `IconMarks` al posto del `bool`.
  - **`TrayController::set_test(&self, test: Option<TestMark { component: String, objective: String }>)`:**
    - prefisso del tooltip `tray.performance.tooltip`;
    - voci di menu `perf_stop` («Ferma il test») e `perf_open` («Apri il test in corso»), solo con un test in corso, prima del separatore di «Esci»;
    - `perf_open` porta a `show_performance(app, run)`.
  - **Uscita:**
    - `QuitAction::AskPerformance`;
    - `quit_action(source, editor_open, editor_dirty, test_running)`: `Tray` con un test in corso dà `AskPerformance`, prima di `AskEditor`; `Flag` dà sempre `Exit`;
    - `AskPerformance` mostra la finestra principale ed emette `performance-quit`;
    - la UI mostra `QuitDialog` (T4) e chiama `performance_quit_confirmed`, che ferma il test, aspetta la fine per al massimo 3 s e richiama `window::quit(app, QuitSource::Tray)`.
  - **Finestra:** `keep_running_on_last_close(close_to_tray, test_running)` vale `true` se c'è un test in corso. In quel caso arriva una volta il toast `performance.closeToTray`.

- [ ] **Step 1: test che falliscono:**
  - Rust:
    - `testing_mark_draws_the_amber_dot`: pixel del punto nel PNG;
    - `test_menu_items_only_while_running`;
    - `tray_quit_with_a_test_asks_first`;
    - `quit_flag_never_asks`, già esistente ed esteso;
    - `closing_the_window_keeps_running_during_a_test`.
  - Vitest:
    - `quit_dialog_confirm_calls_the_backend`;
    - `quit_dialog_cancel_keeps_the_test`.
- [ ] **Step 2:** `cargo test -p oma-app tray window`, `cd app && pnpm test QuitDialog`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test --workspace`, `cd app && pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): tray mark, stop item and quit confirmation during a stress test`.

### Task A22: UI, fondamenta della vista Prestazioni, `Term.svelte` e glossario

Prima di A22–A25: `frontend-design:frontend-design`, con lo stile Synthwave di `app/src/styles/theme.css` e i mockup approvati `stress-setup.html` (B) e `stress-run.html` del brainstorming. I mockup sono descritti nel §3 della spec; i file restano in `.superpowers/brainstorm/`, non versionati.

**Files:**
- Create:
  - `app/src/components/common/Term.svelte` e il suo test;
  - `app/src/components/performance/PerformanceView.svelte`;
  - `app/src/lib/performance/performance.svelte.ts` e il suo test;
  - `app/src/lib/performance/glossary.ts`;
  - `app/src/lib/performance/glossary.test.ts`.
- Modify:
  - `app/src/lib/view.ts` (`View` con `performance`);
  - `app/src/App.svelte`, `app/src/components/TopBar.svelte` e il suo test;
  - `app/src/lib/types.ts`;
  - `app/src/lib/backend/backend.ts`, `tauri.ts`, `mock.ts`, `app/src/test/fake-backend.ts`;
  - `en.json`, `it.json`;
  - `app/src/components/settings/PerformanceSection.svelte`, dove i termini di A18 diventano `Term`.

**Interfaces:**
- Consumes: i comandi e gli eventi di A20; `catalog.json` di DA19; T1, T2 e T4.
- Produces:
  - **`View = 'simple' | 'advanced' | 'settings' | 'performance'`:**
    - `showView('performance')` non scrive `view.last`, come per le impostazioni;
    - `navigate()` gestisce `NavigationTarget.performance`.
  - **`TopBar`:** una terza scheda `view.performance` nel selettore.
  - **Tipi TS** che rispecchiano A2–A4 e A20: `StartRequest`, `Custom`, `Plan`, `Phase`, `RunStatus`, `Session`, `SessionSummary`, `SystemInfo`, `Outcome`.
  - **`Backend`:** i metodi `performanceSystem`, `performancePreview`, `performanceStart`, `performanceStop`, `performanceStatus`, `performanceHistory`, `performanceSession`, `performanceDelete`, `performanceExport`, `performanceQuitConfirmed`, `onPerformanceStatus` e `onPerformanceQuit`.
    - Ci sono in `tauri.ts`, `mock.ts` e `FakeBackend`.
    - Il mock simula un test di 60 s con un errore sul core 2 quando l'URL ha `?perf=error`, e uno superato con `?perf=pass`.
  - **`PerformanceStore`** (`performance.svelte.ts`):
    - `status`, `system`, `history` in `$state.raw`;
    - `connect(backend)`: prima si iscrive, poi legge;
    - `start(request)`, `stop()`, `refreshHistory()`;
    - `running: boolean` derivato.
  - **`PerformanceView.svelte`:**
    - barra laterale con il gruppo «Stress test»: «Nuovo test», che diventa «In corso ●» mentre un test gira, e «Cronologia»;
    - le pagine `new`, `run`, `result:<id>`, `history`; le pagine stesse arrivano in A23–A25;
    - un test in corso apre `run`.
  - **`Term.svelte`:**
    - props `term: string` e lo snippet `children`, opzionale: senza, mostra `glossary.<term>.name`, se c'è, oppure il termine stesso;
    - `<span class="term" tabindex="0" aria-describedby={id}>` sottolineato a puntini;
    - un `<span role="tooltip" id={id}>` con `t('glossary.' + term)`, visibile al passaggio del mouse e con il focus, chiuso con Esc;
    - il tooltip resta dentro la finestra.
  - **`glossary.ts`:** `MODE_TERMS`, `ISA_TERMS` e `PATTERN_TERMS` letti da `testdata/performance/catalog.json` (import JSON di Vite), più `TERMS` = le chiavi di T2.
  - **Testi:** T1, T2 e T4 in italiano esatto e in inglese, in `it.json` e `en.json`.

- [ ] **Step 1: test che falliscono** (Vitest):
  - `glossary.test.ts`:
    - `every_catalog_mode_isa_and_pattern_has_an_entry`, con `.name` e la spiegazione;
    - `every_term_used_in_performance_pages_has_a_key`: legge con `import.meta.glob('../../components/performance/**/*.svelte', { query: '?raw' })` e con `PerformanceSection.svelte` tutti i `<Term term="...">` e controlla `glossary.<term>`;
    - la parità delle chiavi `en`/`it` resta nel test esistente;
  - `Term.test.ts`:
    - `term_is_focusable_and_described`;
    - `tooltip_shows_on_hover_and_focus`;
    - `escape_hides_the_tooltip`;
  - `TopBar.test.ts`: `performance_tab_switches_view`;
  - `performance.test.ts`:
    - `connect_subscribes_before_reading`;
    - `running_follows_status`;
    - `start_while_running_is_ignored`;
  - `App`: `performance_view_is_never_saved_as_last_view`.
- [ ] **Step 2:** `cd app && pnpm test`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): performance view shell, Term tooltip component and glossary`.

### Task A23: UI, procedura guidata e avviso sui rischi

**Files:**
- Create:
  - `app/src/components/performance/StressWizard.svelte` e il suo test;
  - `app/src/components/performance/WizardCustomize.svelte`;
  - `app/src/components/performance/RiskNotice.svelte`;
  - `app/src/lib/performance/format.ts` e il suo test.
- Modify: `PerformanceView.svelte`, `en.json`, `it.json`.

**Interfaces:**
- Consumes: A22; `performanceSystem` e `performancePreview`; `settings.performance`.
- Produces:
  - **I quattro passi (§3.4):**
    1. **Componente:**
       - CPU, con il modello e il numero di core e thread;
       - RAM, con il totale e la quota.

       Senza servizio la CPU resta disponibile con l'avviso `performance.warn.noService`. La RAM non è disponibile sotto i 256 MiB di quota, con il motivo.
    2. **Obiettivo:** due riquadri grandi con i testi di T4.
    3. **Durata:** i preset di A2 con la durata («Rapido · 5 min»…). I nomi sono `performance.preset.<id>`: Rapido, Standard, Lungo, Notte.
    4. **Riepilogo:**
       - le fasi da `performancePreview`, ciascuna con il nome e il `Term` della modalità, il set d'istruzioni, il carico e la durata;
       - gli avvisi: senza servizio, set d'istruzioni rilevato, quota di RAM, macchina virtuale;
       - «Personalizza», che apre `WizardCustomize`;
       - il pulsante «Avvia».
  - **`WizardCustomize`:**
    - per ogni kernel della preview: casella e minuti;
    - set d'istruzioni: automatico più quelli di `system.isa`, senza AVX-512 se manca;
    - thread: tutti oppure uno per core;
    - «Entrambi i thread del core» nelle fasi un core alla volta, con il `Term` `smt`;
    - «Fermati al primo errore».

    Ogni modifica richiama `performancePreview` con il `Custom`, e il totale si aggiorna.
  - **`RiskNotice`:** si mostra prima di «Avvia» finché `riskNoticeSeen` è `false`, con i testi di T4. «Non mostrare più» scrive `riskNoticeSeen = true`.
  - **Avvio:** chiama `performanceStart` e passa alla pagina `run`. Durante un test «Avvia» è disattivato.
  - **`format.ts`:**
    - `formatDuration(s)`: «5 min», «1 h 30 min», «8 h»;
    - `phaseLabel(phase, t)`: il nome di T1 più dimensione e set, per esempio «FFT piccole · core e cache · AVX2».

- [ ] **Step 1: test che falliscono:**
  - `wizard_walks_four_steps_and_back`;
  - `ram_is_disabled_with_reason_below_budget`;
  - `no_service_shows_the_warning_but_allows_cpu`;
  - `summary_lists_phases_with_terms`;
  - `customize_rebuilds_the_preview_and_total`;
  - `avx512_hidden_when_unsupported`;
  - `risk_notice_shows_once_then_never`;
  - `start_goes_to_the_run_page`;
  - `start_disabled_while_running`;
  - `format_duration_cases`.
- [ ] **Step 2:** `cd app && pnpm test StressWizard format`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): stress test wizard with customize and risk notice`.

### Task A24: UI, durante il test e risultato

**Files:**
- Create:
  - `app/src/components/performance/StressRun.svelte` e il suo test;
  - `app/src/components/performance/StressResult.svelte` e il suo test;
  - `app/src/components/performance/CoreGrid.svelte`;
  - `app/src/components/performance/EventLog.svelte`.
- Modify: `PerformanceView.svelte`, `en.json`, `it.json`.

**Interfaces:**
- Consumes:
  - A22 e `RunStatus`;
  - `performanceSession`, `performanceExport` e `performanceStart` con `retryCore`;
  - `HistoryChart.svelte` della vista Avanzata, con `sensors` = i sensori di temperatura e potenza della CPU di DA5 e la finestra di 10 minuti (§3.5);
  - `AnimatedNumber.svelte`.
- Produces:
  - **`StressRun`** (§3.5, mockup `stress-run.html`):
    - intestazione con obiettivo e componente, stato a pillola, tempo trascorso e totale, «Ferma e salva»;
    - barra delle fasi con le etichette;
    - cinque riquadri:
      - temperatura, con il massimo e la soglia di stop;
      - potenza;
      - clock;
      - errori di calcolo, con il numero di verifiche;
      - errori WHEA;
    - il grafico, nascosto senza i sensori, con la nota «Grafico non disponibile senza il servizio»;
    - `CoreGrid` nelle fasi `core_cycle`, con lo stato di ogni core e il core in prova evidenziato;
    - `EventLog` con gli eventi tradotti da `performance.event.<code>`;
    - gli avvisi di `RunStatus.warnings`.

    Ogni termine tecnico passa da `Term`: WHEA, verifica, Tjmax, stop termico, core N, fase.
  - **`StressResult`:**
    - verdetto di T3 con fase, kernel, core, tempo, clock e temperatura al momento dell'errore;
    - con `errors_core` anche il consiglio `performance.advice.core`;
    - azioni:
      - «Riprova solo il core N», con `retryCore { core, kernel }` del primo errore, visibile solo con un core;
      - «Ripeti il test», con lo stesso `request` della sessione;
      - «Esporta (JSON)»;
    - riepilogo della sessione: durata, massimi e medie, WHEA per ID e per APIC con il core;
    - stato di ogni core, ed elenco degli errori (i primi 200, più gli altri contati).
  - **Fine:** a fine test la pagina `run` passa da sola a `result:<id>`.

- [ ] **Step 1: test che falliscono** (con `FakeBackend` e stati finti):
  - `run_shows_tiles_phases_and_stop`;
  - `stop_calls_the_backend`;
  - `core_grid_only_in_core_cycle_phases`;
  - `warnings_are_listed`;
  - `chart_hidden_without_cpu_sensors`;
  - `finishing_moves_to_the_result`;
  - `result_shows_unstable_core_with_advice_and_retry`;
  - `result_without_single_core_has_no_retry`;
  - `export_calls_the_backend`;
  - `system_crash_result_shows_the_phase`.
- [ ] **Step 2:** `cd app && pnpm test StressRun StressResult`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): stress test live and result pages`.

### Task A25: UI, cronologia

**Files:**
- Create: `app/src/components/performance/StressHistory.svelte` e il suo test.
- Modify: `PerformanceView.svelte`, `en.json`, `it.json`.

**Interfaces:**
- Consumes: `performanceHistory`, `performanceDelete`, `performanceStart`.
- Produces:
  - **Lista (§3.6):** data locale, componente, obiettivo, durata e verdetto con il colore dell'esito (`--ok`, `--warn`, `--crit`).
  - **Filtro** per componente: Tutti, CPU, RAM.
  - **Voce:** un clic apre `result:<id>`; «Elimina» chiede conferma; «Ripeti il test» riparte senza rifare i passi.
  - **Lista vuota:** un invito a «Nuovo test».

- [ ] **Step 1: test che falliscono:**
  - `lists_sessions_newest_first`;
  - `filter_by_component`;
  - `delete_asks_then_removes`;
  - `repeat_starts_with_the_same_request`;
  - `empty_history_invites_a_new_test`.
- [ ] **Step 2:** `cd app && pnpm test StressHistory`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): stress test history`.

### Task A26: installer, firma e licenze

**Files:**
- Modify:
  - `scripts/lib/OmaOverlayPayload.psm1`: diventa `Save-OmaHelperExe -Package <oma-overlay|oma-load> ...`, con lo stesso comportamento;
  - `scripts/build-installer-payload.ps1`: compila e mette in staging `target\installer-payload\load\oma-load.exe`;
  - `app/src-tauri/nsis/oma.nsh`:
    - `OMA_LOAD_EXE`;
    - controllo del payload;
    - `File` in `NSIS_HOOK_POSTINSTALL`;
    - `OMA_STOP_LOAD`, accanto a `OMA_STOP_OVERLAY`, in `OmaCloseApp` e in `NSIS_HOOK_PREUNINSTALL`;
    - `Delete /REBOOTOK "$INSTDIR\oma-load.exe"`;
    - `LangString omaLoadFailed` in inglese e italiano;
  - `scripts/lib/OmaSigning.psm1`: `$SignedNames` con cinque nomi; `$PayloadExes` con `load = 'oma-load.exe'`; i percorsi attesi, la verifica del payload e `OwnPayloadNames`;
  - `scripts/sign-shim.ps1`, `scripts/verify-signatures.ps1`: solo la documentazione;
  - `.github/workflows/release.yml`: i due cicli `foreach ($rel in ...)` con `'load\oma-load.exe'`, e l'elenco dell'artefatto con `target/signing/unsigned/oma-load.exe`;
  - `.signpath/artifact-configuration-binaries.xml`: la voce `oma-load.exe`, come quella dell'overlay;
  - i test Pester `BuildInstallerPayload.Tests.ps1`, `SignShim.Tests.ps1` e `VerifySignatures.Tests.ps1` (da quattro a cinque file firmati);
  - `scripts/measure-footprint.ps1` e `MeasureFootprint.Tests.ps1`: `oma-load.exe` contato fra i processi dell'app se c'è;
  - `THIRD_PARTY_NOTICES.md`: sezioni «FIRESTARTER» e «OpenDCDiag», nello stile di quella di PresentMon:
    - che cosa si è adattato e in quali file;
    - commit, copyright e licenza;
    - per FIRESTARTER: «GPL-3.0-or-later, the same licence as OpenMonitor Advanced; the full text is in LICENSE»;
  - `scripts/lib/OmaLicenses.psm1` e `scripts/generate-licenses.ps1`:
    - un ecosistema `Adapted` («Source code adapted into OpenMonitor Advanced»), dopo `Programs`;
    - la voce OpenDCDiag (Apache-2.0, «Copyright 2022 Intel Corporation», testo `scripts/licenses/Apache-2.0.txt`);
    - il test di `Licenses.Tests.ps1` sulle fixture;
  - `THIRD_PARTY_LICENSES.txt`, rigenerato;
  - `docs/release.md` e `CODE_SIGNING.md`: cinque file firmati, con `oma-load.exe` descritto come il processo dei test di carico che l'app avvia.

**Interfaces:**
- Consumes: A7 (risorsa di versione).
- Produces: un setup con `oma-load.exe` firmabile come gli altri quattro file.

- [ ] **Step 1: test che falliscono** (Pester, `-ExcludeTagFilter Integration`):
  - `payload stages oma-load.exe`;
  - `register-payload accepts oma-load.exe`;
  - `import-signed requires the five signed files`;
  - `accepts the five signed files`;
  - `requires exactly the five names`;
  - il test delle licenze con la sezione `Adapted`.
- [ ] **Step 2:** `Import-Module Pester -RequiredVersion 5.7.1; Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:**
  - di nuovo Pester;
  - `pwsh scripts/generate-licenses.ps1`, poi `pwsh scripts/generate-licenses.ps1 -Check`;
  - `pwsh scripts/build-installer-payload.ps1`, solo la compilazione e lo staging, senza installare niente.

  Atteso: PASS.
- [ ] **Step 5: commit** `build: ship and sign oma-load.exe; attribute FIRESTARTER and OpenDCDiag`.

### Task A27: documenti, misure e grafo

**Files:**
- Modify:
  - `CLAUDE.md`: struttura (`crates/oma-load`, `oma-core::load`, `oma-ipc::load`, `oma-win` `private_pipe`/`load_pipe`/`job`/`topology`/`eventlog`/`power`, `app/src-tauri/src/performance/`, `app/src/components/performance/`), comandi (`cargo build -p oma-load` prima di `pnpm tauri dev`, `OMA_LOAD_INJECT`) e stato della M8a1;
  - `README.md` e `README.it.md`: la vista Prestazioni e lo stress test, con il limite della RAM (§14) e il link alle notice;
  - `docs/perf-budget.md`: sezione «M8a1», con la misura a riposo e le misure sotto carico chieste all'utente in A28;
  - `docs/follow-ups.md`: le voci aperte (M8a2, prove in VM dell'installer, confronto della numerazione dei core con BIOS e Ryzen Master).

**Interfaces:**
- Consumes: tutto il branch.
- Produces: documentazione allineata.

- [ ] **Step 1:** `pwsh scripts/measure-footprint.ps1` a riposo, senza test, e i valori in `docs/perf-budget.md`. Atteso: nucleo < 1% CPU, tray < 30 MB, finestra < 200 MB, come prima.
- [ ] **Step 2:** aggiornare i documenti.
- [ ] **Step 3:**
  - `cargo fmt --all --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo test --workspace`;
  - `cd app && pnpm test && pnpm check && pnpm build`;
  - `dotnet test service/OpenMonitorAdvanced.slnx`;
  - `pwsh scripts/check-version.ps1`;
  - `PYTHONHASHSEED=0 graphify update .`.

  Atteso: PASS.
- [ ] **Step 4: commit** `docs: M8a1 stress test documentation and footprint`.

### Task A28: prove dal vivo con l'utente

Le fa l'utente, un blocco alla volta, come nelle milestone precedenti (memoria «user admin shell»: comandi uno per blocco). L'agente prepara i comandi, chiede prima di ogni carico pesante e non usa mai input sintetico. Prima delle prove: `cargo build -p oma-load`, poi `cd app && pnpm tauri dev`.

| # | Prova | Atteso |
|---|---|---|
| P1 | CPU · Verifica normale · Rapido (5 min) | Fine regolare, «Superato», sessione nella cronologia, toast finale. |
| P2 | RAM · Verifica normale · Rapido (15 min) | Fine regolare; la quota di RAM del riepilogo è quella usata (Task Manager). |
| P3 | `$env:OMA_LOAD_INJECT='k5:2'`, poi CPU · Stabilità overclock · Standard, fermato dopo il ciclo per core | «Instabile · core 2», con clock e temperatura; «Riprova solo il core 2» avvia il piano breve. |
| P4 | «Ferma e salva» a metà | «Fermato da te», sessione salvata. |
| P5 | «Ferma il test» dal tray | Come P4. |
| P6 | Chiusura della finestra con un test in corso | Il test continua, toast «Il test continua nella tray», toast finale con il verdetto, clic sul toast che apre il risultato. |
| P7 | Stop termico con `cpuStopC` = 60 | «Fermato: temperatura a N °C» entro due letture sopra la soglia. |
| P8 | «Esci» dal tray durante un test | Domanda di conferma; «Ferma ed esci» salva `stopped_user` ed esce; `oma-load.exe` sparisce dal Task Manager. |
| P9 | Facoltativa: riavvio forzato durante un test, solo se l'utente vuole | Al riavvio dell'app, toast e risultato «Interrotto da un crash del sistema durante …», con gli eventi del registro. |
| P10 | WHEA leggibili | Nessun avviso «Errori hardware non leggibili» su questo PC. |
| P11 | Numerazione dei core | «Core N» dell'app confrontato con Ryzen Master (C01 = core 0) e con il Curve Optimizer del BIOS, se l'utente lo apre. |
| P12 | Tooltip | Ogni termine tecnico di procedura guidata, durante il test, risultato, cronologia e impostazioni mostra la sua spiegazione con il mouse e con Tab. |
| P13 | Budget durante un test | `scripts/measure-footprint.ps1` durante P1: finestra < 200 MB; `oma-load` annotato. |
| P14 | Senza servizio (servizio fermato dall'utente) | Lo stress della CPU parte con l'avviso; nessuno stop termico; il grafico mostra la nota. |

- [ ] **Step 1:** preparare l'elenco con i comandi esatti per l'utente e chiedere il via per ogni prova pesante.
- [ ] **Step 2:** registrare gli esiti in `docs/follow-ups.md` e nella memoria del progetto (`m8a1-followups.md`).
- [ ] **Step 3: commit** `docs: record the M8a1 live checks`.
- [ ] **Step 4:** `superpowers:finishing-a-development-branch`: revisione dell'intero branch, merge in `main` in locale, nessun push senza richiesta.
