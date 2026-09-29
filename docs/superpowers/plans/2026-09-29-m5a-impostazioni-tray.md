# Milestone 5a — Impostazioni e tray: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** le preferenze dell'utente vivono in un `settings.json` versionato, posseduto da Rust e applicato a caldo; la tray diventa completa (icona dinamica, tooltip, menu, chiusura nella tray, avvio con Windows); il servizio riceve dal client quali moduli e quali dischi leggere e dice lo stato di PawnIO; la finestra ha una vista Impostazioni con Generale, Fonti dati e Informazioni.

**Architecture:**
- **`oma-core::settings`:** tipi portabili, default, decodifica tollerante (file) o rigorosa (patch), normalizzazione e unione delle patch. Nessun I/O.
- **Shell (`app/src-tauri/src/settings/`):** `SettingsStore` con revisioni, un thread di scrittura che coalesce le modifiche e scrive in modo atomico, migrazioni idempotenti, comandi `get_settings`/`update_settings` ed evento `oma:settings`. Gli altri moduli della shell (campionatore, GPU, tray, avvio automatico, collegamento al servizio) si iscrivono allo store e applicano le modifiche.
- **Servizio:** protocollo v2 (`Subscribe` con moduli e dischi esclusi, `Hello` con lo stato di PawnIO), aggregazione delle richieste dei client e riconfigurazione di LibreHardwareMonitor sul thread che ne possiede l'albero, secondo la nota di fattibilità `docs/superpowers/references/m5/f1-service-reconfiguration.md`.
- **UI:** store delle impostazioni con migrazione una tantum da `localStorage`, unità e FPS applicati a formattazione e grafici, vista Impostazioni.

**Tech Stack:** invariato rispetto alla M4 (Rust 1.90 pinnato, `windows` 0.62, Tauri 2.11 con `@tauri-apps/cli` 2.11.5, Svelte 5, TypeScript 6, Vitest, .NET SDK 10.0.303, LibreHardwareMonitorLib 0.9.6, MessagePack 3.1.10, xUnit). Nuovo: `serde_json` anche in `oma-core`; feature `windows` `Win32_UI_Shell` in `oma-win`. Nessun nuovo plugin Tauri nella M5a.

**Spec:** `docs/superpowers/specs/2026-09-29-m5-regole-integrazione-design.md` (spec di dettaglio M5, ha la precedenza), §2, §5, §6, §7, §8.1; spec principale `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md` per tutto il resto. Evidenze: `docs/superpowers/references/m5/f1-service-reconfiguration.md` (nota di fattibilità su SMART per disco, riconfigurazione del servizio e PawnIO), da leggere nei task 10–13.

