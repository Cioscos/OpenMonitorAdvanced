# M7a — Manutenzione: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** aggiornare sopra una versione installata senza domande, chiudere il debito tecnico mirato e dividere i file più grandi, poi preparare la release 0.4.1.

**Architecture:**
- **Prima gli split:** sono quattro task di puro spostamento, senza cambi di comportamento, così le correzioni successive si rivedono su file piccoli.
- **Poi le correzioni:** ognuna con il suo test.
- **Infine:**
  - l'app impara `--quit`;
  - l'installer la chiude prima del vecchio disinstallatore e la riapre a fine installazione;
  - documentazione e bump alla 0.4.1.

**Tech Stack:** Rust 1.90 (workspace `oma-core`, `oma-win`, `oma-app`), Tauri 2.11, Svelte 5 + Vitest, .NET 10 + xUnit v3, NSIS (template di tauri-cli 2.11.5), PowerShell 7 + Pester 5.7.1.

**Spec:** `docs/superpowers/specs/2026-10-04-m7-manutenzione-overlay-design.md`, §1 e §2. Nel §2 sono già incluse le correzioni fatte durante la stesura di questo piano: nessun `/UPDATE` verso il vecchio disinstallatore, servizio riavviato dall'app, `GateEpisode` in `DiskPowerProbe.cs`.

**Branch:** `feat/m7a-manutenzione`, da `main`; merge in `main` in locale alla fine. Push e tag solo su richiesta dell'utente.

## Global Constraints

- Codice, commenti e messaggi di commit in inglese (conventional commits); documentazione in italiano con gli accenti corretti.
- Fine riga LF ovunque.
- Ogni commit termina con `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.
- TDD: prima il test che fallisce, poi il codice (skill `superpowers:test-driven-development`). Davanti a un test che fallisce senza motivo chiaro, `superpowers:systematic-debugging`.
- **FFI:**
  - un commento `// SAFETY:` per ogni blocco `unsafe`;
  - un assert di dimensione a compile time per ogni struct FFI scritta a mano;
  - DLL solo da System32;
  - revisione `ffi-safety-reviewer` per i task che toccano `unsafe`.
- `installer.nsi`: si aggiungono solo righe che finiscono con ` ; OMA`. Le righe upstream non si toccano: le verifica `app/src/test/nsis-template.test.ts`. `Italian.nsh` resta invariato; le stringhe nostre vanno in `OMA_LANGSTRINGS` di `oma.nsh`.
- **Split** (Task 1–4):
  - nessun cambio di comportamento, di API pubblica o di percorsi pubblici (`crate::storage::X`, `crate::svc::X` restano validi tramite `pub use`);
  - il numero di test eseguiti non cambia, salvo i test nuovi dichiarati nel task.
- **Orientamento nel codice:** prima `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"`, poi la lettura dei file. Dopo ogni task che cambia codice, `PYTHONHASHSEED=0 graphify update .`.
- **Comandi di verifica:** quelli di `CLAUDE.md` (`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cd app && pnpm test && pnpm check`, `dotnet test service/OpenMonitorAdvanced.slnx`, Pester con `-ExcludeTagFilter Integration`).
- Mai test Pester `Integration`, mai installer eseguiti da un agente, mai clic sintetici: le prove dal vivo le fa l'utente (§«Verifiche dal vivo»).
- Mai `reads_disk_temperatures_on_this_machine` da un agente: lo esegue solo l'utente.
- Mai una ricerca a tutto il disco.

## Review Focus

1. **Build di sviluppo e autostart.** Un'app in debug (`pnpm tauri dev`, `target\debug\oma-app.exe`) con l'avvio automatico attivo **non deve** riscrivere il valore Run verso l'eseguibile di debug. La riparazione vale solo per le build release. Test nel Task 7.
2. **`--quit` senza un'istanza in esecuzione.** Il processo esce senza finestra, senza icona nel tray, senza avviare il campionatore né il servizio e senza scrivere l'autostart. Test nel Task 5.
3. **`DisplayVersion` mancante o illeggibile.** Si va dritti alla chiusura forzata, mai a `RunAsUser … --quit`. Test statico nel Task 6.
4. **Un comando inviato al link con la coda piena** ritorna subito con un errore: non blocca il thread chiamante (main thread o listener dello store). Test nel Task 9.
5. **Gli URI non si controllano come file.** `open_release_page` e `ms-settings:startupapps` vanno aperti senza il controllo «il file non esiste», che vale solo per i percorsi. Test nel Task 8.

---

### Task 1: dividere `crates/oma-win/src/storage.rs`

**Files:**
- Delete: `crates/oma-win/src/storage.rs`
- Create:
  - `crates/oma-win/src/storage/mod.rs`
  - `crates/oma-win/src/storage/temperatures.rs`
  - `crates/oma-win/src/storage/disk_gate.rs`
  - `crates/oma-win/src/storage/tables.rs`
  - `crates/oma-win/src/storage/feed_tests.rs`
- Modify: nessun altro file. `lib.rs` dichiara già `pub mod storage;`, e i moduli fratelli `storage_gate.rs`, `storage_health.rs`, `storage_identity.rs`, `storage_ioctl.rs` e `storage_temperature.rs` restano dove sono.

**Interfaces:**
- Produces: gli stessi percorsi pubblici di oggi (`crate::storage::{DriveEntry, DriveIds, DriveIdTable, DiskStateTable, StorageProvider, SMART_SELECTABLE, drive_keys_for, core_id_for_key, DiskPower, used_pct, …}`) tramite `pub use` in `mod.rs`. I tipi privati (`DiskGate`, `DiskTemperatures`, `Imported`, `Reading`, `Counters`) diventano `pub(super)` con i campi `pub(super)` che i test usano.

Contenuto dei file (le righe si riferiscono a `storage.rs` al commit `cacc85e`):

| File | Contenuto |
|---|---|
| `temperatures.rs` | `DiskTemperatures`, `Imported`, `MAIN_POSITION`, `SnapshotId`, `snapshot_id` (righe 111–252) e i loro test |
| `disk_gate.rs` | `DiskGate` (259–463), `disk_states`, `gates_by_id`, `refresh_one` (466–528), `Reading`, `read_temperatures`, `seek_penalty`, `powered_on` (529–570) e i test del gate (1211–1815) |
| `tables.rs` | `DriveEntry`, `SMART_SELECTABLE`, `SMART_DEFAULT`, `smart_default_off`, `disk_properties`, `drive_keys_for`, `core_id_for_key`, `DriveIds`, `DriveIdTable`, `DiskStateTable` (581–767) e i loro test (2969–3090) |
| `mod.rs` | import, costanti PDH, `DiskInstance` e i suoi helper, `used_pct`, `volume_space`, `Counters`, `StorageProvider` (36–110, 571–580, 770–1101), i test di parsing di base (1108–1209), le dichiarazioni dei sottomoduli e i `pub use` |
| `feed_tests.rs` | i test «the service's feed» (1815–2965), dichiarati in `mod.rs` con `#[cfg(test)] mod feed_tests;` |

