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
- **`Sensor`**: `id` stabile, `device_id`, `kind` (`temperature`, `load`, `clock`, `power`, `voltage`, `current`, `fan`, `data`, `throughput`, `energy`, `flag`, `percent`), `unit`, `label` (chiave i18n oppure testo fornito dal driver), `source` (`pdh`, `d3dkmt`, `nvml`, `nvapi`, `adl`, `igcl`, `lhm`, `wmi`…), `category` (per il raggruppamento nella tabella della vista Avanzata).
- **`Reading`**: `sensor_id`, `value: f64` oppure assente, `timestamp`.
- **ID stabili tra riavvii**, costruiti dall'identità hardware. Esempi: `gpu/pci-0000:01:00.0/temperature/hotspot`, `storage/nvme-<seriale-hash>/temperature/composite`. Regole, impostazioni e selezione dei sensori per il log vi fanno riferimento.
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
- Min, max e media partono dall'avvio dell'app e si possono azzerare dall'interfaccia.
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
| Temperatura NVMe (fallback) | `IOCTL_STORAGE_QUERY_PROPERTY` con `StorageDeviceTemperatureProperty`, se accessibile senza privilegi |
| Rete | `GetIfTable2` (byte, velocità del collegamento), con filtro sulle interfacce fisiche e attive |
| Wi-Fi | `WlanQueryInterface` con `wlan_intf_opcode_rssi`. SSID e qualità della connessione richiedono il consenso alla posizione su Windows 11 24H2 o successivi: se negato vengono omessi. |
| Batteria | `GetSystemPowerStatus`, `IOCTL_BATTERY_QUERY_INFORMATION/STATUS` (capacità di progetto e attuale, cicli, potenza) |
| Sistema | `GetSystemFirmwareTable('RSMB')` per SMBIOS (scheda madre, BIOS); `RtlGetVersion` per la versione di Windows |

### 5.2 Provider `gpu` (senza privilegi, a strati)

1. **Enumerazione:** DXGI e D3DKMT per LUID e ID PCI. Si scartano gli adattatori software (Microsoft Basic Render Driver). Ogni GPU ha un solo record, identificato da LUID e indirizzo PCI.
2. **Livello base, per ogni vendor, sempre attivo:**
   - PDH `GPU Engine(*)`: utilizzo per motore (3D, compute, copy, video encode/decode) e per processo. Si sommano le istanze per LUID e motore, poi si prende il massimo per tipo di motore, come fa Task Manager.
   - PDH `GPU Adapter Memory` e `GPU Process Memory`.
   - D3DKMT `KMTQAITYPE_ADAPTERPERFDATA`: temperatura, frequenza della memoria, potenza in percentuale del limite (non in watt).
   - DXGI `QueryVideoMemoryInfo`.
3. **Arricchimento con le librerie dei vendor**, tutte caricate dinamicamente con `LoadLibraryExW(..., LOAD_LIBRARY_SEARCH_SYSTEM32)` e con i simboli risolti uno alla volta. Un simbolo mancante o `NOT_SUPPORTED` significa "sensore assente".
   - **NVIDIA — NVML:** clock, potenza e limiti, motivi del throttling (`ClocksEventReasons`), PCIe (generazione e larghezza attuali e massime), utilizzo di encoder e decoder, ventola in %.
   - **NVIDIA — NVAPI:** temperature hotspot e memory junction, RPM della ventola, tensione. Le chiamate per hotspot e junction non sono documentate, quindi sono segnate come **sperimentali** (stesso approccio di LibreHardwareMonitor).
   - **AMD — ADL (legacy, `atiadlxx.dll`):** Overdrive e PMLog per hotspot, memoria, VRM, potenza, ventola e tensione. **ADLX è escluso**, perché la sua licenza vieta l'uso in software con licenza libera.
   - **Intel — IGCL (`ControlLib.dll`, solo a 64 bit):** `ctlPowerTelemetryGet` per energia, tensione, clock, temperature, attività, ventole e flag dei limiti (usati come motivi del throttling); `ctlPciGetState`.
4. **Priorità nel merge:** libreria del vendor → D3DKMT → PDH, con la fonte registrata per ogni campo. Utilizzo per motore e per processo vengono sempre da PDH.

**Licenze:**
- Nessun header proprietario viene incluso nel repository, a meno che la licenza lo consenta: l'SDK di NVAPI è MIT.
- Per NVML, ADL e IGCL si scrivono binding propri a partire dalla documentazione pubblica, oppure si scaricano gli header al momento della build.
- Le DLL dei vendor non vengono mai ridistribuite: si caricano da quelle installate con il driver.

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
  - **4 KPI** in alto, definiti per tipo di componente;
  - un **grafico storico** uPlot, con finestra 1m / 5m / 30m / 1h e con le serie da mostrare selezionabili;
  - una **tabella dei sensori** raggruppata per categoria (temperature, carico, clock, potenza, tensioni, ventole…), con colonne attuale, min, max e media, e un pulsante "azzera min/max";
  - un **badge della fonte** su ogni sensore, visibile passandoci sopra.
- **I sensori che richiedono il servizio**, quando questo non è attivo, non compaiono uno per uno. Al loro posto c'è un solo avviso: "N sensori in più disponibili con il servizio".
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
  - buffer tipizzati (`Float64Array`).

### 7.6 Internazionalizzazione

Stringhe in file JSON per lingua (`en`, `it`), con l'inglese come lingua di riserva. Anche i messaggi del banner e delle notifiche, generati dal nucleo, usano chiavi di traduzione con parametri. Un test verifica che le due lingue abbiano le stesse chiavi.

## 8. Gestione errori

- **Provider isolati:**
  - ogni errore mette il provider in stato "degradato", con nuovi tentativi a intervalli crescenti (5 s → 60 s);
  - i suoi sensori mostrano "—" e l'indicazione della fonte non disponibile;
  - un provider non può bloccare lo scheduler.
- **Librerie dei vendor:** un crash dentro una DLL non si può intercettare. Per questo ci sono:
  - un interruttore per ogni provider nelle impostazioni;
  - l'avvio `--safe`, che disattiva tutti gli SDK dei vendor e lascia solo PDH e D3DKMT.
  
  Se l'avvio precedente non si è concluso correttamente, l'app propone la modalità sicura.
- **Servizio:** disconnessioni, timeout e versione del protocollo non compatibile portano alla modalità base, con badge e spiegazione. Non producono mai errori bloccanti.
- **Dati anomali:** valori fuori dall'intervallo fisico plausibile vengono scartati come assenti e registrati nel log di diagnostica. Esempi: temperature < −50 °C o > 150 °C, percentuali < 0 o > 100 dove non ha senso.
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
3. **Valore corretto dell'enum per `D3DKMT_NODE_PERFDATA`** (clock ed eventuale tensione per motore): una prima prova ha restituito `STATUS_INVALID_PARAMETER`.
4. **Driver Intel e Qualcomm e `ADAPTERPERFDATA`:** lo popolano?
5. **Licenza di ADL (legacy):** va verificata prima di usarne i binding; in alternativa si resta sul livello base per AMD.
6. **NVMe via `IOCTL_STORAGE_QUERY_PROPERTY` senza privilegi:** funziona?
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
