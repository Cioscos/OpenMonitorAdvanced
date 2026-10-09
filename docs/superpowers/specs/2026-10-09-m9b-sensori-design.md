# M9b: sensori nuovi

**Stato:** approvata nel brainstorming del 2026-10-09; spike dei registri UMC superato lo stesso giorno (`.superpowers/diag/m9b-umc/`, esclusa da git).

## 1. Obiettivo e ambito

La M9b completa la M9 (monitor) con cinque gruppi di sensori che la spec generale prevede (§5, §7) e che finora mancavano:

1. timing della RAM: i valori SPD di ogni modulo e, sulle CPU AMD Zen, i timing in uso letti dal controller di memoria;
2. la batteria dei portatili: provider, pagina, tessera e regola `battery-low`;
3. energia totale e traffico PCIe delle GPU NVIDIA da NVML;
4. throttling termico e di potenza delle CPU Intel dall'MSR;
5. avviso critico e avviso dei settori dei dischi SATA.

### 1.1 Decisioni del brainstorming

| # | Decisione |
|---|---|
| E1 | Tutti e cinque i gruppi nella M9b, anche quelli che questo PC non può provare (batteria, Intel): coperti da fixture e test con dati finti, collaudo dal vivo in `docs/follow-ups.md`. |
| E2 | Timing della RAM: «tutto». Su ogni CPU i valori SPD (JEDEC) con capacità e limiti termici; sulle AMD Zen anche i timing in uso dal controller (UMC). Su Intel i timing in uso non si leggono: servono i registri MCHBAR e PawnIO non ha un modulo firmato che li esponga. |
| E3 | Dischi SATA nello stile di CrystalDiskInfo: critico se un attributo SMART è sceso alla sua soglia, attenzione se i settori riallocati (05), in attesa (C5) o non correggibili (C6) sono più di 0. |
| E4 | L'energia NVML si mostra come «energia dall'avvio dell'app», in Wh (kWh da 1000 Wh), senza pulsante di azzeramento. |
| E5 | Traffico PCIe ed energia NVML si leggono su un thread loro (approccio 1), mai nel tick. |

### 1.2 Esiti dello spike (UMC, 2026-10-09)

Su questo PC (Ryzen 7 7800X3D, B650 Gaming X AX, AGESA 1.3.0.1c, 2×16 GB DDR5-6000 in A2 e B2) `AmdFamily17.ReadSmn` di LibreHardwareMonitorLib 0.9.6 legge i registri UMC da un processo con privilegi (senza privilegi restituisce 0). La decodifica con la disposizione di ZenStates-Core (`Dictionaries/DDR5Dictionary.cs`, `Hardware/DRAM/Ddr5Timings.cs`, GPL-3.0) coincide con ZenTimings 1.39 campo per campo: CL 36, tRCDRD/tRCDWR 48, tRP 48, tRAS 104, tRC 152, tRRDS 8, tRRDL 16, tFAW 32, tWTRS 8, tWTRL 32, tCWL 34, tWR 72, tRTP 24, tREFI 11677, tRFC 943, tRFC2 512, 1T, GDM attivo, MCLK 3000 MHz. Lo spike leggeva per tRFCsb un solo registro dei quattro e trovava 0 invece di 416: si prende il primo valore diverso da 0 di `0x502C0`–`0x502CC`.

### 1.3 Cosa non cambia

- Il protocollo del servizio resta alla **v4**: sensori e proprietà viaggiano come dati dello schema, senza tipi nuovi nel protocollo.
- Il tick del nucleo, i budget (nucleo < 1% CPU, tray < 30 MB, finestra < 260 MB e < 2,5% CPU) e la cadenza dei grafici.
- Il servizio non legge niente in più quando PawnIO manca o il modulo è spento.
- Nessuna scrittura su MSR, SMN o SMBus: solo letture.

### 1.4 Fuori da questa spec