- [ ] **Step 1: Contare i test di partenza**

Run: `cargo test -p oma-win 2>&1 | grep -E "^test result"`
Expected: annotare il totale «passed» e «ignored» di ogni binario.

- [ ] **Step 2: Spostare il codice nei file della tabella**, aggiustando solo visibilità (`pub(super)`) e `use`. Nessuna riga di logica cambia.

- [ ] **Step 3: Verificare**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p oma-win 2>&1 | grep -E "^test result"`
Expected: stessi totali dello Step 1; clippy pulito.

- [ ] **Step 4: Commit**

```bash
git add crates/oma-win/src/storage.rs crates/oma-win/src/storage/
git commit -m "refactor(oma-win): split storage.rs into a module directory"
```

---

### Task 2: dividere `crates/oma-win/src/svc/link.rs`

**Files:**
- Delete: `crates/oma-win/src/svc/link.rs`
- Create:
  - `crates/oma-win/src/svc/link/mod.rs`
  - `crates/oma-win/src/svc/link/transport.rs`
  - `crates/oma-win/src/svc/link/machine.rs`
  - `crates/oma-win/src/svc/link/machine_tests.rs`
  - `crates/oma-win/src/svc/link/tests/mod.rs`
  - `crates/oma-win/src/svc/link/tests/launch.rs`
  - `crates/oma-win/src/svc/link/tests/stream.rs`
  - `crates/oma-win/src/svc/link/tests/anti_cheat.rs`
  - `crates/oma-win/src/svc/link/tests/lifecycle.rs`

**Interfaces:**
- Produces: gli stessi nomi pubblici di oggi (`ServiceLink`, `LinkCommand`, `LinkSettings`, `LinkSink`, `Connection`, `Connector`, `pipe_connector`, `validate_schema`, `JOIN_WAIT`), riesportati da `link/mod.rs`. `svc/mod.rs` non cambia.

Contenuto (righe di `link.rs` al commit `cacc85e`):

| File | Contenuto |
|---|---|
| `mod.rs` | documentazione del modulo (1–42), `JOIN_WAIT`, `Input`, `NEXT_CONNECTION`, `Driver` (1193–1374), `ServiceLink` (1377–1482), le dichiarazioni dei sottomoduli e i `pub use` |
| `transport.rs` | `Connection`, `LinkSink`, `Connector`, `pipe_connector`, `PipeConnection` (80–162) |
| `machine.rs` | costanti (68–77), `LinkCommand`, `LinkSettings`, `validate_schema`, `Event`, `Effect`, `Phase`, `status()`, `disconnected()`, `Machine` (166–1190) |
| `machine_tests.rs` | i test della funzione di decisione, solo `Machine` (3668–4303); in `machine.rs`: `#[cfg(test)] #[path = "machine_tests.rs"] mod tests;` |
| `tests/mod.rs` | messaggi, SCM finto, connessioni scriptate, harness (1484–1942) |
| `tests/launch.rs` | avvio e controlli di connessione (1943–2276) |
| `tests/stream.rs` | il flusso, «one queue / no wake-ups» (2277–2775) |
| `tests/anti_cheat.rs` | anti-cheat e fix round R21 (2776–3092) |
| `tests/lifecycle.rs` | shutdown, validazione dello schema, richieste delle sorgenti e PawnIO (3093–3667) |

- [ ] **Step 1: Contare i test di partenza** (come nel Task 1, Step 1).
- [ ] **Step 2: Spostare il codice**, solo visibilità e `use`.
- [ ] **Step 3: Verificare:** stessi comandi e stesso esito del Task 1, Step 3.
- [ ] **Step 4: Commit:** `refactor(oma-win): split the service link into machine, transport and driver modules`.

---

### Task 3: estrarre il formattatore delle chiavi di visualizzazione da `rules/health.rs`

**Files:**
- Create: `crates/oma-core/src/rules/display_key.rs`
- Modify:
  - `crates/oma-core/src/rules/health.rs`: togliere le righe 736–805 e il test `display_key_matches_formatter_precision` (1686–1721);
  - `crates/oma-core/src/rules/mod.rs`: `mod display_key;`.

**Interfaces:**
- Produces: `pub(crate) fn display_key(value: f64, unit: Unit) -> [i64; 4]` in `rules::display_key`, con gli helper privati `round`, `stepped`, `bytes_key` e `bits_key`. `health.rs` lo importa (`use super::display_key::display_key;`).

- [ ] **Step 1: Spostare** il codice e il test esistente in `display_key.rs`. Esegui `cargo test -p oma-core`: lo stesso totale di prima.
- [ ] **Step 2: Aggiungere i test dei casi non coperti** in `display_key.rs`. Si confronta solo il primo elemento quando gli altri tre sono zero per costruzione:

```rust
#[test]
fn display_key_rounds_whole_number_units() {
    assert_eq!(display_key(1.5, Unit::Hours)[0], 2);
    assert_eq!(display_key(2.5, Unit::Count)[0], 3);
    assert_eq!(display_key(1234.4, Unit::Rpm)[0], 1234);
}

#[test]
fn display_key_rounds_pcie_fields_half_up() {
    assert_eq!(display_key(3.5, Unit::PcieGeneration)[0], 4);
    assert_eq!(display_key(2.5, Unit::PcieGeneration)[0], 3);
    assert_eq!(display_key(15.5, Unit::Lanes)[0], 16);
}

#[test]
fn display_key_keeps_the_sign_of_negative_temperatures() {
    // -0.5 °C rounds away from zero; in Fahrenheit it is 31.1.
    assert_eq!(display_key(-0.5, Unit::Celsius), [-1, 31, 0, 0]);
}
```

Aggiungere anche `display_key_steps_joules`, con tre valori (999, 1500, 2 500 000). Il valore atteso si ricava a mano da `formatValue` in `app/src/lib/format.ts`, e il calcolo va scritto nel commento del test.

- [ ] **Step 3: Eseguire** `cargo test -p oma-core display_key`. Expected: PASS. Se un'attesa non corrisponde, prima si controlla `format.ts`. Il codice si cambia solo se è il formattatore Rust a divergere dal formatter TypeScript, e allora il fix si annota nel commit.
- [ ] **Step 4: Commit:** `refactor(oma-core): move the display-key formatter out of health.rs`.

---

### Task 4: dividere `SensorHub.cs` e `SensorHubTests.cs`

