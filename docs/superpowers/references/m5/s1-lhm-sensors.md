# S1 (M5b) — Sensori di LibreHardwareMonitor per le regole: throttling CPU, critical warning dei dischi, TjMax

Spike in sola lettura richiesto da §3.1 della spec M5 (`docs/superpowers/specs/2026-09-29-m5-regole-integrazione-design.md`, con §3.3 e la decisione B8 di §1.1). Nessun codice di prodotto è stato modificato; le sonde usa e getta (riflessione sulle API pubbliche, lettura del log NVMe) stanno nello scratchpad della sessione e non sono nel repository.

## Fonti

Codice locale al commit `d3a7667` (riferimenti `file:riga`). Sorgenti di terzi letti online, fissati come nello spike S1 della M4 (`docs/superpowers/references/m4/s1-lhm.md` §6):

- **LHM** = `https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/v0.9.6/LibreHardwareMonitorLib/Hardware/` (tag `v0.9.6`; l'assembly del pacchetto NuGet in uso riporta `InformationalVersion` `0.9.6+3d331e3370efb858411f19511373eff65a218701`);
- **DIT** = `https://github.com/Blacktempel/DiskInfoToolkit/blob/25319eae5781e75bcf141e844ceab2afe94d40ea/DiskInfoToolkit/` (l'assembly di DiskInfoToolkit 1.1.2 riporta `1.1.2+25319eae5781e75bcf141e844ceab2afe94d40ea`, lo stesso commit).

I percorsi sotto sono relativi a queste radici; i numeri di riga si leggono con `#L<a>-L<b>` sull'URL. Il codice di terzi è citato, non copiato.

Documentazione dei produttori (citata per sezione, senza riprodurne il testo):

- **Intel SDM** = Intel 64 and IA-32 Architectures Software Developer's Manual, Vol. 3B, capitolo "Power and Thermal Management", sezione "Thermal Monitoring and Protection"; Vol. 4, voci `IA32_THERM_STATUS` (19CH), `IA32_PACKAGE_THERM_STATUS` (1B1H), `MSR_TEMPERATURE_TARGET` (1A2H);
- **NVMe** = NVM Express Base Specification 2.0, comando Get Log Page, "SMART / Health Information (Log Page Identifier 02h)", byte 0 "Critical Warning";
- **MS-NVMe** = `https://learn.microsoft.com/en-us/windows/win32/fileio/working-with-nvme-devices` (`IOCTL_STORAGE_QUERY_PROPERTY` con `StorageDeviceProtocolSpecificProperty` e `NVMeDataTypeLogPage`);
- **AMD** = tabella ufficiale delle specifiche `https://www.amd.com/en/products/specifications/processors.html` (attributo `data-json` della tabella, campo `maxOperatingTemperatureTjmax`, titolo "Max. Operating Temperature (Tjmax)") e pagine prodotto `https://www.amd.com/en/products/processors/desktops/ryzen/…`, lette il **2026-09-30**.

Misure su questa macchina (Ryzen 7 7800X3D, `ProcessorNameString` = `AMD Ryzen 7 7800X3D 8-Core Processor` con spazi di riempimento in coda, `AMD64 Family 25 Model 97 Stepping 2`), processo non elevato (livello di integrità medio, `S-1-16-8192`).

## Sintesi dei verdetti

1. **Throttling termico della CPU come `flag`: nessun sensore in LHM 0.9.6, né Intel né AMD.** Intel legge `IA32_THERM_STATUS`/`IA32_PACKAGE_THERM_STATUS` ma espone solo il digital readout (bit 22:16); il bit 0 (stato attuale) e il bit 1 (log appiccicoso) non diventano sensori. AMD (`Amd17Cpu`, tabelle PM dello SMU) non ha nulla di simile. `cpu-throttle` resta **senza mapping**, con il limite dichiarato.
2. **Critical warning dei dischi: esiste solo per NVMe, ed è un attributo SMART di DIT, non un sensore LHM.** Il byte 0 del log SMART/Health è `SmartAttributeType.CriticalWarning` in `Storage.Smart.SmartAttributes` (API pubblica), rinfrescato dallo stesso `Update()` che il worker dello storage già esegue, senza comandi in più. I sensori oggi scartati da `MatchStorageSensor` ("Warning Temperature", "Critical Temperature") sono le **soglie** WCTEMP/CCTEMP, non il critical warning: lo scarto è corretto. Per SATA/HDD non c'è un equivalente (DIT non invia SMART RETURN STATUS; il suo `DiskStatus` è un'euristica). In più, **il core non privilegiato può leggere da sé il log NVMe**: provato qui su PhysicalDrive2 e 3 con accesso 0, 2–7 ms a lettura.
3. **TjMax Intel: sì, esatto, come parametro pubblico del sensore.** Il sensore "CPU Package" (e ogni core) ha il parametro "TjMax [°C]" (`ISensor.Parameters[0]`), letto da `MSR_TEMPERATURE_TARGET` per tutte le microarchitetture da Nehalem in poi. Esistono anche i sensori "`<core>` Distance to TjMax" per core (non per il package): temperatura + distanza dello stesso core = TjMax esatto, ma è ridondante con il parametro. Non validabile su questa macchina.
4. **Tabella TjMax AMD (B8): 164 voci desktop verificate su fonte ufficiale AMD** (101 Ryzen consumer 3000–9000 con APU 4000G/5000G/8000G e X3D, 63 Ryzen PRO desktop), di cui 31 anche sulla pagina prodotto; **2 voci non verificabili** (Ryzen 3 3100 e 3300X: AMD non pubblica il valore). Mobile, Threadripper e Ryzen 1000/2000 restano fuori (motivi in §4.5). Identificazione per chiave normalizzata dalla stringa di marca CPUID, con controllo della famiglia CPUID; il valore si associa a `cpu/0/temperature/tctl` ("Core (Tctl/Tdie)"), mai a un Tctl con offset.
5. **Id obiettivo su questa macchina:** `cpu-temp` → `cpu/0/temperature/tctl` (con `tjMaxC` = 89); `cpu-throttle` → nessuno; `disk-temp` → `…/temperature/drive` dei quattro dischi; `disk-wear` → `…/percent/wear` dei due NVMe; `disk-critical` → `…/flag/critical-warning` (nuovo) dei due NVMe. GPU, RAM e volumi vengono dal core non privilegiato.
6. **Fixture:** CPU AMD con e senza offset, CPU Intel non ibrida e ibrida (con i nomi reali di LHM, che oggi il mapping non riconosce), stringhe di marca ambigue, NVMe con critical warning, SATA senza. Elenco in "Per il piano M5b".

---

## 1. Throttling termico della CPU

**Intel (LHM `Cpu/IntelCpu.cs`).** `Update()` (`:564-625`) legge per ogni core `IA32_THERM_STATUS` (`:575`, costante `0x019C` a `:759`) e per il package `IA32_PACKAGE_THERM_STATUS` (`:607`, `0x1B1` a `:756`), controlla il bit 31 "reading valid" ed estrae soltanto i bit 22:16 (`:577-587`, `:609-613`), cioè la distanza da TjMax. Secondo l'Intel SDM (Vol. 4) in quegli stessi registri il bit 0 è lo stato termico **attuale** (TCC/PROCHOT attivo ora) e il bit 1 è il **log** appiccicoso (resta a 1 finché il software non lo azzera): LHM non li espone. Nessun sensore di LHM ha "throttl"/"PROCHOT" nel nome (ricerca su `Cpu/*.cs`, `RyzenSMU.cs`). L'unica occorrenza di "Throttle" nella libreria è `StorageDevice.ThrottleInterval` (`Storage/StorageDevice.cs:63`), che non c'entra.

**AMD (LHM `Cpu/Amd17Cpu.cs`, `RyzenSMU.cs`).** Le temperature vengono da `THM_TCON_CUR_TMP` (`Amd17Cpu.cs:188-190`, costante `0x00059800` a `:817`) e dalle CCD (`:310-335`); i sensori dello SMU sono quelli delle tabelle PM per versione (`RyzenSMU.cs:19-124`: TDC, EDC, clock, tensioni, temperature "Package", "SoC", "L3 (CCDn)", "IOD Hotspot"…). Nessuno è uno stato di throttling. Su questa macchina la tabella PM non ha un layout definito (le 64 righe CPU del dump M4 §9.1 non contengono sensori SMU).

**Via alternativa, non per la M5b.** La classe `LibreHardwareMonitor.PawnIo.IntelMsr` è pubblica (verificato per riflessione sull'assembly 0.9.6), quindi il servizio potrebbe leggere da sé il bit 0 di `IA32_PACKAGE_THERM_STATUS` con PawnIO. È codice nuovo, solo Intel e non verificabile qui: resta un follow-up.

**Verdetto.** `cpu-throttle` resta senza mapping e senza istanze (spec §3.1: "le regole senza evidenza restano senza mapping"). Il limite va dichiarato: "Il throttling termico della CPU non è disponibile con LibreHardwareMonitor 0.9.6".

## 2. Critical warning dei dischi

### 2.1 Cosa scarta oggi `MatchStorageSensor`

`MatchStorageSensor` (`service/OpenMonitorAdvanced.Service/Sensors/SchemaBuilder.cs:483-486`) scarta le temperature il cui nome inizia con "Warning" o "Critical". In LHM sono esattamente due sensori, creati solo per NVMe: "Warning Temperature" (indice 10) e "Critical Temperature" (indice 11) (LHM `Storage/StorageDevice.cs:177-178`), con valore `Smart.TemperatureWarning`/`Smart.TemperatureCritical`. DIT li ricava da WCTEMP/CCTEMP di Identify Controller, con 70/75 °C al posto di uno zero e troncando la conversione da Kelvin (DIT `Disk/DiskHandler.cs:249-250`): da qui il grado in meno rispetto al core (M4 §9.7). Sono **soglie**, non il critical warning: lo scarto va mantenuto, e le soglie restano le proprietà `tempWarningC`/`tempCriticalC` del core (`crates/oma-win/src/storage_temperature.rs:121-135`). Nota: il nome reale è "Warning Temperature", non "Warning" come scritto nella tabella di M4 §9.1; il `StartsWith` lo copre comunque.

### 2.2 Dove sta il byte Critical Warning

- DIT, a ogni aggiornamento NVMe riuscito, ricostruisce la lista degli attributi dal buffer del log SMART/Health (`NVMe/NVMeInterpreter.cs:25-58`, chiamato da `Disk/DiskHandler.cs:34-66`): il primo è `SmartAttributeType.CriticalWarning` dal byte 0, lungo 1 byte (`NVMeInterpreter.cs:29`), con id `0x01` e nome "Critical Warning" (`Smart/SmartAttributeInfoMapping.cs:93`).
- LHM lo traduce in un `SmartAttribute` **senza** `SensorType` (`Storage/SmartAttributeTranslator.cs:755`, contro `AvailableSpare`/`PercentageUsed` a `:757-759` che hanno `SensorType.Level`), quindi non diventa mai un sensore. Resta raggiungibile in due modi pubblici:
  - `StorageDevice.Attributes` (`IReadOnlyList<LibreHardwareMonitor.Hardware.Storage.SmartAttribute>`, `Storage/StorageDevice.cs:61`), aggiornato per id in `Update()` (`:79-90`), con `Value` = `RawValueULong` come `float` (`Storage/SmartAttribute.cs:48`);
  - direttamente in DIT: `((StorageDevice)hw).Storage.Smart.SmartAttributes`, elemento con `Info.Type == SmartAttributeType.CriticalWarning`, valore `Attribute.RawValue[0]` (`byte`).
- API verificata per riflessione sugli assembly in uso: `DiskInfoToolkit.Storage.Smart` (`SmartInfo`, pubblico), `SmartInfo.SmartAttributes` (`List<DiskInfoToolkit.SmartAttribute>`), `SmartAttribute.Info` (`SmartAttributeInfo`: `byte ID`, `SmartAttributeType Type`, `string Name`), `SmartAttribute.Attribute` (`SmartAttributeStructure`: campo `byte[] RawValue`, proprietà `RawValueULong`), enum `DiskInfoToolkit.Interop.Enums.SmartAttributeType` pubblico (`CriticalWarning` = 67), `Storage.IsNVMe` pubblico.
- **Nessun comando in più.** Il byte viene dal log che `Storage.Update()` legge già (`Storage.cs:294-303` → `DiskHandler.UpdateSmartInfo`), cioè dallo stesso `StorageDevice.Update()` che il worker dello storage esegue ogni 30 s (`Storage/StorageDevice.cs:65-94`).
- **Id da usare:** sempre `Info.Type == CriticalWarning` e `Storage.IsNVMe`, mai l'id `0x01` da solo, che per ATA è "Read Error Rate".
- **Valore vecchio se la lettura fallisce.** Se il comando NVMe fallisce, `UpdateSmartInfo` non azzera la lista (per NVMe `NVMeSmart` fa `Clear()` solo su lettura riuscita), quindi resta l'ultimo valore letto. Lo stesso accade già a temperatura, vita e usura, e non è un problema nuovo.

### 2.3 Significato e forma del flag

Secondo la specifica NVMe il byte ha un bit per condizione: 0 spare sotto soglia, 1 temperatura oltre la soglia di sovratemperatura (o sotto quella di sottotemperatura), 2 affidabilità degradata, 3 supporto in sola lettura, 4 backup della memoria volatile fallito, 5 PMR in sola lettura; 6-7 riservati. Il bit 1 è **transitorio** (si spegne quando la temperatura rientra) e la soglia di sovratemperatura predefinita è WCTEMP: con `disk-critical` a 0 s e livello `crit`, un disco caldo andrebbe in critico proprio mentre `disk-temp` va in attenzione.

Il modello vuole i flag a 0/1 (spec §3.4: `flagActive` = valore ≠ 0; §4.3: nel CSV i flag come 0/1), quindi il servizio pubblica `(byte & mask) != 0 ? 1 : 0`. Proposta: `mask = 0x3D` (bit 0, 2, 3, 4, 5), lasciando la temperatura a `disk-temp`. È una scelta da confermare nel piano; l'alternativa è `mask = 0xFF` con il bit 1 documentato.

### 2.4 SATA e HDD

Nessun equivalente:

- LHM crea sensori solo per gli attributi SMART delle tabelle per vendor, e nessuno è uno stato;
- DIT non invia mai `SMART RETURN STATUS` (sottocomando `0xDA`): in `Interop/InteropConstants.cs` ci sono solo `READ_ATTRIBUTES`, `READ_THRESHOLDS` e `ENABLE_SMART` (`:83-85`);
- `SmartInfo.DiskStatus` (`Good`/`Caution`/`Bad`/`Unknown`, `SmartInfo.cs:27`) è un'euristica alla CrystalDiskInfo (`Disk/DiskHandler.cs:1007-1345`): attributo corrente sotto soglia, regole per vendor e soluzioni per bug noti, compreso un controllo dei settori riallocati che non scatta mai (`:1140-1142`, tre uguaglianze in `&&`). Non è l'avviso del disco.

Per la M5b non si mappa niente su SATA/HDD, e il limite va dichiarato: "Il critical warning è disponibile solo per i dischi NVMe". Un eventuale `…/flag/health-bad` da `DiskStatus == Bad` sarebbe una regola diversa, da discutere a parte.

### 2.5 Lettura dal core non privilegiato (solo NVMe)

Sonda usa e getta in C# (scratchpad), processo non elevato. Per ogni disco prima il descrittore (`StorageDeviceProperty`), e il log solo se `BusType == 17` (NVMe). Poi `IOCTL_STORAGE_QUERY_PROPERTY` con `StorageDeviceProtocolSpecificProperty` (50), `ProtocolTypeNvme`, `NVMeDataTypeLogPage`, `ProtocolDataRequestValue = 0x02`, 512 byte (MS-NVMe), su `\\.\PhysicalDriveN` aperto con **accesso 0** come fa già `storage_ioctl.rs`. Toccati solo PhysicalDrive2 e 3, nessun comando ai SATA.

| N | Descrittore | Esito | Critical Warning | Composite | Spare / soglia | Percentage Used | Tempo per lettura |
|---|---|---|---|---|---|---|---|
| 2 | Fanxiang S880 2TB, bus 17 | OK, 560 byte, dati a offset 40 | `0x00` | 46–47 °C | 100 % / 1 % | 5 % | 6,7 ms (media su 20) |
| 3 | SHPP41-2000GM, bus 17 | OK, 560 byte, dati a offset 40 | `0x00` | 46 °C | 100 % / 10 % | 0 % | 2,2 ms (media su 20) |

I valori coincidono con quelli di LHM elevato (M4 §9.1: soglie 1 e 10 %, usura 5 e 0 %). Quindi il core potrebbe pubblicare da sé `…/flag/critical-warning`, e anche `…/percent/wear` e `…/percent/available-spare` dei NVMe, senza servizio. Se lo fa anche il servizio, il merge "core per primo" scarta il duplicato. È una deviazione dalla spec ("Richiede il servizio" per `disk-wear` e `disk-critical`) da decidere col piano; il costo, 2–7 ms ogni 30 s per disco, rientra nel budget. Un comando admin può risvegliare brevemente un NVMe da uno stato APST, cosa che la spec non vieta (vieta solo di risvegliare SATA/HDD).

## 3. TjMax Intel

- **Parametro.** Ogni sensore di temperatura dei core e "CPU Package" nasce con due `ParameterDescription`: "TjMax [°C]" e "TSlope [°C]" (LHM `Cpu/IntelCpu.cs:379-386`, `:401-408`). Il valore è `tjMax[i]`, calcolato nel costruttore. Per Core 2, Atom 45 nm e NetBurst viene da tabelle fisse (`:53-87`, `:259-282`). Per Nehalem e tutte le famiglie successive viene da `GetTjMaxFromMsr()` (`:89-249`), che legge i bit 23:16 di `MSR_TEMPERATURE_TARGET` (`:531-543`, costante `0x01A2` a `:758`) e **ripiega silenziosamente su 100** se la lettura MSR fallisce (`:539`). `ISensor.Parameters` e `IParameter.Value` sono pubblici (riflessione). Il servizio crea `new Computer()` senza impostazioni (`service/OpenMonitorAdvanced.Service/Sensors/LhmTree.cs:317`), quindi i parametri restano quelli di default.
- **Temperatura.** `Value = TjMax − TSlope × readout` (`:577-581` per core, `:609-613` per package), con TSlope = 1.
- **Distanze.** Se la CPU ha il sensore digitale per core, LHM aggiunge per ogni core "`<nome core>` Distance to TjMax", `SensorType.Temperature` (`:415-429`), con valore uguale al readout (`:587`). Per il package non c'è distanza.
- **Esattezza.** Temperatura del core k + distanza del core k = `tjMax[k]` esatto, perché sono calcolate dallo stesso registro nello stesso `Update()` e con TSlope = 1. Il package usa `tjMax[0]`. La derivazione non aggiunge nulla al parametro: `tjMaxC` si legge direttamente da `Parameters` del sensore "CPU Package" (nome del parametro "TjMax [°C]").
- **Quando pubblicarlo.** Solo se il vendor è Intel e CPUID famiglia 6 con un modello diverso da `0x0F`, `0x17`, `0x1C` (cioè valore letto dall'MSR). Con microarchitettura sconosciuta LHM non crea sensori di temperatura (condizione `_microArchitecture != Unknown` a `:357`, `:374`, `:399`, `:416`), quindi non c'è nulla da pubblicare.
- **Limiti.** TjMax è il target di attivazione del TCC; LHM non legge l'offset TCC (bit 29:24 dello stesso MSR, Intel SDM Vol. 4), quindi su macchine con offset (tipico sui portatili) il throttling comincia prima di `tjMaxC`. Il ripiego a 100 su MSR illeggibile non è distinguibile, ma con PawnIO funzionante la lettura riesce, e senza PawnIO il device CPU non viene pubblicato (`SchemaBuilder.cs:84-90`).
- **Nomi reali dei sensori Intel.** I core si chiamano "CPU Core #k" (`Cpu/GenericCpu.cs:125-131`), oppure "P-Core #k" / "E-Core #k" sulle CPU ibride (`IntelCpu.cs:330-351`), oppure "CPU Core" con un solo core. Le distanze sono "CPU Core #k Distance to TjMax", "P-Core #k Distance to TjMax" e così via; ci sono anche "Core Max" e "Core Average" (`:357-369`). Il mapping attuale cerca `^Core #(\d+)$` (`SchemaBuilder.cs:50-51`, `:171-175`, `:234-238`) e **non riconosce nessuno di questi nomi**: temperature e clock dei core Intel oggi finiscono nel fallback `lhm-<tipo>-<indice>`, e le distanze diventano "temperature" `lhm-temperature-<i>` con valori di circa 30–60 °C. Solo "CPU Package" → `cpu/0/temperature/package` è corretto. Questa macchina non può validarlo.

## 4. Tabella TjMax AMD (B8)

### 4.1 Fonte e metodo

La pagina delle specifiche AMD incorpora nell'attributo `data-json` della tabella 745 prodotti. Per ognuno ci sono il nome, la serie, il codename (`formerCodename`), il form factor, `maxOperatingTemperatureTjmax` e il link alla pagina prodotto (`productPages.en`). Da lì si sono prese le serie "Ryzen N000 Series" e "Ryzen PRO N000 Series" con N da 3 a 9 e form factor desktop (con "Desktops" e senza "Laptops"). Dopo la normalizzazione di §4.2 non ci sono né nomi duplicati né valori in conflitto.

Riscontro sulle pagine prodotto: "Max. Operating Temperature (Tjmax)" letto su 31 pagine, sempre uguale alla tabella (Ryzen 7 7800X3D = 89 °C compreso). Poi amd.com ha cominciato a rispondere **403** a curl e WebFetch da questa rete (blocco anti-bot), e le altre pagine non sono state rilette.

Nelle tabelle, "Verifica" vale **pagina** (tabella e pagina prodotto lette, uguali) o **tabella** (solo la tabella ufficiale delle specifiche, che è comunque fonte AMD con lo stesso campo). "URL" è la pagina prodotto collegata dalla tabella; "—" vuol dire che AMD non collega una pagina, e la fonte resta la tabella. "Famiglia" è la famiglia CPUID attesa, derivata dal codename AMD (Matisse, Picasso e Renoir = 17h; Vermeer, Cezanne, Raphael e Phoenix = 19h; Granite Ridge = 1Ah): è un controllo di coerenza nostro, non un dato AMD.

### 4.2 Identificazione

- **Stringa usata:** la stringa di marca CPUID grezza, `GenericCpu.CpuId[0][0].BrandString` (pubblica; `Cpu/GenericCpu.cs:110`, `Cpu/CpuId.cs:110-123`, `:209`), insieme a `CpuId.Family` e `CpuId.Vendor`. Non si usa `IHardware.Name` perché:
  - è già rimaneggiato da LHM (toglie "CPU" ovunque, alcune code "N-Core Processor" ma non "4-Core", tronca a "@": `CpuId.cs:124-148`);
  - può essere rinominato dalle impostazioni (`Hardware.cs:37`, `:52-60`), anche se il servizio oggi non ne usa.
- **Normalizzazione** (la stessa si applica offline ai nomi AMD per costruire la tabella):
  1. togliere `®`, `™`, `(R)`, `(TM)`, `(tm)`, U+FFFD e i caratteri a larghezza zero;
  2. ridurre gli spazi a uno e fare trim;
  3. confronto ordinale **sensibile alle maiuscole** sul prefisso;
  4. match completo di `^AMD Ryzen (?<tier>[3579]) (?<pro>PRO )?(?<model>[0-9]{4}[A-Z0-9]*)(?<tail>.*)$`;
  5. `tail` deve essere vuoto oppure esattamente una di queste code: ` <n>-Core Processor`, ` with Radeon[ <testo>] Graphics`, ` w/ Radeon[ <testo>] Graphics`, ` Processor`, ` Dual Edition`;
  6. chiave = `Ryzen <tier> [PRO ]<model>`.
- **Ambiguo, quindi niente `tjMaxC`** (la regola usa il ripiego 85/95):
  - vendor diverso da AMD;
  - il pattern non corrisponde (per esempio "AMD Eng Sample: 100-…", Threadripper, Ryzen AI, Athlon);
  - una coda diversa da quelle ammesse;
  - chiave assente dalla tabella;
  - famiglia CPUID diversa da quella attesa per la voce (controllo saltato per l'unica voce senza codename, Ryzen 5 PRO 3350GE).
- **Esempio su questa macchina:** `AMD Ryzen 7 7800X3D 8-Core Processor` → chiave `Ryzen 7 7800X3D`, famiglia 19h, attesa 19h → `tjMaxC` = 89.

### 4.3 A quale temperatura si applica

- **"Core (Tctl/Tdie)"** esiste quando LHM non applica offset. LHM legge `THM_TCON_CUR_TMP`, cioè Tctl, la temperatura di controllo che il firmware confronta con il limite, e gestisce il bit di range (−49 °C, `Amd17Cpu.cs:274-276`, `:291-292`). L'offset dipende dal nome della CPU secondo la tabella di k10temp:
  - −20 °C per 1600X, 1700X, 1800X;
  - −27 °C per Threadripper 19xx e 29xx;
  - −10 °C per 2700X;
  - 0 per tutti gli altri (`:280-289`).

  Con offset 0 pubblica un solo sensore "Core (Tctl/Tdie)" (`:302-307`, commento "Zen 2 doesn't have an offset so Tdie and Tctl are the same").
- **"Core (Tctl)" e "Core (Tdie)"** esistono solo con offset: Tctl grezzo e Tdie = Tctl + offset (`:294-300`). Gli indici sono in ordine di creazione (`:133-135`): `temperature/0` "Core (Tctl)", `temperature/1` "Core (Tdie)", `temperature/2` "Core (Tctl/Tdie)" (su questa macchina c'è solo il `/2`, M4 §9.1).
- **"CCD<k> (Tdie)"**: solo per i modelli CPUID `0x31`, `0x71`, `0x21`, `0x61`, `0x44` (`:202-229`), letti da registri SMN per CCD (`:310-335`). Più "CCDs Max (Tdie)" e "CCDs Average (Tdie)" con più di una CCD (`:338-362`). Sono temperature di singolo die, non quella confrontata con Tjmax.
- **Raccomandazione:** `cpu-temp` punta a `cpu/0/temperature/tctl` (solo da "Core (Tctl/Tdie)", come oggi a `SchemaBuilder.cs:155-158`) e il valore della tabella si accoppia solo a quello. Le CCD non sono obiettivo di `cpu-temp`.
- **Mai col Tctl con offset:** "Core (Tctl)" (1600X, 1700X, 1800X, 2700X, Threadripper 1000/2000) non deve mai diventare `tctl`. La tabella non contiene quei modelli (§4.5), e per loro la regola userebbe il ripiego sul Tdie.
- **Gli APU Picasso** (3200G/3400G, Zen+) non hanno offset né in k10temp né in LHM; il valore AMD 95 °C si applica al loro Tctl/Tdie.

### 4.4 Tabella — Ryzen desktop consumer (serie 3000, 4000, 5000, 7000, 8000, 9000)

| Chiave | TjMax °C | Codename AMD | Famiglia | Verifica | URL |
|---|---|---|---|---|---|
| `Ryzen 3 3200G` | 95 | Picasso | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-3-3200g.html |
| `Ryzen 3 3200GE` | 95 | Picasso | 17h | tabella | — |
| `Ryzen 5 3400G` | 95 | Picasso | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-5-3400g.html |
| `Ryzen 5 3400GE` | 95 | Picasso | 17h | tabella | — |
| `Ryzen 5 3500` | 95 | Matisse | 17h | tabella | — |
| `Ryzen 5 3600` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-5-3600.html |
| `Ryzen 5 3600X` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-5-3600x.html |
| `Ryzen 5 3600XT` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-5-3600xt.html |
| `Ryzen 7 3700X` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-7-3700x.html |
| `Ryzen 7 3800X` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-7-3800x.html |
| `Ryzen 7 3800XT` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-7-3800xt.html |
| `Ryzen 9 3900` | 95 | Matisse | 17h | tabella | — |
| `Ryzen 9 3900X` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-9-3900x.html |
| `Ryzen 9 3900XT` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-9-3900xt.html |
| `Ryzen 9 3950X` | 95 | Matisse | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-3000-series/amd-ryzen-9-3950x.html |
| `Ryzen 3 4100` | 95 | Renoir | 17h | tabella | — |
| `Ryzen 3 4300G` | 95 | Renoir | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-4000-series/amd-ryzen-3-4300g.html |
| `Ryzen 3 4300GE` | 95 | Renoir | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-4000-series/amd-ryzen-3-4300ge.html |
| `Ryzen 5 4500` | 95 | Renoir | 17h | tabella | — |
| `Ryzen 5 4600G` | 95 | Renoir | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-4000-series/amd-ryzen-5-4600g.html |
| `Ryzen 5 4600GE` | 95 | Renoir | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-4000-series/amd-ryzen-5-4600ge.html |
| `Ryzen 7 4700G` | 95 | Renoir | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-4000-series/amd-ryzen-7-4700g.html |
| `Ryzen 7 4700GE` | 95 | Renoir | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-4000-series/amd-ryzen-7-4700ge.html |
| `Ryzen 7 4700LE` | 95 | Renoir | 17h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/4000-series/amd-ryzen-7-4700le.html |
| `Ryzen 3 5300G` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-3-5300g.html |
| `Ryzen 3 5300GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-3-5300ge.html |
| `Ryzen 3 5305G` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-3-5305g.html |
| `Ryzen 3 5305GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-3-5305ge.html |
| `Ryzen 5 5500` | 90 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5500.html |
| `Ryzen 5 5500F` | 95 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-5-5500f.html |
| `Ryzen 5 5500GT` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5500gt.html |
| `Ryzen 5 5500X3D` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-5-5500x3d.html |
| `Ryzen 5 5600` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5600.html |
| `Ryzen 5 5600F` | 95 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-5-5600f.html |
| `Ryzen 5 5600G` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5600g.html |
| `Ryzen 5 5600GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5600ge.html |
| `Ryzen 5 5600GT` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5600gt.html |
| `Ryzen 5 5600T` | 95 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-5-5600t.html |
| `Ryzen 5 5600X` | 95 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-5-5600x.html |
| `Ryzen 5 5600X3D` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5600x3d.html |
| `Ryzen 5 5600XT` | 95 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-5-5600xt.html |
| `Ryzen 5 5605G` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5605g.html |
| `Ryzen 5 5605GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-5-5605ge.html |
| `Ryzen 7 5700` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-7-5700.html |
| `Ryzen 7 5700G` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-7-5700g.html |
| `Ryzen 7 5700GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-7-5700ge.html |
| `Ryzen 7 5700X` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-7-5700x.html |
| `Ryzen 7 5700X3D` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-7-5700x3d.html |
| `Ryzen 7 5705G` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-7-5705g.html |
| `Ryzen 7 5705GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen/ryzen-5000-series/amd-ryzen-7-5705ge.html |
| `Ryzen 7 5800` | 95 | Vermeer | 19h | tabella | — |
| `Ryzen 7 5800X` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-7-5800x.html |
| `Ryzen 7 5800X3D` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-7-5800x3d.html |
| `Ryzen 7 5800XT` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-7-5800xt.html |
| `Ryzen 9 5900` | 95 | Vermeer | 19h | tabella | — |
| `Ryzen 9 5900X` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-9-5900x.html |
| `Ryzen 9 5900XT` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-9-5900xt.html |
| `Ryzen 9 5950X` | 90 | Vermeer | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/5000-series/amd-ryzen-9-5950x.html |
| `Ryzen 5 7400` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-5-7400.html |
| `Ryzen 5 7400F` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-5-7400f.html |
| `Ryzen 5 7500` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-5-7500.html |
| `Ryzen 5 7500F` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-5-7500f.html |
| `Ryzen 5 7500X3D` | 89 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-5-7500x3d.html |
| `Ryzen 5 7600` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-5-7600.html |
| `Ryzen 5 7600X` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-5-7600x.html |
| `Ryzen 5 7600X3D` | 89 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-5-7600x3d.html |
| `Ryzen 7 7700` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-7-7700.html |
| `Ryzen 7 7700X` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-7-7700x.html |
| `Ryzen 7 7700X3D` | 89 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-7-7700x3d.html |
| `Ryzen 7 7800X3D` | 89 | Raphael AM5 | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-7-7800x3d.html |
| `Ryzen 9 7900` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-9-7900.html |
| `Ryzen 9 7900X` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-9-7900x.html |
| `Ryzen 9 7900X3D` | 89 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-9-7900x3d.html |
| `Ryzen 9 7950X` | 95 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-9-7950x.html |
| `Ryzen 9 7950X3D` | 89 | Raphael AM5 | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/7000-series/amd-ryzen-9-7950x3d.html |
| `Ryzen 3 8300G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-3-8300g.html |
| `Ryzen 3 8300GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-3-8300ge.html |
| `Ryzen 3 8305G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-3-8305g.html |
| `Ryzen 3 8305GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-3-8305ge.html |
| `Ryzen 5 8400F` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-5-8400f.html |
| `Ryzen 5 8500G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-5-8500g.html |
| `Ryzen 5 8500GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-5-8500ge.html |
| `Ryzen 5 8505G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-5-8505g.html |
| `Ryzen 5 8505GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-5-8505ge.html |
| `Ryzen 5 8600G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-5-8600g.html |
| `Ryzen 5 8605G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-5-8605g.html |
| `Ryzen 7 8700F` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-7-8700f.html |
| `Ryzen 7 8700G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-7-8700g.html |
| `Ryzen 7 8705G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/8000-series/amd-ryzen-7-8705g.html |
| `Ryzen 5 9500F` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-5-9500f.html |
| `Ryzen 5 9600` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-5-9600.html |
| `Ryzen 5 9600X` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-5-9600x.html |
| `Ryzen 7 9700F` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-7-9700f.html |
| `Ryzen 7 9700X` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-7-9700x.html |
| `Ryzen 7 9800X3D` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-7-9800x3d.html |
| `Ryzen 7 9850X3D` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-7-9850x3d.html |
| `Ryzen 9 9900X` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-9-9900x.html |
| `Ryzen 9 9900X3D` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-9-9900x3d.html |
| `Ryzen 9 9950X` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-9-9950x.html |
| `Ryzen 9 9950X3D` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-9-9950x3d.html |
| `Ryzen 9 9950X3D2` | 95 | Granite Ridge AM5 | 1Ah | tabella | https://www.amd.com/en/products/processors/desktops/ryzen/9000-series/amd-ryzen-9-9950x3d2-dual-edition.html |

### 4.5 Tabella — Ryzen PRO desktop

| Chiave | TjMax °C | Codename AMD | Famiglia | Verifica | URL |
|---|---|---|---|---|---|
| `Ryzen 3 PRO 3200G` | 95 | Picasso | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-3000-series/amd-ryzen-3-pro-3200g.html |
| `Ryzen 3 PRO 3200GE` | 95 | Picasso | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-3000-series/amd-ryzen-3-pro-3200ge.html |
| `Ryzen 5 PRO 3350G` | 95 | Picasso | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-3000-series/amd-ryzen-5-pro-3350g.html |
| `Ryzen 5 PRO 3350GE` | 95 | — | — | tabella | — |
| `Ryzen 5 PRO 3400G` | 95 | Picasso | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-3000-series/amd-ryzen-5-pro-3400g.html |
| `Ryzen 5 PRO 3400GE` | 95 | Picasso | 17h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-3000-series/amd-ryzen-5-pro-3400ge.html |
| `Ryzen 5 PRO 3600` | 95 | Matisse | 17h | tabella | — |
| `Ryzen 7 PRO 3700` | 95 | Matisse | 17h | tabella | — |
| `Ryzen 9 PRO 3900` | 95 | Matisse | 17h | tabella | — |
| `Ryzen 3 PRO 4350G` | 95 | Renoir | 17h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-4000-series/amd-ryzen-3-pro-4350g.html |
| `Ryzen 3 PRO 4350GE` | 95 | Renoir | 17h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-4000-series/amd-ryzen-3-pro-4350ge.html |
| `Ryzen 3 PRO 4355G` | 95 | Renoir | 17h | tabella | — |
| `Ryzen 3 PRO 4355GE` | 95 | Renoir | 17h | tabella | — |
| `Ryzen 5 PRO 4650G` | 95 | Renoir | 17h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-4000-series/amd-ryzen-5-pro-4650g.html |
| `Ryzen 5 PRO 4650GE` | 95 | Renoir | 17h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-4000-series/amd-ryzen-5-pro-4650ge.html |
| `Ryzen 5 PRO 4655G` | 95 | Renoir | 17h | tabella | — |
| `Ryzen 5 PRO 4655GE` | 95 | Renoir | 17h | tabella | — |
| `Ryzen 7 PRO 4750G` | 95 | Renoir | 17h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-4000-series/amd-ryzen-7-pro-4750g.html |
| `Ryzen 7 PRO 4750GE` | 95 | Renoir | 17h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-4000-series/amd-ryzen-7-pro-4750ge.html |
| `Ryzen 3 PRO 5350G` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-5000-series/amd-ryzen-3-pro-5350g.html |
| `Ryzen 3 PRO 5350GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-5000-series/amd-ryzen-3-pro-5350ge.html |
| `Ryzen 3 PRO 5355G` | 95 | Cezanne | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/5000-series/amd-ryzen-3-pro-5355g.html |
| `Ryzen 3 PRO 5355GE` | 95 | Cezanne | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/5000-series/amd-ryzen-3-pro-5355ge.html |
| `Ryzen 5 PRO 5645` | 95 | Vermeer | 19h | tabella | — |
| `Ryzen 5 PRO 5650G` | 95 | Cezanne | 19h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-5000-series/amd-ryzen-5-pro-5650g.html |
| `Ryzen 5 PRO 5650GE` | 95 | Cezanne | 19h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-5000-series/amd-ryzen-5-pro-5650ge.html |
| `Ryzen 5 PRO 5655G` | 95 | Cezanne | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/5000-series/amd-ryzen-5-pro-5655g.html |
| `Ryzen 5 PRO 5655GE` | 95 | Cezanne | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/5000-series/amd-ryzen-5-pro-5655ge.html |
| `Ryzen 7 PRO 5750G` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-5000-series/amd-ryzen-7-pro-5750g.html |
| `Ryzen 7 PRO 5750GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-5000-series/amd-ryzen-7-pro-5750ge.html |
| `Ryzen 7 PRO 5755G` | 95 | Cezanne | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/5000-series/amd-ryzen-7-pro-5755g.html |
| `Ryzen 7 PRO 5755GE` | 95 | Cezanne | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/5000-series/amd-ryzen-7-pro-5755ge.html |
| `Ryzen 7 PRO 5845` | 95 | Vermeer | 19h | tabella | — |
| `Ryzen 9 PRO 5945` | 95 | Vermeer | 19h | tabella | — |
| `Ryzen 5 PRO 7445` | 95 | Raphael AM5 | 19h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-7000-series/amd-ryzen-5-pro-7445.html |
| `Ryzen 5 PRO 7645` | 95 | Raphael | 19h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-7000-series/amd-ryzen-5-pro-7645.html |
| `Ryzen 7 PRO 7745` | 95 | Raphael | 19h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-7000-series/amd-ryzen-7-pro-7745.html |
| `Ryzen 9 PRO 7945` | 95 | Raphael | 19h | pagina | https://www.amd.com/en/support/downloads/drivers.html/processors/ryzen-pro/ryzen-pro-7000-series/amd-ryzen-9-pro-7945.html |
| `Ryzen 3 PRO 8300G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-3-pro-8300g.html |
| `Ryzen 3 PRO 8300GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-3-pro-8300ge.html |
| `Ryzen 3 PRO 8305G` | 95 | Phoenix | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-3-pro-8305g.html |
| `Ryzen 3 PRO 8305GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-3-pro-8305ge.html |
| `Ryzen 5 PRO 8500G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-5-pro-8500g.html |
| `Ryzen 5 PRO 8500GE` | 95 | Phoenix | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-5-pro-8500ge.html |
| `Ryzen 5 PRO 8505G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-5-pro-8505g.html |
| `Ryzen 5 PRO 8505GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-5-pro-8505ge.html |
| `Ryzen 5 PRO 8600G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-5-pro-8600g.html |
| `Ryzen 5 PRO 8600GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-5-pro-8600ge.html |
| `Ryzen 5 PRO 8605G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-5-pro-8605g.html |
| `Ryzen 5 PRO 8605GE` | 95 | Phoenix | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-5-pro-8605ge.html |
| `Ryzen 7 PRO 8700G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-7-pro-8700g.html |
| `Ryzen 7 PRO 8700GE` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-7-pro-8700ge.html |
| `Ryzen 7 PRO 8705G` | 95 | Phoenix | 19h | tabella | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-7-pro-8705g.html |
| `Ryzen 7 PRO 8705GE` | 95 | Phoenix | 19h | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/8000-series/amd-ryzen-7-pro-8705ge.html |
| `Ryzen 5 PRO 9645` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-5-pro-9645.html |
| `Ryzen 5 PRO 9655` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-5-pro-9655.html |
| `Ryzen 7 PRO 9745` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-7-pro-9745.html |
| `Ryzen 7 PRO 9755` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-7-pro-9755.html |
| `Ryzen 7 PRO 9755X3D` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-7-pro-9755x3d.html |
| `Ryzen 9 PRO 9945` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-9-pro-9945.html |
| `Ryzen 9 PRO 9955` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-9-pro-9955.html |
| `Ryzen 9 PRO 9965` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-9-pro-9965.html |
| `Ryzen 9 PRO 9965X3D` | 95 | Granite Ridge AM5 | 1Ah | pagina | https://www.amd.com/en/products/processors/desktops/ryzen-pro/9000-series/amd-ryzen-9-pro-9965x3d.html |

### 4.6 Voci non verificate ed esclusioni

- **Non verificate (niente voce):** `Ryzen 3 3100` e `Ryzen 3 3300X`. La tabella AMD non riporta Tjmax per questi due modelli, e le pagine prodotto non erano più raggiungibili (403). Per loro la regola usa il ripiego.
- **Escluse di proposito:**
  - Ryzen 1000/2000 (Zen/Zen+), perché gli X con offset di Tctl rendono ambiguo a quale temperatura si riferisca il Tjmax;
  - Threadripper (offset sulle serie 1000/2000, fuori dall'ambito);
  - mobile e Ryzen AI. Sono raggiungibili nella stessa tabella AMD (per esempio 7840HS, 8745HS a 100 °C), ma sono fuori dall'ambito minimo; i limiti termici reali dei portatili li decide l'OEM e su quei modelli LHM espone solo "Core (Tctl/Tdie)". Si possono aggiungere con lo stesso metodo in un follow-up.
- **Da ricontrollare facoltativamente:** le 133 voci "tabella" sulla pagina prodotto, da un browser, quando amd.com torna raggiungibile.

## 5. Id obiettivo delle regole predefinite su questa macchina

Device: la CPU è legata a `cpu/0` dall'hint `Cpu` (`crates/oma-win/src/svc/provider.rs:137-138`); i dischi si legano all'id del core `storage/device-<hash>` quando modello e seriale coincidono (M4 §9.4, `provider.rs:140-148`), come succede su questa macchina. Gli hash sono omessi come in M4.

| Regola | Id obiettivo (mapping attuale o proposto) | Qui | Note |
|---|---|---|---|
| `cpu-temp` | `cpu/0/temperature/tctl` (AMD, "Core (Tctl/Tdie)"); `cpu/0/temperature/package` (Intel, "CPU Package"); proposto `cpu/0/temperature/tdie` (AMD con offset, "Core (Tdie)") | `tctl` sì, 48 °C a riposo; `tjMaxC` = 89 → attenzione 79 °C, critico 89 °C | `package` e `tdie` solo se presenti; richiede servizio e PawnIO (`SchemaBuilder.cs:84-90`) |
| `cpu-throttle` | — | no | nessun sensore (§1) |
| `disk-temp` | `storage/<disco>/temperature/drive` | tutti e 4: HDD e NVMe dal core (con duplicato dal servizio), Corsair Force LS solo dal servizio | `tempWarningC`/`tempCriticalC` dal core solo sui NVMe (86/87 SHPP41, 90/95 Fanxiang); HDD e SSD SATA usano il ripiego 70/80 |
| `disk-wear` | `storage/<disco>/percent/wear` ("Percentage Used") | i 2 NVMe (0 % e 5 %) | solo se presente; il SATA SSD ha solo `percent/life` (vita residua, altra semantica), l'HDD niente |
| `disk-critical` | `storage/<disco>/flag/critical-warning` (nuovo, §2) | i 2 NVMe | solo NVMe; dal servizio, oppure dal core se si accetta §2.5 |

GPU (`gpu-temp`, `gpu-hotspot`, `gpu-mem-temp`, `gpu-throttle`), `ram-used` e `volume-used` vengono dal core non privilegiato e li mappa un altro agente; `battery-low` non ha provider.

## 6. Fixture

Nomi, tipi e identificatori come li produce LHM (ordine degli indici dal codice citato). Il dettaglio è in "Per il piano M5b", punto 6.

---

## Per il piano M5b

1. **`cpu-temp`.**
   - Selettore: `{ deviceKind: "cpu", sensorKind: "temperature", names: ["tctl", "tdie", "package"] }`. Su ogni CPU al massimo uno dei tre esiste, perché "Core (Tctl/Tdie)" esclude la coppia "Core (Tctl)"/"Core (Tdie)" e "CPU Package" è solo Intel.
   - Soglie: `tjMaxC` − 10 con ripiego 85, `tjMaxC` con ripiego 95 (spec §3.3).
   - Test di selettore sugli snapshot di questa macchina: `cpu/0/temperature/tctl`.
2. **Proprietà `tjMaxC` sul device CPU** (stringa decimale in °C, come `tempWarningC`). Oggi `ProcessDevice` passa `EmptyProperties` (`SchemaBuilder.cs:626`).
   - **AMD:** tabella di §4.4–4.5 incorporata nel servizio, con chiave e famiglia attesa, e identificazione di §4.2.
   - **Intel:** `Parameters` "TjMax [°C]" del sensore "CPU Package", solo alle condizioni di §3.
   - **Dati da aggiungere:** a `HardwareNode` un `CpuInfo?` (Vendor, BrandString, Family, Model, IntelTjMaxC?), riempito in `LhmTree.BuildNode` (`LhmTree.cs:427-443`) da `hardware is GenericCpu`, così `SchemaBuilder` resta puro.
   - Da verificare nel piano: il merge del device `cpu/0` tra core e servizio deve conservare la proprietà del servizio.
3. **Mapping CPU:**
   - "Core (Tdie)" → `temperature/tdie` (etichetta `cpu.temperature.tdie`);
   - "Core (Tctl)" resta nel fallback, mai `tctl`;
   - Intel:
     - correggere i nomi: "CPU Core #k" → `core-<k>`, e decidere `p-core-<k>`/`e-core-<k>` per gli ibridi;
     - scartare "… Distance to TjMax";
     - "Core Max"/"Core Average": fallback o nomi propri, da decidere.

   Il mapping Intel è una correzione non verificabile qui, da tenere separata con fixture dedicate.
4. **`disk-critical`.**
   - Sensore `storage/<disco>/flag/critical-warning`: kind `flag`, unità `boolean` (`crates/oma-core/src/model.rs`: `SensorKind::Flag`, `Unit::Boolean`), etichetta `storage.criticalWarning` (chiave i18n nuova in `en.json`/`it.json`).
   - Valore `(RawValue[0] & 0x3D) != 0 ? 1 : 0` (maschera da confermare, §2.3), letto dopo ogni `StorageDevice.Update()` dall'attributo DIT `CriticalWarning`, solo se `Storage.IsNVMe` e l'attributo esiste.
   - Serve un binding nuovo (non è un `ISensor`): per esempio `StorageInfo` con `IsNvme` e `HasCriticalWarning`, e un tipo di binding "SMART critical warning" risolto dall'hub.
   - `UnitByKind` non conosce `flag`: l'unità va passata esplicitamente.
5. **Decisione per l'utente (§2.5):** leggere il log NVMe anche dal core (accesso 0, non privilegiato) per `critical-warning`, `wear` e `available-spare`, così `disk-wear` e `disk-critical` funzionano senza servizio sui NVMe. Sarebbe una deviazione dalla spec §3.3.
6. **Fixture da aggiungere a `SchemaBuilderTests`:**
   - **AMD di questa macchina:** `HardwareNode("/amdcpu/0", Cpu, "AMD Ryzen 7 7800X3D", [Sensor("/amdcpu/0/temperature/2", Temperature, "Core (Tctl/Tdie)", 2), Sensor("/amdcpu/0/temperature/3", Temperature, "CCD1 (Tdie)", 3)], [])` con `CpuInfo(Vendor.AMD, "AMD Ryzen 7 7800X3D 8-Core Processor", Family 0x19, Model 0x61)` → `tctl` e proprietà `tjMaxC` = `"89"`.
   - **AMD con offset (1800X):** `Sensor("/amdcpu/0/temperature/0", Temperature, "Core (Tctl)", 0)`, `Sensor("/amdcpu/0/temperature/1", Temperature, "Core (Tdie)", 1)`, brand `AMD Ryzen 7 1800X Eight-Core Processor`, famiglia 0x17 → `tdie` presente, nessun `tctl`, nessun `tjMaxC`.
   - **Identificazione** (test puri della funzione di normalizzazione):

     | Brand string | Famiglia | Atteso |
     |---|---|---|
     | `AMD Ryzen 7 5800X3D 8-Core Processor` | 0x19 | 90 |
     | `AMD Ryzen 5 5600G with Radeon Graphics` | 0x19 | 95 |
     | `AMD Ryzen 7 8700G w/ Radeon 780M Graphics` | 0x19 | 95 |
     | `AMD Ryzen 9 9950X3D 16-Core Processor` | 0x1A | 95 |
     | `AMD Ryzen 7 7800X3D 8-Core Processor` | 0x1A (famiglia errata) | niente |
     | `AMD Ryzen 7 7800X3D Engineering` | 0x19 (coda non ammessa) | niente |
     | `AMD Eng Sample: 100-000000910-40_Y` | — | niente |
     | `AMD Ryzen 3 3300X 4-Core Processor` | 0x17 (non in tabella) | niente |
     | `AMD Ryzen Threadripper 3970X 32-Core Processor` | — | niente |
     | `Intel(R) Core(TM) i9-13900K` (vendor Intel) | 6 | niente (tabella solo AMD) |

   - **Intel non ibrida, 8 core** (per esempio i7-9700K, `/intelcpu/0`): `temperature/0` "Core Max", `/1` "Core Average", `/2`…`/9` "CPU Core #1…#8", `/10` "CPU Package", `/11`…`/18` "CPU Core #1…#8 Distance to TjMax" (ordine di `coreSensorId`, `IntelCpu.cs:353-429`), `IntelTjMaxC` = 100 → `package`, `core-1…8`, distanze scartate, `tjMaxC` = `"100"`.
   - **Intel ibrida** (per esempio i5-12600K): "P-Core #1…#6", "E-Core #1…#4", i relativi "… Distance to TjMax", "CPU Package".
   - **NVMe con critical warning:** `HardwareNode("/nvme/2", Storage, "Fanxiang S880 2TB", [… "Composite Temperature", "Warning Temperature" (`/nvme/2/temperature/10`), "Critical Temperature" (`/nvme/2/temperature/11`), "Percentage Used" (`/nvme/2/level/102`) …], [], StorageInfo(2, …, IsNvme: true, HasCriticalWarning: true))` → `flag/critical-warning` emesso, le due soglie ancora scartate, `percent/wear` presente.
   - **SATA** (`/ssd/1` "Corsair Force LS SSD", `/hdd/0` "ST2000DM008-2FR102"): nessun `flag/critical-warning`.
7. **Limiti da dichiarare in UI e documentazione:**
   - throttling CPU non disponibile (LHM 0.9.6);
   - critical warning solo NVMe;
   - `tjMaxC` AMD solo per i modelli desktop in tabella (niente Zen/Zen+, Threadripper, mobile, engineering sample), altrimenti ripiego 85/95;
   - `tjMaxC` Intel = target TCC senza offset;
   - i valori SMART restano all'ultima lettura riuscita;
   - le soglie di temperatura dei dischi SATA sono il ripiego 70/80.
8. **Da fare a mano (utente):** niente per chiudere questo spike. Facoltativo: ricontrollare da browser le voci "tabella" su amd.com.
