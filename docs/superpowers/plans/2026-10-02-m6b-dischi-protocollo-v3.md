# M6b — Dischi e protocollo v3: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** con l'app aperta un HDD in standby resta in standby e Windows riesce a spegnerlo; una chiavetta USB non tiene più spento lo SMART di tutti i dischi; app e interfaccia conoscono lo stato di ogni disco.

**Architecture:** il provider `storage` del nucleo smette di interrogare la temperatura dei dischi rotazionali se non dopo attività recente, e diventa l'unico proprietario della temperatura principale del disco, che prende dal servizio quando c'è. Il servizio aggiunge il fallback SAT a `CHECK POWER MODE`, tiene spento di default lo SMART dei dischi USB e pubblica nel protocollo v3 lo stato per disco e un flag `held` per valore. La qualità dei valori diventa per sensore (`Fresh`, `Held`, `Suspended`) dal provider fino all'interfaccia e alle regole.

**Tech Stack:** Rust 1.90 (`oma-core`, `oma-ipc`, `oma-win`, shell Tauri 2.11), .NET 10 (`oma-service`, xUnit v3), Svelte 5 + TypeScript 6 (Vitest), MessagePack.

**Spec:** `docs/superpowers/specs/2026-10-02-m6b-dischi-protocollo-v3-design.md` (commit `04c1aa0`). Riferimento: `docs/superpowers/references/m5/f1-service-reconfiguration.md`.

**Branch:** `feat/m6b-dischi-protocollo-v3`, aperto da `main` con `superpowers:using-git-worktrees`.

## Global Constraints

- Codice, commenti e commit in inglese (conventional commits); prosa dei documenti in italiano con gli accenti. Fine riga LF.
- TDD: prima il test che fallisce. Dal codice FFI si estraggono helper puri; i test hardware sono `#[ignore = "requires real Windows hardware"]`.
- FFI Rust: `// SAFETY:` su ogni `unsafe`, assert di dimensione per ogni struct FFI. P/Invoke .NET: assert di layout nei test (`Marshal.SizeOf`, `Marshal.OffsetOf`).
- Protocollo: mai `skip_serializing_if`; chiavi sempre presenti, `nil` per gli assenti; fixture solo con `OMA_WRITE_FIXTURES=1` a thread singolo (`protocol/fixtures/README.md`). `PROTOCOL_VERSION = 3` sui due lati; `PIPE_NAME` invariato.
- Nessun comando verso un disco oltre a quelli elencati nella spec: accesso 0 per le query di proprietà; `CHECK POWER MODE` solo nel servizio.
- Mai eseguire sul PC di sviluppo: test Pester `Integration`, installer, input sintetico. Le prove che cambiano alimentazione, avviano sottoscrittori hardware o forzano standby le esegue l'utente; gli agenti preparano build e comandi. I test hardware ignorati li eseguono gli agenti come nelle milestone precedenti, tranne `reads_disk_temperatures_on_this_machine`, che interroga i dischi: quello lo esegue l'utente, a dischi svegli e lontano dalle prove di standby.
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

I percorsi abbreviati `Sensors/...` e `Protocol/...` sono relativi a `service/OpenMonitorAdvanced.Service/`; i test .NET sono in `service/OpenMonitorAdvanced.Service.Tests/Sensors/` o `Protocol/`. I numeri di riga sono riferimenti al checkout iniziale, non vincoli dopo i task precedenti.

**Ordine e gate:** Task 0 prima della decisione sul filtro del servizio; Task 1–3 e 4 per il percorso locale; Task 5–9 per protocollo e servizio; Task 10 dopo il Task 9 (obbligatorio dopo le misure del Task 0); Task 11–14 per integrazione e UI; Task 15 per l'accettazione. Le verifiche dal vivo non concluse restano aperte: non impediscono di preparare i task indipendenti, ma impediscono di dichiarare M6b completata.

---

### Task 0: Prova isolata del servizio e scelta del ramo

Nessuna modifica al prodotto. Registrare versione del servizio, protocollo, timeout disco, richieste di alimentazione e log in «Esito del punto di controllo».

- [ ] **Step 1: preparare il sottoscrittore isolato.** Creare in `target/spike/m6b/service-only/` un progetto Rust temporaneo con `[workspace]` proprio e dipendenza path da `crates/oma-ipc`. Aprire `\\.\pipe\{PIPE_NAME}` in lettura/scrittura con `std::fs::OpenOptions`, usare `encode_frame` e `FrameDecoder` per Hello/Subscribe (1000 ms, moduli non-storage disabilitati, nessun disco disabilitato) e leggere continuamente schema e snapshot per 10 minuti. Non istanziare provider né interrogare dischi. Compilare con `cargo build --manifest-path target/spike/m6b/service-only/Cargo.toml --release` e stampare il comando di avvio per l'utente. Usare il protocollo del servizio testato: prima del Task 5 sul v2; se il servizio v2 non è disponibile, ripetere sul v3 dopo il Task 9, prima di decidere il Task 10. Il progetto temporaneo non si aggiunge al repository.
- [ ] **Step 2 (utente): controllo senza monitoraggio.** App chiusa, servizio fermo, TR-VISION HOME chiuso; annotare `powercfg /requests`, impostare timeout disco a 60 s e osservare passivamente `GetDevicePowerState` per 10 minuti senza toccare `D:`. Atteso: `on=False`. Annotare e poi ripristinare le impostazioni di alimentazione iniziali.
- [ ] **Step 3 (utente): servizio da solo con sottoscrittore.** Stesse condizioni, avviare solo servizio e sottoscrittore del punto 1, aspettare l'identificazione storage e i primi valori SMART; poi ripetere l'osservazione. Se il gate non apre, la prova è inconcludente. Nessuna query di temperatura usata come osservatore.
- [ ] **Step 4: isolare un eventuale fallimento.** Confrontare controllo e servizio isolato; preparare una build diagnostica temporanea del servizio con `Update` SMART escluso ma stessi `CHECK POWER MODE`, da far avviare all'utente. Se anche questa impedisce lo spegnimento, isolare i controlli di stato e le interferenze esterne prima di prescrivere il filtro. Registrare quale I/O mantiene il disco acceso. Il Task 10 è richiesto quando la causa è la lettura SMART; se la prova passa si salta, se è inconcludente la decisione resta aperta. V3 sull'app completa resta obbligatoria in entrambi i rami.

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
#[test] fn a_second_timeout_clears_values_and_suspended_quality() {} // second hung tick: values None, quality Fresh, coverage unavailable
#[test] fn merged_quality_follows_the_winning_sensor_indices() {} // duplicate sensor removed: values and qualities use the same slot.keep mask
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
    // the first fresh tick only establishes a new anchor;
    // then 10 measured seconds above: the instance enters (30 s reached)
}
#[test] fn held_without_a_value_still_resets_timers() {} // transport loss is not an intentional suspension
#[test] fn slow_fresh_measurements_separated_by_held_ticks_still_mature() {} // preserve M5 R1 for a real fresh measure every 30 s
#[test] fn suspended_does_not_cover_a_configuration_error() {} // invalid rule target stays unavailable
#[test] fn standby_suspends_temperature_and_smart_but_not_volume_or_io() {} // mixed qualities, missing volume/I/O still degrades coverage
#[test] fn idle_suspends_only_temperature() {} // missing SMART remains unavailable
```

- [ ] **Step 2:** `cargo test -p oma-core rules` → falliscono.
- [ ] **Step 3: implementazione.** In `Instance::step`, solo `Suspended` precede il ramo del valore assente: conserva timer accumulati e livelli, svuota il solo `anchor_ms` e ritorna `Step::Stay`. La prima misura fresca stabilisce il nuovo anchor senza conteggiare il tempo sospeso; non usare `reset_timers`, che perderebbe il tempo accumulato. `Held` conserva il contratto M5 R1: il tick conservato non matura né resetta, e le misure fresche successive restano valutabili alla loro cadenza; `Held` senza valore resta un dato assente. In `health.rs:448`: `slot.available = slot.instance.problem.is_none() && (value.is_some() || quality == Quality::Suspended)`; un errore di configurazione resta scoperto.
- [ ] **Step 4:** `cargo test -p oma-core` e `cargo test -p oma-core --test rules_alloc` → PASS.
- [ ] **Step 5: commit** `feat(rules): treat a suspended sensor as covered`.

---

### Task 3: Gate locale della temperatura dei dischi

**Files:**
- Create: `crates/oma-win/src/storage_gate.rs`
- Modify: `crates/oma-win/src/lib.rs` (modulo, `ServiceHandles`, `default_providers`), `crates/oma-win/src/storage_ioctl.rs` (seek penalty), `crates/oma-win/src/storage.rs:103-125,264-300,313-481`, `app/src-tauri/src/main.rs:157,197-200`
- Modify (compatibilità del costruttore, senza aggiungerlo a git): `crates/oma-win/examples/m6b_wake.rs`; aggiornare anche i call site nei test e negli esempi tracciati
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
  `storage.rs` riesporta `DiskPower` con `pub use crate::storage_gate::DiskPower`, perché il modulo del gate resta privato e la shell deve nominare il tipo. `ServiceHandles` guadagna `pub disk_states: storage::DiskStateTable`; `StorageProvider::new(drives: DriveIdTable, disk_states: DiskStateTable)`. `oma_ipc::DriveState` arriva nel Task 5: in questo task `ServiceDisk` si definisce con un enum locale provvisorio `DriveState { Active, Standby, Unknown, SmartOff, NoMedia }` nel modulo, sostituito nel Task 5.
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
#[test] fn a_sampling_gap_invalidates_the_activity_window() { /* gap > 10 s or missing current counters clears earlier activity; the first post-gap sample is warm-up, only a subsequent valid positive sample authorizes a read */ }
```

  e in `storage.rs`:

