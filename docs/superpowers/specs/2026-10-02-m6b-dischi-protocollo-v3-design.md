# M6b — Dischi e protocollo v3: design di dettaglio

- **Data:** 2026-10-02
- **Stato:** design del brainstorming del 2026-10-02, rivisto sul codice locale. Le verifiche del §9.2 sono parte del piano, non fatti già acquisiti; la revisione della spec non certifica gli esiti sul dispositivo.
- **Spec principale:** `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`. Per i punti trattati qui (lettura della temperatura dei dischi, gate D6, protocollo tra app e servizio, copertura delle regole sui dischi) questa spec ha la precedenza; il resto resta come nella spec principale e nella spec M5.
- **Riferimenti:** `docs/superpowers/references/m5/f1-service-reconfiguration.md` (F1.3, F1.4: gate D6, chiave di disco, fallback SAT), `docs/follow-ups.md`.

## 1. Intento e confini

La M6b è il secondo dei tre piani della M6. Fa quattro cose:

1. corregge un difetto trovato nello spike: **la lettura della temperatura del nucleo sveglia gli hard disk in standby** e impedisce a Windows di spegnerli;
2. aggiunge al servizio il **fallback SAT** per `CHECK POWER MODE`, così un disco USB non tiene più spento lo SMART di tutti i dischi;
3. porta il protocollo tra app e servizio alla **versione 3**, con lo stato di ogni disco;
4. usa quello stato nell'interfaccia e nelle regole.

**Obiettivo misurabile:** con l'app aperta, un HDD messo in standby resta in standby, e Windows riesce a spegnerlo dopo il tempo impostato nel piano di alimentazione. Con una chiavetta USB collegata, lo SMART degli altri dischi resta acceso.

**Fuori dalla M6b:**

- la ri-identificazione dei dischi all'hot-plug da parte di DiskInfoToolkit (F1.1): limite dichiarato e segnalazione all'autore, nessun codice;
- il comportamento di un hard disk USB in standby dietro un bridge: non verificabile, perché manca l'hardware; limite dichiarato (§8);
- una regola SATA basata su `SmartInfo.DiskStatus` e gli altri follow-up marcati "when touched".

### 1.1 Decisioni del brainstorming

| # | Decisione |
|---|---|
| D1 | Prima lo spike dal vivo, poi la spec. Lo spike ha usato un programma di prova usa e getta (`target/spike/m6b/sat-probe.ps1`, non tracciato). |
| D2 | Temperatura degli HDD, **due percorsi**: quando il servizio offre una temperatura utilizzabile il valore arriva dal servizio; altrimenti il nucleo legge solo dopo attività recente e in assenza dei veti del §5.2. |
| D3 | Dischi USB, scelta **prudente**: la risposta SAT serve a sbloccare gli altri dischi; lo SMART del disco USB resta spento finché l'utente non lo accende (§4.2). |
| D4 | Ri-identificazione all'hot-plug: limite dichiarato e segnalazione a monte. |
| D5 | Protocollo v3 con **stato per disco** (`drives`), non il solo flag `smartGateClosed` previsto nei follow-up (§3). |
| D6 | La release che contiene la M6b è la 0.4.0 e porta anche la correzione dell'installer italiano (commit `57fa804`). |
| D7 | Nelle regole, lo standby confermato sospende le regole su temperatura e SMART del disco; «Inattivo» (senza servizio) sospende la sola regola sulla temperatura. Senza questa scelta, chi ha un HDD e non usa il servizio vedrebbe il banner "dati incompleti" quasi sempre (regola predefinita `disk-temp`). |

## 2. Esito dello spike (2026-10-02)

Macchina: PC di sviluppo. Disco 0: HDD SATA Seagate ST2000DM008, volume `D:`. Disco 4: chiavetta USB SanDisk Extreme (bus 0x07, supporto rimovibile, seek penalty non interrogabile: errore 1). Lo standby è stato forzato con `STANDBY IMMEDIATE` (0xE0) via SAT; lo stato è stato letto con `CHECK POWER MODE` (0xE5).