**Files:**
- Modify: `service/OpenMonitorAdvanced.Service/Sensors/SensorHub.cs`, che diventa `public sealed partial class SensorHub` e tiene campi, costruttore, seam interni, `Subscribe`, consegna, `Dispose`, configurazione, metodi vari e i tipi annidati `Published`, `DesiredConfig`, `Subscriber`, `NoSubscription`.
- Create:
  - `Sensors/SensorHub.Sampler.cs`: righe 303–492;
  - `Sensors/SensorHub.Storage.cs`: righe 493–854 e 1143–1400, più `DiskResolution` e `StorageRound`;
  - `Sensors/SensorHub.Plan.cs`: righe 919–1142, più `Plan`;
  - `Sensors/SchemaComparer.cs`: righe 1890–1925;
  - `Sensors/GateEpisode.cs`: `GateEpisode`, tolto da `DiskPowerProbe.cs:670-823` insieme al suo doc;
  - `service/OpenMonitorAdvanced.Service.Tests/Sensors/SensorHubTests.Reconfiguration.cs` (righe 1964–2639);
  - `…Tests/Sensors/SensorHubTests.DriveList.cs` (2640–3223);
  - `…Tests/Sensors/SensorHubTests.PowerChecks.cs` (3225–fine).
- Modify:
  - `DiskPowerProbe.cs`: rimuovere `GateEpisode`;
  - `SensorHubTests.cs`: diventa `public sealed partial class SensorHubTests` con helper e test del nucleo (fino alla 1962).

**Interfaces:**
- Produces: nessun cambio di API. Nei test, `ValueOf` e `HeldOf` usano un unico helper privato `static int SensorIndex(BuiltSchema schema, string deviceId, string kind, string name)`, oppure la firma equivalente dettata dal ciclo che oggi è duplicato.

- [ ] **Step 1: Contare i test:** `dotnet test service/OpenMonitorAdvanced.slnx` e annotare il totale.
- [ ] **Step 2: Spostare** il codice nei file della lista, con namespace file-scoped `OpenMonitorAdvanced.Service.Sensors`, e i `<see cref>` ancora risolvibili. Gli helper usati da un solo gruppo di test (per esempio `UsbHarness`, `GateHarness`) vanno nel file di quel gruppo.
- [ ] **Step 3: Unificare `ValueOf` e `HeldOf`** sull'helper comune.
- [ ] **Step 4: Verificare:** `dotnet test service/OpenMonitorAdvanced.slnx` dà lo stesso totale, zero avvisi (`TreatWarningsAsErrors`); poi `pwsh scripts/check-trim-warnings.ps1` è invariato.
- [ ] **Step 5: Commit:** `refactor(service): split SensorHub into partial files and move GateEpisode out`.

---

### Task 5: `--quit` nell'app

**Files:**
- Modify: `app/src-tauri/src/main.rs`: il callback single-instance (229–234), `main()` (155–156) e `setup`; i test vicino a `opens_window_ignores_minimized_launches` (528).

**Interfaces:**
- Produces:
  - `enum SecondLaunch { Quit, ShowWindow, Nothing }`;
  - `fn second_launch(args: &[String]) -> SecondLaunch`, che sostituisce `opens_window`;
  - `fn is_quit(args: &[String]) -> bool`.

  Il Task 6 si basa sul contratto da riga di comando: `oma-app.exe --quit` chiude l'istanza in esecuzione con `app.exit(0)`, lo stesso percorso della voce «Esci» del tray.

- [ ] **Step 1: Test che falliscono**

```rust
#[test]
fn second_launch_decides_quit_show_or_nothing() {
    assert!(matches!(second_launch(&args(&["oma-app.exe", "--quit"])), SecondLaunch::Quit));
    assert!(matches!(second_launch(&args(&["oma-app.exe", "--minimized", "--quit"])), SecondLaunch::Quit));
    assert!(matches!(second_launch(&args(&["oma-app.exe", "--minimized"])), SecondLaunch::Nothing));
    assert!(matches!(second_launch(&args(&["oma-app.exe"])), SecondLaunch::ShowWindow));
    assert!(matches!(second_launch(&args(&["oma-app.exe", "--safe"])), SecondLaunch::ShowWindow));
}

#[test]
fn quit_is_recognised_only_as_a_whole_argument() {
    assert!(is_quit(&args(&["oma-app.exe", "--quit"])));
    assert!(!is_quit(&args(&["oma-app.exe", "--quitter"])));
    assert!(!is_quit(&args(&["oma-app.exe"])));
}
```

- [ ] **Step 2: Eseguire** `cargo test -p oma-app second_launch quit_is`. Expected: FAIL, le funzioni non esistono ancora.
- [ ] **Step 3: Implementare.**
  - **Seconda istanza:** il callback single-instance applica `second_launch`: `Quit` porta a `app.exit(0)`, `ShowWindow` a `window::show_main(app)`, `Nothing` a nulla.
  - **Prima istanza con `--quit`:** non c'è nessuno a cui inoltrarlo, quindi in `setup` l'app chiama `app.handle().exit(0)` come prima istruzione, prima di finestra, tray, campionatore, link al servizio e `Autostart::follow`. Nessuno di questi deve partire (Review Focus 2).
  - Se qualche componente parte in `main()` prima del builder, va spostato dopo il controllo oppure saltato con `--quit`. Si documenta con un commento.
- [ ] **Step 4: Eseguire** `cargo test -p oma-app` e clippy. Expected: PASS.
- [ ] **Step 5: Commit:** `feat(app): quit the running instance with --quit`. La prova dal vivo è la U7: la fa l'utente, perché un agente non avvia l'app sul desktop.

---

### Task 6: l'installer chiude l'app prima dell'aggiornamento e la riapre

**Files:**
- Modify:
  - `app/src-tauri/nsis/oma.nsh`: variabili, macro `OMA_CLOSE_APP` e `OMA_RELAUNCH_APP`, funzione `OmaCloseApp` dentro il corpo di `OMA_SECTIONS` (come `OmaInitComponents`, perché usa `${MAINBINARYNAME}` e `${UNINSTKEY}`), chiamata in coda a `NSIS_HOOK_PREINSTALL`;
  - `app/src-tauri/nsis/installer.nsi`: due righe nuove, marcate `; OMA`;
  - `app/src/test/nsis-template.test.ts`.

**Interfaces:**
- Consumes: `oma-app.exe --quit` (Task 5).
- Produces:
  - `Var OmaAppWasRunning`: `"1"` se l'app girava nella sessione dell'utente che installa;
  - `Var OmaAppClosed`: `"1"` dopo la prima chiusura; serve all'idempotenza.

Le due righe in `installer.nsi`:
- subito dopo `  reinst_uninstall:` (riga 347): `    !insertmacro OMA_CLOSE_APP ; OMA`;
- subito dopo `  !insertmacro OMA_ONINSTSUCCESS ; OMA` (riga 750): `  !insertmacro OMA_RELAUNCH_APP ; OMA`.

L'algoritmo di `OmaCloseApp`, che salva e ripristina i registri che usa (`Push`/`Pop`): la pagina di reinstallazione dipende da `$R0`–`$R6`.

