# F1 — Riconfigurazione del servizio: studio di fattibilità per la M5a

Studio in sola lettura richiesto da §2.8 e §10 della spec M5 (`docs/superpowers/specs/2026-09-29-m5-regole-integrazione-design.md`): esclusione SMART per disco prima della discovery, protocollo di riconfigurazione dei worker, stato di PawnIO. Nessun codice di prodotto è stato modificato.

**Fonti.** Codice locale al commit `43fbe2f` (riferimenti `file:riga`). Sorgenti di terzi letti online, fissati come nello spike S1 (`docs/superpowers/references/m4/s1-lhm.md` §6):

- **LHM** = `https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/v0.9.6/LibreHardwareMonitorLib/Hardware/` (tag `v0.9.6`, il pacchetto NuGet 0.9.6 in uso); **LHM-UI** = stesso tag, cartella `LibreHardwareMonitor/UI/`;
- **DIT** = `https://github.com/Blacktempel/DiskInfoToolkit/blob/25319eae5781e75bcf141e844ceab2afe94d40ea/DiskInfoToolkit/` (commit indicato dal `.nuspec` di DiskInfoToolkit 1.1.2 in `~/.nuget/packages/diskinfotoolkit/1.1.2/`);
- **SPD** = `https://github.com/Blacktempel/RAMSPDToolkit/blob/3b47b960e0830fef344624ad5e389675d5f0a1ce/RAMSPDToolkit/` (RAMSPDToolkit 1.4.2);
- **BS** = `https://github.com/Blacktempel/BlackSharp/blob/c70b735c6cec123ee8a046ac4a0bc6c606f52cf0/BlackSharp.Core/`.

I percorsi sotto sono relativi a queste radici; i numeri di riga si leggono con `#L<a>-L<b>` sull'URL.

## Sintesi dei verdetti

