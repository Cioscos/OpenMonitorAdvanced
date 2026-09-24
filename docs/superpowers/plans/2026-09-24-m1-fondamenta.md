# Milestone 1 — Fondamenta: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Obiettivo:** una prima app desktop Windows funzionante. Legge senza privilegi carico e clock della CPU, RAM, dischi e rete, e li mostra nella vista Semplificata con palette Synthwave, tray minima e storico in memoria.

**Architettura:**
- **Monorepo** con un workspace Cargo di tre crate:
  - `oma-core`: modello dati, `Provider`, engine, storico, sampler; indipendente dalla piattaforma;
  - `oma-win`: provider Windows senza privilegi (PDH, `GlobalMemoryStatusEx`, `GetDiskFreeSpaceExW`, `GetIfTable2`);
  - `oma-app`: shell Tauri 2.
- **Frontend** Svelte 5 + TypeScript in `app/`. Riceve uno `Schema` e poi uno `Snapshot` al secondo tramite eventi Tauri.
- **In tray:** alla chiusura della finestra la WebView viene distrutta, mentre il processo Rust continua a campionare.

**Tech stack:** Rust stable ≥ 1.85 (verificato con 1.90), `windows` 0.62, Tauri 2.11, `tauri-plugin-single-instance` 2.4, Node 22, pnpm 10, Svelte 5.57, Vite 8, Vitest 5, TypeScript 6.0. **TypeScript resta alla 6.0: `svelte-check` 4.7 non supporta la 7.**

**Spec:** `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`. Va letta insieme a questo piano. Le sezioni implementate qui sono:
- §2 (processi), §3 (modello dati);
- §4.1 (scheduler), §4.2 (storico, senza min/max/media);
- §4.5 (tray minima), §5.1 (solo CPU, RAM, dischi, rete);
- §7.1, §7.2, §7.5, §7.6 (senza il tile GPU e senza il banner basato su regole);
- §8 (isolamento dei provider, dati anomali), §11, §12 (in parte), §14 punto 1.

**Fuori da M1, rimandato di proposito:**

| Cosa | Milestone |
|---|---|
| GPU | M2 |
| Vista Avanzata, min/max/media, batteria, SMBIOS, Wi-Fi RSSI, metadati completi RAM/dischi, commit memoria (`GetPerformanceInfo`), IOPS dischi | M3 |
| Fallback temperatura NVMe senza privilegi (punto aperto §13.6) | M3, prova hardware prima dell’integrazione |
| Servizio e IPC | M4 |
| Motore regole, notifiche, tray dinamica, log CSV, `settings.json`, avvio automatico | M5 |
| Installer NSIS con servizio e PawnIO | M4 |
| Report sensori e release 1.0 | M6 |

**Prove già fatte durante la stesura (prototipi usa e getta in scratchpad, non da riusare):**
- **Firme `windows` 0.62:** verificate compilando ed eseguendo il codice PDH, memoria, volumi e `GetIfTable2` su Windows 11 in italiano (Ryzen 7 7800X3D).
- **Discovery PDH:** dopo **un solo** `PdhCollectQueryData`, `PdhGetFormattedCounterArrayW` fallisce con `PDH_INVALID_DATA` (0xC0000BBA). `PdhGetRawCounterArrayW` invece restituisce già i nomi delle istanze, quindi la discovery usa quest'ultimo.
- **Tauri 2.11:**
  - tray, `destroy()` della finestra, `prevent_exit` e ricreazione della finestra funzionano;
  - dopo `destroy()` i processi WebView2 passano da 6 a **0** e il processo resta a circa 5 MB privati.
- **Frontend:**
  - Vitest 5 richiede `defineConfig` da `vitest/config`;
  - jsdom non ha `matchMedia` e serve un polyfill, perché `prefersReducedMotion` di `svelte/motion` la usa;
  - `Tween.set(v, { duration: 0 })` è sincrono.

**Esito della revisione del 24 settembre 2026:** corretti isolamento dello scheduler, backoff, identità dei dischi, fallback CPU, validità PDH, sincronizzazione UI, capability, diagnostica e verifica del budget. I prototipi citati sopra sono evidenza storica della stesura, non una verifica dei blocchi aggiornati: eseguire i test indicati durante l’implementazione.

## Global Constraints

- **Piattaforma v1:** Windows 10/11. Il codice specifico di Windows sta **solo** in `crates/oma-win` (e nei `cfg(windows)` di `oma-app`). `oma-core` e l'interfaccia non dipendono dalla piattaforma.
- **Leggerezza:**
  - nessuna modifica della risoluzione del timer di sistema;
  - nessuna animazione continua;
  - rendering dei numeri con interpolazione ≤ 300 ms;
  - rispetto di `prefers-reduced-motion`.
- **Budget (§1.2):**
  - nucleo a riposo < 1% di CPU;
  - processo in tray < 30 MB;
  - finestra aperta < 200 MB in totale, WebView2 compresa.
  
  Misura: working set privato (colonna "Memoria" di Task Manager).
- **Intervallo di campionamento:** default 1 s, configurabile nel nucleo da 500 a 5000 ms; il controllo nelle impostazioni arriva in M5. **Storico:** 1 ora di campioni (3600 a 1 s).
- **L'interfaccia non gira mai con privilegi elevati.** Nessun contenuto remoto. CSP stretta.
- **Stringhe UI:** tutte in `app/src/lib/i18n/en.json` e `it.json`, con l'inglese come lingua di riserva. Stesse chiavi nelle due lingue.
- **ID dei sensori stabili:** `<device_id>/<kind>/<name>`, per esempio `cpu/0/load/total`. Le **chiavi delle etichette** (`Label.key`) sono il contratto tra Rust e UI: la UI le cerca come `sensor.<key>`.
- **Palette Synthwave (§7.5), valori esatti:**

  | Token | Valore |
  |---|---|
  | `--bg` | `#0f0a1a` |
  | `--surface` | `#181126` |
  | `--surface-2` | `#211733` |
  | `--border` | `#2d2042` |
  | `--text` | `#f5eefe` |
  | `--text-muted` | `#9585b0` |
  | `--accent` | `#ff4fd8` |
  | `--accent-2` | `#4cc9f0` |
  | `--ok` | `#3ee8b5` |
  | `--warn` | `#ffc53d` |
  | `--crit` | `#ff4d4d` |
- **Nomi:**
  - prodotto: "OpenMonitor Advanced";
  - prefisso dei crate: `oma-`;
  - identificatore Tauri: `io.github.openmonitoradvanced`.
- **Licenza:** GPL-3.0 (`GPL-3.0-or-later` nei manifest).
- **Lingua di codice e commenti:** inglese. Tutti i commenti del codice sono in inglese, come gli identificatori.

## Review Focus

Condizioni che la spec implica ma che un test di funzionalità non coprirebbe da solo. Ogni riga ha un test nel task che possiede il codice.

1. **Windows non in inglese (per esempio in italiano).** I nomi dei contatori PDH sono localizzati: vanno aggiunti **solo** con `PdhAddEnglishCounterW`. Test: `pdh_english_paths_resolve` in `crates/oma-win/tests/providers.rs` (Task 6), eseguito sulla macchina di sviluppo in italiano.
2. **Hot-plug (disco USB, cavo di rete, Wi-Fi acceso o spento).** Il provider restituisce `Rediscover`, l'engine pubblica una nuova revisione dello schema e la UI scarta gli snapshot di revisioni diverse e riscarica lo schema. Test:
   - `rediscover_rebuilds_schema_and_keeps_history_by_id` (Task 4);
   - `detects_disk_set_changes` (Task 8);
   - `detects_adapter_set_changes` (Task 9);
   - `rejects_snapshot_of_another_revision` e `connect_refetches_schema_on_revision_mismatch` (Task 11).
3. **Sospensione e ripresa, orologio spostato indietro.** Il sampler non deve recuperare a raffica i tick persi, e lo storico non deve restituire campioni fuori ordine. Test:
   - `next_deadline_skips_missed_ticks_after_sleep` (Task 5);
   - `clock_jumping_backwards_keeps_only_samples_after_the_jump` (Task 3).
4. **Più di 64 processori logici (gruppi di processori).** Le istanze `1,x` vanno mantenute e ordinate per gruppo e numero. Test: `processors_sort_by_group_then_number` (Task 6).
5. **Primo campione e reset dei contatori cumulativi.** Il primo campione di rete non ha un tasso e deve dare `None`, non un picco. Un contatore che torna indietro (reset della scheda) non deve produrre valori negativi o enormi. Test:
   - `first_sample_has_no_rate` e `counter_reset_restarts_from_new_baseline` (Task 2);
   - `sanitize` scarta i valori negativi (Task 2).

---

## Mappa dei file

```
Cargo.toml                                   workspace (members aggiunti man mano)
.gitignore  LICENSE  README.md
.github/workflows/ci.yml                     CI Windows (Task 14)
scripts/measure-footprint.ps1                misura del budget (Task 14)
docs/perf-budget.md                          risultati delle misure (Task 14)

crates/oma-core/
  Cargo.toml
  src/lib.rs                                 elenco dei moduli
  src/model.rs                               Device, Sensor, Label, Schema, Snapshot, enum
  src/provider.rs                            trait Provider, Inventory, ProviderError
  src/sanitize.rs                            filtro dei valori anomali (§8)
  src/rate.rs                                CounterRate per i contatori cumulativi
  src/history.rs                             History (ring buffer), HistoryWindow
  src/worker.rs                              un worker persistente per provider, timeout e backoff
  src/engine.rs                              Engine: discovery, poll, backoff, schema
  src/sampler.rs                             thread di campionamento, next_deadline, unix_ms

crates/oma-win/
  Cargo.toml
  src/lib.rs                                 default_providers()
  src/pdh.rs                                 wrapper sicuro di PDH
  src/cpu.rs                                 CpuProvider
  src/memory.rs                              MemoryProvider
  src/storage_identity.rs                    identità persistenti di dischi e volumi
  src/storage.rs                             StorageProvider
  src/network.rs                             NetworkProvider
  tests/providers.rs                         smoke test su hardware reale

app/                                         frontend (pnpm)
  package.json  pnpm-lock.yaml  tsconfig.json  vite.config.ts  index.html  app-icon.svg
  src/main.ts  src/App.svelte  src/App.test.ts  src/vite-env.d.ts  src/test-setup.ts
  src/styles/theme.css
  src/lib/types.ts  src/lib/view.ts  src/lib/format.ts  src/lib/health.ts
  src/lib/series.ts  src/lib/sparkline.ts  src/lib/select.ts  src/lib/live.svelte.ts
  src/lib/i18n/index.svelte.ts  src/lib/i18n/en.json  src/lib/i18n/it.json
  src/lib/backend/backend.ts  src/lib/backend/tauri.ts  src/lib/backend/mock.ts  src/lib/backend/index.ts
  src/test/fake-backend.ts
  src/components/TopBar.svelte
  src/components/common/AnimatedNumber.svelte  src/components/common/Sparkline.svelte
  src/components/simple/SimpleView.svelte  src/components/simple/HealthBanner.svelte  src/components/simple/Tile.svelte
  src/components/advanced/AdvancedPlaceholder.svelte
  (i test *.test.ts stanno accanto ai moduli che testano)

app/src-tauri/                               crate oma-app
  Cargo.toml  build.rs  tauri.conf.json  capabilities/default.json  icons/
  src/main.rs  src/commands.rs  src/tray.rs  src/window.rs
```

---

**Comandi:** i blocchi `bash` richiedono Git Bash; in PowerShell eseguire le righe separatamente e usare `curl.exe` per scaricare la licenza. Non sostituire indiscriminatamente comandi di cancellazione tra shell. I test hardware sono esclusi dalla CI ordinaria e richiesti prima di chiudere M1.

### Task 1: Monorepo e modello dati di `oma-core`

**File:**
- Crea: `Cargo.toml`, `LICENSE`, `README.md`
- Modifica: `.gitignore`
- Crea: `crates/oma-core/Cargo.toml`, `crates/oma-core/src/lib.rs`, `crates/oma-core/src/model.rs`
- Test: `crates/oma-core/src/model.rs` (modulo `tests`)

**Interfacce:**
- Usa: niente.
- Produce (`oma_core::model`):
  - `enum DeviceKind { Cpu, Gpu, Memory, Storage, Network, Motherboard, Battery, FanController, Psu }`
  - `enum SensorKind { Temperature, Load, Clock, Power, Voltage, Current, Fan, Data, Throughput, Energy, Flag, Percent }` con `fn as_str(self) -> &'static str`
  - `enum Unit { Celsius, Percent, Megahertz, Watt, Volt, Ampere, Rpm, Bytes, BytesPerSecond, BitsPerSecond, Joule, Boolean }`
  - `enum Source { Pdh, Win32, IpHelper, Mock }`
  - `struct Label { key: String, arg: Option<String> }` con `Label::new(&str)` e `Label::with_arg(&str, impl Into<String>)`
  - `struct Device { id: String, kind: DeviceKind, name: String, vendor: Option<String>, properties: BTreeMap<String, String> }`
  - `struct Sensor { id, device_id, kind, unit, label, source, category }` con `Sensor::new(device_id: &str, kind: SensorKind, name: &str, unit: Unit, label: Label, source: Source) -> Sensor`, che produce l'id `"{device_id}/{kind}/{name}"`
  - `struct Schema { revision: u64, devices: Vec<Device>, sensors: Vec<Sensor> }` (con `Default`)
  - `struct Reading { sensor_id: String, value: Option<f64>, timestamp_ms: u64 }`
  - `struct Snapshot { revision: u64, seq: u64, timestamp_ms: u64, values: Vec<Option<f64>> }`
  - Serializzazione JSON: strutture in camelCase, enum in snake_case.

- [ ] **Step 1: Crea il workspace e i file di progetto**

`Cargo.toml` (radice):

```toml
[workspace]
resolver = "2"
members = ["crates/oma-core"]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.85"
license = "GPL-3.0-or-later"

[workspace.dependencies]
oma-core = { path = "crates/oma-core" }
oma-win = { path = "crates/oma-win" }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
tracing = "0.1"
```

`.gitignore` (sostituisce il contenuto attuale, `.superpowers/` resta):

```gitignore
.superpowers/
/target/
app/node_modules/
app/dist/
app/src-tauri/gen/schemas/
```

`crates/oma-core/Cargo.toml`:

```toml
[package]
name = "oma-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
serde.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
serde_json.workspace = true
```

`crates/oma-core/src/lib.rs`:

```rust
//! Platform-independent core of OpenMonitor Advanced: data model, providers,
//! engine, history and sampling loop.

pub mod model;
```

`LICENSE`: scarica il testo ufficiale della GPL-3.0:

```bash
curl -fsSL https://www.gnu.org/licenses/gpl-3.0.txt -o LICENSE
```

`README.md`:

```markdown
# OpenMonitor Advanced

Open-source hardware monitor for Windows with a modern UI: a Simple view that tells you at a glance
whether your PC is fine, and an Advanced view with every sensor.

**Status:** milestone 1 (foundations) — CPU, RAM, disks and network without admin rights.
Design: `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`.

## Build

Prerequisites: Windows 10/11, Rust stable ≥ 1.85 (MSVC), Node 22, pnpm 10, WebView2 runtime (preinstalled on Windows 11).

    cd app
    pnpm install
    pnpm tauri dev        # run with hot reload
    pnpm dev              # UI only, in the browser, with a mock backend

Tests: `cargo test --workspace` (after `pnpm build` in `app/`) and `pnpm test` in `app/`.

## License

GPL-3.0-or-later. See `LICENSE`.
```

- [ ] **Step 2: Scrivi i test del modello (falliscono)**

`crates/oma-core/src/model.rs`, parte di test (la parte di implementazione arriva allo Step 4):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sensor_id_is_device_kind_name() {
        let s = Sensor::new("cpu/0", SensorKind::Load, "total", Unit::Percent, Label::new("cpu.load.total"), Source::Pdh);
        assert_eq!(s.id, "cpu/0/load/total");
        assert_eq!(s.device_id, "cpu/0");
    }

    #[test]
    fn sensor_serializes_in_camel_case_with_snake_case_enums() {
        let s = Sensor::new(
            "network/abc",
            SensorKind::Throughput,
            "down",
            Unit::BytesPerSecond,
            Label::new("network.down"),
            Source::IpHelper,
        );
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            json!({
                "id": "network/abc/throughput/down",
                "deviceId": "network/abc",
                "kind": "throughput",
                "unit": "bytes_per_second",
                "label": { "key": "network.down" },
                "source": "ip_helper",
                "category": "throughput"
            })
        );
    }

    #[test]
    fn label_arg_is_serialized_when_present() {
        let label = Label::with_arg("cpu.load.thread", "3");
        assert_eq!(serde_json::to_value(&label).unwrap(), json!({ "key": "cpu.load.thread", "arg": "3" }));
    }

    #[test]
    fn device_kind_uses_snake_case() {
        let d = Device { id: "x".into(), kind: DeviceKind::FanController, name: "X".into(), vendor: None, properties: Default::default() };
        assert_eq!(serde_json::to_value(&d).unwrap()["kind"], json!("fan_controller"));
    }

    #[test]
    fn snapshot_serializes_missing_values_as_null() {
        let snap = Snapshot { revision: 1, seq: 2, timestamp_ms: 3, values: vec![Some(1.5), None] };
        assert_eq!(
            serde_json::to_value(&snap).unwrap(),
            json!({ "revision": 1, "seq": 2, "timestampMs": 3, "values": [1.5, null] })
        );
    }
}
```

- [ ] **Step 3: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core`
Risultato atteso: errore di compilazione (`cannot find type Sensor`, ecc.).

- [ ] **Step 4: Implementa il modello**

In testa a `crates/oma-core/src/model.rs`, prima del modulo `tests`:

```rust
//! Data model shared by providers, the engine and the UI.

use serde::Serialize;

/// Kind of hardware component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Cpu,
    Gpu,
    Memory,
    Storage,
    Network,
    Motherboard,
    Battery,
    FanController,
    Psu,
}

/// What a sensor measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    Temperature,
    Load,
    Clock,
    Power,
    Voltage,
    Current,
    Fan,
    Data,
    Throughput,
    Energy,
    Flag,
    Percent,
}

impl SensorKind {
    /// Segment used in sensor ids (`<device>/<kind>/<name>`).
    pub fn as_str(self) -> &'static str {
        match self {
            SensorKind::Temperature => "temperature",
            SensorKind::Load => "load",
            SensorKind::Clock => "clock",
            SensorKind::Power => "power",
            SensorKind::Voltage => "voltage",
            SensorKind::Current => "current",
            SensorKind::Fan => "fan",
            SensorKind::Data => "data",
            SensorKind::Throughput => "throughput",
            SensorKind::Energy => "energy",
            SensorKind::Flag => "flag",
            SensorKind::Percent => "percent",
        }
    }
}

/// Unit of a sensor value. Values are stored in these base units; the UI
/// converts for display (e.g. bytes/s to bit/s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Celsius,
    Percent,
    Megahertz,
    Watt,
    Volt,
    Ampere,
    Rpm,
    Bytes,
    BytesPerSecond,
    BitsPerSecond,
    Joule,
    Boolean,
}

/// Where a reading comes from; shown as a badge in the Advanced view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Pdh,
    Win32,
    IpHelper,
    Mock,
}

/// Translatable label. The UI looks up `sensor.<key>` in its catalogs and
/// substitutes `{arg}` (e.g. a thread index or a drive letter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arg: Option<String>,
}

impl Label {
    pub fn new(key: &str) -> Self {
        Self { key: key.to_owned(), arg: None }
    }

    pub fn with_arg(key: &str, arg: impl Into<String>) -> Self {
        Self { key: key.to_owned(), arg: Some(arg.into()) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub kind: DeviceKind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub properties: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sensor {
    pub id: String,
    pub device_id: String,
    pub kind: SensorKind,
    pub unit: Unit,
    pub label: Label,
    pub source: Source,
    pub category: String,
}

impl Sensor {
    /// Builds a sensor whose stable id is `<device_id>/<kind>/<name>`.
    pub fn new(device_id: &str, kind: SensorKind, name: &str, unit: Unit, label: Label, source: Source) -> Self {
        Self {
            id: format!("{device_id}/{}/{name}", kind.as_str()),
            device_id: device_id.to_owned(),
            kind,
            unit,
            label,
            source,
            category: kind.as_str().to_owned(),
        }
    }
}

/// Every device and sensor currently known. `revision` changes whenever the
/// hardware set changes; snapshot values are indexed by `sensors` order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Schema {
    pub revision: u64,
    pub devices: Vec<Device>,
    pub sensors: Vec<Sensor>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub sensor_id: String,
    pub value: Option<f64>,
    pub timestamp_ms: u64,
}

/// One sampling cycle. `values[i]` belongs to `schema.sensors[i]` of the
/// schema with the same `revision`; `None` means "not available".
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub seq: u64,
    pub timestamp_ms: u64,
    pub values: Vec<Option<f64>>,
}
```