```rust
#[test] fn an_idle_hdd_is_not_picked_and_does_not_starve_other_disks() { /* two due disks, the older one Plan::Wait: the other is refreshed */ }
#[test] fn an_idle_or_standby_disk_keeps_its_last_values_and_stays_due() { /* values unchanged, read_at unchanged; Wait/Unknown or Wait/NoMedia instead exposes None */ }
#[test] fn a_failed_authorized_read_gives_absent_values() { /* Plan::Local with report None -> values None, read_at = now */ }
#[test] fn a_new_device_id_starts_without_cache() { /* carry-over keyed by device id: another id on the same index gets empty positions and a default Activity */ }
#[test] fn suspended_quality_covers_only_the_temperature_sensors() { /* disk with read, write, active, drive temp, volume: only the temperature slot is Suspended when power is Idle or Standby */ }
#[test] fn cached_temperature_between_reads_is_held_while_io_is_fresh() { /* read at t, next poll at t+1s with recent I/O: temperature Held, throughput Fresh; actual read at t+30s: temperature Fresh */ }
```

- [ ] **Step 2:** `cargo test -p oma-win storage` → falliscono.
- [ ] **Step 3: implementazione.**
  - `discover`: classe del disco da `bus_type` e `seek_penalty`; per `RotationalOrUnknown` nessuna `read_temperatures` alla discovery; `temperatures` e `Activity` si conservano per i dischi con lo **stesso id di dispositivo**, non si ricreano. Per `NonRotational` resta la lettura di oggi.
  - `poll`: `activity.observe(read, write, &now)` per ogni disco (con `fresh` → `None, None`); i candidati al refresh sono i dischi scaduti il cui `plan` non è `Wait`; `Wait` non rinnova `read_at`. `Wait/Idle` e `Wait/Standby` conservano i valori; `Wait/Unknown` o `NoMedia` espongono valori assenti, senza spacciarli per sospensione prevista.
  - Il gap si rileva dai tempi monotono e wall di `Stamp`: un intervallo superiore a `ACTIVITY_WINDOW` in uno dei due, o una loro divergenza che indica sospensione/cambio dell'orologio, invalida l'attività precedente; anche il campione al rientro dal gap non autorizza I/O. Con campione mancante/non valido si azzera l'autorizzazione corrente; zero valido non rinnova la finestra ma conserva un campione positivo ancora entro 10 s.
  - `Provider::quality`: `Suspended` per le temperature con `plan == Wait` e `power` `Idle` o `Standby`; `Held` per temperature già misurate ripubblicate tra due letture autorizzate, `Fresh` solo per una nuova misura o un dato assente non sospeso. I/O, carico e spazio restano `Fresh`. Nella UI solo `Suspended` cambia l'aspetto del valore («Ultima lettura», Task 14): `Held` tra due letture regolari resta visivamente normale, altrimenti ogni temperatura sarebbe in grigio per 29 secondi su 30.
  - `disk_states.publish(...)` a ogni poll.
  - `main.rs`: creare la tabella accanto a `svc_feed`/`svc_drives` e passarla in `ServiceHandles`.
- [ ] **Step 4:** `cargo test -p oma-win` → PASS. `cargo test -p oma-win -- --include-ignored --skip reads_disk_temperatures_on_this_machine` → PASS. Preparare per l'utente `cargo test -p oma-win reads_disk_temperatures_on_this_machine -- --ignored`: eseguirlo solo a disco già sveglio, separato dalle prove di standby.
- [ ] **Step 5: commit** `fix(storage): read an HDD's temperature only after recent activity`.

---

### Task 4: Punto di controllo dal vivo (utente)

Nessun codice di prodotto. Verifica il filtro locale; la decisione del §4.4 richiede anche la prova isolata del Task 0, non la sola app completa.

- [ ] **Step 1:** `cargo build -p oma-win --example m6b_wake --release` e `cd app && pnpm tauri build --no-bundle` (produce `target/release/oma-app.exe`; il servizio installato resta il 0.3.0, protocollo v2: compatibile fino al Task 5).
- [ ] **Step 2 (utente, PowerShell amministratore, app installata chiusa):** verificare che `target/spike/m6b/sat-probe.ps1` esista e che `-Bisect` supporti `storage,poll-storage`; sono strumenti locali non tracciati, quindi se mancano ricrearli prima del checkpoint e documentarne i comandi. Eseguire:
  `pwsh -File target\spike\m6b\sat-probe.ps1 -Drive 0 -Bisect storage,poll-storage`
  Atteso: `still in standby after 50 s` su entrambe le righe.
- [ ] **Step 3 (utente):** avviare `target\release\oma-app.exe`, TR-VISION HOME chiuso, timeout disco di Windows a 60 s. Due prove, una in modalità anti-cheat e una con il servizio collegato:
  `pwsh -File target\spike\m6b\sat-probe.ps1 -Drive 0 -Standby -Method sat16 -Count 10 -IntervalSeconds 30`
  Atteso: `STANDBY` su tutte le righe.
