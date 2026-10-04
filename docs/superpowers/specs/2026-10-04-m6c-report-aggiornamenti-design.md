# M6c — Report, aggiornamenti e licenze: design di dettaglio

- **Data:** 2026-10-04
- **Stato:** design del brainstorming del 2026-10-04, approvato per sezioni dall'utente. Le verifiche del §8.2 sono parte del piano, non fatti già acquisiti.
- **Spec principale:** `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`. Per i punti trattati qui (controllo degli aggiornamenti, report sensori, licenze di terze parti, release 0.4.0) questa spec ha la precedenza; il resto resta come nella spec principale e nelle spec M5, M6a e M6b.
- **Riferimenti:** `CODE_SIGNING.md` (Privacy), `docs/superpowers/specs/2026-09-30-m6a-release-firma-design.md` (§7, condizioni della SignPath Foundation), `docs/release.md`, `docs/follow-ups.md`.

## 1. Intento e confini

La M6c è il terzo e ultimo piano della M6. Fa quattro cose:

1. aggiunge il **controllo degli aggiornamenti**: la prima e unica richiesta di rete dell'app, verso GitHub, solo su azione esplicita dell'utente;
2. aggiunge **«Esporta report sensori»**: un JSON anonimo da allegare alle segnalazioni, con cui la community può riempire la matrice hardware;
3. completa le **licenze di terze parti**: un file generato con gli avvisi e i testi di tutte le dipendenze ridistribuite, controllato in CI;
4. aggiorna la **documentazione** e prepara la **release 0.4.0**, che contiene anche la M6b.

**Obiettivo misurabile:** dall'installer 0.4.0 l'utente può controllare gli aggiornamenti (a mano o una volta al giorno, se lo attiva), esportare un report in cui non compaiono identificativi della macchina, e aprire i testi delle licenze di ogni componente ridistribuito. Senza un'azione dell'utente l'app non fa alcuna richiesta di rete.

**Fuori dalla M6c:**

- la release 1.0, che aspetta la firma SignPath, la conferma dell'autore di PawnIO e la matrice hardware (§1.1, D1);
- l'installazione automatica degli aggiornamenti e il plugin updater di Tauri (spec principale §9);
- l'invio del report: l'utente lo allega a mano a una segnalazione;
- la richiesta all'autore di PawnIO: si prepara solo la bozza, da pubblicare su richiesta dell'utente;
- i follow-up marcati "when touched" e le verifiche manuali ancora dovute dalla M6b.

### 1.1 Decisioni del brainstorming

| # | Decisione |
|---|---|
| D1 | La release che chiude la M6c è la **0.4.0**, con M6b e M6c insieme; la 1.0 arriva dopo firma, conferma di PawnIO e matrice hardware. |
| D2 | Controllo degli aggiornamenti: pulsante **«Controlla ora»** sempre manuale, più la casella **«Controlla automaticamente»**, spenta di default. Compatibile con la condizione della SignPath Foundation: nessuna trasmissione in rete se non la chiede l'utente. |
| D3 | Con il controllo automatico attivo, una nuova versione dà **un toast di Windows una sola volta per versione** (il clic apre Informazioni) e **un segno discreto** sulla voce Informazioni finché resta disponibile. |
| D4 | Il report contiene, oltre a dispositivi, sensori, fonti e valori correnti, la **qualità** di ogni valore e **min, media e max della sessione**. Niente storico. |
| D5 | Anonimato: id dei dischi e di rete **sostituiti da indici**, alias di rete **sostituiti dal tipo** (Ethernet, Wi-Fi) più indice; modelli e vendor restano. |
| D6 | Richiesta HTTP con **WinHTTP** dal lato Rust (`oma-win`), con il crate `windows` già in uso: nessuna nuova dipendenza, proxy di sistema e certificati dello store di Windows. La webview non fa richieste di rete e la CSP non cambia. |
| D7 | Licenze: file **generato** `THIRD_PARTY_LICENSES.txt` (Rust con `cargo-about`, JS da `pnpm`, NuGet dalla pubblicazione del servizio), versionato, controllato in CI, incluso nell'installer. |

## 2. Controllo degli aggiornamenti

### 2.1 Richiesta HTTP (`crates/oma-win/src/http.rs`)

Una funzione sincrona, chiamata solo da un thread in background:

`get_json(url: &str, user_agent: &str, headers: &[(&str, &str)]) -> Result<HttpResponse, HttpError>`