- [ ] **Step 5: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core`
Risultato atteso: `test result: ok. 5 passed`.

- [ ] **Step 6: Formattazione, lint e commit**

```bash
cargo fmt --all
cargo clippy -p oma-core --all-targets -- -D warnings
git add Cargo.toml Cargo.lock .gitignore LICENSE README.md crates/oma-core
git commit -m "feat(core): workspace and data model"
```

---

### Task 2: `Provider`, filtro dei valori anomali e `CounterRate`

**File:**
- Crea: `crates/oma-core/src/provider.rs`, `crates/oma-core/src/sanitize.rs`, `crates/oma-core/src/rate.rs`
- Modifica: `crates/oma-core/src/lib.rs`

**Interfacce:**
- Usa: `oma_core::model::{Device, Sensor, Unit}` (Task 1).
- Produce:
  - `oma_core::provider::Inventory { devices: Vec<Device>, sensors: Vec<Sensor> }` (con `Default`, `Clone`, `PartialEq`)
  - `oma_core::provider::ProviderError { Rediscover, Failed(String) }` (con `thiserror::Error`, `PartialEq`)
  - `oma_core::provider::Provider`: `trait Provider: Send { fn name(&self) -> &'static str; fn discover(&mut self) -> Result<Inventory, ProviderError>; fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError>; }`. `poll` restituisce valori allineati ai `sensors` dell'ultimo `discover`.
  - `oma_core::sanitize::sanitize(unit: Unit, value: Option<f64>) -> Option<f64>`
  - `oma_core::rate::CounterRate` con `new()` e `update(&mut self, value: u64, t_ms: u64) -> Option<f64>` (unità al secondo)

- [ ] **Step 1: Scrivi i test (falliscono)**

`crates/oma-core/src/sanitize.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_stays_none() {
        assert_eq!(sanitize(Unit::Celsius, None), None);
    }

    #[test]
    fn rejects_non_finite() {
        assert_eq!(sanitize(Unit::Watt, Some(f64::NAN)), None);
        assert_eq!(sanitize(Unit::Watt, Some(f64::INFINITY)), None);
    }

    #[test]
    fn temperature_must_be_physically_plausible() {
        assert_eq!(sanitize(Unit::Celsius, Some(-60.0)), None);
        assert_eq!(sanitize(Unit::Celsius, Some(45.0)), Some(45.0));
        assert_eq!(sanitize(Unit::Celsius, Some(151.0)), None);
    }

    #[test]
    fn percent_must_stay_within_0_and_100() {
        assert_eq!(sanitize(Unit::Percent, Some(100.0)), Some(100.0));
        assert_eq!(sanitize(Unit::Percent, Some(100.5)), None);
        assert_eq!(sanitize(Unit::Percent, Some(-0.1)), None);
    }

    #[test]
    fn negative_voltage_rails_are_allowed() {
        assert_eq!(sanitize(Unit::Volt, Some(-12.1)), Some(-12.1));
        assert_eq!(sanitize(Unit::Volt, Some(25.0)), None);
    }

    #[test]
    fn counters_and_rates_cannot_be_negative() {
        assert_eq!(sanitize(Unit::Bytes, Some(-1.0)), None);
        assert_eq!(sanitize(Unit::BytesPerSecond, Some(0.0)), Some(0.0));
    }
}
```

`crates/oma-core/src/rate.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_has_no_rate() {
        let mut r = CounterRate::new();
        assert_eq!(r.update(1_000, 0), None);
    }

    #[test]
    fn rate_is_per_second() {
        let mut r = CounterRate::new();
        r.update(1_000, 0);
        assert_eq!(r.update(2_000, 500), Some(2_000.0));
    }

    #[test]
    fn zero_elapsed_time_yields_none() {
        let mut r = CounterRate::new();
        r.update(1_000, 100);
        assert_eq!(r.update(1_500, 100), None);
    }

    #[test]
    fn counter_reset_restarts_from_new_baseline() {
        let mut r = CounterRate::new();
        r.update(10_000, 0);
        assert_eq!(r.update(50, 1_000), None);
        assert_eq!(r.update(1_050, 2_000), Some(1_000.0));
    }
}
```

`crates/oma-core/src/lib.rs` diventa:

```rust
//! Platform-independent core of OpenMonitor Advanced: data model, providers,
//! engine, history and sampling loop.

pub mod model;
pub mod provider;
pub mod rate;
pub mod sanitize;
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core`
Risultato atteso: errore di compilazione (`cannot find function sanitize`, `cannot find type CounterRate`).

- [ ] **Step 3: Implementa**

`crates/oma-core/src/provider.rs`:

```rust
//! Contract between data sources and the engine.

use crate::model::{Device, Sensor};

/// Devices and sensors a provider exposes. `poll` values follow `sensors` order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inventory {
    pub devices: Vec<Device>,
    pub sensors: Vec<Sensor>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProviderError {
    /// The hardware set changed (e.g. a disk was plugged in): call `discover` again.
    #[error("hardware configuration changed")]
    Rediscover,
    #[error("{0}")]
    Failed(String),
}

/// A source of sensor readings (PDH, a vendor SDK, the privileged service...).
pub trait Provider: Send {
    /// Short name used in logs.
    fn name(&self) -> &'static str;

    /// (Re)initialises the provider and lists its devices and sensors.
    fn discover(&mut self) -> Result<Inventory, ProviderError>;

    /// Reads current values, aligned with the sensors of the last `discover`.
    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError>;
}
```

`crates/oma-core/src/sanitize.rs`, in testa al file:

```rust
//! Drops values outside the physically plausible range (spec §8).

use crate::model::Unit;

pub fn sanitize(unit: Unit, value: Option<f64>) -> Option<f64> {
    let v = value?;
    if !v.is_finite() {
        tracing::debug!(?unit, v, "discarding non-finite value");
        return None;
    }
    let plausible = match unit {
        Unit::Celsius => (-50.0..=150.0).contains(&v),
        Unit::Percent => (0.0..=100.0).contains(&v),
        Unit::Megahertz => (0.0..=20_000.0).contains(&v),
        Unit::Volt => (-20.0..=20.0).contains(&v),
        Unit::Boolean => v == 0.0 || v == 1.0,
        _ => v >= 0.0,
    };
    if plausible {
        Some(v)
    } else {
        tracing::debug!(?unit, v, "discarding implausible value");
        None
    }
}
```

`crates/oma-core/src/rate.rs`, in testa al file:

```rust
//! Converts cumulative counters (bytes, energy...) into per-second rates.

#[derive(Debug, Clone, Default)]
pub struct CounterRate {
    last: Option<(u64, u64)>,
}

impl CounterRate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a cumulative `value` sampled at monotonic time `t_ms` and returns
    /// units per second. Returns `None` for the first sample, when no time has
    /// elapsed, or when the counter went backwards (device reset): the new
    /// value becomes the baseline.
    pub fn update(&mut self, value: u64, t_ms: u64) -> Option<f64> {
        let (prev_value, prev_t) = self.last.replace((value, t_ms))?;
        if t_ms <= prev_t || value < prev_value {
            return None;
        }
        Some((value - prev_value) as f64 * 1000.0 / (t_ms - prev_t) as f64)
    }
}
```

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core`
Risultato atteso: tutti i test OK (5 del modello + 6 di sanitize + 4 di rate).

- [ ] **Step 5: Lint e commit**

```bash
cargo fmt --all
cargo clippy -p oma-core --all-targets -- -D warnings
git add crates/oma-core
git commit -m "feat(core): provider contract, value sanitizer and counter rates"
```

---

### Task 3: storico (`History`)

**File:**
- Crea: `crates/oma-core/src/history.rs`
- Modifica: `crates/oma-core/src/lib.rs` (aggiungi `pub mod history;`)

**Interfacce:**
- Usa: niente di nuovo.
- Produce (`oma_core::history`):
  - `struct HistoryWindow { timestamps_ms: Vec<u64>, series: Vec<Vec<Option<f64>>> }`, serializzata in camelCase (`timestampsMs`)
  - `struct History`, con i metodi:
    - `new(capacity: usize)`
    - `len()` e `is_empty()`
    - `set_sensors(&mut self, ids: &[String])`, che conserva le serie per id
    - `push(&mut self, timestamp_ms: u64, values: &[Option<f64>])`
    - `window(&self, ids: &[String], since_ms: u64) -> HistoryWindow`

- [ ] **Step 1: Scrivi i test (falliscono)**

`crates/oma-core/src/history.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn evicts_oldest_sample_when_full() {
        let mut h = History::new(2);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0)]);
        h.push(2, &[Some(2.0)]);
        h.push(3, &[Some(3.0)]);
        let w = h.window(&ids(&["a"]), 0);
        assert_eq!(w.timestamps_ms, vec![2, 3]);
        assert_eq!(w.series, vec![vec![Some(2.0), Some(3.0)]]);
    }

    #[test]
    fn window_returns_only_recent_samples() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        for t in [1_000, 2_000, 3_000] {
            h.push(t, &[Some(t as f64)]);
        }
        let w = h.window(&ids(&["a"]), 2_000);
        assert_eq!(w.timestamps_ms, vec![2_000, 3_000]);
        assert_eq!(w.series[0], vec![Some(2_000.0), Some(3_000.0)]);
    }

    #[test]
    fn missing_values_round_trip_as_none() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[None]);
        assert_eq!(h.window(&ids(&["a"]), 0).series[0], vec![None]);
    }

    #[test]
    fn set_sensors_keeps_existing_series_and_pads_new_ones() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0)]);
        h.set_sensors(&ids(&["b", "a"]));
        h.push(2, &[Some(20.0), Some(2.0)]);
        let w = h.window(&ids(&["a", "b"]), 0);
        assert_eq!(w.series[0], vec![Some(1.0), Some(2.0)]);
        assert_eq!(w.series[1], vec![None, Some(20.0)]);
    }

    #[test]
    fn unknown_ids_yield_empty_values() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0)]);
        assert_eq!(h.window(&ids(&["nope"]), 0).series[0], vec![None]);
    }

    #[test]
    fn clock_jumping_backwards_keeps_only_samples_after_the_jump() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        for t in [10_000, 20_000, 5_000, 6_000] {
            h.push(t, &[Some(1.0)]);
        }
        assert_eq!(h.window(&ids(&["a"]), 0).timestamps_ms, vec![5_000, 6_000]);
    }

    #[test]
    fn len_counts_samples() {
        let mut h = History::new(10);
        assert!(h.is_empty());
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0)]);
        assert_eq!(h.len(), 1);
    }
}
```

Aggiungi `pub mod history;` in `crates/oma-core/src/lib.rs`, dopo `pub mod model;`, poi riordina i moduli in ordine alfabetico: `history`, `model`, `provider`, `rate`, `sanitize`.

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core history`
Risultato atteso: errore di compilazione (`cannot find type History`).

- [ ] **Step 3: Implementa**

In testa a `crates/oma-core/src/history.rs`:

```rust
//! In-memory ring buffer of recent samples (spec §4.2).

use std::collections::{HashMap, VecDeque};

use serde::Serialize;

/// A slice of history aligned on shared timestamps.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryWindow {
    pub timestamps_ms: Vec<u64>,
    /// One series per requested id, same length as `timestamps_ms`.
    pub series: Vec<Vec<Option<f64>>>,
}

#[derive(Debug)]
pub struct History {
    capacity: usize,
    timestamps: VecDeque<u64>,
    /// NaN marks a missing value; it saves memory compared to `Option<f64>`.
    series: Vec<VecDeque<f64>>,
    index: HashMap<String, usize>,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "history capacity must be positive");
        Self {
            capacity,
            timestamps: VecDeque::with_capacity(capacity),
            series: Vec::new(),
            index: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.timestamps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }

    /// Replaces the sensor list. Series of ids that still exist are kept;
    /// new ids start with missing values for the samples already stored.
    pub fn set_sensors(&mut self, ids: &[String]) {
        let old_index = std::mem::take(&mut self.index);
        let mut previous: HashMap<String, VecDeque<f64>> = old_index
            .into_iter()
            .map(|(id, i)| (id, std::mem::take(&mut self.series[i])))
            .collect();
        let len = self.timestamps.len();
        self.series = ids
            .iter()
            .map(|id| previous.remove(id).unwrap_or_else(|| std::iter::repeat_n(f64::NAN, len).collect()))
            .collect();
        self.index = ids.iter().enumerate().map(|(i, id)| (id.clone(), i)).collect();
    }

    /// Appends one sample per sensor, in `set_sensors` order.
    pub fn push(&mut self, timestamp_ms: u64, values: &[Option<f64>]) {
        assert_eq!(values.len(), self.series.len(), "values must match the sensor list");
        if self.timestamps.len() == self.capacity {
            self.timestamps.pop_front();
            for s in &mut self.series {
                s.pop_front();
            }
        }
        self.timestamps.push_back(timestamp_ms);
        for (s, v) in self.series.iter_mut().zip(values) {
            s.push_back(v.unwrap_or(f64::NAN));
        }
    }

    /// Samples taken at or after `since_ms`. The scan goes backwards from the
    /// newest sample and also stops where time runs backwards (wall clock set
    /// back), so the result is always in chronological order.
    pub fn window(&self, ids: &[String], since_ms: u64) -> HistoryWindow {
        let mut count = 0;
        let mut later = u64::MAX;
        for &t in self.timestamps.iter().rev() {
            if t < since_ms || t > later {
                break;
            }
            later = t;
            count += 1;
        }
        let start = self.timestamps.len() - count;
        let series = ids
            .iter()
            .map(|id| match self.index.get(id) {
                Some(&i) => self.series[i].range(start..).map(|&v| (!v.is_nan()).then_some(v)).collect(),
                None => vec![None; count],
            })
            .collect();
        HistoryWindow { timestamps_ms: self.timestamps.range(start..).copied().collect(), series }
    }
}
```

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core history`
Risultato atteso: `7 passed`.

- [ ] **Step 5: Lint e commit**

```bash
cargo fmt --all
cargo clippy -p oma-core --all-targets -- -D warnings
git add crates/oma-core
git commit -m "feat(core): in-memory sample history"
```

---

### Task 4: `Engine` (discovery, poll, backoff, schema)

**File:**
- Crea: `crates/oma-core/src/engine.rs`, `crates/oma-core/src/worker.rs`
- Modifica: `crates/oma-core/src/lib.rs` (aggiungi `pub mod engine;`)

**Interfacce:**
- Usa:
  - `Provider`, `Inventory`, `ProviderError` (Task 2)
  - `sanitize` (Task 2)
  - `History`, `HistoryWindow` (Task 3)
  - `Schema`, `Snapshot`, `Sensor::new` (Task 1)
- Produce (`oma_core::engine`):
  - `struct TickOutput { snapshot: Snapshot, schema: Option<Schema> }`, dove `schema` è `Some` quando lo schema è cambiato in quel tick (sempre al primo tick)
  - `struct Engine`, con i metodi:
    - `new(providers: Vec<Box<dyn Provider>>, history_capacity: usize)`
    - `tick(&mut self, timestamp_ms: u64, monotonic_ms: u64) -> TickOutput`
    - `schema(&self) -> &Schema`
    - `history(&self) -> &History`
  - `fn backoff_ms(failures: u32) -> u64`

**Comportamento (spec §4.1 e §8):**
- Ogni provider vive in un worker persistente. Un solo timer nel sampler; discovery e poll sono paralleli, con deadline comune di 200 ms. Al timeout si conserva l’ultimo valore (§4.1), senza avviare altre chiamate finché quella pendente termina. Il worker gestisce discovery, backoff e recupero; solo un poll riuscito azzera gli errori consecutivi. Lo shutdown non attende una chiamata Win32 bloccata.
- Un errore di `discover` o di `poll`, o un numero di valori sbagliato, rende il provider degradato. Il nuovo tentativo passa da `discover` dopo `backoff_ms(failures)`: 5 s, 10 s, 20 s, 40 s, poi fisso a 60 s.
- Un provider degradato **resta nello schema**; i suoi valori sono `None`.
- `Err(Rediscover)` richiede una nuova discovery: `discover` viene rieseguito al tick successivo, e se l'inventario cambia si passa a una nuova revisione dello schema.

- [ ] **Step 1: Scrivi i test (falliscono)**

`crates/oma-core/src/engine.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use crate::provider::ProviderError;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    type PollResult = Result<Vec<Option<f64>>, ProviderError>;

    #[derive(Default)]
    struct Script {
        inventory: Inventory,
        discover_errors: VecDeque<ProviderError>,
        polls: VecDeque<PollResult>,
        discover_calls: usize,
        poll_calls: usize,
    }

    struct Fake {
        name: &'static str,
        script: Arc<Mutex<Script>>,
    }

    impl Provider for Fake {
        fn name(&self) -> &'static str {
            self.name
        }

        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            let mut s = self.script.lock().unwrap();
            s.discover_calls += 1;
            match s.discover_errors.pop_front() {
                Some(e) => Err(e),
                None => Ok(s.inventory.clone()),
            }
        }

        fn poll(&mut self) -> PollResult {
            let mut s = self.script.lock().unwrap();
            s.poll_calls += 1;
            let n = s.inventory.sensors.len();
            s.polls.pop_front().unwrap_or_else(|| Ok(vec![Some(1.0); n]))
        }
    }

    fn inventory(device: &str, sensors: &[&str]) -> Inventory {
        Inventory {
            devices: vec![Device { id: device.into(), kind: DeviceKind::Cpu, name: device.into(), vendor: None, properties: Default::default() }],
            sensors: sensors
                .iter()
                .map(|n| Sensor::new(device, SensorKind::Load, n, Unit::Percent, Label::new("test"), Source::Mock))
                .collect(),
        }
    }

    fn fake(name: &'static str, inv: Inventory) -> (Box<dyn Provider>, Arc<Mutex<Script>>) {
        let script = Arc::new(Mutex::new(Script { inventory: inv, ..Default::default() }));
        (Box::new(Fake { name, script: script.clone() }), script)
    }

    fn failed() -> ProviderError {
        ProviderError::Failed("boom".into())
    }

    #[test]
    fn first_tick_discovers_and_publishes_schema() {
        let (p, _) = fake("a", inventory("dev/a", &["x", "y"]));
        let mut e = Engine::new(vec![p], 10);
        let out = e.tick(1_000, 1_000);
        let schema = out.schema.expect("schema on first tick");
        assert_eq!(schema.revision, 1);
        assert_eq!(schema.sensors.len(), 2);
        assert_eq!(out.snapshot.revision, 1);
        assert_eq!(out.snapshot.seq, 1);
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(1.0)]);
        assert!(e.tick(2_000, 2_000).schema.is_none());
    }

    #[test]
    fn engine_without_providers_publishes_an_empty_schema() {
        let mut e = Engine::new(Vec::new(), 10);
        let out = e.tick(0, 0);
        assert_eq!(out.schema.map(|s| s.revision), Some(1));
        assert!(out.snapshot.values.is_empty());
    }

    #[test]
    fn implausible_values_become_none() {
        let (p, script) = fake("a", inventory("dev/a", &["x", "y"]));
        script.lock().unwrap().polls.push_back(Ok(vec![Some(150.0), Some(f64::NAN)]));
        let mut e = Engine::new(vec![p], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![None, None]);
    }

    #[test]
    fn failing_provider_backs_off_and_recovers() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script.lock().unwrap().polls.push_back(Err(failed()));
        let mut e = Engine::new(vec![p], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![None]);
        assert_eq!(e.tick(1_000, 1_000).snapshot.values, vec![None]);
        assert_eq!(script.lock().unwrap().discover_calls, 1);
        assert_eq!(e.tick(5_000, 5_000).snapshot.values, vec![Some(1.0)]);
        assert_eq!(script.lock().unwrap().discover_calls, 2);
    }

    #[test]
    fn repeated_failures_grow_the_backoff() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script.lock().unwrap().discover_errors.extend([failed(), failed()]);
        let mut e = Engine::new(vec![p], 10);
        e.tick(0, 0);
        e.tick(5_000, 5_000);
        e.tick(14_999, 14_999);
        assert_eq!(script.lock().unwrap().discover_calls, 2);
        let out = e.tick(15_000, 15_000);
        assert_eq!(script.lock().unwrap().discover_calls, 3);
        assert_eq!(out.schema.map(|s| s.revision), Some(2));
        assert_eq!(out.snapshot.values, vec![Some(1.0)]);
    }

    #[test]
    fn rediscover_rebuilds_schema_and_keeps_history_by_id() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        let mut e = Engine::new(vec![p], 10);
        e.tick(1_000, 1_000);
        {
            let mut s = script.lock().unwrap();
            s.polls.push_back(Err(ProviderError::Rediscover));
            s.inventory = inventory("dev/a", &["x", "z"]);
        }
        let out = e.tick(2_000, 2_000);
        assert!(out.schema.is_none());
        assert_eq!(out.snapshot.values, vec![None]);
        let out = e.tick(3_000, 3_000);
        assert_eq!(out.schema.map(|s| s.revision), Some(2));
        assert_eq!(out.snapshot.values, vec![Some(1.0), Some(1.0)]);
        let w = e.history().window(&["dev/a/load/x".into(), "dev/a/load/z".into()], 0);
        assert_eq!(w.timestamps_ms, vec![1_000, 2_000, 3_000]);
        assert_eq!(w.series[0], vec![Some(1.0), None, Some(1.0)]);
        assert_eq!(w.series[1], vec![None, None, Some(1.0)]);
    }

    #[test]
    fn wrong_value_count_degrades_provider() {
        let (p, script) = fake("a", inventory("dev/a", &["x", "y"]));
        script.lock().unwrap().polls.push_back(Ok(vec![Some(1.0)]));
        let mut e = Engine::new(vec![p], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![None, None]);
        e.tick(1_000, 1_000);
        assert_eq!(script.lock().unwrap().poll_calls, 1);
    }

    #[test]
    fn failure_of_one_provider_does_not_affect_another() {
        let (a, script_a) = fake("a", inventory("dev/a", &["x"]));
        let (b, _) = fake("b", inventory("dev/b", &["y"]));
        script_a.lock().unwrap().polls.push_back(Err(failed()));
        let mut e = Engine::new(vec![a, b], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![None, Some(1.0)]);
    }

    #[test]
    fn successful_rediscovery_does_not_reset_poll_backoff() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script.lock().unwrap().polls.extend([Err(failed()), Err(failed())]);
        let mut e = Engine::new(vec![p], 10);
        e.tick(100_000, 0);
        e.tick(1_000, 5_000); // Wall clock moves backwards; retry still occurs.
        e.tick(2_000, 14_999);
        assert_eq!(script.lock().unwrap().poll_calls, 2);
        assert_eq!(e.tick(3_000, 15_000).snapshot.values, vec![Some(1.0)]);
    }

    struct Blocked(std::sync::mpsc::Receiver<()>);
    impl Provider for Blocked {
        fn name(&self) -> &'static str { "blocked" }
        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            let _ = self.0.recv();
            Ok(Inventory::default())
        }
        fn poll(&mut self) -> PollResult { Ok(Vec::new()) }
    }
    #[test]
    fn blocked_discovery_does_not_block_other_providers_or_drop() {
        let (release, wait) = std::sync::mpsc::channel();
        let (fast, _) = fake("fast", inventory("dev/fast", &["x"]));
        let mut e = Engine::new(vec![Box::new(Blocked(wait)), fast], 10);
        let start = Instant::now();
        assert_eq!(e.tick(0, 0).snapshot.values, vec![Some(1.0)]);
        assert!(start.elapsed() < Duration::from_secs(1));
        let start = Instant::now();
        drop(e);
        assert!(start.elapsed() < Duration::from_millis(100));
        drop(release);
    }

    struct SlowPoll {
        wait: std::sync::mpsc::Receiver<()>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Provider for SlowPoll {
        fn name(&self) -> &'static str { "slow-poll" }
        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(inventory("dev/slow", &["x"]))
        }
        fn poll(&mut self) -> PollResult {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
                let _ = self.wait.recv();
            }
            Ok(vec![Some(42.0)])
        }
    }
    #[test]
    fn timed_out_poll_reuses_last_value_without_queuing_more_work() {
        let (release, wait) = std::sync::mpsc::channel();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slow = SlowPoll { wait, calls: calls.clone() };
        let (fast, script) = fake("fast", inventory("dev/fast", &["x"]));
        let mut e = Engine::new(vec![Box::new(slow), fast], 10);
        assert_eq!(e.tick(0, 0).snapshot.values, vec![Some(42.0), Some(1.0)]);
        script.lock().unwrap().polls.push_back(Ok(vec![Some(2.0)]));
        assert_eq!(e.tick(1_000, 1_000).snapshot.values, vec![Some(42.0), Some(2.0)]);
        e.tick(2_000, 2_000);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        drop(e);
        drop(release);
    }

    #[test]
    fn backoff_doubles_up_to_one_minute() {
        assert_eq!([1, 2, 3, 4, 5, 10].map(backoff_ms), [5_000, 10_000, 20_000, 40_000, 60_000, 60_000]);
    }
}
```

Aggiungi `pub mod engine;` in `crates/oma-core/src/lib.rs`, in ordine alfabetico, prima di `history`.

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core engine`
Risultato atteso: errore di compilazione (`cannot find type Engine`).

- [ ] **Step 3: Implementa**

In testa a `crates/oma-core/src/engine.rs`:

```rust
//! Parallel provider sampling with one shared deadline, schema revisions and history.
use std::time::{Duration, Instant};
use crate::history::History;
use crate::model::{Schema, Snapshot};
use crate::provider::{Inventory, Provider};
use crate::sanitize::sanitize;
use crate::worker::Worker;

pub fn backoff_ms(failures: u32) -> u64 {
    (5_000u64 << failures.saturating_sub(1).min(4)).min(60_000)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TickOutput {
    pub snapshot: Snapshot,
    pub schema: Option<Schema>,
}

struct Slot {
    worker: Worker,
    inventory: Inventory,
    last: Vec<Option<f64>>,
}

pub struct Engine {
    slots: Vec<Slot>,
    schema: Schema,
    history: History,
    seq: u64,
}

impl Engine {
    pub fn new(providers: Vec<Box<dyn Provider>>, history_capacity: usize) -> Self {
        Self {
            slots: providers.into_iter().map(|p| Slot {
                worker: Worker::spawn(p), inventory: Inventory::default(), last: Vec::new(),
            }).collect(),
            schema: Schema::default(), history: History::new(history_capacity), seq: 0,
        }
    }
    pub fn schema(&self) -> &Schema { &self.schema }
    pub fn history(&self) -> &History { &self.history }
    pub fn sequence(&self) -> u64 { self.seq }

    /// `timestamp_ms` is Unix time for display; `monotonic_ms` drives retry deadlines.
    pub fn tick(&mut self, timestamp_ms: u64, monotonic_ms: u64) -> TickOutput {
        // One budget for the entire cycle, not N sequential provider timeouts.
        let deadline = Instant::now() + Duration::from_millis(200);
        for slot in &mut self.slots { slot.worker.start(monotonic_ms); }
        let mut changed = self.schema.revision == 0;
        for slot in &mut self.slots {
            if let Some(sample) = slot.worker.finish(deadline) {
                changed |= slot.inventory != sample.inventory;
                slot.inventory = sample.inventory;
                slot.last = sample.values;
            }
            // A timeout retains the last values (§4.1). An explicit error returns
            // None from the worker (§8). No extra worker/request is spawned while busy.
        }
        if changed {
            self.schema = Schema {
                revision: self.schema.revision + 1,
                devices: self.slots.iter().flat_map(|s| s.inventory.devices.iter().cloned()).collect(),
                sensors: self.slots.iter().flat_map(|s| s.inventory.sensors.iter().cloned()).collect(),
            };
            self.history.set_sensors(&self.schema.sensors.iter().map(|s| s.id.clone()).collect::<Vec<_>>());
        }
        let values: Vec<_> = self.slots.iter().flat_map(|s| s.last.iter().copied())
            .zip(&self.schema.sensors).map(|(value, sensor)| sanitize(sensor.unit, value)).collect();
        self.history.push(timestamp_ms, &values);
        self.seq += 1;
        TickOutput {
            snapshot: Snapshot { revision: self.schema.revision, seq: self.seq, timestamp_ms, values },
            schema: changed.then(|| self.schema.clone()),
        }
    }
}
```

`crates/oma-core/src/worker.rs` (aggiungi `mod worker;` in `lib.rs`):

