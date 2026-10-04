# Guida alla release e alla firma del codice

Guida operativa per chi non ha mai firmato codice. Il design è nella spec
`docs/superpowers/specs/2026-09-30-m6a-release-firma-design.md`; la politica pubblica è in
`CODE_SIGNING.md`.

**Stato:** la domanda alla SignPath Foundation è **prevista, non ancora inviata**. Finché non c'è
l'approvazione, il workflow `.github/workflows/release.yml` produce release **senza firma**; i
passi 1-5 qui sotto servono per quando la Foundation risponde.

## Come funziona, in breve

- Il push di un tag `vX.Y.Z` (su un commit di `main`) avvia `release.yml`: costruisce l'installer
  in CI e prepara una **bozza** di release su GitHub, con il setup e `SHA256SUMS.txt`. Note e
  pubblicazione restano manuali.
- `workflow_dispatch` (solo da `main`) è la prova generale: stessa pipeline, policy `test-signing`,
  nessuna release; il setup resta come artifact del run.
- Con la firma attiva il job chiede **due** approvazioni su SignPath (arrivano per email): la
  prima per `oma-app.exe`, `oma-service.exe` e `uninstall.exe` (configurazione `binaries`), la
  seconda per il setup (configurazione `setup`).
- Senza credenziali SignPath la bozza contiene l'installer non firmato, finché `REQUIRE_SIGNING`
  non è `true` (passo 5).

## 1. La domanda alla Foundation