```
if OmaAppClosed == "1": return            ; second call (page + PREINSTALL hook)
OmaAppClosed = "1"
FindProcessCurrentUser "<exe>" -> 0 ? OmaAppWasRunning = "1"
FindProcess "<exe>" -> not 0 ? return     ; nothing runs anywhere
version = ReadRegStr SHCTX UNINSTKEY "DisplayVersion"
if version != "" and SemverCompare(version, "0.4.1") >= 0 and FileExists "$INSTDIR\<exe>":
    RunAsUser "$INSTDIR\<exe>" "--quit"
    repeat up to 40 times: Sleep 250; FindProcess "<exe>" -> not 0 ? return
KillProcess "<exe>"; Sleep 500
result not in {0, 2}: DetailPrint "Could not close ${PRODUCTNAME}"   ; CheckIfAppIsRunning reports it later
```

`OMA_RELAUNCH_APP` in `.onInstSuccess`:
- riapre l'app con `RunAsUser "$INSTDIR\<exe>" "--minimized"` solo se valgono tutte e tre le condizioni:
  - `OmaAppWasRunning == "1"`;
  - installazione silenziosa (`${Silent}`) o `$PassiveMode = 1`;
  - nella riga di comando non c'è `/R`, che avvia già l'app da sé;
- in modalità grafica non fa nulla: provvede la casella «Avvia» della pagina finale, già spuntata;
- salva e ripristina i registri che usa.

- [ ] **Step 1: Test statici che falliscono** in `nsis-template.test.ts`, nello stile dei test esistenti:
  - `installer.nsi` contiene `!insertmacro OMA_CLOSE_APP ; OMA` esattamente una volta, sulla riga che segue `reinst_uninstall:`;
  - `installer.nsi` contiene `!insertmacro OMA_RELAUNCH_APP ; OMA` esattamente una volta, sulla riga che segue `!insertmacro OMA_ONINSTSUCCESS ; OMA`;
  - l'ultima istruzione di `!macro NSIS_HOOK_PREINSTALL` è `!insertmacro OMA_CLOSE_APP`, e le prime tre restano quelle di oggi;
  - in `Function OmaCloseApp`:
    - `FindProcessCurrentUser` viene prima del primo `KillProcess` e del primo `RunAsUser`;
    - l'unico `RunAsUser` con `--quit` sta in un blocco condizionato da `SemverCompare` contro `"0.4.1"` e da un controllo che la versione non sia vuota (Review Focus 3);
    - `KillProcess` compare dopo il ciclo di attesa;
    - i `Push` e i `Pop` sono bilanciati;
  - `OMA_RELAUNCH_APP` usa `--minimized` e controlla `/R`.

  Il test di deriva rispetto a `upstream-2.11.5.nsi` deve continuare a passare senza modifiche.

- [ ] **Step 2: Eseguire** `cd app && pnpm test nsis-template`. Expected: FAIL sui test nuovi.
- [ ] **Step 3: Implementare** macro, funzione e righe secondo l'algoritmo; variabili dichiarate in cima a `oma.nsh` come le altre `Oma*`.
- [ ] **Step 4: Eseguire** `cd app && pnpm test nsis-template`. Expected: PASS.
- [ ] **Step 5: Compilare l'installer** senza eseguirlo: `cd app && pnpm tauri build --bundles nsis`. Expected: build riuscita. NSIS fallisce in compilazione su macro o variabili sbagliate.
- [ ] **Step 6: Commit:** `feat(installer): close the running app before an upgrade and reopen it afterwards`.

---

### Task 7: riparare il percorso del valore Run all'avvio

**Files:**
- Modify:
  - `app/src-tauri/src/autostart.rs`: trait `StartupEntry`, `RunEntry`, `Unsupported`, `Autostart::follow`, `FakeEntry` nei test;
  - `README.md`: tra i limiti noti, il valore Run di un utente standard che sopravvive alla disinstallazione perMachine.

**Interfaces:**
- Produces:
  - `fn needs_repair(stored: Option<&str>, expected: &str) -> bool`: confronto senza distinzione fra maiuscole e minuscole ASCII;
  - `StartupEntry::repair(&self) -> io::Result<bool>`: `true` se ha riscritto.
- Consumes: `oma_win::autostart::{RunKey, command_line}` (`crates/oma-win/src/autostart.rs:27`).

- [ ] **Step 1: Test che falliscono**

```rust
#[test]
fn needs_repair_only_for_a_stale_path() {
    let expected = r#""C:\Program Files\OpenMonitor Advanced\oma-app.exe" --minimized"#;
    assert!(!needs_repair(None, expected));
    assert!(!needs_repair(Some(expected), expected));
    assert!(!needs_repair(Some(&expected.to_lowercase()), expected));
    assert!(needs_repair(Some(r#""D:\Old\oma-app.exe" --minimized"#), expected));
}

#[test]
fn follow_repairs_a_stale_entry_once_at_startup() { /* FakeEntry with a stale value: repair() called once, before any set() */ }

#[test]
fn a_failed_repair_is_logged_and_changes_nothing_else() { /* FakeEntry::repair -> Err: setting unchanged, no set() call */ }
```

- [ ] **Step 2: Eseguire** `cargo test -p oma-app autostart`. Expected: FAIL.
- [ ] **Step 3: Implementare.**
  - `RunEntry::repair` legge con `RunKey::read()`. Se c'è un valore e `needs_repair` è vero, riscrive con `RunKey::write(&self.exe)`.
  - `Unsupported::repair` restituisce `Ok(false)`.
  - `Autostart::follow` chiama `repair()` una volta, prima di sottoscriversi, **solo se `!cfg!(debug_assertions)`** (Review Focus 1). Nel codice va un commento che spiega perché: una build di sviluppo riscriverebbe l'avvio automatico verso `target\debug`.
  - Esito: `tracing::info!` se il valore è stato riparato, `tracing::warn!` se la riparazione fallisce.
- [ ] **Step 4: Eseguire** `cargo test -p oma-app autostart` e clippy. Expected: PASS.
- [ ] **Step 5: Commit:** `fix(app): repair a stale autostart path at startup`.

---

### Task 8: `shell_open` robusto

**Files:**
- Modify:
  - `crates/oma-win/src/shell_open.rs`;
  - `app/src-tauri/src/commands.rs` (wrapper 462–469 e `open_known_path` 451);
  - `app/src-tauri/src/report.rs` (`reveal_sensor_report`), `app/src-tauri/src/updates.rs` (`open_release_page`), `app/src-tauri/src/log/commands.rs` (`open_folder`);
  - `app/src/components/settings/AboutSection.svelte`, `GeneralSection.svelte`;
  - `app/src/lib/i18n/en.json`, `it.json`.
- Create: `app/src/lib/openFailure.ts` e il suo `.test.ts`.

**Interfaces:**
- Produces in `oma-win`:
  - `pub fn open(target: &OsStr, timeout: Duration) -> Result<(), OpenError>`;
  - `pub enum OpenError { TimedOut, Os(io::Error) }`.
- Produces nell'app:
  - `pub(crate) const SHELL_OPEN_TIMEOUT: Duration = Duration::from_secs(10)`;
  - `pub(crate) fn open_path(path: &Path) -> Result<(), String>`: se il percorso non esiste, `Err("shell.error.missing")`;
  - `pub(crate) fn open_uri(uri: &str) -> Result<(), String>`: nessun controllo di esistenza.

  In entrambe `TimedOut` diventa `"shell.error.timeout"` e `Os(e)` diventa `e.to_string()`.