```rust
//! One persistent worker per provider. No timer and at most one in-flight request.
use std::sync::mpsc::{self, Receiver, SyncSender, RecvTimeoutError};
use std::time::Instant;
use crate::engine::backoff_ms;
use crate::provider::{Inventory, Provider, ProviderError};

pub(crate) struct Sample {
    pub inventory: Inventory,
    pub values: Vec<Option<f64>>,
}

pub(crate) struct Worker {
    tx: SyncSender<u64>,
    rx: Receiver<Sample>,
    pending: bool,
}

impl Worker {
    pub fn spawn(mut provider: Box<dyn Provider>) -> Self {
        let (tx, requests) = mpsc::sync_channel::<u64>(1);
        let (responses, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new().name(format!("oma-{}", provider.name())).spawn(move || {
            let mut inventory = Inventory::default();
            let mut discover = true;
            let mut failures = 0u32;
            let mut retry_at = 0u64;
            while let Ok(now) = requests.recv() {
                let mut values = vec![None; inventory.sensors.len()];
                if now >= retry_at {
                    // A Rust panic is isolated; native DLL access violations are not catchable.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if discover {
                            inventory = provider.discover()?;
                            discover = false;
                        }
                        let polled = provider.poll()?;
                        if polled.len() != inventory.sensors.len() {
                            return Err(ProviderError::Failed("poll value count mismatch".into()));
                        }
                        Ok(polled)
                    })).unwrap_or_else(|_| Err(ProviderError::Failed("provider panicked".into())));
                    match result {
                        Ok(polled) => {
                            values = polled;
                            failures = 0; // Only a successful poll ends a failure streak.
                        }
                        Err(ProviderError::Rediscover) => {
                            discover = true;
                            values = vec![None; inventory.sensors.len()];
                        }
                        Err(err) => {
                            failures = failures.saturating_add(1);
                            retry_at = now.saturating_add(backoff_ms(failures));
                            discover = true;
                            values = vec![None; inventory.sensors.len()];
                            tracing::warn!(provider = provider.name(), %err, failures, "provider degraded");
                        }
                    }
                }
                if responses.send(Sample { inventory: inventory.clone(), values }).is_err() { break; }
            }
        }).expect("provider worker");
        // Deliberately do not join a worker executing an uninterruptible Win32 call.
        // Dropping the channels ends an idle worker; blocked workers exit with the process.
        Self { tx, rx, pending: false }
    }

    pub fn start(&mut self, monotonic_ms: u64) {
        if !self.pending && self.tx.try_send(monotonic_ms).is_ok() { self.pending = true; }
    }

    pub fn finish(&mut self, deadline: Instant) -> Option<Sample> {
        if !self.pending { return None; }
        match self.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(sample) => { self.pending = false; Some(sample) }
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => { self.pending = false; None }
        }
    }
}
```

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core engine`
Risultato atteso: tutti i test engine PASS, inclusi backoff dopo rediscovery e provider bloccato.

- [ ] **Step 5: Lint e commit**

```bash
cargo fmt --all
cargo clippy -p oma-core --all-targets -- -D warnings
git add crates/oma-core
git commit -m "feat(core): engine with failure isolation and schema revisions"
```

---

### Task 5: `Sampler` (thread di campionamento)

**File:**
- Crea: `crates/oma-core/src/sampler.rs`
- Modifica: `crates/oma-core/src/lib.rs` (aggiungi `pub mod sampler;` in ordine alfabetico, dopo `sanitize`)

**Interfacce:**
- Usa: `Engine`, `TickOutput` (Task 4); `Provider`, `Inventory` (Task 2); `Sensor::new` (Task 1).
- Produce (`oma_core::sampler`):
  - `fn next_deadline(prev: Instant, now: Instant, interval: Duration) -> Instant`
  - `fn unix_ms() -> u64`
  - `struct Sampler`, con:
    - `Sampler::spawn<F: FnMut(&TickOutput) + Send + 'static>(engine: Arc<Mutex<Engine>>, interval: Duration, on_tick: F) -> Sampler`
    - `stop(self)`; anche il `Drop` ferma il thread e aspetta che termini

**Regole:**
- Un solo thread con timer, chiamato `oma-sampler`; i worker dei provider attendono richieste senza timer propri.
- **Nessuna modifica della risoluzione del timer:** si usano solo `park_timeout` e `sleep` standard.
- `on_tick` viene chiamato **dopo** aver rilasciato il lock dell'engine.
- Dopo una sospensione i tick persi si saltano, non si recuperano.

- [ ] **Step 1: Scrivi i test (falliscono)**

`crates/oma-core/src/sampler.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use crate::provider::{Inventory, Provider, ProviderError};
    use std::sync::mpsc;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn next_deadline_advances_by_one_interval() {
        let t0 = Instant::now();
        assert_eq!(next_deadline(t0, t0 + ms(10), ms(1_000)), t0 + ms(1_000));
    }

    #[test]
    fn next_deadline_skips_missed_ticks_after_sleep() {
        let t0 = Instant::now();
        assert_eq!(next_deadline(t0, t0 + ms(10_500), ms(1_000)), t0 + ms(11_500));
    }

    #[test]
    fn next_deadline_on_exact_boundary_moves_forward() {
        let t0 = Instant::now();
        assert_eq!(next_deadline(t0, t0 + ms(3_000), ms(1_000)), t0 + ms(4_000));
    }

    struct Const;

    impl Provider for Const {
        fn name(&self) -> &'static str {
            "const"
        }

        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(Inventory {
                devices: vec![Device { id: "d".into(), kind: DeviceKind::Cpu, name: "d".into(), vendor: None, properties: Default::default() }],
                sensors: vec![Sensor::new("d", SensorKind::Load, "x", Unit::Percent, Label::new("t"), Source::Mock)],
            })
        }

        fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
            Ok(vec![Some(42.0)])
        }
    }

    #[test]
    fn configured_interval_retains_one_hour() {
        assert!(sample_interval(499).is_err());
        assert!(sample_interval(5_001).is_err());
        for (ms, capacity) in [(500, 7200), (1000, 3600), (5000, 720)] {
            assert_eq!(history_capacity(sample_interval(ms).unwrap()), capacity);
        }
    }

    #[test]
    fn sampler_ticks_and_stops_promptly() {
        let engine = Arc::new(Mutex::new(Engine::new(vec![Box::new(Const)], 16)));
        let (tx, rx) = mpsc::channel();
        let sampler = Sampler::spawn(engine.clone(), ms(20), move |out| {
            let _ = tx.send(out.snapshot.seq);
        });
        let seqs: Vec<u64> = (0..3).map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap()).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
        let started = Instant::now();
        sampler.stop();
        assert!(started.elapsed() < ms(500));
        assert!(engine.lock().unwrap().history().len() >= 3);
    }
}
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core sampler`
Risultato atteso: errore di compilazione (`cannot find function next_deadline`).

- [ ] **Step 3: Implementa**

In testa a `crates/oma-core/src/sampler.rs`:

```rust
//! Background sampling loop: one coalesced timer, never raises the system
//! timer resolution (spec §4.1).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::engine::{Engine, TickOutput};

/// Next tick after `prev`. Ticks missed while the machine slept are skipped
/// instead of being replayed in a burst.
pub fn next_deadline(prev: Instant, now: Instant, interval: Duration) -> Instant {
    let next = prev + interval;
    if next > now {
        return next;
    }
    now + interval // Coalesce missed ticks without integer overflow or catch-up bursts.
}

/// Validated application sampling interval; tests may use shorter intervals directly.
pub fn sample_interval(ms: u64) -> Result<Duration, &'static str> {
    if (500..=5_000).contains(&ms) { Ok(Duration::from_millis(ms)) }
    else { Err("sampling interval must be 500..=5000 ms") }
}

pub fn history_capacity(interval: Duration) -> usize {
    (3_600_000u128 / interval.as_millis()) as usize
}

/// Wall-clock time in milliseconds since the Unix epoch.
pub fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct Sampler {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Sampler {
    /// Starts ticking `engine` every `interval`, calling `on_tick` after each
    /// tick with the engine lock already released.
    pub fn spawn<F>(engine: Arc<Mutex<Engine>>, interval: Duration, mut on_tick: F) -> Self
    where
        F: FnMut(&TickOutput) + Send + 'static,
    {
        assert!(!interval.is_zero(), "sampling interval must be positive");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("oma-sampler".into())
            .spawn(move || {
                let epoch = Instant::now();
                let mut deadline = epoch;
                while !stop_flag.load(Ordering::Acquire) {
                    let output = engine.lock().unwrap_or_else(PoisonError::into_inner).tick(unix_ms(), epoch.elapsed().as_millis() as u64);
                    on_tick(&output);
                    deadline = next_deadline(deadline, Instant::now(), interval);
                    while !stop_flag.load(Ordering::Acquire) {
                        let now = Instant::now();
                        if now >= deadline {
                            break;
                        }
                        std::thread::park_timeout(deadline - now);
                    }
                }
            })
            .expect("failed to spawn the sampler thread");
        Self { stop, thread: Some(thread) }
    }

    /// Stops the loop and waits for the current tick to finish.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}
```

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core`
Risultato atteso: tutti i test di `oma-core` OK (quelli di `sampler` compresi).

- [ ] **Step 5: Lint e commit**

```bash
cargo fmt --all
cargo clippy -p oma-core --all-targets -- -D warnings
git add crates/oma-core
git commit -m "feat(core): background sampler with sleep-safe scheduling"
```

---

### Task 6: `oma-win`: wrapper PDH e `CpuProvider`

**File:**
- Modifica: `Cargo.toml` (radice): `members = ["crates/oma-core", "crates/oma-win"]`
- Crea: `crates/oma-win/Cargo.toml`, `crates/oma-win/src/lib.rs`, `crates/oma-win/src/pdh.rs`, `crates/oma-win/src/cpu.rs`
- Test: `crates/oma-win/src/cpu.rs` (unitari), `crates/oma-win/tests/providers.rs` (smoke test su hardware reale)

**Interfacce:**
- Usa: `Provider`, `Inventory`, `ProviderError` (Task 2); `Device`, `DeviceKind`, `Sensor::new`, `Label`, `SensorKind`, `Unit`, `Source` (Task 1).
- Produce:
  - `oma_win::pdh` (interno al crate), con:
    - `Query::open() -> Result<Query, PdhError>`
    - `Query::add_english(&mut self, path: &str) -> Result<Counter, PdhError>`
    - `Query::collect(&mut self) -> Result<(), PdhError>`
    - `Query::value(&self, Counter) -> Option<f64>`
    - `Query::array(&self, Counter) -> Result<Vec<(String, f64)>, PdhError>`, vuoto finché i dati non sono pronti
    - `Query::instances(&self, Counter) -> Result<Vec<String>, PdhError>`, valido dopo un solo `collect`
    - `impl From<PdhError> for ProviderError`
  - `oma_win::cpu::CpuProvider` (con `new()` e `Default`). Device `cpu/0`, sensori in quest'ordine:
    1. `cpu/0/load/total` (`cpu.load.total`)
    2. `cpu/0/load/thread-<g>-<n>` per ogni processore logico, ordinati per (gruppo, numero) (`cpu.load.thread`, con `arg` = indice 0-based)
    3. `cpu/0/clock/effective` (`cpu.clock.effective`, in MHz)
  - `oma_win::default_providers() -> Vec<Box<dyn Provider>>`

- [ ] **Step 1: Crea il crate**

`Cargo.toml` (radice), riga `members`:

```toml
members = ["crates/oma-core", "crates/oma-win"]
```

`crates/oma-win/Cargo.toml`:

```toml
[package]
name = "oma-win"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
oma-core.workspace = true
tracing.workspace = true
sha2 = "0.10"

[target.'cfg(windows)'.dependencies.windows]
version = "0.62"
features = [
  "Win32_Foundation",
  "Win32_Security",
  "Win32_System_IO",
  "Win32_System_Ioctl",
  "Win32_NetworkManagement_IpHelper",
  "Win32_NetworkManagement_Ndis",
  "Win32_Storage_FileSystem",
  "Win32_System_Performance",
  "Win32_System_Registry",
  "Win32_System_SystemInformation",
]
```

`crates/oma-win/src/lib.rs`:

```rust
//! Unprivileged Windows data providers for OpenMonitor Advanced.
#![cfg(windows)]

pub mod cpu;
mod pdh;

use oma_core::provider::Provider;

/// Every unprivileged Windows provider, in display order.
pub fn default_providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(cpu::CpuProvider::new())]
}
```

- [ ] **Step 2: Scrivi i test (falliscono)**

`crates/oma-win/src/cpu.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn lp(group: u16, number: u16) -> LogicalProcessor {
        LogicalProcessor { group, number }
    }

    #[test]
    fn parses_group_and_number() {
        assert_eq!(parse_instance("0,7"), Some(lp(0, 7)));
        assert_eq!(parse_instance("1,63"), Some(lp(1, 63)));
    }

    #[test]
    fn ignores_total_instances() {
        assert_eq!(parse_instance("_Total"), None);
        assert_eq!(parse_instance("0,_Total"), None);
    }

    #[test]
    fn processors_sort_by_group_then_number() {
        let mut v: Vec<_> = ["1,0", "0,63", "0,2"].iter().filter_map(|n| parse_instance(n)).collect();
        v.sort_unstable();
        assert_eq!(v, vec![lp(0, 2), lp(0, 63), lp(1, 0)]);
    }

    #[test]
    fn instance_name_round_trips() {
        assert_eq!(lp(1, 5).instance(), "1,5");
    }

    #[test]
    fn utility_falls_back_to_processor_time() {
        let mut paths = Vec::new();
        let value = add_load_counter(|path| {
            paths.push(path.to_owned());
            if path == UTILITY { Err("missing utility") } else { Ok(42) }
        });
        assert_eq!(value, Ok(42));
        assert_eq!(paths, vec![UTILITY, TIME]);
        assert!(add_load_counter::<(), _>(|_| Err("missing both")).is_err());
    }

    #[test]
    fn effective_clock_scales_nominal_frequency() {
        let mhz = effective_clock_mhz(4201.0, 104.35);
        assert!((mhz - 4383.74).abs() < 0.01, "{mhz}");
    }

    #[test]
    fn load_is_capped_at_100() {
        assert_eq!(load_pct(104.0), Some(100.0));
        assert_eq!(load_pct(-1.0), None);
        assert_eq!(load_pct(f64::NAN), None);
    }
}
```

`crates/oma-win/tests/providers.rs` (smoke test: parlano con Windows vero, sono `#[ignore]` in CI (§12) e si eseguono esplicitamente sulla macchina di sviluppo in italiano):

```rust
#![cfg(windows)]

use std::time::Duration;

use oma_core::provider::{Inventory, Provider};
use oma_win::cpu::CpuProvider;

/// Discovers, waits for a second PDH sample, polls, and checks alignment.
fn discover_and_poll(p: &mut dyn Provider) -> (Inventory, Vec<Option<f64>>) {
    let inventory = p.discover().expect("discover");
    std::thread::sleep(Duration::from_millis(1_100));
    let values = p.poll().expect("poll");
    assert_eq!(values.len(), inventory.sensors.len(), "values must align with sensors");
    (inventory, values)
}

#[test]
#[ignore = "requires real Windows hardware"]
fn pdh_english_paths_resolve() {
    // Discovery adds every counter with PdhAddEnglishCounterW: on a non-English
    // Windows this fails if a localized API is used by mistake.
    CpuProvider::new().discover().expect("english PDH counter paths must resolve");
}

#[test]
#[ignore = "requires real Windows hardware"]
fn cpu_provider_reports_load_and_clock() {
    let mut p = CpuProvider::new();
    let (inventory, values) = discover_and_poll(&mut p);
    assert_eq!(inventory.devices.len(), 1);
    assert!(!inventory.devices[0].name.is_empty());
    let thread_sensors = inventory.sensors.iter().filter(|s| s.id.contains("/load/thread-")).count();
    assert!(thread_sensors > 0); // available_parallelism may be restricted by affinity/job limits.
    let total = values[0].expect("total load");
    assert!((0.0..=100.0).contains(&total));
    if let Some(clock) = values.last().copied().flatten() {
        assert!((0.0..=20_000.0).contains(&clock), "clock {clock} MHz");
    }
}
```

- [ ] **Step 3: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-win` (unitari); su hardware reale: `cargo test -p oma-win --test providers -- --ignored`
Risultato atteso: errore di compilazione (`file not found for module cpu` / `pdh`).

- [ ] **Step 4: Implementa il wrapper PDH**

`crates/oma-win/src/pdh.rs`:

```rust
//! Minimal safe wrapper over the Performance Data Helper (PDH) API.

use std::fmt;

use oma_core::provider::ProviderError;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhGetRawCounterArrayW, PdhOpenQueryW, PDH_CSTATUS_VALID_DATA, PDH_CSTATUS_NEW_DATA, PDH_FMT,
    PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
    PDH_RAW_COUNTER_ITEM_W,
};

/// `PDH_FMT_DOUBLE | PDH_FMT_NOCAP100`: windows-rs 0.62 does not export
/// `PDH_FMT_NOCAP100` (0x8000).
const FMT_DOUBLE_NOCAP: PDH_FMT = PDH_FMT(PDH_FMT_DOUBLE.0 | 0x8000);
/// Returned while a rate counter has fewer than two samples.
const PDH_INVALID_DATA: u32 = 0xC000_0BBA;
/// Returned when a wildcard counter currently has no instances.
const PDH_NO_DATA: u32 = 0x8000_07D5;
/// Instances can appear between the size query and the read; retry a few times.
const MAX_ARRAY_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdhError {
    pub call: &'static str,
    pub status: u32,
}

impl fmt::Display for PdhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} failed with PDH status {:#010x}", self.call, self.status)
    }
}

impl std::error::Error for PdhError {}

impl From<PdhError> for ProviderError {
    fn from(e: PdhError) -> Self {
        ProviderError::Failed(e.to_string())
    }
}

fn check(call: &'static str, status: u32) -> Result<(), PdhError> {
    if status == 0 {
        Ok(())
    } else {
        Err(PdhError { call, status })
    }
}

fn valid_status(status: u32) -> bool {
    matches!(status, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_new_and_unchanged_data_only() {
        assert!(valid_status(PDH_CSTATUS_VALID_DATA));
        assert!(valid_status(PDH_CSTATUS_NEW_DATA));
        assert!(!valid_status(PDH_INVALID_DATA));
    }
}

pub struct Query {
    handle: PDH_HQUERY,
}

// SAFETY: a PDH query handle is not bound to the creating thread, and a Query
// is only used through `&mut`/`&` by the provider that owns it.
unsafe impl Send for Query {}

#[derive(Debug, Clone, Copy)]
pub struct Counter(PDH_HCOUNTER);

// SAFETY: counter handles are plain identifiers owned by their Query.
unsafe impl Send for Counter {}

impl Query {
    pub fn open() -> Result<Self, PdhError> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: valid out-pointer; a null data source means live data.
        check("PdhOpenQueryW", unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) })?;
        Ok(Self { handle })
    }

    /// Adds a counter by its English path, so it resolves on every Windows
    /// display language (localized names differ, e.g. on Italian Windows).
    pub fn add_english(&mut self, path: &str) -> Result<Counter, PdhError> {
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: the query handle is open and the out-pointer is valid.
        let status = unsafe { PdhAddEnglishCounterW(self.handle, &HSTRING::from(path), 0, &mut counter) };
        check("PdhAddEnglishCounterW", status)?;
        Ok(Counter(counter))
    }

    pub fn collect(&mut self) -> Result<(), PdhError> {
        // SAFETY: the query handle is open.
        check("PdhCollectQueryData", unsafe { PdhCollectQueryData(self.handle) })
    }

    /// Formatted value of a single-instance counter; `None` until two samples
    /// exist or when PDH marks the value invalid.
    pub fn value(&self, counter: Counter) -> Option<f64> {
        let mut value = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: the counter belongs to this query; the out-pointer is valid.
        let status = unsafe { PdhGetFormattedCounterValue(counter.0, FMT_DOUBLE_NOCAP, None, &mut value) };
        // SAFETY: PDH_FMT_DOUBLE fills the `doubleValue` union member.
        if status == 0 && valid_status(value.CStatus) {
            Some(unsafe { value.Anonymous.doubleValue })
        } else {
            None
        }
    }

    /// Formatted values of a wildcard counter as `(instance, value)`. Empty
    /// while no data is available yet; invalid instance values are NaN.
    pub fn array(&self, counter: Counter) -> Result<Vec<(String, f64)>, PdhError> {
        const CALL: &str = "PdhGetFormattedCounterArrayW";
        for _ in 0..MAX_ARRAY_ATTEMPTS {
            let (mut size, mut count) = (0u32, 0u32);
            // SAFETY: a size query with a null buffer.
            let status =
                unsafe { PdhGetFormattedCounterArrayW(counter.0, FMT_DOUBLE_NOCAP, &mut size, &mut count, None) };
            match status {
                PDH_MORE_DATA => {}
                PDH_NO_DATA | PDH_INVALID_DATA => return Ok(Vec::new()),
                other => return Err(PdhError { call: CALL, status: other }),
            }
            // u64 storage keeps the items 8-byte aligned.
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
            // SAFETY: `buffer` holds at least `size` bytes.
            let status = unsafe {
                PdhGetFormattedCounterArrayW(counter.0, FMT_DOUBLE_NOCAP, &mut size, &mut count, Some(items))
            };
            match status {
                0 => {}
                PDH_MORE_DATA => continue,
                PDH_NO_DATA | PDH_INVALID_DATA => return Ok(Vec::new()),
                other => return Err(PdhError { call: CALL, status: other }),
            }
            // SAFETY: PDH wrote `count` items into `buffer`, which outlives the slice.
            let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
            return Ok(items
                .iter()
                .map(|item| {
                    // SAFETY: `szName` points into `buffer`.
                    let name = unsafe { item.szName.to_string() }.unwrap_or_default();
                    let value = if valid_status(item.FmtValue.CStatus) {
                        // SAFETY: PDH_FMT_DOUBLE fills the `doubleValue` union member.
                        unsafe { item.FmtValue.Anonymous.doubleValue }
                    } else {
                        f64::NAN
                    };
                    (name, value)
                })
                .collect());
        }
        Err(PdhError { call: CALL, status: PDH_MORE_DATA })
    }

    /// Instance names of a wildcard counter. Unlike `array`, this works right
    /// after the first `collect`, so discovery does not have to wait.
    pub fn instances(&self, counter: Counter) -> Result<Vec<String>, PdhError> {
        const CALL: &str = "PdhGetRawCounterArrayW";
        for _ in 0..MAX_ARRAY_ATTEMPTS {
            let (mut size, mut count) = (0u32, 0u32);
            // SAFETY: a size query with a null buffer.
            let status = unsafe { PdhGetRawCounterArrayW(counter.0, &mut size, &mut count, None) };
            match status {
                PDH_MORE_DATA => {}
                PDH_NO_DATA => return Ok(Vec::new()),
                other => return Err(PdhError { call: CALL, status: other }),
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let items = buffer.as_mut_ptr().cast::<PDH_RAW_COUNTER_ITEM_W>();
            // SAFETY: `buffer` holds at least `size` bytes.
            let status = unsafe { PdhGetRawCounterArrayW(counter.0, &mut size, &mut count, Some(items)) };
            match status {
                0 => {}
                PDH_MORE_DATA => continue,
                PDH_NO_DATA => return Ok(Vec::new()),
                other => return Err(PdhError { call: CALL, status: other }),
            }
            // SAFETY: PDH wrote `count` items into `buffer`, which outlives the slice.
            let items = unsafe { std::slice::from_raw_parts(items, count as usize) };
            // SAFETY: `szName` points into `buffer`.
            return Ok(items.iter().map(|item| unsafe { item.szName.to_string() }.unwrap_or_default()).collect());
        }
        Err(PdhError { call: CALL, status: PDH_MORE_DATA })
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: the handle is open and not used after this point.
        unsafe {
            let _ = PdhCloseQuery(self.handle);
        }
    }
}
```

- [ ] **Step 5: Implementa `CpuProvider`**

In testa a `crates/oma-win/src/cpu.rs`, prima del modulo `tests`:

```rust
//! CPU load and effective clock from PDH "Processor Information" counters.

use std::collections::HashMap;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::core::w;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

use crate::pdh::{Counter, Query};

const DEVICE_ID: &str = "cpu/0";
const UTILITY: &str = r"\Processor Information(*)\% Processor Utility";
const TIME: &str = r"\Processor Information(*)\% Processor Time";
const PERFORMANCE: &str = r"\Processor Information(_Total)\% Processor Performance";
const FREQUENCY: &str = r"\Processor Information(_Total)\Processor Frequency";
const TOTAL_INSTANCE: &str = "_Total";

/// A "Processor Information" instance such as "0,7" (group 0, processor 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct LogicalProcessor {
    pub group: u16,
    pub number: u16,
}

impl LogicalProcessor {
    pub(crate) fn instance(self) -> String {
        format!("{},{}", self.group, self.number)
    }
}

/// Parses "group,number"; `None` for "_Total" and per-group "N,_Total".
pub(crate) fn parse_instance(name: &str) -> Option<LogicalProcessor> {
    let (group, number) = name.split_once(',')?;
    Some(LogicalProcessor { group: group.trim().parse().ok()?, number: number.trim().parse().ok()? })
}

fn add_load_counter<T, E>(mut add: impl FnMut(&str) -> Result<T, E>) -> Result<T, E> {
    add(UTILITY).or_else(|_| add(TIME))
}

/// Task Manager's estimated clock: nominal frequency × % performance.
pub(crate) fn effective_clock_mhz(nominal_mhz: f64, performance_pct: f64) -> f64 {
    nominal_mhz * performance_pct / 100.0
}

/// Processor Utility exceeds 100 % while boosting; Task Manager caps it and so do we.
pub(crate) fn load_pct(utility: f64) -> Option<f64> {
    (utility.is_finite() && utility >= 0.0).then(|| utility.min(100.0))
}

fn cpu_name() -> String {
    let mut buffer = [0u16; 256];
    let mut bytes = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: `buffer` and `bytes` describe writable memory of that size.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0"),
            w!("ProcessorNameString"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return "CPU".to_owned();
    }
    let chars = (bytes as usize / 2).saturating_sub(1); // drop the terminating NUL
    let name = String::from_utf16_lossy(&buffer[..chars]).trim().to_owned();
    if name.is_empty() {
        "CPU".to_owned()
    } else {
        name
    }
}