| Prova | Esito |
|---|---|
| `CHECK POWER MODE` nativo (`IOCTL_ATA_PASS_THROUGH`) su HDD in standby, ripetuto | risponde `count 0x00` in 2 ms, il disco resta fermo |
| Lo stesso via SAT a 16 byte (`IOCTL_SCSI_PASS_THROUGH`, CDB 0x85), ogni 30 s per 5 minuti | come sopra: il disco resta in standby |
| SAT a 12 byte (CDB 0xA1) sull'HDD SATA | errore Win32 1: non si usa |
| Chiavetta USB: comando nativo | errore Win32 50 (come nel log del servizio) |
| Chiavetta USB: SAT a 16 byte | risponde `status 0x50, count 0xFF` (attivo) in 1 ms |
| App aperta, servizio fermo (modalità anti-cheat), HDD in standby | il disco riparte entro 35 s |
| App chiusa | il disco resta in standby per 5 minuti |
| Provider del nucleo uno alla volta | solo `storage` sveglia il disco; `cpu`, `gpu`, `memory`, `network` no |
| `storage` a regime (discovery a disco sveglio, poi standby) | un solo poll lento, a 32 s, di 2907 ms: il risveglio |
| `IOCTL_STORAGE_QUERY_PROPERTY` con `StorageDeviceTemperatureProperty` (52) su HDD in standby | **2868 ms, poi disco attivo**; a disco sveglio 22 ms |
| Layout delle partizioni, geometria, descrittore, `GetDiskFreeSpaceExW`, `GetVolumeNameForVolumeMountPointW`, contatori PDH | innocenti: il disco resta in standby |
| `GetDevicePowerState` su HDD in standby forzato | `on=True`: Windows conosce solo gli spegnimenti decisi da lui |
| Servizio con HDD in standby | log `/hdd/0 is in standby: SMART skipped`: non lo sveglia |
| Windows, timeout disco 60 s, app chiusa, nessuna attività | spegne il disco (`on=False`) |
| Lo stesso con l'app aperta, 10 minuti senza attività | il disco resta acceso |

**Causa.** `read_temperatures` (`crates/oma-win/src/storage.rs`) interroga la temperatura alla discovery e poi ogni 30 s (`TEMPERATURE_PERIOD`), protetta solo da `GetDevicePowerState`. Su un HDD SATA quella query arriva al disco: lo sveglia se dorme e, a disco sveglio, azzera il timer di inattività di Windows.

**Byte catturati**, da usare come vettori di test (sense data di `IOCTL_SCSI_PASS_THROUGH`):

- HDD SATA attivo, stato SCSI 0x00: `72 00 00 00 00 00 00 0E 09 0C 00 00 00 FF 00 FF 00 00 00 00 E0 50` (formato a descrittori; anche con `82` al byte 15);
- HDD SATA in standby, stato SCSI 0x00: `72 00 00 00 00 00 00 0E 09 0C 00 00 00 00 00 00 00 00 00 00 E0 50`;
- chiavetta USB attiva, stato SCSI 0x02: `F0 00 01 00 50 00 FF 0A 00 00 00 00 00 1D 00 00 00 00` (formato fisso, ASC/ASCQ 00/1D).

**Fuori dal nostro codice.** Con TR-VISION HOME (`WFanManager.exe`) aperto, che tiene le richieste di alimentazione `DISPLAY` e `SYSTEM`, Windows non ha spento il disco in oltre 150 s di inattività; dopo averlo chiuso lo ha spento. È un indizio, non una prova.

## 3. Protocollo v3

`PROTOCOL_VERSION` passa da 2 a 3 (`crates/oma-ipc`, `service/.../Protocol/`). App e servizio si installano insieme: una versione diversa dà `Incompatible`, come oggi. Valgono le regole esistenti: chiavi sempre presenti, `nil` per gli assenti, fixture rigenerate solo con `OMA_WRITE_FIXTURES=1` a thread singolo.

### 3.1 Blocco `service` dello schema

Si aggiunge `drives` e si toglie `smartBlockedBy`:

```
service: {
  activeModules: [string],
  smartDisabledDrives: [string],
  reconfiguration: "applied" | "pending" | "failed",
  drives: [ { physicalDrive: u32, key: string | nil, model: string | nil,
              state: string, blocksSmart: bool } ]
}
```