- `HttpResponse { status: u16, body: Vec<u8> }`; `HttpError` distingue almeno: rete assente o host non risolto, timeout, errore TLS o certificato, risposta troppo grande, altro errore di WinHTTP (con il codice).
- Sessione con `WinHttpOpen` e `WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY` (proxy di sistema e WPAD); solo `https` (`WINHTTP_FLAG_SECURE`); `WINHTTP_OPTION_SECURE_PROTOCOLS` limitato a TLS 1.2 e 1.3; `WINHTTP_DISABLE_COOKIES`; redirect solo da HTTPS a HTTPS (comportamento predefinito di WinHTTP, da fissare esplicitamente).
- Timeout di risoluzione, connessione, invio e ricezione con `WinHttpSetTimeouts`, e una scadenza complessiva di **10 s** misurata dal chiamante; corpo letto fino a **256 KB**, oltre si interrompe con l'errore "risposta troppo grande".
- Il certificato è verificato da WinHTTP con lo store di Windows; nessuna opzione per ignorare gli errori di certificato.
- Feature del crate `windows` da aggiungere a `oma-win`: `Win32_Networking_WinHttp`. Gli handle si chiudono in un tipo RAII; ogni blocco `unsafe` ha il suo `// SAFETY:`. Dopo il task, revisione con l'agente `ffi-safety-reviewer`.

### 2.2 Logica pura (`crates/oma-core/src/updates.rs`)