- I timing in uso su Intel (E2) e sulle APU AMD con LPDDR5 o LPDDR4.
- FCLK e UCLK (vengono dalla tabella PM della SMU, non dall'UMC), tensioni della RAM e i timing secondari oltre a quelli di §2.3.
- Un throttling della CPU per AMD: LibreHardwareMonitor non ha un sensore e la SMU non ha un bit documentato.
- Il collaudo con più di una batteria: ogni batteria ha la sua pagina, ma il caso non si prova.

## 2. Timing della RAM

### 2.1 Dove stanno

Tutto nel servizio, come **proprietà** del device `memory/0` (sono costanti fino al riavvio, non sensori). Due proprietà con un valore JSON, perché i moduli e i canali sono in numero variabile e una chiave per campo farebbe decine di chiavi:

- `memoryModules`: un array, un oggetto per DIMM con l'SPD (§2.2);
- `memoryTimings`: un oggetto con i timing in uso (§2.3), solo su AMD Zen supportate.

Il JSON è camelCase, con numeri e niente stringhe libere tranne lo slot e il tipo. La UI lo convalida (§7.1) e lo ignora se non ha la forma attesa. Le due chiavi entrano in `HIDDEN_PROPERTIES` (non compaiono nella lista generica delle proprietà) e nella lista bianca del report (M6c §4).

### 2.2 SPD (`memoryModules`)

`BuildMemoryDevice` oggi scarta tutto tranne le temperature `DIMM #i`. Con PawnIO e il modulo della memoria acceso, per ogni `/memory/dimm/<i>` legge anche:

| Campo | Da LHM | Note |
|---|---|---|
| `slot` | indice del DIMM | stringa, come «DIMM #1» nei sensori |
| `capacityBytes` | `data` «Capacity» (GB) | × 2³⁰ |
| `tempHighC`, `tempCritC` | `temperature` «High», «Critical High limit» | soglie del sensore del modulo; 0 = assente |
| `tckMinNs`, `taaNs`, `trcdNs`, `trpNs`, `trasNs`, `trcNs`, `trfc1Ns`, `trfc2Ns`, `trfcSbNs` | `timing` per nome | ns; un nome che manca o vale 0 non compare |

Da `tckMinNs` la UI ricava la velocità JEDEC (MT/s = 2000 / tCK) e i timing in cicli (`ceil(t / tCK − 0,01)`, la regola di arrotondamento JEDEC). I nomi esatti dei timing si prendono da `DimmMemory.cs` di LHM 0.9.6 e si fissano in una fixture di `SchemaBuilderTests` (letti dal dump reale di questo PC). Le temperature dei DIMM restano sensori come oggi.

### 2.3 Timing in uso (`memoryTimings`)

Un lettore nuovo nel servizio, `Sensors/UmcTimings.cs`:

- **Quando:** solo con PawnIO, il modulo della memoria acceso e una CPU AMD supportata; una volta alla costruzione dello schema (i timing non cambiano senza riavvio), mai nel tick.
- **CPU supportate:** famiglia 19h modelli 60h–7Fh (Zen 4 desktop, DDR5, provata qui) e 1Ah modelli 40h–4Fh (Zen 5 desktop, DDR5, solo fixture) con la disposizione DDR5; famiglia 17h modello 71h e 19h modello 21h (Zen 2 e Zen 3 desktop, DDR4, solo fixture) con la disposizione DDR4 di ZenStates-Core. Ogni altro modello: niente `memoryTimings`.
- **Lettura:** `AmdFamily17.ReadSmn`, con il mutex globale `Access_PCI` preso intorno a ogni gruppo di letture (lo chiede il modulo PawnIO e lo usano LHM e HWiNFO). Canali da 0 a 11 a `i << 20`; un canale c'è se il bit 19 di `0x50DF0` vale 0 e il bit 0 di `0x50000` o di `0x50008` vale 1.
- **Campi DDR5** (bit da `DDR5Dictionary.cs`): `mclkMhz` (bit 15:0 di `0x50200`, già in MHz: 3000 = 3000 MHz), `dataRate` (× 2), `cmd2t` (bit 17 di `0x50200`), `gdm` (bit 18), `cl`, `trcdRd`, `trcdWr`, `trp`, `tras`, `trc`, `trrdS`, `trrdL`, `tfaw`, `twtrS`, `twtrL`, `tcwl`, `twr`, `trtp`, `trefi`, `trfc`, `trfc2`, `trfcSb` (§1.2), `refreshMode` (`normal`/`fgr`/`mixed`/`pbOnly` da `0x5012C`), più `trfcNs` calcolato (tRFC2 in modo FGR o misto).
- **Campi DDR4:** lo stesso insieme, dalla disposizione DDR4 (`DDR4Dictionary.cs`), senza `trfcSb`.
- **Canali diversi:** se tutti i canali hanno gli stessi valori, `memoryTimings` ne ha una copia sola; altrimenti ha `channels`, un array con un oggetto per canale (lettera A, B, …).
- **Plausibilità:** CL fuori da 4–80, MCLK fuori da 400–5000 MHz o tutti i campi a 0 scartano il canale; nessun canale valido = nessuna proprietà. Si scrive una riga nel log del servizio.
- **Licenza:** offset e campi vengono da ZenStates-Core (GPL-3.0); attribuzione in `THIRD_PARTY_NOTICES.md`, nessun file copiato.

## 3. Batteria

### 3.1 Provider

Un provider nuovo nel nucleo, `crates/oma-win/src/battery.rs` (`BatteryProvider`), senza privilegi:

- **Scoperta:** `SetupDiGetClassDevsW` su `GUID_DEVICE_BATTERY` (`DIGCF_PRESENT | DIGCF_DEVICEINTERFACE`), apertura del percorso dell'interfaccia (`GENERIC_READ | GENERIC_WRITE`, condivisione lettura e scrittura), `IOCTL_BATTERY_QUERY_TAG`. Nessuna batteria = nessun device: su un desktop il provider non pubblica niente e non costa niente.
- **Statico** (`IOCTL_BATTERY_QUERY_INFORMATION`): `BatteryInformation` (chimica, capacità di progetto e piena in mWh, cicli, flag `BATTERY_CAPACITY_RELATIVE`), nome del device e produttore. Diventano proprietà: `designCapacityWh`, `fullChargeCapacityWh`, `wearPct` (100 − piena / progetto × 100), `cycleCount`, `chemistry`, `manufacturer`, `model`. Si rileggono ogni 60 s: un cambio è un cambio di schema, come per le altre proprietà.
- **Dinamico** (`IOCTL_BATTERY_QUERY_STATUS` a ogni tick, più `BatteryEstimatedTime` ogni 10 s):

| Sensore | Kind / nome | Unità | Note |
|---|---|---|---|
| Carica | `percent/charge` | `percent` | capacità attuale / piena × 100; con `BATTERY_CAPACITY_RELATIVE` è il valore riportato |
| Potenza | `power/rate` | `watt` | positiva in carica, negativa in scarica; `BATTERY_UNKNOWN_RATE` = assente |
| Tensione | `voltage/voltage` | `volt` | `BATTERY_UNKNOWN_VOLTAGE` = assente |
| Energia residua | `energy/remaining` | `joule` | mWh × 3,6; assente con capacità relativa |
| Tempo stimato | `duration/remaining` | `seconds` (nuova) | solo in scarica; `BATTERY_UNKNOWN_TIME` = assente |
| In carica | `flag/charging` | `boolean` | `BATTERY_CHARGING` |
| Alimentatore | `flag/ac-online` | `boolean` | `BATTERY_POWER_ON_LINE` |

- **Id:** `battery/<n>`, con `n` l'ordine di enumerazione. Se il tag cambia (batteria tolta e rimessa) il provider rilegge il tag; se la batteria sparisce i valori sono assenti e la scoperta successiva la toglie.
- **Kind e unità nuovi:** `SensorKind::Duration` (`duration`) e `Unit::Seconds` (`seconds`) nel modello del nucleo e in `types.ts`; la UI li mostra come «1 h 23 min» (riusa `duration.hoursMinutes`). Il servizio non li produce, quindi il protocollo non cambia.

### 3.2 Regola `battery-low`

Esiste già (soglie 15% e 5%) e finora non aveva istanze. Vale solo in scarica: la regola non scatta finché `flag/ac-online` dello stesso device vale 1. È una condizione fissa della regola `battery-low` nel motore delle regole, non un campo nuovo del modello delle regole (le regole dell'utente non cambiano formato).

### 3.3 Interfaccia

- **Vista Avanzata:** la voce «Batteria» compare da sola quando c'è il device (`nav.ts` la ha già). KPI: carica, potenza, tempo stimato, usura. Serie predefinite del grafico: carica e potenza. Le informazioni del device mostrano capacità, usura, cicli, chimica, produttore e modello.
- **Vista Semplice:** una tessera «Batteria» solo sui portatili (spec generale §7.2), dopo le altre: carica come valore principale con la barra (animata con `transform`, mai `width`), stato «In carica» / «In scarica» / «Collegata», tempo stimato e potenza, minigrafico della carica. La griglia di `simpleGrid.ts` gestisce già 5 e 6 tessere.

## 4. NVML: energia e traffico PCIe

### 4.1 Lettura fuori dal tick

`nvmlDeviceGetPcieThroughput` blocca per circa 30 ms (campiona 20 ms nel driver) e `nvmlDeviceGetTotalEnergyConsumption` ha un p95 di circa 9 ms: nel tick farebbero scattare il grafico. Il provider della GPU ha quindi un thread suo, `oma-gpu-slow`, uno solo per tutte le GPU NVIDIA:

- **Ritmo:** un giro al secondo; per ogni GPU legge energia, RX e TX.
- **Domanda:** il tick segna l'ora dell'ultimo `poll`; se non arrivano `poll` da 3 s il thread si addormenta su una condition variable e il tick lo risveglia. Con l'app in tray e nessun consumatore il thread dorme, come il tick.
- **Valori al tick:** il tick prende l'ultimo valore senza aspettare (un `Mutex` breve, nessuna chiamata NVML nel tick). Un valore più vecchio di 3 s è assente.
- **NVML:** lo stesso caricamento di `NvmlLayer` (una sola istanza della libreria per processo, già così); il thread usa gli handle dei device già aperti.
- **Supporto:** l'energia esiste da Volta, il throughput da Maxwell; il primo errore `NOT_SUPPORTED` toglie quel sensore dalla GPU (sondato in `attach`, come gli altri campi).

### 4.2 Sensori

| Sensore | Kind / nome | Unità | Note |
|---|---|---|---|
| Energia dall'avvio | `energy/total` | `joule` | mJ / 1000, contata dalla prima lettura riuscita (E4). Se il contatore torna indietro (driver ricaricato), si somma quanto già contato e si riparte dalla nuova base. |
| PCIe in ricezione | `throughput/pcie-rx` | `bytes_per_second` | KB/s × 1024 |
| PCIe in trasmissione | `throughput/pcie-tx` | `bytes_per_second` | KB/s × 1024 |

Sono sensori normali: tabella, grafico, regole dell'utente e log CSV. Non entrano tra le serie predefinite del grafico della GPU. L'unità del traffico segue l'impostazione byte/bit.

## 5. Throttling della CPU Intel

Nel servizio, solo con PawnIO e una CPU Intel:

- **Registro:** `IA32_PACKAGE_THERM_STATUS` (0x1B1), l'unico dei due candidati permesso dal modulo PawnIO `IntelMSR` (`MSR_CORE_PERF_LIMIT_REASONS`, 0x64F, non è nella lista).
- **Sensori:** per ogni package, letto con l'affinità di un thread di quel package:
  - `flag/throttle-thermal` dal bit 0 (stato termico in corso): accende la regola `cpu-throttle`, finora senza istanze;
  - `flag/throttle-power` dal bit 10 (limitazione di potenza del package in corso), solo se `CPUID.06H:EAX[4]` (PLN) è 1.
- **Quando:** a ogni tick del servizio (una lettura MSR per package, costo trascurabile). Se la prima lettura alla costruzione dello schema fallisce, i due sensori non ci sono.
- **Etichette:** le stesse chiavi i18n delle GPU (`*.throttle.thermal`, `*.throttle.power`) con prefisso `cpu`.

## 6. Dischi SATA

Nel servizio, nello stesso passaggio SMART dei dischi ATA (`_tree.Update`), quindi mai con lo SMART del disco spento e mai con un HDD fermo o in standby (i valori restano quelli precedenti, come gli altri sensori SMART). Solo dischi non NVMe con attributi SMART.

Una funzione pura, `SataHealth.Evaluate(IReadOnlyList<SmartAttribute>)`, testata con fixture:

- **critico:** un attributo con `Threshold` > 0 e `CurrentValue` ≤ `Threshold`;
- **attenzione:** i 32 bit bassi del valore grezzo di 05h, C5h o C6h > 0 (E3).

| Sensore | Kind / nome | Unità | Regola |
|---|---|---|---|
| Avviso critico | `flag/critical-warning` | `boolean` | `disk-critical`, già esistente (stesso nome dell'NVMe) |
| Avviso settori | `flag/sector-warning` | `boolean` | `disk-sectors`, nuova: livello di attenzione, durata 0 |
| Settori riallocati | `counter/reallocated-sectors` | `count` | — |
| Settori in attesa | `counter/pending-sectors` | `count` | — |
| Settori non correggibili | `counter/uncorrectable-sectors` | `count` | — |

Un contatore compare solo se il disco ha quell'attributo. Non si usa `SmartInfo.DiskStatus` di DiskInfoToolkit: la regola di E3 deve essere esplicita e testabile. La regola `disk-sectors` entra nelle regole predefinite e nella fixture `default-rules.json`, letta dai test Rust e TypeScript.

## 7. Interfaccia comune

### 7.1 Pagina della RAM

Un componente nuovo, `components/advanced/MemoryTimings.svelte`, sotto il grafico della pagina della RAM:

- **«Timing in uso»** (solo con `memoryTimings`): velocità e MCLK, poi la riga breve «CL-tRCD-tRP-tRAS» (es. «36-48-48-104»), tRC, tRFC (cicli e ns), 1T/2T, GDM e modo di refresh, poi i secondari di §2.3 in una griglia. Con canali diversi, una colonna per canale.
- **«Moduli (SPD)»** (con `memoryModules`): una riga per modulo con slot, capacità, velocità JEDEC, CL-tRCD-tRP-tRAS JEDEC in cicli e soglie termiche.
- **Tooltip** (`Term.svelte`) su ogni sigla (CL, tRCD, tRP, tRAS, tRC, tRFC, GDM, 1T/2T, JEDEC, SPD, MCLK, refresh) e una nota fissa sulla sezione SPD: «Valori di specifica del modulo (JEDEC): con EXPO o XMP attivo la RAM usa i timing in uso, non questi».
- **Senza servizio o senza PawnIO:** la sezione non c'è; resta l'avviso generico di oggi («Temperature, tensioni e altri sensori disponibili con il servizio»).
- **Convalida:** `lib/advanced/memoryTimings.ts`, funzione pura `parseMemoryTimings(props)` → oggetto tipizzato o `null`; numeri finiti e non negativi, array con al massimo 16 moduli e 12 canali; tutto il resto si scarta. Il testo si mostra sempre come testo, mai `{@html}`.

### 7.2 Etichette e tooltip

Ogni sensore nuovo ha la sua chiave i18n in `en.json` e `it.json`; ogni termine tecnico nuovo (PCIe RX/TX, energia dall'avvio, usura, cicli, settori riallocati / in attesa / non correggibili, limitazione di potenza) ha un tooltip in parole semplici.

## 8. Prestazioni

- **Nucleo:** il provider della batteria fa un `DeviceIoControl` per tick e solo dove c'è una batteria; il thread NVML dorme 1 s fra un giro e l'altro (circa 60 ms di attesa bloccata nel driver, non CPU). Il nucleo resta sotto l'1% di CPU.
- **Servizio:** UMC una volta per schema; MSR una lettura per package per tick; SATA dentro il passaggio SMART esistente.
- **Misura:** `scripts/measure-footprint.ps1` a fine milestone, a riposo e con la finestra aperta sulla pagina della GPU con le serie PCIe nel grafico; i valori vanno in `docs/perf-budget.md`.

## 9. Test e verifiche

### 9.1 Test automatici

- **Servizio (xUnit):** fixture di `SchemaBuilderTests` con il dump SPD reale di questo PC; `UmcTimings` con dump dei registri (il DDR5 di §1.2 come fixture vera, DDR4 e Zen 5 costruite dalla disposizione di ZenStates-Core), canali diversi, valori implausibili, CPU non supportata; `SataHealth.Evaluate` con soglia raggiunta, settori, attributi assenti, disco NVMe escluso; throttling Intel con letture MSR finte (bit 0, bit 10, PLN assente, lettura fallita).
- **Nucleo (Rust):** helper puri della batteria (decodifica di `BATTERY_STATUS` e `BATTERY_INFORMATION`, valori sconosciuti, capacità relativa, usura); contatore dell'energia (base, ritorno indietro); domanda e scadenza del thread lento con un orologio finto; regola `battery-low` sospesa con `ac-online`; regola `disk-sectors` nella fixture.
- **FFI:** assert di dimensione a compile time per `BATTERY_QUERY_INFORMATION`, `BATTERY_INFORMATION`, `BATTERY_STATUS`, `BATTERY_WAIT_STATUS`; commento `// SAFETY:` su ogni `unsafe`; revisione `ffi-safety-reviewer`.
- **UI (Vitest):** `parseMemoryTimings` (forma valida, campi mancanti, valori assurdi, troppi moduli), rendering di `MemoryTimings`, tessera e pagina della batteria, formattazione di `seconds`, chiavi i18n uguali nelle due lingue.

### 9.2 Prove dal vivo su questo PC

1. Pagina RAM con il servizio e PawnIO: i timing in uso coincidono con ZenTimings (§1.2, tRFCsb 416 compreso); la sezione SPD mostra i due moduli.
2. Pagina GPU: energia che cresce, RX/TX che salgono in un gioco o nel benchmark della GPU; nessuno scatto del grafico a 60 FPS; CPU del nucleo nel budget.
3. Disco SATA (se presente): i contatori dei settori compaiono, avvisi spenti; con l'HDD in standby il disco resta fermo.
4. Senza batteria: nessuna voce «Batteria», nessuna tessera.

### 9.3 Da collaudare altrove (`docs/follow-ups.md`)

- Batteria su un portatile: carica, potenza con il segno, tempo stimato, usura, regola `battery-low` solo in scarica.
- Throttling Intel su un PC Intel: termico sotto carico con dissipatore limitato, potenza con PL1 basso.
- Timing in uso su Zen 2, Zen 3 e Zen 5.
- Un disco SATA con settori riallocati o un attributo sotto soglia.