- Una voce per ogni disco fisico che `DiskPowerProbe` enumera, in ordine di `physicalDrive`, anche mentre il gate D6 è chiuso e anche per dischi che LHM non espone. Con il modulo storage spento si conserva l'ultimo elenco noto, con `state = "smartOff"` e `blocksSmart = false`: non si enumera e non si esegue I/O periodico per mantenerlo aggiornato. Se storage non è mai stato acceso, l'elenco è vuoto. Alla riattivazione si aggiorna prima di leggere SMART.
- `key` è la `driveKey` esistente (`sha256(trim(model) \0 trim(serial))`), `nil` se il descrittore non ha modello o seriale. `model` è il modello del descrittore, `nil` se assente.
- `state`, con questa precedenza quando storage è acceso (quando è spento vale il caso del precedente punto):

  | Valore | Significato |
  |---|---|
  | `noMedia` | il driver riferisce supporto assente/non pronto; non è una prova che un HDD USB sia sveglio |
  | `smartOff` | il servizio non interroga questo disco: storage spento, disco in `smartDisabledDrives`, oppure disco spento di default e non abilitato (§4.2) |
  | `standby` | `CHECK POWER MODE` risponde standby |
  | `active` | risponde attivo, oppure il disco non richiede il controllo (`RequiresPowerCheck` falso: NVMe, SSD, virtuali) |
  | `unknown` | richiede il controllo e nessuna via risponde |

- `state` descrive la disponibilità del percorso SMART, non la freschezza di una temperatura. `smartOff` può nascondere una risposta di standby usata dal gate: non autorizza una lettura del nucleo.
- `blocksSmart` è `true` per i dischi che tengono chiuso il gate D6 prima della prima abilitazione di LHM storage. "Gate chiuso" equivale a "almeno una voce con `blocksSmart`": il flag `smartGateClosed` dei follow-up non serve. Dopo l'abilitazione tutti i `blocksSmart` sono falsi: un HDD che si addormenta sospende solo i propri aggiornamenti. Storage spento non ha un gate in attesa.
- Ogni cambio di `drives` è un cambio di struttura: incrementa la revisione e arriva con lo snapshot coerente.
- Un valore di `state` sconosciuto al client si tratta come `unknown`.

### 3.2 `Subscribe`

Si aggiunge `smartEnabledDrives: [string]`, le `driveKey` dei dischi spenti di default che il client vuole accesi. Validazione rigida, come per `smartDisabledDrives`: al massimo 64 chiavi, ognuna esadecimale minuscola di 64 caratteri; una chiave presente in entrambi gli elenchi è `bad_request`.

### 3.3 Lato app

- `ServiceSources` (`crates/oma-ipc/src/status.rs`) espone `drives` e perde `smart_blocked_by`; la vista Fonti ricava i nomi dei dischi bloccanti da `drives`.
- Il provider `storage` legge dal feed del servizio una vista immutabile di stato e temperature coerente con schema, revisione e connessione. Non dipende dall'ordine dei poll dei provider `storage` e `svc`, che lavorano in parallelo. Uno stato si associa a un disco core solo con `physicalDrive` e `key` entrambi corrispondenti e chiave non ambigua (§7).

### 3.4 Qualità dei valori nello snapshot

Si aggiunge allo snapshot v3 `held: [bool]`, con la stessa lunghezza e lo stesso ordine di `values`. Ogni elemento indica che il valore è conservato da una misura precedente, non appena misurato. Per `nil` il flag è `false`; lunghezze diverse o `nil` con `held = true` sono errori di protocollo. Il servizio conserva questa informazione dalla lettura del worker fino alla pubblicazione: un nuovo `seq` non rende nuova una misura SMART. Anche la ripubblicazione della cache storage tra due giri è `held`, dopo la prima pubblicazione della misura. Il client combina il flag con la ripetizione dello snapshot e con l'eventuale mancata risposta del provider. Questo è distinto dalla scadenza del collegamento: un feed scaduto non è una misura conservata volontariamente.

## 4. Servizio

### 4.1 Fallback SAT in `DiskPowerProbe`