**Decisioni del piano** (interpretazioni della spec prese qui; ognuna con il suo costo):
- **P1. `rules` e `log` restano opachi nella M5a:** si leggono e si riscrivono come `serde_json::Value` invariati (default `{"overrides":{},"custom":[]}` e `{}`), e una patch che li tocca è rifiutata con `settings.error.readOnlyField`. M5b e M5c li tipizzano. *(Costo: nessuna validazione del loro contenuto fino alla M5b.)*
- **P2. `advanced` e `view` hanno campi facoltativi** (`Option` per campo, e le chiavi di `series` presenti solo se impostate): "assente" e "impostato al default" restano distinguibili, il writer non materializza default mai scelti (spec §2.4) e l'importazione da `localStorage` riempie solo i campi assenti. *(Costo: la UI legge questi campi con un default suo.)*
- **P3. La modalità anti-cheat si salva subito:** `set_anti_cheat` aggiorna lo store e chiede un salvataggio immediato (senza la coalescenza di 500 ms). Se il salvataggio fallisce la modalità resta applicata in memoria, lo stato di persistenza diventa `error` e la UI lo mostra (spec §2.2: la revisione resta dirty e si ritenta). Cambia la regola M4 "salvataggio fallito ⇒ nessun cambio". *(Costo: dopo un salvataggio fallito e un riavvio, la modalità può tornare spenta; l'utente è stato avvisato.)*
- **P4. `throughputUnit` vale per il traffico di rete** (vista Semplificata e pagine Rete); i dischi restano sempre in byte/s, come oggi. *(Costo: nessuno; un disco in bit/s non ha senso pratico.)*
- **P5. `SettingsState.seq`:** oltre a `revision` (cambia a ogni modifica applicata) e `persistedRevision`, ogni emissione dello stato porta un `seq` monotono, così la UI scarta anche gli aggiornamenti di sola persistenza o di effetti arrivati fuori ordine (spec §2.3).
- **P6. Vista dalla tray:** "Vista Semplificata" e "Vista Avanzata" mettono la vista in una richiesta pendente nella shell e aprono la finestra; la UI la legge con `take_pending_view` al montaggio e, se la finestra era già aperta, la riceve con l'evento `oma:navigate`. Scegliere una vista dalla tray aggiorna anche `view.last`.

## Global Constraints

- **Vincoli di M1–M4, tutti ancora validi:**
  - codice specifico di Windows solo in `crates/oma-win` e nei `cfg(windows)` di `oma-app`; `oma-core` e `oma-ipc` restano portabili;
  - budget: nucleo < 1% di CPU, tray < 30 MB, finestra < 200 MB WebView2 compresa, servizio < 1% di CPU e < 80 MB di memoria privata con un client a 1 s;
  - CSP invariata; nessun contenuto remoto; ogni nuovo comando Tauri registrato in tre punti: `generate_handler!` in `main.rs`, `app/src-tauri/build.rs`, `app/src-tauri/capabilities/default.json`;
  - `en.json` e `it.json` con le stesse chiavi; solo i token di `theme.css`; nessuna animazione continua oltre ai grafici;
  - codice, commenti e commit in inglese (conventional commits); prosa in italiano con gli accenti;
  - `// SAFETY:` su ogni `unsafe`, assert di dimensione a compile time per ogni struct FFI;
  - protocollo: mai `skip_serializing_if` sui tipi del protocollo, chiavi sempre presenti; fixture rigenerate solo con `OMA_WRITE_FIXTURES=1` a thread singolo;
  - mai input sintetico sul desktop dell'utente: tray, finestra, toast e scorciatoie li prova l'utente, su richiesta;
  - dopo ogni task che tocca il codice: `$env:PYTHONHASHSEED = '0'; graphify update .`; nei brief dei subagent vanno i comandi `graphify query`, `explain` e `path`, e le skill da usare (`superpowers:test-driven-development`, `superpowers:systematic-debugging`, per la UI nuova `frontend-design:frontend-design`).
- **File delle impostazioni:** `%APPDATA%\OpenMonitorAdvanced\settings.json`, chiavi camelCase, `version: 1`; temporaneo `settings.json.tmp`; file corrotto conservato come `settings.json.bad-<AAAAMMGG-hhmmss>-<suffisso>`.
- **Valori:** `intervalMs` 500–5000 a passi di 500 (default 1000); `chartFps` 60 | 30 | 15 (default 60); `advanced.window` 60 | 300 | 1800 | 3600 (default 300); correzione al valore valido più vicino, a pari distanza il maggiore.
- **Scrittura:** al più una ogni 500 ms, sempre alla chiusura; flush finale con attesa massima di 2 s; dopo un errore si ritenta ogni 5 s.
- **Tray:** icona RGBA 32×32; tooltip al massimo 127 unità UTF-16; cifre chiare `#f5eefe` su sfondo neutro `#211733` (`--surface-2`).
- **Avvio con Windows:** valore `OpenMonitor Advanced` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, dato `"<percorso dell'eseguibile>" --minimized`; `StartupApproved\Run` solo letto, mai scritto.
- **Protocollo:** `PROTOCOL_VERSION = 2`; nome della pipe invariato (`OpenMonitorAdvanced.Sensors.v1`); moduli `cpu`, `motherboard`, `memory`, `storage`, `controller`, `psu`; stati di PawnIO `ok`, `missing`, `unavailable`, `unknown`, `rebootPending`.
- **Comandi di verifica:** dalla radice del repository, salvo `cd app`. PowerShell 7; variabili d'ambiente con `$env:NOME = 'valore'`; `sc.exe`, mai l'alias `sc`.

## Review Focus

Condizioni che la spec implica e che i test di funzionalità da soli non coprirebbero, in ordine di probabilità. Ogni riga ha i suoi test nel task indicato.

1. **`%APPDATA%` sincronizzato (OneDrive, profilo roaming) o un antivirus che tiene aperto `settings.json`:** `ReplaceFileW` fallisce con una violazione di condivisione. Ci si aspetta che la modifica resti applicata, lo stato diventi `error` con il motivo, il writer ritenti e, al successo, torni `ok` senza perdere revisioni. Test: `replace_failure_keeps_dirty_and_retries`, `older_revision_never_overwrites_newer` (Task 2).
2. **Tray e UI che cambiano la stessa preferenza quasi insieme** (anti-cheat dalla tray mentre le Impostazioni sono aperte). Ci si aspetta uno stato finale coerente, lo stesso per tray, UI e file. Test: `concurrent_updates_serialize_last_writer_wins` (Task 2), `tray_and_settings_view_agree_on_anti_cheat` (Task 3), `stale_settings_event_is_ignored` (Task 14).
3. **Intervallo cambiato con lo storico pieno e il servizio collegato:** nessun buco né panic, 1 ora di storico ancora coperta, nuovo `Subscribe` al servizio. Test: `shrinking_capacity_keeps_newest`, `growing_capacity_keeps_everything`, `interval_change_applies_on_next_tick` (Task 4), `set_interval_resubscribes_when_connected` (Task 4).
4. **Valori estremi nell'icona della tray:** sensore assente, temperatura negativa, 100 °C, 212 °F, NaN. Ci si aspetta "—" per assente o non finito, tre cifre o il meno che stanno nei 32 px, mai un panic. Test: `icon_text_handles_extremes`, `render_fits_three_digits_and_minus` (Task 6).
5. **Il sensore scelto per l'icona sparisce** (GPU scollegata, servizio fermato): l'icona passa al sensore automatico invece di restare su "—" per sempre, e torna al sensore scelto quando riappare. Test: `missing_icon_sensor_falls_back_to_auto` (Task 7).

---

## Mappa dei file

**Rust**
- `crates/oma-core/src/settings/` (nuovo): `mod.rs` (tipi e default), `decode.rs` (decodifica tollerante e rigorosa, normalizzazione), `patch.rs` (unione delle patch, errori). `crates/oma-core/Cargo.toml`: `serde_json`.
- `crates/oma-core/src/history.rs`, `engine.rs`, `sampler.rs`: capacità dello storico e intervallo modificabili.
- `crates/oma-win/src/fsutil.rs` (nuovo): sostituzione atomica di un file. `crates/oma-win/src/autostart.rs` (nuovo): valore `Run` e lettura di `StartupApproved`. `crates/oma-win/src/shell_open.rs` (nuovo): `ShellExecuteW` per cartelle e file noti.
- `crates/oma-win/src/gpu/mod.rs`: interruttori per libreria.
- `crates/oma-win/src/svc/link.rs`, `feed.rs`, `status.rs`: canale unico, niente riconnessione dopo `Incompatible`/`PidMismatch`, intervallo e richiesta di fonti modificabili, stato di PawnIO.
- `crates/oma-ipc/src/message.rs`, `lib.rs`, `status.rs`, `tests/`: protocollo v2.
- `app/src-tauri/src/settings/` (nuovo): `mod.rs`, `store.rs` (store, revisioni, ascoltatori), `writer.rs` (thread di scrittura), `migrate.rs` (migrazioni), `commands.rs`.
- `app/src-tauri/src/i18n.rs` (nuovo), `tray_icon.rs` (nuovo), `tray.rs`, `window.rs`, `main.rs`, `service.rs`, `commands.rs`, `autostart.rs` (nuovo), `build.rs`, `capabilities/default.json`.

**.NET** (`service/OpenMonitorAdvanced.Service/`): `Protocol/Messages.cs`, `Protocol/MessageCodec.cs`, `Protocol/ProtocolConstants.cs`, `Pipe/ClientSession.cs`, `Sensors/SensorHub.cs`, `Sensors/LhmTree.cs`, `Sensors/IHardwareTree.cs`, `Sensors/PawnIoProbe.cs` e i file nuovi indicati dai task 11–13; test in `OpenMonitorAdvanced.Service.Tests/`.

**UI** (`app/src/`)
- `lib/settings.svelte.ts` (nuovo), `lib/units.svelte.ts` (nuovo), `lib/types.ts`, `lib/backend/{backend,tauri,mock}.ts`, `lib/advanced/persist.ts`, `lib/format.ts`, `lib/view.ts`, `lib/i18n/{en,it}.json`, `main.ts`, `App.svelte`.
- `components/TopBar.svelte`, `components/advanced/{HistoryChart,DevicePage}.svelte`, `components/common/Sparkline.svelte`, `components/simple/SimpleView.svelte`.
- `components/settings/` (nuovo): `SettingsView.svelte`, `GeneralSection.svelte`, `SourcesSection.svelte`, `AboutSection.svelte`, `PersistenceNotice.svelte`, `controls/{Toggle,Segmented,SelectField}.svelte`; `components/ServiceExplainer.svelte` (estratto da `TopBar.svelte`).

**Installer:** `app/src-tauri/nsis/oma.nsh` (rimozione del valore `Run` alla disinstallazione), `app/src/test/nsis-template.test.ts`.

**Documenti:** `docs/follow-ups.md`, `docs/perf-budget.md`, `README.md`, `README.it.md`, `CLAUDE.md` (piani M5).

---

### Task 1: `oma-core::settings` — tipi, default, decodifica e patch

**Files:**
- Create: `crates/oma-core/src/settings/mod.rs`, `crates/oma-core/src/settings/decode.rs`, `crates/oma-core/src/settings/patch.rs`
- Modify: `crates/oma-core/src/lib.rs` (`pub mod settings;`), `crates/oma-core/Cargo.toml` (`serde_json.workspace = true` tra le dipendenze)
- Test: moduli `#[cfg(test)]` negli stessi file

**Interfaces:**
- Produces (in `oma_core::settings`):
  ```rust
  pub const SETTINGS_VERSION: u32 = 1;
  pub struct Settings {
      pub version: u32,
      pub general: General,
      pub tray: Tray,
      pub sources: Sources,
      pub advanced: AdvancedState,
      pub view: ViewState,
      pub rules: serde_json::Value,   // P1: opaque in M5a
      pub log: serde_json::Value,     // P1: opaque in M5a
      pub migrations: Migrations,
  }
  pub enum Language { System, En, It }                 // "system" | "en" | "it"
  pub enum TemperatureUnit { C, F }                    // "c" | "f"
  pub enum ThroughputUnit { Bits, Bytes }              // "bits" | "bytes"
  pub enum ChartFps { Fps60, Fps30, Fps15 }            // JSON numbers 60 | 30 | 15
  pub enum DefaultView { Simple, Advanced, Last }      // "simple" | "advanced" | "last"
  pub enum ViewKind { Simple, Advanced }               // "simple" | "advanced"
  pub struct General { pub language: Language, pub temperature_unit: TemperatureUnit,
      pub throughput_unit: ThroughputUnit, pub interval_ms: u32, pub chart_fps: ChartFps,
      pub default_view: DefaultView }
  pub struct Tray { pub close_to_tray: bool, pub autostart: bool, pub icon_sensor: Option<String> }
  pub struct VendorLibraries { pub nvml: bool, pub nvapi: bool, pub adl: bool, pub igcl: bool }
  pub struct ServiceModules { pub cpu: bool, pub motherboard: bool, pub memory: bool,
      pub storage: bool, pub controller: bool, pub psu: bool }
  pub struct Sources { pub vendor_libraries: VendorLibraries, pub anti_cheat: bool,
      pub service_modules: ServiceModules, pub smart_disabled_drives: Vec<String> }
  pub struct AdvancedState { pub section: Option<String>, pub window: Option<u32>,
      pub series: std::collections::BTreeMap<String, Vec<String>> }   // P2
  pub struct ViewState { pub last: Option<ViewKind> }                   // P2
  pub struct Migrations { pub service_v1: bool, pub webview_v1: bool }
  impl Default for Settings;                   // spec §2.1 defaults
  impl ServiceModules { pub fn disabled(&self) -> Vec<&'static str>; } // names of the modules turned off, in the order above

  pub struct Diagnostic { pub path: String, pub kind: DiagnosticKind }  // path like "general.intervalMs"
  pub enum DiagnosticKind { WrongType, UnknownVariant, Corrected { from: String, to: String }, MissingVersion }
  pub enum VersionStatus { Current, Future(u32) }
  pub struct Decoded { pub settings: Settings, pub version: VersionStatus, pub diagnostics: Vec<Diagnostic> }
  pub fn decode_lenient(value: &serde_json::Value) -> Decoded;   // file loading
  pub fn encode(settings: &Settings) -> serde_json::Value;       // camelCase; None fields of advanced/view omitted

  pub struct PatchError { pub field: String, pub key: &'static str }
  pub fn apply_patch(current: &Settings, patch: &serde_json::Value) -> Result<Settings, PatchError>;
  ```
- **Regole:**
  - `decode_lenient`: chiavi mancanti → default senza diagnostica; chiavi sconosciute ignorate senza diagnostica; tipo errato o variante sconosciuta → default del campo con diagnostica; numeri fuori insieme → valore valido più vicino (a pari distanza il maggiore) con `Corrected`; `version` assente → trattata come 1 con `MissingVersion`; `version` > 1 → `VersionStatus::Future(v)` e decodifica di ciò che si capisce. `rules` e `log` si conservano come sono se sono oggetti, altrimenti il default.
  - `apply_patch`: la patch deve essere un oggetto; si unisce a `encode(current)` ricorsivamente sugli oggetti, gli array si sostituiscono interi, le chiavi omesse restano; poi si decodifica in modo **rigoroso**: qualsiasi diagnostica è un errore. Errori (con `key` come chiave i18n): `settings.error.notObject`, `settings.error.unknownField`, `settings.error.type`, `settings.error.range`, `settings.error.null` (per `null` su un campo non facoltativo; facoltativi: `tray.iconSensor`, `advanced.section`, `advanced.window`, `view.last`), `settings.error.readOnlyField` (per `version`, `migrations`, `rules`, `log`). `field` è il percorso camelCase del primo campo che fallisce.

- [ ] **Step 1: test che falliscono** (in `settings/*.rs`):
  - `defaults_match_the_spec`: `encode(&Settings::default())` è uguale a questo JSON (spec §2.1, senza `advanced`/`view` materializzati):
    ```json
    {"version":1,"general":{"language":"system","temperatureUnit":"c","throughputUnit":"bits","intervalMs":1000,"chartFps":60,"defaultView":"last"},
     "tray":{"closeToTray":true,"autostart":false,"iconSensor":null},
     "sources":{"vendorLibraries":{"nvml":true,"nvapi":true,"adl":true,"igcl":true},"antiCheat":false,
       "serviceModules":{"cpu":true,"motherboard":true,"memory":true,"storage":true,"controller":true,"psu":true},"smartDisabledDrives":[]},
     "advanced":{"series":{}},"view":{},"rules":{"overrides":{},"custom":[]},"log":{},
     "migrations":{"serviceV1":false,"webviewV1":false}}
    ```
  - `missing_keys_take_defaults_silently`: `{"version":1}` → `Settings::default()`, nessuna diagnostica;
  - `unknown_keys_are_ignored`: `{"version":1,"foo":1,"general":{"bar":true}}` → default, nessuna diagnostica;
  - `interval_is_snapped_to_the_nearest_step`: 700 → 500, 800 → 1000, 750 → 1000, 9000 → 5000, 0 → 500, ognuno con `Corrected`;
  - `fps_and_window_are_snapped`: `chartFps` 45 → 60 (a pari distanza tra 30 e 60 il maggiore), 20 → 15; `advanced.window` 100 → 60, 1000 → 300;
  - `wrong_types_fall_back_per_field`: `"intervalMs":"fast"` e `"closeToTray":"yes"` → default dei due campi, due `WrongType`, gli altri campi letti;
  - `unknown_enum_variant_falls_back`: `"language":"de"` → `System` con `UnknownVariant`;
  - `future_version_decodes_what_it_can`: `{"version":7,"general":{"intervalMs":2000}}` → `Future(7)`, `interval_ms == 2000`;
  - `missing_version_is_treated_as_1`;
  - `round_trip_is_stable`: `decode_lenient(&encode(s)).settings == s` per il default e per un valore con tutti i campi cambiati;
  - `patch_merges_objects_and_replaces_arrays`: patch `{"general":{"intervalMs":2000}}` cambia solo quel campo; patch `{"sources":{"smartDisabledDrives":["a"]}}` poi `{"sources":{"smartDisabledDrives":[]}}` → lista vuota; patch `{"advanced":{"series":{"gpu/x":["id"]}}}` aggiunge solo quella chiave;
  - `patch_rejects_invalid_values`: `intervalMs: 700` → `Err(field "general.intervalMs", key "settings.error.range")` (in una patch non si corregge); `language: "de"` → `settings.error.type`; `closeToTray: null` → `settings.error.null`; `tray.iconSensor: null` → ok; `{"nope":1}` → `settings.error.unknownField`; `{"version":2}`, `{"migrations":{"webviewV1":true}}`, `{"rules":{}}` → `settings.error.readOnlyField`; `[]` → `settings.error.notObject`;
  - `failed_patch_changes_nothing`: dopo un `Err` il valore corrente è invariato (la funzione è pura: verificare che restituisca l'errore e che `current` non sia toccato).
- [ ] **Step 2:** `cargo test -p oma-core settings` → FAIL (modulo inesistente).
- [ ] **Step 3: implementa** i tipi con `serde` (`rename_all = "camelCase"`; enum in minuscolo come sopra; `ChartFps` come numero) e le due funzioni. La decodifica tollerante lavora campo per campo sul `serde_json::Value`, così un campo rotto non trascina il resto; quella rigorosa riusa lo stesso codice trattando ogni diagnostica come errore.
- [ ] **Step 4:** `cargo test -p oma-core && cargo clippy -p oma-core --all-targets -- -D warnings` → verdi.
- [ ] **Step 5: commit** `feat(core): settings model with tolerant decoding and strict patches`.

---

### Task 2: store delle impostazioni nella shell — file, revisioni, writer, comandi

**Files:**
- Create: `crates/oma-win/src/fsutil.rs` (e `pub mod fsutil;` in `crates/oma-win/src/lib.rs`)
- Create: `app/src-tauri/src/settings/mod.rs`, `store.rs`, `writer.rs`, `commands.rs`
- Modify: `app/src-tauri/src/main.rs` (crea lo store prima del `Builder`, lo gestisce con `.manage`, emette `oma:settings`, flush alla chiusura), `app/src-tauri/build.rs`, `app/src-tauri/capabilities/default.json`
- Test: `#[cfg(test)]` in `store.rs` e `writer.rs`; `fsutil.rs` con un test su una cartella temporanea

**Interfaces:**
- Consumes: `Settings`, `decode_lenient`, `encode`, `apply_patch`, `PatchError`, `VersionStatus` (Task 1).
- Produces:
  ```rust
  // oma_win::fsutil
  pub fn replace_file(tmp: &Path, target: &Path) -> std::io::Result<()>; // ReplaceFileW; MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) when target does not exist

  // app/src-tauri/src/settings
  pub const EVENT_SETTINGS: &str = "oma:settings";
  pub fn settings_path() -> Option<PathBuf>;            // %APPDATA%\OpenMonitorAdvanced\settings.json
  pub trait SettingsFs: Send + Sync {
      fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>>;       // Ok(None) = not found
      fn write_atomic(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>; // tmp + fsync + replace_file
      fn preserve(&self, path: &Path, to: &Path) -> std::io::Result<()>;      // rename, never overwrites `to`
      fn remove(&self, path: &Path) -> std::io::Result<()>;
  }
  pub struct RealFs;                                     // cfg(windows) uses fsutil::replace_file
  #[serde(tag = "kind", rename_all = "camelCase")]
  pub enum Persistence { Ok, Pending, Recovered { path: String }, ReadOnly { reason: String }, Error { reason: String } }
  #[serde(tag = "kind", rename_all = "camelCase")]
  pub enum EffectStatus { Idle, Pending, Applied, Failed { reason: String } }
  pub struct ApplyStatus { pub service: EffectStatus, pub autostart: EffectStatus, pub vendor_libraries: EffectStatus }
  pub struct SettingsState { pub settings: serde_json::Value, pub revision: u64, pub persisted_revision: u64,
      pub seq: u64, pub persistence: Persistence, pub apply_status: ApplyStatus }   // camelCase (P5)
  pub enum Effect { Service, Autostart, VendorLibraries }
  pub type Listener = Box<dyn Fn(&Settings, &SettingsState) + Send + Sync>;
  pub struct SettingsStore { /* Arc<Inner> */ }
  impl SettingsStore {
      pub fn open(path: Option<PathBuf>, fs: Arc<dyn SettingsFs>) -> Self;
      pub fn settings(&self) -> Settings;
      pub fn state(&self) -> SettingsState;
      pub fn update(&self, patch: &serde_json::Value) -> Result<SettingsState, PatchError>;
      pub fn update_with(&self, change: impl FnOnce(&mut Settings)) -> SettingsState; // internal changes (migrations, tray, autostart read-back)
      pub fn flush_now(&self, timeout: Duration) -> Result<(), String>;            // P3; waits for this revision
      pub fn set_effect(&self, effect: Effect, status: EffectStatus);
      pub fn subscribe(&self, listener: Listener);   // called after every applied change and every state change
      pub fn shutdown(&self, timeout: Duration) -> Result<(), String>;  // final flush, then stops the writer
  }
  #[tauri::command] pub fn get_settings(store: State<'_, SettingsStore>) -> SettingsState;
  #[tauri::command] pub fn update_settings(store: State<'_, SettingsStore>, patch: serde_json::Value)
      -> Result<SettingsState, PatchErrorDto>;   // PatchErrorDto { field: String, key: String }
  ```
- **Regole:**
  - **Apertura:** file assente → default, `Persistence::Ok`, nessuna scrittura finché non cambia qualcosa; JSON valido → `decode_lenient`, diagnostiche nel log (`tracing::warn!`), `Ok`; `version` futura → `ReadOnly { reason: "futureVersion" }`, nessuna scrittura mai (le modifiche restano in memoria); JSON non valido → `preserve` nel nome `settings.json.bad-<AAAAMMGG-hhmmss>-<pid>` (se esiste già, suffisso `-2`, `-3`…), poi default salvati e `Recovered { path }`; se `preserve` fallisce o la lettura dà un errore diverso da "non trovato" → `Error { reason }` con scritture bloccate, default solo in memoria.
  - **Revisioni:** ogni modifica applicata incrementa `revision` e rende lo store dirty; `persisted_revision` sale solo dopo una scrittura riuscita di quella revisione (o di una successiva); `seq` sale a ogni emissione; lo stato `Pending` vale mentre `revision > persisted_revision` senza errori.
  - **Writer** (thread `oma-settings-writer`): scrive l'ultima revisione al più una volta ogni 500 ms; un errore imposta `Error { reason }`, lascia dirty e ritenta dopo 5 s; non scrive mai una revisione più vecchia di quella già scritta; `flush_now` e `shutdown` saltano l'attesa di 500 ms e aspettano al massimo il `timeout` (la chiusura usa 2 s e registra nel log un eventuale fallimento).
  - **Serializzazione delle modifiche:** tutte le modifiche passano da un solo mutex; gli ascoltatori si chiamano fuori dal mutex, in ordine di `seq`.
  - `main.rs`: un ascoltatore emette `EVENT_SETTINGS` con lo `SettingsState` se la finestra esiste; `RunEvent::Exit` chiama `shutdown(Duration::from_secs(2))` prima di chiudere il collegamento al servizio.

- [ ] **Step 1: test che falliscono** (con un `FakeFs` in memoria che può far fallire `write_atomic`, `preserve` o `read` a comando):
  - `missing_file_starts_with_defaults_and_writes_nothing`;
  - `valid_file_is_loaded_and_diagnostics_do_not_block`;
  - `corrupt_json_is_preserved_then_defaults_are_saved` (nome conservato con il prefisso `settings.json.bad-`, stato `Recovered`, il file originale non viene sovrascritto prima della conservazione);
  - `preserve_failure_blocks_writes` e `read_error_blocks_writes` (stato `Error`, nessuna `write_atomic` anche dopo un `update`);
  - `future_version_is_read_only_forever` (dopo `update` nessuna scrittura, stato `ReadOnly`);
  - `updates_coalesce_into_one_write` (tre `update` in 100 ms → una `write_atomic` con la revisione 3);
  - `replace_failure_keeps_dirty_and_retries` (primo `write_atomic` fallisce → `Error`, `persisted_revision` invariato; dopo il ritentativo → `Ok` e `persisted_revision == revision`; usare un ritardo di ritentativo iniettato di pochi millisecondi nei test);
  - `older_revision_never_overwrites_newer` (una scrittura lenta della revisione 1 e una più recente della 2: il file finale contiene la 2);
  - `concurrent_updates_serialize_last_writer_wins` (8 thread che cambiano `general.intervalMs`: lo stato finale e il file coincidono, `revision == 8`);
  - `invalid_patch_changes_nothing` (errore restituito con `field` e `key`, `revision` invariata, nessuna emissione);
  - `listeners_see_every_change_in_seq_order`;
  - `flush_now_waits_for_the_write` e `shutdown_is_bounded` (con un `write_atomic` bloccato, `shutdown(50 ms)` restituisce `Err` entro circa 50 ms);
  - `fsutil`: `replace_file_replaces_and_creates` su una cartella temporanea (target esistente e target assente).
- [ ] **Step 2:** `cargo test -p oma-app settings` → FAIL.
- [ ] **Step 3: implementa** store, writer, `RealFs`, comandi ed evento; registra `get_settings` e `update_settings` nei tre punti.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` → verdi.
- [ ] **Step 5: commit** `feat(app): persisted settings store with coalesced atomic writes`.

---

### Task 3: migrazioni e modalità anti-cheat nelle impostazioni

**Files:**
- Create: `app/src-tauri/src/settings/migrate.rs`
- Modify: `app/src-tauri/src/service.rs` (`ToggleState` legge e scrive tramite lo store; via `anti_cheat_path`, `load_anti_cheat`, `save_anti_cheat` dal percorso di scrittura), `app/src-tauri/src/settings/commands.rs` (`import_webview_state`), `main.rs`, `build.rs`, `capabilities/default.json`
- Test: `#[cfg(test)]` in `migrate.rs` e `service.rs`

**Interfaces:**
- Consumes: `SettingsStore`, `SettingsFs`, `FakeFs` (Task 2).
- Produces:
  ```rust
  pub fn migrate_service_v1(store: &SettingsStore, fs: &dyn SettingsFs, legacy: Option<&Path>, file_existed: bool);
  pub struct LegacyWebviewState { pub section: Option<String>, pub window: Option<u32>,
      pub series: BTreeMap<String, Vec<String>>, pub view: Option<String> }   // camelCase JSON
  #[tauri::command] pub fn import_webview_state(store: State<'_, SettingsStore>, legacy: LegacyWebviewState)
      -> Result<SettingsState, String>;   // Err("persist_failed" | "read_only")
  // service.rs
  impl ServiceShell { pub fn new(store: Arc<SettingsStore>, /* windows: */ status_table: ServiceStatusTable) -> Self; }
  ```
- **Regole (spec §2.4):**
  - `migrate_service_v1` (all'avvio, dopo `open`): se `migrations.serviceV1` è già vero non fa nulla (a parte cancellare un legacy rimasto, solo con `persistence` `Ok`). Se il file corrente **non** esisteva e il legacy esiste, importa `antiCheat`; in ogni caso imposta `serviceV1 = true` con `update_with` e chiama `flush_now(2 s)`; il legacy si cancella solo se il flush riesce. In `ReadOnly` o `Error` non tocca né lo store né il legacy.
  - `import_webview_state`: se `webviewV1` è già vero → `Ok(state)` senza cambiare nulla. Altrimenti riempie solo i campi ancora `None` (`section`, `window` se tra 60/300/1800/3600, `view.last` se `simple`/`advanced`) e le chiavi di `series` assenti, imposta `webviewV1 = true` e chiama `flush_now(2 s)`: `Ok` solo se il flush riesce, altrimenti `Err("persist_failed")` (la UI allora non cancella `localStorage` e riprova al prossimo avvio). In `ReadOnly` → `Err("read_only")`.
  - **Anti-cheat (P3):** `ServiceShell::set_anti_cheat` fa `update_with(antiCheat = enabled)`, manda `LinkCommand::SetAntiCheat`, aggiorna la casella della tray e chiama `flush_now(2 s)`; un errore di flush non annulla la modalità e compare nello stato di persistenza; il comando restituisce `Ok(status)`. Un ascoltatore dello store tiene allineata la casella della tray quando `antiCheat` cambia da un'altra origine.

- [ ] **Step 1: test che falliscono:**
  - `legacy_anti_cheat_is_imported_when_no_current_file` (legacy `{"antiCheat":true}`, nessun file → `antiCheat == true`, `serviceV1 == true`, legacy cancellato dopo il flush);
  - `current_file_wins_over_legacy` (file con `antiCheat: false` e legacy `true` → resta `false`, `serviceV1 == true`, legacy cancellato);
  - `legacy_is_kept_when_the_flush_fails` e `migration_retries_on_next_start`;
  - `read_only_store_never_touches_legacy`;
  - `webview_import_fills_only_absent_fields` (store con `advanced.window = 60` impostato: l'importazione con `window: 3600, section: "gpu/x"` lascia 60 e imposta la sezione);
  - `webview_import_is_idempotent` (seconda chiamata → nessun cambio di revisione);
  - `webview_import_reports_persist_failure`;
  - `webview_import_with_nothing_legacy_sets_the_marker`;
  - in `service.rs`: `anti_cheat_applies_even_when_the_flush_fails` (sostituisce `failed_save_does_not_toggle_or_claim_persistence`), `tray_and_settings_view_agree_on_anti_cheat` (un `update` dello store con `antiCheat` cambia la casella finta), `tray_and_command_use_the_same_toggle_path` aggiornato allo store.
- [ ] **Step 2:** `cargo test -p oma-app` → FAIL.
- [ ] **Step 3: implementa;** in `main.rs` l'ordine è: `SettingsStore::open` → `migrate_service_v1` → `ServiceShell::new(store.clone(), …)`. Registra `import_webview_state` nei tre punti.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` → verdi.
- [ ] **Step 5: commit** `feat(app): migrate the anti-cheat flag and web view state into settings`.

---

### Task 4: intervallo di campionamento modificabile a caldo

**Files:**
- Modify: `crates/oma-core/src/history.rs` (`set_capacity`), `crates/oma-core/src/engine.rs` (`set_history_capacity`), `crates/oma-core/src/sampler.rs` (`IntervalHandle`)
- Modify: `crates/oma-win/src/svc/link.rs` (`LinkCommand::SetInterval`)
- Modify: `app/src-tauri/src/main.rs`, `app/src-tauri/src/commands.rs` (`AppState` con `IntervalHandle`; `get_session` riporta l'intervallo corrente)
- Test: moduli di test degli stessi file

**Interfaces:**
- Consumes: `SettingsStore::subscribe`, `Settings::general.interval_ms` (Task 1–2).
- Produces:
  ```rust
  impl History { pub fn set_capacity(&mut self, capacity: usize); }        // keeps the newest samples
  impl Engine { pub fn set_history_capacity(&mut self, capacity: usize); }
  #[derive(Clone)] pub struct IntervalHandle(/* Arc<AtomicU64> + thread handle for unpark */);
  impl IntervalHandle { pub fn new(interval: Duration) -> Self; pub fn get(&self) -> Duration; pub fn set(&self, interval: Duration); }
  impl Sampler { pub fn spawn<F>(engine: Arc<Mutex<Engine>>, interval: IntervalHandle, on_tick: F) -> Self; }
  // oma_win::svc
  pub enum LinkCommand { SetAntiCheat(bool), Start, SetInterval(u32) }
  ```
- **Regole:**
  - il ciclo del campionatore rilegge l'intervallo dopo ogni tick; `set` sveglia il thread (`unpark`), che ricalcola la scadenza come `inizio dell'ultimo tick + nuovo intervallo` (o subito, se già passata): il nuovo ritmo vale dal tick successivo, senza raffiche;
  - alla modifica di `general.intervalMs` la shell, nell'ordine: `engine.set_history_capacity(history_capacity(nuovo))`, `interval.set(nuovo)`, `LinkCommand::SetInterval(ms)`; all'avvio l'intervallo iniziale viene dalle impostazioni, non più da `SAMPLE_INTERVAL` (che resta come default in `oma-core::settings`);
  - `set_capacity` più piccola scarta i campioni più vecchi (timestamps e serie insieme); più grande li tiene tutti; le statistiche non cambiano;
  - il link, se collegato, manda subito un nuovo `Subscribe` con il nuovo intervallo e usa il nuovo valore per il timeout dello snapshot vecchio (3 intervalli); se non collegato lo usa alla prossima connessione.

- [ ] **Step 1: test che falliscono:**
  - `shrinking_capacity_keeps_newest` (10 campioni, capacità 4 → restano gli ultimi 4, serie allineate);
  - `growing_capacity_keeps_everything` (poi `push` fino alla nuova capacità senza perdite);
  - `interval_change_applies_on_next_tick` (con `spawn_ticker` e un tick finto: intervallo 20 ms, poi `set(80 ms)`: dopo il cambio la distanza tra i tick è ≥ 70 ms e nessun tick parte due volte di seguito entro 20 ms);
  - `set_interval_resubscribes_when_connected` (nel test del link con `FakeConn`: dopo `SetInterval(2000)` il messaggio inviato è `Subscribe` con `interval_ms == 2000`);
  - `set_interval_while_disconnected_is_used_on_connect`;
  - in `commands.rs`: `session_reports_the_current_interval`.
- [ ] **Step 2:** `cargo test --workspace` → FAIL.
- [ ] **Step 3: implementa.**
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` → verdi.
- [ ] **Step 5: commit** `feat(core): change the sampling interval without restarting`.

---

### Task 5: interruttori per libreria dei vendor GPU

**Files:**
- Modify: `crates/oma-win/src/gpu/mod.rs` (`VendorSwitch` per libreria, creazione pigra per libreria), `crates/oma-win/src/lib.rs` (riesportazioni)
- Modify: `app/src-tauri/src/commands.rs` (`StartupState`, versione non Windows di `VendorSwitch`), `app/src-tauri/src/main.rs` (maschera iniziale dalle impostazioni; ascoltatore)
- Test: `crates/oma-win/src/gpu/mod.rs` (test esistenti con i layer finti), `commands.rs`

**Interfaces:**
- Consumes: `Settings::sources.vendor_libraries` (Task 1), `SettingsStore::subscribe`, `set_effect(Effect::VendorLibraries, …)` (Task 2).
- Produces:
  ```rust
  #[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Vendor { Nvml, Nvapi, Adl, Igcl }
  #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)] pub struct VendorMask(u8);   // bit per Vendor
  impl VendorMask { pub const ALL: VendorMask; pub fn contains(self, v: Vendor) -> bool; pub fn with(self, v: Vendor, on: bool) -> Self; }
  impl VendorSwitch {
      pub fn new(master: bool, libraries: VendorMask) -> Self;   // master = !safe_mode
      pub fn enabled(&self) -> bool;                              // master
      pub fn enable(&self);                                       // "Riattiva" (master on)
      pub fn set_libraries(&self, libraries: VendorMask);
      pub fn effective(&self) -> VendorMask;                      // libraries if master, else empty
  }
  type MakeLayer = Box<dyn FnOnce() -> Box<dyn GpuLayer> + Send>;
  // GpuProvider::with_layers(enumerate, base, make_vendor: [(Vendor, MakeLayer); 4], switch)
  ```
- **Regole (spec §2.5):**
  - un layer vendor si crea (e la sua DLL si carica) solo alla prima discovery che lo vede attivo nella maschera effettiva, poi resta per tutta la vita del processo;
  - la discovery usa solo i layer della maschera effettiva, in ordine di priorità nvml, nvapi, adl, igcl, poi i layer base;
  - `poll` confronta la maschera effettiva con quella vista alla discovery e, se diversa, chiede `Rediscover`;
  - la modalità sicura (`master = false`) ha la precedenza sugli interruttori; "Riattiva" riaccende il master e rispetta gli interruttori;
  - la shell imposta la maschera iniziale dalle impostazioni e, a ogni modifica, chiama `set_libraries` e `set_effect(VendorLibraries, Applied)`. `StartupStatus` resta com'è: dice solo della modalità sicura.

- [ ] **Step 1: test che falliscono** (layer finti con contatore di creazioni):
  - `disabled_vendor_is_never_created`;
  - `enabling_one_vendor_creates_only_that_layer`;
  - `disabling_a_loaded_vendor_excludes_it_without_dropping` (il contatore di drop resta 0 e i campi ricadono sui layer base);
  - `mask_change_requests_rediscover`;
  - `safe_mode_master_overrides_libraries` e `reenable_respects_library_switches`;
  - i test esistenti del provider GPU aggiornati alla nuova firma.
- [ ] **Step 2:** `cargo test -p oma-win gpu` → FAIL.
- [ ] **Step 3: implementa.**
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` → verdi; `cargo test -p oma-win -- --include-ignored` con i test hardware GPU verdi (RTX 4080 e iGPU AMD).
- [ ] **Step 5: commit** `feat(gpu): per-library vendor switches`.

---

### Task 6: traduzioni in Rust, icona e tooltip della tray (funzioni pure)

**Files:**
- Create: `app/src-tauri/src/i18n.rs`, `app/src-tauri/src/tray_icon.rs`
- Modify: `app/src-tauri/src/tray.rs` (sostituisce `labels_for` con `i18n`), `app/src/lib/i18n/en.json`, `it.json` (chiavi della tray)
- Test: `#[cfg(test)]` negli stessi file

**Interfaces:**
- Consumes: `Language`, `TemperatureUnit` (Task 1); `oma_core::model::{Schema, Unit, Label}`.
- Produces:
  ```rust
  // i18n.rs
  #[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Lang { En, It }
  pub fn resolve(language: Language, system_locale: &str) -> Lang;          // System → it for "it", "it-IT", "it_CH"…, else En
  pub fn t(lang: Lang, key: &str, params: &[(&str, &str)]) -> String;      // lang → en → key; replaces {name}
  pub fn sensor_label(lang: Lang, label: &Label) -> String;                // "sensor.<key>" with {arg}
  pub const RUST_KEYS: &[&str];   // every catalog key the Rust code uses
  // tray_icon.rs
  pub const ICON_SIZE: u32 = 32;
  pub struct IconStyle { pub background: [u8; 4], pub foreground: [u8; 4] }
  pub const NEUTRAL: IconStyle;   // background #211733, foreground #f5eefe
  pub fn icon_text(value: Option<f64>, unit: Unit, temperature: TemperatureUnit) -> String;
  pub fn render(text: &str, style: IconStyle) -> Vec<u8>;                  // RGBA, ICON_SIZE² × 4 bytes
  pub struct TooltipItem { pub label_key: &'static str, pub value: Option<f64>, pub unit: Unit }
  pub fn tooltip(lang: Lang, items: &[TooltipItem], temperature: TemperatureUnit) -> String;
  ```
- **Regole:**
  - chiavi nuove: `tray.open` (esistente), `tray.viewSimple`, `tray.viewAdvanced`, `tray.antiCheat` (esistente), `tray.quit` (esistente), `tray.tooltip.cpu` ("CPU"), `tray.tooltip.gpu` ("GPU"), `tray.tooltip.ram` ("RAM"); en: "Simple view", "Advanced view"; it: "Vista Semplificata", "Vista Avanzata";
  - `icon_text`: `None` o non finito → "—"; temperatura convertita in °F se richiesto (`c * 9/5 + 32`); arrotondamento all'intero; limitato a −99…999; nessuna lettera di unità nel testo; l'unità è un piccolo segno a parte in alto a destra (`unit_mark`: `°` per `Unit::Celsius`, sia °C sia °F, `%` per `Unit::Percent`, nessuno per le altre unità e per "—"; `render(text, mark, style)`), deciso con l'utente dopo la verifica dal vivo della tray;
  - `render`: quadrato arrotondato (raggio 6 px, angoli trasparenti) nel colore di sfondo, testo centrato nel colore del primo piano con un font bitmap scritto a mano nel sorgente per `0–9`, `-` e `—` (niente dipendenze); fino a 2 caratteri le cifre sono alte almeno 16 px, con 3 caratteri il testo sta tutto entro i 30 px centrali;
  - `tooltip`: `"CPU 45 °C · GPU 62 °C · RAM 48 %"`, con i numeri formattati come nella UI (temperatura intera, percentuale intera), voci senza valore omesse, `" · "` come separatore; se supera 127 unità UTF-16 si tronca all'ultima voce intera e si aggiunge `…`.

- [ ] **Step 1: test che falliscono:**
  - `resolve_follows_settings_then_system` (`It` con `Language::It`; `System` + "it-IT" → `It`; `System` + "de-DE" → `En`; `En` + "it-IT" → `En`);
  - `t_falls_back_to_english_then_key` e `t_replaces_params`;
  - `sensor_label_uses_the_catalog_and_arg` (`Label::with_arg("cpu.load.thread","3")` → testo del catalogo con 3);
  - `rust_keys_exist_in_both_catalogs` (ogni chiave di `RUST_KEYS` in `en.json` e `it.json`);
  - `icon_text_handles_extremes` (`None` → "—"; `NaN` → "—"; 45.4 °C → "45"; 100 → "100"; 100 °C in °F → "212"; −5.6 → "-6"; 1500 → "999"; −150 → "-99");
  - `render_is_rgba_32x32` (lunghezza 4096; pixel (0,0) trasparente; pixel (16,2) nel colore di sfondo);
  - `render_fits_three_digits_and_minus` (per "212", "-99", "—": nessun pixel del primo piano nelle colonne 0 e 31);
  - `render_draws_two_digits_large` (per "88" i pixel del primo piano coprono almeno 16 righe);
  - `tooltip_formats_and_truncates` (esempio sopra; con un valore assente la voce manca; con etichette lunghe la stringa è ≤ 127 unità UTF-16 e finisce con `…`).
- [ ] **Step 2:** `cargo test -p oma-app` → FAIL.
- [ ] **Step 3: implementa;** `tray.rs` usa `i18n::t` al posto di `labels_for` (i test `italian_locales_get_italian_labels` e `other_locales_fall_back_to_english` passano su `i18n::resolve` + `t`).
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cd app && pnpm test` → verdi (il test delle chiavi i18n della UI vede le chiavi nuove in entrambe le lingue).
- [ ] **Step 5: commit** `feat(app): Rust catalog lookups and tray icon rendering`.

---

### Task 7: tray completa, chiusura nella tray, seconda istanza e flush del log

**Files:**
- Modify: `app/src-tauri/src/tray.rs` (menu, `TrayController`), `app/src-tauri/src/window.rs` (vista pendente), `app/src-tauri/src/main.rs` (callback del campionatore sempre attivo per la tray, `ExitRequested`, single-instance, `run_return`), `app/src-tauri/src/commands.rs` (`take_pending_view`), `build.rs`, `capabilities/default.json`
- Test: `#[cfg(test)]` in `tray.rs`, `window.rs`, `main.rs` (funzioni pure estratte)

**Interfaces:**
- Consumes: `i18n`, `tray_icon` (Task 6); `SettingsStore` (Task 2); `ViewKind` (Task 1).
- Produces:
  ```rust
  pub const EVENT_NAVIGATE: &str = "oma:navigate";                 // payload: "simple" | "advanced"
  pub struct NavState(Mutex<Option<ViewKind>>);
  pub fn show_main_on(app: &AppHandle, view: ViewKind);            // window.rs: pending view, show, emit if open
  #[tauri::command] pub fn take_pending_view(nav: State<'_, NavState>) -> Option<ViewKind>;
  pub struct TrayController { /* TrayIcon, menu items, last icon key, last tooltip */ }
  impl TrayController {
      pub fn update(&self, schema: &Schema, snapshot: &Snapshot, settings: &Settings);  // called every tick
      pub fn relabel(&self, lang: Lang);
  }
  pub fn icon_sensor(schema: &Schema, chosen: Option<&str>) -> Option<String>;      // pure
  pub fn opens_window(args: &[String]) -> bool;                                     // pure: false with "--minimized"
  pub fn keep_running_on_last_close(close_to_tray: bool) -> bool;                   // pure
  ```
- **Regole (spec §2.6):**
  - menu nell'ordine: Apri, Vista Semplificata, Vista Avanzata, separatore, Modalità compatibile anti-cheat, separatore, Esci (le voci del log arrivano con la M5c); le etichette si rigenerano con `set_text` quando cambia `general.language`;
  - `icon_sensor`: il sensore scelto se esiste nello schema; altrimenti `…/temperature/core` della prima GPU con `properties.integrated != "true"`; altrimenti la temperatura CPU `cpu/0/temperature/package` o `cpu/0/temperature/tctl`; altrimenti `cpu/0/load/total`; `None` se non c'è nulla;
  - tooltip: CPU = temperatura (package o tctl) se esiste, altrimenti carico totale; GPU = temperatura core della GPU dedicata principale; RAM = `memory/0/load/...` (etichetta `memory.load`);
  - `update` chiama `set_icon` e `set_tooltip` solo se testo o stile dell'icona, o il testo del tooltip, sono cambiati; il callback del campionatore aggiorna la tray a ogni tick anche a finestra chiusa, mentre gli eventi verso la UI restano solo con la finestra aperta;
  - `ExitRequested { code: None }`: `prevent_exit()` solo se `keep_running_on_last_close(settings.tray.close_to_tray)`;
  - single-instance: la finestra si apre solo se `opens_window(&args)`;
  - `main` usa `app.run_return(...)`; dopo il ritorno fa `drop` della guardia del log e poi `std::process::exit(code)`;
  - voci "Vista Semplificata"/"Vista Avanzata": `show_main_on(app, view)` e `update_with(view.last = view)` (P6).

- [ ] **Step 1: test che falliscono:**
  - `icon_sensor_prefers_the_chosen_one`, `icon_sensor_auto_picks_the_dedicated_gpu`, `icon_sensor_falls_back_to_cpu_then_load`, `missing_icon_sensor_falls_back_to_auto` (scelto assente → automatico; poi presente → scelto);
  - `update_skips_unchanged_icon_and_tooltip` (con un `TrayBackend` finto dietro un trait: due tick uguali → una sola `set_icon`, una sola `set_tooltip`);
  - `opens_window_ignores_minimized_launches` (`["oma-app.exe","--minimized"]` → false; `["oma-app.exe"]` → true);
  - `last_close_exits_when_close_to_tray_is_off`;
  - `pending_view_is_taken_once`.
- [ ] **Step 2:** `cargo test -p oma-app` → FAIL.
- [ ] **Step 3: implementa;** `TrayController` usa un trait `TrayBackend` (`set_icon(Vec<u8>)`, `set_tooltip(String)`, `set_labels(...)`) implementato sulla `TrayIcon` di Tauri, così la logica si testa senza finestra. Registra `take_pending_view` nei tre punti.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo build -p oma-app` → verdi.
- [ ] **Step 5: commit** `feat(app): full tray with a dynamic icon, view items and close-to-tray`.

---

### Task 8: avvio con Windows

**Files:**
- Create: `crates/oma-win/src/autostart.rs` (e `pub mod autostart;`), `app/src-tauri/src/autostart.rs`
- Modify: `app/src-tauri/src/main.rs` (ascoltatore di `tray.autostart`), `build.rs`, `capabilities/default.json`, `app/src-tauri/nsis/oma.nsh` (rimozione alla disinstallazione), `app/src/test/nsis-template.test.ts`
- Test: `crates/oma-win/src/autostart.rs` (test su una sottochiave di test in HKCU), `app/src-tauri/src/autostart.rs`, `nsis-template.test.ts`

**Interfaces:**
- Consumes: `SettingsStore::{subscribe, update_with, set_effect}`, `Effect::Autostart` (Task 2).
- Produces:
  ```rust
  // oma_win::autostart
  pub const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
  pub const APPROVED_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
  pub const VALUE_NAME: &str = "OpenMonitor Advanced";
  pub fn command_line(exe: &Path) -> String;                    // "\"<exe>\" --minimized"
  pub struct RunKey { pub run_subkey: String, pub approved_subkey: String, pub value_name: String }
  pub enum Effective { NotConfigured, Enabled, DisabledByWindows, Unknown }
  impl RunKey {
      pub fn production() -> Self;
      pub fn read(&self) -> std::io::Result<Option<String>>;
      pub fn write(&self, exe: &Path) -> std::io::Result<()>;
      pub fn remove(&self) -> std::io::Result<()>;                // Ok if absent
      pub fn effective(&self) -> Effective;                        // reads APPROVED_SUBKEY only
  }
  pub fn approved_state(bytes: Option<&[u8]>) -> Effective;       // pure
  // app/src-tauri/src/autostart.rs
  #[serde(rename_all = "camelCase")] pub struct AutostartStatus { pub configured: bool, pub effective: Effective, pub error: Option<String> }
  #[tauri::command] pub fn refresh_autostart(store: State<'_, SettingsStore>) -> AutostartStatus;
  ```
- **Regole (spec §2.6):**
  - `approved_state`: valore assente → `Enabled` (se la voce `Run` esiste); primo byte `0x02` o `0x06` → `Enabled`; `0x03` o `0x07` → `DisabledByWindows`; vuoto o altro → `Unknown`. Il formato non è documentato: il valore si legge soltanto, mai si scrive; `Unknown` nella UI diventa "stato gestito da Windows" con il link `ms-settings:startupapps`;
  - al cambio di `tray.autostart` la shell scrive o rimuove il valore `Run` e imposta `set_effect(Autostart, Applied | Failed { reason })`; una scrittura fallita non viene dichiarata riuscita e `tray.autostart` torna al valore letto;
  - `refresh_autostart` rilegge il registro e, se la voce configurata non coincide con `tray.autostart`, lo riallinea con `update_with`;
  - verifica prima di chiudere il task: con `reg query` su `HKCU\...\StartupApproved\Run` si leggono i valori già presenti su questo PC e si confrontano con lo stato mostrato da Gestione attività, chiedendo all'utente di leggerlo (nessuna scrittura);
  - NSIS: nella macro di disinstallazione di `oma.nsh`, fuori dalla modalità aggiornamento, `DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "OpenMonitor Advanced"`.

- [ ] **Step 1: test che falliscono:**
  - `command_line_quotes_the_path` (`C:\Program Files\OpenMonitor Advanced\oma-app.exe` → `"C:\Program Files\OpenMonitor Advanced\oma-app.exe" --minimized`);
  - `approved_state_reads_known_prefixes` (i casi sopra);
  - `run_value_round_trips` su `Software\OpenMonitorAdvanced\Tests\Run-<pid>` (scrittura, lettura, rimozione, rimozione di un valore assente = `Ok`; la sottochiave di test si cancella alla fine);
  - `failed_write_is_not_reported_as_applied` (con un `RunKey` su una sottochiave non scrivibile o con un trait finto);
  - `refresh_realigns_the_setting`;
  - in `nsis-template.test.ts`: `uninstall removes the autostart value outside update mode`.
- [ ] **Step 2:** `cargo test --workspace && cd app && pnpm test` → FAIL.
- [ ] **Step 3: implementa;** registra `refresh_autostart` nei tre punti.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cd app && pnpm test` → verdi.
- [ ] **Step 5: commit** `feat(app): start with Windows through the user's Run key`.

---

### Task 9: collegamento al servizio — canale unico e niente riconnessione dopo un'incompatibilità

**Files:**
- Modify: `crates/oma-win/src/svc/link.rs` (lettore della pipe che scrive nel canale dei comandi; `next_event` senza fette da 50 ms; macchina a stati per `Incompatible`/`PidMismatch`)
- Test: moduli di test di `link.rs`

**Interfaces:**
- Consumes: niente di nuovo.
- Produces: nessuna API pubblica nuova; `POLL_SLICE` sparisce.
- **Regole (spec §2.9, `docs/follow-ups.md`):**
  - il thread di lettura della connessione inoltra messaggi e chiusure nello stesso canale di `Input` (nuove varianti `Input::Message(u64, Message)` e `Input::Closed(u64, CloseReason)`, con l'id della connessione così un evento di una connessione già chiusa si scarta); `next_event` si blocca con `recv_timeout(scadenza)` o `recv()` e non si sveglia più a 20 Hz;
  - dopo `Incompatible` o `PidMismatch` la macchina non si riconnette ogni 5 s: continua a interrogare l'SCM a ogni `retry` e si riconnette solo su `LinkCommand::Start` o quando l'SCM mostra un cambio di stato del servizio (per esempio `Stopped` → `Running`, cioè un nuovo processo).

- [ ] **Step 1: test che falliscono:**
  - `connected_link_does_not_wake_between_events` (con un connettore finto che conta le chiamate a `recv`: in 1 s di inattività, con intervallo 1000 ms e nessun messaggio, il thread non esegue più di 3 attese);
  - `messages_and_commands_share_one_queue` (un comando inviato mentre arriva uno snapshot viene gestito entro 10 ms);
  - `late_events_of_a_closed_connection_are_ignored`;
  - `incompatible_is_not_retried_until_start` (dopo `Incompatible`, 20 s di tempo finto senza nuove connessioni; `Start` → una connessione);
  - `pid_mismatch_waits_for_an_scm_change` (con l'SCM finto che passa da `Running(pid 1)` a `Stopped` a `Running(pid 2)` → una nuova connessione);
  - i test esistenti che usavano `POLL_SLICE` aggiornati.
- [ ] **Step 2:** `cargo test -p oma-win svc` → FAIL.
- [ ] **Step 3: implementa.**
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings` → verdi; `cargo test -p oma-win -- --include-ignored svc` (pipe vera con il server di prova) verde.
- [ ] **Step 5: commit** `perf(svc): block the link thread until the next event`.

---

**Decisioni prese dalla nota di fattibilità** (`docs/superpowers/references/m5/f1-service-reconfiguration.md`, da leggere per intero nei Task 10–13; i riferimenti `file:riga` sono allo stato `43fbe2f`):
- **P7. D6 resta globale (verdetto c di F1):** nessuna API pubblica di LibreHardwareMonitor 0.9.6 filtra un disco prima della discovery, e aggirare il gate sveglierebbe il disco. L'interruttore "SMART disattivato per il disco X" vale solo dopo la discovery: niente `CHECK POWER MODE`, niente `Update` e disco fuori dallo schema del servizio. La UI dichiara il limite. Il follow-up del disco USB resta aperto, con la soluzione indicata in F1.4 (ripiego SAT, fuori dalla M5a).
- **P8. `driveKey` sul filo:** `smartDisabledDrives` porta `sha256(trim(model) + "\0" + trim(serial))` dei testi del descrittore, in esadecimale minuscolo. L'app persiste l'id core del disco e lo traduce a ogni `Subscribe`; un disco senza modello o seriale nel descrittore non è selezionabile.
- **P9. Blocco `service` nello `Schema` del protocollo:** `service: { activeModules, smartDisabledDrives, reconfiguration: "applied" | "pending" | "failed", smartBlockedBy }`, con chiavi sempre presenti; un suo cambio conta come cambio di struttura. Serve ad `applyStatus.service` e alla distinzione tra richiesta locale e stato globale. Si rigenera anche `schema.msgpack`.
- **P10. Storage "morbido":** spegnere lo storage non chiama mai `IsStorageEnabled = false` a runtime. Il giro dei dischi si ferma, cache e dischi risolti si svuotano e il gruppo resta aperto; la prima abilitazione passa sempre dal gate D6.
- **P11. PawnIO:** `ok` quando il device si apre, anche senza chiave di disinstallazione (con un log informativo). `rebootPending` solo con il marcatore `HKLM\SOFTWARE\OpenMonitorAdvanced` `PawnIoRebootRequestedUtc`, scritto dall'installer nel ramo 3010, più recente del boot corrente. Lo stato PnP del devnode finisce solo nel log.

### Task 10: protocollo v2, `driveKey` e stato di PawnIO

**Files:**
- Modify: `crates/oma-ipc/src/lib.rs` (`PROTOCOL_VERSION = 2`), `crates/oma-ipc/src/message.rs` (nuovi campi), `crates/oma-ipc/tests/*` (fixture); create `crates/oma-ipc/src/drive_key.rs`
- Modify: `service/OpenMonitorAdvanced.Service/Protocol/{Messages.cs,MessageCodec.cs,ProtocolConstants.cs}`, `Sensors/PawnIoProbe.cs`, `ServiceHost.cs`, `Pipe/ClientSession.cs` (Hello), `Pipe/PipeListener.cs` o le opzioni che passano lo stato a `ClientSession`; create `Sensors/DriveKey.cs`, `Sensors/PawnIoStatus.cs`
- Modify: `protocol/fixtures/{hello,subscribe,schema}.msgpack`, `protocol/fixtures/README.md`, `app/src-tauri/nsis/oma.nsh` (marcatore nel ramo 3010, rimozione alla disinstallazione), `app/src/test/nsis-template.test.ts`
- Test: `crates/oma-ipc/tests/`, `service/OpenMonitorAdvanced.Service.Tests/Protocol/CodecTests.cs`, `…Tests/Sensors/PawnIoStatusTests.cs`, `…Tests/Sensors/DriveKeyTests.cs`, `…Tests/Pipe/PipeListenerTests.cs`

**Interfaces:**
- Produces (Rust, `oma_ipc`):
  ```rust
  pub const PROTOCOL_VERSION: u32 = 2;
  pub struct Hello { pub protocol_version: u32, pub service_version: String, pub pawn_io: String }   // "ok"|"missing"|"unavailable"|"unknown"|"rebootPending"
  pub struct Subscribe { pub interval_ms: u32, pub disabled_modules: Vec<String>, pub smart_disabled_drives: Vec<String> }
  pub struct WireServiceState { pub active_modules: Vec<String>, pub smart_disabled_drives: Vec<String>,
      pub reconfiguration: String, pub smart_blocked_by: Vec<String> }     // "applied"|"pending"|"failed"
  pub struct WireSchema { pub devices: Vec<WireDevice>, pub sensors: Vec<WireSensor>, pub service: WireServiceState }
  pub fn drive_key(model: &str, serial: &str) -> Option<String>;   // None if either is empty after trim
  pub const MODULES: [&str; 6] = ["cpu", "motherboard", "memory", "storage", "controller", "psu"];
  pub const MAX_DRIVE_KEYS: usize = 64;
  ```
- Produces (.NET): `ProtocolConstants.Version = 2`; `SubscribeMessage(uint IntervalMs, IReadOnlyList<string> DisabledModules, IReadOnlyList<string> SmartDisabledDrives)`; `HelloMessage(…, string PawnIo)`; `SchemaMessage(…, ServiceStateBlock Service)`; `static string? DriveKey.Compute(string? model, string? serial)`; `enum PawnIoStatus { Ok, Missing, Unavailable, Unknown, RebootPending }` con `static PawnIoStatus PawnIoClassifier.Classify(KeyState key, int? openError, long? markerUtc, long bootUtc)`, dove `openError` è `null` se il device si è aperto e il resto sono FILETIME UTC.
- **Regole:**
  - chiavi MessagePack in snake_case come oggi (`disabled_modules`, `smart_disabled_drives`, `pawn_io`, `service`, `active_modules`, `reconfiguration`, `smart_blocked_by`), sempre presenti, mai `skip_serializing_if`;
  - il servizio rifiuta con `bad_request` un `Subscribe` con un modulo fuori da `MODULES`, più di 64 chiavi, o una chiave che non è esadecimale minuscolo di 64 caratteri;
  - `drive_key` e `DriveKey.Compute` coincidono sul vettore condiviso `protocol/fixtures/drive_key.json`, che contiene coppie `{model, serial, key}`, compresi spazi iniziali e finali, caratteri non ASCII e un caso con il seriale vuoto → nessuna chiave;
  - `Classify` segue la tabella di F3.3 con la decisione P11; lo stato si calcola una volta per processo in `ServiceHost` e si condivide con il hub (`_pawnIoAvailable = status == Ok`) e con `ClientSession` per `Hello`;
  - in questo task il servizio compila il blocco `service` con lo stato attuale (tutti i moduli, `reconfiguration: "applied"`, liste vuote); i Task 11–12 lo rendono reale;
  - NSIS: nel ramo 3010 del setup di PawnIO scrive `PawnIoRebootRequestedUtc` (FILETIME UTC come stringa decimale, `GetSystemTimeAsFileTime`); il disinstallatore, fuori dalla modalità aggiornamento, lo rimuove.

- [ ] **Step 1: test che falliscono:**
  - Rust: `fixtures_round_trip_byte_for_byte` aggiornato ai tre messaggi; `drive_key_matches_the_shared_vector`; `subscribe_v2_keeps_every_key`;
  - .NET: `CodecTests` byte per byte sulle stesse fixture; `UnknownModuleIsABadRequest`; `TooManyDriveKeysIsABadRequest`; `MalformedDriveKeyIsABadRequest`; `DriveKeyUsesTheTrimmedDescriptorModelAndSerial`; `ADiskWithoutDescriptorSerialHasNoDriveKey`; `PawnIoClassifyTable` (una riga di test per ogni riga di F3.3, più `ok` senza chiave); `MarkerThatIsNotANumberIsIgnored`; `HelloCarriesThePawnIoStatus`;
  - `nsis-template.test.ts`: `3010 writes the PawnIO reboot marker` e `uninstall removes the marker outside update mode`.
- [ ] **Step 2:** `cargo test -p oma-ipc` e `dotnet test service/OpenMonitorAdvanced.slnx` → FAIL.
- [ ] **Step 3: implementa;** rigenera le fixture con `$env:OMA_WRITE_FIXTURES = '1'; cargo test -p oma-ipc -- --test-threads=1`, poi rimuovi la variabile e rilancia senza. Aggiorna `protocol/fixtures/README.md`. Adegua i punti Rust che costruiscono `Subscribe` e `Hello` (`svc/link.rs`, test del link, `fake_server.rs`) con liste vuote e `pawn_io` ignorato fino al Task 13.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && dotnet test service/OpenMonitorAdvanced.slnx && pwsh scripts/check-trim-warnings.ps1 && cd app && pnpm test` → verdi.
- [ ] **Step 5: commit** `feat(ipc): protocol v2 with source requests, drive keys and PawnIO status`.

---

### Task 11: servizio — sostituzione atomica di `Subscribe` e aggregazione delle richieste

**Files:**
- Modify: `service/OpenMonitorAdvanced.Service/Sensors/ISensorFeed.cs`, `Sensors/SensorHub.cs`, `Pipe/ClientSession.cs`; create `Sensors/FeedRequest.cs` (`ServiceModules`, `FeedRequest`, `EffectiveConfig`)
- Test: `…Tests/Sensors/EffectiveConfigTests.cs`, `…Tests/Sensors/SensorHubTests.cs`, `…Tests/Pipe/PipeListenerTests.cs`, `…Tests/Pipe/PipeTestSupport.cs` (`FakeFeed.Update`)

**Interfaces:**
- Consumes: `SubscribeMessage` v2 (Task 10).
- Produces (F2.3):
  ```csharp
  [Flags] public enum ServiceModules { None = 0, Cpu = 1, Motherboard = 2, Memory = 4, Storage = 8, Controller = 16, Psu = 32, All = 63 }
  public sealed record FeedRequest(uint IntervalMs, ServiceModules Disabled, IReadOnlySet<string> SmartDisabledDrives);
  public sealed record EffectiveConfig(ServiceModules Enabled, IReadOnlySet<string> SmartDisabledDrives)
  { public static EffectiveConfig? Compute(IReadOnlyCollection<FeedRequest> requests); }
  public interface ISensorFeed { IFeedSubscription Subscribe(FeedRequest request, Action<FeedUpdate> onUpdate); }
  public interface IFeedSubscription : IDisposable { void Update(FeedRequest request); }
  ```
- **Regole:**
  - `Compute`: un modulo è acceso se almeno una richiesta lo tiene acceso; lo SMART di un disco è acceso se almeno una richiesta con lo storage acceso lo tiene acceso; senza richieste → `null`;
  - nel hub `Subscribe`, `Update` e `Unsubscribe` ricalcolano, nella stessa sezione critica di `_subLock`, l'intervallo minimo e la configurazione; se cambia, pubblicano `_desired` (riferimento immutabile con versione crescente e istante della richiesta) e svegliano i worker; con `null` `_desired` non cambia;
  - `ClientSession`: il primo `Subscribe` chiama `_feed.Subscribe`, i successivi `_subscription.Update`, senza `Dispose` e senza passare da zero sottoscrittori; ogni richiesta accettata forza uno `Schema` al prossimo aggiornamento di quel client;
  - nessun callback della pipe chiama LibreHardwareMonitor;
  - in questo task `_desired` si calcola e si espone nel blocco `service` come `pending` quando differisce dall'applicato; l'applicatore arriva nel Task 12 (fino ad allora la richiesta resta `pending`: lo dichiara il commit).

- [ ] **Step 1: test che falliscono:** `AModuleStaysOnIfAnySubscriberWantsIt`, `SmartOfADriveNeedsASubscriberWithStorageOn`, `NoSubscribersKeepsTheLastConfiguration`, `ReplacingARequestIsAtomic`, `PipeCallbacksNeverTouchTheTree`, `ResubscribeDoesNotDropTheStorageCache`, `EveryAcceptedSubscribeIsFollowedByASchema`, `LastSubscriberLeavingKeepsTheEffectiveConfiguration`, `ResubscribeUpdatesTheRequestWithoutUnsubscribing` (estende `ResubscribeChangesTheInterval`).
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx` → FAIL.
- [ ] **Step 3: implementa.**
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx && pwsh scripts/check-trim-warnings.ps1` → verdi.
- [ ] **Step 5: commit** `feat(service): atomic resubscribe and aggregated source requests`.

---

### Task 12: servizio — applicazione dei moduli, storage morbido e SMART per disco

**Files:**
- Modify: `service/OpenMonitorAdvanced.Service/Sensors/{SensorHub.cs,LhmTree.cs,IHardwareTree.cs,DiskPowerProbe.cs,SchemaBuilder.cs}`
- Test: `…Tests/Sensors/SensorHubTests.cs`, `SensorHubFakes.cs` (`FakeTree.OpenedModules`, `SetModules` con contatore e thread chiamante, radici iniziali per tipo; `FakeDisks` con un contatore di interrogazioni per disco), `…Tests/Sensors/LhmTreeTests.cs`

**Interfaces:**
- Consumes: `_desired`, `EffectiveConfig` (Task 11); `DriveKey.Compute` (Task 10).
- Produces:
  ```csharp
  // IHardwareTree
  IReadOnlyList<HardwareNode> Open(ServiceModules enabled);   // Computer created with these non-storage groups only
  void SetModules(ServiceModules enabled);                     // non-storage groups only; reconciles before returning
  // IDiskPowerProbe: exposes the drive keys of the gate blockers for smartBlockedBy
  ```
- **Regole (F2.3, P7, P10):**
  - al primo tick `OpenTree` apre solo i moduli richiesti (lo storage resta fuori, come per D6): un modulo spento nelle impostazioni non viene mai costruito;
  - **sequenza del sampler**, all'inizio di `RunDue` e sul thread `oma-sampler`:
    1. se la versione di `_desired` è già applicata, non fa nulla;
    2. applica subito il filtro dello schema: le radici dei moduli spenti e i dischi con la `driveKey` disattivata escono dal piano, e il primo snapshot del tick esce con lo schema nuovo, nella stessa revisione;
    3. per i gruppi non storage, chiede il parcheggio allo storage worker e continua a campionare senza attendere; quando il parcheggio è confermato chiama `SetModules`, poi rilascia il worker;
    4. oltre 15 s (`ReconfigureTimeout`, `internal init` come `WorkerJoinTimeout`) lo stato diventa `failed`, con un solo warning per richiesta, e si riprova a ogni tick senza chiusure forzate;
  - dopo aver tolto la memoria, `GC.Collect(); GC.WaitForPendingFinalizers();` con il driver ancora caricato, prima di un'eventuale riattivazione;
  - **storage worker**, ai confini del giro: parcheggio con `Volatile`/`Interlocked` e l'attesa esistente, senza lock durante l'I/O. Con lo storage spento, cache e dischi risolti vuoti e nessun gate, `Describe`, `IsSpunDown` o `Update`. Con lo storage acceso il giro normale: i dischi con la chiave disattivata si risolvono con `Describe` e poi si saltano;
  - il blocco `service` dello schema riporta moduli attivi, dischi con lo SMART spento, `reconfiguration` e le `driveKey` che tengono chiuso il gate (`smartBlockedBy`); ogni suo cambio incrementa la revisione.

- [ ] **Step 1: test che falliscono:** `ModulesDisabledBeforeTheFirstTickAreNeverOpened`, `DisablingAModuleDropsItsDevicesInTheSnapshotsRevision`, `SettersWaitForTheStorageWorkerToPark`, `ABlockedStorageWorkerLeadsToFailedWithoutApplying`, `DisablingStorageClearsItsCacheAndStopsDiskIo`, `ReEnablingStorageDoesNotReloadTheGroup`, `StorageEnabledForTheFirstTimeLaterStillGoesThroughTheD6Gate`, `DisposeReleasesAParkedStorageWorker`, `ASmartDisabledDiskIsNeitherPowerCheckedNorUpdated`, `ASmartDisabledDiskLeavesTheSchemaAndReturnsWithItsId`, `ASmartDisabledDiskStillHoldsTheD6Gate`, `GateBlockersAreReportedAsDriveKeys`, `OpenCreatesOnlyTheRequestedGroups`, `SetModulesNeverTouchesStorage`.
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx` → FAIL.
- [ ] **Step 3: implementa.**
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx && pwsh scripts/check-trim-warnings.ps1` → verdi.
- [ ] **Step 5: commit** `feat(service): apply module and per-disk SMART requests on the owning threads`.

---

### Task 13: app — richiesta delle fonti al servizio, stato effettivo e PawnIO

**Files:**
- Modify: `crates/oma-ipc/src/status.rs` (`ServiceStatus` con `pawn_io` e `sources`; non più `Copy`), `crates/oma-win/src/svc/{link.rs,feed.rs,provider.rs}`, `crates/oma-win/src/storage.rs` (chiave del descrittore per `DriveEntry`), `app/src-tauri/src/service.rs`, `app/src-tauri/src/main.rs` (ascoltatore di `sources.serviceModules` e `sources.smartDisabledDrives`), `app/src/lib/types.ts`, `app/src/lib/backend/mock.ts`
- Test: moduli di test di `status.rs`, `link.rs`, `provider.rs`, `storage.rs`, `service.rs`; `mock.test.ts`

**Interfaces:**
- Consumes: protocollo v2 e `drive_key` (Task 10); blocco `service` reale (Task 11–12); `SettingsStore`, `Effect::Service` (Task 2).
- Produces:
  ```rust
  pub enum PawnIoStatus { Ok, Missing, Unavailable, Unknown, RebootPending }   // camelCase in JSON
  pub enum Reconfiguration { Applied, Pending, Failed }
  pub struct ServiceSources { pub active_modules: Vec<String>, pub smart_disabled_drives: Vec<String>,   // core device ids
      pub reconfiguration: Reconfiguration, pub smart_blocked_by: Vec<String> }                          // core device ids
  pub struct ServiceStatus { pub state: ServiceState, pub detail: Option<ServiceDetail>,
      pub pawn_io: Option<PawnIoStatus>, pub sources: Option<ServiceSources> }   // None when not connected
  pub struct SourceRequest { pub disabled_modules: Vec<String>, pub smart_disabled_drives: Vec<String> } // core ids
  pub enum LinkCommand { SetAntiCheat(bool), Start, SetInterval(u32), SetSources(SourceRequest) }
  pub fn drive_keys_for(request: &[String], drives: &[DriveEntry]) -> Vec<String>;   // pure: core id → driveKey, unknown ids dropped
  ```
- **Regole:**
  - il link traduce a ogni `Subscribe` gli id core in `driveKey` con la tabella dei dischi corrente e invia `SetSources` al servizio (nuovo `Subscribe`) solo se è collegato; alla connessione usa l'ultima richiesta;
  - `SvcProvider` scarta localmente i device dei moduli esclusi dall'utente e i dischi con lo SMART spento, anche quando un altro client li tiene accesi. Corrispondenza tra moduli e kind: `cpu` → `Cpu`, `motherboard` → `Motherboard`, `memory` → `Memory`, `storage` → `Storage`, `controller` → `FanController`, `psu` → `Psu`; verificare i kind reali in `SchemaBuilder.cs`;
  - `applyStatus.service`: `Pending` dall'invio finché il blocco `service` non riflette la richiesta con `reconfiguration: applied`; `Failed { reason: "reconfigurationFailed" }` con `failed`; `Idle` senza servizio. Con un altro client che tiene acceso un modulo spento da noi resta `Applied` (la richiesta locale è rispettata dal filtro locale) e `sources.active_modules` lo mostra acceso: la UI (Task 16) spiega la differenza;
  - `smart_blocked_by` si traduce da `driveKey` in id core per la UI (una chiave senza corrispondenza si mostra come "disco sconosciuto");
  - `pawn_io` arriva da `Hello` e resta visibile finché il servizio è collegato; `oma:service` parte anche quando cambiano `pawn_io` o `sources`.

- [ ] **Step 1: test che falliscono:** `drive_keys_for_translates_and_drops_unknown`, `set_sources_resubscribes_with_drive_keys`, `last_request_is_sent_on_connect`, `excluded_module_devices_are_filtered_locally`, `smart_disabled_drive_is_filtered_locally`, `apply_status_goes_pending_then_applied`, `failed_reconfiguration_is_reported`, `pawn_io_status_reaches_the_service_status`, `service_status_serializes_with_pawn_io_and_sources` (JSON camelCase con `pawnIo`, `sources.activeModules`…); UI: tipi e backend finto con `pawnIo` e `sources` (`?pawnio=rebootPending` nel backend finto).
- [ ] **Step 2:** `cargo test --workspace && cd app && pnpm test` → FAIL.
- [ ] **Step 3: implementa.**
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo test -p oma-win -- --include-ignored svc && cd app && pnpm test && pnpm check` → verdi.
- [ ] **Step 5: commit** `feat(svc): send source requests to the service and expose its effective state`.

---

### Task 14: UI — store delle impostazioni, migrazione, vista iniziale e navigazione

**Files:**
- Create: `app/src/lib/settings.svelte.ts`, `app/src/lib/settings.test.ts`
- Modify: `app/src/lib/types.ts`, `app/src/lib/backend/backend.ts`, `tauri.ts`, `mock.ts` (e i loro test), `app/src/lib/advanced/persist.ts` (+ test), `app/src/lib/view.ts`, `app/src/main.ts`, `app/src/App.svelte`, `app/src/lib/i18n/index.svelte.ts`
- Test: `settings.test.ts`, `persist.test.ts`, `mock.test.ts`, `tauri.test.ts`, `AdvancedView.test.ts`

**Interfaces:**
- Consumes: comandi `get_settings`, `update_settings`, `import_webview_state`, `take_pending_view`, `refresh_autostart`; eventi `oma:settings`, `oma:navigate` (Task 2, 3, 7, 8).
- Produces:
  ```ts
  // types.ts: mirrors of the Rust types
  export type Persistence = { kind: 'ok' } | { kind: 'pending' } | { kind: 'recovered'; path: string }
    | { kind: 'readOnly'; reason: string } | { kind: 'error'; reason: string };
  export type EffectStatus = { kind: 'idle' | 'pending' | 'applied' } | { kind: 'failed'; reason: string };
  export interface SettingsState { settings: Settings; revision: number; persistedRevision: number; seq: number;
    persistence: Persistence; applyStatus: { service: EffectStatus; autostart: EffectStatus; vendorLibraries: EffectStatus } }
  export interface Settings { /* spec §2.1, camelCase; advanced.section?, advanced.window?, view.last? optional */ }
  export interface PatchError { field: string; key: string }
  export type SettingsPatch = DeepPartial<Omit<Settings, 'version' | 'migrations' | 'rules' | 'log'>>;
  // backend.ts additions
  getSettings(): Promise<SettingsState>;
  updateSettings(patch: SettingsPatch): Promise<SettingsState>;          // rejects with PatchError
  onSettings(cb: (state: SettingsState) => void): Promise<Unsubscribe>;
  importWebviewState(legacy: LegacyWebviewState): Promise<SettingsState>;
  takePendingView(): Promise<'simple' | 'advanced' | null>;
  onNavigate(cb: (view: 'simple' | 'advanced') => void): Promise<Unsubscribe>;
  refreshAutostart(): Promise<AutostartStatus>;
  // settings.svelte.ts
  export class SettingsStore {
    state: SettingsState | null;                 // $state
    errors: Record<string, string>;              // field → i18n key of the last failed patch
    async connect(backend: Backend): Promise<Unsubscribe>;   // subscribes first, then reads
    async update(patch: SettingsPatch): Promise<boolean>;    // false + errors[field] on PatchError
  }
  export const settings: SettingsStore;          // app-wide instance
  export async function migrateLegacyState(backend: Backend, store: SettingsStore, storage: Storage): Promise<void>;
  export function initialView(state: SettingsState, pending: 'simple' | 'advanced' | null): 'simple' | 'advanced';
  // view.ts
  export type View = 'simple' | 'advanced' | 'settings';
  ```
- **Regole:**
  - `connect`: sottoscrive `onSettings` **prima** di `getSettings`; accetta uno stato solo se `seq` è maggiore di quello corrente (P5);
  - lingua: `language === 'system'` → `detectLocale(navigator.languages)`, altrimenti quella scelta; `i18n.locale` si aggiorna a ogni stato;
  - `migrateLegacyState`: se `migrations.webviewV1` è falso raccoglie `oma.advanced.section`, `oma.advanced.window`, le chiavi `oma.advanced.series.*` e `oma.view`, chiama `importWebviewState` e, solo se riesce, cancella quelle chiavi; se `webviewV1` è già vero cancella le chiavi rimaste; un errore lascia tutto com'è (nuovo tentativo al prossimo avvio);
  - `persist.ts`: `loadSection`, `loadWindow`, `loadSeries` leggono dallo store (con i default di oggi: `DEFAULT_WINDOW` 300); `saveSection`, `saveWindow`, `saveSeries` mandano patch (`{advanced:{section}}`, `{advanced:{window}}`, `{advanced:{series:{[id]: ids}}}`); nessun accesso a `localStorage` fuori da `migrateLegacyState`;
  - `initialView`: vista pendente della tray > `defaultView` `simple`/`advanced` > `view.last` > `simple`; ogni cambio di vista Semplificata/Avanzata manda `{view:{last}}`; `'settings'` non si salva;
  - "Dati non aggiornati" usa `settings.general.intervalMs` invece di `session.intervalMs`, quando disponibile;
  - backend finto: impostazioni in memoria con revisioni e `seq`; il parametro di query `?settings=recovered|readOnly|error` simula gli stati di persistenza; `updateSettings` applica le stesse regole di errore dei casi principali (campo sconosciuto, intervallo fuori passo).

- [ ] **Step 1: test che falliscono** (Vitest):
  - `stale settings event is ignored` (stato con `seq` 5 poi uno con `seq` 4 → resta il 5);
  - `subscribes before reading` (ordine delle chiamate sul backend finto);
  - `failed patch exposes the field error`;
  - `system language follows the browser` e `explicit language wins`;
  - `legacy keys are imported then removed`, `legacy keys stay when the import fails`, `already migrated only removes leftovers`;
  - `persist reads and writes through settings` (nessuna chiamata a `localStorage.setItem`);
  - `initial view: pending beats default beats last`;
  - `mock backend rejects an off-step interval`.
- [ ] **Step 2:** `cd app && pnpm test` → FAIL.
- [ ] **Step 3: implementa.**
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build` → verdi.
- [ ] **Step 5: commit** `feat(ui): settings store, one-time migration and view restore`.

---

### Task 15: UI — unità di misura e FPS dei grafici

**Files:**
- Create: `app/src/lib/units.svelte.ts`, `app/src/lib/units.test.ts`
- Modify: `app/src/lib/format.ts` (+ `format.test.ts`), `app/src/components/advanced/DevicePage.svelte`, `HistoryChart.svelte` (+ test), `app/src/lib/advanced/chartData.ts` (+ test), `app/src/components/common/Sparkline.svelte` (+ test), `app/src/components/simple/SimpleView.svelte`, `app/src/lib/chartFrameClock.ts`
- Test: i file di test indicati

**Interfaces:**
- Consumes: `settings` (Task 14).
- Produces:
  ```ts
  // units.svelte.ts
  export const display: { temperature: 'c' | 'f'; throughput: 'bits' | 'bytes'; chartFps: 60 | 30 | 15 };  // $state, fed by settings
  export function toDisplayTemperature(celsius: number, unit: 'c' | 'f'): number;
  export function temperatureSymbol(unit: 'c' | 'f'): '°C' | '°F';
  // format.ts
  export function formatTemperature(celsius: number | null, locale: string, unit?: 'c' | 'f'): string;  // default display.temperature
  ```
- **Regole:**
  - °F = °C × 9/5 + 32, con le stesse cifre decimali di oggi; ogni temperatura mostrata (tile, KPI, tabelle, min/max/media, asse e legenda del grafico) passa da qui;
  - rete (P4): vista Semplificata e pagine Rete usano `display.throughput`; i dischi restano `bytes`;
  - `HistoryChart`: le serie in `celsius` si convertono in °F per disegno, asse e legenda quando serve; sulle pagine Rete in `bits` le serie `bytes_per_second` si moltiplicano per 8 e l'asse dice bit/s (chiude il follow-up "grafico di rete in byte/s"); un cambio di unità ridisegna il grafico senza ricaricare lo storico;
  - FPS: `HistoryChart` e `Sparkline` passano `display.chartFps` a `subscribeChartFrame` e si risottoscrivono quando cambia.

- [ ] **Step 1: test che falliscono:**
  - `fahrenheit conversion` (0 → 32, 100 → 212, −40 → −40; `formatTemperature(null, …)` → "—");
  - `network pages follow the throughput setting and disks stay in bytes`;
  - `history chart converts temperature series and axis to °F`;
  - `network chart axis is in bit/s when bits are chosen`;
  - `chart and sparkline resubscribe when fps changes` (con `subscribeChartFrame` finto: 60 → 30 dà una disiscrizione e una nuova iscrizione a 30).
- [ ] **Step 2:** `cd app && pnpm test` → FAIL.
- [ ] **Step 3: implementa.**
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build` → verdi.
- [ ] **Step 5: commit** `feat(ui): temperature and throughput units and chart frame rate`.

---

### Task 16: UI — vista Impostazioni (Generale, Fonti dati, Informazioni)

**Skill:** `frontend-design:frontend-design` per l'aspetto, dentro la palette Synthwave e i token di `theme.css`.

**Files:**
- Create: `app/src/components/settings/SettingsView.svelte`, `GeneralSection.svelte`, `SourcesSection.svelte`, `AboutSection.svelte`, `PersistenceNotice.svelte`, `controls/Toggle.svelte`, `controls/Segmented.svelte`, `controls/SelectField.svelte`, `SettingsView.test.ts`, `SourcesSection.test.ts`; `app/src/components/ServiceExplainer.svelte` (estratto da `TopBar.svelte`)
- Create: `crates/oma-win/src/shell_open.rs` (`Win32_UI_Shell` in `crates/oma-win/Cargo.toml`); comandi `get_app_info` e `open_known_path` in `app/src-tauri/src/commands.rs`
- Modify: `app/src/components/TopBar.svelte` (ingranaggio, uso di `ServiceExplainer`), `app/src/App.svelte` (vista `settings`, Esc), `app/src/lib/i18n/en.json`, `it.json`, `app/src/lib/backend/*`, `app/src-tauri/tauri.conf.json` (risorsa `THIRD_PARTY_NOTICES.md`), `build.rs`, `capabilities/default.json`
- Test: i file di test indicati, `TopBar.test.ts`, test Rust di `open_known_path`

**Interfaces:**
- Consumes: `settings`, `display` (Task 14–15); `ServiceStatus` con `pawnIo` e lo stato dei moduli (Task 10–13); `refreshAutostart` (Task 8).
- Produces:
  ```rust
  #[serde(rename_all = "camelCase")] pub struct AppInfo { pub version: String, pub service_version: Option<String>,
      pub protocol_version: u32, pub settings_path: Option<String>, pub logs_path: Option<String> }
  #[tauri::command] pub fn get_app_info(...) -> AppInfo;
  #[serde(rename_all = "camelCase")] pub enum KnownPath { SettingsFolder, LogsFolder, ThirdPartyNotices }
  #[tauri::command] pub fn open_known_path(target: KnownPath) -> Result<(), String>;   // only these three, never an arbitrary path
  // oma_win::shell_open
  pub fn open(path: &Path) -> std::io::Result<()>;                                      // ShellExecuteW "open"
  ```
- **Regole (spec §2.7):**
  - `SettingsView`: navigazione a sinistra con Generale, Fonti dati, Informazioni (Regole e Log CSV non compaiono nella M5a); Esc e "Indietro" tornano alla vista precedente; ogni controllo manda subito la sua patch; l'errore di un campo compare sotto il campo con il testo della chiave i18n;
  - `PersistenceNotice`: `recovered` ("impostazioni ripristinate; file originale conservato in {path}"), `readOnly` ("impostazioni di una versione più recente: le modifiche non verranno salvate"), `error` ("impossibile salvare le impostazioni: {reason}; nuovo tentativo in corso"); niente con `ok` e `pending`;
  - **Generale:** lingua (Sistema, English, Italiano), temperatura (°C/°F), traffico di rete (bit/s, byte/s), intervallo (0,5–5 s a passi di 0,5), FPS dei grafici (60/30/15) con accanto a 60 la nota "aumenta leggermente l'uso della CPU", vista predefinita (Semplificata, Avanzata, Ultima usata), chiudi nella tray, avvio con Windows (con lo stato effettivo di `refreshAutostart` letto all'apertura della sezione: "disattivato da Windows" o "stato gestito da Windows" con il link alle impostazioni di avvio), sensore dell'icona della tray (Automatico + sensori di temperatura e carico dello schema, per dispositivo);
  - **Fonti dati:** interruttori NVML, NVAPI, ADL, IGCL con la nota "se la libreria è già caricata, la disattivazione completa avviene al prossimo avvio"; `ServiceExplainer` (stesso testo e stessa azione del badge); interruttore anti-cheat; stato di PawnIO (`ok` non mostra nulla di speciale; `missing` invita a reinstallare con "Sensori avanzati"; `unavailable` spiega che il driver è installato ma non caricato; `unknown` lo dice senza azioni; `rebootPending` invita a riavviare, non a spegnere); moduli del servizio (sei interruttori) e, sotto "Dischi", un interruttore SMART per ogni disco del core con modello e seriale nel descrittore (gli altri disattivati con la spiegazione). Senza servizio i controlli sono disattivati, con la spiegazione. Un modulo spento da te ma attivo sul servizio (`sources.activeModules`) mostra "tenuto acceso da un altro utente; qui è nascosto". Sotto gli interruttori SMART c'è sempre il limite di P7: "Disattivare lo SMART di un disco non evita la prima identificazione dei dischi: finché un disco rotazionale non conferma di essere attivo, lo SMART resta spento per tutti i dischi". Con `smartBlockedBy` non vuoto: "Lo SMART è spento per tutti i dischi perché {disco} non conferma lo stato di alimentazione". `applyStatus.service` `pending` mostra "in applicazione…", `failed` mostra l'errore;
  - **Informazioni:** versione dell'app e del servizio, versione del protocollo, licenza GPL-3.0-or-later, "Licenze di terze parti" (`open_known_path(ThirdPartyNotices)`), cartelle delle impostazioni e dei log con "Apri cartella";
  - accessibilità: ogni controllo ha un'etichetta, la navigazione si usa da tastiera, il focus torna all'ingranaggio uscendo.

- [ ] **Step 1: test che falliscono** (Vitest, con il backend finto):
  - `gear opens settings and Escape returns to the previous view`;
  - `each control sends its patch` (lingua, unità, intervallo 2,5 s → `intervalMs: 2500`, FPS, vista predefinita, chiudi nella tray, avvio con Windows, sensore dell'icona `null` per Automatico);
  - `fps 60 shows the CPU hint`;
  - `field error is shown next to the field`;
  - `persistence notice for recovered, readOnly and error`;
  - `sources: vendor toggles patch vendorLibraries`, `anti-cheat toggle calls setAntiCheat`, `service controls are disabled without the service`;
  - `about shows versions and opens known paths`;
  - Rust: `open_known_path_rejects_nothing_but_the_three_targets` (la deserializzazione di un valore diverso fallisce).
- [ ] **Step 2:** `cd app && pnpm test` → FAIL.
- [ ] **Step 3: implementa;** registra `get_app_info` e `open_known_path` nei tre punti; aggiungi `THIRD_PARTY_NOTICES.md` a `bundle.resources`.
- [ ] **Step 4:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cd app && pnpm test && pnpm check && pnpm build` → verdi.
- [ ] **Step 5: controllo visivo dell'utente:** `cd app && pnpm dev` nel browser (backend finto); l'utente guarda le tre sezioni in italiano e in inglese, a 900 px senza scorrimento orizzontale, e le tre note di persistenza (`?settings=recovered|readOnly|error`). Nessun clic sintetico.
- [ ] **Step 6: commit** `feat(ui): settings view with general, data sources and about`.

---

### Task 17: verifica dal vivo, budget e documentazione (con l'utente)

**Files:**
- Modify: `docs/perf-budget.md` (righe M5a), `docs/follow-ups.md` (voci chiuse e nuove, "last update: M5a"), `README.md`, `README.it.md` (impostazioni, avvio con Windows, chiusura nella tray), `CLAUDE.md` (stato dei piani M5)

- [ ] **Step 1: verifiche automatiche complete:**
  ```bash
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  cargo test -p oma-win -- --include-ignored
  cd app && pnpm test && pnpm check && pnpm build
  dotnet test service/OpenMonitorAdvanced.slnx
  pwsh scripts/check-trim-warnings.ps1
  ```
  Expected: tutto verde.
- [ ] **Step 2: installer e servizio aggiornati:** `pwsh scripts/build-installer-payload.ps1`, poi `cd app && pnpm tauri build --bundles nsis`; l'utente installa (UAC) e conferma che il servizio parla il protocollo 2 (niente badge "incompatibile").
- [ ] **Step 3: controlli dal vivo chiesti all'utente**, uno per volta, osservando senza input sintetico:
  - primo avvio dopo l'aggiornamento: anti-cheat e vista Avanzata ritrovati (migrazioni), `settings.json` creato, `service.json` sparito;
  - icona dinamica e tooltip nella tray; "Vista Semplificata"/"Vista Avanzata" dalla tray aprono la vista giusta;
  - lingua cambiata dalle Impostazioni: UI e menu della tray cambiano subito;
  - intervallo a 2 s: grafici continui, servizio ancora collegato;
  - chiudi nella tray spento: chiudere la finestra chiude l'app;
  - avvio con Windows acceso: dopo un logout/login l'app parte nella tray; spento da Gestione attività: le Impostazioni lo dicono;
  - ogni modulo del servizio spento e riacceso da Fonti dati: nel log del servizio nessuna eccezione né riavvio, la memoria riattivata torna in pochi secondi; se possibile con due client (due sessioni utente), il modulo resta acceso finché uno lo vuole;
  - HDD SATA in standby (piano energetico "spegni il disco dopo" 1–2 minuti) con lo SMART disattivato per quel disco dopo la discovery: il disco scende in standby e ci resta (nessuna riga "is active" per quel disco nel log);
  - una chiavetta USB collegata: si cerca nel log del servizio `keeps storage disabled` per capire se una seek penalty ignota blocca il gate D6 (F1.3);
  - stato di PawnIO in Fonti dati (`ok` su questo PC); gli scenari "driver fermo", "disinstallato" e "3010" restano per la VM e si aggiungono ai controlli dovuti.
- [ ] **Step 4: budget:** `pwsh scripts/measure-footprint.ps1 -Service` con la tray (icona dinamica attiva) e con la finestra sulle Impostazioni e sulla vista Avanzata; righe in `docs/perf-budget.md`. Se un limite non è rispettato, lo si segnala all'utente invece di allargarlo.
- [ ] **Step 5: documentazione e memoria:** `docs/follow-ups.md` chiude "log guard", "single-instance arguments", "devCsp" se toccato, "network chart in byte/s", "20 Hz link wake", "Incompatible reconnects", "PawnIO status" e "localStorage advanced state". Riscrive la riga del disco USB (la soluzione è il ripiego SAT di F1.4, da pianificare come spike, non un interruttore) e aggiunge: la reidentificazione con risveglio dei dischi non identificati a ogni `DBT_DEVNODES_CHANGED` del thread di hot-plug di DiskInfoToolkit (F1.1); la chiavetta USB con seek penalty ignota se il controllo dal vivo la conferma; gli scenari PawnIO in VM; il possibile ritardo SMBus dei finalizzatori `~SPDAccessor` alla riattivazione rapida della memoria (F2.3). Aggiornare la memoria dei follow-up M5.
- [ ] **Step 6: commit** `docs: record the M5a checks, budget and follow-ups`.
