# OpenMonitor Advanced — Design v1

- **Data:** 2026-09-24
- **Stato:** approvato in brainstorming, in attesa di revisione della spec
- **Licenza del progetto:** GPL-3.0

## 1. Obiettivo e contesto

OpenMonitor Advanced è un software **open source** per il monitoraggio delle risorse hardware di un PC. Vogliamo abbinare la copertura dei sensori dei tool storici (HWiNFO, HWMonitor, LibreHardwareMonitor) a un'interfaccia **moderna e accattivante**. La loro principale debolezza è proprio la grafica, ferma agli anni '90 e 2000.

### 1.1 Requisiti raccolti

| Tema | Decisione |
|---|---|
| Piattaforme | **Windows 10/11 nella v1.** L'architettura deve permettere di aggiungere Linux senza riscrivere nucleo e interfaccia. macOS è fuori perimetro. |
| Pubblico | **Gamer e appassionati PC.** |
| Funzioni v1 | Monitoraggio dal vivo; storico breve con grafici e min/max/media; icona nella system tray; avvisi a soglia; log su CSV. |
| Fuori dalla v1 | Overlay in-game (OSD), controllo delle ventole, overclock. Ognuno sarà un sotto-progetto con una propria spec. Anche il colore d'accento personalizzabile è rimandato. |
| Organizzazione UI | Primo livello: **vista Semplificata e vista Avanzata**. Secondo livello: **sezioni per tipo di hardware** (CPU, GPU, RAM, dischi, rete, scheda madre, batteria). |
| Lingue | **Inglese e italiano**, con tutte le stringhe in file di traduzione fin dall'inizio. |
| Leggerezza | L'app non deve falsare le misure: consumo a riposo minimo, nessuna modifica della risoluzione del timer di sistema; lo scorrimento continuo dei soli grafici visibili rispetta il budget (§7.5). |

### 1.2 Criteri di successo