- Produces in TypeScript: `export function openFailureText(error: unknown): string`. Per le chiavi `shell.error.missing`, `shell.error.timeout` e `log.error.folderMissing` restituisce `t('settings.openFailed', { reason: t(key) })`; altrimenti `t('settings.openFailed', { reason: String(error) })`.

Chiavi i18n nuove:

| Chiave | en | it |
|---|---|---|
| `shell.error.missing` | The file or folder does not exist | Il file o la cartella non esiste |
| `shell.error.timeout` | Windows did not respond in time | Windows non ha risposto in tempo |

- [ ] **Step 1: Test che falliscono:**
  - in `shell_open.rs`, test di `outcome`/`OpenError` (codice > 32 = ok, codice 2 = `Os` con `NotFound`) e test puri sul timeout: un helper `wait_with_timeout(rx, timeout)` che con un canale senza risposta restituisce `TimedOut`;
  - nell'app, `open_known_path_sends_uris_without_an_existence_check`: una funzione pura `fn open_target(target: KnownPath, dirs: &KnownDirs) -> OpenTarget` con `enum OpenTarget { Path(PathBuf), Uri(&'static str) }`, e `StartupAppsSettings` deve dare `Uri` (Review Focus 5);
  - `open_path_reports_a_missing_path`: un percorso inesistente dà `Err("shell.error.missing")`;
  - in `openFailure.test.ts`, le tre chiavi tradotte e un testo di sistema passato invariato.
- [ ] **Step 2: Eseguire** `cargo test -p oma-win shell_open`, `cargo test -p oma-app open_` e `cd app && pnpm test openFailure`. Expected: FAIL.
- [ ] **Step 3: Implementare.**
  - **Thread della shell:** `ShellExecuteExW` con `SHELLEXECUTEINFOW { cbSize, fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI, lpVerb: "open", lpFile, nShow: SW_SHOWNORMAL }` (assert di dimensione della struct se scritta a mano). In caso di FALSE, `GetLastError` diventa `io::Error`. Il thread manda l'esito su un canale.
  - **Chiamante:** usa `recv_timeout`; allo scadere restituisce `TimedOut` e lascia il thread staccato.
  - **Interfaccia:** i quattro punti dell'UI usano `openFailureText`.
- [ ] **Step 4: Eseguire** i test dello Step 2 più `pnpm check` e clippy. Expected: PASS.
- [ ] **Step 5: Revisione `ffi-safety-reviewer`** su `shell_open.rs`.
- [ ] **Step 6: Commit:** `fix: open shell targets with a timeout and report missing paths`.

---

### Task 9: link al servizio, coda limitata e stato durante `Held`

**Files:**
- Modify: `crates/oma-win/src/svc/link/mod.rs`, `machine.rs`, `transport.rs` (dopo il Task 2), i test del link, `app/src-tauri/src/service.rs` (`send_to_link`, 393–402).

**Interfaces:**
- Produces:
  - `pub const LINK_QUEUE_CAPACITY: usize = 256`;
  - `pub struct LinkBusy;`, che implementa `Display` e `Error`;
  - `ServiceLink::send(&self, command: LinkCommand) -> Result<(), LinkBusy>`;
  - `send_to_link` registra `tracing::warn!("service link queue full; {command:?} dropped")` quando riceve `Err`.

Regole della coda (`mpsc::sync_channel(LINK_QUEUE_CAPACITY)`):

| Ingresso | Invio | Con la coda piena |
|---|---|---|
| `Input::Command` (da `send`) | `try_send` | `Err(LinkBusy)` subito, il chiamante non si blocca (Review Focus 4) |
| `Input::Message` con uno `Snapshot` (dal lettore della pipe) | `try_send` | lo snapshot si scarta (ne arriva uno nuovo all'intervallo successivo); si conta e si registra un `warn!` una volta per episodio |
| altri `Input::Message` (Hello, Schema, Error, lista dei dischi) e `Input::Closed` | `send` bloccante sul thread del lettore | il lettore aspetta: è un thread dedicato. Se il link è chiuso, `send` fallisce e il lettore esce |
| `Input::Shutdown` | `try_send` dopo `stop.store(true)` | il thread deve vedere il flag `stop` tra un ingresso e l'altro, e `shutdown` torna entro `JOIN_WAIT` anche con la coda piena |

Stato in `Held`: in `Machine::on_held_query`:
- `ServiceQuery::NotInstalled` imposta lo stato «non installato»;
- `ServiceQuery::State { state: Stopped, .. }` imposta lo stato «fermo», come fa il resto della macchina per quelle risposte;
- in entrambi i casi la fase resta `Held` e la baseline si aggiorna come oggi. Così l'utente vede che il servizio è stato disinstallato o fermato anche dopo `Incompatible` o `PidMismatch`;
- la riconnessione resta legata a `Start` o a un `Running`/`StartPending` diverso dalla baseline.

- [ ] **Step 1: Test che falliscono:**
  - `send_returns_busy_when_the_queue_is_full`: un link il cui thread è bloccato in un effetto lento (con le connessioni scriptate esistenti); 256 comandi riempiono la coda; il 257° restituisce `Err(LinkBusy)` in meno di 50 ms;
  - `shutdown_returns_within_join_wait_with_a_full_queue`;
  - `a_full_queue_drops_snapshots_but_keeps_the_schema`;
  - nei test della macchina: `held_incompatible_shows_not_installed_after_an_uninstall` e `held_pid_mismatch_shows_stopped_when_the_service_stops`;
  - i test esistenti `incompatible_is_not_retried_until_start` e `decide_incompatible_is_kept_across_an_explicit_start` continuano a passare.
- [ ] **Step 2: Eseguire** `cargo test -p oma-win svc::link`. Expected: FAIL sui test nuovi.
- [ ] **Step 3: Implementare** secondo la tabella e le regole. Aggiornare il doc di `LinkSink` (oggi «the queue is not bounded») e quello del modulo.
- [ ] **Step 4: Eseguire** `cargo test -p oma-win` e `cargo test -p oma-app`, poi clippy. Expected: PASS.
- [ ] **Step 5: Commit:** `fix(oma-win): bound the service link queue and show the service state while held`.

---

### Task 10: piccole correzioni in `oma-win`

**Files:**
- Modify: `crates/oma-win/src/memory.rs`, `crates/oma-win/src/storage/mod.rs` (dopo il Task 1), `crates/oma-win/src/pdh.rs`, `crates/oma-win/src/storage_temperature.rs` (316–343).

**Interfaces:**
- Produces:
  - `crate::memory::used_pct(total: u64, available: u64) -> Option<f64>`, unico; `storage` lo importa;
  - in `pdh.rs`:
    - `fn items_fit(count: u32, item_size: usize, buffer_bytes: usize) -> bool`: `count × item_size ≤ buffer_bytes` senza overflow;
    - `fn item_name(name: PWSTR) -> String`: stringa vuota se il puntatore è nullo;
    - `const ERROR_INVALID_DATA: u32 = 13`, usato come `PdhError { call, status: ERROR_INVALID_DATA }` quando il conteggio non sta nel buffer.

- [ ] **Step 1: Test che falliscono** in `pdh.rs`:

```rust
#[test]
fn items_fit_rejects_counts_beyond_the_buffer() {
    assert!(items_fit(2, 24, 48));
    assert!(!items_fit(3, 24, 48));
    assert!(!items_fit(u32::MAX, usize::MAX / 2, 1024));
}

