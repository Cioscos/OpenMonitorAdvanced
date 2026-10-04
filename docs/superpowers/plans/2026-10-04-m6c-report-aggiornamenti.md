# M6c — Report, aggiornamenti e licenze: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** aggiungere a OpenMonitor Advanced il controllo opzionale degli aggiornamenti, l'export anonimo del report sensori e il file completo delle licenze di terze parti, poi preparare la release 0.4.0 (M6b + M6c).

**Architecture:** la logica pura (risposta di GitHub, versioni, pianificazione, report e anonimato) vive in `oma-core` e si testa senza rete né hardware. La richiesta HTTP passa per WinHTTP in `oma-win`, l'unico punto con codice Windows nuovo. La shell Tauri (`oma-app`) tiene lo stato, il thread del controllo automatico, i toast e i comandi. L'interfaccia aggiunge tutto in Impostazioni › Informazioni. Le licenze si generano con uno script PowerShell (`cargo-about` per Rust, un manifest prodotto dal build Vite per il JS, `project.assets.json` per NuGet) e si controllano in CI.

**Tech Stack:** Rust 1.90 (crate `windows` 0.62, serde_json), Tauri 2.11, Svelte 5 + TypeScript 6 + Vitest, PowerShell 7 + Pester 5.7.1, `cargo-about`, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-04-m6c-report-aggiornamenti-design.md` (commit `6d95a05`); per il resto la spec principale `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`.

## Global Constraints

- Branch `feat/m6c-report-aggiornamenti`; merge in `main` in locale; push e tag solo su richiesta dell'utente.
- TDD: prima il test che fallisce, poi il codice (`superpowers:test-driven-development`).
- Codice, commenti e commit in inglese (conventional commits); prosa dei documenti in italiano con gli accenti corretti; fine riga LF ovunque.
- Ogni commit termina con `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.
- Ogni blocco `unsafe` ha il suo `// SAFETY:`; ogni struct FFI nuova ha un assert di dimensione a compile time.
- Nessuna nuova dipendenza Rust di runtime oltre alle feature del crate `windows`; nessuna dipendenza npm nuova.
- La webview non fa richieste di rete: la CSP e `connect-src ipc: http://ipc.localhost` restano invariate.
- Il servizio `oma-service` non tocca la rete e non cambia in questa milestone.
- URL interrogato: `https://api.github.com/repos/Cioscos/OpenMonitorAdvanced/releases/latest`; prefisso ammesso per la pagina: `https://github.com/Cioscos/OpenMonitorAdvanced/releases/`.
- Intestazioni: `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`, `User-Agent: OpenMonitorAdvanced/<versione> (+https://github.com/Cioscos/OpenMonitorAdvanced)`.
- Tempi: scadenza complessiva della richiesta 10 s; corpo massimo 256 KB (262 144 byte); primo controllo automatico 60 s dopo l'avvio; poi 24 h dall'ultimo successo; 6 h dopo un errore; il thread si risveglia almeno ogni 1 h.
- `updates.checkAutomatically` default `false`; `SETTINGS_VERSION` resta `1`.
- Versioni `vX.Y.Z` con tre componenti canoniche senza zeri iniziali, ciascuna tra 0 e 65535.
- Chiavi i18n identiche in `app/src/lib/i18n/en.json` e `it.json`.
- Comandi Tauri nuovi: in `app/src-tauri/build.rs`, nel `generate_handler!` di `main.rs` e in `capabilities/default.json` (aggiornando la `description`).
- Mai clic sintetici o UI Automation sul desktop; mai installer eseguiti su questo PC; mai test Pester `Integration` su questo PC; mai ricerche a tutto il disco.
- Orientamento nel codice: `graphify query "<domanda>"`, `graphify explain "<simbolo>"`, `graphify path "<A>" "<B>"` prima di grep; dopo modifiche al codice `PYTHONHASHSEED=0 graphify update .`.

## Review Focus

