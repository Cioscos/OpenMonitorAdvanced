# Milestone 6a — Release e firma del codice: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** una pipeline di release su GitHub Actions che, dal push di un tag `vX.Y.Z`, costruisce l'installer, lo firma con SignPath (quando attivo) e ne fa una bozza di release con `SHA256SUMS.txt` e attestazione di provenienza. Prima dell'attivazione sono ammesse release non firmate; dopo il collaudo `REQUIRE_SIGNING=true` impedisce il ritorno accidentale al percorso non firmato.

**Stato della revisione:** corretto il 2026-09-30 dopo confronto con la spec e fonti primarie (§ "Esito della revisione del piano"). Nessun task eseguito; gli esiti dello spike e del collaudo restano da compilare.

**Architecture:**
- **Script PowerShell** in `scripts/`, sottili, con la logica in moduli `scripts/lib/*.psm1` testati con Pester 5. I comandi esterni (`git`, `gh`, `cargo`, 7-Zip, firme) passano per parametri iniettabili.
- **Shim di firma** (`scripts/sign-shim.ps1`) agganciato a `bundle.windows.signCommand` di Tauri solo in CI. Due passate: `collect` raccoglie i file che Tauri vorrebbe firmare, `apply` li sostituisce con le copie firmate da SignPath dopo aver controllato lo SHA-256.
- **Workflow:** `release.yml` (un job sequenziale nell'environment `release`), composite action `setup-toolchain` condivisa con `ci.yml`, nuovi job CI `scripts` (Pester) e `actionlint`.
- **Documenti:** `CODE_SIGNING.md`, guida operativa `docs/release.md`, copie versionate delle artifact configuration di SignPath in `.signpath/`.

**Tech Stack:** PowerShell 7, Pester 5.7.1 (fissato), GitHub Actions (`windows-latest`, `ubuntu-latest`), `signpath/github-action-submit-signing-request`, `actions/attest`, `gh` CLI, 7-Zip, actionlint (versione fissata con checksum), Tauri CLI 2.11.5, NSIS del bundler di Tauri. Nessun cambio a Rust, UI, protocollo o logica del servizio; nel servizio cambia solo il metadato `Product`.

**Spec:** `docs/superpowers/specs/2026-09-30-m6a-release-firma-design.md` (commit `fb8f9a1`); spec principale `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md` per il resto.

**Decisioni del piano** (interpretazioni della spec prese qui):
- **L1. Moduli e script sottili.** La logica sta in `scripts/lib/OmaCommon.psm1`, `OmaVersion.psm1`, `OmaSigning.psm1`, `OmaReleaseNotes.psm1` e `OmaRelease.psm1`. Gli script fanno parsing dei parametri, import e codice d'uscita. I test importano i moduli e provano anche il comportamento degli entry point tramite `pwsh -File`. `Invoke-OmaNative` restituisce stdout/stderr/exit code separati e fallisce su nonzero, salvo predicati git/lookup API gestiti esplicitamente dal chiamante; non confonde JSON e diagnostica. Gli adapter ne consumano `.Stdout`, non serializzano l'oggetto risultato come JSON remoto.
- **L2. Stato dello shim.** `sign-shim.ps1 -Mode init` crea lo stato dedicato verificato sotto `target/` e ci scrive `manifest.json` e i due config Tauri `tauri.sign.collect.json`/`tauri.sign.apply.json`. `collect`/`apply` ricevono `-StateRoot`/`-Path` e usano contesto GitHub indipendente; i config dello spike locale passano un contesto locale esplicito. I percorsi nel manifest sono assoluti, normalizzati con `[IO.Path]::GetFullPath` e confrontati senza distinzione di maiuscole; il Task 2 definisce i limiti obbligatori prima del reset dello stato.
- **L3. Nome del setup.** Il setup finale si copia sempre in `target/signing/final/OpenMonitor.Advanced_X.Y.Z_x64-setup.exe`, con i punti al posto degli spazi: è il nome che GitHub dà all'asset (come per la 0.2.0), così `SHA256SUMS.txt`, attestazione e asset coincidono.
- **L4. Certificati attesi** in `.signpath/certificates.json`: `release.subject` (DN esatto con CN `SignPath Foundation`), `release.thumbprints`, `test.subject`, `test.thumbprints` e `test.rootThumbprints` (liste). Per `test` si configura anche il percorso del certificato pubblico della radice di prova, verificato contro il pin prima dell'uso. Un campo obbligatorio vuoto fa fallire la policy corrispondente con il messaggio "no approved certificate configured for policy <p>". Il rinnovo si fa con un commit; i certificati/pin iniziali si acquisiscono dalla configurazione SignPath verificata, non da un file scaricato di origine ignota.
- **L5. Verifica crittografica della policy `test`.** `X509Chain.Build` verifica una catena di certificati, non l'integrità della firma sul PE; `UnknownError` non è un'eccezione accettabile in base alla sola catena. In un runner GitHub ospitato effimero o una VM di collaudo, un helper importa temporaneamente la sola radice di prova configurata in `Cert:\LocalMachine\Root` (non `CurrentUser\Root`: l'aggiunta a quello store apre una finestra di conferma di Windows che in un runner senza desktop blocca o fallisce; `LocalMachine` richiede un processo amministratore, come sui runner ospitati), verifica la firma incorporata con stato `Valid`, firmatario di prova fissato, timestamp valido, e rimuove in `finally` soltanto il certificato che ha aggiunto. Se la radice esisteva, non la rimuove. La policy `release` non importa radici e usa esclusivamente la fiducia ordinaria di Windows. L'ambiente è "isolato dichiarato" solo con `$env:RUNNER_ENVIRONMENT -eq 'github-hosted'`, oppure con `$env:OMA_ISOLATED_TRUST -eq '1'` impostato a mano in una VM o in Windows Sandbox; in più il processo dev'essere amministratore. Altrimenti la verifica `test` si ferma prima dell'import. La variabile allenta solo la guardia sull'ambiente, non i controlli di firma; i test unitari usano provider iniettati. Fonte: [Microsoft, WinVerifyTrust](https://learn.microsoft.com/en-us/windows/win32/api/wintrust/nf-wintrust-winverifytrust). Il metodo completo va collaudato con SignPath prima di abilitarlo.
- **L6. Marcatori delle note.** `<!-- oma:changes:start -->` / `<!-- oma:changes:end -->` racchiudono le novità scritte a mano; `<!-- oma:generated:start -->` / `<!-- oma:generated:end -->` il blocco tecnico. Ciascuno deve comparire esattamente una volta, nell'ordine changes → generated.
- **L7. Gate CI.** Il preflight chiede a `gh api` i run di `ci.yml` del repository corrente, `event=push`, `head_branch=main`, con `head_sha` uguale allo SHA del run di release. Valuta il run pertinente più recente e il suo ultimo `run_attempt`: richiede `completed`, `conclusion: success` e tutti i job `checks`, `service`, `installer`, `scripts`, `actionlint` riusciti. Un verde vecchio non maschera un rerun rosso/in corso. Run e job sono paginati; non si usano job di tentativi precedenti. I nomi restano gli id senza `name:` o si aggiorna esplicitamente la lista attesa. Errori API/autenticazione non diventano "nessun run". Fonti: [run GitHub](https://docs.github.com/en/rest/actions/workflow-runs), [job GitHub](https://docs.github.com/en/rest/actions/workflow-jobs).
- **L8. `--locked`.** La prima passata usa `pnpm tauri build --bundles nsis -- --locked`; lo spike conferma il passaggio a cargo con Tauri CLI 2.11.5. La CLI documenta `--` per gli argomenti al runner ([sorgenti Tauri](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.5/crates/tauri-cli/src/build.rs)). Se il relay non funziona, correggere l'invocazione o usare un runner che imponga `--locked`; non eseguire una build sbloccata. `cargo metadata --locked` e il controllo del diff del lockfile restano controlli aggiuntivi, non sostituti del vincolo durante la build.
- **L9. Riconoscimento dell'uninstaller e dei plugin.** Le regex `UninstallerPathPattern` e `PluginPathPattern` si ricavano dallo spike (Task 1) e si scrivono nella sezione "Esito dello spike" di questo piano. Il Task 2 le usa come costanti di `OmaSigning.psm1`.
- **L10. Versioni delle azioni.** In `release.yml` e nella composite action ogni `uses:` è fissato allo SHA completo, con commento `# vX.Y.Z`. `ci.yml` conserva i tag che ha, salvo ciò che passa alla composite action. L'implementer ricava gli SHA con `gh api repos/<owner>/<repo>/git/ref/tags/<tag>` (e dereferenzia i tag annotati).

## Global Constraints

- **Branch:** `feat/m6a-release-firma` da `main` (`fb8f9a1` o successivo), aperto con `superpowers:using-git-worktrees`; merge in `main` in locale; **push solo su richiesta dell'utente**.
- **Build locali invariate:** `pnpm tauri build` senza `--config` si comporta come oggi. Lo shim entra solo con i config generati da `sign-shim.ps1 -Mode init`.
- **Template NSIS:** la modifica minima è `!uninstfinalize '${UNINSTALLERSIGNCOMMAND}' = 0`, con commento `; OMA`. Se lo spike impone il fallback, si valida la pre-generazione e si aggiorna la spec prima dei task dipendenti. Un dubbio irrisolto blocca il percorso firmato, non il lavoro indipendente autorizzato.
- **Mai firmare binari separati di terzi:** plugin NSIS, `PawnIO_setup.exe` e runtime .NET separati non entrano negli artifact inviati a SignPath. `oma-service.exe` è self-contained e incorpora dipendenze: questo packaging va dichiarato alla Foundation come nella spec §2.
- **Nessun bypass dei controlli di firma nel workflow:** nessun parametro o variabile disattiva il confronto SHA-256, la verifica delle firme o il gate CI.
- **Stato dello shim in `target/signing/`,** ricreato vuoto a ogni run, mai in cache.
- **Versioni:** `X.Y.Z` con regex `^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$`, ogni componente ≤ 65535; versione PE `X.Y.Z.0`.
- **Testi fissi:**
  - attribuzione: `Free code signing provided by SignPath.io, certificate by SignPath Foundation`;
  - riga non firmata: `The installer is not code-signed yet, so Windows SmartScreen may warn you: choose *More info* → *Run anyway*.`
- **Attese SignPath:** `wait-for-completion-timeout-in-seconds: 5400`; job `timeout-minutes: 240`.
- **Stile:** codice, commenti, commit, `CODE_SIGNING.md` e template delle note in inglese (conventional commits); `docs/release.md` e prosa dei piani in italiano con gli accenti; fine riga LF.
- **Comandi:** PowerShell 7, dalla radice del repository; per i comandi UI `Push-Location app` e `try { … } finally { Pop-Location }`.
- **Test locali:** tutte le invocazioni Pester di questo piano escludono `Integration`, salvo quelle esplicitamente riservate al runner/VM. Cleanup e ripristino del template/payload/ambiente si eseguono anche se un task fallisce; nessuna fixture di prova diventa payload di una release successiva.
- **Pester in locale:** `Install-Module Pester -Scope CurrentUser -RequiredVersion 5.7.1 -Force`, poi `Import-Module Pester -RequiredVersion 5.7.1`. Non disabilitare preventivamente il controllo dell'editore. Per default `Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI`; i test `Integration` che modificano gli store dei certificati girano solo nell'ambiente isolato, in una seconda invocazione esplicita.
- **Verifiche dal vivo:** mai input sintetico sul desktop dell'utente. Installazioni di prova solo in Windows Sandbox o in una VM, e le avvia l'utente.
- **graphify e subagent:**
  - dopo ogni task che tocca il codice: `$env:PYTHONHASHSEED = '0'; graphify update .`;
  - nei brief dei subagent vanno `graphify query`, `graphify explain`, `graphify path` e le skill `superpowers:test-driven-development` e `superpowers:systematic-debugging`;
  - la documentazione di Tauri, Pester e GitHub Actions si consulta con context7 o Microsoft Learn, non a memoria.

## Review Focus

Condizioni che la spec implica e che i test di funzionalità da soli non coprirebbero, in ordine di probabilità. Ogni riga ha i suoi test nel task indicato.

1. **Un aggiornamento di Tauri cambia i file passati a `signCommand`** (un plugin nuovo, un sidecar, un altro percorso temporaneo dell'uninstaller). Ci si aspetta che la release si fermi con un messaggio che nomina il file, non che firmi o salti qualcosa in silenzio. Test: `rejects_an_unexpected_path`, `rejects_a_second_uninstaller`, `check_fails_when_the_uninstaller_was_never_called` (Task 2).
2. **Rerun dopo un'approvazione scaduta o un errore a metà upload:** il run non dichiara successo con asset misti o note incoerenti e non modifica deliberatamente una release pubblicata. Gli upload non sono atomici: una bozza parziale deve essere segnalata come non pubblicabile, poi recuperata con un rerun; non si promette che uno stato intermedio non esista. Test: `update_switches_the_generated_block_and_keeps_changes` (Task 5), `refuses_a_published_release`, `verifies_remote_assets_after_upload`, `partial_upload_is_reported` (Task 6).
3. **Tag spinto per sbaglio** su un commit non in `main`, con versione sbagliata o su un commit con CI rossa: ci si aspetta che il run si fermi prima di qualsiasi richiesta di firma. Test: `tag_not_on_main_fails`, `annotated_tag_resolves_to_its_commit` (Task 3), `ci_gate_requires_every_job` (Task 6).
4. **Credenziali SignPath a metà o tolte dopo l'attivazione:** errore, non release non firmata. Test: `partial_configuration_fails`, `required_signing_without_credentials_fails` (Task 6).
5. **Percorsi con spazi e directory corrente diversa** (Tauri gira in `app/`, makensis altrove; il setup si chiama `OpenMonitor Advanced_…`). Test: `works_from_another_directory_with_spaces` (Task 2), `final_setup_name_uses_dots` (Task 6).

---

## Mappa dei file

- **Nuovi script:**
  - `scripts/sign-shim.ps1`, `scripts/check-version.ps1`, `scripts/bump-version.ps1`;
  - `scripts/verify-signatures.ps1`, `scripts/render-release-notes.ps1`;
  - `scripts/release-preflight.ps1`, `scripts/publish-draft.ps1`.
- **Nuovi moduli:** `scripts/lib/OmaCommon.psm1`, `OmaVersion.psm1`, `OmaSigning.psm1`, `OmaReleaseNotes.psm1`, `OmaRelease.psm1`.
- **Test:** `scripts/tests/{Common,SignShim,Version,VerifySignatures,BuildInstallerPayload,ReleaseNotes,Release}.Tests.ps1`, dati in `scripts/tests/fixtures/`.
- **Pin PawnIO condivisi:** piccolo modulo/config `scripts/lib/OmaPawnIoPins.psm1` usato da build del payload e verificatore; hash nel file già esistente `pawnio.sha256` resta fonte unica.
- **CI e release:**
  - `.github/actions/setup-toolchain/action.yml`, `.github/workflows/release.yml`, `.github/release-notes-template.md` (nuovi);
  - `.github/workflows/ci.yml` (modificato).
- **SignPath:** `.signpath/artifact-configuration-binaries.xml`, `.signpath/artifact-configuration-setup.xml`, `.signpath/certificates.json`.
- **Dopo l'approvazione:** certificato pubblico della radice di prova in `.signpath/`, mai chiavi private; pin e configurazione attiva verificati nel collaudo differito.
- **Modificati:**
  - `app/src-tauri/nsis/installer.nsi` (una riga);
  - `service/OpenMonitorAdvanced.Service/OpenMonitorAdvanced.Service.csproj` (`Product`);
  - `scripts/build-installer-payload.ps1` (controllo `ProductName`).
- **Documenti:**
  - `CODE_SIGNING.md`, `docs/release.md` (nuovi);
  - `README.md`, `README.it.md`, `CLAUDE.md`, `docs/follow-ups.md`;
  - spec principale §9 e §13;
  - questo piano (esito dello spike ed esito dell'esecuzione).

---

### Task 1: spike di fattibilità (gate, spec §3.4)

Gli script dello spike vivono in `target/spike/`, che non si committa. Il risultato è la sezione "Esito dello spike" di questo piano, con comandi esatti, versioni della toolchain, hash e log. Un fallimento di determinismo/sostituzione/quoting blocca i task della firma finché non si valida il fallback e si allinea la spec. La parte comune del Task 2, poi i Task 3, 5 e 6, possono procedere: dipendono dal modulo comune, non dallo shim. Il Task 4 richiede comunque lo schema del manifest e i risultati sui metadati; i Task 8 e il collaudo firmato non possono ignorare il gate. Lo spike locale non dimostra l'ammissibilità né il comportamento del certificato SignPath reale.

**Files:**
- Create (non committati): `target/spike/log-shim.ps1`, `target/spike/sign-log.json`
- Modify (temporaneo, ripristinato a fine task): `app/src-tauri/nsis/installer.nsi` riga 101
- Modify: questo piano, sezione "Esito dello spike"

**Interfaces:**
- Produces: nella sezione "Esito dello spike"
  - le regex `UninstallerPathPattern` e `PluginPathPattern`;
  - la forma esatta di `signCommand` che funziona (strutturata `cmd`/`args`);
  - l'esito del determinismo;
  - la visibilità dei file con 7-Zip;
  - i valori di `ProductName`, `ProductVersion` e `FileVersion` di app, uninstaller e servizio;
  - l'esito di `-- --locked` (L8);
  - la decisione: **procedi** oppure **fallback**.

- [ ] **Step 1: shim di registrazione.**
  - `log-shim.ps1 -Mode record|replace -Path <p> -Out <dir>` scrive in `<dir>/calls.jsonl`, per ogni chiamata: percorso ricevuto, directory corrente, `$args` grezzi, SHA-256 e dimensione.
  - In `record` copia il file in `<dir>/files/<n>-<nome>`.
  - In `replace`, solo per `app` e `uninstaller`, confronta lo SHA-256 con `record`; se coincide, sovrascrive con la copia registrata più 4096 byte `0x4F`. `setup` viene solo registrato; i cinque plugin vengono riconosciuti esplicitamente e lasciati intatti in tutte le modalità. Un file inatteso fallisce; si conserva separatamente l'hash della copia marcata attesa.
  - Con `-FailOn uninstaller` esce con 1 sulla chiamata dell'uninstaller.

  Il config `sign-log.json` usa la forma strutturata `{"bundle":{"windows":{"signCommand":{"cmd":"pwsh","args":[...,"%1"]}}}}` con percorsi assoluti.
- [ ] **Step 2: passata A.**
  - `pwsh scripts/build-installer-payload.ps1`.
  - Poi, in `app/`: `pnpm tauri build --bundles nsis --config ../target/spike/sign-log.json -- --locked` (`record`, output in `target/spike/A`).
  - Conservare subito il setup A in `target/spike/A/`: il bundle B sovrascrive l'output NSIS. Salvare anche copia originale del servizio e hash degli input di build/UI e delle copie dei plugin.
  - Annotare:
    - tutte le chiamate (attese: 5 plugin, `oma-app.exe`, uninstaller, setup);
    - il percorso e il nome dell'uninstaller;
    - la cartella delle copie dei plugin;
    - se `--locked` è arrivato a cargo (log di `-v`).
- [ ] **Step 3: passata B, sostituzioni reali.**
  - Sostituire `target/installer-payload/service/oma-service.exe` con una copia più 1 MiB di `0x00`, per cambiare la dimensione del payload.
  - `pnpm tauri bundle --bundles nsis --config …` con `log-shim` in `replace`.
  - Registrare:
    - che `oma-app.exe` e l'uninstaller ricevuti hanno lo stesso SHA-256 della passata A;
    - che i plugin non sono cambiati;
    - che `tauri bundle` non ha ricompilato Rust, la UI o il servizio (log, hash dell'app originale dopo il ripristino di Tauri, hash del payload marcato e degli asset UI). L'mtime dell'app può cambiare durante patch/ripristino: non è un criterio di fallimento.
  - Conservare subito il setup B in `target/spike/B/` e i suoi hash, prima del bundle successivo.
- [ ] **Step 4: passata C, tempo e temporanei diversi.** Almeno 2 minuti dopo, creare `target/spike/tmp-c`, salvare `$env:TEMP`/`$env:TMP`, impostarli su percorsi assoluti e ripetere la passata B con gli stessi payload marcati (senza append cumulativi). Conservare il setup C. In `finally` ripristinare ambiente, template e payload originale anche in caso di errore.
- [ ] **Step 5: contenuto del setup.**
  - `& 'C:\Program Files\7-Zip\7z.exe' l -slt <setup>` sulle copie conservate di A e B. Rilevare il percorso/versione di 7-Zip anche in CI, senza presumere che `7z` sia nel PATH; controllare l'exit code.
  - Registrare se compaiono `oma-app.exe`, `oma-service.exe`, `uninstall.exe` e `PawnIO_setup.exe`.
  - Estrarre quelli visibili (`7z x`) e controllare che in B abbiano lo SHA-256 delle copie "firmate" (marcate) dello Step 3.
- [ ] **Step 6: prova negativa.**
  - Con `installer.nsi` riga 101 **senza** `= 0` e `-FailOn uninstaller`: registrare se makensis prosegue (atteso: sì).
  - Con `!uninstfinalize '${UNINSTALLERSIGNCOMMAND}' = 0 ; OMA`: makensis deve fermarsi, e `pnpm tauri bundle` uscire con un codice diverso da 0.
  - Ripristinare la riga originale: la modifica definitiva la fa il Task 2 con i suoi test.
- [ ] **Step 7: metadati.** `(Get-Item <file>).VersionInfo | Format-List ProductName, ProductVersion, FileVersion` per `oma-app.exe` e l'uninstaller raccolti in A e per `oma-service.exe`.
- [ ] **Step 8: uninstaller installato (con l'utente).**
  - Se allo Step 5 l'uninstaller non è visibile, chiedere all'utente di installare il setup di B in **Windows Sandbox**, oppure in una VM se Sandbox non è attivo.
  - L'utente riporta lo SHA-256 di `C:\Program Files\OpenMonitor Advanced\uninstall.exe` con `Get-FileHash`; atteso: quello della copia marcata.
  - Se l'utente non può, il punto resta "da verificare" e lo si annota come verifica manuale dovuta. Non blocca, perché la prova con SignPath del Task 10 lo ripete.
- [ ] **Step 9: esito.**
  - Scrivere la sezione "Esito dello spike" (valori degli Step 2-8, regex di L9, decisione).
  - Se la decisione è **fallback**, validarlo e aggiornare la spec prima dei task dipendenti; se resta irrisolto, registrare il blocco e proseguire solo con le parti indipendenti indicate sopra.
  - Commit `docs: record the M6a signing spike`.

---

### Task 2: modulo comune, shim di firma e gate dell'uninstaller

**Files:**
- Create: `scripts/lib/OmaCommon.psm1`, `scripts/lib/OmaSigning.psm1`, `scripts/sign-shim.ps1`, `scripts/tests/Common.Tests.ps1`, `scripts/tests/SignShim.Tests.ps1`
- Modify: `app/src-tauri/nsis/installer.nsi:101`

**Interfaces:**
- Consumes: le regex `UninstallerPathPattern` e `PluginPathPattern` e la forma di `signCommand` dalla sezione "Esito dello spike".
- Produces:
  ```powershell
  # OmaCommon.psm1
  function Invoke-OmaNative { param([string]$FilePath, [string[]]$ArgumentList, [string]$WorkingDirectory, [switch]$AllowFailure) }
      # -> @{ Stdout; Stderr; ExitCode }; throws on nonzero unless AllowFailure, needed for git predicates/API 404
  function Get-OmaSha256 { param([string]$Path) }            # -> lowercase hex string
  function Resolve-OmaPath { param([string]$Path) }          # -> [IO.Path]::GetFullPath, no trailing separator
  # OmaSigning.psm1
  function Initialize-OmaSigningState { param([string]$StateRoot, [string]$RepoRoot, [string]$Commit, [string]$Version, [string]$RunId, [string]$RunAttempt) }
      # recreates $StateRoot empty; writes manifest.json, tauri.sign.collect.json, tauri.sign.apply.json
  function Invoke-OmaSignShim { param([ValidateSet('collect','apply')][string]$Mode, [string]$StateRoot, [string]$Path) }
  function Register-OmaService { param([string]$StateRoot, [string]$Path) }            # collect-only, role 'service'
  function Import-OmaSignedFiles { param([string]$StateRoot, [string]$From) }           # exact set oma-app.exe, uninstall.exe, oma-service.exe -> signed/, records signed sha256
  function Assert-OmaSigningPass { param([string]$StateRoot, [ValidateSet('collect','apply')][string]$Pass, [pscustomobject]$ExpectedContext) }
  ```
  Manifest (`manifest.json`, `schema: 1`):
  ```json
  { "schema": 1, "commit": "…", "version": "0.3.0", "runId": "…", "runAttempt": "1",
    "expected": { "app": "<abs>/target/release/oma-app.exe", "setupDir": "<abs>/target/release/bundle/nsis",
                  "setupName": "OpenMonitor Advanced_0.3.0_x64-setup.exe" },
    "collect": [ { "role": "app|uninstaller|setup|service|plugin", "path": "…", "name": "oma-app.exe|uninstall.exe|…", "sha256": "…", "after": "…" } ],
    "apply":   [ … ],
    "signed":  { "oma-app.exe": "<sha>", "uninstall.exe": "<sha>", "oma-service.exe": "<sha>" } }
  ```
  `sign-shim.ps1` parametri: `-Mode init|collect|apply|register-service|import-signed|check`, `-StateRoot`, `-Path`, `-From`, `-Pass`, `-RepoRoot` (default dalla radice dello script), e `-Commit`, `-Version`, `-RunId`, `-RunAttempt` richiesti per `init` e `check`. In CI il wrapper usa il contesto GitHub indipendente per il confronto anche nelle chiamate `collect`/`apply`; in locale lo spike/config passa un contesto locale esplicito. Exit 0 solo in caso di successo; il messaggio d'errore va su stderr.
- **Regole:**
  - **`init`:** prima di qualsiasi cancellazione risolve `RepoRoot` e `StateRoot`, richiede uno stato dedicato strettamente sotto `<RepoRoot>/target/` (non `target` stesso, radice repo/disco o directory dei payload), e rifiuta reparse point/junction lungo il percorso. Poi svuota solo quel target verificato con `-LiteralPath` e scrive i config. `Resolve-OmaPath` preserva le radici, richiede percorsi assoluti per lo stato e non deduce fiducia da una regex senza controllo del parent. `signCommand`: `pwsh -NoProfile -NonInteractive -File <abs>/scripts/sign-shim.ps1 -Mode <m> -StateRoot <abs> -Path %1`.
  - **Riconoscimento del ruolo:**
    - `app` se il percorso è `expected.app`;
    - `setup` se è `expected.setupDir/expected.setupName`;
    - `plugin` se corrisponde a `PluginPathPattern` e il nome è uno dei cinque della spec §3.1;
    - `uninstaller` se corrisponde a `UninstallerPathPattern`;
    - qualunque altro percorso: errore `unexpected file passed to signCommand: <path>`.
  - **Duplicati:** un secondo `app`, `uninstaller` o `setup` nella stessa passata è un errore `duplicate <role> call`.
  - **`collect`:** copia `app` e `uninstaller` in `unsigned/<name>`; `register-service` copia lì anche `oma-service.exe`. Il setup viene registrato senza copiarlo in `unsigned/`, che deve contenere esattamente i tre binari. I plugin restano intatti; hash prima/dopo devono coincidere.
  - **`apply`:**
    - `app` e `uninstaller`: lo SHA-256 dev'essere uguale a quello della passata `collect`, altrimenti errore `hash mismatch for <name>: collected <a>, received <b>`. Poi sovrascrive con `signed/<name>`, e se manca è un errore `signed copy missing for <name>`;
    - `setup` e `plugin`: solo registrati; i plugin devono restare intatti anche rispetto alla raccolta. Prima della copia, il file in `signed/` deve ancora coincidere con il suo hash importato; dopo la copia l'hash del destinatario deve essere quello firmato.
  - **`check`:** in `collect` richiede esattamente un `app`, un `uninstaller`, un `setup` e un `service`; in `apply` un `app`, un `uninstaller` e un `setup`. Controlla i plugin realmente presenti censiti nello spike. Il contesto atteso è passato indipendentemente dal workflow (`GITHUB_SHA`, versione validata, run id/attempt); non è letto dallo stesso manifest da verificare. Dopo `collect` le righe di raccolta sono congelate; `apply` aggiorna solo il proprio registro e il setup finale, senza alterare gli hash non firmati.
  - **`import-signed`:** richiede in `From` esattamente i tre nomi, niente di più e niente di meno, prima di copiare. La verifica della firma è a parte (Task 4) e il workflow la esegue prima dell'import.

- [ ] **Step 1: test che falliscono** (`Common.Tests.ps1`, `SignShim.Tests.ps1`, con file finti di pochi byte sotto un repo finto in `$TestDrive/repo/target/signing`). Il modulo comune può essere implementato e committato separatamente se lo spike blocca lo shim:
  - `Invoke-OmaNative`: `returns_stdout`, `throws_on_nonzero_exit`;
  - `init_creates_empty_state_and_configs`: il config `collect` contiene `%1` come argomento a sé e percorsi assoluti;
  - `collect_records_role_name_and_hash`;
  - `apply_replaces_with_the_signed_copy`;
  - `apply_fails_on_hash_mismatch`;
  - `apply_fails_when_signed_copy_missing`;
  - `apply_leaves_the_setup_untouched`;
  - `plugins_are_left_intact`;
  - `rejects_an_unexpected_path`;
  - `rejects_a_second_uninstaller`;
  - `register_service_records_role_service`;
  - `check_fails_when_the_uninstaller_was_never_called`;
  - `check_fails_on_manifest_of_another_run`;
  - `import_signed_rejects_extra_or_missing_files`;
  - `init_rejects_repo_root_target_root_and_junctions`, `apply_rejects_tampered_signed_copy`, `check_uses_independent_run_context`;
  - `works_from_another_directory_with_spaces`: `StateRoot` e file sotto `$TestDrive/repo con spazi/target/signing/`, eseguito con `pwsh -File` da un'altra directory corrente;
  - `script_exit_codes`: 0 in caso di successo, diverso da 0 in caso d'errore, via `pwsh -File`.
- [ ] **Step 2:** `Invoke-Pester -Path scripts/tests/Common.Tests.ps1, scripts/tests/SignShim.Tests.ps1 -CI` → FAIL (moduli mancanti).
- [ ] **Step 3: implementa** `OmaCommon.psm1`, `OmaSigning.psm1` e `sign-shim.ps1` secondo le Interfaces. Le scritture del manifest passano da un file temporaneo e poi un rename.
- [ ] **Step 4:** stesso comando → PASS.
- [ ] **Step 5: gate dell'uninstaller.**
  - Riga 101 di `installer.nsi`: `  !uninstfinalize '${UNINSTALLERSIGNCOMMAND}' = 0 ; OMA`.
  - Verifica: prima `build-installer-payload.ps1`; `init` in `target/signing-local` con contesto locale esplicito, poi in `app/` `pnpm tauri build --bundles nsis --config ../target/signing-local/tauri.sign.collect.json -- --locked`. Registrare il servizio, poi dalla radice `sign-shim.ps1 -Mode check -Pass collect -StateRoot …` con il contesto atteso. Atteso: exit 0 e un setup prodotto; ripetere anche la prova negativa del comando uninstaller con il codice definitivo.
- [ ] **Step 6: commit** `feat(release): add the two-pass signing shim`.

---

### Task 3: controllo e bump della versione

**Files:**
- Create: `scripts/lib/OmaVersion.psm1`, `scripts/check-version.ps1`, `scripts/bump-version.ps1`, `scripts/tests/Version.Tests.ps1`, `scripts/tests/fixtures/version/` (copie minime dei cinque file e di un `Cargo.lock`)

**Interfaces:**
- Consumes: `Invoke-OmaNative` (Task 2).
- Produces:
  ```powershell
  function ConvertTo-OmaVersion { param([string]$Text) }       # -> [version] (Major, Minor, Build); throws on non-canonical or > 65535
  function Get-OmaVersionSources { param([string]$RepoRoot) }   # -> [pscustomobject[]] @{ File; Version ([string] or $null); Pattern }
  function Test-OmaVersionConsistency { param([string]$RepoRoot, [string]$Tag, [scriptblock]$Cargo, [scriptblock]$Git, [string]$ExpectedSha) } # -> [string[]] problems (empty = ok)
  function Set-OmaVersion { param([string]$RepoRoot, [string]$Version, [scriptblock]$Cargo, [scriptblock]$Git) } # bump with backup/rollback
  ```
  - **Campi letti:**
    - `Cargo.toml` → `version = "X.Y.Z"` nella sezione `[workspace.package]`;
    - `app/package.json` e `app/src-tauri/tauri.conf.json` → `"version": "X.Y.Z"` di primo livello;
    - `README.md` → `**Status:** … (version X.Y.Z)`;
    - `README.it.md` → `**Stato:** … (versione X.Y.Z)`.
  - **`check-version.ps1 [-Tag vX.Y.Z] [-ExpectedSha <sha>]`:** stampa ogni problema su una riga ed esce con 1 se ce n'è almeno uno; il workflow passa sempre `ExpectedSha` dal contesto GitHub.
  - **`bump-version.ps1 X.Y.Z`:** esce con 0 e stampa i tre comandi git della spec §6.2.
- **Regole di `Test-OmaVersionConsistency`:**
  - versioni diverse o campi mancanti: una riga per file;
  - `cargo metadata --locked --format-version 1 --no-deps` eseguito dalla radice e fallito;
  - con `-Tag`:
    - il tag non corrisponde alla regex `^v…$` della spec o non è `v` più la versione;
    - `git fetch origin +refs/heads/main:refs/remotes/origin/main` fallisce (aggiornamento esplicito del ref usato dall'ascendenza);
    - `git rev-parse <tag>^{commit}` è diverso da `git rev-parse HEAD`;
    - `git merge-base --is-ancestor <sha> origin/main` fallisce.
  - Le chiamate git distinguono exit 1 (predicato falso) da exit >1 (errore operativo); gli errori si aggiungono all'elenco senza perdere le altre incongruenze. In CI `HEAD` e commit del tag devono coincidere anche con lo SHA del run, passato indipendentemente. `Git`/`Cargo` hanno default reali e un contratto comune con exit code; tutti gli argomenti vanno in un array, non in una stringa shell.
- **Regole di `Set-OmaVersion`:**
  - prima di scrivere verifica che la consistenza corrente sia vuota di problemi e che la nuova versione sia maggiore in confronto numerico;
  - salva i cinque file e `Cargo.lock` in memoria come byte;
  - riscrive solo la sottostringa della versione, conservando i fine riga LF;
  - esegue `cargo update --workspace --offline`, poi `Test-OmaVersionConsistency`;
  - su qualsiasi errore ripristina i sei file byte per byte e rilancia;
  - rifiuta modifiche a `Cargo.lock` estranee al bump dei pacchetti del workspace.
  - Il controllo è strutturale: identifica i pacchetti locali dal metadata, permette anche i riferimenti interni al workspace che cambiano versione e vieta nuovi pacchetti, cambi di source/checksum o aggiornamenti di dipendenze esterne. Il diff testuale delle sole righe `version` sarebbe troppo restrittivo per lockfile con dipendenze disambiguate.

- [ ] **Step 1: test che falliscono** (`Version.Tests.ps1`, su copie delle fixture in `$TestDrive`, con `Cargo` e `Git` finti):
  - `ConvertTo-OmaVersion`: `accepts_canonical` (`0.3.0`, `65535.0.1`), `rejects_leading_zeros` (`0.03.0`), `rejects_above_pe_limit` (`0.65536.0`), `rejects_prerelease` (`0.3.0-rc.1`);
  - `all_aligned_has_no_problems`;
  - `one_file_different_is_reported` (un caso per ognuno dei cinque file);
  - `missing_field_is_reported`;
  - `every_problem_is_listed` (due file sbagliati → due righe);
  - `reads_the_italian_readme` (`Stato`/`versione`);
  - `locked_metadata_failure_is_reported`;
  - `tag_mismatch_fails`, `non_numeric_tag_fails` (`v0.3`, `v0.3.0-rc1`);
  - `annotated_tag_resolves_to_its_commit`;
  - `tag_not_head_fails`, `tag_not_on_main_fails`;
  - bump: `bump_updates_every_file_with_lf`, `bump_rejects_downgrade_and_same`, `bump_compares_numerically` (`0.10.0` > `0.9.0`), `bump_rolls_back_on_cargo_failure` (byte identici, `Cargo.lock` compreso), `bump_rejects_unrelated_lock_changes`.
- [ ] **Step 2:** `Invoke-Pester -Path scripts/tests/Version.Tests.ps1 -CI` → FAIL.
- [ ] **Step 3: implementa** `OmaVersion.psm1` e i due script.
- [ ] **Step 4:** stesso comando → PASS; poi `pwsh scripts/check-version.ps1` sul repository → exit 0 (0.2.0 ovunque).
- [ ] **Step 5: commit** `feat(release): add version check and bump scripts`.

---

### Task 4: verifica delle firme e metadati del servizio

**Files:**
- Create: `scripts/verify-signatures.ps1`, `.signpath/certificates.json`, `scripts/tests/VerifySignatures.Tests.ps1`
- Modify: `scripts/lib/OmaSigning.psm1` (funzioni di verifica), `service/OpenMonitorAdvanced.Service/OpenMonitorAdvanced.Service.csproj` (`<Product>OpenMonitor Advanced</Product>`), `scripts/build-installer-payload.ps1` (passo 4: `ProductName` atteso)
- Create: `scripts/tests/BuildInstallerPayload.Tests.ps1`, `scripts/lib/OmaPawnIoPins.psm1`. Nel checkout attuale `scripts/tests/` non esiste: i test del gate `ProductName` sono nuovi, non aggiornamenti di una suite già presente.

**Interfaces:**
- Consumes: manifest del Task 2 (`signed`); `Get-OmaSha256`, `Invoke-OmaNative`.
- Produces:
  ```powershell
  function Test-OmaSignature { param([string]$Path, [ValidateSet('release','test')][string]$Policy, [pscustomobject]$Certificates,
      [scriptblock]$SignatureProvider, [scriptblock]$ChainProvider, [scriptblock]$EmbeddedSignatureProvider) }  # -> [string[]] problems
  function Test-OmaPayload { param([string]$Setup, [ValidateSet('release','test','none')][string]$Policy, [string]$Manifest,
      [string]$Version, [pscustomobject]$Certificates, [scriptblock]$Extractor, [scriptblock]$SignatureProvider,
      [scriptblock]$ChainProvider, [scriptblock]$VersionInfoProvider, [scriptblock]$EmbeddedSignatureProvider) } # -> [string[]] problems
  ```
  `verify-signatures.ps1 -Policy release|test|none -Version X.Y.Z (-Setup <p> -Manifest <p> | -Files <dir>)`. Il manifest è obbligatorio per `-Setup` in tutte le policy. Con `-Files` richiede esattamente i tre nomi e verifica firme/metadati prima dell'import (senza confrontare con hash firmati ancora non importati). Esce con 1 se ci sono problemi; `none` scrive `::warning::unsigned build`. I default dei provider sono reali, nessun parametro del workflow permette provider finti.
  `.signpath/certificates.json` iniziale: `{ "release": { "subject": "", "thumbprints": [] }, "test": { "subject": "", "thumbprints": [], "rootThumbprints": [], "rootCertificatePath": "" } }`. La policy `none` non richiede certificati SignPath configurati.
- **Regole:**
  - **`release`:**
    - `Status` `Valid`;
    - `SignerCertificate.Subject` uguale a `release.subject` e thumbprint nella lista;
    - `TimeStamperCertificate` presente e timestamp verificato e legato alla firma dall'`EmbeddedSignatureProvider`, non solo presenza dell'oggetto. Il provider reale usa il verificatore Authenticode di Windows/SDK (per esempio `signtool verify /pa /all /tw`, con exit code e diagnostica verificati nello spike), rifiuta firme solo di catalogo e restituisce separatamente esito crittografico ed esito del timestamp. Rilevare il path SDK di signtool senza presumere il PATH; `/tw` segnala l'assenza del timestamp e warning/error non sono successo ([manuale SignTool](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool)).
  - **`test`:** importa/rimuove la radice fissata nell'ambiente isolato come L5, poi richiede `Valid`, firmatario/leaf/root configurati e gli stessi controlli crittografici/timestamp. `UnknownError`, `HashMismatch`, `NotSigned`, `NotTrusted`, `Incompatible` e qualunque altro errore dopo l'import falliscono; anche `Valid` con un certificato diverso da quello atteso fallisce.
  - **Metadati** del setup e di `oma-app.exe`, `oma-service.exe`, `uninstall.exe`: `ProductName` "OpenMonitor Advanced", `ProductVersion` e `FileVersion` corrispondenti alla versione corrente normalizzata nei formati rilevati nello spike. Lo spike registra il formato, non fissa per sempre il valore `0.2.0`.
  - **Setup:**
    - `7z x` in una cartella temporanea, con exit code controllato;
    - richiede esattamente una copia di `oma-app.exe` e di `oma-service.exe`, e di `uninstall.exe` se lo spike lo ha trovato visibile;
    - lo SHA-256 di ognuno deve coincidere con `manifest.signed`;
    - `PawnIO_setup.exe` deve comparire esattamente una volta, avere lo SHA-256 di `app/src-tauri/nsis/pawnio.sha256`, firma `Valid` e subject/thumbprint originali già fissati in `build-installer-payload.ps1`. Non sostituire il controllo dell'autore con il solo stato `Valid`.
  - **Con `none`:** setup non vuoto, estrazione riuscita, presenza dei payload propri, PawnIO come sopra. Gli hash non si confrontano col manifest (non ci sono copie firmate), ma devono coincidere con quelli di `manifest.collect`.
  - **Uninstaller non visibile a 7-Zip:** in `release|test` verifica comunque copia firmata importata e hash/esito di sostituzione nel manifest, poi riepilogo `installed uninstaller signature not verified`. Con `none` verifica raccolta/completamento della passata senza cercare copie firmate inesistenti. Nessuna dichiarazione di collaudo dell'uninstaller installato prima della prova isolata del Task 10.

- [ ] **Step 1: test che falliscono** (fornitori finti che restituiscono oggetti con `Status`, `SignerCertificate`, `TimeStamperCertificate`; estrattore finto che scrive file in una cartella):
  - `release_accepts_valid_expected_signer`;
  - `release_rejects_substring_subject` (`CN=SignPath Foundation Fake`);
  - `release_rejects_unknown_thumbprint`;
  - `release_fails_without_configured_certificate`;
  - `rejects_hash_mismatch_status`, `rejects_missing_timestamp`;
  - `test_accepts_only_valid_signature_of_configured_leaf_and_root`;
  - `test_rejects_unknown_error_even_with_valid_certificate_chain`;
  - `test_rejects_untrusted_root_of_other_root`, `test_rejects_valid_but_wrong_leaf`;
  - `test_trust_is_cleaned_up_on_failure`, `test_keeps_preexisting_root`, `test_refuses_nonisolated_environment`;
  - `rejects_catalog_signature`, `rejects_invalid_timestamp`, `metadata_setup_and_product_file_versions_checked`;
  - `payload_hash_must_match_manifest`;
  - `payload_missing_or_duplicate_fails`;
  - `extractor_failure_fails`;
  - `pawnio_hash_and_signature_checked`;
  - `none_policy_checks_content_and_warns`;
  - `metadata_product_name_checked`;
  - integrazione con tag `Integration`, **solo runner effimero/VM**: generare una fixture PE propria da sorgente minimale sotto `$TestDrive`, senza rifirmare `where.exe` o altro codice di Windows (che può essere firmato anche via catalogo). Creare certificato di prova con chiave privata temporanea, firmare la fixture e testare l'helper crittografico reale con trust L5. Modificare un byte nella sezione di codice coperta da Authenticode, non nel checksum PE/security directory/overlay escluso dall'hash: la verifica deve fallire. Cleanup di fixture, chiave e soli certificati aggiunti in `finally`/`AfterAll`. Questo test prova integrità e cleanup; non usa una TSA esterna per fabbricare un timestamp e non pretende di far passare la policy completa senza timestamp. Il gate timestamp completo è coperto con provider finti positivi/negativi e con il successivo artifact SignPath reale. Fonte: [PowerShell e precedenza delle firme di catalogo](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.security/get-authenticodesignature?view=powershell-7.5).
- [ ] **Step 2:** `Invoke-Pester -Path scripts/tests/VerifySignatures.Tests.ps1 -ExcludeTagFilter Integration -CI` → FAIL.
- [ ] **Step 3: implementa** le funzioni, lo script e il file dei certificati.
- [ ] **Step 4: metadati del servizio.**
  - `<Product>OpenMonitor Advanced</Product>` nel csproj.
  - In `build-installer-payload.ps1`, accanto al controllo di `FileVersion` (righe 138-141), `ProductName -ne 'OpenMonitor Advanced'` → `Fail`.
  - Prima aggiungere i test per il gate `ProductName` (caso valido/errato e metadati mancanti), con accesso ai metadati iniettabile o helper puro su `VersionInfo`. Rendere i pin PawnIO riutilizzabili dal verificatore senza importare/eseguire lo script di build, usando `OmaPawnIoPins.psm1`; dimostrare che gli stessi pin sono consumati da build/verifica.
  - `pwsh scripts/build-installer-payload.ps1` → exit 0; `dotnet test service/OpenMonitorAdvanced.slnx` → verde.
- [ ] **Step 5:** `Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI` → PASS.
- [ ] **Step 6: commit** `feat(release): verify signatures and payload of the installer`.

---

### Task 5: note di rilascio

**Files:**
- Create: `.github/release-notes-template.md`, `scripts/lib/OmaReleaseNotes.psm1`, `scripts/render-release-notes.ps1`, `scripts/tests/ReleaseNotes.Tests.ps1`

**Interfaces:**
- Produces:
  ```powershell
  function New-OmaReleaseNotes { param([string]$TemplatePath, [string]$Version, [bool]$Signed) }           # -> [string]
  function Update-OmaReleaseNotes { param([string]$ExistingBody, [string]$TemplatePath, [string]$Version, [bool]$Signed) } # -> [string]
  ```
  `render-release-notes.ps1 -Version X.Y.Z -Signed:$true|$false -Out <file> [-Existing <file>]`.
- **Template** (inglese, marcatori L6):
  - blocco `changes` con `<!-- Write the changes here -->`;
  - blocco `generated` con `## Install`:
    - avvio;
    - `/S` per l'installazione silenziosa e `/NOSENSORS` per escludere il componente dei sensori avanzati;
    - `{{SIGNING}}`: l'attribuzione se firmato, la riga non firmata dei Global Constraints se no;
    - link a `CODE_SIGNING.md`;
  - `## Verify your download`:
    - `Get-FileHash .\OpenMonitor.Advanced_{{VERSION}}_x64-setup.exe -Algorithm SHA256`, da confrontare con `SHA256SUMS.txt`;
    - `gh attestation verify OpenMonitor.Advanced_{{VERSION}}_x64-setup.exe --repo Cioscos/OpenMonitorAdvanced`.

- [ ] **Step 1: test che falliscono:**
  - `renders_signed_variant`: contiene l'attribuzione, non la riga non firmata;
  - `renders_unsigned_variant`: il contrario;
  - `replaces_every_placeholder`: nessun `{{` residuo;
  - `update_switches_the_generated_block_and_keeps_changes`: un corpo con novità scritte a mano e blocco non firmato, aggiornato come firmato, conserva le novità byte per byte;
  - `update_rejects_missing_markers`, `update_rejects_duplicate_markers`, `update_rejects_wrong_order`.
- [ ] **Step 2:** `Invoke-Pester -Path scripts/tests/ReleaseNotes.Tests.ps1 -CI` → FAIL.
- [ ] **Step 3: implementa** il modulo, lo script e il template.
- [ ] **Step 4:** stesso comando → PASS.
- [ ] **Step 5: commit** `feat(release): render draft release notes`.

---

### Task 6: preflight e bozza di release

**Files:**
- Create: `scripts/lib/OmaRelease.psm1`, `scripts/release-preflight.ps1`, `scripts/publish-draft.ps1`, `scripts/tests/Release.Tests.ps1`

**Interfaces:**
- Consumes: `Invoke-OmaNative`, `Get-OmaSha256`; `New-OmaReleaseNotes` e `Update-OmaReleaseNotes` (Task 5).
- Produces:
  ```powershell
  function Get-OmaSigningMode { param([bool]$HasToken, [string]$OrganizationId, [string]$RequireSigning) } # -> 'enabled' | 'disabled'; throws on partial or required-but-missing
  function Assert-OmaRunRef { param([string]$EventName, [string]$Ref) }  # push: refs/tags/v*; workflow_dispatch: refs/heads/main; otherwise throws
  function Assert-OmaCiGreen { param([string]$Repo, [string]$Sha, [scriptblock]$Gh) }   # L7
  function Get-OmaReleaseState { param([string]$Repo, [string]$Tag, [scriptblock]$Gh) } # -> 'absent' | 'draft' | 'published'
  function Get-OmaFinalSetupName { param([string]$Version) }            # -> "OpenMonitor.Advanced_$Version`_x64-setup.exe"
  function Write-OmaSha256Sums { param([string]$Setup, [string]$Out) }   # "<sha256 lowercase>  <file name>\n"
  function Publish-OmaDraft { param([string]$Repo, [string]$Tag, [string]$Version, [string]$Setup, [string]$Sums,
      [bool]$Signed, [string]$TemplatePath, [scriptblock]$Gh) }
  ```
  - **`release-preflight.ps1`:** con `-EventName`, `-Ref`, `-Sha`, `-Repo`, `-HasToken`, `-OrganizationId`, `-RequireSigning`, `[-Tag]`. `HasToken` è uno string enum `true|false` convertito esplicitamente: `[bool]'false'` in PowerShell è true. `RequireSigning` assente/vuoto significa false prima dell'attivazione; `true|false` si convertono esplicitamente e valori sconosciuti falliscono. Esegue `Assert-OmaRunRef` (inclusa regex/versione numerica sui tag), `Get-OmaSigningMode`, `Assert-OmaCiGreen` e, per i tag, `Get-OmaReleaseState ≠ 'published'`. Scrive `signing=enabled|disabled`, `signpath-policy=release-signing|test-signing` e `verify-policy=release|test|none` in `$env:GITHUB_OUTPUT`: gli slug SignPath non sono i nomi ammessi da `verify-signatures.ps1`.
  - **Trasporto GitHub:** l'adapter `Gh` usa `Invoke-OmaNative`, restituisce risultato/exit code strutturati e separa stderr dal JSON. `Get-OmaReleaseState` distingue assenza confermata (lookup autorizzato con 404/risultato assente) da errori 401/403/5xx, rete o JSON invalido, che falliscono. Tutti i comandi release specificano `--repo <Repo>`; tutti i `gh api` specificano il repository nel path e `--method GET` per le query con `-f`, oltre alla paginazione dove necessaria. Usare `gh release view` per trovare anche la bozza, senza dedurla dal solo endpoint REST dei tag pubblicati.
  - Con firma `enabled`, il preflight richiede anche configurazione del progetto/slug SignPath e certificati L4 completi per la policy selezionata: un placeholder o una lista vuota fallisce prima della build/richiesta. Non serve interrogare SignPath né esporre il token nel preflight; la configurazione remota viene confrontata durante l'attivazione.
  - **`publish-draft.ps1`:** stessi parametri di `Publish-OmaDraft`, con exit code.
- **Regole di `Publish-OmaDraft`:**
  - **`absent`:** `gh release create <tag> --draft --verify-tag --title "OpenMonitor Advanced <v>" --notes-file <file> <setup> <sums>`.
  - **`draft`:**
    - legge il corpo (`gh release view --json body,isDraft`) e lo aggiorna con `Update-OmaReleaseNotes`: marcatori non validi → errore **prima** di toccare gli asset;
    - ricontrolla `isDraft` subito prima di ogni scrittura;
    - `gh release edit --notes-file`, poi `gh release upload --clobber` dei due asset.
  - **`published`:** errore `release <tag> is already published`.
  - **Alla fine:** `gh release view --json assets` deve elencare esattamente i due nomi, stato `uploaded`, dimensioni attese e `digest` `sha256:<hash>` uguali ai file locali. Si verifica anche l'hash di `SHA256SUMS.txt` e che il suo contenuto nomini il setup corretto. Alcune versioni di gh/API possono restituire digest null: rilevare `gh --version`, usare REST con versione API esplicita oppure scaricare in temporanei i soli asset della bozza via API autenticata e confrontare gli hash. Un digest mancante non diventa né successo implicito né fallimento irrecuperabile della pipeline. Fonte: [esportazione degli asset di GitHub CLI](https://github.com/cli/cli/blob/trunk/pkg/cmd/release/shared/fetch.go). Qualsiasi mismatch fallisce con `remote assets do not match; do not publish this draft, re-run the workflow`.
  - Le scritture non sono atomiche e un `isDraft` letto prima non elimina una race con la pubblicazione manuale: resta obbligatorio non pubblicare durante un run, come nella spec. In caso di errore parziale non si pubblica, non si cancellano indiscriminatamente asset e non si dichiara la bozza pronta. Le note vanno in un file temporaneo UTF-8/LF e passano con `--notes-file`, mai attraverso concatenazione shell.

- [ ] **Step 1: test che falliscono** (`Gh` finto che registra le chiamate e restituisce JSON preparati):
  - `disabled_when_both_missing`;
  - `partial_configuration_fails` (due casi);
  - `required_signing_without_credentials_fails`;
  - `false_env_string_is_not_true`, `invalid_boolean_configuration_fails`, `preflight_maps_signpath_and_verify_policies`;
  - `dispatch_from_other_branch_fails`;
  - `push_of_a_branch_fails`;
  - `ci_gate_requires_every_job` (un job `skipped` → errore);
  - `ci_gate_fails_without_completed_run`;
  - `ci_gate_rejects_old_green_when_latest_attempt_failed`, `ci_gate_requires_main_push_of_same_repo`, `ci_gate_reads_paginated_jobs`;
  - `final_setup_name_uses_dots`;
  - `sums_format_is_sha256sum` (due spazi, LF finale, hash minuscolo);
  - `creates_a_new_draft`;
  - `updates_an_existing_draft_keeping_changes`;
  - `refuses_a_published_release`;
  - `rechecks_draft_before_writing` (`isDraft` diventa false tra la lettura e la scrittura → errore, nessun upload);
  - `invalid_markers_fail_before_upload`;
  - `verifies_remote_assets_after_upload`;
  - `null_digest_downloads_and_hashes_both_assets`, `api_failure_is_not_absent_release`, `partial_upload_is_reported`;
  - `preflight_writes_outputs`.
- [ ] **Step 2:** `Invoke-Pester -Path scripts/tests/Release.Tests.ps1 -CI` → FAIL.
- [ ] **Step 3: implementa** il modulo e i due script.
- [ ] **Step 4:** `Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI` → PASS.
- [ ] **Step 5: commit** `feat(release): add release preflight and draft publishing`.

---

### Task 7: CI — composite action, job `scripts` e `actionlint`

**Files:**
- Create: `.github/actions/setup-toolchain/action.yml`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces:
  - la composite action `./.github/actions/setup-toolchain`, senza input. Fa:
    - `rustup show`;
    - `Swatinem/rust-cache`;
    - `pnpm/action-setup` (versione 10);
    - `actions/setup-node` (Node 22, cache pnpm, `cache-dependency-path: app/pnpm-lock.yaml`);
    - `actions/setup-dotnet` (`global-json-file: global.json`).

    Ogni `uses:` è fissato allo SHA (L10) e ogni `run:` ha `shell: pwsh`.
  - i job CI `scripts` e `actionlint`, i cui nomi richiede il gate L7.
- **Job `scripts`** (`windows-latest`, `shell: pwsh`):
  0. checkout del repository e setup .NET da `global.json` per costruire la fixture PE del test Integration;
  1. installa Pester 5.7.1 se `Get-Module -ListAvailable Pester | Where-Object Version -eq '5.7.1'` è vuoto;
  2. `Import-Module Pester -RequiredVersion 5.7.1`;
  3. `Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI`;
  4. in un altro step/processo `pwsh`, `Import-Module Pester -RequiredVersion 5.7.1; Invoke-Pester -Path scripts/tests -TagFilter Integration -CI`, con ambiente isolato dichiarato.

  I test con tag `Integration` girano anche qui: il runner è effimero.
- **Job `actionlint`** (`ubuntu-latest`):
  - checkout del repository;
  - scarica il tarball di una versione fissata dalle release di `rhysd/actionlint`;
  - verifica lo SHA-256 contro il valore scritto nel workflow;
  - esegue `./actionlint -color`.

  Commento nel workflow: actionlint non verifica gli step interni della composite action.
  Aggiungere validazione dello schema `action.yml` della composite action e verifica esplicita degli input/shell/path dei suoi step; il collaudo CI nei job consumatori resta obbligatorio. I checksum del tarball Linux e dello zip Windows sono pin distinti: stessa versione di actionlint non significa stesso hash tra piattaforme.
- **Job `installer` e `checks`:** usano la composite action al posto dei loro step di setup, nient'altro.

- [ ] **Step 1: modifica** i due file.
- [ ] **Step 2: verifica locale:**
  - scaricare la stessa versione di actionlint per Windows in `target/tools/` (con il pin SHA-256 dello specifico archivio Windows) ed eseguirla dalla radice → nessun errore;
  - `Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI` → PASS.
- [ ] **Step 3: commit** `ci: share the toolchain setup and run script tests and actionlint`.

La prova reale di questo task avviene su GitHub nel Task 10, dopo il push dell'utente.

---

### Task 8: workflow di release e configurazioni SignPath

**Files:**
- Create: `.github/workflows/release.yml`, `.signpath/artifact-configuration-binaries.xml`, `.signpath/artifact-configuration-setup.xml`

**Interfaces:**
- Consumes: tutti gli script dei Task 2-6, la composite action del Task 7, l'esito dello spike (L8).
- **Trigger e impostazioni:**
  - `on: push: tags: ['v*.*.*']` e `workflow_dispatch`;
  - `permissions: contents: read` a livello di workflow;
  - `concurrency: { group: release-${{ github.ref }}, cancel-in-progress: false }`.
- **Job `release`:**
  - `runs-on: windows-latest`, `timeout-minutes: 240`, `environment: release`;
  - `permissions: { actions: read, contents: write, id-token: write, attestations: write }`;
  - `if: (github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')) || (github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main')`.
- **Step in ordine** (valori di contesto passati come `env:`, mai interpolati nel testo di `run:`):
  1. `actions/checkout` con `fetch-depth: 0` e `persist-credentials: false`;
  2. `release-preflight.ps1` (`id: pre`), con presenza del token calcolata come stringa `true|false` in `env`; passa esplicitamente `vars.SIGNPATH_ORGANIZATION_ID`, `vars.REQUIRE_SIGNING`, evento/ref/SHA/repo. `GH_TOKEN` solo qui e negli step `gh`, il token SignPath soltanto alle due azioni di firma;
  3. composite action (installa le toolchain prima dei comandi cargo/dotnet); `check-version.ps1` con contesto SHA del run e `-Tag` solo sui tag;
  4. `pnpm install --frozen-lockfile` in `app/`; `build-installer-payload.ps1`;
  5. `sign-shim.ps1 -Mode init` (`StateRoot` = `$env:GITHUB_WORKSPACE/target/signing`) con commit/versione/run id/attempt espliciti, poi `register-service` su `target/installer-payload/service/oma-service.exe`;
  6. prima passata secondo L8 con `--config ../target/signing/tauri.sign.collect.json`, poi `sign-shim.ps1 -Mode check -Pass collect`;
  7. solo con `steps.pre.outputs.signing == 'enabled'`:
     1. `upload-artifact` dei tre path espliciti in `target/signing/unsigned/` (`oma-app.exe`, `uninstall.exe`, `oma-service.exe`), layout piatto ZIP, nome `binaries-${{ github.run_id }}-${{ github.run_attempt }}`, `if-no-files-found: error`; nessun secondo path al servizio nel payload;
     2. richiesta SignPath con `id: sign-binaries`, `api-token` dal secret, `organization-id` dalla variabile, `project-slug` fissato per il progetto approvato, `artifact-configuration-slug: binaries`, `signing-policy-slug` dall'output `signpath-policy`, `github-artifact-id` dall'upload specifico, `wait-for-completion: true`, timeout 5400, directory assoluta `returned-binaries` inizialmente vuota e parametri `version`/metadati serializzati come stringhe JSON valide;
     3. `verify-signatures.ps1 -Files <returned-binaries> -Version <v> -Policy <verify-policy>` con la policy scelta e tutti i metadati/firme verificati;
     4. `sign-shim.ps1 -Mode import-signed`;
     5. copia di `signed/oma-service.exe` sul payload;
     6. seconda passata `pnpm tauri bundle --bundles nsis --config ../target/signing/tauri.sign.apply.json`, poi `check -Pass apply`;
     7. copia del setup in `target/signing/setup/`;
     8. `upload-artifact` dell'unico setup atteso con nome `setup-${{ github.run_id }}-${{ github.run_attempt }}` e `if-no-files-found: error`; richiesta SignPath con `id: sign-setup`, **tutti** gli stessi input comuni del punto 2, il proprio `artifact-id`, configurazione `setup`, timeout 5400 e directory assoluta `returned-setup` vuota. Verificare esattamente un nome atteso al ritorno;
  8. copia del setup finale (firmato: dall'unico file atteso in `returned-setup`; non firmato: path della prima passata registrato nel manifest) in `target/signing/final/<Get-OmaFinalSetupName>`; esporre path assoluto, versione e contesto tramite output/variabili per tutti gli step successivi, senza glob;
  9. `verify-signatures.ps1 -Setup <final> -Manifest <manifest> -Version <v> -Policy <verify-policy>`;
  10. `Write-OmaSha256Sums`;
  11. `actions/attest` (`subject-path` = il setup finale), poi `gh attestation verify` sullo stesso file;
  12. tag: `publish-draft.ps1`; dispatch: `upload-artifact` del setup e di `SHA256SUMS.txt`;
  13. riepilogo con `if: always()` in `$env:GITHUB_STEP_SUMMARY`: esito, modalità di firma, policy, eventuale SHA-256 del setup, id/URL delle richieste SignPath e verifiche rimaste manuali. Se firma/upload falliscono, indicare la richiesta pendente o la bozza parziale da recuperare; non dichiarare artifact pronti quando mancano.
  Tutti i passi hanno `shell: pwsh` dove necessario e exit code espliciti dei comandi nativi. Le due azioni SignPath hanno input documentati identici salvo artifact/config/output ([documentazione SignPath](https://docs.signpath.io/trusted-build-systems/github)).
- **Artifact configuration** (radice `<zip-file>`, nessuna wildcard):
  - wrapper `<artifact-configuration xmlns="http://signpath.io/artifact-configuration/v1">`, blocco `<parameters>` con parametri obbligatori dichiarati e poi `<zip-file>`; "radice zip" descrive il contenitore, non elimina il wrapper XML;
  - `binaries`: **tre elementi separati** `<pe-file>` con path esatti `oma-app.exe`, `uninstall.exe`, `oma-service.exe` (il carattere `|` non è un'alternativa ammessa nel path), vincoli prodotto/versione nel formato rilevato per ciascun ruolo, direttiva `<authenticode-sign/>` e timestamp gestito da SignPath;
  - `setup`: un `<pe-file>` con nome esatto parametrizzato sul nome/versione dell'installer e gli stessi vincoli di prodotto/versione/firma. Distinguere `version=X.Y.Z` per il nome da eventuali parametri `peVersion=X.Y.Z.0`/ProductVersion per i metadati; non cambiare il nome del setup per adattarlo alla versione PE.

  Ognuno ha un commento in testa: "copy of the active SignPath configuration; keep in sync (docs/release.md)". Validare XML/parametri contro lo schema ufficiale fissato e collaudare le configurazioni attive su artifact di esempio; campi obbligatori mancanti/file extra devono essere rifiutati. Fonte: [schema e struttura SignPath](https://docs.signpath.io/artifact-configuration/syntax).

- [ ] **Step 1: scrivi** `release.yml` e i due XML.
- [ ] **Step 2: verifica locale:**
  - actionlint (come nel Task 7) → nessun errore;
  - revisione di sicurezza di `release.yml` e composite action: niente interpolazione di `github.*` in `run:`, token solo dove serve, policy distinte corrette, artifact-id/input/timeout di entrambe le firme completi, nessun percorso non firmato quando `signing == enabled`. Usare `security-review` se disponibile; la sua assenza non blocca la revisione manuale dei requisiti;
  - verificare lo schema della composite action e degli XML, poi provare localmente la sequenza non firmata completa (preflight GitHub escluso dal solo harness locale, non dal workflow) fino a setup finale, manifest, verifica payload e checksum. Un lint verde non dimostra che la pipeline funzioni.
- [ ] **Step 3: commit** `ci: add the release workflow and SignPath configurations`.

---

### Task 9: documentazione

**Files:**
- Create: `CODE_SIGNING.md`, `docs/release.md`
- Modify: `README.md`, `README.it.md`, `CLAUDE.md`, `docs/follow-ups.md`, `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md` (§9 riga "Firma dei binari", §13 punto 7)

- [ ] **Step 1: `CODE_SIGNING.md`** (inglese), secondo la spec §7:
  - ruoli (@Cioscos in *Committers and reviewers* e *Approvers*);
  - stato corrispondente ai fatti (domanda prevista oppure inviata/in attesa), senza attribuzione finché la firma non è attiva;
  - cosa si firma e cosa no (D4, D5);
  - privacy verificata rispetto all'app, installer e componenti di terzi: distinguere download automatici/WebView2 e richieste esplicite dell'utente, con link alle policy pertinenti; non affermare "nessuna trasmissione salvo…" senza controllare il comportamento effettivo dei componenti;
  - come verificare una firma (`Get-AuthenticodeSignature`).
- [ ] **Step 2: `docs/release.md`** (italiano), i sette punti della spec §7, compresi:
  - quali campi di `.signpath/certificates.json` riempire dopo l'approvazione e come confermare pin/subject della leaf e radice con la configurazione SignPath; aggiunta della radice pubblica di prova e verifica `test` solo su runner/VM isolati;
  - come confrontare le artifact configuration attive con le copie in `.signpath/`;
  - come annullare una richiesta pendente prima di "Re-run all jobs";
  - `REQUIRE_SIGNING=true` dopo il primo collaudo reale firmato riuscito e prima della pubblicazione della prima release firmata;
  - il divieto di pubblicare la bozza durante il run;
  - il token SignPath di un utente con soli diritti di submitter, la protezione di `main` e la policy SignPath vincolata a repository, workflow e ref attesi (spec §4.3).
- [ ] **Step 3: README (en, it):**
  - sezione "Verify your download" (checksum e attestazione);
  - link a `CODE_SIGNING.md`;
  - la riga sul SmartScreen resta com'è.
- [ ] **Step 4: altri documenti:**
  - `CLAUDE.md`: comandi `check-version`, `bump-version`, Pester con `-ExcludeTagFilter Integration -CI` in locale, test Integration in ambiente isolato, flusso di release e M6a nell'elenco delle milestone;
  - spec principale: le due righe rimandano alla spec M6a, con "ammissione e collaudo della firma pendenti";
  - `docs/follow-ups.md`:
    - chiude `nsExec.dll` (D5);
    - aggiunge la privacy per M6c;
    - aggiunge le verifiche manuali dopo l'approvazione (spec §8.2);
    - aggiunge l'uninstaller installato, se lo spike l'ha lasciato aperto.
- [ ] **Step 5: commit** `docs: add the code signing policy and release guide`.

---

### Task 10: collaudo con l'utente e chiusura

**Files:**
- Modify: questo piano ("Esito dell'esecuzione"), `docs/follow-ups.md` ("last update: M6a"); eventuale memoria personale solo se disponibile nel sistema del worker.

- [ ] **Step 1: verifiche complete in locale:**
  ```bash
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  dotnet test service/OpenMonitorAdvanced.slnx
  pwsh -c "Import-Module Pester -RequiredVersion 5.7.1; Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI"
  pwsh scripts/check-version.ps1
  ```
  Aggiungere UI `pnpm check`, `pnpm test`, `pnpm build` in `app/`, actionlint/schema, build reale del payload e installer non firmato con verifica del manifest/contenuto/checksum. Confrontare il template con `upstream-2.11.5.nsi` tramite diff locale e verificare che le variazioni siano marcate `; OMA`: exit 1 del diff significa differenze attese, exit >1 è errore. `nsis-template-drift.ps1 -To <versione>` è una preview di upgrade, non un lint del template locale, e non si usa con `-Apply` in questo collaudo. Expected: test/build/check con exit code 0. Eseguire anche il bump su una copia temporanea di un workspace Cargo minimale **reale** e verificare `cargo metadata --locked`: i soli fake non provano l'allineamento del lockfile.
- [ ] **Step 2: revisione dell'intero branch** (`superpowers:requesting-code-review`), correzioni, poi merge in `main` in locale (`superpowers:finishing-a-development-branch`).
- [ ] **Step 3: con l'utente**, un punto per volta:
  1. l'utente fa il push di `main` e controlla che la CI sia verde, compresi `scripts` e `actionlint`;
  2. crea l'environment `release` (senza secret SignPath) e il ruleset sui tag seguendo `docs/release.md`;
  3. avvia `workflow_dispatch` da `main`: atteso un artifact con il setup e `SHA256SUMS.txt`, `verify-signatures -Policy none` verde e l'attestazione verificata;
  4. decide se fare subito la prima release non firmata (per esempio la 0.3.0): bump, commit e push di `main`, attendere il verde CI su **quel nuovo SHA**, poi tag/push, bozza, controllo note/asset e pubblicazione; tag e commit spinti insieme possono far fallire il gate perché CI è ancora in corso (si riesegue dopo il verde);
  5. invia la domanda alla SignPath Foundation, se non l'ha già fatto.

  Misurare la durata di build/bundle/verifiche: le due attese da 90 minuti lasciano solo un'ora delle 4 disponibili. Senza le firme reali si misura soltanto il costo del percorso non firmato, non si dichiara collaudato il budget dell'intero percorso firmato.
- [ ] **Step 3b: collaudo differito dopo l'approvazione Foundation** (rimane pendente se l'approvazione non arriva in questa milestone):
  1. confermare l'ammissibilità del packaging e configurare progetto/policy/origin, credenziali, pin e radice pubblica di prova con un commit su `main` e CI verde;
  2. dispatch con `test-signing`: due richieste approvate, ritorni verificati, setup con tre binari propri firmati e checksum/attestazione corretti; collaudo trust e cleanup senza modifiche al PC dell'utente;
  3. release con `release-signing` fino alla **bozza**, senza pubblicazione; verificarne gli asset scaricati e le firme in una VM/Sandbox: app, servizio, uninstaller installato, UAC installazione/disinstallazione, comportamento SmartScreen;
  4. misurare il run completo, impostare `REQUIRE_SIGNING=true`, aggiornare attribuzione/README e solo dopo i controlli pubblicare manualmente la prima release firmata;
  5. provare che credenziali tolte/configurazione parziale causino un errore prima della firma, che una release pubblicata resti intatta e che le note restino coerenti nel recupero di una bozza. Le prove che mutano credenziali si fanno durante il collaudo controllato, senza run concorrenti.
- [ ] **Step 4: chiusura:**
  - "Esito dell'esecuzione" in questo piano;
  - follow-up: le verifiche manuali dovute dopo l'approvazione della Foundation, spec §8.2;
  - eventuale memoria personale solo se il sistema di memoria del worker è configurato; gli esiti verificabili rimangono in questo piano e `docs/follow-ups.md`, senza dipendere da file privati esterni al repository;
  - commit `docs: record the M6a outcome and follow-ups`.

---

## Esito dello spike

*(Da compilare nel Task 1.)*

## Esito dell'esecuzione

*(Da compilare nel Task 10.)*

## Esito della revisione del piano (2026-09-30)

| Rilievo | Correzione |
|---|---|
| Catena del certificato usata per accettare `UnknownError`, senza provare l'integrità del PE. | L5 e Task 4: verifica Authenticode reale, pin del firmatario di prova, trust temporaneo solo isolato, timestamp verificato. |
| Fixture firmava codice Windows, mancava timestamp e poteva usare una firma di catalogo. | Fixture PE propria; test crypto e policy completa distinti; Integration esclusi dal PC di lavoro. |
| Spike sostituiva anche plugin/setup, perdeva il setup A e usava mtime instabili. | Ruoli espliciti, copie A/B/C conservate, hash/log e cleanup in ogni uscita. |
| `--locked` poteva diventare una build sbloccata; fetch non aggiornava esplicitamente `origin/main`. | Relay obbligatorio a cargo e refspec di fetch esplicito. |
| Contesto manifest confrontato con sé stesso; reset dello stato senza limiti del percorso. | Contesto GitHub indipendente, controllo delle copie firmate e directory di reset verificata. |
| Booleani PowerShell da stringhe e slug SignPath confusi con policy di verifica. | Parsing `true|false`, output `signpath-policy`/`verify-policy` separati. |
| Upload dei binari/layout e input della seconda firma incompleti; XML ambiguo. | Tre path espliciti, input/timeout di entrambe le richieste completi, wrapper/parametri/schema XML. |
| Gate CI poteva accettare un vecchio verde o job di altri tentativi. | Main push dello stesso repository/SHA, ultimo run/attempt e paginazione. |
| Stato bozza/JSON API, digest null e upload parziale non coperti. | Errori distinti da assenza, controllo degli asset con fallback di download e recupero documentato. |
| Collaudo firmato richiamato ma assente dal Task 10. | Step 3b esplicito, pubblicazione subordinata al controllo dell'uninstaller installato. |
| Test payload presunti esistenti e validazioni locali incomplete. | Nuovi test dedicati, pin PawnIO condivisi, schema/drift/UI/build e bump Cargo reale. |

Le fonti primarie sono linkate nei punti pertinenti. La revisione corregge le istruzioni; non certifica la fattibilità delle due passate né sostituisce i collaudi ancora pendenti.
