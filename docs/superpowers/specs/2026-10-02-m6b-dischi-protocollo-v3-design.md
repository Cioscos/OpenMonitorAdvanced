# M6b — Dischi e protocollo v3: design di dettaglio

- **Data:** 2026-10-02
- **Stato:** design del brainstorming del 2026-10-02, basato sullo spike dal vivo dello stesso giorno (§2). Le verifiche del §9.2 sono parte del piano, non fatti già acquisiti.
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
| D2 | Temperatura degli HDD, **due percorsi**: con il servizio collegato il valore arriva dal servizio; senza, il nucleo legge solo dopo attività recente del disco (§5). |
| D3 | Dischi USB, scelta **prudente**: la risposta SAT serve a sbloccare gli altri dischi; lo SMART del disco USB resta spento finché l'utente non lo accende (§4.2). |
| D4 | Ri-identificazione all'hot-plug: limite dichiarato e segnalazione a monte. |
| D5 | Protocollo v3 con **stato per disco** (`drives`), non il solo flag `smartGateClosed` previsto nei follow-up (§3). |
| D6 | La release che contiene la M6b è la 0.4.0 e porta anche la correzione dell'installer italiano (commit `57fa804`). |

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

- Una voce per ogni disco fisico che `DiskPowerProbe` enumera, in ordine di `physicalDrive`. L'elenco esiste anche mentre il gate D6 è chiuso e anche con il modulo storage spento (allora ogni voce ha `state = "smartOff"`).
- `key` è la `driveKey` esistente (`sha256(trim(model) \0 trim(serial))`), `nil` se il descrittore non ha modello o seriale. `model` è il modello del descrittore, `nil` se assente.
- `state`, con questa precedenza:

  | Valore | Significato |
  |---|---|
  | `noMedia` | lettore senza supporto |
  | `smartOff` | il servizio non interroga questo disco: storage spento, disco in `smartDisabledDrives`, oppure disco spento di default e non abilitato (§4.2) |
  | `standby` | `CHECK POWER MODE` risponde standby |
  | `active` | risponde attivo, oppure il disco non richiede il controllo (`RequiresPowerCheck` falso: NVMe, SSD, virtuali) |
  | `unknown` | richiede il controllo e nessuna via risponde |

- `blocksSmart` è `true` per i dischi che tengono chiuso il gate D6. "Gate chiuso" equivale a "almeno una voce con `blocksSmart`": il flag `smartGateClosed` dei follow-up non serve.
- Ogni cambio di `drives` è un cambio di struttura: incrementa la revisione e arriva con lo snapshot coerente.
- Un valore di `state` sconosciuto al client si tratta come `unknown`.

### 3.2 `Subscribe`

Si aggiunge `smartEnabledDrives: [string]`, le `driveKey` dei dischi spenti di default che il client vuole accesi. Validazione rigida, come per `smartDisabledDrives`: al massimo 64 chiavi, ognuna esadecimale minuscola di 64 caratteri; una chiave presente in entrambi gli elenchi è `bad_request`.

### 3.3 Lato app

- `ServiceSources` (`crates/oma-ipc/src/status.rs`) espone `drives` e perde `smart_blocked_by`; la vista Fonti ricava i nomi dei dischi bloccanti da `drives`.
- Il provider `svc` pubblica lo stato per disco in una tabella condivisa con il provider `storage` (stessa tecnica di `DriveIdTable`), indicizzata per `physicalDrive` e confermata con `key` quando c'è.

## 4. Servizio

### 4.1 Fallback SAT in `DiskPowerProbe`