struct Counters {
    query: Query,
    utility: Counter,
    performance: Option<Counter>,
    frequency: Option<Counter>,
}

#[derive(Default)]
pub struct CpuProvider {
    counters: Option<Counters>,
    processors: Vec<LogicalProcessor>,
}

impl CpuProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Provider for CpuProvider {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        let mut query = Query::open()?;
        let utility = add_load_counter(|path| query.add_english(path))?;
        let performance = query.add_english(PERFORMANCE).ok();
        let frequency = query.add_english(FREQUENCY).ok();
        query.collect()?;
        let mut processors: Vec<_> = query.instances(utility)?.iter().filter_map(|n| parse_instance(n)).collect();
        processors.sort_unstable();

        let mut sensors = vec![Sensor::new(
            DEVICE_ID,
            SensorKind::Load,
            "total",
            Unit::Percent,
            Label::new("cpu.load.total"),
            Source::Pdh,
        )];
        sensors.extend(processors.iter().enumerate().map(|(index, p)| {
            Sensor::new(
                DEVICE_ID,
                SensorKind::Load,
                &format!("thread-{}-{}", p.group, p.number),
                Unit::Percent,
                Label::with_arg("cpu.load.thread", index.to_string()),
                Source::Pdh,
            )
        }));
        sensors.push(Sensor::new(
            DEVICE_ID,
            SensorKind::Clock,
            "effective",
            Unit::Megahertz,
            Label::new("cpu.clock.effective"),
            Source::Pdh,
        ));

        self.counters = Some(Counters { query, utility, performance, frequency });
        self.processors = processors;
        Ok(Inventory {
            devices: vec![Device { id: DEVICE_ID.to_owned(), kind: DeviceKind::Cpu, name: cpu_name(), vendor: None, properties: Default::default() }],
            sensors,
        })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let counters = self.counters.as_mut().ok_or(ProviderError::Rediscover)?;
        counters.query.collect()?;
        let utility = counters.query.array(counters.utility)?;
        if !utility.is_empty() {
            let seen = utility.iter().filter(|(name, _)| parse_instance(name).is_some()).count();
            if seen != self.processors.len() {
                return Err(ProviderError::Rediscover);
            }
        }
        let by_instance: HashMap<&str, f64> = utility.iter().map(|(n, v)| (n.as_str(), *v)).collect();

        let mut values = Vec::with_capacity(self.processors.len() + 2);
        values.push(by_instance.get(TOTAL_INSTANCE).copied().and_then(load_pct));
        values.extend(
            self.processors.iter().map(|p| by_instance.get(p.instance().as_str()).copied().and_then(load_pct)),
        );
        let clock = match (counters.frequency.and_then(|c| counters.query.value(c)), counters.performance.and_then(|c| counters.query.value(c))) {
            (Some(nominal), Some(performance)) => Some(effective_clock_mhz(nominal, performance)),
            _ => None,
        };
        values.push(clock);
        Ok(values)
    }
}
```

- [ ] **Step 6: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-win` (unitari); su hardware reale: `cargo test -p oma-win --test providers -- --ignored`
Risultato atteso: test unitari PASS; i test `providers.rs` restano ignorati nel comando ordinario e devono passare nella sessione hardware esplicita. Esegui anche sulla macchina di sviluppo con Windows in italiano: `pdh_english_paths_resolve` deve passare.

- [ ] **Step 7: Lint e commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add Cargo.toml Cargo.lock crates/oma-win
git commit -m "feat(win): PDH wrapper and CPU load/clock provider"
```

---

### Task 7: `MemoryProvider`

**File:**
- Crea: `crates/oma-win/src/memory.rs`
- Modifica: `crates/oma-win/src/lib.rs`, `crates/oma-win/tests/providers.rs`

**Interfacce:**
- Usa: gli stessi tipi di `oma-core` del Task 6.
- Produce: `oma_win::memory::MemoryProvider` (con `Default`). Device `memory/0` con nome `"RAM"` e sensori, in quest'ordine:
  1. `memory/0/load/used` (Percent, `memory.load`)
  2. `memory/0/data/used` (Bytes, `memory.used`)
  3. `memory/0/data/total` (Bytes, `memory.total`)

- [ ] **Step 1: Scrivi i test (falliscono)**

`crates/oma-win/src/memory.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn used_is_total_minus_available() {
        assert_eq!(used_bytes(32, 8), 24);
        assert_eq!(used_bytes(8, 32), 0);
    }

    #[test]
    fn used_percentage() {
        assert_eq!(used_pct(32, 8), Some(75.0));
        assert_eq!(used_pct(0, 0), None);
    }
}
```

In `crates/oma-win/tests/providers.rs` aggiungi `use oma_win::memory::MemoryProvider;` agli import e il test:

```rust
#[test]
#[ignore = "requires real Windows hardware"]
fn memory_provider_reports_usage() {
    let mut p = MemoryProvider;
    let (_, values) = discover_and_poll(&mut p);
    let pct = values[0].expect("load");
    assert!((0.0..=100.0).contains(&pct));
    let used = values[1].expect("used");
    let total = values[2].expect("total");
    assert!(total > 0.0 && used <= total);
}
```

In `crates/oma-win/src/lib.rs` aggiungi `pub mod memory;` (dopo `pub mod cpu;`) e il provider:

```rust
pub fn default_providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(cpu::CpuProvider::new()), Box::new(memory::MemoryProvider)]
}
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-win` (unitari); su hardware reale: `cargo test -p oma-win --test providers -- --ignored`
Risultato atteso: errore di compilazione (`file not found for module memory`).

- [ ] **Step 3: Implementa**

In testa a `crates/oma-win/src/memory.rs`:

```rust
//! Physical memory usage from GlobalMemoryStatusEx.

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

const DEVICE_ID: &str = "memory/0";

pub(crate) fn used_bytes(total: u64, available: u64) -> u64 {
    total.saturating_sub(available)
}

pub(crate) fn used_pct(total: u64, available: u64) -> Option<f64> {
    (total > 0).then(|| used_bytes(total, available) as f64 * 100.0 / total as f64)
}

fn memory_status() -> Result<MEMORYSTATUSEX, ProviderError> {
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `dwLength` is set as the API requires.
    unsafe { GlobalMemoryStatusEx(&mut status) }
        .map_err(|e| ProviderError::Failed(format!("GlobalMemoryStatusEx: {e}")))?;
    Ok(status)
}

#[derive(Default)]
pub struct MemoryProvider;

impl Provider for MemoryProvider {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        memory_status()?;
        Ok(Inventory {
            devices: vec![Device { id: DEVICE_ID.to_owned(), kind: DeviceKind::Memory, name: "RAM".to_owned(), vendor: None, properties: Default::default() }],
            sensors: vec![
                Sensor::new(DEVICE_ID, SensorKind::Load, "used", Unit::Percent, Label::new("memory.load"), Source::Win32),
                Sensor::new(DEVICE_ID, SensorKind::Data, "used", Unit::Bytes, Label::new("memory.used"), Source::Win32),
                Sensor::new(DEVICE_ID, SensorKind::Data, "total", Unit::Bytes, Label::new("memory.total"), Source::Win32),
            ],
        })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let s = memory_status()?;
        Ok(vec![
            used_pct(s.ullTotalPhys, s.ullAvailPhys),
            Some(used_bytes(s.ullTotalPhys, s.ullAvailPhys) as f64),
            Some(s.ullTotalPhys as f64),
        ])
    }
}
```

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-win` (unitari); su hardware reale: `cargo test -p oma-win --test providers -- --ignored`
Risultato atteso: tutti OK (compresi `used_*` e `memory_provider_reports_usage`).

- [ ] **Step 5: Lint e commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add crates/oma-win
git commit -m "feat(win): memory usage provider"
```

---

### Task 8: `StorageProvider`

**File:**
- Crea: `crates/oma-win/src/storage.rs`, `crates/oma-win/src/storage_identity.rs`
- Modifica: `crates/oma-win/src/lib.rs`, `crates/oma-win/tests/providers.rs`

**Interfacce:**
- Usa: `crate::pdh::{Query, Counter}` (Task 6) e i tipi di `oma-core`.
- Produce: `oma_win::storage::StorageProvider` (con `Default`).
  - Un device per disco fisico identificabile, `storage/device-<sha256>`, con nome `"Disk <n> (C:, D:)"`, oppure `"Disk <n>"` se il disco non ha volumi.
  - Sensori per disco, in quest'ordine:
    1. `throughput/read` (BytesPerSecond, `storage.read`)
    2. `throughput/write` (BytesPerSecond, `storage.write`)
    3. `load/active` (Percent, `storage.active`)
  - Per ogni volume con lettera, due sensori in più:
    - `percent/volume-<guid>` (Percent, `storage.volumeUsed`, `arg` = `"C:"`)
    - `data/volume-<guid>-free` (Bytes, `storage.volumeFree`, `arg` = `"C:"`)

**Nota:** le lettere PDH servono solo a trovare i mount point attuali. Anche un disco locale può bloccare una chiamata Win32: il timeout del Task 4 è obbligatorio. Identità persistenti di dischi e volumi: Step 3a, senza usare indice PDH o lettera come ID.

- [ ] **Step 1: Scrivi i test (falliscono)**

`crates/oma-win/src/storage.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_disk_with_one_volume() {
        let d = parse_disk_instance("2 C:").unwrap();
        assert_eq!(d.index, 2);
        assert_eq!(d.volumes, vec!["C:".to_string()]);
    }

    #[test]
    fn parses_disk_with_several_or_no_volumes() {
        assert_eq!(parse_disk_instance("0 C: D:").unwrap().volumes, vec!["C:", "D:"]);
        assert!(parse_disk_instance("1").unwrap().volumes.is_empty());
        assert_eq!(parse_disk_instance("3 e:").unwrap().volumes, vec!["E:"]);
    }

    #[test]
    fn total_instance_is_not_a_disk() {
        assert_eq!(parse_disk_instance("_Total"), None);
    }

    #[test]
    fn disks_are_sorted_by_index() {
        let disks = disk_instances(&["2 C:".into(), "_Total".into(), "0 D:".into()]);
        assert_eq!(disks.iter().map(|d| d.index).collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn detects_disk_set_changes() {
        let known = disk_instances(&["0 C:".into()]);
        assert!(!disks_changed(&known, &["0 C:".into(), "_Total".into()]));
        assert!(disks_changed(&known, &["0 C:".into(), "1 E:".into(), "_Total".into()]));
    }

    #[test]
    fn active_time_is_the_complement_of_idle() {
        assert!((active_pct(99.9).unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(active_pct(120.0), Some(0.0));
        assert_eq!(active_pct(f64::NAN), None);
    }

    #[test]
    fn volume_usage() {
        assert_eq!(used_pct(200, 50), Some(75.0));
        assert_eq!(used_pct(0, 0), None);
    }

    #[test]
    fn disk_names_list_volumes() {
        assert_eq!(disk_name(&parse_disk_instance("0 C: D:").unwrap()), "Disk 0 (C:, D:)");
        assert_eq!(disk_name(&parse_disk_instance("1").unwrap()), "Disk 1");
    }
}
```

In `crates/oma-win/tests/providers.rs` aggiungi `use oma_win::storage::StorageProvider;` e:

```rust
#[test]
#[ignore = "requires real Windows hardware"]
fn storage_provider_reports_disks_and_volumes() {
    let mut p = StorageProvider::default();
    let (inventory, values) = discover_and_poll(&mut p);
    assert!(!inventory.devices.is_empty(), "at least the system disk");
    for (sensor, value) in inventory.sensors.iter().zip(&values) {
        if sensor.label.key == "storage.volumeUsed" {
            let pct = value.expect("volume usage");
            assert!((0.0..=100.0).contains(&pct), "{} = {pct}", sensor.id);
        }
    }
}
```

In `crates/oma-win/src/lib.rs` aggiungi `pub mod storage;` e `mod storage_identity;` e il provider in coda a `default_providers()`:

```rust
pub fn default_providers() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(cpu::CpuProvider::new()),
        Box::new(memory::MemoryProvider),
        Box::new(storage::StorageProvider::default()),
    ]
}
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-win` (unitari); su hardware reale: `cargo test -p oma-win --test providers -- --ignored`
Risultato atteso: errore di compilazione (`file not found for module storage`).

- [ ] **Step 3: Implementa**

In testa a `crates/oma-win/src/storage.rs`:

```rust
//! Physical disk throughput and activity (PDH) plus volume usage.

use std::collections::HashMap;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::pdh::{Counter, Query};
use crate::storage_identity::{disk_identity, volume_identity};

const READ: &str = r"\PhysicalDisk(*)\Disk Read Bytes/sec";
const WRITE: &str = r"\PhysicalDisk(*)\Disk Write Bytes/sec";
const IDLE: &str = r"\PhysicalDisk(*)\% Idle Time";

/// A "PhysicalDisk" instance such as "2 C: D:" (disk 2 holding C: and D:).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiskInstance {
    pub instance: String,
    pub index: u32,
    pub volumes: Vec<String>,
}

/// `None` for "_Total".
pub(crate) fn parse_disk_instance(name: &str) -> Option<DiskInstance> {
    let mut parts = name.split_whitespace();
    let index = parts.next()?.parse().ok()?;
    let volumes = parts
        .filter(|p| p.len() == 2 && p.ends_with(':') && p.as_bytes()[0].is_ascii_alphabetic())
        .map(|p| p.to_ascii_uppercase())
        .collect();
    Some(DiskInstance { instance: name.to_owned(), index, volumes })
}

pub(crate) fn disk_instances(names: &[String]) -> Vec<DiskInstance> {
    let mut disks: Vec<_> = names.iter().filter_map(|n| parse_disk_instance(n)).collect();
    disks.sort_by_key(|d| d.index);
    disks
}

pub(crate) fn disks_changed(known: &[DiskInstance], names: &[String]) -> bool {
    disk_instances(names) != known
}

pub(crate) fn disk_name(disk: &DiskInstance) -> String {
    if disk.volumes.is_empty() {
        format!("Disk {}", disk.index)
    } else {
        format!("Disk {} ({})", disk.index, disk.volumes.join(", "))
    }
}

pub(crate) fn active_pct(idle: f64) -> Option<f64> {
    idle.is_finite().then(|| (100.0 - idle).clamp(0.0, 100.0))
}

pub(crate) fn used_pct(total: u64, free: u64) -> Option<f64> {
    (total > 0).then(|| total.saturating_sub(free) as f64 * 100.0 / total as f64)
}

/// `(total, free)` bytes of a volume such as "C:"; `None` if unavailable.
fn volume_space(volume: &str) -> Option<(u64, u64)> {
    let root = HSTRING::from(format!("{volume}\\"));
    let (mut total, mut free) = (0u64, 0u64);
    // SAFETY: valid root path and out-pointers.
    unsafe { GetDiskFreeSpaceExW(&root, None, Some(&mut total), Some(&mut free)) }.ok()?;
    Some((total, free))
}

struct Counters {
    query: Query,
    read: Counter,
    write: Counter,
    idle: Counter,
}

#[derive(Default)]
pub struct StorageProvider {
    counters: Option<Counters>,
    disks: Vec<DiskInstance>,
    disk_ids: HashMap<u32, String>,
    volume_ids: HashMap<String, String>,
}

impl Provider for StorageProvider {
    fn name(&self) -> &'static str {
        "storage"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        let mut query = Query::open()?;
        let read = query.add_english(READ)?;
        let write = query.add_english(WRITE)?;
        let idle = query.add_english(IDLE)?;
        query.collect()?;
        let disks = disk_instances(&query.instances(read)?);

        // Resolve stable identities only during discovery; never persist PDH indices.
        let mut disk_ids: HashMap<u32, String> = disks.iter()
            .filter_map(|d| disk_identity(d.index).map(|id| (d.index, id))).collect();
        let mut counts = HashMap::<String, usize>::new();
        for id in disk_ids.values() { *counts.entry(id.clone()).or_default() += 1; }
        disk_ids.retain(|_, id| counts[id] == 1); // Ambiguous serials must not merge disks.
        let volume_ids: HashMap<String, String> = disks.iter().flat_map(|d| &d.volumes)
            .filter_map(|v| volume_identity(v).map(|id| (v.clone(), id))).collect();
        let mut devices = Vec::new();
        let mut sensors = Vec::new();
        for disk in &disks {
            let Some(id) = disk_ids.get(&disk.index).cloned() else {
                tracing::warn!(index = disk.index, "disk has no unique persistent identity; omitted");
                continue;
            };
            devices.push(Device { id: id.clone(), kind: DeviceKind::Storage, name: disk_name(disk), vendor: None, properties: Default::default() });
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "read",
                Unit::BytesPerSecond,
                Label::new("storage.read"),
                Source::Pdh,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "write",
                Unit::BytesPerSecond,
                Label::new("storage.write"),
                Source::Pdh,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Load,
                "active",
                Unit::Percent,
                Label::new("storage.active"),
                Source::Pdh,
            ));
            for volume in &disk.volumes {
                let Some(volume_id) = volume_ids.get(volume) else { continue; };
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Percent,
                    &format!("volume-{}", volume_id),
                    Unit::Percent,
                    Label::with_arg("storage.volumeUsed", volume.clone()),
                    Source::Win32,
                ));
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Data,
                    &format!("volume-{}-free", volume_id),
                    Unit::Bytes,
                    Label::with_arg("storage.volumeFree", volume.clone()),
                    Source::Win32,
                ));
            }
        }
        self.counters = Some(Counters { query, read, write, idle });
        self.disks = disks;
        self.disk_ids = disk_ids;
        self.volume_ids = volume_ids;
        Ok(Inventory { devices, sensors })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let counters = self.counters.as_mut().ok_or(ProviderError::Rediscover)?;
        counters.query.collect()?;
        let read = counters.query.array(counters.read)?;
        // Raw instances distinguish a missing disk from rate-counter warm-up.
        if disks_changed(&self.disks, &counters.query.instances(counters.read)?) {
            return Err(ProviderError::Rediscover);
        }
        let read: HashMap<String, f64> = read.into_iter().collect();
        let write: HashMap<String, f64> = counters.query.array(counters.write)?.into_iter().collect();
        let idle: HashMap<String, f64> = counters.query.array(counters.idle)?.into_iter().collect();
        let finite = |map: &HashMap<String, f64>, key: &str| map.get(key).copied().filter(|v| v.is_finite());

        let mut values = Vec::new();
        for disk in &self.disks {
            if !self.disk_ids.contains_key(&disk.index) { continue; }
            values.push(finite(&read, &disk.instance));
            values.push(finite(&write, &disk.instance));
            values.push(finite(&idle, &disk.instance).and_then(active_pct));
            for volume in &disk.volumes {
                if !self.volume_ids.contains_key(volume) { continue; }
                match volume_space(volume) {
                    Some((total, free)) => {
                        values.push(used_pct(total, free));
                        values.push(Some(free as f64));
                    }
                    None => values.extend([None, None]),
                }
            }
        }
        Ok(values)
    }
}
```


- [ ] **Step 3a: Identità persistenti (§3), prima di compilare il provider**

Il numero `PhysicalDriveN` serve solo ad aprire il dispositivo durante la discovery.
L'ID è un SHA-256 di vendor, modello e seriale, con separatori; la lettera del volume
è solo un'etichetta, mentre il suffisso dei sensori usa il GUID del volume. Un seriale
assente, non leggibile o duplicato non viene sostituito con un indice instabile:
si omette quel disco e si registra la causa nel log. Verificare sulla matrice hardware
che l'utente standard possa leggere il descrittore; il supporto a identità alternative
per controller senza seriale richiede un'estensione esplicita, non un falso ID stabile.

`crates/oma-win/src/storage_identity.rs`:

```rust
//! Persistent identities; raw serial numbers never leave this module.
use sha2::{Digest, Sha256};
use windows::core::HSTRING;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{CreateFileW, GetVolumeNameForVolumeMountPointW,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_PROPERTY_QUERY,
    StorageDeviceProperty, PropertyStandardQuery};

fn descriptor_text(bytes: &[u8], field: usize) -> Option<&str> {
    let offset = u32::from_le_bytes(bytes.get(field..field + 4)?.try_into().ok()?) as usize;
    if offset < 36 { return None; }
    let tail = bytes.get(offset..)?;
    let end = tail.iter().position(|&b| b == 0)?;
    let text = std::str::from_utf8(&tail[..end]).ok()?.trim();
    (!text.is_empty()).then_some(text)
}

fn identity_from_descriptor(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 36 { return None; }
    let serial = descriptor_text(bytes, 24)?;
    let vendor = descriptor_text(bytes, 12).unwrap_or("");
    let model = descriptor_text(bytes, 16).unwrap_or("");
    let hash = Sha256::digest(format!("{vendor}\0{model}\0{serial}").as_bytes());
    Some(format!("storage/device-{hash:x}"))
}

pub(crate) fn disk_identity(index: u32) -> Option<String> {
    let path = HSTRING::from(format!(r"\\.\PhysicalDrive{index}"));
    // SAFETY: valid path; zero desired access only queries metadata, never writes.
    let handle = unsafe { CreateFileW(&path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE,
        None, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, None) }.ok()?;
    let query = STORAGE_PROPERTY_QUERY { PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery, ..Default::default() };
    let mut bytes = vec![0u8; 65_536];
    let mut returned = 0u32;
    // SAFETY: both buffers and the returned-size pointer are valid for the call.
    let result = unsafe { DeviceIoControl(handle, IOCTL_STORAGE_QUERY_PROPERTY,
        Some((&query as *const STORAGE_PROPERTY_QUERY).cast()), std::mem::size_of_val(&query) as u32,
        Some(bytes.as_mut_ptr().cast()), bytes.len() as u32, Some(&mut returned), None) };
    // SAFETY: sole owned handle, no longer used after this call.
    unsafe { let _ = CloseHandle(handle); }
    result.ok()?;
    if returned as usize > bytes.len() { return None; }
    bytes.truncate(returned as usize);
    identity_from_descriptor(&bytes)
}

