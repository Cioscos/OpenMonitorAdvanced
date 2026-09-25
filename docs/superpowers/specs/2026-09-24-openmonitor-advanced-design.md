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
| Leggerezza | L'app non deve falsare le misure: consumo a riposo minimo, nessuna modifica della risoluzione del timer di sistema, nessuna animazione continua. |

### 1.2 Criteri di successo

- Un utente apre l'app e capisce **in un'occhiata** se il PC sta bene, grazie alla vista Semplificata con il banner di stato.
- Un appassionato trova nella vista Avanzata temperature, clock, potenze, tensioni, throttling, min/max e storico di ogni componente.
- **Budget di prestazioni**, misurato a ogni milestone:
  - nucleo a riposo < 1% di CPU;
  - processo in tray (finestra chiusa) < 30 MB di RAM;
  - finestra aperta < 200 MB in totale, WebView2 compresa.
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
  - Interroga i sensori **solo mentre almeno un client è sottoscritto**.
  - Non espone rete né comandi di scrittura verso l'hardware.

### 2.3 Predisposizione per Linux

I provider implementano un trait `Provider`. Su Linux si aggiungerà un provider per hwmon, sysfs, powercap e DRM fdinfo; quasi tutto è leggibile senza root. Nucleo, protocollo e interfaccia restano invariati, e su Linux il servizio .NET non serve. Tutte le parti specifiche di Windows restano nel crate `oma-win`.

## 3. Modello dati

