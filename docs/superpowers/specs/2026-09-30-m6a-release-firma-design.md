# M6a — Release e firma del codice: design di dettaglio

- **Data:** 2026-09-30
- **Stato:** design approvato nel brainstorming del 2026-09-30; le verifiche di fattibilità del §3.4 sono il primo task del piano, non fatti già verificati.
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
| D2 | Firma con **SignPath Foundation** (gratuita per progetti open source). Azure Artifact Signing è escluso: ai privati è riservato a USA e Canada. Un certificato OV commerciale è escluso per costo e reputazione iniziale nulla. |
| D3 | Automazione di livello A: tag, poi build, poi firma, poi **bozza** di release; note e pubblicazione restano manuali. |
| D4 | Si firmano setup, `oma-app.exe`, `oma-service.exe` e **anche l'uninstaller** (scelta B), così la conferma UAC della disinstallazione non mostra "Editore sconosciuto". |
| D5 | Non si firmano file di terzi: `PawnIO_setup.exe` resta con la firma del suo autore, `nsExec.dll` e gli altri plugin NSIS restano senza firma (le condizioni di SignPath vietano di firmare codice altrui). |
| D6 | Extra inclusi: controllo della versione nel workflow, script di bump, `SHA256SUMS.txt`, attestazione di provenienza di GitHub. Winget escluso. |
| D7 | Approccio 1: **un solo job sequenziale** in `release.yml`. Scartati più job collegati da artifact (lo stato di `target/` andrebbe ricostruito in ogni job) e la firma per file tramite `signCommand` verso SignPath (un'approvazione per file e nessuna verifica d'origine). |
| D8 | La pipeline funziona anche **senza** SignPath: senza secret gli step di firma si saltano e la bozza esce con l'installer non firmato. |
| D9 | Firma tramite i ganci `signCommand` di Tauri con uno **shim** a due passate (§3), non con un uninstaller pre-generato nel template NSIS. Il template non si tocca. |

## 2. SignPath in breve

Per chi legge senza conoscere SignPath (la guida operativa completa è `docs/release.md`, §7):

1. **Domanda alla Foundation** (`https://signpath.org/apply`). Requisiti (`https://signpath.org/terms`): licenza OSI senza doppia licenza commerciale (GPL-3.0-or-later), nessun componente proprietario, progetto mantenuto e già rilasciato, funzioni descritte nella pagina di download.
2. **Configurazione su SignPath.io** dopo l'approvazione: GitHub App di SignPath sul repository, trusted build system GitHub.com, un *progetto* con due *signing policy* (`test-signing` con certificato di prova, `release-signing` con il certificato della Foundation) e le *artifact configuration* che dicono quali file firmare dentro lo zip inviato.
3. **A ogni firma:** il workflow carica i file con `actions/upload-artifact` e l'azione `signpath/github-action-submit-signing-request` (v3 o successiva) crea la richiesta; l'approvatore la approva a mano su SignPath; l'azione scarica i file firmati. SignPath verifica l'origine: repository, commit, workflow e runner ospitato da GitHub.
4. **Obblighi:** pagina "Code signing policy" con i ruoli, frase "Free code signing provided by SignPath.io, certificate by SignPath Foundation", dichiarazione sulla privacy, autenticazione a due fattori su GitHub e SignPath per tutti i ruoli, nome prodotto e versione nei metadati dei file firmati, approvazione manuale di ogni release.

L'editore mostrato da Windows è "SignPath Foundation". La firma non elimina subito l'avviso di SmartScreen: dal 2024 nemmeno i certificati EV danno reputazione immediata. Il certificato condiviso della Foundation, però, ha già una reputazione propria e la firma la accumula a nome dell'editore invece che del singolo file.

## 3. Meccanismo di firma

### 3.1 Il vincolo: Tauri modifica l'exe durante il bundle

Nel bundler di tauri-cli 2.11.5 (`crates/tauri-bundler/src/bundle.rs`), per ogni tipo di pacchetto:

1. copia `oma-app.exe` in un file temporaneo;
2. lo **modifica** con `patch_binary`, che scrive nell'exe il tipo di pacchetto (`nsis`);
3. se la firma è configurata (`settings.windows().can_sign()`), lo firma;
4. costruisce l'installer NSIS: con la firma configurata, il template esegue `!uninstfinalize` con lo stesso comando sull'uninstaller e alla fine Tauri firma il setup;
5. ripristina l'exe originale.

Un exe firmato **prima** del bundle perderebbe la firma al passo 2. Bisogna quindi firmare la versione già modificata, cioè quella che Tauri passa al comando di firma.

### 3.2 Lo shim `scripts/sign-shim.ps1`

In CI, e solo lì, `bundle.windows.signCommand` punta allo shim, passato con `--config`; `pnpm tauri build` in locale resta com'è. Lo shim non firma nulla: raccoglie o sostituisce file.