pub(crate) fn volume_identity(letter: &str) -> Option<String> {
    let mut buffer = [0u16; 64];
    let root = HSTRING::from(format!("{letter}\\"));
    // SAFETY: valid mount point and output buffer.
    unsafe { GetVolumeNameForVolumeMountPointW(&root, &mut buffer) }.ok()?;
    let end = buffer.iter().position(|&v| v == 0)?;
    let name = String::from_utf16_lossy(&buffer[..end]).to_ascii_lowercase();
    Some(name.strip_prefix(r"\\?\volume{")?.strip_suffix("}\\")?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor(serial: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 36];
        bytes[24..28].copy_from_slice(&36u32.to_le_bytes());
        bytes.extend_from_slice(serial);
        bytes.push(0);
        bytes
    }
    #[test]
    fn identity_depends_on_hardware_not_disk_number() {
        let serial = descriptor(b"serial-a");
        assert_eq!(identity_from_descriptor(&serial), identity_from_descriptor(&serial));
        assert_ne!(identity_from_descriptor(&serial), identity_from_descriptor(&descriptor(b"serial-b")));
        assert!(!identity_from_descriptor(&serial).unwrap().contains("serial-a"));
    }
    #[test]
    fn missing_or_malformed_serial_has_no_identity() {
        assert!(identity_from_descriptor(&descriptor(b"")).is_none());
        assert!(identity_from_descriptor(&[0; 36]).is_none());
        let mut bytes = descriptor(b"abc");
        bytes[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(identity_from_descriptor(&bytes).is_none());
    }
}
```

Test hardware aggiuntivo: annotare ID, scollegare/ricollegare il disco USB cambiando
indice e lettera, e verificare gli stessi ID per lo stesso hardware e volume.
Una sostituzione con altro hardware non deve recuperare lo storico del disco precedente.

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-win` (unitari); su hardware reale: `cargo test -p oma-win --test providers -- --ignored`
Risultato atteso: tutti OK.

- [ ] **Step 5: Lint e commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add crates/oma-win
git commit -m "feat(win): disk throughput, activity and volume usage provider"
```

---

### Task 9: `NetworkProvider`

**File:**
- Crea: `crates/oma-win/src/network.rs`
- Modifica: `crates/oma-win/src/lib.rs`, `crates/oma-win/tests/providers.rs`

**Interfacce:**
- Usa: `oma_core::rate::CounterRate` (Task 2) e i tipi di `oma-core`.
- Produce: `oma_win::network::NetworkProvider` (con `Default`).
  - Un device per scheda monitorata, `network/<guid minuscolo>`, con nome uguale all'alias di Windows (per esempio "Ethernet").
  - Sensori per scheda, in quest'ordine:
    1. `throughput/down` (BytesPerSecond, `network.down`)
    2. `throughput/up` (BytesPerSecond, `network.up`)
    3. `throughput/link-speed` (BitsPerSecond, `network.linkSpeed`)
  - Le schede sono ordinate per alias e poi per GUID.

**Filtro (spec §5.1: "interfacce fisiche e attive"):**
- tipo Ethernet (6) o Wi-Fi (71);
- flag `HardwareInterface` (bit 0) impostato;
- flag `FilterInterface` (bit 1) assente, per escludere i duplicati NDIS LWF;
- `OperStatus == Up`.

Se l'insieme delle schede monitorate cambia (cavo collegato o scollegato, Wi-Fi acceso o spento) il provider restituisce `Rediscover`.

- [ ] **Step 1: Scrivi i test (falliscono)**

`crates/oma-win/src/network.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn row(guid: &str, if_type: u32, flags: u8, up: bool) -> InterfaceRow {
        InterfaceRow {
            guid: guid.into(),
            alias: guid.to_uppercase(),
            if_type,
            flags,
            up,
            in_octets: 0,
            out_octets: 0,
            link_bps: 1_000_000_000,
        }
    }

    #[test]
    fn monitors_connected_physical_ethernet_and_wifi() {
        assert!(is_monitored(&row("a", 6, 0b01, true)));
        assert!(is_monitored(&row("b", 71, 0b01, true)));
    }

    #[test]
    fn skips_virtual_filter_loopback_and_disconnected_interfaces() {
        assert!(!is_monitored(&row("virtual", 6, 0b00, true)));
        assert!(!is_monitored(&row("filter", 6, 0b11, true)));
        assert!(!is_monitored(&row("loopback", 24, 0b01, true)));
        assert!(!is_monitored(&row("down", 6, 0b01, false)));
    }

    #[test]
    fn detects_adapter_set_changes() {
        let rows = vec![row("a", 6, 1, true), row("b", 71, 1, false)];
        assert_eq!(monitored_guids(&rows), vec!["a".to_string()]);
        let rows_after_wifi_connects = vec![row("a", 6, 1, true), row("b", 71, 1, true)];
        assert_eq!(monitored_guids(&rows_after_wifi_connects), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn wide_strings_stop_at_nul() {
        let mut w = [0u16; 8];
        w[..3].copy_from_slice(&[b'W' as u16, b'i' as u16, b'-' as u16]);
        assert_eq!(wide_to_string(&w), "Wi-");
    }
}
```

In `crates/oma-win/tests/providers.rs` aggiungi `use oma_win::network::NetworkProvider;` e:

```rust
#[test]
#[ignore = "requires real Windows hardware"]
fn network_provider_values_align_with_sensors() {
    // CI runners may expose no physical adapter: only alignment is guaranteed.
    let mut p = NetworkProvider::default();
    let (inventory, values) = discover_and_poll(&mut p);
    assert_eq!(inventory.sensors.len(), inventory.devices.len() * 3);
    for v in values.into_iter().flatten() {
        assert!(v >= 0.0);
    }
}
```

In `crates/oma-win/src/lib.rs` aggiungi `pub mod network;` e completa `default_providers()`:

```rust
pub fn default_providers() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(cpu::CpuProvider::new()),
        Box::new(memory::MemoryProvider),
        Box::new(storage::StorageProvider::default()),
        Box::new(network::NetworkProvider::default()),
    ]
}
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-win` (unitari); su hardware reale: `cargo test -p oma-win --test providers -- --ignored`
Risultato atteso: errore di compilazione (`file not found for module network`).

- [ ] **Step 3: Implementa**

In testa a `crates/oma-win/src/network.rs`:

```rust
//! Per-adapter network throughput and link speed from GetIfTable2.

use std::collections::HashMap;
use std::time::Instant;

use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
use oma_core::provider::{Inventory, Provider, ProviderError};
use oma_core::rate::CounterRate;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
const IF_TYPE_IEEE80211: u32 = 71;
const FLAG_HARDWARE_INTERFACE: u8 = 0x01;
const FLAG_FILTER_INTERFACE: u8 = 0x02;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InterfaceRow {
    pub guid: String,
    pub alias: String,
    pub if_type: u32,
    pub flags: u8,
    pub up: bool,
    pub in_octets: u64,
    pub out_octets: u64,
    pub link_bps: u64,
}

/// Connected physical Ethernet/Wi-Fi adapters; skips virtual adapters, NDIS
/// filter (LWF) duplicates and disconnected interfaces.
pub(crate) fn is_monitored(row: &InterfaceRow) -> bool {
    matches!(row.if_type, IF_TYPE_ETHERNET_CSMACD | IF_TYPE_IEEE80211)
        && row.flags & FLAG_HARDWARE_INTERFACE != 0
        && row.flags & FLAG_FILTER_INTERFACE == 0
        && row.up
}

/// Sorted GUIDs of the monitored adapters; a change means "rediscover".
pub(crate) fn monitored_guids(rows: &[InterfaceRow]) -> Vec<String> {
    let mut guids: Vec<String> = rows.iter().filter(|r| is_monitored(r)).map(|r| r.guid.clone()).collect();
    guids.sort();
    guids
}

pub(crate) fn wide_to_string(wide: &[u16]) -> String {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

fn read_interfaces() -> Result<Vec<InterfaceRow>, ProviderError> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: valid out-pointer; the table is freed below with FreeMibTable.
    let status = unsafe { GetIfTable2(&mut table) };
    if status != ERROR_SUCCESS || table.is_null() {
        return Err(ProviderError::Failed(format!("GetIfTable2 failed: {status:?}")));
    }
    // SAFETY: on success `table` points to `NumEntries` rows until freed.
    let rows = unsafe {
        let t = &*table;
        std::slice::from_raw_parts(t.Table.as_ptr(), t.NumEntries as usize)
    };
    let result = rows
        .iter()
        .map(|r| InterfaceRow {
            guid: format!("{:?}", r.InterfaceGuid).to_ascii_lowercase(),
            alias: wide_to_string(&r.Alias),
            if_type: r.Type,
            flags: r.InterfaceAndOperStatusFlags._bitfield,
            up: r.OperStatus == IfOperStatusUp,
            in_octets: r.InOctets,
            out_octets: r.OutOctets,
            link_bps: r.ReceiveLinkSpeed,
        })
        .collect();
    // SAFETY: `table` came from GetIfTable2 and is not used afterwards.
    unsafe { FreeMibTable(table as *const _) };
    Ok(result)
}

struct Adapter {
    guid: String,
    down: CounterRate,
    up: CounterRate,
}

pub struct NetworkProvider {
    epoch: Instant,
    adapters: Vec<Adapter>,
    known: Vec<String>,
}

impl Default for NetworkProvider {
    fn default() -> Self {
        Self { epoch: Instant::now(), adapters: Vec::new(), known: Vec::new() }
    }
}

impl NetworkProvider {
    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

impl Provider for NetworkProvider {
    fn name(&self) -> &'static str {
        "network"
    }

    fn discover(&mut self) -> Result<Inventory, ProviderError> {
        let mut rows: Vec<InterfaceRow> = read_interfaces()?.into_iter().filter(is_monitored).collect();
        rows.sort_by(|a, b| a.alias.cmp(&b.alias).then_with(|| a.guid.cmp(&b.guid)));
        let t = self.now_ms();

        let mut devices = Vec::new();
        let mut sensors = Vec::new();
        let mut adapters = Vec::new();
        for row in &rows {
            let id = format!("network/{}", row.guid);
            devices.push(Device { id: id.clone(), kind: DeviceKind::Network, name: row.alias.clone(), vendor: None, properties: Default::default() });
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "down",
                Unit::BytesPerSecond,
                Label::new("network.down"),
                Source::IpHelper,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "up",
                Unit::BytesPerSecond,
                Label::new("network.up"),
                Source::IpHelper,
            ));
            sensors.push(Sensor::new(
                &id,
                SensorKind::Throughput,
                "link-speed",
                Unit::BitsPerSecond,
                Label::new("network.linkSpeed"),
                Source::IpHelper,
            ));
            // Prime the baselines so the first poll already yields a rate.
            let mut down = CounterRate::new();
            let mut up = CounterRate::new();
            down.update(row.in_octets, t);
            up.update(row.out_octets, t);
            adapters.push(Adapter { guid: row.guid.clone(), down, up });
        }
        self.known = monitored_guids(&rows);
        self.adapters = adapters;
        Ok(Inventory { devices, sensors })
    }

    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let rows = read_interfaces()?;
        if monitored_guids(&rows) != self.known {
            return Err(ProviderError::Rediscover);
        }
        let t = self.now_ms();
        let by_guid: HashMap<&str, &InterfaceRow> = rows.iter().map(|r| (r.guid.as_str(), r)).collect();
        let mut values = Vec::with_capacity(self.adapters.len() * 3);
        for adapter in &mut self.adapters {
            let row = by_guid.get(adapter.guid.as_str()).ok_or(ProviderError::Rediscover)?;
            values.push(adapter.down.update(row.in_octets, t));
            values.push(adapter.up.update(row.out_octets, t));
            values.push(Some(row.link_bps as f64));
        }
        Ok(values)
    }
}
```

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-win` (unitari); su hardware reale: `cargo test -p oma-win --test providers -- --ignored`
Risultato atteso: tutti OK.

- [ ] **Step 5: Lint e commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add crates/oma-win
git commit -m "feat(win): network throughput and link speed provider"
```

---

### Task 10: scaffold del frontend, tema, tipi, formattazione e i18n

**File:**
- Crea:
  - `app/package.json`, `app/tsconfig.json`, `app/vite.config.ts`, `app/index.html`
  - `app/src/vite-env.d.ts`, `app/src/test-setup.ts`
  - `app/src/styles/theme.css`
  - `app/src/lib/types.ts`, `app/src/lib/view.ts`, `app/src/lib/format.ts`
  - `app/src/lib/i18n/index.svelte.ts`, `app/src/lib/i18n/en.json`, `app/src/lib/i18n/it.json`
- Test: `app/src/lib/format.test.ts`, `app/src/lib/i18n/i18n.test.ts`

**Interfacce:**
- Usa: la forma JSON prodotta dal Task 1 (camelCase, enum in snake_case).
- Produce:
  - `types.ts`: `DeviceKind`, `SensorKind`, `Unit`, `Source`, `Label`, `Device`, `Sensor`, `Schema`, `Snapshot`, `HistoryWindow`, `HistorySeed` (storico con revisione e sequenza atomiche).
  - `view.ts`: `type View = 'simple' | 'advanced'`.
  - `i18n/index.svelte.ts`:
    - `type Locale = 'en' | 'it'`
    - `i18n.locale` (stato reattivo)
    - `detectLocale(languages: readonly string[]): Locale`
    - `translate(locale, key, params?)`
    - `t(key, params?)`, che usa `i18n.locale`
    - `catalogs`
  - `format.ts`: tutte accettano `null` e in quel caso restituiscono `"—"`.
    - `formatPercent(v, locale)`
    - `formatBytes(v, locale)`
    - `formatRate(bytesPerSecond, mode: 'bits' | 'bytes', locale)`
    - `formatClock(mhz, locale)`
    - `formatDuration(ms, t)`
    - `DASH`

- [ ] **Step 1: Crea la configurazione del progetto**

`app/package.json`:

```json
{
  "name": "oma-app-ui",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "license": "GPL-3.0-or-later",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "check": "svelte-check --tsconfig ./tsconfig.json",
    "test": "vitest run",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "2.11.1"
  },
  "devDependencies": {
    "@sveltejs/vite-plugin-svelte": "7.3.1",
    "@tauri-apps/cli": "2.11.5",
    "@testing-library/svelte": "5.4.2",
    "@tsconfig/svelte": "5.0.8",
    "@types/node": "22.20.4",
    "jsdom": "30.1.1",
    "svelte": "5.57.1",
    "svelte-check": "4.7.6",
    "typescript": "6.0.3",
    "vite": "8.3.1",
    "vitest": "5.0.1"
  },
  "packageManager": "pnpm@10.15.0"
}
```

`app/tsconfig.json`:

```json
{
  "extends": "@tsconfig/svelte/tsconfig.json",
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "strict": true,
    "noEmit": true,
    "types": ["node", "vitest/globals"]
  },
  "include": ["src/**/*.ts", "src/**/*.svelte", "vite.config.ts"]
}
```

`app/vite.config.ts` (**`defineConfig` va importato da `vitest/config`**, altrimenti `svelte-check` rifiuta la chiave `test`):

```ts
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  resolve: process.env.VITEST ? { conditions: ['browser'] } : undefined,
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['src/**/*.test.ts'],
    setupFiles: ['src/test-setup.ts'],
  },
});
```

`app/src/test-setup.ts` (**jsdom non ha `matchMedia`**; `prefersReducedMotion` di `svelte/motion` la chiama quando il modulo viene importato):

```ts
// jsdom has no matchMedia; svelte/motion's prefersReducedMotion needs it.
if (!window.matchMedia) {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}
```

`app/src/vite-env.d.ts`:

```ts
/// <reference types="svelte" />
/// <reference types="vite/client" />
```

`app/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>OpenMonitor Advanced</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

`app/src/styles/theme.css` (palette Synthwave, spec §7.5):

```css
:root {
  --bg: #0f0a1a;
  --surface: #181126;
  --surface-2: #211733;
  --border: #2d2042;
  --text: #f5eefe;
  --text-muted: #9585b0;
  --accent: #ff4fd8;
  --accent-2: #4cc9f0;
  --ok: #3ee8b5;
  --warn: #ffc53d;
  --crit: #ff4d4d;

  --radius: 12px;
  --font: 'Segoe UI Variable', 'Segoe UI', system-ui, sans-serif;
  color-scheme: dark;
}

* {
  box-sizing: border-box;
}

html,
body {
  margin: 0;
  min-height: 100%;
  background: var(--bg);
  color: var(--text);
  font-family: var(--font);
  -webkit-font-smoothing: antialiased;
}

body {
  user-select: none;
}

button {
  font: inherit;
  color: inherit;
}

.label {
  font-size: 11px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--text-muted);
}

@media (prefers-reduced-motion: reduce) {
  *,
  *::before,
  *::after {
    transition: none !important;
    animation: none !important;
  }
}
```

Installa le dipendenze (crea `app/pnpm-lock.yaml`):

```bash
cd app && pnpm install
```

- [ ] **Step 2: Scrivi i tipi e il file della vista**

`app/src/lib/types.ts` (rispecchia `oma_core::model` e `oma_core::history`):

```ts
export type DeviceKind =
  | 'cpu'
  | 'gpu'
  | 'memory'
  | 'storage'
  | 'network'
  | 'motherboard'
  | 'battery'
  | 'fan_controller'
  | 'psu';

export type SensorKind =
  | 'temperature'
  | 'load'
  | 'clock'
  | 'power'
  | 'voltage'
  | 'current'
  | 'fan'
  | 'data'
  | 'throughput'
  | 'energy'
  | 'flag'
  | 'percent';

export type Unit =
  | 'celsius'
  | 'percent'
  | 'megahertz'
  | 'watt'
  | 'volt'
  | 'ampere'
  | 'rpm'
  | 'bytes'
  | 'bytes_per_second'
  | 'bits_per_second'
  | 'joule'
  | 'boolean';

export type Source = 'pdh' | 'win32' | 'ip_helper' | 'mock';

/** Translation key (looked up as `sensor.<key>`) plus optional `{arg}`. */
export interface Label {
  key: string;
  arg?: string;
}

export interface Device {
  id: string;
  kind: DeviceKind;
  name: string;
  vendor?: string;
  properties?: Record<string, string>;
}

export interface Sensor {
  id: string;
  deviceId: string;
  kind: SensorKind;
  unit: Unit;
  label: Label;
  source: Source;
  category: string;
}

export interface Schema {
  revision: number;
  devices: Device[];
  sensors: Sensor[];
}

/** `values[i]` belongs to `schema.sensors[i]` of the schema with the same revision. */
export interface Snapshot {
  revision: number;
  seq: number;
  timestampMs: number;
  values: (number | null)[];
}

export interface HistoryWindow {
  timestampsMs: number[];
  series: (number | null)[][];
}

/** Atomic watermark attached by the backend while holding the engine lock. */
export interface HistorySeed extends HistoryWindow {
  revision: number;
  seq: number;
}
```

`app/src/lib/view.ts`:

```ts
export type View = 'simple' | 'advanced';
```

- [ ] **Step 3: Scrivi i test di formattazione e i18n (falliscono)**

`app/src/lib/format.test.ts`:

```ts
import { DASH, formatBytes, formatClock, formatDuration, formatPercent, formatRate } from './format';
import { translate } from './i18n/index.svelte';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);

test('null values render as a dash', () => {
  expect(formatPercent(null, 'en')).toBe(DASH);
  expect(formatBytes(null, 'en')).toBe(DASH);
  expect(formatRate(null, 'bits', 'en')).toBe(DASH);
  expect(formatClock(null, 'en')).toBe(DASH);
});

test('percent has no decimals', () => {
  expect(formatPercent(35.4, 'en')).toBe('35%');
  expect(formatPercent(99.6, 'it')).toBe('100%');
});

test('bytes use binary steps with Windows-style unit names', () => {
  expect(formatBytes(512, 'en')).toBe('512 B');
  expect(formatBytes(1536, 'en')).toBe('1.5 KB');
  expect(formatBytes(17.9 * 1024 ** 3, 'en')).toBe('17.9 GB');
  expect(formatBytes(17.9 * 1024 ** 3, 'it')).toBe('17,9 GB');
  expect(formatBytes(250 * 1024 ** 2, 'en')).toBe('250 MB');
});

test('rates in bits use decimal steps', () => {
  expect(formatRate(6_000_000, 'bits', 'en')).toBe('48 Mbit/s');
  expect(formatRate(125_000, 'bits', 'it')).toBe('1,0 Mbit/s');
  expect(formatRate(0, 'bits', 'en')).toBe('0.0 bit/s');
});

test('rates in bytes reuse byte units', () => {
  expect(formatRate(120 * 1024 ** 2, 'bytes', 'en')).toBe('120 MB/s');
});

test('clock switches to GHz from 1000 MHz', () => {
  expect(formatClock(4383.7, 'en')).toBe('4.38 GHz');
  expect(formatClock(4383.7, 'it')).toBe('4,38 GHz');
  expect(formatClock(800, 'en')).toBe('800 MHz');
});

test('durations', () => {
  expect(formatDuration(5 * 60_000, tEn)).toBe('5 min');
  expect(formatDuration(125 * 60_000, tEn)).toBe('2 h 5 min');
  expect(formatDuration(-1, tEn)).toBe('0 min');
});
```

`app/src/lib/i18n/i18n.test.ts`:

```ts
import { catalogs, detectLocale, translate } from './index.svelte';

test('both catalogs define the same keys', () => {
  expect(Object.keys(catalogs.it).sort()).toEqual(Object.keys(catalogs.en).sort());
});

test('interpolates named parameters', () => {
  expect(translate('en', 'health.since', { duration: '5 min' })).toBe('for 5 min');
  expect(translate('it', 'health.since', { duration: '5 min' })).toBe('da 5 min');
});

test('unknown keys fall back to the key itself', () => {
  expect(translate('it', 'nope.nope')).toBe('nope.nope');
});

test('missing parameters stay visible', () => {
  expect(translate('en', 'health.since')).toBe('for {duration}');
});

test('picks the first supported language', () => {
  expect(detectLocale(['it-IT', 'en-US'])).toBe('it');
  expect(detectLocale(['de-DE', 'it'])).toBe('it');
  expect(detectLocale(['de-DE'])).toBe('en');
  expect(detectLocale([])).toBe('en');
});
```

- [ ] **Step 4: Esegui i test e verifica che falliscano**

Esegui: `cd app && pnpm test`
Risultato atteso: FAIL (`Failed to resolve import "./format"`).

- [ ] **Step 5: Implementa i18n e formattazione**

`app/src/lib/i18n/en.json`:

```json
{
  "app.title": "OpenMonitor Advanced",
  "tray.open": "Open",
  "tray.quit": "Quit",
  "view.label": "View",
  "view.simple": "Simple",
  "view.advanced": "Advanced",
  "settings.title": "Settings",
  "settings.comingSoon": "Settings — coming soon",
  "service.baseMode": "Basic mode",
  "service.baseModeHint": "Advanced sensors (CPU temperatures, fans, voltages) need the OpenMonitor Advanced service, which is not installed.",
  "health.monitoring": "Monitoring active",
  "health.since": "for {duration}",
  "duration.minutes": "{n} min",
  "duration.hoursMinutes": "{h} h {m} min",
  "tile.cpu": "CPU",
  "tile.memory": "RAM",
  "tile.netDisk": "Network · Disks",
  "tile.diskIo": "read {read} · write {write}",
  "advanced.comingSoon": "The Advanced view arrives in milestone 3.",
  "sensor.cpu.load.total": "Total load",
  "sensor.cpu.load.thread": "Thread {arg} load",
  "sensor.cpu.clock.effective": "Estimated clock",
  "sensor.memory.load": "Memory load",
  "sensor.memory.used": "Used memory",
  "sensor.memory.total": "Total memory",
  "sensor.storage.read": "Read rate",
  "sensor.storage.write": "Write rate",
  "sensor.storage.active": "Active time",
  "sensor.storage.volumeUsed": "Volume {arg} used",
  "sensor.storage.volumeFree": "Volume {arg} free",
  "sensor.network.down": "Download",
  "sensor.network.up": "Upload",
  "sensor.network.linkSpeed": "Link speed"
}
```

`app/src/lib/i18n/it.json`:

```json
{
  "app.title": "OpenMonitor Advanced",
  "tray.open": "Apri",
  "tray.quit": "Esci",
  "view.label": "Vista",
  "view.simple": "Semplice",
  "view.advanced": "Avanzata",
  "settings.title": "Impostazioni",
  "settings.comingSoon": "Impostazioni — in arrivo",
  "service.baseMode": "Modalità base",
  "service.baseModeHint": "I sensori avanzati (temperature CPU, ventole, tensioni) richiedono il servizio di OpenMonitor Advanced, che non è installato.",
  "health.monitoring": "Monitoraggio attivo",
  "health.since": "da {duration}",
  "duration.minutes": "{n} min",
  "duration.hoursMinutes": "{h} h {m} min",
  "tile.cpu": "CPU",
  "tile.memory": "RAM",
  "tile.netDisk": "Rete · Dischi",
  "tile.diskIo": "lettura {read} · scrittura {write}",
  "advanced.comingSoon": "La vista Avanzata arriva con la milestone 3.",
  "sensor.cpu.load.total": "Carico totale",
  "sensor.cpu.load.thread": "Carico thread {arg}",
  "sensor.cpu.clock.effective": "Clock stimato",
  "sensor.memory.load": "Memoria in uso",
  "sensor.memory.used": "Memoria usata",
  "sensor.memory.total": "Memoria totale",
  "sensor.storage.read": "Velocità di lettura",
  "sensor.storage.write": "Velocità di scrittura",
  "sensor.storage.active": "Tempo attivo",
  "sensor.storage.volumeUsed": "Volume {arg} occupato",
  "sensor.storage.volumeFree": "Volume {arg} libero",
  "sensor.network.down": "Download",
  "sensor.network.up": "Upload",
  "sensor.network.linkSpeed": "Velocità del collegamento"
}
```

`app/src/lib/i18n/index.svelte.ts`:

```ts
import en from './en.json';
import it from './it.json';

export type Locale = 'en' | 'it';
export type Params = Record<string, string | number>;
export type Translate = (key: string, params?: Params) => string;

export const catalogs: Record<Locale, Record<string, string>> = { en, it };
const SUPPORTED: readonly Locale[] = ['en', 'it'];

class I18nState {
  locale = $state<Locale>('en');
}

/** Current UI language; reading it inside templates makes them reactive. */
export const i18n = new I18nState();

/** First supported language in the user's preference list, else English. */
export function detectLocale(languages: readonly string[]): Locale {
  for (const language of languages) {
    const base = language.toLowerCase().split('-')[0];
    const match = SUPPORTED.find((l) => l === base);
    if (match) return match;
  }
  return 'en';
}

export function translate(locale: Locale, key: string, params: Params = {}): string {
  const template = catalogs[locale][key] ?? catalogs.en[key] ?? key;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) => (name in params ? String(params[name]) : whole));
}

export const t: Translate = (key, params) => translate(i18n.locale, key, params);
```

`app/src/lib/format.ts`:

```ts
import type { Locale, Translate } from './i18n/index.svelte';

export const DASH = '—';

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];
const BIT_UNITS = ['bit/s', 'kbit/s', 'Mbit/s', 'Gbit/s', 'Tbit/s'];
const formatters = new Map<string, Intl.NumberFormat>();

function num(value: number, digits: number, locale: Locale): string {
  const key = `${locale}:${digits}`;
  let formatter = formatters.get(key);
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits });
    formatters.set(key, formatter);
  }
  return formatter.format(value);
}

const missing = (v: number | null): v is null => v === null || !Number.isFinite(v);

export function formatPercent(value: number | null, locale: Locale): string {
  return missing(value) ? DASH : `${num(value, 0, locale)}%`;
}

/** Binary steps (1024) with the unit names Windows shows (KB, MB, GB). */
export function formatBytes(bytes: number | null, locale: Locale): string {
  if (missing(bytes)) return DASH;
  let value = bytes;
  let unit = 0;
  while (Math.abs(value) >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${num(value, digits, locale)} ${BYTE_UNITS[unit]}`;
}