- [ ] **Step 4 (agente, senza privilegi, mentre l'utente non tocca `D:`):** per ognuna delle due modalità osservare per 10 minuti `GetDevicePowerState` del disco 0 e il contatore `\Disco fisico(0 D:)\Trasferimenti disco/sec` (comando usato nello spike). Atteso: `on=False` entro pochi minuti.
- [ ] **Step 5:** scrivere l'esito in fondo a questo piano ("Esito del punto di controllo"), insieme al controllo a app/servizio chiusi e alle richieste di alimentazione del Task 0. Se la modalità anti-cheat fallisce, correggere il percorso locale e ripetere la prova. Se fallisce solo il servizio collegato, confrontare con il Task 0 e isolare la causa prima di scegliere il Task 10. Un esito inconcludente tiene aperto il gate. Se la build non si collega al servizio installato, ripetere gli Step 3-4 dopo il Task 12; la decisione condizionale non si considera acquisita.

---

### Task 5: Protocollo v3 in `oma-ipc`

**Files:**
- Modify: `crates/oma-ipc/src/lib.rs:14-24`, `message.rs:45-84,124-129`, `frame.rs:301-315` (e i test a `:416`, `:878-907`), `status.rs:104-135,189-222`, `crates/oma-ipc/tests/fixtures.rs`, `protocol/fixtures/*.msgpack`, `protocol/fixtures/README.md`
- Modify: `crates/oma-win/src/svc/feed.rs` (coerenza di schema e snapshot), `crates/oma-win/src/svc/link.rs:617-672` (richieste e stato)
- Modify (compatibilità dei tipi): `crates/oma-win/src/svc/status.rs:185`, `svc/provider.rs` (letterali di `WireSnapshot`), `app/src-tauri/src/service.rs:755`, `crates/oma-win/src/storage_gate.rs` (usa `oma_ipc::DriveState`)

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
- `SvcFeed::set_schema`: una variazione di `service.drives` invalida lo snapshot precedente e incrementa `generation`, anche con dispositivi e sensori invariati; solo il successivo snapshot della stessa connessione restituisce autorità ai nuovi stati. Un cambio del solo `reconfiguration` senza cambi di dischi/sensori conserva il comportamento attuale. Test in `svc/feed.rs`: `a_drive_state_change_drops_the_previous_snapshot`, `an_equal_drive_table_keeps_the_snapshot`, `clearing_the_feed_revokes_drive_authority`.

**Valori delle fixture** (`reference` in `tests/fixtures.rs`):
- hello: `protocol_version: PROTOCOL_VERSION`;
- subscribe: `smart_disabled_drives: [KEY_A]`, `smart_enabled_drives: [KEY_B]`;
- schema `service.drives`: `{0, Some(KEY_A), Some("Samsung SSD 990 PRO 2TB"), "smartOff", false}` e `{1, None, Some("ST2000DM008-2UB102"), "standby", true}`;
- snapshot: `held: [false, false, true, false]`; snapshot_empty: `held: []`.

- [ ] **Step 1: test che falliscono.** In `tests/fixtures.rs`: `protocol_constants_are_v3` (`assert_eq!(PROTOCOL_VERSION, 3)`), `subscribe_v3_keeps_every_key` (chiavi `["disabled_modules", "interval_ms", "smart_disabled_drives", "smart_enabled_drives"]`). In `frame.rs`: `a_snapshot_whose_held_length_differs_is_rejected`, `held_without_a_value_is_rejected`, `a_non_finite_value_loses_its_held_flag`. In `status.rs`: `an_unknown_drive_state_reads_as_unknown` (`DriveState::from_wire("spinning") == DriveState::Unknown`), e `service_status_serializes_with_pawn_io_and_sources` esteso con `"drives":[{"physicalDrive":1,"deviceId":null,"model":"ST2000DM008-2UB102","state":"standby","blocksSmart":true}]`.
- [ ] **Step 2:** `cargo test -p oma-ipc` → falliscono.
- [ ] **Step 3: implementazione** dei tipi e della validazione; provvisoriamente `link.rs::refresh_sources` costruisce `drives` (`device_id` con `core_id_for_key`) e `smart_blocked_by`; il Task 11 sostituisce questa associazione con numero e chiave univoci. `subscribe_message` traduce e tronca a `MAX_DRIVE_KEYS` anche `smart_enabled_drives`, e `on_drives_changed` confronta entrambi gli elenchi.
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
  Aggiungere test della validazione di `smart_enabled_drives`: 64 chiavi valide accettate, 65 rifiutate, caratteri maiuscoli/non esadecimali o lunghezza diversa da 64 rifiutati. Conservare la decodifica di Hello v2 necessaria a segnalare `Incompatible`; aggiungere entrambe le direzioni 2/3 ai test di handshake.
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
[Fact] public void RemovingAndReaddingTheSameIdentityForgetsTheRoute()
[Fact] public void AMissingIdentityDoesNotKeepARouteAcrossEnumeration()
[Fact] public void ASuccessfulIoctlWithUnknownRegistersTriesSat()
[Fact] public void DescriptorSenseIsAcceptedWithScsiStatusZero()
[Fact] public void ScsiPassThroughIsFiftySixBytesOnX64()                 // SizeOf == 56; OffsetOf(SenseInfoOffset) == 32; OffsetOf(Cdb) == 36
```

- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter "FullyQualifiedName~SatSense|FullyQualifiedName~DiskPowerProbe"` → falliscono.
- [ ] **Step 3: implementazione.** `SatSense` maschera il bit VALID (`sense[0] & 0x7F`), rispetta la lunghezza restituita e quella dichiarata (`8 + sense[7]`) e controlla i limiti di ogni descrittore. Il fallback parte quando la via nativa non dà una risposta interpretabile, anche se l'IOCTL è riuscita. Il ricordo della via è per numero di disco, con modello e seriale di quando è stato scritto. Gli errori Win32 della via SAT usano `Win32ErrorLog` con una propria etichetta.
  La cache conserva la via, mai una vecchia risposta di stato: tra i retry della via «nessuna» ritorna `null`. Riconciliare le identità a ogni enumerazione e rimuovere le vie dei dischi scomparsi; senza modello/seriale verificabili non conservarle attraverso rediscovery. La slice di sense deriva dai byte realmente restituiti da `DeviceIoControl`, limitata all'offset/buffer di sense e alla sua lunghezza, non dai 32 byte allocati a prescindere.
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
      /// Precedence: noMedia, smartOff, then the power answer (standby/active/unknown).
      /// Active without a check only when RequiresPowerCheck is false; otherwise unasked is unknown.
      internal static string Of(DriveFacts facts, bool smartOff, bool asked, bool? spunDown);
      internal static bool IsSmartOff(DriveFacts facts, string? key, EffectiveConfig config);
  }
  ```
- `SmartEnabledDrives` effettivo: unione degli elenchi delle richieste con lo storage attivo. `SmartDisabledDrives` resta l'intersezione di oggi.
- Aggiornare anche `EffectiveConfig.AllOn` (abilitati vuoti: USB resta spento), `StoragePart`, `Equals` e `GetHashCode` per entrambi gli insiemi. Senza sottoscrittori `Compute` continua a restituire `null`, conservando l'ultima configurazione effettiva.
- `IsSmartOff`: storage spento, oppure chiave in `SmartDisabledDrives`, oppure `SmartOffByDefault` e chiave assente da `SmartEnabledDrives` (un disco senza chiave non si accende).
- Dopo l'apertura del gate, enumerare tutte le `DriveFacts` e controllare una volta per giro ogni disco che richiede power check e non è SMART off, anche se LHM non lo espone. Riutilizzare quella risposta per aggiornamento e `drives`. Nessun power check periodico per USB/default-off o dischi disabilitati; `CheckGate` li controlla comunque prima della prima identificazione.
- Pubblicazione coerente: sostituire `_drives`, `_resolvedDisks` e `_storageCache` separati con un unico riferimento immutabile `StorageRound(long Generation, long Timestamp, IReadOnlyList<WireDrive> Drives, IReadOnlyDictionary<string, DiskResolution> Resolved, IReadOnlyDictionary<string, double?> Values)`. Lo storage worker lo sostituisce una sola volta al termine del giro. Il sampler cattura un riferimento per tick e usa quello per `UpdateServiceState`, ricostruzione dello schema e valori; non può accoppiare standby nuovo a valori appena misurati del giro precedente. Il Task 9 aggiunge `Held` a questo record. Nessun lock condiviso durante I/O.

- [ ] **Step 1: test che falliscono.**

```csharp
// DriveStatesTests
[Theory] /* (availability, smartOff, asked, spunDown) -> state */
// NoMedia,any,any,any -> "noMedia"; Present,true,true,true -> "smartOff"; Present,false,true,true -> "standby";
// Present,false,true,false -> "active"; Present,false,true,null -> "unknown"; Present,false,false,null (NVMe) -> "active"
// Unreadable,false,false,null -> "unknown"; rotational Present,false,false,null -> "unknown"

// EffectiveConfigTests
[Fact] public void ADefaultOffDriveIsEnabledIfAnyStorageSubscriberEnablesIt()
[Fact] public void EnabledDrivesOfASubscriberWithStorageOffAreIgnored()
[Fact] public void EnabledSetsParticipateInEqualityAndStoragePart()
[Fact] public void NoSubscribersKeepThePreviousConfiguration()

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
[Fact] public void AnOffUsbDiskReceivesNoPeriodicPowerChecksOrUpdates()
[Fact] public void AnEnabledDiskAbsentFromLhmStillHasACurrentPowerState()
[Fact] public void AConcurrentRoundCannotMixDriveStateAndSnapshotValues() // pause sampler after capture, publish next round, assert one captured round throughout
```

- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter FullyQualifiedName~Sensors` → falliscono.
- [ ] **Step 3: implementazione.** Pubblicare `StorageRound` secondo il contratto sopra: gate chiuso dagli esiti di `CheckGate`, gate aperto da `Enumerate()` più le risposte del giro, storage spento dall'ultimo elenco senza I/O e con valori vuoti. `DiskResolution` conserva i `DriveFacts`; i punti che leggono `SmartDisabledDrives` (`:491`, `:777`, `:831`) usano `DriveStates.IsSmartOff`. `FeedRequest.From` legge `SmartEnabledDrives`. Schema e snapshot pubblicati nello stesso tick usano la stessa vista; un cambio di `drives` fa incrementare la revisione anche senza variazione dei sensori.
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx` → PASS (adattati `ASmartDisabledDiskStillHoldsTheD6Gate` e gli altri test del gate alla nuova interfaccia).
- [ ] **Step 5: commit** `feat(service): publish per-drive state and keep USB disks' SMART off by default`.

---

### Task 9: Valori conservati (`held`) nel servizio

**Files:**
- Modify: `service/OpenMonitorAdvanced.Service/Sensors/SensorHub.cs:305-349,431-549,909,1390`
- Test: `SensorHubTests.cs` (helper nuovo `HeldOf(updates, at, kind, name)` accanto a `ValueOf` a `:89`)