1. **F1, esclusione SMART per disco prima della discovery: verdetto (c).** Con l'API pubblica di LHM 0.9.6 non esiste un filtro per disco applicabile alla creazione dell'hardware: `StorageGroup` è `internal` e il suo costruttore chiama `StorageManager.ReloadStorages()`, che apre e identifica **ogni** disco; il costruttore di `DiskInfoToolkit.Storage` è `internal` e prende un tipo `internal`; `ISettings`, `HardwareAdded` e `StoragesChanged` arrivano dopo l'identificazione. L'alternativa (b), togliere il disco dal gate D6 lasciando che la discovery lo tocchi, **sveglia di sicuro** un HDD USB in standby (lettura del settore 0 incondizionata prima dei comandi SAT) e, se l'identificazione fallisce, lo risveglia a ogni `DBT_DEVNODES_CHANGED`: la spec la vieta. Si conserva il gate globale D6. L'interruttore per disco resta realizzabile **dopo** la discovery (nessun comando periodico al disco: niente CHECK POWER MODE, niente SMART), con il limite dichiarato nella UI. La via concreta per il follow-up USB non passa da LHM: è un fallback SAT (`ATA PASS-THROUGH(16)` via `IOCTL_SCSI_PASS_THROUGH`) per `CHECK POWER MODE` in `DiskPowerProbe`, da verificare dal vivo sul disco dell'utente.
2. **F2, protocollo di riconfigurazione: fattibile con modifiche contenute.** Il modello attuale (due thread proprietari, nessun lock condiviso durante l'I/O, pubblicazione per scambio di riferimenti immutabili) regge un "applicatore" posseduto dal sampler. Punti chiave: aggregazione pura sotto `_subLock`; `Subscribe` successivo come **sostituzione atomica** (oggi è `Dispose` + `Subscribe` e passa da zero sottoscrittori); i moduli non storage cambiano con i setter LHM chiamati **solo dal sampler**, dopo un handshake di parcheggio non bloccante con lo storage worker e con timeout che porta a uno stato di errore; lo storage si disattiva **in modo "soft"** (il gruppo LHM resta aperto), perché disattivarlo e riattivarlo con il setter ricarica tutti i dischi (nuovo gate D6) e lascia un `StorageGroup` agganciato a un evento statico. Serve un modo per comunicare al client lo stato effettivo globale e l'esito (applicato, in attesa, fallito), richiesto anche da `applyStatus` di §2.3.
3. **F3, PawnIO: oggi la probe distingue solo "utilizzabile sì/no".** Con segnali già disponibili si ottengono `ok`, `missing`, `unavailable` e `unknown` in modo onesto. `rebootPending` richiede un'evidenza che oggi non esiste: l'installer gestisce l'exit code 3010 solo con `SetRebootFlag` e non lascia traccia. Minimo onesto: un marcatore scritto dall'installer con l'istante dell'installazione, confrontato dal servizio con l'istante di boot corrente.

---

## F1 — Esclusione SMART per disco prima della discovery

### F1.1 Cosa succede alla creazione dello storage in LHM 0.9.6

**Catena di chiamate.** `LhmTree.EnableStorage()` imposta `computer.IsStorageEnabled = true` (`service/OpenMonitorAdvanced.Service/Sensors/LhmTree.cs:148-167`, setter a `:160`). Su un `Computer` aperto il setter esegue `Add(new StorageGroup(_settings))` (LHM `Computer.cs:296-311`). Il costruttore del gruppo (LHM `Storage/StorageGroup.cs:23-31`) chiama `AddHardware`, che:

1. esegue `StorageManager.ReloadStorages()` (LHM `Storage/StorageGroup.cs:40`);
2. crea un `StorageDevice` per ogni `Storage` valido (`:43`);
3. si iscrive all'evento statico `StorageManager.StoragesChanged` (`:45`).

Solo dopo, `Computer.Add` genera `HardwareAdded` per ogni disco (LHM `Computer.cs:455-459`).

**Cosa fa `ReloadStorages`** (DIT `StorageManager.cs:107-164`), per tutti i dischi, senza filtri:

- enumerazione SetupAPI dei controller e dei dischi (`StorageDetector.GetStorageDevices`, DIT `StorageDetector.cs:47`), con `CreateFile(GENERIC_READ|GENERIC_WRITE)` e `IOCTL_STORAGE_GET_DEVICE_NUMBER` su ogni interfaccia disco (DIT `StorageDetector.cs:269-282`, `Storage.cs:317-325`; apertura in BS `Interop/Windows/Utilities/SafeFileHandler.cs`, `OpenHandle`);
- sondaggio di `\\.\PhysicalDrive0…63` non ancora visti (DIT `StorageManager.cs:125-161`);
- per ciascun disco, `new Storage(controller, device)` (DIT `StorageManager.cs:179-186`).

**Il costruttore di `Storage`** (DIT `Storage.cs:43-99`) apre il disco in lettura/scrittura, poi esegue:

- `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX` (`Storage.cs:535-543`);
- `IOCTL_STORAGE_QUERY_PROPERTY(StorageDeviceProperty)` (`:560-603`);
- il layout delle partizioni con `IOCTL_DISK_GET_DRIVE_LAYOUT_EX`, `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS` e `GetDiskFreeSpaceEx` sui volumi (DIT `Partition.cs:130, 192, 249, 323`);
- infine `IdentifyDisk` (`Storage.cs:503-533`, poi DIT `Identifiers/DeviceIdentifier.cs:31-537`).

**`IdentifyDisk` per tipo di bus:**

- **ATA/SATA/sconosciuto** (`DeviceIdentifier.cs:37-81`): `IDENTIFY DEVICE` (0xEC) via `SMART_RCV_DRIVE_DATA`/pass-through (`DoIdentifyDevicePd`, `:789`, `:806`). Se il primo tentativo fallisce, `DiskHandler.WakeUp` (`:53`) **legge il settore 0** con `ReadFile` (DIT `Disk/DiskHandler.cs:228-234`) e riprova. Poi `DiskHandler.AddDisk` legge **SMART READ DATA** e le soglie, due volte per verificare la coerenza (DIT `Disk/DiskHandler.cs:578-599` per il percorso `PHYSICAL_DRIVE`).
- **NVMe** (`DeviceIdentifier.cs:82-123`): Identify e log SMART/health tramite `IOCTL_STORAGE_QUERY_PROPERTY` o le varianti dei vendor; non c'è piatto da svegliare.
- **Ogni altro bus, USB compreso** (`DeviceIdentifier.cs:125-534`): prima **`DiskHandler.WakeUp(handle)` incondizionato** (`:127`), poi una serie di tentativi SAT/JMicron/Sunplus/Cypress/Logitec/Prolific/IO-DATA e USB-NVMe (`:129-533`). Se nessuno riesce restituisce `false` (`:537`): `Storage.IsValid = false` e il disco **non entra** in `StorageManager.Storages`, quindi LHM non lo espone affatto.

**Possono svegliare un HDD in standby?** Sì, con certezza per l'USB: la lettura del settore 0 a `DeviceIdentifier.cs:127` è un accesso al supporto. Per un SATA in standby sono a rischio `IDENTIFY` (secondo il firmware, può essere servito senza spin-up), il `WakeUp` di ripiego e `SMART READ DATA`. Le IOCTL di geometria e layout di norma sono servite dalla cache di `partmgr`/`disk.sys`, ma non è garantito. È coerente con quanto già documentato in s1-lhm.md §6b.

**Filtri per disco nell'API pubblica: nessuno.**

- `StorageGroup` è `internal` (LHM `Storage/StorageGroup.cs:14`): non si costruisce né si configura dall'esterno.
- `ISettings` (LHM `ISettings.cs`) espone solo `Contains/SetValue/GetValue/Remove` di valori testuali. LHM lo usa per nomi e impostazioni dei sensori (`Hardware.cs:37`), mai per escludere dispositivi.
- `Computer.HardwareAdded` scatta **dopo** la costruzione del gruppo, quindi dopo identificazione e SMART (LHM `Computer.cs:455-459`); `StorageManager.StoragesChanged` scatta dopo `new Storage` (DIT `StorageManager.cs:444-466`). Entrambi arrivano troppo tardi.
- `StorageDevice.ThrottleInterval` è statico e globale, e agisce solo su `Update()` (LHM `Storage/StorageDevice.cs:63-72`).
- `LibreHardwareMonitor.Hardware.Storage.StorageDevice` ha un costruttore pubblico (LHM `Storage/StorageDevice.cs:47-55`) che non fa I/O: legge in memoria `_storage.Smart` (`:152-229`). Però richiede un `DiskInfoToolkit.Storage`, il cui costruttore è `internal` e prende `DiskInfoToolkit.Internal.StorageDevice`, anch'esso `internal` (DIT `Storage.cs:43`, `Internal/StorageDevice.cs:17`). Gli unici `Storage` ottenibili pubblicamente vengono da `ReloadStorages()` (tutti i dischi) o dal thread di hot-plug. Il campo `StorageManager._Storages` è `public static` (DIT `StorageManager.cs:76`), ma `ReloadStorages` lo ricostruisce da zero: non aiuta.
- **Ipotesi scartate:**
  - reflection sul costruttore interno: non è API pubblica e si rompe con il trimming del servizio (`service/trim-allowlist.txt`, `scripts/check-trim-warnings.ps1`);
  - un handle esclusivo (`dwShareMode = 0`) tenuto aperto sul disco durante `ReloadStorages`, così che l'apertura di DIT fallisca prima di ogni I/O (`Storage.cs:58-66`): comportamento di condivisione dei device disco non documentato, bloccherebbe altri strumenti (backup, CrystalDiskInfo) e non copre il thread di hot-plug, che riprova in momenti arbitrari (sotto).

**Buco già esistente, indipendente dalla M5.** Il costruttore statico di `StorageManager` avvia un thread di ascolto `WM_DEVICECHANGE` che non si ferma mai (DIT `StorageManager.cs:37-52`, già in s1-lhm.md §3). Alla prima abilitazione dello storage:

- a ogni `DBT_DEVNODES_CHANGED` (qualunque device PnP che cambia) `HandleUnpartitionedDrive` riesamina tutti i dischi e ricrea `new Storage(...)` per ogni `DriveNumber` che non è in `_Storages` (DIT `StorageManager.cs:336-472`, "added" a `:426-435`). Un disco la cui identificazione fallisce (`IsValid = false`) non entra mai in `_Storages` e viene **reidentificato, con `WakeUp`, a ogni cambio di devnode**;
- un disco collegato a caldo viene identificato subito da quel thread, fuori dal nostro gate. Qui il rischio è basso, perché un disco appena collegato di solito gira.

Oggi il primo caso non si presenta per l'HDD USB che non conferma lo stato di alimentazione: quel disco tiene chiuso il gate, lo storage non si abilita mai e il thread non parte. Si presenta invece se un disco di quel tipo viene **collegato dopo** l'abilitazione. Va messo tra i follow-up come limite noto di D6.

### F1.2 Cosa fa ogni `Update()` successivo e cosa supporta già il nostro codice

`StorageDevice.Update()` (LHM `Storage/StorageDevice.cs:65-94`), a ogni chiamata (con `ThrottleInterval` a zero, il nostro caso):

1. `UpdatePerformanceSensors`: `CreateFile` in lettura/scrittura sul percorso fisico e `IOCTL_DISK_PERFORMANCE` (`:265-313`);
2. `_storage.Update()` (DIT `Storage.cs:294-303`): `DiskHandler.UpdateSmartInfo`, cioè una lettura SMART completa ogni volta (DIT `Disk/DiskHandler.cs:34-226`). Con **`WakeUp` incondizionato prima di ogni lettura** per i bridge SAT/USB (`:142`, `:152`), e dopo una lettura fallita per `PHYSICAL_DRIVE`/CSMI (`:80`, `:117`). Poi rilegge le partizioni;
3. aggiorna i sensori in memoria.

**Il nostro codice salta già l'`Update` per singolo disco.** Il ciclo di `SensorHub.StorageOnce` (`service/OpenMonitorAdvanced.Service/Sensors/SensorHub.cs:334-403`) gestisce ogni disco separatamente:

- disco con identificatore duplicato: saltato (`:346-352`);
- identità non risolta: saltato (`:354-358`);
- "no media": saltato (`:362-369`);
- disco rotazionale in standby o in stato ignoto: saltato (`:371-389`);
- altrimenti `_tree.Update(root)` (`:391-399`).

Aggiungere "SMART disattivato per questo disco" è un'altra condizione `continue` nello stesso punto. Il disco va però tolto anche da `resolved`, così esce dallo schema e non resta con tutti i valori `nil`, e va saltata anche la `CHECK POWER MODE`: per quel disco non serve. Nota: saltare l'`Update` non tocca il gate D6, che si valuta prima (`SensorHub.cs:315-318`, `TryEnableStorage` a `:698-735`). È esattamente il punto sollevato dalla spec (§2.8).

### F1.3 Il gate D6, l'inventario dei dischi e lo spazio degli id

**Chi deve confermare.**

- `DiskPowerProbe.AllRotationalDisksActive` (`service/OpenMonitorAdvanced.Service/Sensors/DiskPowerProbe.cs:75-80`) enumera **da sé** `\\.\PhysicalDrive0…63` con apertura ad accesso 0 (`EnumeratePhysicalDrives`, `:161-173`; `DescribePhysicalDrive`, `:175-227`), senza LHM.
- Poi applica `FindGateBlockers` (`:87-105`): ogni disco con `DriveFacts.RequiresPowerCheck` (`IHardwareTree.cs:103-106`) deve rispondere `IsSpunDown == false`. Il requisito vale per tutti i dischi tranne quelli senza media, virtuali, Storage Spaces, NVMe o con seek penalty `false`.
- `IsSpunDown` usa `IOCTL_ATA_PASS_THROUGH` con `CHECK POWER MODE` (`DiskPowerProbe.cs:265-326`).
- Con un solo disco bloccante `TryEnableStorage` restituisce `false` e riprova a ogni giro da 30 s (`SensorHub.cs:698-720`).
- Dopo l'abilitazione, ogni disco rotazionale viene controllato a ogni giro (`SensorHub.cs:371-389`).

**L'HDD USB del follow-up** (`docs/follow-ups.md:10`). Su `usbstor`/`uaspstor` `IOCTL_ATA_PASS_THROUGH` di norma non è supportata: la probe risponde `null` e il disco blocca il gate per sempre. È probabile, ma non verificabile dal sorgente, che lo stesso bridge accetti SAT via `IOCTL_SCSI_PASS_THROUGH`. È il canale che DIT usa per identificarlo, e che `smartctl -d sat -n standby` usa per la stessa domanda.

**Da verificare dal vivo.** Una chiavetta USB o un lettore di schede con supporto inserito: se `StorageDeviceSeekPenaltyProperty` fallisce, la seek penalty è ignota, `RequiresPowerCheck` vale `true` e anche la chiavetta terrebbe spento lo SMART di tutti. Controllo: collegare una chiavetta e cercare nel log del servizio la riga `PhysicalDriveN (bus 0x07, …) keeps storage disabled` (`DiskPowerProbe.cs:150-158`).

**Da dove viene l'inventario.** Ci sono due inventari sicuri, entrambi disponibili anche quando D6 tiene chiuso lo storage:

- **Servizio:** `DiskPowerProbe` (sopra) fornisce numero di drive, modello e seriale del descrittore, bus e seek penalty, tutto con accesso 0. Oggi però l'enumerazione completa è privata: `IDiskPowerProbe` espone solo `Describe(n)`, `IsSpunDown(n)` e `AllRotationalDisksActive()` (`IHardwareTree.cs:45-63`).
- **App:** il provider storage del core (`crates/oma-win/src/storage.rs:137-142`, `DriveEntry { index, device_id, model, serial }`) conosce ogni disco con il suo id core (`storage/device-<sha256(vendor\0model\0serial)>` per il livello seriale, `crates/oma-win/src/storage_identity.rs:76-90`, oppure i livelli GPT/MBR/PnP), senza privilegi.

**Lo spazio degli id oggi.**

| Livello | Id | Quando esiste | Stabile? |
|---|---|---|---|
| LHM | `/hdd|ssd|nvme/<DriveNumber>` (LHM `Storage/StorageDevice.cs:48`, prefisso da `StorageGroup.cs:72-80`) | solo dopo la discovery | no: il numero cambia con l'hot-plug (s1-lhm.md §6a) |
| Servizio, schema | `lhm-<sha256(model, seriale IDENTIFY)>` o, se il seriale manca o non è univoco, `lhm-<sha256(identificatore LHM)>`; fissato una volta pubblicato (`SchemaBuilder.cs:385-389`, pin in `SensorHub.cs:584-606`) | solo dopo la discovery, e solo per i dischi risolti (`SensorHub.cs:566-580`) | sì se c'è il seriale; no con il ripiego sull'identificatore |
| Hint | `StorageHint(PhysicalDrive, DescriptorModel, DescriptorSerial)` (`Protocol/Messages.cs:39`, `SchemaBuilder.cs:398-400`) | con il device del servizio | numero no, modello e seriale sì |
| App | `storage/<id core>` quando `storage_binding` fa coincidere modello e seriale del descrittore con quelli del core per lo stesso `PhysicalDrive` (`crates/oma-win/src/svc/provider.rs:37-61`, `:111-121`); altrimenti `storage/lhm-…` | id core sempre; `storage/lhm-…` solo dopo la discovery | sì |

**Conseguenza.** Se `smartDisabledDrives` contenesse "gli id dei device disco lato servizio" (`lhm-…`), come dice alla lettera §2.8, i dischi non sarebbero selezionabili finché D6 blocca lo storage, perché lo schema non contiene alcun disco. L'id resterebbe inoltre instabile per i dischi senza seriale. La stessa §2.8 chiede che "gli id selezionabili provengano dall'inventario sicuro". Proposta coerente con entrambi i vincoli:

- **Chiave di disco sul filo (`driveKey`):** `sha256(trim(model) \0 trim(serial))` in esadecimale minuscolo, calcolata sui testi del **descrittore** (`STORAGE_DEVICE_DESCRIPTOR`), gli stessi che l'hint trasporta e che `storage_binding` confronta già (con `trim` da entrambe le parti).
  - Il servizio la calcola da `StorageInfo.DescriptorModel/DescriptorSerial` dopo `Resolve` (`SensorHub.cs:624-673`), oppure da `DriveFacts` prima della discovery.
  - L'app la calcola da `DriveEntry.model/serial`.
- **L'app persiste l'id core** del disco (`sources.smartDisabledDrives` nelle impostazioni), che è ciò che la UI mostra; a ogni `Subscribe` lo traduce nella `driveKey` del disco presente con quell'id.
- **Dischi senza modello o seriale nel descrittore** (VHDX, alcuni bridge): nessuna chiave, interruttore disattivato con la spiegazione.
- **Vettore di test condiviso** (per esempio in `protocol/fixtures/` o come costante nei due test), così Rust e .NET calcolano la stessa chiave byte per byte.
- **Spec da correggere:** in §2.8, "id dei device disco lato servizio" diventa "chiave del descrittore del disco (`driveKey`)".

### F1.4 Verdetto e raccomandazione

**(a) Filtro reale prima della discovery con l'API pubblica: non fattibile.** Vedi F1.1: nessun punto di estensione precede `ReloadStorages()` e la costruzione di `Storage` è interna.

**(b) Escludere il disco dal gate D6 lasciando che la discovery lo tocchi una volta: tecnicamente banale, ma il risveglio è certo.** Per un disco USB in standby `DeviceIdentifier.cs:127` legge il settore 0 prima di qualunque tentativo. Se poi l'identificazione fallisce (il bridge rifiuta tutto) il disco non compare in LHM, quindi non se ne otterrebbe nemmeno lo SMART, e viene risvegliato di nuovo a ogni `DBT_DEVNODES_CHANGED` (F1.1). Se invece l'identificazione riesce (bridge SAT), ogni `Update` successivo fa un `WakeUp` prima della lettura SMART (`DiskHandler.cs:142/152`), e il nostro controllo di stato non lo evita, perché per quel disco la probe ATA non risponde. La spec vieta di "aggirare il gate per rendere operativo l'interruttore": **scartata**.

**(c) Raccomandata: si conserva il gate globale D6 e l'interruttore per disco vale solo dopo la discovery.**

- **Semantica dell'interruttore "SMART disattivato per il disco X":**
  - il servizio non invia più a X né `CHECK POWER MODE` né SMART;
  - X esce dallo schema lato servizio;
  - l'app continua a mostrarlo con i dati del core (I/O, temperatura quando il driver la espone).

  È utile per davvero: lascia andare in standby un HDD che l'utente vuole spento. Una lettura SMART con `WakeUp` ogni 30 s, oppure anche solo un comando periodico, può impedire lo spin-down; va verificato dal vivo insieme ai controlli HDD standby ancora aperti della M4.
- **Limite dichiarato nella UI**, sotto l'interruttore, per esempio: «Disattivare lo SMART di un disco non evita la prima identificazione dei dischi: finché un disco rotazionale non conferma di essere attivo, lo SMART resta spento per tutti i dischi». Il follow-up USB resta aperto (§2.8).
- **Facoltativo ma consigliato per la UX:** il servizio indica quali dischi tengono chiuso il gate (`driveKey` dei `DriveBlocker`, già calcolati in `FindGateBlockers`), così la UI dice «Lo SMART è spento per tutti i dischi perché *Disco X* non conferma lo stato di alimentazione». Vedi il blocco di stato proposto in F2.3.
- **Soluzione del follow-up USB, fuori da LHM e fuori dalla M5a (o come spike separato):** `CHECK POWER MODE` via SAT in `DiskPowerProbe`, come ripiego quando `IOCTL_ATA_PASS_THROUGH` fallisce.
  - Comando: `IOCTL_SCSI_PASS_THROUGH` con CDB `ATA PASS-THROUGH(16)` (0x85), protocollo non-data, `CK_COND = 1`, comando 0xE5; il settore count si legge dal descrittore "ATA Status Return" dei sense data.
  - Un disco USB che risponde diventa confermabile: il gate si apre quando il disco gira e LHM lo identifica senza svegliarlo.
  - **Verifica dal vivo obbligatoria:** con il disco in standby, la probe deve rispondere 0x00 e il disco restare fermo. Alcuni bridge riattivano il disco su qualunque pass-through, oppure non restituiscono i registri: in quel caso resta `null`, come oggi.

### F1.5 Test per la scelta (c)

Con i fake esistenti (`service/OpenMonitorAdvanced.Service.Tests/Sensors/SensorHubFakes.cs`: `FakeTree` con `Storage`, `EnableStorageCount`, `Updates()`; `FakeDisks` con `Facts`, `SpunDown`, `SpunDownQueries`, `AllActive`), aggiungendo a `FakeDisks` un contatore per drive delle interrogazioni di stato:

- `ASmartDisabledDiskIsNeitherPowerCheckedNorUpdated`: disco con chiave in `smartDisabledDrives`; dopo `StorageOnce` nessuna `IsSpunDown` e nessun `Update` per quel drive, gli altri dischi invariati.
- `ASmartDisabledDiskLeavesTheSchemaAndReturnsWithItsId`: esce dallo schema con una nuova revisione; riattivato, torna con lo stesso id `lhm-…` (seriale univoco).
- `ASmartDisabledDiskStillHoldsTheD6Gate`: `AllActive = false` per quel disco, quindi `EnableStorageCount == 0` anche se è disattivato. Documenta il verdetto (c).
- `DriveKeyUsesTheTrimmedDescriptorModelAndSerial`: funzione pura, con lo stesso vettore del test Rust.
- `ADiskWithoutDescriptorSerialHasNoDriveKey`.
- `SmartStaysOnIfAnyStorageSubscriberWantsIt` e `ASubscriberWithStorageOffKeepsNoSmartOn`: aggregazione, vedi F2.
- Lato Rust (`crates/oma-win`): la chiave da `DriveEntry`; la traduzione da id core persistito a chiave al `Subscribe`; un id che non corrisponde a un disco presente non produce chiavi.
- Dal vivo: HDD SATA in standby con la finestra aperta; con lo SMART disattivato per quel disco dopo la discovery, il disco scende in standby e ci resta (nessuna riga "is active" nel log per quel drive).

---

## F2 — Protocollo di riconfigurazione dei worker

### F2.1 Il modello dei thread di oggi

| Esecutore | Dove | Cosa fa con LHM |
|---|---|---|
| Accept loop della pipe | `PipeListener.ExecuteAsync` (`Pipe/PipeListener.cs:63`), una `Task.Run` per client (`:146-156`) | nulla |
| Sessione client | `ClientSession.RunAsync` (`Pipe/ClientSession.cs:75-114`): loop di lettura e loop di scrittura asincroni sul thread pool | il reader chiama `_feed.Subscribe` (`:187-236`); il callback del feed accoda soltanto (`:239-264`) |
| `oma-sampler` | creato al primo `Subscribe` (`Sensors/SensorHub.cs:162-168`); `RunLoop` (`:897-934`) → `RunDue` (`:259-302`) → `TickOnce` (`:177-256`) | `Open()` (`OpenTree`, `:515-544`); `Update` di ogni radice non storage (`:189-200`; `Plan.UpdateRoots` esclude lo storage, `:958`); `Read` dei sensori non storage (`:234`); ricostruzione dello schema su `_structureDirty` (`:205-216`); consegna ai sottoscrittori sul proprio thread (`DeliverDue`, `:767-808`) |
| `oma-storage` | stesso punto; `RunStorageDue` (`:416-444`) ogni 30 s → `StorageOnce` (`:308-413`) | gate D6 ed `EnableStorage` (`:315`, `:698-735`); `Describe` e `IsSpunDown`; `Update` e `Read` dei soli dischi; pubblica `_storageCache` (`:412`) |
| Callback LHM | `HardwareAdded/Removed` su qualunque thread, anche quello di hot-plug di DIT (`LhmTree.cs:235-240`); `SensorAdded/Removed` sul thread che aggiorna (`LhmTree.cs` classe `Entry`) | registrano soltanto (flag `_membershipDirty`, `Entry.Dirty`, `_structureDirty`) |
| Arresto per inattività | `IdleShutdown` (`IdleShutdown.cs`), timer | nulla; conta le **connessioni**, non le sottoscrizioni |

**Lock e pubblicazione.**

- **Nel hub:**
  - `_subLock` protegge solo i sottoscrittori e le scadenze (`SensorHub.cs:61-73`), mai tenuto durante I/O o callback;
  - `Published` (revisione, schema e snapshot insieme) è un riferimento immutabile sostituito in blocco (`:90`, `:254`, `:940`);
  - `_storageCache` e `_resolvedDisks` sono volatili e immutabili dopo la pubblicazione (`:91-92`);
  - `_structureDirty` è un intero `Interlocked` (`:94`).
- **In `LhmTree`:** `_structureLock` (`LhmTree.cs:40`) protegge solo `_entries` e la composizione, mai durante I/O. `Reconcile` (`:242-279`) legge `computer.Hardware` (una copia fatta sotto il lock di LHM) e i sensori in memoria dell'hardware nuovo. `Update` (`:115-141`) cerca l'entry nella composizione corrente e, se manca, non fa nulla (`:117-120`).
- **Revisione dello schema:** calcolata solo dal sampler. `BuildPlan` (`SensorHub.cs:566-615`) incrementa la revisione quando `SchemaComparer.SameStructure` è falso; il `Published` di quel tick porta lo schema nuovo e lo snapshot costruito con i binding nuovi.
- **`EnableStorage` rispetto allo storage worker:** lo chiama lo storage worker stesso, all'inizio di un giro e dopo il gate (`SensorHub.cs:315-318`). Genera `HardwareAdded`; il sampler riconcilia e ricostruisce al tick successivo; i dischi entrano nello schema solo dopo essere stati risolti dallo storage worker (`:405-409`, `:576-579`). La creazione del gruppo storage avviene quindi **in parallelo agli `Update` del sampler**: è già in produzione dalla M4.

**Lacuna da chiudere comunque.** Il secondo `Subscribe` di una sessione oggi fa `previous.Dispose()` e poi `_feed.Subscribe(...)` (`ClientSession.cs:199-209`). Con un solo client il hub passa per zero sottoscrittori: `Unsubscribe` svuota `_storageCache` (`SensorHub.cs:824-829`) e, con l'aggregazione M5, la configurazione efficace oscillerebbe (storage acceso → spento → acceso). La spec vuole una sostituzione atomica (§2.8).

### F2.2 Cosa fa LHM quando cambia un setter `IsXEnabled` su un `Computer` aperto

- **Setter** (LHM `Computer.cs:154-169` CPU, `:218-233` memoria, `:296-311` storage; gli altri sono uguali): nessun lock. Legge `_open`, poi `Add(new XGroup(...))` oppure `RemoveType<XGroup>()`, poi scrive il flag. **Il costruttore del gruppo, cioè la detection con il suo I/O, gira sul thread chiamante, fuori da ogni lock.**
- **`Add`** (`Computer.cs:436-460`): sotto `_lock` inserisce il gruppo e si iscrive agli eventi `IHardwareChanged`; fuori dal lock genera `HardwareAdded` per ogni hardware.
- **`Remove`** (`Computer.cs:462-485`): sotto `_lock` toglie il gruppo e annulla l'iscrizione; fuori dal lock genera `HardwareRemoved` per ogni hardware, poi **`group.Close()`**.
- **Cosa chiude `Close`:**
  - `Amd17Cpu.Close` chiude il modulo PawnIO e l'SMU (LHM `Cpu/Amd17Cpu.cs:81-86`); `IntelCpu.Close` il modulo PawnIO (`Cpu/IntelCpu.cs:558-562`);
  - `Motherboard.Close` chiude la Super I/O (LPC) e gli lm-sensors (`Motherboard/Motherboard.cs:199-209`);
  - `MemoryGroup.Close` annulla il task di retry e chiude i DIMM (`Memory/MemoryGroup.cs:82-96`), che restano ai finalizzatori `~SPDAccessor`, i quali ripristinano la pagina SPD via SMBus (SPD `SPD/SPDAccessor.cs:33-37`; vedi anche il commento in `LhmTree.cs:170-183`);
  - `StorageGroup.Close()` è vuoto (LHM `Storage/StorageGroup.cs:82`).
- **Come LHM stesso evita la corsa.** Nella GUI i setter girano sul thread UI (LHM-UI `MainForm.cs:261`, `:276`), mentre gli update girano su un `BackgroundWorker` (`MainForm.cs:931-939`, `:561-563`) che passa da `Computer.Accept` → `Traverse`, e `Traverse` **tiene `_lock` per l'intero giro di update** (LHM `Computer.cs:411-424`; `UpdateVisitor.cs:13-20`). Quindi in LHM:
  - un gruppo si chiude solo dopo essere stato tolto sotto `_lock`, cioè mai durante un update in corso di **qualunque** gruppo;
  - la costruzione di un gruppo può invece sovrapporsi agli update degli altri.
- **Il nostro caso.** Noi non usiamo `Traverse`: chiamiamo `hardware.Update()` direttamente (`LhmTree.cs:334-341`), quindi **LHM non ci protegge**. Un `RemoveType<CpuGroup>` sul thread A mentre il thread B è dentro `Amd17Cpu.Update()` chiuderebbe il modulo PawnIO sotto l'update. Da qui la barriera richiesta dalla spec.
- **Eventi e memoria.**
  - `MemoryGroup` aggiunge i DIMM anche più tardi da un task del thread pool, con `HardwareAdded` (`Memory/MemoryGroup.cs:125-143`, `:176-196`): il nostro handler registra soltanto, va bene.
  - Disattivare e riattivare lo storage con il setter crea un **nuovo** `StorageGroup`, quindi un nuovo `ReloadStorages()` con identificazione di tutti i dischi, che richiede di nuovo il gate D6. Il gruppo vecchio resta iscritto a `StorageManager.StoragesChanged` (statico), perché `AddHardware` annulla solo la propria iscrizione (`StorageGroup.cs:37`) e `Close` non fa nulla: a ogni ciclo si perde un gruppo con i suoi `StorageDevice`.

### F2.3 Design proposto

**Principio.** Ogni gruppo LHM ha un solo thread proprietario, che è anche l'unico a chiuderlo. In più si aggiunge un parcheggio esplicito dello storage worker prima di qualunque setter, così si replica la garanzia di LHM ("nessun `Close` mentre un update è in corso") senza tenere lock durante l'I/O.

**Tipi, nello stile di `Sensors/`:**

```csharp
[Flags] public enum ServiceModules { None = 0, Cpu = 1, Motherboard = 2, Memory = 4, Storage = 8, Controller = 16, Psu = 32, All = 63 }

public sealed record FeedRequest(uint IntervalMs, ServiceModules Disabled, IReadOnlySet<string> SmartDisabledDrives);

/// Pure: a module is on if at least one request keeps it on; a drive's SMART is on if at
/// least one request with Storage on keeps it on. No requests: null (keep the last one).
public sealed record EffectiveConfig(ServiceModules Enabled, IReadOnlySet<string> SmartDisabledDrives)
{
    public static EffectiveConfig? Compute(IReadOnlyCollection<FeedRequest> requests);
}

public interface ISensorFeed { IFeedSubscription Subscribe(FeedRequest request, Action<FeedUpdate> onUpdate); }
public interface IFeedSubscription : IDisposable { void Update(FeedRequest request); } // atomic replace

// IHardwareTree
IReadOnlyList<HardwareNode> Open(ServiceModules enabled);   // Computer created with these groups only
void SetModules(ServiceModules enabled);                     // non-storage groups only; reconciles before returning
```

**Aggregazione** (in `SensorHub`, sotto `_subLock`). Ogni `Subscriber` tiene la propria `FeedRequest`.

- `Subscribe`, `IFeedSubscription.Update` e `Unsubscribe` ricalcolano insieme, nella stessa sezione critica, l'intervallo minimo (`RecomputeSamplingLocked`, `SensorHub.cs:834-844`) e `EffectiveConfig.Compute`.
- Se la configurazione cambia: `_desired = new ConfigRequest(version + 1, config, requestedAt)` (riferimento immutabile, volatile), poi sveglia il sampler e lo storage worker.
- I callback della pipe fanno solo questo: nessuna chiamata a LHM (§2.8).
- `Update` imposta anche `ForceSchema = true` sul sottoscrittore (letto dal sampler in `DeliverDue`), così il primo aggiornamento dopo ogni `Subscribe` porta lo schema e il client ha un punto di sincronizzazione sicuro.
- **Senza sottoscrittori** `Compute` restituisce `null` e `_desired` non cambia: resta l'ultima configurazione, niente oscillazioni tra una riconnessione e l'altra, e il ciclo di inattività della M4 invariato. Resta anche lo svuotamento di `_storageCache` all'uscita dell'ultimo client (`SensorHub.cs:824-829`).

**`ClientSession`.** Il primo `Subscribe` chiama `_feed.Subscribe(request, …)`; i successivi chiamano `_subscription.Update(request)`, senza `Dispose` e senza cambiare `_generation`. La decodifica rifiuta con `bad_request`:

- un modulo sconosciuto;
- più di 64 `driveKey` (come `MaxProbedDrives`, `DiskPowerProbe.cs:29`);
- una chiave che non è esadecimale di 64 caratteri.

App e servizio si installano insieme, quindi la rigidità non costa nulla.

**Apertura.** Al primo tick `OpenTree` usa `_desired.Config.Enabled` (senza storage, per D6): un modulo spento nelle impostazioni **non viene mai costruito**. È il caso d'uso anti-crash, e il più comune.

**Sequenza del sampler** (all'inizio di `RunDue`, prima di `TickOnce`; tutto sul thread `oma-sampler`):

1. Legge `_desired`. Se `desired.Version == _appliedVersion` non fa nulla.
2. **Effetto sullo schema, subito:** `_schemaConfig = desired.Config` e `_structureDirty = 1`.
   - `BuildPlan` esclude le radici dei moduli spenti (per `HardwareType`) e le radici storage se lo storage è spento o se la `driveKey` del disco è disattivata.
   - Il piano si ricostruisce **prima** del ciclo di update di questo tick, quindi le radici spente non si aggiornano più già da ora e lo snapshot del tick esce con lo schema nuovo, nello stesso `Published` (stessa revisione).
   - Una radice esclusa sparisce da `Plan.Owner/FromStorage`: la cache dei suoi valori non viene più letta.
3. **Effetto sui gruppi LHM non storage**, se `desired.Config.Enabled & ~Storage` differisce da `_appliedTreeModules`:
   - se `_parkAck != desired.Version`: scrive `_parkRequest = desired.Version`, sveglia lo storage worker e **continua a campionare** (non attende). Se `now - requestedAt > ReconfigureTimeout` (proposta: 15 s, `internal init` come `WorkerJoinTimeout`), passa allo stato `failed`, registra un warning una volta per richiesta e continua a riprovare a ogni tick: nessuna chiusura forzata;
   - se `_parkAck == desired.Version`: `_tree.SetModules(...)` (setter LHM e `Reconcile` sincrono in `LhmTree` prima di restituire, così nessuna entry di un gruppo chiuso resta nella composizione quando il sampler aggiorna di nuovo); poi `_appliedTreeModules = …`, `_structureDirty = 1`, `_parkRelease = desired.Version`, e sveglia lo storage worker. Un'eccezione dei setter porta a `failed` con il log, e il rilascio avviene comunque.
4. La richiesta è **applicata** quando i gruppi coincidono e lo storage worker ha confermato `_storageSeenVersion >= desired.Version` (sotto). A quel punto `_appliedVersion = desired.Version`.

**Storage worker** (`RunStorageDue`, ai confini di un giro, mai a metà):

- **Parcheggio:** se `_parkRequest > _parkAck`, scrive `_parkAck = _parkRequest`, sveglia il sampler e, finché `_parkRelease < _parkAck`, restituisce `Timeout.InfiniteTimeSpan`, cioè resta in `wake.Wait` (`SensorHub.cs:925-932`) senza I/O. Nessun lock tenuto durante l'I/O: solo interi `Volatile/Interlocked` e i `ManualResetEventSlim` già esistenti. Il `Dispose` interrompe l'attesa con il token (`:467-469`).
- **Configurazione storage, applicata dal suo proprietario e in modo soft:**
  - legge `_desired` e scrive `_storageSeenVersion = version` all'inizio del giro;
  - **storage spento:** se la cache non è vuota, pubblica `_storageCache = StorageCache.Empty` e `_resolvedDisks = {}` e imposta `_structureDirty = 1`; poi restituisce l'intervallo successivo senza gate D6, senza `Describe`, senza `IsSpunDown` e senza `Update`. **Il `StorageGroup` resta aperto:** niente `IsStorageEnabled = false`, per i motivi di F2.2 (nuovo `ReloadStorages` e nuovo gate a ogni riattivazione, gruppo perso a ogni ciclo). Chiuderlo non fermerebbe comunque il thread statico di hot-plug di DIT;
  - **storage acceso:** giro normale. Se `_storageEnabled` è falso passa dal gate D6 esattamente come oggi; i dischi con la chiave disattivata si risolvono (`Describe`, accesso 0), si escludono da `resolved` e si saltano senza controllo di stato né update (F1.4).

**Moduli non storage con il setter "duro".**

- Disattivare il modulo `controller` o `psu` deve rilasciare gli handle HID (per esempio per il software del vendor), e il caso anti-crash vuole il gruppo davvero chiuso.
- Riattivarlo ripete la detection sul sampler: per la memoria circa 2,7 s di SMBus/SPD (s1-lhm.md §9.6). È un ritardo di un tick, una tantum su azione dell'utente, da dichiarare nel piano.
- **Rischio da verificare:** dopo `IsMemoryEnabled = false` i finalizzatori `~SPDAccessor` scrivono su SMBus dal thread dei finalizzatori. Se la memoria si riattiva subito, la nuova detection sul sampler potrebbe sovrapporsi a quelle scritture. Da controllare se RAMSPDToolkit serializza SMBus con il mutex globale (SPD `Mutexes/WorldMutexManager.cs`). Mitigazione semplice: dopo aver rimosso la memoria, il sampler esegue `GC.Collect(); GC.WaitForPendingFinalizers();` con il driver ancora caricato, quindi senza il problema descritto in `LhmTree.cs:170-183`, che riguarda solo lo scaricamento del driver.

**Disconnessione di un client.** `CloseSubscription` → `Unsubscribe` → ricalcolo. Se restano sottoscrittori, la configurazione può cambiare (per esempio lo storage si spegne se lo voleva solo il client uscito). Se non ne restano, si conserva l'ultima configurazione.

**`Dispose` del hub invariato:** attende i worker con `WorkerJoinTimeout` e non chiude l'albero se uno è bloccato (`SensorHub.cs:451-502`). Un worker parcheggiato esce al segnale del token.

**Come il client conosce lo stato effettivo.** §2.3 vuole che `applyStatus` distingua gli effetti esterni "pendenti o falliti (servizio…)" e §2.8 che la UI distingua la richiesta locale dall'attività globale. Dedurlo dallo schema (i device di un tipo presenti = modulo acceso) non distingue "in attesa" da "fallito" né da "tenuto acceso da un altro client". Proposta minima, da decidere nel piano perché aggiunge un campo al protocollo v2 e la rigenerazione di `schema.msgpack` (§2.8 cita solo `hello` e `subscribe`):

- `SchemaMessage` aggiunge `service: { activeModules: [string], smartDisabledDrives: [string], reconfiguration: "applied" | "pending" | "failed", smartBlockedBy: [string] }`, con chiavi sempre presenti (regola R10):
  - `activeModules` e `smartDisabledDrives` sono i valori **globali** efficaci;
  - `smartBlockedBy` contiene le `driveKey` che tengono chiuso il gate D6, per il messaggio della UI di F1.4.
- Ogni cambio di questo blocco conta come cambio di struttura e incrementa la revisione, quindi arriva con lo snapshot coerente. Il `ForceSchema` dopo ogni `Subscribe` garantisce che il client veda uno stato successivo alla propria richiesta.
- La UI filtra comunque le fonti che il client ha escluso (§2.8).

**Rischi di F2.**

- **Moduli con zero hardware:** con la soluzione "deduci dallo schema" risultano indistinguibili da quelli spenti. Il blocco `service` risolve.
- **La soluzione "soft" per lo storage non libera memoria:** i `StorageDevice` restano. È accettabile rispetto al budget (< 80 MB con tutti i moduli accesi, §7 della spec M5); da misurare con `scripts/measure-footprint.ps1`.
- **Corse residue nel ciclo di vita dei gruppi LHM.** L'analisi del sorgente non mostra risorse condivise tra la costruzione dei gruppi non storage sul sampler e l'I/O disco dello storage worker. I mutex di bus di LHM (`Mutexes`) coordinano già detection e update. Il parcheggio serve per replicare la garanzia più forte di LHM, non per un difetto dimostrato.
- **Stato `failed` permanente:** se lo storage worker resta bloccato in una chiamata al driver (per esempio un bridge USB appeso), il setter non viene mai applicato e lo stato resta `failed`, mentre schema e update riflettono già la richiesta. È il comportamento voluto dalla spec (nessuna chiusura forzata).

### F2.4 Test xUnit

**Harness.** `SensorHubTests.Harness` (`StartWorkers = false`, `FakeTimeProvider`, `TickOnce`/`RunDue`/`StorageOnce`/`RunStorageDue` chiamati dal test, `SensorHubTests.cs:49-84`). `FakeTree` va esteso con:

- `OpenedModules`;
- `SetModules`, con contatore e nome del thread chiamante;
- `Initial` suddiviso per tipo, perché `SetModules` aggiunga o tolga le radici e generi `HardwareChanged`, come fa oggi `EnableStorage` (`SensorHubFakes.cs:100-109`).

`FakeFeed` (`Pipe/PipeTestSupport.cs:19`) va esteso con `Update(request)`.

**Aggregazione, funzione pura:**

- `AModuleStaysOnIfAnySubscriberWantsIt`;
- `SmartOfADriveNeedsASubscriberWithStorageOn`;
- `NoSubscribersKeepsTheLastConfiguration`;
- `ReplacingARequestIsAtomic`: nessuna configurazione intermedia "zero sottoscrittori".

**Hub:**

- `PipeCallbacksNeverTouchTheTree`: dopo `Subscribe` e `Update`, `SetModules` ed `EnableStorage` restano a 0 finché non girano `RunDue`/`RunStorageDue`.
- `ModulesDisabledBeforeTheFirstTickAreNeverOpened`: `OpenedModules` senza il modulo.
- `DisablingAModuleDropsItsDevicesInTheSnapshotsRevision`: lo schema nuovo e il primo snapshot con i valori allineati (helper `ValueOf`) nello stesso aggiornamento; `Updates(root)` fermo da quel tick.
- `SettersWaitForTheStorageWorkerToPark`: senza `RunStorageDue` nessun `SetModules`; dopo `RunStorageDue` (parcheggio) il `RunDue` successivo applica; lo storage worker riprende solo dopo il rilascio.
- `ABlockedStorageWorkerLeadsToFailedWithoutApplying`: `FakeTree.BeforeUpdate` blocca l'update di un disco su un thread reale, come in `SlowStorageDoesNotBlockCpuSampling` (`SensorHubTests.cs:391`); il test avanza il tempo oltre `ReconfigureTimeout`, poi verifica stato `failed`, un solo warning, campionamento CPU continuo, nessun `SetModules`. Dopo lo sblocco: applicato.
- `DisablingStorageClearsItsCacheAndStopsDiskIo`: da quel giro nessuna `IsSpunDown`, `Describe`, `AllRotationalDisksActive` o `Update` dei dischi; schema senza dischi.
- `ReEnablingStorageDoesNotReloadTheGroup`: `EnableStorageCount` resta 1; i controlli di stato per disco ripartono.
- `StorageEnabledForTheFirstTimeLaterStillGoesThroughTheD6Gate`.
- `ResubscribeDoesNotDropTheStorageCache`: un solo client che cambia intervallo mantiene i valori dei dischi.
- `EveryAcceptedSubscribeIsFollowedByASchema`.
- `LastSubscriberLeavingKeepsTheEffectiveConfiguration`.
- `DisposeReleasesAParkedStorageWorker`: con i worker veri, come `WorkersOpenOnTheSamplerThreadAndNeverUpdateAfterClose` (`SensorHubTests.cs:600`).

**`LhmTree`**, sul `Computer` senza gruppi già usato in `LhmTreeTests.cs:35`:

- `OpenCreatesOnlyTheRequestedGroups`: verifica i flag `IsXEnabled` del `Computer` passato dal seam `createComputer`;
- `SetModulesNeverTouchesStorage`.

**Pipe e codec:**

- `ResubscribeUpdatesTheRequestWithoutUnsubscribing`, che estende `ResubscribeChangesTheInterval` (`PipeListenerTests.cs:64`);
- `UnknownModuleIsABadRequest`;
- `TooManyDriveKeysIsABadRequest`;
- fixture `subscribe.msgpack`, `hello.msgpack` ed eventualmente `schema.msgpack` confrontate byte per byte in `CodecTests` e nei test Rust.

**Dal vivo** (niente input sintetico: le azioni nella finestra le fa l'utente). Con il servizio installato e l'app aperta, l'utente spegne e riaccende ogni modulo:

- il log non deve mostrare eccezioni né riavvii del servizio;
- la memoria riattivata torna in pochi secondi;
- con due client (per esempio due sessioni utente) il modulo resta acceso finché uno lo vuole.

---

## F3 — Stato di PawnIO

### F3.1 Cosa rileva oggi `PawnIoProbe`

`PawnIoProbe.IsAvailable()` (`service/OpenMonitorAdvanced.Service/Sensors/PawnIoProbe.cs:24-55`) restituisce un `bool`: `installed && opens`, dove:

- `installed` significa che la chiave `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO` esiste nella vista a 64 bit (`:17`, `:28-38`); un'eccezione di registro vale "non installato", con un warning;
- `opens` significa che `CreateFileW("\\?\GLOBALROOT\Device\PawnIO", GENERIC_READ|GENERIC_WRITE)` riesce (`:18`, `:42-44`).

Registra la versione e il codice Win32, ma non li classifica. Il hub lo chiama una volta, all'apertura dell'albero (`SensorHub.cs:519`), e il `bool` decide solo se lo schema pubblica i sensori che dipendono da PawnIO (`SchemaBuilder.cs:68-89`).

**Cosa non distingue:** installato ma driver non caricato; accesso negato (per esempio `oma-service run` da una console non elevata); diagnosi fallita; riavvio necessario. Inoltre `Hello` parte appena il client si connette (`ClientSession.cs:80`), prima che il hub apra l'albero: lo stato per `Hello` va calcolato indipendentemente dall'apertura.

### F3.2 Segnali disponibili realisticamente

| Segnale | Cosa prova | Note |
|---|---|---|
| Chiave `Uninstall\PawnIO` (vista a 64 bit) con `DisplayVersion` | il setup di PawnIO è stato eseguito | l'installer usa la stessa chiave (`app/src-tauri/nsis/oma.nsh:26`, `:569-605`) |
| `CreateFile` del device ed errore Win32 | il driver è caricato e accessibile. Errore 2/3 (oggetto device assente): driver non caricato; 5: accesso negato | i codici esatti con driver assente vanno verificati dal vivo (sotto) |
| Servizio driver `PawnIO` nell'SCM (`QueryServiceStatusEx`) | installato come driver (tipo 1) e in esecuzione o fermo. Su questo PC: `KERNEL_DRIVER`, `RUNNING`, `Start = 3`, `ImagePath` nel DriverStore (`oem123.inf`) | utile per il dettaglio di `unavailable` (installato ma fermo); non prova un riavvio pendente |
| Devnode `ROOT\PAWNIO\0000` (`HKLM\SYSTEM\CurrentControlSet\Services\PawnIO\Enum`) con `CM_Get_DevNode_Status` | stato PnP del boot corrente; `CM_PROB_NEED_RESTART` (14) o `DN_NEED_RESTART` sono evidenza del sistema operativo riferita al boot corrente | API documentata di cfgmgr32; da verificare in VM se il caso 3010 di PawnIO produce davvero questo stato. È evidenza dell'OS, non "dell'installer" come chiede §2.8: si può usare solo come conferma |
| `PendingFileRenameOperations` | un qualunque programma ha operazioni in attesa di riavvio | non specifico di PawnIO: **scartato** |
| Marcatore dell'installer | oggi **non esiste**: con l'exit code 3010 l'installer chiama solo `SetRebootFlag true` (`oma.nsh:599-600`) e propaga 3010 (`:518-522`) | da aggiungere (sotto) |

**Marcatore proposto (evidenza esplicita dell'installer).**

- **Scrittura:** nel ramo `3010` di `oma.nsh:599-600` l'installer (elevato) scrive `HKLM\SOFTWARE\OpenMonitorAdvanced` `PawnIoRebootRequestedUtc` = FILETIME UTC dell'istante corrente, come stringa decimale (`System::Call 'kernel32::GetSystemTimeAsFileTime(*l .r0)'`).
- **Rimozione:** la toglie il disinstallatore. Il servizio non la scrive e non la cancella.
- **Confronto:** il servizio calcola il boot corrente come `DateTime.UtcNow - TimeSpan.FromMilliseconds(Environment.TickCount64)`. `GetTickCount64` include il tempo di sospensione e ibernazione; con l'avvio rapido di Windows lo spegnimento è un'ibernazione del kernel e **non** applica un driver in attesa, quindi il marcatore resta "del boot corrente" finché l'utente non sceglie Riavvia. È il comportamento giusto: la UI deve dire «Riavvia», non «spegni e riaccendi».
- **Marcatore del boot corrente:** `markerUtc > bootUtc`. Un marcatore di un boot precedente è irrilevante di per sé: niente pulizia necessaria.

### F3.3 Mappatura minima e onesta

Calcolata **una volta per processo** in `ServiceHost.CreateSensorHub` (`ServiceHost.cs:99-108`), come valore pigro condiviso tra `Hello` (via `PipeListenerOptions` o un piccolo provider iniettato in `ClientSession`) e il hub (`_pawnIoAvailable = () => status == Ok`). Il servizio si avvia su richiesta ed esce dopo 2 minuti di inattività, quindi un cambio di stato si vede alla connessione successiva. La mappatura è una funzione pura `PawnIoStatus Classify(keyState, openError, markerUtc, bootUtc)`:

| Device | Chiave | Marcatore | Stato |
|---|---|---|---|
| si apre | qualsiasi | — | `ok` (con chiave assente, un log informativo; è un cambiamento rispetto a oggi, dove serve anche la chiave: da decidere, oppure si tiene `ok` solo con la chiave e chiave assente diventa `unknown`) |
| errore 2/3 (device assente) | assente | — | `missing` |
| errore 2/3 | presente | del boot corrente | `rebootPending` |
| errore 2/3 | presente | assente o di un boot precedente | `unavailable` (installato ma non caricato: driver fermo o bloccato, per esempio dalla blocklist dei driver vulnerabili o da HVCI) |
| errore 5 o altro errore | presente | — | `unavailable` (presente ma non accessibile) |
| altro errore | assente | — | `unknown` |
| lettura del registro fallita | — | — | `unknown` (a meno che il device si apra: allora `ok`) |

Lo stato SCM e il devnode si possono aggiungere al log come dettaglio, senza cambiare la mappatura. Se il proprietario della spec accetta l'evidenza dell'OS, `CM_PROB_NEED_RESTART` può diventare una seconda condizione per `rebootPending`.

**Verifiche dal vivo** (lo scenario VM "the reboot PawnIO requests (exit code 3010)" è già tra quelle dovute, `docs/follow-ups.md:59`):

- codice di `CreateFile` con il driver fermo (`sc stop PawnIO` in una VM) e con PawnIO disinstallato: atteso 2 o 3;
- codice con `oma-service run` da una console non elevata: atteso 5;
- installazione che termina con 3010 in VM: il marcatore viene scritto, il servizio risponde `rebootPending`, e dopo Riavvia risponde `ok`;
- stato del devnode nello stesso scenario, per decidere se usarlo.

**Test xUnit:**

- `Classify` guidata da tabella, con tutte le righe sopra;
- parsing del marcatore (stringa non numerica: ignorata, con un log);
- `HelloCarriesThePawnIoStatus` (estende `ClientReceivesHelloThenSchemaAndSnapshot`, `PipeListenerTests.cs:23`);
- fixture `hello.msgpack` rigenerata con `OMA_WRITE_FIXTURES=1`.

Lato installer: controllo statico del ramo 3010 (il commento in `oma.nsh:9-12` cita già un controllo statico) e rimozione della chiave nell'hook di disinstallazione.

---

## Impatto sul piano M5a

- **Decisione D6 nel piano:** verdetto (c). Il gate globale resta; l'interruttore SMART per disco agisce solo dopo la discovery (nessun `CHECK POWER MODE`, nessun `Update`, disco fuori dallo schema del servizio); la UI dichiara il limite. Aggiornare `docs/follow-ups.md:10`: la soluzione proposta è il fallback SAT, non un interruttore. Aggiungere il buco dell'hot-plug di DIT (reidentificazione con `WakeUp` a ogni `DBT_DEVNODES_CHANGED` dei dischi non identificati).
- **Correzione della spec §2.8:** `smartDisabledDrives` porta `driveKey` = `sha256(trim(model) \0 trim(serial))` del descrittore, non gli id `lhm-…`; l'app persiste l'id core e traduce al `Subscribe`; i dischi senza modello o seriale non sono selezionabili.
- **Task "chiave di disco":** funzione pura in .NET e in Rust con un vettore di test condiviso; `IDiskPowerProbe` espone l'enumerazione (o le chiavi dei `DriveBlocker`) per `smartBlockedBy`.
- **Task "protocollo v2":**
  - `PROTOCOL_VERSION = 2`;
  - `Subscribe { intervalMs, disabledModules, smartDisabledDrives }` con validazione rigida;
  - `Hello { …, pawnIo }`;
  - decisione sul blocco `service` in `SchemaMessage` (consigliato, serve a `applyStatus`);
  - fixture rigenerate a thread singolo;
  - test byte per byte nei due linguaggi.
- **Task "sostituzione atomica":** `ISensorFeed.Subscribe(FeedRequest, …)` restituisce `IFeedSubscription` con `Update`; `ClientSession` non fa più `Dispose` + `Subscribe`; schema forzato dopo ogni richiesta accettata.
- **Task "aggregazione":** `EffectiveConfig.Compute` pura; `_desired` versionato sotto `_subLock`; nessun cambio senza sottoscrittori.
- **Task "applicatore nel sampler":**
  - `Open(ServiceModules)`;
  - `SetModules` con `Reconcile` sincrono in `LhmTree`;
  - filtro dello schema immediato;
  - handshake `_parkRequest`/`_parkAck`/`_parkRelease` con lo storage worker;
  - `ReconfigureTimeout` con stato `failed`, senza chiusure forzate;
  - valutare `GC.WaitForPendingFinalizers` dopo la rimozione della memoria.
- **Task "storage soft":** storage spento = nessun giro e cache e dischi risolti svuotati; mai `IsStorageEnabled = false` a runtime; riattivazione senza `ReloadStorages`; prima abilitazione sempre dal gate D6.
- **Task "PawnIoStatus":** `Classify` pura e calcolo una volta per processo, condiviso tra `Hello` e hub; marcatore `PawnIoRebootRequestedUtc` scritto nel ramo 3010 di `oma.nsh` e rimosso alla disinstallazione.
- **Verifiche manuali da mettere nel piano** (niente input sintetico: le chiede all'utente):
  - toggle di ogni modulo dal vivo, con due client;
  - HDD SATA in standby con lo SMART disattivato per quel disco;
  - chiavetta USB e gate D6 (seek penalty ignota);
  - scenari PawnIO in VM (driver fermo, disinstallato, 3010);
  - misura del budget con `scripts/measure-footprint.ps1` dopo toggle ripetuti (fughe di memoria o di handle).
- **Fuori dalla M5a, ma da pianificare:** spike del fallback SAT per `CHECK POWER MODE` sull'HDD USB dell'utente. È la sola strada che chiude davvero il follow-up senza toccare LHM.
