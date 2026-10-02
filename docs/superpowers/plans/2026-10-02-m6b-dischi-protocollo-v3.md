# M6b — Dischi e protocollo v3: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** con l'app aperta un HDD in standby resta in standby e Windows riesce a spegnerlo; una chiavetta USB non tiene più spento lo SMART di tutti i dischi; app e interfaccia conoscono lo stato di ogni disco.

**Architecture:** il provider `storage` del nucleo smette di interrogare la temperatura dei dischi rotazionali se non dopo attività recente, e diventa l'unico proprietario della temperatura principale del disco, che prende dal servizio quando c'è. Il servizio aggiunge il fallback SAT a `CHECK POWER MODE`, tiene spento di default lo SMART dei dischi USB e pubblica nel protocollo v3 lo stato per disco e un flag `held` per valore. La qualità dei valori diventa per sensore (`Fresh`, `Held`, `Suspended`) dal provider fino all'interfaccia e alle regole.

**Tech Stack:** Rust 1.90 (`oma-core`, `oma-ipc`, `oma-win`, shell Tauri 2.11), .NET 10 (`oma-service`, xUnit v3), Svelte 5 + TypeScript 6 (Vitest), MessagePack.

**Spec:** `docs/superpowers/specs/2026-10-02-m6b-dischi-protocollo-v3-design.md` (commit `c9b67d7`). Riferimento: `docs/superpowers/references/m5/f1-service-reconfiguration.md`.

**Branch:** `feat/m6b-dischi-protocollo-v3`, aperto da `main` con `superpowers:using-git-worktrees`.

## Global Constraints

- Codice, commenti e commit in inglese (conventional commits); prosa dei documenti in italiano con gli accenti. Fine riga LF.
- TDD: prima il test che fallisce. Dal codice FFI si estraggono helper puri; i test hardware sono `#[ignore = "requires real Windows hardware"]`.
- FFI Rust: `// SAFETY:` su ogni `unsafe`, assert di dimensione per ogni struct FFI. P/Invoke .NET: assert di layout nei test (`Marshal.SizeOf`, `Marshal.OffsetOf`).
- Protocollo: mai `skip_serializing_if`; chiavi sempre presenti, `nil` per gli assenti; fixture solo con `OMA_WRITE_FIXTURES=1` a thread singolo (`protocol/fixtures/README.md`). `PROTOCOL_VERSION = 3` sui due lati; `PIPE_NAME` invariato.
- Nessun comando verso un disco oltre a quelli elencati nella spec: accesso 0 per le query di proprietà; `CHECK POWER MODE` solo nel servizio.
- Mai eseguire sul PC di sviluppo: test Pester `Integration`, installer, input sintetico. Le verifiche dal vivo le esegue l'utente; gli agenti preparano i comandi.
- Push e scritture su GitHub solo su richiesta dell'utente.
- Orientamento nel codice: `graphify query "<domanda>"`, `graphify explain "<simbolo>"` prima di grep; dopo modifiche al codice `PYTHONHASHSEED=0 graphify update .`.
- Valori fissati dalla spec: finestra di attività 10 s; periodo della temperatura 30 s; via SAT "nessuna" riprovata al più ogni 5 minuti; timeout del pass-through 5 s; 32 byte di sense; al massimo 64 chiavi per elenco.
- Verifica di ogni task prima del commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, i test del linguaggio toccato.

## Review Focus

1. **Disco già fermo all'avvio dell'app, servizio collegato.** Nessuna query locale, nessun risveglio; la temperatura compare alla prima misura del servizio (Task 12, `a_disk_asleep_at_startup_is_never_queried`).
2. **Servizio che si scollega mentre l'HDD dorme.** Lo stato del servizio perde autorità subito, non parte una lettura forzata, un allarme attivo non si chiude (Task 12, `a_stale_feed_has_no_authority`; Task 2, `an_active_alert_survives_suspension`).
3. **Hot-plug: un disco diverso sullo stesso `PhysicalDriveN`.** Non eredita cache della temperatura, finestra di attività, via SAT ricordata né stato del servizio (Task 3, `a_new_device_id_starts_without_cache`; Task 7, `ANewModelOrSerialForgetsTheRoute`; Task 11, `a_reused_drive_number_with_another_key_is_not_associated`).
4. **Ripresa dalla sospensione del PC.** L'attività di prima della sospensione non autorizza una lettura al risveglio (Task 3, `activity_before_a_suspend_does_not_count`).
5. **Due client con scelte diverse.** Un client che non ha abilitato un disco USB non ne mostra lo SMART anche se un altro client lo tiene acceso (Task 11, `a_usb_disk_enabled_by_another_client_is_filtered_locally`).

---

## Mappa dei file

| File | Responsabilità in M6b |
|---|---|
| `crates/oma-core/src/provider.rs`, `worker.rs`, `engine.rs` | qualità per sensore |
| `crates/oma-core/src/rules/instance.rs`, `health.rs` | `Suspended` nelle regole |
| `crates/oma-core/src/settings/` | `sources.smartEnabledDrives` |
| `crates/oma-win/src/storage_gate.rs` (nuovo) | decisione pura: classe del disco, attività, piano di lettura |
| `crates/oma-win/src/storage.rs`, `storage_ioctl.rs` | integrazione del gate, tabella degli stati, feed del servizio |
| `crates/oma-win/src/svc/drives.rs` (nuovo), `svc/provider.rs`, `svc/link.rs` | associazione disco↔servizio, qualità dai flag `held`, `Subscribe` v3 |
| `crates/oma-ipc/src/message.rs`, `status.rs`, `frame.rs`, `lib.rs`, `tests/fixtures.rs` | protocollo v3 |
| `service/.../Protocol/` | protocollo v3 lato .NET |
| `service/.../Sensors/SatSense.cs` (nuovo), `DiskPowerProbe.cs` | fallback SAT |
| `service/.../Sensors/DriveStates.cs` (nuovo), `SensorHub.cs`, `FeedRequest.cs`, `IHardwareTree.cs` | elenco `drives`, USB spento di default, valori `held` |
| `app/src-tauri/src/main.rs`, `service.rs`, `commands.rs` | qualità e stati verso l'interfaccia |
| `app/src/lib/`, `app/src/components/` | etichette di stato, «Ultima lettura», vista Fonti, interruttore USB |

---

### Task 1: Qualità per sensore nel nucleo

**Files:**
- Modify: `crates/oma-core/src/provider.rs`, `crates/oma-core/src/worker.rs:14-62`, `crates/oma-core/src/engine.rs:19-30,44,190-210,258-286`
- Test: `crates/oma-core/src/engine.rs` (`mod tests`, struct `Script` a `:328`)

**Interfaces:**
- Produces:
  ```rust
  // provider.rs (engine.rs re-exports it: `pub use crate::provider::Quality;`)
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum Quality { Fresh, Held, Suspended }

  pub trait Provider: Send {
      // existing methods unchanged
      /// Per-value quality of the last `poll`, aligned with its values.
      /// `None`: every value follows `repeated()`.
      fn quality(&self) -> Option<Vec<Quality>> { None }
  }
  ```
  `Sample.repeated: bool` diventa `Sample.quality: Vec<Quality>` (stessa lunghezza di `values`). `Slot.held: bool` diventa `Slot.quality: Vec<Quality>`.
- `Suspended` significa: la fonte non misura di proposito (disco fermo); può accompagnare un valore assente.

- [ ] **Step 1: test che falliscono** in `engine.rs`, con `Script` esteso da `quality: Option<Vec<Quality>>`:

```rust
#[test]
fn per_sensor_quality_marks_only_the_flagged_value() {
    // inventory("d", &["a", "b"]); poll Ok(vec![Some(1.0), Some(2.0)]);
    // script.quality = Some(vec![Quality::Fresh, Quality::Held]);
    assert_eq!(out.quality, vec![Quality::Fresh, Quality::Held]);
}

#[test]
fn suspended_is_reported_even_without_a_value() {
    // poll Ok(vec![None]); script.quality = Some(vec![Quality::Suspended]);
    assert_eq!(out.quality, vec![Quality::Suspended]);
}

#[test]
fn a_quality_vector_of_the_wrong_length_falls_back_to_repeated() {
    // two values, quality Some(vec![Quality::Held]), repeated = false
    assert_eq!(out.quality, vec![Quality::Fresh, Quality::Fresh]);
}

#[test]
fn a_timed_out_slot_holds_its_values_and_keeps_suspended() {
    // first tick: values [Some(1.0), Some(2.0)], quality [Fresh, Suspended]; second tick: the provider blocks
    assert_eq!(out.quality, vec![Quality::Held, Quality::Suspended]);
}
```

- [ ] **Step 2:** `cargo test -p oma-core engine` → i quattro test falliscono (non compilano).
- [ ] **Step 3: implementazione.**
  - `worker.rs`: dopo `poll()`, `quality = provider.quality().filter(|q| q.len() == values.len()).unwrap_or_else(|| vec![if provider.repeated() { Held } else { Fresh }; values.len()])`. Su `Rediscover` o errore: tutti `Fresh`.
  - `engine.rs`: campione arrivato → `slot.quality = sample.quality`; primo timeout → ogni elemento diventa `Held`, tranne i `Suspended`; secondo timeout → tutti `Fresh` (i valori sono già azzerati).
  - Qualità finale per sensore: `Suspended` resta `Suspended` anche senza valore; `Held` solo con un valore plausibile; altrimenti `Fresh`.
  - Aggiornare i costruttori di `TickOutput` in `crates/oma-core/src/sampler.rs:302`, `app/src-tauri/src/notifier.rs:353`, `app/src-tauri/src/log/session/tests.rs:343` solo se non compilano.
- [ ] **Step 4:** `cargo test --workspace` → PASS, compresi `repeated_provider_marks_its_values_held` e `timed_out_slot_marks_republished_values_held`.
- [ ] **Step 5: commit** `feat(core): carry a quality per sensor from the providers`.

---

### Task 2: `Suspended` nelle regole

**Files:**
- Modify: `crates/oma-core/src/rules/instance.rs:322-332`, `crates/oma-core/src/rules/health.rs:448-452`
- Test: gli stessi file (`Rig`, `tick_with`, schema storage inline a `health.rs:1167`)

**Interfaces:**
- Consumes: `Quality::Suspended` (Task 1).
- Produces: un sensore `Suspended` è **disponibile** per la copertura anche senza valore; non matura, non azzera i timer, non aggiorna `value` né `last_valid_ms`.

- [ ] **Step 1: test che falliscono.**

```rust
// health.rs — disk schema with `storage/0/temperature/drive`, rule builtin("disk-temp")
#[test]
fn a_suspended_disk_temperature_keeps_the_coverage_complete() {
    rig.tick_with(&[None], &[Quality::Suspended]);
    assert_eq!(rig.report().coverage, Coverage::Complete);
    assert!(rig.report().unavailable_targets.is_empty());
}

#[test]
fn a_suspended_value_does_not_refresh_last_valid() {
    rig.tick(&[Some(40.0)]);                      // at WALL0
    rig.tick_with(&[Some(40.0)], &[Quality::Suspended]);
    // the slot still reports WALL0 as its last valid time
}

#[test]
fn an_active_alert_survives_suspension() {
    // 85.0 °C fresh for 31 s -> critical alert; then 60 s of (None, Suspended)
    assert_eq!(keys(&rig.report()), vec![key("disk-temp", "storage/0/temperature/drive")]);
}

#[test]
fn an_absent_value_that_is_not_suspended_still_degrades_coverage() {
    rig.tick_with(&[None], &[Quality::Fresh]);
    assert_eq!(rig.report().coverage, Coverage::Unavailable);
}

// instance.rs
#[test]
fn suspended_without_a_value_neither_resets_nor_matures() {
    // 20 s above the threshold, 60 s of step(rule, None, Suspended, ..) == Step::Stay,
    // then 10 more seconds above: the instance enters (30 s reached)
}
```

- [ ] **Step 2:** `cargo test -p oma-core rules` → falliscono.
- [ ] **Step 3: implementazione.** In `Instance::step` il controllo `quality != Quality::Fresh => Step::Stay` precede il ramo del valore assente. In `health.rs:448`: `slot.available = slot.instance.problem.is_none() && (value.is_some() || quality == Quality::Suspended)`.
- [ ] **Step 4:** `cargo test -p oma-core` e `cargo test -p oma-core --test rules_alloc` → PASS.
- [ ] **Step 5: commit** `feat(rules): treat a suspended sensor as covered`.

---

### Task 3: Gate locale della temperatura dei dischi

**Files:**
- Create: `crates/oma-win/src/storage_gate.rs`
- Modify: `crates/oma-win/src/lib.rs` (modulo, `ServiceHandles`), `crates/oma-win/src/storage_ioctl.rs` (seek penalty), `crates/oma-win/src/storage.rs:103-125,264-300,313-481`, `app/src-tauri/src/main.rs:157,197-200`
- Test: `storage_gate.rs` (`mod tests`), `storage.rs` (`mod tests`)

**Interfaces:**
- Produces (tutto `pub(crate)` salvo dove indicato):
  ```rust
  pub enum DiskClass { NonRotational, RotationalOrUnknown }
  /// NonRotational: bus 14, 15, 16 (virtual, file-backed virtual, Storage Spaces), 17 (NVMe),
  /// or `seek_penalty == Some(false)`.
  pub fn disk_class(bus_type: Option<i32>, seek_penalty: Option<bool>) -> DiskClass;

  pub const ACTIVITY_WINDOW: Duration = Duration::from_secs(10);
  #[derive(Default)] pub struct Activity { /* last active poll, last poll */ }
  impl Activity {
      /// `read`/`write`: the PDH rates of this poll; `None` on the warm-up poll or when missing.
      pub fn observe(&mut self, read: Option<f64>, write: Option<f64>, at: &Stamp);
      pub fn recent(&self, at: &Stamp) -> bool;
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
  #[serde(rename_all = "camelCase")]
  pub enum DiskPower { Active, Idle, Standby, Unknown }      // pub: the shell serializes it

  pub struct ServiceTemperature { pub value: f64, pub held: bool }
  pub enum ServiceDisk {
      Absent,
      Present { state: oma_ipc::DriveState, blocks_smart: bool, temperature: Option<ServiceTemperature> },
  }
  pub enum Plan { Local, Service(ServiceTemperature), Wait }
  pub fn plan(class: DiskClass, powered_on: Option<bool>, service: &ServiceDisk, recent: bool) -> Plan;
  pub fn power(class: DiskClass, powered_on: Option<bool>, service: &ServiceDisk, recent: bool) -> DiskPower;

  // storage.rs
  #[derive(Clone, Default)] pub struct DiskStateTable(/* Arc<Mutex<..>> */);
  impl DiskStateTable {
      pub fn publish(&self, states: Vec<(String, DiskPower)>);  // (core device id, power); bumps the generation on change
      pub fn get(&self) -> (u64, Vec<(String, DiskPower)>);
  }
  // storage_ioctl.rs
  impl PhysicalDrive { pub(crate) fn seek_penalty(&self) -> Option<bool>; }  // StorageDeviceSeekPenaltyProperty, byte 8
  ```
  `ServiceHandles` guadagna `pub disk_states: storage::DiskStateTable`; `StorageProvider::new(drives: DriveIdTable, disk_states: DiskStateTable)`. `oma_ipc::DriveState` arriva nel Task 5: in questo task `ServiceDisk` si definisce con un enum locale provvisorio `DriveState { Active, Standby, Unknown, SmartOff, NoMedia }` nel modulo, sostituito nel Task 5.
- In questo task il servizio è sempre `ServiceDisk::Absent`.

**Tabella di `plan` (spec §5.2), nell'ordine di valutazione:**