- `IsSpunDown` prova prima `IOCTL_ATA_PASS_THROUGH`, come oggi. Se la chiamata fallisce, prova `IOCTL_SCSI_PASS_THROUGH` con `ATA PASS-THROUGH(16)`: CDB `85 06 20 00 00 00 00 00 00 00 00 00 00 00 E5 00` (protocollo non-data, `CK_COND = 1`), `DataIn = SCSI_IOCTL_DATA_UNSPECIFIED`, 32 byte di sense, timeout 5 s.
- I registri si leggono dal sense data, **senza richiedere lo stato SCSI CHECK CONDITION** (l'HDD SATA risponde con stato 0x00):
  - formato a descrittori (codice 0x72 o 0x73): descrittore 0x09 di lunghezza almeno 0x0C; error al byte 3, sector count al byte 5, status al byte 13 del descrittore;
  - formato fisso (0x70 o 0x71) con ASC/ASCQ 00/1D: error al byte 3, status al byte 4, sector count al byte 6.
  - Poi `InterpretAtaResult`, invariato. Nessun registro: `null`.
- Per ogni disco si ricorda la via che ha risposto (nativa, SAT, nessuna), per non ripetere a ogni giro un comando che il driver rifiuta. Il ricordo si azzera quando modello o seriale del `PhysicalDriveN` cambiano. La via "nessuna" si riprova al più ogni 5 minuti.
- Struct di `SCSI_PASS_THROUGH` con assert di dimensione (56 byte su x64) nei test, come per `ATA_PASS_THROUGH_EX`.

### 4.2 Dischi spenti di default

- Un disco con bus USB (`BusType` 0x07) è **spento di default**: non riceve SMART né controlli di stato periodici, ed esce dallo schema come un disco in `smartDisabledDrives`.
- Diventa acceso se almeno un sottoscrittore con lo storage attivo lo elenca in `smartEnabledDrives`. Un disco senza `key` non si può accendere.
- **Gate D6.** Un disco spento di default partecipa comunque al gate, perché la prima identificazione di LibreHardwareMonitor tocca tutti i dischi (F1.1): viene interrogato quando il gate è chiuso, e blocca finché non risponde "attivo". Con il fallback SAT la chiavetta dello spike risponde e non blocca più.
- `EffectiveConfig.Compute` (pura) estende l'aggregazione: lo SMART di un disco spento di default è acceso se almeno una richiesta con lo storage attivo lo abilita; per gli altri dischi resta la regola di oggi.

### 4.3 Elenco `drives`

Lo compila lo storage worker a ogni giro (30 s), dai `DriveFacts` e dalle risposte che ha già: nessun comando in più verso i dischi. Con il gate chiuso usa l'esito di `FindGateBlockers`.

### 4.4 Lettura SMART e timer di inattività di Windows (condizionale)

Lo spike non ha misurato se la lettura SMART del servizio, ogni 30 s su un HDD attivo, impedisce a Windows di spegnerlo. Lo misura la verifica V3 del §9.2. **Se V3 fallisce con il servizio collegato**, il servizio applica agli HDD la stessa regola dell'attività del §5.2: `Update` di un disco rotazionale solo se il disco ha avuto letture o scritture dall'ultimo `Update` (contatori di `IOCTL_DISK_PERFORMANCE`, già letti); altrimenti restano i valori precedenti. Se V3 passa, questa regola non si implementa.

## 5. Nucleo dell'app (`crates/oma-win`)

### 5.1 Classe del disco

Alla discovery il provider `storage` stabilisce per ogni disco, con handle ad accesso 0:

- **non rotazionale:** bus NVMe, oppure `StorageDeviceSeekPenaltyProperty` risponde `false`;
- **rotazionale o ignoto:** tutti gli altri (seek penalty `true`, oppure query fallita, come sulla chiavetta).

### 5.2 Quando leggere la temperatura

La decisione è una funzione pura, usata sia alla discovery sia nel refresh ogni 30 s:

| Disco | Stato dal servizio | Azione |
|---|---|---|
| non rotazionale | qualsiasi | legge, come oggi |
| rotazionale o ignoto | `active` o `standby` | non legge |
| rotazionale o ignoto | `smartOff`, `unknown`, `noMedia` o servizio non collegato | legge solo con attività recente |

- Resta la protezione di oggi: con `GetDevicePowerState` falso non si legge mai.
- **Attività recente:** in almeno un poll degli ultimi 10 s il contatore PDH di lettura o di scrittura del disco è maggiore di zero. I contatori PDH non contano le nostre query, quindi la lettura non si alimenta da sola.
- Una lettura scaduta e non eseguita **resta scaduta**: si rivaluta a ogni poll senza I/O, e parte al primo poll con attività. La regola "un disco per poll" resta.
- Alla discovery un disco rotazionale senza attività nota non viene letto: i suoi sensori di temperatura si dichiarano alla prima lettura riuscita, con il meccanismo di rediscovery che esiste già per i dischi addormentati.
- Lo stesso vale per `read_health`, già limitato agli NVMe: nessun cambiamento.

### 5.3 Una sola temperatura per disco

L'interfaccia continua a mostrare una temperatura per disco, con lo stesso id di sensore. Quando il nucleo non legge (seconda riga della tabella) il valore viene dal servizio, se il servizio espone la temperatura SMART di quel disco. Il piano fissa il meccanismo dopo aver letto il merge per fonte dell'engine e `svc/provider.rs` (oggi a parità di id vince il primo provider); il vincolo è: nessun sensore doppio, nessun buco quando il servizio si scollega (si torna alla terza riga). Se il servizio dichiara il disco `active` o `standby` ma non ne espone la temperatura, il nucleo applica la terza riga.

### 5.4 Valore non aggiornato

Mentre la lettura è sospesa, il sensore ripete l'ultimo valore con `Quality::Held`, senza scadenza: resta visibile, in grigio, finché il disco non lavora di nuovo. Se non c'è mai stata una lettura, il valore è assente.

Il provider espone per ogni disco una proprietà di stato per l'interfaccia: `active`, `standby` (dal servizio, oppure `GetDevicePowerState` falso), `idle` (lettura sospesa per inattività, senza stato dal servizio).

## 6. Interfaccia e regole

- **Pagina del disco** (vista Semplificata e Avanzata): con `standby` l'etichetta "In standby"; con `idle` l'etichetta "Inattivo". I valori `Held` restano in grigio, come già fa l'interfaccia per gli altri valori ripetuti.
- **Vista Fonti:** il testo del blocco SMART nomina i dischi con `blocksSmart`, usando `model` e, se manca, "Disco N". Sostituisce l'uso di `smartBlockedBy`.
- **Impostazioni › Fonti:** l'interruttore SMART per disco esistente compare spento per i dischi USB, con la riga: «Alcuni adattatori USB non segnalano lo standby: accendere lo SMART può tenere sveglio il disco». Nuova impostazione `sources.smartEnabledDrives` (id core persistiti, tradotti in `driveKey` al `Subscribe`, come `smartDisabledDrives`).
- **Regole** (`crates/oma-core/src/rules/health.rs`): un disco in stato `standby` o `idle` conta come **coperto** per le regole sui suoi sensori; il banner resta "tutto a posto". Un valore `Held` di un disco in quello stato non si valuta. Un allarme già attivo resta com'è (regola esistente dei dati non disponibili).
- **i18n:** chiavi nuove in `en.json` e `it.json`, stesse chiavi.

## 7. Errori e casi limite

- **Bridge che rifiuta SAT o non restituisce i registri:** stato `unknown`, il disco blocca il gate come oggi. Nessun peggioramento.
- **Bridge che risponde sempre "attivo":** con D3 il disco USB non viene interrogato in modo periodico; può essere svegliato una volta, all'apertura del gate, dall'identificazione di LibreHardwareMonitor.
- **Disco senza modello o seriale:** ha una voce in `drives` con `key = nil`; si può nominare ("Disco N"), non si può accendere o spegnere singolarmente.
- **Numero di disco che cambia con l'hot-plug:** la tabella degli stati vale per la revisione corrente; un cambio di dischi cambia `drives` e quindi la revisione. Il nucleo usa lo stato solo se `key` coincide, quando entrambe esistono.
- **Servizio che si scollega:** lo stato per disco si scarta subito; vale la regola dell'attività.
- **Disco molto attivo:** la lettura torna alla cadenza di oggi (ogni 30 s).

## 8. Limiti dichiarati

Da scrivere in `docs/follow-ups.md` e, quelli visibili all'utente, nel README ("Known limits"):

- il comportamento di un hard disk USB in standby dietro un bridge non è stato verificato: per questo lo SMART dei dischi USB è spento di default;
- la ri-identificazione all'hot-plug di DiskInfoToolkit può svegliare un disco che la libreria non riesce a identificare, se viene collegato dopo l'accensione dello SMART; va segnalata all'autore;
- senza servizio, la temperatura di un HDD inattivo non si aggiorna;
- un programma che tiene una richiesta di alimentazione `SYSTEM` può impedire a Windows di spegnere i dischi (osservato con TR-VISION HOME).

## 9. Verifiche

### 9.1 Test automatici (TDD)

- **.NET:** lettura dei registri dai tre vettori del §2, più sense troncato, descrittore assente, formato fisso senza 00/1D; ordine nativo poi SAT e ricordo della via; `drives` e precedenza di `state`; `blocksSmart`; aggregazione con `smartEnabledDrives`; un disco USB attivo non blocca il gate ed è `smartOff`; `bad_request` per chiave in entrambi gli elenchi; assert di layout di `SCSI_PASS_THROUGH`.
- **Rust:** tabella di decisione del §5.2 (pura); finestra di attività di 10 s; lettura scaduta che resta scaduta; classe del disco; `Held` senza scadenza; decodifica di `drives` e di uno `state` sconosciuto; traduzione degli id persistiti in `smartEnabledDrives`.
- **Protocollo:** fixture `schema.msgpack` e `subscribe.msgpack` v3, confrontate byte per byte nei due linguaggi; `hello` con versione 3.
- **Regole:** copertura con un disco in `standby` e in `idle`; allarme attivo che non si chiude.
- **UI (Vitest):** etichette di stato, testo della vista Fonti con e senza modello, interruttore USB spento di default.

### 9.2 Verifiche dal vivo, con l'utente

Con `sat-probe.ps1` (terminale amministratore) e l'HDD del PC di sviluppo; niente input sintetico.

| # | Condizione | Atteso |
|---|---|---|
| V1 | app aperta, servizio collegato, HDD in standby forzato | resta in standby per 5 minuti |
| V2 | app aperta, modalità anti-cheat, HDD in standby forzato | resta in standby per 5 minuti |
| V3 | app aperta, nessuna attività su `D:`, TR-VISION chiuso, in entrambe le modalità | Windows spegne il disco (`GetDevicePowerState` falso) entro pochi minuti |
| V4 | chiavetta collegata all'avvio del servizio | lo SMART degli altri dischi resta acceso; la chiavetta è `smartOff` e non blocca |
| V5 | HDD in standby | pagina del disco "In standby", banner "tutto a posto", ultimo valore in grigio |
| V6 | chiavetta: nel log del servizio, identificata o no da LibreHardwareMonitor | solo da annotare (riguarda il limite dell'hot-plug) |
| V7 | budget di `docs/perf-budget.md` | rispettato, misurato con `scripts/measure-footprint.ps1` |

V3 con il servizio collegato decide il §4.4.

## 10. Punti che il piano deve fissare

- il meccanismo del §5.3 (una sola temperatura per disco), dopo la lettura del merge dell'engine;
- se la temperatura SMART del servizio per gli HDD ha già lo stesso id del sensore del nucleo;
- l'ordine dei task: prima la correzione del nucleo (§5), che è indipendente dal protocollo, poi protocollo, servizio, interfaccia;
- la rimozione del programma di prova `crates/oma-win/examples/m6b_wake.rs` prima della chiusura del branch.
