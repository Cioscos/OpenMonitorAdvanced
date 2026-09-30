# M6a — Release e firma del codice: design di dettaglio

- **Data:** 2026-09-30
- **Stato:** design del brainstorming del 2026-09-30, corretto dopo revisione tecnica nella stessa data (§9). Le verifiche di fattibilità del §3.4 sono un gate del piano, non fatti già verificati; la firma completa resta da collaudare dopo l'approvazione della Foundation.
- **Spec principale:** `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`. Per i punti trattati qui (firma dei binari, §9; punto aperto §13.7; release della §14 punto 6) questa spec ha la precedenza; tutto il resto resta come nella spec principale.

## 1. Intento e confini

La M6 ("Rifinitura e 1.0", spec principale §14 punto 6) si divide in tre piani in sequenza, ognuno con la propria spec o sezione, il proprio branch e la propria revisione finale:

- **M6a, release e firma** (questa spec);
- **M6b, spike sui dischi e protocollo v3** (fallback SAT `CHECK POWER MODE`, ri-identificazione all'hot-plug, flag `smartGateClosed`);
- **M6c, report, aggiornamenti e documentazione** ("Esporta report sensori", controllo degli aggiornamenti, licenze, documentazione, release 1.0).

La M6a fa due cose:

- una **pipeline di release** su GitHub Actions: il push di un tag `vX.Y.Z` costruisce l'installer in CI e ne fa una **bozza** di release su GitHub, che l'utente completa con le note e pubblica a mano;
- la **firma Authenticode** con il certificato gratuito della **SignPath Foundation**, per ridurre gli avvisi di SmartScreen e rendere riconoscibile l'editore.

Oggi le release (0.1.0, 0.2.0) si costruiscono in locale e si caricano a mano, senza firma; il job `installer` di `ci.yml` costruisce già l'installer su `windows-latest` ma lo lascia come artifact.

**Obiettivo:** a SignPath approvato, ogni release contiene un setup firmato, con dentro `oma-app.exe`, `oma-service.exe` e `uninstall.exe` firmati, un `SHA256SUMS.txt` e un'attestazione di provenienza. Prima dell'approvazione la stessa pipeline produce le release senza firma.

**Fuori dalla M6a:** pubblicazione su winget (dopo la 1.0), l'updater e il controllo degli aggiornamenti (M6c), l'installer MSI.

### 1.1 Decisioni del brainstorming

| # | Decisione |
|---|---|
| D1 | M6 in tre piani: M6a release e firma, M6b spike dischi e protocollo v3, M6c report, aggiornamenti e documentazione. Si parte dalla M6a. |
| D2 | Firma con **SignPath Foundation** (gratuita per progetti open source). Azure Artifact Signing Public Trust è escluso: ai privati è riservato a USA e Canada ([requisiti Microsoft](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)). Un certificato OV commerciale è escluso per costo e assenza di garanzia sulla reputazione iniziale. |
| D3 | Automazione di livello A: tag, poi build, poi firma, poi **bozza** di release; note e pubblicazione restano manuali. |
| D4 | Si firmano setup, `oma-app.exe`, `oma-service.exe` e **anche l'uninstaller** (scelta B), così la conferma UAC della disinstallazione non mostra "Editore sconosciuto". |
| D5 | Non si firmano file di terzi: `PawnIO_setup.exe` resta con la firma del suo autore, `nsExec.dll` e gli altri plugin NSIS restano senza firma (le condizioni di SignPath vietano di firmare codice altrui). |
| D6 | Extra inclusi: controllo della versione nel workflow, script di bump, `SHA256SUMS.txt`, attestazione di provenienza di GitHub. Winget escluso. |
| D7 | Approccio 1: **un solo job sequenziale** in `release.yml`. Scartati più job collegati da artifact (lo stato di `target/` andrebbe ricostruito in ogni job) e la firma per file tramite `signCommand` verso SignPath (un'approvazione per file e nessuna verifica d'origine). |
| D8 | Prima dell'attivazione la pipeline funziona anche **senza** SignPath: senza credenziali la bozza contiene l'installer non firmato. Dopo il collaudo `REQUIRE_SIGNING=true` impedisce downgrade accidentali (§4.2). |
| D9 | Firma tramite i ganci `signCommand` di Tauri con uno **shim** a due passate (§3), subordinato allo spike. Il template riceve almeno il controllo `= 0` su `!uninstfinalize`, con riga marcata `; OMA`; la pre-generazione dell'uninstaller resta un fallback da validare, non una soluzione già dimostrata. |

## 2. SignPath in breve

Per chi legge senza conoscere SignPath (la guida operativa completa è `docs/release.md`, §7):

1. **Domanda alla Foundation** (`https://signpath.org/apply`). Requisiti (`https://signpath.org/terms`): licenza OSI senza doppia licenza commerciale (GPL-3.0-or-later), nessun componente proprietario, progetto mantenuto e già rilasciato, funzioni descritte nella pagina di download.
2. **Configurazione su SignPath.io** dopo l'approvazione: GitHub App di SignPath sul repository, trusted build system GitHub.com, un *progetto* con due *signing policy* (`test-signing` con certificato di prova, `release-signing` con il certificato della Foundation) e le *artifact configuration* che dicono quali file firmare dentro lo zip inviato.
3. **A ogni firma:** il workflow carica i file con `actions/upload-artifact` e l'azione `signpath/github-action-submit-signing-request` (v3 o successiva) crea la richiesta; l'approvatore la approva a mano su SignPath; l'azione scarica i file firmati. SignPath verifica l'origine: repository, commit, workflow e runner ospitato da GitHub.
4. **Obblighi:** pagina "Code signing policy" con i ruoli, frase "Free code signing provided by SignPath.io, certificate by SignPath Foundation", dichiarazione sulla privacy, autenticazione a due fattori su GitHub e SignPath per tutti i ruoli, nome prodotto e versione nei metadati dei file firmati, approvazione manuale di ogni release.

L'editore mostrato da Windows è "SignPath Foundation". La firma non garantisce l'assenza dell'avviso di SmartScreen, neppure con un certificato EV: Windows considera sia la reputazione del certificato sia quella dell'hash del file. Non si assume una reputazione preesistente del certificato effettivamente assegnato al progetto. Fonte: [Microsoft, reputazione SmartScreen](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

L'ammissione alla Foundation è discrezionale. La domanda deve dichiarare il servizio self-contained con dipendenze .NET/LHM, PawnIO e l'installer/uninstaller generato da NSIS: l'ammissibilità di questo packaging va confermata, non dedotta dal solo fatto che i componenti siano open source ([condizioni Foundation](https://signpath.org/terms)). Finché non c'è approvazione, le attribuzioni pubbliche devono presentare la firma come prevista, senza affermare che sia già fornita.

## 3. Meccanismo di firma

### 3.1 Il vincolo: Tauri modifica l'exe durante il bundle

Nel bundler di tauri-cli 2.11.5 (`crates/tauri-bundler/src/bundle.rs`), per ogni tipo di pacchetto:

1. copia `oma-app.exe` in un file temporaneo;
2. lo **modifica** con `patch_binary`, che scrive nell'exe il tipo di pacchetto (`nsis`);
3. se la firma è configurata (`settings.windows().can_sign()`), lo firma;
4. costruisce l'installer NSIS: con la firma configurata, il template esegue `!uninstfinalize` con lo stesso comando sull'uninstaller e alla fine Tauri firma il setup;
5. ripristina l'exe originale.

Inoltre il bundler NSIS chiama `signCommand` su cinque plugin: `NSISdl.dll`, `StartMenu.dll`, `System.dll`, `nsDialogs.dll`, `additional/nsis_tauri_utils.dll`. Queste chiamate non vanno interpretate come uninstaller e non devono inviare codice di terzi a SignPath. Fonte: [bundler NSIS di Tauri 2.11.5](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.5/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs).

Un exe firmato **prima** del bundle perderebbe la firma al passo 2. Bisogna quindi firmare la versione già modificata, cioè quella che Tauri passa al comando di firma.

### 3.2 Lo shim `scripts/sign-shim.ps1`

In CI, e solo lì, `bundle.windows.signCommand` punta allo shim, passato con `--config`; `pnpm tauri build` in locale resta com'è. Lo shim non firma nulla: raccoglie o sostituisce file.

```
pwsh -NoProfile -File <percorso-assoluto>/scripts/sign-shim.ps1 -Mode collect|apply -Path "%1"
```

Il config temporaneo usa la forma strutturata di `signCommand` (`cmd` e `args`, con `%1` come argomento a sé). Script e radice dello stato hanno percorsi assoluti: Tauri viene eseguito da `app/`, mentre `!uninstfinalize` può usare un'altra directory corrente. Quoting, spazi e apici nel percorso si verificano nello spike; il comando mostrato sopra è illustrativo. Fonte: [implementazione del comando di firma Tauri](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.5/crates/tauri-bundler/src/bundle/windows/sign.rs).

Stato in `target/signing/`, ricreato vuoto all'inizio di ciascun run e mai recuperato dalla cache:

- `unsigned/`: le copie raccolte;
- `signed/`: i file firmati restituiti da SignPath;
- `manifest.json`: commit, versione, run/attempt e, per ogni file, ruolo (`app`, `uninstaller`, `setup`, `service`), nome canonico e SHA-256 della versione non firmata. Il manifest raccolto non viene sovrascritto in `apply`; gli esiti delle due passate sono registrati separatamente.

Riconoscimento del ruolo dal percorso normalizzato ricevuto:

- `app`: `oma-app.exe` nel percorso di build atteso;
- `setup`: il percorso di output atteso per versione/architettura, non solo un suffisso;
- plugin di terzi: solo l'elenco del §3.1, nelle copie dei plugin della toolchain Tauri attesa; esito positivo senza modifica né inclusione nell'artifact di firma, con hash prima/dopo e log esplicito;
- `uninstaller`: solo il PE temporaneo `.exe` identificato dallo spike, accettato **una sola volta** per passata; il nome canonico nell'artifact è `uninstall.exe`.

Qualunque percorso non previsto o ruolo duplicato fa fallire lo shim: nessun generico "altro file = uninstaller". Lo spike definisce il riconoscimento del temporaneo NSIS e dei plugin; una nuova versione di Tauri richiede di riconfermare questo contratto.

**Modalità `collect`** (prima passata, `tauri build --bundles nsis`): copia il file in `unsigned/`, registra ruolo e SHA-256, esce con 0 senza modificarlo. Il setup prodotto da questa passata è quello **non firmato**.

**Modalità `apply`** (seconda passata, `tauri bundle --bundles nsis`):

- `app` e `uninstaller`: calcola lo SHA-256 del file ricevuto e lo confronta con quello registrato in `collect`. Se diverso fallisce ("il file da firmare non è quello firmato"); se uguale sovrascrive il file con la copia firmata di `signed/`, e fallisce se questa manca;
- `setup`: lo registra e lo lascia com'è; la firma arriva dopo (§3.3, passo 6).

`oma-service.exe` non passa da Tauri: lo include `oma.nsh` con `File` da `target/installer-payload/service/`. Nessuno lo modifica, quindi si registra nel manifest con uno step apposito (`sign-shim.ps1 -Mode collect -Role service -Path …`) e, dopo la firma, la copia firmata sostituisce quella del payload prima della seconda passata.

**Gate di ogni passata:** `!uninstfinalize '${UNINSTALLERSIGNCOMMAND}' = 0` deve interrompere makensis se lo shim fallisce; oggi il template non contiene il confronto. Dopo il bundle uno step richiede esattamente una chiamata riuscita per `app`, `uninstaller` e `setup`, più la registrazione esplicita del `service` in `collect`. Zero chiamate all'uninstaller è un errore anche se makensis esce con 0. Tutti gli eseguibili esterni hanno il codice d'uscita controllato esplicitamente. Fonte: [manuale NSIS, comandi di finalizzazione](https://nsis.sourceforge.io/Docs/Chapter5.html).

### 3.3 Sequenza completa

1. `tauri build --bundles nsis` con lo shim in `collect`: raccoglie `oma-app.exe` (modificato) e l'uninstaller; si aggiunge `oma-service.exe`.
2. **Firma n. 1:** `upload-artifact` dei tre file (`oma-app.exe`, `uninstall.exe`, `oma-service.exe`), poi richiesta SignPath con l'artifact configuration `binaries`, poi i file firmati in `target/signing/signed/`.
3. Il `oma-service.exe` firmato sostituisce quello in `target/installer-payload/service/`.
4. `tauri bundle --bundles nsis` con lo shim in `apply`: stesso exe modificato e stesso uninstaller (controllati con lo SHA-256), sostituiti dalle versioni firmate prima di finire nel setup.
5. Il setup della seconda passata va in `target/signing/setup/`.
6. **Firma n. 2:** richiesta SignPath con l'artifact configuration `setup`.

Gli upload hanno nomi distinti comprendenti `run_id` e `run_attempt`, `if-no-files-found: error` e un layout piatto esplicito. Ogni richiesta usa l'`artifact-id` del proprio upload, gli slug di progetto/policy/configurazione espliciti, `wait-for-completion: true` e una directory di ritorno vuota. La configurazione usa una radice `<zip-file>` e solo i nomi attesi, senza wildcard per altri PE. I tre file restituiti vengono verificati con la policy selezionata **prima** della sostituzione e del secondo bundle; si memorizzano anche i loro hash firmati. Nessun file extra o mancante è accettato. Il setup restituito dalla seconda firma è copiato in una directory finale separata: checksum, attestazione e upload non usano glob che possano prendere il setup non firmato.

Senza SignPath (D8) si fanno solo il passo 1 e i passi del §5; il setup finale è quello della prima passata.

### 3.4 Verifiche di fattibilità (primo task del piano, spike)

1. **Determinismo:** l'`oma-app.exe` modificato e l'uninstaller passati allo shim sono identici byte per byte tra `tauri build` e il successivo `tauri bundle`, senza ricompilare Rust/.NET, con gli stessi input salvo le firme? La prova deve includere la sostituzione del servizio e dell'app con file di dimensioni diverse e le modalità `collect`/`apply` realmente previste. Cambiare il payload può incidere sull'uninstaller NSIS: non basta confrontare due bundle entrambi non firmati. Ripetere con un intervallo temporale e directory temporanee differenti. Se il confronto fallisce, si valida la pre-generazione nel template (uninstaller firmato incluso con `File`), con righe marcate `; OMA`; si aggiorna questa spec con il meccanismo dimostrato prima di proseguire. Non si disabilita il confronto SHA-256.
2. **Sostituzione dell'uninstaller in `!uninstfinalize`:** verificare che makensis includa la versione sostituita. Nello spike locale si controlla la marcatura/hash della copia installata; con SignPath si controlla la firma reale di `uninstall.exe` in `$INSTDIR` (§8.2). L'installazione di prova si esegue in una VM/ambiente isolato, senza modificare driver e servizio sul PC di lavoro.
3. **Percorso passato allo shim** per l'uninstaller, e quoting di `%1` dentro `!uninstfinalize` (il comando finisce nel template NSIS tra apici singoli).
4. **7-Zip vede i file firmati dentro il setup?** Serve per il controllo automatico del §5.1. Se l'uninstaller non è visibile, il suo controllo passa tra le verifiche manuali.
5. **Metadati:** `oma-app.exe` ha nome prodotto "OpenMonitor Advanced" e la versione (da `tauri.conf.json`). `oma-service.exe` oggi non imposta `Product` e ricade sul nome dell'assembly: il piano aggiunge `Product` ("OpenMonitor Advanced") al progetto del servizio. Si controlla anche la risorsa di versione dell'uninstaller.

6. **Fallimenti e plugin:** registrare tutte le chiamate allo shim, verificare che i plugin restino byte per byte intatti e che un errore intenzionale dell'uninstaller interrompa la compilazione. Confermare inoltre che il secondo bundle non ricompili o ripubblichi il servizio, cancellando la copia firmata. I file finti dello spike usano un harness locale con verificatore iniettato; nessun bypass dei controlli di firma è disponibile nel workflow di release.

L'esito dello spike si scrive nel piano e si allinea questa spec prima dei task successivi che dipendono dalla firma. Se lo spike resta inconcludente, si può completare il percorso non firmato ma non dichiarare realizzato quello firmato.

## 4. Workflow `.github/workflows/release.yml`

### 4.1 Avvio

- **Push di un tag `v*.*.*`:** release vera, signing policy `release-signing`.
- **`workflow_dispatch`** (solo da `main`): prova generale con la policy `test-signing`, che usa un certificato di prova non riconosciuto da Windows. Carica il setup come artifact del run e **non** crea nessuna release.

Il filtro del tag è un glob, non valida SemVer: il preflight richiede `^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$`. Il job ha un guard su evento e tipo di ref: dispatch solo con `refs/heads/main`, push solo su tag (la validazione numerica avviene nello script di preflight); checkout del commit del run. Una concurrency per workflow/ref con `cancel-in-progress: false` evita scritture concorrenti sulla stessa bozza.

### 4.2 Job

Un solo job `release` su `windows-latest`, `timeout-minutes: 240`, nell'environment `release`. Step in ordine:

1. checkout con la storia completa (`fetch-depth: 0`, serve per il controllo su `origin/main`);
2. `scripts/check-version.ps1` (con `-Tag` solo per i tag; §6.1);
3. composite action `.github/actions/setup-toolchain` (§4.4);
4. `pnpm install --frozen-lockfile`, poi `scripts/build-installer-payload.ps1`;
5. §3.3 passo 1 (con `--config` che imposta `signCommand` sullo shim in `collect`);
6. se la firma è attiva: §3.3 passi 2-6;
7. §5: verifica delle firme, `SHA256SUMS.txt`, attestazione;
8. per un tag: bozza di release (§5.3); per `workflow_dispatch`: `upload-artifact` del setup e di `SHA256SUMS.txt`.

Prima di compilare si controllano configurazione della firma e, per i tag, eventuale release già pubblicata. Le build Rust usano `--locked`. Il preflight di release richiede anche un run CI completato con successo sullo stesso SHA (job checks, service, installer, scripts e actionlint): non basta che il commit sia un antenato di `main`. Se assente/in corso/fallito, il run si ferma prima delle richieste di firma e si può rieseguire dopo il verde.

**Firma attiva:** la variabile `SIGNPATH_ORGANIZATION_ID` e il secret `SIGNPATH_API_TOKEN` esistono entrambi. Prima dell'attivazione, se entrambi sono assenti gli step si saltano e il log lo dice con un `::notice::`; con un solo dei due il workflow fallisce. Dopo il collaudo reale si imposta nell'environment `REQUIRE_SIGNING=true`: credenziali assenti diventano un errore anche per il dispatch, evitando un ritorno accidentale a release non firmate. Il secret non si usa direttamente in `if:`: uno step calcola l'output booleano senza stampare il valore. Un errore SignPath non ripiega mai sul percorso non firmato.

**Attese:** ogni richiesta usa `wait-for-completion-timeout-in-seconds: 5400` (90 minuti). Le due attese lasciano un'ora delle 4 disponibili per build, bundle e verifiche: il collaudo CI deve misurare che basti. Se un'attesa scade, il job fallisce; la richiesta può restare pendente su SignPath, quindi si annulla quella precedente prima di rilanciare "Re-run all jobs". Il limite documentato di 3 riesecuzioni riguarda la valutazione delle pipeline policy; la policy effettiva deve consentire i rerun. Si registrano URL/ID delle richieste nel riepilogo del run.

### 4.3 Sicurezza

- **Environment `release`:** contiene il secret `SIGNPATH_API_TOKEN` e le variabili `SIGNPATH_ORGANIZATION_ID` e `REQUIRE_SIGNING`. Le deployment rule ammettono separatamente tag `v*` e branch `main`. Il token appartiene a un utente con soli diritti di submitter sulle policy necessarie; l'approvazione resta al proprietario.
- **Ruleset sui tag `v*`:** solo il proprietario del repository può crearli, aggiornarli o cancellarli.
- **Permessi del job:** `actions: read` (SignPath e verifica CI), `contents: write` (release), `id-token: write` e `attestations: write` (attestazione); a livello di workflow `contents: read`. SignPath richiede anche accesso in lettura agli artifact/job ([integrazione GitHub](https://docs.signpath.io/trusted-build-systems/github)).
- **Azioni esterne** nel workflow di release e nella composite action fissate allo SHA completo, con commento della versione leggibile; il solo tag di major non è immutabile. Checkout con `persist-credentials: false`, token GitHub esposto agli step che ne hanno bisogno e token SignPath solo alle richieste di firma. Nomi ref e versioni passano tramite variabili/argomenti, mai interpolati come codice shell.
- Nessuno step esegue codice di una PR: il workflow parte solo da tag o da `main`.

Questi vincoli richiedono protezione anche di `main` e revisione dei cambiamenti ai workflow/script di build. Il solo environment non garantisce che un commit puntato da un tag sia affidabile. La policy SignPath deve vincolare repository, workflow e ref attesi, oltre al runner ospitato da GitHub.

### 4.4 Composite action `.github/actions/setup-toolchain`

Raccoglie la preparazione oggi duplicata nei job di `ci.yml`:

- `rustup show` (toolchain da `rust-toolchain.toml`) e `Swatinem/rust-cache`;
- pnpm 10 e Node 22 con cache;
- .NET da `global.json`.

La usano il job `installer` di `ci.yml` e il job `release`, così le due build non divergono. Il job `checks` di `ci.yml` la usa se ha bisogno degli stessi strumenti.

## 5. Verifica, checksum, attestazione e bozza

Tutto si calcola sul **setup finale** (dopo la firma n. 2, o dopo la prima passata senza firma): sono i byte che l'utente scarica.

### 5.1 `scripts/verify-signatures.ps1`

```
pwsh scripts/verify-signatures.ps1 -Setup <percorso> -Policy release|test|none
```

- **`release`:** firma incorporata valida sul setup e sui binari propri, identità del certificato approvato per il progetto (CN esatto `SignPath Foundation` e certificato consentito, senza semplice ricerca di sottostringa), timestamp valido e metadati prodotto/versione attesi. La configurazione del certificato deve consentire un rinnovo esplicito. Estrae il setup con 7-Zip, controllandone l'exit code, e richiede esattamente i file attesi: `oma-app.exe`, `oma-service.exe` e, se visibile nello spike, `uninstall.exe`. Gli hash dei file estratti devono coincidere con quelli firmati del §3.3; la firma da sola non identifica il payload di questo run. `PawnIO_setup.exe` mantiene hash e firmatario già fissati dal progetto.
- **`test`:** stessi controlli di integrità, identità del certificato di prova atteso, metadati e hash. Il solo certificato presente non basta: `HashMismatch`, `NotSigned`, `UnknownError` e gli altri errori falliscono; la sola catena non attendibile del certificato di prova è ammessa. Lo spike valida il metodo (eventuale verifica con trust temporaneo nel runner) senza indebolire la policy `release`. Fonte: [stati della firma PowerShell](https://learn.microsoft.com/en-us/dotnet/api/system.management.automation.signaturestatus).
- **`none`:** setup non vuoto, estrazione riuscita, presenza dei payload propri e integrità/firma originale di PawnIO; scrive l'avviso "unsigned build". Anche il percorso non firmato deve verificare il contenuto dell'installer.

Se 7-Zip non espone l'uninstaller, si verifica comunque la copia firmata restituita da SignPath e l'esito `apply`, ma il riepilogo dichiara che la firma dell'uninstaller **installato** resta da verificare. Il collaudo manuale del §8.2 è obbligatorio prima della prima pubblicazione firmata e dopo modifiche al meccanismo NSIS.

L'accesso alle firme passa per un parametro iniettabile, così i test Pester usano un finto.

### 5.2 Checksum e attestazione

- **`SHA256SUMS.txt`** nel formato di `sha256sum` (`<hash minuscolo>  <nome file>`), una riga per il setup.
- **Attestazione** con `actions/attest` sul solo setup finale: per nuove integrazioni è l'azione raccomandata da [GitHub](https://github.com/actions/attest-build-provenance). Si verifica in CI sullo stesso file prima di creare/aggiornare la bozza. Verifica per chiunque:

  ```
  gh attestation verify OpenMonitor.Advanced_X.Y.Z_x64-setup.exe --repo Cioscos/OpenMonitorAdvanced
  ```

### 5.3 Bozza di release

Solo per i tag:

- **titolo** `OpenMonitor Advanced X.Y.Z`; **asset**: il setup e `SHA256SUMS.txt`;
- **testo** da `.github/release-notes-template.md`, riempito da `scripts/render-release-notes.ps1` con:
  - il segnaposto `<!-- Write the changes here -->`, per le novità scritte a mano;
  - la sezione "Install": avvio, `/S`, `/NOSENSORS`, e poi, secondo lo stato della firma, **o** la riga "not code-signed yet, SmartScreen may warn you (*More info* → *Run anyway*)" **o** l'attribuzione "Free code signing provided by SignPath.io, certificate by SignPath Foundation";
  - la sezione "Verify your download" con `Get-FileHash -Algorithm SHA256` e `gh attestation verify`.
- **Creazione:** `gh release create vX.Y.Z --draft --verify-tag --title … --notes-file … <asset>`.
- **Riesecuzioni:** se esiste già una **bozza** per il tag, il testo contiene marcatori per separare novità manuali e blocco tecnico generato (firma, installazione, verifica). Si preservano le novità e si aggiorna il solo blocco tecnico, così non resta l'avviso "non firmato" con un asset firmato o viceversa. Marcatori assenti/ambigui fanno fallire lo step prima di modificare gli asset. Setup e checksum sono sostituiti con `gh release upload --clobber` solo dopo tutti i gate; si verifica la coppia di asset remoti a fine upload. L'operazione non è atomica: in caso di errore la bozza non va pubblicata e il run si riesegue. Si ricontrolla `isDraft` immediatamente prima delle scritture; la guida vieta la pubblicazione manuale durante il run. Se la release è già **pubblicata**, lo step fallisce senza toccarla.

Il testo del template è in inglese, come le release esistenti.

## 6. Versione

### 6.1 `scripts/check-version.ps1 [-Tag vX.Y.Z]`

La versione vive in cinque file:

- `Cargo.toml` (`[workspace.package] version`);
- `app/package.json`;
- `app/src-tauri/tauri.conf.json`;
- la riga "Status" di `README.md` ("version X.Y.Z");
- la riga "Stato" di `README.it.md` ("versione X.Y.Z").

Il servizio la riceve da `tauri.conf.json` in `build-installer-payload.ps1`.

Lo script fallisce, elencando **tutte** le incongruenze e non solo la prima, se:

- i cinque file non hanno la stessa versione, o un file non contiene il campo atteso;
- `Cargo.lock` non è allineato (`cargo metadata --locked --format-version 1 --no-deps` fallisce);
- con `-Tag`: il tag non è `v` più la versione, oppure il commit del tag non è raggiungibile da `origin/main` (`git merge-base --is-ancestor`).

Le versioni usano tre componenti canoniche senza zeri iniziali, ciascuna tra 0 e 65535; la versione PE normalizzata è `X.Y.Z.0`. La lettura dei README distingue `Status/version` da `Stato/versione`. In CI si aggiorna esplicitamente `origin/main`, si risolve il tag con `^{commit}` (anche annotato) e si richiede che coincida con `HEAD`/SHA del run. `cargo metadata` si esegue dalla radice del workspace. La verifica d'ascendenza non sostituisce il gate CI del §4.2.

### 6.2 `scripts/bump-version.ps1 <X.Y.Z>`

- accetta solo `X.Y.Z` secondo i limiti del §6.1: niente pre-release in questa milestone (scelta di scope, non impossibilità di rappresentare SemVer nei metadati Windows);
- rifiuta una versione minore o uguale a quella attuale;
- aggiorna i cinque file (solo la riga o il campo della versione, conservando fine riga LF e formattazione) e `Cargo.lock` (`cargo update --workspace --offline`);
- esegue `check-version.ps1` come controllo finale;
- prima di scrivere verifica tutti i campi attesi e l'allineamento corrente; prepara backup dei cinque file e di `Cargo.lock` e li ripristina byte per byte se l'aggiornamento Cargo o il controllo finale falliscono. Il confronto delle versioni è numerico, non lessicografico; nessun aggiornamento di dipendenze estraneo al bump è accettato;
- **non** crea commit e **non** fa tag: stampa i comandi successivi (`git commit -am "chore: release X.Y.Z"`, `git tag vX.Y.Z`, `git push origin main vX.Y.Z`). Il push resta una decisione dell'utente.

### 6.3 Flusso di una release

1. `pwsh scripts/bump-version.ps1 0.3.0`, poi commit, tag e push.
2. Il workflow costruisce e, con la firma attiva, chiede due approvazioni su SignPath (arrivano per email).
3. Si rivede la bozza, si scrivono le novità e si pubblica.

## 7. Documentazione

- **`CODE_SIGNING.md`** (inglese, linkato dai due README), sul modello richiesto da SignPath:
  - ruoli: *Committers and reviewers* e *Approvers*, entrambi il proprietario del repository (@Cioscos);
  - la frase d'attribuzione dopo l'attivazione; prima si dichiara solo lo stato della domanda;
  - cosa si firma e cosa no (D4, D5);
  - la dichiarazione sulla privacy, verificata anche per componenti di terzi e bootstrapper WebView2 dell'installer, con link alle policy pertinenti. In M6c il controllo degli aggiornamenti introduce una chiamata a GitHub: si rivedono informativa, comportamento e opzioni di disattivazione richieste dalle condizioni Foundation (voce nei follow-up).
- **`docs/release.md`** (italiano), guida passo passo per chi non ha mai firmato codice:
  1. la domanda alla Foundation e cosa scrivere nel modulo;
  2. l'autenticazione a due fattori su GitHub e su SignPath;
  3. GitHub App e trusted build system;
  4. progetto, policy `test-signing` e `release-signing`, artifact configuration (copiate da `.signpath/`);
  5. environment `release`, credenziali, `REQUIRE_SIGNING`, protezioni di `main`/tag e configurazione del certificato atteso;
  6. il flusso di una release (§6.3) e la prova con `workflow_dispatch`;
  7. cosa fare se un'approvazione scade, se lo shim segnala uno SHA-256 diverso o se la verifica delle firme fallisce.
- **`.signpath/artifact-configuration-binaries.xml`** e **`.signpath/artifact-configuration-setup.xml`:** copie versionate delle artifact configuration con radice zip e allowlist del §3.3. La configurazione attiva sta nell'interfaccia SignPath: la guida richiede confronto con le copie del commit prima dell'attivazione e dopo ogni modifica. Ogni `pe-file` impone Authenticode, timestamp, nome prodotto "OpenMonitor Advanced" e versione normalizzata; lo spike rileva i valori effettivi di `ProductVersion`/`FileVersion` (in particolare `X.Y.Z` rispetto a `X.Y.Z.0`) e definisce i parametri appropriati, senza presumere formati identici tra Rust, .NET e NSIS.
- **README (en, it):**
  - sezione "Verify your download" (checksum e attestazione);
  - link a `CODE_SIGNING.md`;
  - la riga sul SmartScreen si aggiorna quando la firma è attiva (non prima: finché la Foundation non approva resta com'è).
- **`CLAUDE.md`:** comandi di bump e di verifica della versione, e il flusso di release.
- **Spec principale:** §9 e §13.7 rimandano a questa spec come decisione progettuale; ammissione e collaudo della firma restano pendenti fino alla verifica reale.
- **`docs/follow-ups.md`:**
  - la voce su `nsExec.dll` si chiude come "codice di terzi, non firmabile da noi" (D5);
  - si aggiunge la voce sulla dichiarazione della privacy per M6c.

## 8. Test e collaudo

### 8.1 Test automatici

**Pester 5** per gli script nuovi, in `scripts/tests/*.Tests.ps1`, con file finti al posto dei binari e un fornitore di firme iniettato:

- `sign-shim.ps1`:
  - `collect` registra ruolo e SHA-256;
  - `apply` sostituisce con la copia firmata;
  - `apply` fallisce per uno SHA-256 diverso, per un file firmato mancante e per un secondo file non riconosciuto;
  - il `setup` in `apply` resta intatto;
  - il ruolo `service`;
  - allowlist dei plugin senza modifica, file inatteso/duplicato, manifest incompleto o di un'altra versione/run, directory corrente diversa e percorsi con spazi;
- `check-version.ps1`:
  - tutto allineato;
  - un file alla volta con versione diversa;
  - campo mancante;
  - tag sbagliato;
  - elenco completo delle incongruenze;
  - README italiano, tag annotato, tag non numerico, commit diverso da HEAD e ascendenza negativa;
- `bump-version.ps1`, su una copia temporanea dei cinque file:
  - aggiornamento corretto con fine riga LF;
  - rifiuto di downgrade, stessa versione e formato non valido;
  - limiti PE, zeri iniziali, confronto numerico e rollback anche di `Cargo.lock` su errore;
- `verify-signatures.ps1`: le tre policy con firme finte (valide, di prova, assenti, firmatario sbagliato, hash corrotto, timestamp mancante), payload mancante/duplicato, hash non corrispondente ed estrattore fallito;
- `render-release-notes.ps1`: variante firmata/non firmata, cambio stato preservando le novità manuali e rifiuto di marcatori ambigui;
- helper del workflow: configurazione assente/parziale/obbligatoria, ref dispatch vietato, gate CI e release già pubblicata, errore di upload senza pubblicazione.

I comandi `cargo` e `git` sono parametri iniettabili dove servono, come `DotnetExe` in `build-installer-payload.ps1`.

**CI:**

- nuovo job `scripts` in `ci.yml`, su `windows-latest`, shell `pwsh`: importa esplicitamente una versione Pester 5 fissata e verificata (la installa se assente), poi `Invoke-Pester -Path scripts/tests -CI`, perché senza `-CI` i test falliti possono lasciare exit code 0 ([documentazione Pester](https://pester.dev/docs/commands/Invoke-Pester#-ci));
- job `actionlint` su `ubuntu-latest`, versione fissata, per `ci.yml` e `release.yml`. Legge i metadati della composite action referenziata, ma non ne verifica gli step interni: questi richiedono controllo dello schema e collaudo attraverso i workflow che la usano ([limiti documentati di actionlint](https://github.com/rhysd/actionlint/blob/main/docs/checks.md#action-metadata-syntax)).

In locale serve Pester 5 (`Install-Module Pester -Scope CurrentUser -MinimumVersion 5.0`): Windows include solo la 3.4.

### 8.2 Collaudo della milestone

1. Lo spike del §3.4, in locale, con lo shim in `collect` e `apply` senza firme reali. Copie con byte di marcatura e dimensione variata provano sostituzione/confronto degli SHA-256 e l'effetto dei payload diversi; non provano Authenticode, timestamp, compatibilità SignPath o correttezza del setup installato. La prova negativa del comando uninstaller è obbligatoria.
2. `workflow_dispatch` senza secret: percorso non firmato fino all'artifact, con `verify-signatures.ps1 -Policy none`.
3. Prima release reale non firmata (per esempio la 0.3.0) con il nuovo flusso: bump, tag, bozza, pubblicazione a mano.

**Verifiche manuali dovute dopo l'approvazione della Foundation** (in `docs/follow-ups.md`):

- `workflow_dispatch` con `test-signing`;
- una release con `release-signing`;
- l'installazione del setup firmato: firme di `oma-app.exe`, `oma-service.exe` e `uninstall.exe` in `$INSTDIR`, editore nella conferma UAC d'installazione e di disinstallazione;
- il comportamento di SmartScreen sul download.

La M6a può chiudersi con il percorso non firmato collaudato e la firma documentata come condizionata; non con una dichiarazione di firma pronta senza queste prove. Nessuna prima release firmata si pubblica prima del controllo dell'uninstaller installato.

## 9. Esito della revisione tecnica (2026-09-30)

Correzioni integrate rispetto al brainstorming:

| Criticità | Correzione / gate |
|---|---|
| Lo shim scambiava i plugin che Tauri firma per uninstaller; la build si sarebbe fermata. | Allowlist esplicita e plugin lasciati intatti (§3.1–3.2). |
| `!uninstfinalize` non imponeva exit code 0; il fallimento poteva essere ignorato. | Modifica minima al template e controllo completo delle chiamate (§3.2). |
| Determinismo dedotto senza cambiare i payload come nella vera seconda passata. | Spike con sostituzioni e tempi/percorso variabili; fallback da dimostrare (§3.4). |
| Firma presente accettata anche se corrotta; un firmatario generico non identifica il payload. | Policy distinte, identità attesa, timestamp e hash dei file incorporati (§5.1). |
| Rerun e configurazione potevano produrre downgrade non firmati o note incoerenti. | `REQUIRE_SIGNING`, concurrency e blocchi tecnici aggiornabili (§4–5). |
| Mancavano permessi Actions, gate CI sullo SHA e precisione sui ref. | Preflight e permessi espliciti (§4). |
| README italiano letto come inglese; bump parziale possibile. | Campo `Stato`, limiti PE e rollback dei sei file (§6). |
| Pester senza propagazione degli errori; copertura actionlint sovrastimata. | `-CI`, import fissato e limiti espliciti del lint (§8). |
| Reputazione SmartScreen e idoneità Foundation considerate garantite. | Fonti primarie, domanda trasparente e firma reale ancora da collaudare (§2, §8). |

Le fonti online sono citate accanto ai requisiti che supportano. Il confronto statico con i sorgenti non sostituisce lo spike né una prova con il certificato reale.
