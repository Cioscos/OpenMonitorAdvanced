# OpenMonitor Advanced

[English](README.md) · **Italiano**

[![CI](https://github.com/Cioscos/OpenMonitorAdvanced/actions/workflows/ci.yml/badge.svg)](https://github.com/Cioscos/OpenMonitorAdvanced/actions/workflows/ci.yml)
[![Licenza: GPL v3+](https://img.shields.io/badge/licenza-GPL--3.0--or--later-blue.svg)](LICENSE)
![Piattaforma: Windows 10/11](https://img.shields.io/badge/piattaforma-Windows%2010%20%7C%2011-0078D4.svg)

Un monitor hardware gratuito e open source per Windows 10 e 11, con un'interfaccia moderna.

- La **vista Semplificata** ti dice a colpo d'occhio se il PC sta bene.
- La **vista Avanzata** mostra ogni sensore, con grafici storici e statistiche.

CPU, RAM, dischi, rete e GPU (NVIDIA, AMD, Intel) si leggono **senza privilegi di
amministratore**. Un servizio Windows facoltativo aggiunge i sensori che li richiedono:
temperature, tensioni, ventole e dati SMART.

> **Stato:** sviluppo iniziale (versione 0.1.0). Aspettati imperfezioni e cambiamenti
> incompatibili tra una versione e l'altra.

## Funzionalità

- **Due viste.** La vista Semplificata ha un riquadro per componente. Un clic sul riquadro apre
  la pagina di quel componente nella vista Avanzata.
- **Una pagina per componente** nella vista Avanzata: la CPU, ogni GPU (anche quelle integrate),
  la RAM, ogni disco e ogni scheda di rete. Ogni pagina mostra:
  - quattro valori chiave;
  - un grafico storico degli ultimi 1, 5, 30 o 60 minuti, con fino a 8 serie e 2 unità. Le
    finestre di 30 e 60 minuti disegnano una banda minimo/massimo, così i picchi brevi restano
    visibili;
  - una tabella di tutti i sensori con valore attuale, minimo, massimo e medio. Le statistiche
    partono dall'avvio dell'app e continuano anche quando l'app è nel tray. Passando sopra un
    sensore vedi da dove arriva il suo valore;
  - i dettagli statici del dispositivo, come il collegamento PCIe, i limiti di potenza e le
    soglie di temperatura;
  - per le GPU, i processi che le usano, con carico e memoria.
- **Vista Impostazioni.** Lingua, temperatura in °C o °F, velocità di rete in bit/s o byte/s,
  intervallo di campionamento (da 0,5 a 5 s), frequenza di aggiornamento dei grafici (60, 30 o
  15 FPS), vista predefinita, chiusura nel tray, avvio con Windows e sensore mostrato
  dall'icona del tray. La sezione *Regole e avvisi* elenca le regole predefinite e le tue (vedi
  sotto). La sezione *Log CSV* imposta cartella, sensori, intervallo, limite di dimensione e
  scorciatoie del log (vedi sotto). La sezione *Fonti dati* attiva o disattiva ogni libreria dei produttori
  di GPU, la modalità compatibile con gli anti-cheat, ogni modulo del servizio e, disco per
  disco, la lettura SMART, e mostra lo stato di PawnIO. *Informazioni* elenca versioni e
  licenze. Le impostazioni sono salvate in `%APPDATA%\OpenMonitorAdvanced\settings.json`.
- **Icona nel tray.** L'icona mostra dal vivo il sensore scelto: una temperatura come numero,
  un carico come barra verticale. Diventa ambra o rossa quando una regola è in attenzione o in
  critico, e allora il suggerimento comincia con il problema prima di CPU, GPU e RAM. Il menu apre
  direttamente la vista Semplificata o Avanzata.
- **Regole e avvisi.** Le regole predefinite controllano le temperature di CPU, GPU e dischi, il
  throttling della GPU, l'uso della RAM, i volumi pieni, l'usura e il critical warning degli NVMe.
  Le soglie della CPU seguono il TjMax del processore, quando è noto. Ogni regola ha un livello di
  attenzione e uno critico, ciascuno con una soglia e il tempo per cui il valore deve superarla,
  più un'isteresi. Puoi modificarle, spegnerle, ripristinarle o aggiungere una regola tua su
  qualsiasi sensore, anche con *Crea regola…* da un sensore della vista Avanzata. Il banner della
  vista Semplificata dice se è tutto in ordine, oppure quali problemi sono attivi e da quanto.
  L'ingresso nel livello critico fa partire anche una notifica di Windows (anche il livello di
  attenzione, se lo attivi), al massimo una ogni 5 minuti per regola, dispositivo e livello; il
  clic sulla notifica apre la pagina del dispositivo.
- **Log CSV.** Un registratore a cassette nella barra in alto apre un pannello con registra,
  pausa e stop; il menu del tray ha le stesse voci, e un puntino sull'icona del tray compare
  mentre una registrazione è in corso. Una scorciatoia globale, **Ctrl+Alt+Shift+R** per
  impostazione predefinita, la avvia e la ferma anche con la finestra chiusa, e una seconda
  scorciatoia facoltativa mette in pausa e riprende. Il file è UTF-8 con BOM, separato da
  virgole, con fine riga CRLF: una riga per ogni tick campionato (o ogni 1-60 tick), un
  timestamp locale con lo scarto dall'UTC e una colonna per sensore, chiamata
  `Dispositivo / Sensore [unità] {id}`. I file vanno in `Documenti\OpenMonitor Advanced\logs`,
  salvo un'altra cartella a tua scelta. Una nuova parte (`-part2`, `-part3`, ...) parte quando il file
  raggiunge il limite di dimensione (100 MiB per impostazione predefinita, da 10 a 2048) o
  quando cambiano le colonne: lingua, unità o sensori selezionati. *Impostazioni › Log CSV*
  imposta cartella, sensori, intervallo, limite di dimensione e scorciatoie. Un errore, come una
  unità USB rimossa, ferma la registrazione con una notifica e il motivo nel pannello.
- **Supporto GPU** per NVIDIA, AMD e Intel, tramite Windows e le librerie installate con il driver
  grafico.
- **Leggero.** Il monitor non deve falsare ciò che misura. Il budget è meno dell'1% di CPU a riposo, meno di
  30 MB nel tray, meno di 200 MB con la finestra aperta (WebView2 compresa).
- **Interfaccia e installer in italiano e in inglese.**

## Download e installazione

1. Scarica `OpenMonitor.Advanced_<versione>_x64-setup.exe` dall'
   [ultima release](https://github.com/Cioscos/OpenMonitorAdvanced/releases/latest).
2. Avvialo. L'installer **non è ancora firmato digitalmente**, perciò Windows SmartScreen può
   mostrare un avviso: scegli *Ulteriori informazioni* → *Esegui comunque*.
3. Lascia selezionato il componente **Sensori avanzati** se vuoi temperature, tensioni, ventole e
   dati SMART (vedi [Servizio dei sensori](#servizio-dei-sensori)). Deselezionalo per
   un'installazione che non tocca driver né servizi.

Requisiti: Windows 10 o 11 a 64 bit, con il runtime Microsoft Edge WebView2 (preinstallato su
Windows 11). I privilegi di amministratore servono solo durante l'installazione.

Per le installazioni automatiche, `/S` avvia l'installer in modalità silenziosa e `/NOSENSORS`
esclude il componente Sensori avanzati.

## Supporto GPU

I dati delle GPU non richiedono mai i privilegi di amministratore. Ogni GPU ha il carico per
motore e la memoria dedicata e condivisa dai contatori di prestazioni di Windows, più
temperatura del core, frequenze e potenza dove il driver le fornisce. In più l'app usa le
librerie dei produttori installate con il driver grafico:

| Produttore | Libreria | Cosa aggiunge |
|---|---|---|
| NVIDIA | NVML | Temperatura, frequenze, potenza della scheda e limite, ventola, VRAM, motivi di throttling |
| NVIDIA | NVAPI | Temperature hotspot e memory junction, tensione del core (*sperimentale*: chiamate non documentate) |
| AMD | ADL | Temperature, frequenze, potenza, ventola e tensione, dove la GPU le espone |
| Intel Arc / Xe | IGCL | Implementato, non ancora verificato su hardware Intel |

Le librerie dei produttori si caricano solo da `System32` e non vengono mai ridistribuite.

**Modalità sicura.** Un crash dentro una libreria di un produttore non si può intercettare. Avvia
`oma-app.exe --safe` per saltare le librerie dei produttori e usare solo i dati di Windows. Dopo
un crash nativo l'app parte da sola in modalità sicura la volta successiva. In entrambi i casi un
avviso offre **Riattiva** per ricaricare le librerie senza riavviare.

## Servizio dei sensori

Il servizio Windows facoltativo `oma-service` legge i sensori che richiedono i privilegi di
amministratore: temperature e tensioni di CPU e scheda madre, SPD della RAM, controller di
ventole e RGB, salute SMART/NVMe dei dischi. Usa
[LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) e il driver
[PawnIO](https://pawnio.eu/), firmato da Microsoft, e gira come `LocalSystem`.

- **L'app funziona anche senza.** Senza il servizio, CPU, RAM, dischi, rete e GPU continuano a
  funzionare. Con il servizio, le pagine di CPU, RAM e dischi mostrano i sensori aggiuntivi e
  compare una pagina Scheda madre. Un badge nella barra in alto spiega perché il servizio non è
  collegato e propone l'azione giusta.
- **Non parte con Windows.** L'app avvia il servizio quando serve e il servizio si ferma da solo
  2 minuti dopo che l'ultimo client si è scollegato.
- **Modalità compatibile con gli anti-cheat.** Dal menu del tray puoi fermare il servizio e
  impedire all'app di riavviarlo, per i giochi con un anti-cheat. Questo chiude il servizio e ogni
  handle aperto su PawnIO. Non scarica il driver PawnIO, che Windows carica all'avvio e che altri
  programmi (per esempio FanControl) possono condividere. FACEIT accetta PawnIO 2.2.0; non sono
  noti blocchi da parte di Vanguard, EAC o BattlEye. Nulla di questo è ancora stato provato con un
  gioco protetto da un anti-cheat reale.
- **Limiti noti.** Su un PC con più utenti collegati, ognuno di loro può fermare il servizio per
  tutti. Un disco che Windows segnala in standby non viene interrogato, quindi non mostra dati di
  salute mentre dorme. Se un disco collegato non conferma il proprio stato di alimentazione
  all'avvio del servizio (per esempio una chiavetta USB), la lettura SMART resta spenta per tutti
  i dischi finché non viene scollegato. Mentre un disco dorme, il banner può segnalare dati
  incompleti. Il throttling termico della CPU non è disponibile con LibreHardwareMonitor 0.9.6,
  quindi quella regola non ha un sensore. Il critical warning dei dischi c'è solo per gli NVMe. Il
  TjMax del processore è noto per le CPU Intel e per i modelli AMD desktop della tabella
  integrata; le altre CPU usano le soglie di ripiego (85/95 °C). Excel con il punto e virgola come
  separatore di elenco (molte impostazioni regionali europee) mostra il log CSV in una sola
  colonna: aprilo con *Dati* → *Da testo/CSV* e scegli la virgola.

## Compilare dal sorgente

### Prerequisiti

- Windows 10/11 x64
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) con il carico di
  lavoro *Sviluppo di applicazioni desktop con C++* (MSVC e Windows SDK)
- [Rust](https://rustup.rs/): `rustup` installa alla prima build la toolchain fissata (1.90.0, da
  `rust-toolchain.toml`)
- [Node.js](https://nodejs.org/) 22 e [pnpm](https://pnpm.io/) 10
- [.NET SDK](https://dotnet.microsoft.com/download) 10.0.303 o una patch 10.0.3xx successiva
  (fissato in `global.json`), per il servizio dei sensori
- PowerShell 7 (`pwsh`), per gli script di build

Il bundler di Tauri scarica NSIS da solo la prima volta che compili l'installer.

### Avvio in sviluppo

```bash
cd app
pnpm install
pnpm tauri dev    # l'app completa
pnpm dev          # solo l'interfaccia, nel browser, con un backend finto e hot reload
```

### Test e controlli

```bash
cd app && pnpm install && pnpm build && cd ..   # la build Rust richiede app/dist
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && pnpm test && pnpm check && cd ..
dotnet test service/OpenMonitorAdvanced.slnx
cargo test -p oma-win -- --include-ignored      # esegue anche i test che richiedono una GPU reale
```

### Compilare l'installer

```bash
pwsh scripts/build-installer-payload.ps1
cd app
pnpm tauri build --bundles nsis
```

Lo script del payload pubblica `oma-service` (self-contained, trimmed, file singolo, win-x64) e
scarica il setup ufficiale di PawnIO 2.2.0, verificato con lo SHA-256 in
`app/src-tauri/nsis/pawnio.sha256`. PawnIO non viene mai incluso nel repository. L'installer
finisce in `target/release/bundle/nsis/`.

## Struttura del progetto

| Percorso | Contenuto |
|---|---|
| `crates/oma-core` | Modello dati, scheduler di campionamento, merge per fonte, storico. Niente codice Windows. |
| `crates/oma-win` | Provider Windows: PDH, D3DKMT, DXGI, NVML, NVAPI, ADL, IGCL, dischi, rete |
| `crates/oma-ipc` | Tipi del protocollo, codifica MessagePack e framing per parlare con `oma-service` |
| `app/src-tauri` | Shell Tauri 2 (`oma-app`): comandi, tray, finestra, modalità sicura, template e hook NSIS |
| `app/src` | Interfaccia Svelte 5 + TypeScript, traduzioni italiana e inglese |
| `service/` | `oma-service`, servizio Windows .NET 10 basato su LibreHardwareMonitorLib, con i suoi test |
| `protocol/fixtures/` | Messaggi MessagePack di riferimento condivisi dai test del protocollo Rust e .NET |
| `docs/` | Specifica di progetto, piani per milestone e budget delle prestazioni |
| `scripts/` | Payload dell'installer, misura dei consumi e altri script di build |

## Contribuire

Issue e pull request sono benvenute. Prima di aprire una pull request, esegui i controlli di
[Test e controlli](#test-e-controlli). Codice, commenti e messaggi di commit sono in inglese e
seguono i [Conventional Commits](https://www.conventionalcommits.org/). I documenti di progetto in
`docs/` sono in italiano.

## Licenza

OpenMonitor Advanced è software libero, distribuito con la
[GNU General Public License v3.0 o successiva](LICENSE). Puoi usarlo, studiarlo, condividerlo e
modificarlo; se distribuisci una versione modificata, devi pubblicarne il codice sorgente con la
stessa licenza.

I componenti di terze parti e le loro licenze sono elencati in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