- **Richiesta:** `GET https://api.github.com/repos/Cioscos/OpenMonitorAdvanced/releases/latest` con `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28` e `User-Agent: OpenMonitorAdvanced/<versione> (+https://github.com/Cioscos/OpenMonitorAdvanced)`. Nessun token, nessun altro dato.
- **Parsing:** dalla risposta si leggono solo `tag_name`, `html_url`, `draft` e `prerelease`. Una bozza o una pre-release si scartano (l'endpoint `latest` non dovrebbe restituirle, ma si controlla). Stato HTTP diverso da 200, JSON non valido o campi mancanti danno "risposta non valida".
- **Versioni:** il tag deve essere `vX.Y.Z` con tre componenti canoniche tra 0 e 65535, come nelle regole della M6a; il confronto è numerico. La versione installata è quella dell'app (`CARGO_PKG_VERSION` di `oma-app`). Una release uguale o più vecchia dà "aggiornato"; una più nuova dà "disponibile".
- **URL:** `html_url` si accetta solo se inizia con `https://github.com/Cioscos/OpenMonitorAdvanced/releases/`; altrimenti la risposta non è valida. L'UI non passa mai un URL al backend: il comando di apertura usa quello validato e salvato.
- **Pianificazione:** `next_check(now_ms, started_ms, state, auto) -> Option<u64>` restituisce l'ora (ms, orologio di sistema) del prossimo controllo automatico, `None` con la casella spenta: primo controllo automatico 60 s dopo l'avvio se è dovuto; poi 24 h dopo l'ultimo controllo **riuscito**; dopo un errore, nuovo tentativo dopo 6 h. Un ultimo controllo con ora nel futuro (orologio spostato indietro) conta come dovuto.
- **Toast:** `should_notify(available, state) -> bool`: solo se la versione disponibile è più nuova di quella installata e diversa da `notifiedVersion`; solo per i controlli automatici. Un controllo manuale mostra il risultato nella pagina e non fa toast, ma registra la versione come notificata, così un controllo automatico successivo non ripete l'avviso.

### 2.3 Impostazioni e stato

- **Impostazioni:** nuova sezione `updates` con `checkAutomatically: bool`, default `false`. Il decoder delle impostazioni ignora le chiavi sconosciute e dà il default a quelle mancanti, quindi la versione del formato resta `1`; un file scritto dalla 0.4.0 e letto dalla 0.3.0 perde solo questa preferenza. Si modifica con `update_settings` come le altre (patch validata), e in sola lettura (file di una versione più recente) la casella è disattivata come le altre.
- **Stato:** `%APPDATA%\OpenMonitorAdvanced\update-state.json`, scritto in modo atomico, con `lastAttemptMs`, `lastSuccessMs`, `latest` (`{version, url}` dell'ultima risposta valida) e `notifiedVersion`. Un file assente o illeggibile vale come vuoto (al peggio si ripete un toast una volta). Non sta in `settings.json` perché non è una preferenza e non deve cambiare la revisione delle impostazioni.

### 2.4 Shell (`app/src-tauri/src/updates.rs`)

- **Thread di controllo:** creato all'avvio, attende su una condition variable il minimo tra la prossima scadenza e **1 h** (così dopo una sospensione il controllo dovuto parte entro un'ora); viene svegliato quando la casella cambia o arriva un controllo manuale. Con la casella spenta non fa richieste.
- **Una richiesta alla volta:** un controllo manuale mentre ne è in corso un altro attende quello e ne mostra l'esito.
- **Comandi Tauri:** `check_updates() -> UpdateStatus` (manuale, attende l'esito), `get_update_status() -> UpdateStatus`, `open_release_page()` (apre nel browser l'URL validato con `oma_win::shell_open`). Evento `oma:update-status` a ogni cambio di stato. Capability nuove nel file `default.json`, con la descrizione aggiornata.
- **`UpdateStatus`:** `{ state: "idle" | "checking" | "upToDate" | "available" | "error", current, latest?: {version}, checkedAtMs?, error?: "offline" | "timeout" | "tls" | "http" | "invalid" }`. `available` si ricalcola contro la versione installata a ogni avvio, così dopo l'aggiornamento il segno sparisce da solo.
- **Toast:** con il `Toaster` condiviso (regole e log); stringa di avvio nuova `{"open":"about"}`, `LaunchTarget::About`, che apre la finestra sulle Impostazioni, sezione Informazioni (`NavigationTarget` esteso con la sezione delle impostazioni). Testi da `it.json`/`en.json` lato shell (`i18n.rs`): titolo «OpenMonitor Advanced X.Y.Z disponibile», corpo «Apri Informazioni per scaricarla».
- **Versione finta (solo debug):** con `cfg(debug_assertions)`, la variabile d'ambiente `OMA_UPDATE_FAKE_CURRENT=X.Y.Z` sostituisce la versione installata nel confronto e nello User-Agent; nelle build di release non esiste.

### 2.5 Interfaccia (Impostazioni › Informazioni)

- Riga **«Aggiornamenti»** in `AboutSection.svelte`:
  - pulsante «Controlla ora», disattivato durante un controllo;
  - stato: «Controllo in corso…», «Hai l'ultima versione (X.Y.Z)», «Disponibile la versione X.Y.Z» con il pulsante «Pagina della release», oppure l'errore tradotto («Nessuna connessione», «Tempo scaduto», «Connessione sicura non riuscita», «GitHub ha risposto con un errore», «Risposta non valida»), sempre con l'ora dell'ultimo controllo;
  - casella «Controlla automaticamente (una volta al giorno)» con la nota: «Il controllo contatta GitHub (api.github.com) e invia solo l'indirizzo IP e la versione dell'app. Non installa nulla.»
- **Segno:** un pallino sulla voce Informazioni del menu delle impostazioni quando lo stato è `available`, con testo accessibile («Aggiornamento disponibile»).
- Le chiavi nuove vanno in `en.json` e `it.json` con le stesse chiavi; il backend finto (`mock.ts`) e `FakeBackend` imitano i comandi.

## 3. «Esporta report sensori»

### 3.1 Costruzione (`crates/oma-core/src/report.rs`)

Funzione pura `build_report(input: &ReportInput) -> serde_json::Value`, testabile senza hardware. `ReportInput` contiene schema, ultimo snapshot con la qualità, statistiche della sessione, stato del servizio e dei dischi, impostazioni delle fonti, stato della modalità sicura, versioni e ora.

Contenuto, con chiavi camelCase e valori nelle unità interne:

- `format: 1`, `generatedAt` (ISO 8601 in UTC), `app` (versione dell'app, del servizio o `null`, del protocollo), `os` (`major.minor.build` da `RtlGetVersion`, che non dipende dal manifest);
- `state`: servizio (stato del collegamento, modalità anti-cheat), modalità sicura GPU e motivo, provider attivi e spenti (dalle impostazioni delle fonti), stato di ogni disco (`active`, `idle`, `standby`, `unknown`);
- `devices`: `id`, `kind`, `name`, `vendor`, `properties` filtrate (§3.2);
- `sensors`: `id`, `deviceId`, `kind`, `unit`, `label` (chiave i18n e argomento), `source`, `category`, `experimental`, `value` (`null` se assente o non finito), `quality` (`fresh`, `held`, `suspended`), `stats` (`min`, `avg`, `max`, `count`, oppure `null` se non disponibili, per esempio durante un cambio di revisione dello schema).

Le etichette restano chiavi i18n, quindi il report è uguale in tutte le lingue; le unità scelte nell'interfaccia (°F, bit/s) non contano.

### 3.2 Anonimato

- **Id:** ogni device id che inizia con `storage/` diventa `storage/disk-N`, ogni id che inizia con `network/` diventa `network/adapter-N`, con N assegnato nell'ordine dello schema. La sostituzione vale ovunque l'id compare: dispositivi, prefisso degli id dei sensori (`<device_id>/<kind>/<name>`), `deviceId` dei sensori, stato dei dischi.
- **Nomi di rete:** il provider di rete aggiunge al dispositivo la proprietà `adapterType` (`ethernet` o `wifi`, dal tipo di interfaccia), che compare anche nella pagina del dispositivo con l'etichetta «Tipo di adattatore»; nel report il nome diventa «Ethernet N» o «Wi-Fi N» (N per tipo), e senza la proprietà «Adapter N».
- **Proprietà:** passano solo le chiavi di una lista bianca, cioè le chiavi `property.*` che l'interfaccia conosce oggi (`pciAddress`, `integrated`, `pcieMaxGen`, `pcieMaxWidth`, `powerLimit*`, `temp*`, `tjMaxC`, `availableSpareThresholdPct`) più `adapterType`. Una chiave nuova entra nel report solo aggiungendola alla lista.
- **Mai nel report:** percorsi, nome utente, nome del computer, indirizzi di rete, regole, impostazioni diverse dalle fonti, storico.

### 3.3 Shell e interfaccia

- Comando `export_sensor_report() -> Result<Option<ExportedReport>, String>`: raccoglie l'input sotto i lock esistenti (stesso percorso di `get_stats`), costruisce il JSON, apre «Salva con nome» con `tauri-plugin-dialog` (nome `oma-report-AAAAMMGG-HHMMSS.json` nell'ora locale, cartella Documenti, filtro `.json`) e scrive in modo atomico. `None` se l'utente annulla. La cartella dell'ultimo report resta nello stato della shell.
- Comando `reveal_sensor_report()`: apre in Esplora file la cartella dell'ultimo report esportato; non riceve percorsi dall'UI.
- In Informazioni: pulsante «Esporta report sensori» con una riga di spiegazione («Un file JSON anonimo con dispositivi, sensori, fonti e valori, da allegare alle segnalazioni»); a esportazione riuscita un messaggio con il nome del file e il pulsante «Apri cartella»; in caso di errore il testo dell'errore. Un annullamento non mostra nulla.

## 4. Licenze di terze parti

### 4.1 File generato

`THIRD_PARTY_LICENSES.txt`, nella radice del repository, versionato, testo UTF-8 con fine riga LF, ordinato in modo deterministico. Contiene:

1. **Rust:** con `cargo-about` (versione fissata), configurato da `about.toml` e da un template; solo le dipendenze compilate in `oma-app` e nei crate del workspace per `x86_64-pc-windows-msvc`, escluse quelle di sviluppo e di build. Le licenze accettate stanno in `about.toml` (MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, MPL-2.0, BSL-1.0, CC0-1.0 e quelle che risultano dal primo censimento, ognuna motivata); una licenza fuori lista fa fallire la generazione.
2. **JavaScript:** le dipendenze di produzione del bundle (`pnpm licenses list --prod --json` in `app/`), con il file di licenza copiato dal pacchetto in `node_modules`.
3. **.NET:** i pacchetti NuGet che finiscono nella pubblicazione di `oma-service` (dal file `.deps.json` della pubblicazione, esclusi analyzer e pacchetti di sola build), con il file di licenza del pacchetto se c'è, altrimenti il testo standard dell'espressione SPDX (testi in `scripts/licenses/`), più il runtime .NET 10 (MIT).
4. **Testi:** per ogni licenza con testo standard (Apache-2.0, MPL-2.0…) il testo completo una sola volta; per le licenze con avviso proprio (MIT, BSD) il testo con il copyright di ciascun titolare.

`THIRD_PARTY_NOTICES.md` resta il documento scritto a mano (fonti dei binding GPU, LibreHardwareMonitor, PawnIO) e rimanda al file generato.

### 4.2 Strumenti

- `scripts/generate-licenses.ps1` rigenera il file; `-Check` lo rigenera in una cartella temporanea e fallisce se differisce da quello versionato.
- In CI un job esegue `-Check` dopo il build del servizio; fallisce se il file non è aggiornato.
- Test Pester per le parti pure dello script (unione, ordinamento, deduplica dei testi).

### 4.3 Installer e interfaccia

- `tauri.conf.json` aggiunge la risorsa `THIRD_PARTY_LICENSES.txt` accanto a `THIRD_PARTY_NOTICES.txt`.
- `KnownPath::ThirdPartyLicenses` nuovo; in Informazioni, alla riga «Licenza», due pulsanti: «Avvisi di terze parti» (il file attuale) e «Testi delle licenze» (il file nuovo).

### 4.4 Questioni aperte da chiudere nel piano

- **`Mono.Posix.NETStandard` 1.0.0:** leggere i termini dietro il fwlink e riportarli in `THIRD_PARTY_NOTICES.md`. Se non permettono la ridistribuzione, ci si ferma e si chiede all'utente (alternative: escludere l'assembly dalla pubblicazione, dato che serve solo su Linux e macOS, dopo averlo verificato).
- **PawnIO:** la conferma dell'autore resta requisito della 1.0. Si scrive la bozza della richiesta in `docs/follow-ups.md`; si pubblica solo su richiesta dell'utente.