/** Network-style rates: bits with decimal steps, or bytes with binary steps. */
export function formatRate(bytesPerSecond: number | null, mode: 'bits' | 'bytes', locale: Locale): string {
  if (missing(bytesPerSecond)) return DASH;
  if (mode === 'bytes') return `${formatBytes(bytesPerSecond, locale)}/s`;
  let value = bytesPerSecond * 8;
  let unit = 0;
  while (Math.abs(value) >= 1000 && unit < BIT_UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${num(value, value < 10 ? 1 : 0, locale)} ${BIT_UNITS[unit]}`;
}

export function formatClock(mhz: number | null, locale: Locale): string {
  if (missing(mhz)) return DASH;
  return mhz >= 1000 ? `${num(mhz / 1000, 2, locale)} GHz` : `${num(mhz, 0, locale)} MHz`;
}

export function formatDuration(ms: number, t: Translate): string {
  const minutes = Math.max(0, Math.floor(ms / 60_000));
  if (minutes < 60) return t('duration.minutes', { n: minutes });
  return t('duration.hoursMinutes', { h: Math.floor(minutes / 60), m: minutes % 60 });
}
```

- [ ] **Step 6: Esegui i test e verifica che passino**

Esegui: `cd app && pnpm test && pnpm check`
Risultato atteso: test PASS (7 di formattazione e 5 di i18n); `svelte-check` con `0 ERRORS`.

- [ ] **Step 7: Commit**

```bash
git add app
git commit -m "feat(ui): frontend scaffold, Synthwave theme, formatting and i18n"
```

---

### Task 11: livello dati della UI (backend, `SeriesBuffer`, `LiveStore`, selettori)

**File:**
- Crea:
  - `app/src/lib/backend/backend.ts`, `app/src/lib/backend/tauri.ts`, `app/src/lib/backend/mock.ts`, `app/src/lib/backend/index.ts`
  - `app/src/lib/series.ts`, `app/src/lib/live.svelte.ts`, `app/src/lib/select.ts`
  - `app/src/test/fake-backend.ts`
- Test: `app/src/lib/series.test.ts`, `app/src/lib/backend/mock.test.ts`, `app/src/lib/live.test.ts`, `app/src/lib/select.test.ts`

**Interfacce:**
- Usa: `types.ts`, `catalogs` (Task 10).
- Produce:
  - `Backend`:
    - `getSchema(): Promise<Schema>`
    - `getHistory(ids: string[], seconds: number): Promise<HistorySeed>`
    - `onSchema(cb): Promise<Unsubscribe>`
    - `onSnapshot(cb): Promise<Unsubscribe>`
    - dove `type Unsubscribe = () => void`
  - `createBackend(): Backend`: usa Tauri se `isTauri()`, altrimenti il mock.
  - `createTauriBackend()`: comandi `get_schema` e `get_history`, eventi `oma:schema` e `oma:snapshot` (vedi Task 13).
  - `MOCK_SCHEMA`, `mockValues(tick: number)`, `createMockBackend(intervalMs = 1000)`.
  - `SeriesBuffer(capacity)`: `push(v: number | null)`, `toArray(): number[]` (NaN = mancante), `clear()`, `length`.
  - `LiveStore(capacity = 300)`, con i campi reattivi:
    - `schema`, `values`
    - `timestampMs`, `firstTimestampMs`
    - `capacity`
    
    e i metodi:
    - `applySchema(schema)`
    - `applySnapshot(snap): boolean`, che restituisce `false` se la revisione non corrisponde
    - `seedHistory(ids, history)`
    - `value(id)`
    - `series(id): number[]`
  - `connect(store, backend): Promise<Unsubscribe>`
  - `select.ts`:
    - `type ValueOf = (id: string) => number | null`
    - `cpuSummary`, `memorySummary`, `storageSummary`, `networkSummary`
    - `sumSeries(series: number[][]): number[]`

**Contratto tra Rust e UI:** i suffissi `mock-ssd`, `mock-guid` e `mock-eth` sono identità sintetiche delle fixture; nessun selettore deve dipendere da questi valori. I selettori trovano i sensori per `device.kind` + `label.key` (+ `label.arg`), **mai** interpretando l'id. `MOCK_SCHEMA` usa la stessa struttura degli ID e le stesse chiavi prodotte dai provider Rust dei Task 6–9.

- [ ] **Step 1: Scrivi l'interfaccia del backend e il fake per i test**

`app/src/lib/backend/backend.ts`:

```ts
import type { HistorySeed, Schema, Snapshot } from '../types';

export type Unsubscribe = () => void;

/** Everything the UI needs from the sampling core (Tauri, or a mock in the browser). */
export interface Backend {
  getSchema(): Promise<Schema>;
  getHistory(ids: string[], seconds: number): Promise<HistorySeed>;
  onSchema(cb: (schema: Schema) => void): Promise<Unsubscribe>;
  onSnapshot(cb: (snapshot: Snapshot) => void): Promise<Unsubscribe>;
}
```

`app/src/test/fake-backend.ts`:

```ts
import type { Backend, Unsubscribe } from '../lib/backend/backend';
import type { HistorySeed, HistoryWindow, Schema, Snapshot } from '../lib/types';

/** Hand-driven backend for tests: emit events explicitly. */
export class FakeBackend implements Backend {
  schema: Schema;
  history: HistoryWindow = { timestampsMs: [], series: [] };
  schemaCalls = 0;
  #schemaListeners = new Set<(s: Schema) => void>();
  #snapshotListeners = new Set<(s: Snapshot) => void>();

  constructor(schema: Schema) {
    this.schema = schema;
  }

  async getSchema(): Promise<Schema> {
    this.schemaCalls++;
    return this.schema;
  }

  async getHistory(ids: string[]): Promise<HistorySeed> {
    return { revision: this.schema.revision, seq: 0, timestampsMs: this.history.timestampsMs, series: ids.map((_, i) => this.history.series[i] ?? []) };
  }

  async onSchema(cb: (s: Schema) => void): Promise<Unsubscribe> {
    this.#schemaListeners.add(cb);
    return () => this.#schemaListeners.delete(cb);
  }

  async onSnapshot(cb: (s: Snapshot) => void): Promise<Unsubscribe> {
    this.#snapshotListeners.add(cb);
    return () => this.#snapshotListeners.delete(cb);
  }

  emitSchema(schema: Schema): void {
    this.schema = schema;
    this.#schemaListeners.forEach((cb) => cb(schema));
  }

  emitSnapshot(snapshot: Snapshot): void {
    this.#snapshotListeners.forEach((cb) => cb(snapshot));
  }
}
```

- [ ] **Step 2: Scrivi i test (falliscono)**

`app/src/lib/series.test.ts`:

```ts
import { SeriesBuffer } from './series';

test('keeps the most recent values up to capacity', () => {
  const b = new SeriesBuffer(3);
  [1, 2, 3, 4].forEach((v) => b.push(v));
  expect(b.toArray()).toEqual([2, 3, 4]);
  expect(b.length).toBe(3);
});

test('null is stored as NaN', () => {
  const b = new SeriesBuffer(2);
  b.push(null);
  expect(Number.isNaN(b.toArray()[0])).toBe(true);
});

test('clear empties the buffer', () => {
  const b = new SeriesBuffer(2);
  b.push(1);
  b.clear();
  expect(b.toArray()).toEqual([]);
});
```

`app/src/lib/backend/mock.test.ts`:

```ts
import { catalogs } from '../i18n/index.svelte';
import { MOCK_SCHEMA, createMockBackend, mockValues } from './mock';

test('mock values align with the mock schema', () => {
  for (const tick of [0, 1, 50, 1000]) {
    expect(mockValues(tick)).toHaveLength(MOCK_SCHEMA.sensors.length);
  }
});

test('every mock sensor label has a translation', () => {
  for (const sensor of MOCK_SCHEMA.sensors) {
    expect(catalogs.en[`sensor.${sensor.label.key}`], sensor.label.key).toBeDefined();
  }
});

test('mock backend emits one snapshot per interval while subscribed', async () => {
  vi.useFakeTimers();
  try {
    const backend = createMockBackend(1000);
    const seqs: number[] = [];
    const off = await backend.onSnapshot((s) => seqs.push(s.seq));
    vi.advanceTimersByTime(3000);
    off();
    vi.advanceTimersByTime(3000);
    expect(seqs).toEqual([1, 2, 3]);
  } finally {
    vi.useRealTimers();
  }
});

test('mock history returns one series per id', async () => {
  const backend = createMockBackend();
  const h = await backend.getHistory(['cpu/0/load/total', 'unknown'], 10);
  expect(h.timestampsMs).toHaveLength(10);
  expect(h.series[0]).toHaveLength(10);
  expect(h.series[1].every((v) => v === null)).toBe(true);
});
```

`app/src/lib/live.test.ts`:

```ts
import { FakeBackend } from '../test/fake-backend';
import { MOCK_SCHEMA, mockValues } from './backend/mock';
import { LiveStore, connect } from './live.svelte';

const snapshot = (seq: number, revision = 1) => ({
  revision,
  seq,
  timestampMs: 1000 * seq,
  values: mockValues(seq),
});

test('applySnapshot updates values, timestamps and series', () => {
  const store = new LiveStore(3);
  store.applySchema(MOCK_SCHEMA);
  expect(store.applySnapshot(snapshot(1))).toBe(true);
  expect(store.applySnapshot(snapshot(2))).toBe(true);
  expect(store.value('cpu/0/load/total')).toBe(mockValues(2)[0]);
  expect(store.series('cpu/0/load/total')).toEqual([mockValues(1)[0], mockValues(2)[0]]);
  expect(store.firstTimestampMs).toBe(1000);
  expect(store.timestampMs).toBe(2000);
});

test('rejects snapshot of another revision', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  expect(store.applySnapshot(snapshot(1, 2))).toBe(false);
  expect(store.value('cpu/0/load/total')).toBeNull();
});

test('unknown sensors read as null and empty series', () => {
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  expect(store.value('nope')).toBeNull();
  expect(store.series('nope')).toEqual([]);
});

test('connect seeds sparklines from history', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.history = { timestampsMs: [500, 1500], series: [[10, 20]] };
  const store = new LiveStore(5);
  const off = await connect(store, backend);
  expect(store.series('cpu/0/load/total')).toEqual([10, 20]);
  expect(store.firstTimestampMs).toBe(500);
  off();
});

test('late history keeps snapshots received while the request was pending', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  let resolve!: (h: import('./types').HistorySeed) => void;
  backend.getHistory = () => new Promise((done) => { resolve = done; });
  const store = new LiveStore();
  const connecting = connect(store, backend);
  await vi.waitFor(() => expect(resolve).toBeDefined());
  backend.emitSnapshot(snapshot(2));
  resolve({ revision: 1, seq: 1, timestampsMs: [1000], series: MOCK_SCHEMA.sensors.map((_, i) => [mockValues(1)[i]]) });
  const off = await connecting;
  expect(store.series('cpu/0/load/total')).toEqual([mockValues(1)[0], mockValues(2)[0]]);
  expect(store.timestampMs).toBe(2000);
  backend.emitSnapshot(snapshot(1));
  expect(store.timestampMs).toBe(2000);
  off();
});

test('failed connection unsubscribes and does not mutate the store', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.getHistory = async () => { throw new Error('offline'); };
  const store = new LiveStore();
  await expect(connect(store, backend)).rejects.toThrow('offline');
  backend.emitSchema(MOCK_SCHEMA);
  backend.emitSnapshot(snapshot(1));
  expect(store.schema).toBeNull();
});

test('connect refetches schema on revision mismatch', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  const off = await connect(store, backend);
  expect(backend.schemaCalls).toBe(1);
  backend.schema = { ...MOCK_SCHEMA, revision: 7 };
  backend.emitSnapshot(snapshot(1, 7));
  await vi.waitFor(() => expect(backend.schemaCalls).toBe(2));
  off();
});
```

`app/src/lib/select.test.ts`:

```ts
import { MOCK_SCHEMA, mockValues } from './backend/mock';
import { cpuSummary, memorySummary, networkSummary, storageSummary, sumSeries } from './select';
import type { Schema } from './types';

const values = mockValues(1);
const valueOf = (id: string) => {
  const i = MOCK_SCHEMA.sensors.findIndex((s) => s.id === id);
  return i < 0 ? null : values[i];
};

test('cpu summary', () => {
  const cpu = cpuSummary(MOCK_SCHEMA, valueOf)!;
  expect(cpu.name).toBe('Mock Ryzen 7 7800X3D');
  expect(cpu.loadId).toBe('cpu/0/load/total');
  expect(cpu.load).toBe(values[0]);
  expect(cpu.clockMhz).toBe(valueOf('cpu/0/clock/effective'));
});

test('memory summary', () => {
  const mem = memorySummary(MOCK_SCHEMA, valueOf)!;
  expect(mem.totalBytes).toBe(32 * 1024 ** 3);
  expect(mem.usedPct).toBe(valueOf('memory/0/load/used'));
});

test('storage summary prefers the C: volume', () => {
  const disk = storageSummary(MOCK_SCHEMA, valueOf)!;
  expect(disk.volume).toEqual({ letter: 'C:', usedPct: 65 });
  expect(disk.readBps).toBe(valueOf('storage/device-mock-ssd/throughput/read'));
});

test('network summary sums every adapter', () => {
  const schema: Schema = {
    ...MOCK_SCHEMA,
    devices: [...MOCK_SCHEMA.devices, { id: 'network/wifi', kind: 'network', name: 'Wi-Fi' }],
    sensors: [
      ...MOCK_SCHEMA.sensors,
      { id: 'network/wifi/throughput/down', deviceId: 'network/wifi', kind: 'throughput', unit: 'bytes_per_second', label: { key: 'network.down' }, source: 'mock', category: 'throughput' },
    ],
  };
  const extra = (id: string) => (id === 'network/wifi/throughput/down' ? 100 : valueOf(id));
  const net = networkSummary(schema, extra)!;
  expect(net.downBps).toBe((valueOf('network/mock-eth/throughput/down') ?? 0) + 100);
  expect(net.downIds).toEqual(['network/mock-eth/throughput/down', 'network/wifi/throughput/down']);
});

test('summaries are null when the device kind is missing', () => {
  const empty: Schema = { revision: 1, devices: [], sensors: [] };
  expect(cpuSummary(empty, valueOf)).toBeNull();
  expect(memorySummary(empty, valueOf)).toBeNull();
  expect(storageSummary(empty, valueOf)).toBeNull();
  expect(networkSummary(empty, valueOf)).toBeNull();
});

test('sumSeries right-aligns and ignores gaps', () => {
  expect(sumSeries([[1, 2, 3], [10, NaN, 30]])).toEqual([11, 2, 33]);
  expect(sumSeries([[1, 2], [5, 6, 7]])).toEqual([5, 7, 9]);
  expect(Number.isNaN(sumSeries([[NaN]])[0])).toBe(true);
  expect(sumSeries([])).toEqual([]);
});
```

- [ ] **Step 3: Esegui i test e verifica che falliscano**

Esegui: `cd app && pnpm test`
Risultato atteso: FAIL (`Failed to resolve import "./series"`, `"./mock"`, ...).

- [ ] **Step 4: Implementa `SeriesBuffer` e il backend mock**

`app/src/lib/series.ts`:

```ts
/** Fixed-capacity ring buffer of samples; NaN marks a missing value. */
export class SeriesBuffer {
  readonly capacity: number;
  #data: Float64Array;
  #start = 0;
  #length = 0;

  constructor(capacity: number) {
    this.capacity = capacity;
    this.#data = new Float64Array(capacity);
  }

  get length(): number {
    return this.#length;
  }

  push(value: number | null): void {
    const v = value ?? NaN;
    if (this.#length < this.capacity) {
      this.#data[(this.#start + this.#length) % this.capacity] = v;
      this.#length++;
    } else {
      this.#data[this.#start] = v;
      this.#start = (this.#start + 1) % this.capacity;
    }
  }

  toArray(): number[] {
    return Array.from({ length: this.#length }, (_, i) => this.#data[(this.#start + i) % this.capacity]);
  }

  clear(): void {
    this.#start = 0;
    this.#length = 0;
  }
}
```

`app/src/lib/backend/mock.ts`:

```ts
import type { Label, Schema, Sensor, SensorKind, Snapshot, Unit } from '../types';
import type { Backend } from './backend';

const THREADS = 8;
const GIB = 1024 ** 3;

const sensor = (id: string, deviceId: string, kind: SensorKind, unit: Unit, label: Label): Sensor => ({
  id,
  deviceId,
  kind,
  unit,
  label,
  source: 'mock',
  category: kind,
});

/** Same ids and label keys the Rust providers produce (crates/oma-win). */
export const MOCK_SCHEMA: Schema = {
  revision: 1,
  devices: [
    { id: 'cpu/0', kind: 'cpu', name: 'Mock Ryzen 7 7800X3D' },
    { id: 'memory/0', kind: 'memory', name: 'RAM' },
    { id: 'storage/device-mock-ssd', kind: 'storage', name: 'Disk 0 (C:)' },
    { id: 'network/mock-eth', kind: 'network', name: 'Ethernet' },
  ],
  sensors: [
    sensor('cpu/0/load/total', 'cpu/0', 'load', 'percent', { key: 'cpu.load.total' }),
    ...Array.from({ length: THREADS }, (_, i) =>
      sensor(`cpu/0/load/thread-0-${i}`, 'cpu/0', 'load', 'percent', { key: 'cpu.load.thread', arg: String(i) }),
    ),
    sensor('cpu/0/clock/effective', 'cpu/0', 'clock', 'megahertz', { key: 'cpu.clock.effective' }),
    sensor('memory/0/load/used', 'memory/0', 'load', 'percent', { key: 'memory.load' }),
    sensor('memory/0/data/used', 'memory/0', 'data', 'bytes', { key: 'memory.used' }),
    sensor('memory/0/data/total', 'memory/0', 'data', 'bytes', { key: 'memory.total' }),
    sensor('storage/device-mock-ssd/throughput/read', 'storage/device-mock-ssd', 'throughput', 'bytes_per_second', { key: 'storage.read' }),
    sensor('storage/device-mock-ssd/throughput/write', 'storage/device-mock-ssd', 'throughput', 'bytes_per_second', { key: 'storage.write' }),
    sensor('storage/device-mock-ssd/load/active', 'storage/device-mock-ssd', 'load', 'percent', { key: 'storage.active' }),
    sensor('storage/device-mock-ssd/percent/volume-mock-guid', 'storage/device-mock-ssd', 'percent', 'percent', { key: 'storage.volumeUsed', arg: 'C:' }),
    sensor('storage/device-mock-ssd/data/volume-mock-guid-free', 'storage/device-mock-ssd', 'data', 'bytes', { key: 'storage.volumeFree', arg: 'C:' }),
    sensor('network/mock-eth/throughput/down', 'network/mock-eth', 'throughput', 'bytes_per_second', { key: 'network.down' }),
    sensor('network/mock-eth/throughput/up', 'network/mock-eth', 'throughput', 'bytes_per_second', { key: 'network.up' }),
    sensor('network/mock-eth/throughput/link-speed', 'network/mock-eth', 'throughput', 'bits_per_second', { key: 'network.linkSpeed' }),
  ],
};

/** Deterministic plausible values for tick `t`, in MOCK_SCHEMA sensor order. */
export function mockValues(t: number): (number | null)[] {
  const wave = (period: number, phase = 0) => (Math.sin((t + phase) / period) + 1) / 2;
  const total = 20 + 50 * wave(9);
  const threads = Array.from({ length: THREADS }, (_, i) => Math.min(100, total * (0.6 + 0.1 * i)));
  const memTotal = 32 * GIB;
  const memUsed = memTotal * (0.5 + 0.1 * wave(30));
  return [
    total,
    ...threads,
    4200 + 400 * wave(7),
    (memUsed / memTotal) * 100,
    memUsed,
    memTotal,
    120e6 * wave(5),
    30e6 * wave(6, 2),
    60 * wave(5),
    65,
    700 * GIB,
    6e6 * wave(4),
    4e5 * wave(4, 1),
    1e9,
  ];
}

/** Browser-only backend used by `pnpm dev` and component tests. */
export function createMockBackend(intervalMs = 1000): Backend {
  let seq = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  const listeners = new Set<(s: Snapshot) => void>();
  const emit = () => {
    seq++;
    const snapshot: Snapshot = { revision: MOCK_SCHEMA.revision, seq, timestampMs: Date.now(), values: mockValues(seq) };
    listeners.forEach((cb) => cb(snapshot));
  };
  return {
    getSchema: async () => MOCK_SCHEMA,
    getHistory: async (ids, seconds) => {
      const n = Math.max(0, Math.min(seconds, 300));
      const now = Date.now();
      const ticks = Array.from({ length: n }, (_, i) => seq - n + 1 + i);
      const indices = ids.map((id) => MOCK_SCHEMA.sensors.findIndex((s) => s.id === id));
      return {
        revision: MOCK_SCHEMA.revision,
        seq,
        timestampsMs: ticks.map((_, i) => now - (n - 1 - i) * intervalMs),
        series: indices.map((k) => ticks.map((tick) => (k < 0 ? null : mockValues(tick)[k]))),
      };
    },
    onSchema: async () => () => {},
    onSnapshot: async (cb) => {
      listeners.add(cb);
      timer ??= setInterval(emit, intervalMs);
      return () => {
        listeners.delete(cb);
        if (listeners.size === 0 && timer !== undefined) {
          clearInterval(timer);
          timer = undefined;
        }
      };
    },
  };
}
```

`app/src/lib/backend/tauri.ts`:

```ts
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { HistorySeed, Schema, Snapshot } from '../types';
import type { Backend } from './backend';

/** Command and event names are defined in app/src-tauri (commands.rs, main.rs). */
export function createTauriBackend(): Backend {
  return {
    getSchema: () => invoke<Schema>('get_schema'),
    getHistory: (ids, seconds) => invoke<HistorySeed>('get_history', { ids, seconds }),
    onSchema: (cb) => listen<Schema>('oma:schema', (e) => cb(e.payload)),
    onSnapshot: (cb) => listen<Snapshot>('oma:snapshot', (e) => cb(e.payload)),
  };
}
```

`app/src/lib/backend/index.ts`:

```ts
import { isTauri } from '@tauri-apps/api/core';
import type { Backend } from './backend';
import { createMockBackend } from './mock';
import { createTauriBackend } from './tauri';

export type { Backend, Unsubscribe } from './backend';

export function createBackend(): Backend {
  return isTauri() ? createTauriBackend() : createMockBackend();
}
```

- [ ] **Step 5: Implementa `LiveStore`, `connect` e i selettori**

`app/src/lib/live.svelte.ts`:

```ts
import type { Backend, Unsubscribe } from './backend/backend';
import { SeriesBuffer } from './series';
import type { HistorySeed, Schema, Snapshot } from './types';

/** Five minutes at the default 1 s interval. */
export const SPARKLINE_POINTS = 300;

/** Latest values plus a short in-UI history per sensor, for sparklines. */
export class LiveStore {
  readonly capacity: number;
  schema = $state.raw<Schema | null>(null);
  values = $state.raw<(number | null)[]>([]);
  timestampMs = $state(0);
  firstTimestampMs = $state(0);
  /** Bumped whenever series change, so readers of `series()` re-run. */
  #tick = $state(0);
  #lastSeq = -1;
  #index = new Map<string, number>();
  #series = new Map<string, SeriesBuffer>();

  constructor(capacity = SPARKLINE_POINTS) {
    this.capacity = capacity;
  }

  applySchema(schema: Schema): void {
    if (this.schema && schema.revision <= this.schema.revision) return;
    this.schema = schema;
    this.#index = new Map(schema.sensors.map((s, i) => [s.id, i]));
    for (const id of [...this.#series.keys()]) {
      if (!this.#index.has(id)) this.#series.delete(id);
    }
    for (const s of schema.sensors) {
      if (!this.#series.has(s.id)) {
        const buffer = new SeriesBuffer(this.capacity);
        const previousLength = Math.max(0, ...[...this.#series.values()].map((b) => b.length));
        for (let i = 0; i < previousLength; i++) buffer.push(null);
        this.#series.set(s.id, buffer);
      }
    }
    this.values = schema.sensors.map(() => null);
    this.#tick++;
  }

  /** Returns false when the snapshot belongs to another schema revision. */
  applySnapshot(snapshot: Snapshot): boolean {
    const schema = this.schema;
    if (!schema || snapshot.revision !== schema.revision || snapshot.values.length !== schema.sensors.length) {
      return false;
    }
    if (snapshot.seq <= this.#lastSeq) return true; // Duplicate/out-of-order event.
    if (snapshot.timestampMs < this.timestampMs) {
      for (const buffer of this.#series.values()) buffer.clear();
      this.firstTimestampMs = snapshot.timestampMs;
    }
    this.#lastSeq = snapshot.seq;
    this.values = snapshot.values;
    schema.sensors.forEach((s, i) => this.#series.get(s.id)?.push(snapshot.values[i]));
    this.timestampMs = snapshot.timestampMs;
    if (this.firstTimestampMs === 0) this.firstTimestampMs = snapshot.timestampMs;
    this.#tick++;
    return true;
  }

  /** Replaces sparkline buffers with history fetched from the core. */
  seedHistory(ids: string[], history: HistorySeed): void {
    if (history.revision !== this.schema?.revision) return;
    this.#lastSeq = history.seq;
    ids.forEach((id, k) => {
      const buffer = this.#series.get(id);
      if (!buffer) return;
      buffer.clear();
      for (const v of history.series[k] ?? []) buffer.push(v);
    });
    const last = history.timestampsMs.at(-1);
    if (last !== undefined) {
      this.timestampMs = last;
      this.values = ids.map((_, i) => history.series[i]?.at(-1) ?? null);
    }
    const first = history.timestampsMs[0];
    if (first !== undefined && (this.firstTimestampMs === 0 || first < this.firstTimestampMs)) {
      this.firstTimestampMs = first;
    }
    this.#tick++;
  }

  value(id: string): number | null {
    const i = this.#index.get(id);
    return i === undefined ? null : (this.values[i] ?? null);
  }

  series(id: string): number[] {
    void this.#tick;
    return this.#series.get(id)?.toArray() ?? [];
  }
}

/** Wires a store to a backend: seeds history, follows schema and snapshot events. */
export async function connect(store: LiveStore, backend: Backend): Promise<Unsubscribe> {
  let stopped = false;
  let refreshing: Promise<void> | null = null;
  let initializing = true;
  let requestedRevision = 0;
  let queue: Snapshot[] = [];
  const off: Unsubscribe[] = [];
  const stop = () => { stopped = true; off.splice(0).forEach((fn) => fn()); queue = []; };
  const refresh = (): Promise<void> => {
    refreshing ??= (async () => {
      do {
        const schema = await backend.getSchema();
        const ids = schema.sensors.map((s) => s.id);
        const history = await backend.getHistory(ids, store.capacity);
        if (stopped) return;
        // Hardware may change between the two commands; never seed a different schema.
        if (history.revision !== schema.revision || schema.revision < requestedRevision) continue;
        store.applySchema(schema);
        store.seedHistory(ids, history);
        for (const snapshot of queue.sort((a, b) => a.seq - b.seq)) {
          if (snapshot.revision === schema.revision && snapshot.seq > history.seq) store.applySnapshot(snapshot);
        }
        queue = [];
        initializing = false;
        return;
      } while (!stopped);
    })().finally(() => { refreshing = null; });
    return refreshing;
  };
  const recover = () => { void refresh().catch((error) => { stop(); console.error('backend refresh failed', error); }); };
  try {
    off.push(await backend.onSchema((schema) => {
      if (stopped || schema.revision <= (store.schema?.revision ?? 0)) return;
      requestedRevision = Math.max(requestedRevision, schema.revision);
      if (!initializing) recover();
    }));
    off.push(await backend.onSnapshot((snapshot) => {
      if (stopped || snapshot.revision < (store.schema?.revision ?? 0)) return;
      requestedRevision = Math.max(requestedRevision, snapshot.revision);
      if (initializing || refreshing || snapshot.revision !== store.schema?.revision) {
        queue.push(snapshot);
        if (queue.length > store.capacity) queue.shift();
        if (!initializing) recover();
      } else if (!store.applySnapshot(snapshot)) recover();
    }));
    await refresh();
    return stop;
  } catch (error) {
    stop();
    throw error;
  }
}
```

`app/src/lib/select.ts`:

```ts
import type { DeviceKind, Schema, Sensor } from './types';

export type ValueOf = (id: string) => number | null;

const devicesOf = (schema: Schema, kind: DeviceKind) => schema.devices.filter((d) => d.kind === kind);

const sensorsWith = (schema: Schema, deviceIds: string[], key: string): Sensor[] =>
  schema.sensors.filter((s) => deviceIds.includes(s.deviceId) && s.label.key === key);

function sum(values: (number | null)[]): number | null {
  const present = values.filter((v): v is number => v !== null);
  return present.length ? present.reduce((a, b) => a + b, 0) : null;
}

const read = (valueOf: ValueOf, sensor: Sensor | undefined) => (sensor ? valueOf(sensor.id) : null);

export interface CpuSummary {
  name: string;
  load: number | null;
  loadId: string | null;
  clockMhz: number | null;
}

export function cpuSummary(schema: Schema, valueOf: ValueOf): CpuSummary | null {
  const cpu = devicesOf(schema, 'cpu')[0];
  if (!cpu) return null;
  const load = sensorsWith(schema, [cpu.id], 'cpu.load.total')[0];
  const clock = sensorsWith(schema, [cpu.id], 'cpu.clock.effective')[0];
  return { name: cpu.name, load: read(valueOf, load), loadId: load?.id ?? null, clockMhz: read(valueOf, clock) };
}

export interface MemorySummary {
  usedBytes: number | null;
  totalBytes: number | null;
  usedPct: number | null;
  loadId: string | null;
}

export function memorySummary(schema: Schema, valueOf: ValueOf): MemorySummary | null {
  const mem = devicesOf(schema, 'memory')[0];
  if (!mem) return null;
  return {
    usedBytes: read(valueOf, sensorsWith(schema, [mem.id], 'memory.used')[0]),
    totalBytes: read(valueOf, sensorsWith(schema, [mem.id], 'memory.total')[0]),
    usedPct: read(valueOf, sensorsWith(schema, [mem.id], 'memory.load')[0]),
    loadId: sensorsWith(schema, [mem.id], 'memory.load')[0]?.id ?? null,
  };
}

export interface StorageSummary {
  readBps: number | null;
  writeBps: number | null;
  volume: { letter: string; usedPct: number | null } | null;
  readIds: string[];
}

export function storageSummary(schema: Schema, valueOf: ValueOf): StorageSummary | null {
  const ids = devicesOf(schema, 'storage').map((d) => d.id);
  if (ids.length === 0) return null;
  const volumes = sensorsWith(schema, ids, 'storage.volumeUsed');
  const system = volumes.find((s) => s.label.arg === 'C:') ?? volumes[0];
  return {
    readIds: sensorsWith(schema, ids, 'storage.read').map((s) => s.id),
    readBps: sum(sensorsWith(schema, ids, 'storage.read').map((s) => valueOf(s.id))),
    writeBps: sum(sensorsWith(schema, ids, 'storage.write').map((s) => valueOf(s.id))),
    volume: system ? { letter: system.label.arg ?? '', usedPct: valueOf(system.id) } : null,
  };
}

export interface NetworkSummary {
  downBps: number | null;
  upBps: number | null;
  downIds: string[];
}

export function networkSummary(schema: Schema, valueOf: ValueOf): NetworkSummary | null {
  const ids = devicesOf(schema, 'network').map((d) => d.id);
  if (ids.length === 0) return null;
  const down = sensorsWith(schema, ids, 'network.down');
  return {
    downBps: sum(down.map((s) => valueOf(s.id))),
    upBps: sum(sensorsWith(schema, ids, 'network.up').map((s) => valueOf(s.id))),
    downIds: down.map((s) => s.id),
  };
}

/** Element-wise sum of right-aligned series; NaN where no series has a value. */
export function sumSeries(series: number[][]): number[] {
  const length = Math.max(0, ...series.map((s) => s.length));
  return Array.from({ length }, (_, i) => {
    let total = 0;
    let any = false;
    for (const s of series) {
      const v = s[s.length - length + i];
      if (v !== undefined && Number.isFinite(v)) {
        total += v;
        any = true;
      }
    }
    return any ? total : NaN;
  });
}
```

- [ ] **Step 6: Esegui i test e verifica che passino**

Esegui: `cd app && pnpm test && pnpm check`
Risultato atteso: tutti i test PASS; `svelte-check` con `0 ERRORS`.

- [ ] **Step 7: Commit**

```bash
git add app
git commit -m "feat(ui): backend abstraction, live store and summary selectors"
```

---

### Task 12: componenti della vista Semplificata, barra superiore e `App`

**File:**
- Crea:
  - `app/src/lib/sparkline.ts`, `app/src/lib/health.ts`
  - `app/src/components/common/Sparkline.svelte`, `app/src/components/common/AnimatedNumber.svelte`
  - `app/src/components/simple/HealthBanner.svelte`, `app/src/components/simple/Tile.svelte`, `app/src/components/simple/SimpleView.svelte`
  - `app/src/components/advanced/AdvancedPlaceholder.svelte`
  - `app/src/components/TopBar.svelte`, `app/src/App.svelte`, `app/src/main.ts`
- Test: `app/src/lib/sparkline.test.ts`, `app/src/App.test.ts`

**Interfacce:**
- Usa:
  - `LiveStore`, `connect` (Task 11)
  - `createBackend`, `Backend` (Task 11)
  - i selettori (Task 11)
  - `t`, `i18n`, `detectLocale` (Task 10)
  - `format*` (Task 10)
  - `View` (Task 10)
- Produce:
  - `sparklinePath(values: number[], width: number, height: number, min?: number, max?: number, capacity?: number): string`
  - `HealthState { level: 'neutral' | 'ok' | 'warn' | 'crit'; messageKey: string; params?: Params; sinceMs: number }` e `monitoringHealth(startedAtMs)`
  - il componente `App`, con props opzionali `backend` e `store` per i test

**Note:**
- Il banner di M1 è neutro: non assegna lo stato `ok` senza il motore regole. Dice solo "Monitoraggio attivo · da N min" dopo il primo campione. Il motore regole arriva con la M5 e sostituirà `monitoringHealth`.
- Il badge "Modalità base" è sempre visibile, perché il servizio non esiste ancora (M4). Il badge è un disclosure accessibile già in M1; l’installazione/avvio del servizio arriva in M4.
- Il tile GPU arriva con la M2.

- [ ] **Step 1: Scrivi i test (falliscono)**

`app/src/lib/sparkline.test.ts`:

```ts
import { sparklinePath } from './sparkline';

test('draws a line scaled to min/max', () => {
  expect(sparklinePath([0, 50, 100], 100, 10, 0, 100)).toBe('M0 10L50 5L100 0');
});

test('gaps split the line', () => {
  expect(sparklinePath([0, NaN, 100], 100, 10, 0, 100)).toBe('M0 10M100 0');
});

test('short series are right-aligned to the capacity', () => {
  expect(sparklinePath([100], 100, 10, 0, 100, 3)).toBe('M100 0');
});

test('empty or all-missing series draw nothing', () => {
  expect(sparklinePath([], 100, 10)).toBe('');
  expect(sparklinePath([NaN, NaN], 100, 10)).toBe('');
});

test('flat series sit on the baseline', () => {
  expect(sparklinePath([0, 0], 100, 10)).toBe('M0 10L100 10');
});
```

`app/src/App.test.ts`:

```ts
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n } from './lib/i18n/index.svelte';
beforeEach(() => { localStorage.clear(); i18n.locale = 'en'; });
afterEach(cleanup);
import { flushSync } from 'svelte';
import App from './App.svelte';
import { MOCK_SCHEMA, mockValues } from './lib/backend/mock';
import { LiveStore } from './lib/live.svelte';
import { FakeBackend } from './test/fake-backend';

test('simple view shows the banner and the CPU, RAM and network tiles', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: Date.now(), values: mockValues(1) });
  flushSync();

  expect(screen.getByText('Monitoring active')).toBeTruthy();
  expect(screen.getByText('Basic mode')).toBeTruthy();
  expect(screen.getByText('CPU')).toBeTruthy();
  expect(screen.getByText('RAM')).toBeTruthy();
  expect(screen.getByText('Network · Disks')).toBeTruthy();
});

test('clicking a tile opens the advanced view, the toggle goes back', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  flushSync();

  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: 1000, values: mockValues(1) });
  flushSync();
  await fireEvent.click(screen.getByText('CPU').closest('button')!);
  expect(screen.getByText('The Advanced view arrives in milestone 3.')).toBeTruthy();
  await fireEvent.click(screen.getByRole('tab', { name: 'Simple' }));
  expect(screen.getByText('Monitoring active')).toBeTruthy();
});
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cd app && pnpm test`
Risultato atteso: FAIL (`Failed to resolve import "./sparkline"`, `"./App.svelte"`).

- [ ] **Step 3: Implementa le funzioni pure**

`app/src/lib/sparkline.ts`:

```ts
const round = (n: number) => Math.round(n * 10) / 10;

/**
 * SVG path for a sparkline. NaN values break the line; series shorter than
 * `capacity` are right-aligned so new samples always enter from the right.
 */
export function sparklinePath(
  values: number[],
  width: number,
  height: number,
  min = 0,
  max?: number,
  capacity = values.length,
): string {
  const finite = values.filter(Number.isFinite);
  if (finite.length === 0) return '';
  const hi = max ?? Math.max(min, ...finite);
  const span = hi - min || 1;
  const step = capacity > 1 ? width / (capacity - 1) : 0;
  const offset = capacity - values.length;
  let path = '';
  let drawing = false;
  values.forEach((v, i) => {
    if (!Number.isFinite(v)) {
      drawing = false;
      return;
    }
    const clamped = Math.min(Math.max(v, min), hi);
    const x = (offset + i) * step;
    const y = height - ((clamped - min) / span) * height;
    path += `${drawing ? 'L' : 'M'}${round(x)} ${round(y)}`;
    drawing = true;
  });
  return path;
}
```

`app/src/lib/health.ts`:

```ts
import type { Params } from './i18n/index.svelte';

export type HealthLevel = 'neutral' | 'ok' | 'warn' | 'crit';

export interface HealthState {
  level: HealthLevel;
  messageKey: string;
  params?: Params;
  sinceMs: number;
}

/**
 * Milestone 1 has no rules engine yet (milestone 5): the banner only reports
 * that monitoring is running and for how long.
 */
export function monitoringHealth(startedAtMs: number): HealthState {
  return { level: 'neutral', messageKey: 'health.monitoring', sinceMs: startedAtMs };
}
```

- [ ] **Step 4: Implementa i componenti**

`app/src/components/common/Sparkline.svelte`:

```svelte
<script lang="ts">
  import { sparklinePath } from '../../lib/sparkline';

  let {
    values,
    capacity,
    color = 'var(--accent)',
    min = 0,
    max,
  }: { values: number[]; capacity: number; color?: string; min?: number; max?: number } = $props();

  const WIDTH = 150;
  const HEIGHT = 34;
  const path = $derived(sparklinePath(values, WIDTH, HEIGHT, min, max, capacity));
</script>

<svg class="sparkline" viewBox="0 0 {WIDTH} {HEIGHT}" preserveAspectRatio="none" aria-hidden="true">
  <path
    d={path}
    fill="none"
    stroke={color}
    stroke-width="2"
    stroke-linejoin="round"
    stroke-linecap="round"
    vector-effect="non-scaling-stroke"
  />
</svg>

<style>
  .sparkline {
    display: block;
    width: 100%;
    height: 34px;
  }
</style>
```

`app/src/components/common/AnimatedNumber.svelte`:

```svelte
<script lang="ts">
  import { cubicOut } from 'svelte/easing';
  import { Tween, prefersReducedMotion } from 'svelte/motion';

  let { value, format }: { value: number | null; format: (v: number | null) => string } = $props();

  const tween = new Tween(0, {
    duration: () => (prefersReducedMotion.current ? 0 : 300),
    easing: cubicOut,
  });
  let initialized = false;

  $effect(() => {
    if (value === null) return;
    if (!initialized) {
      initialized = true;
      tween.set(value, { duration: 0 });
    } else {
      tween.target = value;
    }
  });
</script>

<span>{value === null ? format(null) : format(tween.current)}</span>
```

`app/src/components/simple/HealthBanner.svelte`:

```svelte
<script lang="ts">
  import { formatDuration } from '../../lib/format';
  import type { HealthState } from '../../lib/health';
  import { t } from '../../lib/i18n/index.svelte';

  let { health, nowMs }: { health: HealthState; nowMs: number } = $props();
</script>

<section
  class="banner"
  class:ok={health.level === 'ok'}
  class:warn={health.level === 'warn'}
  class:crit={health.level === 'crit'}
  role="status"
>
  <div class="dot" aria-hidden="true">{health.level === 'neutral' ? '•' : health.level === 'ok' ? '✓' : '!'}</div>
  <div>
    <div class="title">{t(health.messageKey, health.params)}</div>
    <div class="sub">{t('health.since', { duration: formatDuration(nowMs - health.sinceMs, t) })}</div>
  </div>
</section>

<style>
  .banner {
    --state: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 16px 18px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: linear-gradient(135deg, color-mix(in srgb, var(--state) 14%, var(--surface)), var(--surface));
  }
  .ok { --state: var(--ok); }
  .warn {
    --state: var(--warn);
  }
  .crit {
    --state: var(--crit);
  }
  .dot {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    font-size: 20px;
    color: var(--state);
    background: color-mix(in srgb, var(--state) 18%, transparent);
  }
  .title {
    font-size: 20px;
    font-weight: 600;
  }
  .sub {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
```

`app/src/components/simple/Tile.svelte`:

```svelte
<script lang="ts">
  import type { Snippet } from 'svelte';

  let { label, onclick, children }: { label: string; onclick?: () => void; children: Snippet } = $props();
</script>

<button class="tile" type="button" {onclick}>
  <div class="label">{label}</div>
  {@render children()}
</button>

<style>
  .tile {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
    padding: 14px;
    text-align: left;
    cursor: pointer;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    transition: border-color 0.15s;
  }
  .tile:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .tile:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
```

`app/src/components/simple/SimpleView.svelte`:

```svelte
<script lang="ts">
  import { formatBytes, formatClock, formatPercent, formatRate } from '../../lib/format';
  import { monitoringHealth } from '../../lib/health';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import { cpuSummary, memorySummary, networkSummary, storageSummary, sumSeries } from '../../lib/select';
  import AnimatedNumber from '../common/AnimatedNumber.svelte';
  import Sparkline from '../common/Sparkline.svelte';
  import HealthBanner from './HealthBanner.svelte';
  import Tile from './Tile.svelte';

  let { store, onOpenAdvanced }: { store: LiveStore; onOpenAdvanced: () => void } = $props();

  const valueOf = (id: string) => store.value(id);
  const locale = $derived(i18n.locale);
  const cpu = $derived(store.schema ? cpuSummary(store.schema, valueOf) : null);
  const mem = $derived(store.schema ? memorySummary(store.schema, valueOf) : null);
  const disk = $derived(store.schema ? storageSummary(store.schema, valueOf) : null);
  const net = $derived(store.schema ? networkSummary(store.schema, valueOf) : null);
  const netSeries = $derived(net ? sumSeries(net.downIds.map((id) => store.series(id))) : []);
  const health = $derived(monitoringHealth(store.firstTimestampMs));
</script>

<div class="simple">
  {#if store.timestampMs > 0}<HealthBanner {health} nowMs={store.timestampMs} />{/if}

  <div class="grid">
    {#if cpu}
      <Tile label={t('tile.cpu')} onclick={onOpenAdvanced}>
        <div class="big"><AnimatedNumber value={cpu.load} format={(v) => formatPercent(v, locale)} /></div>
        <div class="sub">{cpu.name} · {formatClock(cpu.clockMhz, locale)}</div>
        {#if cpu.loadId}
          <Sparkline values={store.series(cpu.loadId)} capacity={store.capacity} max={100} />
        {/if}
      </Tile>
    {/if}

    {#if mem}
      <Tile label={t('tile.memory')} onclick={onOpenAdvanced}>
        <div class="big">
          {formatBytes(mem.usedBytes, locale)} <span class="unit">/ {formatBytes(mem.totalBytes, locale)}</span>
        </div>
        <div class="bar" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={mem.usedPct ?? 0}>
          <i style:width="{mem.usedPct ?? 0}%"></i>
        </div>
        <div class="sub">{formatPercent(mem.usedPct, locale)}</div>
        {#if mem.loadId}<Sparkline values={store.series(mem.loadId)} capacity={store.capacity} max={100} />{/if}
      </Tile>
    {/if}

    {#if net || disk}
      <Tile label={t('tile.netDisk')} onclick={onOpenAdvanced}>
        {#if net}
          <div class="big rate">
            ↓ {formatRate(net.downBps, 'bits', locale)}
            <span class="unit">↑ {formatRate(net.upBps, 'bits', locale)}</span>
          </div>
          <Sparkline values={netSeries} capacity={store.capacity} color="var(--accent-2)" />
        {/if}
        {#if disk}
          {#if !net}<Sparkline values={sumSeries(disk.readIds.map((id) => store.series(id)))} capacity={store.capacity} />{/if}
          <div class="sub">
            {#if disk.volume}{disk.volume.letter} {formatPercent(disk.volume.usedPct, locale)} ·
            {/if}{t('tile.diskIo', {
              read: formatRate(disk.readBps, 'bytes', locale),
              write: formatRate(disk.writeBps, 'bytes', locale),
            })}
          </div>
        {/if}
      </Tile>
    {/if}
  </div>
</div>

<style>
  .simple {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 12px;
  }
  .big {
    font-size: 28px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .big.rate {
    font-size: 20px;
  }
  .unit {
    font-size: 14px;
    font-weight: 400;
    color: var(--text-muted);
  }
  .sub {
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar {
    height: 6px;
    margin: 6px 0 2px;
    overflow: hidden;
    border-radius: 4px;
    background: var(--border);
  }
  .bar i {
    display: block;
    height: 100%;
    border-radius: 4px;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
    transition: width 0.3s ease-out;
  }
</style>
```

`app/src/components/advanced/AdvancedPlaceholder.svelte`:

```svelte
<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
</script>

<section class="placeholder">
  <p>{t('advanced.comingSoon')}</p>
</section>

<style>
  .placeholder {
    display: grid;
    place-items: center;
    min-height: 320px;
    color: var(--text-muted);
    border: 1px dashed var(--border);
    border-radius: var(--radius);
  }
</style>
```

`app/src/components/TopBar.svelte`:

```svelte
<script lang="ts">
  import { t } from '../lib/i18n/index.svelte';
  import type { View } from '../lib/view';

  let {
    view,
    onViewChange,
    serviceAvailable,
  }: { view: View; onViewChange: (view: View) => void; serviceAvailable: boolean } = $props();
</script>

<header class="topbar">
  <div class="brand"><span class="logo" aria-hidden="true"></span>{t('app.title')}</div>

  <div class="seg" role="tablist" aria-label={t('view.label')}>
    <button role="tab" aria-selected={view === 'simple'} class:on={view === 'simple'} onclick={() => onViewChange('simple')}>
      {t('view.simple')}
    </button>
    <button
      role="tab"
      aria-selected={view === 'advanced'}
      class:on={view === 'advanced'}
      onclick={() => onViewChange('advanced')}
    >
      {t('view.advanced')}
    </button>
  </div>

  <div class="right">
    {#if !serviceAvailable}
      <details class="badge"><summary>{t('service.baseMode')}</summary><p>{t('service.baseModeHint')}</p></details>
    {/if}
    <button class="icon" type="button" disabled title={t('settings.comingSoon')} aria-label={t('settings.title')}>⚙</button>
  </div>
</header>

<style>
  .topbar {
    position: sticky;
    top: 0;
    z-index: 1;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 12px;
    padding: 12px 20px;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    border-bottom: 1px solid var(--border);
    backdrop-filter: blur(8px);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    font-weight: 600;
  }
  .logo {
    width: 14px;
    height: 14px;
    border-radius: 4px;
    background: linear-gradient(135deg, var(--accent), var(--accent-2));
  }
  .seg {
    display: flex;
    padding: 3px;
    border-radius: 10px;
    background: var(--surface-2);
  }
  .seg button {
    padding: 6px 14px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .seg button.on {
    background: var(--accent);
    color: #1a0616;
    font-weight: 600;
  }
  .right {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 10px;
  }
  .badge {
    padding: 4px 10px;
    font-size: 12px;
    border-radius: 999px;
    color: var(--warn);
    border: 1px solid color-mix(in srgb, var(--warn) 45%, transparent);
    cursor: help;
  }
  .icon {
    width: 32px;
    height: 32px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
  }
  .icon:disabled {
    opacity: 0.5;
  }
</style>
```

`app/src/App.svelte`:

```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import AdvancedPlaceholder from './components/advanced/AdvancedPlaceholder.svelte';
  import SimpleView from './components/simple/SimpleView.svelte';
  import TopBar from './components/TopBar.svelte';
  import { createBackend, type Backend } from './lib/backend';
  import { LiveStore, connect } from './lib/live.svelte';
  import type { View } from './lib/view';

  let { backend = createBackend(), store = new LiveStore() }: { backend?: Backend; store?: LiveStore } = $props();
  let view = $state<View>((localStorage.getItem('oma.view') === 'advanced') ? 'advanced' : 'simple');
  let visible = $state(!document.hidden);
  $effect(() => { localStorage.setItem('oma.view', view); });

  onMount(() => {
    const visibility = () => { visible = !document.hidden; };
    document.addEventListener('visibilitychange', visibility);
    let off: (() => void) | undefined;
    let cancelled = false;
    connect(store, backend)
      .then((unsubscribe) => {
        if (cancelled) unsubscribe();
        else off = unsubscribe;
      })
      .catch((error) => console.error('backend connection failed', error));
    return () => {
      cancelled = true;
      document.removeEventListener('visibilitychange', visibility);
      off?.();
    };
  });
</script>

<TopBar {view} onViewChange={(v) => (view = v)} serviceAvailable={false} />
<main>
  {#if visible}
  {#if view === 'simple'}
    <SimpleView {store} onOpenAdvanced={() => (view = 'advanced')} />
  {:else}
    <AdvancedPlaceholder />
  {/if}
  {/if}
</main>

<style>
  main {
    max-width: 1100px;
    margin: 0 auto;
    padding: 20px;
  }
</style>
```

`app/src/main.ts`:

```ts
import { mount } from 'svelte';
import App from './App.svelte';
import { detectLocale, i18n } from './lib/i18n/index.svelte';
import './styles/theme.css';

i18n.locale = detectLocale(navigator.languages);
mount(App, { target: document.getElementById('app')! });
```

- [ ] **Step 5: Esegui i test e verifica che passino**

Esegui: `cd app && pnpm test && pnpm check && pnpm build`
Risultato atteso:
- tutti i test PASS (compresi i 5 di `sparkline` e i 2 di `App`);
- `svelte-check` con `0 ERRORS 0 WARNINGS`;
- `vite build` crea `app/dist/`.

- [ ] **Step 6: Verifica visiva nel browser**

Esegui: `cd app && pnpm dev` e apri `http://localhost:1420`. Controlla che:
- la palette sia Synthwave;
- il banner mostri "Monitoraggio attivo" se il browser è in italiano;
- ci siano i tile CPU (con minigrafico), RAM (con barra) e Rete · Dischi;
- i valori cambino ogni secondo con una breve animazione;
- un clic su un tile porti al segnaposto della vista Avanzata.

Ferma il server con Ctrl+C.

- [ ] **Step 7: Commit**

```bash
git add app
git commit -m "feat(ui): Simple view, top bar and app shell"
```

---

### Task 13: shell Tauri (`oma-app`): comandi, eventi, tray e ciclo di vita della finestra

**File:**
- Modifica: `Cargo.toml` (radice): `members = ["crates/oma-core", "crates/oma-win", "app/src-tauri"]`
- Crea:
  - `app/app-icon.svg`
  - `app/src-tauri/Cargo.toml`, `app/src-tauri/build.rs`, `app/src-tauri/tauri.conf.json`, `app/src-tauri/capabilities/default.json`
  - `app/src-tauri/src/main.rs`, `app/src-tauri/src/commands.rs`, `app/src-tauri/src/tray.rs`, `app/src-tauri/src/window.rs`
  - `app/src-tauri/icons/*`, generate dal comando `tauri icon`
- Test: `app/src-tauri/src/tray.rs` e `app/src-tauri/src/commands.rs` (moduli `tests`)

**Interfacce:**
- Usa:
  - `Engine` (Task 4), `Sampler`, `unix_ms` (Task 5), `HistoryWindow` (Task 3), `Schema` (Task 1)
  - `oma_win::default_providers()` (Task 9)
- Produce, per la UI del Task 11:
  - il comando `get_schema() -> Schema`
  - il comando `get_history(ids: Vec<String>, seconds: u64) -> HistorySeed`
  - l'evento `oma:schema` (payload `Schema`)
  - l'evento `oma:snapshot` (payload `Snapshot`)
- Riga di comando: il flag `--minimized` avvia l'app solo nella tray.

**Comportamento (spec §2.2 e §4.5):**
- La finestra `main` viene creata nel codice, non da `tauri.conf.json`.
- Chiudere la finestra la **distrugge** (WebView2 esce). `RunEvent::ExitRequested { code: None }` viene intercettato con `prevent_exit()`, così il processo resta nella tray.
- Tray: "Apri" (o clic sinistro) ricrea o mostra la finestra; "Esci" chiama `app.exit(0)`. Le etichette sono in italiano se la lingua di sistema è italiana, altrimenti in inglese.
- Il sampler emette gli eventi **solo se la finestra esiste**. Quando il documento è nascosto, `App` smonta i grafici e le interpolazioni: i campioni continuano ad arrivare nello store ma il rendering riprende solo alla visibilità (§7.5).
- È consentita una sola istanza: una seconda esecuzione mostra la finestra esistente.

- [ ] **Step 1: Crea il crate, la configurazione e le icone**

`Cargo.toml` (radice), riga `members`:

```toml
members = ["crates/oma-core", "crates/oma-win", "app/src-tauri"]
```

`app/src-tauri/Cargo.toml`:

```toml
[package]
name = "oma-app"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[build-dependencies]
tauri-build = { version = "2.6", features = [] }

[dependencies]
oma-core.workspace = true
serde.workspace = true
tracing.workspace = true
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
serde_json.workspace = true
tauri = { version = "2.11", features = ["tray-icon"] }
tauri-plugin-single-instance = "2.4"
sys-locale = "0.3"

[target.'cfg(windows)'.dependencies]
oma-win.workspace = true
```

`app/src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&["get_schema", "get_history"]),
        ),
    ).expect("Tauri build")
}
```

`app/src-tauri/tauri.conf.json`:

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "OpenMonitor Advanced",
  "version": "0.1.0",
  "identifier": "io.github.openmonitoradvanced",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "pnpm dev",
    "beforeBuildCommand": "pnpm build"
  },
  "app": {
    "windows": [],
    "security": {
      "csp": "default-src 'self'; connect-src ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; img-src 'self' data:"
    }
  },
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.ico"]
  }
}
```

`app/src-tauri/capabilities/default.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Main window: sensor reads and event subscriptions only.",
  "windows": ["main"],
  "permissions": ["core:event:allow-listen", "core:event:allow-unlisten", "allow-get-schema", "allow-get-history"]
}
```

`app/app-icon.svg` (icona Synthwave, già provata con `tauri icon`):

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="#ff4fd8"/>
      <stop offset="1" stop-color="#4cc9f0"/>
    </linearGradient>
  </defs>
  <rect x="64" y="64" width="896" height="896" rx="200" fill="#0f0a1a"/>
  <rect x="64" y="64" width="896" height="896" rx="200" fill="none" stroke="#2d2042" stroke-width="24"/>
  <polyline points="200,560 380,560 450,380 560,700 630,480 700,560 824,560" fill="none" stroke="url(#g)" stroke-width="64" stroke-linecap="round" stroke-linejoin="round"/>
</svg>
```