- **`Device`**: `id` stabile, `kind` (`cpu`, `gpu`, `memory`, `storage`, `network`, `motherboard`, `battery`, `fan_controller`, `psu`), `name`, `vendor`, proprietà statiche (modello, driver, capacità…).
- **`Sensor`**: `id` stabile, `device_id`, `kind` (`temperature`, `load`, `clock`, `power`, `voltage`, `current`, `fan`, `data`, `throughput`, `energy`, `flag`, `percent`), `unit`, `label` (chiave i18n oppure testo fornito dal driver), `source` (`pdh`, `dxgi`, `d3dkmt`, `nvml`, `nvapi`, `adl`, `igcl`, `lhm`, `wmi`…), `category` (per il raggruppamento nella tabella della vista Avanzata), `experimental` (vero per le letture ottenute da chiamate non documentate, vedi §5.2: l'interfaccia le segna come sperimentali).
- **`Reading`**: `sensor_id`, `value: f64` oppure assente, `timestamp`.
- **ID stabili tra riavvii**, costruiti dall'identità hardware. Esempi: `gpu/pci-0000:01:00.0/temperature/hotspot`, `storage/device-<hash>/temperature/drive`. Regole, impostazioni e selezione dei sensori per il log vi fanno riferimento. Per i dischi l'identità segue una catena di ripiego (§5.1).
- **Merge:**
  - quando più fonti forniscono lo stesso sensore logico, vince quella con priorità più alta (priorità per campo, vedi §5.2);
  - la fonte scelta viene registrata e mostrata nell'interfaccia;
  - la scelta segue la fonte, mai il nome del vendor.

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

Un **unico motore** alimenta sia il banner di stato della vista Semplificata sia le notifiche, così i due non si contraddicono mai.

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

**Nessuna regola predefinita sulle ventole ferme.** Molte GPU e alcuni case spengono le ventole di proposito a basso carico. L'utente può creare regole sulle ventole a mano.

### 4.4 Logger CSV

- Si avvia e si ferma dall'interfaccia o dalla tray.
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
- **Menu:** Apri, vista Semplificata/Avanzata, Avvia/Ferma log, Modalità compatibile anti-cheat, Esci.
- **Comportamento:**
  - chiudere la finestra la riduce nella tray (disattivabile);
  - avvio automatico con Windows opzionale;
  - una sola istanza alla volta.

### 4.6 Impostazioni

File `%APPDATA%\OpenMonitorAdvanced\settings.json`, con versione dello schema e migrazioni. Contiene:
- vista predefinita, lingua, unità (°C/°F, bit/s o byte/s), intervallo di aggiornamento;
- regole;
- sensori selezionati per il log e per la tray;
- interruttori per ogni provider;
- comportamento della tray e avvio automatico;
- controllo opzionale degli aggiornamenti.

## 5. Acquisizione dati (Windows)

### 5.1 Provider `sys` (senza privilegi)

| Area | Fonte |
|---|---|
| Carico CPU totale e per core | PDH `Processor Information(*)\% Processor Utility` (come Task Manager su Windows 10/11); se il contatore non è disponibile, `% Processor Time` |
| Clock CPU (stima) | PDH `% Processor Performance` × frequenza base, lo stesso metodo di Task Manager. Il clock effettivo per core arriva dal servizio. |
| RAM | `GlobalMemoryStatusEx`, `GetPerformanceInfo`; SMBIOS tipo 17 e `Win32_PhysicalMemory` per velocità e moduli |
| Dischi: spazio e throughput | `GetDiskFreeSpaceEx`; PDH `PhysicalDisk(*)` (letture e scritture al secondo, byte/s, % tempo attivo) |
| Dischi: identità | Catena di ripiego, dal livello più forte: numero di serie (`storage/device-…`, invariato dalla M1; un seriale non UTF-8 si usa come byte grezzi) → GUID del disco GPT (`storage/gpt-…`, da `IOCTL_DISK_GET_DRIVE_LAYOUT_EX`) → firma MBR più dimensione del disco (`storage/mbr-…`) → instance id PnP (`storage/pnp-…`, da SetupAPI; legato alla porta, cambia se il disco viene spostato). Ogni valore entra nell'ID solo come hash SHA-256. Un livello si usa solo se il suo valore è unico tra i dischi della macchina, perché i cloni copiano GUID e firma. Un disco si omette solo se falliscono tutti i livelli, con un avviso nel log che indica il motivo per ciascuno. Tutte le chiamate usano `\\.\PhysicalDriveN` aperto con accesso 0, senza privilegi. |
| Temperatura dei dischi | `IOCTL_STORAGE_QUERY_PROPERTY` con `StorageDeviceTemperatureProperty`, senza privilegi (verificato in M3 su NVMe e SATA). Il supporto dipende da disco e driver: un disco senza supporto risponde `ERROR_INVALID_FUNCTION` e non ha sensori. Il sensore 0 è la temperatura del disco (la "composite" degli NVMe, `…/temperature/drive`), gli altri sono sensori aggiuntivi (`…/temperature/sensor-<n>`); `0x8000` significa "non riportato". Le soglie warning e critical diventano proprietà del device (`tempWarningC`, `tempCriticalC`) per le regole della M5. Lettura ogni 30 s (§4.1), al massimo un disco per ciclo (fino a circa 140 ms per un NVMe che esce da uno stato a basso consumo). Un disco in standby (`GetDevicePowerState`) non viene interrogato, per non risvegliarlo. |
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
     - **Stato:** implementato e coperto da test con funzioni finte, **non ancora verificato su hardware Intel**. Dalla M3 legge anche il link PCIe correnti (`ctlPciGetState` per generazione e larghezza correnti); i massimi non si leggono da IGCL, ma dal livello base PnP (punto 5), identico su ogni vendor.
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
- **Conversione:** l'albero Hardware/Sensor di LibreHardwareMonitor viene tradotto nel modello di §3, con ID stabili basati sull'identificatore hardware di LibreHardwareMonitor.
- **Build self-contained**, ridotta (trimmed) se LibreHardwareMonitorLib lo tollera (da verificare), altrimenti non ridotta.

## 6. Protocollo IPC

- **Trasporto:** named pipe `\\.\pipe\OpenMonitorAdvanced.Sensors.v1`, creata dal servizio con `FILE_FLAG_FIRST_PIPE_INSTANCE` e un descrittore di sicurezza esplicito:
  - SYSTEM e Administrators: controllo completo;
  - utenti interattivi: lettura e scrittura dei messaggi.
  
  Il client si connette con `SECURITY_IDENTIFICATION`.
- **Frame:** lunghezza `u32` little-endian seguita dal payload **MessagePack**. Dimensione massima di 4 MB; un frame più grande chiude la connessione.
- **Messaggi:**
  - `Hello { protocol_version, service_version }` (servizio → client); se la versione del protocollo non è compatibile, l'interfaccia chiede di aggiornare il servizio;
  - `Subscribe { interval_ms }` (client → servizio); il servizio limita l'intervallo a 250–5000 ms;
  - `Schema { devices[], sensors[] }` (servizio → client), inviato all'avvio della sottoscrizione e quando cambia l'hardware;
  - `Snapshot { seq, timestamp, values[] }` (servizio → client); `values` è indicizzato secondo l'ordine dello schema corrente e i valori assenti sono `nil`;
  - `Error { code, message }`.
- **Nessun messaggio di scrittura verso l'hardware.** Il servizio espone solo letture di alto livello, mai accesso grezzo a MSR, porte I/O o memoria fisica.
- **Riconnessione:** il client riprova ogni 5 s. Mentre è disconnesso l'app è in modalità base.
- **Riferimenti condivisi:** `protocol/fixtures/` contiene i messaggi MessagePack di riferimento, usati dai test di entrambi i lati.

## 7. Interfaccia

### 7.1 Barra superiore (sempre visibile)

- Nome dell'app.
- Selettore **Semplice / Avanzata**; l'app ricorda l'ultima scelta.
- Icona delle impostazioni.
- **Badge "Modalità base"** quando il servizio è assente, fermato o non raggiungibile. Cliccandolo si apre una spiegazione di cosa si perde e l'invito a installare o avviare il servizio.

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
    - il grafico si aggiorna al ritmo dei dati e si ferma quando la finestra non è visibile;
  - una **tabella dei sensori** raggruppata per categoria (temperature, carico, clock, potenza, tensioni, ventole…), con colonne attuale, min, max e media, e un pulsante "azzera min/max", che azzera le statistiche dei sensori della pagina (§4.2). I sensori sperimentali sono segnati come tali;
  - un **badge della fonte** su ogni sensore, visibile al passaggio del mouse o con il focus da tastiera (le righe della tabella sono raggiungibili da tastiera);
  - le **informazioni del device**: le proprietà statiche, per esempio indirizzo PCI, link PCIe massimo, limiti di potenza e soglie di temperatura;
  - per le GPU, la **tabella dei processi** che usano la GPU, con carico, motore, memoria dedicata e condivisa: al massimo 20 righe, aggiornate ogni 2 s mentre la pagina è visibile, nascoste se il provider non pubblica un aggiornamento da più di 2,5 s.
- Sezione, finestra del grafico e serie scelte per ogni pagina restano salvate nella WebView (`localStorage`) finché le impostazioni della M5 non le sostituiscono. Un clic su un riquadro della vista Semplificata apre la pagina corrispondente.
- **I sensori che richiedono il servizio**, quando questo non è attivo, non compaiono uno per uno. Al loro posto c'è un solo avviso: "N sensori in più disponibili con il servizio". L'avviso arriva con il servizio (M4): in M3 non esistono ancora sensori del servizio.
- La voce **Batteria** compare solo quando esiste un device batteria: in M3 nessun provider lo crea ancora.
- **Nessuna pagina "Panoramica"**: quel ruolo lo svolge la vista Semplificata.

### 7.4 Impostazioni

- Generale (lingua, unità, intervallo, tray, avvio automatico).
- Regole e avvisi (tabella delle regole con modifica e creazione).
- Log CSV.
- Fonti dati (interruttori per provider, stato del servizio, modalità anti-cheat).
- Informazioni (versione, licenze di terze parti, **"Esporta report sensori"**: un JSON anonimo con dispositivi, sensori, fonti e valori correnti da allegare alle segnalazioni).

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
  - nessuna animazione continua;
  - si rispetta `prefers-reduced-motion`.
- **Grafici:**
  - un solo ciclo di rendering condiviso;
  - aggiornamento al ritmo dei dati;
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
- **Servizio:** disconnessioni, timeout e versione del protocollo non compatibile portano alla modalità base, con badge e spiegazione. Non producono mai errori bloccanti.
- **Dati anomali:** valori fuori dall'intervallo fisico plausibile vengono scartati come assenti e registrati nel log di diagnostica, al massimo una riga al minuto per sensore. Esempi: temperature < −50 °C o > 150 °C, percentuali < 0 o > 100 dove non ha senso.
- **Nucleo:** un panic durante un ciclo di campionamento viene registrato nel log e il ciclo successivo parte regolarmente. Un disallineamento tra valori e sensori non ferma lo storico: i valori mancanti diventano assenti e quelli in più si scartano. Se l'interfaccia non riceve dati per più di max(5 s, 5 intervalli), la barra superiore mostra "Dati non aggiornati".
- **Log di diagnostica:** `tracing` in `%LOCALAPPDATA%\OpenMonitorAdvanced\logs`, a rotazione. Il servizio scrive in un file proprio.

## 9. Sicurezza

- L'interfaccia non gira mai con privilegi elevati.
- Il servizio ha una superficie minima: nessuna rete, sola lettura, validazione di ogni messaggio, limiti sulla dimensione dei frame, ACL sulla pipe.
- CSP stretta in Tauri, nessun contenuto remoto; i comandi Tauri sono limitati tramite capability.
- **PawnIO:**
  - si includono solo il setup ufficiale firmato (ridistribuibile) e i moduli ufficiali;
  - nessun modulo proprio nella v1;
  - niente WinRing0 né inpoutx64.
- **Aggiornamenti nella v1:** solo un controllo opzionale delle nuove release su GitHub, con link al download, senza installazione automatica.
- **Firma dei binari:** da valutare con SignPath.io (firma gratuita per progetti open source), per ridurre gli avvisi di SmartScreen. È un punto aperto (§13).

## 10. Installazione e distribuzione

- Un solo **installer NSIS**, generato dal bundler di Tauri e personalizzato, che contiene app e servizio. Dimensione stimata: circa 50 MB, per via di .NET self-contained.
- **Opzione "Sensori avanzati", attiva di default:**
  - installa e avvia `oma-service`;
  - se PawnIO non è presente (chiave di registro `Uninstall\PawnIO`), esegue il suo setup ufficiale incluso;
  - richiede una sola conferma UAC.
- **Modalità compatibile anti-cheat:** ferma il servizio e lo reimposta sull'avvio manuale; si disattiva con un clic. ⚠️ Da verificare: se basta questo, o se va fermato anche il driver PawnIO perché FACEIT non lo rilevi.
- La disinstallazione rimuove app e servizio. PawnIO resta, perché può essere condiviso con altri programmi come FanControl.

## 11. Struttura del repository

```
crates/oma-core/                    modello dati, scheduler, merge, storico, regole, logger CSV
crates/oma-win/                     provider Windows: sys, gpu (PDH, D3DKMT, NVML, NVAPI, ADL, IGCL)
crates/oma-ipc/                     tipi del protocollo + client named pipe
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
- **`oma-service`:** xUnit sulla conversione da LibreHardwareMonitor al modello, con sensori finti.
- **`oma-win`:** test di integrazione eseguiti su hardware reale, marcati `#[ignore]` in CI. Test unitari sulla logica di aggregazione PDH e sull'associazione LUID ↔ PCI, con dati registrati.
- **UI:** Vitest su store, formattazione delle unità e completezza delle traduzioni. **Modalità "backend finto"**: la UI gira nel browser con dati registrati, per sviluppare la grafica e per i test dei componenti.
- **Hardware reale:** checklist manuale per ogni release su una matrice di macchine (NVIDIA, AMD, Intel dedicata e integrata, un portatile), più il "report sensori" inviato dalla community.
- **Budget di prestazioni** (§1.2), misurato a ogni milestone.
- **CI (GitHub Actions, Windows):** `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `dotnet test`, `pnpm test`, `pnpm check`. Build dell'installer quando si crea un tag.

## 13. Punti aperti da verificare in implementazione

1. **FACEIT e PawnIO:** basta fermare il servizio perché l'anti-cheat accetti il sistema, o bisogna fermare anche il driver?
2. **LibreHardwareMonitorLib con trimming e NativeAOT:** incide sulla dimensione dell'installer.
3. **Valore corretto dell'enum per `D3DKMT_NODE_PERFDATA`** (clock ed eventuale tensione per motore): una prima prova ha restituito `STATUS_INVALID_PARAMETER`. **Risolto in M2:** `KMTQAITYPE_NODEPERFDATA` vale 61. La struttura va passata con la dimensione esatta di 56 byte: con una dimensione diversa la chiamata restituisce `STATUS_INVALID_PARAMETER`. La `Frequency` del nodo 0 è il clock core e coincide con `nvidia-smi`. La tensione vale 0 su NVIDIA e circa 1110–1125 (probabilmente mV) sull'iGPU AMD, quindi non si usa.
4. **Driver Intel e Qualcomm e `ADAPTERPERFDATA`:** lo popolano? **In parte risolto in M2:** i driver NVIDIA e AMD lo popolano, anche per l'iGPU AMD (temperatura a passi di 1 °C, potenza in % del limite, frequenza della DRAM). Intel e Qualcomm restano da verificare, perché non c'era hardware disponibile. Se un driver non lo popola (temperatura 0), quei sensori semplicemente non compaiono.
5. **Licenza di ADL (legacy):** va verificata prima di usarne i binding; in alternativa si resta sul livello base per AMD. **Risolto in M2:** si usano binding scritti a mano dalla documentazione pubblica e `atiadlxx.dll` si carica a runtime da `System32`. Gli header di AMD non si includono e non si scaricano, perché la loro EULA esclude le licenze come la GPL. ADLX resta escluso.
6. **NVMe via `IOCTL_STORAGE_QUERY_PROPERTY` senza privilegi:** funziona? **Risolto in M3:** sì. `StorageDeviceTemperatureProperty` su `\\.\PhysicalDriveN` aperto con accesso 0 funziona da utente normale su Windows 11 (build 26200), sia per NVMe sia per SATA. Il supporto dipende dal disco: un SSD SATA risponde `ERROR_INVALID_FUNCTION` (non supportato, non un problema di permessi). Vedi §5.1.
7. **Firma del codice:** va verificata l'idoneità a SignPath.io.

## 14. Milestone

Ogni milestone avrà un proprio piano di implementazione.

1. **Fondamenta:** monorepo, CI, modello dati, scheduler, provider `sys`, shell Tauri con tray minima, vista Semplificata con CPU, RAM, dischi e rete.
2. **GPU:** enumerazione, livello base PDH/D3DKMT, NVML, NVAPI, ADL, IGCL, merge con priorità.
3. **Vista Avanzata:** barra laterale, pagine per componente, grafici uPlot, storico, tabelle con min/max/media.
4. **Servizio:** `oma-service` con LibreHardwareMonitorLib, protocollo IPC con le fixture, installer NSIS con PawnIO, modalità anti-cheat.
5. **Regole e integrazione:** motore regole, banner di stato, notifiche, tray completa, log CSV, impostazioni, traduzioni it/en.
6. **Rifinitura e 1.0:** verifica del budget di prestazioni, "Esporta report sensori", documentazione, licenze di terze parti, release.

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