**Interfaces:**
- Consumes: `SnapshotMessage.Held` (Task 6).
- Produces: il `StorageRound` del Task 8 esteso con `IReadOnlySet<string> Held`. Per riconoscere la ripubblicazione, il sampler ricorda l'ultima `Generation` pubblicata; timestamp o nuovo `seq` da soli non indicano una nuova misura.
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
[Fact] public void UnknownNoMediaDisabledAndReadErrorsPublishMissingNotHeld()
[Fact] public void AReplacementDiskDoesNotInheritHeldValues()            // same LHM id or number, changed verified key
[Fact] public void AStalledStorageWorkerExpiresEvenPreviouslyKeptValues() // beyond 60 s, missing and held false
```

  `ASpunDownHddIsSkippedAndReadsAsMissing` (`:250`) si divide nei primi due test.
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter FullyQualifiedName~SensorHub` → falliscono.
- [ ] **Step 3: implementazione.** Copiare solo valori non null del disco con identità verificata e invariata, non per solo numero o identificatore LHM. La finestra di trasporto di `FreshStorageValues` (due giri, 60 s) si applica al `StorageRound` catturato dal sampler: un giro standby concluso rinnova il timestamp e conserva la misura; un worker bloccato non lo rinnova e scade. Quindi la conservazione può durare ore solo con giri regolarmente conclusi e standby ancora confermato. Un dato non finito/assente ha sempre `held = false`; spegnimento storage, idle del hub, rimozione o cambio d'identità eliminano la cache corrispondente.
- [ ] **Step 4:** `dotnet test service/OpenMonitorAdvanced.slnx` → PASS.
- [ ] **Step 5: commit** `feat(service): keep a sleeping disk's last values and flag them as held`.

---

### Task 10: controlli di stato e SMART solo dopo attività, nel servizio

Obbligatorio (riscritto il 2026-10-02 dopo le misure del Task 0, spec §2 e §4.4): `CHECK POWER MODE` azzera da solo il timer di inattività di Windows e riaccende un disco spento da Windows. Si esegue dopo il Task 9; la parte Rust (stato `idle`) entra nel Task 12.

**Files:**
- Create: `service/OpenMonitorAdvanced.Service/Sensors/DiskActivityProbe.cs`
- Modify: `Sensors/IHardwareTree.cs`, `Sensors/DiskPowerProbe.cs` (gate), `Sensors/DriveStates.cs`, `Sensors/SensorHub.cs` (storage worker), `ServiceHost.cs:118`
- Test: `DiskActivityTests.cs` (nuovo), `DriveStatesTests.cs`, `DiskPowerProbeTests.cs`, `SensorHubTests.cs`, `SensorHubFakes.cs`

**Interfaces:**
- Produces:
  ```csharp
  public interface IDiskActivityProbe
  {
      DiskCounters? Read(int driveNumber);     // IOCTL_DISK_PERFORMANCE (0x00070020), access 0
      bool? PoweredOn(int driveNumber);        // GetDevicePowerState, access 0; null when the call fails
  }
  public readonly record struct DiskCounters(long ReadCount, long WriteCount);
  internal static class DiskActivity
  {
      internal static readonly TimeSpan Window = TimeSpan.FromSeconds(10);
      /// True only when both samples exist, no counter went back and at least one grew.
      internal static bool Between(DiskCounters? earlier, DiskCounters? later);
  }
  // DriveStates
  internal const string Idle = "idle";
  /// Precedence: noMedia, smartOff, standby (Windows reports the disk off, or the disk answered standby),
  /// idle (needs a power check, not asked this round because there was no recent activity),
  /// then active / unknown as before.
  ```
- `DISK_PERFORMANCE` è di 88 byte: `ReadCount` (u32) all'offset 40, `WriteCount` (u32) all'offset 44; assert di layout nei test.
- **Worker a regime** (gate aperto), per un disco con `RequiresPowerCheck` e SMART acceso, nell'ordine:
  1. `PoweredOn == false` → `standby`, nessun comando, valori del giro precedente conservati come `held` (regole del Task 9);
  2. attività recente → `IsSpunDown` e, se attivo, `Update`, come oggi;
  3. altrimenti → `idle`, nessun `IsSpunDown`, nessun `Update`, valori conservati come `held` con le stesse regole di identità e di finestra del Task 9.
  `PoweredOn == null` vale acceso. SSD, NVMe e virtuali: cadenza di 30 s invariata, nessun campionamento.
- **Campionamento** (sostituito dal Task 16: baseline alla fine del giro precedente): nella prima stesura il worker si sveglia 10 s prima di ogni giro per la baseline e campiona di nuovo all'inizio del giro, prima di ogni comando. La baseline appartiene a numero fisico e chiave verificata invariati e dista al più 10 s; si azzera a sospensione/gap, hot-plug, storage spento e assenza di sottoscrittori. Primo giro senza baseline: nessuna attività.
- **Gate D6:**
  1. primo giro di un episodio: `IsSpunDown` una volta per ogni disco con `RequiresPowerCheck` che non risulta spento da Windows; un disco con `PoweredOn == false` è `standby` e blocca senza comandi;
  2. gate chiuso: si richiede solo ai dischi bloccanti, e solo con attività recente su di loro; un bloccante senza contatore leggibile (`Read == null`) al più ogni 5 minuti; le risposte "attivo" dell'episodio si conservano senza rinnovarle;
  3. quando nessuno blocca più: un controllo completo, una sola volta, prima di `EnableStorage`; se trova uno standby il gate resta chiuso e l'episodio continua dal punto 2.
- L'enumerazione per giro (`Enumerate`, `Describe`) usa solo la query del descrittore; verificare leggendo il codice che non invii `IOCTL_DISK_GET_LENGTH_INFO`, `IOCTL_STORAGE_CHECK_VERIFY`, `SMART_GET_VERSION` o altre richieste che arrivano al disco, e riportarlo nel report.

