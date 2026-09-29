# M5 — Regole e integrazione: design di dettaglio

- **Data:** 2026-09-29
- **Stato:** approvato in brainstorming, in attesa di revisione della spec scritta
- **Spec principale:** `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`. Per i punti trattati qui, questa spec di dettaglio ha la precedenza; tutto il resto (palette, budget, sicurezza, protocollo, convenzioni) resta come nella spec principale.

## 1. Intento e confini

La M5 trasforma OpenMonitor Advanced da monitor che mostra i dati a monitor che li giudica e li conserva:

- un **motore regole** unico, in `oma-core`, alimenta il banner di stato, le notifiche e il colore della tray, così i tre non si contraddicono mai, anche a finestra chiusa (spec principale §4.3);
- le **impostazioni** diventano un file versionato in `%APPDATA%\OpenMonitorAdvanced\settings.json` con una schermata dedicata (§4.6, §7.4);
- la **tray** diventa completa: icona dinamica, tooltip, menu completo, chiusura nella tray, avvio con Windows (§4.5);
- il **log CSV** si avvia e si ferma dalla UI (con un registratore in stile nastro), dalla tray e da una scorciatoia globale (§4.4);
- tutte le nuove stringhe esistono in inglese e in italiano con le stesse chiavi (§7.6).

**Fuori dalla M5:**

- il controllo degli aggiornamenti, rimandato alla M6 insieme alla prima release (decisione del brainstorming: prima della 1.0 non ci sono release da controllare);
- "Esporta report sensori" (M6, come da spec principale);
- un provider batteria: la regola predefinita esiste ma resta senza istanze;
- l'energia NVML e il throughput PCIe (`docs/follow-ups.md`): non servono alle regole predefinite e restano fuori dal tick.

### 1.1 Decisioni del brainstorming