1. Leggi le condizioni (<https://signpath.org/terms>), poi invia la domanda da
   <https://signpath.org/apply>.
2. Requisiti da soddisfare: licenza OSI senza doppia licenza commerciale (GPL-3.0-or-later),
   nessun componente proprietario, progetto mantenuto e già rilasciato, funzioni descritte nella
   pagina di download (il README).
3. Nel modulo:
   - indica il repository `https://github.com/Cioscos/OpenMonitorAdvanced` e le release 0.1.0 e
     0.2.0 già pubblicate;
   - dichiara con trasparenza il packaging: servizio .NET self-contained con
     LibreHardwareMonitorLib, driver PawnIO (firmato dal suo autore, che noi non firmiamo),
     console PresentMon 2.6.0 (firmata da Intel, ridistribuita senza modifiche e non firmata da
     noi) e installer/uninstaller generati da NSIS con i suoi plugin. L'ammissibilità del packaging va
     confermata dalla Foundation, non data per scontata;
   - linka `CODE_SIGNING.md` come pagina "Code signing policy".
4. L'ammissione è discrezionale. Finché non arriva, nei README e nelle note non va scritta nessuna
   attribuzione a SignPath e la riga sullo SmartScreen resta com'è.

## 2. Autenticazione a due fattori

La Foundation la richiede per tutti i ruoli. Attivala sull'account GitHub del proprietario
(*Settings* → *Password and authentication*) e sull'account SignPath, prima della domanda.

## 3. GitHub App e trusted build system

Dopo l'approvazione, su SignPath.io:

1. installa la GitHub App di SignPath sul repository `Cioscos/OpenMonitorAdvanced`
   ([integrazione GitHub](https://docs.signpath.io/trusted-build-systems/github));
2. aggiungi il trusted build system **GitHub.com** all'organizzazione e collegalo al progetto.

SignPath verifica così l'origine di ogni richiesta: repository, commit, workflow e runner
ospitato da GitHub.

## 4. Progetto, policy e artifact configuration

Su SignPath crea:

- il progetto con slug **`OpenMonitorAdvanced`** (è `SIGNPATH_PROJECT_SLUG` in `release.yml`);
- due *signing policy*:
  - **`release-signing`**, con il certificato della Foundation: la usano i push di tag;
  - **`test-signing`**, con il certificato di prova: la usa `workflow_dispatch`;
- due *artifact configuration*, copiate dalle versioni nel repository:
  - slug **`binaries`** da `.signpath/artifact-configuration-binaries.xml` (`oma-app.exe`,
    `uninstall.exe`, `oma-service.exe`);
  - slug **`setup`** da `.signpath/artifact-configuration-setup.xml` (il setup).

**Policy vincolate all'origine.** Le due signing policy devono consentire solo il repository
`Cioscos/OpenMonitorAdvanced`, il workflow `.github/workflows/release.yml`, i ref attesi (tag
`v*` per `release-signing`, branch `main` per `test-signing`) e il runner ospitato da GitHub.
Imposta l'approvazione manuale su entrambe. Il solo environment di GitHub non basta (spec §4.3).
La policy effettiva deve anche consentire le riesecuzioni del run.

**Confronto con le copie.** La configurazione attiva vive nell'interfaccia di SignPath, quella
versionata in `.signpath/`. Prima di attivare e dopo ogni modifica, apri le due artifact
configuration su SignPath e confrontale riga per riga con i file del commit che stai per
rilasciare (radice `<zip-file>`, nomi dei file, `product-name`, `product-version`,
`file-version`, nessun carattere jolly). Ogni differenza va risolta, in un verso o nell'altro,
prima della release.

## 5. Environment `release`, credenziali, protezioni e certificati attesi

### Environment e credenziali

Su GitHub (*Settings* → *Environments* → `release`):

- *deployment branches and tags*: ammetti separatamente i tag `v*` e il branch `main`;
- **secret `SIGNPATH_API_TOKEN`**: il token API di un utente SignPath con **soli diritti di
  submitter** sulle due policy (non il proprietario, che resta l'approvatore);
- **variabile `SIGNPATH_ORGANIZATION_ID`**: l'ID dell'organizzazione SignPath;
- **variabile `REQUIRE_SIGNING`**: vedi sotto.

Servono **entrambe** le credenziali (`SIGNPATH_ORGANIZATION_ID` e `SIGNPATH_API_TOKEN`) oppure
nessuna. Se ne esiste una sola, il preflight fallisce. Con nessuna, il run salta le richieste di
firma e lo dice con un `::notice::`.

### `REQUIRE_SIGNING`

Imposta `REQUIRE_SIGNING=true` **dopo il primo collaudo reale firmato riuscito** (la prova
generale con `test-signing` e i collaudi manuali in fondo a questa guida) e **prima di pubblicare
la prima release firmata**. Da quel momento credenziali assenti diventano un errore anche per
`workflow_dispatch`, e un ritorno accidentale a release non firmate è impossibile. Nessun
parametro disattiva la verifica delle firme.

### Protezione di `main` e dei tag

- Ruleset sui tag `v*`: solo il proprietario può crearli, aggiornarli o cancellarli.
- Protezione di `main` (niente force-push, revisione dei cambiamenti a workflow, script e
  `.signpath/`): la firma si fida del commit che `main` contiene.
- Il workflow parte solo da un tag o da `main` e non esegue mai codice di una pull request.

### `.signpath/certificates.json`

Il file è vuoto finché non c'è un certificato. Dopo l'approvazione riempi i campi con i valori
della pagina del certificato della policy su SignPath, poi confronta **foglia e radice** (subject
e thumbprint) con la configurazione della policy: non copiarli da un file già firmato.

| Campo | Contenuto |
|---|---|
| `release.subject` | DN esatto del certificato della Foundation; il CN deve essere esattamente `SignPath Foundation` |
| `release.thumbprints` | thumbprint dei certificati approvati per le release; più di uno solo durante un rinnovo |
| `test.subject` | DN esatto del certificato di prova |
| `test.thumbprints` | thumbprint della foglia di prova |
| `test.rootThumbprints` | thumbprint della radice pubblica di prova |
| `test.rootCertificatePath` | percorso del file con la sola parte **pubblica** della radice di prova (relativo alla radice del repository, oppure assoluto) |

Il certificato di prova non è riconosciuto da Windows. Per questo si aggiunge al repository la
radice pubblica di prova (percorso in `test.rootCertificatePath`) e
`verify-signatures.ps1 -Policy test` la installa in `Cert:\LocalMachine\Root` **solo** su un
runner ospitato da GitHub, oppure in una VM o in Windows Sandbox con `OMA_ISOLATED_TRUST=1`, da
amministratore, e la rimuove alla fine se l'ha aggiunta lui. **Mai sul PC di lavoro.** Un file con
chiave privata viene rifiutato.

In caso di rinnovo del certificato di rilascio si aggiunge il nuovo thumbprint accanto al vecchio,
si rilascia, poi si toglie il vecchio.

## 6. Flusso di una release e prova con `workflow_dispatch`

### Prova generale (dopo l'approvazione)

1. Su GitHub: *Actions* → *Release* → *Run workflow*, branch `main`. Usa `test-signing`.
2. Approva le due richieste su SignPath (arrivano per email).
3. Il run termina con l'artifact `release-<run>-<attempt>` (setup e `SHA256SUMS.txt`) e non crea
   nessuna release.

### Release vera

1. Aggiorna la versione con `pwsh scripts/bump-version.ps1 0.3.0`. Lo script aggiorna i cinque
   file e `Cargo.lock`, non crea commit né tag e stampa i quattro comandi successivi:

   ```
   git commit -am "chore: release 0.3.0"
   git tag v0.3.0
   git push origin main
   # aspetta che ci.yml su main sia verde per questo commit (tutti e cinque i job), poi:
   git push origin v0.3.0
   ```

   Non spingere `main` e il tag insieme: il tag avvia subito `release.yml`, il cui gate CI
   fallirebbe perché la CI di quel commit è ancora in corso. Il push è una tua decisione. Per
   controllare in qualsiasi momento che le versioni siano allineate:
   `pwsh scripts/check-version.ps1 [-Tag vX.Y.Z] [-ExpectedSha <sha>]` elenca tutte le incongruenze.
2. Prima di spingere il tag, aspetta che la CI di `main` sia verde **per lo stesso commit** (vedi
   sotto).
3. Il push del tag avvia `release.yml`. Il preflight (`scripts/release-preflight.ps1`) controlla ref,
   credenziali, certificati, CI verde e release non ancora pubblicata, prima di qualsiasi
   richiesta di firma.
4. Con la firma attiva, approva le due richieste su SignPath.
5. A fine run apri la **bozza** su GitHub, scrivi le novità tra i marcatori
   `<!-- oma:changes:start -->` e `<!-- oma:changes:end -->` e **pubblica a mano**.

### Il gate CI

Il preflight richiede l'ultimo run di `ci.yml` per un push su `main` **sullo stesso SHA**, all'ultimo
tentativo, con i job `checks`, `service`, `installer`, `scripts` e `actionlint` tutti riusciti.
Se qualcuno ha usato *Re-run failed jobs*, il gate può segnalare job mancanti: usa **Re-run all
jobs** sul run di `ci.yml`. Se un run di release si ferma comunque al gate perché la CI era
ancora in corso, aspetta che diventi verde e usa **Re-run all jobs** sul run di release.

### Non pubblicare la bozza durante il run

**Non pubblicare la bozza mentre il workflow è in corso.** Il workflow ricontrolla `isDraft`
subito prima di scrivere, ma l'aggiornamento di setup e checksum non è atomico: pubblicare nel
mezzo può lasciare una release con asset misti. Aspetta il riepilogo del run ("created or
updated, remote assets verified"). Una release già pubblicata non viene mai toccata dal workflow.

### Verifiche locali degli script

Servono Pester 5.7.1 (Windows include solo la 3.4):

```powershell
Install-Module Pester -Scope CurrentUser -RequiredVersion 5.7.1 -Force
Import-Module Pester -RequiredVersion 5.7.1
Invoke-Pester -Path scripts/tests -ExcludeTagFilter Integration -CI
```

`testResults.xml` è ignorato da git. I test `Integration` modificano gli store dei certificati e
si eseguono **solo** in un ambiente isolato: il runner di GitHub (lo fa il job `scripts` di
`ci.yml`) oppure una VM o Windows Sandbox con `OMA_ISOLATED_TRUST=1`. Mai sul PC di lavoro.

Ogni script ha un blocco di aiuto in testa: `check-version.ps1`, `bump-version.ps1`,
`verify-signatures.ps1`, `sign-shim.ps1`, `release-preflight.ps1`, `publish-draft.ps1`,
`render-release-notes.ps1`.

## 7. Quando qualcosa va storto

### Un'approvazione scade o una richiesta resta pendente

Ogni richiesta attende al massimo 90 minuti (`wait-for-completion-timeout-in-seconds: 5400`). Se
scade, il job fallisce ma **la richiesta può restare pendente su SignPath**. Prima di rilanciare:

1. apri l'URL della richiesta (id e URL sono nel riepilogo del run);
2. **annulla la richiesta pendente** su SignPath;
3. solo dopo usa *Re-run all jobs*.

Un errore di SignPath non ripiega mai sul percorso non firmato.

### Lo shim segnala uno SHA-256 diverso

`sign-shim.ps1` dice che il file da firmare non è quello firmato: l'`oma-app.exe` o l'uninstaller
della seconda passata non coincide byte per byte con quello inviato a SignPath. Il controllo non
va aggirato. Rilancia l'intero run con *Re-run all jobs*; se si ripete, il determinismo del bundle
è cambiato (per esempio dopo un aggiornamento di Tauri o di NSIS) e va rifatto lo spike del piano
M6a prima di rilasciare.

### Lo shim rifiuta un percorso o un ruolo

Un aggiornamento di `tauri-cli` può cambiare i file passati a `signCommand` (un plugin nuovo, un
altro percorso temporaneo dell'uninstaller). Il run si ferma nominando il file e non firma né
salta nulla in silenzio. Il contratto di `scripts/lib/OmaSigning.psm1` (allowlist dei plugin e
riconoscimento di `nstXXXX.tmp`) va riconfermato con lo spike.

### La verifica delle firme fallisce

`verify-signatures.ps1` elenca ogni problema. Cause tipiche:

- **firmatario o thumbprint diverso** da `.signpath/certificates.json`: confronta con il
  certificato mostrato da SignPath (passo 5); in caso di rinnovo aggiungi il nuovo thumbprint;
- **timestamp mancante**: senza timestamp la firma smette di valere alla scadenza del certificato;
- **hash diverso** da quello del manifest: i file restituiti non sono quelli inviati, non
  pubblicare;
- **metadati** (nome prodotto, versione) diversi da quelli delle artifact configuration:
  confronta con `.signpath/` (passo 4).

### La bozza è parziale

Se il riepilogo dice che il caricamento è fallito o è parziale, **non pubblicare la bozza**:
rilancia il workflow, che aggiorna il solo blocco tecnico e conserva le novità scritte a mano.

## Collaudi manuali prima della prima release firmata

7-Zip non vede `uninstall.exe` dentro il setup (spike M6a), quindi la firma dell'uninstaller
**installato** non si verifica in automatico. In Windows Sandbox o in una VM (mai sul PC di
lavoro, e senza input sintetico sul desktop dell'utente):

1. installa il setup preso dall'artifact del `workflow_dispatch` o dalla bozza;
2. in `C:\Program Files\OpenMonitor Advanced` controlla con `Get-AuthenticodeSignature` le firme
   di `oma-app.exe`, `service\oma-service.exe` e `uninstall.exe`;
3. controlla l'editore nella conferma UAC dell'installazione e in quella della disinstallazione;
4. scarica il setup dal browser e annota il comportamento di SmartScreen.

L'elenco completo delle verifiche dovute è in `docs/follow-ups.md`.

## Dopo la prima release firmata

- Aggiungi l'attribuzione ("Free code signing provided by SignPath.io, certificate by SignPath
  Foundation") a `CODE_SIGNING.md`, ai README e alle note di release.
- Aggiorna la riga sullo SmartScreen dei README solo se il comportamento osservato lo giustifica.
- Imposta `REQUIRE_SIGNING=true`, se non l'hai già fatto.
- Conferma alla prima richiesta reale i punti aperti di `docs/follow-ups.md`.