| Condizione | `plan` | `power` |
|---|---|---|
| `powered_on == Some(false)` | `Wait` | `Standby` |
| servizio `NoMedia` | `Wait` | `Unknown` |
| `NonRotational` | `Local` | `Active` |
| servizio `blocks_smart` | `Wait` | `Standby` se lo stato è `Standby`, altrimenti `Unknown` |
| servizio `Standby` | `Wait` | `Standby` |
| servizio `Active` con temperatura | `Service(t)` | `Active` |
| negli altri casi, `recent` | `Local` | `Active` |
| negli altri casi, non `recent` | `Wait` | `Idle` |

- [ ] **Step 1: test che falliscono** in `storage_gate.rs`:

```rust
#[test] fn the_decision_table_matches_the_spec() { /* one assert per row above, plus:
    RotationalOrUnknown + Present{SmartOff, false, None} + recent  -> Plan::Local
    RotationalOrUnknown + Present{Active, false, None}  + !recent -> Plan::Wait / DiskPower::Idle
    RotationalOrUnknown + Present{Standby, false, None} + recent  -> Plan::Wait / DiskPower::Standby */ }
#[test] fn nvme_and_virtual_buses_are_non_rotational() { /* 14, 15, 16, 17 -> NonRotational; Some(11)+Some(true), None+None -> RotationalOrUnknown; Some(11)+Some(false) -> NonRotational */ }
#[test] fn activity_needs_a_positive_finite_rate_within_ten_seconds() { /* observe(Some(0.0), Some(4096.0)) at t; recent at t+10s is true, at t+11s false */ }
#[test] fn a_warm_up_or_missing_counter_is_not_activity() { /* observe(None, None), observe(Some(f64::NAN), None), observe(Some(0.0), Some(0.0)) -> never recent */ }
#[test] fn activity_before_a_suspend_does_not_count() { /* observe active at wall t; next observe(Some(0.0), Some(0.0)) with wall t+3600s and mono t+2s -> recent is false */ }
```

  e in `storage.rs`:

```rust
#[test] fn an_idle_hdd_is_not_picked_and_does_not_starve_other_disks() { /* two due disks, the older one Plan::Wait: the other is refreshed */ }
#[test] fn a_waiting_disk_keeps_its_last_values_and_stays_due() { /* values unchanged, read_at unchanged */ }
#[test] fn a_failed_authorized_read_gives_absent_values() { /* Plan::Local with report None -> values None, read_at = now */ }
#[test] fn a_new_device_id_starts_without_cache() { /* carry-over keyed by device id: another id on the same index gets empty positions and a default Activity */ }
#[test] fn suspended_quality_covers_only_the_temperature_sensors() { /* disk with read, write, active, drive temp, volume: only the temperature slot is Suspended when power is Idle or Standby */ }
```

- [ ] **Step 2:** `cargo test -p oma-win storage` → falliscono.
- [ ] **Step 3: implementazione.**
  - `discover`: classe del disco da `bus_type` e `seek_penalty`; per `RotationalOrUnknown` nessuna `read_temperatures` alla discovery; `temperatures` e `Activity` si conservano per i dischi con lo **stesso id di dispositivo**, non si ricreano. Per `NonRotational` resta la lettura di oggi.
  - `poll`: `activity.observe(read, write, &now)` per ogni disco (con `fresh` → `None, None`); i candidati al refresh sono i dischi scaduti il cui `plan` non è `Wait`; `Wait` non tocca `values` né `read_at`. Con `plan == Wait` e `power == Unknown` i valori di temperatura del disco diventano assenti.
  - `Provider::quality`: `Suspended` per i sensori di temperatura di un disco con `plan == Wait` e `power` `Idle` o `Standby`; `Fresh` per tutto il resto.
  - `disk_states.publish(...)` a ogni poll.
  - `main.rs`: creare la tabella accanto a `svc_feed`/`svc_drives` e passarla in `ServiceHandles`.
- [ ] **Step 4:** `cargo test -p oma-win` → PASS. `cargo test -p oma-win -- --include-ignored reads_disk_temperatures_on_this_machine` → PASS.
- [ ] **Step 5: commit** `fix(storage): read an HDD's temperature only after recent activity`.

---

### Task 4: Punto di controllo dal vivo (utente)

Nessun codice di prodotto. Serve a decidere il ramo del §4.4 della spec prima di toccare il servizio.

- [ ] **Step 1:** `cargo build -p oma-win --example m6b_wake --release` e `cd app && pnpm tauri build --no-bundle` (produce `target/release/oma-app.exe`; il servizio installato resta il 0.3.0, protocollo v2: compatibile fino al Task 5).
- [ ] **Step 2 (utente, PowerShell amministratore, app installata chiusa):**
  `pwsh -File target\spike\m6b\sat-probe.ps1 -Drive 0 -Bisect storage,poll-storage`
  Atteso: `still in standby after 50 s` su entrambe le righe.
- [ ] **Step 3 (utente):** avviare `target\release\oma-app.exe`, TR-VISION HOME chiuso, timeout disco di Windows a 60 s. Due prove, una in modalità anti-cheat e una con il servizio collegato:
  `pwsh -File target\spike\m6b\sat-probe.ps1 -Drive 0 -Standby -Method sat16 -Count 10 -IntervalSeconds 30`
  Atteso: `STANDBY` su tutte le righe.