- [ ] **Step 1: test che falliscono.** `DiskActivityTests`: `ACounterThatGrewIsActivity`, `AMissingBaselineIsNotActivity`, `ACounterThatWentBackIsNotActivity` (anche un contatore cresciuto mentre l'altro cala), `DiskPerformanceIsEightyEightBytes`. `DriveStatesTests`: righe nuove per `idle` e per `standby` da `PoweredOn == false`. `SensorHubTests`:
  `ADiskThatWindowsTurnedOffIsStandbyWithoutAnyCommand` (SpunDownQueries e Updates invariati, valore `held`), `AnIdleHddIsNeitherAskedNorUpdatedAndKeepsHeldValues` (stato `"idle"`), `AnHddWithRecentIoIsAskedAndUpdated`, `SolidStateDisksAreUpdatedEveryRound`, `TheWorkerWakesTenSecondsBeforeARound` (dopo un giro `RunStorageDue()` restituisce 20 s, alla baseline 10 s), `ABaselineFromAnotherIdentityOrBeforeSuspendIsNotUsed`, `ALateBaselineDoesNotAuthorizeSmart`, `TheFirstRoundAfterTheGateOpensDoesNotAskAgain`, `AnIdleDiskAfterAStallKeepsNothing` (finestra del Task 9), `StorageOffAndNoSubscribersPerformNoActivityIo`, `TheClosedGateAsksEachDriveOnceThenOnlyBlockersWithActivity`, `ABlockerWithoutCountersIsRetriedEveryFiveMinutes`, `ADriveThatWindowsTurnedOffBlocksTheGateWithoutACommand`, `TheGateRunsOneFullCheckBeforeOpening`, `AStandbyFoundByTheFinalCheckKeepsTheGateClosed`, `AFailedPowerStateCallCountsAsOn`.
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter "FullyQualifiedName~DiskActivity|FullyQualifiedName~DriveStates|FullyQualifiedName~SensorHub"` → falliscono.
- [ ] **Step 3: implementazione.** Collegare il probe a `ServiceHost` e alla seam di test di `SensorHub`; `TimeProvider` per baseline, schedule e ritentativo dei 5 minuti. Nessun lock durante I/O; lo stato dell'episodio del gate vive nel worker. `dotnet test service/OpenMonitorAdvanced.slnx` e `pwsh scripts/check-trim-warnings.ps1` → PASS, nessun avviso nuovo.
- [ ] **Step 4 (utente):** con la build del Task 15: servizio da solo con il sottoscrittore v3 e poi app completa collegata; osservazione passiva di 10 minuti ciascuna. Atteso: `on=False`. Se fallisce, isolare e ripetere V1–V3 e V8.
- [ ] **Step 5: commit** `fix(service): ask a disk its power mode and SMART only after recent activity`.

---

### Task 11: Impostazione, richiesta e provider `svc`

**Files:**
- Create: `crates/oma-win/src/svc/drives.rs`
- Modify: `crates/oma-core/src/settings/mod.rs:223-232,332`, `decode.rs:156-158`, `patch.rs:95`; `app/src-tauri/src/service.rs:212-222`; `crates/oma-win/src/storage.rs:142-190` (`DriveEntry`, `disk_properties`); `crates/oma-win/src/svc/provider.rs:37-61,119-200,242-330`, `svc/link.rs` (associazione e traduzione delle richieste); `crates/oma-win/src/svc/mod.rs`
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
- `source_accepted` è falso se storage è disabilitato, l'id core è esplicitamente disabilitato o il disco default-off non è esplicitamente abilitato. `refresh_sources` associa `SourceDrive.device_id` con numero **e** chiave univoci, usando `wire_drive_for`, non il solo `core_id_for_key`. In `subscribe_message`, tradurre entrambi gli elenchi in chiavi con deduplicazione e limite 64; la precedenza del disabled va applicata anche dopo la traduzione, così id diversi non producono la stessa chiave nei due elenchi.
- `SvcProvider`:
  - `bind` scarta, per un disco associato, il sensore `temperature`/`drive` (lo possiede `storage`, Task 12) e l'intero dispositivo se `!source_accepted`;
  - `quality()`: `Suspended` per i sensori SMART di un disco associato in `"standby"` solo con feed corrente, anche con valore assente, non per I/O o spazio del core. Altrimenti `Held` con valore se `snapshot.held[i]` o `seq` ripetuto; `Fresh` negli altri casi. Snapshot mancante/scaduto revoca subito l'eccezione standby: valori assenti, qualità `Fresh`, nessuna copertura concessa dal vecchio stato.

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
#[test] fn a_stale_or_cleared_feed_does_not_suspend_smart_rules() {}
#[test] fn translated_keys_are_unique_disjoint_and_bounded() {}
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
- Modify: `crates/oma-win/src/storage.rs`, `crates/oma-win/src/lib.rs:42-53`; aggiornare costruttori nei test/esempi e nel programma temporaneo `m6b_wake.rs` (non aggiungerlo a git)
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
- Con `Plan::Service(t)`: il valore principale è `t.value`; qualità `Held` se `t.held` **o** se storage sta rileggendo lo stesso snapshot della stessa generazione del feed; `Fresh` solo alla prima adozione di una nuova misura. Nessuna query locale. Valutare il feed a ogni poll, indipendentemente dalla scadenza locale di 30 s: una nuova misura del servizio deve essere adottata senza attendere quel termine e senza consumare il budget di query locali. Se manca il sensore principale, la prima misura lo dichiara con `Rediscover`; discovery conserva cache e posizioni per identità, senza rifare la query locale.
- **Stato `idle` del servizio (spec §3.1, §4.4, aggiunto il 2026-10-02):** `oma_ipc::DriveState` guadagna `Idle` (filo `"idle"`, JSON `"idle"`; test `an_idle_drive_state_round_trips` in `status.rs`). In `storage_gate::decide`, subito dopo la riga del servizio `Standby`: servizio `Idle` → `Plan::Wait`, `DiskPower::Idle`, anche con `recent` vero (il servizio possiede la fonte: nessuna query locale); righe corrispondenti in `the_decision_table_matches_the_spec`. La temperatura del servizio di un disco `idle` si importa come storica, con qualità `Suspended`, come per lo standby. Test: `an_idle_service_disk_is_not_queried_and_shows_its_last_reading`.
- Con `Plan::Wait` in standby confermato, conservare e importare l'eventuale temperatura storica del servizio, con qualità `Suspended`; non richiedere `state == active` per recuperare la cache standby. `noMedia` e veto ignoto non importano valori. I sensori locali aggiuntivi restano distinti e seguono il gate locale; un valore importato dal servizio dichiara solo la posizione principale, non inventa `sensor-N`.
- Alla scadenza o allo scollegamento: il sensore resta dichiarato con l'ultimo valore, e vale la riga locale della tabella (attività recente → lettura; altrimenti `Idle` e `Suspended`).

- [ ] **Step 1: test che falliscono** (puri: `FeedView` costruita a mano, lettore di temperatura passato come closure):

```rust
#[test] fn a_stale_feed_has_no_authority() {}            // snapshot received 3 * interval + 1 ms ago -> ServiceDisk::Absent
#[test] fn a_disk_asleep_at_startup_is_never_queried() {} // Present{Standby, .., None}: the reader closure is never called, no temperature sensor declared, power Standby
#[test] fn the_first_service_measure_declares_the_sensor_without_a_local_query() {}
#[test] fn the_service_temperature_replaces_the_local_read() {} // Present{Active, false, Some{41.0, held: false}} -> value 41.0, Fresh, reader not called
#[test] fn a_held_service_temperature_is_held() {}
#[test] fn rereading_the_same_service_snapshot_is_held() {}
#[test] fn a_new_service_measure_is_adopted_before_the_local_deadline() {}
#[test] fn a_standby_service_value_is_historical_and_suspended() {}
#[test] fn rediscovery_keeps_the_imported_temperature_without_a_local_query() {}
#[test] fn a_refused_source_falls_back_to_the_activity_rule() {} // disk in smart_disabled_drives: temperature None -> Plan::Local only when recent
#[test] fn a_blocking_drive_is_never_queried_locally() {}
#[test] fn losing_the_service_keeps_the_sensor_and_its_last_value() {} // next poll, idle: value 41.0, Suspended, reader not called
#[test] fn standby_from_the_service_is_not_kept_after_a_disconnect() {} // power becomes Idle, not Standby
#[test] fn reconnecting_with_another_key_drops_the_old_measure() {}
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
  #[tauri::command]
  pub(crate) fn get_disk_states(state: tauri::State<'_, crate::AppState>) -> Vec<DiskStateEntry>;
  ```
  `oma:disk-states` si emette nel callback del tick quando cambia la generazione di `DiskStateTable`.
  Conservare la stessa tabella in `AppState`, registrare il comando in `generate_handler!` e predisporre il ramo non-Windows che restituisce `[]`, senza importare tipi dal crate Windows. L'evento è sempre l'elenco completo: un elenco vuoto revoca gli stati precedenti. `DiskStateTable::publish` elimina i dischi rimossi e incrementa la generazione anche in questo caso.
- Produces (TypeScript):
  ```ts
  export interface Snapshot { revision: number; seq: number; timestampMs: number; values: (number | null)[]; quality?: number[]; }
  export type DiskPower = 'active' | 'idle' | 'standby' | 'unknown';
  export interface DiskStateEntry { deviceId: string; power: DiskPower; }
  // Backend: getDiskStates(): Promise<DiskStateEntry[]>;
  // onDiskStates(cb: (states: DiskStateEntry[]) => void): Promise<Unsubscribe>
  // LiveStore: quality(id: string): 0 | 1 | 2;  diskPower(deviceId: string): DiskPower | undefined
  ```

- [ ] **Step 1: test che falliscono.** Rust: `snapshot_event_serializes_quality_next_to_the_values` (JSON con `"values":[1.0,null]` e `"quality":[1,2]`), `quality_codes_map_the_three_states`. Vitest: `quality defaults to fresh when the payload has none`, `quality follows the snapshot`, `disk power comes from the backend and updates on the event`, `a rejected snapshot cannot overwrite quality`, `an empty disk event clears old power states`, `an event received during bootstrap wins over the initial disk query`, `disconnect removes the disk listener`.
- [ ] **Step 2:** `cargo test -p oma-app` e `cd app && pnpm test` → falliscono.
- [ ] **Step 3: implementazione.** `LiveStore` accetta qualità solo insieme a uno snapshot accettato per revisione/seq; vettore assente, di lunghezza sbagliata o codici invalidi → fallback `Fresh`. Reset della qualità a cambio schema/history seed, senza conservare indici del vecchio schema. In `connect`, registrare il listener dischi prima della query iniziale, contando gli eventi per evitare che una risposta iniziale tardiva sovrascriva uno stato più recente; rilasciarlo con gli altri listener. Il mock e il fake backend restituiscono stati vuoti e nessuna qualità di default; il mock con servizio espone un HDD in standby e temperature storiche, per verificare la UI.
- [ ] **Step 4:** `cargo test --workspace` e `cd app && pnpm test && pnpm check` → PASS.
- [ ] **Step 5: commit** `feat(app): send value quality and disk power state to the UI`.

---

### Task 14: Interfaccia

**Files:**
- Modify: `app/src/components/advanced/SensorTable.svelte:64-67,146-159`, `app/src/components/advanced/KpiRow.svelte:29-31`, `app/src/components/advanced/DevicePage.svelte:43-58`, `app/src/lib/advanced/pages.ts:178,262`, `app/src/components/settings/SourcesSection.svelte:22-47,131-157`, `app/src/lib/settingsView.ts:36-49`, `app/src/lib/types.ts:156-173,259-264`, `app/src/lib/backend/mock.ts:210-218`, `mockSettings.ts:55-60,256-261`, `app/src/lib/i18n/en.json`, `it.json`
- Modify (rimozione di `smartBlockedBy`): `crates/oma-ipc/src/status.rs`, `crates/oma-win/src/svc/link.rs`, `svc/status.rs`, `app/src-tauri/src/service.rs`
- Test: `app/src/components/advanced/SensorTable.test.ts`, `DevicePage.test.ts` (stessa directory), `KpiRow.test.ts` (nuovo), `app/src/lib/advanced/pages.test.ts`, `app/src/components/settings/SourcesSection.test.ts`, test mock e i18n esistenti

**Interfaces:**
- Consumes: `LiveStore.quality`, `LiveStore.diskPower` (Task 13); `ServiceSources.drives` (Task 5); proprietà `smartDefault` (Task 11).
- Produces: `blockingDiskNames(drives: SourceDrive[], schema: Schema | null, t): string[]` — nome del dispositivo dello schema se `deviceId` lo trova, altrimenti `model`, altrimenti `settings.sources.smart.diskNumber`.
- Estendere `KpiDef` con `sensorId?: string` per i KPI di misura diretta e passare a `KpiRow` `qualityOf: (id: string) => 0 | 1 | 2`. Il KPI può così leggere la qualità del sensore effettivo (il suo `id` è un nome di KPI, non un id sensore); non usare `secondary` chiusa su una qualità ottenuta alla discovery. Conservare `secondary` esistente per le altre informazioni e comporla con «Ultima lettura» nel rendering quando c'è un valore storico. Il KPI del picco resta una statistica storica, non una nuova misura.

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
  - `SensorTable.test.ts`: `a suspended value is muted and labelled as the last reading`; `a held value looks like a fresh one`; `a suspended sensor without a value shows no reading`. `KpiRow.test.ts`: stesse verifiche sulla temperatura principale, inclusa la transizione dinamica `Fresh → Held → Suspended → Fresh` (grigio solo in `Suspended`).
  - `DevicePage.test.ts`: `a disk in standby shows its state`; `an idle disk shows "Inattivo"`; `an active disk shows no state label`; `an unknown or removed disk keeps no state label`.
  - `SourcesSection.test.ts`: `a closed smart gate names the disk by device, model or number` (tre dischi bloccanti: uno nello schema, uno con solo `model`, uno senza nulla → "Disco 4"); `a usb disk starts with smart off and shows the warning`; `turning a usb disk on adds it to smartEnabledDrives` (patch `{ sources: { smartEnabledDrives: ['storage/usb'], smartDisabledDrives: [] } }`); `turning a normal disk off removes it from smartEnabledDrives`.
  - `pages.test.ts`: `smartDefault` è nascosta come `smartSelectable`.
- [ ] **Step 2:** `cd app && pnpm test` → falliscono.
- [ ] **Step 3: implementazione.** Un valore presente con qualità `Suspended` usa `color: var(--text-muted)` e «Ultima lettura» in tabella e KPI; `Held` non cambia l'aspetto; con valore assente si mostra il trattino, senza fingere una lettura. L'etichetta di stato usa `.tag` accanto all'intestazione della pagina del disco, solo nella vista Avanzata: la vista Semplificata non nomina i singoli dischi e resta com'è (decisione dell'utente, 2026-10-02). La UI legge lo stato corrente dal `LiveStore`, mai da proprietà di discovery. `setSmart` scrive atomicamente entrambi gli elenchi disgiunti: accendere USB aggiunge enabled e rimuove disabled; spegnere rimuove enabled e aggiunge disabled. Rimuovere `smartBlockedBy`/`smart_blocked_by` da Rust, TypeScript, mock e test.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build` e `cargo test --workspace` → PASS.
- [ ] **Step 5: commit** `feat(ui): show disk power state, last readings and the drives that block SMART`.

---

### Task 16: finestra di attività del servizio estesa all'intero intervallo

Aggiunto il 2026-10-02 dopo la revisione finale, su decisione dell'utente. Si esegue prima del Task 15.

**Files:**
- Modify: `service/OpenMonitorAdvanced.Service/Sensors/DiskActivityProbe.cs` (`ActivityWatch`, `DiskActivity`), `Sensors/SensorHub.cs` (storage worker: `RunStorageDue`, `StorageOnce`)
- Test: `DiskActivityTests.cs`, `SensorHubTests.cs`

**Interfaces:**
- Il riferimento (baseline) non si prende più 10 s prima del giro: si prende **alla fine di ogni giro**, dopo l'ultimo `IsSpunDown` e l'ultimo `Update` di quel giro e prima della pubblicazione. Il worker torna a un solo risveglio per giro (30 s).
- Attività recente = contatori cresciuti tra quel riferimento e il campione all'inizio del giro successivo. Il riferimento vale se appartiene alla stessa identità, se l'elenco dei dischi non è cambiato e se non è più vecchio di `StorageInterval` + 2 s; le invalidazioni restano quelle del Task 10 (sospensione/gap, hot-plug, storage spento, assenza di sottoscrittori). Un giro fallito prima della fine non lascia riferimento.
- `DiskActivity.Window` (10 s) sparisce dal servizio; resta invariata la finestra di 10 s del nucleo (`storage_gate::ACTIVITY_WINDOW`).
- Tutte le altre regole del Task 10 e le decisioni R11–R13 e I3 restano invariate.

- [ ] **Step 1: test che falliscono:** `TheBaselineIsTakenAtTheEndOfARound` (dopo un giro `RunStorageDue()` restituisce 30 s, nessun risveglio intermedio), `IoAnywhereBetweenTwoRoundsIsActivity` (contatori cresciuti 1 s dopo la fine del giro: il giro successivo chiede e aggiorna), `TheServicesOwnQueriesAreNotActivity` (contatori cresciuti durante il giro, prima del riferimento: il giro successivo è `idle`), `ABaselineOlderThanOneIntervalIsNotUsed`, `AFailedRoundLeavesNoBaseline`; adattare `TheWorkerWakesTenSecondsBeforeARound`, `ALateBaselineDoesNotAuthorizeSmart` e gli altri test che presuppongono il risveglio a −10 s, conservando ciò che dimostravano.
- [ ] **Step 2:** `dotnet test service/OpenMonitorAdvanced.slnx --filter "FullyQualifiedName~DiskActivity|FullyQualifiedName~SensorHub"` → falliscono.
- [ ] **Step 3: implementazione**; `dotnet test service/OpenMonitorAdvanced.slnx` e `pwsh scripts/check-trim-warnings.ps1` → PASS.
- [ ] **Step 4: commit** `fix(service): watch disk activity across the whole interval between two rounds`.

---

### Task 17: le letture sospese non contano come misure

Aggiunto il 2026-10-02 dopo la revisione finale, su decisione dell'utente (spec §6). Si esegue prima del Task 15.

**Files:**
- Modify: il punto in cui il tick entra nello storico (`crates/oma-core/src/history.rs` o il suo chiamante in `sampler.rs`/shell), `crates/oma-core/src/stats.rs` se le statistiche non derivano dallo storico, `crates/oma-core/src/csv.rs` (`row_line`), `app/src-tauri/src/log/session.rs` (`on_tick`) e i test accanto; la documentazione del formato CSV dove esiste (spec M5c, `docs/`, README)
- Test: gli stessi file; `app/src-tauri/src/log/session/tests.rs`

**Interfaces:**
- Consumes: `TickOutput.quality: Vec<Quality>` (Task 1).
- Produces:
  - **Storico e statistiche:** un valore con qualità `Suspended` entra nello storico come assente (`None`), con qualunque valore lo accompagni. `Fresh` e `Held` restano invariati. Il valore corrente pubblicato all'interfaccia (`oma:snapshot`) non cambia: resta l'ultima lettura con il suo codice di qualità.
  - **CSV:** la cella di un valore `Suspended` è il testo `suspended` (senza virgolette, minuscolo, uguale in tutte le lingue: è un dato, non un'etichetta); un valore assente non sospeso resta la cella vuota di oggi; un valore `Suspended` senza lettura precedente è anch'esso `suspended`. Se il file ha un indicatore di versione o un'intestazione che descrive il formato, va aggiornato; nessuna migrazione dei file esistenti.
  - Nessuna allocazione nuova per tick nel percorso del log e dello storico oltre a quelle di oggi (budget di `docs/perf-budget.md`).
- Il tray non cambia. Verificare leggendo `tray.rs` se può mostrare una temperatura di disco: se sì, riportarlo nel report senza modificarlo.

- [ ] **Step 1: test che falliscono:** storico — `a_suspended_value_enters_the_history_as_absent`, `a_held_value_enters_the_history_unchanged`, `statistics_ignore_suspended_ticks` (min/max/media/picco calcolati su misure vere con tick sospesi in mezzo), `the_history_window_has_a_gap_while_a_sensor_is_suspended`; CSV — `a_suspended_value_is_written_as_the_word_suspended`, `an_absent_value_is_still_an_empty_cell`, `a_held_value_is_written_as_a_number`, `a_suspended_cell_needs_no_quoting_under_either_separator`; sessione di log — `a_tick_with_a_suspended_sensor_writes_suspended_in_its_column`.
- [ ] **Step 2:** `cargo test -p oma-core` e `cargo test -p oma-app log` → falliscono.
- [ ] **Step 3: implementazione.** Un solo punto di conversione per lo storico e uno per il log; nessuna modifica ai provider né al payload dell'interfaccia.
- [ ] **Step 4:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cd app && pnpm test && pnpm check` → PASS.
- [ ] **Step 5: commit** `feat(core): keep suspended readings out of the history and mark them in the CSV log`.

---

### Task 15: Verifiche dal vivo, documenti e chiusura

**Files:**
- Modify: `docs/follow-ups.md`, `README.md`, `README.it.md`, `CLAUDE.md`, `docs/perf-budget.md`, questo piano ("Esito dell'esecuzione")
- Remove from the working tree after verification: `crates/oma-win/examples/m6b_wake.rs` (programma temporaneo non tracciato; non includerlo in alcun commit). Archiviare prima in `target/spike/m6b/` se contiene modifiche locali da conservare; non eliminare altri file non tracciati

- [ ] **Step 1: build per l'utente.** `pwsh scripts/build-installer-payload.ps1`, poi `cd app && pnpm tauri build --bundles nsis`. L'installer lo esegue l'utente.
- [ ] **Step 2: verifiche V1–V9 della spec §9.2, con l'utente**, una per volta, con `sat-probe.ps1` e l'osservazione di `GetDevicePowerState`. V3 su entrambe le modalità è obbligatoria per chiudere la M6b: un fallimento si isola (query del nucleo, `CHECK POWER MODE`, SMART, programma esterno), si corregge e si ripetono V1–V3 e V8. V6: cercare nel log del servizio se la chiavetta compare tra i dischi di LibreHardwareMonitor.
- [ ] **Step 3: budget.** `pwsh scripts/measure-footprint.ps1` e righe M6b in `docs/perf-budget.md`.
- [ ] **Step 4: `docs/follow-ups.md`.** Chiudere: USB e gate D6 (fallback SAT), `smartGateClosed` (sostituito da `drives`), copertura delle regole con l'HDD in standby, controllo "HDD standby" con la causa trovata. Aggiungere i limiti del §8 della spec e, tra i controlli dovuti, l'hard disk USB in standby. Scrivere la bozza della segnalazione a DiskInfoToolkit (ri-identificazione a ogni `DBT_DEVNODES_CHANGED`): si pubblica solo su richiesta dell'utente.
- [ ] **Step 5: README** ("Known limits" in entrambe le lingue) con i limiti visibili all'utente; **`CLAUDE.md`**: stato della M6b e una riga sul protocollo v3.
- [ ] **Step 6:** archiviare se necessario e rimuovere il solo programma temporaneo `crates/oma-win/examples/m6b_wake.rs`; in PowerShell: `$env:PYTHONHASHSEED = '0'`, poi `graphify update .`. Nessun aggiornamento del grafo necessario per sole modifiche ai documenti.
- [ ] **Step 7: verifica completa.**
  ```
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  dotnet test service/OpenMonitorAdvanced.slnx
  pwsh scripts/check-trim-warnings.ps1
  pnpm --dir app test
  pnpm --dir app check
  pnpm --dir app build
  ```
  I comandi sopra partono dalla radice. In più gli agenti eseguono `cargo test -p oma-win -- --include-ignored --skip reads_disk_temperatures_on_this_machine`; l'utente esegue `cargo test -p oma-win reads_disk_temperatures_on_this_machine -- --ignored` con i dischi già svegli e dopo le prove di standby. Se il filtro del Task 10 non è stato deciso o V3 non passa in entrambe le modalità, registrare M6b come non completata anche con tutte le suite verdi.
- [ ] **Step 8:** scrivere "Esito dell'esecuzione" in questo piano (verifiche, decisioni, ciò che resta non verificato) e fare commit `docs: record the M6b outcome and follow-ups`. Poi `superpowers:requesting-code-review` sull'intero branch (con `ffi-safety-reviewer` per `storage_ioctl.rs` e `protocol-parity-reviewer` per il protocollo) e `superpowers:finishing-a-development-branch`.

La release 0.4.0 (spec D6) segue il flusso di `docs/release.md` dopo il merge, su richiesta dell'utente.

---

## Esito del punto di controllo (Task 0 e Task 4)

Prove del 2026-10-02 sul PC di sviluppo: disco 0 = HDD SATA ST2000DM008 (`D:`), piano Prestazioni eccellenti, timeout disco 60 s (CA), TR-VISION HOME chiuso, servizio installato 0.3.0 (protocollo v2). Osservazioni passive di 10 minuti con `target/spike/m6b/watch-power.ps1` (accesso 0); log in `target/spike/m6b/` (non tracciati).

| Prova | Esito |
|---|---|
| Controllo: app, servizio e TR-VISION chiusi | `on=False` dopo 3 min 44 s |
| Task 0, servizio da solo con sottoscrittore (chiavetta collegata, gate chiuso, nessuna lettura SMART) | **mai spento** in 10 minuti |
| `CHECK POWER MODE` nativo da solo, ogni 30 s | **mai spento**; il primo comando, a disco spento da Windows, 3084 ms |
| `CHECK POWER MODE` via SAT16 da solo, ogni 30 s | **mai spento** |
| `IOCTL_DISK_PERFORMANCE` (accesso 0) ogni 15 s | `on=False` dopo 5 minuti; a disco spento risponde in 4 ms senza riaccenderlo |
| Task 4, bisezione `storage,poll-storage` (binari di `b5e69a1`) | `still in standby after 50 s` su entrambe |
| Task 4, app di prova da sola, senza sonde | `on=False` dopo circa 5 minuti, poi spento fino alla fine |
| Task 4, app di prova, standby forzato, SAT16 ogni 30 s | prima esecuzione: `STANDBY` per 4 minuti, poi attivo alle 17:58 (non riprodotto, causa ignota); seconda: `STANDBY` su 20 righe, contatori fermi |

**Decisione.** Il percorso locale (Task 3) è confermato in modalità anti-cheat. Il mancato spegnimento con il servizio non dipende dalla lettura SMART ma dal controllo di stato: il Task 10 è obbligatorio ed è stato riscritto (spec §4.4, approvato dall'utente il 2026-10-02). La modalità con servizio collegato si verifica con la build v3 nel Task 15.

## Esito dell'esecuzione

Eseguito tra il 2026-10-02 e il 2026-10-04 in modalità subagent-driven sul branch `feat/m6b-dischi-protocollo-v3` (base `c222a97`): un implementer e una revisione per ciascun task, le revisioni dedicate di FFI e parità del protocollo, poi la revisione dell'intero branch con un'unica ondata di correzioni prima della build per le verifiche dal vivo (ruling R17). Le prove dal vivo sono state fatte con l'utente sul PC di sviluppo, senza input sintetico; log e strumenti in `target/spike/m6b/` (non tracciati).

**Esito: M6b completata.** Con l'app aperta, con o senza servizio, Windows spegne l'HDD inattivo (V3 in entrambe le modalità) e uno standby forzato resta tale (V1, V2); con la chiavetta USB collegata lo SMART degli altri dischi resta acceso (V4). Il filtro del Task 10 è stato deciso (stato di alimentazione di Windows e attività del disco) e V3 è passata in entrambe le modalità, quindi la condizione del Task 15 per dichiarare completata la M6b è soddisfatta.

| Passo | Esito |
|---|---|
| Task 0 e Task 4 (punto di controllo) | Vedi «Esito del punto di controllo». Il servizio 0.3.0 da solo impediva lo spegnimento anche senza leggere lo SMART: `CHECK POWER MODE` azzera il timer di inattività di Windows e riaccende un disco spento da Windows. Da qui il Task 10 riscritto (spec §4.4, approvato dall'utente il 2026-10-02). |
| Task 1-14 | Completati, ciascuno rivisto; Task 6 anche con la revisione di parità del protocollo. |
| Revisione finale (`c222a97..1372dea`) | «Con correzioni», nessun Critical. Corretti I1 (temperatura principale persa da un SSD/NVMe associato al servizio), I2 (un tick senza sensori del servizio a ogni cambio della tabella dei dischi), I3 (SMART vuoto di un HDD quieto dopo una ripresa o un giro tardivo), più README delle fixture, `smartOn` e un test di `DevicePage` (`1372dea..820b880`); riesame mirato senza nuovi rilievi. Revisione FFI: nessun nuovo `unsafe`. Parità del protocollo: rispettata. |
| Task 16 (`130d854`) | Finestra di attività del servizio estesa a tutto l'intervallo tra due giri: riferimento dei contatori preso alla fine del giro precedente (decisione dell'utente, spec §4.4). |
| Task 17 (`8a62521`, `4608bed`) | Valori `Suspended` fuori da storico e statistiche, cella `suspended` nel log CSV, interruzione della serie dal vivo (decisione dell'utente). |
| Build per l'utente | Installer di `4608bed` (`OpenMonitor Advanced_0.3.0_x64-setup.exe`, 12,6 MB); installato ed eseguito dall'utente. |
| Prova 1, servizio da solo | Hello con protocollo 3, gate aperto, chiavetta `smartOff` senza blocco; disco 0 `idle` dal secondo giro, spento da Windows alle 22:10:36 e rimasto spento. |
| Prova 2, servizio da solo con I/O su `D:` | Avvio con l'HDD spento: `standby`, gate chiuso, schema senza dispositivi. Accesso a `D:`: `active`, gate aperto, nuova lettura; poi `idle`, e di nuovo `active` all'apertura di un file: i contatori `IOCTL_DISK_PERFORMANCE` vedono l'I/O di altri handle. |
| V3, app e servizio (prova 3) | Prima corsa dopo un avvio a freddo: spegnimento dopo circa 11 minuti, per I/O di terzi (la temperatura massima registrata prova una lettura su attività vera). Corsa pulita: contatori fermi dalle 01:36:06, disco spento alle 01:41:51 e rimasto spento. **Superata.** |
| V3 e V8, app senza servizio (anti-cheat) | Prima corsa con la build `4608bed` e servizio fermo: disco spento alle 22:36:14 e rimasto spento, HDD «Inattivo» senza temperatura (mai letto senza attività). V8 del 2026-10-04: contatori fermi dalle 02:07:29, disco spento alle 02:12:44 e rimasto spento. **Superate.** |
| V1, standby forzato, app e servizio (prova 4) | Contatori fermi dalle 01:51:11; Windows ha spento il disco alle 01:56:11, circa 5,5 minuti dopo l'ultimo comando della sonda: nessun comando del servizio ha azzerato il timer. **Superata.** |
| V2, standby forzato, anti-cheat | Prima prova: due scritture subito dopo lo standby (con ogni probabilità scritture differite di NTFS dovute all'accesso dell'utente alla cartella), che hanno risvegliato il disco; l'app non scrive su `D:`. Ripetizione con un minuto di attesa dopo l'accesso: `STANDBY IMMEDIATE` alle 02:38:52, nessuna lettura né scrittura, Windows spegne il disco alle 02:44:23 e l'app passa da «Inattivo» a «In standby» (senza servizio l'app non vede uno standby deciso dal firmware). **Superata.** |
| Letture SMART e contatori di attività (spec §4.4, «da confermare dal vivo») | Prova 1: giro con domanda e lettura SMART alle 22:05:00, giro successivo `idle` alle 22:05:29 senza accessi dell'utente: le letture del servizio non risultano attività (il riferimento a fine giro le esclude comunque). |
| V4, chiavetta all'avvio del servizio | La chiavetta (PhysicalDrive4, bus 0x07) dà ancora gli errori Win32 1 e 50 sulle vie native ma non tiene più spento lo storage; SMART degli altri dischi acceso. **Superata.** |
| V5, vista Semplificata con l'HDD in standby | Banner «Tutto in ordine», nessun allarme sul disco. **Superata.** |
| V6, chiavetta nel log | Con il servizio 0.3.0 LibreHardwareMonitor la identificava (`SanDisk pSSD` con spazi e NUL nel nome); con il servizio v3 è `smartOff` e LHM non la costruisce. Annotata. |
| V7, budget | Finestra 0,92 % (7 processi; nucleo 0,05 %), 166,9 MB; tray 0,05 %, 18,2 MB; servizio 0,03-0,09 %, 57,1-62,5 MB. **Superata** (`docs/perf-budget.md`, M6b). |
| V9, HDD «Inattivo», anti-cheat acceso e spento | Avviso della modalità base, tag «Inattivo», 35 °C «Ultima lettura», righe del solo servizio rimosse; al ritorno del servizio una sola riga «Temperatura», lettura fresca al primo giro dell'episodio (R11), nessun nuovo allarme. **Superata.** I «—» visti una volta in min/max/media subito dopo la riconnessione erano il transitorio delle revisioni dello schema: verificato a parte, chiuso. |
| Verifica completa su `4608bed` più la correzione del commento di `SensorHub.cs` | `cargo fmt --check` e `cargo clippy -D warnings` puliti; `cargo test --workspace` 1063 superati, 0 falliti; `dotnet test` 532 superati; `check-trim-warnings.ps1` OK; Vitest 630 superati in 44 file; `svelte-check` 0 errori e 0 avvisi; `pnpm build` riuscita; `cargo test -p oma-win -- --include-ignored` (esclusi `reads_disk_temperatures_on_this_machine` e `records_this_machine_schema`) 511 superati, 0 falliti. |
| Pulizia | `crates/oma-win/examples/m6b_wake.rs` archiviato in `target/spike/m6b/` e rimosso, mai committato; grafo aggiornato. |
| Merge e release | Revisione dell'intero branch e chiusura del branch a cura del controller. La release 0.4.0 (D6) segue `docs/release.md` dopo il merge, su richiesta dell'utente. |

**Decisioni prese durante l'esecuzione** (il registro completo è nel ledger SDD):

- **Task 10 obbligatorio e riscritto** (2026-10-02, approvato dall'utente): il servizio non chiede nulla a un disco che Windows ha spento o che non ha avuto attività di lettura/scrittura; nuovo stato `idle` nel protocollo. R11 e R13: un disco che Windows riporta acceso si interroga una volta, senza attività, la prima volta che è osservato (inizio di episodio, SMART acceso, disco nuovo), con l'occasione consumata prima della domanda; un passaggio da spento ad acceso conta come attività.
- **Task 16** (decisione dell'utente): la finestra di attività del servizio copre tutto l'intervallo tra due giri, non solo gli ultimi 10 s (era il ruling R19, portato all'utente invece di correggerlo in silenzio).
- **Task 17** (decisione dell'utente): un valore sospeso non entra in storico e statistiche e nel CSV è la parola `suspended`; il formato del CSV cambia senza compatibilità con i file precedenti.
- **R3:** dopo una disconnessione la temperatura storica di un disco inattivo è `Suspended`, non `Held`.
- **R7:** il poll di riscaldamento PDH dopo una rediscovery non apre né chiude la finestra di attività.
- **R8:** un disco senza modello e seriale non conserva la via del controllo di stato e riceve entrambi i comandi a ogni giro in cui viene interrogato.
- **R9:** con `SenseInfoLength = 0` il sense data si limita ai byte restituiti.
- **R10:** i valori conservati passano solo da un giro iniziato entro due intervalli; con I3 un giro scaduto riapre l'interrogazione una tantum.
- **R15:** sotto il servizio i sensori di temperatura aggiuntivi di un disco seguono da soli la regola dell'attività locale.
- **R17:** revisione finale e correzioni prima della build per le verifiche dal vivo, così l'utente ha provato la build corretta.
- **R18:** un'unica ondata di correzioni (I1-I3 e tre minori); tutto il resto in `docs/follow-ups.md`.

**Non verificato:**

- `cargo test -p oma-win reads_disk_temperatures_on_this_machine -- --ignored`, da eseguire dall'utente con i dischi svegli dopo le prove di standby: **dovuto**.
- Un hard disk USB dietro un bridge (manca l'hardware), le verifiche suggerite dalla revisione finale (SSD senza sensore locale con il servizio, HDD che alterna attivo e inattivo con un grafico della CPU aperto, sospensione e ripresa con un HDD quieto), l'hot-plug di un disco con lo storage acceso: in `docs/follow-ups.md`, «Manual checks owed after M6b».
- Limiti dichiarati (spec §8 e quelli emersi dal vivo, tra cui l'HDD addormentato all'avvio del servizio che tiene chiuso il gate per tutti i dischi): «Limits declared in M6b» in `docs/follow-ups.md` e «Known limits» nei README. La segnalazione a DiskInfoToolkit è in bozza nello stesso file e si pubblica solo su richiesta dell'utente.