- `IsSpunDown` prova prima `IOCTL_ATA_PASS_THROUGH`, come oggi. Se non produce una risposta interpretabile (anche con IOCTL riuscita), prova `IOCTL_SCSI_PASS_THROUGH` con `ATA PASS-THROUGH(16)`: CDB `85 06 20 00 00 00 00 00 00 00 00 00 00 00 E5 00` (protocollo non-data, `CK_COND = 1`), `DataIn = SCSI_IOCTL_DATA_UNSPECIFIED`, 32 byte di sense, timeout 5 s. Una risposta nativa valida di standby non attiva il fallback.
- I registri si leggono dal sense data, **senza richiedere lo stato SCSI CHECK CONDITION** (l'HDD SATA risponde con stato 0x00):
  - formato a descrittori (codice 0x72 o 0x73): descrittore 0x09 di lunghezza almeno 0x0C; error al byte 3, sector count al byte 5, status al byte 13 del descrittore;
  - formato fisso (0x70 o 0x71) con ASC/ASCQ 00/1D: error al byte 3, status al byte 4, sector count al byte 6.
  - Poi `InterpretAtaResult`, invariato. Nessun registro: `null`.
- Il parser maschera il bit VALID del response code (il vettore USB inizia con `F0`), rispetta la lunghezza effettivamente restituita e quella dichiarata dal sense data e controlla i limiti di ogni descrittore. Risposte troncate, registri assenti e risultati ATA non interpretabili restano `unknown`: non aprono il gate.
- Per ogni disco si ricorda la via che ha dato una risposta interpretabile (nativa, SAT, nessuna), per non ripetere a ogni giro un comando che il driver rifiuta. Se la via scelta smette di rispondere, si prova l'altra; nessun vecchio esito "attivo" viene riutilizzato per autorizzare SMART. Il ricordo si azzera alla rimozione/riapparizione o quando modello o seriale del `PhysicalDriveN` cambiano; senza identità verificabile non si conserva attraverso rediscovery. La via "nessuna" si riprova al più ogni 5 minuti, restando `unknown` tra i tentativi.
- Struct di `SCSI_PASS_THROUGH` con assert di dimensione (56 byte su x64) nei test, come per `ATA_PASS_THROUGH_EX`.

### 4.2 Dischi spenti di default

- Un disco con bus USB (`BusType` 0x07) è **spento di default**: dopo l'identificazione iniziale non riceve SMART né controlli di stato periodici, ed esce dall'elenco dei dispositivi/sensori SMART come un disco in `smartDisabledDrives`; resta nella tabella diagnostica `service.drives`.
- Diventa acceso se almeno un sottoscrittore con lo storage attivo lo elenca in `smartEnabledDrives`. Un disco senza `key` non si può accendere.
- **Gate D6.** Anche i dischi disabilitati esplicitamente partecipano al gate prima dell'identificazione, perché la prima identificazione di LibreHardwareMonitor tocca tutti i dischi (F1.1). Si controllano solo quelli con `RequiresPowerCheck = true`: standby o risposta sconosciuta bloccano, anche con `state = "smartOff"`; una risposta valida di attività sblocca. Per quelli senza power check non si invia il comando. Con il fallback SAT la chiavetta dello spike risponde e non blocca più. Il gate non viene riaperto a ogni successivo standby né a una riattivazione soft di storage già identificato.
- `EffectiveConfig.Compute` (pura) estende l'aggregazione: lo SMART di un disco spento di default è acceso se almeno una richiesta con lo storage attivo lo abilita; per gli altri dischi resta la regola di oggi.

### 4.3 Elenco `drives`

Lo compila lo storage worker a ogni giro (30 s) con storage attivo, dai `DriveFacts` e dalle risposte che ha già, compresi i dischi assenti dalle radici LHM. L'enumerazione accesso 0 necessaria a mantenere l'elenco non deve eseguire SMART né identificazione LHM. Con il gate chiuso conserva gli esiti di tutti i controlli, non solo l'elenco restituito da `FindGateBlockers`, per non interrogare due volte lo stesso disco. Stati, schema e cache dei valori si pubblicano come una vista coerente; nessun lock condiviso resta acquisito durante I/O. Con storage spento vale il §3.1.

### 4.4 Lettura SMART e timer di inattività di Windows (condizionale)

Lo spike non ha misurato se la lettura SMART del servizio, ogni 30 s su un HDD attivo, impedisce a Windows di spegnerlo. V3 (§9.2) è un gate obbligatorio di accettazione: un fallimento non si archivia come limite. Il piano prevede prima una prova del servizio isolato per decidere il ramo, poi V3 sull'app completa per verificarlo.

**Se la lettura del servizio impedisce lo spegnimento**, si aggiunge una misura passiva dell'attività per disco nel worker. Il codice del servizio oggi non legge direttamente `IOCTL_DISK_PERFORMANCE`: l'eventuale probe, la baseline e la sua innocuità sono lavoro da pianificare, non infrastruttura già disponibile. Una misura mancante, una baseline iniziale o un contatore resettato non dimostrano attività. L'autorizzazione usa una finestra recente e limitata, come i 10 s del §5.2, e contatori che non includano le nostre query; il solo delta accumulato negli ultimi 30 s non basta. La cadenza di osservazione deve permettere questa distinzione senza eseguire `Update` per ottenere i contatori. `Update` di un disco rotazionale o ignoto richiede sia stato attivo confermato sia attività recente; altrimenti i valori precedenti sono `held` (§3.4). Se la prova isolata passa, non si aggiunge questo filtro, ma V3 resta obbligatoria. Un risultato inconcludente lascia aperto il gate di accettazione.

In entrambi i rami lo standby confermato conserva l'ultima temperatura valida come `held`, senza fingere una nuova misura. `unknown`, `noMedia`, un errore di lettura o una fonte disabilitata non sono una sospensione normale: i dati SMART correnti restano assenti. La cache conservata per lo standby è distinta dalla cache di trasporto con scadenza (`FreshStorageValues`, oggi 60 s); non si estende la validità di una connessione guasta.

## 5. Nucleo dell'app (`crates/oma-win`)

### 5.1 Classe del disco

Alla discovery il provider `storage` stabilisce per ogni disco, con handle ad accesso 0:

- **non rotazionale / virtuale:** bus NVMe, virtuale, file-backed virtuale o Storage Spaces, oppure `StorageDeviceSeekPenaltyProperty` risponde `false`, coerentemente con `DriveFacts.RequiresPowerCheck` del servizio;
- **rotazionale o ignoto:** tutti gli altri (seek penalty `true`, oppure query fallita, come sulla chiavetta).

### 5.2 Quando leggere la temperatura

La decisione è una funzione pura, usata sia alla discovery sia nel refresh ogni 30 s:

| Disco | Stato dal servizio | Azione |
|---|---|---|
| qualsiasi | `noMedia` oppure `GetDevicePowerState = false` | non legge |
| non rotazionale / virtuale | qualsiasi altro stato | legge, come oggi |
| rotazionale o ignoto | `standby` | non legge, anche se manca una temperatura SMART |
| rotazionale o ignoto | `active`, temperatura del servizio disponibile e fonte accettata dal client | usa il servizio, non interroga la temperatura locale |
| rotazionale o ignoto | `active` senza temperatura utilizzabile, `smartOff`, `unknown` o servizio non collegato | legge solo con attività recente, salvo il veto del gate |

- Resta la protezione di oggi: con `GetDevicePowerState` falso non si legge mai.
- **Veto del gate:** una voce associata con `blocksSmart = true` vieta anche la query locale di temperatura. `smartOff` non consente di aggirare uno standby o uno stato sconosciuto che stanno bloccando l'identificazione. I/O PDH e spazio dei volumi continuano normalmente.
- **Attività recente:** in almeno un poll valido degli ultimi 10 s il contatore PDH di lettura o di scrittura del disco è finito e maggiore di zero. Il primo poll di riscaldamento, contatori mancanti/non validi o un gap di campionamento non autorizzano la lettura. La finestra si azzera a cambio d'identità e dopo sospensione; non si usa attività vecchia per leggere al risveglio. Che le nostre query non alimentino questi contatori è un requisito da verificare dal vivo, non un esito già dimostrato dallo spike.
- Una lettura scaduta e non eseguita **resta scaduta**: si rivaluta a ogni poll senza I/O, e parte al primo poll con attività. La regola "un disco per poll" resta: si sceglie il più vecchio tra i dischi scaduti **e autorizzati**, così un HDD fermo non impedisce il refresh degli altri. Solo un tentativo effettivo aggiorna il termine; un tentativo fallito mantiene il normale intervallo prima di riprovare.
- Alla discovery un disco rotazionale senza attività nota non viene letto: i suoi sensori di temperatura si dichiarano alla prima lettura riuscita, con il meccanismo di rediscovery che esiste già per i dischi addormentati.
- Lo stesso vale per `read_health`, già limitato agli NVMe: nessun cambiamento.

### 5.3 Una sola temperatura per disco

Una sola temperatura principale `temperature/drive` per disco **associato**, con id stabile. Il codice già normalizza a `drive` sia la posizione 0 del nucleo (`storage_temperature.rs`) sia `Temperature`/`Composite Temperature` del servizio (`SchemaBuilder.StorageSensor`); non va presunto che ogni disco esponga questo sensore. I sensori aggiuntivi `sensor-N` restano distinti: non sono duplicati della temperatura principale.

Il provider `storage` diventa l'unico proprietario della temperatura principale dei dischi associati: legge la misura del servizio dalla vista condivisa (§3.3), altrimenti applica il §5.2, e conserva l'ultima misura valida per quella identità. `svc` omette la stessa temperatura dai dischi associati, mantenendo gli altri sensori SMART; per un disco non associabile conserva la pagina separata prevista da D3. Non si cambia l'ordine globale dei provider per far vincere una fonte.

L'arrivo della prima misura, locale o del servizio, può dichiarare il sensore tramite rediscovery senza una query di temperatura alla discovery. Alla disconnessione il sensore già dichiarato e la sua cache restano: una temperatura storica si mostra come tale, non si promette una misura attuale. Non si forza una lettura per colmare il passaggio. Una riconnessione non trasferisce dati a un'identità diversa; le preferenze locali filtrano la fonte anche se un altro client tiene SMART acceso.

### 5.4 Valore non aggiornato

Mentre la lettura è sospesa per standby o inattività, il sensore ripete l'ultimo valore con `Quality::Held`, senza scadenza: resta visibile, in grigio, anche per ore. L'interfaccia lo identifica come «Ultima lettura», non come temperatura attuale. Se non c'è mai stata una misura valida, il valore è assente. Un errore dopo una lettura autorizzata produce un dato assente, non una sospensione per inattività; la rimozione o il cambio d'identità elimina la cache.

**Qualità per sensore obbligatoria.** Oggi `Provider::repeated`, il worker e `Slot.held` nell'engine descrivono l'intero provider. Vanno estesi per trasportare qualità allineate ai valori: una temperatura `Held` non rende `Held` throughput, carico, spazio libero o gli altri dischi. La mancata risposta del worker continua a usare le regole di timeout dell'engine; non permette una conservazione infinita. La qualità arriva anche nel payload per il frontend: non si presume che la UI già riceva `TickOutput.quality`.

Il provider espone uno stato per disco: `standby` solo con evidenza corrente dal servizio o `GetDevicePowerState` falso, `idle` se manca attività recente e la temperatura locale è sospesa, `active` con attività/stato attivo confermati, `unknown` negli altri casi. La UI usa «Inattivo» per `idle`, senza affermare che il motore sia fermo. Il piano deve trasportare questo stato corrente a UI e regole, evitando che una proprietà di discovery ormai vecchia sopravviva a scollegamento, hot-plug o ripresa.

## 6. Interfaccia e regole

- **Pagina del disco** (vista Semplificata e Avanzata): con `standby` l'etichetta "In standby"; con `idle` l'etichetta "Inattivo". Le temperature `Held` si mostrano in grigio con «Ultima lettura», usando la qualità ricevuta dal backend (§5.4).
- **Vista Fonti:** il testo del blocco SMART nomina i dischi con `blocksSmart`, usando `model` e, se manca, "Disco N". Sostituisce l'uso di `smartBlockedBy`.
- **Impostazioni › Fonti:** l'interruttore SMART per disco esistente compare spento per i dischi USB, con la riga: «Alcuni adattatori USB non segnalano lo standby: accendere lo SMART può tenere sveglio il disco». Nuova impostazione `sources.smartEnabledDrives` (id core persistiti, tradotti in `driveKey` al `Subscribe`, come `smartDisabledDrives`).
- Il nucleo espone la classificazione USB anche senza servizio, per applicare il default nella UI. L'accensione/rimozione dell'abilitazione e lo spegnimento aggiornano atomicamente i due elenchi persistiti, mantenendoli disgiunti. Un altro client può tenere SMART acceso nel servizio, ma questo client continua a filtrarlo secondo la propria scelta; in assenza di sottoscrittori resta la configurazione effettiva precedente, come oggi.
- **Regole** (`crates/oma-core/src/rules/health.rs`): lo standby confermato è una sospensione attesa delle sole regole sui sensori che richiedono l'accesso al disco (temperatura e SMART), quindi non peggiora la copertura di quelle regole. Non dichiara coperto l'intero disco: spazio libero, I/O, errori di configurazione e altri sensori seguono le regole normali. «Inattivo» (`idle`, §5.4) sospende allo stesso modo la **sola regola sulla temperatura** di quel disco (D7): un disco che non lavora non si scalda per carico, e leggerlo vorrebbe dire svegliarlo. Non sospende le regole SMART né altro, e vale solo finché lo stato corrente è `idle`: stato `unknown`, scollegamento del servizio senza stato locale e feed scaduto non concedono l'eccezione. Un valore `Held` non avanza soglie né timer e non aggiorna `last_valid_ms`; per temperatura/SMART sospesi non basta `value.is_some()` a renderli disponibili. Un allarme già attivo resta visibile e non si chiude con dati conservati o assenti. Il banner può restare "tutto a posto" in standby solo se le altre regole sono coperte e non ci sono allarmi attivi.
- **i18n:** chiavi nuove in `en.json` e `it.json`, stesse chiavi.

## 7. Errori e casi limite

- **Bridge che rifiuta SAT o non restituisce i registri:** stato `unknown`, il disco blocca il gate come oggi. Nessun peggioramento.
- **Bridge che risponde sempre "attivo":** con D3 il disco USB non viene interrogato in modo periodico; può essere svegliato una volta, all'apertura del gate, dall'identificazione di LibreHardwareMonitor.
- **Disco senza modello o seriale:** ha una voce in `drives` con `key = nil`; si può nominare ("Disco N"), non si può accendere o spegnere singolarmente.
- **Numero di disco che cambia con l'hot-plug:** la tabella degli stati vale per la revisione e connessione correnti. Per associare stato e temperatura al disco core servono lo stesso numero fisico e chiavi presenti, uguali e univoche nelle due tabelle, coerentemente con `storage_binding`. Chiavi mancanti o duplicate non si associano per solo numero; nessuna cache o autorizzazione viene ereditata dal nuovo disco.
- **Servizio che si scollega o feed scaduto:** lo stato per disco perde subito autorità; la temperatura locale torna alla regola dell'attività. Si può mantenere una temperatura storica dello stesso disco con `Held`, ma non lo stato standby del servizio né la sua eccezione di copertura. La scadenza usa il criterio esistente del feed (oggi tre intervalli negoziati).
- **Disco molto attivo:** la lettura torna alla cadenza di oggi (ogni 30 s).

## 8. Limiti dichiarati

Da scrivere in `docs/follow-ups.md` e, quelli visibili all'utente, nel README ("Known limits"):

- il comportamento di un hard disk USB in standby dietro un bridge non è stato verificato: per questo lo SMART dei dischi USB è spento di default;
- la ri-identificazione all'hot-plug di DiskInfoToolkit può svegliare un disco che la libreria non riesce a identificare, se viene collegato dopo l'accensione dello SMART; va segnalata all'autore;
- senza servizio, la temperatura di un HDD inattivo non si aggiorna;
- nello spike, con TR-VISION HOME aperto e richieste `DISPLAY`/`SYSTEM` presenti, Windows non ha spento il disco; non è stata dimostrata una relazione causale. Documentare l'interferenza osservata, senza attribuire a ogni richiesta `SYSTEM` questo effetto.

## 9. Verifiche

### 9.1 Test automatici (TDD)

- **.NET:** lettura dei registri dai tre vettori del §2, più sense troncato, lunghezze incoerenti, descrittore assente, formato fisso senza 00/1D; fallback anche per esito nativo non interpretabile, senza fallback su standby valido; ricordo e invalidazione della via; `drives` anche per dischi assenti da LHM; precedenza di `state`; `blocksSmart` prima e dopo l'identificazione; aggregazione con `smartEnabledDrives` e più client; USB attivo `smartOff` che non blocca e USB disabilitato in standby che blocca; storage spento senza I/O; `bad_request` per chiave in entrambi gli elenchi; assert di layout di `SCSI_PASS_THROUGH`. Cache e flag `held` per standby, ripubblicazione tra giri, errori e ripresa; se necessario il filtro del §4.4, baseline, reset, attività recente e query che non si autoalimentano.
- **Rust:** tabella di decisione del §5.2 (pura), incluso standby senza temperatura, `noMedia`, veto del gate e fonte rifiutata localmente; finestra di attività di 10 s, warm-up, contatori invalidi e ripresa; lettura scaduta che resta scaduta e non blocca il refresh degli altri dischi; classe del disco; `Held` senza scadenza solo per sospensione prevista; qualità per sensore con I/O fresco e temperatura conservata nello stesso poll; decodifica di `drives` e di uno `state` sconosciuto; traduzione degli id persistiti in `smartEnabledDrives`.
- **Integrazione dei provider:** prima misura proveniente dal servizio senza query locale di discovery; temperatura principale unica e id stabile; sensori aggiuntivi conservati; collegamento, scollegamento e feed scaduto senza letture forzate; chiave assente/duplicata, riuso del numero fisico e hot-plug senza riutilizzo di stati o cache; cambi di stato senza proprietà di discovery obsolete.
- **Protocollo:** fixture `schema.msgpack`, `subscribe.msgpack`, `snapshot.msgpack` e `hello.msgpack` v3, confrontate byte per byte nei due linguaggi; validazione di lunghezza e contenuto di `held`; versioni 2/3 incompatibili; revisione, stato e valori pubblicati coerentemente.
- **Regole:** standby confermato che sospende solo temperatura/SMART; `idle` che sospende la sola regola sulla temperatura; stato sconosciuto e feed scaduto che non concedono copertura; I/O e spazio libero valutati normalmente; temperatura storica che non avanza timer né `last_valid_ms`; allarme attivo che non si chiude.
- **UI (Vitest):** etichette di stato, temperatura storica in grigio con «Ultima lettura» e dato mai misurato assente; testo della vista Fonti con e senza modello; interruttore USB spento di default; banner coerente con copertura e allarmi.

### 9.2 Verifiche dal vivo, con l'utente

Con `sat-probe.ps1` (terminale amministratore) e l'HDD del PC di sviluppo; niente input sintetico.

| # | Condizione | Atteso |
|---|---|---|
| V1 | app aperta, servizio collegato, HDD in standby forzato | resta in standby per 5 minuti |
| V2 | app aperta, modalità anti-cheat, HDD in standby forzato | resta in standby per 5 minuti |
| V3 | app aperta, nessuna attività su `D:`, TR-VISION chiuso, in entrambe le modalità | Windows spegne il disco (`GetDevicePowerState` falso) entro pochi minuti |
| V4 | chiavetta collegata all'avvio del servizio | lo SMART degli altri dischi resta acceso; la chiavetta è `smartOff` e non blocca |
| V5 | HDD in standby confermato, altre regole coperte, nessun allarme attivo | pagina del disco "In standby", banner "tutto a posto", ultimo valore in grigio con «Ultima lettura»; se mai misurato, valore assente |
| V6 | chiavetta: nel log del servizio, identificata o no da LibreHardwareMonitor | solo da annotare (riguarda il limite dell'hot-plug) |
| V7 | budget di `docs/perf-budget.md` | rispettato, misurato con `scripts/measure-footprint.ps1` |
| V8 | app senza servizio, HDD con breve attività reale e poi inattivo oltre la finestra di 10 s | la temperatura locale può aggiornarsi durante l'attività, poi resta storica; le query non generano nuova attività PDH e Windows spegne il disco |
| V9 | HDD inattivo senza standby confermato, poi scollegamento/ricollegamento del servizio | «Inattivo» sospende solo la regola sulla temperatura (banner "tutto a posto" se il resto è coperto); una sola temperatura principale, senza query forzate e senza chiusura degli allarmi attivi |

V3 va eseguita con un timeout disco esplicito (come i 60 s dello spike), un controllo a app e servizio chiusi nelle stesse condizioni e le richieste di alimentazione annotate. Prima della scelta del ramo del §4.4 si prova anche il servizio da solo con un sottoscrittore, senza provider storage del nucleo. Non si interroga la temperatura per controllare lo standby. Un fallimento va isolato tra query del nucleo, `CHECK POWER MODE`, SMART e interferenze esterne; dopo la correzione si ripetono V1–V3 e V8. Il superamento di V3 sull'app completa è necessario per chiudere M6b.

## 10. Punti che il piano deve fissare

- i punti di integrazione del feed immutabile con `storage`, la rediscovery senza query locale e il trasporto dello stato corrente a UI e regole, rispettando il proprietario unico del §5.3;
- il contratto di qualità per sensore attraverso provider, worker, engine e frontend, e `held` nello snapshot v3; non usare `Provider::repeated` per sospendere solo una temperatura;
- il probe passivo, la cadenza di campionamento e i test del ramo condizionale del §4.4, dopo la prova isolata: non presumere contatori del servizio già disponibili;
- l'ordine dei task: prova isolata del servizio per fissare il ramo condizionale; filtro locale anti-risveglio e selezione dei dischi autorizzati; protocollo v3 e cache/stati del servizio; integrazione della temperatura e qualità per sensore; UI e regole; verifiche dal vivo obbligatorie. Il filtro locale si può sviluppare separatamente, ma il §5 completo dipende dal feed v3;
- la rimozione del programma di prova `crates/oma-win/examples/m6b_wake.rs` prima della chiusura del branch.