Genera le icone e rimuovi quelle per le piattaforme mobili:

```bash
cd app && pnpm tauri icon app-icon.svg -o src-tauri/icons && rm -rf src-tauri/icons/android src-tauri/icons/ios
```

- [ ] **Step 2: Scrivi i test (falliscono)**

`app/src-tauri/src/tray.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn italian_locales_get_italian_labels() {
        assert_eq!(labels_for("it-IT").open, "Apri");
        assert_eq!(labels_for("it").quit, "Esci");
    }

    #[test]
    fn other_locales_fall_back_to_english() {
        assert_eq!(labels_for("en-US").open, "Open");
        assert_eq!(labels_for("de-DE").quit, "Quit");
        assert_eq!(labels_for("").open, "Open");
    }
}
```

`app/src-tauri/src/commands.rs`, parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_window_is_capped_at_one_hour() {
        assert_eq!(history_since(10_000_000, 300), 10_000_000 - 300_000);
        assert_eq!(history_since(10_000_000, 999_999), 10_000_000 - 3_600_000);
    }

    #[test]
    fn history_window_never_underflows() {
        assert_eq!(history_since(1_000, 300), 0);
    }
}
```

- [ ] **Step 3: Implementa i moduli**

`app/src-tauri/src/commands.rs`, in testa al file:

```rust
//! Tauri commands called by the UI (see app/src/lib/backend/tauri.ts).