```
pwsh -NoProfile -File scripts/sign-shim.ps1 -Mode collect|apply -Path "%1"
```

Stato in `target/signing/`:

- `unsigned/`: le copie raccolte;
- `signed/`: i file firmati restituiti da SignPath;
- `manifest.json`: per ogni file il ruolo (`app`, `uninstaller`, `setup`, `service`), il nome e lo SHA-256 della versione non firmata.

Riconoscimento del ruolo dal percorso ricevuto:

- `app`: il file si chiama `oma-app.exe`;
- `setup`: il nome finisce con `_x64-setup.exe`;
- `uninstaller`: qualunque altro file, accettato **una sola volta** per passata.

Un secondo file non riconosciuto fa fallire lo shim: se Tauri inizia a firmare file nuovi (per esempio un sidecar), la release si ferma invece di proseguire in silenzio. Lo spike del §3.4 registra il percorso reale che `!uninstfinalize` passa per l'uninstaller e, se ha una forma stabile, il riconoscimento si restringe a quella.

**Modalità `collect`** (prima passata, `tauri build --bundles nsis`): copia il file in `unsigned/`, registra ruolo e SHA-256, esce con 0 senza modificarlo. Il setup prodotto da questa passata è quello **non firmato**.

**Modalità `apply`** (seconda passata, `tauri bundle --bundles nsis`):

- `app` e `uninstaller`: calcola lo SHA-256 del file ricevuto e lo confronta con quello registrato in `collect`. Se diverso fallisce ("il file da firmare non è quello firmato"); se uguale sovrascrive il file con la copia firmata di `signed/`, e fallisce se questa manca;
- `setup`: lo registra e lo lascia com'è; la firma arriva dopo (§3.3, passo 6).

`oma-service.exe` non passa da Tauri: lo include `oma.nsh` con `File` da `target/installer-payload/service/`. Nessuno lo modifica, quindi si registra nel manifest con uno step apposito (`sign-shim.ps1 -Mode collect -Role service -Path …`) e, dopo la firma, la copia firmata sostituisce quella del payload prima della seconda passata.

### 3.3 Sequenza completa

1. `tauri build --bundles nsis` con lo shim in `collect`: raccoglie `oma-app.exe` (modificato) e l'uninstaller; si aggiunge `oma-service.exe`.
2. **Firma n. 1:** `upload-artifact` dei tre file (`oma-app.exe`, `uninstall.exe`, `oma-service.exe`), poi richiesta SignPath con l'artifact configuration `binaries`, poi i file firmati in `target/signing/signed/`.
3. Il `oma-service.exe` firmato sostituisce quello in `target/installer-payload/service/`.
4. `tauri bundle --bundles nsis` con lo shim in `apply`: stesso exe modificato e stesso uninstaller (controllati con lo SHA-256), sostituiti dalle versioni firmate prima di finire nel setup.
5. Il setup della seconda passata va in `target/signing/setup/`.
6. **Firma n. 2:** richiesta SignPath con l'artifact configuration `setup`.

Senza SignPath (D8) si fanno solo il passo 1 e i passi del §5; il setup finale è quello della prima passata.

### 3.4 Verifiche di fattibilità (primo task del piano, spike)

1. **Determinismo:** l'`oma-app.exe` modificato e l'uninstaller passati allo shim sono identici byte per byte tra `tauri build` e il successivo `tauri bundle` sullo stesso commit? Per l'exe è atteso (stesso binario, stessa modifica). Per l'uninstaller di NSIS è probabile ma non garantito: se contiene dati che cambiano a ogni compilazione (orari, nomi temporanei), il meccanismo del §3.2 non regge per l'uninstaller e si torna alla pre-generazione nel template (una compilazione che scrive solo l'uninstaller, poi firmato e incluso con `File` nella compilazione vera), con righe marcate `; OMA` in `installer.nsi`.
2. **Sostituzione dell'uninstaller in `!uninstfinalize`:** makensis rilegge il file dopo il comando, quindi include la versione sostituita. Si verifica installando il setup prodotto e controllando la firma di `uninstall.exe` in `$INSTDIR`.
3. **Percorso passato allo shim** per l'uninstaller, e quoting di `%1` dentro `!uninstfinalize` (il comando finisce nel template NSIS tra apici singoli).
4. **7-Zip vede i file firmati dentro il setup?** Serve per il controllo automatico del §5.1. Se l'uninstaller non è visibile, il suo controllo passa tra le verifiche manuali.
5. **Metadati:** `oma-app.exe` ha nome prodotto "OpenMonitor Advanced" e la versione (da `tauri.conf.json`). `oma-service.exe` oggi non imposta `Product` e ricade sul nome dell'assembly: il piano aggiunge `Product` ("OpenMonitor Advanced") al progetto del servizio. Si controlla anche la risorsa di versione dell'uninstaller.