- [ ] **Step 4 (agente, senza privilegi, mentre l'utente non tocca `D:`):** per ognuna delle due modalità osservare per 10 minuti `GetDevicePowerState` del disco 0 e il contatore `\Disco fisico(0 D:)\Trasferimenti disco/sec` (comando usato nello spike). Atteso: `on=False` entro pochi minuti.
- [ ] **Step 5:** scrivere l'esito in fondo a questo piano ("Esito del punto di controllo"). **Se con il servizio collegato Windows non spegne il disco**, il Task 10 si esegue; altrimenti si salta e lo si annota. Un esito inconcludente tiene aperto il Task 10. Se la build di sviluppo non riesce a collegarsi al servizio installato, annotarlo e ripetere gli Step 3-4 dopo il Task 12, lasciando il Task 10 aperto fino ad allora.

---

### Task 5: Protocollo v3 in `oma-ipc`

**Files:**
- Modify: `crates/oma-ipc/src/lib.rs:14-24`, `message.rs:45-84,124-129`, `frame.rs:301-315` (e i test a `:416`, `:878-907`), `status.rs:104-135,189-222`, `crates/oma-ipc/tests/fixtures.rs`, `protocol/fixtures/*.msgpack`, `protocol/fixtures/README.md`
- Modify (solo per compilare): `crates/oma-win/src/svc/link.rs:617-672`, `svc/status.rs:185`, `svc/provider.rs` e `svc/feed.rs` (letterali di `WireSnapshot`), `app/src-tauri/src/service.rs:755`, `crates/oma-win/src/storage_gate.rs` (usa `oma_ipc::DriveState`)

**Interfaces:**
- Produces:
  ```rust
  pub const PROTOCOL_VERSION: u32 = 3;

  pub struct Subscribe { pub interval_ms: u32, pub disabled_modules: Vec<String>,
                         pub smart_disabled_drives: Vec<String>, pub smart_enabled_drives: Vec<String> }
  pub struct WireDrive { pub physical_drive: u32, pub key: Option<String>, pub model: Option<String>,
                         pub state: String, pub blocks_smart: bool }
  pub struct WireServiceState { pub active_modules: Vec<String>, pub smart_disabled_drives: Vec<String>,
                                pub reconfiguration: String, pub drives: Vec<WireDrive> }
  pub struct WireSnapshot { pub seq: u64, pub timestamp_ms: u64, pub values: Vec<Option<f64>>, pub held: Vec<bool> }

  // status.rs, camelCase
  pub enum DriveState { Active, Standby, Unknown, SmartOff, NoMedia }
  impl DriveState { pub fn from_wire(value: &str) -> Self }   // "active" | "standby" | "unknown" | "smartOff" | "noMedia"; anything else -> Unknown
  pub struct SourceDrive { pub physical_drive: u32, pub device_id: Option<String>, pub model: Option<String>,
                           pub state: DriveState, pub blocks_smart: bool }
  pub struct ServiceSources { /* existing fields */ pub drives: Vec<SourceDrive>, pub smart_blocked_by: Vec<String> }
  pub struct SourceRequest { pub disabled_modules: Vec<String>, pub smart_disabled_drives: Vec<String>,
                             pub smart_enabled_drives: Vec<String> }
  ```
- `ServiceSources.smart_blocked_by` resta fino al Task 14, ricavato da `drives` (id core dei dischi con `blocks_smart` e `device_id`), per non rompere l'interfaccia a metà piano.
- `decode_payload` rifiuta (errore di messaggio non valido già esistente) uno snapshot con `held.len() != values.len()` o con `held[i]` vero e `values[i]` assente; la correzione dei valori non finiti azzera anche il loro `held`.

**Valori delle fixture** (`reference` in `tests/fixtures.rs`):
- hello: `protocol_version: PROTOCOL_VERSION`;
- subscribe: `smart_disabled_drives: [KEY_A]`, `smart_enabled_drives: [KEY_B]`;
- schema `service.drives`: `{0, Some(KEY_A), Some("Samsung SSD 990 PRO 2TB"), "smartOff", false}` e `{1, None, Some("ST2000DM008-2UB102"), "standby", true}`;
- snapshot: `held: [false, false, true, false]`; snapshot_empty: `held: []`.

- [ ] **Step 1: test che falliscono.** In `tests/fixtures.rs`: `protocol_constants_are_v3` (`assert_eq!(PROTOCOL_VERSION, 3)`), `subscribe_v3_keeps_every_key` (chiavi `["disabled_modules", "interval_ms", "smart_disabled_drives", "smart_enabled_drives"]`). In `frame.rs`: `a_snapshot_whose_held_length_differs_is_rejected`, `held_without_a_value_is_rejected`, `a_non_finite_value_loses_its_held_flag`. In `status.rs`: `an_unknown_drive_state_reads_as_unknown` (`DriveState::from_wire("spinning") == DriveState::Unknown`), e `service_status_serializes_with_pawn_io_and_sources` esteso con `"drives":[{"physicalDrive":1,"deviceId":null,"model":"ST2000DM008-2UB102","state":"standby","blocksSmart":true}]`.
- [ ] **Step 2:** `cargo test -p oma-ipc` → falliscono.
- [ ] **Step 3: implementazione** dei tipi e della validazione; `link.rs::refresh_sources` costruisce `drives` (`device_id` con `core_id_for_key`) e `smart_blocked_by`; `subscribe_message` traduce e tronca a `MAX_DRIVE_KEYS` anche `smart_enabled_drives`, e `on_drives_changed` confronta entrambi gli elenchi.
- [ ] **Step 4: rigenerare le fixture:**
  ```
  $env:OMA_WRITE_FIXTURES = '1'; cargo test -p oma-ipc --test fixtures -- --test-threads=1
  Remove-Item Env:OMA_WRITE_FIXTURES; cargo test -p oma-ipc --test fixtures
  ```
  Aggiornare in `protocol/fixtures/README.md` la nota di versione ("Protocol version 3 (M6b) added…"), il contenuto logico di subscribe, service e snapshot, e la tabella delle dimensioni.
- [ ] **Step 5:** `cargo test --workspace` → PASS (compresi `protocol_version_mismatch_is_incompatible` e gli altri test di `link.rs`). I test .NET del protocollo falliscono fino al Task 6: è atteso.
- [ ] **Step 6: commit** `feat(ipc): protocol v3 with per-drive state and held flags`.

---

### Task 6: Protocollo v3 nel servizio

**Files:**
- Modify: `service/OpenMonitorAdvanced.Service/Protocol/Messages.cs:23-47`, `MessageCodec.cs:97-146,428-507,572-624,915-978`, `ProtocolConstants.cs:11`, `Sensors/FeedRequest.cs:30`, `Sensors/SensorHub.cs:958,1238-1251,1554-1558`, `Pipe/ClientSession.cs`
- Test: `service/OpenMonitorAdvanced.Service.Tests/Protocol/CodecTests.cs`, `Pipe/PipeListenerTests.cs:29,168,249,389`, `Sensors/SensorHubTests.cs:1240,2001-2028`

**Interfaces:**
- Produces:
  ```csharp
  public const uint Version = 3;                       // ProtocolConstants
  public sealed record WireDrive(uint PhysicalDrive, string? Key, string? Model, string State, bool BlocksSmart);
  public sealed record ServiceStateBlock(IReadOnlyList<string> ActiveModules, IReadOnlyList<string> SmartDisabledDrives,
      string Reconfiguration, IReadOnlyList<WireDrive> Drives);
  public sealed record SubscribeMessage(uint IntervalMs, IReadOnlyList<string> DisabledModules,
      IReadOnlyList<string> SmartDisabledDrives, IReadOnlyList<string> SmartEnabledDrives);
  public sealed record SnapshotMessage(ulong Seq, ulong TimestampMs, IReadOnlyList<double?> Values, IReadOnlyList<bool> Held);
  ```
- Chiavi sul filo, nell'ordine dei campi Rust del Task 5: `smart_enabled_drives`, `drives` (`physical_drive`, `key`, `model`, `state`, `blocks_smart`), `held`.
- Provvisorio fino al Task 8: `Drives` contiene solo i dischi bloccanti del gate (`state` = `"standby"` o `"unknown"`, `BlocksSmart = true`); `Held` è tutto `false`; `FeedRequest.From` ignora `SmartEnabledDrives`.

- [ ] **Step 1: test che falliscono** in `CodecTests`: `Reference` aggiornato ai valori del Task 5; `SubscribeWithoutTheV3ListIsRejected`; `AKeyInBothListsIsABadRequest` (messaggio `"a drive key cannot be both enabled and disabled"`); `TooManyEnabledDriveKeysIsABadRequest`; `ASnapshotWhoseHeldLengthDiffersIsRejected`; `HeldWithoutAValueIsRejected`; `AServiceBlockWithoutDrivesIsRejected`. In `PipeListenerTests` i letterali di versione diventano `3`. In `SensorHubTests` `GateBlockersAreReportedAsDriveKeys` diventa `GateBlockersAreReportedAsDrives`:
  ```csharp
  Assert.Equal([new WireDrive(0, HddKey, "ST2000DM008-2FR102", "standby", true)], schema.Service.Drives);
  ```
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter FullyQualifiedName~Protocol` → falliscono.
- [ ] **Step 3: implementazione.** Scrittori e lettori nel codec (serve uno scrittore di array di `bool`); `PublishGateBlockers` conserva i `DriveBlocker`, non le sole chiavi, così anche un disco senza chiave compare; `SchemaComparer.SameServiceState` confronta `Drives`.
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx` → PASS. `cargo test -p oma-ipc` → PASS.
- [ ] **Step 5: revisione di parità** con l'agente `protocol-parity-reviewer` sul diff dei Task 5 e 6; correggere quanto trova.
- [ ] **Step 6: commit** `feat(service): protocol v3 codec`.

---

### Task 7: Fallback SAT per `CHECK POWER MODE`

**Files:**
- Create: `service/OpenMonitorAdvanced.Service/Sensors/SatSense.cs`
- Modify: `Sensors/DiskPowerProbe.cs`, `Sensors/IHardwareTree.cs:69`, `Sensors/SensorHub.cs:506`
- Test: `service/OpenMonitorAdvanced.Service.Tests/Sensors/SatSenseTests.cs` (nuovo), `DiskPowerProbeTests.cs`, `SensorHubFakes.cs` (`FakeDisks`)

**Interfaces:**
- Produces:
  ```csharp
  internal static class SatSense
  {
      /// ATA status and sector count from SAT sense data (descriptor format 0x72/0x73 with a 0x09
      /// descriptor of at least 0x0C bytes, or fixed format 0x70/0x71 with ASC/ASCQ 00/1D).
      internal static bool TryReadRegisters(ReadOnlySpan<byte> sense, out byte status, out byte sectorCount);
  }
  // IDiskPowerProbe
  bool? IsSpunDown(int driveNumber, string? model, string? serial);
  // DiskPowerProbe: new test seam
  internal DiskPowerProbe(Func<IReadOnlyList<DriveFacts>> enumerateDrives, Func<int, bool?> nativeCheck,
      Func<int, bool?> satCheck, TimeProvider time, ILogger? log = null);
  ```
- Il costruttore di test esistente resta, con `satCheck = _ => null` e `TimeProvider.System`.
- CDB: `85 06 20 00 00 00 00 00 00 00 00 00 00 00 E5 00`; `IOCTL_SCSI_PASS_THROUGH = 0x0004D004`; `DataIn = 2` (`SCSI_IOCTL_DATA_UNSPECIFIED`); `SCSI_PASS_THROUGH` di 56 byte su x64 (`SenseInfoOffset` a 32, `Cdb` a 36) seguito da 32 byte di sense.

- [ ] **Step 1: test che falliscono.** `SatSenseTests`, con i vettori della spec §2:

```csharp
private static readonly byte[] SataActive  = Convert.FromHexString("72000000000000 0E 090C000000FF00FF00000000E050".Replace(" ", ""));
private static readonly byte[] SataStandby = Convert.FromHexString("72000000000000 0E 090C00000000000000000000E050".Replace(" ", ""));
private static readonly byte[] UsbActive   = Convert.FromHexString("F00001005000FF0A00000000001D00000000");

[Fact] public void ReadsTheDescriptorFormat()   // SataActive -> true, status 0x50, count 0xFF; SataStandby -> count 0x00
[Fact] public void ReadsTheFixedFormatWithTheValidBitSet()   // UsbActive -> true, status 0x50, count 0xFF
[Fact] public void AFixedFormatWithoutAtaInformationIsUnknown()   // UsbActive with byte 13 = 0x00 -> false
[Fact] public void ATruncatedOrInconsistentSenseIsUnknown()
// SataActive[..12]; SataActive with byte 7 = 0xFF; descriptor code 0x0A; descriptor length 0x04; empty span: all false, none throws
```

  `DiskPowerProbeTests`:

```csharp
[Fact] public void SatIsTriedWhenTheNativeCommandGivesNoAnswer()          // native null, sat false -> IsSpunDown == false
[Fact] public void ANativeStandbyAnswerDoesNotTryTheFallback()           // native true -> sat never called
[Fact] public void TheWorkingRouteIsRememberedPerDrive()                 // second call: native not called again for that drive
[Fact] public void WhenTheRememberedRouteStopsAnsweringTheOtherIsTried()
[Fact] public void ADeadRouteIsRetriedOnlyAfterFiveMinutes()             // FakeTimeProvider: 4 min 59 s -> no calls, null; 5 min -> both tried
[Fact] public void ANewModelOrSerialForgetsTheRoute()
[Fact] public void ScsiPassThroughIsFiftySixBytesOnX64()                 // SizeOf == 56; OffsetOf(SenseInfoOffset) == 32; OffsetOf(Cdb) == 36
```

- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter "FullyQualifiedName~SatSense|FullyQualifiedName~DiskPowerProbe"` → falliscono.
- [ ] **Step 3: implementazione.** `SatSense` maschera il bit VALID (`sense[0] & 0x7F`), rispetta la lunghezza restituita e quella dichiarata (`8 + sense[7]`) e controlla i limiti di ogni descrittore. Il fallback parte quando la via nativa non dà una risposta interpretabile, anche se l'IOCTL è riuscita. Il ricordo della via è per numero di disco, con modello e seriale di quando è stato scritto. Gli errori Win32 della via SAT usano `Win32ErrorLog` con una propria etichetta.
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx` → PASS; `pwsh scripts/check-trim-warnings.ps1` → nessun avviso nuovo.
- [ ] **Step 5: commit** `feat(service): ask the power mode through SAT when ATA pass-through fails`.

---

### Task 8: Elenco `drives` e dischi USB spenti di default

**Files:**
- Create: `service/OpenMonitorAdvanced.Service/Sensors/DriveStates.cs`
- Modify: `Sensors/IHardwareTree.cs:66-136`, `Sensors/DiskPowerProbe.cs:75-108`, `Sensors/FeedRequest.cs:27-90`, `Sensors/SensorHub.cs:431-549,777,831,916-990,1238-1251,1383`
- Test: `DriveStatesTests.cs` (nuovo), `EffectiveConfigTests.cs`, `SensorHubTests.cs`, `DiskPowerProbeTests.cs`, `SensorHubFakes.cs`

**Interfaces:**
- Produces:
  ```csharp
  // DriveFacts
  public const uint BusTypeUsb = 0x07;
  public bool SmartOffByDefault => BusType == BusTypeUsb;

  public sealed record DriveCheck(DriveFacts Drive, bool Asked, bool? SpunDown)
  {
      public bool Blocks => Asked && SpunDown != false;
      public string? Key => DriveKey.Compute(Drive.Model, Drive.Serial);
  }
  // IDiskPowerProbe: GateBlockers() is replaced
  IReadOnlyList<DriveCheck> CheckGate();          // every enumerated drive; Asked only when RequiresPowerCheck
  IReadOnlyList<DriveFacts> Enumerate();          // access 0, no power command

  public sealed record FeedRequest(uint IntervalMs, ServiceModules Disabled,
      IReadOnlySet<string> SmartDisabledDrives, IReadOnlySet<string> SmartEnabledDrives);
  public sealed record EffectiveConfig(ServiceModules Enabled,
      IReadOnlySet<string> SmartDisabledDrives, IReadOnlySet<string> SmartEnabledDrives);

  internal static class DriveStates
  {
      internal const string Active = "active", Standby = "standby", Unknown = "unknown", SmartOff = "smartOff", NoMedia = "noMedia";
      /// Precedence: noMedia, smartOff, then the power answer (standby/active/unknown), active when no check is required.
      internal static string Of(DriveFacts facts, bool smartOff, bool asked, bool? spunDown);
      internal static bool IsSmartOff(DriveFacts facts, string? key, EffectiveConfig config);
  }
  ```
- `SmartEnabledDrives` effettivo: unione degli elenchi delle richieste con lo storage attivo. `SmartDisabledDrives` resta l'intersezione di oggi.
- `IsSmartOff`: storage spento, oppure chiave in `SmartDisabledDrives`, oppure `SmartOffByDefault` e chiave assente da `SmartEnabledDrives` (un disco senza chiave non si accende).

- [ ] **Step 1: test che falliscono.**

```csharp
// DriveStatesTests
[Theory] /* (availability, smartOff, asked, spunDown) -> state */
// NoMedia,any,any,any -> "noMedia"; Present,true,true,true -> "smartOff"; Present,false,true,true -> "standby";
// Present,false,true,false -> "active"; Present,false,true,null -> "unknown"; Present,false,false,null (NVMe) -> "active"

// EffectiveConfigTests
[Fact] public void ADefaultOffDriveIsEnabledIfAnyStorageSubscriberEnablesIt()
[Fact] public void EnabledDrivesOfASubscriberWithStorageOffAreIgnored()

// SensorHubTests (UsbStick(): DriveFacts with BusType 0x07, SeekPenalty null, model "SanDisk Extreme")
[Fact] public void AnActiveUsbDiskDoesNotBlockTheGateAndIsSmartOff()
// drives contains new WireDrive(4, UsbKey, "SanDisk Extreme", "smartOff", false); EnableStorageCount == 1; no Update of its root
[Fact] public void AUsbDiskInStandbyBlocksTheGateEvenWhenSmartOff()     // "smartOff", BlocksSmart true; EnableStorageCount == 0
[Fact] public void AnEnabledUsbDiskIsPowerCheckedAndUpdated()
[Fact] public void TheDriveListCoversDisksThatLhmDoesNotExpose()
[Fact] public void AfterTheGateOpensNoDriveBlocksSmart()                 // an HDD going to standby later: "standby", BlocksSmart false
[Fact] public void StorageOffKeepsTheLastDriveListWithoutDiskIo()        // every entry "smartOff"/false; DescribeCalls and SpunDownQueries unchanged
[Fact] public void StorageNeverEnabledPublishesNoDrives()
[Fact] public void ADriveStateChangeBumpsTheSchemaRevision()
[Fact] public void GateChecksAreNotRepeatedForTheDriveList()             // SpunDownQueriesOf(n) == 1 per gate round
```

- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter FullyQualifiedName~Sensors` → falliscono.
- [ ] **Step 3: implementazione.** Lo storage worker pubblica `_drives` (riferimento immutabile, volatile) insieme a `_storageCache`: con il gate chiuso dagli esiti di `CheckGate`; con il gate aperto da `Enumerate()` più le risposte già ottenute nel giro; con lo storage spento dall'ultimo elenco, senza I/O. `DiskResolution` conserva i `DriveFacts`. I punti che oggi leggono `SmartDisabledDrives` (`:491`, `:777`, `:831`) usano `DriveStates.IsSmartOff`. `FeedRequest.From` legge `SmartEnabledDrives`. `UpdateServiceState` usa `_drives`. Nessun lock tenuto durante l'I/O.
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx` → PASS (adattati `ASmartDisabledDiskStillHoldsTheD6Gate` e gli altri test del gate alla nuova interfaccia).
- [ ] **Step 5: commit** `feat(service): publish per-drive state and keep USB disks' SMART off by default`.

---

### Task 9: Valori conservati (`held`) nel servizio

**Files:**
- Modify: `service/OpenMonitorAdvanced.Service/Sensors/SensorHub.cs:305-349,431-549,909,1390`
- Test: `SensorHubTests.cs` (helper nuovo `HeldOf(updates, at, kind, name)` accanto a `ValueOf` a `:89`)

**Interfaces:**
- Consumes: `SnapshotMessage.Held` (Task 6).
- Produces: `StorageCache(long Timestamp, IReadOnlyDictionary<string, double?> Values, IReadOnlySet<string> Held)`.
- Regole:
  - disco con standby **confermato** (`IsSpunDown == true`): i suoi valori del giro precedente passano al giro nuovo, in `Held`;
  - stato ignoto, `noMedia`, errore di lettura, disco spento: nessun valore conservato;
  - un valore di storage è `held` nello snapshot se è in `Held`, oppure se quel giro è già stato pubblicato una volta; un valore `nil` non è mai `held`; i valori non di storage non sono mai `held`.

- [ ] **Step 1: test che falliscono.**

```csharp
[Fact] public void AStandbyDiskKeepsItsLastValuesAsHeld()
// round 1 active: temperature 34; round 2 spun down: ValueOf == 34 and HeldOf == true; Updates(root) unchanged
[Fact] public void ADiskAsleepFromTheStartHasNoValues()                  // value null, held false
[Fact] public void AnUnknownPowerStateDoesNotKeepValues()
[Fact] public void StorageValuesAreHeldAfterTheirFirstPublication()      // first tick after a round: false; next tick: true
[Fact] public void ANewRoundPublishesFreshValuesAgain()
[Fact] public void NonStorageValuesAreNeverHeld()
[Fact] public void KeptValuesDoNotSurviveAnIdleHub()                     // extends StorageValuesDoNotSurviveAnIdlePeriod
```

  `ASpunDownHddIsSkippedAndReadsAsMissing` (`:250`) si divide nei primi due test.
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter FullyQualifiedName~SensorHub` → falliscono.
- [ ] **Step 3: implementazione.** La finestra di `FreshStorageValues` (due giri) resta com'è: i valori conservati entrano nella cache del giro nuovo, quindi non la allungano.
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx` → PASS.
- [ ] **Step 5: commit** `feat(service): keep a sleeping disk's last values and flag them as held`.

---

### Task 10 (condizionale): filtro dell'attività nel servizio

Si esegue **solo se** il punto di controllo del Task 4 mostra che, con il servizio collegato, Windows non spegne il disco. Se si salta, scriverlo nell'esito.

**Files:**
- Create: `service/OpenMonitorAdvanced.Service/Sensors/DiskActivityProbe.cs`
- Modify: `Sensors/IHardwareTree.cs`, `Sensors/SensorHub.cs` (storage worker), `ServiceHost.cs:118`
- Test: `DiskActivityTests.cs` (nuovo), `SensorHubTests.cs`, `SensorHubFakes.cs`

**Interfaces:**
- Produces:
  ```csharp
  public interface IDiskActivityProbe { DiskCounters? Read(int driveNumber); }   // IOCTL_DISK_PERFORMANCE, access 0
  public readonly record struct DiskCounters(long ReadCount, long WriteCount);
  internal static class DiskActivity
  {
      internal static readonly TimeSpan Window = TimeSpan.FromSeconds(10);
      /// True only when both samples exist and a counter grew; a missing sample or a counter going back is not activity.
      internal static bool Between(DiskCounters? earlier, DiskCounters? later);
  }
  ```
- Lo storage worker prende un campione 10 s prima di ogni giro e uno all'inizio del giro. `Update` di un disco rotazionale o ignoto richiede stato attivo confermato **e** `DiskActivity.Between` vero; altrimenti i valori del giro precedente restano, `held`.

- [ ] **Step 1: test che falliscono:** `ACounterThatGrewIsActivity`, `AMissingBaselineIsNotActivity`, `ACounterThatWentBackIsNotActivity`; in `SensorHubTests`: `AnActiveButIdleHddIsNotUpdatedAndKeepsHeldValues`, `AnHddWithRecentIoIsUpdated`, `SolidStateDisksAreUpdatedEveryRound`, `TheWorkerWakesTenSecondsBeforeARound` (`RunStorageDue()` restituisce 20 s, poi 10 s).
- [ ] **Step 2:** eseguirli → falliscono.
- [ ] **Step 3: implementazione**, con assert di layout per `DISK_PERFORMANCE` (88 byte).
- [ ] **Step 4 (utente):** ripetere lo Step 4 del Task 4 con il servizio collegato e verificare che le letture dei contatori non impediscano lo spegnimento.
- [ ] **Step 5: commit** `feat(service): update an HDD's SMART only after recent disk activity`.

---

### Task 11: Impostazione, richiesta e provider `svc`

**Files:**
- Create: `crates/oma-win/src/svc/drives.rs`
- Modify: `crates/oma-core/src/settings/mod.rs:223-232,332`, `decode.rs:156-158`, `patch.rs:95`; `app/src-tauri/src/service.rs:212-222`; `crates/oma-win/src/storage.rs:142-190` (`DriveEntry`, `disk_properties`); `crates/oma-win/src/svc/provider.rs:37-61,119-200,242-330`; `crates/oma-win/src/svc/mod.rs`
- Test: gli stessi file

**Interfaces:**
- Produces:
  ```rust
  // settings: Sources { .., pub smart_enabled_drives: Vec<String> }   JSON "smartEnabledDrives", patch leaf
  // DriveEntry { .., pub smart_default_off: bool }                    bus type 7 (USB)
  pub const SMART_DEFAULT: &str = "smartDefault";                     // device property, value "off", only when smart_default_off

  // svc/drives.rs
  /// The service's entry for this core disk: same physical drive number, both keys present and equal,
  /// and that key unique in both tables.
  pub(crate) fn wire_drive_for<'a>(entry: &DriveEntry, drives: &DriveIds, service: &'a WireServiceState) -> Option<&'a WireDrive>;
  /// Whether this client accepts the service as a source for the disk.
  pub(crate) fn source_accepted(entry: &DriveEntry, request: &SourceRequest) -> bool;
  ```
- `request_of(settings)`: `smart_enabled_drives` = impostazione meno gli id presenti in `smart_disabled_drives`.
- `SvcProvider`:
  - `bind` scarta, per un disco associato, il sensore `temperature`/`drive` (lo possiede `storage`, Task 12) e l'intero dispositivo se `!source_accepted`;
  - `quality()`: `Suspended` per ogni sensore di un disco associato il cui `WireDrive.state` è `"standby"`; altrimenti `Held` se `snapshot.held[i]` o se il `seq` è ripetuto; altrimenti `Fresh`.

- [ ] **Step 1: test che falliscono.**

```rust
// settings
#[test] fn smart_enabled_drives_round_trips() {}          // encode -> decode_lenient, and apply_patch with {"sources":{"smartEnabledDrives":["storage/device-aaa"]}}
// service.rs
#[test] fn an_id_in_both_lists_is_requested_as_disabled_only() {}
#[test] fn smart_enabled_drives_changes_are_sent_as_a_request() {}
// svc/drives.rs
#[test] fn a_drive_is_associated_by_number_and_unique_key() {}
#[test] fn a_reused_drive_number_with_another_key_is_not_associated() {}
#[test] fn a_missing_or_duplicated_key_is_not_associated() {}
#[test] fn a_default_off_disk_is_accepted_only_when_enabled() {}
// svc/provider.rs
#[test] fn the_main_temperature_of_a_bound_disk_is_left_to_the_storage_provider() {
    // wire_schema() with temperature/drive and temperature/sensor-1 on a bound disk:
    // the inventory keeps `storage/device-bbb/temperature/sensor-1` only
}
#[test] fn an_unbound_disk_keeps_its_main_temperature() {}
#[test] fn a_usb_disk_enabled_by_another_client_is_filtered_locally() {}
#[test] fn sensors_of_a_disk_in_standby_are_suspended() {}
#[test] fn held_flags_become_held_quality() {}            // snapshot held [false, true] -> [Fresh, Held]
// storage.rs
#[test] fn disk_properties_mark_a_usb_disk_as_default_off() {}
```

- [ ] **Step 2:** `cargo test --workspace` → falliscono.
- [ ] **Step 3: implementazione.** `storage_binding` passa in `svc/drives.rs` e resta l'unica regola di associazione; `smart_disabled_drive_is_filtered_locally` (`provider.rs:966`) si aggiorna al nuovo inventario.
- [ ] **Step 4:** `cargo test --workspace` → PASS.
- [ ] **Step 5: commit** `feat(app): request default-off drives and take quality from the service`.

---

### Task 12: Il provider `storage` usa il servizio

**Files:**
- Modify: `crates/oma-win/src/storage.rs`, `crates/oma-win/src/lib.rs:42-53`
- Test: `crates/oma-win/src/storage.rs` (`mod tests`)

**Interfaces:**
- Consumes: `plan`, `power`, `ServiceDisk` (Task 3); `wire_drive_for`, `source_accepted` (Task 11); `SvcFeed::view()`, `FeedView { schema, snapshot: Option<(Instant, WireSnapshot)>, interval, request }`.
- Produces:
  ```rust
  StorageProvider::new(drives: DriveIdTable, disk_states: DiskStateTable, feed: SvcFeed)
  /// What the service says about one core disk, from an immutable view of the feed.
  /// `Absent` when the schema or the snapshot is missing, the snapshot is older than three
  /// intervals, or the disk is not associated.
  pub(crate) fn service_disk(entry: &DriveEntry, drives: &DriveIds, view: &FeedView, now: Instant) -> ServiceDisk;
  ```
- La temperatura del servizio è il sensore `temperature`/`drive` del dispositivo associato; `temperature` è `Some` solo con valore presente e `source_accepted`.
- Con `Plan::Service(t)`: il valore del sensore principale è `t.value`; qualità `Held` se `t.held`, altrimenti `Fresh`; nessuna query locale. Se il disco non ha ancora il sensore principale, la prima misura del servizio lo dichiara con `Rediscover`.
- Alla scadenza o allo scollegamento: il sensore resta dichiarato con l'ultimo valore, e vale la riga locale della tabella (attività recente → lettura; altrimenti `Idle` e `Suspended`).

- [ ] **Step 1: test che falliscono** (puri: `FeedView` costruita a mano, lettore di temperatura passato come closure):

```rust
#[test] fn a_stale_feed_has_no_authority() {}            // snapshot received 3 * interval + 1 ms ago -> ServiceDisk::Absent
#[test] fn a_disk_asleep_at_startup_is_never_queried() {} // Present{Standby, .., None}: the reader closure is never called, no temperature sensor declared, power Standby
#[test] fn the_first_service_measure_declares_the_sensor_without_a_local_query() {}
#[test] fn the_service_temperature_replaces_the_local_read() {} // Present{Active, false, Some{41.0, held: false}} -> value 41.0, Fresh, reader not called
#[test] fn a_held_service_temperature_is_held() {}
#[test] fn a_refused_source_falls_back_to_the_activity_rule() {} // disk in smart_disabled_drives: temperature None -> Plan::Local only when recent
#[test] fn a_blocking_drive_is_never_queried_locally() {}
#[test] fn losing_the_service_keeps_the_sensor_and_its_last_value() {} // next poll, idle: value 41.0, Suspended, reader not called
#[test] fn standby_from_the_service_is_not_kept_after_a_disconnect() {} // power becomes Idle, not Standby
```

- [ ] **Step 2:** `cargo test -p oma-win storage` → falliscono.
- [ ] **Step 3: implementazione.** Una sola `feed.view()` per poll; nessuna dipendenza dall'ordine dei poll di `storage` e `svc`. `default_providers` passa `service.feed.clone()` e `service.disk_states.clone()`. `storage_gate::DriveState` provvisorio è già `oma_ipc::DriveState` dal Task 5.
- [ ] **Step 4:** `cargo test --workspace` → PASS, compreso `duplicate_sensor_id_keeps_the_first_provider`.
- [ ] **Step 5: commit** `feat(storage): take the main disk temperature from the service when it has one`.

---

### Task 13: Qualità e stato dei dischi verso l'interfaccia

**Files:**
- Modify: `app/src-tauri/src/main.rs:32-33,349-383,251`, `app/src-tauri/src/commands.rs`, `app/src/lib/types.ts:93-98`, `app/src/lib/backend/backend.ts:27-40`, `tauri.ts:31-47`, `mock.ts`, `app/src/test/fake-backend.ts`, `app/src/lib/live.svelte.ts`
- Test: `app/src-tauri/src/commands.rs` (test del payload), `app/src/lib/live.svelte.test.ts`, `app/src/lib/backend/mock.test.ts`

**Interfaces:**
- Produces (Rust):
  ```rust
  /// The `oma:snapshot` payload: the snapshot plus one quality code per value (0 fresh, 1 held, 2 suspended).
  #[derive(Serialize)] #[serde(rename_all = "camelCase")]
  pub(crate) struct SnapshotEvent<'a> { #[serde(flatten)] pub snapshot: &'a Snapshot, pub quality: Vec<u8> }
  pub(crate) fn quality_codes(quality: &[Quality]) -> Vec<u8>;

  pub const EVENT_DISK_STATES: &str = "oma:disk-states";
  #[derive(Serialize)] #[serde(rename_all = "camelCase")]
  pub(crate) struct DiskStateEntry { pub device_id: String, pub power: DiskPower }
  #[tauri::command] fn get_disk_states(..) -> Vec<DiskStateEntry>;
  ```
  `oma:disk-states` si emette nel callback del tick quando cambia la generazione di `DiskStateTable`.
- Produces (TypeScript):
  ```ts
  export interface Snapshot { revision: number; seq: number; timestampMs: number; values: (number | null)[]; quality?: number[]; }
  export type DiskPower = 'active' | 'idle' | 'standby' | 'unknown';
  export interface DiskStateEntry { deviceId: string; power: DiskPower; }
  // Backend: getDiskStates(): Promise<DiskStateEntry[]>; onDiskStates(cb): Unlisten
  // LiveStore: quality(id: string): 0 | 1 | 2;  diskPower(deviceId: string): DiskPower | undefined
  ```

- [ ] **Step 1: test che falliscono.** Rust: `snapshot_event_serializes_quality_next_to_the_values` (JSON con `"values":[1.0,null]` e `"quality":[1,2]`), `quality_codes_map_the_three_states`. Vitest: `quality defaults to fresh when the payload has none`, `quality follows the snapshot`, `disk power comes from the backend and updates on the event`.
- [ ] **Step 2:** `cargo test -p oma-app` e `cd app && pnpm test` → falliscono.
- [ ] **Step 3: implementazione.** Il mock e il fake backend restituiscono un elenco vuoto di stati e nessuna `quality` di default; il mock con servizio espone un HDD in `standby` per lo sviluppo dell'interfaccia.
- [ ] **Step 4:** `cargo test --workspace` e `cd app && pnpm test && pnpm check` → PASS.
- [ ] **Step 5: commit** `feat(app): send value quality and disk power state to the UI`.

---

### Task 14: Interfaccia

**Files:**
- Modify: `app/src/components/advanced/SensorTable.svelte:64-67,146-159`, `KpiRow.svelte:29-31`, `DevicePage.svelte:43-58`, `app/src/lib/advanced/pages.ts:178,262`, `app/src/components/settings/SourcesSection.svelte:22-47,131-157`, `app/src/lib/settingsView.ts:36-49`, `app/src/lib/types.ts:156-173,259-264`, `app/src/lib/backend/mock.ts:210-218`, `mockSettings.ts:55-60,256-261`, `app/src/lib/i18n/en.json`, `it.json`
- Modify (rimozione di `smartBlockedBy`): `crates/oma-ipc/src/status.rs`, `crates/oma-win/src/svc/link.rs`, `svc/status.rs`, `app/src-tauri/src/service.rs`
- Test: `SensorTable.test.ts`, `DevicePage.test.ts`, `pages.test.ts`, `SourcesSection.test.ts`, `mock.test.ts`, `i18n.test.ts`

**Interfaces:**
- Consumes: `LiveStore.quality`, `LiveStore.diskPower` (Task 13); `ServiceSources.drives` (Task 5); proprietà `smartDefault` (Task 11).
- Produces: `blockingDiskNames(drives: SourceDrive[], schema: Schema | null, t): string[]` — nome del dispositivo dello schema se `deviceId` lo trova, altrimenti `model`, altrimenti `settings.sources.smart.diskNumber`.

**Testi (chiavi nuove, stesse chiavi nei due cataloghi):**

| Chiave | it | en |
|---|---|---|
| `storage.power.standby` | In standby | In standby |
| `storage.power.idle` | Inattivo | Idle |
| `value.lastReading` | Ultima lettura | Last reading |
| `settings.sources.smart.diskNumber` | Disco {n} | Disk {n} |
| `settings.sources.smart.usbWarning` | Alcuni adattatori USB non segnalano lo standby: accendere lo SMART può tenere sveglio il disco | Some USB adapters do not report standby: turning SMART on can keep the disk awake |

`settings.sources.smart.unknownDisk` si elimina.

- [ ] **Step 1: test che falliscono.**
  - `SensorTable.test.ts`: `a suspended value is muted and labelled as the last reading`; `a held value looks like a fresh one`; `a suspended sensor without a value shows no reading`.
  - `DevicePage.test.ts`: `a disk in standby shows its state`; `an idle disk shows "Inattivo"`; `an active disk shows no state label`.
  - `SourcesSection.test.ts`: `a closed smart gate names the disk by device, model or number` (tre dischi bloccanti: uno nello schema, uno con solo `model`, uno senza nulla → "Disco 4"); `a usb disk starts with smart off and shows the warning`; `turning a usb disk on adds it to smartEnabledDrives` (patch `{ sources: { smartEnabledDrives: ['storage/usb'], smartDisabledDrives: [] } }`); `turning a normal disk off removes it from smartEnabledDrives`.
  - `pages.test.ts`: `smartDefault` è nascosta come `smartSelectable`.
- [ ] **Step 2:** `cd app && pnpm test` → falliscono.
- [ ] **Step 3: implementazione.** Il valore `Suspended` usa `color: var(--text-muted)` e la riga secondaria «Ultima lettura» (in `KpiRow` attraverso `KpiDef.secondary`); l'etichetta di stato usa lo stile `.tag` esistente, accanto all'intestazione della pagina del disco. `setSmart` scrive sempre entrambi gli elenchi, disgiunti. Rimuovere `smartBlockedBy`/`smart_blocked_by` da Rust, TypeScript, mock e test.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build` e `cargo test --workspace` → PASS.
- [ ] **Step 5: commit** `feat(ui): show disk power state, last readings and the drives that block SMART`.

---

### Task 15: Verifiche dal vivo, documenti e chiusura

**Files:**
- Modify: `docs/follow-ups.md`, `README.md`, `README.it.md`, `CLAUDE.md`, `docs/perf-budget.md`, questo piano ("Esito dell'esecuzione")
- Delete: `crates/oma-win/examples/m6b_wake.rs` (non tracciato), dopo le verifiche

- [ ] **Step 1: build per l'utente.** `pwsh scripts/build-installer-payload.ps1`, poi `cd app && pnpm tauri build --bundles nsis`. L'installer lo esegue l'utente.
- [ ] **Step 2: verifiche V1–V9 della spec §9.2, con l'utente**, una per volta, con `sat-probe.ps1` e l'osservazione di `GetDevicePowerState`. V3 su entrambe le modalità è obbligatoria per chiudere la M6b: un fallimento si isola (query del nucleo, `CHECK POWER MODE`, SMART, programma esterno), si corregge e si ripetono V1–V3 e V8. V6: cercare nel log del servizio se la chiavetta compare tra i dischi di LibreHardwareMonitor.
- [ ] **Step 3: budget.** `pwsh scripts/measure-footprint.ps1` e righe M6b in `docs/perf-budget.md`.
- [ ] **Step 4: `docs/follow-ups.md`.** Chiudere: USB e gate D6 (fallback SAT), `smartGateClosed` (sostituito da `drives`), copertura delle regole con l'HDD in standby, controllo "HDD standby" con la causa trovata. Aggiungere i limiti del §8 della spec e, tra i controlli dovuti, l'hard disk USB in standby. Scrivere la bozza della segnalazione a DiskInfoToolkit (ri-identificazione a ogni `DBT_DEVNODES_CHANGED`): si pubblica solo su richiesta dell'utente.
- [ ] **Step 5: README** ("Known limits" in entrambe le lingue) con i limiti visibili all'utente; **`CLAUDE.md`**: stato della M6b e una riga sul protocollo v3.
- [ ] **Step 6:** cancellare `crates/oma-win/examples/m6b_wake.rs`; `PYTHONHASHSEED=0 graphify update .`.
- [ ] **Step 7: verifica completa.**
  ```
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  cargo test -p oma-win -- --include-ignored
  dotnet test service/OpenMonitorAdvanced.slnx
  pwsh scripts/check-trim-warnings.ps1
  cd app && pnpm test && pnpm check && pnpm build
  ```
- [ ] **Step 8:** scrivere "Esito dell'esecuzione" in questo piano (verifiche, decisioni, ciò che resta non verificato) e fare commit `docs: record the M6b outcome and follow-ups`. Poi `superpowers:requesting-code-review` sull'intero branch (con `ffi-safety-reviewer` per `storage_ioctl.rs` e `protocol-parity-reviewer` per il protocollo) e `superpowers:finishing-a-development-branch`.

La release 0.4.0 (spec D6) segue il flusso di `docs/release.md` dopo il merge, su richiesta dell'utente.

---

## Esito del punto di controllo (Task 4)

Da compilare.

## Esito dell'esecuzione

Da compilare.