## 5. Documentazione e release 0.4.0

- **Spec principale:** §4.6 (impostazione `updates.checkAutomatically`), §7.4 (Informazioni), §9 (rete e aggiornamenti: unica richiesta di rete, solo su azione dell'utente), §14 (la M6 chiude con la 0.4.0; la 1.0 dopo firma, PawnIO e matrice hardware).
- **`CODE_SIGNING.md`, Privacy:** al posto del paragrafo "no update check" il testo del §5.1.
- **README.md e README.it.md:** controllo degli aggiornamenti e cosa invia; export del report; licenze; paragrafo «Segnalare un problema» che chiede di allegare il report.
- **Note di rilascio 0.4.0** (nelle novità manuali della bozza): novità di M6b e M6c; app e servizio devono avere la stessa versione per il protocollo v3, e l'installer aggiorna entrambi; correzione dei testi italiani dell'installer.
- **`docs/follow-ups.md`, `docs/perf-budget.md`, `CLAUDE.md`:** esiti della milestone, stato della M6, eventuali nuovi limiti.
- **Release:** alla fine `scripts/bump-version.ps1 0.4.0`, commit e tag `v0.4.0` in locale. Il push si fa solo su richiesta dell'utente; il workflow crea la bozza, che l'utente rivede e pubblica. La release è non firmata, come la 0.3.0.

### 5.1 Testo della Privacy (`CODE_SIGNING.md`)

Da usare come base, in inglese come il resto del file:

> **Update check (optional).** The app can check whether a newer release exists. It does so only when you press *Check now* in Settings › About, or once a day if you turn on *Check automatically* (off by default). The check is a single HTTPS request to `api.github.com` for the latest release of this repository; it sends your IP address, as any connection does, and a User-Agent with the app version. No identifiers, no settings and no sensor data are sent. GitHub's own policy applies to that request: [GitHub Privacy Statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement). The app never downloads or installs anything: it shows a link to the release page. The service never uses the network.

Il resto della sezione resta valido; la frase "The application … send no data over the network" diventa "send no data over the network, except the optional update check below".

## 6. Errori e casi limite

- **Senza rete, proxy che chiede autenticazione (407), GitHub non raggiungibile o limite di richieste (403/429):** stato `error` con la categoria, nessun toast, nuovo tentativo automatico dopo 6 h.
- **Versione installata più nuova dell'ultima release** (build di sviluppo): «Hai l'ultima versione».
- **Release disponibile e poi installata:** al riavvio lo stato si ricalcola e il segno sparisce; `notifiedVersion` impedisce un secondo toast per la stessa versione.
- **Casella spenta durante un controllo:** la richiesta in corso finisce, il risultato si mostra, nessun toast.
- **Toast disattivati in Windows:** il segno e lo stato in Informazioni restano.
- **Report senza servizio o subito dopo l'avvio:** si esporta lo stesso; valori e statistiche `null` dove mancano, stato del servizio esplicito.
- **Report con file non scrivibile:** errore mostrato con il testo di sistema; nessun file parziale (scrittura atomica).
- **Valori non finiti (NaN, infinito):** `null` nel JSON.
- **Licenza nuova in una dipendenza:** la generazione fallisce finché la licenza non è valutata e aggiunta ad `about.toml` con il motivo.

## 7. Limiti dichiarati

- Il controllo degli aggiornamenti dipende da GitHub: con l'API non raggiungibile si vede solo l'errore. Senza installazione automatica, l'aggiornamento resta un download manuale.
- Il report descrive ciò che l'app vede nel momento dell'esportazione; non contiene lo storico.
- Gli hash dei dischi e i GUID di rete restano nei file dell'app (storico, impostazioni): l'anonimato vale per il report.

## 8. Verifiche

### 8.1 Test automatici (TDD)

- **Rust, `oma-core`:** parsing di risposte reali di GitHub salvate come fixture (release normale, bozza, pre-release, tag malformato, componenti oltre 65535, URL di un altro repository o non HTTPS, campi mancanti, JSON non valido); confronto delle versioni; `next_check` (primo avvio, 24 h, 6 h dopo un errore, orologio indietro, casella spenta); `should_notify` (una volta per versione, nessun toast per i controlli manuali ma versione registrata); `build_report` con schema di prova (contenuto, qualità, statistiche `null`, valori non finiti) e anonimato (nel JSON non compaiono gli hash, i GUID e gli alias delle fixture; id dei sensori riscritti in modo coerente; proprietà fuori lista escluse); decodifica e patch di `updates.checkAutomatically`.
- **Rust, `oma-win`:** classificazione degli errori di WinHTTP (pura); un test `#[ignore = "requires network"]` che interroga GitHub davvero.
- **Rust, shell:** stato dell'aggiornamento letto da file assente, illeggibile e valido; `LaunchTarget::About` e la stringa di avvio; una richiesta alla volta.
- **UI (Vitest):** riga Aggiornamenti in tutti gli stati, pulsante disattivato durante il controllo, casella e nota, segno sul menu; export riuscito, annullato e fallito; due pulsanti delle licenze; chiavi i18n allineate.
- **Pester:** parti pure di `generate-licenses.ps1`.

### 8.2 Verifiche dal vivo, con l'utente

Niente input sintetico: le azioni nella finestra e sui toast le fa l'utente.

| # | Condizione | Atteso |
|---|---|---|
| U1 | app di sviluppo, «Controlla ora» | «Hai l'ultima versione»; nessuna richiesta prima del clic (verificato con il log della shell) |
| U2 | `OMA_UPDATE_FAKE_CURRENT=0.2.0`, «Controlla ora» | «Disponibile la versione 0.3.0»; «Pagina della release» apre la pagina della v0.3.0 |
| U3 | come U2, casella automatica attiva, riavvio dell'app | dopo circa 60 s un solo toast; il clic apre Informazioni; un secondo riavvio non ripete il toast; il segno resta |
| U4 | rete disattivata, «Controlla ora» | errore «Nessuna connessione», nessun toast |
| U5 | «Esporta report sensori», con e senza servizio | file salvato; contiene tutti i dispositivi; nessun hash, GUID o alias originale (controllo con uno script) |
| U6 | pulsanti delle licenze | si aprono i due file installati con l'app |
| U7 | budget di `docs/perf-budget.md` | rispettato, misurato con `scripts/measure-footprint.ps1` con la casella automatica attiva |
| U8 | installer 0.4.0 costruito in locale | contiene `THIRD_PARTY_LICENSES.txt`; si ispeziona l'archivio, non si esegue l'installer su questo PC |

## 9. Punti che il piano deve fissare

- l'ordine dei task: WinHTTP e logica pura degli aggiornamenti; shell, stato e toast; interfaccia; report (proprietà `adapterType`, builder, comando, interfaccia); licenze (strumenti, file, CI, installer); documentazione; verifiche dal vivo; bump della versione;
- dove la shell legge le statistiche e la qualità per il report senza tenere i lock durante il dialogo di salvataggio;
- come `NavigationTarget` porta la sezione delle impostazioni fino all'interfaccia;
- la versione di `cargo-about` e come la CI la installa;
- come lo script delle licenze ottiene il `.deps.json` in CI (pubblicazione del servizio già presente o restore dedicato).