use std::sync::PoisonError;

use oma_core::history::HistoryWindow;
use oma_core::model::Schema;
use oma_core::sampler::unix_ms;
use tauri::State;

use crate::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySeed {
    revision: u64,
    seq: u64,
    #[serde(flatten)]
    history: HistoryWindow,
}

/// Longest history the UI may request: the whole buffer (1 h).
const MAX_HISTORY_SECONDS: u64 = 3_600;

pub(crate) fn history_since(now_ms: u64, seconds: u64) -> u64 {
    now_ms.saturating_sub(seconds.min(MAX_HISTORY_SECONDS) * 1_000)
}

#[tauri::command]
pub fn get_schema(state: State<'_, AppState>) -> Schema {
    state.engine.lock().unwrap_or_else(PoisonError::into_inner).schema().clone()
}

#[tauri::command]
pub fn get_history(state: State<'_, AppState>, ids: Vec<String>, seconds: u64) -> HistorySeed {
    let since = history_since(unix_ms(), seconds);
    let engine = state.engine.lock().unwrap_or_else(PoisonError::into_inner);
    HistorySeed {
        revision: engine.schema().revision,
        seq: engine.sequence(),
        history: engine.history().window(&ids, since),
    }
}
```

`app/src-tauri/src/window.rs`:

```rust
//! Main window lifecycle: created on demand, destroyed on close so WebView2
//! releases its memory while the app keeps sampling in the tray.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const MAIN: &str = "main";

/// Shows the main window, creating it if it was closed (destroyed).
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("OpenMonitor Advanced")
        .inner_size(1100.0, 720.0)
        .min_inner_size(900.0, 600.0)
        .build();
    if let Err(err) = result {
        tracing::error!(%err, "cannot create the main window");
    }
}
```

`app/src-tauri/src/tray.rs`, in testa al file:

```rust
//! Minimal tray icon (milestone 1): open the window or quit.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use crate::window;

pub struct TrayLabels {
    pub open: String,
    pub quit: String,
}

/// The webview may be destroyed, so tray labels are localized in Rust.
pub fn labels_for(locale: &str) -> TrayLabels {
    let en: serde_json::Value = serde_json::from_str(include_str!("../../src/lib/i18n/en.json")).expect("en catalog");
    let it: serde_json::Value = serde_json::from_str(include_str!("../../src/lib/i18n/it.json")).expect("it catalog");
    let base = locale.split(['-', '_']).next().unwrap_or("");
    let catalog = if base.eq_ignore_ascii_case("it") { &it } else { &en };
    let text = |key: &str| catalog[key].as_str().or_else(|| en[key].as_str()).expect("tray key").to_owned();
    TrayLabels { open: text("tray.open"), quit: text("tray.quit") }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let labels = labels_for(&sys_locale::get_locale().unwrap_or_default());
    let open = MenuItem::with_id(app, "open", labels.open, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("bundle icon").clone())
        .tooltip("OpenMonitor Advanced")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => window::show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                window::show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}
```

`app/src-tauri/src/main.rs`:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod tray;
mod window;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use oma_core::engine::Engine;
use oma_core::provider::Provider;
use oma_core::sampler::{Sampler, history_capacity};
use tauri::{Emitter, Manager, RunEvent};

/// Default sampling interval (spec §4.1).
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const EVENT_SCHEMA: &str = "oma:schema";
const EVENT_SNAPSHOT: &str = "oma:snapshot";

pub struct AppState {
    pub engine: Arc<Mutex<Engine>>,
}

/// Owns the sampler so it can be stopped cleanly on exit.
struct SamplerGuard(Mutex<Option<Sampler>>);

fn providers() -> Vec<Box<dyn Provider>> {
    #[cfg(windows)]
    {
        oma_win::default_providers()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn main() {
    let logs = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA"))
        .join("OpenMonitorAdvanced").join("logs");
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("oma-app").max_log_files(7).build(logs).expect("diagnostic log");
    let (writer, _log_guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(writer)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "oma_core=debug,oma_app=info".into()),
        )
        .init();

    let start_minimized = std::env::args().any(|arg| arg == "--minimized");
    let engine = Arc::new(Mutex::new(Engine::new(providers(), history_capacity(SAMPLE_INTERVAL))));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| window::show_main(app)))
        .manage(AppState { engine: engine.clone() })
        .invoke_handler(tauri::generate_handler![commands::get_schema, commands::get_history])
        .setup(move |app| {
            tray::build(app.handle())?;
            if !start_minimized {
                window::show_main(app.handle());
            }
            let handle = app.handle().clone();
            let sampler = Sampler::spawn(engine, SAMPLE_INTERVAL, move |out| {
                // Nobody listens while the window is closed: skip serialization.
                if handle.get_webview_window(window::MAIN).is_none() {
                    return;
                }
                if let Some(schema) = &out.schema {
                    let _ = handle.emit(EVENT_SCHEMA, schema);
                }
                let _ = handle.emit(EVENT_SNAPSHOT, &out.snapshot);
            });
            app.manage(SamplerGuard(Mutex::new(Some(sampler))));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application");

    app.run(|app, event| match event {
        // Last window closed: keep sampling in the tray. Explicit exits carry a code.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::Exit => {
            if let Some(guard) = app.try_state::<SamplerGuard>() {
                if let Some(sampler) = guard.0.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    sampler.stop();
                }
            }
        }
        _ => {}
    });
}
```

- [ ] **Step 4: Esegui i test e verifica che passino**

`tauri::generate_context!()` richiede che `app/dist` esista al momento della compilazione:

```bash
cd app && pnpm build && cd .. && cargo test -p oma-app
```

Risultato atteso: 4 test OK (2 di `tray` e 2 di `commands`).

- [ ] **Step 5: Verifica manuale dell'app vera**

Esegui: `cd app && pnpm tauri dev`. Controlla che:
1. La finestra si apra con valori reali: nome della tua CPU, RAM, dischi, rete. I valori si aggiornano ogni secondo.
2. Chiudendo la finestra l'icona resti nella tray. In Task Manager non devono restare processi `msedgewebview2.exe` figli di `oma-app`.
3. Dopo circa 30 s, "Apri" dal menu della tray ricrei la finestra, con i minigrafici **già popolati** con la storia accumulata mentre la finestra era chiusa.
4. Una seconda esecuzione di `target\debug\oma-app.exe` porti in primo piano la finestra esistente, senza aprire una seconda istanza.
5. "Esci" chiuda il processo.

- [ ] **Step 6: Lint e commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
git add Cargo.toml Cargo.lock app/app-icon.svg app/src-tauri
git commit -m "feat(app): Tauri shell with tray, window lifecycle and live events"
```

---

### Task 14: CI, misura del budget e documentazione

**File:**
- Crea: `.github/workflows/ci.yml`, `scripts/measure-footprint.ps1`, `docs/perf-budget.md`
- Modifica: `README.md` (sezione "Performance budget")

**Interfacce:**
- Usa: tutto quanto sopra; `--minimized` (Task 13).
- Produce: la CI su ogni push e ogni PR, e lo script di misura che le milestone successive riuseranno.

- [ ] **Step 1: Scrivi il workflow di CI**

`.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  checks:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v5

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy

      - uses: Swatinem/rust-cache@v2

      - uses: pnpm/action-setup@v4
        with:
          version: 10

      - uses: actions/setup-node@v5
        with:
          node-version: 22
          cache: pnpm
          cache-dependency-path: app/pnpm-lock.yaml

      - name: Install UI dependencies
        working-directory: app
        run: pnpm install --frozen-lockfile

      # tauri::generate_context!() needs app/dist at compile time.
      - name: Build UI
        working-directory: app
        run: pnpm build

      - name: Rust format
        run: cargo fmt --all --check

      - name: Rust lint
        run: cargo clippy --workspace --all-targets -- -D warnings

      - name: Rust tests
        run: cargo test --workspace

      - name: UI type check
        working-directory: app
        run: pnpm check

      - name: UI tests
        working-directory: app
        run: pnpm test
```

- [ ] **Step 2: Scrivi lo script di misura**

`scripts/measure-footprint.ps1`:

```powershell
<#
.SYNOPSIS
  Measures OpenMonitor Advanced against the performance budget (spec §1.2).
.DESCRIPTION
  Starts the release build, waits for warm-up, then reports the app's CPU
  usage and the private working set (Task Manager "Memory" column) of the app
  and of its WebView2 child processes.
.EXAMPLE
  ./scripts/measure-footprint.ps1              # window open
  ./scripts/measure-footprint.ps1 -Minimized   # tray only
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\oma-app.exe'),
    [int]$WarmupSeconds = 15,
    [int]$SampleSeconds = 30,
    [switch]$Minimized
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path
$proc = if ($Minimized) {
    Start-Process -FilePath $exePath -ArgumentList '--minimized' -WindowStyle Hidden -PassThru
} else {
    Start-Process -FilePath $exePath -WindowStyle Hidden -PassThru
}

try {
    Start-Sleep -Seconds $WarmupSeconds
    $proc.Refresh()
    if ($proc.HasExited) { throw "The measured instance exited (another instance may already be running)." }
    $cpuStart = $proc.TotalProcessorTime
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $SampleSeconds
    $proc.Refresh()
    $cpuEnd = $proc.TotalProcessorTime
    $cpuPercent = ($cpuEnd - $cpuStart).TotalMilliseconds / $elapsed.Elapsed.TotalMilliseconds / [Environment]::ProcessorCount * 100

    # Follow the actual process tree, including renderer grandchildren.
    $processes = @(Get-CimInstance Win32_Process)
    $descendants = [Collections.Generic.HashSet[int]]::new()
    [void]$descendants.Add($proc.Id)
    do {
        $changed = $false
        foreach ($child in $processes) {
            if ($descendants.Contains([int]$child.ParentProcessId)) {
                if ($descendants.Add([int]$child.ProcessId)) { $changed = $true }
            }
        }
    } while ($changed)
    $webviews = @($processes | Where-Object {
        $_.Name -eq 'msedgewebview2.exe' -and $descendants.Contains([int]$_.ProcessId)
    })
    $ids = @($proc.Id) + @($webviews | ForEach-Object { [int]$_.ProcessId })
    $perf = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
        Where-Object { $ids -contains [int]$_.IDProcess })
    if ($perf.Count -ne $ids.Count) { throw "Missing process memory counters; measurement is invalid." }
    $appPrivate = ($perf | Where-Object { [int]$_.IDProcess -eq $proc.Id }).WorkingSetPrivate
    $totalPrivate = ($perf | Measure-Object -Property WorkingSetPrivate -Sum).Sum

    [pscustomobject]@{
        Mode              = if ($Minimized) { 'tray' } else { 'window' }
        CorePercentCpu    = [math]::Round($cpuPercent, 2)
        AppPrivateMB      = [math]::Round($appPrivate / 1MB, 1)
        WebView2Processes = $webviews.Count
        TotalPrivateMB    = [math]::Round($totalPrivate / 1MB, 1)
    } | Format-List
}
finally {
    Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
}
```

- [ ] **Step 3: Compila in release ed esegui le misure**

```bash
cd app && pnpm tauri build --no-bundle && cd ..
pwsh -File scripts/measure-footprint.ps1
pwsh -File scripts/measure-footprint.ps1 -Minimized
```

Risultato atteso, confrontato con il budget della spec §1.2:

| Modalità | Voce | Budget |
|---|---|---|
| `window` | `CorePercentCpu` | < 1 |
| `window` | `TotalPrivateMB` | < 200 |
| `tray` | `WebView2Processes` | 0 |
| `tray` | `AppPrivateMB` | < 30 |
| `tray` | `CorePercentCpu` | < 1 |

Se una voce supera il budget, **fermati e segnalalo**: non allentare il budget e non proseguire.

- [ ] **Step 4: Registra i risultati**

`docs/perf-budget.md`. Sostituisci ogni `<...>` con i numeri misurati allo Step 3 e con la macchina usata:

```markdown
# Performance budget

Budget (spec §1.2), measured with `scripts/measure-footprint.ps1` on a release build.
Memory = private working set (Task Manager "Memory" column); CPU = share of all logical processors.

| Milestone | Machine | Mode | App CPU % | App private MB | WebView2 procs | Total private MB | Budget met |
|---|---|---|---|---|---|---|---|
| M1 | <CPU, RAM, Windows version> | window | <CorePercentCpu> | <AppPrivateMB> | <WebView2Processes> | <TotalPrivateMB> | <yes/no> |
| M1 | <same machine> | tray | <CorePercentCpu> | <AppPrivateMB> | 0 | <TotalPrivateMB> | <yes/no> |

Budget: app CPU < 1 % at idle; tray < 30 MB; window open < 200 MB in total.
```

In `README.md` aggiungi prima di `## License`:

```markdown
## Performance budget

The monitor must not distort what it measures. Budgets and the latest measurements are in
`docs/perf-budget.md`; run `scripts/measure-footprint.ps1` on a release build to reproduce them.
```

- [ ] **Step 5: Verifica finale completa**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && pnpm check && pnpm test
```

Risultato atteso: tutto OK, senza avvisi.

- [ ] **Step 6: Commit**

```bash
git add .github scripts docs/perf-budget.md README.md
git commit -m "ci: Windows checks, footprint measurement and M1 budget results"
```

---

## Verifiche di regressione della revisione

**Evidenza raccolta durante questa revisione:** i blocchi del piano sono stati estratti in un workspace temporaneo escluso da Git. Sono passati 39 test unitari `oma-core`, 24 test unitari `oma-win`, 5 test hardware Windows eseguiti esplicitamente, 39 test frontend, Clippy con `-D warnings`, `pnpm check` e `pnpm build`. Non sono stati eseguiti la shell Tauri completa, il ciclo tray/WebView, hot-plug fisico o le misure di footprint: restano gate dell’implementazione qui sotto. Questa verifica del piano non segna come implementati i task.

- [ ] Task 4: provider lento sia in discovery sia in poll; il provider veloce continua, il ciclo resta entro il budget comune, i timeout riusano l’ultimo valore e gli errori espliciti producono `None`. Nessuna crescita di thread o richieste e uscita dell'app anche col worker bloccato.
- [ ] Task 6: test unitario del fallback e dei due stati PDH validi; test su Windows italiano da utente standard, compreso clock opzionale.
- [ ] Task 8: riavvio e hot-plug con rinumerazione; identità di disco/volume conservate, sostituzione con altro hardware distinta. Documentare dischi omessi per identità non leggibile/ambigua.
- [ ] Task 11: ritardare la risposta dello storico mentre arrivano eventi; nessun campione nuovo perso o duplicato. Cambiare revisione fra `get_schema` e `get_history`; scartare la coppia incoerente e ripetere la lettura. Rifiuti delle Promise liberano tutte le sottoscrizioni.
- [ ] Task 12: minimizzare/ripristinare la finestra senza rendering dei grafici nascosti; provare reduced-motion; verificare RAM e dischi senza rete, badge accessibile e vista ricordata dopo la ricreazione della WebView.
- [ ] Task 13: da `main` consentire solo i comandi dichiarati; da una WebView senza capability verificare il rifiuto di `get_schema` e `get_history`. Generare un valore anomalo e verificare il file ruotato in `%LOCALAPPDATA%\OpenMonitorAdvanced\logs`, anche nella build release.
- [ ] Task 14: misurare sia l’avvio `--minimized` sia la chiusura di una finestra già aperta; in entrambi i casi zero processi WebView2 discendenti. Rifiutare una misura se il processo avviato esce per single-instance o mancano contatori memoria.

Riferimenti tecnici verificati per queste correzioni: [stati dei contatori PDH](https://learn.microsoft.com/en-us/windows/win32/perfctrs/checking-pdh-interface-return-values), [capability e `AppManifest::commands` in Tauri](https://v2.tauri.app/security/capabilities/). Le firme Win32 sono state confrontate anche con i sorgenti locali di `windows` 0.62.2.

## Copertura della spec (auto-revisione)

| Spec | Dove |
|---|---|
| §2 processi: nucleo Rust nel processo app, WebView distrutta alla chiusura | Task 13 (verificato anche allo Step 5 e nel Task 14) |
| §2.3 predisposizione per Linux (trait `Provider`, codice Windows isolato) | Task 2, Task 6 (`#![cfg(windows)]`), Task 13 (`providers()` con `cfg`) |
| §3 modello dati, ID stabili, merge con fonte | Task 1 (metadati opzionali e categoria), Task 8 Step 3a (identità persistenti). Il merge tra più fonti arriva con la M2, quando ci saranno fonti concorrenti; in M1 ogni sensore ha una sola fonte, comunque registrata in `source`. |
| §4.1 un solo timer, niente risoluzione del timer, contatori cumulativi, dati lenti | Task 4 (worker persistenti, deadline comune, timeout), Task 5 (intervallo validato e capacità derivata), Task 2 (`CounterRate`). La frequenza ridotta per SMART e SMBIOS arriverà con quei dati (M3/M4). |
| §4.2 storico di 1 h, buchi nel grafico | Task 3, Task 4, Task 13. Min/max/media: M3. |
| §4.5 tray minima, istanza singola | Task 13 |
| §5.1 CPU, RAM, dischi, rete | Task 6–9 |
| §7.1 barra superiore con badge "Modalità base" | Task 12 |
| §7.2 vista Semplificata B (senza GPU e senza regole) | Task 12 |
| §7.5 palette e animazioni | Task 10 (tema), Task 12 (`AnimatedNumber`, `prefers-reduced-motion`) |
| §7.6 i18n it/en, test di parità delle chiavi | Task 10, Task 13 (tray legge gli stessi JSON) |
| §8 provider isolati con backoff, dati anomali e log ruotato | Task 4 (tempo monotono e reset dopo poll riuscito), Task 2, Task 13 |
| §9 UI non elevata, CSP, nessun contenuto remoto | Task 13 (`tauri.conf.json`, capability per `main` con soli listen/unlisten e comandi applicativi dichiarati in `AppManifest`) |
| §12 test (engine con provider finto, backend finto per la UI, smoke test, CI, budget) | Task 1–14 |
| §14 milestone 1 | Tutto il piano |