L'esito dello spike si scrive nel piano prima dei task successivi.

## 4. Workflow `.github/workflows/release.yml`

### 4.1 Avvio

- **Push di un tag `v*.*.*`:** release vera, signing policy `release-signing`.
- **`workflow_dispatch`** (solo da `main`): prova generale con la policy `test-signing`, che usa un certificato di prova non riconosciuto da Windows. Carica il setup come artifact del run e **non** crea nessuna release.

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

**Firma attiva:** la variabile `SIGNPATH_ORGANIZATION_ID` e il secret `SIGNPATH_API_TOKEN` esistono entrambi. Altrimenti gli step di firma si saltano e il log lo dice con un `::notice::`. Con un solo dei due il workflow fallisce: una configurazione a metà è un errore, non un'assenza.

**Attese:** ogni richiesta di firma usa `wait-for-completion-timeout-in-seconds: 5400` (90 minuti), così due approvazioni lente stanno nelle 4 ore del job. Se un'attesa scade, il job fallisce. Si rilancia il run intero con "Re-run all jobs": SignPath accetta fino a 3 riesecuzioni dello stesso run.

### 4.3 Sicurezza

- **Environment `release`:** contiene il secret `SIGNPATH_API_TOKEN` e la variabile `SIGNPATH_ORGANIZATION_ID`. Le deployment rule ammettono solo i tag `v*` e il branch `main`: una PR o un altro branch non vedono mai il token.
- **Ruleset sui tag `v*`:** solo il proprietario del repository può crearli, aggiornarli o cancellarli.
- **Permessi del job:** `contents: write` (release), `id-token: write` e `attestations: write` (attestazione); a livello di workflow `contents: read`.
- **Azioni di terzi** con versione fissata come in `ci.yml`. L'azione di SignPath si fissa allo SHA del commit, perché riceve il token.
- Nessuno step esegue codice di una PR: il workflow parte solo da tag o da `main`.

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

- **`release`:** `Get-AuthenticodeSignature` deve dare `Valid` sul setup, con il soggetto del firmatario che contiene `SignPath Foundation`. Estrae il setup con 7-Zip in una cartella temporanea e richiede lo stesso su `oma-app.exe`, `oma-service.exe` e, se lo spike del §3.4 punto 4 lo rende visibile, `uninstall.exe`. Controlla anche che `PawnIO_setup.exe` abbia ancora la firma del suo autore.
- **`test`:** stessi file, ma basta che la firma sia presente (il certificato di prova non è attendibile per Windows, quindi lo stato non è `Valid`).
- **`none`:** controlla che il setup esista e scrive un avviso "unsigned build".

L'accesso alle firme passa per un parametro iniettabile, così i test Pester usano un finto.

### 5.2 Checksum e attestazione

- **`SHA256SUMS.txt`** nel formato di `sha256sum` (`<hash minuscolo>  <nome file>`), una riga per il setup.
- **Attestazione** con `actions/attest-build-provenance` sul setup. Verifica per chiunque:

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
- **Riesecuzioni:** se esiste già una **bozza** per il tag, si sostituiscono solo gli asset (`gh release upload --clobber`) e il testo resta com'è, perché l'utente potrebbe averlo già modificato. Se la release è già **pubblicata**, lo step fallisce senza toccarla.

Il testo del template è in inglese, come le release esistenti.

## 6. Versione

### 6.1 `scripts/check-version.ps1 [-Tag vX.Y.Z]`

La versione vive in cinque file:

- `Cargo.toml` (`[workspace.package] version`);
- `app/package.json`;
- `app/src-tauri/tauri.conf.json`;
- la riga "Status" di `README.md` ("version X.Y.Z");
- la riga "Status" di `README.it.md`.

Il servizio la riceve da `tauri.conf.json` in `build-installer-payload.ps1`.

Lo script fallisce, elencando **tutte** le incongruenze e non solo la prima, se:

- i cinque file non hanno la stessa versione, o un file non contiene il campo atteso;
- `Cargo.lock` non è allineato (`cargo metadata --locked --format-version 1 --no-deps` fallisce);
- con `-Tag`: il tag non è `v` più la versione, oppure il commit del tag non è raggiungibile da `origin/main` (`git merge-base --is-ancestor`).

### 6.2 `scripts/bump-version.ps1 <X.Y.Z>`

- accetta solo `X.Y.Z` numerico: niente pre-release, perché la versione dei file Windows è solo numerica;
- rifiuta una versione minore o uguale a quella attuale;
- aggiorna i cinque file (solo la riga o il campo della versione, conservando fine riga LF e formattazione) e `Cargo.lock` (`cargo update --workspace --offline`);
- esegue `check-version.ps1` come controllo finale;
- **non** crea commit e **non** fa tag: stampa i comandi successivi (`git commit -am "chore: release X.Y.Z"`, `git tag vX.Y.Z`, `git push origin main vX.Y.Z`). Il push resta una decisione dell'utente.