#[test]
fn a_null_item_name_reads_as_empty() {
    assert_eq!(item_name(PWSTR::null()), "");
}
```

- [ ] **Step 2: Eseguire** `cargo test -p oma-win pdh`. Expected: FAIL.
- [ ] **Step 3: Implementare.**
  - **`array` e `instances`:** controllano `items_fit` prima di `from_raw_parts`, e usano `item_name` per `szName`. Il `// SAFETY:` va aggiornato con la nuova garanzia.
  - **`used_pct`:** si toglie la copia da `storage/mod.rs` insieme al suo test duplicato `volume_usage`. Il test `used_percentage` di `memory.rs` resta, e gli si aggiunge il caso `used_pct(200, 50) == Some(75.0)`.
  - **Test hardware `reads_disk_temperatures_on_this_machine`:** per ogni disco si misura la lettura **tre volte** e si confronta con il limite di 200 ms il **minimo** dei tre tempi. Così un picco di carico non fa fallire il test, che però resta rosso per un disco sempre lento. Il test resta `#[ignore]` e lo esegue solo l'utente.
- [ ] **Step 4: Eseguire** `cargo test -p oma-win` e clippy. Expected: PASS.
- [ ] **Step 5: Revisione `ffi-safety-reviewer`** su `pdh.rs`.
- [ ] **Step 6: Commit:** `fix(oma-win): bound PDH item arrays, share used_pct and steady the disk timing test`.

---

### Task 11: test instabili di `log::session`

**Files:**
- Modify: `app/src-tauri/src/log/session/tests.rs`: test `late_start_ack_is_stopped` (1163), `commands_are_serialized_during_a_slow_stop` (1210), `stop_timeout_moves_to_error` (1226), `exit_stops_the_session_within_the_bound` (1241) e il `Gate` di prova (120–135).

**Interfaces:** nessuna; cambiano solo i test.

- [ ] **Step 1: Riprodurre** con la skill `superpowers:systematic-debugging`.
  - Mettere sotto carico la CPU: un secondo terminale con `cargo build --workspace --release`, oppure `cargo test --workspace` in parallelo.
  - Ripetere `cargo test -p oma-app log::session -- --test-threads=16` almeno 30 volte e annotare quale asserzione fallisce e con quale tempo.
- [ ] **Step 2: Correggere la causa,** non i numeri a caso:
  - dove il test aspetta un evento, si attende una condizione (`wait_until` o il gate) invece di un `sleep` reale;
  - il `flush_delay` del gate si sostituisce con un blocco che il test rilascia da sé;
  - un timeout che *deve* scadere (`with_timeout(100 ms)` con il gate bloccato) resta: non può fallire nel verso sbagliato;
  - i limiti superiori sui tempi trascorsi (`elapsed < 2 s`, `< 3 s`) si portano a 10 s, perché misurano «non resta appeso», non le prestazioni.
- [ ] **Step 3: Verificare:** 50 esecuzioni consecutive sotto lo stesso carico dello Step 1, tutte verdi. Il conteggio va riportato nel messaggio di commit.
- [ ] **Step 4: Commit:** `test(app): make the log session timing tests wait on conditions`.

---

### Task 12: grafico con tutta la serie sospesa

**Files:**
- Modify:
  - `app/src/components/advanced/HistoryChart.svelte` (`yRange` 112–131, sovrapposizione del testo);
  - `app/src/components/advanced/HistoryChart.test.ts`;
  - `app/src/lib/i18n/en.json`, `it.json`.

**Interfaces:**
- Produces: chiave `chart.suspended`, en «No readings while the device is idle or in standby», it «Nessuna lettura mentre il dispositivo è inattivo o in standby».

- [ ] **Step 1: Test che falliscono** in `HistoryChart.test.ts`, con i dati di prova già usati da `a suspended reading leaves a gap in the plotted data`:
  - `a fully suspended window keeps a y range and says why it is empty`: tutti i punti della finestra sospesi; la scala Y ha minimo e massimo finiti, e compare il testo `chart.suspended`;
  - `a partly suspended window shows no suspended notice`.
- [ ] **Step 2: Eseguire** `cd app && pnpm test HistoryChart`. Expected: FAIL.
- [ ] **Step 3: Implementare.**
  - **`yRange`:** con `dataMin` o `dataMax` nulli restituisce, in ordine:
    1. `yShown.get(key)`;
    2. l'intervallo morbido di `scaleOptions(unit)`;
    3. `[0, 1]`.
  - **Testo `chart.suspended`:** compare centrato sopra il grafico solo se **ogni** serie della finestra visibile non ha valori **e** la sua qualità più recente è «sospesa» (codice 2).
- [ ] **Step 4: Eseguire** `cd app && pnpm test && pnpm check`. Expected: PASS.
- [ ] **Step 5: Commit:** `fix(ui): keep the axis and explain a fully suspended chart`.

---

### Task 13: servizio, testi del client troncati e nomi ripuliti

**Files:**
- Create:
  - `service/OpenMonitorAdvanced.Service/Protocol/ProtocolText.cs`;
  - `service/OpenMonitorAdvanced.Service/Sensors/DisplayName.cs`;
  - `…Tests/Protocol/ProtocolTextTests.cs`;
  - `…Tests/Sensors/DisplayNameTests.cs`.
- Modify:
  - `Protocol/MessageCodec.cs` (410, 524, 857);
  - `Sensors/FeedRequest.cs` (160);
  - `Sensors/SchemaBuilder.cs` (nomi dei `WireDevice`: 105, 109, 139, 358, 378, 527, `ProcessDevice` 693; argomento `lhm.raw` 655);
  - `Sensors/LhmTree.cs` (log alla 389);
  - `Sensors/GateEpisode.cs` (log del modello, dopo il Task 4);
  - `…Tests/Sensors/SchemaBuilderTests.cs`, `…Tests/Protocol/CodecTests.cs`.

**Interfaces:**
- Produces:
  - `internal static string ProtocolText.Clip(string? text, int max = 64)`:
    - `null` diventa `""`;
    - ogni carattere di controllo (`char.IsControl`) diventa `?`;
    - oltre `max` caratteri il testo si taglia e si aggiunge `…`, senza spezzare una coppia surrogata;
  - `internal static string DisplayName.Clean(string? name)`:
    - toglie ogni carattere di controllo, NUL compreso, e gli spazi in testa e in coda;
    - lascia invariati gli spazi interni;
    - `null` diventa `""`.