1. **Casella spenta mentre una richiesta automatica è in corso:** l'utente si aspetta nessun toast. Test `auto_result_after_disable_shows_no_toast` nel Task 4.
2. **`update-state.json` non scrivibile** (cartella in sola lettura, disco pieno): l'utente si aspetta al massimo un toast per versione nella sessione, non uno ogni ora. Test `unwritable_state_still_notifies_once_per_session` nel Task 4.
3. **Controllo fallito dopo uno riuscito:** l'utente si aspetta di vedere l'errore senza perdere la versione disponibile già nota (il segno resta). Test `failed_check_keeps_known_latest` nel Task 1 e `error_status_keeps_available_badge` nel Task 6.
4. **Report esportato mentre lo schema cambia revisione** (snapshot o statistiche di un'altra revisione): l'utente si aspetta valori `null`, mai valori attribuiti al sensore sbagliato. Test `mismatched_revision_gives_null_values` nel Task 8.
5. **Id o alias originali dentro altri campi** (argomento dell'etichetta, nome del dispositivo, stato dei dischi): l'utente si aspetta che non compaiano da nessuna parte nel file. Test `no_original_identifier_anywhere` nel Task 8, che scandisce il JSON serializzato intero.

---

## Mappa dei file

| File | Responsabilità |
|---|---|
| `crates/oma-core/src/updates.rs` (nuovo) | versioni, parsing della risposta, validazione URL, stato persistito, pianificazione, decisione del toast |
| `crates/oma-core/src/report.rs` (nuovo) | costruzione del report e anonimato |
| `crates/oma-core/src/settings/{mod,decode,patch}.rs` | sezione `updates` |
| `crates/oma-core/src/engine.rs` | ultimo snapshot con qualità |
| `crates/oma-win/src/http.rs` (nuovo) | GET HTTPS con WinHTTP |
| `crates/oma-win/src/os_version.rs` (nuovo) | versione di Windows con `RtlGetVersion` |
| `crates/oma-win/src/network.rs` | proprietà `adapterType` |
| `app/src-tauri/src/updates.rs` (nuovo) | stato su file, thread, comandi, toast degli aggiornamenti |
| `app/src-tauri/src/report.rs` (nuovo) | raccolta dell'input, dialogo, scrittura, comandi del report |
| `app/src-tauri/src/{notifier,window,commands,main}.rs`, `build.rs`, `capabilities/default.json`, `tauri.conf.json` | toast `About`, navigazione verso una sezione, `KnownPath::ThirdPartyLicenses`, registrazione |
| `app/src/lib/updates.svelte.ts` (nuovo) | stato degli aggiornamenti lato UI |
| `app/src/components/settings/AboutSection.svelte`, `SettingsView.svelte`, `App.svelte`, `lib/view.ts`, `lib/types.ts`, `lib/backend/{backend,tauri,mock}.ts`, `test/fake-backend.ts`, `lib/advanced/pages.ts`, `lib/i18n/{en,it}.json` | interfaccia |
| `about.toml`, `about.hbs`, `scripts/generate-licenses.ps1`, `scripts/lib/OmaLicenses.psm1`, `scripts/licenses/*.txt`, `scripts/tests/Licenses.Tests.ps1`, `app/vite.config.ts`, `THIRD_PARTY_LICENSES.txt`, `.github/workflows/ci.yml` | licenze |

---

### Task 1: logica pura degli aggiornamenti (`oma-core`)

**Files:**
- Create: `crates/oma-core/src/updates.rs`, `crates/oma-core/testdata/github/latest-release.json`
- Modify: `crates/oma-core/src/lib.rs` (`pub mod updates;`)

**Interfaces:**
- Produces:
  - `pub struct Version { pub major: u16, pub minor: u16, pub patch: u16 }` (`Copy`, `Ord`, `Display` come `X.Y.Z`); `Version::parse(text: &str) -> Option<Version>` (senza `v`); `Version::parse_tag(tag: &str) -> Option<Version>` (con `v` obbligatoria).
  - `pub struct Release { pub version: Version, pub url: String }`.
  - `pub enum CheckError { Offline, Timeout, Tls, Http(u16), Invalid }`, con `fn category(&self) -> &'static str` che dà `"offline" | "timeout" | "tls" | "http" | "invalid"`.
  - `pub const LATEST_RELEASE_URL: &str`, `pub const RELEASE_PAGE_PREFIX: &str`, `pub fn user_agent(version: &str) -> String`, `pub const REQUEST_HEADERS: [(&str, &str); 2]` (Accept e X-GitHub-Api-Version).
  - `pub fn parse_latest(status: u16, body: &[u8]) -> Result<Release, CheckError>`.
  - `#[derive(Serialize, Deserialize, Default)] #[serde(rename_all = "camelCase")] pub struct UpdateState { pub last_attempt_ms: Option<u64>, pub last_success_ms: Option<u64>, pub latest: Option<StoredRelease>, pub notified_version: Option<String> }` con `StoredRelease { version: String, url: String }`.
  - `UpdateState::record_success(&mut self, now_ms: u64, release: &Release)`, `UpdateState::record_failure(&mut self, now_ms: u64)`, `UpdateState::available(&self, current: Version) -> Option<Release>` (la `latest` salvata, rivalidata, se più nuova di `current`).
  - `pub fn next_check_ms(now_ms: u64, started_ms: u64, state: &UpdateState, auto: bool) -> Option<u64>`.
  - `pub fn should_notify(current: Version, state: &UpdateState, release: &Release) -> bool`.
  - Costanti: `FIRST_CHECK_DELAY_MS = 60_000`, `SUCCESS_INTERVAL_MS = 86_400_000`, `RETRY_INTERVAL_MS = 21_600_000`, `MAX_BODY_BYTES = 262_144`.

- [ ] **Step 1: scrivere i test che falliscono** (modulo `tests` in `updates.rs`):
  - `version_parses_canonical_only`: `parse("0.4.0")` = 0.4.0; `parse_tag("v1.2.3")` = 1.2.3; `None` per `"1.2"`, `"1.2.3.4"`, `"01.2.3"`, `"1.2.65536"`, `"v1.2.3"` passato a `parse`, `"1.2.3"` passato a `parse_tag`, `"v1.2.3-rc1"`, `""`.
  - `version_orders_numerically`: `0.10.0 > 0.9.9`, `1.0.0 > 0.99.99`.
  - `parses_real_release_response`: `parse_latest(200, include_bytes!("../testdata/github/latest-release.json"))` = versione 0.3.0 e URL `https://github.com/Cioscos/OpenMonitorAdvanced/releases/tag/v0.3.0`. La fixture ha la forma della risposta REST di GitHub (con `assets`, `body`, `author` e altri campi da ignorare).
  - `rejects_draft_prerelease_and_bad_fields`: `draft: true` → `Invalid`; `prerelease: true` → `Invalid`; `tag_name: "release-3"` → `Invalid`; `html_url` di un altro repository, `http://` o con prefisso simile (`…/OpenMonitorAdvanced-evil/releases/…`) → `Invalid`; campi mancanti o JSON non valido → `Invalid`; status 403 → `Http(403)`; corpo vuoto con 200 → `Invalid`.
  - `user_agent_names_version_and_repo`: `user_agent("0.4.0")` = `"OpenMonitorAdvanced/0.4.0 (+https://github.com/Cioscos/OpenMonitorAdvanced)"`.
  - `next_check_schedule` (tabella, `started = 1_000_000`):
    - `auto = false` → `None` per qualunque stato;
    - stato vuoto → `started + 60_000`;
    - ultimo successo a `t` (> started) → `t + 86_400_000`;
    - ultimo successo a `started - 10 h` → `started + 14 h`;
    - successo lontano nel passato (oltre 24 h) → `started + 60_000` (mai prima di 60 s dall'avvio);
    - ultimo tentativo fallito a `t` dopo l'ultimo successo → `t + 21_600_000`, ma non prima di `started + 60_000`;
    - `last_success_ms` o `last_attempt_ms` più grandi di `now_ms` (orologio indietro) → `started + 60_000`.
  - `notify_once_per_version`: con `current` 0.4.0 e release 0.5.0, `should_notify` è vero con `notified_version: None`, falso con `Some("0.5.0")`, vero con `Some("0.4.9")`; falso con release 0.4.0 o 0.3.0.
  - `failed_check_keeps_known_latest` (Review Focus 3): dopo `record_success(t, 0.5.0)` e `record_failure(t + 1)`, `available(0.4.0)` resta 0.5.0, `last_success_ms` resta `t`, `last_attempt_ms` è `t + 1`.
  - `available_drops_stale_or_invalid_latest`: `latest` con versione uguale o più vecchia di `current`, oppure con URL fuori prefisso (file modificato a mano) → `None`.
  - `state_round_trips_and_tolerates_garbage`: serializzazione e deserializzazione camelCase; per un JSON con campi sconosciuti la deserializzazione funziona (niente `deny_unknown_fields`).

- [ ] **Step 2: verificare che falliscano**

  Comando: `cargo test -p oma-core updates`. Esito atteso: errori di compilazione (modulo assente).

- [ ] **Step 3: implementare `updates.rs`** con serde_json (già dipendenza). `record_success` imposta `last_attempt_ms` e `last_success_ms` a `now_ms` e salva `latest`. `record_failure` tocca solo `last_attempt_ms`. In `next_check_ms`, un tentativo è fallito quando `last_attempt_ms > last_success_ms` (o il successo manca).

- [ ] **Step 4: verificare che passino**

  Comandi: `cargo test -p oma-core updates`, `cargo clippy -p oma-core --all-targets -- -D warnings`. Esito atteso: PASS, nessun avviso.

- [ ] **Step 5: commit** con il messaggio `feat(core): add the update check logic`.

---

### Task 2: impostazione `updates.checkAutomatically`

**Files:**
- Modify: `crates/oma-core/src/settings/mod.rs` (struct `Updates { check_automatically: bool }`, campo `updates` in `Settings`, `encode`, `everything_changed`), `crates/oma-core/src/settings/decode.rs`, `crates/oma-core/src/settings/patch.rs` (voce `("updates", Node::Object(&[("checkAutomatically", leaf())]))` in `SCHEMA`), `app/src/lib/types.ts` (`Settings.updates: { checkAutomatically: boolean }`), `app/src/lib/backend/mock.ts` e `app/src/test/settings.ts` (default), eventuali fixture di impostazioni nei test UI.

**Interfaces:**
- Produces: `Settings.updates.check_automatically: bool`, JSON `{"updates": {"checkAutomatically": false}}`; lato TS `settings.updates.checkAutomatically`.

- [ ] **Step 1: test che falliscono**
  - In `mod.rs`, il test del JSON di default include `"updates": {"checkAutomatically": false}`.
  - `everything_changed` usa `true`.
  - `updates_default_off_and_round_trip`: un file senza `updates` decodifica a `false` senza diagnostica; `{"updates":{"checkAutomatically":true}}` diventa `true`; `"yes"` dà `false` con `WrongType` sul percorso `updates.checkAutomatically`.
  - In `patch.rs`: una patch `{"updates":{"checkAutomatically":true}}` viene applicata; `{"updates":{"nope":1}}` e `{"updates":null}` danno gli stessi errori delle altre sezioni.
- [ ] **Step 2:** `cargo test -p oma-core settings` deve fallire.
- [ ] **Step 3:** implementare seguendo il modello di `tray.closeToTray` (decode con `reader.section`/`reader.boolean`, encode, schema della patch).
- [ ] **Step 4:** passano `cargo test -p oma-core settings` e `cd app && pnpm test && pnpm check`, aggiornando i default TS e le fixture che confrontano l'oggetto completo.
- [ ] **Step 5: commit** con il messaggio `feat(settings): add updates.checkAutomatically`.

---

### Task 3: GET HTTPS con WinHTTP (`oma-win`)

**Files:**
- Create: `crates/oma-win/src/http.rs`
- Modify: `crates/oma-win/src/lib.rs` (`pub mod http;`), `crates/oma-win/Cargo.toml` (feature `Win32_Networking_WinHttp`, in ordine alfabetico)

**Interfaces:**
- Consumes: `oma_core::updates::{CheckError, MAX_BODY_BYTES}`.
- Produces: `pub struct HttpResponse { pub status: u16, pub body: Vec<u8> }`, `pub fn get(url: &str, user_agent: &str, headers: &[(&str, &str)], deadline: Duration, max_body: usize) -> Result<HttpResponse, CheckError>`, `pub(crate) fn classify(code: u32) -> CheckError` (pura).

- [ ] **Step 1: test che falliscono**
  - `classify_maps_winhttp_errors`:
    - `ERROR_WINHTTP_NAME_NOT_RESOLVED` (12007) e `ERROR_WINHTTP_CANNOT_CONNECT` (12029) → `Offline`;
    - `ERROR_WINHTTP_TIMEOUT` (12002) → `Timeout`;
    - `ERROR_WINHTTP_SECURE_FAILURE` (12175) e i codici 12037, 12038, 12044, 12045, 12057 → `Tls`;
    - un codice qualsiasi → `Invalid`.
  - `#[ignore = "requires network"] fetches_the_latest_release`: `get(LATEST_RELEASE_URL, &user_agent("0.0.0"), &REQUEST_HEADERS, 10 s, MAX_BODY_BYTES)` dà status 200 e `parse_latest` va a buon fine.
- [ ] **Step 2:** `cargo test -p oma-win http` deve fallire.
- [ ] **Step 3: implementare `get`.**
  - Handle in un tipo RAII che chiama `WinHttpCloseHandle`.
  - `WinHttpOpen` con `WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`.
  - `WinHttpSetOption`: `WINHTTP_OPTION_SECURE_PROTOCOLS` = TLS 1.2 | TLS 1.3; `WINHTTP_OPTION_DISABLE_FEATURE` con `WINHTTP_DISABLE_COOKIES`; `WINHTTP_OPTION_REDIRECT_POLICY` = `WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP`.
  - `WinHttpSetTimeouts` con i quattro timeout ricavati da `deadline`.
  - URL scomposto con `WinHttpCrackUrl`; uno schema diverso da `https` dà `Invalid` senza connettersi.
  - `WinHttpOpenRequest` con `WINHTTP_FLAG_SECURE`.
  - Lettura con `WinHttpQueryDataAvailable`/`WinHttpReadData` fino a `max_body`; oltre il limite, `Invalid`.
  - Prima di ogni chiamata bloccante si controlla la scadenza complessiva.
  - Lo stato HTTP si legge con `WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER`.
  - Gli errori di WinHTTP passano da `classify(GetLastError)`.
- [ ] **Step 4:** passano `cargo test -p oma-win http`, poi `cargo test -p oma-win http -- --ignored` (rete: lo esegue il controller, non è richiesto in CI) e `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 5: commit** con il messaggio `feat(win): add an HTTPS GET over WinHTTP`. Dopo la revisione del task, il controller avvia l'agente `ffi-safety-reviewer` sul diff.

---

### Task 4: servizio degli aggiornamenti nella shell

**Files:**
- Create: `app/src-tauri/src/updates.rs`
- Modify: `app/src-tauri/src/main.rs` (stato gestito, avvio del thread, `generate_handler!`), `app/src-tauri/build.rs`, `app/src-tauri/capabilities/default.json`, `app/src-tauri/src/i18n.rs` (chiavi usate dal Rust), `app/src/lib/i18n/{en,it}.json` (`updates.toast.title` = «OpenMonitor Advanced {version} disponibile» / "OpenMonitor Advanced {version} available", `updates.toast.body` = «Apri Informazioni per scaricarla» / "Open About to download it")

**Interfaces:**
- Consumes: Task 1 (tutto), Task 2 (`settings.updates.check_automatically`), Task 3 (`oma_win::http::get`), `notifier::ToastSink`, `oma_win::fsutil::replace_file`, `commands::shell_open`.
- Produces:
  - `pub trait Fetch: Send + Sync { fn fetch(&self, user_agent: &str) -> Result<(u16, Vec<u8>), CheckError>; }` con l'implementazione reale `WinHttpFetch`.
  - `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct UpdateStatus { state: UpdateStateKind, current: String, latest: Option<LatestVersion>, checked_at_ms: Option<u64>, error: Option<&'static str> }`, con `UpdateStateKind` in camelCase (`idle`, `checking`, `upToDate`, `available`, `error`) e `LatestVersion { version: String }`.
  - `pub struct UpdateService` (`Arc` gestito da Tauri) con `new(fetch, state_path: Option<PathBuf>, current: Version, toasts: Arc<dyn ToastSink>, lang)`, `status() -> UpdateStatus`, `check_now() -> UpdateStatus` (bloccante, una richiesta alla volta), `set_auto(bool)`, `spawn_scheduler(started_ms)`.
  - Comandi Tauri: `check_updates`, `get_update_status`, `open_release_page`. Evento `oma:update-status` (payload `UpdateStatus`). Costante `EVENT_UPDATE_STATUS`.
  - Variabile `OMA_UPDATE_FAKE_CURRENT` letta solo sotto `cfg(debug_assertions)`.

- [ ] **Step 1: test che falliscono** (`Fetch` finto, toaster registratore, cartella temporanea):
  - `state_file_missing_or_garbage_is_empty`: un file assente o con contenuto non JSON dà `UpdateState::default()`.
  - `manual_check_reports_available_without_toast`: con il fetch che restituisce 0.5.0 e la versione corrente 0.4.0, `check_now()` dà `available` e `latest.version == "0.5.0"`; zero toast; il file ha `notifiedVersion: "0.5.0"`.
  - `auto_check_toasts_once`: due controlli automatici della stessa 0.5.0 producono un solo toast, con launch `{"open":"about"}` e titolo tradotto.
  - `auto_result_after_disable_shows_no_toast` (Review Focus 1): il fetch si blocca su una barriera; si chiama `set_auto(false)`, poi si sblocca; nessun toast, ma lo stato mostra `available`.
  - `unwritable_state_still_notifies_once_per_session` (Review Focus 2): con `state_path` dentro una cartella che non esiste, due controlli automatici danno un solo toast; `status()` resta corretto.
  - `error_maps_category`: il fetch restituisce `Err(Timeout)` → stato `error` con `error: "timeout"`, `checked_at_ms` presente, nessun toast; lo stato salvato ha `last_attempt_ms` aggiornato e non `last_success_ms`.
  - `concurrent_manual_checks_share_one_request`: due `check_now()` in parallelo con il fetch bloccato contano una sola chiamata al fetch e restituiscono lo stesso esito.
  - `error_status_keeps_latest` (Review Focus 3): dopo un successo con 0.5.0 e un errore, `status()` ha `state: error` e `latest.version == "0.5.0"`; `status()` riempie sempre `latest` da `UpdateState::available()`.
  - `fake_current_only_in_debug`: con `cfg(debug_assertions)`, `current_version("0.4.0", Some("0.2.0"))` dà 0.2.0; un valore non valido viene ignorato (0.4.0).
- [ ] **Step 2:** `cargo test -p oma-app updates` deve fallire.
- [ ] **Step 3: implementare.**
  - **Stato:** `Mutex<Inner>` con `UpdateState`, l'esito corrente e un flag `in_flight`; una `Condvar` sveglia lo scheduler e chi attende `check_now`.
  - **Scrittura:** su un file temporaneo e poi `replace_file`. Se la scrittura fallisce, si registra un `warn` e si tiene `notified_version` in memoria.
  - **Scheduler:** in un thread chiamato `oma-updates`. Attende `min(next_check_ms - now, 1 h)`, ricalcola tutto al risveglio e salta il controllo con la casella spenta.
  - **Casella:** `set_auto` è collegata al listener delle impostazioni, come per l'autostart e la tray.
  - **Comandi:** `open_release_page` apre con `shell_open(Path::new(url))` solo l'URL di `available()`, altrimenti restituisce un errore. Ogni cambio di stato emette `oma:update-status`.
  - **Cartella dello stato:** la stessa delle impostazioni (`settings_path()` → cartella padre).
- [ ] **Step 4:** passano `cargo test -p oma-app` e `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 5: commit** con il messaggio `feat(app): check GitHub for updates on request or daily`.

---

### Task 5: il toast porta a Informazioni

**Files:**
- Modify: `app/src-tauri/src/notifier.rs` (`LaunchTarget::About`, `launch_for_about()`), `app/src-tauri/src/window.rs` (campo `settings_section: Option<String>` in `NavigationTarget`, serializzato come `settingsSection` e omesso se assente; `NavigationTarget::about(view: ViewKind)`; `pub fn show_about(app)`), `app/src-tauri/src/updates.rs` (usa `launch_for_about`), `app/src/lib/types.ts` (`NavigationTarget.settingsSection?: 'about'`), `app/src/lib/view.ts` (`SettingsTarget.section` accetta `'about'`), `app/src/App.svelte` (`navigate`), `app/src/components/settings/SettingsView.svelte` se il tipo lo richiede.

**Interfaces:**
- Consumes: Task 4 (toast con launch).
- Produces: `launch_for_about() -> String` = `{"open":"about"}`; `launch_target` restituisce `Some(LaunchTarget::About)` per quella stringa; `show_about` naviga con `NavigationTarget { view: <settings.view.last o Simple>, device_id: None, settings_section: Some("about") }`.

- [ ] **Step 1: test che falliscono**
  - Rust: `about_launch_round_trips` (`launch_target(&launch_for_about()) == Some(LaunchTarget::About)`; `{"open":"other"}` dà ancora `None`).
  - Rust: `about_target_serializes_section` (JSON `{"view":"simple","settingsSection":"about"}`, senza `deviceId`).
  - Vitest in `App.test.ts`: `navigate with settingsSection opens About`. Il backend finto emette `oma:navigate` con `{view:'simple', settingsSection:'about'}`; la schermata delle impostazioni appare sulla sezione Informazioni; «Indietro» torna a Semplice.
- [ ] **Step 2:** i test falliscono.
- [ ] **Step 3: implementare.**
  - In `system_toaster`, `LaunchTarget::About` chiama `crate::window::show_about`.
  - In `App.svelte`, `navigate` chiama `showView(target.view)` e poi, con `settingsSection === 'about'`, `openSettings({ section: 'about' })`.
- [ ] **Step 4:** passano `cargo test -p oma-app`, `cd app && pnpm test && pnpm check`.
- [ ] **Step 5: commit** con il messaggio `feat(app): open Settings › About from the update toast`.

---

### Task 6: interfaccia degli aggiornamenti

**Files:**
- Create: `app/src/lib/updates.svelte.ts`, `app/src/components/settings/AboutSection.test.ts` (se non esiste; altrimenti si estende)
- Modify: `app/src/lib/types.ts` (`UpdateStatus`), `app/src/lib/backend/{backend,tauri,mock}.ts`, `app/src/test/fake-backend.ts`, `app/src/components/settings/AboutSection.svelte`, `app/src/components/settings/SettingsView.svelte` (segno sulla voce `about`), `app/src/lib/i18n/{en,it}.json`

**Interfaces:**
- Consumes: Task 4 (comandi ed evento), Task 2 (impostazione).
- Produces:
  - `Backend.checkUpdates(): Promise<UpdateStatus>`, `getUpdateStatus(): Promise<UpdateStatus>`, `openReleasePage(): Promise<void>`, `onUpdateStatus(cb): Promise<Unsubscribe>`.
  - Store `updates` con `state: UpdateStatus | null`, `connect(backend)` e `check()`.
- Chiavi i18n (it / en):

| Chiave | Italiano | Inglese |
|---|---|---|
| `settings.about.updates` | «Aggiornamenti» | "Updates" |
| `settings.about.checkNow` | «Controlla ora» | "Check now" |
| `settings.about.checking` | «Controllo in corso…» | "Checking…" |
| `settings.about.upToDate` | «Hai l'ultima versione ({version})» | "You have the latest version ({version})" |
| `settings.about.available` | «Disponibile la versione {version}» | "Version {version} is available" |
| `settings.about.releasePage` | «Pagina della release» | "Release page" |
| `settings.about.lastChecked` | «Ultimo controllo: {time}» | "Last checked: {time}" |
| `settings.about.checkAutomatically` | «Controlla automaticamente (una volta al giorno)» | "Check automatically (once a day)" |
| `settings.about.updatesNote` | «Il controllo contatta GitHub (api.github.com) e invia solo l'indirizzo IP e la versione dell'app. Non installa nulla.» | "The check contacts GitHub (api.github.com) and sends only your IP address and the app version. It installs nothing." |
| `settings.about.updateBadge` | «Aggiornamento disponibile» | "Update available" |
| `settings.about.error.offline` | «Nessuna connessione» | "No connection" |
| `settings.about.error.timeout` | «Tempo scaduto» | "Timed out" |
| `settings.about.error.tls` | «Connessione sicura non riuscita» | "Secure connection failed" |
| `settings.about.error.http` | «GitHub ha risposto con un errore» | "GitHub answered with an error" |
| `settings.about.error.invalid` | «Risposta non valida» | "Invalid response" |

- [ ] **Step 1: test Vitest che falliscono** (`AboutSection.test.ts`, `SettingsView.test.ts`):
  - `shows up to date after Check now`: il clic mostra «Hai l'ultima versione (0.4.0)» e l'ora dell'ultimo controllo.
  - `disables Check now while checking`: con lo stato `checking` il pulsante è disattivato.
  - `available shows version and release page button`: il clic sul pulsante chiama `openReleasePage`.
  - `each error category has its text`: le cinque categorie.
  - `checkbox patches updates.checkAutomatically and shows the note`: l'aggiornamento parte con `{updates:{checkAutomatically:true}}`; la casella è disattivata con impostazioni in sola lettura.
  - `badge on About only when available`: il pallino con testo accessibile «Aggiornamento disponibile» compare solo con lo stato `available`.
  - `error_status_keeps_available_badge` (Review Focus 3): con lo stato `error` e `latest` più nuovo, il segno resta e il testo mostra l'errore.
  - `mock backend implements update commands`: in `pnpm dev`, il backend finto risponde a `getUpdateStatus` con `idle`.
- [ ] **Step 2:** `cd app && pnpm test` fallisce.
- [ ] **Step 3: implementare.**
  - Il segno compare quando `state.latest` non è `null`: il backend riempie `latest` solo con una versione più nuova di quella installata, anche quando lo stato è `error` (Task 4, `error_status_keeps_latest`).
  - Stile come le altre righe di `AboutSection` (`dl`/`row`).
  - L'ora si formatta con la locale corrente.
- [ ] **Step 4:** passano `cd app && pnpm test && pnpm check && pnpm build`.
- [ ] **Step 5: commit** con il messaggio `feat(ui): add the update check to Settings › About`.

---

### Task 7: proprietà `adapterType` delle schede di rete

**Files:**
- Modify: `crates/oma-win/src/network.rs`, `app/src/lib/advanced/pages.ts` (valore tradotto, ordine), `app/src/lib/i18n/{en,it}.json` (`property.adapterType` = «Tipo di adattatore» / "Adapter type"; `property.adapterType.ethernet` = «Ethernet»; `property.adapterType.wifi` = «Wi-Fi»), test relativi.

**Interfaces:**
- Produces: `Device.properties["adapterType"]` ∈ {`"ethernet"`, `"wifi"`} per ogni dispositivo di rete; `pub(crate) fn adapter_type(if_type: u32) -> Option<&'static str>`.

- [ ] **Step 1: test che falliscono**
  - Rust `adapter_type_from_if_type`: `IF_TYPE_ETHERNET_CSMACD` → `"ethernet"`, `IF_TYPE_IEEE80211` → `"wifi"`, altro → `None`.
  - Rust: il test di discovery esistente con righe finte verifica la proprietà nei dispositivi.
  - Vitest `pages.test.ts`: `adapterType is shown translated`. `{adapterType:'wifi'}` dà la riga «Tipo di adattatore» con valore «Wi-Fi».
- [ ] **Step 2:** i test falliscono.
- [ ] **Step 3: implementare.** Il valore si inserisce in `properties` del `Device`; in `propertyRows` si traduce con `property.adapterType.<valore>` se la chiave esiste.
- [ ] **Step 4:** passano `cargo test -p oma-win network`, `cd app && pnpm test`.
- [ ] **Step 5: commit** con il messaggio `feat(network): expose the adapter type as a device property`.

---

### Task 8: costruzione del report e anonimato (`oma-core`)

**Files:**
- Create: `crates/oma-core/src/report.rs`
- Modify: `crates/oma-core/src/lib.rs`, `crates/oma-core/src/engine.rs` (ultimo snapshot)

**Interfaces:**
- Consumes: `Schema`, `Snapshot`, `Quality`, `SensorStats`, `Sources` (`oma_core::settings`).
- Produces:
  - `Engine::latest(&self) -> Option<(&Snapshot, &[Quality])>`: snapshot e qualità dell'ultimo `tick` (salvati in `tick` prima di restituire `TickOutput`).
  - `pub struct ReportInput<'a>` con i campi seguenti:
    - `generated_at_ms: u64`;
    - versioni: `app_version: &'a str`, `service_version: Option<&'a str>`, `protocol_version: u32`, `os_version: Option<&'a str>`;
    - servizio: `service_state: &'a str` (la serializzazione camelCase di `ServiceState`), `anti_cheat: bool`;
    - modalità sicura: `safe_mode: bool`, `safe_mode_reason: Option<&'a str>`;
    - fonti e dischi: `sources: &'a Sources`, `disk_states: &'a [(String, &'a str)]`;
    - dati: `schema: &'a Schema`, `snapshot: Option<(&'a Snapshot, &'a [Quality])>`, `stats: &'a [Option<SensorStats>]`, `stats_revision: u64`.
  - `pub fn build_report(input: &ReportInput) -> serde_json::Value`.
  - `pub const REPORT_FORMAT: u32 = 1`.
  - `pub const REPORT_PROPERTIES: &[&str]`: `pciAddress`, `integrated`, `pcieMaxGen`, `pcieMaxWidth`, `powerLimitMinW`, `powerLimitMaxW`, `powerLimitDefaultW`, `tempSlowdownC`, `tempShutdownC`, `tempMaxC`, `tempWarningC`, `tempCriticalC`, `tjMaxC`, `availableSpareThresholdPct`, `adapterType`.

- [ ] **Step 1: test che falliscono**, con uno schema di prova (CPU, GPU, due dischi `storage/device-<hash>`, due schede `network/{GUID}` con alias «VPN ufficio» e «Casa di Mario», `adapterType` `ethernet`/`wifi`, una proprietà `serialNumber` inventata):
  - `report_has_format_versions_and_state`: `format == 1`; `generatedAt` in ISO 8601 UTC (`1970-01-01T00:00:00Z` per 0 ms; un caso noto come `1_759_536_000_000` → `2025-10-04T00:00:00Z`); `app`, `os`, `state.service`, `state.safeMode`.
  - `report_sources_exclude_drive_lists`: `state.sources` ha `vendorLibraries`, `antiCheat` e `serviceModules`, e non `smartDisabledDrives`/`smartEnabledDrives`.
  - `sensors_carry_value_quality_and_stats`: per ogni sensore `value`, `quality` (`fresh`/`held`/`suspended`) e `stats {min,avg,max,count}`. `NaN` e infinito danno `null`. Senza snapshot, `value: null` e `quality: "fresh"`.
  - `mismatched_revision_gives_null_values` (Review Focus 4): uno snapshot di revisione diversa dallo schema dà `value: null` per tutti; `stats_revision` diverso dà `stats: null` per tutti.
  - `ids_are_replaced_consistently`:
    - i dispositivi diventano `storage/disk-1`, `storage/disk-2`, `network/adapter-1`, `network/adapter-2`, nell'ordine dello schema;
    - gli id dei sensori diventano `storage/disk-1/temperature/drive` e così via;
    - `deviceId` dei sensori e `state.disks[].deviceId` coincidono con i nuovi id;
    - gli altri id (`cpu/…`, `gpu/…`, `lhm/…`) restano invariati.
  - `network_names_become_type_and_index`: i nomi diventano «Ethernet 1» e «Wi-Fi 1»; una scheda senza `adapterType` diventa «Adapter N».
  - `properties_are_whitelisted`: `serialNumber` assente; `pciAddress` e `adapterType` presenti.
  - `no_original_identifier_anywhere` (Review Focus 5): nel JSON serializzato intero non compaiono gli hash, i GUID (in qualunque maiuscolo/minuscolo) né gli alias delle fixture. Il caso include un `Label.arg` uguale all'alias e uno stato disco con l'id originale.
  - In `engine.rs`: `latest_returns_last_tick_snapshot`, prima `None` e dopo un tick la stessa `revision` e `seq` di `TickOutput`.
- [ ] **Step 2:** `cargo test -p oma-core report engine` fallisce.
- [ ] **Step 3: implementare.**
  - `generatedAt`: se in `oma_core::csv` esiste già un helper di data UTC si riusa; altrimenti si usa l'algoritmo civil-from-days.
  - La sostituzione degli id avviene una volta, con una mappa originale → nuovo costruita sui dispositivi dello schema.
  - Una stringa di sensore il cui prefisso è un id mappato si riscrive.
  - Un `Label.arg` che coincide con un nome di rete originale diventa il nuovo nome.
- [ ] **Step 4:** passano `cargo test -p oma-core` e `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] **Step 5: commit** con il messaggio `feat(core): build the anonymous sensor report`.

---

### Task 9: export del report nella shell e nell'interfaccia

**Files:**
- Create: `app/src-tauri/src/report.rs`, `crates/oma-win/src/os_version.rs`
- Modify: `crates/oma-win/src/lib.rs`, `crates/oma-win/Cargo.toml` (feature per `RtlGetVersion`: `Wdk_System_SystemServices`, da verificare nel crate `windows` 0.62), `app/src-tauri/src/main.rs`, `build.rs`, `capabilities/default.json`, `app/src/lib/backend/{backend,tauri,mock}.ts`, `app/src/test/fake-backend.ts`, `app/src/lib/types.ts`, `AboutSection.svelte` e il suo test, `app/src/lib/i18n/{en,it}.json`

**Interfaces:**
- Consumes: Task 8 (`build_report`, `Engine::latest`), `AppState` (engine, `disk_states`), `ServiceShell::status()`, `StartupState::current()`, `SettingsStore` (fonti), `oma_win::known_folder::documents_dir`, `oma_win::local_time`, `tauri-plugin-dialog`, `oma_win::fsutil::replace_file`.
- Produces:
  - `oma_win::os_version::os_version() -> Option<String>` (`"10.0.26300"`), con la funzione pura `format_version(major, minor, build) -> String`.
  - Comandi `export_sensor_report() -> Result<Option<ExportedReport>, String>` (`ExportedReport { file_name: String }`) e `reveal_sensor_report() -> Result<(), String>`.
  - `pub(crate) fn report_file_name(local: LocalTime) -> String`, che dà `oma-report-AAAAMMGG-HHMMSS.json`.
  - TS: `Backend.exportSensorReport(): Promise<{ fileName: string } | null>`, `revealSensorReport(): Promise<void>`.
- Chiavi i18n:

| Chiave | Italiano | Inglese |
|---|---|---|
| `settings.about.report` | «Report sensori» | "Sensor report" |
| `settings.about.exportReport` | «Esporta report sensori» | "Export sensor report" |
| `settings.about.reportNote` | «Un file JSON anonimo con dispositivi, sensori, fonti e valori, da allegare alle segnalazioni.» | "An anonymous JSON file with devices, sensors, sources and values, to attach to bug reports." |
| `settings.about.reportSaved` | «Report salvato: {file}» | "Report saved: {file}" |
| `settings.about.openFolder` | (esistente) | (esistente) |

- [ ] **Step 1: test che falliscono**
  - Rust `report_file_name_uses_local_time` (2026-10-04 09:05:07 → `oma-report-20261004-090507.json`).
  - Rust `format_version_joins_parts`.
  - Rust `gather_input_without_snapshot_still_builds`: con un `Engine` senza tick il report si costruisce, con `value: null`.
  - Vitest `export shows saved message and Open folder`: il clic su «Apri cartella» chiama `revealSensorReport`.
  - Vitest `cancelled export shows nothing`.
  - Vitest `failed export shows the error text`.
- [ ] **Step 2:** i test falliscono.
- [ ] **Step 3: implementare.**
  - L'input si raccoglie copiando i dati necessari sotto il lock dell'engine, una volta (schema, ultimo snapshot e qualità, statistiche di tutti i sensori con la loro revisione), poi il lock si rilascia **prima** del dialogo.
  - Il dialogo è `app.dialog().file().set_file_name(..).set_directory(documents).add_filter("JSON", &["json"]).blocking_save_file()`.
  - La scrittura avviene su un file temporaneo nella stessa cartella, poi `replace_file`.
  - La cartella dell'ultimo report si tiene in uno stato gestito `ReportState`; `reveal_sensor_report` apre quella cartella con `shell_open`, altrimenti restituisce un errore.
  - JSON con `serde_json::to_vec_pretty`.
- [ ] **Step 4:** passano `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cd app && pnpm test && pnpm check && pnpm build`.
- [ ] **Step 5: commit** con il messaggio `feat(app): export the sensor report from Settings › About`. Dopo la revisione del task, `ffi-safety-reviewer` su `os_version.rs`.

---

### Task 10: generatore delle licenze e controllo in CI

**Files:**
- Create: `about.toml`, `about.hbs`, `scripts/generate-licenses.ps1`, `scripts/lib/OmaLicenses.psm1`, `scripts/licenses/` (testi standard: `MIT.txt`, `Apache-2.0.txt`, `MPL-2.0.txt` e quelli richiesti dal censimento NuGet), `scripts/tests/Licenses.Tests.ps1`, `THIRD_PARTY_LICENSES.txt`
- Modify: `app/vite.config.ts` (plugin locale che, con `OMA_LICENSE_MANIFEST=<file>`, scrive in `generateBundle` l'elenco ordinato dei pacchetti di `node_modules` finiti nel bundle), `.github/workflows/ci.yml` (job `installer`: passo «Check third-party licences» dopo «Build installer payload», con `cargo install cargo-about --locked --version <versione fissata>`), `THIRD_PARTY_NOTICES.md` (paragrafo iniziale che rimanda a `THIRD_PARTY_LICENSES.txt`)

**Interfaces:**
- Produces:
  - `pwsh scripts/generate-licenses.ps1 [-Check]`: scrive `THIRD_PARTY_LICENSES.txt`; con `-Check` lo genera in una cartella temporanea e fallisce, elencando le differenze, se diverso.
  - Funzioni del modulo (testate con Pester): `Get-OmaNuGetRuntimePackages -AssetsJson <path> -Target <string>`, `Merge-OmaLicenseSections -Sections <object[]>`, `ConvertTo-OmaLicenseText -Entries <object[]>`.
- Decisioni:
  - **Rust:** `cargo about generate --workspace --target x86_64-pc-windows-msvc about.hbs` con `ignore-dev-dependencies = true` e `ignore-build-dependencies = true` in `about.toml`. La versione di `cargo-about` è l'ultima stabile al momento dell'esecuzione (`cargo search cargo-about`), fissata nello script e nella CI. Le licenze in `accepted` sono solo quelle trovate nel censimento, ciascuna con un commento che la motiva.
  - **JS:** `OMA_LICENSE_MANIFEST=<tmp> pnpm build` in `app/`, poi per ogni pacchetto del manifest `node_modules/<nome>/package.json` (versione, licenza) e il suo file `LICENSE*`. Svelte entra così, anche se è una devDependency, perché il suo runtime è nel bundle.
  - **NuGet:** `dotnet restore service/OpenMonitorAdvanced.Service -r win-x64`, poi `obj/project.assets.json`, target `net10.0-windows…/win-x64`. Si includono i pacchetti con almeno un asset `runtime`, `native` o `runtimeTargets` diverso da `_._`. Licenza dal file del pacchetto in `~/.nuget/packages/<id>/<versione>/`, altrimenti dall'espressione del `.nuspec` con il testo da `scripts/licenses/`. Si aggiunge il runtime .NET 10 (MIT).
  - **Formato:** una sezione per ecosistema; voci ordinate per nome e versione; testi standard una sola volta in fondo, con i rimandi; UTF-8 senza BOM, LF; nessuna data né percorso locale, così il file è deterministico.

- [ ] **Step 1: test Pester che falliscono** (`Licenses.Tests.ps1`, con fixture minime in `scripts/tests/fixtures/licenses/`):
  - `NuGet runtime packages exclude analyzers and placeholders`: un `project.assets.json` di prova con un pacchetto `runtime`, uno con solo `_._` e un analyzer; solo il primo è nel risultato.
  - `Merge sorts entries and deduplicates standard texts`: due pacchetti Apache-2.0 danno un solo testo Apache e due voci che lo richiamano.
  - `Output is deterministic`: due chiamate con input in ordine diverso danno lo stesso testo, con fine riga LF e senza BOM.
- [ ] **Step 2:** `Import-Module Pester -RequiredVersion 5.7.1; Invoke-Pester -Path scripts/tests/Licenses.Tests.ps1 -CI` fallisce.
- [ ] **Step 3: implementare** il modulo, lo script, `about.toml`, `about.hbs` e il plugin Vite. Poi eseguire `pwsh scripts/generate-licenses.ps1`. Se compare una licenza nuova, valutarla: se è permissiva e compatibile con la GPL-3.0-or-later si aggiunge con un commento; altrimenti si ferma il task con `BLOCKED` e la si riporta.
- [ ] **Step 4: verificare**
  - `pwsh scripts/generate-licenses.ps1 -Check` termina con codice 0;
  - la suite Pester completa (`-ExcludeTagFilter Integration`) passa;
  - `cd app && pnpm build` senza la variabile non scrive manifest;
  - `actionlint` in CI (non in locale, se non è installato).
- [ ] **Step 5: commit** con il messaggio `build: generate THIRD_PARTY_LICENSES.txt and check it in CI`.

---

### Task 11: licenze nell'installer e in Informazioni

**Files:**
- Modify: `app/src-tauri/tauri.conf.json` (`"resources"` aggiunge `"../../THIRD_PARTY_LICENSES.txt": "THIRD_PARTY_LICENSES.txt"`), `app/src-tauri/src/commands.rs` (`KnownPath::ThirdPartyLicenses` e il suo percorso in `KnownDirs::target`), `app/src/lib/types.ts` (`KnownPath`), `AboutSection.svelte` e il suo test, `app/src/lib/i18n/{en,it}.json` (`settings.about.thirdParty` resta «Avvisi di terze parti» / "Third-party notices"; nuovo `settings.about.licenseTexts` = «Testi delle licenze» / "Licence texts")

**Interfaces:**
- Consumes: Task 10 (file presente nella radice).
- Produces: `KnownPath::ThirdPartyLicenses` (JSON `"thirdPartyLicenses"`).

- [ ] **Step 1: test che falliscono**
  - Rust: il test esistente dei percorsi di `KnownDirs` copre `ThirdPartyLicenses`, che punta a `<resources>/THIRD_PARTY_LICENSES.txt`.
  - Vitest `licence row has notices and texts buttons`: due pulsanti; il secondo chiama `openKnownPath('thirdPartyLicenses')`.
- [ ] **Step 2:** i test falliscono.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** passano `cargo test -p oma-app`, `cd app && pnpm test && pnpm check`.
- [ ] **Step 5: commit** con il messaggio `feat(app): ship the licence texts and open them from About`.

---

### Task 12: documentazione e privacy

**Files:**
- Modify:
  - `CODE_SIGNING.md` (Privacy, testo del §5.1 della spec);
  - `README.md` e `README.it.md` (aggiornamenti, report, licenze, «Segnalare un problema» / "Reporting a problem");
  - `THIRD_PARTY_NOTICES.md` (termini di `Mono.Posix.NETStandard`);
  - `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md` (§4.6, §7.4, §9, §14);
  - `docs/follow-ups.md` (bozza della richiesta all'autore di PawnIO, bozza delle note di rilascio 0.4.0, voce Privacy chiusa, voce `Mono.Posix` chiusa o aggiornata);
  - `CLAUDE.md` (stato della M6, comando `generate-licenses.ps1`).

- [ ] **Step 1: termini di `Mono.Posix.NETStandard`.** Si leggono i termini dal `.nuspec` del pacchetto in `$USERPROFILE/.nuget/packages/mono.posix.netstandard/1.0.0/` e dal fwlink (WebFetch). Se i termini permettono la ridistribuzione, si riportano nome e licenza in `THIRD_PARTY_NOTICES.md` e la voce dei follow-up si chiude. Se non la permettono, `BLOCKED` con il testo trovato.
- [ ] **Step 2: Privacy in `CODE_SIGNING.md`.**
  - Si sostituisce il paragrafo «The app currently has no update check…» con il testo del §5.1 della spec.
  - La frase iniziale dell'elenco diventa «send no data over the network, except the optional update check below».
- [ ] **Step 3: README.**
  - Sezioni brevi: controllo aggiornamenti (cosa invia, spento di default, come si attiva), report sensori (cosa contiene e cosa no), licenze (i due file).
  - Il paragrafo «Segnalare un problema» chiede il report allegato.
  - Le due lingue restano allineate per sezioni.
- [ ] **Step 4: spec principale.** Le modifiche del §5 della spec M6c; il §14 dice che la M6 chiude con la 0.4.0 e che la 1.0 aspetta firma, conferma di PawnIO e matrice hardware.
- [ ] **Step 5: follow-up.** Le bozze sono testo da pubblicare solo su richiesta.
  - La richiesta all'autore di PawnIO va in inglese: si chiede conferma di poter ridistribuire il setup ufficiale non modificato, citando la verifica della firma e lo SHA-256 fissato.
  - Le note di rilascio 0.4.0 vanno in inglese, divise in novità della M6b e della M6c, con la nota «app and service must be the same version (protocol v3); the installer updates both» e la correzione dei testi italiani dell'installer.
- [ ] **Step 6: verificare** che `pwsh scripts/check-version.ps1` passi e che i link relativi nei README puntino a file esistenti (controllo con `Test-Path` sui percorsi citati).
- [ ] **Step 7: commit** con il messaggio `docs: document the update check, the sensor report and the licences`.

---

### Task 13: verifiche dal vivo con l'utente

Il controller le esegue con l'utente; nessun input sintetico.

**Files:**
- Modify: `docs/superpowers/plans/2026-10-04-m6c-report-aggiornamenti.md` (sezione «Esito dell'esecuzione»), `docs/perf-budget.md`, `docs/follow-ups.md`

- [ ] **Step 1:** `cd app && pnpm tauri dev`. L'utente esegue U1 («Controlla ora» → «Hai l'ultima versione»). Nel log della shell non ci sono richieste prima del clic.
- [ ] **Step 2:** U2. Si riavvia con `OMA_UPDATE_FAKE_CURRENT=0.2.0`; l'utente preme «Controlla ora», vede «Disponibile la versione 0.3.0» e apre la pagina della release.
- [ ] **Step 3:** U3. L'utente attiva la casella e riavvia: dopo circa 60 s compare un toast, e il clic apre Informazioni. Un secondo riavvio non mostra altri toast; il segno resta. Prima del secondo riavvio si annota il contenuto di `update-state.json`.
- [ ] **Step 4:** U4. L'utente disattiva la rete e preme «Controlla ora»: compare «Nessuna connessione» e nessun toast.
- [ ] **Step 5:** U5. L'utente esporta il report con il servizio attivo e poi in modalità anti-cheat. Uno script nello scratchpad confronta il file con lo schema dell'app: tutti i dispositivi sono presenti, e nessun id `storage/device-`/`storage/gpt-`/`storage/mbr-`/`storage/pnp-`, GUID o alias di rete reale compare.
- [ ] **Step 6:** U6. I due pulsanti delle licenze si verificano nell'app installata dall'installer, alla prima occasione su una VM. In `tauri dev` si controlla che il percorso risolto esista nella cartella delle risorse.
- [ ] **Step 7:** U7. `pwsh scripts/measure-footprint.ps1` con la casella automatica attiva, finestra e tray; i valori entro i limiti vanno in `docs/perf-budget.md`.
- [ ] **Step 8:** U8. `pwsh scripts/build-installer-payload.ps1`, poi `cd app && pnpm tauri build --bundles nsis`. Si apre l'archivio con 7-Zip (`7z l`) e si verifica la presenza di `THIRD_PARTY_LICENSES.txt`. L'installer non si esegue.
- [ ] **Step 9:** registrare gli esiti nel piano e nei follow-up; commit con il messaggio `docs: record the M6c live checks`.

---

### Task 14: versione 0.4.0

**Files:**
- Modify: i cinque file di versione e `Cargo.lock` (tramite lo script)

- [ ] **Step 1:** `pwsh scripts/bump-version.ps1 0.4.0`.
- [ ] **Step 2:** `pwsh scripts/check-version.ps1` passa; `cargo test --workspace` e `cd app && pnpm test` passano (alcuni test confrontano la versione).
- [ ] **Step 3:** commit con il messaggio `chore: release 0.4.0`.
- [ ] **Step 4:** il tag `v0.4.0` si crea **dopo il merge in `main`**, su `main`, e si fa push solo su richiesta dell'utente (`docs/release.md`). Il workflow crea la bozza; le novità si incollano dalla bozza del Task 12.

---

## Esito dell'esecuzione

Task 1-12 completati, ciascuno con la revisione dedicata; la revisione finale dell'intero branch ha prodotto correzioni, applicate prima dei controlli dal vivo.

Controlli dal vivo del Task 13 (2026-10-04, con l'utente):

- U1 superato: «Controlla ora» mostra «Hai l'ultima versione (0.3.0)»; `update-state.json` nasce solo dal clic.
- U2 superato: con una versione finta 0.2.0 compaiono il segno e «Disponibile la versione 0.3.0»; «Controlla ora» lo mantiene e la pagina della release si apre.
- U3 superato: il toast arriva dopo circa 60 s e il clic apre Informazioni; al secondo avvio il controllo automatico riparte dopo 60 s, senza nuovi toast e con i segni al loro posto.
- U4 superato: offline, «Controlla ora» mostra «Nessuna connessione» e nessun toast.
- U5 superato: due report (servizio connesso: 10 dispositivi, 171 sensori; anti-cheat: 9 dispositivi, 91 sensori) senza id di archiviazione, GUID, hash lunghi, utente, macchina, seriali o MAC.
- U6 superato in `tauri dev`: entrambi i pulsanti aprono i file delle licenze; il controllo nell'app installata resta da fare su una VM.
- U7 superato: build release di `54da8a7`, nucleo 0,04 % (finestra e tray), app 20,7 MB e 18,0 MB, servizio 0,01 % e circa 63 MB (dettagli in `docs/perf-budget.md`).
- U8 superato: payload e installer NSIS (setup 0.3.0, 12,7 MB) con `THIRD_PARTY_LICENSES.txt` e `THIRD_PARTY_NOTICES.txt` all'interno; l'installer non è stato eseguito.

Durante U1 è emerso un bug preesistente di M6b: `get_disk_states` mancava da `build.rs` e dalla capability ("not allowed"). Corretto in `653b981`, con un test di guardia che confronta `generate_handler!`, `build.rs` e la capability.

Due aggiunte richieste dall'utente durante i controlli: un pallino sull'ingranaggio delle impostazioni quando un aggiornamento è noto, con la stessa regola del segno su Informazioni (`b4fe719`), e il tooltip «Impostazioni (aggiornamento disponibile)» (`54da8a7`).

Resta da fare, annotato in `docs/follow-ups.md`: il ripiego WinHTTP con TLS 1.2 su una VM Windows 10, la prima esecuzione in CI del passo sulle licenze (con la cache di cargo-about) e i pulsanti delle licenze nell'app installata su una VM. Rinviati: due `.sr-only` adiacenti in TopBar e CSS `.dot`/`.sr-only` duplicato tra TopBar e SettingsView. La release 0.4.0 (Task 14) resta in attesa della richiesta dell'utente.