| # | Decisione |
|---|---|
| B1 | Una spec di dettaglio unica e **tre piani in sequenza**, ognuno con il proprio branch e la propria revisione finale: M5a impostazioni e tray, M5b regole, M5c log CSV. |
| B2 | **Toast solo per il livello critico** di default; il livello attenzione si vede nel banner e nel colore della tray. Un toast all'ingresso in critico, poi silenzio per quell'istanza finché non rientra e sono passati almeno 5 minuti. |
| B3 | Controllo degli aggiornamenti rimandato alla M6. |
| B4 | Le impostazioni **le possiede Rust**: tipi in `oma-core`, I/O nella shell, la UI legge e scrive tramite comandi. |
| B5 | Il motore regole gira **dentro `Engine::tick`**, sul thread del campionatore. |
| B6 | Il CSV si scrive su un **thread dedicato** alimentato da un canale limitato. |
| B7 | La CPU ha **due regole predefinite** (temperatura e throttling termico) invece di una regola con due condizioni. |
| B8 | TjMax dei Ryzen da una **tabella per famiglia** nel servizio, esposta come proprietà del device CPU. |
| B9 | Lo spazio libero si esprime come **percentuale usata** (≥ 90% / ≥ 97%), perché il sensore esistente è quello. |
| B10 | Il log si comanda anche da un **registratore in stile nastro** nella barra superiore e da una **scorciatoia globale** (richiesta dell'utente). |

## 2. Impostazioni (M5a)

### 2.1 Formato

File `%APPDATA%\OpenMonitorAdvanced\settings.json`, chiavi camelCase. I tipi stanno in `oma-core::settings` (portabili, senza codice Windows); lettura e scrittura del file stanno nella shell (`app/src-tauri`).

```jsonc
{
  "version": 1,
  "general": {
    "language": "system",        // "system" | "en" | "it"
    "temperatureUnit": "c",      // "c" | "f"
    "throughputUnit": "bits",    // "bits" | "bytes"
    "intervalMs": 1000,          // 500–5000, a passi di 500
    "chartFps": 60,              // 60 | 30 | 15
    "defaultView": "last"        // "simple" | "advanced" | "last"
  },
  "tray": {
    "closeToTray": true,
    "autostart": false,
    "iconSensor": null           // null = automatico; altrimenti un id sensore
  },
  "sources": {
    "vendorLibraries": { "nvml": true, "nvapi": true, "adl": true, "igcl": true },
    "antiCheat": false,
    "serviceModules": {
      "cpu": true, "motherboard": true, "memory": true,
      "storage": true, "controller": true, "psu": true
    },
    "smartDisabledDrives": []    // id dei device disco
  },
  "advanced": {
    "section": null,
    "window": 300,               // 60 | 300 | 1800 | 3600
    "series": {}                 // id sezione -> id sensori
  },
  "view": { "last": "simple" },  // ultima vista scelta, per defaultView = "last"
  "rules": { "overrides": {}, "custom": [] },  // M5b, §3.5
  "log": { }                     // M5c, §4.5
}
```

- `iconSensor = null` significa: temperatura core della GPU dedicata principale; se non c'è, temperatura della CPU; se manca anche quella (servizio assente), carico della CPU.
- `chartFps`: accanto a 60 FPS la UI dice che la frequenza più alta aumenta leggermente l'uso della CPU; 30 e 15 sono più leggeri (memoria del progetto "chart FPS"). Il valore alimenta l'orologio dei frame condiviso dei grafici, già predisposto per 30 e 15.
- Le sezioni `rules` e `log` fanno parte del formato fin dalla M5a con i loro default: M5b e M5c le riempiono senza cambiare `version`, perché ogni chiave nuova ha un default.

### 2.2 Robustezza

- **Chiavi mancanti** prendono il default; **chiavi sconosciute** si ignorano e si perdono alla scrittura successiva (il file non è pensato per essere modificato a mano).
- **Valori fuori intervallo** si correggono al valore valido più vicino (per esempio `intervalMs: 700` diventa 500, `800` diventa 1000, `750` diventa 1000 perché a pari distanza vince il maggiore, `9000` diventa 5000) e la correzione finisce nel log di diagnostica.
- **File illeggibile o JSON non valido:** il file si rinomina `settings.json.bad-<AAAAMMGG-hhmmss>` e l'app parte con i default. La UI mostra una riga di avviso nelle Impostazioni, con il nome del file conservato.
- **`version` più recente di quella nota** (downgrade dell'app): il file non viene mai sovrascritto. L'app usa ciò che capisce, tiene le modifiche solo in memoria e la UI mostra l'avviso "impostazioni di una versione più recente: le modifiche non verranno salvate".
- **Scrittura atomica:** si scrive `settings.json.tmp` e lo si sostituisce con `ReplaceFileW` (oppure `MoveFileExW` con `MOVEFILE_REPLACE_EXISTING` se il file non esiste ancora). Un solo writer, protetto da un mutex, serializza tutte le scritture.
- **Scritture ravvicinate** (per esempio uno slider) si coalescono: la patch si applica subito in memoria, il file si scrive al più una volta ogni 500 ms e sempre alla chiusura dell'app.

### 2.3 Comandi ed eventi

- `get_settings() -> Settings`: le impostazioni effettive, con lo stato del file (`ok` | `recovered` con il nome del file conservato | `readOnly`).
- `update_settings(patch) -> Settings`: la patch è un oggetto parziale con la stessa forma del file. Rust la applica, valida, corregge, salva e restituisce il risultato; un errore di validazione (per esempio in una regola, §3.6) restituisce il campo e la chiave i18n del messaggio, senza applicare nulla.
- Evento `oma:settings`: parte dopo ogni modifica applicata, anche se l'origine è la tray (per esempio l'anti-cheat), così una finestra aperta resta allineata.

### 2.4 Migrazioni al primo avvio della M5

- **Anti-cheat:** se `%LOCALAPPDATA%\OpenMonitorAdvanced\service.json` esiste, il suo valore va in `sources.antiCheat`, poi il file si cancella. Se la scrittura di `settings.json` fallisce, `service.json` resta dov'è e la migrazione si ritenta al prossimo avvio.
- **Vista Avanzata e ultima vista:** la UI, al primo avvio in cui `advanced` e `view` sono ai valori di default e in `localStorage` esistono le chiavi `oma.advanced.*` o `oma.view` (`App.svelte`), le invia con `update_settings` una sola volta, poi le cancella. Da quel momento `persist.ts` e `App.svelte` leggono e scrivono tramite le impostazioni, non più tramite `localStorage`.

### 2.5 Applicazione a caldo

Nessuna impostazione richiede un riavvio.

- **Lingua:** la UI cambia catalogo. La tray rigenera le etichette del menu con la lingua scelta; con `system` continua a seguire `sys_locale` come nella M4.
- **Unità:** solo formattazione nella UI (`format.ts`), nel tooltip della tray, nei messaggi delle regole e nel CSV. Si chiude così il follow-up del grafico di rete, oggi in byte/s su asse e legenda anche quando KPI e tabella sono in bit/s.
- **Intervallo:** il campionatore adotta il nuovo intervallo al tick successivo e lo comunica al servizio con un nuovo `Subscribe`. Lo storico si conserva: la capacità si ricalcola per coprire 1 ora al nuovo intervallo e, se diminuisce, si tengono i campioni più recenti. Le statistiche restano. La decimazione per 30 minuti e 1 ora lavora sui timestamp, quindi uno storico con due intervalli diversi resta corretto.
- **FPS dei grafici:** l'orologio dei frame condiviso cambia cadenza al frame successivo.
- **Librerie dei vendor:** disattivarne una la esclude dal tick successivo, e i suoi campi ricadono sui livelli base (D3DKMT, PDH). La DLL, se era già caricata, non si scarica mai (regola del progetto); all'avvio seguente non viene caricata. È l'interruttore anti-crash di §8 della spec principale. Riattivarla la carica subito, come "Riattiva" della modalità sicura. La modalità sicura resta una condizione di sessione e ha la precedenza sugli interruttori.
- **Moduli del servizio e SMART per disco:** si inviano al servizio con un nuovo `Subscribe` (§2.8).

### 2.6 Tray

- **Menu:**
  1. Apri
  2. Vista Semplificata
  3. Vista Avanzata
  4. separatore
  5. Log: Avvia / Pausa / Riprendi / Ferma (voci aggiunte dalla M5c, §4.4)
  6. Modalità compatibile anti-cheat (casella)
  7. separatore
  8. Esci

  "Vista Semplificata" e "Vista Avanzata" aprono la finestra (creandola se serve) sulla vista scelta.
- **Icona dinamica:**
  - un'icona RGBA 32×32 disegnata in Rust con un font bitmap delle cifre, del segno meno e del trattino, scritto a mano nel sorgente: niente dipendenze di font né rasterizzatori;
  - mostra il valore arrotondato del sensore di `iconSensor`, nelle unità di visualizzazione e senza il simbolo dell'unità (due cifre, tre se servono, per esempio `100` o `-5`); "—" se il valore è assente;
  - lo sfondo è un quadrato arrotondato: nella M5a sempre neutro (`--surface-2`), dalla M5b il colore del livello (§3.4), dalla M5c con un pallino rosso nell'angolo durante la registrazione;
  - si ridisegna e si invia a Windows solo quando cambiano numero, colore o pallino. Il rendering è una funzione pura, testata sui pixel.
- **Tooltip:** `CPU 45 °C · GPU 62 °C · RAM 48 %`, con le unità scelte; le voci senza valore si omettono. Si tronca entro i 127 caratteri di `NOTIFYICONDATA` e si aggiorna solo quando il testo cambia. Dalla M5b, se il livello non è `ok`, il verdetto precede i valori.
- **Chiudi nella tray** (`closeToTray`): se è attivo, chiudere la finestra la distrugge e l'app resta nella tray (comportamento attuale); se è disattivato, chiudere la finestra chiude l'app.
- **Avvio con Windows** (`autostart`):
  - `oma-win` scrive il valore `OpenMonitor Advanced` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, con il percorso dell'eseguibile tra virgolette seguito da `--minimized`, senza plugin;
  - l'interruttore mostra lo stato reale del registro, riletto all'apertura delle Impostazioni, perché l'utente può disattivare l'avvio anche da Gestione attività. `StartupApproved\Run` è la chiave con cui Gestione attività disattiva la voce: se lì risulta disattivata, l'interruttore è spento. Riattivarlo da noi ripristina anche quella voce;
  - il campo `tray.autostart` segue lo stato letto;
  - il disinstallatore rimuove il valore `Run` dell'utente che disinstalla (hook NSIS in `oma.nsh`). I valori di altri utenti restano: puntano a un eseguibile che non c'è più, e Windows li ignora. È un limite accettato.
- **Seconda istanza:** il callback di single-instance legge gli argomenti; apre la finestra solo se la seconda istanza è stata lanciata senza `--minimized`. `scripts/measure-footprint.ps1 -FillHistoryMinutes`, che si basa su una seconda istanza senza argomenti, continua a funzionare.

### 2.7 Schermata Impostazioni

- È una terza vista nella finestra principale, accanto a Semplificata e Avanzata: niente seconda WebView, per il budget di memoria. Si apre dall'icona a ingranaggio della barra superiore.
- Sezioni a sinistra: **Generale**, **Regole e avvisi** (M5b), **Log CSV** (M5c), **Fonti dati**, **Informazioni**. Nella M5a le sezioni della M5b e della M5c non compaiono ancora.
- Ogni modifica si applica subito tramite `update_settings`, senza un pulsante Salva. Esc o "Indietro" riportano alla vista precedente. Gli errori di validazione compaiono accanto al campo.
- **Generale:** lingua, unità di temperatura e di throughput, intervallo di aggiornamento, FPS dei grafici con l'avviso, vista predefinita, chiudi nella tray, avvio con Windows, sensore dell'icona della tray (elenco dei sensori di temperatura e carico, più "Automatico").
- **Fonti dati:**
  - interruttori per NVML, NVAPI, ADL e IGCL, con la nota che la disattivazione ha pieno effetto al prossimo avvio se la libreria era già caricata;
  - stato del servizio, con la stessa spiegazione e la stessa azione del badge della modalità base (§7.1 della spec principale);
  - interruttore della modalità anti-cheat, raggiungibile così anche con il servizio collegato;
  - stato di PawnIO (§2.8);
  - moduli del servizio (CPU, scheda madre, memoria, dischi, controller, alimentatore) e, sotto "dischi", un interruttore SMART per ogni disco noto al servizio. Senza servizio questi controlli sono disattivati, con la spiegazione.
- **Informazioni:** versione dell'app e del servizio (dal `Hello`), versione del protocollo, licenza GPL-3.0, link a `THIRD_PARTY_NOTICES.md` incluso nel pacchetto, cartelle di impostazioni e log con "Apri cartella".

### 2.8 Protocollo v2

`PROTOCOL_VERSION` passa da 1 a 2. App e servizio si installano insieme; una coppia disallineata cade nel caso `Incompatible` già gestito.

- **`Subscribe`** aggiunge:
  - `disabledModules: [string]`, con i valori `cpu`, `motherboard`, `memory`, `storage`, `controller`, `psu`;
  - `smartDisabledDrives: [string]`, gli id dei device disco lato servizio.

  Le chiavi sono sempre presenti (liste vuote se non c'è nulla), come vuole la regola del protocollo.
- **`Hello`** aggiunge `pawnIo: string`: `ok`, `missing` (driver non installato o non caricabile) oppure `rebootPending` (installato ma serve un riavvio). La UI lo mostra in Fonti dati e, se non è `ok`, con una riga nel popup del badge del servizio, che oggi scompare lasciando mancare i sensori di CPU, scheda madre e DIMM senza spiegazione (`docs/follow-ups.md`).
- **Più client:** il servizio è condiviso tra gli utenti della macchina. Un modulo si spegne solo se **tutti** i client collegati lo vogliono spento, e lo stesso vale per lo SMART di un disco. Quando un client si scollega, la configurazione effettiva si ricalcola. Accendere o spegnere un modulo usa i setter di `Computer` di LibreHardwareMonitor (che aggiungono o rimuovono il gruppo su un computer aperto) e rigenera lo schema. Per i dischi resta valido il filtro D6: lo SMART di un disco con lo SMART spento non si legge e quel disco non partecipa alla conferma dello stato di alimentazione. Questo chiude il follow-up del disco USB il cui bridge rifiuta il pass-through ATA.
- **Fixture:** si rigenerano `hello.msgpack` e `subscribe.msgpack` con `OMA_WRITE_FIXTURES=1`, a thread singolo; i test Rust e .NET le confrontano byte per byte.

### 2.9 Correzioni della shell incluse nella M5a

Sono follow-up di `docs/follow-ups.md` che toccano gli stessi file.

- **Flush del log di diagnostica:** `App::run` diventa `run_return` (o il flush avviene in `RunEvent::Exit`), così le ultime righe non si perdono.
- **Link al servizio senza risveglio a 20 Hz:** comandi ed eventi della pipe confluiscono in un solo canale, e il thread si blocca fino alla scadenza invece di leggere la pipe a fette da 50 ms.
- **`Incompatible` e `PidMismatch`:** nessuna riconnessione automatica ogni 5 s, perché ogni connessione azzerava il timer di inattività del servizio. Si riprova solo con "Avvia" o con un cambio di stato del servizio visto dall'SCM.

## 3. Regole (M5b)

### 3.1 Spike iniziale S1: sensori di LibreHardwareMonitor

Prima di scrivere le mappature, uno spike su questa macchina (Ryzen con iGPU AMD, RTX 4080, NVMe e SATA) fa il dump di nomi, tipi e valori dei `SensorNode` di CPU e dischi, con lo strumento `LhmDump` dello spike M4 o con una fixture di `SchemaBuilderTests`. Decide:

- se esiste un sensore di throttling termico della CPU (Intel: "Thermal throttling" o simili; AMD: probabilmente assente) e come mapparlo come `flag`;
- quale sensore NVMe o SMART porta il critical warning, oggi scartato da `MatchStorageSensor` insieme alle soglie ("Warning…", "Critical…"), da mappare come `flag` (`…/flag/critical-warning`);
- se LibreHardwareMonitor espone TjMax per Intel (per esempio da "Distance to TjMax" più la temperatura del core);
- la **tabella TjMax per famiglia AMD** (B8), per nome o famiglia/modello del processore. Valori iniziali da verificare nelle fonti AMD: Zen 4 e Zen 5 desktop 95 °C, varianti X3D 89 °C, Zen 3 desktop 90 °C, mobile secondo la famiglia. Il valore diventa la proprietà `tjMaxC` del device CPU nel servizio.

L'esito si registra in `docs/superpowers/references/m5/` come per gli spike della M4. Una regola il cui sensore non esiste non ha istanze e non compare nel banner.

### 3.2 Modello

Tipi in `oma-core::rules`.

- **`Rule`:**
  - `id`: stringa stabile; `gpu-temp`, `cpu-temp` e così via per le predefinite, `custom-<uuid>` per le personalizzate;
  - `target`: `{ "sensor": "<id>" }` oppure un selettore `{ "deviceKind": "gpu", "sensorKind": "temperature", "names": ["core"] }`. `names` è un elenco di segmenti `<name>` dell'id del sensore; vuoto significa tutti;
  - `condition`: `above` | `below` | `flagActive`;
  - `warn` e `crit`, facoltativi, ognuno `{ threshold, durationS }`. `threshold` è `{ "fixed": 83 }` oppure `{ "property": "tempWarningC", "offset": 0, "fallback": 70 }`: si legge la proprietà del device del sensore, le si somma `offset`, e se la proprietà manca o non è un numero si usa `fallback`. Con `flagActive` la soglia non c'è;
  - `hysteresis`: `{ amount, durationS }`, di default `{ 3, 10 }`, nelle unità di base del sensore;
  - `enabled`;
  - `notify`: `{ warn, crit }`, di default `{ false, true }` (B2).
- **Unità:** soglie e isteresi si salvano nelle unità di base del sensore (°C, byte/s, %). La UI le converte da e verso le unità di visualizzazione; per °F l'isteresi si converte come differenza (× 9/5, senza il +32).
- **Nel file** (`rules`) si salvano solo:
  - `overrides`: per id di regola predefinita, i soli campi cambiati (`enabled`, `warn`, `crit`, `hysteresis`, `notify`);
  - `custom`: le regole personalizzate complete.

  Una nuova regola predefinita di una versione futura compare quindi da sola, e "Ripristina" su una regola cancella la sua voce in `overrides`. Un override che si riferisce a una regola predefinita che non esiste più si ignora.

### 3.3 Regole predefinite

| Id | Obiettivo | Condizione | Attenzione | Critico | Durata (att. / crit.) | Note |
|---|---|---|---|---|---|---|
| `cpu-temp` | temperatura del package CPU (nome definito dallo spike S1) | `above` | `tjMaxC` − 10, ripiego 85 | `tjMaxC`, ripiego 95 | 30 s / 10 s | Richiede il servizio (B7, B8). |
| `cpu-throttle` | flag di throttling termico CPU | `flagActive` | — | attivo | — / 10 s | Solo se lo spike S1 trova il sensore. |
| `gpu-temp` | temperatura core di ogni GPU | `above` | 83 | 90 | 30 s / 10 s | |
| `gpu-hotspot` | hotspot di ogni GPU | `above` | 95 | 105 | 30 s / 10 s | Solo se il sensore esiste. |
| `gpu-mem-temp` | temperatura memoria (junction) di ogni GPU | `above` | 100 | 105 | 30 s / 10 s | |
| `gpu-throttle` | `…/flag/throttle-thermal` di ogni GPU | `flagActive` | attivo | — | 10 s / — | |
| `disk-temp` | temperatura di ogni disco (`…/temperature/drive`) | `above` | `tempWarningC`, ripiego 70 | `tempCriticalC`, ripiego 80 | 30 s / 30 s | La spec principale dice "SSD NVMe"; qui vale per ogni disco con il sensore, perché anche i SATA con temperatura hanno WCTEMP/CCTEMP o il ripiego. |
| `disk-wear` | `…/percent/wear` di ogni disco | `above` | 90 | — | 0 s | Richiede il servizio; con `above` 90 vale "≥ 90" (§3.4). |
| `disk-critical` | `…/flag/critical-warning` di ogni disco | `flagActive` | — | attivo | — / 0 s | Richiede il servizio e lo spike S1. |
| `volume-used` | percentuale usata di ogni volume | `above` | 90 | 97 | 0 s / 0 s | B9. |
| `ram-used` | RAM usata (%) | `above` | 90 | 97 | 60 s / 30 s | |
| `battery-low` | carica della batteria | `below` | 15 | 5 | 0 s / 0 s | Nessuna istanza finché non c'è un provider batteria. |

Gli id dei sensori obiettivo si fissano nel piano della M5b, dopo lo spike, con un test che verifica che ogni selettore predefinito trovi almeno un sensore negli snapshot registrati di questa macchina (tranne `battery-low` e le regole marcate "solo se esiste").

**Nessuna regola predefinita sulle ventole ferme** (spec principale §4.3).

### 3.4 Valutazione

- **Istanze:** a ogni cambio di schema, ogni regola attiva si espande in istanze, una per sensore concreto che corrisponde al `target`. Per ogni istanza si risolvono le soglie dalle proprietà del device. Un'istanza che esisteva già (stesso id di regola e di sensore) conserva il suo stato; un sensore che sparisce porta via la sua istanza. Anche modificare una regola ricrea le sue istanze, azzerandone lo stato.
- **Confronti:** `above` scatta con valore ≥ soglia, `below` con valore ≤ soglia, `flagActive` con valore ≠ 0.
- **Macchina a stati per istanza:** livelli `ok`, `warn`, `crit`.
  - **Ingresso:** il livello candidato è il più grave la cui condizione è vera. Se resta vera ininterrottamente per la durata di quel livello, l'istanza entra nel livello. Una durata di 0 s fa entrare subito, al primo tick. Da `ok` si può entrare direttamente in `crit`, se la condizione del critico regge per la sua durata.
  - **Uscita:** da un livello si esce quando il valore resta oltre la soglia di quel livello meno l'isteresi (più l'isteresi per `below`; per `flagActive`, con il flag a 0) per tutta la durata dell'isteresi. Da `crit` si scende a `warn` se la condizione di `warn` è ancora vera, altrimenti a `ok`.
  - **Valori assenti:** non fanno entrare né uscire da un livello e azzerano i timer di ingresso e di uscita in corso. Un buco di un tick non fa scattare nulla e non spegne un allarme.
- **Orologio:** quello monotono del tick, passato al motore. I test usano un orologio finto e non dormono.
- **Livello complessivo:** il più grave tra le istanze; `neutral` se non esiste nessuna istanza, per esempio con tutte le regole disattivate.

### 3.5 Uscite

- **`HealthReport`**, prodotto dal tick e inviato alla UI con l'evento `oma:health` solo quando cambia; il comando `get_health` lo restituisce all'apertura della finestra:
  - `level`: `neutral` | `ok` | `warn` | `crit`;
  - `sinceMs`: istante (ora di sistema) da cui dura il livello complessivo;
  - `alerts[]`: una voce per istanza in `warn` o `crit`, con `ruleId`, `sensorId`, `level`, `value`, `threshold`, `sinceMs`, `messageKey` e `params`.

  "Cambia" significa: cambia il livello complessivo, o l'insieme o il livello degli allarmi, oppure il valore arrotondato di un allarme (per il testo del banner). Il valore si arrotonda alla precisione di visualizzazione, così un sensore che oscilla di decimi non genera eventi a ogni tick.
- **Banner della vista Semplificata:**
  - un allarme: "GPU surriscaldata (92 °C)", da `rule.<id>.message` con `{device}` (nome del device) e `{value}` formattato nelle unità scelte;
  - più allarmi: "N problemi", con un elenco a tendina accessibile da tastiera, ordinato per gravità e poi per durata;
  - nessun allarme: "Tutto in ordine" con la durata;
  - `neutral`: il messaggio attuale "monitoraggio attivo da…".

  Le regole personalizzate usano messaggi generici: `rule.custom.above` ("{sensor} sopra {threshold} ({value})"), `rule.custom.below`, `rule.custom.flag`.
- **Notifiche (toast di Windows):**
  - implementate in `oma-win` con le API WinRT del crate `windows` (`ToastNotificationManager`, `ToastNotification`), senza plugin;
  - un toast quando un'istanza con `notify` attivo per quel livello entra nel livello; poi silenzio per quell'istanza e quel livello finché non esce dal livello **e** sono passati almeno 5 minuti dall'ultimo toast (B2). Il cooldown è una funzione pura e ha i suoi test;
  - titolo e testo localizzati in Rust con gli stessi cataloghi `en.json`/`it.json` della tray, nella lingua delle impostazioni;
  - un clic sul toast, finché l'app è in esecuzione (sempre vero, perché vive nella tray), apre la finestra sulla pagina del device nella vista Avanzata tramite l'evento `Activated`. Un clic dopo la chiusura dell'app non fa nulla: niente attivatore COM registrato;
  - **AUMID:** quello della scorciatoia del menu Start creata dall'installer. Il piano verifica quale AUMID imposta il template NSIS di Tauri e, se serve, lo fissa sull'`identifier` `io.github.openmonitoradvanced` in `oma.nsh`. In sviluppo (`pnpm tauri dev`, senza scorciatoia) i toast possono comparire sotto un'altra identità o non comparire: limite accettato;
  - un toast che fallisce finisce nel log di diagnostica e non ha altri effetti.
- **Tray:** lo sfondo dell'icona segue il livello: `ok` `--ok`, `warn` `--warn`, `crit` `--crit`, `neutral` `--surface-2`. Le cifre sono chiare su `neutral` e scure sui colori di stato, per il contrasto. Il tooltip antepone il verdetto ("GPU surriscaldata (92 °C) · CPU 45 °C · …") quando il livello è `warn` o `crit`.

### 3.6 Impostazioni › Regole e avvisi

- **Tabella delle regole:** nome, obiettivo (dispositivo e sensore, oppure "ogni GPU"), soglie attenzione e critico nelle unità scelte, durate, interruttore "attiva", campanella per livello, badge "modificata" con "Ripristina" per le predefinite, "Elimina" per le personalizzate.
- **Pannello di modifica:** gli stessi campi, più l'isteresi. Le soglie derivate da una proprietà mostrano il valore risolto ("95 °C, da TjMax") e possono diventare fisse modificandole.
- **Nuova regola:** sensore scelto con una ricerca per dispositivo ed etichetta; condizione (le opzioni dipendono dall'unità: `flagActive` solo per i flag); soglia di attenzione e/o di critico; durata; isteresi; notifiche.
- **Scorciatoia dalla vista Avanzata:** nella tabella dei sensori, un'icona "Crea regola…" su hover o focus della riga apre le Impostazioni sul pannello "Nuova regola" già compilato con quel sensore.
- **Validazione in Rust**, nell'`update_settings`:
  - almeno una soglia tra attenzione e critico;
  - con `above` il critico ≥ l'attenzione, con `below` il critico ≤ l'attenzione;
  - durate da 0 a 600 s;
  - isteresi con quantità ≥ 0 e durata da 0 a 600 s;
  - sensore o selettore sintatticamente valido; una regola personalizzata su un sensore che oggi non esiste è ammessa, perché il sensore può arrivare con il servizio.

  La UI mostra l'errore accanto al campo.

### 3.7 Costo

La valutazione costa O(istanze) confronti per tick: qualche centinaio di istanze al massimo, meno di un microsecondo. Nessuna allocazione per tick quando lo stato non cambia; l'espansione dei selettori avviene solo al cambio di schema o di regole; `oma:health` non parte se non cambia nulla.

## 4. Log CSV (M5c)

### 4.1 Stati e comandi

- **Stati:** `idle` → `recording` ⇄ `paused` → `idle`, più `error` (con il motivo), da cui si esce con un nuovo avvio.
- **Comandi Rust:** `log_start`, `log_pause`, `log_resume`, `log_stop`, `get_log_status`. Sono gli stessi per UI, tray e scorciatoia; un comando non valido nello stato corrente (per esempio `log_pause` in `idle`) non fa nulla e restituisce lo stato.
- **Evento `oma:log`:** stato, percorso del file corrente, tempo registrato (pause escluse), righe scritte, byte, numero della parte, motivo dell'errore. Parte a ogni cambio di stato e, durante la registrazione, al più una volta al secondo per i contatori.
- **Pausa:** il file resta aperto e non si scrivono righe; alla ripresa i timestamp mostrano il salto.
- **Stop:** svuota il buffer e chiude il file. Anche durante la registrazione il buffer si svuota almeno ogni 5 s o ogni 64 KB, così un crash perde al massimo pochi secondi. La chiusura dell'app ferma la registrazione in modo pulito.

### 4.2 Architettura (B6)

- A ogni tick il campionatore, se la registrazione è attiva e il tick è uno di quelli da registrare, manda al thread di scrittura una riga grezza: timestamp, offset del fuso e valori delle colonne scelte. Il canale è limitato (per esempio 64 righe); se è pieno la riga si scarta, il conteggio delle righe scartate si registra nel log di diagnostica e compare nell'evento `oma:log`.
- Il **formatter** è una funzione pura in `oma-core::csv`: intestazione, righe, numeri, BOM. Il thread di scrittura, nella shell, gestisce file, buffer, dimensione e parti.
- **Offset del fuso:** la shell lo calcola per ogni riga con le API di Windows in `oma-win` (con cache per minuto), così il cambio dell'ora legale si vede nelle righe e il formatter resta portabile.

### 4.3 Formato

Dalla spec principale §4.4, con queste precisazioni:

- **Nome del file:** `oma-<AAAA-MM-GG_hh-mm-ss>.csv`, all'ora locale di avvio; le parti successive sono `…-part2.csv`, `…-part3.csv`.
- **Nuova parte**, con la sua intestazione e il BOM:
  - oltre la dimensione massima (100 MB di default, da 10 MB a 2 GB);
  - quando cambia l'insieme delle colonne, per esempio perché si collega un disco o arriva il servizio.

  Ogni file resta così coerente per Excel.
- **Colonne:**
  - prima colonna `Timestamp`, in ISO 8601 all'ora locale con i millisecondi e l'offset (`2026-09-29T14:03:12.000+02:00`);
  - poi un sensore per colonna, nell'ordine dello schema, con intestazione `Dispositivo / Sensore [unità]` nella lingua dell'app, per esempio `NVIDIA GeForce RTX 4080 / Temperatura core [°C]`. Un campo che contiene `,`, `"` o un a capo va tra virgolette doppie, con le virgolette interne raddoppiate.
- **Valori:** nelle **unità di visualizzazione** scelte all'avvio della registrazione (`[°F]`, `[bit/s]`), con al massimo 3 decimali e senza zeri finali; i flag come 0/1; i valori assenti come celle vuote. Cambiare unità durante la registrazione vale dalla parte successiva.
- **Intervallo:** ogni N tick dello scheduler, con N tra 1, 2, 5, 10, 30 e 60 (di default 1). Si scrive l'ultimo valore del tick, senza medie.
- **Colonne scelte:** tutti i sensori (default) oppure un elenco di id. Un sensore scelto che non esiste non ha colonna; se arriva, apre una nuova parte.
- **Errore di scrittura** (disco pieno, chiavetta rimossa, cartella senza permessi): la registrazione passa in `error` con il motivo; se la finestra è chiusa parte un toast, a prescindere dalle impostazioni delle regole.

### 4.4 Registratore nella UI e nella tray

- **Pulsante nella barra superiore**, sempre visibile:
  - a riposo, un'icona a cassetta;
  - in registrazione, un **pallino rosso** (`--crit`) che lampeggia e il contatore del nastro `00:12:47`;
  - in pausa, il simbolo ⏸ ambra (`--warn`) fisso e il contatore fermo;
  - in errore, un triangolo `--crit` fisso.
- **"Deck" a comparsa**, aperto da un clic sul pulsante:
  - due bobine SVG che girano solo mentre si registra e il deck è aperto;
  - un display a segmenti neon con tempo registrato, righe, dimensione e numero della parte;
  - un'etichetta di cassetta con il nome del file;
  - i tasti di trasporto **REC ●**, **PAUSA ❚❚** e **STOP ■**, con il contorno neon e lo stato premuto. REC in pausa riprende; i tasti non validi nello stato corrente sono disattivati;
  - "Apri cartella";
  - in errore, il motivo e REC per ripartire.

  L'aspetto si definisce nel task con la skill `frontend-design`, dentro la palette Synthwave: il rosso della registrazione è `--crit`, il rosa `--accent` si usa con parsimonia, e il neon resta sui bordi e sui simboli, senza bagliori diffusi. L'utente lo controlla nel browser con il backend finto prima del merge.
- **Leggerezza:**
  - il pallino lampeggia a 1 Hz a passi discreti: acceso 500 ms, spento 500 ms, con un timer della UI che cambia una classe, cioè 2 ridisegni al secondo e non un'animazione continua a 60 FPS. È l'unica animazione continua fuori dai grafici, consentita in deroga a §7.5 della spec principale, e la sua cadenza è fissa;
  - le bobine girano solo con il deck aperto;
  - con la finestra non visibile il timer si ferma;
  - con `prefers-reduced-motion` il pallino resta acceso fisso e le bobine restano ferme.
- **Tray:** le voci del log seguono lo stato ("Avvia log" in `idle` ed `error`; "Pausa log" e "Ferma log" in `recording`; "Riprendi log" e "Ferma log" in `paused`). Durante la registrazione l'icona dinamica ha un pallino rosso fisso in un angolo (§2.6); in pausa il pallino non c'è.

### 4.5 Impostazioni › Log CSV

La sezione `log` di `settings.json`:

```jsonc
"log": {
  "folder": null,              // null = Documenti\OpenMonitor Advanced\logs
  "sensors": null,             // null = tutti; altrimenti un elenco di id
  "everyTicks": 1,             // 1 | 2 | 5 | 10 | 30 | 60
  "maxFileMb": 100,            // 10–2048
  "hotkeyToggle": "Ctrl+Alt+Shift+R",   // null = nessuna
  "hotkeyPause": null
}
```

- La cartella di default si ottiene con `SHGetKnownFolderPath(FOLDERID_Documents)`, quindi segue lo spostamento della cartella Documenti. Si crea al primo avvio della registrazione.
- **Cartella:** selettore di cartelle di `tauri-plugin-dialog`, invocato solo dal lato Rust tramite un comando nostro (nessuna capability del plugin esposta a JavaScript), più "Apri cartella" (`ShellExecuteW` in `oma-win`) e "Ripristina".
- **Sensori:** "Tutti" oppure un albero dispositivo › categoria › sensore con caselle.
- **Intervallo:** mostrato come tempo effettivo ("ogni 5 s") calcolato dall'intervallo dello scheduler.
- **Scorciatoie:** un campo che registra la combinazione premuta, con almeno due modificatori tra Ctrl, Alt e Shift più un tasto. Accanto, lo stato della registrazione della scorciatoia: "attiva" oppure "già usata da un'altra app".

### 4.6 Scorciatoia globale

- Si registra con `tauri-plugin-global-shortcut` (basato su `RegisterHotKey`) **solo dal lato Rust**: nessuna capability del plugin esposta a JavaScript. Funziona anche a finestra chiusa e in gioco.
- `RegisterHotKey` non installa hook della tastiera, quindi non è un comportamento che gli anti-cheat penalizzano.
- **Default:** `Ctrl+Alt+Shift+R` avvia e ferma la registrazione, attiva di default; la scorciatoia di pausa/ripresa non ha una combinazione di default.
- Se la registrazione della combinazione fallisce (è già di un'altra app), la scorciatoia resta inattiva, il fatto finisce nel log e la sezione Log CSV lo mostra.
- Un avvio o uno stop dati dalla scorciatoia mostrano un breve toast di conferma, perché in gioco la tray non si vede; con un gioco a schermo intero Windows lo trattiene, come ogni toast.

## 5. Internazionalizzazione

- Tutte le nuove stringhe (impostazioni, regole, messaggi del banner, toast, tray, registratore) sono chiavi in `en.json` e `it.json`. Il test esistente verifica che le due lingue abbiano le stesse chiavi.
- Le stringhe usate da Rust (tray, toast, intestazioni del CSV) si leggono dagli stessi cataloghi, inclusi con `include_str!` come oggi in `tray.rs`, attraverso un piccolo modulo di traduzione condiviso nella shell. Questo modulo sostituisce `labels_for` e gestisce anche i parametri `{name}` e le etichette dei sensori (`sensor.<key>` con `{arg}`).
- Un test Rust verifica che ogni chiave usata dal codice Rust esista in entrambi i cataloghi.

## 6. Test

TDD come nelle milestone precedenti: prima i test che falliscono, poi l'implementazione.

- **`oma-core`, test puri:**
  - `settings`: default, chiavi mancanti e sconosciute, correzione dei valori fuori intervallo, patch parziali, file corrotto, versione futura, round-trip JSON;
  - `rules`: espansione dei selettori e conservazione dello stato al cambio di schema, soglie da proprietà con ripiego, durate di ingresso, 0 s, isteresi per `above`, `below` e `flagActive`, `crit → warn`, `ok → crit` diretto, valori assenti, livello complessivo, `HealthReport` che cambia solo quando serve, override e ripristino delle predefinite. Tutto con l'orologio finto;
  - `csv`: BOM, intestazione con virgolette, offset positivo e negativo, cambio dell'ora legale tra due righe, decimali e zeri finali, celle vuote, flag, conversione delle unità, nuova parte per dimensione e per colonne, pausa.
- **Shell e `oma-win`:**
  - scrittura atomica, coalescenza, recupero del file corrotto, migrazione di `service.json` su una cartella temporanea;
  - valore `Run` e `StartupApproved\Run` su una chiave di test sotto HKCU;
  - rendering dell'icona della tray (pixel attesi per cifre, trattino, colori e pallino);
  - traduzioni Rust e troncamento del tooltip;
  - cooldown dei toast e selezione delle voci della tray per stato, come funzioni pure;
  - thread di scrittura CSV con un writer finto: coda piena, errore di scrittura, flush.

  Il toast vero, la scorciatoia vera e il selettore di cartelle sono test `#[ignore]` o controlli manuali.
- **Protocollo v2:** nuove fixture di `Hello` e `Subscribe`, confrontate byte per byte da Rust e da .NET; xUnit per l'unione dei moduli tra più client, lo SMART spento per disco con il filtro D6, il ricalcolo alla disconnessione di un client e lo stato di PawnIO.
- **Servizio (M5b):** fixture di `SchemaBuilderTests` con i sensori trovati dallo spike S1 (throttling, critical warning) e la tabella TjMax.
- **UI (Vitest):**
  - store delle impostazioni e migrazione una tantum da `localStorage`;
  - conversione delle unità nelle soglie e nell'isteresi;
  - banner con 0, 1 e N allarmi e con `neutral`;
  - tabella e pannello delle regole, errori di validazione;
  - stati del registratore, compreso il pallino fisso con reduced motion;
  - stesse chiavi nelle due lingue.

  Il backend finto simula impostazioni, regole, allarmi e log, per sviluppare e verificare la UI nel browser.
- **Verifiche dal vivo:** solo osservazione. I clic su tray, toast, finestra e scorciatoia li fa l'utente, su richiesta (regola del progetto).

## 7. Budget

Si misura con `scripts/measure-footprint.ps1` alla fine di ogni piano e si registra in `docs/perf-budget.md`:

- tray < 30 MB, con icona dinamica, regole attive e, dalla M5c, un log in corso;
- CPU a riposo < 1%, con il log a 1 s e il pallino che lampeggia;
- finestra < 200 MB, WebView2 compresa, con le Impostazioni aperte e con la vista Avanzata;
- servizio < 1% di CPU e < 80 MB con i moduli tutti accesi.

## 8. Piani

Un piano per sotto-milestone, eseguito con subagent-driven development, ognuno su un proprio branch con merge in `main` in locale.

1. **M5a — `feat/m5a-impostazioni-tray`:**
   - tipi, I/O e migrazioni delle impostazioni;
   - applicazione a caldo di lingua, unità, intervallo, FPS e librerie dei vendor;
   - vista Impostazioni con Generale, Fonti dati e Informazioni;
   - tray completa con icona dinamica neutra, tooltip, avvio con Windows, chiusura nella tray e seconda istanza;
   - protocollo v2 con moduli, SMART per disco e stato di PawnIO;
   - correzioni della shell di §2.9;
   - modulo di traduzione condiviso in Rust.
2. **M5b — `feat/m5b-regole`:**
   - spike S1;
   - mappature e tabella TjMax nel servizio;
   - motore regole e `HealthReport`;
   - banner, toast e colore della tray;
   - Regole e avvisi, con la scorciatoia "Crea regola…" dalla vista Avanzata.
3. **M5c — `feat/m5c-log-csv`:**
   - formatter e thread di scrittura;
   - comandi e voci della tray;
   - scorciatoia globale;
   - registratore nella barra superiore;
   - sezione Log CSV;
   - misura finale del budget della M5.

Alla fine della M5c si aggiornano `docs/follow-ups.md` (voci chiuse e nuove), il README se cambia il comportamento visibile e la memoria dei follow-up.

## 9. Modifiche alla spec principale

Da applicare insieme a questa spec:

- §4.3: la CPU ha due regole (B7), la temperatura dei dischi vale per ogni disco con il sensore, lo spazio libero si esprime come percentuale usata (B9); rimando a questa spec per modello e valutazione.
- §4.4: registratore nella UI e scorciatoia globale (B10); rimando a questa spec.
- §4.5 e §4.6: rimando a questa spec per menu, icona e formato di `settings.json`.
- §7.4 e §9: il controllo degli aggiornamenti passa alla M6 (B3).
- §7.5: la deroga per il pallino del registratore (§4.4 di questa spec).
- §14: la M5 si esegue in tre piani (B1).
