# Milestone 6a — Release e firma del codice: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** una pipeline di release su GitHub Actions che, dal push di un tag `vX.Y.Z`, costruisce l'installer, lo firma con SignPath (quando attivo) e ne fa una bozza di release con `SHA256SUMS.txt` e attestazione di provenienza; senza SignPath la stessa pipeline produce release non firmate.

**Architecture:**
- **Script PowerShell** in `scripts/`, sottili, con la logica in moduli `scripts/lib/*.psm1` testati con Pester 5. I comandi esterni (`git`, `gh`, `cargo`, 7-Zip, firme) passano per parametri iniettabili.
- **Shim di firma** (`scripts/sign-shim.ps1`) agganciato a `bundle.windows.signCommand` di Tauri solo in CI. Due passate: `collect` raccoglie i file che Tauri vorrebbe firmare, `apply` li sostituisce con le copie firmate da SignPath dopo aver controllato lo SHA-256.
- **Workflow:** `release.yml` (un job sequenziale nell'environment `release`), composite action `setup-toolchain` condivisa con `ci.yml`, nuovi job CI `scripts` (Pester) e `actionlint`.
- **Documenti:** `CODE_SIGNING.md`, guida operativa `docs/release.md`, copie versionate delle artifact configuration di SignPath in `.signpath/`.

**Tech Stack:** PowerShell 7, Pester 5.7.1 (fissato), GitHub Actions (`windows-latest`, `ubuntu-latest`), `signpath/github-action-submit-signing-request`, `actions/attest`, `gh` CLI, 7-Zip, actionlint (versione fissata con checksum), Tauri CLI 2.11.5, NSIS del bundler di Tauri. Nessun cambio a Rust, UI, protocollo o logica del servizio; nel servizio cambia solo il metadato `Product`.

**Spec:** `docs/superpowers/specs/2026-09-30-m6a-release-firma-design.md` (commit `fb8f9a1`); spec principale `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md` per il resto.

**Decisioni del piano** (interpretazioni della spec prese qui):
- **L1. Moduli e script sottili.** La logica sta in `scripts/lib/OmaCommon.psm1`, `OmaVersion.psm1`, `OmaSigning.psm1`, `OmaReleaseNotes.psm1` e `OmaRelease.psm1`. Gli script in `scripts/` fanno solo parsing dei parametri, import e codice d'uscita. I test Pester importano i moduli, e ogni script ha anche un test end-to-end via `pwsh -File`. Ogni comando esterno si esegue con `Invoke-OmaNative`, che fallisce su exit code diverso da 0.
- **L2. Stato dello shim.** `sign-shim.ps1 -Mode init` crea `target/signing/` vuota e ci scrive `manifest.json` (contesto e chiamate) e i due config Tauri `tauri.sign.collect.json` e `tauri.sign.apply.json`. Le modalità `collect` e `apply` ricevono solo `-StateRoot` e `-Path`. Tutti i percorsi nel manifest sono assoluti e normalizzati con `[IO.Path]::GetFullPath`, confrontati senza distinzione di maiuscole.
- **L3. Nome del setup.** Il setup finale si copia sempre in `target/signing/final/OpenMonitor.Advanced_X.Y.Z_x64-setup.exe`, con i punti al posto degli spazi: è il nome che GitHub dà all'asset (come per la 0.2.0), così `SHA256SUMS.txt`, attestazione e asset coincidono.
- **L4. Certificati attesi** in `.signpath/certificates.json`: `release.subject` (DN esatto), `release.thumbprints` e `test.rootThumbprints` (liste). Una lista vuota fa fallire la policy corrispondente con il messaggio "no approved certificate configured for policy <p>". Un rinnovo si fa aggiungendo un thumbprint con un commit.
- **L5. Policy `test` senza toccare gli store di Windows.** Con stato `UnknownError`, `verify-signatures` costruisce una `X509Chain` del certificato del firmatario e accetta **solo** se l'unico problema della catena è `UntrustedRoot` e la radice ha un thumbprint in `test.rootThumbprints`. Qualsiasi altro stato fallisce.
- **L6. Marcatori delle note.** `<!-- oma:changes:start -->` / `<!-- oma:changes:end -->` racchiudono le novità scritte a mano; `<!-- oma:generated:start -->` / `<!-- oma:generated:end -->` il blocco tecnico. Ciascuno deve comparire esattamente una volta, nell'ordine changes → generated.
- **L7. Gate CI.** Il preflight chiede a `gh api` i run di `ci.yml` con `head_sha` uguale allo SHA del run di release e richiede almeno un run `completed` con `conclusion: success` in cui i job `checks`, `service`, `installer`, `scripts` e `actionlint` sono tutti `success` (dall'endpoint dei job del run; i job di `ci.yml` non hanno `name:`, quindi il nome coincide con l'id).
- **L8. `--locked`.** La prima passata usa `pnpm tauri build --bundles nsis -- --locked` se lo spike conferma che l'argomento arriva a cargo. Altrimenti si usa `pnpm tauri build --bundles nsis`, preceduto da `cargo metadata --locked` (già in `check-version`) e seguito da `git diff --exit-code Cargo.lock`.
- **L9. Riconoscimento dell'uninstaller e dei plugin.** Le regex `UninstallerPathPattern` e `PluginPathPattern` si ricavano dallo spike (Task 1) e si scrivono nella sezione "Esito dello spike" di questo piano. Il Task 2 le usa come costanti di `OmaSigning.psm1`.
- **L10. Versioni delle azioni.** In `release.yml` e nella composite action ogni `uses:` è fissato allo SHA completo, con commento `# vX.Y.Z`. `ci.yml` conserva i tag che ha, salvo ciò che passa alla composite action. L'implementer ricava gli SHA con `gh api repos/<owner>/<repo>/git/ref/tags/<tag>` (e dereferenzia i tag annotati).

## Global Constraints

- **Branch:** `feat/m6a-release-firma` da `main` (`fb8f9a1` o successivo), aperto con `superpowers:using-git-worktrees`; merge in `main` in locale; **push solo su richiesta dell'utente**.
- **Build locali invariate:** `pnpm tauri build` senza `--config` si comporta come oggi. Lo shim entra solo con i config generati da `sign-shim.ps1 -Mode init`.
- **Template NSIS:** l'unica modifica è `!uninstfinalize '${UNINSTALLERSIGNCOMMAND}' = 0`, con commento `; OMA` sulla stessa riga. Nient'altro, a meno che lo spike non imponga il fallback, e in quel caso ci si ferma e si torna dall'utente.
- **Mai firmare codice di terzi:** plugin NSIS, `PawnIO_setup.exe` e runtime .NET non entrano negli artifact inviati a SignPath.
- **Nessun bypass dei controlli di firma nel workflow:** nessun parametro o variabile disattiva il confronto SHA-256, la verifica delle firme o il gate CI.
- **Stato dello shim in `target/signing/`,** ricreato vuoto a ogni run, mai in cache.
- **Versioni:** `X.Y.Z` con regex `^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$`, ogni componente ≤ 65535; versione PE `X.Y.Z.0`.
- **Testi fissi:**
  - attribuzione: `Free code signing provided by SignPath.io, certificate by SignPath Foundation`;
  - riga non firmata: `The installer is not code-signed yet, so Windows SmartScreen may warn you: choose *More info* → *Run anyway*.`
- **Attese SignPath:** `wait-for-completion-timeout-in-seconds: 5400`; job `timeout-minutes: 240`.
- **Stile:** codice, commenti, commit, `CODE_SIGNING.md` e template delle note in inglese (conventional commits); `docs/release.md` e prosa dei piani in italiano con gli accenti; fine riga LF.
- **Comandi:** PowerShell 7, dalla radice del repository; per i comandi UI `Push-Location app` e `try { … } finally { Pop-Location }`.
- **Pester in locale:** `Install-Module Pester -Scope CurrentUser -RequiredVersion 5.7.1 -Force -SkipPublisherCheck`, poi `Import-Module Pester -RequiredVersion 5.7.1`. I test si lanciano con `Invoke-Pester -Path scripts/tests -CI`.
- **Verifiche dal vivo:** mai input sintetico sul desktop dell'utente. Installazioni di prova solo in Windows Sandbox o in una VM, e le avvia l'utente.
- **graphify e subagent:**
  - dopo ogni task che tocca il codice: `$env:PYTHONHASHSEED = '0'; graphify update .`;
  - nei brief dei subagent vanno `graphify query`, `graphify explain`, `graphify path` e le skill `superpowers:test-driven-development` e `superpowers:systematic-debugging`;
  - la documentazione di Tauri, Pester e GitHub Actions si consulta con context7 o Microsoft Learn, non a memoria.

## Review Focus

Condizioni che la spec implica e che i test di funzionalità da soli non coprirebbero, in ordine di probabilità. Ogni riga ha i suoi test nel task indicato.

1. **Un aggiornamento di Tauri cambia i file passati a `signCommand`** (un plugin nuovo, un sidecar, un altro percorso temporaneo dell'uninstaller). Ci si aspetta che la release si fermi con un messaggio che nomina il file, non che firmi o salti qualcosa in silenzio. Test: `rejects_an_unexpected_path`, `rejects_a_second_uninstaller`, `check_fails_when_the_uninstaller_was_never_called` (Task 2).
2. **Rerun dopo un'approvazione scaduta o un errore a metà upload:** nessuna bozza con asset misti (setup firmato con note "non firmato", o checksum di un altro setup), nessuna modifica a una release già pubblicata. Test: `update_switches_the_generated_block_and_keeps_changes` (Task 5), `refuses_a_published_release`, `verifies_remote_assets_after_upload` (Task 6).
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
- **Test:** `scripts/tests/{Common,SignShim,Version,VerifySignatures,ReleaseNotes,Release}.Tests.ps1`, dati in `scripts/tests/fixtures/`.
- **CI e release:**
  - `.github/actions/setup-toolchain/action.yml`, `.github/workflows/release.yml`, `.github/release-notes-template.md` (nuovi);
  - `.github/workflows/ci.yml` (modificato).
- **SignPath:** `.signpath/artifact-configuration-binaries.xml`, `.signpath/artifact-configuration-setup.xml`, `.signpath/certificates.json`.
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

Nessun codice da tenere: gli script dello spike vivono in `target/spike/`, che non si committa. Il risultato è la sezione "Esito dello spike" di questo piano. Se anche uno dei punti 1-3 fallisce, **ci si ferma**: si riporta all'utente e si aggiorna la spec (fallback con pre-generazione dell'uninstaller) prima di qualsiasi altro task. I task 3-6 non dipendono dallo spike e, in quel caso, possono procedere.

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
  - In `replace` confronta lo SHA-256 con la registrazione della passata `record` per lo stesso ruolo. Se coincide, sovrascrive il file con la copia registrata più 4096 byte `0x4F`: una "firma" finta di dimensione diversa.
  - Con `-FailOn uninstaller` esce con 1 sulla chiamata dell'uninstaller.

  Il config `sign-log.json` usa la forma strutturata `{"bundle":{"windows":{"signCommand":{"cmd":"pwsh","args":[...,"%1"]}}}}` con percorsi assoluti.
- [ ] **Step 2: passata A.**
  - `pwsh scripts/build-installer-payload.ps1`.
  - Poi, in `app/`: `pnpm tauri build --bundles nsis --config ../target/spike/sign-log.json -- --locked` (`record`, output in `target/spike/A`).
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
    - che `tauri bundle` non ha ricompilato Rust, la UI o il servizio (orari di modifica di `target/release/oma-app.exe` e del payload invariati).
- [ ] **Step 4: passata C, tempo e temporanei diversi.** Almeno 2 minuti dopo, con `$env:TEMP` e `$env:TMP` puntati a `target/spike/tmp-c`, ripetere lo Step 3. Stesso esito atteso.
- [ ] **Step 5: contenuto del setup.**
  - `& 'C:\Program Files\7-Zip\7z.exe' l -slt <setup>` sui setup di A e B. In CI 7-Zip è `7z` nel PATH del runner.
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
  - Se la decisione è **fallback**, fermarsi e riportare all'utente.
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
  function Invoke-OmaNative { param([string]$FilePath, [string[]]$ArgumentList, [string]$WorkingDirectory) } # -> [string[]] stdout; throws "<FilePath> exited with <code>: <stderr>"
  function Get-OmaSha256 { param([string]$Path) }            # -> lowercase hex string
  function Resolve-OmaPath { param([string]$Path) }          # -> [IO.Path]::GetFullPath, no trailing separator
  # OmaSigning.psm1
  function Initialize-OmaSigningState { param([string]$StateRoot, [string]$RepoRoot, [string]$Commit, [string]$Version, [string]$RunId, [string]$RunAttempt) }
      # recreates $StateRoot empty; writes manifest.json, tauri.sign.collect.json, tauri.sign.apply.json
  function Invoke-OmaSignShim { param([ValidateSet('collect','apply')][string]$Mode, [string]$StateRoot, [string]$Path) }
  function Register-OmaService { param([string]$StateRoot, [string]$Path) }            # collect-only, role 'service'
  function Import-OmaSignedFiles { param([string]$StateRoot, [string]$From) }           # exact set oma-app.exe, uninstall.exe, oma-service.exe -> signed/, records signed sha256
  function Assert-OmaSigningPass { param([string]$StateRoot, [ValidateSet('collect','apply')][string]$Pass) }
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
  `sign-shim.ps1` parametri: `-Mode init|collect|apply|register-service|import-signed|check`, `-StateRoot`, `-Path`, `-From`, `-Pass`, più quelli di init (`-Commit`, `-Version`, `-RunId`, `-RunAttempt`). Exit 0 solo in caso di successo; il messaggio d'errore va su stderr.
- **Regole:**
  - **`init`:** svuota e ricrea `StateRoot`, poi scrive i config. `signCommand` è in forma strutturata: `pwsh -NoProfile -NonInteractive -File <abs>/scripts/sign-shim.ps1 -Mode <m> -StateRoot <abs> -Path %1`.
  - **Riconoscimento del ruolo:**
    - `app` se il percorso è `expected.app`;
    - `setup` se è `expected.setupDir/expected.setupName`;
    - `plugin` se corrisponde a `PluginPathPattern` e il nome è uno dei cinque della spec §3.1;
    - `uninstaller` se corrisponde a `UninstallerPathPattern`;
    - qualunque altro percorso: errore `unexpected file passed to signCommand: <path>`.
  - **Duplicati:** un secondo `app`, `uninstaller` o `setup` nella stessa passata è un errore `duplicate <role> call`.
  - **`collect`:** copia `app` e `uninstaller` in `unsigned/<name>` e registra tutto senza modificare il file. Per i `plugin` registra lo SHA-256 prima e dopo, e i due devono coincidere.
  - **`apply`:**
    - `app` e `uninstaller`: lo SHA-256 dev'essere uguale a quello della passata `collect`, altrimenti errore `hash mismatch for <name>: collected <a>, received <b>`. Poi sovrascrive con `signed/<name>`, e se manca è un errore `signed copy missing for <name>`;
    - `setup` e `plugin`: solo registrati.
  - **`check`:** in `collect` richiede esattamente un `app`, un `uninstaller`, un `setup` e un `service`; in `apply` un `app`, un `uninstaller` e un `setup`, ciascuno con SHA-256. Il manifest deve avere `schema: 1` e gli stessi `commit`, `version`, `runId` e `runAttempt` del contesto; altrimenti errore.
  - **`import-signed`:** richiede in `From` esattamente i tre nomi, niente di più e niente di meno, prima di copiare. La verifica della firma è a parte (Task 4) e il workflow la esegue prima dell'import.

- [ ] **Step 1: test che falliscono** (`Common.Tests.ps1`, `SignShim.Tests.ps1`, con file finti di pochi byte in `$TestDrive`):
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
  - `works_from_another_directory_with_spaces`: `StateRoot` e file sotto `$TestDrive/dir con spazi/`, eseguito con `pwsh -File` da un'altra directory corrente;
  - `script_exit_codes`: 0 in caso di successo, diverso da 0 in caso d'errore, via `pwsh -File`.
- [ ] **Step 2:** `Invoke-Pester -Path scripts/tests/Common.Tests.ps1, scripts/tests/SignShim.Tests.ps1 -CI` → FAIL (moduli mancanti).
- [ ] **Step 3: implementa** `OmaCommon.psm1`, `OmaSigning.psm1` e `sign-shim.ps1` secondo le Interfaces. Le scritture del manifest passano da un file temporaneo e poi un rename.
- [ ] **Step 4:** stesso comando → PASS.
- [ ] **Step 5: gate dell'uninstaller.**
  - Riga 101 di `installer.nsi`: `  !uninstfinalize '${UNINSTALLERSIGNCOMMAND}' = 0 ; OMA`.
  - Verifica: in locale `init` in `target/signing-local`, poi `pnpm tauri build --bundles nsis --config ../target/signing-local/tauri.sign.collect.json`, poi `pwsh scripts/sign-shim.ps1 -Mode check -Pass collect -StateRoot …`. Prima del `check` si registra il servizio con `-Mode register-service`. Atteso: exit 0 e un setup prodotto.
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
  function Test-OmaVersionConsistency { param([string]$RepoRoot, [string]$Tag, [scriptblock]$Cargo, [scriptblock]$Git) } # -> [string[]] problems (empty = ok)
  function Set-OmaVersion { param([string]$RepoRoot, [string]$Version, [scriptblock]$Cargo) } # bump with backup/rollback
  ```
  - **Campi letti:**
    - `Cargo.toml` → `version = "X.Y.Z"` nella sezione `[workspace.package]`;
    - `app/package.json` e `app/src-tauri/tauri.conf.json` → `"version": "X.Y.Z"` di primo livello;
    - `README.md` → `**Status:** … (version X.Y.Z)`;
    - `README.it.md` → `**Stato:** … (versione X.Y.Z)`.
  - **`check-version.ps1 [-Tag vX.Y.Z]`:** stampa ogni problema su una riga ed esce con 1 se ce n'è almeno uno.
  - **`bump-version.ps1 X.Y.Z`:** esce con 0 e stampa i tre comandi git della spec §6.2.
- **Regole di `Test-OmaVersionConsistency`:**
  - versioni diverse o campi mancanti: una riga per file;
  - `cargo metadata --locked --format-version 1 --no-deps` eseguito dalla radice e fallito;
  - con `-Tag`:
    - il tag non corrisponde alla regex `^v…$` della spec o non è `v` più la versione;
    - `git fetch origin main` fallisce;
    - `git rev-parse <tag>^{commit}` è diverso da `git rev-parse HEAD`;
    - `git merge-base --is-ancestor <sha> origin/main` fallisce.
- **Regole di `Set-OmaVersion`:**
  - prima di scrivere verifica che la consistenza corrente sia vuota di problemi e che la nuova versione sia maggiore in confronto numerico;
  - salva i cinque file e `Cargo.lock` in memoria come byte;
  - riscrive solo la sottostringa della versione, conservando i fine riga LF;
  - esegue `cargo update --workspace --offline`, poi `Test-OmaVersionConsistency`;
  - su qualsiasi errore ripristina i sei file byte per byte e rilancia;
  - rifiuta se `Cargo.lock` cambia fuori dalle righe `version` dei pacchetti del workspace.

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

**Interfaces:**
- Consumes: manifest del Task 2 (`signed`); `Get-OmaSha256`, `Invoke-OmaNative`.
- Produces:
  ```powershell
  function Test-OmaSignature { param([string]$Path, [ValidateSet('release','test')][string]$Policy, [pscustomobject]$Certificates,
      [scriptblock]$SignatureProvider, [scriptblock]$ChainProvider) }  # -> [string[]] problems
  function Test-OmaPayload { param([string]$Setup, [ValidateSet('release','test','none')][string]$Policy, [string]$Manifest,
      [string]$Version, [pscustomobject]$Certificates, [scriptblock]$Extractor, [scriptblock]$SignatureProvider,
      [scriptblock]$ChainProvider, [scriptblock]$VersionInfoProvider) } # -> [string[]] problems
  ```
  `verify-signatures.ps1 -Policy release|test|none -Version X.Y.Z (-Setup <p> [-Manifest <p>] | -Files <dir>)`. Con `-Files` verifica i tre binari restituiti dalla firma n. 1 prima dell'import. Esce con 1 se ci sono problemi, e con `none` scrive `::warning::unsigned build`.
  `.signpath/certificates.json` iniziale: `{ "release": { "subject": "", "thumbprints": [] }, "test": { "rootThumbprints": [] } }`.
- **Regole:**
  - **`release`:**
    - `Status` `Valid`;
    - `SignerCertificate.Subject` uguale a `release.subject` e thumbprint nella lista;
    - `TimeStamperCertificate` presente.
  - **`test`:** `Valid`, oppure `UnknownError` con la catena come in L5, più il timestamp. Qualunque altro stato (`HashMismatch`, `NotSigned`, `NotTrusted`, `Incompatible`) fallisce.
  - **Metadati** di `oma-app.exe`, `oma-service.exe` e `uninstall.exe`: `ProductName` "OpenMonitor Advanced", versioni secondo i valori registrati nello spike.
  - **Setup:**
    - `7z x` in una cartella temporanea, con exit code controllato;
    - richiede esattamente una copia di `oma-app.exe` e di `oma-service.exe`, e di `uninstall.exe` se lo spike lo ha trovato visibile;
    - lo SHA-256 di ognuno deve coincidere con `manifest.signed`;
    - `PawnIO_setup.exe` deve avere lo SHA-256 di `app/src-tauri/nsis/pawnio.sha256` e firma `Valid`.
  - **Con `none`:** setup non vuoto, estrazione riuscita, presenza dei payload propri, PawnIO come sopra. Gli hash non si confrontano col manifest (non ci sono copie firmate), ma devono coincidere con quelli di `manifest.collect`.
  - **Uninstaller non visibile a 7-Zip:** il riepilogo contiene `installed uninstaller signature not verified`.

- [ ] **Step 1: test che falliscono** (fornitori finti che restituiscono oggetti con `Status`, `SignerCertificate`, `TimeStamperCertificate`; estrattore finto che scrive file in una cartella):
  - `release_accepts_valid_expected_signer`;
  - `release_rejects_substring_subject` (`CN=SignPath Foundation Fake`);
  - `release_rejects_unknown_thumbprint`;
  - `release_fails_without_configured_certificate`;
  - `rejects_hash_mismatch_status`, `rejects_missing_timestamp`;
  - `test_accepts_only_untrusted_root_of_configured_root`;
  - `test_rejects_untrusted_root_of_other_root`;
  - `payload_hash_must_match_manifest`;
  - `payload_missing_or_duplicate_fails`;
  - `extractor_failure_fails`;
  - `pawnio_hash_and_signature_checked`;
  - `none_policy_checks_content_and_warns`;
  - `metadata_product_name_checked`;
  - integrazione con tag `Integration`: con `New-SelfSignedCertificate -Type CodeSigningCert -CertStoreLocation Cert:\CurrentUser\My`, firmare con `Set-AuthenticodeSignature` una copia di un exe di sistema piccolo (per esempio `C:\Windows\System32\where.exe`). Poi `ChainProvider` reale, con `test.rootThumbprints` = thumbprint del certificato, verifica che la logica L5 accetti la catena e che, dopo aver alterato un byte, lo stato sia `HashMismatch` e venga rifiutato. Il certificato si rimuove in `AfterAll`. Non si tocca `Cert:\CurrentUser\Root`.
- [ ] **Step 2:** `Invoke-Pester -Path scripts/tests/VerifySignatures.Tests.ps1 -CI` → FAIL.
- [ ] **Step 3: implementa** le funzioni, lo script e il file dei certificati.
- [ ] **Step 4: metadati del servizio.**
  - `<Product>OpenMonitor Advanced</Product>` nel csproj.
  - In `build-installer-payload.ps1`, accanto al controllo di `FileVersion` (righe 138-141), `ProductName -ne 'OpenMonitor Advanced'` → `Fail`.
  - `pwsh scripts/build-installer-payload.ps1` → exit 0; `dotnet test service/OpenMonitorAdvanced.slnx` → verde.
- [ ] **Step 5:** `Invoke-Pester -Path scripts/tests -CI` → PASS.
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
  - **`release-preflight.ps1`:** con `-EventName`, `-Ref`, `-Sha`, `-Repo`, `-HasToken`, `-OrganizationId`, `-RequireSigning`, `[-Tag]`. Esegue in ordine `Assert-OmaRunRef`, `Get-OmaSigningMode`, `Assert-OmaCiGreen` e, per i tag, `Get-OmaReleaseState ≠ 'published'`. Scrive `signing=enabled|disabled` e `policy=release-signing|test-signing` in `$env:GITHUB_OUTPUT`.
  - **`publish-draft.ps1`:** stessi parametri di `Publish-OmaDraft`, con exit code.
- **Regole di `Publish-OmaDraft`:**
  - **`absent`:** `gh release create <tag> --draft --verify-tag --title "OpenMonitor Advanced <v>" --notes-file <file> <setup> <sums>`.
  - **`draft`:**
    - legge il corpo (`gh release view --json body,isDraft`) e lo aggiorna con `Update-OmaReleaseNotes`: marcatori non validi → errore **prima** di toccare gli asset;
    - ricontrolla `isDraft` subito prima di ogni scrittura;
    - `gh release edit --notes-file`, poi `gh release upload --clobber` dei due asset.
  - **`published`:** errore `release <tag> is already published`.
  - **Alla fine:** `gh release view --json assets` deve elencare esattamente i due nomi, con `digest` `sha256:<hash>` uguale a quello locale. Altrimenti errore `remote assets do not match; do not publish this draft, re-run the workflow`.

- [ ] **Step 1: test che falliscono** (`Gh` finto che registra le chiamate e restituisce JSON preparati):
  - `disabled_when_both_missing`;
  - `partial_configuration_fails` (due casi);
  - `required_signing_without_credentials_fails`;
  - `dispatch_from_other_branch_fails`;
  - `push_of_a_branch_fails`;
  - `ci_gate_requires_every_job` (un job `skipped` → errore);
  - `ci_gate_fails_without_completed_run`;
  - `final_setup_name_uses_dots`;
  - `sums_format_is_sha256sum` (due spazi, LF finale, hash minuscolo);
  - `creates_a_new_draft`;
  - `updates_an_existing_draft_keeping_changes`;
  - `refuses_a_published_release`;
  - `rechecks_draft_before_writing` (`isDraft` diventa false tra la lettura e la scrittura → errore, nessun upload);
  - `invalid_markers_fail_before_upload`;
  - `verifies_remote_assets_after_upload`;
  - `preflight_writes_outputs`.
- [ ] **Step 2:** `Invoke-Pester -Path scripts/tests/Release.Tests.ps1 -CI` → FAIL.
- [ ] **Step 3: implementa** il modulo e i due script.
- [ ] **Step 4:** `Invoke-Pester -Path scripts/tests -CI` → PASS.
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
  1. installa Pester 5.7.1 se `Get-Module -ListAvailable Pester | Where-Object Version -eq '5.7.1'` è vuoto;
  2. `Import-Module Pester -RequiredVersion 5.7.1`;
  3. `Invoke-Pester -Path scripts/tests -CI`.

  I test con tag `Integration` girano anche qui: il runner è effimero.
- **Job `actionlint`** (`ubuntu-latest`):
  - scarica il tarball di una versione fissata dalle release di `rhysd/actionlint`;
  - verifica lo SHA-256 contro il valore scritto nel workflow;
  - esegue `./actionlint -color`.

  Commento nel workflow: actionlint non verifica gli step interni della composite action.
- **Job `installer` e `checks`:** usano la composite action al posto dei loro step di setup, nient'altro.

- [ ] **Step 1: modifica** i due file.
- [ ] **Step 2: verifica locale:**
  - scaricare la stessa versione di actionlint per Windows in `target/tools/` (con lo stesso controllo dello SHA-256) ed eseguirla dalla radice → nessun errore;
  - `Invoke-Pester -Path scripts/tests -CI` → PASS.
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
  2. `release-preflight.ps1` (`id: pre`), con `HasToken: ${{ secrets.SIGNPATH_API_TOKEN != '' }}` calcolato in `env` e mai stampato; `GH_TOKEN: ${{ github.token }}` solo qui e negli step `gh`;
  3. `check-version.ps1` (con `-Tag $env:GITHUB_REF_NAME` solo sui tag);
  4. composite action; `pnpm install --frozen-lockfile` in `app/`; `build-installer-payload.ps1`;
  5. `sign-shim.ps1 -Mode init` (`StateRoot` = `$env:GITHUB_WORKSPACE/target/signing`), poi `register-service` su `target/installer-payload/service/oma-service.exe`;
  6. prima passata secondo L8 con `--config ../target/signing/tauri.sign.collect.json`, poi `sign-shim.ps1 -Mode check -Pass collect`;
  7. solo con `steps.pre.outputs.signing == 'enabled'`:
     1. `upload-artifact` di `target/signing/unsigned/*` e del servizio, con nome `binaries-${{ github.run_id }}-${{ github.run_attempt }}` e `if-no-files-found: error`;
     2. richiesta SignPath (`artifact-configuration-slug: binaries`, `signing-policy-slug: ${{ steps.pre.outputs.policy }}`, `github-artifact-id`, `wait-for-completion: true`, `wait-for-completion-timeout-in-seconds: 5400`, `output-artifact-directory: target/signing/returned-binaries`, parametro `version`);
     3. `verify-signatures.ps1 -Files target/signing/returned-binaries` con la policy scelta;
     4. `sign-shim.ps1 -Mode import-signed`;
     5. copia di `signed/oma-service.exe` sul payload;
     6. seconda passata `pnpm tauri bundle --bundles nsis --config ../target/signing/tauri.sign.apply.json`, poi `check -Pass apply`;
     7. copia del setup in `target/signing/setup/`;
     8. `upload-artifact` del setup con nome `setup-${{ github.run_id }}-${{ github.run_attempt }}` e `if-no-files-found: error`, poi richiesta SignPath (`artifact-configuration-slug: setup`, directory `target/signing/returned-setup`);
  8. copia del setup finale (firmato: da `returned-setup`; non firmato: dalla prima passata) in `target/signing/final/<Get-OmaFinalSetupName>`;
  9. `verify-signatures.ps1 -Setup … -Manifest … -Policy release|test|none`;
  10. `Write-OmaSha256Sums`;
  11. `actions/attest` (`subject-path` = il setup finale), poi `gh attestation verify` sullo stesso file;
  12. tag: `publish-draft.ps1`; dispatch: `upload-artifact` del setup e di `SHA256SUMS.txt`;
  13. riepilogo in `$env:GITHUB_STEP_SUMMARY`: modalità di firma, policy, SHA-256 del setup, id delle richieste SignPath (output dell'azione).
- **Artifact configuration** (radice `<zip-file>`, nessuna wildcard):
  - `binaries`: tre `<pe-file path="oma-app.exe|uninstall.exe|oma-service.exe" product-name="OpenMonitor Advanced" product-version="${version}">` con `<authenticode-sign/>`, dove `${version}` usa il formato registrato nello spike;
  - `setup`: un `<pe-file>` col nome del setup.

  Ognuno ha un commento in testa: "copy of the active SignPath configuration; keep in sync (docs/release.md)". La sintassi esatta dei parametri si verifica sulla documentazione SignPath prima di scriverli.

- [ ] **Step 1: scrivi** `release.yml` e i due XML.
- [ ] **Step 2: verifica locale:**
  - actionlint (come nel Task 7) → nessun errore;
  - revisione di sicurezza del workflow con la skill `security-review`, limitata a `release.yml` e alla composite action: niente interpolazione di `github.*` in `run:`, token solo dove serve, nessun percorso non firmato quando `signing == enabled`.
- [ ] **Step 3: commit** `ci: add the release workflow and SignPath configurations`.

---

### Task 9: documentazione

**Files:**
- Create: `CODE_SIGNING.md`, `docs/release.md`
- Modify: `README.md`, `README.it.md`, `CLAUDE.md`, `docs/follow-ups.md`, `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md` (§9 riga "Firma dei binari", §13 punto 7)

- [ ] **Step 1: `CODE_SIGNING.md`** (inglese), secondo la spec §7:
  - ruoli (@Cioscos in *Committers and reviewers* e *Approvers*);
  - stato: "application to SignPath Foundation pending", senza attribuzione finché la firma non è attiva;
  - cosa si firma e cosa no (D4, D5);
  - privacy: nessuna trasmissione di dati, salvo il bootstrapper WebView2 di Microsoft, che l'installer scarica se il runtime manca (link alla privacy di Microsoft);
  - come verificare una firma (`Get-AuthenticodeSignature`).
- [ ] **Step 2: `docs/release.md`** (italiano), i sette punti della spec §7, compresi:
  - quali campi di `.signpath/certificates.json` riempire dopo l'approvazione e come ricavare i thumbprint da un file firmato;
  - come confrontare le artifact configuration attive con le copie in `.signpath/`;
  - come annullare una richiesta pendente prima di "Re-run all jobs";
  - `REQUIRE_SIGNING=true` dopo la prima release firmata;
  - il divieto di pubblicare la bozza durante il run;
  - il token SignPath di un utente con soli diritti di submitter, la protezione di `main` e la policy SignPath vincolata a repository, workflow e ref attesi (spec §4.3).
- [ ] **Step 3: README (en, it):**
  - sezione "Verify your download" (checksum e attestazione);
  - link a `CODE_SIGNING.md`;
  - la riga sul SmartScreen resta com'è.
- [ ] **Step 4: altri documenti:**
  - `CLAUDE.md`: comandi `check-version`, `bump-version`, `Invoke-Pester -Path scripts/tests -CI`, il flusso di release e M6a nell'elenco delle milestone;
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
- Modify: questo piano ("Esito dell'esecuzione"), `docs/follow-ups.md` ("last update: M6a"), la memoria (`m6a-followups.md` e il suo indice)

- [ ] **Step 1: verifiche complete in locale:**
  ```bash
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  dotnet test service/OpenMonitorAdvanced.slnx
  pwsh -c "Import-Module Pester -RequiredVersion 5.7.1; Invoke-Pester -Path scripts/tests -CI"
  pwsh scripts/check-version.ps1
  ```
  Expected: tutti con exit code 0.
- [ ] **Step 2: revisione dell'intero branch** (`superpowers:requesting-code-review`), correzioni, poi merge in `main` in locale (`superpowers:finishing-a-development-branch`).
- [ ] **Step 3: con l'utente**, un punto per volta:
  1. l'utente fa il push di `main` e controlla che la CI sia verde, compresi `scripts` e `actionlint`;
  2. crea l'environment `release` (senza secret SignPath) e il ruleset sui tag seguendo `docs/release.md`;
  3. avvia `workflow_dispatch` da `main`: atteso un artifact con il setup e `SHA256SUMS.txt`, `verify-signatures -Policy none` verde e l'attestazione verificata;
  4. decide se fare subito la prima release non firmata (per esempio la 0.3.0): bump, commit, tag, push, bozza, controllo delle note, pubblicazione;
  5. invia la domanda alla SignPath Foundation, se non l'ha già fatto.

  Misurare anche la durata del job di release, per controllare che le 4 ore bastino (spec §4.2).
- [ ] **Step 4: chiusura:**
  - "Esito dell'esecuzione" in questo piano;
  - follow-up: le verifiche manuali dovute dopo l'approvazione della Foundation, spec §8.2;
  - memoria `m6a-followups.md` con indice in `MEMORY.md`;
  - commit `docs: record the M6a outcome and follow-ups`.

---

## Esito dello spike

*(Da compilare nel Task 1.)*

## Esito dell'esecuzione

*(Da compilare nel Task 10.)*