### 6.3 Flusso di una release

1. `pwsh scripts/bump-version.ps1 0.3.0`, poi commit, tag e push.
2. Il workflow costruisce e, con la firma attiva, chiede due approvazioni su SignPath (arrivano per email).
3. Si rivede la bozza, si scrivono le novità e si pubblica.

## 7. Documentazione

- **`CODE_SIGNING.md`** (inglese, linkato dai due README), sul modello richiesto da SignPath:
  - ruoli: *Committers and reviewers* e *Approvers*, entrambi il proprietario del repository (@Cioscos);
  - la frase d'attribuzione;
  - cosa si firma e cosa no (D4, D5);
  - la dichiarazione sulla privacy: il programma non trasmette dati ad altri sistemi in rete se non su richiesta esplicita dell'utente. In M6c il controllo degli aggiornamenti introduce una chiamata a GitHub: la dichiarazione va aggiornata lì (voce nei follow-up).
- **`docs/release.md`** (italiano), guida passo passo per chi non ha mai firmato codice:
  1. la domanda alla Foundation e cosa scrivere nel modulo;
  2. l'autenticazione a due fattori su GitHub e su SignPath;
  3. GitHub App e trusted build system;
  4. progetto, policy `test-signing` e `release-signing`, artifact configuration (copiate da `.signpath/`);
  5. environment `release`, secret, variabile e ruleset sui tag;
  6. il flusso di una release (§6.3) e la prova con `workflow_dispatch`;
  7. cosa fare se un'approvazione scade, se lo shim segnala uno SHA-256 diverso o se la verifica delle firme fallisce.
- **`.signpath/artifact-configuration-binaries.xml`** e **`.signpath/artifact-configuration-setup.xml`:** copie versionate delle artifact configuration. La configurazione attiva sta nell'interfaccia di SignPath e queste copie vanno tenute allineate a mano. Ogni `pe-file` ha la firma Authenticode e i vincoli su nome prodotto ("OpenMonitor Advanced") e versione, questa passata come parametro dal workflow.
- **README (en, it):**
  - sezione "Verify your download" (checksum e attestazione);
  - link a `CODE_SIGNING.md`;
  - la riga sul SmartScreen si aggiorna quando la firma è attiva (non prima: finché la Foundation non approva resta com'è).
- **`CLAUDE.md`:** comandi di bump e di verifica della versione, e il flusso di release.
- **Spec principale:** §9 (firma dei binari) e §13.7 marcati come risolti, con un rimando a questa spec.
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
- `check-version.ps1`:
  - tutto allineato;
  - un file alla volta con versione diversa;
  - campo mancante;
  - tag sbagliato;
  - elenco completo delle incongruenze;
- `bump-version.ps1`, su una copia temporanea dei cinque file:
  - aggiornamento corretto con fine riga LF;
  - rifiuto di downgrade, stessa versione e formato non valido;
- `verify-signatures.ps1`: le tre policy con firme finte (valide, di prova, assenti, firmatario sbagliato);
- `render-release-notes.ps1`: variante firmata e non firmata.

I comandi `cargo` e `git` sono parametri iniettabili dove servono, come `DotnetExe` in `build-installer-payload.ps1`.

**CI:**

- nuovo job `scripts` in `ci.yml`, su `windows-latest` (Pester 5 è preinstallato), che esegue `Invoke-Pester scripts/tests`;
- job `actionlint` su `ubuntu-latest` per `ci.yml`, `release.yml` e la composite action.

In locale serve Pester 5 (`Install-Module Pester -Scope CurrentUser -MinimumVersion 5.0`): Windows include solo la 3.4.

### 8.2 Collaudo della milestone

1. Lo spike del §3.4, in locale, con lo shim in `collect` e `apply` senza firme reali. I file "firmati" sono copie con un byte di marcatura in coda, che bastano a verificare sostituzione e confronto degli SHA-256.
2. `workflow_dispatch` senza secret: percorso non firmato fino all'artifact, con `verify-signatures.ps1 -Policy none`.
3. Prima release reale non firmata (per esempio la 0.3.0) con il nuovo flusso: bump, tag, bozza, pubblicazione a mano.

**Verifiche manuali dovute dopo l'approvazione della Foundation** (in `docs/follow-ups.md`):

- `workflow_dispatch` con `test-signing`;
- una release con `release-signing`;
- l'installazione del setup firmato: firme di `oma-app.exe`, `oma-service.exe` e `uninstall.exe` in `$INSTDIR`, editore nella conferma UAC d'installazione e di disinstallazione;
- il comportamento di SmartScreen sul download.