**Regola:** `DisplayName.Clean` si applica **solo** ai nomi mostrati (nomi dei dispositivi nello schema, argomento `lhm.raw`, log). **Mai** alle identità: `StorageIdentityKey`, `DriveKey.Compute`, gli id dei dispositivi e i confronti tipo `s.Name == "CPU Total"` restano sui nomi originali, così le impostazioni per disco e gli id non cambiano.

- [ ] **Step 1: Test che falliscono:**
  - `ProtocolTextTests`:
    - un testo di 100 000 caratteri diventa lungo 65 (64 più `…`);
    - `"a\0b\nc"` diventa `"a?b?c"`;
    - una coppia surrogata a cavallo del limite non si spezza;
    - `null` diventa `""`;
  - `CodecTests`: `UnknownModuleIsABadRequest` continua a trovare `"gpu"`; un nuovo `AHugeUnknownTypeIsClipped` (tipo di 1 MiB) produce un messaggio di al massimo 128 caratteri, e la risposta `ErrorMessage` si codifica senza eccezioni con `MessageCodec.EncodeFrame`;
  - `DisplayNameTests`: `"SanDisk pSSD   \0\0\0"` diventa `"SanDisk pSSD"`, `"  CPU  Total \t"` diventa `"CPU  Total"`, `null` diventa `""`;
  - `SchemaBuilderTests`: un nodo storage con `Name = "SanDisk pSSD   \0"` produce un `WireDevice` con nome `"SanDisk pSSD"`, mentre id e hint d'identità restano **identici** a quelli che darebbe lo stesso nodo oggi.
- [ ] **Step 2: Eseguire** `dotnet test service/OpenMonitorAdvanced.slnx`. Expected: FAIL sui test nuovi.
- [ ] **Step 3: Implementare** nei punti elencati.
- [ ] **Step 4: Eseguire** `dotnet test service/OpenMonitorAdvanced.slnx` e `pwsh scripts/check-trim-warnings.ps1`. Expected: PASS, nessun avviso nuovo.
- [ ] **Step 5: Commit:** `fix(service): clip client text in bad_request and clean LHM display names`.

---

### Task 14: notices dei pacchetti NuGet Microsoft

**Files:**
- Modify:
  - `scripts/lib/OmaLicenses.psm1` (`Get-OmaNuGetPackageLicense`, 73–121);
  - `scripts/generate-licenses.ps1` (ciclo 156–169);
  - `scripts/tests/Licenses.Tests.ps1`;
  - `THIRD_PARTY_LICENSES.txt`, rigenerato.

**Interfaces:**
- Produces: l'oggetto di `Get-OmaNuGetPackageLicense` ha in più `Notices`, cioè il testo del file `THIRD-PARTY-NOTICES.TXT` alla radice del pacchetto (nome senza distinzione fra maiuscole e minuscole), oppure `$null`.

- [ ] **Step 1: Test Pester che fallisce** nel `Describe 'Get-OmaNuGetPackageLicense'`: un pacchetto finto con `THIRD-PARTY-NOTICES.TXT` fra i `$Files` dà `Notices` uguale al contenuto; un pacchetto senza dà `$null`.
- [ ] **Step 2: Eseguire** `Import-Module Pester -RequiredVersion 5.7.1; Invoke-Pester -Path scripts/tests/Licenses.Tests.ps1 -CI`. Expected: FAIL.
- [ ] **Step 3: Implementare.**
  - **`Get-OmaNuGetPackageLicense`:** legge il file delle notices.
  - **`generate-licenses.ps1`:** quando `Notices` c'è, aggiunge `[pscustomobject]@{ Title = 'Third-party notices'; Body = $lic.Notices }` ai testi della voce.
  - La deduplicazione esistente fa sì che i testi identici a quello del runtime non aggiungano corpi nuovi. L'unico atteso è quello di `System.IO.FileSystem.AccessControl` 5.0.0.
- [ ] **Step 4: Rigenerare e verificare.**
  - Controllare che `cargo about --version` stampi `0.9.2`.
  - Eseguire `pwsh scripts/generate-licenses.ps1`, poi `pwsh scripts/generate-licenses.ps1 -Check`.
  - Expected: il `-Check` passa. Il diff di `THIRD_PARTY_LICENSES.txt` mostra un solo corpo nuovo e i riferimenti `[Third-party notices (…)]` nelle voci dei pacchetti Microsoft.
- [ ] **Step 5: Eseguire** tutta la suite Pester (`-ExcludeTagFilter Integration`). Expected: PASS.
- [ ] **Step 6: Commit:** `build(licenses): include the third-party notices of Microsoft NuGet packages`.

---

### Task 15: documentazione e bump alla 0.4.1

**Files:**
- Modify:
  - `docs/follow-ups.md`: le voci chiuse vanno in una nuova sezione «Closed in M7a»; si aggiunge la bozza delle note di rilascio della 0.4.1; si aggiorna l'intestazione «last update»;
  - `README.md`: aggiornamento senza conferme; limiti noti dell'avvio automatico;
  - `CLAUDE.md`: nell'elenco delle milestone, la M7 con la M7a completata e la M7b–d da fare; la struttura (`storage/`, `svc/link/`);
  - questo piano: sezione «Esito dell'esecuzione»;
  - i file di versione, tramite `pwsh scripts/bump-version.ps1 0.4.1`.

- [ ] **Step 1: Aggiornare** follow-up, README e `CLAUDE.md`.
  - Ogni voce chiusa nomina il commit che la chiude.
  - La voce «Upgrade over an installed version» (riga 12 di oggi) si chiude, con la decisione dell'utente del 2026-10-04 che sostituisce quella del 2026-10-02. Resta aperta solo la segnalazione a Tauri dei segnaposto malformati.