- Un utente apre l'app e capisce **in un'occhiata** se il PC sta bene, grazie alla vista Semplificata con il banner di stato.
- Un appassionato trova nella vista Avanzata temperature, clock, potenze, tensioni, throttling, min/max e storico di ogni componente.
- **Budget di prestazioni**, misurato a ogni milestone:
  - nucleo a riposo < 1% di CPU;
  - processo in tray (finestra chiusa) < 30 MB di RAM;
  - finestra aperta < 200 MB in totale, WebView2 compresa;
  - servizio `oma-service` con un client sottoscritto a 1 s: < 1% di CPU e < 80 MB di memoria privata (limite separato da quello dell'app, introdotto in M4).
- Senza privilegi amministrativi, e quindi anche in "modalità anti-cheat", l'app funziona in **modalità base** con utilizzo di CPU, RAM, dischi e rete e con **tutte le metriche GPU**.

## 2. Architettura

Si è scelto l'**approccio A**: app Tauri con nucleo Rust senza privilegi, più un servizio .NET opzionale basato su LibreHardwareMonitor per i sensori che richiedono un driver kernel.

```
┌──────────────────── oma-app (Tauri 2, utente normale) ────────────────────┐
│  UI: Svelte 5 + TypeScript + uPlot  (WebView2)                            │
│        ▲ eventi "snapshot"             │ comandi (vista, regole, log…)    │
│  ──────┴───────────────────────────────▼──────────────────────────────    │
│  oma-core (Rust)                                                          │
│   • Scheduler: un solo timer (default 1 s, configurabile 0,5–5 s)         │
│   • Provider senza privilegi:                                             │
│       sys → carico/clock CPU, RAM, I/O dischi, rete, batteria, SMBIOS     │
│       gpu → base PDH/D3DKMT + NVML/NVAPI/ADL/IGCL caricati a runtime      │
│       svc → client named pipe verso il servizio                           │
│   • Merge per sensore con priorità di fonte e registrazione dell'origine  │
│   • Storico (ring buffer in RAM), motore regole, logger CSV, tray         │
└───────────────────────────────▲───────────────────────────────────────────┘
                                │ named pipe con ACL, solo lettura dei sensori
┌───────────────────────────────┴── oma-service (.NET 10, servizio Windows) ┐
│  LibreHardwareMonitorLib + driver PawnIO:                                 │
│  temp/tensioni/potenza/throttling CPU, Super I/O (ventole, sensori        │
│  scheda madre), SPD/timing RAM, SMART/NVMe, controller ventole USB, PSU   │
│  Opzionale: se assente o fermato → l'app gira in "modalità base"          │
└───────────────────────────────────────────────────────────────────────────┘
```

### 2.1 Perché questa architettura

- **Su Windows solo CPU, scheda madre e timing della RAM richiedono un driver kernel.** Servono accessi a MSR, porte I/O e SMBus.
  - L'unica via praticabile nel 2026 è **PawnIO**. WinRing0 è nella blocklist di Microsoft e viene segnalato da Defender (CVE-2020-14979).
  - Un driver proprietario richiederebbe firma WHCP e un certificato EV: dall'aprile 2026 Windows non si fida più dei driver cross-signed.
- **Le GPU non richiedono privilegi.** NVML, NVAPI, ADL, IGCL, i contatori PDH e D3DKMT funzionano tutti da utente normale, come verificato su una RTX 4080 e su un'iGPU AMD.
- **LibreHardwareMonitorLib** (MPL-2.0, molto attiva, su PawnIO dalla versione 0.9.5) contiene anni di supporto per singoli chip e singole schede. Riscriverlo in Rust ritarderebbe la v1 di molto. La parte .NET resta isolata dietro il protocollo IPC e si potrà sostituire pezzo per pezzo.
- **Tauri + Svelte** è il modo più efficace per ottenere un'interfaccia animata e moderna mantenendo un backend nativo efficiente. Rust chiama senza costi aggiuntivi le API C dei vendor e di Win32.

### 2.2 Processi e ciclo di vita

- **oma-app** è l'unico processo avviato dall'utente e non è mai elevato.
  - Contiene il nucleo Rust, che raccoglie, registra e valuta le regole, e la WebView.
  - **Quando si chiude la finestra la WebView viene distrutta**, liberando circa 100 MB. Il processo Rust resta nella tray e continua a raccogliere dati, scrivere il log e valutare le regole.
- **oma-service** è un servizio Windows che gira come LocalSystem, perché PawnIO richiede privilegi amministrativi.
  - Interroga i sensori **solo mentre almeno un client è sottoscritto**. LibreHardwareMonitor, e quindi l'accesso a PawnIO, si apre alla prima sottoscrizione e resta aperto fino all'uscita del processo (verificato in M4). Due i motivi:
    - `Close()` non rilascia gli handle PawnIO dell'SMBus;
    - `Open()` costa circa 4,5 s con i dischi.

    L'uscita dopo 2 minuti senza client rilascia tutto.
  - Non espone rete né comandi di scrittura verso l'hardware.
  - **Avvio manuale, avviato dall'app** (decisione M4). Il servizio non parte con Windows. L'installer, l'unico passaggio elevato, aggiunge al descrittore di sicurezza del servizio un ACE che concede agli utenti interattivi (`IU`) **solo** l'avvio e l'arresto (`RP`, `WP`). Non concede mai `SERVICE_CHANGE_CONFIG`, `WRITE_DAC` o `WRITE_OWNER`, che permetterebbero di ottenere privilegi di amministratore. È lo stesso schema di EasyAntiCheat_EOS e dei servizi Epic, che anzi concedono avvio e arresto a chiunque.
  - L'app, all'apertura, avvia il servizio se è installato e la modalità anti-cheat è spenta, poi si collega. Senza client per 2 minuti il servizio si ferma da solo con codice d'uscita 0: Windows lo vede come un arresto pulito e non applica le azioni di ripristino. Chiusa l'app, quindi, non resta nulla in esecuzione. **Precisato in M4:** l'avvio automatico scatta solo se la **prima** risposta conclusiva dell'SCM all'apertura dell'app è "fermo"; `NotInstalled`, `AccessDenied`, `StopPending` o un errore transitorio in quel momento non portano mai a un avvio automatico successivo (l'app mostra invece "Avvia"). Mentre l'app è scollegata dalla pipe, il badge aggiorna il motivo mostrato con nuove interrogazioni all'SCM, ma non avvia mai il servizio da solo.
  - Limite accettato: su un PC con più utenti collegati, uno può fermare il servizio anche per gli altri.
- **Modalità compatibile anti-cheat:** l'app ferma il servizio e non lo riavvia finché la modalità resta attiva; lo stato persiste tra i riavvii (in M4 in un file in `%LOCALAPPDATA%\OpenMonitorAdvanced\`, dalla M5 in `settings.json`). Si attiva dal menu della tray (§4.5); si disattiva dalla tray o dal badge della barra superiore (§7.1). **Precisazione M4:** attivare la modalità significa "questa app non si connetterà né avvierà il servizio"; l'arresto viene richiesto subito e confermato con interrogazioni all'SCM (dettaglio `stopping` finché non arriva), con timeout di 30 s (`stopFailed` oltre quel limite). La preferenza non contende il servizio ad altri client o utenti: non lo ferma per loro conto né lo tiene fermo se qualcun altro lo riavvia.
  - Il driver PawnIO **resta caricato**: è un device Plug and Play (`ROOT\PAWNIO\0000`) che Windows carica all'avvio, indipendentemente da chi lo usa, e non si scarica quando si chiude l'ultimo handle. Toglierlo richiede privilegi di amministratore e disturberebbe altri programmi che lo usano (per esempio FanControl), quindi l'app non lo fa. Fermare il servizio chiude comunque ogni handle verso PawnIO e ogni processo del progetto con privilegi, cioè ciò che un anti-cheat euristico può notare.
  - Il blocco di FACEIT su PawnIO dipendeva dal certificato di firma delle versioni precedenti alla 2.1.0, non dall'uso del driver: con PawnIO 2.2.0 (firmato da Microsoft) FACEIT lo accetta. Per questo l'installer aggiorna PawnIO se è più vecchio della 2.2.0 (§10). Per Vanguard, EAC e BattlEye non risultano blocchi di PawnIO (ricerca di settembre 2026).

### 2.3 Predisposizione per Linux

I provider implementano un trait `Provider`. Su Linux si aggiungerà un provider per hwmon, sysfs, powercap e DRM fdinfo; quasi tutto è leggibile senza root. Nucleo, protocollo e interfaccia restano invariati, e su Linux il servizio .NET non serve. Tutte le parti specifiche di Windows restano nel crate `oma-win`.

## 3. Modello dati

- **`Device`**: `id` stabile, `kind` (`cpu`, `gpu`, `memory`, `storage`, `network`, `motherboard`, `battery`, `fan_controller`, `psu`), `name`, `vendor`, proprietà statiche (modello, driver, capacità…).
- **`Sensor`**: `id` stabile, `device_id`, `kind` (`temperature`, `load`, `clock`, `power`, `voltage`, `current`, `fan`, `data`, `throughput`, `energy`, `flag`, `percent`, `link`, `counter`; `counter` con le unità `hours` e `count` arriva in M4 per le ore e i cicli di accensione dei dischi), `unit`, `label` (chiave i18n oppure testo fornito dal driver), `source` (`pdh`, `win32`, `dxgi`, `d3dkmt`, `nvml`, `nvapi`, `adl`, `igcl`, `pnp`, `lhm`, `wmi`…), `category` (per il raggruppamento nella tabella della vista Avanzata), `experimental` (vero per le letture ottenute da chiamate non documentate, vedi §5.2: l'interfaccia le segna come sperimentali).
- **`Reading`**: `sensor_id`, `value: f64` oppure assente, `timestamp`.
- **ID stabili tra riavvii**, costruiti dall'identità hardware. Esempi: `gpu/pci-0000:01:00.0/temperature/hotspot`, `storage/device-<hash>/temperature/drive`. Regole, impostazioni e selezione dei sensori per il log vi fanno riferimento. Per i dischi l'identità segue una catena di ripiego (§5.1).
- **Merge:**
  - quando più fonti forniscono lo stesso sensore logico, vince quella con priorità più alta (priorità per campo, vedi §5.2);
  - la fonte scelta viene registrata e mostrata nell'interfaccia;
  - la scelta segue la fonte, mai il nome del vendor;
  - **tra provider** (M4): se due provider espongono lo stesso ID di sensore vince quello che viene prima nell'elenco dei provider, e il sensore dell'altro si scarta. Il provider `svc` è l'ultimo, quindi sui doppioni vince sempre il nucleo senza privilegi e il servizio aggiunge solo ciò che manca: quando il servizio parte o si ferma, i sensori che esistevano già non cambiano fonte e il loro storico non si azzera. I device con lo stesso ID si fondono: nome e vendor del primo provider, proprietà unite con precedenza al primo.

## 4. Nucleo `oma-core`

### 4.1 Scheduler

- Un solo timer coalescente, di default 1 s, configurabile da 0,5 a 5 s. **Non modifica mai la risoluzione del timer di sistema.**
- Ogni provider viene interrogato in parallelo con un tempo massimo. Se lo supera, per quel ciclo si usano gli ultimi valori.
- Frequenze per categoria:
  - dati dinamici: a ogni ciclo;
  - SMART e salute dei dischi: ogni 30 s;
  - dati statici (SMBIOS, modelli, driver): all'avvio e quando cambia l'hardware.
- I contatori cumulativi (energia, tempo di attività dei motori GPU) richiedono due campioni: il primo è solo la base. Il ritorno a zero del contatore viene gestito.

### 4.2 Storico

- Un ring buffer per sensore, **1 ora di campioni** alla frequenza corrente, con i timestamp condivisi. Con circa 300 sensori a 1 s sono circa 5 MB.
- **Min, max e media** si calcolano nel nucleo (`oma-core`), per ogni sensore, dal primo ciclo dopo l'avvio dell'app, e non nell'interfaccia: la WebView viene distrutta quando si chiude la finestra (§2.2), mentre le statistiche devono coprire anche il tempo passato nella tray. I valori assenti non contano.
  - Il pulsante "azzera" di una pagina della vista Avanzata azzera le statistiche dei soli sensori di quella pagina.
  - Quando cambia lo schema, le statistiche di un sensore restano solo se ID, fonte e unità sono invariati, come lo storico.
- L'istante del primo ciclo (`startedAtMs`) è esposto all'interfaccia: il banner "monitoraggio attivo da…" conta da lì anche dopo aver riaperto la finestra dalla tray.
- **Storico inviato all'interfaccia:** le finestre da 1 e 5 minuti arrivano con tutti i campioni. Per 30 minuti e 1 ora lo storico arriva decimato ad al massimo 900 punti per serie, in intervalli bilanciati le cui dimensioni differiscono al massimo di un campione. Ogni intervallo interamente valido dà due punti, minimo e massimo, così i picchi restano visibili; i timestamp sono i confini dell'inviluppo, non gli istanti reali degli estremi. Se un intervallo contiene un valore assente, per quella serie entrambi i punti sono assenti: i buchi si ampliano conservativamente all'intervallo. La decimazione serve al budget di memoria della finestra: in una prova, 20 serie da 3600 punti portavano il totale a 219 MB.
- I periodi in cui una fonte non è disponibile sono registrati come valori assenti e appaiono come buchi nei grafici.

### 4.3 Motore regole (banner di stato e notifiche)

Un **unico motore** alimenta sia il banner di stato della vista Semplificata sia le notifiche, così i due non si contraddicono mai. Modello, valutazione, uscite e regole predefinite definitive sono nella spec di dettaglio della M5 (`docs/superpowers/specs/2026-09-29-m5-regole-integrazione-design.md`, §3), che ha la precedenza su questa sezione.

- **Regola:**
  - `sensor` oppure un selettore, per esempio "temperatura core di ogni GPU";
  - condizione (`>`, `<`, flag attivo);
  - soglia **attenzione** e soglia **critico**;
  - **durata minima** prima dell'attivazione;
  - **isteresi** per il rientro, di default 3 unità sotto la soglia per 10 s;
  - `attiva`;
  - `notifica` (toast di Windows) per livello.
- **Stati:** `ok`, `attenzione`, `critico`. Il banner mostra la regola attiva più grave, con un messaggio localizzato (per esempio "GPU surriscaldata (92 °C)"). Se le regole attive sono più di una mostra "N problemi", con l'elenco a tendina, e indica da quanto tempo dura lo stato.
- **L'utente può modificare ogni regola predefinita** (soglie, durata, attiva sì/no) e **creare regole personalizzate** su qualsiasi sensore: "sensore X > valore per N secondi".

**Regole predefinite:**

| Regola | Attenzione | Critico | Durata (att. / crit.) | Note |
|---|---|---|---|---|
| Temperatura CPU | ≥ TjMax − 10 °C | throttling termico attivo, oppure ≥ TjMax | 30 s / 10 s | Relativa al TjMax, perché i Ryzen 7000/9000 lavorano normalmente a 95 °C. Se TjMax è ignoto: 85 / 95 °C. Richiede il servizio. |
| Temperatura GPU (core) | ≥ 83 °C | ≥ 90 °C | 30 s / 10 s | |
| GPU hotspot | ≥ 95 °C | ≥ 105 °C | 30 s / 10 s | Solo se il sensore esiste. |
| Temperatura memoria GPU (junction) | ≥ 100 °C | ≥ 105 °C | 30 s / 10 s | GDDR6X e simili. |
| Throttling termico GPU | attivo | — | 10 s | |
| Temperatura SSD NVMe | ≥ soglia "warning" del disco (WCTEMP; 70 °C se assente) | ≥ soglia "critical" del disco (CCTEMP; 80 °C se assente) | 30 s | |
| Salute disco | usura ≥ 90% | SMART critical warning attivo | — | Richiede il servizio. |
| Spazio libero su un volume | < 10% | < 3% | — | |
| RAM usata | ≥ 90% | ≥ 97% | 60 s / 30 s | |
| Batteria in scarica | ≤ 15% | ≤ 5% | — | Solo portatili. |

**Precisazioni M5:** la CPU ha due regole (temperatura, e throttling termico come critico); TjMax dei Ryzen viene da una tabella per famiglia nel servizio; la temperatura vale per ogni disco con il sensore, non solo per gli NVMe; lo spazio libero si esprime come percentuale usata (≥ 90% / ≥ 97%). Le notifiche di default partono solo per il livello critico.

**Nessuna regola predefinita sulle ventole ferme.** Molte GPU e alcuni case spengono le ventole di proposito a basso carico. L'utente può creare regole sulle ventole a mano.

### 4.4 Logger CSV

- Si avvia e si ferma dall'interfaccia (un registratore in stile nastro nella barra superiore, con REC, pausa e stop), dalla tray o da una scorciatoia globale (di default Ctrl+Alt+Shift+R). Dettagli nella spec della M5 (`docs/superpowers/specs/2026-09-29-m5-regole-integrazione-design.md`, §4).
- Si sceglie quali sensori registrare (di default tutti) e con quale intervallo (di default quello dello scheduler).
- **Formato:**
  - un file per sessione in `Documenti\OpenMonitor Advanced\logs\` (cartella configurabile);
  - prima colonna: timestamp ISO 8601 in ora locale con offset;
  - intestazione `Dispositivo / Sensore [unità]`;
  - separatore `,`, punto decimale `.`, codifica UTF-8 con BOM, per la compatibilità con Excel.
- Oltre una dimensione massima configurabile (di default 100 MB) il file viene diviso in parti numerate.

### 4.5 Tray

- **Icona dinamica:**
  - mostra il valore di un sensore scelto dall'utente, di default la temperatura core della GPU principale, oppure della CPU se non c'è una GPU dedicata;
  - il colore segue lo stato del motore regole.
- **Tooltip:** i valori chiave (CPU, GPU, RAM).
- **Menu:** Apri, vista Semplificata/Avanzata, Avvia/Ferma log, Modalità compatibile anti-cheat, Esci. La voce anti-cheat (una casella) arriva in M4; il resto del menu completo in M5 (menu, icona e tooltip definitivi: `docs/superpowers/specs/2026-09-29-m5-regole-integrazione-design.md`, §2.6).
- **Comportamento:**
  - chiudere la finestra la riduce nella tray (disattivabile);
  - avvio automatico con Windows opzionale;
  - una sola istanza alla volta.

### 4.6 Impostazioni

File `%APPDATA%\OpenMonitorAdvanced\settings.json`, con versione dello schema e migrazioni (formato definitivo: `docs/superpowers/specs/2026-09-29-m5-regole-integrazione-design.md`, §2). Contiene:
- vista predefinita, lingua, unità (°C/°F, bit/s o byte/s), intervallo di aggiornamento;
- regole;
- sensori selezionati per il log e per la tray;
- interruttori per ogni provider;
- comportamento della tray e avvio automatico;
- controllo opzionale degli aggiornamenti (dalla M6).

## 5. Acquisizione dati (Windows)

### 5.1 Provider `sys` (senza privilegi)

| Area | Fonte |
|---|---|
| Carico CPU totale e per core | PDH `Processor Information(*)\% Processor Utility` (come Task Manager su Windows 10/11); se il contatore non è disponibile, `% Processor Time` |
| Clock CPU (stima) | PDH `% Processor Performance` × frequenza base, lo stesso metodo di Task Manager. Il clock effettivo per core arriva dal servizio. |
| RAM | `GlobalMemoryStatusEx`, `GetPerformanceInfo`; SMBIOS tipo 17 e `Win32_PhysicalMemory` per velocità e moduli |
| Dischi: spazio e throughput | `GetDiskFreeSpaceEx`; PDH `PhysicalDisk(*)` (letture e scritture al secondo, byte/s, % tempo attivo) |
| Dischi: identità | Catena di ripiego, dal livello più forte: numero di serie (`storage/device-…`, invariato dalla M1; un seriale non UTF-8 si usa come byte grezzi) → GUID del disco GPT (`storage/gpt-…`, da `IOCTL_DISK_GET_DRIVE_LAYOUT_EX`) → firma MBR più dimensione del disco (`storage/mbr-…`) → instance id PnP (`storage/pnp-…`, da SetupAPI; legato alla porta, cambia se il disco viene spostato). Ogni valore entra nell'ID solo come hash SHA-256. Un livello si usa solo se il suo valore è unico tra i dischi della macchina, perché i cloni copiano GUID e firma. Un disco si omette solo se falliscono tutti i livelli, con un avviso nel log che indica il motivo per ciascuno. Tutte le chiamate usano `\\.\PhysicalDriveN` aperto con accesso 0, senza privilegi. |
| Temperatura dei dischi | `IOCTL_STORAGE_QUERY_PROPERTY` con `StorageDeviceTemperatureProperty`, senza privilegi (verificato in M3 su NVMe e SATA). Il supporto dipende da disco e driver: un disco senza supporto risponde `ERROR_INVALID_FUNCTION` e non ha sensori. Il sensore 0 è la temperatura del disco (la "composite" degli NVMe, `…/temperature/drive`), gli altri sono sensori aggiuntivi (`…/temperature/sensor-<n>`); `0x8000` significa "non riportato". Le soglie warning e critical diventano proprietà del device (`tempWarningC`, `tempCriticalC`) per le regole della M5. Lettura ogni 30 s (§4.1), al massimo un disco per ciclo (fino a circa 140 ms per un NVMe che esce da uno stato a basso consumo). Un disco che Windows segnala in standby (`GetDevicePowerState`) non viene interrogato; questa funzione vede solo gli stati D gestiti dal sistema operativo, non lo standby ATA interno del disco, e non è ancora verificato su hardware se l'interrogazione periodica impedisca a un HDD inattivo di fermarsi (o resetti il timer di inattività di Windows). |
| Rete | `GetIfTable2` (byte, velocità del collegamento), con filtro sulle interfacce fisiche e attive |
| Wi-Fi | `WlanQueryInterface` con `wlan_intf_opcode_rssi`. SSID e qualità della connessione richiedono il consenso alla posizione su Windows 11 24H2 o successivi: se negato vengono omessi. |
| Batteria | `GetSystemPowerStatus`, `IOCTL_BATTERY_QUERY_INFORMATION/STATUS` (capacità di progetto e attuale, cicli, potenza) |
| Sistema | `GetSystemFirmwareTable('RSMB')` per SMBIOS (scheda madre, BIOS); `RtlGetVersion` per la versione di Windows |

### 5.2 Provider `gpu` (senza privilegi, a strati)

Verificato in M2 da utente normale su una RTX 4080 (driver 617.14) e sull'iGPU AMD Raphael (RDNA2).

1. **Enumerazione:**
   - DXGI `EnumAdapters1`, scartando gli adattatori software (`DXGI_ADAPTER_FLAG_SOFTWARE`, per esempio Microsoft Basic Render Driver). Ogni GPU ha un solo record, identificato dal LUID.
   - L'indirizzo PCI viene da D3DKMT `ADAPTERADDRESS` (bus `0xFFFFFFFF` = nessun indirizzo). Il dominio PCI non è noto e vale sempre `0000`. ID del device: `gpu/pci-0000:01:00.0`; senza indirizzo PCI, `gpu/ven-<vendor>-dev-<device>-<n>`.
   - **Integrata o dedicata:** DXCore `IsIntegrated`, letto solo se `IsPropertySupported` lo conferma; in alternativa il bit `HybridIntegrated` (bit 5) di D3DKMT `ADAPTERTYPE`; altrimenti la GPU è considerata dedicata. `DedicatedVideoMemory` non basta a distinguerle. Il risultato è la proprietà `integrated` del device.
2. **Livello base, per ogni vendor, sempre attivo:**
   - **PDH `GPU Engine(*)\Utilization Percentage`:** carico per motore (3D, compute, copy, video decode, video encode).
     - Il tipo di motore è la parte del nome dell'istanza dopo `_engtype_` e può contenere spazi (`Video Codec 0`).
     - Si sommano i processi per LUID e motore. Il carico core è il massimo tra i motori; il carico di un tipo è il massimo tra i motori di quel tipo, come in Task Manager.
     - Il contatore è un tasso: il primo campione dopo la discovery serve solo da base.
     - Lo stesso contatore dà l'utilizzo per processo (M3, punto 5).
   - **PDH `GPU Adapter Memory(*)`:** memoria dedicata e condivisa in uso. `GPU Process Memory(*)` dà la memoria per processo (M3, punto 5).
   - **D3DKMT `KMTQAITYPE_ADAPTERPERFDATA` (62):**
     - temperatura in decimi di °C;
     - potenza in decimi di % del limite (non in watt);
     - frequenza della memoria in Hz;
     - ventola in RPM, solo se `ADAPTERPERFDATA_CAPS` riporta un `MaxFanRPM` maggiore di zero: in quel caso 0 RPM è un valore vero (ventola ferma).

     Il driver AMD popola la struttura anche per l'iGPU (temperatura a passi di 1 °C, potenza %, frequenza della DRAM). Potenza % e frequenza della memoria si usano solo per le GPU dedicate.
   - **D3DKMT `KMTQAITYPE_NODEPERFDATA` (61):** il clock core è la `Frequency` del nodo 0 e coincide con il clock grafico di `nvidia-smi`. La struttura va passata con la dimensione esatta (56 byte), altrimenti la chiamata restituisce `STATUS_INVALID_PARAMETER`. La tensione del nodo non si usa: vale 0 su NVIDIA.
   - **DXGI:** memoria dedicata totale (`DedicatedVideoMemory`), solo per le GPU dedicate.
   - **`QueryVideoMemoryInfo` di DXGI è escluso:** riporta solo la memoria del processo chiamante.
   - Un `NTSTATUS` negativo da D3DKMT durante il campionamento avvia una nuova discovery. Succede, per esempio, dopo un aggiornamento del driver o un TDR, quando l'handle non è più valido.
3. **Arricchimento con le librerie dei vendor**, tutte caricate dinamicamente con `LoadLibraryExW(..., LOAD_LIBRARY_SEARCH_SYSTEM32)` e con i simboli risolti uno alla volta. Un simbolo mancante o `NOT_SUPPORTED` significa "sensore assente".

   **Ogni libreria si carica una sola volta e resta caricata fino alla fine del processo:**
   - ADL non restituisce la memoria quando viene scaricata;
   - dopo `NvAPI_Unload` i puntatori di NVAPI restano pendenti (crash verificato);
   - `nvmlShutdown` riscrive circa 18 MB di pagine.
   - **NVIDIA — NVML (`nvml.dll`):**
     - **Sensori:** temperatura core; clock core e memoria; potenza della scheda e limite applicato; potenza in % del limite (può superare 100); ventola in % e in RPM; VRAM usata e totale; throttling da `ClocksEventReasons` (per potenza con i bit `0x4`, `0x80`; termico con `0x20`, `0x40`). Il bit generico `HwSlowdown` (`0x8`) da solo non identifica una causa termica.
     - **Chiamate escluse dal campionamento:** `TotalEnergyConsumption` (p95 8 ms) e `PcieThroughput` (blocca per 30 ms).
     - **Memoria:** `nvmlInit_v2` scrive i 19,4 MB della sezione `.data` della `nvml.dll` del DriverStore e li lascia nel working set privato. Subito dopo l'inizializzazione l'app chiama `VirtualUnlock` su quella sezione (`ERROR_NOT_LOCKED` è l'esito atteso), e l'aumento scende da +19,3 MB a +0,5 MB. Se la sezione non si trova, l'app lo registra nel log e prosegue.
   - **NVIDIA — NVAPI (`nvapi64.dll`):** temperature hotspot e memory junction (`GPU_ThermalGetSensors`, valori divisi per 256, maschera dei sensori scelta provando la più ampia accettata) e tensione core (`GPU_ClientVoltRailsGetStatus`, in µV). Sono chiamate non documentate, quindi i tre sensori sono **sperimentali** (stesso approccio di LibreHardwareMonitor) e l'interfaccia li segna come tali. Gli indici dei sensori dipendono dall'architettura, ricavata dal device ID PCI:

     | Architettura | Device ID | Hotspot | Junction |
     |---|---|---|---|
     | Turing / Ampere | `0x1E00`–`0x25FF` | 1 | 9 (solo con maschera ≥ `0x3FF`) |
     | Ada | `0x2680`–`0x28FF` | 1 | 7 |
     | Blackwell | `0x2B80`–`0x2FFF` | — | 2 |

     Con un'architettura sconosciuta non c'è nessuno dei tre sensori. Gli RPM della ventola vengono da NVML: `GetTachReading` restituisce `NOT_SUPPORTED` sulla RTX 4080.
   - **AMD — ADL (legacy, `atiadlxx.dll`):**
     - **Lettura:** PMLog in memoria condivisa (`Overdrive8_PMLog_ShareMemory`, circa 3 µs per lettura); in alternativa `New_QueryPMLogData_Get`.
     - **Associazione:** l'elenco degli adattatori di ADL ha un record per ogni uscita video e comprende anche le GPU di altri vendor. Si tengono i record con `iVendorID == 1002`, si deduplicano per bus/device/function e si associano all'indirizzo PCI.
     - **iGPU:** temperatura (`TEMP_GFX`) e clock core (`GFXCLK`). `GFX_POWER`, `ASIC_POWER` e `GFX_VOLTAGE` seguono la CPU sulle APU desktop e non si usano.
     - **GPU dedicate** (non ancora verificate su hardware): temperature core, hotspot e memoria; clock core e memoria; potenza della scheda (`BOARD_POWER`, altrimenti `ASIC_POWER`); ventola in RPM e in %; tensione core.
     - **ADLX è escluso**, perché la sua licenza vieta l'uso in software con licenza libera.
   - **Intel — IGCL (`ControlLib.dll`, solo a 64 bit):**
     - **Sensori:** con `ctlPowerTelemetryGet`, temperature di GPU e VRAM, clock, tensione, ventola, potenza (calcolata dalla variazione del contatore di energia tra due campioni) e flag dei limiti di potenza e temperatura, usati come motivi del throttling.
     - **Versioni:** struttura di telemetria da 1024 byte (versione 1), con ripiego a 808 byte (versione 0) per i runtime più vecchi.
     - **Stato:** implementato e coperto da test con funzioni finte, **non ancora verificato su hardware Intel**. Dalla M3 legge anche il link PCIe corrente (`ctlPciGetState` per generazione e larghezza correnti); i massimi non si leggono da IGCL, ma dal livello base PnP (punto 5), identico su ogni vendor.
4. **Priorità nel merge**, per ogni GPU e per ogni campo: NVML → NVAPI → ADL → IGCL → D3DKMT → DXGI → PDH.
   - La fonte si sceglie alla discovery: è il livello con la priorità più alta che dichiara quel campo per quella GPU. Viene registrata nel sensore (`source`).
   - Se in un ciclo la fonte scelta non dà un valore, il valore è assente. Non si ripiega su un'altra fonte a runtime. Se una successiva discovery cambia fonte o unità, si azzera lo storico del solo sensore interessato, senza cambiarne l'ID.
   - Ogni 5 s si verifica anche la topologia, compreso il caso di zero adattatori: una GPU riconnessa viene scoperta anche quando non esistono più handle da invalidare. A topologia invariata non si riattaccano i livelli.
   - Il carico per motore viene sempre da PDH.
   - Le proprietà statiche del device (punto 5) seguono la stessa priorità: per ogni chiave vince il livello più prioritario che la fornisce. PnP fornisce solo proprietà, nessun sensore. `pciAddress` e `integrated` vengono dall'enumerazione e nessun livello li sovrascrive.
5. **Aggiunte della M3** (vista Avanzata):
   - **Utilizzo per processo**, dal livello PDH, che già legge `GPU Engine(*)`, più `GPU Process Memory(*)` (`Dedicated Usage`, `Shared Usage`; circa +50 µs per ciclo).
     - Il carico di un processo è quello del suo motore più occupato (massimo tra i motori, limitato a 0–100), coerente con il carico core dell'adattatore. Il tipo di motore si indica solo se il carico è maggiore di zero.
     - I nomi dei processi vengono da `CreateToolhelp32Snapshot` (documentata e senza aprire i processi; circa 2,4 ms), al massimo una volta per ciclo e solo quando compare un pid sconosciuto, con una cache che scarta i pid spariti. Non si usano `OpenProcess` (fallisce per dwm, System e i servizi) né la chiamata non documentata `NtQuerySystemInformation` (88). Pid 0 = "Idle", pid 4 = "System".
     - Non sono sensori: niente ID né storico. La vista Avanzata legge l'elenco con il comando `get_gpu_processes` (al massimo 20 righe, per carico e poi per memoria dedicata). Il primo campione dopo un attach non ha il carico, perché il contatore è un tasso.
     - La tabella nasconde le sue righe se il provider non ha pubblicato un aggiornamento da più di 2,5 s (provider bloccato), oltre ad azzerarle sugli errori del provider.
   - **Encoder e decoder da NVML** (`EncoderUtilization`, `DecoderUtilization`): campi nuovi, solo NVML, distinti dal carico dei motori video di PDH, che resta solo PDH.
   - **Link PCIe:** generazione e larghezza correnti come sensori (tipo `link`, unità `pcie_generation` e `lanes`) da NVML e IGCL. Generazione e larghezza massime sono proprietà del device (`pcieMaxGen`, `pcieMaxWidth`), lette **solo** dal nuovo livello base **PnP** (`CM_Get_DevNode_PropertyW`, proprietà PCI del device, letta alla discovery), identico per ogni vendor perché è una capacità del device, non del driver: `nvmlDeviceGetMaxPcieLinkGeneration/Width` di NVML riportano il valore limitato da device e slot insieme, non il solo massimo del device, e non si leggono; IGCL non legge `ctlPciGetProperties` per lo stesso motivo. Il valore "corrente" di PnP non si usa: viene fissato all'avvio del device e non si aggiorna.
   - **Limiti statici come proprietà** del device, letti una volta all'attach da NVML: limite di potenza minimo, massimo e predefinito (`powerLimitMinW`, `powerLimitMaxW`, `powerLimitDefaultW`) e soglie di temperatura slowdown, shutdown e massima (`tempSlowdownC`, `tempShutdownC`, `tempMaxC`). Servono alla vista Avanzata e alle regole della M5.
   - **Indirizzo PCI conservato per LUID:** se un'enumerazione successiva non riporta l'indirizzo di un adattatore, resta quello già noto, così l'ID del device non cambia.

   **Esclusi (decisione D10 del piano M3):**
   - NVML `TotalEnergyConsumption` (p95 circa 9 ms, e nessun uso prima del log CSV e delle regole della M5) e `PcieThroughput` (blocca per 31 ms);
   - gli elenchi dei processi di NVML: `usedGpuMemory` non è disponibile sotto WDDM;
   - i sensori ADL 40 e 41 (valori costanti, unità non documentata).

**Licenze:**
- Nessun header proprietario è incluso nel repository e nessuno viene scaricato durante la build.
- **NVAPI:** ID e strutture vengono dagli header dell'SDK (MIT); l'avviso è in `THIRD_PARTY_NOTICES.md`. Gli ID non documentati (`0x65FE3AAD`, `0x465F9BCF`) sono fatti di interoperabilità documentati da LibreHardwareMonitor, citato come fonte. Il suo codice non viene copiato.
- **NVML, ADL e IGCL:** binding scritti a mano dalla documentazione pubblica (nomi dei simboli, layout, costanti) e verificati sulle librerie installate. Per ADL è l'unica via compatibile, perché l'EULA degli header AMD esclude la GPL.
- Le DLL dei vendor non vengono mai ridistribuite: si caricano solo da `System32`, dove le installa il driver.

### 5.3 Servizio `oma-service`

- **.NET 10**, Worker Service ospitato come servizio Windows, **LibreHardwareMonitorLib** (NuGet, MPL-2.0).
- **Moduli attivi:** `Cpu`, `Motherboard`, `Memory` (SPD tramite PawnIO SMBus), `Storage` (SMART/NVMe), `Controller` (controller ventole e RGB USB), `Psu`.
- **Moduli spenti:** `Gpu`, `Network`, `Battery`, già coperti dal nucleo Rust.
- **Conversione:** l'albero Hardware/Sensor di LibreHardwareMonitor viene tradotto nel modello di §3 dal servizio stesso.
  - **Nomi canonici:** i sensori che misurano una grandezza che il nucleo già conosce ricevono lo stesso nome del nucleo (per esempio il carico totale della CPU diventa `…/load/total`, la temperatura di un disco `…/temperature/drive`), così il merge tra provider (§3) riconosce i doppioni. Gli altri ricevono un nome stabile derivato dall'identificatore del sensore di LibreHardwareMonitor.
  - **Etichette:** i sensori noti usano chiavi i18n; gli altri usano la chiave `lhm.raw` con il testo di LibreHardwareMonitor come argomento, mostrato così com'è. Sul protocollo ogni sensore porta `label_key` e `label_arg`.
  - **Aggancio ai device del nucleo:** per ogni hardware lo `Schema` porta un **indizio d'identità**:
    - per la CPU, l'indice;
    - per i dischi, il numero `PhysicalDriveN` con modello e seriale del descrittore `STORAGE_DEVICE_DESCRIPTOR`, letti dal servizio come li legge il nucleo. Il seriale che LibreHardwareMonitor legge dal disco è diverso da quello del descrittore sugli NVMe (verificato in M4), e Rust aggancia solo se modello **e** seriale del descrittore sono entrambi presenti, coincidono e sono univoci: due dischi entrambi senza seriale (`None == None`) non contano come identici, quindi non si agganciano tra loro né aggancia un disco a un device del nucleo che non ha un proprio seriale univoco (deciso in M4, per non fondere per errore dischi diversi privi di seriale);
    - per la RAM, nessuno: c'è un solo device. Il provider `svc` usa l'indizio per dare al device l'ID che il nucleo usa già (`cpu/…`, `memory/…`, `storage/…`), così i sensori del servizio compaiono nelle pagine esistenti. Scheda madre, controller delle ventole e alimentatori, e ogni hardware senza un device corrispondente nel nucleo, diventano device propri con ID `<kind>/lhm-<hash>`. L'hash è lo SHA-256 dell'identificatore hardware di LibreHardwareMonitor; per i dischi è quello di modello e seriale, così l'ID non cambia se Windows rinumera i dischi.
  - **Senza PawnIO** (driver assente o non accessibile) CPU, Super I/O e moduli RAM non si pubblicano, perché LibreHardwareMonitor restituirebbe valori finti, come 0 W, 0 °C e una VID fissa.
  - **Storico interno:** il servizio lo spegne su ogni sensore (`ValuesTimeWindow = 0`); quello predefinito di un giorno crescerebbe di decine di MB.
  - La fonte dei sensori del servizio è `lhm`.
- **Frequenze:** dati dinamici all'intervallo della sottoscrizione più rapida; SMART e salute dei dischi ogni 30 s (§4.1). **D6, deciso nello spike M4:** prima di ogni `Update` dello storage, ogni disco potenzialmente rotazionale deve confermare con `ATA CHECK POWER MODE` di essere attivo (il valore `0x40`, spindle down con cache NV, conta come attivo per i dischi ibridi obsoleti); NVMe, dischi virtuali (bus `0xE`/`0xF`), Storage Spaces (`0x10`), lettori di schede vuoti e dischi con `seek-penalty=false` non richiedono la conferma. Se anche un solo disco non riesce a confermare, lo SMART resta spento per **tutti** i dischi in quel giro (filtro per-disco, mai una risposta transitoria messa in cache). Limite accettato: un disco USB il cui bridge rifiuta il pass-through ATA non ha mai lo SMART di LibreHardwareMonitor (interruttore per singolo modulo, M5; `docs/follow-ups.md`). Non ancora verificato su hardware reale (Task 15). Un errore su un singolo hardware rende assenti solo i suoi sensori e finisce nel log.
- **Build:** self-contained, file singolo, win-x64. Ridotta (trimmed) solo se i sensori letti sono identici con e senza riduzione: LibreHardwareMonitorLib si dichiara compatibile ma sopprime gli avvisi di trimming sui percorsi WMI. Senza riduzione il servizio aggiunge circa 23 MB compressi all'installer.
- **Log:** file proprio a rotazione in `<cartella del servizio>\logs`, cioè `$INSTDIR\service\logs` sotto Program Files (7 file al massimo). **Precisazioni della revisione finale di M4:**
  - all'inizio i log stavano in `%ProgramData%\OpenMonitorAdvanced\logs`, ma chiunque può creare cartelle in `C:\ProgramData`: una cartella creata prima da un utente (sua, o una giunzione) dirotterebbe le scritture e le cancellazioni del servizio, che gira come SYSTEM, e neanche l'installer la mette al sicuro, perché l'utente può tenere aperto un handle su quella cartella durante l'installazione. **Decisione R30 (sostituisce R27):** i log stanno nella cartella del servizio, che non ha un genitore creabile dagli utenti, quindi nessuno può piazzarla prima;
  - l'installer protegge la cartella del servizio come prima (proprietario Administrators, niente ereditarietà, controllo completo a SYSTEM e Administrators, lettura agli Users, anche per allegare i log a una segnalazione senza elevazione). In un aggiornamento conserva la sottocartella `logs` reale con i suoi file e la riporta all'ACL ereditato; una giunzione o un collegamento al suo posto viene rimosso come collegamento;
  - il servizio ricontrolla prima della prima scrittura di ogni giorno la propria cartella e `logs`: nessun punto di reparse, proprietario SYSTEM o Administrators, nessun diritto di scrittura o cancellazione ad altri. Se il controllo fallisce (per esempio un avvio di sviluppo da una cartella `bin` dell'utente) rinuncia al log su file e lo segnala una volta (console, oppure registro eventi Applicazione). Se manca `logs`, lo crea con lo stesso ACL esplicito;
  - la disinstallazione e la deselezione del componente eliminano i log insieme al servizio; un aggiornamento li conserva (R30, al posto di R27, che li lasciava in `%ProgramData%`).

## 6. Protocollo IPC

- **Trasporto:** named pipe `\\.\pipe\OpenMonitorAdvanced.Sensors.v1`, creata dal servizio con `FILE_FLAG_FIRST_PIPE_INSTANCE` e un descrittore di sicurezza esplicito:
  - SYSTEM e Administrators: controllo completo;
  - utenti interattivi: lettura e scrittura dei messaggi, **senza** il diritto di creare nuove istanze della pipe. `GRGW` non va bene, perché comprende `FILE_APPEND_DATA`, che su una pipe significa "crea un'istanza": un utente potrebbe affiancare un proprio server a quello del servizio. Il descrittore è `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012019b;;;IU)`, cioè `ReadWrite | Synchronize` (verificato in M4).

  La pipe si crea con `CreateNamedPipeW` (P/Invoke) e `PIPE_REJECT_REMOTE_CLIENTS`: la funzione .NET `NamedPipeServerStreamAcl.Create` non rifiuta i client remoti. Il servizio crea la successiva istanza in ascolto prima di passare quella connessa al suo gestore, così il nome della pipe non resta mai libero tra una connessione e l'altra. Il client si connette con `SECURITY_IDENTIFICATION` e legge con I/O overlapped, così il thread di lettura si ferma senza blocchi (`CancelIoEx` e poi `GetOverlappedResult`). **Precisato in M4:** il limite è di al massimo 8 istanze della pipe **in tutto**, istanza in ascolto compresa, non 8 client oltre ad essa; l'ottavo client che si connette ottiene comunque la sua sessione subito, senza attese.
- **Verifica del server (M4):** mentre il servizio è fermo un altro processo potrebbe creare una pipe con lo stesso nome. Dopo la connessione il client confronta il PID del server della pipe (`GetNamedPipeServerProcessId`) con il PID del servizio registrato (`QueryServiceStatusEx`); se non coincidono si scollega e lo registra nel log. Se invece è il servizio stesso a trovare il nome della pipe già preso all'avvio, esce con codice 1 tramite un arresto ordinato, senza lasciare che l'SCM lo riavvii.
- **Frame:** lunghezza `u32` little-endian seguita dal payload **MessagePack**. Dimensione massima di 4 MB; un frame più grande chiude la connessione.
- **Codifica:** ogni messaggio è una mappa con chiavi stringa in un ordine fisso, interi nella forma più corta, numeri reali sempre `float64`, valori assenti `nil`. Così Rust (`rmp-serde`) e .NET (MessagePack-CSharp) producono gli stessi byte.
- **Messaggi:**
  - `Hello { protocol_version, service_version }` (servizio → client); se la versione del protocollo non è compatibile, l'interfaccia chiede di aggiornare il servizio;
  - `Subscribe { interval_ms }` (client → servizio); il servizio limita l'intervallo a 250–5000 ms. **Precisato in M4:** la scadenza di consegna è per sottoscrittore su una cadenza fissa (non da un contatore che riparte a ogni invio), quindi una consegna di recupero dopo un invio in ritardo può arrivare a meno di un intervallo dalla precedente;
  - `Schema { devices[], sensors[] }` (servizio → client), inviato all'avvio della sottoscrizione e quando cambia l'hardware; ogni device porta l'indizio d'identità di §5.3. Per il client un nuovo `Schema` equivale a una nuova discovery;
  - `Snapshot { seq, timestamp, values[] }` (servizio → client); `values` è indicizzato secondo l'ordine dello schema corrente e i valori assenti sono `nil`;
  - `Error { code, message }`.
- **Nessun messaggio di scrittura verso l'hardware.** Il servizio espone solo letture di alto livello, mai accesso grezzo a MSR, porte I/O o memoria fisica. Ignora i codici di controllo personalizzati del servizio (128–255), che gli utenti interattivi possono inviare con i diritti predefiniti.
- **Valori non aggiornati:** il provider `svc` legge la pipe in un thread proprio e al tick restituisce l'ultimo `Snapshot` ricevuto, senza bloccare lo scheduler. Se quell'ultimo `Snapshot` ha più di 3 intervalli, i valori del servizio sono assenti (buchi nei grafici, non valori congelati).
- **Riconnessione:** il client riprova ogni 5 s. Mentre è disconnesso l'app è in modalità base.
- **Riferimenti condivisi:** `protocol/fixtures/` contiene i messaggi MessagePack di riferimento, usati dai test di entrambi i lati.

## 7. Interfaccia

### 7.1 Barra superiore (sempre visibile)

- Nome dell'app.
- Selettore **Semplice / Avanzata**; l'app ricorda l'ultima scelta.
- Icona delle impostazioni.
- **Badge "Modalità base"** quando il servizio è assente, fermato o non raggiungibile. Cliccandolo si apre una spiegazione di cosa si perde, con il motivo e l'azione adatta (M4):
  - servizio non installato: invito a reinstallare con l'opzione "Sensori avanzati";
  - modalità anti-cheat attiva: "Disattiva modalità anti-cheat";
  - servizio in avvio: nessuna azione;
  - servizio non raggiungibile (fermo, avvio fallito, verifica del PID fallita): "Avvia";
  - versione del protocollo incompatibile: invito ad aggiornare.

  Con il servizio collegato il badge non c'è: la modalità anti-cheat si attiva dal menu della tray (dalla M5 anche dalle impostazioni, §7.4).

### 7.2 Vista Semplificata (layout "B")

- **Banner di stato** in alto: icona e colore dello stato, verdetto a parole ("Tutto in ordine", "GPU surriscaldata (92 °C)"), durata dello stato. È pilotato dal motore regole di §4.3.
- **Riquadri** con il valore principale, la temperatura colorata in base allo stato e un minigrafico degli ultimi 5 minuti:
  - CPU;
  - **una per ogni GPU dedicata**; l'iGPU compare solo se non c'è una GPU dedicata;
  - RAM (con barra);
  - Rete + Dischi;
  - Batteria, solo sui portatili.
- **Un clic su un riquadro** apre la pagina corrispondente nella vista Avanzata.

### 7.3 Vista Avanzata (layout "A")

- **Barra laterale:**
  - CPU;
  - GPU, con una voce per scheda se sono più di una. Qui compaiono tutte le GPU, iGPU compresa, a differenza della vista Semplificata;
  - RAM;
  - Dischi, con una voce per disco;
  - Rete, con una voce per scheda di rete;
  - Scheda madre (ventole, tensioni, temperature), solo con il servizio attivo;
  - Batteria, solo sui portatili.
  
  Le voci senza dati non compaiono. La vista si apre sull'ultima sezione visitata, oppure sulla CPU.
- **Ogni pagina ha:**
  - **4 KPI** in alto, definiti per tipo di componente: i primi quattro disponibili di un elenco per tipo (per la GPU: carico, temperatura, potenza, VRAM, poi clock);
  - un **grafico storico** uPlot, con finestra 1m / 5m / 30m / 1h e con le serie da mostrare selezionabili:
    - **al massimo 8 serie e 2 unità di misura** insieme, con due assi verticali (sinistro e destro); oltre questi limiti il selettore non aggiunge serie. È il limite che tiene la finestra nel budget di memoria;
    - le finestre da 30 minuti e 1 ora usano lo storico decimato (§4.2), quelle da 1 e 5 minuti tutti i campioni;
    - le etichette dell'asse del tempo seguono la lingua dell'app (24 ore in italiano);
    - i dati si aggiornano al ritmo dei sensori; lo scorrimento visivo dei grafici segue il design dell'intermezzo pre-M5 e si ferma quando la finestra non è visibile;
  - una **tabella dei sensori** raggruppata per categoria (temperature, carico, clock, potenza, tensioni, ventole…), con colonne attuale, min, max e media, e un pulsante "azzera min/max", che azzera le statistiche dei sensori della pagina (§4.2). I sensori sperimentali sono segnati come tali;
  - un **badge della fonte** su ogni sensore, visibile al passaggio del mouse o con il focus da tastiera (le righe della tabella sono raggiungibili da tastiera);
  - le **informazioni del device**: le proprietà statiche, per esempio indirizzo PCI, link PCIe massimo, limiti di potenza e soglie di temperatura;
  - per le GPU, la **tabella dei processi** che usano la GPU, con carico, motore, memoria dedicata e condivisa: al massimo 20 righe, aggiornate ogni 2 s mentre la pagina è visibile, nascoste se il provider non pubblica un aggiornamento da più di 2,5 s.
- Sezione, finestra del grafico e serie scelte per ogni pagina restano salvate nella WebView (`localStorage`) finché le impostazioni della M5 non le sostituiscono. Un clic su un riquadro della vista Semplificata apre la pagina corrispondente.
- **I sensori che richiedono il servizio**, quando questo non è attivo, non compaiono uno per uno. Al loro posto, nelle pagine CPU, RAM e dischi, c'è un solo avviso generico, senza numero (decisione M4): "Temperature, tensioni e altri sensori disponibili con il servizio". Il numero non si mostra perché senza il servizio l'app non può saperlo.
- La voce **Batteria** compare solo quando esiste un device batteria: in M3 nessun provider lo crea ancora.
- **Nessuna pagina "Panoramica"**: quel ruolo lo svolge la vista Semplificata.

### 7.4 Impostazioni

- Generale (lingua, unità, intervallo, tray, avvio automatico).
- Regole e avvisi (tabella delle regole con modifica e creazione).
- Log CSV.
- Fonti dati (interruttori per provider, stato del servizio, modalità anti-cheat).
- Informazioni (versione, licenze di terze parti; dalla M6 il controllo degli aggiornamenti e **"Esporta report sensori"**: un JSON anonimo con dispositivi, sensori, fonti e valori correnti da allegare alle segnalazioni).

### 7.5 Stile visivo — palette "Synthwave" (solo tema scuro nella v1)

Tutti i colori sono token CSS, così un tema chiaro o un accento personalizzabile si possono aggiungere in seguito senza toccare i componenti.

| Token | Valore | Uso |
|---|---|---|
| `--bg` | `#0f0a1a` | sfondo dell'app |
| `--surface` | `#181126` | riquadri e card |
| `--surface-2` | `#211733` | controlli e hover |
| `--border` | `#2d2042` | bordi e divisori |
| `--text` | `#f5eefe` | testo principale |
| `--text-muted` | `#9585b0` | etichette e testo secondario |
| `--accent` | `#ff4fd8` | elemento attivo, serie primaria |
| `--accent-2` | `#4cc9f0` | serie secondaria |
| `--ok` | `#3ee8b5` | stato ok |
| `--warn` | `#ffc53d` | stato attenzione |
| `--crit` | `#ff4d4d` | stato critico |

- **Il rosa neon va usato con parsimonia** (accento, serie principali, elemento attivo), senza bagliori diffusi, per non stancare nelle sessioni lunghe. La vista Avanzata è più sobria della Semplificata.
- I colori di stato restano sempre distinti dall'accento.
- **Animazioni:**
  - i numeri cambiano con un'interpolazione breve (`tweened`, ≤ 300 ms);
  - i grafici scorrono continuamente fino a circa 60 FPS quando sono visibili (60, 30 o 15 FPS dalle impostazioni della M5); le altre animazioni non sono continue, con una sola deroga: il pallino del registratore CSV lampeggia a 1 Hz a passi discreti durante la registrazione (spec della M5, §4.4);
  - si rispetta `prefers-reduced-motion`.
- **Grafici:**
  - un solo ciclo di rendering condiviso;
  - i campioni restano al ritmo dei sensori; lo scorrimento visivo, la curvatura, il punto bianco finale e il glow leggero seguono `docs/superpowers/specs/2026-09-27-fluid-charts-design.md`;
  - rendering sospeso quando la finestra non è visibile;
  - buffer tipizzati (`Float64Array`) per le serie interne della vista Semplificata; per il grafico uPlot della vista Avanzata, array di `number | null`, perché `null` rappresenta i buchi. Il costo delle copie e della coda dal vivo rientra nella misura del budget con la finestra aperta per almeno un'ora.

### 7.6 Internazionalizzazione

Stringhe in file JSON per lingua (`en`, `it`), con l'inglese come lingua di riserva. Anche i messaggi del banner e delle notifiche, generati dal nucleo, usano chiavi di traduzione con parametri. Un test verifica che le due lingue abbiano le stesse chiavi.

## 8. Gestione errori

- **Provider isolati:**
  - ogni errore mette il provider in stato "degradato", con nuovi tentativi a intervalli crescenti (5 s → 60 s);
  - i suoi sensori mostrano "—" e l'indicazione della fonte non disponibile;
  - un provider non può bloccare lo scheduler.
- **Librerie dei vendor:** un crash dentro una DLL non si può intercettare. Per questo ci sono:
  - un interruttore per ogni provider nelle impostazioni (M5);
  - la **modalità sicura**: le librerie dei vendor GPU (NVML, NVAPI, ADL, IGCL) non vengono caricate e restano solo D3DKMT, DXGI e PDH. Si attiva con l'avvio `--safe` oppure automaticamente dopo un crash.

  **Rilevamento dei crash:**
  - All'avvio l'app installa un filtro per le eccezioni non gestite (`SetUnhandledExceptionFilter`).
  - Se il processo sta per terminare per un crash nativo, il filtro scrive `%LOCALAPPDATA%\OpenMonitorAdvanced\crash.txt` con il codice dell'eccezione e il percorso del modulo in cui è avvenuta.
  - Il filtro è best-effort: non copre `__fastfail`/abort, terminazioni forzate, né garantisce di riuscire a scrivere su un processo corrotto. Il file non viene scritto durante una normale chiusura o uno spegnimento.
  - All'avvio successivo l'app legge e cancella il file e parte in modalità sicura.
  - Sotto la barra superiore un avviso spiega il motivo, con il nome della DLL, e offre **"Riattiva"**: le librerie si caricano subito, senza riavviare l'app, e restano caricate fino alla chiusura.
- **Servizio:** disconnessioni, timeout, versione del protocollo non compatibile e verifica del PID fallita (§6) portano alla modalità base, con badge, motivo e spiegazione. Non producono mai errori bloccanti. Un crash del servizio fa scattare il riavvio automatico configurato dall'installer (§10); l'app si ricollega da sola.
- **Dati anomali:** valori fuori dall'intervallo fisico plausibile vengono scartati come assenti e registrati nel log di diagnostica, al massimo una riga al minuto per sensore. Esempi: temperature < −50 °C o > 150 °C, percentuali < 0 o > 100 dove non ha senso.
- **Nucleo:** un panic durante un ciclo di campionamento viene registrato nel log e il ciclo successivo parte regolarmente. Un disallineamento tra valori e sensori non ferma lo storico: i valori mancanti diventano assenti e quelli in più si scartano. Se l'interfaccia non riceve dati per più di max(5 s, 5 intervalli), la barra superiore mostra "Dati non aggiornati".
- **Log di diagnostica:** `tracing` in `%LOCALAPPDATA%\OpenMonitorAdvanced\logs`, a rotazione. Il servizio scrive in un file proprio.

## 9. Sicurezza

- L'interfaccia non gira mai con privilegi elevati.
- Il servizio ha una superficie minima: nessuna rete, sola lettura, validazione di ogni messaggio, limiti sulla dimensione dei frame e sul numero di client, ACL sulla pipe, codici di controllo personalizzati ignorati.
- **Diritti sul servizio:** gli utenti interattivi possono solo avviarlo e fermarlo (§2.2). L'installer aggiunge l'ACE al descrittore esistente, letto al momento, invece di sostituirlo con una stringa fissa: `sc sdset` sostituisce l'intero descrittore e un errore toglierebbe i diritti a SYSTEM e agli amministratori. L'eseguibile sta in `Program Files`, scrivibile solo dagli amministratori, e il percorso del servizio è tra virgolette.
- **Pipe impersonata:** il client verifica che il server della pipe sia il processo del servizio (§6).
- CSP stretta in Tauri, nessun contenuto remoto; i comandi Tauri sono limitati tramite capability.
- **PawnIO:**
  - si includono solo il setup ufficiale firmato (ridistribuibile) e i moduli ufficiali; è pratica comune (LibreHardwareMonitor e FanControl lo includono), ma prima della 1.0 si chiede conferma all'autore;
  - versione minima 2.2.0 (§10);
  - nessun modulo proprio nella v1;
  - niente WinRing0 né inpoutx64.
- **Aggiornamenti nella v1 (dalla M6):** solo un controllo opzionale delle nuove release su GitHub, con link al download, senza installazione automatica.
- **Firma dei binari:** decisa con SignPath.io (firma gratuita per progetti open source), per ridurre gli avvisi di SmartScreen; il design è nella spec M6a (`docs/superpowers/specs/2026-09-30-m6a-release-firma-design.md`). Ammissione alla Foundation e collaudo della firma pendenti (§13).

## 10. Installazione e distribuzione

- Un solo **installer NSIS**, generato dal bundler di Tauri, che contiene app e servizio. Dimensione stimata: circa 50 MB, per via di .NET self-contained.
- **Installazione per tutta la macchina** (`installMode: perMachine`): una sola conferma UAC all'avvio dell'installer, cartella in `Program Files`.
- **Template NSIS proprio** (decisione M4), copiato dal template di tauri-cli 2.11.5: gli hook di Tauri (`NSIS_HOOK_*`) non permettono di aggiungere una pagina di scelta nel punto giusto. Un test confronta il template con la copia di riferimento di Tauri da cui deriva, così un aggiornamento di Tauri segnala le differenze da riallineare.
- **Opzione "Sensori avanzati"**, una pagina Componenti attiva di default. In modalità silenziosa (`/S`) vale il default; `/NOSENSORS` la esclude. La scelta resta registrata in HKLM, così gli aggiornamenti la mantengono. Se è attiva:
  - copia i file del servizio e crea `oma-service` in avvio manuale, con il percorso tra virgolette e il riavvio automatico dopo un crash;
  - aggiunge i diritti di avvio e arresto per gli utenti interattivi (§2.2, §9);
  - se PawnIO manca o è più vecchio della 2.2.0 (chiave `Uninstall\PawnIO`, valore `DisplayVersion`, vista a 64 bit), esegue in silenzio il setup ufficiale incluso (`-install -silent`). L'esito 3010 significa che serve un riavvio e l'installer lo segnala.
  - **Precisato in M4:** il componente richiede che la cartella d'installazione (percorso normalizzato, senza reparse point) sia dentro `Program Files`; un'installazione silenziosa fuori da `Program Files` senza `/NOSENSORS` fallisce con codice di uscita 2, così come un fallimento di STOP, dell'helper di disinstallazione o del setup di PawnIO. Il 3010 si segnala solo a installazione riuscita. La cartella del servizio riceve una DACL esplicita (controllo completo per SYSTEM e Administrators, lettura ed esecuzione per Users) invece di ereditare i permessi di `Program Files`. Limite accettato: una cartella di terze parti dentro `Program Files` che concede a Users il permesso di modifica supera comunque il controllo, che verifica solo il percorso (`docs/follow-ups.md`).
- **Prima di copiare i file** l'installer ferma il servizio e aspetta che sia davvero fermo: il template di Tauri chiude solo l'app.
- **Build:** uno script pubblica il servizio e scarica il setup ufficiale di PawnIO nella versione fissata, verificandone lo SHA-256. Il setup non si versiona in git.
- **Modalità compatibile anti-cheat:** vedi §2.2. Non cambia la configurazione del servizio e non richiede privilegi.
- La disinstallazione ferma e rimuove il servizio; in modalità aggiornamento non lo tocca. PawnIO resta, perché può essere condiviso con altri programmi come FanControl. **Precisato in M4:** se l'arresto del servizio fallisce o va in timeout, l'helper di disinstallazione restituisce 1 e **non** cancella il servizio, per non lasciare Windows con un servizio a metà rimosso mentre è ancora in esecuzione.

## 11. Struttura del repository

```
crates/oma-core/                    modello dati, scheduler, merge, storico, regole, logger CSV
crates/oma-win/                     provider Windows: sys, gpu (PDH, D3DKMT, NVML, NVAPI, ADL, IGCL),
                                    svc (client named pipe, controllo del servizio)
crates/oma-ipc/                     tipi del protocollo, codifica e framing (portabile, senza codice Windows)
app/src-tauri/                      shell Tauri: comandi, eventi, tray, ciclo di vita della WebView
app/src/                            UI Svelte 5 + TypeScript + uPlot + i18n
service/OpenMonitorAdvanced.Service/        servizio .NET + LibreHardwareMonitorLib
service/OpenMonitorAdvanced.Service.Tests/
protocol/fixtures/                  messaggi MessagePack di riferimento condivisi
docs/
```

## 12. Test e verifica

- **`oma-core`**, test unitari su:
  - merge per priorità di fonte;
  - motore regole (durate, isteresi, stato più grave, regole personalizzate), con un orologio finto;
  - ring buffer e min/max/media;
  - contatori cumulativi e ritorno a zero;
  - logger CSV (formato e divisione del file).
  
  Un `Provider` finto riproduce snapshot registrati da macchine reali.
- **Protocollo:** i test Rust e .NET codificano e decodificano le fixture di `protocol/fixtures/` e verificano che i byte coincidano.
- **`oma-service`:** xUnit sulla conversione da LibreHardwareMonitor al modello, con sensori finti; limiti del protocollo (intervallo, dimensione dei frame, messaggi non validi); arresto dopo 2 minuti senza client, con un orologio finto.
- **Provider `svc` e client (M4):** trasporto finto per snapshot vecchi, nuovo schema e disconnessione; verifica del PID e stati del servizio come funzioni pure; pipe vera con un server di prova non privilegiato e un nome di pipe diverso (`#[ignore]`).
- **Merge tra provider:** a parità di ID vince il primo provider; device con lo stesso ID fusi.
- **`oma-win`:** test di integrazione eseguiti su hardware reale, marcati `#[ignore]` in CI. Test unitari sulla logica di aggregazione PDH e sull'associazione LUID ↔ PCI, con dati registrati.
- **UI:** Vitest su store, formattazione delle unità e completezza delle traduzioni. **Modalità "backend finto"**: la UI gira nel browser con dati registrati, per sviluppare la grafica e per i test dei componenti.
- **Hardware reale:** checklist manuale per ogni release su una matrice di macchine (NVIDIA, AMD, Intel dedicata e integrata, un portatile), più il "report sensori" inviato dalla community.
- **Budget di prestazioni** (§1.2), misurato a ogni milestone.
- **CI (GitHub Actions, Windows):** `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `dotnet test`, `pnpm test`, `pnpm check`. Build dell'installer quando si crea un tag.

## 13. Punti aperti da verificare in implementazione

1. **FACEIT e PawnIO:** basta fermare il servizio perché l'anti-cheat accetti il sistema, o bisogna fermare anche il driver? **In gran parte risolto in M4 (ricerca):** il blocco di FACEIT dipendeva dal certificato delle versioni di PawnIO precedenti alla 2.1.0; con la 2.2.0 FACEIT funziona anche con il driver caricato (conferme dell'autore di PawnIO e di FanControl, 2026). Il driver resta caricato comunque, perché lo carica Windows (§2.2). Resta una verifica manuale con un gioco vero.
2. **LibreHardwareMonitorLib con trimming e NativeAOT:** incide sulla dimensione dell'installer. Si decide con lo spike di M4 (§5.3): NativeAOT è escluso, perché i percorsi WMI di LibreHardwareMonitorLib non sono sicuri sotto AOT.
3. **Valore corretto dell'enum per `D3DKMT_NODE_PERFDATA`** (clock ed eventuale tensione per motore): una prima prova ha restituito `STATUS_INVALID_PARAMETER`. **Risolto in M2:** `KMTQAITYPE_NODEPERFDATA` vale 61. La struttura va passata con la dimensione esatta di 56 byte: con una dimensione diversa la chiamata restituisce `STATUS_INVALID_PARAMETER`. La `Frequency` del nodo 0 è il clock core e coincide con `nvidia-smi`. La tensione vale 0 su NVIDIA e circa 1110–1125 (probabilmente mV) sull'iGPU AMD, quindi non si usa.
4. **Driver Intel e Qualcomm e `ADAPTERPERFDATA`:** lo popolano? **In parte risolto in M2:** i driver NVIDIA e AMD lo popolano, anche per l'iGPU AMD (temperatura a passi di 1 °C, potenza in % del limite, frequenza della DRAM). Intel e Qualcomm restano da verificare, perché non c'era hardware disponibile. Se un driver non lo popola (temperatura 0), quei sensori semplicemente non compaiono.
5. **Licenza di ADL (legacy):** va verificata prima di usarne i binding; in alternativa si resta sul livello base per AMD. **Risolto in M2:** si usano binding scritti a mano dalla documentazione pubblica e `atiadlxx.dll` si carica a runtime da `System32`. Gli header di AMD non si includono e non si scaricano, perché la loro EULA esclude le licenze come la GPL. ADLX resta escluso.
6. **NVMe via `IOCTL_STORAGE_QUERY_PROPERTY` senza privilegi:** funziona? **Risolto in M3:** sì. `StorageDeviceTemperatureProperty` su `\\.\PhysicalDriveN` aperto con accesso 0 funziona da utente normale su Windows 11 (build 26200), sia per NVMe sia per SATA. Il supporto dipende dal disco: un SSD SATA risponde `ERROR_INVALID_FUNCTION` (non supportato, non un problema di permessi). Vedi §5.1.
7. **Firma del codice:** il design è nella spec M6a (`docs/superpowers/specs/2026-09-30-m6a-release-firma-design.md`); ammissione a SignPath.io e collaudo della firma pendenti fino alla verifica reale.

## 14. Milestone

Ogni milestone avrà un proprio piano di implementazione.

1. **Fondamenta:** monorepo, CI, modello dati, scheduler, provider `sys`, shell Tauri con tray minima, vista Semplificata con CPU, RAM, dischi e rete.
2. **GPU:** enumerazione, livello base PDH/D3DKMT, NVML, NVAPI, ADL, IGCL, merge con priorità.
3. **Vista Avanzata:** barra laterale, pagine per componente, grafici uPlot, storico, tabelle con min/max/media.
4. **Servizio:** `oma-service` con LibreHardwareMonitorLib, protocollo IPC con le fixture, installer NSIS con PawnIO, modalità anti-cheat.
4.5. **Intermezzo grafici fluidi:** scorrimento a circa 60 FPS delle due viste, curve morbide, punto finale bianco e glow leggero; design in `docs/superpowers/specs/2026-09-27-fluid-charts-design.md`.
5. **Regole e integrazione:** motore regole, banner di stato, notifiche, tray completa, log CSV, impostazioni, traduzioni it/en. Design di dettaglio in `docs/superpowers/specs/2026-09-29-m5-regole-integrazione-design.md`; si esegue in tre piani: M5a impostazioni e tray, M5b regole, M5c log CSV.
6. **Rifinitura e 1.0:** verifica del budget di prestazioni, controllo degli aggiornamenti, "Esporta report sensori", documentazione, licenze di terze parti, release.

## Appendice A — Riferimenti principali della ricerca (settembre 2026)

- LibreHardwareMonitor (MPL-2.0, v0.9.6, su PawnIO dalla 0.9.5): https://github.com/LibreHardwareMonitor/LibreHardwareMonitor — PR #1857
- PawnIO (driver GPL-2.0 con eccezione per IOCTL; libreria e moduli LGPL-2.1; setup firmato ridistribuibile): https://github.com/namazso/PawnIO, https://pawnio.eu/
- Avviso di Microsoft su WinRing0: https://support.microsoft.com/en-us/windows/microsoft-defender-antivirus-alert-vulnerabledriver-winnt-winring0-eb057830-d77b-41a2-9a34-015a5d203c42
- Fine della fiducia nei driver cross-signed (aprile 2026): https://techcommunity.microsoft.com/blog/windows-itpro-blog/advancing-windows-driver-security-removing-trust-for-the-cross-signed-driver-pro/4504818
- FACEIT e PawnIO: https://github.com/Rem0o/FanControl.Releases/issues/3789
- NVML: https://docs.nvidia.com/deploy/nvml-api/ — NVAPI (MIT): https://github.com/NVIDIA/nvapi
- ADLX (licenza incompatibile): https://github.com/GPUOpen-LibrariesAndSDKs/ADLX
- IGCL: https://github.com/intel/drivers.gpu.control-library
- D3DKMT_ADAPTER_PERFDATA: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/d3dkmthk/ns-d3dkmthk-_d3dkmt_adapter_perfdata
- GPU in Task Manager (PDH): https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/
- Riferimenti di codice con licenza compatibile: PresentMon (MIT), System Informer `gpumon.c` (MIT), nvml-wrapper (MIT/Apache), amdgpu_top (MIT), nvtop e Mission Center (GPL-3.0)
- Tauri 2: https://v2.tauri.app/ — uPlot: https://github.com/leeoniya/uPlot
- Wi-Fi e consenso alla posizione: https://learn.microsoft.com/en-us/windows/win32/nativewifi/wi-fi-access-location-changes
- PawnIO e FACEIT, blocco risolto dalla 2.1.0/2.2.0 (nuovo certificato): https://github.com/namazso/PawnIO.Setup/issues/1, https://github.com/Rem0o/FanControl.Releases/issues/3660, https://github.com/Rem0o/FanControl.Releases/issues/4114
- PawnIO come device PnP con avvio a richiesta (INF): https://github.com/namazso/PawnIO/blob/master/PawnIO/PawnIO.inf.in — setup ufficiale: https://github.com/namazso/PawnIO.Setup/releases
- Diritti di accesso ai servizi e rischi: https://learn.microsoft.com/en-us/windows/win32/services/service-security-and-access-rights — modifica del DACL: https://learn.microsoft.com/en-us/windows/win32/services/modifying-the-dacl-for-a-service
- Worker Service come servizio Windows (.NET): https://learn.microsoft.com/en-us/dotnet/core/extensions/windows-service
- Installer NSIS di Tauri (installMode, hook, template): https://v2.tauri.app/distribute/windows-installer/