- [ ] **Step 2: Bozza delle note di rilascio della 0.4.1** in `docs/follow-ups.md`, sezione «Draft: release notes for 0.4.1», nello stile della bozza della 0.4.0. Contenuto:
  - aggiornamento senza conferme (dalla 0.4.0 l'app si chiude in modo forzato, dalla 0.4.1 in poi in modo ordinato);
  - le correzioni visibili: grafico in standby, messaggi di apertura file, nomi dei dischi.
- [ ] **Step 3: Bump alla 0.4.1.** Eseguire `pwsh scripts/bump-version.ps1 0.4.1`, poi `pwsh scripts/check-version.ps1`. Expected: le cinque versioni e `Cargo.lock` a 0.4.1.
- [ ] **Step 4: Verifica completa:** `cargo fmt --all --check`, clippy, `cargo test --workspace`, `cd app && pnpm test && pnpm check && pnpm build`, `dotnet test service/OpenMonitorAdvanced.slnx`, Pester senza `Integration`, `pwsh scripts/generate-licenses.ps1 -Check`. Poi `PYTHONHASHSEED=0 graphify update .`.
- [ ] **Step 5: Commit:** prima `docs: record the M7a outcome`, poi `chore: release 0.4.1`. Nessun tag e nessun push, che si fanno solo su richiesta dell'utente.
- [ ] **Step 6: Costruire l'installer 0.4.1 senza eseguirlo:** `cd app && pnpm tauri build --bundles nsis`. Lo SHA-256 del setup va riportato all'utente per le verifiche dal vivo.

---

## Verifiche dal vivo (con l'utente)

Le esegue l'utente. L'agente prepara il setup e annota l'esito nel piano.

| # | Prova | Atteso |
|---|---|---|
| U1 | Con la 0.4.0 installata e l'app nel tray, si esegue il setup 0.4.1 in modalità grafica e si lascia l'opzione predefinita «Disinstalla prima» | nessun messaggio «in esecuzione»; a fine installazione, con «Avvia» spuntata, l'app si apre e il servizio riparte con lei |
| U2 | Con la 0.4.1 installata e l'app nel tray, `setup.exe /S` (stessa versione) | nessuna finestra e nessuna domanda; l'app si chiude in modo ordinato (`--quit`) e torna nel tray (`--minimized`) |
| U3 | Come U2, ma con l'app chiusa | l'app non si apre |
| U4 | Disinstallazione da Impostazioni › App con l'app aperta | la domanda «in esecuzione» c'è ancora |
| U5 | Si cancella la cartella dei log, poi «Apri cartella»; si usano i pulsanti di Informazioni | messaggio «Il file o la cartella non esiste»; gli altri pulsanti funzionano |
| U6 | Grafico di un HDD in standby con una finestra di 1 min | asse Y visibile e testo «Nessuna lettura mentre il dispositivo è inattivo o in standby» |
| U7 | `oma-app.exe --quit` dal terminale con l'app aperta, poi di nuovo con l'app chiusa | la prima volta l'app esce; la seconda il processo termina senza finestra |

## Esito dell'esecuzione

Branch `feat/m7a-manutenzione` (da `main` ad8f744), eseguito task per task con revisione dedicata; nessun tag e nessun push.

### Commit per task

| Task | Commit |
|---|---|
| 1 | `4b88fca` split di `storage.rs` in una cartella di moduli |
| 2 | `fbee564` split del link al servizio in macchina a stati, trasporto e driver |
| 3 | `7d5a650` formatter della chiave di visualizzazione fuori da `health.rs`; `1653e60` test di arrotondamento, segno e joule |
| 4 | `0dfdb90` split di `SensorHub` in file parziali, `GateEpisode` in un file proprio |
| 5 | `12d6445` `--quit` per chiudere l'istanza in esecuzione |
| 6 | `08daff0` l'installer chiude l'app prima dell'aggiornamento e la riapre dopo |
| 7 | `a977a65` riparazione del percorso dell'avvio automatico all'avvio |
| 8 | `e42b819` apertura con timeout e messaggio per percorso mancante; `8cb9f75` messaggi tradotti nel pulsante della cartella dei log |
| 9 | `f7ea18e` coda del link limitata e stato del servizio visibile in `Held` |
| 10 | `0df0c00` array PDH limitati, `used_pct` condiviso, test di tempo dei dischi stabilizzato; `ad147f6` `item_name` diventa `unsafe` con i suoi invarianti |
| 11 | `98bd9f7` test di sessione del log che aspettano condizioni |
| 12 | `1e957a5` asse mantenuto e spiegazione nel grafico tutto sospeso |
| 13 | `d1508af` testo del client troncato in `bad_request` e nomi LHM puliti |
| 14 | `6181767` avvisi di terze parti dei pacchetti NuGet Microsoft nelle licenze |
| 15 | documentazione (`docs: record the M7a outcome`) e bump alla 0.4.1 (`chore: release 0.4.1`) |

### Decisioni prese durante l'esecuzione

- I trailer dei commit nominano il modello che ha scritto il commit, non la stringa fissa del piano; la storia non è stata riscritta.
- Task 2: gli helper di test `machine()`, `subscribed()` e `subscribed_with()` stanno in `link/tests/mod.rs`, usati sia dai test della macchina sia da quelli del driver: è l'allargamento minimo per uno spostamento puro.
- Task 5: `main()` controlla `--quit` per prima e lancia `run_quit_only` (solo il plugin single-instance) invece di uscire nel `setup` dell'app completa. È più rigoroso sul punto di revisione 2: niente log, niente marcatore di crash, niente scritture delle impostazioni. Se U7 rivelasse un blocco, la correzione è piccola.
- Task 6: le quattro preoccupazioni dell'implementer sono accettate senza modifiche. La reinstallazione grafica della stessa versione chiude l'app senza domanda, la pagina di reinstallazione può attendere fino a 10 s e la casella «Avvia» apre la finestra senza minimizzarla, come da decisione dell'utente e §2.1 della spec. Il kill per nome di `oma-app.exe` chiude anche una build di sviluppo in esecuzione, come prescrive il piano (`KillProcess`).
- Task 7: la frase sul limite noto dell'avvio automatico nel README è in inglese, perché il README è interamente in inglese.
- Task 8: `open_target` restituisce `Option<OpenTarget>`, perché alcune cartelle note possono mancare; serve ai casi di percorso assente già esistenti.
- Task 9: il lettore usa un nuovo tentativo di 2 ms, annullabile, per i messaggi che non sono snapshot, invece di un invio bloccante, che andrebbe in stallo perché il thread del link attende il lettore alla chiusura. Con «stopped» in stato `Held` si mostra `disconnected()`, perché `ServiceState` non ha `Stopped` (come nel ciclo di connessione).
- Task 10: `item_name` è una `unsafe fn` con contratto `# Safety` e commenti `SAFETY` nei punti di chiamata. La regola della spec (`SAFETY` su ogni `unsafe`) prevale sulla firma del piano.
- Task 11: accettate le correzioni di due ulteriori test instabili sotto carico (`old_writer_events_cannot_mutate_a_new_session`, `busy_lock_ticks_keep_the_every_n_spacing`), oltre ai quattro del piano. Le prove di stress hanno saturato la CPU: da allora niente generatori di carico né cicli lunghi senza chiedere.
- Step 6 del Task 15 (build del setup 0.4.1 e SHA-256) rinviato a dopo la revisione finale dell'intero branch e le sue correzioni, altrimenti il setup sarebbe precedente alle correzioni.

### Verifiche dal vivo

| # | Esito |
|---|---|
| U1 | da fare (utente) |
| U2 | da fare (utente) |
| U3 | da fare (utente) |
| U4 | da fare (utente) |
| U5 | da fare (utente) |
| U6 | da fare (utente) |
| U7 | da fare (utente): oltre alla chiusura e al processo senza finestra, il valore Run e il marcatore di crash devono restare invariati |

Il setup 0.4.1 e il suo SHA-256 si preparano dopo la revisione finale del branch.
