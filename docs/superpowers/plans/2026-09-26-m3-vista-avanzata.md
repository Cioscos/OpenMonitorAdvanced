# Milestone 3 — Vista Avanzata: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Obiettivo.** La vista Avanzata (layout "A", spec §7.3) mostra ogni componente in una pagina dedicata, raggiungibile da una barra laterale. Ogni pagina contiene:
- 4 KPI;
- un grafico storico uPlot con finestra 1m / 5m / 30m / 1h e scelta delle serie;
- una tabella dei sensori raggruppata per categoria, con valore attuale, min, max e media, e un pulsante "azzera";
- il badge della fonte e il segno "sperimentale";
- le proprietà statiche del dispositivo;
- sulle GPU, l'elenco dei processi che le usano.

La milestone chiude anche i punti che la spec rimanda a M3 (§4.2, §5.1, §5.2 punto 5, §13.6) e i seguiti obbligatori di M1.

**Architettura.**
- **Nucleo.** `oma-core` aggiunge le statistiche dall'avvio (`Stats`), l'istante di avvio del campionamento e lo storico sottocampionato per le finestre lunghe. Tre nuovi comandi Tauri li espongono: `get_stats`, `reset_stats`, `get_session`.
- **Provider.** `oma-win` aggiunge:
  - l'identità di riserva dei dischi;
  - le temperature dei dischi;
  - i nuovi campi GPU: carico encoder e decoder, link PCIe;
  - le proprietà statiche dei dispositivi: limiti e soglie;
  - l'uso delle GPU per processo, pubblicato in una tabella condivisa che il comando `get_gpu_processes` legge.
- **Interfaccia.** La UI Svelte costruisce la vista Avanzata sopra lo store live esistente:
  - barra laterale, pagine e grafico uPlot;
  - tabella con statistiche interrogate ogni secondo, solo mentre la finestra è visibile.

**Tech stack.** Invariato rispetto a M2: Rust 1.90 (pinnato), `windows` 0.62, Tauri 2.11, Svelte 5.57, TypeScript 6.0, Vitest 5. In più:
- `uplot` 1.6.32 (MIT, 23 KB gzip, compatibile con la CSP attuale senza modifiche);
- le feature di `windows` `Win32_Devices_DeviceAndDriverInstallation` (Task 4), `Win32_System_Power` (Task 5), `Win32_Devices_Properties` (Task 6), `Win32_System_Diagnostics_ToolHelp` (Task 8).

Ogni feature si aggiunge nel task che la usa per primo.

**Spec.** `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`, da leggere insieme a questo piano. Sezioni implementate qui:
- §4.2: min/max/media dall'avvio, con azzeramento;
- §5.1: temperatura dei dischi;
- §5.2 punto 5: extra GPU;
- §7.1: badge "dati non aggiornati";
- §7.2: clic su un riquadro verso la pagina Avanzata;
- §7.3 completa;
- §13.6 risolto.

**Prove fatte durante la stesura** (prototipi usa e getta, su questa macchina, da utente normale):
- **uPlot in WebView2** (build release, `scripts/measure-footprint.ps1`, 60 s, finestra aperta):

  | Caso | CPU | Totale privato |
  |---|---|---|
  | Senza grafico | 0,04% | 113,1 MB |
  | 8 serie × 3600 punti | 0,08% | 148,0 MB |
  | 20 serie × 3600 punti | 0,04% | 219,0 MB (oltre il budget) |

  - `setData` con 20 × 3600 punti costa 0,04–0,2 ms.
  - Il costo è la memoria, non la CPU: da qui D3 e D4.
  - `visibilitychange` funziona in WebView2. Con `--minimized` la finestra non esiste proprio.
- **PDH per processo:**
  - un ciclo con circa 776 istanze di motore costa 1,77 ms (mediana), di cui circa 50 µs per `GPU Process Memory`;
  - nessuna perdita di memoria su 900 raccolte con processi che nascono e muoiono;
  - `OpenProcess` fallisce per 99 processi su 245, `dwm.exe` compreso;
  - Toolhelp32 dà i nomi di tutti i processi in circa 2,4 ms per istantanea.
- **NVML (RTX 4080, driver 617.14):**
  - costano 0,1 µs ciascuno: utilizzo di encoder e decoder, generazione e larghezza PCIe attuali e massime, vincoli del limite di potenza e soglie di temperatura;
  - valori letti: PCIe Gen1 a riposo e Gen4 sotto carico, x16; limite di potenza 150/370/320 W; soglie 94/99/90 °C;
  - da escludere: `PcieThroughput` blocca per circa 31 ms; `TotalEnergyConsumption` ha il p95 a circa 9 ms; le liste dei processi in esecuzione non danno la memoria su WDDM.
- **PnP, `CM_Get_DevNode_PropertyW`, insieme di proprietà PCI:**
  - per le proprietà massime del link, qualunque vendor: Gen4 x16 su entrambe le GPU, circa 35 µs;
  - il link "attuale" di PnP è una fotografia presa all'avvio del dispositivo, non un valore vivo.
- **ADL, sensori PMLog 40/41 sull'iGPU:** restano costanti (3/16) e l'unità non è documentata, quindi si escludono.
- **Dischi** (4 dischi: HDD SATA, SSD SATA, 2 NVMe; tutti GPT):
  - `IOCTL_STORAGE_QUERY_PROPERTY`, `IOCTL_DISK_GET_DRIVE_LAYOUT_EX` e `IOCTL_STORAGE_GET_DEVICE_NUMBER` funzionano con accesso 0, senza privilegi;
  - anche l'id dell'istanza PnP del disco si legge senza privilegi;
  - `StorageDeviceTemperatureProperty` funziona senza privilegi su NVMe e HDD. L'SSD SATA non la supporta (`ERROR_INVALID_FUNCTION`: manca il supporto, non è un problema di privilegi). Questo risolve §13.6.

**Verifica della stesura precedente.** La stesura precedente riporta che un integratore ha applicato i Task 1–14 in ordine in un worktree temporaneo. I risultati seguenti sono evidenze storiche riportate nel documento, non una nuova esecuzione del piano corretto in questa revisione:
- dopo ogni task, test Rust e frontend verdi, clippy e fmt puliti, `pnpm check` a 0 errori, parità delle chiavi en/it;
- conteggi finali: oma-core 79, oma-app 16, oma-win 194 (+18 ignorati) e labels 1; Vitest 168 test in 22 file;
- test hardware tutti verdi: 212 della libreria oma-win e 11 del provider;
- prova rapida del budget con la pagina GPU aperta, grafico a 1 h con 8 serie e 5 minuti di storico: finestra 128 MB, tray 18 MB, CPU 0,02–0,04%. Il Task 14 deve registrare solo le misure della build finale, con un'ora di storico.

Non verificati:
- i passi "verifica manuale (utente)";
- la misura con 61 minuti di storico sul profilo reale;
- IGCL su hardware Intel.

**Revisione del piano (25 settembre 2026).** Dopo la verifica dell'integratore sono stati corretti alcuni algoritmi e contratti:
- decimazione a intervalli bilanciati con buchi conservativi (Task 2, 9);
- temperature dei dischi per `Index` del driver, con nuovo tentativo sui dischi in standby (Task 5);
- pulizia della tabella dei processi sugli errori del provider (Task 8);
- grafico e statistiche legati alla revisione dello schema e alla visibilità (Task 11, 12, 13);
- misura con la finestra visibile per 61 minuti (Task 14).

Per queste correzioni sono stati aggiunti test di regressione. I conteggi attesi nei task li includono, ma sono calcolati e non osservati: se un conteggio differisce solo di qualche unità e tutti i test passano, l'implementer lo segnala nel report senza bloccarsi. Le verifiche complete e hardware si rieseguono durante l'implementazione.

Conteggi finali attesi dopo la revisione (quelli dell'integratore sono sopra): oma-core 81, oma-app 16, oma-win 197 (+18 ignorati), labels 1; Vitest 176 test in 22 file.

Limite noto: sulle pagine Rete, KPI e tabella usano i bit/s come la vista Semplificata, mentre l'asse e la legenda del grafico restano in byte/s. La scelta delle unità arriva con le impostazioni (M5).

**Decisioni** (già prese; ognuna con il costo se fosse sbagliata):
- **D1. Min/max/media nel nucleo, dall'avvio dell'app.** La WebView viene distrutta alla chiusura della finestra, mentre §4.2 chiede statistiche "dall'avvio". L'azzeramento vale per i sensori della pagina. Al cambio di fonte o di unità si azzera il singolo sensore, come per lo storico. *(Costo: trascurabile, un confronto e una somma per sensore a ogni ciclo.)*
- **D2. `startedAtMs`** è l'istante del primo ciclo del motore, letto con `get_session`. Il banner della vista Semplificata lo usa al posto del primo campione visto dalla finestra (seguito di M1). *(Costo: nessuno.)*
- **D3. Sottocampionamento nel nucleo per le finestre da 30 minuti in su.** `get_history` accetta `maxPoints`: al massimo 900 punti, con l'inviluppo min/max di 2 punti per intervallo bilanciato. *(Costo: una curva meno fine a 30m/1h; i picchi restano visibili negli intervalli interamente validi, i buchi si ampliano conservativamente all'intervallo.)*
- **D4. Al massimo 8 serie e 2 unità di misura per grafico.** *(Costo: per confrontare più di 8 sensori bisogna cambiare selezione.)*
- **D5. Uso GPU per processo nello strato PDH** (per ogni processo, il motore più carico) e pubblicato in `GpuProcessTable`. Non sono sensori: niente id e niente storico. *(Costo: circa 50 µs per ciclo sempre, anche in tray.)*
- **D6. Nomi dei processi con Toolhelp32**, solo quando compare un pid sconosciuto, con cache. Niente `NtQuerySystemInformation` (non documentata) e niente `OpenProcess`. *(Costo: circa 2,5 ms nei cicli in cui compaiono nuovi processi GPU.)*
- **D7. Link PCIe.**
  - Il valore attuale viene da NVML e da IGCL; IGCL è verificato solo con fake. Per le altre GPU manca, perché nessuna fonte senza privilegi lo dà vivo.
  - I valori massimi sono proprietà del dispositivo: da PnP per ogni vendor, oppure da NVML.

  *(Costo: le GPU AMD e le iGPU non mostrano il link attuale.)*
- **D8. Encoder e decoder NVML sono campi a sé** (`gpu.load.encoder` e `gpu.load.decoder`). I carichi per motore "Video encode/decode" restano solo da PDH. *(Costo: su NVIDIA ci sono due righe simili con significati diversi.)*
- **D9. Limiti e soglie come proprietà statiche del dispositivo.** Per la GPU (NVML): limite di potenza min/max/predefinito, soglie di temperatura. Per i dischi: soglie di attenzione e critica. Servono alle regole di M5. *(Costo: se il driver cambia i limiti a caldo, si vedono al primo rediscover.)*
- **D10. Esclusi da M3:**
  - energia NVML e throughput PCIe (troppo lenti);
  - liste dei processi NVML e sensori ADL 40/41;
  - pagina Batteria: manca il provider, si vedrà quando ci sarà un dispositivo batteria;
  - avviso "N sensori in più con il servizio" (M4);
  - `settings.json` (M5).

  Nel frattempo lo stato della vista Avanzata si salva in `localStorage`. *(Costo: le preferenze della vista Avanzata ripartono da capo con un nuovo profilo WebView2.)*
- **D11. Identità di riserva dei dischi:** numero di serie → GUID del disco GPT → firma MBR con la dimensione → id dell'istanza PnP. A ogni livello, gli id ambigui su questa macchina si scartano. Gli id basati sul numero di serie non cambiano. *(Costo: un disco senza serie spostato di porta, che arriva al livello PnP, cambia id e perde lo storico.)*
- **D12. Dati non aggiornati.**
  - La UI mostra un badge se non arrivano snapshot per più di max(5 s, 5 intervalli).
  - Il campionatore sopravvive a un panic in `Engine::tick`.
  - `History::push` non va più in panic sulle lunghezze.

  *(Costo: un tick perso diventa un buco nel grafico invece di un blocco.)*

## Global Constraints

Valgono tutti i vincoli di M1 e M2:
- Windows solo in `crates/oma-win` e nei `cfg(windows)` di `oma-app`;
- budget: nucleo a riposo < 1% di CPU, tray < 30 MB, finestra aperta < 200 MB in totale con WebView2; si misura il working set privato;
- intervallo di 1 s e storico di 1 ora;
- nessun privilegio e CSP stretta: `default-src 'self'; connect-src ipc: http://ipc.localhost; style-src 'self' 'unsafe-inline'; img-src 'self' data:`, invariata;
- stringhe in `en.json` e `it.json` con le stesse chiavi;
- id `<device_id>/<kind>/<name>` e chiavi `Label.key` come contratto Rust↔UI. Ogni nuova chiave di sensore va anche in `crates/oma-win/tests/labels.rs`;
- palette Synthwave: solo i token di `theme.css`;
- nessuna animazione continua; rispettare `prefers-reduced-motion`;
- licenza `GPL-3.0-or-later`: nessun testo da header dei vendor, nessun tag SPDX di terzi nei nostri sorgenti;
- librerie dei vendor caricate solo da System32 e mai scaricate;
- codice e commenti in inglese.

Vincoli in più per M3:
- **Rendering:** il grafico e i polling della pagina (statistiche 1 s, processi 2 s) si fermano quando `document.visibilityState` è `hidden`. Con la finestra chiusa non c'è WebView (spec §2.2).
- **Grafico:** al massimo 8 serie e 2 unità. Le finestre ≥ 30 min si chiedono con `maxPoints` 900. Colori da `getComputedStyle` sui token.
- **Nuovi comandi Tauri in tre punti:** `generate_handler!` di `main.rs`, `build.rs`, `capabilities/default.json`.
- **Tempo per ciclo:** tutto il provider `gpu`, compresi i contatori per processo, resta entro i 200 ms della scadenza. Mai chiamate NVML bloccanti nel ciclo.
- **Verifiche dal vivo:** mai input sintetico o UI Automation sul desktop. Solo avvio e arresto di processi, log, lettura via CDP, oppure un passo "verifica manuale (utente)" esplicito.
- **Stato della vista Avanzata** in `localStorage` (chiavi `oma.advanced.section`, `oma.advanced.window`, `oma.advanced.series.<id>`), letto e scritto dentro `try/catch`.

## Review Focus

Condizioni che la spec implica ma che un test di funzionalità non coprirebbe da solo, in ordine di probabilità. Ogni riga ha un test nel task che possiede il codice.

1. **Finestra chiusa nella tray e riaperta.** La WebView viene distrutta e ricreata (spec §2.2). Min/max/media continuano dall'avvio dell'app e non dalla riapertura. La pagina, la finestra temporale e le serie scelte vengono ripristinate. Il banner della vista Semplificata conta dall'avvio del nucleo. Test:
   - `stats_cover_every_tick_since_start` e `started_at_is_the_timestamp_of_the_first_tick` (Task 1);
   - `session_reports_the_first_tick_and_the_interval` (Task 3);
   - `the last visited section is restored` e `the health banner counts from the start of the core session` (Task 10);
   - `the saved selection of the section wins over the defaults` (Task 11).
2. **L'hardware cambia mentre una pagina è aperta:** "Riattiva" delle librerie dei vendor, una chiavetta USB, un rediscover della GPU. Sensori che spariscono o cambiano fonte non devono lasciare serie fantasma né statistiche di un'altra fonte, e una sezione sparita riporta alla CPU. Test:
   - `stats_follow_the_history_retention_rule` e `a_removed_id_that_comes_back_starts_empty` (Task 1);
   - `fitSelection drops unknown ids and whatever breaks the limits` (Task 11);
   - `a missing section falls back to the CPU without forgetting the choice` (Task 10);
   - `a reply that started before a reset is dropped` (Task 12).
3. **Un'ora di storico su molte serie.** Il budget di 200 MB vale anche con la pagina GPU aperta, 8 serie e finestra di 1 ora: la decimazione non deve mai superare 900 punti, i picchi degli intervalli interamente validi devono restare visibili e i buchi non devono sparire. Test:
   - `decimated_output_never_exceeds_max_points`, `decimation_emits_min_then_max_per_bucket`, `near_one_hour_has_no_oversized_final_bucket` e `a_mixed_bucket_preserves_the_gap_conservatively` (Task 2);
   - `history_with_max_points_is_decimated_and_clamped` (Task 3);
   - `long windows ask for decimated history and the choice persists` e `trim keeps exactly the window measured back from now` (Task 11);
   - misura del Task 14.
4. **Documento segnalato come nascosto da WebView2 (`visibilityState === 'hidden'`).** Grafico, statistiche e processi smettono di aggiornarsi; anche le risposte già in viaggio non devono ridisegnare il grafico. Alla ricomparsa ripartono subito, senza raffiche di richieste accumulate. La sola sovrapposizione di un'altra finestra non è una condizione rilevata dal codice: il contratto segue l'evento di visibilità. Test:
   - `rendering pauses while hidden and history is reloaded when visible` (Task 11);
   - `stops polling while hidden and polls at once when visible again` e `a slow reply does not pile up requests` (Task 12);
   - `refreshes every 2 s only while visible` (Task 13).
5. **Dati che smettono di arrivare** (un panic nel ciclo, un provider bloccato) **e dischi senza numero di serie** (VM, Storage Spaces, RAID RST, dischi clonati). L'interfaccia deve dire che i dati sono fermi invece di congelarsi in silenzio. Un disco, e il suo volume C:, non deve mai sparire solo perché manca il seriale. Test:
   - `a_panicking_tick_is_skipped_and_sampling_goes_on` e `push_with_too_few_values_stores_the_rest_as_missing` (Task 2);
   - `the stale badge appears after five silent seconds and goes away with new data` (Task 10);
   - `serial_less_disk_uses_the_gpt_disk_id`, `cloned_gpt_disks_fall_back_to_pnp` e `serial_ids_are_unchanged_since_m1` (Task 4).

---

## Mappa dei file

```
crates/oma-core/src/stats.rs              Stats, SensorStats                                            (Task 1)
crates/oma-core/src/engine.rs             stats, started_at_ms, rate limit dei log                      (Task 1, 2)
crates/oma-core/src/history.rs            window_decimated, push senza panic                           (Task 2)
crates/oma-core/src/sampler.rs            catch_unwind attorno al tick                                  (Task 2)
crates/oma-core/src/model.rs, sanitize.rs SensorKind::Link, Unit::PcieGeneration/Lanes, Source::Pnp    (Task 6)
app/src-tauri/src/commands.rs, main.rs, build.rs, capabilities/default.json
                                          get_history maxPoints, get_stats, reset_stats, get_session    (Task 3)
                                          get_gpu_processes, GpuProcessTable                            (Task 8)
crates/oma-win/src/storage_identity.rs    catena di identità di riserva                                 (Task 4)
crates/oma-win/src/storage_ioctl.rs       PhysicalDrive (accesso 0), IOCTL, letture little-endian       (Task 4, 5)
crates/oma-win/src/storage_temperature.rs StorageDeviceTemperatureProperty, aggiornamento ogni 30 s     (Task 5)
crates/oma-win/src/storage.rs             assegnazione degli id, temperature, soglie                    (Task 4, 5)
crates/oma-win/src/gpu/field.rs           +4 campi                                                      (Task 6)
crates/oma-win/src/gpu/layer.rs, mod.rs   GpuLayer::properties, merge, PCI per LUID                     (Task 6)
crates/oma-win/src/gpu/pnp.rs             PnpLayer: link massimo                                        (Task 6)
crates/oma-win/src/gpu/nvml.rs            encoder/decoder, PCIe, limiti e soglie                       (Task 6)
crates/oma-win/src/gpu/igcl.rs            PCIe via ctlPciGetState/Properties                            (Task 7)
crates/oma-win/src/gpu/processes.rs       GpuProcess, GpuProcessTable                                   (Task 8)
crates/oma-win/src/gpu/procname.rs        ProcessNames (Toolhelp32 + cache)                             (Task 8)
crates/oma-win/src/gpu/pdh.rs             contatori per processo                                        (Task 8)
crates/oma-win/src/lib.rs                 default_providers(vendor, processes)                          (Task 8)
crates/oma-win/tests/labels.rs            nuove chiavi di sensore                                       (Task 5, 6)

app/package.json                          uplot 1.6.32                                                  (Task 9)
app/src/lib/types.ts, backend/*.ts (+ decimate.ts, mockStats.ts), test/fake-backend.ts, format.ts,
  i18n/*.json, styles/theme.css                                                                       (Task 9)
app/src/lib/advanced/nav.ts, advanced/persist.ts, lib/stale.ts, live.svelte.ts,
  components/advanced/AdvancedView.svelte, Sidebar.svelte, DevicePage.svelte (minima), App.svelte,
  TopBar.svelte, simple/SimpleView.svelte                                                             (Task 10)
app/src/lib/advanced/chartData.ts, advanced/labels.ts, components/advanced/HistoryChart.svelte,
  test/uplot-stub.ts, test-setup.ts                                                                   (Task 11)
app/src/lib/advanced/pages.ts (groupSensors), advanced/statsPoller.svelte.ts,
  components/advanced/SensorTable.svelte                                                              (Task 12)
app/src/lib/advanced/pages.ts (kpisFor), KpiRow.svelte, DevicePage.svelte, DeviceInfo.svelte,
  GpuProcesses.svelte                                                                                 (Task 13)

scripts/seed-advanced-view.ps1, scripts/measure-footprint.ps1, docs/follow-ups.md,
  docs/perf-budget.md, README.md, docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md    (Task 14)
```

---

**Comandi:** i blocchi `bash` richiedono Git Bash. I test hardware (`#[ignore = "requires real Windows hardware"]`) si eseguono con `cargo test -p oma-win -- --include-ignored` e vanno fatti passare prima di chiudere M3. Macchina di sviluppo:
- RTX 4080 in `0000:01:00.0`;
- iGPU AMD in `0000:11:00.0`;
- nessuna GPU Intel;
- 4 dischi GPT: un HDD SATA, un SSD SATA senza sensore di temperatura, 2 NVMe.

**Ricerca nel codice:** su questa macchina è installato graphify. Prima di grep, usa `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"`.


### Task 1: `oma-core`: statistiche min/max/media (`Stats`), inizio del monitoraggio e integrazione nell'engine

**File:**
- Crea: `crates/oma-core/src/stats.rs`
- Modifica:
  - `crates/oma-core/src/lib.rs` (`pub mod stats;`)
  - `crates/oma-core/src/engine.rs` (campi `stats` e `started_at_ms`, tre metodi nuovi, aggiornamento in `tick`)
- Test:
  - `crates/oma-core/src/stats.rs` (modulo `tests`)
  - `crates/oma-core/src/engine.rs` (modulo `tests`)

**Interfacce:**
- Usa (già presenti):
  - `History::set_sensors(&mut self, ids: &[String])` e la regola di conservazione dell'engine: al cambio di schema una serie sopravvive solo se id, fonte e unità del sensore sono invariati (lista `retained` in `Engine::tick`);
  - `sanitize_sensor(sensor, value)`: i valori arrivano già ripuliti, quindi un valore implausibile è `None` e non entra nelle statistiche;
  - il fake `Script`/`Fake` e l'helper `inventory(device, &[nomi])` dei test di `engine.rs` (sensori `Load`, unità `Percent`, fonte `Mock`, id `"<device>/load/<nome>"`; a script esaurito il poll restituisce `Some(1.0)` per ogni sensore).
- Produce (nuova API pubblica, usata dal Task 3 e, tramite i comandi, dai Task 10 e 12):
  ```rust
  // crates/oma-core/src/stats.rs
  #[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SensorStats { pub min: f64, pub max: f64, pub avg: f64, pub count: u64 }
  #[derive(Debug, Default)]
  pub struct Stats { /* index: HashMap<String, usize>, acc: Vec<Acc> */ }
  impl Stats {
      pub fn new() -> Self;
      pub fn set_sensors(&mut self, ids: &[String]);
      pub fn push(&mut self, values: &[Option<f64>]);
      pub fn get(&self, ids: &[String]) -> Vec<Option<SensorStats>>;
      pub fn reset(&mut self, ids: &[String]);
  }
  // crates/oma-core/src/engine.rs
  impl Engine {
      pub fn stats(&self) -> &Stats;
      pub fn stats_mut(&mut self) -> &mut Stats;
      pub fn started_at_ms(&self) -> Option<u64>;
  }
  ```
  JSON di `SensorStats`: `{ "min": number, "max": number, "avg": number, "count": number }` (tipo `SensorStats` in `app/src/lib/types.ts`, Task 9).

**Perché nel core (decisione D1).** La spec §4.2 dice che min, max e media "partono dall'avvio dell'app". La WebView viene distrutta alla chiusura della finestra (spec §2.2) e l'app continua a campionare nella tray: se le statistiche vivessero nella UI ripartirebbero da zero a ogni apertura. Il costo nel core è trascurabile: per sensore e per tick un confronto per il minimo, uno per il massimo e una somma.

**Regole:**
- `Stats` tiene un accumulatore per sensore (minimo, massimo, somma, numero di campioni validi), nell'ordine dell'ultimo `set_sensors`.
- `set_sensors` ha la stessa semantica di `History::set_sensors`: gli accumulatori degli id ancora presenti restano, gli id nuovi partono vuoti. Un id rimosso e poi ricomparso riparte vuoto.
- `push` riceve i valori nell'ordine di `set_sensors`. Salta `None` e i valori non finiti. Se la lunghezza non coincide non va in panic: i valori in più si ignorano, i sensori senza valore non ricevono campioni.
- `get` restituisce un elemento per id richiesto, nello stesso ordine. `None` vale sia per un id sconosciuto sia per un sensore che non ha ancora campioni validi. `avg` è `somma / count`.
- `reset` riporta a vuoto gli accumulatori degli id indicati; gli id sconosciuti si ignorano. È il pulsante "azzera" di una pagina della vista Avanzata (Task 12), che passa gli id dei sensori della pagina.
- Nell'engine lo stesso elenco `retained` usato per lo storico si applica alle statistiche: `stats.set_sensors(&retained)` e poi `stats.set_sensors(&tutti_gli_id)`. Quindi un sensore che cambia fonte o unità riparte da zero sia nello storico sia nelle statistiche. `stats.push(&values)` si chiama subito dopo `history.push`, con gli stessi valori già ripuliti.
- `started_at_ms` è il `timestamp_ms` (ora Unix) del primo tick dell'engine e poi non cambia più (decisione D2). Il `HealthBanner` della UI lo userà al posto del primo campione visto dalla finestra (Task 10): è il follow-up M1 "il banner conta dal primo campione ricevuto, sbagliato dopo la riapertura dalla tray".

La somma è un `f64`: anche dopo un mese a 1 s (circa 2,6 milioni di campioni) di valori fino a 10^10 B/s l'errore di arrotondamento sulla media resta molte cifre sotto quelle mostrate. Non serve la somma compensata.

- [ ] **Step 1: Scrivi i test di `Stats` (falliscono)**

In `crates/oma-core/src/lib.rs` aggiungi il modulo, in ordine alfabetico dopo `sanitize`:

```rust
pub mod sanitize;
pub mod stats;
mod worker;
```

Crea `crates/oma-core/src/stats.rs` con il solo modulo dei test (l'implementazione arriva allo Step 3, sopra questo modulo):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn stats(min: f64, max: f64, avg: f64, count: u64) -> Option<SensorStats> {
        Some(SensorStats {
            min,
            max,
            avg,
            count,
        })
    }

    #[test]
    fn tracks_min_max_avg_and_count() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a", "b"]));
        s.push(&[Some(10.0), Some(-1.0)]);
        s.push(&[Some(30.0), Some(-3.0)]);
        s.push(&[Some(20.0), Some(-2.0)]);
        assert_eq!(
            s.get(&ids(&["a", "b"])),
            vec![stats(10.0, 30.0, 20.0, 3), stats(-3.0, -1.0, -2.0, 3)]
        );
    }

    #[test]
    fn unknown_ids_and_sensors_without_samples_have_no_stats() {
        let mut s = Stats::new();
        assert_eq!(s.get(&ids(&["a"])), vec![None]);
        s.set_sensors(&ids(&["a"]));
        assert_eq!(s.get(&ids(&["a", "nope"])), vec![None, None]);
    }

    #[test]
    fn missing_and_non_finite_values_are_skipped() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a"]));
        for v in [None, Some(f64::NAN), Some(4.0), Some(f64::INFINITY)] {
            s.push(&[v]);
        }
        s.push(&[Some(f64::NEG_INFINITY)]);
        assert_eq!(s.get(&ids(&["a"])), vec![stats(4.0, 4.0, 4.0, 1)]);
    }

    #[test]
    fn set_sensors_keeps_persisting_ids_and_starts_new_ones_empty() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a", "gone"]));
        s.push(&[Some(1.0), Some(5.0)]);
        s.set_sensors(&ids(&["new", "a"]));
        s.push(&[Some(7.0), Some(3.0)]);
        assert_eq!(
            s.get(&ids(&["a", "new", "gone"])),
            vec![stats(1.0, 3.0, 2.0, 2), stats(7.0, 7.0, 7.0, 1), None]
        );
    }

    #[test]
    fn a_removed_id_that_comes_back_starts_empty() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a"]));
        s.push(&[Some(1.0)]);
        s.set_sensors(&[]);
        s.set_sensors(&ids(&["a"]));
        assert_eq!(s.get(&ids(&["a"])), vec![None]);
    }

    #[test]
    fn reset_clears_only_the_named_sensors() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a", "b"]));
        s.push(&[Some(1.0), Some(2.0)]);
        s.reset(&ids(&["a", "unknown"]));
        assert_eq!(
            s.get(&ids(&["a", "b"])),
            vec![None, stats(2.0, 2.0, 2.0, 1)]
        );
        s.push(&[Some(9.0), None]);
        assert_eq!(s.get(&ids(&["a"])), vec![stats(9.0, 9.0, 9.0, 1)]);
    }

    #[test]
    fn a_value_count_mismatch_never_panics() {
        let mut s = Stats::new();
        s.set_sensors(&ids(&["a", "b"]));
        s.push(&[Some(1.0)]);
        s.push(&[Some(2.0), Some(3.0), Some(99.0)]);
        s.push(&[]);
        assert_eq!(
            s.get(&ids(&["a", "b"])),
            vec![stats(1.0, 2.0, 1.5, 2), stats(3.0, 3.0, 3.0, 1)]
        );
    }

    #[test]
    fn serializes_with_the_ts_contract_keys() {
        let value = serde_json::to_value(SensorStats {
            min: 1.0,
            max: 3.0,
            avg: 2.0,
            count: 2,
        })
        .expect("serialize");
        assert_eq!(
            value,
            serde_json::json!({ "min": 1.0, "max": 3.0, "avg": 2.0, "count": 2 })
        );
    }
}
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core stats`
Risultato atteso: la compilazione dei test fallisce con errori del tipo `failed to resolve: use of undeclared type Stats` (E0433) e `cannot find type SensorStats in this scope` (E0412): i tipi non esistono ancora.

- [ ] **Step 3: Implementa `Stats`**

In `crates/oma-core/src/stats.rs`, **sopra** il modulo `tests`, inserisci:

```rust
//! Running min/max/average per sensor since the app started (spec §4.2).
//!
//! They live in the core, not in the UI: the WebView is destroyed when the
//! window closes (spec §2.2), while the statistics must cover the whole
//! session, tray time included.

use std::collections::HashMap;

use serde::Serialize;

/// Statistics of one sensor over the samples seen since start or last reset.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorStats {
    pub min: f64,
    pub max: f64,
    pub avg: f64,
    /// Number of valid samples behind `avg`.
    pub count: u64,
}

#[derive(Debug, Clone, Copy)]
struct Acc {
    min: f64,
    max: f64,
    sum: f64,
    count: u64,
}

impl Acc {
    const EMPTY: Self = Self {
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        sum: 0.0,
        count: 0,
    };

    fn add(&mut self, value: f64) {
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.sum += value;
        self.count += 1;
    }

    fn stats(&self) -> Option<SensorStats> {
        (self.count > 0).then(|| SensorStats {
            min: self.min,
            max: self.max,
            avg: self.sum / self.count as f64,
            count: self.count,
        })
    }
}

/// One accumulator per sensor, in the order given to `set_sensors`.
#[derive(Debug, Default)]
pub struct Stats {
    index: HashMap<String, usize>,
    acc: Vec<Acc>,
}

impl Stats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the sensor list, like `History::set_sensors`: accumulators of
    /// ids that are still present are kept, new ids start empty.
    pub fn set_sensors(&mut self, ids: &[String]) {
        let previous: HashMap<String, Acc> = std::mem::take(&mut self.index)
            .into_iter()
            .map(|(id, i)| (id, self.acc[i]))
            .collect();
        self.acc = ids
            .iter()
            .map(|id| previous.get(id).copied().unwrap_or(Acc::EMPTY))
            .collect();
        self.index = ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
    }

    /// Adds one sample per sensor, in `set_sensors` order. Missing and
    /// non-finite values are skipped; values beyond the sensor list are
    /// ignored and sensors without a value simply get no sample.
    pub fn push(&mut self, values: &[Option<f64>]) {
        for (acc, value) in self.acc.iter_mut().zip(values) {
            if let Some(v) = value.filter(|v| v.is_finite()) {
                acc.add(v);
            }
        }
    }

    /// Statistics per requested id; `None` for an unknown id or a sensor
    /// without valid samples yet.
    pub fn get(&self, ids: &[String]) -> Vec<Option<SensorStats>> {
        ids.iter()
            .map(|id| self.index.get(id).and_then(|&i| self.acc[i].stats()))
            .collect()
    }

    /// Restarts the statistics of the given sensors; unknown ids are ignored.
    pub fn reset(&mut self, ids: &[String]) {
        for id in ids {
            if let Some(&i) = self.index.get(id) {
                self.acc[i] = Acc::EMPTY;
            }
        }
    }
}
```

`Acc` non deriva `Default`: un accumulatore vuoto deve partire da minimo `+∞` e massimo `-∞`, non da 0, altrimenti un sensore con soli valori negativi (tensioni negative, §8) avrebbe massimo 0. Il costruttore è la costante `Acc::EMPTY`.

- [ ] **Step 4: Esegui i test di `Stats` e verifica che passino**

Esegui: `cargo test -p oma-core stats`
Risultato atteso: 8 test OK in `stats::tests`.

- [ ] **Step 5: Scrivi i test dell'engine (falliscono)**

In `crates/oma-core/src/engine.rs`, nel modulo `tests`, aggiungi quanto segue subito prima di `backoff_doubles_up_to_one_minute`:

```rust
    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stats_cover_every_tick_since_start() {
        let (p, script) = fake("a", inventory("dev/a", &["x", "y"]));
        script.lock().unwrap().polls.extend([
            Ok(vec![Some(10.0), None]),
            Ok(vec![Some(30.0), Some(150.0)]),
            Ok(vec![Some(20.0), Some(5.0)]),
        ]);
        let mut e = Engine::new(vec![p], 10);
        for t in [1_000, 2_000, 3_000] {
            e.tick(t, t);
        }
        let got = e.stats().get(&ids(&["dev/a/load/x", "dev/a/load/y"]));
        let x = got[0].expect("x has samples");
        assert_eq!((x.min, x.max, x.avg, x.count), (10.0, 30.0, 20.0, 3));
        // 150 % is implausible and sanitized away before it reaches the stats.
        let y = got[1].expect("y has samples");
        assert_eq!((y.min, y.max, y.avg, y.count), (5.0, 5.0, 5.0, 1));
    }

    #[test]
    fn stats_follow_the_history_retention_rule() {
        let (p, script) = fake("a", inventory("dev/a", &["x", "y"]));
        let mut e = Engine::new(vec![p], 10);
        e.tick(1_000, 1_000);
        {
            let mut s = script.lock().unwrap();
            s.inventory.sensors[0].source = Source::Nvml;
            s.inventory.sensors.push(Sensor::new(
                "dev/a",
                SensorKind::Load,
                "z",
                Unit::Percent,
                Label::new("test"),
                Source::Mock,
            ));
            s.polls.push_back(Err(ProviderError::Rediscover));
        }
        e.tick(2_000, 2_000);
        e.tick(3_000, 3_000);
        let got = e
            .stats()
            .get(&ids(&["dev/a/load/x", "dev/a/load/y", "dev/a/load/z"]));
        // x changed source: its statistics restart with the new schema.
        assert_eq!(got[0].map(|s| s.count), Some(1));
        // y is unchanged: the sample of the first tick is still counted.
        assert_eq!(got[1].map(|s| s.count), Some(2));
        assert_eq!(got[2].map(|s| s.count), Some(1));
    }

    #[test]
    fn stats_reset_through_the_engine() {
        let (p, _) = fake("a", inventory("dev/a", &["x", "y"]));
        let mut e = Engine::new(vec![p], 10);
        e.tick(1_000, 1_000);
        e.stats_mut().reset(&ids(&["dev/a/load/x"]));
        let got = e.stats().get(&ids(&["dev/a/load/x", "dev/a/load/y"]));
        assert_eq!(got[0], None);
        assert_eq!(got[1].map(|s| s.count), Some(1));
        e.tick(2_000, 2_000);
        let got = e.stats().get(&ids(&["dev/a/load/x"]));
        assert_eq!(got[0].map(|s| s.count), Some(1));
    }

    #[test]
    fn started_at_is_the_timestamp_of_the_first_tick() {
        let mut e = Engine::new(Vec::new(), 10);
        assert_eq!(e.started_at_ms(), None);
        e.tick(5_000, 0);
        e.tick(6_000, 1_000);
        assert_eq!(e.started_at_ms(), Some(5_000));
    }
```

Sequenza del secondo test: al tick 2 il poll risponde `Rediscover`, quindi quel tick pubblica `None` per tutti e nessuna statistica riceve campioni; al tick 3 la nuova discovery produce lo schema con `x` in fonte `Nvml` (non conservata), `y` invariata (conservata) e `z` nuova.

- [ ] **Step 6: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core engine`
Risultato atteso: la compilazione dei test fallisce con `no method named stats found for struct Engine` (E0599), e lo stesso per `stats_mut` e `started_at_ms`.

- [ ] **Step 7: Integra `Stats` e `started_at_ms` nell'engine**

In `crates/oma-core/src/engine.rs`:

1. Negli `use`, dopo `use crate::sanitize::sanitize_sensor;`, aggiungi:

```rust
use crate::stats::Stats;
```

2. Sostituisci la struct `Engine` con:

```rust
pub struct Engine {
    slots: Vec<Slot>,
    schema: Schema,
    history: History,
    /// Min/max/average since start, same retention rule as `history`.
    stats: Stats,
    /// Unix time of the first tick: when monitoring started (not the window).
    started_at_ms: Option<u64>,
    seq: u64,
}
```

3. In `Engine::new`, sostituisci le ultime righe dell'inizializzazione:

```rust
            schema: Schema::default(),
            history: History::new(history_capacity),
            stats: Stats::new(),
            started_at_ms: None,
            seq: 0,
        }
    }
```

4. Dopo `pub fn sequence(&self) -> u64 { self.seq }` aggiungi:

```rust
    pub fn stats(&self) -> &Stats {
        &self.stats
    }
    pub fn stats_mut(&mut self) -> &mut Stats {
        &mut self.stats
    }
    /// `timestamp_ms` of the first tick; `None` before it.
    pub fn started_at_ms(&self) -> Option<u64> {
        self.started_at_ms
    }
```

5. In `tick`, subito dopo `let deadline = Instant::now() + Duration::from_millis(200);`, aggiungi:

```rust
        self.started_at_ms.get_or_insert(timestamp_ms);
```

6. Nel ramo `if changed { … }`, sostituisci la riga `self.history.set_sensors(&retained);` con:

```rust
            // Series and statistics survive only for sensors whose id, source
            // and unit are unchanged; the second call adds the new sensors.
            self.history.set_sensors(&retained);
            self.stats.set_sensors(&retained);
```

e sostituisci la chiamata finale del ramo, cioè il blocco

```rust
            self.history.set_sensors(
                &self
                    .schema
                    .sensors
                    .iter()
                    .map(|s| s.id.clone())
                    .collect::<Vec<_>>(),
            );
        }
```

con:

```rust
            let ids: Vec<String> = self.schema.sensors.iter().map(|s| s.id.clone()).collect();
            self.history.set_sensors(&ids);
            self.stats.set_sensors(&ids);
        }
```

7. Sostituisci `self.history.push(timestamp_ms, &values);` con:

```rust
        self.history.push(timestamp_ms, &values);
        self.stats.push(&values);
```

Il resto di `tick` resta invariato.

- [ ] **Step 8: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core`
Risultato atteso: tutti OK, 12 test in più rispetto a prima del task (8 in `stats::tests`, 4 in `engine::tests`: `stats_cover_every_tick_since_start`, `stats_follow_the_history_retention_rule`, `stats_reset_through_the_engine`, `started_at_is_the_timestamp_of_the_first_tick`). I test M1/M2 sullo storico (`rediscover_rebuilds_schema_and_keeps_history_by_id`, `source_change_resets_only_the_changed_series`) passano invariati.

- [ ] **Step 9: Lint e commit**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/oma-core/src/stats.rs crates/oma-core/src/lib.rs crates/oma-core/src/engine.rs
git commit -m "feat(core): per-sensor min/max/avg since start and monitoring start time"
```

---

---

### Task 2: `oma-core`: storico decimato e robustezza del campionamento (`History::push` senza panic, `catch_unwind` nel sampler, log dei valori scartati limitato)

**File:**
- Modifica:
  - `crates/oma-core/src/history.rs` (`push` tollerante, `window_decimated`, funzione privata `decimate`)
  - `crates/oma-core/src/sampler.rs` (`catch_unwind` attorno al tick, `spawn_ticker` privato, `log_panic`, `panic_message`)
  - `crates/oma-core/src/sanitize.rs` (`DiscardLog`, `DISCARD_LOG_INTERVAL_MS`; `sanitize` non scrive più log)
  - `crates/oma-core/src/engine.rs` (campo `discards`, log per sensore dei valori scartati)
- Test:
  - `crates/oma-core/src/history.rs`, `crates/oma-core/src/sampler.rs`, `crates/oma-core/src/sanitize.rs`, `crates/oma-core/src/engine.rs` (moduli `tests`)

**Interfacce:**
- Usa:
  - Task 1: `Engine` con i campi `stats` e `started_at_ms`; in `tick` il ramo `if changed` termina con `let ids: Vec<String> = …; self.history.set_sensors(&ids); self.stats.set_sensors(&ids);` e dopo `self.history.push(timestamp_ms, &values);` c'è `self.stats.push(&values);`.
  - M1: `History::window(&self, ids: &[String], since_ms: u64) -> HistoryWindow`, `Sampler::spawn(engine, interval, on_tick)`, `sanitize_sensor(sensor, value)`, il provider `Const` dei test di `sampler.rs` (un sensore, valore sempre `Some(42.0)`).
- Produce:
  ```rust
  // crates/oma-core/src/history.rs
  impl History {
      pub fn window_decimated(&self, ids: &[String], since_ms: u64, max_points: usize) -> HistoryWindow;
  }
  // crates/oma-core/src/sanitize.rs
  pub const DISCARD_LOG_INTERVAL_MS: u64 = 60_000;
  #[derive(Debug, Default)]
  pub struct DiscardLog { /* last_logged_ms: HashMap<String, u64> */ }
  impl DiscardLog {
      pub fn should_log(&mut self, sensor_id: &str, now_ms: u64) -> bool;
      pub fn retain(&mut self, ids: &[String]);
  }
  ```
  `Sampler::spawn` mantiene firma e comportamento; cambia solo il fatto che un panic nel tick non ferma più il thread. `History::push` mantiene la firma. `sanitize(unit, value)` e `sanitize_sensor(sensor, value)` mantengono firme e risultati; non scrivono più righe di log.

**Decimazione (decisione D3).** Nello spike uPlot 20 serie × 3600 punti grezzi occupano 219 MB in totale, oltre il budget di 200 MB; 8 serie × 3600 ne occupano 148. Le finestre di 30 min e 1 h chiedono quindi al core al massimo 900 righe (Task 3 e Task 11), mentre 1 min e 5 min restano grezze. La riduzione è un inviluppo min/max, così picchi e cali brevi degli intervalli interamente validi restano visibili anche dopo la riduzione:
- con `max_points < 2`, oppure con campioni ≤ `max_points`, il risultato è identico a `window()`;
- altrimenti i campioni della finestra (già filtrati da `since_ms`) si dividono in `buckets = max_points / 2` gruppi consecutivi con confini `b * len / buckets` e `(b + 1) * len / buckets`: le dimensioni differiscono al massimo di un campione, senza concentrare il resto nell'ultimo gruppo;
- ogni gruppo produce due righe: (primo timestamp del gruppo, minimo di ogni serie) e (ultimo timestamp del gruppo, massimo di ogni serie);
- una serie con anche un solo valore assente nel gruppo produce `None, None`: il buco viene ampliato conservativamente al gruppo e non nascosto; min/max e picchi si conservano nei gruppi interamente validi;
- righe in uscita = `2 × buckets` ≤ `max_points`. Esempio reale: 3600 campioni con `max_points = 900` danno 450 gruppi da 8 campioni, cioè 900 righe.

Poiché `len > max_points ≥ 2 × buckets`, ogni gruppo ha almeno 2 campioni. I timestamp sono i confini dell'inviluppo, non gli istanti effettivi degli estremi: l'ordine minimo/massimo non descrive l'ordine temporale delle letture originali. L'asse x conserva l'ordine dello storico.

**Robustezza (decisione D12, follow-up M1 "il thread del sampler muore in silenzio").** In M1 `History::push` fa `assert_eq!` sulla lunghezza dei valori e il sampler chiama `Engine::tick` senza protezione. Un panic lì termina il thread `oma-sampler`: la UI resta ferma sugli ultimi valori per sempre, senza nulla nel log (in release non c'è console e il panic hook scrive su stderr). Ora:
- `History::push` non va mai in panic. Se i valori sono meno dei sensori, i mancanti diventano "assenti" (NaN); se sono di più, quelli in eccesso si scartano. La prima discrepanza scrive un `tracing::error!`, le successive no.
- Il sampler esegue ogni tick dentro `std::panic::catch_unwind(AssertUnwindSafe(…))`. Su panic scrive `engine tick panicked` a livello error, con il messaggio del panic, salta `on_tick` per quel giro e continua. Per non riempire il log con un panic a ogni secondo, scrive la riga al 1°, 2°, 4°, 8°… panic consecutivo; un tick riuscito azzera il conteggio.
- Il mutex dell'engine resta avvelenato dopo un panic avvenuto con il lock preso. Il sampler, come già i comandi Tauri, lo riprende con `PoisonError::into_inner`: il campionamento continua.
- `catch_unwind` funziona solo con `panic = "unwind"`, che è il default. Il workspace non imposta `panic = "abort"` in nessun profilo (vale già per `worker.rs`, che isola allo stesso modo i provider): non aggiungerlo.

L'indicatore "dati non aggiornati" della UI è nel Task 10.

**Log dei valori scartati (follow-up M1).** Il filtro di log predefinito in release è `oma_core=debug`. In M1 `sanitize` scrive una riga debug per ogni valore scartato, quindi un sensore bloccato su una lettura implausibile scrive una riga al secondo per tutta la durata dell'app. Ora `sanitize` non scrive più log. L'engine, che conosce l'id del sensore, scrive `discarding implausible value` (con id, unità e valore grezzo) al massimo una volta ogni 60 s per sensore, misurati sul tempo monotono (`monotonic_ms` di `tick`). La riga copre anche i valori non finiti, che prima avevano un messaggio a parte. Al cambio di schema `DiscardLog` dimentica i sensori spariti.

- [ ] **Step 1: Scrivi i test di `History::push` (falliscono)**

In `crates/oma-core/src/history.rs`, nel modulo `tests`, aggiungi subito prima di `len_counts_samples`:

```rust
    #[test]
    fn push_with_too_few_values_stores_the_rest_as_missing() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a", "b"]));
        h.push(1, &[Some(1.0)]);
        let w = h.window(&ids(&["a", "b"]), 0);
        assert_eq!(w.series, vec![vec![Some(1.0)], vec![None]]);
    }

    #[test]
    fn push_with_too_many_values_drops_the_extra_ones() {
        let mut h = History::new(10);
        h.set_sensors(&ids(&["a"]));
        h.push(1, &[Some(1.0), Some(2.0)]);
        h.push(2, &[Some(3.0)]);
        let w = h.window(&ids(&["a"]), 0);
        assert_eq!(w.timestamps_ms, vec![1, 2]);
        assert_eq!(w.series, vec![vec![Some(1.0), Some(3.0)]]);
    }

```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core push_with_too`
Risultato atteso: 2 test FALLISCONO con il panic `assertion left == right failed: values must match the sensor list` (left 1 / right 2 e left 2 / right 1).

- [ ] **Step 3: Rendi `History::push` tollerante**

In `crates/oma-core/src/history.rs`:

1. Sostituisci la struct `History` con:

```rust
#[derive(Debug)]
pub struct History {
    capacity: usize,
    timestamps: VecDeque<u64>,
    /// NaN marks a missing value; it saves memory compared to `Option<f64>`.
    series: Vec<VecDeque<f64>>,
    index: HashMap<String, usize>,
    /// A value count that does not match the sensor list is logged once.
    mismatch_logged: bool,
}
```

2. In `History::new`, sostituisci l'inizializzazione con:

```rust
        Self {
            capacity,
            timestamps: VecDeque::with_capacity(capacity),
            series: Vec::new(),
            index: HashMap::new(),
            mismatch_logged: false,
        }
```

3. Sostituisci l'intero metodo `push` (commento compreso) con:

```rust
    /// Appends one sample per sensor, in `set_sensors` order. A value count
    /// that does not match the sensor list is a bug upstream, but it must not
    /// stop sampling: missing values are stored as missing, extra ones are
    /// dropped, and the mismatch is logged once.
    pub fn push(&mut self, timestamp_ms: u64, values: &[Option<f64>]) {
        if values.len() != self.series.len() && !self.mismatch_logged {
            self.mismatch_logged = true;
            tracing::error!(
                expected = self.series.len(),
                got = values.len(),
                "history values do not match the sensor list; padding or truncating"
            );
        }
        if self.timestamps.len() == self.capacity {
            self.timestamps.pop_front();
            for s in &mut self.series {
                s.pop_front();
            }
        }
        self.timestamps.push_back(timestamp_ms);
        for (i, s) in self.series.iter_mut().enumerate() {
            s.push_back(values.get(i).copied().flatten().unwrap_or(f64::NAN));
        }
    }
```

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core history`
Risultato atteso: tutti i test di `history::tests` OK, compresi i 2 nuovi.

- [ ] **Step 5: Scrivi i test della decimazione (falliscono)**

In `crates/oma-core/src/history.rs`, nel modulo `tests`, aggiungi subito prima di `len_counts_samples`:

```rust
    fn filled(values: &[Option<f64>]) -> History {
        let mut h = History::new(100);
        h.set_sensors(&ids(&["a"]));
        for (i, v) in values.iter().enumerate() {
            h.push((i as u64 + 1) * 1_000, &[*v]);
        }
        h
    }

    #[test]
    fn decimation_is_the_raw_window_when_samples_fit() {
        let h = filled(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
        let a = ids(&["a"]);
        assert_eq!(h.window_decimated(&a, 0, 4), h.window(&a, 0));
        assert_eq!(h.window_decimated(&a, 0, 1), h.window(&a, 0));
        assert_eq!(h.window_decimated(&a, 0, 0), h.window(&a, 0));
    }

    #[test]
    fn decimation_emits_min_then_max_per_bucket() {
        let v = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0, 5.0, 3.0].map(Some);
        let w = filled(&v).window_decimated(&ids(&["a"]), 0, 4);
        // Two buckets of five samples: [3 1 4 1 5] and [9 2 6 5 3].
        assert_eq!(w.timestamps_ms, vec![1_000, 5_000, 6_000, 10_000]);
        assert_eq!(
            w.series,
            vec![vec![Some(1.0), Some(5.0), Some(2.0), Some(9.0)]]
        );
    }

    #[test]
    fn balanced_buckets_distribute_the_remainder() {
        let v: Vec<Option<f64>> = (1..=11).map(|i| Some(i as f64)).collect();
        let w = filled(&v).window_decimated(&ids(&["a"]), 0, 5);
        // 5 / 2 = 2 balanced buckets: five samples, then six.
        assert_eq!(w.timestamps_ms, vec![1_000, 5_000, 6_000, 11_000]);
        assert_eq!(
            w.series,
            vec![vec![Some(1.0), Some(5.0), Some(6.0), Some(11.0)]]
        );
    }

    #[test]
    fn a_bucket_without_values_emits_none_twice() {
        let v = [None, None, None, Some(2.0), Some(4.0), Some(7.0)];
        let w = filled(&v).window_decimated(&ids(&["a", "unknown"]), 0, 4);
        assert_eq!(w.timestamps_ms, vec![1_000, 3_000, 4_000, 6_000]);
        assert_eq!(w.series[0], vec![None, None, Some(2.0), Some(7.0)]);
        assert_eq!(w.series[1], vec![None; 4]);
    }

    #[test]
    fn a_mixed_bucket_preserves_the_gap_conservatively() {
        let v = [Some(1.0), None, Some(3.0), Some(4.0), Some(5.0), Some(6.0)];
        let w = filled(&v).window_decimated(&ids(&["a"]), 0, 4);
        assert_eq!(w.series[0], vec![None, None, Some(4.0), Some(6.0)]);
    }

    #[test]
    fn near_one_hour_has_no_oversized_final_bucket() {
        let mut h = History::new(3_600);
        h.set_sensors(&ids(&["a"]));
        for i in 1..=3_599 {
            h.push(i * 1_000, &[Some(i as f64)]);
        }
        let w = h.window_decimated(&ids(&["a"]), 0, 900);
        assert_eq!(w.timestamps_ms.len(), 900);
        assert_eq!(w.timestamps_ms.first(), Some(&1_000));
        assert_eq!(w.timestamps_ms.last(), Some(&3_599_000));
        for pair in w.timestamps_ms.chunks_exact(2) {
            assert!((6_000..=7_000).contains(&(pair[1] - pair[0])));
        }
    }

    #[test]
    fn decimation_applies_after_the_since_filter() {
        let v: Vec<Option<f64>> = (1..=10).map(|i| Some(i as f64)).collect();
        let w = filled(&v).window_decimated(&ids(&["a"]), 5_000, 2);
        assert_eq!(w.timestamps_ms, vec![5_000, 10_000]);
        assert_eq!(w.series, vec![vec![Some(5.0), Some(10.0)]]);
    }

    #[test]
    fn decimated_output_never_exceeds_max_points() {
        for n in 0..40 {
            let v: Vec<Option<f64>> = (0..n).map(|i| Some(i as f64)).collect();
            let h = filled(&v);
            for max_points in 2..45 {
                let w = h.window_decimated(&ids(&["a"]), 0, max_points);
                assert!(
                    w.timestamps_ms.len() <= max_points,
                    "n={n} max={max_points}"
                );
                assert_eq!(w.series[0].len(), w.timestamps_ms.len());
                assert!(w.timestamps_ms.windows(2).all(|p| p[0] <= p[1]));
            }
        }
    }

```

- [ ] **Step 6: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core history`
Risultato atteso: la compilazione dei test fallisce con `error[E0599]: no method named window_decimated found for struct History`.

- [ ] **Step 7: Implementa `window_decimated`**

In `crates/oma-core/src/history.rs`, dentro `impl History`, subito dopo la fine del metodo `window` (e prima della `}` che chiude l'`impl`), aggiungi:

```rust

    /// Like `window`, but at most `max_points` rows: long windows are reduced
    /// to a min/max envelope so a 1 h chart stays light (decision D3).
    /// Samples are split into `max_points / 2` consecutive balanced buckets
    /// whose sizes differ by at most one sample; each bucket yields two
    /// rows, (first timestamp, per-series minimum) and (last timestamp,
    /// per-series maximum). These are envelope bounds, not actual extremum
    /// times. Any missing value makes that series yield `None` twice for the
    /// bucket; peaks survive in fully valid buckets. With `max_points < 2`, or when
    /// the samples already fit, the result is exactly `window`.
    pub fn window_decimated(
        &self,
        ids: &[String],
        since_ms: u64,
        max_points: usize,
    ) -> HistoryWindow {
        decimate(self.window(ids, since_ms), max_points)
    }
```

Poi, dopo la `}` che chiude `impl History` e prima di `#[cfg(test)]`, aggiungi la funzione privata:

```rust
fn decimate(raw: HistoryWindow, max_points: usize) -> HistoryWindow {
    let len = raw.timestamps_ms.len();
    if max_points < 2 || len <= max_points {
        return raw;
    }
    let buckets = max_points / 2;
    let bounds = |b: usize| (b * len / buckets, (b + 1) * len / buckets);
    let mut timestamps_ms = Vec::with_capacity(buckets * 2);
    for b in 0..buckets {
        let (start, end) = bounds(b);
        timestamps_ms.push(raw.timestamps_ms[start]);
        timestamps_ms.push(raw.timestamps_ms[end - 1]);
    }
    let series = raw
        .series
        .iter()
        .map(|values| {
            let mut out = Vec::with_capacity(buckets * 2);
            for b in 0..buckets {
                let (start, end) = bounds(b);
                if values[start..end].iter().any(Option::is_none) {
                    out.extend([None, None]);
                    continue;
                }
                let mut min: Option<f64> = None;
                let mut max: Option<f64> = None;
                for &v in values[start..end].iter().flatten() {
                    min = Some(min.map_or(v, |m| m.min(v)));
                    max = Some(max.map_or(v, |m| m.max(v)));
                }
                out.push(min);
                out.push(max);
            }
            out
        })
        .collect();
    HistoryWindow {
        timestamps_ms,
        series,
    }
}
```

La decimazione lavora sull'uscita di `window`, così eredita senza duplicarli il filtro `since_ms`, l'arresto quando l'orologio torna indietro e le serie `None` per gli id sconosciuti. Il costo è una finestra grezza temporanea (3600 righe × al massimo 8 serie, circa 460 KB) solo durante la chiamata.

- [ ] **Step 8: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core history`
Risultato atteso: tutti i test di `history::tests` OK (8 nuovi di decimazione, 2 di `push`, i 7 di M1).

- [ ] **Step 9: Scrivi i test del sampler (falliscono)**

In `crates/oma-core/src/sampler.rs`, nel modulo `tests`, aggiungi subito prima di `sampler_ticks_and_stops_promptly`:

```rust
    fn output(seq: u64) -> TickOutput {
        TickOutput {
            snapshot: crate::model::Snapshot {
                revision: 1,
                seq,
                timestamp_ms: 0,
                values: Vec::new(),
            },
            schema: None,
        }
    }

    #[test]
    fn a_panicking_tick_is_skipped_and_sampling_goes_on() {
        let mut calls = 0u64;
        let tick = move |_: u64, _: u64| {
            calls += 1;
            if calls == 2 {
                panic!("tick {calls} failed");
            }
            output(calls)
        };
        let (tx, rx) = mpsc::channel();
        let sampler = Sampler::spawn_ticker(ms(10), tick, move |out| {
            let _ = tx.send(out.snapshot.seq);
        });
        let seqs: Vec<u64> = (0..3)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        sampler.stop();
        // The second tick panicked: nothing was published for it.
        assert_eq!(seqs, vec![1, 3, 4]);
    }

    #[test]
    fn a_poisoned_engine_keeps_sampling() {
        let engine = Arc::new(Mutex::new(Engine::new(vec![Box::new(Const)], 16)));
        let poisoner = engine.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the engine mutex");
        })
        .join();
        assert!(engine.is_poisoned());
        let (tx, rx) = mpsc::channel();
        let sampler = Sampler::spawn(engine, ms(10), move |out| {
            let _ = tx.send(out.snapshot.values.clone());
        });
        let values = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        sampler.stop();
        assert_eq!(values, vec![Some(42.0)]);
    }

    #[test]
    fn repeated_panics_are_logged_with_exponential_spacing() {
        let logged: Vec<u64> = (1..=20).filter(|&n| log_panic(n)).collect();
        assert_eq!(logged, vec![1, 2, 4, 8, 16]);
    }

    #[test]
    fn panic_messages_are_extracted_from_both_payload_kinds() {
        let literal = catch_unwind(|| panic!("literal")).unwrap_err();
        assert_eq!(panic_message(literal.as_ref()), "literal");
        let formatted = catch_unwind(|| panic!("tick {}", 7)).unwrap_err();
        assert_eq!(panic_message(formatted.as_ref()), "tick 7");
        let other = catch_unwind(|| std::panic::panic_any(5u8)).unwrap_err();
        assert_eq!(panic_message(other.as_ref()), "non-string panic payload");
    }

```

Nessun provider reale può far andare in panic `Engine::tick`: i panic dei provider sono già isolati nel worker e, dopo lo Step 3, `History::push` non ne produce più. Per questo il ciclo del sampler diventa una funzione privata `spawn_ticker` con il tick iniettabile, e il primo test inietta un tick che va in panic alla seconda chiamata. Il secondo test copre il mutex avvelenato con l'engine vero. Passa già oggi, perché il lock usa `PoisonError::into_inner`, e protegge quel comportamento da regressioni. I panic dei test stampano su stderr la riga `thread '…' panicked at …`: è atteso.

- [ ] **Step 10: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core sampler`
Risultato atteso: la compilazione dei test fallisce con `no function or associated item named spawn_ticker found for struct Sampler`, `cannot find function log_panic in this scope`, `cannot find function panic_message in this scope` e `cannot find function catch_unwind in this scope`.

- [ ] **Step 11: Proteggi il ciclo del sampler**

In `crates/oma-core/src/sampler.rs`:

1. Negli `use`, prima di `use std::sync::atomic::{AtomicBool, Ordering};`, aggiungi:

```rust
use std::panic::{catch_unwind, AssertUnwindSafe};
```

2. Subito prima di `pub struct Sampler {` aggiungi:

```rust
/// A tick that panics is logged on the 1st, 2nd, 4th, 8th... panic in a row:
/// a tick that panics every second must not flood the log.
fn log_panic(consecutive: u64) -> bool {
    consecutive.is_power_of_two()
}

/// Text of a panic payload (`panic!` with a literal or a formatted message).
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

```

3. Sostituisci l'intero metodo `Sampler::spawn` con i due metodi seguenti. Il ciclo di attesa fino alla scadenza, `stop` e `Drop` restano invariati:

```rust
    /// Starts ticking `engine` every `interval`, calling `on_tick` after each
    /// tick with the engine lock already released.
    pub fn spawn<F>(engine: Arc<Mutex<Engine>>, interval: Duration, on_tick: F) -> Self
    where
        F: FnMut(&TickOutput) + Send + 'static,
    {
        let tick = move |timestamp_ms, monotonic_ms| {
            engine
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .tick(timestamp_ms, monotonic_ms)
        };
        Self::spawn_ticker(interval, tick, on_tick)
    }

    /// The loop behind `spawn`, with the tick injectable for tests. A panic in
    /// `tick` must not end sampling (spec §8): it is caught, logged, that
    /// iteration publishes nothing and the loop goes on. A panic while the
    /// engine lock is held poisons the mutex; `spawn` keeps locking it through
    /// the poison, like the Tauri commands do.
    fn spawn_ticker<T, F>(interval: Duration, mut tick: T, mut on_tick: F) -> Self
    where
        T: FnMut(u64, u64) -> TickOutput + Send + 'static,
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
                let mut panics = 0u64;
                while !stop_flag.load(Ordering::Acquire) {
                    let monotonic_ms = epoch.elapsed().as_millis() as u64;
                    match catch_unwind(AssertUnwindSafe(|| tick(unix_ms(), monotonic_ms))) {
                        Ok(output) => {
                            panics = 0;
                            on_tick(&output);
                        }
                        Err(payload) => {
                            panics += 1;
                            if log_panic(panics) {
                                tracing::error!(
                                    panic = panic_message(payload.as_ref()),
                                    consecutive = panics,
                                    "engine tick panicked"
                                );
                            }
                        }
                    }
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
        Self {
            stop,
            thread: Some(thread),
        }
    }
```

`on_tick` resta fuori da `catch_unwind`: gira senza il lock dell'engine ed è codice della shell (emissione degli eventi Tauri). Proteggerlo nasconderebbe errori della shell senza salvare lo stato dell'engine.

- [ ] **Step 12: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core sampler`
Risultato atteso: 9 test OK in `sampler::tests` (i 5 di M1 più i 4 nuovi). Su stderr compaiono le righe dei panic volontari dei test.

- [ ] **Step 13: Scrivi i test del limite sui log (falliscono)**

In `crates/oma-core/src/sanitize.rs`, nel modulo `tests`, aggiungi subito prima di `power_ratio_can_exceed_100_but_utilization_cannot`:

```rust
    #[test]
    fn discard_log_allows_one_line_per_sensor_per_minute() {
        let mut log = DiscardLog::default();
        assert!(log.should_log("a", 1_000));
        assert!(!log.should_log("a", 2_000));
        assert!(!log.should_log("a", 60_999));
        // Another sensor has its own budget.
        assert!(log.should_log("b", 2_000));
        assert!(log.should_log("a", 61_000));
        assert!(!log.should_log("a", 120_999));
        assert!(log.should_log("a", 121_000));
    }

    #[test]
    fn discard_log_forgets_removed_sensors() {
        let mut log = DiscardLog::default();
        assert!(log.should_log("a", 0));
        assert!(log.should_log("b", 0));
        log.retain(&["b".to_owned()]);
        assert!(log.should_log("a", 1_000));
        assert!(!log.should_log("b", 1_000));
    }

```

In `crates/oma-core/src/engine.rs`, nel modulo `tests`, aggiungi subito prima di `failing_provider_backs_off_and_recovers`:

```rust
    #[test]
    fn discarded_values_are_logged_at_most_once_a_minute_per_sensor() {
        let (p, script) = fake("a", inventory("dev/a", &["x"]));
        script
            .lock()
            .unwrap()
            .polls
            .extend([Ok(vec![Some(150.0)]), Ok(vec![Some(150.0)])]);
        let mut e = Engine::new(vec![p], 10);
        e.tick(0, 0);
        // The engine used this sensor's budget at 0 ms (monotonic time)...
        assert!(!e.discards.should_log("dev/a/load/x", 1_000));
        e.tick(1_000, 1_000);
        // ...and a new discard within the minute did not renew it.
        assert!(e.discards.should_log("dev/a/load/x", 60_000));
    }

```

Il test dell'engine legge il campo privato `discards`, accessibile perché il modulo `tests` è figlio di `engine`. Verifica il collegamento: il tick a 0 ms ha consumato il budget del sensore `x` (valore 150 % scartato) e il secondo scarto, dentro il minuto, non l'ha rinnovato.

- [ ] **Step 14: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core discard`
Risultato atteso: la compilazione dei test fallisce con `failed to resolve: use of undeclared type DiscardLog` in `sanitize.rs` e `no field discards on type Engine` in `engine.rs`.

- [ ] **Step 15: Implementa `DiscardLog` e spostane l'uso nell'engine**

In `crates/oma-core/src/sanitize.rs`, sostituisci l'inizio del file, dal commento `//!` fino al controllo `is_finite` compreso, cioè il blocco:

```rust
//! Drops values outside the physically plausible range (spec §8).

use crate::model::Unit;

pub fn sanitize(unit: Unit, value: Option<f64>) -> Option<f64> {
    let v = value?;
    if !v.is_finite() {
        tracing::debug!(?unit, v, "discarding non-finite value");
        return None;
    }
```

con:

```rust
//! Drops values outside the physically plausible range (spec §8).

use std::collections::{HashMap, HashSet};

use crate::model::Unit;

/// Minimum time between two "discarding implausible value" lines for the
/// same sensor.
pub const DISCARD_LOG_INTERVAL_MS: u64 = 60_000;

/// Rate limit for the debug line written when a value is discarded. The
/// release log filter keeps `oma_core=debug`, so without it a sensor stuck on
/// an implausible reading would write one line per tick for as long as the
/// app runs.
#[derive(Debug, Default)]
pub struct DiscardLog {
    last_logged_ms: HashMap<String, u64>,
}

impl DiscardLog {
    /// True when a discard of `sensor_id` may be logged at `now_ms`
    /// (monotonic): the first time, then at most once per
    /// `DISCARD_LOG_INTERVAL_MS`.
    pub fn should_log(&mut self, sensor_id: &str, now_ms: u64) -> bool {
        match self.last_logged_ms.get_mut(sensor_id) {
            Some(last) if now_ms.saturating_sub(*last) < DISCARD_LOG_INTERVAL_MS => false,
            Some(last) => {
                *last = now_ms;
                true
            }
            None => {
                self.last_logged_ms.insert(sensor_id.to_owned(), now_ms);
                true
            }
        }
    }

    /// Forgets sensors that are no longer in the schema.
    pub fn retain(&mut self, ids: &[String]) {
        let keep: HashSet<&str> = ids.iter().map(String::as_str).collect();
        self.last_logged_ms
            .retain(|id, _| keep.contains(id.as_str()));
    }
}

/// Plausible value or `None`. It does not log: the engine logs discards per
/// sensor through `DiscardLog`.
pub fn sanitize(unit: Unit, value: Option<f64>) -> Option<f64> {
    let v = value?;
    if !v.is_finite() {
        return None;
    }
```

Poi, sempre in `sanitize`, sostituisci la coda della funzione:

```rust
    if plausible {
        Some(v)
    } else {
        tracing::debug!(?unit, v, "discarding implausible value");
        None
    }
}
```

con:

```rust
    plausible.then_some(v)
}
```

Il blocco `let plausible = match unit { … };` tra le due parti resta invariato: il Task 6 vi aggiungerà i rami di `PcieGeneration` e `Lanes`.

In `crates/oma-core/src/engine.rs`:

1. Sostituisci `use crate::sanitize::sanitize_sensor;` con:

```rust
use crate::sanitize::{sanitize_sensor, DiscardLog};
```

2. Nella struct `Engine`, tra `started_at_ms` e `seq`, aggiungi:

```rust
    /// Rate limit of the "discarding implausible value" debug line.
    discards: DiscardLog,
```

3. In `Engine::new`, tra `started_at_ms: None,` e `seq: 0,`, aggiungi:

```rust
            discards: DiscardLog::default(),
```

4. In `tick`, sostituisci la fine del ramo `if changed` e il calcolo di `values`, cioè il blocco:

```rust
            self.history.set_sensors(&ids);
            self.stats.set_sensors(&ids);
        }
        let values: Vec<_> = self
            .slots
            .iter()
            .flat_map(|s| s.last.iter().copied())
            .zip(&self.schema.sensors)
            .map(|(value, sensor)| sanitize_sensor(sensor, value))
            .collect();
```

con:

```rust
            self.history.set_sensors(&ids);
            self.stats.set_sensors(&ids);
            self.discards.retain(&ids);
        }
        let discards = &mut self.discards;
        let values: Vec<_> = self
            .slots
            .iter()
            .flat_map(|s| s.last.iter().copied())
            .zip(&self.schema.sensors)
            .map(|(value, sensor)| {
                let clean = sanitize_sensor(sensor, value);
                if let (Some(raw), None) = (value, clean) {
                    if discards.should_log(&sensor.id, monotonic_ms) {
                        tracing::debug!(
                            sensor = %sensor.id,
                            unit = ?sensor.unit,
                            value = raw,
                            "discarding implausible value"
                        );
                    }
                }
                clean
            })
            .collect();
```

La riga successiva (`self.history.push(timestamp_ms, &values);`) resta invariata. Il prestito separato `let discards = &mut self.discards;` serve perché la closure legge anche `self.slots` e `self.schema`.

- [ ] **Step 16: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core`
Risultato atteso: tutti OK, 17 test in più rispetto alla fine del Task 1 (10 in `history`, 4 in `sampler`, 2 in `sanitize`, 1 in `engine`). I test M1 di `sanitize` (`rejects_non_finite`, `temperature_must_be_physically_plausible`, …) passano invariati: i risultati di `sanitize` non cambiano.

- [ ] **Step 17: Lint e commit**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/oma-core/src/history.rs crates/oma-core/src/sampler.rs crates/oma-core/src/sanitize.rs crates/oma-core/src/engine.rs
git commit -m "feat(core): decimated history windows and a sampler that survives tick panics"
```

---

---

### Task 3: `oma-app`: comandi `get_history` con `maxPoints`, `get_stats`, `reset_stats`, `get_session`

**File:**
- Modifica:
  - `app/src-tauri/src/commands.rs` (parametro `max_points` di `get_history`, tre comandi nuovi, helper puri testabili)
  - `app/src-tauri/src/main.rs` (`AppState.interval_ms`, `generate_handler!`)
  - `app/src-tauri/build.rs` (i tre comandi nel manifest)
  - `app/src-tauri/capabilities/default.json` (i tre permessi; l'elenco diventa uno per riga)
- Crea (generati da `tauri-build` alla prima compilazione, da versionare):
  - `app/src-tauri/permissions/autogenerated/get_stats.toml`
  - `app/src-tauri/permissions/autogenerated/reset_stats.toml`
  - `app/src-tauri/permissions/autogenerated/get_session.toml`
- Test: `app/src-tauri/src/commands.rs` (modulo `tests`)

**Interfacce:**
- Usa:
  - Task 1: `oma_core::stats::SensorStats` (serializzato `{ min, max, avg, count }`), `Engine::stats(&self) -> &Stats`, `Engine::stats_mut(&mut self) -> &mut Stats`, `Stats::get(&self, &[String]) -> Vec<Option<SensorStats>>`, `Stats::reset(&mut self, &[String])`, `Engine::started_at_ms(&self) -> Option<u64>`.
  - Task 2: `History::window_decimated(&self, ids: &[String], since_ms: u64, max_points: usize) -> HistoryWindow`.
  - M1: `History::window`, `history_since(now_ms, seconds)`, `HistorySeed`, `AppState { engine }`, `SAMPLE_INTERVAL` (1 s) in `main.rs`.
- Produce, per la UI (Task 9 `tauri.ts`, Task 10 `HealthBanner`/indicatore di dati fermi, Task 11 grafico, Task 12 tabella):

  | comando (JS) | argomenti JS | risposta JSON |
  |---|---|---|
  | `get_history` | `{ ids: string[], seconds: number, maxPoints?: number }` | `{ revision, seq, timestampsMs, series }` (invariata) |
  | `get_stats` | `{ ids: string[] }` | `{ revision: number, stats: ({ min, max, avg, count } \| null)[] }` |
  | `reset_stats` | `{ ids: string[] }` | `null` |
  | `get_session` | nessuno | `{ startedAtMs: number \| null, intervalMs: number }` |

  Firme Rust:
  ```rust
  pub fn get_history(state: State<'_, AppState>, ids: Vec<String>, seconds: u64, max_points: Option<u32>) -> HistorySeed;
  pub fn get_stats(state: State<'_, AppState>, ids: Vec<String>) -> StatsReply;
  pub fn reset_stats(state: State<'_, AppState>, ids: Vec<String>);
  pub fn get_session(state: State<'_, AppState>) -> Session;
  #[serde(rename_all = "camelCase")] pub struct StatsReply { revision: u64, stats: Vec<Option<SensorStats>> }
  #[serde(rename_all = "camelCase")] pub struct Session { pub started_at_ms: Option<u64>, pub interval_ms: u64 }
  pub struct AppState { pub engine: Arc<Mutex<Engine>>, pub interval_ms: u64 }
  ```

**Comportamento:**
- Tauri converte in camelCase i nomi degli argomenti: il parametro Rust `max_points` in JS è `maxPoints`. Un argomento `Option<T>` assente nel payload vale `None` (in Tauri 2.11 il deserializzatore dei comandi chiama `visit_none` se la chiave manca), quindi le chiamate M1/M2 `invoke('get_history', { ids, seconds })` restano valide e ricevono la finestra grezza di sempre.
- `maxPoints` presente → `window_decimated(…, maxPoints.clamp(2, 3600))`. Il minimo 2 è la coppia min/max di un solo gruppo. Oltre 3600 righe non c'è niente da ridurre: 3600 è la capienza dello storico a 1 s. La UI chiede `maxPoints: 900` per le finestre di 30 min e 1 h, e nessun `maxPoints` per 1 min e 5 min (decisione D3, Task 11).
- `get_stats` restituisce le statistiche nello stesso ordine degli id richiesti, `null` per un id sconosciuto o senza campioni validi, insieme alla `revision` dello schema: la UI scarta una risposta arrivata dopo un cambio di schema. La UI interroga ogni secondo solo i sensori della pagina aperta, e solo mentre la finestra è visibile (Task 12).
- `reset_stats` azzera min/max/media degli id indicati (pulsante "azzera" della pagina); gli id sconosciuti si ignorano.
- `get_session` espone il `timestamp_ms` del primo tick dell'engine e l'intervallo di campionamento. Il `HealthBanner` conta la durata del monitoraggio da `startedAtMs` anche dopo una riapertura dalla tray (follow-up M1, decisione D2). L'indicatore di dati fermi usa `intervalMs` (soglia `max(5000, 5 × intervalMs)`, decisione D12, Task 10).
- Come i comandi M1, tutti girano fuori dal thread principale (`#[tauri::command(async)]`): il sampler tiene il lock dell'engine fino a circa 200 ms per tick.
- La logica sta in tre funzioni pure (`history_window`, `stats_reply`, `session`), testate con un `Engine` vero e un provider finto. I comandi si limitano a prendere il lock e chiamarle.
- Ogni comando nuovo va registrato in tre posti: `generate_handler!` in `main.rs`, il manifest in `build.rs` e il permesso `allow-…` in `capabilities/default.json`. Un comando mancante nel manifest o nella capability compila, ma a runtime la chiamata viene rifiutata. Il Task 8 aggiungerà `get_gpu_processes` subito dopo `get_session` negli stessi tre elenchi, che per questo sono scritti un elemento per riga.

- [ ] **Step 1: Scrivi i test (falliscono)**

In `app/src-tauri/src/commands.rs`, sostituisci l'apertura del modulo `tests`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
```

con la stessa apertura seguita da helper e test nuovi (i test M1/M2 esistenti restano sotto, invariati):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use oma_core::model::{Device, DeviceKind, Label, Sensor, SensorKind, Source, Unit};
    use oma_core::provider::{Inventory, Provider, ProviderError};

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// One CPU load sensor whose readings come from a fixed list.
    struct Scripted(std::collections::VecDeque<f64>);

    impl Provider for Scripted {
        fn name(&self) -> &'static str {
            "scripted"
        }

        fn discover(&mut self) -> Result<Inventory, ProviderError> {
            Ok(Inventory {
                devices: vec![Device {
                    id: "cpu/0".into(),
                    kind: DeviceKind::Cpu,
                    name: "cpu".into(),
                    vendor: None,
                    properties: Default::default(),
                }],
                sensors: vec![Sensor::new(
                    "cpu/0",
                    SensorKind::Load,
                    "total",
                    Unit::Percent,
                    Label::new("cpu.load.total"),
                    Source::Mock,
                )],
            })
        }

        fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
            Ok(vec![self.0.pop_front()])
        }
    }

    fn engine_after(values: &[f64]) -> Engine {
        let provider = Scripted(values.iter().copied().collect());
        let mut engine = Engine::new(vec![Box::new(provider)], 3_600);
        for (i, _) in values.iter().enumerate() {
            let t = 1_000 * (i as u64 + 1);
            engine.tick(t, t);
        }
        engine
    }

    const LOAD: &str = "cpu/0/load/total";

    #[test]
    fn history_without_max_points_is_the_raw_window() {
        let engine = engine_after(&[10.0, 20.0, 30.0, 40.0, 50.0]);
        let w = history_window(engine.history(), &ids(&[LOAD]), 0, None);
        assert_eq!(w.timestamps_ms.len(), 5);
    }

    #[test]
    fn history_with_max_points_is_decimated_and_clamped() {
        let engine = engine_after(&[10.0, 20.0, 30.0, 40.0, 50.0, 60.0]);
        let history = engine.history();
        let load = ids(&[LOAD]);
        let w = history_window(history, &load, 0, Some(4));
        assert_eq!(w.timestamps_ms, vec![1_000, 3_000, 4_000, 6_000]);
        assert_eq!(
            w.series[0],
            vec![Some(10.0), Some(30.0), Some(40.0), Some(60.0)]
        );
        // 0 and 1 are raised to 2 points: one bucket, minimum then maximum.
        for tiny in [0, 1] {
            let w = history_window(history, &load, 0, Some(tiny));
            assert_eq!(w.series[0], vec![Some(10.0), Some(60.0)]);
        }
        // Huge values are capped at 3600, which still fits all six samples.
        let w = history_window(history, &load, 0, Some(u32::MAX));
        assert_eq!(w, history.window(&load, 0));
    }

    #[test]
    fn stats_reply_reports_min_max_avg_in_request_order() {
        let engine = engine_after(&[10.0, 30.0, 20.0]);
        let reply = stats_reply(&engine, &ids(&["unknown", LOAD]));
        assert_eq!(
            serde_json::to_value(&reply).expect("serialize"),
            serde_json::json!({
                "revision": 1,
                "stats": [null, { "min": 10.0, "max": 30.0, "avg": 20.0, "count": 3 }]
            })
        );
    }

    #[test]
    fn reset_restarts_the_statistics() {
        let mut engine = engine_after(&[10.0, 30.0]);
        engine.stats_mut().reset(&ids(&[LOAD]));
        assert_eq!(stats_reply(&engine, &ids(&[LOAD])).stats, vec![None]);
    }

    #[test]
    fn session_reports_the_first_tick_and_the_interval() {
        let engine = Engine::new(Vec::new(), 10);
        assert_eq!(
            serde_json::to_value(session(&engine, 1_000)).expect("serialize"),
            serde_json::json!({ "startedAtMs": null, "intervalMs": 1000 })
        );
        let engine = engine_after(&[1.0, 2.0]);
        assert_eq!(
            serde_json::to_value(session(&engine, 1_000)).expect("serialize"),
            serde_json::json!({ "startedAtMs": 1000, "intervalMs": 1000 })
        );
    }
```

`Scripted` restituisce i valori della lista uno per tick (poi `None`). Dal primo tick l'engine fa discovery e poll nello stesso giro, quindi dopo N tick lo storico ha N campioni con timestamp 1000, 2000, … Con 6 campioni e `maxPoints = 4` si hanno 2 gruppi da 3: `[10 20 30]` → (1000, 10), (3000, 30); `[40 50 60]` → (4000, 40), (6000, 60). I tick girano davvero sui thread worker, ma il poll risponde subito: non c'è attesa sulla scadenza di 200 ms.

- [ ] **Step 2: Esegui i test e verifica che falliscano**

La compilazione di `oma-app` incorpora `app/dist` (`generate_context!`): se la cartella manca, creala prima.

```bash
cd app && pnpm build && cd ..
cargo test -p oma-app
```

Risultato atteso: la compilazione dei test fallisce con `cannot find function history_window in this scope`, `cannot find function stats_reply in this scope` e `cannot find function session in this scope`.

- [ ] **Step 3: Implementa helper e comandi**

In `app/src-tauri/src/commands.rs`:

1. Sostituisci gli `use` di `oma_core`:

```rust
use oma_core::history::HistoryWindow;
use oma_core::model::Schema;
use oma_core::sampler::unix_ms;
```

con:

```rust
use oma_core::engine::Engine;
use oma_core::history::{History, HistoryWindow};
use oma_core::model::Schema;
use oma_core::sampler::unix_ms;
use oma_core::stats::SensorStats;
```

2. Subito dopo la funzione `history_since` aggiungi:

```rust

/// Bounds of `maxPoints`: the envelope needs two rows per bucket, and more
/// rows than the 1 h buffer holds at 1 s would be the raw window anyway.
const MIN_POINTS: u32 = 2;
const MAX_POINTS: u32 = 3_600;

/// Raw window without `max_points`, min/max envelope with it (decision D3).
pub(crate) fn history_window(
    history: &History,
    ids: &[String],
    since_ms: u64,
    max_points: Option<u32>,
) -> HistoryWindow {
    match max_points {
        None => history.window(ids, since_ms),
        Some(n) => {
            history.window_decimated(ids, since_ms, n.clamp(MIN_POINTS, MAX_POINTS) as usize)
        }
    }
}

/// Min/max/average since start (or the last reset) for the requested ids, in
/// the same order; `null` for unknown ids and sensors without valid samples.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsReply {
    revision: u64,
    stats: Vec<Option<SensorStats>>,
}

pub(crate) fn stats_reply(engine: &Engine, ids: &[String]) -> StatsReply {
    StatsReply {
        revision: engine.schema().revision,
        stats: engine.stats().get(ids),
    }
}

/// Monitoring session facts the UI cannot know: the WebView is recreated on
/// every window open, the sampler has been running since the app started.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// Unix time of the first sample; `null` before the first tick.
    pub started_at_ms: Option<u64>,
    pub interval_ms: u64,
}

pub(crate) fn session(engine: &Engine, interval_ms: u64) -> Session {
    Session {
        started_at_ms: engine.started_at_ms(),
        interval_ms,
    }
}
```

3. Sostituisci l'intero comando `get_history`:

```rust
#[tauri::command(async)]
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

con `get_history` esteso e i tre comandi nuovi:

```rust
/// `maxPoints` is optional in JS: callers that omit it get the raw window.
#[tauri::command(async)]
pub fn get_history(
    state: State<'_, AppState>,
    ids: Vec<String>,
    seconds: u64,
    max_points: Option<u32>,
) -> HistorySeed {
    let since = history_since(unix_ms(), seconds);
    let engine = state.engine.lock().unwrap_or_else(PoisonError::into_inner);
    HistorySeed {
        revision: engine.schema().revision,
        seq: engine.sequence(),
        history: history_window(engine.history(), &ids, since, max_points),
    }
}

#[tauri::command(async)]
pub fn get_stats(state: State<'_, AppState>, ids: Vec<String>) -> StatsReply {
    let engine = state.engine.lock().unwrap_or_else(PoisonError::into_inner);
    stats_reply(&engine, &ids)
}

/// Restarts min/max/average of the given sensors (the page's reset button).
#[tauri::command(async)]
pub fn reset_stats(state: State<'_, AppState>, ids: Vec<String>) {
    state
        .engine
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .stats_mut()
        .reset(&ids);
}

#[tauri::command(async)]
pub fn get_session(state: State<'_, AppState>) -> Session {
    let engine = state.engine.lock().unwrap_or_else(PoisonError::into_inner);
    session(&engine, state.interval_ms)
}
```

In `app/src-tauri/src/main.rs`:

1. Sostituisci `AppState` con:

```rust
pub struct AppState {
    pub engine: Arc<Mutex<Engine>>,
    /// Sampling interval in milliseconds, reported by `get_session`.
    pub interval_ms: u64,
}
```

2. Nel builder sostituisci `.manage(AppState { … })` e `.invoke_handler(…)`:

```rust
        .manage(AppState {
            engine: engine.clone(),
        })
        .manage(StartupState::new(switch, status))
        .invoke_handler(tauri::generate_handler![
            commands::get_schema,
            commands::get_history,
            commands::get_startup_status,
            commands::enable_vendor_libraries
        ])
```

con:

```rust
        .manage(AppState {
            engine: engine.clone(),
            interval_ms: SAMPLE_INTERVAL.as_millis() as u64,
        })
        .manage(StartupState::new(switch, status))
        .invoke_handler(tauri::generate_handler![
            commands::get_schema,
            commands::get_history,
            commands::get_stats,
            commands::reset_stats,
            commands::get_session,
            commands::get_startup_status,
            commands::enable_vendor_libraries,
        ])
```

La virgola finale dopo l'ultimo comando è accettata da `generate_handler!` e rende l'aggiunta del Task 8 una sola riga.

`app/src-tauri/build.rs` (file completo):

```rust
fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_schema",
            "get_history",
            "get_stats",
            "reset_stats",
            "get_session",
            "get_startup_status",
            "enable_vendor_libraries",
        ]),
    ))
    .expect("Tauri build")
}
```

`app/src-tauri/capabilities/default.json` (file completo):

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Main window: sensor reads, statistics and session, event subscriptions and leaving GPU safe mode.",
  "windows": ["main"],
  "permissions": [
    "core:event:allow-listen",
    "core:event:allow-unlisten",
    "allow-get-schema",
    "allow-get-history",
    "allow-get-stats",
    "allow-reset-stats",
    "allow-get-session",
    "allow-get-startup-status",
    "allow-enable-vendor-libraries"
  ]
}
```

- [ ] **Step 4: Esegui i test e verifica che passino**

```bash
cargo test -p oma-app
```

Risultato atteso: 16 test OK, cioè gli 11 di M1/M2 più i 5 nuovi (`history_without_max_points_is_the_raw_window`, `history_with_max_points_is_decimated_and_clamped`, `stats_reply_reports_min_max_avg_in_request_order`, `reset_restarts_the_statistics`, `session_reports_the_first_tick_and_the_interval`).

La compilazione ha generato i tre permessi. Controlla che esistano con questo contenuto (generati, non vanno modificati a mano):

`app/src-tauri/permissions/autogenerated/get_stats.toml`:

```toml
# Automatically generated - DO NOT EDIT!

[[permission]]
identifier = "allow-get-stats"
description = "Enables the get_stats command without any pre-configured scope."
commands.allow = ["get_stats"]

[[permission]]
identifier = "deny-get-stats"
description = "Denies the get_stats command without any pre-configured scope."
commands.deny = ["get_stats"]
```

`app/src-tauri/permissions/autogenerated/reset_stats.toml`:

```toml
# Automatically generated - DO NOT EDIT!

[[permission]]
identifier = "allow-reset-stats"
description = "Enables the reset_stats command without any pre-configured scope."
commands.allow = ["reset_stats"]

[[permission]]
identifier = "deny-reset-stats"
description = "Denies the reset_stats command without any pre-configured scope."
commands.deny = ["reset_stats"]
```

`app/src-tauri/permissions/autogenerated/get_session.toml`:

```toml
# Automatically generated - DO NOT EDIT!

[[permission]]
identifier = "allow-get-session"
description = "Enables the get_session command without any pre-configured scope."
commands.allow = ["get_session"]

[[permission]]
identifier = "deny-get-session"
description = "Denies the get_session command without any pre-configured scope."
commands.deny = ["get_session"]
```

`get_history.toml` non cambia: il permesso riguarda il comando, non i suoi argomenti.

- [ ] **Step 5: Lint**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Risultato atteso: nessun avviso di clippy, tutti i test OK.

- [ ] **Step 6: Verifica dal vivo dei comandi nella WebView (CDP)**

La UI del Task 9 non chiama ancora i comandi nuovi. Questa verifica li invoca direttamente nella WebView tramite il Chrome DevTools Protocol e controlla la cosa che i test Rust non vedono: manifest e capability. Un permesso mancante fa fallire l'`invoke` solo a runtime.

Sulla macchina di sviluppo l'utente può avere già in esecuzione `target\release\oma-app.exe`. La verifica non lo tocca:
- compila in una cartella target separata, perché l'eseguibile in uso è bloccato da Windows;
- usa un identificatore diverso, altrimenti il plugin di istanza singola passerebbe il controllo all'istanza già aperta. L'identificatore separa anche il profilo della WebView: WebView2 lo crea in `%LOCALAPPDATA%\io.github.openmonitoradvanced.verify\EBWebView`, perché Tauri ricava la cartella dalla Known Folder di Windows e non dalla variabile `LOCALAPPDATA`;
- reindirizza `LOCALAPPDATA`, così log e crash marker dell'app (`OpenMonitorAdvanced\logs`, `crash.txt`), che l'app ricava dalla variabile, restano fuori dalle cartelle reali.

Non usare input sintetico di mouse o tastiera.

1. Crea `$env:TEMP\oma-m3-verify\cdp-invoke.mjs` (script temporaneo, fuori dal repository):

```js
// Calls Tauri commands inside the running WebView through the Chrome DevTools
// Protocol. Usage: node cdp-invoke.mjs <port>
const port = process.argv[2] ?? '9333';
const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === 'page');
if (!page) throw new Error('no page target');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = reject;
});
let nextId = 1;
const pending = new Map();
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
};
function evaluate(expression) {
  const id = nextId++;
  ws.send(
    JSON.stringify({
      id,
      method: 'Runtime.evaluate',
      params: { expression, awaitPromise: true, returnByValue: true },
    }),
  );
  return new Promise((resolve) => pending.set(id, resolve));
}
const calls = [
  `invoke('get_session')`,
  `invoke('get_schema').then((s) => s.sensors.slice(0, 2).map((x) => x.id))`,
  `invoke('get_schema').then((s) => invoke('get_stats', { ids: [s.sensors[0].id, 'nope'] }))`,
  `invoke('get_schema').then((s) => invoke('get_history', { ids: [s.sensors[0].id], seconds: 3600 })).then((h) => h.timestampsMs.length)`,
  `invoke('get_schema').then((s) => invoke('get_history', { ids: [s.sensors[0].id], seconds: 3600, maxPoints: 4 })).then((h) => ({ rows: h.timestampsMs.length, series: h.series }))`,
  `invoke('get_schema').then((s) => invoke('reset_stats', { ids: [s.sensors[0].id] }).then(() => invoke('get_stats', { ids: [s.sensors[0].id] })))`,
];
for (const call of calls) {
  const expression = `(() => { const invoke = window.__TAURI_INTERNALS__.invoke; return ${call}; })()`;
  const msg = await evaluate(expression);
  const result = msg.result?.exceptionDetails
    ? `ERROR ${JSON.stringify(msg.result.exceptionDetails.exception?.value ?? msg.result.exceptionDetails.text)}`
    : JSON.stringify(msg.result?.result?.value);
  console.log(`${call}\n  -> ${result}`);
}
ws.close();
```

2. Dalla radice del repository, in **un'unica** sessione PowerShell (le variabili d'ambiente impostate qui valgono solo per questa sessione), compila, avvia, interroga e chiudi:

```powershell
$verify = "$env:TEMP\oma-m3-verify"
New-Item -ItemType Directory -Force "$verify\localappdata" | Out-Null
Set-Content "$verify\tauri.verify.json" '{"identifier":"io.github.openmonitoradvanced.verify"}'
$env:CARGO_TARGET_DIR = "$verify\target"
Push-Location app
pnpm tauri build --no-bundle --config "$verify\tauri.verify.json"
Pop-Location
Remove-Item Env:CARGO_TARGET_DIR
$realLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = "$verify\localappdata"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9333"
$app = Start-Process "$verify\target\release\oma-app.exe" -PassThru
Start-Sleep -Seconds 10
node "$verify\cdp-invoke.mjs" 9333
Stop-Process -Id $app.Id
$env:LOCALAPPDATA = $realLocalAppData
Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
```

La prima compilazione nella cartella separata dura circa un minuto e mezzo. Si apre la finestra dell'app (UI M2): è attesa e viene chiusa da `Stop-Process`.

Risultato atteso (i numeri variano; qui una prova sulla macchina di sviluppo dopo 10 s):

```text
invoke('get_session')
  -> {"startedAtMs":1790346011806,"intervalMs":1000}
invoke('get_schema').then((s) => s.sensors.slice(0, 2).map((x) => x.id))
  -> ["cpu/0/load/total","cpu/0/load/thread-0-0"]
invoke('get_schema').then((s) => invoke('get_stats', { ids: [s.sensors[0].id, 'nope'] }))
  -> {"revision":2,"stats":[{"min":16.25,"max":33.45,"avg":25.26,"count":8},null]}
invoke('get_schema').then((s) => invoke('get_history', { ids: [s.sensors[0].id], seconds: 3600 })).then((h) => h.timestampsMs.length)
  -> 10
invoke('get_schema').then((s) => invoke('get_history', { ids: [s.sensors[0].id], seconds: 3600, maxPoints: 4 })).then((h) => ({ rows: h.timestampsMs.length, series: h.series }))
  -> {"rows":4,"series":[[16.25,26.84,23.05,33.45]]}
invoke('get_schema').then((s) => invoke('reset_stats', { ids: [s.sensors[0].id] }).then(() => invoke('get_stats', { ids: [s.sensors[0].id] })))
  -> {"revision":2,"stats":[null]}
```

Da controllare:
- nessuna riga `ERROR`: un comando assente da `generate_handler!`, dal manifest di `build.rs` o dalla capability produce una riga `ERROR` con il messaggio di rifiuto di Tauri;
- `startedAtMs` è un'ora Unix di pochi secondi prima;
- `count` di `get_stats` è minore o uguale al numero di righe della finestra grezza: il carico CPU è un contatore a tasso e il primo campione è vuoto (§4.1);
- con `maxPoints: 4` si ottengono esattamente 4 righe, alternate minimo/massimo;
- dopo `reset_stats` il sensore torna `null`;
- l'istanza dell'utente, se c'era, è ancora in esecuzione (`Get-Process oma-app` mostra il suo `Path` in `target\release`).

Alla fine si possono cancellare la cartella `$env:TEMP\oma-m3-verify` e il profilo WebView dell'identificatore di verifica (`Remove-Item -Recurse -Force "$env:TEMP\oma-m3-verify", "$env:LOCALAPPDATA\io.github.openmonitoradvanced.verify"`). Il profilo reale `io.github.openmonitoradvanced` non viene toccato.

- [ ] **Step 7: Commit**

```bash
git add app/src-tauri/src/commands.rs app/src-tauri/src/main.rs app/src-tauri/build.rs app/src-tauri/capabilities/default.json app/src-tauri/permissions/autogenerated/get_stats.toml app/src-tauri/permissions/autogenerated/reset_stats.toml app/src-tauri/permissions/autogenerated/get_session.toml
git commit -m "feat(app): stats, session and decimated history commands"
```

---

---

### Task 4: `oma-win`: identità dei dischi con catena di ripiego (D11)

**File:**
- Crea: `crates/oma-win/src/storage_ioctl.rs` (handle di un disco aperto con accesso 0, `DeviceIoControl`, letture little-endian)
- Modifica:
  - `crates/oma-win/src/storage_identity.rs` (catena seriale → GPT → MBR → PnP, regola dell'ambiguità per livello, seriale non UTF-8)
  - `crates/oma-win/src/storage.rs` (la discovery usa la catena)
  - `crates/oma-win/src/lib.rs` (`mod storage_ioctl;`)
  - `crates/oma-win/Cargo.toml` (feature `Win32_Devices_DeviceAndDriverInstallation`)
- Test:
  - moduli `#[cfg(test)]` di `storage_ioctl.rs` e `storage_identity.rs` (unitari puri + un test hardware `#[ignore]`);
  - `crates/oma-win/tests/providers.rs` invariato: `storage_provider_reports_disks_and_volumes` va rieseguito.

**Interfacce:**
- Usa (M1): `StorageProvider::discover` in `storage.rs`, che oggi chiama `disk_identity(index)` e scarta sia i dischi senza seriale sia quelli con seriale duplicato; `volume_identity(letter)` (invariata).
- Produce (in `storage_ioctl.rs`, usate anche dal Task 5):
  ```rust
  pub(crate) fn le_u32(bytes: &[u8], offset: usize) -> Option<u32>;
  pub(crate) fn le_i64(bytes: &[u8], offset: usize) -> Option<i64>;
  pub(crate) struct PhysicalDrive(HANDLE); // chiude l'handle nel Drop
  impl PhysicalDrive {
      pub(crate) fn open(index: u32) -> Option<Self>;          // \\.\PhysicalDrive<index>, accesso 0
      pub(crate) fn open_path(path: &str) -> Option<Self>;
      pub(crate) fn ioctl(&self, code: u32, input: Option<&[u8]>, capacity: usize) -> Option<Vec<u8>>;
      pub(crate) fn query_property(&self, property: STORAGE_PROPERTY_ID, capacity: usize) -> Option<Vec<u8>>;
      pub(crate) fn disk_number(&self) -> Option<u32>;         // IOCTL_STORAGE_GET_DEVICE_NUMBER, solo dischi
  }
  ```
- Produce (in `storage_identity.rs`, contratto D11):
  ```rust
  pub(crate) fn disk_identity(index: u32) -> Option<String>;   // firma invariata: livello seriale
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub(crate) enum IdentityTier { Serial, Gpt, Mbr, Pnp }
  #[derive(Debug, Clone, Default, PartialEq, Eq)]
  pub(crate) struct DiskIdentityCandidates { pub serial: Option<String>, pub gpt: Option<String>, pub mbr: Option<String>, pub pnp: Option<String> }
  pub(crate) fn disk_identity_candidates(index: u32) -> DiskIdentityCandidates;
  pub(crate) fn assign_disk_ids(candidates: &BTreeMap<u32, DiskIdentityCandidates>) -> BTreeMap<u32, (String, IdentityTier)>;
  ```

**Fatti verificati** (spike `diskspike` e prova di questo task su questa macchina, da utente normale, senza elevazione):
- Tutte le IOCTL usate funzionano con `\\.\PhysicalDriveN` aperto con accesso 0 (`FILE_SHARE_READ | FILE_SHARE_WRITE`): `IOCTL_STORAGE_QUERY_PROPERTY`, `IOCTL_DISK_GET_DRIVE_LAYOUT_EX` (GUID del disco GPT su tutti e 4 i dischi), `IOCTL_DISK_GET_DRIVE_GEOMETRY_EX` (dimensione del disco; `IOCTL_DISK_GET_LENGTH_INFO` invece richiede accesso in lettura e non si usa), `IOCTL_STORAGE_GET_DEVICE_NUMBER`.
- **Instance id PnP:** SetupAPI (`SetupDiGetClassDevsW` con `GUID_DEVINTERFACE_DISK`, che `windows` 0.62 esporta in `Win32::System::Ioctl`). Il percorso dell'interfaccia non è `\\.\PhysicalDriveN`: si apre il percorso e si confronta `IOCTL_STORAGE_GET_DEVICE_NUMBER` (tipo `FILE_DEVICE_DISK` = 7, non esportato dal crate). Esempio: `SCSI\DISK&VEN_NVME&PROD_SHPP41-2000GM\5&1EB1FAB4&0&000000`. L'id contiene l'indirizzo della porta: cambia se il disco viene spostato, quindi è l'ultimo livello.
- **Costo misurato:** `disk_identity_candidates` per i 4 dischi 1,0–2,6 ms in totale, di cui circa 0,5 ms per le 4 enumerazioni SetupAPI. Si esegue solo nella discovery.
- **Questa macchina:** 4 dischi, tutti GPT e con seriale (Seagate ST2000DM008 SATA, Corsair Force LS SATA, Fanxiang S880 NVMe, SHPP41-2000GM NVMe); nessuno ha la firma MBR. I livelli di ripiego si verificano su hardware togliendo i seriali ai candidati reali (test `every_disk_of_this_machine_has_an_identity`); un disco senza seriale vero (VHDX) non si può creare senza privilegi amministrativi.

**Regole fissate qui (D11):**
- **Id per livello**, sempre `storage/<prefisso>-<sha256 esadecimale>`; nessun dato grezzo esce dal modulo:
  - seriale: `storage/device-` + sha256(`vendor\0model\0seriale`), **identico a M1/M2** per i seriali UTF-8 (il test `serial_ids_are_unchanged_since_m1` fissa un hash calcolato con il codice di M1). Un seriale non UTF-8 si usa come byte grezzi, senza gli spazi ASCII ai bordi, invece di scartare il disco;
  - GPT: `storage/gpt-` + sha256 dei 16 byte del GUID del disco; GUID tutto zero = assente;
  - MBR: `storage/mbr-` + sha256(firma `u32` LE ++ dimensione del disco `u64` LE); firma 0 o dimensione ignota = assente;
  - PnP: `storage/pnp-` + sha256 dell'instance id in maiuscolo (Windows lo tratta senza distinzione tra maiuscole e minuscole).
- **Assegnazione** (`assign_disk_ids`, pura): per ogni livello, nell'ordine seriale → GPT → MBR → PnP, un disco non ancora identificato prende il valore di quel livello solo se è **unico tra tutti i dischi della macchina**, identificati o no. Un clone byte per byte ha lo stesso GUID GPT del disco originale anche quando l'originale è identificato dal seriale: in quel caso il clone scende al livello PnP. Un disco si omette solo se tutti i livelli falliscono.
- **Log:** un disco omesso produce `warn!` con la causa per ogni livello (per esempio `serial: unavailable; gpt: shared with another disk; mbr: unavailable; pnp: unavailable`); un disco identificato da un livello di ripiego produce una riga `info!` con il livello scelto e i livelli saltati. Il `warn!` di `storage.rs` si sposta quindi in `assign_disk_ids`.
- Gli indici PDH restano usati solo durante la discovery, come in M1.

- [ ] **Step 1: Feature di `windows` e dichiarazione del modulo**

In `crates/oma-win/Cargo.toml`, nell'elenco `features` di `windows`, aggiungi dopo `"Wdk_Graphics_Direct3D",`:

```toml
  "Win32_Devices_DeviceAndDriverInstallation",
```

(Se un task precedente l'ha già aggiunta, non duplicarla.)

In `crates/oma-win/src/lib.rs` sostituisci

```rust
mod storage_identity;
```

con

```rust
mod storage_identity;
mod storage_ioctl;
```

- [ ] **Step 2: Scrivi i test di `storage_ioctl.rs` (falliscono)**

Crea `crates/oma-win/src/storage_ioctl.rs` con la sola parte di test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_fields_within_bounds() {
        let bytes = [1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(le_u32(&bytes, 0), Some(1));
        assert_eq!(le_i64(&bytes, 4), Some(-1));
        assert_eq!(le_u32(&bytes, 9), None);
        assert_eq!(le_i64(&bytes, 5), None);
        assert_eq!(le_u32(&bytes, usize::MAX), None);
    }

    #[test]
    fn disk_number_only_for_disks() {
        let mut bytes = [0u8; 12];
        bytes[0..4].copy_from_slice(&FILE_DEVICE_DISK.to_le_bytes());
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(parse_disk_number(&bytes), Some(3));
        bytes[0..4].copy_from_slice(&2u32.to_le_bytes()); // CD-ROM
        assert_eq!(parse_disk_number(&bytes), None);
        assert_eq!(parse_disk_number(&bytes[..6]), None);
    }
}
```

- [ ] **Step 3: Sostituisci i test di `storage_identity.rs` (falliscono)**

In `crates/oma-win/src/storage_identity.rs` sostituisci l'intero modulo `#[cfg(test)] mod tests { … }` (dalla riga `#[cfg(test)]` alla fine del file) con:

```rust
#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

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
        assert_eq!(
            identity_from_descriptor(&serial),
            identity_from_descriptor(&serial)
        );
        assert_ne!(
            identity_from_descriptor(&serial),
            identity_from_descriptor(&descriptor(b"serial-b"))
        );
        assert!(!identity_from_descriptor(&serial)
            .unwrap()
            .contains("serial-a"));
    }

    #[test]
    fn serial_ids_are_unchanged_since_m1() {
        // sha256("\0\0serial-a"): no vendor, no model, as computed by M1.
        assert_eq!(
            identity_from_descriptor(&descriptor(b"  serial-a ")).as_deref(),
            Some("storage/device-1963b90ffdd2185fc60ac5ebd1a877aa8bca8a2098d890bd4ffcb4b5d9ec4cee")
        );
    }

    #[test]
    fn non_utf8_serial_is_hashed_raw() {
        let raw = identity_from_descriptor(&descriptor(b" \xFF\xFEserial ")).expect("raw serial");
        assert!(raw.starts_with("storage/device-"));
        assert_ne!(
            Some(raw),
            identity_from_descriptor(&descriptor(b"\xFF\xFDserial"))
        );
    }

    #[test]
    fn missing_or_malformed_serial_has_no_identity() {
        assert!(identity_from_descriptor(&descriptor(b"")).is_none());
        assert!(identity_from_descriptor(&descriptor(b"   ")).is_none());
        assert!(identity_from_descriptor(&[0; 36]).is_none());
        let mut bytes = descriptor(b"abc");
        bytes[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(identity_from_descriptor(&bytes).is_none());
    }

    fn layout(style: u32, union: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 48];
        bytes[0..4].copy_from_slice(&style.to_le_bytes());
        bytes[8..8 + union.len()].copy_from_slice(union);
        bytes
    }

    const GUID_A: [u8; 16] = [
        0x18, 0xFE, 0x37, 0xB9, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    ];

    #[test]
    fn gpt_identity_hashes_the_disk_guid() {
        let gpt = gpt_identity(&layout(1, &GUID_A)).expect("gpt id");
        assert!(gpt.starts_with("storage/gpt-"));
        assert_eq!(gpt.len(), "storage/gpt-".len() + 64);
        let mut other = GUID_A;
        other[15] ^= 1;
        assert_ne!(Some(gpt), gpt_identity(&layout(1, &other)));
        assert_eq!(gpt_identity(&layout(1, &[0; 16])), None);
        assert_eq!(gpt_identity(&layout(0, &GUID_A)), None); // MBR disk
        assert_eq!(gpt_identity(&layout(2, &GUID_A)), None); // RAW disk
        assert_eq!(gpt_identity(&layout(1, &GUID_A)[..20]), None);
    }

    #[test]
    fn mbr_identity_needs_signature_and_size() {
        let signature = 0x1234_ABCDu32.to_le_bytes();
        let mbr = mbr_identity(&layout(0, &signature), Some(512 << 30)).expect("mbr id");
        assert!(mbr.starts_with("storage/mbr-"));
        assert_ne!(
            Some(mbr),
            mbr_identity(&layout(0, &signature), Some(256 << 30)),
            "same signature, different size"
        );
        assert_eq!(mbr_identity(&layout(0, &signature), None), None);
        assert_eq!(mbr_identity(&layout(0, &signature), Some(0)), None);
        assert_eq!(mbr_identity(&layout(0, &[0; 4]), Some(512 << 30)), None);
        assert_eq!(mbr_identity(&layout(1, &signature), Some(512 << 30)), None);
    }

    #[test]
    fn disk_size_from_geometry() {
        let mut geometry = vec![0u8; 40];
        geometry[24..32].copy_from_slice(&2_000_398_934_016i64.to_le_bytes());
        assert_eq!(disk_size(&geometry), Some(2_000_398_934_016));
        geometry[24..32].copy_from_slice(&(-1i64).to_le_bytes());
        assert_eq!(disk_size(&geometry), None);
        assert_eq!(disk_size(&geometry[..30]), None);
    }

    #[test]
    fn pnp_identity_is_case_insensitive_and_hashed() {
        let id = r"SCSI\DISK&VEN_NVME&PROD_SHPP41-2000GM\5&1EB1FAB4&0&000000";
        let hashed = pnp_identity(id).expect("pnp id");
        assert!(hashed.starts_with("storage/pnp-"));
        assert!(!hashed.to_uppercase().contains("SHPP41"));
        assert_eq!(pnp_identity(&id.to_lowercase()), Some(hashed));
        assert_eq!(pnp_identity("  "), None);
    }

    fn candidates(
        serial: Option<&str>,
        gpt: Option<&str>,
        mbr: Option<&str>,
        pnp: Option<&str>,
    ) -> DiskIdentityCandidates {
        DiskIdentityCandidates {
            serial: serial.map(str::to_owned),
            gpt: gpt.map(str::to_owned),
            mbr: mbr.map(str::to_owned),
            pnp: pnp.map(str::to_owned),
        }
    }

    fn ids(disks: &[(u32, DiskIdentityCandidates)]) -> BTreeMap<u32, (String, IdentityTier)> {
        assign_disk_ids(&disks.iter().cloned().collect())
    }

    #[test]
    fn unique_serials_win() {
        let result = ids(&[
            (0, candidates(Some("s0"), Some("g0"), None, Some("p0"))),
            (1, candidates(Some("s1"), Some("g1"), None, Some("p1"))),
        ]);
        assert_eq!(result[&0], ("s0".to_owned(), IdentityTier::Serial));
        assert_eq!(result[&1], ("s1".to_owned(), IdentityTier::Serial));
    }

    #[test]
    fn ambiguous_serials_fall_back_to_the_gpt_disk_id() {
        let result = ids(&[
            (0, candidates(Some("same"), Some("g0"), None, Some("p0"))),
            (1, candidates(Some("same"), Some("g1"), None, Some("p1"))),
        ]);
        assert_eq!(result[&0], ("g0".to_owned(), IdentityTier::Gpt));
        assert_eq!(result[&1], ("g1".to_owned(), IdentityTier::Gpt));
    }

    #[test]
    fn serial_less_disk_uses_the_gpt_disk_id() {
        let result = ids(&[
            (0, candidates(Some("s0"), Some("g0"), None, Some("p0"))),
            (1, candidates(None, Some("g1"), None, Some("p1"))),
        ]);
        assert_eq!(result[&0].1, IdentityTier::Serial);
        assert_eq!(result[&1], ("g1".to_owned(), IdentityTier::Gpt));
    }

    #[test]
    fn cloned_gpt_disks_fall_back_to_pnp() {
        // A byte-for-byte clone shares the GPT disk id with its source, even
        // when the source itself is identified by its serial.
        let result = ids(&[
            (0, candidates(Some("s0"), Some("g"), None, Some("p0"))),
            (1, candidates(None, Some("g"), None, Some("p1"))),
            (2, candidates(None, Some("g"), None, Some("p2"))),
        ]);
        assert_eq!(result[&0], ("s0".to_owned(), IdentityTier::Serial));
        assert_eq!(result[&1], ("p1".to_owned(), IdentityTier::Pnp));
        assert_eq!(result[&2], ("p2".to_owned(), IdentityTier::Pnp));
    }

    #[test]
    fn mbr_disk_without_serial_uses_the_mbr_signature() {
        let result = ids(&[(3, candidates(None, None, Some("m3"), Some("p3")))]);
        assert_eq!(result[&3], ("m3".to_owned(), IdentityTier::Mbr));
    }

    #[test]
    fn disk_is_omitted_only_when_every_tier_fails() {
        let result = ids(&[
            (0, candidates(None, None, None, None)),
            (1, candidates(None, Some("g"), None, Some("p"))),
            (2, candidates(None, Some("g"), None, Some("p"))),
            (3, candidates(None, None, None, Some("p3"))),
        ]);
        assert_eq!(result.keys().copied().collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn omission_causes_name_every_tier() {
        let (assigned, causes) = resolve(
            &[
                (0, candidates(None, Some("g"), None, None)),
                (1, candidates(Some("s1"), Some("g"), None, None)),
            ]
            .into_iter()
            .collect(),
        );
        assert!(!assigned.contains_key(&0));
        assert_eq!(
            causes[&0],
            vec![
                "serial: unavailable",
                "gpt: shared with another disk",
                "mbr: unavailable",
                "pnp: unavailable",
            ]
        );
        assert!(!causes.contains_key(&1), "disk 1 used its first tier");
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn every_disk_of_this_machine_has_an_identity() {
        let found: BTreeMap<u32, DiskIdentityCandidates> = (0..16)
            .filter(|&index| PhysicalDrive::open(index).is_some())
            .map(|index| (index, disk_identity_candidates(index)))
            .collect();
        assert!(!found.is_empty(), "no physical drive");
        for (index, c) in &found {
            println!("disk {index}: {c:?}");
            assert_eq!(c.serial, disk_identity(*index), "disk {index}");
            assert!(c.pnp.is_some(), "disk {index}: no PnP instance id");
            assert!(
                c.gpt.is_some() || c.mbr.is_some(),
                "disk {index}: no partition-table identity"
            );
        }
        let assigned = assign_disk_ids(&found);
        assert_eq!(assigned.len(), found.len(), "every disk identified");

        // The same disks without a serial: the fallback tiers alone still
        // identify every disk, uniquely.
        for strip in [1usize, 3] {
            let stripped: BTreeMap<u32, DiskIdentityCandidates> = found
                .iter()
                .map(|(&index, c)| {
                    let mut c = c.clone();
                    c.serial = None;
                    if strip == 3 {
                        c.gpt = None;
                        c.mbr = None;
                    }
                    (index, c)
                })
                .collect();
            let fallback = assign_disk_ids(&stripped);
            assert_eq!(fallback.len(), found.len(), "strip {strip}");
            let unique: BTreeSet<&String> = fallback.values().map(|(id, _)| id).collect();
            assert_eq!(unique.len(), fallback.len(), "strip {strip}");
            assert!(fallback
                .values()
                .all(|(_, tier)| *tier != IdentityTier::Serial));
        }
    }
}
```

- [ ] **Step 4: Esegui i test e verifica che falliscano**

```bash
cargo test -p oma-win --lib storage
```

Risultato atteso: la compilazione dei test fallisce con errori `cannot find function …` (`le_u32`, `parse_disk_number`, `resolve`, `gpt_identity`, `mbr_identity`, `disk_size`, `pnp_identity`, `assign_disk_ids`, `disk_identity_candidates`…) e `cannot find type …`/`failed to resolve` per `DiskIdentityCandidates`, `IdentityTier`, `PhysicalDrive`.

- [ ] **Step 5: Implementa `storage_ioctl.rs`**

In `crates/oma-win/src/storage_ioctl.rs` inserisci, sopra `#[cfg(test)]`:

```rust
//! Metadata-only access to disks: `\\.\PhysicalDriveN` (or a disk interface
//! path) opened with zero desired access, so no administrator rights are
//! needed and no data is ever read or written.

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_ID, STORAGE_PROPERTY_QUERY,
};
use windows::Win32::System::IO::DeviceIoControl;

/// `FILE_DEVICE_DISK` device type (winioctl.h); not exported by the `windows` crate.
const FILE_DEVICE_DISK: u32 = 7;

const _: () = assert!(size_of::<STORAGE_PROPERTY_QUERY>() == 12);
const _: () = assert!(size_of::<STORAGE_DEVICE_NUMBER>() == 12);
const _: () = assert!(std::mem::offset_of!(STORAGE_DEVICE_NUMBER, DeviceNumber) == 4);

/// Little-endian `u32` at `offset`, `None` past the end.
pub(crate) fn le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Little-endian `i64` at `offset`, `None` past the end.
pub(crate) fn le_i64(bytes: &[u8], offset: usize) -> Option<i64> {
    Some(i64::from_le_bytes(
        bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// Disk number from a `STORAGE_DEVICE_NUMBER`; `None` for devices that are not disks.
fn parse_disk_number(bytes: &[u8]) -> Option<u32> {
    if le_u32(bytes, 0)? != FILE_DEVICE_DISK {
        return None;
    }
    le_u32(bytes, 4)
}

/// An open disk handle, closed on drop.
pub(crate) struct PhysicalDrive(HANDLE);

impl PhysicalDrive {
    /// Opens `\\.\PhysicalDrive<index>`; `None` if it does not exist.
    pub(crate) fn open(index: u32) -> Option<Self> {
        Self::open_path(&format!(r"\\.\PhysicalDrive{index}"))
    }

    /// Opens a disk device path, such as a disk device interface path.
    pub(crate) fn open_path(path: &str) -> Option<Self> {
        let path = HSTRING::from(path);
        // SAFETY: valid NUL-terminated path; zero desired access only allows
        // metadata queries, never reads or writes of disk data.
        let handle = unsafe {
            CreateFileW(
                &path,
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .ok()?;
        Some(Self(handle))
    }

    /// Sends `code` with an optional input buffer and returns the bytes the
    /// driver wrote (at most `capacity`); `None` if the request failed.
    pub(crate) fn ioctl(
        &self,
        code: u32,
        input: Option<&[u8]>,
        capacity: usize,
    ) -> Option<Vec<u8>> {
        let mut out = vec![0u8; capacity];
        let mut returned = 0u32;
        // SAFETY: the handle is open; the input slice and the output buffer are
        // valid for the lengths passed; `returned` is a valid out-pointer.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                input.map(|bytes| bytes.as_ptr().cast()),
                input.map_or(0, |bytes| bytes.len() as u32),
                Some(out.as_mut_ptr().cast()),
                out.len() as u32,
                Some(&mut returned),
                None,
            )
        }
        .ok()?;
        let returned = returned as usize;
        if returned > out.len() {
            return None;
        }
        out.truncate(returned);
        Some(out)
    }

    /// `IOCTL_STORAGE_QUERY_PROPERTY` standard query for `property`.
    pub(crate) fn query_property(
        &self,
        property: STORAGE_PROPERTY_ID,
        capacity: usize,
    ) -> Option<Vec<u8>> {
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: property,
            QueryType: PropertyStandardQuery,
            ..Default::default()
        };
        // SAFETY: STORAGE_PROPERTY_QUERY is plain data; the slice covers
        // exactly its bytes and does not outlive `query`.
        let input = unsafe {
            std::slice::from_raw_parts(
                (&query as *const STORAGE_PROPERTY_QUERY).cast::<u8>(),
                size_of::<STORAGE_PROPERTY_QUERY>(),
            )
        };
        self.ioctl(IOCTL_STORAGE_QUERY_PROPERTY, Some(input), capacity)
    }

    /// The N of `\\.\PhysicalDriveN` for this device; `None` if it is not a disk.
    pub(crate) fn disk_number(&self) -> Option<u32> {
        let bytes = self.ioctl(
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            size_of::<STORAGE_DEVICE_NUMBER>(),
        )?;
        parse_disk_number(&bytes)
    }
}

impl Drop for PhysicalDrive {
    fn drop(&mut self) {
        // SAFETY: this value is the sole owner of the handle, never used after drop.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

```

- [ ] **Step 6: Implementa la catena in `storage_identity.rs`**

In `crates/oma-win/src/storage_identity.rs` sostituisci tutto ciò che precede `#[cfg(test)]` (commento del modulo, `use`, `descriptor_text`, `identity_from_descriptor`, `disk_identity`, `volume_identity`) con:

```rust
//! Persistent identities. Raw serial numbers, disk GUIDs, MBR signatures and
//! PnP instance ids never leave this module: only their SHA-256 hashes do.
//!
//! Disk identity is a fallback chain (serial, GPT disk id, MBR signature plus
//! size, PnP instance id). A tier is used only when its value is unique among
//! the disks of this machine, so two disks never merge under one id.
use std::collections::{BTreeMap, HashMap};

use sha2::{Digest, Sha256};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE,
    DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SP_DEVINFO_DATA,
};
use windows::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
use windows::Win32::System::Ioctl::{
    StorageDeviceProperty, DISK_GEOMETRY_EX, DRIVE_LAYOUT_INFORMATION_EX,
    DRIVE_LAYOUT_INFORMATION_GPT, DRIVE_LAYOUT_INFORMATION_MBR, GUID_DEVINTERFACE_DISK,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, PARTITION_STYLE_GPT,
    PARTITION_STYLE_MBR,
};

use crate::storage_ioctl::{le_i64, le_u32, PhysicalDrive};

/// DRIVE_LAYOUT_INFORMATION_EX: `PartitionStyle` at 0, the MBR/GPT union at 8.
const LAYOUT_STYLE: usize = 0;
const LAYOUT_UNION: usize = 8;
/// DISK_GEOMETRY_EX: `DiskSize` follows the 24-byte DISK_GEOMETRY.
const GEOMETRY_DISK_SIZE: usize = 24;

const _: () = assert!(std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, PartitionStyle) == 0);
const _: () = assert!(std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_EX, Anonymous) == 8);
const _: () = assert!(std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_GPT, DiskId) == 0);
const _: () = assert!(size_of::<windows::core::GUID>() == 16);
const _: () = assert!(std::mem::offset_of!(DRIVE_LAYOUT_INFORMATION_MBR, Signature) == 0);
const _: () = assert!(std::mem::offset_of!(DISK_GEOMETRY_EX, DiskSize) == 24);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<SP_DEVICE_INTERFACE_DATA>() == 32);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<SP_DEVINFO_DATA>() == 32);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() == 8);
const _: () = assert!(std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath) == 4);

/// Bytes of the NUL-terminated string whose offset is stored at `field`.
fn descriptor_field(bytes: &[u8], field: usize) -> Option<&[u8]> {
    let offset = le_u32(bytes, field)? as usize;
    if offset < 36 {
        return None;
    }
    let tail = bytes.get(offset..)?;
    let end = tail.iter().position(|&b| b == 0)?;
    Some(&tail[..end])
}

fn descriptor_text(bytes: &[u8], field: usize) -> Option<&str> {
    let text = std::str::from_utf8(descriptor_field(bytes, field)?)
        .ok()?
        .trim();
    (!text.is_empty()).then_some(text)
}

/// The serial as hashed: the trimmed text when it is UTF-8 (ids unchanged
/// since M1), otherwise the raw bytes without surrounding ASCII whitespace.
fn descriptor_serial(bytes: &[u8]) -> Option<&[u8]> {
    let raw = descriptor_field(bytes, 24)?;
    let serial = match std::str::from_utf8(raw) {
        Ok(text) => text.trim().as_bytes(),
        Err(_) => raw.trim_ascii(),
    };
    (!serial.is_empty()).then_some(serial)
}

fn identity_from_descriptor(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 36 {
        return None;
    }
    let serial = descriptor_serial(bytes)?;
    let vendor = descriptor_text(bytes, 12).unwrap_or("");
    let model = descriptor_text(bytes, 16).unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update(vendor.as_bytes());
    hasher.update([0]);
    hasher.update(model.as_bytes());
    hasher.update([0]);
    hasher.update(serial);
    Some(format!("storage/device-{:x}", hasher.finalize()))
}

/// GPT tier: hash of the disk GUID stored in the GPT header.
fn gpt_identity(layout: &[u8]) -> Option<String> {
    if le_u32(layout, LAYOUT_STYLE)? != PARTITION_STYLE_GPT.0 as u32 {
        return None;
    }
    let disk_id = layout.get(LAYOUT_UNION..LAYOUT_UNION + 16)?;
    if disk_id.iter().all(|&b| b == 0) {
        return None;
    }
    Some(format!("storage/gpt-{:x}", Sha256::digest(disk_id)))
}

/// MBR tier: hash of the 32-bit disk signature and the disk size. Signature 0
/// means "never initialised" and is not an identity.
fn mbr_identity(layout: &[u8], disk_size: Option<u64>) -> Option<String> {
    if le_u32(layout, LAYOUT_STYLE)? != PARTITION_STYLE_MBR.0 as u32 {
        return None;
    }
    let signature = le_u32(layout, LAYOUT_UNION).filter(|&s| s != 0)?;
    let size = disk_size.filter(|&s| s > 0)?;
    let mut hasher = Sha256::new();
    hasher.update(signature.to_le_bytes());
    hasher.update(size.to_le_bytes());
    Some(format!("storage/mbr-{:x}", hasher.finalize()))
}

/// `DiskSize` of a DISK_GEOMETRY_EX, in bytes.
fn disk_size(geometry: &[u8]) -> Option<u64> {
    le_i64(geometry, GEOMETRY_DISK_SIZE).and_then(|size| u64::try_from(size).ok())
}

/// PnP tier: hash of the device instance id (case-insensitive in Windows).
fn pnp_identity(instance_id: &str) -> Option<String> {
    let id = instance_id.trim().to_uppercase();
    (!id.is_empty()).then(|| format!("storage/pnp-{:x}", Sha256::digest(id.as_bytes())))
}

/// Serial tier: `storage/device-<sha256(vendor\0model\0serial)>`.
pub(crate) fn disk_identity(index: u32) -> Option<String> {
    let bytes = PhysicalDrive::open(index)?.query_property(StorageDeviceProperty, 65_536)?;
    identity_from_descriptor(&bytes)
}

/// Identity tiers, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentityTier {
    /// Travels with the drive.
    Serial,
    /// Stored on the media; duplicated by byte-for-byte clones.
    Gpt,
    /// 32-bit signature plus disk size; duplicated by clones.
    Mbr,
    /// Bound to the port or slot: changes if the disk is moved.
    Pnp,
}

impl IdentityTier {
    const ALL: [IdentityTier; 4] = [
        IdentityTier::Serial,
        IdentityTier::Gpt,
        IdentityTier::Mbr,
        IdentityTier::Pnp,
    ];

    fn name(self) -> &'static str {
        match self {
            IdentityTier::Serial => "serial",
            IdentityTier::Gpt => "gpt",
            IdentityTier::Mbr => "mbr",
            IdentityTier::Pnp => "pnp",
        }
    }
}

/// Full candidate ids (`storage/<tier prefix>-<sha256 hex>`) of one disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DiskIdentityCandidates {
    pub serial: Option<String>,
    pub gpt: Option<String>,
    pub mbr: Option<String>,
    pub pnp: Option<String>,
}

impl DiskIdentityCandidates {
    fn get(&self, tier: IdentityTier) -> Option<&str> {
        match tier {
            IdentityTier::Serial => self.serial.as_deref(),
            IdentityTier::Gpt => self.gpt.as_deref(),
            IdentityTier::Mbr => self.mbr.as_deref(),
            IdentityTier::Pnp => self.pnp.as_deref(),
        }
    }
}

/// Reads every identity tier of `\\.\PhysicalDrive<index>`; a tier that
/// cannot be read is `None`.
pub(crate) fn disk_identity_candidates(index: u32) -> DiskIdentityCandidates {
    let drive = PhysicalDrive::open(index);
    let layout = drive
        .as_ref()
        .and_then(|d| d.ioctl(IOCTL_DISK_GET_DRIVE_LAYOUT_EX, None, 65_536));
    let size = drive
        .as_ref()
        .and_then(|d| d.ioctl(IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, None, 256))
        .and_then(|geometry| disk_size(&geometry));
    DiskIdentityCandidates {
        serial: disk_identity(index),
        gpt: layout.as_deref().and_then(gpt_identity),
        mbr: layout.as_deref().and_then(|l| mbr_identity(l, size)),
        pnp: pnp_instance_id(index).as_deref().and_then(pnp_identity),
    }
}

/// Assigned ids plus, per disk, why each tier before the chosen one (or every
/// tier, for an omitted disk) could not be used.
type Resolution = (
    BTreeMap<u32, (String, IdentityTier)>,
    BTreeMap<u32, Vec<String>>,
);

fn resolve(candidates: &BTreeMap<u32, DiskIdentityCandidates>) -> Resolution {
    let mut assigned = BTreeMap::new();
    let mut causes: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for tier in IdentityTier::ALL {
        // Ambiguity is judged against every disk of the machine, identified
        // or not: a value shared with any other disk is not an identity.
        let mut counts = HashMap::<&str, usize>::new();
        for c in candidates.values() {
            if let Some(id) = c.get(tier) {
                *counts.entry(id).or_default() += 1;
            }
        }
        for (&index, c) in candidates {
            if assigned.contains_key(&index) {
                continue;
            }
            match c.get(tier) {
                None => causes
                    .entry(index)
                    .or_default()
                    .push(format!("{}: unavailable", tier.name())),
                Some(id) if counts[id] > 1 => causes
                    .entry(index)
                    .or_default()
                    .push(format!("{}: shared with another disk", tier.name())),
                Some(id) => {
                    assigned.insert(index, (id.to_owned(), tier));
                }
            }
        }
    }
    (assigned, causes)
}

/// Picks for every disk the strongest tier whose value is unique on this
/// machine. Disks missing from the result have no usable identity and must
/// be omitted; the cause is logged per tier.
pub(crate) fn assign_disk_ids(
    candidates: &BTreeMap<u32, DiskIdentityCandidates>,
) -> BTreeMap<u32, (String, IdentityTier)> {
    let (assigned, causes) = resolve(candidates);
    for (index, reasons) in &causes {
        let reasons = reasons.join("; ");
        match assigned.get(index) {
            Some((_, tier)) => tracing::info!(
                index,
                tier = tier.name(),
                skipped = %reasons,
                "disk identified by a fallback identity"
            ),
            None => tracing::warn!(
                index,
                causes = %reasons,
                "disk has no unique persistent identity; omitted"
            ),
        }
    }
    assigned
}

/// Owns a SetupAPI device information set.
struct DeviceInfoSet(HDEVINFO);

impl Drop for DeviceInfoSet {
    fn drop(&mut self) {
        // SAFETY: sole owner of the set, never used after drop.
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// Device path and device element of one disk interface.
fn interface_detail(
    set: &DeviceInfoSet,
    interface: &SP_DEVICE_INTERFACE_DATA,
) -> Option<(String, SP_DEVINFO_DATA)> {
    let mut required = 0u32;
    // SAFETY: size query with no output buffer; the set and the interface
    // data are valid. It fails with ERROR_INSUFFICIENT_BUFFER by design.
    let _ = unsafe {
        SetupDiGetDeviceInterfaceDetailW(set.0, interface, None, 0, Some(&mut required), None)
    };
    let required = required as usize;
    let path_offset = std::mem::offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
    if required <= path_offset {
        return None;
    }
    // u32 storage keeps the variable-length structure 4-byte aligned.
    let mut buffer = vec![0u32; required.div_ceil(4)];
    let detail = buffer
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    // SAFETY: the buffer holds at least `required` bytes, is suitably aligned,
    // and cbSize must be the size of the fixed part, as documented.
    unsafe {
        (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
    }
    let mut device = SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    };
    // SAFETY: `detail` points to `required` writable bytes; `device` has its
    // cbSize set; the set and the interface data are valid.
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set.0,
            interface,
            Some(detail),
            required as u32,
            None,
            Some(&mut device),
        )
    }
    .ok()?;
    // SAFETY: the path lies within the `required` bytes of the buffer, right
    // after cbSize, as UTF-16 code units.
    let path = unsafe {
        std::slice::from_raw_parts(
            buffer.as_ptr().cast::<u8>().add(path_offset).cast::<u16>(),
            (required - path_offset) / 2,
        )
    };
    let end = path.iter().position(|&c| c == 0).unwrap_or(path.len());
    Some((String::from_utf16_lossy(&path[..end]), device))
}

fn instance_id(set: &DeviceInfoSet, device: &SP_DEVINFO_DATA) -> Option<String> {
    // Device instance ids are at most 200 characters (MAX_DEVICE_ID_LEN).
    let mut buffer = [0u16; 512];
    // SAFETY: `device` belongs to `set`; the buffer is writable for its length.
    unsafe { SetupDiGetDeviceInstanceIdW(set.0, device, Some(&mut buffer), None) }.ok()?;
    let end = buffer.iter().position(|&c| c == 0)?;
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// PnP device instance id of `\\.\PhysicalDrive<index>`. The disk interface
/// path is not the PhysicalDrive path, so each interface is opened and matched
/// by its disk number.
fn pnp_instance_id(index: u32) -> Option<String> {
    // SAFETY: valid interface class GUID, no enumerator, no parent window.
    let set = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_DISK),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }
    .ok()?;
    let set = DeviceInfoSet(set);
    for member in 0u32.. {
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: valid set, class GUID and interface data with cbSize set.
        // Failure (ERROR_NO_MORE_ITEMS) ends the enumeration.
        let more = unsafe {
            SetupDiEnumDeviceInterfaces(
                set.0,
                None,
                &GUID_DEVINTERFACE_DISK,
                member,
                &mut interface,
            )
        };
        if more.is_err() {
            return None;
        }
        let Some((path, device)) = interface_detail(&set, &interface) else {
            continue;
        };
        if PhysicalDrive::open_path(&path).and_then(|d| d.disk_number()) == Some(index) {
            return instance_id(&set, &device);
        }
    }
    None
}

pub(crate) fn volume_identity(letter: &str) -> Option<String> {
    let mut buffer = [0u16; 64];
    let root = HSTRING::from(format!("{letter}\\"));
    // SAFETY: valid mount point and output buffer.
    unsafe { GetVolumeNameForVolumeMountPointW(&root, &mut buffer) }.ok()?;
    let end = buffer.iter().position(|&v| v == 0)?;
    let name = String::from_utf16_lossy(&buffer[..end]).to_ascii_lowercase();
    Some(
        name.strip_prefix(r"\\?\volume{")?
            .strip_suffix("}\\")?
            .to_owned(),
    )
}

```

- [ ] **Step 7: Usa la catena nella discovery (`storage.rs`)**

In `crates/oma-win/src/storage.rs`:

1. sostituisci

```rust
use std::collections::HashMap;
```

con

```rust
use std::collections::{BTreeMap, HashMap};
```

2. sostituisci

```rust
use crate::storage_identity::{disk_identity, volume_identity};
```

con

```rust
use crate::storage_identity::{
    assign_disk_ids, disk_identity_candidates, volume_identity, DiskIdentityCandidates,
};
```

3. in `discover`, sostituisci

```rust
        // Resolve stable identities only during discovery; never persist PDH indices.
        let mut disk_ids: HashMap<u32, String> = disks
            .iter()
            .filter_map(|d| disk_identity(d.index).map(|id| (d.index, id)))
            .collect();
        let mut counts = HashMap::<String, usize>::new();
        for id in disk_ids.values() {
            *counts.entry(id.clone()).or_default() += 1;
        }
        disk_ids.retain(|_, id| counts[id] == 1); // Ambiguous serials must not merge disks.
```

con

```rust
        // Resolve stable identities only during discovery; never persist PDH indices.
        // Fallback chain and ambiguity rule: storage_identity::assign_disk_ids.
        let candidates: BTreeMap<u32, DiskIdentityCandidates> = disks
            .iter()
            .map(|d| (d.index, disk_identity_candidates(d.index)))
            .collect();
        let disk_ids: HashMap<u32, String> = assign_disk_ids(&candidates)
            .into_iter()
            .map(|(index, (id, _tier))| (index, id))
            .collect();
```

4. sempre in `discover`, sostituisci

```rust
            let Some(id) = disk_ids.get(&disk.index).cloned() else {
                tracing::warn!(
                    index = disk.index,
                    "disk has no unique persistent identity; omitted"
                );
                continue;
            };
```

con

```rust
            // assign_disk_ids has already logged why a disk has no identity.
            let Some(id) = disk_ids.get(&disk.index).cloned() else {
                continue;
            };
```

`poll` non cambia: salta già i dischi assenti da `disk_ids`.

- [ ] **Step 8: Esegui i test e verifica che passino**

```bash
cargo fmt --all
cargo test -p oma-win --lib storage
```

Risultato atteso: `test result: ok. 27 passed; 0 failed; 1 ignored` (10 test di `storage`, 15 di `storage_identity`, 2 di `storage_ioctl`; ignorato `every_disk_of_this_machine_has_an_identity`).

- [ ] **Step 9: Verifica su hardware reale**

```bash
cargo test -p oma-win --lib storage_identity -- --include-ignored --nocapture
cargo test -p oma-win --test providers -- --ignored storage
```

Risultato atteso sulla macchina di sviluppo:
- il primo comando stampa 4 righe `disk 0` … `disk 3`, ognuna con `serial: Some("storage/device-…")`, `gpt: Some("storage/gpt-…")`, `mbr: None`, `pnp: Some("storage/pnp-…")`, e termina con `16 passed`. Il test verifica anche che senza seriali tutti e 4 i dischi restino identificati in modo univoco dal GUID GPT e, senza seriali né GPT/MBR, dall'instance id PnP;
- il secondo comando: `storage_provider_reports_disks_and_volumes ... ok`.

Gli id `storage/device-…` restano quelli di M1/M2 (lo garantisce `serial_ids_are_unchanged_since_m1`), quindi lo storico e le impostazioni future che li citano restano validi.

- [ ] **Step 10: Lint e commit**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/oma-win/Cargo.toml crates/oma-win/src/lib.rs crates/oma-win/src/storage_ioctl.rs crates/oma-win/src/storage_identity.rs crates/oma-win/src/storage.rs
git commit -m "feat(win): disk identity fallback chain (serial, GPT, MBR, PnP)"
```

---

---

### Task 5: `oma-win`: temperature dei dischi e soglie come proprietà (§5.1, §13.6)

**File:**
- Crea: `crates/oma-win/src/storage_temperature.rs` (lettura e decodifica di `StorageDeviceTemperatureProperty`, regole di dichiarazione e di aggiornamento)
- Modifica:
  - `crates/oma-win/src/storage_ioctl.rs` (Task 4: `le_u16`, `le_i16`, `PhysicalDrive::powered_on`)
  - `crates/oma-win/src/storage.rs` (sensori di temperatura, proprietà `tempWarningC`/`tempCriticalC`, aggiornamento ogni 30 s)
  - `crates/oma-win/src/lib.rs` (`mod storage_temperature;`)
  - `crates/oma-win/Cargo.toml` (feature `Win32_System_Power`)
  - `app/src/lib/i18n/en.json`, `app/src/lib/i18n/it.json` (`sensor.storage.temperature`, `sensor.storage.temperatureSensor`)
  - `crates/oma-win/tests/labels.rs` (le due chiavi nuove)
  - `crates/oma-win/tests/providers.rs` (il test hardware dello storage controlla le temperature)
- Test: modulo `#[cfg(test)]` di `storage_temperature.rs` (unitari puri + un test hardware `#[ignore]`), `storage_ioctl.rs`, `tests/labels.rs`, `tests/providers.rs`.

**Interfacce:**
- Usa (Task 4): `crate::storage_ioctl::{PhysicalDrive, le_u32}`: `PhysicalDrive::open(index) -> Option<PhysicalDrive>` apre `\\.\PhysicalDrive<index>` con accesso 0; `query_property(STORAGE_PROPERTY_ID, capacity) -> Option<Vec<u8>>` esegue `IOCTL_STORAGE_QUERY_PROPERTY` (query standard) e restituisce i byte scritti dal driver; il campo privato `PhysicalDrive.0` è l'`HANDLE`. In `storage.rs` la discovery del Task 4 calcola `disk_ids: HashMap<u32, String>` con `assign_disk_ids` e salta i dischi senza identità con `let Some(id) = disk_ids.get(&disk.index).cloned() else { continue; };`.
- Usa (M1): `Sensor::new(device_id, kind, name, unit, label, source)`, `Label::new`, `Label::with_arg`, `Device.properties: BTreeMap<String, String>`, la regola `fresh` di `StorageProvider::poll`.
- Produce:
  ```rust
  // storage_ioctl.rs
  pub(crate) fn le_u16(bytes: &[u8], offset: usize) -> Option<u16>;
  pub(crate) fn le_i16(bytes: &[u8], offset: usize) -> Option<i16>;
  impl PhysicalDrive { pub(crate) fn powered_on(&self) -> Option<bool>; } // GetDevicePowerState
  // storage_temperature.rs
  pub(crate) const TEMPERATURE_PERIOD: Duration; // 30 s
  #[derive(Debug, Clone, PartialEq)]
  pub(crate) struct TemperatureReport { pub sensors: BTreeMap<usize, Option<f64>>, pub warning_c: Option<i16>, pub critical_c: Option<i16> }
  pub(crate) fn parse_temperatures(bytes: &[u8]) -> Option<TemperatureReport>;
  pub(crate) fn query_temperatures(drive: &PhysicalDrive) -> Option<TemperatureReport>;
  pub(crate) fn declared_positions(report: &TemperatureReport) -> Vec<usize>;
  pub(crate) fn sensor_name(position: usize) -> String;   // "drive" | "sensor-<n>"
  pub(crate) fn sensor_label(position: usize) -> Label;   // storage.temperature | storage.temperatureSensor {arg n}
  pub(crate) fn declared_values(report: Option<&TemperatureReport>, positions: &[usize]) -> Vec<Option<f64>>;
  pub(crate) fn temperature_properties(report: Option<&TemperatureReport>) -> BTreeMap<String, String>;
  pub(crate) fn refresh_due(read_at: Instant, now: Instant) -> bool;
  pub(crate) fn next_refresh(reads: impl IntoIterator<Item = (u32, Instant)>, now: Instant) -> Option<u32>;
  pub(crate) fn may_query(powered_on: Option<bool>) -> bool;
  ```
- Contratto verso la UI (Task 13): sensori `<disk>/temperature/drive` (etichetta `storage.temperature`) e `<disk>/temperature/sensor-<i>` (etichetta `storage.temperatureSensor`, argomento `<i>`), unità `celsius`, fonte `win32`, categoria `temperature`; proprietà del device `tempWarningC`, `tempCriticalC` (stringhe decimali, °C). Le traduzioni `property.tempWarningC`/`property.tempCriticalC` appartengono ai task della UI, non a questo.

**Fatti verificati** (spike `diskspike` e prova di questo task su questa macchina, da utente normale):
- `StorageDeviceTemperatureProperty` (id 52) funziona senza privilegi, con il disco aperto con accesso 0. Risposte reali:

  | Disco | Sensori (°C) | Warning | Critical |
  |---|---|---|---|
  | 0, Seagate ST2000DM008 (HDD SATA) | 39 | 60 | non riportata (`0x8000`) |
  | 1, Corsair Force LS (SSD SATA) | `ERROR_INVALID_FUNCTION`: non supportato, nessun sensore | — | — |
  | 2, Fanxiang S880 (NVMe) | 48, 48, 40 | 90 | 95 |
  | 3, SHPP41-2000GM (NVMe) | 48, 42, 53 | 86 | 87 |

- **Layout** (`windows` 0.62, `Win32::System::Ioctl`, verificato con asserzioni a compile time): `STORAGE_TEMPERATURE_DATA_DESCRIPTOR` ha `CriticalTemperature: i16` a 8, `WarningTemperature: i16` a 10, `InfoCount: u16` a 12, l'array `TemperatureInfo` a 24; ogni `STORAGE_TEMPERATURE_INFO` è di 16 byte con `Temperature: i16` a 2. `STORAGE_TEMPERATURE_VALUE_NOT_REPORTED` = `0x8000` (cioè `i16::MIN`).
- **Tempi misurati** per una lettura: HDD 1,5–26 ms, NVMe 2,5–4 ms; la prima lettura di un NVMe inattivo (uscita da uno stato a basso consumo) ha richiesto 139 ms. Per questo il poll aggiorna **al massimo un disco per ciclo**: il costo di un tick resta quello di un solo disco, entro la scadenza di 200 ms.
- `GetDevicePowerState` sul disco aperto con accesso 0 risponde in 2–9 µs (`Some(true)` per tutti e 4 i dischi, accesi). Serve a non risvegliare un HDD in standby solo per leggerne la temperatura: la spec (§1.1, "Leggerezza") chiede che l'app non falsi ciò che misura.

**Regole fissate qui:**
- **Dichiarazione** (nella discovery): si legge il disco una volta; ogni `STORAGE_TEMPERATURE_INFO.Index` con un valore riportato diventa un sensore. Indice 0 = temperatura del disco (la "composite" degli NVMe), id `…/temperature/drive`; indice `i > 0` = `…/temperature/sensor-<i>`. Il campo `Index` identifica il sensore, non la posizione nell'array: gli indici possono essere sparsi o riordinati ([contratto Windows](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-storage_temperature_info)). I sensori stanno dopo `…/load/active` e prima dei volumi del disco, nello stesso ordine in `discover` e in `poll`. Il nome storico degli helper `declared_positions` e dei campi `positions` indica qui gli indici del driver.
- **Soglie:** `tempWarningC` e `tempCriticalC` diventano proprietà del device solo se riportate (diverse da `0x8000`) e maggiori di 0.
- **Aggiornamento:** ogni disco si rilegge quando la sua lettura ha almeno 30 s (spec §4.1, "SMART e salute dei dischi: ogni 30 s"); in mezzo si ripete l'ultimo valore. Per ciclo si rilegge al massimo un disco, quello con la lettura più vecchia. La temperatura non è un tasso: anche il primo poll dopo la discovery ha il valore letto nella discovery.
- **Dischi in standby:** se `GetDevicePowerState` dice che il disco è spento, non lo si interroga. Nel poll i suoi valori sono assenti fino alla lettura successiva; nella discovery il disco non dichiara sensori di temperatura. Si conserva comunque una voce di pianificazione per ogni disco identificato, anche senza sensori: ogni 30 s viene ritentato, sempre al massimo un disco per poll. Quando compaiono indici riportati non ancora dichiarati, il provider richiede `Rediscover`; il worker non ha una discovery periodica implicita. Se Windows non sa rispondere (`None`), il disco si legge.
- Un disco che risponde con meno sensori di quelli dichiarati dà `None` per gli indici mancanti: l'allineamento dei valori con i sensori non cambia mai fuori dalla discovery. I tentativi falliti aggiornano anch'essi la scadenza, evitando retry a ogni tick.

- [ ] **Step 1: Feature, modulo, lettori little-endian e stato di alimentazione**

In `crates/oma-win/Cargo.toml`, nell'elenco `features` di `windows`, aggiungi dopo `"Win32_System_Performance",`:

```toml
  "Win32_System_Power",
```

In `crates/oma-win/src/lib.rs` sostituisci

```rust
mod storage_ioctl;
```

con

```rust
mod storage_ioctl;
mod storage_temperature;
```

In `crates/oma-win/src/storage_ioctl.rs`:

1. sostituisci `use windows::core::HSTRING;` con

```rust
use windows::core::{BOOL, HSTRING};
```

2. sostituisci `use windows::Win32::System::IO::DeviceIoControl;` con

```rust
use windows::Win32::System::Power::GetDevicePowerState;
use windows::Win32::System::IO::DeviceIoControl;
```

3. subito prima della funzione `le_u32` (e del suo commento `///`) aggiungi

```rust
/// Little-endian `u16` at `offset`, `None` past the end.
pub(crate) fn le_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

/// Little-endian `i16` at `offset`, `None` past the end.
pub(crate) fn le_i16(bytes: &[u8], offset: usize) -> Option<i16> {
    le_u16(bytes, offset).map(|v| v as i16)
}

```

4. in `impl PhysicalDrive`, dopo il metodo `disk_number`, aggiungi

```rust

    /// `Some(false)` while the disk is spun down or in a low-power state,
    /// `None` if Windows cannot tell. Asking does not wake the disk.
    pub(crate) fn powered_on(&self) -> Option<bool> {
        let mut on = BOOL(0);
        // SAFETY: the handle is open and `on` is a valid out-pointer.
        let known = unsafe { GetDevicePowerState(self.0, &mut on) }.as_bool();
        known.then(|| on.as_bool())
    }
```

5. nel test `reads_little_endian_fields_within_bounds`, dopo `assert_eq!(le_u32(&bytes, usize::MAX), None);` aggiungi

```rust
        assert_eq!(le_u16(&bytes, 0), Some(1));
        assert_eq!(le_i16(&bytes, 4), Some(-1));
        assert_eq!(le_i16(&bytes, 11), None);
```

- [ ] **Step 2: Scrivi i test di `storage_temperature.rs` (falliscono)**

Crea `crates/oma-win/src/storage_temperature.rs` con la sola parte di test (i valori sono quelli reali della tabella sopra):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const NOT_REPORTED: i16 = i16::MIN; // 0x8000

    fn descriptor(critical: i16, warning: i16, temperatures: &[i16]) -> Vec<u8> {
        let mut bytes = vec![0u8; INFO + INFO_SIZE * temperatures.len().max(1)];
        bytes[CRITICAL..CRITICAL + 2].copy_from_slice(&critical.to_le_bytes());
        bytes[WARNING..WARNING + 2].copy_from_slice(&warning.to_le_bytes());
        bytes[INFO_COUNT..INFO_COUNT + 2]
            .copy_from_slice(&(temperatures.len() as u16).to_le_bytes());
        for (i, t) in temperatures.iter().enumerate() {
            let at = INFO + i * INFO_SIZE;
            bytes[at..at + 2].copy_from_slice(&(i as u16).to_le_bytes());
            bytes[at + INFO_TEMPERATURE..at + INFO_TEMPERATURE + 2]
                .copy_from_slice(&t.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn parses_an_nvme_report() {
        // Disk 2 of the development machine: composite plus two sensors.
        let report = parse_temperatures(&descriptor(95, 90, &[48, 48, 39])).unwrap();
        assert_eq!(report.sensors, BTreeMap::from([(0, Some(48.0)), (1, Some(48.0)), (2, Some(39.0))]));
        assert_eq!(report.warning_c, Some(90));
        assert_eq!(report.critical_c, Some(95));
    }

    #[test]
    fn not_reported_values_and_thresholds_are_absent() {
        // Disk 0 of the development machine (SATA HDD): no critical threshold.
        let report =
            parse_temperatures(&descriptor(NOT_REPORTED, 60, &[39, NOT_REPORTED])).unwrap();
        assert_eq!(report.sensors, BTreeMap::from([(0, Some(39.0)), (1, None)]));
        assert_eq!(report.warning_c, Some(60));
        assert_eq!(report.critical_c, None);
        let zero = parse_temperatures(&descriptor(0, -5, &[30])).unwrap();
        assert_eq!((zero.warning_c, zero.critical_c), (None, None));
    }

    #[test]
    fn truncated_descriptors_are_safe() {
        let mut bytes = descriptor(95, 90, &[48, 48, 39]);
        bytes.truncate(INFO + INFO_SIZE + 3); // only the first entry fits
        assert_eq!(
            parse_temperatures(&bytes).unwrap().sensors,
            BTreeMap::from([(0, Some(48.0))])
        );
        assert_eq!(parse_temperatures(&bytes[..12]), None);
    }

    #[test]
    fn sensors_are_declared_only_where_reported() {
        let report = parse_temperatures(&descriptor(95, 90, &[NOT_REPORTED, 41, 48])).unwrap();
        let positions = declared_positions(&report);
        assert_eq!(positions, vec![1, 2]);
        assert_eq!(sensor_name(0), "drive");
        assert_eq!(sensor_name(2), "sensor-2");
        assert_eq!(sensor_label(0), Label::new("storage.temperature"));
        assert_eq!(
            sensor_label(1),
            Label::with_arg("storage.temperatureSensor", "1")
        );
        assert_eq!(
            declared_values(Some(&report), &positions),
            vec![Some(41.0), Some(48.0)]
        );
    }

    #[test]
    fn declared_values_stay_aligned_when_a_refresh_changes() {
        let fewer = parse_temperatures(&descriptor(95, 90, &[47])).unwrap();
        assert_eq!(
            declared_values(Some(&fewer), &[0, 2]),
            vec![Some(47.0), None]
        );
        assert_eq!(declared_values(None, &[0, 2]), vec![None, None]);
    }

    #[test]
    fn sparse_reordered_indices_keep_their_identity() {
        let mut bytes = descriptor(95, 90, &[48, 39]);
        bytes[INFO..INFO + 2].copy_from_slice(&7u16.to_le_bytes());
        bytes[INFO + INFO_SIZE..INFO + INFO_SIZE + 2].copy_from_slice(&0u16.to_le_bytes());
        let report = parse_temperatures(&bytes).unwrap();
        assert_eq!(declared_positions(&report), vec![0, 7]);
        assert_eq!(declared_values(Some(&report), &[0, 7, 1]), vec![Some(39.0), Some(48.0), None]);
        let mut reversed = bytes.clone();
        reversed[INFO..INFO + INFO_SIZE].copy_from_slice(&bytes[INFO + INFO_SIZE..INFO + 2 * INFO_SIZE]);
        reversed[INFO + INFO_SIZE..INFO + 2 * INFO_SIZE].copy_from_slice(&bytes[INFO..INFO + INFO_SIZE]);
        assert_eq!(parse_temperatures(&reversed), Some(report));
        bytes[INFO..INFO + 2].copy_from_slice(&0u16.to_le_bytes());
        assert!(parse_temperatures(&bytes).is_none(), "duplicate sensor identity");
    }

    #[test]
    fn thresholds_become_device_properties() {
        let nvme = parse_temperatures(&descriptor(87, 86, &[47])).unwrap();
        let properties = temperature_properties(Some(&nvme));
        assert_eq!(properties["tempWarningC"], "86");
        assert_eq!(properties["tempCriticalC"], "87");
        let hdd = parse_temperatures(&descriptor(NOT_REPORTED, 60, &[39])).unwrap();
        assert!(!temperature_properties(Some(&hdd)).contains_key("tempCriticalC"));
        assert!(temperature_properties(None).is_empty());
    }

    #[test]
    fn refresh_every_thirty_seconds() {
        let start = Instant::now();
        assert!(!refresh_due(start, start + Duration::from_secs(29)));
        assert!(refresh_due(start, start + TEMPERATURE_PERIOD));
        // A read stamped after `now` is never due.
        assert!(!refresh_due(start + Duration::from_secs(5), start));
    }

    #[test]
    fn one_disk_per_poll_oldest_first() {
        let start = Instant::now();
        let reads = [
            (0, start + Duration::from_secs(2)),
            (2, start),
            (3, start + Duration::from_secs(40)),
        ];
        let now = start + Duration::from_secs(33);
        assert_eq!(next_refresh(reads, now), Some(2));
        assert_eq!(
            next_refresh([(0, start + Duration::from_secs(2))], now),
            Some(0)
        );
        assert_eq!(next_refresh(reads, start + Duration::from_secs(10)), None);
        assert_eq!(next_refresh([], now), None);
        // Equal read times: the lowest disk index first, deterministically.
        assert_eq!(next_refresh([(3, start), (1, start)], now), Some(1));
    }

    #[test]
    fn sleeping_disks_are_not_queried() {
        assert!(may_query(Some(true)));
        assert!(may_query(None));
        assert!(!may_query(Some(false)));
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn reads_disk_temperatures_on_this_machine() {
        let mut with_temperature = 0;
        for index in 0..16 {
            let Some(drive) = PhysicalDrive::open(index) else {
                continue;
            };
            let powered = drive.powered_on();
            if !may_query(powered) {
                println!("disk {index}: spun down, not queried");
                continue;
            }
            let started = Instant::now();
            let report = query_temperatures(&drive);
            let elapsed = started.elapsed();
            println!("disk {index}: powered {powered:?}, {elapsed:?}, {report:?}");
            // A single disk must fit in the 200 ms tick deadline.
            assert!(
                elapsed < Duration::from_millis(200),
                "disk {index}: {elapsed:?}"
            );
            if let Some(Some(celsius)) = report.as_ref().and_then(|r| r.sensors.get(&0)) {
                assert!((5.0..=90.0).contains(celsius), "disk {index}: {celsius} °C");
                with_temperature += 1;
            }
        }
        assert!(with_temperature >= 1, "no disk reports a temperature");
    }
}
```

- [ ] **Step 3: Esegui i test e verifica che falliscano**

```bash
cargo test -p oma-win --lib storage
```

Risultato atteso: la compilazione dei test fallisce con errori `cannot find function …` (`parse_temperatures`, `declared_positions`, `sensor_name`, `refresh_due`, `next_refresh`, `may_query`…) e `cannot find value` per `INFO`, `INFO_SIZE`, `CRITICAL`, `TEMPERATURE_PERIOD`.

- [ ] **Step 4: Implementa `storage_temperature.rs`**

In `crates/oma-win/src/storage_temperature.rs` inserisci, sopra `#[cfg(test)]`:

```rust
//! Disk temperatures from `StorageDeviceTemperatureProperty`: no administrator
//! rights needed; support depends on the drive and its driver (a drive without
//! support answers ERROR_INVALID_FUNCTION and simply has no sensors).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use oma_core::model::Label;
use windows::Win32::System::Ioctl::{
    StorageDeviceTemperatureProperty, STORAGE_TEMPERATURE_DATA_DESCRIPTOR,
    STORAGE_TEMPERATURE_INFO, STORAGE_TEMPERATURE_VALUE_NOT_REPORTED,
};

use crate::storage_ioctl::{le_i16, le_u16, PhysicalDrive};

/// STORAGE_TEMPERATURE_DATA_DESCRIPTOR field offsets.
const CRITICAL: usize = 8;
const WARNING: usize = 10;
const INFO_COUNT: usize = 12;
const INFO: usize = 24;
/// STORAGE_TEMPERATURE_INFO size and `Temperature` offset.
const INFO_SIZE: usize = 16;
const INFO_TEMPERATURE: usize = 2;

const _: () = assert!(
    std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, CriticalTemperature) == CRITICAL
);
const _: () = assert!(
    std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, WarningTemperature) == WARNING
);
const _: () =
    assert!(std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, InfoCount) == INFO_COUNT);
const _: () =
    assert!(std::mem::offset_of!(STORAGE_TEMPERATURE_DATA_DESCRIPTOR, TemperatureInfo) == INFO);
const _: () = assert!(size_of::<STORAGE_TEMPERATURE_INFO>() == INFO_SIZE);
const _: () =
    assert!(std::mem::offset_of!(STORAGE_TEMPERATURE_INFO, Temperature) == INFO_TEMPERATURE);

/// Spec §4.1: disk health data is refreshed every 30 s; the last values are
/// repeated in between.
pub(crate) const TEMPERATURE_PERIOD: Duration = Duration::from_secs(30);

/// One `StorageDeviceTemperatureProperty` answer, in °C.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TemperatureReport {
    /// Driver Index -> value (0 = composite); independent of descriptor order.
    /// `None` = not reported. Duplicate indices invalidate the report.
    pub sensors: BTreeMap<usize, Option<f64>>,
    pub warning_c: Option<i16>,
    pub critical_c: Option<i16>,
}

fn reported(raw: i16) -> Option<i16> {
    (u32::from(raw as u16) != STORAGE_TEMPERATURE_VALUE_NOT_REPORTED).then_some(raw)
}

pub(crate) fn parse_temperatures(bytes: &[u8]) -> Option<TemperatureReport> {
    let count = usize::from(le_u16(bytes, INFO_COUNT)?);
    let mut sensors = BTreeMap::new();
    for i in 0..count {
        let at = INFO + i * INFO_SIZE;
        let Some(record) = bytes.get(at..at + INFO_SIZE) else { break };
        let index = usize::from(le_u16(record, 0)?);
        let value = reported(le_i16(record, INFO_TEMPERATURE)?).map(f64::from);
        if sensors.insert(index, value).is_some() {
            return None;
        }
    }
    let threshold = |offset| le_i16(bytes, offset).and_then(reported).filter(|&t| t > 0);
    Some(TemperatureReport {
        sensors,
        warning_c: threshold(WARNING),
        critical_c: threshold(CRITICAL),
    })
}

/// Queries the drive; `None` if the drive does not support the property.
pub(crate) fn query_temperatures(drive: &PhysicalDrive) -> Option<TemperatureReport> {
    parse_temperatures(&drive.query_property(StorageDeviceTemperatureProperty, 4096)?)
}

/// Driver indices that get a sensor: those reported at discovery, sorted by Index.
pub(crate) fn declared_positions(report: &TemperatureReport) -> Vec<usize> {
    report
        .sensors
        .iter()
        .filter_map(|(&index, value)| value.map(|_| index))
        .collect()
}

/// Sensor id segment: `drive` for driver Index 0, `sensor-<n>` otherwise.
pub(crate) fn sensor_name(position: usize) -> String {
    if position == 0 {
        "drive".to_owned()
    } else {
        format!("sensor-{position}")
    }
}

pub(crate) fn sensor_label(position: usize) -> Label {
    if position == 0 {
        Label::new("storage.temperature")
    } else {
        Label::with_arg("storage.temperatureSensor", position.to_string())
    }
}

/// Values in declared driver-index order; a missing report or index is `None`.
pub(crate) fn declared_values(
    report: Option<&TemperatureReport>,
    positions: &[usize],
) -> Vec<Option<f64>> {
    positions
        .iter()
        .map(|&p| report.and_then(|r| r.sensors.get(&p).copied().flatten()))
        .collect()
}

/// Device properties `tempWarningC` / `tempCriticalC`, when the drive reports them.
pub(crate) fn temperature_properties(
    report: Option<&TemperatureReport>,
) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    if let Some(report) = report {
        if let Some(warning) = report.warning_c {
            properties.insert("tempWarningC".to_owned(), warning.to_string());
        }
        if let Some(critical) = report.critical_c {
            properties.insert("tempCriticalC".to_owned(), critical.to_string());
        }
    }
    properties
}

/// True when a read is at least `TEMPERATURE_PERIOD` old.
pub(crate) fn refresh_due(read_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(read_at) >= TEMPERATURE_PERIOD
}

/// The disk to refresh on this poll: the one with the oldest due read. One
/// disk per poll bounds the cost of a tick (an HDD answers in about 20 ms, an
/// NVMe drive leaving a low-power state in up to about 140 ms).
pub(crate) fn next_refresh(
    reads: impl IntoIterator<Item = (u32, Instant)>,
    now: Instant,
) -> Option<u32> {
    reads
        .into_iter()
        .filter(|&(_, read_at)| refresh_due(read_at, now))
        .min_by_key(|&(index, read_at)| (read_at, index))
        .map(|(index, _)| index)
}

/// A disk known to be spun down is not queried: the query could wake it up.
pub(crate) fn may_query(powered_on: Option<bool>) -> bool {
    powered_on != Some(false)
}

```

- [ ] **Step 5: Dichiara e campiona le temperature in `storage.rs`**

In `crates/oma-win/src/storage.rs`:

1. sostituisci la prima riga

```rust
//! Physical disk throughput and activity (PDH) plus volume usage.
```

con

```rust
//! Physical disk throughput and activity (PDH), disk temperatures and volume usage.
```

2. sostituisci

```rust
use std::collections::{BTreeMap, HashMap};
```

con

```rust
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;
```

3. subito dopo il blocco `use crate::storage_identity::{ … };` aggiungi

```rust
use crate::storage_ioctl::PhysicalDrive;
use crate::storage_temperature::{
    declared_positions, declared_values, may_query, next_refresh, query_temperatures, sensor_label,
    sensor_name, temperature_properties, TemperatureReport,
};
```

4. subito prima di `struct Counters {` aggiungi

```rust
/// Temperature sensors of one disk: the driver indices declared at
/// discovery, their latest values (repeated between refreshes) and when they
/// were read.
struct DiskTemperatures {
    positions: Vec<usize>,
    values: Vec<Option<f64>>,
    read_at: Instant,
}

impl DiskTemperatures {
    /// Refreshes values and the attempt deadline, including failed/asleep reads.
    /// New driver indices require a schema rebuild, never a value-vector resize.
    fn refresh(&mut self, report: Option<&TemperatureReport>, now: Instant) -> bool {
        self.values = declared_values(report, &self.positions);
        self.read_at = now;
        report.is_some_and(|r| {
            declared_positions(r).iter().any(|i| !self.positions.contains(i))
        })
    }
}

/// Temperatures of disk `index`. A disk known to be spun down is not queried,
/// because the query could wake it up: `None`, as for an unsupported disk.
fn read_temperatures(index: u32) -> Option<TemperatureReport> {
    PhysicalDrive::open(index)
        .filter(|drive| may_query(drive.powered_on()))
        .and_then(|drive| query_temperatures(&drive))
}

```

5. in `pub struct StorageProvider`, sostituisci

```rust
    volume_ids: HashMap<String, String>,
    /// Set by `discover`; consumed by the next `poll`. See `take_fresh`.
```

con

```rust
    volume_ids: HashMap<String, String>,
    /// Every identified disk, including those waiting for temperature support/wake.
    temperatures: HashMap<u32, DiskTemperatures>,
    /// Set by `discover`; consumed by the next `poll`. See `take_fresh`.
```

6. in `discover`, sostituisci

```rust
        let mut devices = Vec::new();
        let mut sensors = Vec::new();
        for disk in &disks {
```

con

```rust
        let mut devices = Vec::new();
        let mut sensors = Vec::new();
        let mut temperatures = HashMap::new();
        for disk in &disks {
```

7. sempre in `discover`, sostituisci

```rust
            devices.push(Device {
                id: id.clone(),
                kind: DeviceKind::Storage,
                name: disk_name(disk),
                vendor: None,
                properties: Default::default(),
            });
```

con

```rust
            // Unknown/asleep disks remain scheduled; a later successful probe
            // requests rediscovery when it reveals undeclared sensor indices.
            let report = read_temperatures(disk.index);
            devices.push(Device {
                id: id.clone(),
                kind: DeviceKind::Storage,
                name: disk_name(disk),
                vendor: None,
                properties: temperature_properties(report.as_ref()),
            });
```

8. sempre in `discover`, sostituisci

```rust
                Label::new("storage.active"),
                Source::Pdh,
            ));
            for volume in &disk.volumes {
```

con

```rust
                Label::new("storage.active"),
                Source::Pdh,
            ));
            let positions = report.as_ref().map(declared_positions).unwrap_or_default();
            for &position in &positions {
                sensors.push(Sensor::new(
                    &id,
                    SensorKind::Temperature,
                    &sensor_name(position),
                    Unit::Celsius,
                    sensor_label(position),
                    Source::Win32,
                ));
            }
            temperatures.insert(
                disk.index,
                DiskTemperatures {
                    values: declared_values(report.as_ref(), &positions),
                    positions,
                    read_at: Instant::now(),
                },
            );
            for volume in &disk.volumes {
```

9. alla fine di `discover`, sostituisci

```rust
        self.volume_ids = volume_ids;
        self.fresh = true;
```

con

```rust
        self.volume_ids = volume_ids;
        self.temperatures = temperatures;
        self.fresh = true;
```

10. in `poll`, sostituisci

```rust
        let finite =
            |map: &HashMap<String, f64>, key: &str| map.get(key).copied().filter(|v| v.is_finite());
```

con

```rust
        let finite =
            |map: &HashMap<String, f64>, key: &str| map.get(key).copied().filter(|v| v.is_finite());
        let reads = self.temperatures.iter().map(|(&i, t)| (i, t.read_at));
        if let Some(index) = next_refresh(reads, Instant::now()) {
            if let Some(disk) = self.temperatures.get_mut(&index) {
                let report = read_temperatures(index);
                if disk.refresh(report.as_ref(), Instant::now()) {
                    return Err(ProviderError::Rediscover);
                }
            }
        }
```

11. sempre in `poll`, sostituisci

```rust
                values.push(finite(&idle, &disk.instance).and_then(active_pct));
            }
            for volume in &disk.volumes {
```

con

```rust
                values.push(finite(&idle, &disk.instance).and_then(active_pct));
            }
            // Not a rate: the last read is valid on the first poll too.
            if let Some(temperatures) = self.temperatures.get(&disk.index) {
                values.extend(temperatures.values.iter().copied());
            }
            for volume in &disk.volumes {
```

`StorageProvider` deriva `Default`: `HashMap` lo implementa, quindi non serve altro.

12. nel modulo `tests` di `storage.rs`, aggiungi il test puro del recupero dopo standby o errore transitorio:

```rust
    #[test]
    fn sleeping_disk_is_retried_and_new_indices_request_discovery() {
        use std::time::Duration;
        use crate::storage_temperature::TEMPERATURE_PERIOD;
        let start = Instant::now();
        let mut disk = DiskTemperatures {
            positions: vec![], values: vec![], read_at: start,
        };
        let first = start + TEMPERATURE_PERIOD;
        assert_eq!(next_refresh([(0, disk.read_at)], first), Some(0));
        assert!(!disk.refresh(None, first)); // still asleep / transient failure
        assert_eq!(next_refresh([(0, disk.read_at)], first + Duration::from_secs(1)), None);
        let awake = TemperatureReport {
            sensors: BTreeMap::from([(0, Some(42.0))]),
            warning_c: None, critical_c: None,
        };
        assert_eq!(next_refresh([(0, disk.read_at)], first + TEMPERATURE_PERIOD), Some(0));
        assert!(disk.refresh(Some(&awake), first + TEMPERATURE_PERIOD));
        assert!(disk.values.is_empty(), "schema changes only in discover");
        disk.positions = vec![0]; // subsequent discovery declares the new sensor
        assert!(!disk.refresh(Some(&awake), first + TEMPERATURE_PERIOD));
        assert_eq!(disk.values, vec![Some(42.0)]);
        assert!(!disk.refresh(None, first + TEMPERATURE_PERIOD));
        assert_eq!(disk.values, vec![None]);
    }
```

- [ ] **Step 6: Traduzioni ed elenco delle chiavi**

In `app/src/lib/i18n/en.json`, dopo `"sensor.storage.volumeFree": "Volume {arg} free",` aggiungi:

```json
  "sensor.storage.temperature": "Temperature",
  "sensor.storage.temperatureSensor": "Temperature sensor {arg}",
```

In `app/src/lib/i18n/it.json`, dopo `"sensor.storage.volumeFree": "Volume {arg} libero",` aggiungi:

```json
  "sensor.storage.temperature": "Temperatura",
  "sensor.storage.temperatureSensor": "Sensore di temperatura {arg}",
```

In `crates/oma-win/tests/labels.rs`, nell'array `KEYS`, dopo `"storage.volumeFree",` aggiungi:

```rust
    "storage.temperature",
    "storage.temperatureSensor",
```

- [ ] **Step 7: Il test hardware del provider controlla le temperature**

In `crates/oma-win/tests/providers.rs`:

1. sostituisci `use oma_core::model::{Sensor, Source};` con

```rust
use oma_core::model::{Sensor, SensorKind, Source, Unit};
```

2. sostituisci l'intera funzione `storage_provider_reports_disks_and_volumes` (attributi compresi) con:

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
        if sensor.kind == SensorKind::Temperature {
            assert_eq!(sensor.unit, Unit::Celsius, "{}", sensor.id);
            // Read at discovery and repeated until the 30 s refresh.
            let celsius = value.expect("disk temperature");
            assert!((5.0..=90.0).contains(&celsius), "{} = {celsius}", sensor.id);
        }
    }
    let temperatures = inventory
        .sensors
        .iter()
        .filter(|s| s.kind == SensorKind::Temperature)
        .count();
    println!("{temperatures} disk temperature sensors");
    assert!(temperatures > 0, "at least one disk reports a temperature");
    for device in &inventory.devices {
        for key in ["tempWarningC", "tempCriticalC"] {
            if let Some(value) = device.properties.get(key) {
                let celsius: i16 = value.parse().expect("integer °C");
                assert!(
                    (40..=150).contains(&celsius),
                    "{} {key} = {celsius}",
                    device.id
                );
            }
        }
    }
}
```

- [ ] **Step 8: Esegui i test e verifica che passino**

```bash
cargo fmt --all
cargo test -p oma-win --lib storage
cargo test -p oma-win --test labels
cd app && pnpm test src/lib/i18n && cd ..
```

Risultato atteso:
- `cargo test -p oma-win --lib storage`: `test result: ok. 38 passed; 0 failed; 2 ignored` (11 `storage`, 15 `storage_identity`, 2 `storage_ioctl`, 10 `storage_temperature`; ignorati i due test hardware);
- `labels`: `1 passed`;
- Vitest: il test di parità delle chiavi en/it passa.

- [ ] **Step 9: Verifica su hardware reale**

```bash
cargo test -p oma-win --lib storage_temperature -- --include-ignored --nocapture
cargo test -p oma-win --test providers -- --ignored storage --nocapture
```

Risultato atteso sulla macchina di sviluppo (i gradi variano di qualche unità):
- il primo comando stampa per ogni disco lo stato di alimentazione, il tempo e il resoconto, per esempio `disk 0: powered Some(true), 1.5ms, Some(TemperatureReport { sensors: {0: Some(39.0)}, warning_c: Some(60), critical_c: None })`, `disk 1: powered Some(true), 27µs, None`, `disk 2: … sensors: {0: Some(48.0), 1: Some(48.0), 2: Some(40.0)}, warning_c: Some(90), critical_c: Some(95)`, `disk 3: … warning_c: Some(86), critical_c: Some(87)`; tutti i test passano, compreso quello degli indici sparsi e riordinati. Se l'HDD è in standby, la riga è `disk 0: spun down, not queried` e il test passa lo stesso;
- il secondo comando stampa `7 disk temperature sensors` (1 dell'HDD, 3 per ciascun NVMe, nessuno per l'SSD SATA) e termina con `1 passed`. Con l'HDD in standby all'avvio del test sono 6.

- [ ] **Step 10: Lint e commit**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/oma-win/Cargo.toml crates/oma-win/src/lib.rs crates/oma-win/src/storage_ioctl.rs crates/oma-win/src/storage_temperature.rs crates/oma-win/src/storage.rs crates/oma-win/tests/labels.rs crates/oma-win/tests/providers.rs app/src/lib/i18n/en.json app/src/lib/i18n/it.json
git commit -m "feat(win): disk temperatures and thresholds from StorageDeviceTemperatureProperty"
```

---

---

### Task 6: GPU: unità e categoria del link PCIe, campi encoder/decoder e link, proprietà statiche dai layer (PnP, NVML), indirizzo PCI per LUID

**File:**
- Modifica: `crates/oma-core/src/model.rs` (`SensorKind::Link`, `Unit::PcieGeneration`, `Unit::Lanes`, `Source::Pnp`)
- Modifica: `crates/oma-core/src/sanitize.rs` (intervalli plausibili delle due nuove unità)
- Modifica: `crates/oma-win/Cargo.toml` (feature windows-rs `Win32_Devices_Properties`; `Win32_Devices_DeviceAndDriverInstallation` c'è già dal Task 4)
- Modifica: `crates/oma-win/src/gpu/field.rs` (4 campi nuovi, `ALL` da 22 a 26)
- Modifica: `crates/oma-win/src/gpu/layer.rs` (metodo fornito `GpuLayer::properties`)
- Modifica: `crates/oma-win/src/gpu/mod.rs` (fusione delle proprietà, `PnpLayer` tra i layer base, indirizzo PCI conservato per LUID)
- Crea: `crates/oma-win/src/gpu/pnp.rs` (`PnpLayer`)
- Modifica: `crates/oma-win/src/gpu/nvml.rs` (encoder/decoder, link attuale, proprietà statiche)
- Modifica: `crates/oma-win/tests/labels.rs`, `app/src/lib/i18n/en.json`, `app/src/lib/i18n/it.json`
- Test: moduli `#[cfg(test)]` di `model.rs`, `sanitize.rs`, `field.rs`, `mod.rs`, `pnp.rs`, `nvml.rs`; test hardware `#[ignore]` in `pnp.rs`, `nvml.rs` e `crates/oma-win/tests/providers.rs`

**Interfacce:**
- Usa (M2): `GpuLayer`, `Readings`, `GpuField`, `Adapter` (`luid`, `pci`, `vendor_id`), `PciAddress`, `GpuProvider::with_layers`, `super::enumerate::{enumerate, luid_to_u64}`, `oma_core::merge::assign`, le funzioni `call_u32`/`call_u32_arg`/`probe`/`collect` di `nvml.rs`.
- Produce (oma-core):
  ```rust
  pub enum SensorKind { /* … */ Link }            // as_str() == "link", serde "link"
  pub enum Unit { /* … */ PcieGeneration, Lanes } // serde "pcie_generation", "lanes"
  pub enum Source { /* … */ Pnp }                 // serde "pnp"
  // sanitize: PcieGeneration accettata solo se intera e in 1..=7; Lanes in 1..=32
  ```
- Produce (oma-win, `pub(crate)`):
  ```rust
  // field.rs — ordine di ALL: LoadEncoder, LoadDecoder subito dopo LoadVideoEncode; PcieLinkGen, PcieLinkWidth in coda
  GpuField::LoadEncoder   // Load, "encoder",    Percent,        "gpu.load.encoder"
  GpuField::LoadDecoder   // Load, "decoder",    Percent,        "gpu.load.decoder"
  GpuField::PcieLinkGen   // Link, "pcie-gen",   PcieGeneration, "gpu.pcie.gen"
  GpuField::PcieLinkWidth // Link, "pcie-width", Lanes,          "gpu.pcie.width"
  pub const ALL: [GpuField; 26];

  // layer.rs
  fn properties(&self, adapter: usize) -> BTreeMap<String, String> { BTreeMap::new() } // metodo fornito

  // pnp.rs
  pub(crate) fn link_properties(max_speed: Option<u32>, max_width: Option<u32>) -> BTreeMap<String, String>;
  pub(crate) fn split_multi_sz(list: &[u16]) -> Vec<Vec<u16>>;
  #[derive(Default)] pub(crate) struct PnpLayer; // impl GpuLayer, source Source::Pnp, nessun campo

  // mod.rs (private)
  fn restore_pci(known: &mut HashMap<u64, PciAddress>, adapters: &mut [Adapter]);
  fn merge_properties(device: &mut BTreeMap<String, String>, layers: impl IntoIterator<Item = BTreeMap<String, String>>);
  ```
- Chiavi delle proprietà (valori: stringhe decimali semplici): `pcieMaxGen`, `pcieMaxWidth`, `powerLimitMinW`, `powerLimitMaxW`, `powerLimitDefaultW`, `tempSlowdownC`, `tempShutdownC`, `tempMaxC`. `pciAddress` e `integrated` restano del provider e nessun layer può sovrascriverle.
- Non tocca `app/src/lib/types.ts`: i valori `'link'`, `'pcie_generation'`, `'lanes'` e `'pnp'` entrano nei tipi TypeScript nel Task 9. Il frontend continua a compilare perché riceve stringhe che per ora non interpreta.

Fatti verificati su questa macchina (spike `gpuspike-m3` e verifica del codice di questo task) e regole fissate qui:
- **Link attuale (decisione D7).** Solo NVML lo dà dal vivo: `nvmlDeviceGetCurrPcieLinkGeneration` / `…Width` costano 0,1 µs. Sulla RTX 4080 la generazione è 1 a riposo (ASPM, non è un guasto), sale a 4 sotto carico e passa per 2 mentre scende; la larghezza resta 16. Le proprietà PnP "CurrentLinkSpeed/Width" sono una fotografia presa all'avvio del dispositivo: leggono sempre 4 mentre NVML legge 1. Per questo non alimentano mai un sensore. I sensori ADL 40/41 sono esclusi (valori costanti 3/16, unità non documentata).
- **Link massimo (decisione D7).** È una proprietà del dispositivo. Il PnP la dà per qualunque fornitore, senza privilegi: set di proprietà PCI `{3AB22E31-8264-4B4E-9AF5-A8D2D8E33E62}`, pid 11 `MaxLinkSpeed` (numero di generazione, 4 = 16 GT/s) e pid 12 `MaxLinkWidth` (corsie), entrambi `DEVPROP_TYPE_UINT32`. Valori qui: RTX `PCI\VEN_10DE&DEV_2704…` Gen 4 x16, Raphael `PCI\VEN_1002&DEV_164E…` Gen 4 x16; il Basic Render (`ROOT\BasicRender`) non ha chiavi PCI. NVML (`MaxPcieLinkGeneration` = 4, `MaxPcieLinkWidth` = 16) dà gli stessi valori e, avendo priorità più alta, vince.
- **Da adattatore a nodo PnP (metodo A dello spike).** `CM_Get_Device_Interface_ListW(GUID_DISPLAY_DEVICE_ARRIVAL {1CA05180-A699-450A-9A0C-DE4FBE3DDD89}, PRESENT)`. Poi, per ogni percorso d'interfaccia: `D3DKMTOpenAdapterFromDeviceName` (dà il LUID; l'handle si chiude subito), `CM_Get_Device_Interface_PropertyW(DEVPKEY_Device_InstanceId)` e infine `CM_Locate_DevNodeW`. La corrispondenza col LUID è esatta. Costo misurato: 62 µs per tutta la lista più circa 9 µs per proprietà, una sola volta per discovery. `DEVPKEY_Device_LocationInfo` è localizzata ("Bus PCI 1, dispositivo 0, funzione 0"): non va mai analizzata. In windows-rs 0.62 `DEVPKEY_PciDevice_*` esiste solo sotto `NetworkManagement_WiFi`, quindi le chiavi e il GUID si definiscono nel nostro codice.
- **NVML (decisioni D8, D9).** Firme (nostre dichiarazioni, cdecl):
  - `nvmlDeviceGetEncoderUtilization` / `nvmlDeviceGetDecoderUtilization`: `(dev, u32* util_pct, u32* sampling_period_us)`, 0,1 µs. Il valore è la media dei motori NVENC/NVDEC: 49 % con ffmpeg `hevc_nvenc`, mentre PDH mostrava 50,1 + 49,0 sui due motori.
  - `nvmlDeviceGetMaxPcieLinkGeneration` / `…Width`, `nvmlDeviceGetPowerManagementDefaultLimit`: `(dev, u32*)`.
  - `nvmlDeviceGetPowerManagementLimitConstraints`: `(dev, u32* min_mW, u32* max_mW)`.
  - `nvmlDeviceGetTemperatureThreshold`: `(dev, u32 kind, u32* °C)`, con kind 0 = SHUTDOWN, 1 = SLOWDOWN, 3 = GPU_MAX.

  Valori della RTX 4080 (driver 617.14): vincoli 150 000 / 370 000 mW, limite predefinito 320 000 mW, soglie SLOWDOWN 94 °C, SHUTDOWN 99 °C, GPU_MAX 90 °C. Da qui le proprietà `powerLimitMinW` 150, `powerLimitMaxW` 370, `powerLimitDefaultW` 320, `tempSlowdownC` 94, `tempShutdownC` 99, `tempMaxC` 90. I valori statici si leggono una sola volta, in `attach`. Una chiamata fallita o un valore 0 lasciano fuori la chiave. Non si usano mai `PcieThroughput` (blocca per circa 31 ms), `TotalEnergyConsumption` (p95 circa 9 ms) né gli elenchi dei processi in esecuzione (su WDDM `usedGpuMemory` è sempre N/A): vedi D10.
- **`LoadEncoder`/`LoadDecoder` (decisione D8)** sono campi solo NVML, distinti da `LoadVideoEncode`/`LoadVideoDecode`: questi ultimi restano l'aggregazione PDH per motore (§5.2).
- **Priorità invariata:** NVML → NVAPI → ADL → IGCL → D3DKMT → DXGI → PDH → PnP. Il PnP dà solo proprietà e non dichiara campi. Per ogni chiave vince il primo layer attivo che la fornisce: in modalità sicura i layer dei fornitori non sono attivi, quindi restano solo le proprietà PnP.
- **Indirizzo PCI per LUID (seguito M2).** Se un'enumerazione successiva perde il `pci` di un adattatore con lo stesso LUID (la query D3DKMT ADAPTERADDRESS fallita una volta), il provider rimette l'indirizzo già noto. Senza questa regola l'id del dispositivo passerebbe da `gpu/pci-…` a `gpu/ven-…` e la cronologia andrebbe persa. La regola vale sia nel `discover` sia nel controllo di topologia di `poll`, così un indirizzo perso per un solo giro non causa una rediscovery.

- [ ] **Step 1: Scrivi i test del modello e della sanitizzazione (falliscono)**

In `crates/oma-core/src/model.rs`, nel modulo `tests`, subito prima di `fn experimental_is_serialized_only_when_true`, aggiungi:

```rust
    #[test]
    fn link_kind_units_and_pnp_source_serialize_in_snake_case() {
        assert_eq!(SensorKind::Link.as_str(), "link");
        assert_eq!(
            serde_json::to_value(SensorKind::Link).unwrap(),
            json!("link")
        );
        assert_eq!(
            serde_json::to_value([Unit::PcieGeneration, Unit::Lanes]).unwrap(),
            json!(["pcie_generation", "lanes"])
        );
        assert_eq!(serde_json::to_value(Source::Pnp).unwrap(), json!("pnp"));
    }
```

In `crates/oma-core/src/sanitize.rs`, nel modulo `tests`, subito prima di `fn power_ratio_can_exceed_100_but_utilization_cannot`, aggiungi:

```rust
    #[test]
    fn pcie_generation_is_a_whole_number_from_1_to_7() {
        assert_eq!(sanitize(Unit::PcieGeneration, Some(1.0)), Some(1.0));
        assert_eq!(sanitize(Unit::PcieGeneration, Some(4.0)), Some(4.0));
        assert_eq!(sanitize(Unit::PcieGeneration, Some(7.0)), Some(7.0));
        assert_eq!(sanitize(Unit::PcieGeneration, Some(0.0)), None);
        assert_eq!(sanitize(Unit::PcieGeneration, Some(8.0)), None);
        assert_eq!(sanitize(Unit::PcieGeneration, Some(3.5)), None);
    }

    #[test]
    fn lanes_range_from_1_to_32() {
        assert_eq!(sanitize(Unit::Lanes, Some(1.0)), Some(1.0));
        assert_eq!(sanitize(Unit::Lanes, Some(16.0)), Some(16.0));
        assert_eq!(sanitize(Unit::Lanes, Some(32.0)), Some(32.0));
        assert_eq!(sanitize(Unit::Lanes, Some(0.0)), None);
        assert_eq!(sanitize(Unit::Lanes, Some(64.0)), None);
    }
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-core`
Risultato atteso: errori di compilazione `error[E0599]: no variant or associated item named `Link` found for enum `SensorKind`` (e lo stesso per `PcieGeneration`, `Lanes`, `Pnp`).

- [ ] **Step 3: Aggiungi le varianti e gli intervalli**

In `crates/oma-core/src/model.rs`:

Nell'enum `SensorKind`, dopo `Percent,`:

```rust
    Percent,
    /// A bus link, e.g. the PCIe link of a GPU.
    Link,
}
```

In `SensorKind::as_str`, dopo il ramo `SensorKind::Percent => "percent",`:

```rust
            SensorKind::Percent => "percent",
            SensorKind::Link => "link",
```

Nell'enum `Unit`, dopo `Boolean,`:

```rust
    Boolean,
    /// PCIe link generation (1 = 2.5 GT/s ... 5 = 32 GT/s).
    PcieGeneration,
    /// Number of active link lanes.
    Lanes,
}
```

Nell'enum `Source`, tra `Igcl,` e `Mock,`:

```rust
    Igcl,
    /// Windows Plug and Play device properties (cfgmgr32).
    Pnp,
    Mock,
}
```

In `crates/oma-core/src/sanitize.rs`, dentro il blocco `match unit { … }` di `sanitize` (il Task 2 riscrive il resto della funzione, ma lascia intatto questo `match`), aggiungi i due rami subito dopo il ramo `Unit::Boolean`:

```rust
        Unit::Boolean => v == 0.0 || v == 1.0,
        Unit::PcieGeneration => (1.0..=7.0).contains(&v) && v.fract() == 0.0,
        Unit::Lanes => (1.0..=32.0).contains(&v),
```

(Il ramo finale `_ => v >= 0.0,` resta per ultimo.)

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-core`
Risultato atteso: tutti OK, compresi `link_kind_units_and_pnp_source_serialize_in_snake_case`, `pcie_generation_is_a_whole_number_from_1_to_7` e `lanes_range_from_1_to_32`.

- [ ] **Step 5: Scrivi il test dei quattro campi GPU nuovi (fallisce)**

In `crates/oma-win/src/gpu/field.rs`, nel modulo `tests`, subito prima di `fn sensor_id_matches_the_spec_example`, aggiungi:

```rust
    #[test]
    fn m3_fields_match_the_contract() {
        let row = |f: GpuField| (f.kind(), f.name(), f.unit(), f.label_key());
        assert_eq!(
            row(GpuField::LoadEncoder),
            (
                SensorKind::Load,
                "encoder",
                Unit::Percent,
                "gpu.load.encoder"
            )
        );
        assert_eq!(
            row(GpuField::LoadDecoder),
            (
                SensorKind::Load,
                "decoder",
                Unit::Percent,
                "gpu.load.decoder"
            )
        );
        assert_eq!(
            row(GpuField::PcieLinkGen),
            (
                SensorKind::Link,
                "pcie-gen",
                Unit::PcieGeneration,
                "gpu.pcie.gen"
            )
        );
        assert_eq!(
            row(GpuField::PcieLinkWidth),
            (
                SensorKind::Link,
                "pcie-width",
                Unit::Lanes,
                "gpu.pcie.width"
            )
        );
        // Encoder/decoder follow the PDH per-engine loads; the link comes last.
        let position = |f: GpuField| GpuField::ALL.iter().position(|&x| x == f).unwrap();
        assert_eq!(position(GpuField::LoadEncoder), 6);
        assert_eq!(position(GpuField::LoadDecoder), 7);
        assert_eq!(position(GpuField::PcieLinkGen), 24);
        assert_eq!(position(GpuField::PcieLinkWidth), 25);
    }
```

Esegui: `cargo test -p oma-win --lib gpu::field`
Risultato atteso: errori di compilazione `no variant or associated item named `LoadEncoder` found for enum `GpuField`` (e lo stesso per gli altri tre).

- [ ] **Step 6: Aggiungi i campi, le etichette e le traduzioni**

In `crates/oma-win/src/gpu/field.rs`:

Nell'enum `GpuField`, subito dopo `LoadVideoEncode,`:

```rust
    LoadVideoEncode,
    /// NVENC utilization as NVML reports it (average of the encoder engines).
    LoadEncoder,
    /// NVDEC utilization as NVML reports it (average of the decoder engines).
    LoadDecoder,
    MemoryDedicatedUsed,
```

e in coda, dopo `ThrottleThermal,`:

```rust
    ThrottleThermal,
    /// Current PCIe link generation (live: drops to Gen 1 at idle with ASPM).
    PcieLinkGen,
    /// Current PCIe link width in lanes.
    PcieLinkWidth,
}
```

`ALL` passa da 22 a 26 elementi: sostituisci `pub const ALL: [GpuField; 22] = [` con `pub const ALL: [GpuField; 26] = [`, inserisci `GpuField::LoadEncoder,` e `GpuField::LoadDecoder,` subito dopo `GpuField::LoadVideoEncode,`, e aggiungi in coda, dopo `GpuField::ThrottleThermal,`:

```rust
        GpuField::ThrottleThermal,
        GpuField::PcieLinkGen,
        GpuField::PcieLinkWidth,
    ];
```

In `kind()`, sostituisci il primo ramo e aggiungi l'ultimo:

```rust
            LoadCore | Load3d | LoadCompute | LoadCopy | LoadVideoDecode | LoadVideoEncode
            | LoadEncoder | LoadDecoder => SensorKind::Load,
```

```rust
            ThrottlePower | ThrottleThermal => SensorKind::Flag,
            PcieLinkGen | PcieLinkWidth => SensorKind::Link,
```

In `name()`, dopo `LoadVideoEncode => "video-encode",` e dopo `ThrottleThermal => "throttle-thermal",`:

```rust
            LoadVideoEncode => "video-encode",
            LoadEncoder => "encoder",
            LoadDecoder => "decoder",
```

```rust
            ThrottleThermal => "throttle-thermal",
            PcieLinkGen => "pcie-gen",
            PcieLinkWidth => "pcie-width",
```

In `unit()`, sostituisci il ramo delle percentuali e aggiungi i due rami del link:

```rust
            LoadCore | Load3d | LoadCompute | LoadCopy | LoadVideoDecode | LoadVideoEncode
            | LoadEncoder | LoadDecoder | PowerLimitPercent | FanPercent => Unit::Percent,
```

```rust
            ThrottlePower | ThrottleThermal => Unit::Boolean,
            PcieLinkGen => Unit::PcieGeneration,
            PcieLinkWidth => Unit::Lanes,
```

In `label_key()`, dopo `LoadVideoEncode => "gpu.load.videoEncode",` e dopo `ThrottleThermal => "gpu.throttle.thermal",`:

```rust
            LoadVideoEncode => "gpu.load.videoEncode",
            LoadEncoder => "gpu.load.encoder",
            LoadDecoder => "gpu.load.decoder",
```

```rust
            ThrottleThermal => "gpu.throttle.thermal",
            PcieLinkGen => "gpu.pcie.gen",
            PcieLinkWidth => "gpu.pcie.width",
```

(Gli altri layer non hanno `match` esaustivi su `GpuField`: `nvml.rs` e `igcl.rs` terminano con `_ =>`, quindi compilano senza modifiche.)

In `crates/oma-win/tests/labels.rs`, nell'array `KEYS`, dopo `"gpu.load.videoEncode",` e dopo `"gpu.throttle.thermal",`:

```rust
    "gpu.load.videoEncode",
    "gpu.load.encoder",
    "gpu.load.decoder",
```

```rust
    "gpu.throttle.thermal",
    "gpu.pcie.gen",
    "gpu.pcie.width",
];
```

In `app/src/lib/i18n/en.json`, dopo la riga `"sensor.gpu.load.videoEncode": "Video encode load",` e dopo la riga `"sensor.gpu.voltage.core": "Core voltage",`:

```json
  "sensor.gpu.load.videoEncode": "Video encode load",
  "sensor.gpu.load.encoder": "Encoder load",
  "sensor.gpu.load.decoder": "Decoder load",
```

```json
  "sensor.gpu.voltage.core": "Core voltage",
  "sensor.gpu.pcie.gen": "PCIe link generation",
  "sensor.gpu.pcie.width": "PCIe link width",
```

In `app/src/lib/i18n/it.json`, negli stessi punti:

```json
  "sensor.gpu.load.videoEncode": "Carico codifica video",
  "sensor.gpu.load.encoder": "Carico encoder",
  "sensor.gpu.load.decoder": "Carico decoder",
```

```json
  "sensor.gpu.voltage.core": "Tensione core",
  "sensor.gpu.pcie.gen": "Generazione link PCIe",
  "sensor.gpu.pcie.width": "Larghezza link PCIe",
```

Esegui: `cargo test -p oma-win --lib gpu::field` e `cargo test -p oma-win --test labels`
Risultato atteso: tutti OK, compresi `m3_fields_match_the_contract`, `all_follows_declaration_order_without_duplicates` e `every_label_key_is_checked_by_the_labels_test`; `every_provider_label_key_has_a_translation` passa.

- [ ] **Step 7: Scrivi i test della fusione delle proprietà e dell'indirizzo PCI per LUID (falliscono)**

In `crates/oma-win/src/gpu/mod.rs`, modulo `tests`:

Aggiungi il campo in fondo alla struct `Script`:

```rust
        attach_calls: usize,
        sample_calls: usize,
        /// Per adapter: the static properties the layer reports.
        properties: Vec<BTreeMap<String, String>>,
    }
```

Nell'`impl GpuLayer for FakeLayer`, dopo `is_experimental`:

```rust
        fn properties(&self, adapter: usize) -> BTreeMap<String, String> {
            let s = self.script.lock().unwrap();
            s.properties.get(adapter).cloned().unwrap_or_default()
        }
```

E subito prima di `fn topology_changes_are_detected_without_live_handles`:

```rust
    fn props(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn layer_properties_are_merged_by_priority() {
        let (nvml, nvml_script) = fake(Source::Nvml, &[&[], &[]]);
        let (pnp, pnp_script) = fake(Source::Pnp, &[&[], &[]]);
        nvml_script.lock().unwrap().properties = vec![props(&[
            ("pcieMaxGen", "4"),
            ("powerLimitDefaultW", "320"),
            ("pciAddress", "9999:99:99.9"),
        ])];
        pnp_script.lock().unwrap().properties = vec![
            props(&[("pcieMaxGen", "3"), ("pcieMaxWidth", "16")]),
            props(&[("pcieMaxGen", "4"), ("pcieMaxWidth", "16")]),
        ];
        let (mut p, _) = provider(
            vec![nvidia(), amd_igpu()],
            vec![pnp],
            vec![nvml],
            &VendorSwitch::new(true),
        );
        let inventory = p.discover().unwrap();
        let nv = &inventory.devices[0].properties;
        assert_eq!(nv["pcieMaxGen"], "4", "NVML outranks PnP");
        assert_eq!(nv["pcieMaxWidth"], "16", "PnP fills the gap");
        assert_eq!(nv["powerLimitDefaultW"], "320");
        assert_eq!(nv["pciAddress"], "0000:01:00.0", "not overridable");
        assert_eq!(nv["integrated"], "false");
        let amd = &inventory.devices[1].properties;
        assert_eq!(
            amd,
            &props(&[
                ("integrated", "true"),
                ("pciAddress", "0000:11:00.0"),
                ("pcieMaxGen", "4"),
                ("pcieMaxWidth", "16"),
            ])
        );
        assert!(inventory.sensors.is_empty(), "properties declare no sensor");
    }

    #[test]
    fn safe_mode_keeps_base_layer_properties_only() {
        let (nvml, nvml_script) = fake(Source::Nvml, &[&[]]);
        let (pnp, pnp_script) = fake(Source::Pnp, &[&[]]);
        nvml_script.lock().unwrap().properties = vec![props(&[("tempMaxC", "90")])];
        pnp_script.lock().unwrap().properties = vec![props(&[("pcieMaxGen", "4")])];
        let (mut p, _) = provider(
            vec![nvidia()],
            vec![pnp],
            vec![nvml],
            &VendorSwitch::new(false),
        );
        let device = &p.discover().unwrap().devices[0];
        assert_eq!(device.properties["pcieMaxGen"], "4");
        assert!(!device.properties.contains_key("tempMaxC"));
    }

    #[test]
    fn pci_address_is_kept_per_luid_when_the_kernel_query_fails() {
        let topology = Arc::new(Mutex::new(vec![nvidia()]));
        let enumerated = topology.clone();
        let mut gpu = GpuProvider::with_layers(
            Box::new(move || Ok(enumerated.lock().unwrap().clone())),
            vec![],
            Box::new(Vec::new),
            VendorSwitch::new(false),
        );
        assert_eq!(
            gpu.discover().unwrap().devices[0].id,
            "gpu/pci-0000:01:00.0"
        );

        // The next enumeration loses the PCI address of the same adapter (same LUID).
        topology.lock().unwrap()[0].pci = None;
        gpu.state.topology_checked =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        assert_eq!(gpu.poll(), Ok(vec![]), "not a topology change");
        let device = &gpu.discover().unwrap().devices[0];
        assert_eq!(device.id, "gpu/pci-0000:01:00.0");
        assert_eq!(device.properties["pciAddress"], "0000:01:00.0");

        // A different LUID without an address still gets the ordinal id.
        let mut other = nvidia();
        other.luid = 0x4242;
        other.pci = None;
        topology.lock().unwrap().push(other);
        let ids: Vec<_> = gpu
            .discover()
            .unwrap()
            .devices
            .iter()
            .map(|d| d.id.clone())
            .collect();
        assert_eq!(ids, ["gpu/pci-0000:01:00.0", "gpu/ven-10de-dev-2704-0"]);
    }

    #[test]
    fn restore_pci_records_and_restores_by_luid() {
        let mut known = HashMap::new();
        let mut first = [nvidia(), amd_igpu()];
        restore_pci(&mut known, &mut first);
        assert_eq!(known.len(), 2);
        let mut second = [nvidia(), amd_igpu()];
        second[1].pci = None;
        restore_pci(&mut known, &mut second);
        assert_eq!(second[1].pci, amd_igpu().pci);
        let mut unknown = [virtual_adapter(0x1414)];
        restore_pci(&mut known, &mut unknown);
        assert_eq!(unknown[0].pci, None);
    }
```

Esegui: `cargo test -p oma-win --lib gpu::tests`
Risultato atteso: errori di compilazione, tra cui `error[E0407]: method `properties` is not a member of trait `GpuLayer``, `cannot find function `restore_pci` in this scope` e `failed to resolve: use of undeclared type `HashMap``.

- [ ] **Step 8: Implementa `GpuLayer::properties`, la fusione e `restore_pci`**

In `crates/oma-win/src/gpu/layer.rs`, dopo il metodo `is_experimental` del trait (l'import `BTreeMap` c'è già):

```rust
    /// Static per-adapter properties (index as in the last attach); merged into Device.properties,
    /// higher-priority layer wins per key. Called once per discover, after attach.
    fn properties(&self, _adapter: usize) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
```

In `crates/oma-win/src/gpu/mod.rs`:

Commento del modulo:

```rust
//! GPU provider: one device per physical adapter, each field read from the
//! highest-priority layer that supports it (spec §5.2): vendor libraries, then
//! D3DKMT, DXGI and PDH. Layers also contribute static device properties
//! (PnP for the PCIe maximum link, NVML for limits).
```

Import (sostituiscono le righe esistenti):

```rust
use std::collections::{BTreeMap, BTreeSet, HashMap};
```

```rust
use adapter::{Adapter, PciAddress};
```

Campo nuovo in fondo a `pub struct GpuProvider`:

```rust
    switch: VendorSwitch,
    state: State,
    /// PCI address last seen per LUID, kept across discovers (see `restore_pci`).
    known_pci: HashMap<u64, PciAddress>,
}
```

In `with_layers`, il commento del parametro `base` diventa `// priority order: d3dkmt, dxgi, pdh, pnp` e il costruttore inizializza il campo:

```rust
            switch,
            state: State::default(),
            known_pci: HashMap::new(),
        }
```

Subito prima di `fn device(adapter: &Adapter, id: &str) -> Device`:

```rust
/// Gives back the PCI address of an adapter whose kernel query failed this time but
/// worked before (same LUID), so its device id does not change; records every address
/// seen. A failed D3DKMT address query must not rename a GPU (M2 follow-up).
fn restore_pci(known: &mut HashMap<u64, PciAddress>, adapters: &mut [Adapter]) {
    for adapter in adapters {
        match adapter.pci {
            Some(pci) => {
                known.insert(adapter.luid, pci);
            }
            None => adapter.pci = known.get(&adapter.luid).copied(),
        }
    }
}

/// Adds layer properties to a device's own ones: the first layer (highest priority) wins
/// per key, and keys the device already has (`pciAddress`, `integrated`) are never replaced.
fn merge_properties(
    device: &mut BTreeMap<String, String>,
    layers: impl IntoIterator<Item = BTreeMap<String, String>>,
) {
    for properties in layers {
        for (key, value) in properties {
            device.entry(key).or_insert(value);
        }
    }
}
```

In `discover`, l'enumerazione diventa:

```rust
        self.state = State::default();
        let mut adapters = (self.enumerate)()?;
        restore_pci(&mut self.known_pci, &mut adapters);
```

e, nel ciclo sugli adattatori, la riga `inventory.devices.push(device(adapter, &id));` diventa:

```rust
            let mut gpu = device(adapter, &id);
            merge_properties(
                &mut gpu.properties,
                layers.iter().map(|layer| layer.properties(index)),
            );
            inventory.devices.push(gpu);
```

(`layers` è la lista dei layer attivi in ordine di priorità, già usata per `attach`: le proprietà si chiedono dopo tutti gli `attach`.)

In `poll`, nel controllo di topologia:

```rust
            let mut topology = (self.enumerate)()?;
            restore_pci(&mut self.known_pci, &mut topology);
            self.state.topology_checked = Some(now);
```

- [ ] **Step 9: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-win --lib gpu::tests`
Risultato atteso: tutti OK, compresi `layer_properties_are_merged_by_priority`, `safe_mode_keeps_base_layer_properties_only`, `pci_address_is_kept_per_luid_when_the_kernel_query_fails`, `restore_pci_records_and_restores_by_luid` e i test M2 esistenti (`devices_have_stable_ids_names_vendors_and_properties`, `topology_changes_are_detected_without_live_handles`).

- [ ] **Step 10: Scrivi `PnpLayer` con i suoi test**

In `crates/oma-win/Cargo.toml`, nell'elenco `features` della dipendenza `windows`, `Win32_Devices_DeviceAndDriverInstallation` c'è già dal Task 4 (identità PnP dei dischi). Aggiungi solo `Win32_Devices_Properties`, subito dopo di essa; l'inizio dell'elenco diventa:

```toml
  "Wdk_Graphics_Direct3D",
  "Win32_Devices_DeviceAndDriverInstallation",
  "Win32_Devices_Properties",
  "Win32_Foundation",
```

In `crates/oma-win/src/gpu/mod.rs`, dichiara il modulo dopo `pub(crate) mod pdh;`:

```rust
pub(crate) mod pdh;
pub(crate) mod pnp;
pub(crate) mod trim;
```

Crea `crates/oma-win/src/gpu/pnp.rs`:

```rust
//! Vendor-neutral PCIe link capability from the Plug and Play property store (cfgmgr32).
//!
//! Each display adapter interface is opened once with D3DKMT to learn its LUID, then its
//! device node gives the PCI "max link speed/width" properties. These are static (what the
//! link can do). The PnP "current link" pair is a snapshot taken when the device started and
//! is never refreshed (it reads Gen 4 while NVML reports Gen 1 at idle), so it is not used.
//! The layer declares no sensors: it only contributes device properties.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem::size_of;

use oma_core::model::Source;
use oma_core::provider::ProviderError;
use windows::core::{GUID, PCWSTR};
use windows::Wdk::Graphics::Direct3D::{
    D3DKMTCloseAdapter, D3DKMTOpenAdapterFromDeviceName, D3DKMT_CLOSEADAPTER,
    D3DKMT_OPENADAPTERFROMDEVICENAME,
};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_PropertyW, CM_Locate_DevNodeW, CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
    CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_InstanceId, DEVPROPTYPE};
use windows::Win32::Foundation::DEVPROPKEY;

use super::adapter::Adapter;
use super::enumerate::luid_to_u64;
use super::field::GpuField;
use super::layer::{GpuLayer, Readings};

const _: () = assert!(size_of::<D3DKMT_OPENADAPTERFROMDEVICENAME>() == 24);
const _: () = assert!(size_of::<DEVPROPKEY>() == 20);

/// Interface class of every display adapter (GUID_DISPLAY_DEVICE_ARRIVAL).
const DISPLAY_ADAPTER_INTERFACE: GUID = GUID::from_u128(0x1ca05180_a699_450a_9a0c_de4fbe3ddd89);
/// Property set of PCI devices; every key below is a DEVPROP_TYPE_UINT32.
const PCI_PROPERTY_SET: GUID = GUID::from_u128(0x3ab22e31_8264_4b4e_9af5_a8d2d8e33e62);
/// Highest link speed the device supports, as a generation number (4 = 16 GT/s).
const MAX_LINK_SPEED: DEVPROPKEY = DEVPROPKEY {
    fmtid: PCI_PROPERTY_SET,
    pid: 11,
};
/// Highest link width the device supports, in lanes.
const MAX_LINK_WIDTH: DEVPROPKEY = DEVPROPKEY {
    fmtid: PCI_PROPERTY_SET,
    pid: 12,
};
/// A device instance id has at most 200 characters (MAX_DEVICE_ID_LEN) plus the NUL.
const INSTANCE_ID_LEN: usize = 256;

/// `pcieMaxGen` / `pcieMaxWidth` from the raw PnP values; implausible values are left out.
pub(crate) fn link_properties(
    max_speed: Option<u32>,
    max_width: Option<u32>,
) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    if let Some(generation) = max_speed.filter(|g| (1..=7).contains(g)) {
        properties.insert("pcieMaxGen".to_owned(), generation.to_string());
    }
    if let Some(lanes) = max_width.filter(|w| (1..=32).contains(w)) {
        properties.insert("pcieMaxWidth".to_owned(), lanes.to_string());
    }
    properties
}

/// Splits a REG_MULTI_SZ-style list into its strings, each keeping its NUL terminator.
pub(crate) fn split_multi_sz(list: &[u16]) -> Vec<Vec<u16>> {
    list.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut z = s.to_vec();
            z.push(0);
            z
        })
        .collect()
}

/// Paths of the present display adapter interfaces (NUL-terminated UTF-16).
fn display_interfaces() -> Vec<Vec<u16>> {
    let mut len = 0u32;
    // SAFETY: `len` is a valid out pointer and the class GUID outlives the call.
    let cr = unsafe {
        CM_Get_Device_Interface_List_SizeW(
            &mut len,
            &DISPLAY_ADAPTER_INTERFACE,
            PCWSTR::null(),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };
    if cr != CR_SUCCESS || len == 0 {
        return Vec::new();
    }
    let mut list = vec![0u16; len as usize];
    // SAFETY: `list` has the length the size call returned.
    let cr = unsafe {
        CM_Get_Device_Interface_ListW(
            &DISPLAY_ADAPTER_INTERFACE,
            PCWSTR::null(),
            &mut list,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };
    if cr != CR_SUCCESS {
        return Vec::new();
    }
    split_multi_sz(&list)
}

/// LUID of the adapter behind an interface path, from a handle closed right away.
fn interface_luid(path: &[u16]) -> Option<u64> {
    let mut open = D3DKMT_OPENADAPTERFROMDEVICENAME {
        pDeviceName: PCWSTR(path.as_ptr()),
        ..Default::default()
    };
    // SAFETY: `path` is NUL-terminated and outlives the call; `open` is writable.
    if unsafe { D3DKMTOpenAdapterFromDeviceName(&mut open) }.0 < 0 {
        return None;
    }
    let close = D3DKMT_CLOSEADAPTER {
        hAdapter: open.hAdapter,
    };
    // SAFETY: the handle was just opened and is not used afterwards.
    unsafe {
        let _ = D3DKMTCloseAdapter(&close);
    }
    Some(luid_to_u64(open.AdapterLuid))
}

/// Device node of the device that exposes an interface path.
fn interface_devnode(path: &[u16]) -> Option<u32> {
    let mut kind = DEVPROPTYPE(0);
    let mut id = [0u16; INSTANCE_ID_LEN];
    let mut size = size_of::<[u16; INSTANCE_ID_LEN]>() as u32;
    // SAFETY: `path` is NUL-terminated; `id` is a writable buffer of `size` bytes.
    let cr = unsafe {
        CM_Get_Device_Interface_PropertyW(
            PCWSTR(path.as_ptr()),
            &DEVPKEY_Device_InstanceId,
            &mut kind,
            Some(id.as_mut_ptr().cast()),
            &mut size,
            0,
        )
    };
    if cr != CR_SUCCESS {
        return None;
    }
    let mut devnode = 0u32;
    // SAFETY: `id` holds a NUL-terminated instance id (the buffer was zeroed and is larger
    // than the longest id); `devnode` is a valid out pointer.
    let cr =
        unsafe { CM_Locate_DevNodeW(&mut devnode, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) };
    (cr == CR_SUCCESS).then_some(devnode)
}

/// LUID -> device node of every present display adapter.
fn devnodes_by_luid() -> HashMap<u64, u32> {
    display_interfaces()
        .iter()
        .filter_map(|path| Some((interface_luid(path)?, interface_devnode(path)?)))
        .collect()
}

/// A 32-bit device property; `None` when absent (e.g. a non-PCI adapter).
fn property_u32(devnode: u32, key: &DEVPROPKEY) -> Option<u32> {
    let mut kind = DEVPROPTYPE(0);
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `value` is a writable buffer of `size` bytes and `key` outlives the call.
    let cr = unsafe {
        CM_Get_DevNode_PropertyW(
            devnode,
            key,
            &mut kind,
            Some((&mut value as *mut u32).cast()),
            &mut size,
            0,
        )
    };
    (cr == CR_SUCCESS && size == 4).then_some(value)
}

/// Base layer that only contributes the PCIe maximum link as device properties.
#[derive(Default)]
pub(crate) struct PnpLayer {
    /// Per adapter of the last attach.
    properties: Vec<BTreeMap<String, String>>,
}

impl GpuLayer for PnpLayer {
    fn source(&self) -> Source {
        Source::Pnp
    }

    fn attach(&mut self, adapters: &[Adapter]) -> Vec<BTreeSet<GpuField>> {
        let devnodes = devnodes_by_luid();
        self.properties = adapters
            .iter()
            .map(|adapter| match devnodes.get(&adapter.luid) {
                Some(&devnode) => link_properties(
                    property_u32(devnode, &MAX_LINK_SPEED),
                    property_u32(devnode, &MAX_LINK_WIDTH),
                ),
                None => BTreeMap::new(),
            })
            .collect();
        vec![BTreeSet::new(); adapters.len()]
    }

    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        Ok(vec![Readings::new(); self.properties.len()])
    }

    fn properties(&self, adapter: usize) -> BTreeMap<String, String> {
        self.properties.get(adapter).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_properties_keep_plausible_values_only() {
        assert_eq!(
            link_properties(Some(4), Some(16)),
            BTreeMap::from([
                ("pcieMaxGen".to_owned(), "4".to_owned()),
                ("pcieMaxWidth".to_owned(), "16".to_owned()),
            ])
        );
        assert_eq!(
            link_properties(Some(0), Some(64)),
            BTreeMap::new(),
            "zero generation and 64 lanes are not plausible"
        );
        assert_eq!(
            link_properties(None, Some(8)),
            BTreeMap::from([("pcieMaxWidth".to_owned(), "8".to_owned())])
        );
        assert_eq!(link_properties(None, None), BTreeMap::new());
    }

    #[test]
    fn multi_sz_list_is_split_into_terminated_strings() {
        let list: Vec<u16> = "ab\0cd\0\0".encode_utf16().collect();
        assert_eq!(
            split_multi_sz(&list),
            vec![
                "ab\0".encode_utf16().collect::<Vec<u16>>(),
                "cd\0".encode_utf16().collect::<Vec<u16>>(),
            ]
        );
        assert!(split_multi_sz(&[0, 0]).is_empty());
    }

    #[test]
    fn layer_without_attach_has_no_properties() {
        let layer = PnpLayer::default();
        assert_eq!(layer.source(), Source::Pnp);
        assert!(layer.properties(0).is_empty());
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn reads_the_max_link_of_both_gpus() {
        let adapters = crate::gpu::enumerate::enumerate().expect("enumerate");
        let mut layer = PnpLayer::default();
        let fields = layer.attach(&adapters);
        assert_eq!(fields.len(), adapters.len());
        assert!(
            fields.iter().all(BTreeSet::is_empty),
            "PnP declares no sensors"
        );
        for vendor_id in [0x10DE, 0x1002] {
            let index = adapters
                .iter()
                .position(|a| a.vendor_id == vendor_id)
                .expect("adapter present");
            let properties = layer.properties(index);
            println!("{}: {properties:?}", adapters[index].name);
            assert_eq!(
                properties.get("pcieMaxGen").map(String::as_str),
                Some("4")
            );
            assert_eq!(
                properties.get("pcieMaxWidth").map(String::as_str),
                Some("16")
            );
        }
        assert_eq!(layer.sample().expect("sample").len(), adapters.len());
    }
}
```

Infine, in `GpuProvider::new` (`mod.rs`), aggiungi il layer in coda ai layer base e aggiorna il commento:

```rust
    /// The real provider: DXGI/DXCore/D3DKMT enumeration, the base layers
    /// (D3DKMT, DXGI, PDH, PnP) always on, and the vendor libraries (NVML,
    /// NVAPI, ADL, IGCL) loaded on the first discover that sees `switch` on.
```

```rust
                Box::new(pdh::PdhLayer::default()),
                Box::new(pnp::PnpLayer::default()),
            ],
```

- [ ] **Step 11: Esegui i test di `pnp` e verifica che passino**

Esegui: `cargo test -p oma-win --lib gpu::pnp`
Risultato atteso: 3 test passati (`link_properties_keep_plausible_values_only`, `multi_sz_list_is_split_into_terminated_strings`, `layer_without_attach_has_no_properties`) e 1 ignorato (`reads_the_max_link_of_both_gpus`). Le asserzioni di dimensione (`D3DKMT_OPENADAPTERFROMDEVICENAME` = 24 byte, `DEVPROPKEY` = 20 byte) compilano.

- [ ] **Step 12: Scrivi i test degli extra NVML (falliscono)**

In `crates/oma-win/src/gpu/nvml.rs`, modulo `tests`, subito prima di `fn pci_lookup_helpers`:

```rust
    #[test]
    fn link_values_of_zero_are_missing() {
        assert_eq!(link_value(4), Some(4.0));
        assert_eq!(link_value(16), Some(16.0));
        assert_eq!(link_value(0), None);
    }

    /// The values the RTX 4080 of the development machine reports (spike, driver 617.14).
    fn rtx_4080_static() -> StaticReads {
        StaticReads {
            max_link_gen: (SUCCESS, 4),
            max_link_width: (SUCCESS, 16),
            power_min_mw: (SUCCESS, 150_000),
            power_max_mw: (SUCCESS, 370_000),
            power_default_mw: (SUCCESS, 320_000),
            temp_slowdown: (SUCCESS, 94),
            temp_shutdown: (SUCCESS, 99),
            temp_gpu_max: (SUCCESS, 90),
        }
    }

    #[test]
    fn static_reads_become_decimal_properties() {
        let properties = rtx_4080_static().properties();
        let expected = [
            ("pcieMaxGen", "4"),
            ("pcieMaxWidth", "16"),
            ("powerLimitMinW", "150"),
            ("powerLimitMaxW", "370"),
            ("powerLimitDefaultW", "320"),
            ("tempSlowdownC", "94"),
            ("tempShutdownC", "99"),
            ("tempMaxC", "90"),
        ];
        assert_eq!(
            properties,
            expected
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<BTreeMap<_, _>>()
        );
    }

    #[test]
    fn failed_or_zero_static_reads_are_left_out() {
        let reads = StaticReads {
            power_min_mw: (ERROR_NOT_SUPPORTED, 150_000),
            power_max_mw: (ERROR_NOT_SUPPORTED, 370_000),
            power_default_mw: (SUCCESS, 152_500),
            temp_gpu_max: (SUCCESS, 0),
            ..rtx_4080_static()
        };
        let properties = reads.properties();
        assert!(!properties.contains_key("powerLimitMinW"));
        assert!(!properties.contains_key("powerLimitMaxW"));
        assert!(!properties.contains_key("tempMaxC"));
        assert_eq!(properties["powerLimitDefaultW"], "152.5");
        assert_eq!(properties.len(), 5);
    }
```

Nel test hardware `nvml_reads_the_rtx_4080`, dopo il blocco finale `if let Some(fan) = r.get(&FanPercent) { … }`, aggiungi:

```rust
        // M3 extras: encoder/decoder utilization and the live link (Gen 1 at idle with ASPM,
        // up to Gen 4 under load; always x16 on this board).
        for field in [LoadEncoder, LoadDecoder, PcieLinkGen, PcieLinkWidth] {
            assert!(supported[nvidia].contains(&field), "NVML lacks {field:?}");
        }
        assert!((0.0..=100.0).contains(&r[&LoadEncoder]));
        assert!((0.0..=100.0).contains(&r[&LoadDecoder]));
        let generation = r[&PcieLinkGen];
        assert!((1.0..=4.0).contains(&generation), "PCIe gen {generation}");
        assert_eq!(r[&PcieLinkWidth], 16.0);
        let properties = layer.properties(nvidia);
        println!("NVML properties: {properties:?}");
        for (key, value) in [
            ("pcieMaxGen", "4"),
            ("pcieMaxWidth", "16"),
            ("powerLimitMinW", "150"),
            ("powerLimitMaxW", "370"),
            ("powerLimitDefaultW", "320"),
            ("tempSlowdownC", "94"),
            ("tempShutdownC", "99"),
            ("tempMaxC", "90"),
        ] {
            assert_eq!(
                properties.get(key).map(String::as_str),
                Some(value),
                "{key}"
            );
        }
        for (i, adapter) in adapters.iter().enumerate() {
            if i != nvidia {
                assert!(layer.properties(i).is_empty(), "{}", adapter.name);
            }
        }
```

Esegui: `cargo test -p oma-win --lib gpu::nvml`
Risultato atteso: errori di compilazione `cannot find function `link_value``, `cannot find struct `StaticReads`` e `cannot find type `BTreeMap``.

- [ ] **Step 13: Implementa gli extra NVML**

In `crates/oma-win/src/gpu/nvml.rs`:

Commento del modulo (prime due righe) e import:

```rust
//! NVIDIA layer over NVML (nvml.dll, loaded from System32 only): core temperature, clocks,
//! board power and limit, fans, dedicated memory, throttle reasons, encoder/decoder
//! utilization and the live PCIe link, plus static limits as device properties.
```

```rust
use std::collections::{BTreeMap, BTreeSet};
```

Dopo `const THERMAL_REASONS …;`, le costanti delle soglie; `FIELDS` passa a 16 elementi:

```rust
/// Temperature threshold kinds of `nvmlDeviceGetTemperatureThreshold`.
const THRESHOLD_SHUTDOWN: u32 = 0;
const THRESHOLD_SLOWDOWN: u32 = 1;
const THRESHOLD_GPU_MAX: u32 = 3;

/// Fields this layer can offer, probed per GPU at attach.
const FIELDS: [GpuField; 16] = [
    GpuField::MemoryDedicatedUsed,
    GpuField::MemoryDedicatedTotal,
    GpuField::TemperatureCore,
    GpuField::ClockCore,
    GpuField::ClockMemory,
    GpuField::PowerBoard,
    GpuField::PowerLimit,
    GpuField::PowerLimitPercent,
    GpuField::FanPercent,
    GpuField::FanRpm,
    GpuField::ThrottlePower,
    GpuField::ThrottleThermal,
    GpuField::LoadEncoder,
    GpuField::LoadDecoder,
    GpuField::PcieLinkGen,
    GpuField::PcieLinkWidth,
];
```

Dopo `type U32ArgFn = …;`:

```rust
/// Two u32 out values: (utilization %, sampling period µs) or (min mW, max mW).
type U32PairFn = unsafe extern "C" fn(DeviceHandle, *mut u32, *mut u32) -> Ret;
```

Subito prima di `/// Bus id string accepted by …` (`fn bus_id`):

```rust
/// A generation or lane count; 0 means "not reported".
fn link_value(value: u32) -> Option<f64> {
    (value > 0).then(|| f64::from(value))
}

/// Static values read once at attach (decision D9): each is the NVML return code and the raw
/// value (mW for power, °C for temperatures).
#[derive(Debug, Clone, Copy)]
struct StaticReads {
    max_link_gen: (Ret, u32),
    max_link_width: (Ret, u32),
    power_min_mw: (Ret, u32),
    power_max_mw: (Ret, u32),
    power_default_mw: (Ret, u32),
    temp_slowdown: (Ret, u32),
    temp_shutdown: (Ret, u32),
    temp_gpu_max: (Ret, u32),
}

impl StaticReads {
    /// Device properties (plain decimal strings); a failed call or a zero value is left out.
    fn properties(&self) -> BTreeMap<String, String> {
        let entries = [
            ("pcieMaxGen", self.max_link_gen, 1),
            ("pcieMaxWidth", self.max_link_width, 1),
            ("powerLimitMinW", self.power_min_mw, 1000),
            ("powerLimitMaxW", self.power_max_mw, 1000),
            ("powerLimitDefaultW", self.power_default_mw, 1000),
            ("tempSlowdownC", self.temp_slowdown, 1),
            ("tempShutdownC", self.temp_shutdown, 1),
            ("tempMaxC", self.temp_gpu_max, 1),
        ];
        entries
            .into_iter()
            .filter(|(_, (ret, value), _)| *ret == SUCCESS && *value > 0)
            .map(|(key, (_, value), divisor)| {
                (
                    key.to_owned(),
                    (f64::from(value) / f64::from(divisor)).to_string(),
                )
            })
            .collect()
    }
}
```

(`f64::to_string` scrive 150.0 come "150" e 152.5 come "152.5": sono le stringhe decimali semplici del contratto.)

In fondo alla struct `Api`:

```rust
    event_reasons: Option<U64Fn>,
    throttle_reasons: Option<U64Fn>,
    encoder_utilization: Option<U32PairFn>,
    decoder_utilization: Option<U32PairFn>,
    curr_link_gen: Option<U32Fn>,
    curr_link_width: Option<U32Fn>,
    max_link_gen: Option<U32Fn>,
    max_link_width: Option<U32Fn>,
    power_constraints: Option<U32PairFn>,
    power_default_limit: Option<U32Fn>,
    temperature_threshold: Option<U32ArgFn>,
}
```

Subito prima di `fn call_u64`:

```rust
fn call_u32_pair(f: Option<U32PairFn>, device: DeviceHandle) -> (Ret, u32, u32) {
    let Some(f) = f else {
        return (ERROR_FUNCTION_NOT_FOUND, 0, 0);
    };
    let (mut first, mut second) = (0u32, 0u32);
    // SAFETY: as in `call_u32`, with two valid out pointers.
    let ret = unsafe { f(device, &mut first, &mut second) };
    (ret, first, second)
}
```

In `Api::resolve`, dopo `throttle_reasons: …,` (tutte facoltative: un simbolo mancante toglie solo i campi o le proprietà che ne dipendono):

```rust
                throttle_reasons: library.symbol(c"nvmlDeviceGetCurrentClocksThrottleReasons"),
                encoder_utilization: library.symbol(c"nvmlDeviceGetEncoderUtilization"),
                decoder_utilization: library.symbol(c"nvmlDeviceGetDecoderUtilization"),
                curr_link_gen: library.symbol(c"nvmlDeviceGetCurrPcieLinkGeneration"),
                curr_link_width: library.symbol(c"nvmlDeviceGetCurrPcieLinkWidth"),
                max_link_gen: library.symbol(c"nvmlDeviceGetMaxPcieLinkGeneration"),
                max_link_width: library.symbol(c"nvmlDeviceGetMaxPcieLinkWidth"),
                power_constraints: library
                    .symbol(c"nvmlDeviceGetPowerManagementLimitConstraints"),
                power_default_limit: library.symbol(c"nvmlDeviceGetPowerManagementDefaultLimit"),
                temperature_threshold: library.symbol(c"nvmlDeviceGetTemperatureThreshold"),
            })
```

Nell'`impl Api`, subito prima di `/// Reads one field of `device` …` (`fn read`):

```rust
    /// Static limits and link capability of `device`, read once at attach.
    fn static_reads(&self, device: DeviceHandle) -> StaticReads {
        let (constraints, power_min, power_max) = call_u32_pair(self.power_constraints, device);
        let threshold = |kind| call_u32_arg(self.temperature_threshold, device, kind);
        StaticReads {
            max_link_gen: call_u32(self.max_link_gen, device),
            max_link_width: call_u32(self.max_link_width, device),
            power_min_mw: (constraints, power_min),
            power_max_mw: (constraints, power_max),
            power_default_mw: call_u32(self.power_default_limit, device),
            temp_slowdown: threshold(THRESHOLD_SLOWDOWN),
            temp_shutdown: threshold(THRESHOLD_SHUTDOWN),
            temp_gpu_max: threshold(THRESHOLD_GPU_MAX),
        }
    }
```

In `fn read`, tra il ramo `GpuField::ThrottleThermal => { … }` e il ramo finale `_ => (ERROR_NOT_SUPPORTED, None),`:

```rust
            GpuField::LoadEncoder => {
                let (ret, percent, _period_us) = call_u32_pair(self.encoder_utilization, device);
                (ret, Some(f64::from(percent)))
            }
            GpuField::LoadDecoder => {
                let (ret, percent, _period_us) = call_u32_pair(self.decoder_utilization, device);
                (ret, Some(f64::from(percent)))
            }
            GpuField::PcieLinkGen => {
                let (ret, generation) = call_u32(self.curr_link_gen, device);
                (ret, link_value(generation))
            }
            GpuField::PcieLinkWidth => {
                let (ret, lanes) = call_u32(self.curr_link_width, device);
                (ret, link_value(lanes))
            }
```

Nella struct `Bound`, in fondo:

```rust
    fields: BTreeSet<GpuField>,
    /// Static limits and link capability (decision D9), read once here.
    properties: BTreeMap<String, String>,
}
```

In `NvmlLayer::bind`, la costruzione diventa:

```rust
        let fields = probe(&FIELDS, |field| self.api.read(device, fans, field));
        let properties = self.api.static_reads(device).properties();
        Some(Bound {
            device,
            fans,
            fields,
            properties,
        })
```

Nell'`impl GpuLayer for NvmlLayer`, dopo `fn sample`:

```rust
    fn properties(&self, adapter: usize) -> BTreeMap<String, String> {
        self.bound
            .get(adapter)
            .and_then(Option::as_ref)
            .map(|b| b.properties.clone())
            .unwrap_or_default()
    }
```

- [ ] **Step 14: Esegui i test unitari e verifica che passino**

Esegui: `cargo test -p oma-win --lib gpu::`
Risultato atteso: tutti OK, compresi `link_values_of_zero_are_missing`, `static_reads_become_decimal_properties` e `failed_or_zero_static_reads_are_left_out`. Il test M2 `support_is_what_answers_at_attach` resta valido (`FIELDS.len() - 2`).

- [ ] **Step 15: Aggiungi i test hardware del provider**

In `crates/oma-win/tests/providers.rs` l'import del modello resta quello del Task 5 (`use oma_core::model::{Sensor, SensorKind, Source, Unit};`), che contiene già `SensorKind`. Subito prima di `fn gpu_provider_loads_vendor_libraries_when_reenabled` (con i suoi attributi), aggiungi:

```rust
/// Value of device property `key` of GPU `id`.
fn gpu_property<'a>(inventory: &'a Inventory, id: &str, key: &str) -> Option<&'a str> {
    inventory
        .devices
        .iter()
        .find(|d| d.id == id)
        .and_then(|d| d.properties.get(key))
        .map(String::as_str)
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_reports_pcie_link_and_static_limits() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut p = GpuProvider::new(VendorSwitch::new(true));
    let (inventory, values) = discover_and_poll(&mut p);
    assert_gpu_devices(&inventory);

    // Live link from NVML: Gen 1 at idle (ASPM), up to Gen 4 under load; x16 on this board.
    let (i, generation) = gpu_sensor(&inventory, NVIDIA, "link/pcie-gen");
    assert_eq!(generation.source, Source::Nvml);
    let gen = values[i].expect("NVIDIA PCIe generation");
    assert!((1.0..=4.0).contains(&gen), "Gen {gen}");
    let (i, width) = gpu_sensor(&inventory, NVIDIA, "link/pcie-width");
    assert_eq!(width.source, Source::Nvml);
    assert_eq!(values[i], Some(16.0));
    for name in ["load/encoder", "load/decoder"] {
        let (i, sensor) = gpu_sensor(&inventory, NVIDIA, name);
        assert_eq!(sensor.source, Source::Nvml, "{name}");
        let pct = values[i].expect(name);
        assert!((0.0..=100.0).contains(&pct), "{name} {pct}");
    }
    // No live link source for the AMD iGPU (PnP "current" is not live, ADL 40/41 excluded).
    assert!(!inventory
        .sensors
        .iter()
        .any(|s| s.device_id == AMD && s.kind == SensorKind::Link));

    for (key, value) in [
        ("pcieMaxGen", "4"),
        ("pcieMaxWidth", "16"),
        ("powerLimitMinW", "150"),
        ("powerLimitMaxW", "370"),
        ("powerLimitDefaultW", "320"),
        ("tempSlowdownC", "94"),
        ("tempShutdownC", "99"),
        ("tempMaxC", "90"),
    ] {
        assert_eq!(gpu_property(&inventory, NVIDIA, key), Some(value), "{key}");
    }
    // The iGPU gets the vendor-neutral PnP maximum link only.
    assert_eq!(gpu_property(&inventory, AMD, "pcieMaxGen"), Some("4"));
    assert_eq!(gpu_property(&inventory, AMD, "pcieMaxWidth"), Some("16"));
    assert_eq!(gpu_property(&inventory, AMD, "powerLimitMaxW"), None);
}

#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_in_safe_mode_keeps_the_pnp_max_link() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut p = GpuProvider::new(VendorSwitch::new(false));
    let inventory = p.discover().expect("discover");
    for id in [NVIDIA, AMD] {
        assert_eq!(
            gpu_property(&inventory, id, "pcieMaxGen"),
            Some("4"),
            "{id}"
        );
        assert_eq!(
            gpu_property(&inventory, id, "pcieMaxWidth"),
            Some("16"),
            "{id}"
        );
        assert_eq!(gpu_property(&inventory, id, "tempMaxC"), None, "{id}");
    }
    assert!(!inventory.sensors.iter().any(|s| s.id.contains("/link/")));
}
```

(Il Task 8 cambia la firma in `GpuProvider::new(switch, processes)` e aggiorna queste due chiamate insieme alle altre.)

- [ ] **Step 16: Verifica su hardware reale**

Esegui: `cargo test -p oma-win -- --include-ignored`
Risultato atteso: tutti OK. Verificato durante la stesura su questa macchina (RTX 4080, driver 617.14, e iGPU Raphael):
- `gpu::pnp::tests::reads_the_max_link_of_both_gpus` stampa `NVIDIA GeForce RTX 4080: {"pcieMaxGen": "4", "pcieMaxWidth": "16"}` e `AMD Radeon(TM) Graphics: {"pcieMaxGen": "4", "pcieMaxWidth": "16"}`;
- `gpu::nvml::tests::nvml_reads_the_rtx_4080` stampa `NVML properties: {"pcieMaxGen": "4", "pcieMaxWidth": "16", "powerLimitDefaultW": "320", "powerLimitMaxW": "370", "powerLimitMinW": "150", "tempMaxC": "90", "tempShutdownC": "99", "tempSlowdownC": "94"}`; a riposo il link attuale vale Gen 1 x16 (Gen 4 con un carico in corso) ed encoder/decoder valgono 0 %;
- `gpu_provider_reports_pcie_link_and_static_limits`: `link/pcie-gen`, `link/pcie-width`, `load/encoder` e `load/decoder` della RTX hanno sorgente NVML; la iGPU AMD non ha sensori `link` ma ha le proprietà PnP Gen 4 x16;
- `gpu_provider_in_safe_mode_keeps_the_pnp_max_link`: senza librerie dei fornitori entrambe le GPU hanno `pcieMaxGen` 4 e `pcieMaxWidth` 16 e nessuna soglia NVML;
- i test GPU M2 (`gpu_provider_finds_both_gpus_with_merged_sources`, `…_in_safe_mode_uses_only_base_layers`, `…_loads_vendor_libraries_when_reenabled`) continuano a passare.

Se la RTX sta lavorando (per esempio un video in riproduzione), la generazione attuale può valere 2–4: il test accetta 1..=4.

- [ ] **Step 17: Lint e commit**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && pnpm test && cd ..
git add crates/oma-core/src/model.rs crates/oma-core/src/sanitize.rs crates/oma-win/Cargo.toml crates/oma-win/src/gpu/field.rs crates/oma-win/src/gpu/layer.rs crates/oma-win/src/gpu/mod.rs crates/oma-win/src/gpu/pnp.rs crates/oma-win/src/gpu/nvml.rs crates/oma-win/tests/labels.rs crates/oma-win/tests/providers.rs app/src/lib/i18n/en.json app/src/lib/i18n/it.json
git commit -m "feat(win): PCIe link, encoder/decoder load and static GPU limits"
```

(`pnpm test` verifica la parità delle chiavi tra `en.json` e `it.json`. Se `Cargo.lock` cambia, cosa che per le sole feature di windows-rs non succede, aggiungilo al commit.)

---

---

### Task 7: `oma-win`: link PCIe da IGCL (`ctlPciGetState`/`ctlPciGetProperties`, solo fake)

**File:**
- Modifica: `crates/oma-win/src/gpu/igcl.rs`
- Test: modulo `#[cfg(test)]` di `gpu/igcl.rs` (fake di ControlLib già esistente, esteso con le due funzioni PCI; il test hardware `igcl_reads_an_intel_gpu` stampa anche il link)

**Interfacce:**
- Usa (Task 6): `GpuField::PcieLinkGen`, `GpuField::PcieLinkWidth` (unità `PcieGeneration` e `Lanes`), il metodo fornito `GpuLayer::properties(&self, adapter: usize) -> BTreeMap<String, String>` e le chiavi di proprietà `pcieMaxGen` / `pcieMaxWidth`.
- Usa (M2): `IgclLayer`, `Api`, `Bound`, `read_telemetry`, `ERROR_DEVICE_UNAVAILABLE`, `ERROR_DEVICE_LOST` e il fake thread-local `FAKE` / `FAKE_API` / `one_device` / `card` / `adapter` dei test.
- Produce (privati a `igcl.rs`):
  ```rust
  #[repr(C)] struct PciLocation { size: u32, version: u8, domain: u32, bus: u32, device: u32, function: u32 } // 24 B, align 4
  #[repr(C)] struct PciSpeed { size: u32, version: u8, generation: i32, width: i32, max_bandwidth: i64 }    // 24 B, align 8
  #[repr(C)] struct PciProperties { size: u32, version: u8, address: PciLocation, max_speed: PciSpeed,
                                    resizable_bar_supported: u8, resizable_bar_enabled: u8 }                // 64 B
  #[repr(C)] struct PciState { size: u32, version: u8, speed: PciSpeed }                                    // 32 B
  type PciPropertiesFn = unsafe extern "C" fn(DeviceHandle, *mut PciProperties) -> CtlResult;
  type PciStateFn = unsafe extern "C" fn(DeviceHandle, *mut PciState) -> CtlResult;
  struct Api { /* … */ pci_properties: Option<PciPropertiesFn>, pci_state: Option<PciStateFn> }
  fn link_value(value: i32) -> Option<f64>;
  fn link_fields(speed: &PciSpeed) -> BTreeSet<GpuField>;
  fn link_readings(speed: &PciSpeed, fields: &BTreeSet<GpuField>) -> Readings;
  fn max_link_properties(properties: &PciProperties) -> BTreeMap<String, String>;
  fn read_pci_state(api: &Api, device: DeviceHandle) -> Option<PciState>;
  fn read_pci_properties(api: &Api, device: DeviceHandle) -> Option<PciProperties>;
  // IgclLayer: impl GpuLayer::properties (pcieMaxGen/pcieMaxWidth per adattatore)
  ```

Fatti (spike `gpuspike-m3`, voce 5: solo documentazione, nessun hardware Intel su questa macchina) e regole fissate qui:
- **Funzioni.** `ctl_result_t ctlPciGetProperties(ctl_device_adapter_handle_t, ctl_pci_properties_t*)` ("address, max speed") e `ctl_result_t ctlPciGetState(ctl_device_adapter_handle_t, ctl_pci_state_t*)` ("current speed"). La documentazione le dichiara thread-safe e senza lock. Convenzione cdecl, come il resto di IGCL su x86_64. Entrambe sono **facoltative**: se il runtime non le esporta, il layer perde solo i campi del link.
- **Layout** (ordine dei membri della reference HTML pubblica, calcolato a mano per x86_64 MSVC e fissato con `pin!`):
  - `ctl_pci_address_t` 24 B: `Size@0`, `Version@4`, `domain@8`, `bus@12`, `device@16`, `function@20`;
  - `ctl_pci_speed_t` 24 B, allineamento 8: `Size@0`, `Version@4`, `gen@8` (i32), `width@12` (i32), `maxBandwidth@16` (i64, byte/s su tutte le corsie); `-1` = sconosciuto;
  - `ctl_pci_properties_t` 64 B: `Size@0`, `Version@4`, `address@8`, `maxSpeed@32`, `resizable_bar_supported@56`, `resizable_bar_enabled@57` (bool C → `u8`), 6 B di padding finale;
  - `ctl_pci_state_t` 32 B: `Size@0`, `Version@4`, `speed@8`.
- **Incertezze dichiarate (come per il resto del layer IGCL).**
  1. Dimensioni calcolate a mano, non misurate contro l'header come i layout v298 già in `igcl.rs`.
  2. Non è noto quale `Version` passare: si usa 0, come per gli altri record. Non è noto nemmeno se il runtime controlla `Size`/`Version` dei record annidati; si compilano comunque tutti.
  3. `gen` è il numero di generazione (1..5) secondo la documentazione ("The link generation").
  4. La documentazione non dice se `ctlPciGetState` sia dal vivo. Per questo il campo è dichiarato solo se, all'`attach`, la lettura riesce con valori > 0.
- **Regole.** In `attach`, per un dispositivo la cui telemetria risponde: una lettura di `ctlPciGetState` dichiara `PcieLinkGen`/`PcieLinkWidth` per i valori > 0; una lettura di `ctlPciGetProperties` produce `pcieMaxGen` (se 1..=7) e `pcieMaxWidth` (se 1..=32). A ogni `sample` si rilegge lo stato solo se almeno un campo del link è dichiarato. Una lettura fallita, qualunque sia il codice (anche `DEVICE_UNAVAILABLE` in D3 o durante un TDR), fa mancare solo i campi del link per quel tick: non tocca la telemetria e non chiede `Rediscover`, perché della perdita del dispositivo si occupa già la telemetria con `ERROR_DEVICE_LOST`. `-1` o 0 in un tick significa "manca", non 0. Un dispositivo "pending" (non disponibile all'`attach`) non ha né campi né proprietà finché una nuova discovery non lo rilegge.
- **Priorità.** IGCL sta sotto NVML/NVAPI/ADL e sopra i layer base. Per le GPU Intel è quindi l'unica sorgente del link attuale, mentre `pcieMaxGen`/`pcieMaxWidth` di IGCL prevalgono su quelli PnP (Task 6).

- [ ] **Step 1: Scrivi i test (falliscono)**

In `crates/oma-win/src/gpu/igcl.rs`, modulo `tests`:

Sostituisci la struct `Fake` (e aggiungi `FakePci` e la costante subito prima) con:

```rust
    /// PCI replies of one device: `states` in order (the last one repeats), `max` always.
    #[derive(Clone)]
    struct FakePci {
        states: Vec<Result<(i32, i32), CtlResult>>,
        max: Result<(i32, i32), CtlResult>,
    }

    /// Any failure code: the layer treats every non-success the same way.
    const FAKE_PCI_FAILURE: CtlResult = ERROR_DEVICE_UNAVAILABLE;

    #[derive(Default)]
    struct Fake {
        init_replies: Vec<CtlResult>,
        init_versions: Vec<u32>,
        devices: Vec<FakeDevice>,
        /// (size, version) announced by every telemetry request.
        telemetry_calls: Vec<(u32, u8)>,
        /// Per device index; a device without an entry fails both PCI calls.
        pci: Vec<FakePci>,
        /// (record size, nested speed size) announced by every PCI state request.
        pci_state_calls: Vec<(u32, u32)>,
        /// (record size, nested address size, nested speed size) of every properties request.
        pci_properties_calls: Vec<(u32, u32, u32)>,
    }
```

(I test M2 costruiscono `Fake` con `..Fake::default()`: i campi nuovi restano vuoti, quindi per loro le chiamate PCI falliscono e non dichiarano nulla.)

Sostituisci `const FAKE_API: Api = …;` con le due funzioni fake e la nuova costante:

```rust
    unsafe extern "C" fn fake_pci_state(device: DeviceHandle, out: *mut PciState) -> CtlResult {
        FAKE.with_borrow_mut(|f| {
            // SAFETY: the layer passes a sized state record.
            let sizes = unsafe { ((*out).size, (*out).speed.size) };
            f.pci_state_calls.push(sizes);
            let Some(pci) = f.pci.get_mut(device_index(device)) else {
                return FAKE_PCI_FAILURE;
            };
            let reply = if pci.states.len() > 1 {
                pci.states.remove(0)
            } else {
                pci.states[0]
            };
            match reply {
                Ok((generation, width)) => {
                    // SAFETY: as above.
                    unsafe {
                        (*out).speed.generation = generation;
                        (*out).speed.width = width;
                    }
                    SUCCESS
                }
                Err(rc) => rc,
            }
        })
    }

    unsafe extern "C" fn fake_pci_properties(
        device: DeviceHandle,
        out: *mut PciProperties,
    ) -> CtlResult {
        FAKE.with_borrow_mut(|f| {
            // SAFETY: the layer passes a sized properties record.
            let sizes = unsafe { ((*out).size, (*out).address.size, (*out).max_speed.size) };
            f.pci_properties_calls.push(sizes);
            match f.pci.get(device_index(device)).map(|p| p.max) {
                Some(Ok((generation, width))) => {
                    // SAFETY: as above.
                    unsafe {
                        (*out).max_speed.generation = generation;
                        (*out).max_speed.width = width;
                    }
                    SUCCESS
                }
                Some(Err(rc)) => rc,
                None => FAKE_PCI_FAILURE,
            }
        })
    }

    const FAKE_API: Api = Api {
        init: fake_init,
        enumerate: fake_enumerate,
        properties: fake_properties,
        telemetry: fake_telemetry,
        pci_properties: Some(fake_pci_properties),
        pci_state: Some(fake_pci_state),
    };
```

Subito prima di `fn item_values_follow_their_type_tag`, aggiungi:

```rust
    fn speed(generation: i32, width: i32) -> PciSpeed {
        PciSpeed {
            generation,
            width,
            ..PciSpeed::new()
        }
    }

    #[test]
    fn pci_records_announce_their_sizes() {
        let state = PciState::new();
        assert_eq!((state.size, state.speed.size), (32, 24));
        let properties = PciProperties::new();
        assert_eq!(
            (
                properties.size,
                properties.address.size,
                properties.max_speed.size
            ),
            (64, 24, 24)
        );
    }

    #[test]
    fn link_fields_and_readings_skip_unknown_values() {
        use GpuField::*;
        assert_eq!(
            link_fields(&speed(4, 8)),
            BTreeSet::from([PcieLinkGen, PcieLinkWidth])
        );
        assert_eq!(link_fields(&speed(-1, 16)), BTreeSet::from([PcieLinkWidth]));
        assert!(link_fields(&speed(-1, -1)).is_empty());
        assert!(link_fields(&speed(0, 0)).is_empty());

        let both = BTreeSet::from([PcieLinkGen, PcieLinkWidth]);
        assert_eq!(
            link_readings(&speed(1, 8), &both),
            Readings::from([(PcieLinkGen, 1.0), (PcieLinkWidth, 8.0)])
        );
        // Unknown this tick: missing, not zero.
        assert_eq!(
            link_readings(&speed(-1, 8), &both),
            Readings::from([(PcieLinkWidth, 8.0)])
        );
        // A field not declared at attach is never reported.
        assert_eq!(
            link_readings(&speed(4, 8), &BTreeSet::from([PcieLinkWidth])),
            Readings::from([(PcieLinkWidth, 8.0)])
        );
    }

    #[test]
    fn max_link_properties_skip_unknown_values() {
        let mut properties = PciProperties::new();
        properties.max_speed = speed(4, 16);
        assert_eq!(
            max_link_properties(&properties),
            BTreeMap::from([
                ("pcieMaxGen".to_owned(), "4".to_owned()),
                ("pcieMaxWidth".to_owned(), "16".to_owned()),
            ])
        );
        properties.max_speed = speed(-1, 8);
        assert_eq!(
            max_link_properties(&properties),
            BTreeMap::from([("pcieMaxWidth".to_owned(), "8".to_owned())])
        );
        properties.max_speed = speed(-1, -1);
        assert!(max_link_properties(&properties).is_empty());
    }

    #[test]
    fn layer_reports_the_live_pcie_link_and_the_max_link() {
        use GpuField::*;
        let mut fake = one_device(vec![Ok(card(1.0, 100.0))]);
        fake.pci = vec![FakePci {
            states: vec![
                Ok((4, 8)),  // attach probe
                Ok((1, 8)),  // idle
                Ok((-1, 8)), // generation unknown this tick
            ],
            max: Ok((4, 16)),
        }];
        let mut layer = fake_layer(fake).expect("init");
        let adapters = [adapter(0x0000_0001_0000_ABCD, 0x8086, 3)];

        let supported = layer.attach(&adapters);
        assert!(supported[0].contains(&PcieLinkGen));
        assert!(supported[0].contains(&PcieLinkWidth));
        assert_eq!(
            layer.properties(0),
            BTreeMap::from([
                ("pcieMaxGen".to_owned(), "4".to_owned()),
                ("pcieMaxWidth".to_owned(), "16".to_owned()),
            ])
        );
        assert!(layer.properties(1).is_empty());

        let first = layer.sample().expect("first sample");
        assert_eq!(first[0][&PcieLinkGen], 1.0);
        assert_eq!(first[0][&PcieLinkWidth], 8.0);
        assert_eq!(first[0][&ClockCore], 2400.0);
        let second = layer.sample().expect("second sample");
        assert!(!second[0].contains_key(&PcieLinkGen));
        assert_eq!(second[0][&PcieLinkWidth], 8.0);

        let (state_calls, properties_calls) =
            FAKE.with_borrow(|f| (f.pci_state_calls.clone(), f.pci_properties_calls.clone()));
        assert_eq!(state_calls, [(32, 24); 3]);
        assert_eq!(properties_calls, [(64, 24, 24)]);
    }

    #[test]
    fn failed_link_read_drops_only_the_link_fields() {
        use GpuField::*;
        let mut fake = one_device(vec![Ok(card(1.0, 100.0))]);
        fake.pci = vec![FakePci {
            states: vec![Ok((4, 16)), Err(ERROR_DEVICE_UNAVAILABLE)],
            max: Err(ERROR_DEVICE_UNAVAILABLE),
        }];
        let mut layer = fake_layer(fake).expect("init");
        layer.attach(&[adapter(0x0000_0001_0000_ABCD, 0x8086, 3)]);

        assert!(layer.properties(0).is_empty());
        let readings = layer.sample().expect("sample");
        assert!(!readings[0].contains_key(&PcieLinkGen));
        assert!(!readings[0].contains_key(&PcieLinkWidth));
        assert_eq!(readings[0][&ClockCore], 2400.0);
    }

    #[test]
    fn runtime_without_pci_exports_keeps_telemetry() {
        FAKE.set(one_device(vec![Ok(card(1.0, 100.0))]));
        let api = Api {
            pci_properties: None,
            pci_state: None,
            ..FAKE_API
        };
        let mut layer = IgclLayer::start(api, None).expect("init");
        let supported = layer.attach(&[adapter(0x0000_0001_0000_ABCD, 0x8086, 3)]);

        assert!(supported[0].contains(&GpuField::ClockCore));
        assert!(!supported[0].contains(&GpuField::PcieLinkGen));
        assert!(!supported[0].contains(&GpuField::PcieLinkWidth));
        assert!(layer.properties(0).is_empty());
        assert_eq!(
            layer.sample().expect("sample")[0][&GpuField::ClockCore],
            2400.0
        );
        assert!(FAKE.with_borrow(|f| f.pci_state_calls.is_empty()));
    }
```

Nel test hardware `igcl_reads_an_intel_gpu`, dopo il blocco finale `if supported[intel].contains(&GpuField::PowerBoard) { … }`, aggiungi:

```rust
        // PCIe link (unverified layout): print it for a manual comparison with GPU-Z.
        println!(
            "IGCL link {:?} x{:?}, max {:?}",
            readings[intel].get(&GpuField::PcieLinkGen),
            readings[intel].get(&GpuField::PcieLinkWidth),
            layer.properties(intel)
        );
        if let Some(generation) = readings[intel].get(&GpuField::PcieLinkGen) {
            assert!((1.0..=6.0).contains(generation), "PCIe gen {generation}");
        }
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-win --lib gpu::igcl`
Risultato atteso: errori di compilazione, tra cui `cannot find type `PciState` in this scope`, `cannot find type `PciProperties``, `struct `Api` has no field named `pci_properties`` e `cannot find function `link_fields``.

- [ ] **Step 3: Implementa i record, le chiamate e le regole**

In `crates/oma-win/src/gpu/igcl.rs`:

Alla fine del commento del modulo (dopo il paragrafo "Not verified on real hardware …"), aggiungi:

```rust
//!
//! PCIe link (M3): `ctlPciGetState` gives the current generation/width (sensors) and
//! `ctlPciGetProperties` the maximum ones (device properties). Their records were laid out
//! from the public IGCL API reference in documented member order and computed by hand for
//! x86_64 MSVC, not measured against a header; both exports are optional.
```

Import:

```rust
use std::collections::{BTreeMap, BTreeSet};
```

Subito prima di `macro_rules! pin {`:

```rust
/// PCI location as IGCL reports it (`ctl_pci_address_t`).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct PciLocation {
    size: u32,
    version: u8,
    domain: u32,
    bus: u32,
    device: u32,
    function: u32,
}

/// Link generation, lane count and bandwidth (`ctl_pci_speed_t`); -1 means unknown.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct PciSpeed {
    size: u32,
    version: u8,
    generation: i32,
    width: i32,
    /// Bytes per second over all lanes.
    max_bandwidth: i64,
}

/// Static PCI properties (`ctl_pci_properties_t`, filled by `ctlPciGetProperties`).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct PciProperties {
    size: u32,
    version: u8,
    address: PciLocation,
    max_speed: PciSpeed,
    resizable_bar_supported: u8,
    resizable_bar_enabled: u8,
}

/// Current PCI state (`ctl_pci_state_t`, filled by `ctlPciGetState`).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
struct PciState {
    size: u32,
    version: u8,
    speed: PciSpeed,
}
```

Dopo l'ultima riga `pin!(PowerTelemetry, …)` e prima delle due `const _: () = assert!(…TELEMETRY…)`:

```rust
pin!(PciLocation, 24, 4);
pin!(PciLocation, domain @ 8);
pin!(PciLocation, function @ 20);
pin!(PciSpeed, 24, 8);
pin!(PciSpeed, generation @ 8);
pin!(PciSpeed, width @ 12);
pin!(PciSpeed, max_bandwidth @ 16);
pin!(PciProperties, 64, 8);
pin!(PciProperties, address @ 8);
pin!(PciProperties, max_speed @ 32);
pin!(PciProperties, resizable_bar_supported @ 56);
pin!(PciProperties, resizable_bar_enabled @ 57);
pin!(PciState, 32, 8);
pin!(PciState, speed @ 8);
```

Subito prima di `impl TelemetryItem {`:

```rust
impl PciSpeed {
    fn new() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            ..Self::default()
        }
    }
}

impl PciState {
    /// Record with its own and the nested speed record's size filled in (IGCL convention;
    /// whether the runtime checks the nested one is not documented).
    fn new() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            version: 0,
            speed: PciSpeed::new(),
        }
    }
}

impl PciProperties {
    /// As `PciState::new`, for every nested record.
    fn new() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            address: PciLocation {
                size: size_of::<PciLocation>() as u32,
                ..PciLocation::default()
            },
            max_speed: PciSpeed::new(),
            ..Self::default()
        }
    }
}

/// A generation or lane count; -1 (unknown) and 0 are missing.
fn link_value(value: i32) -> Option<f64> {
    (value > 0).then(|| f64::from(value))
}

/// Link fields a device supports according to one PCI state read.
fn link_fields(speed: &PciSpeed) -> BTreeSet<GpuField> {
    let mut fields = BTreeSet::new();
    if link_value(speed.generation).is_some() {
        fields.insert(GpuField::PcieLinkGen);
    }
    if link_value(speed.width).is_some() {
        fields.insert(GpuField::PcieLinkWidth);
    }
    fields
}

/// Current link values of one PCI state read, limited to the declared fields.
fn link_readings(speed: &PciSpeed, fields: &BTreeSet<GpuField>) -> Readings {
    let mut readings = Readings::new();
    for (field, value) in [
        (GpuField::PcieLinkGen, speed.generation),
        (GpuField::PcieLinkWidth, speed.width),
    ] {
        if let (true, Some(value)) = (fields.contains(&field), link_value(value)) {
            readings.insert(field, value);
        }
    }
    readings
}

/// `pcieMaxGen` / `pcieMaxWidth` from the static PCI properties; unknown values are left out.
fn max_link_properties(properties: &PciProperties) -> BTreeMap<String, String> {
    let speed = &properties.max_speed;
    let mut map = BTreeMap::new();
    if (1..=7).contains(&speed.generation) {
        map.insert("pcieMaxGen".to_owned(), speed.generation.to_string());
    }
    if (1..=32).contains(&speed.width) {
        map.insert("pcieMaxWidth".to_owned(), speed.width.to_string());
    }
    map
}
```

Sostituisci la dichiarazione dei tipi di funzione e di `Api` (da `type TelemetryFn …` alla fine di `struct Api`) con:

```rust
type TelemetryFn = unsafe extern "C" fn(DeviceHandle, *mut PowerTelemetry) -> CtlResult;
type PciPropertiesFn = unsafe extern "C" fn(DeviceHandle, *mut PciProperties) -> CtlResult;
type PciStateFn = unsafe extern "C" fn(DeviceHandle, *mut PciState) -> CtlResult;

#[derive(Clone, Copy)]
struct Api {
    init: InitFn,
    enumerate: EnumerateFn,
    properties: PropertiesFn,
    telemetry: TelemetryFn,
    /// Optional: runtimes without the PCI exports only lose the link fields.
    pci_properties: Option<PciPropertiesFn>,
    pci_state: Option<PciStateFn>,
}

/// One `ctlPciGetState` read; `None` without the export or on any failure.
fn read_pci_state(api: &Api, device: DeviceHandle) -> Option<PciState> {
    let f = api.pci_state?;
    let mut state = PciState::new();
    // SAFETY: `device` came from ctlEnumerateDevices on a live session; `state` is a sized
    // record that outlives the call.
    (unsafe { f(device, &mut state) } == SUCCESS).then_some(state)
}

/// One `ctlPciGetProperties` read; `None` without the export or on any failure.
fn read_pci_properties(api: &Api, device: DeviceHandle) -> Option<PciProperties> {
    let f = api.pci_properties?;
    let mut properties = PciProperties::new();
    // SAFETY: as in `read_pci_state`.
    (unsafe { f(device, &mut properties) } == SUCCESS).then_some(properties)
}
```

In fondo alla struct `Bound`:

```rust
    pending: bool,
    /// Maximum PCIe link (`pcieMaxGen`, `pcieMaxWidth`), read once at attach.
    properties: BTreeMap<String, String>,
}
```

In `IgclLayer::load`, nell'inizializzazione di `Api`, dopo `telemetry: …?,` (senza `?`: sono facoltative):

```rust
                telemetry: library.symbol(c"ctlPowerTelemetryGet")?,
                pci_properties: library.symbol(c"ctlPciGetProperties"),
                pci_state: library.symbol(c"ctlPciGetState"),
            }
```

In `attach`, il ramo `Ok(t)` diventa:

```rust
                Ok(t) => {
                    let (mut fields, energy) = supported_fields(&t);
                    if let Some(state) = read_pci_state(&self.api, device) {
                        fields.extend(link_fields(&state.speed));
                    }
                    let properties = read_pci_properties(&self.api, device)
                        .map(|p| max_link_properties(&p))
                        .unwrap_or_default();
                    bound[i] = Some(Bound {
                        device,
                        layout,
                        fields,
                        energy,
                        meter: EnergyMeter::default(),
                        warned: false,
                        pending: false,
                        properties,
                    });
                }
```

e nel ramo `Err(ERROR_DEVICE_UNAVAILABLE)` il `Bound` "pending" riceve `properties: BTreeMap::new(),` dopo `pending: true,`.

In `sample`, nel ramo `Ok(t)`, tra il blocco dell'energia e `all.push(readings);`:

```rust
                    let link = [GpuField::PcieLinkGen, GpuField::PcieLinkWidth];
                    if link.iter().any(|f| bound.fields.contains(f)) {
                        // A failed link read only drops the link fields for this tick.
                        if let Some(state) = read_pci_state(&api, bound.device) {
                            readings.extend(link_readings(&state.speed, &bound.fields));
                        }
                    }
                    all.push(readings);
```

Nell'`impl GpuLayer for IgclLayer`, dopo `fn sample`:

```rust
    fn properties(&self, adapter: usize) -> BTreeMap<String, String> {
        self.bound
            .get(adapter)
            .and_then(Option::as_ref)
            .map(|b| b.properties.clone())
            .unwrap_or_default()
    }
```

- [ ] **Step 4: Esegui i test e verifica che passino**

Esegui: `cargo test -p oma-win --lib gpu::igcl`
Risultato atteso: tutti OK. Ai test M2 si aggiungono sei test nuovi: `pci_records_announce_their_sizes`, `link_fields_and_readings_skip_unknown_values`, `max_link_properties_skip_unknown_values`, `layer_reports_the_live_pcie_link_and_the_max_link`, `failed_link_read_drops_only_the_link_fields` e `runtime_without_pci_exports_keeps_telemetry`. Gli `assert!` di `pin!` sui quattro record compilano. I due test hardware restano ignorati.

- [ ] **Step 5: Verifica su hardware reale (nessuna GPU Intel qui)**

Esegui: `cargo test -p oma-win --lib gpu::igcl -- --include-ignored --nocapture`
Risultato atteso su questa macchina: tutti OK. `igcl_absent_without_intel_gpu` passa perché `ControlLib.dll` è assente; `igcl_reads_an_intel_gpu` stampa `no Intel adapter: skipped`. Su una macchina con GPU Intel lo stesso test stampa `IGCL link Some(..) xSome(..), max {..}`: confronta a mano generazione e larghezza con GPU-Z (scheda "Bus Interface") prima di togliere la nota "unverified".

- [ ] **Step 6: Lint e commit**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add crates/oma-win/src/gpu/igcl.rs
git commit -m "feat(win): PCIe link state and max link from IGCL"
```

---

---

### Task 8: GPU per processo: contatori PDH dei processi, nomi dei processi, `GpuProcessTable` e comando `get_gpu_processes`

**File:**
- Crea: `crates/oma-win/src/gpu/processes.rs` (`GpuProcess`, `GpuProcessTable`)
- Crea: `crates/oma-win/src/gpu/procname.rs` (`ProcessNames`, cache Toolhelp)
- Modifica: `crates/oma-win/src/gpu/pdh.rs` (pid nelle istanze dei motori, contatori `GPU Process Memory`, carico per processo, pubblicazione)
- Modifica: `crates/oma-win/src/gpu/mod.rs` (moduli, `GpuProvider::new(switch, processes)`, mappa id dispositivo → LUID)
- Modifica: `crates/oma-win/src/lib.rs` (`default_providers(vendor, processes)`)
- Modifica: `crates/oma-win/Cargo.toml` (dipendenza `serde`, feature `Win32_System_Diagnostics_ToolHelp`), `Cargo.lock`
- Modifica: `crates/oma-win/tests/providers.rs` (nuova firma e test hardware)
- Modifica: `app/src-tauri/src/commands.rs`, `app/src-tauri/src/main.rs`, `app/src-tauri/build.rs`, `app/src-tauri/capabilities/default.json`
- Crea (generato dalla build): `app/src-tauri/permissions/autogenerated/get_gpu_processes.toml`
- Test: moduli `#[cfg(test)]` di `processes.rs`, `procname.rs`, `pdh.rs`, `mod.rs`; test hardware `#[ignore]` in `procname.rs`, `pdh.rs` e `tests/providers.rs`

**Interfacce:**
- Usa (M2): `PdhLayer`, `EngineInstance`, `parse_engine`, `parse_luid`, `aggregate`, `adapter_readings`, `crate::pdh::{Query, Counter}` (`array()` restituisce `Vec<(String, f64)>`; i valori non validi sono NaN), `crate::network::wide_to_string`, `GpuProvider::with_layers` e il fake `provider(...)` dei test di `mod.rs`.
- Usa (Task 6): `GpuProvider` con `known_pci` e `merge_properties`, `PnpLayer` tra i layer base (questo task aggiunge righe accanto a quelle).
- Usa (Task 3): in `main.rs`, `build.rs` e `capabilities/default.json` il Task 3 ha già aggiunto `get_stats`, `reset_stats` e `get_session`. Qui si aggiunge `get_gpu_processes` **dopo** le loro voci.
- Produce:
  ```rust
  // crates/oma-win/src/gpu/processes.rs (riesportati come oma_win::gpu::{GpuProcess, GpuProcessTable})
  #[derive(Debug, Clone, PartialEq, serde::Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct GpuProcess { pub pid: u32, pub name: String, pub load_percent: Option<f64>, pub engine: Option<String>, pub dedicated_bytes: Option<u64>, pub shared_bytes: Option<u64> }
  #[derive(Clone, Default)]
  pub struct GpuProcessTable(Arc<Mutex<Inner>>);
  impl GpuProcessTable {
      pub fn new() -> Self;
      pub fn processes(&self, device_id: &str) -> Vec<GpuProcess>;          // ordinati, <= 20 righe
      pub(crate) fn set_devices(&self, devices: Vec<(String, u64)>);        // id dispositivo -> LUID, da GpuProvider::discover
      pub(crate) fn publish(&self, by_luid: HashMap<u64, Vec<GpuProcess>>); // da PdhLayer::sample
  }
  // crates/oma-win/src/gpu/procname.rs
  pub(crate) struct ProcessNames; // Default = Toolhelp; with_snapshot(Box<dyn FnMut() -> Vec<(u32, String)> + Send>)
  impl ProcessNames { pub(crate) fn update(&mut self, pids: &BTreeSet<u32>); pub(crate) fn name(&self, pid: u32) -> String; }
  // crates/oma-win/src/gpu/pdh.rs
  pub(crate) struct EngineInstance { pub pid: u32, pub luid: u64, pub engine: u32, pub engtype: String }
  pub(crate) fn parse_process_memory(instance: &str) -> Option<(u32, u64)>;               // (pid, LUID)
  pub(crate) struct ProcessUsage { pub load: Option<f64>, pub engine: Option<String>, pub dedicated: Option<u64>, pub shared: Option<u64> }
  pub(crate) fn process_usage(engines: Option<&[(EngineInstance, f64)]>, dedicated: &[(String, f64)], shared: &[(String, f64)]) -> BTreeMap<(u64, u32), ProcessUsage>;
  pub(crate) fn process_rows(usage: BTreeMap<(u64, u32), ProcessUsage>, name: impl Fn(u32) -> String) -> HashMap<u64, Vec<GpuProcess>>;
  impl PdhLayer { pub(crate) fn new(processes: GpuProcessTable) -> Self; }
  // crates/oma-win/src/gpu/mod.rs, lib.rs
  pub fn GpuProvider::new(switch: VendorSwitch, processes: GpuProcessTable) -> Self;
  pub fn default_providers(vendor: gpu::VendorSwitch, processes: gpu::GpuProcessTable) -> Vec<Box<dyn Provider>>;
  // app/src-tauri/src/commands.rs
  pub struct GpuProcessState(pub GpuProcessTable);
  #[tauri::command(async)] pub fn get_gpu_processes(state: State<'_, GpuProcessState>, device_id: String) -> Vec<GpuProcess>; // JS: invoke('get_gpu_processes', { deviceId })
  ```
- JSON di una riga (contratto TS `GpuProcess` del Task 9): `{ "pid": 2096, "name": "dwm.exe", "loadPercent": 3.5, "engine": "3D", "dedicatedBytes": 2000000000, "sharedBytes": null }`.

Fatti verificati su questa macchina (spike `gpuspike-m3`, voce 1, e verifica del codice di questo task) e regole fissate qui:
- **Contatori (decisione D5).** Si aggiungono con `PdhAddEnglishCounterW` alla query già usata dal layer PDH GPU: `\GPU Process Memory(*)\Dedicated Usage` e `\GPU Process Memory(*)\Shared Usage`. Sono byte e sono validi già al primo campione. Il costo marginale è di circa 50 µs per tick: il collect e l'array di `GPU Engine(*)` si pagano già oggi, e con i tre contatori in una sola query il tick misurato dura circa 1,8 ms in mediana. Un processo nuovo compare al collect successivo senza aggiungere di nuovo il contatore. Un processo terminato sparisce senza errori. Un processo che vive meno di un intervallo non si vede mai. In 90 s a 10 Hz, con 45 ffmpeg brevi, la memoria privata è rimasta piatta.
- **Istanze.**
  - Motore: `pid_26328_luid_0x00000000_0x00018036_phys_0_eng_6_engtype_VideoEncode`. Il tipo del motore può contenere spazi ("Video Codec 0", "High Priority 3D").
  - Memoria di processo: `pid_26328_luid_0x00000000_0x00018036_phys_0`. Più righe `phys_N` dello stesso (pid, LUID) si sommano.
  - Il pid 4 (System: traffico di copia/paging del kernel) compare come "processo".
- **Carico per processo = motore singolo più occupato** (massimo sui motori, limitato a 0..=100). È coerente con `LoadCore` dell'adattatore, che è il motore più occupato. Esempio misurato con ffmpeg `hevc_nvenc` sulla RTX: 3D 73,7, VideoEncode 50,1 e 49,0. La somma per tipo darebbe 99,1 (VideoEncode), più del `LoadCore` dell'adattatore (73,7); il massimo dà 73,7 ("3D"). `engine` è il tipo di quel motore ed è valorizzato solo se il carico è > 0: quando tutto vale 0 il tipo sarebbe arbitrario.
- **Primo tick dopo un (ri)attach:** `load_percent` è `None` per tutti (contatore a tasso non ancora innescato, regola `fresh` del layer); la memoria c'è già. Un processo appena comparso ha tutte le righe dei motori NaN (`PDH_CSTATUS_INVALID_DATA` per voce): il suo carico è `None`, non 0.
- **Nomi (decisione D6).** Con `OpenProcess` falliscono 99 processi su 245 per un utente normale, tra cui dwm.exe (che occupa 1,9–2,5 GB sulla RTX), csrss, System e i servizi: quindi non si usa. Non si usa nemmeno l'interfaccia non documentata `NtQuerySystemInformation(88)`. Si usa `CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)` + `Process32FirstW/NextW`: documentata, senza handle di processo, circa 2,4 ms, e dà `szExeFile` ("dwm.exe"). La cache è `HashMap<pid, String>`:
  - lo snapshot si prende al massimo una volta per tick e solo se c'è un pid senza nome;
  - un pid assente dai contatori per un tick si dimentica;
  - pid 0 → "Idle" e pid 4 → "System", senza snapshot;
  - un pid che non compare nello snapshot (processo appena terminato) si memorizza come "PID <n>", così non provoca uno snapshot a ogni tick.

  Unico caso perso: un processo termina e il suo pid viene riusato nello stesso tick; è trascurabile. Costo se la scelta fosse sbagliata: circa 2,5 ms nei tick con processi GPU nuovi.
- **Tabella.** Righe per LUID, pubblicate a ogni tick (con la finestra chiusa costano circa 0,3 ms: parse più una `HashMap` piccola). `processes(device_id)` mappa l'id del dispositivo al LUID (impostato da `discover`), ordina per carico decrescente (sconosciuto in fondo), poi per memoria dedicata decrescente (sconosciuta in fondo), poi per pid, e restituisce al massimo 20 righe. Un dispositivo sconosciuto dà una lista vuota. Se il layer PDH non ha contatori o un campione fallisce, la tabella si svuota: niente righe vecchie. Un errore di lettura dei soli contatori per processo (`debug!`) non tocca le letture dell'adattatore.
- **Non sono sensori:** niente id, niente cronologia, niente statistiche. Gli elenchi dei processi NVML restano esclusi (D10: su WDDM `usedGpuMemory` è sempre N/A e ogni chiamata costa circa 0,6 ms).
- **Righe osservate qui** (secondo campione del test hardware, con Chrome che riproduce un video): `chrome.exe` 12,9 % "VideoDecode" con 186 MB dedicati; `System` (pid 4) 0,4 % "Copy"; `dwm.exe` 0,2 % "3D" con 2,46 GB dedicati.

- [ ] **Step 1: Scrivi `GpuProcessTable` con i suoi test**

In `crates/oma-win/Cargo.toml`, tra le dipendenze, aggiungi `serde` (con la feature `derive` presa dal workspace):

```toml
[dependencies]
oma-core.workspace = true
serde.workspace = true
tracing.workspace = true
sha2 = "0.10"
```

e, nell'elenco `features` di `windows`, in ordine alfabetico dopo `"Win32_System_Diagnostics_Debug",`:

```toml
  "Win32_System_Diagnostics_Debug",
  "Win32_System_Diagnostics_ToolHelp",
```

In `crates/oma-win/src/gpu/mod.rs`, dichiara i moduli e riesporta i tipi pubblici (dopo `pub(crate) mod pnp;` del Task 6):

```rust
pub(crate) mod pdh;
pub(crate) mod pnp;
pub(crate) mod processes;
pub(crate) mod procname;
pub(crate) mod trim;

pub use processes::{GpuProcess, GpuProcessTable};
```

Crea `crates/oma-win/src/gpu/processes.rs`:

```rust
//! Per-process GPU usage (decision D5), shared between the GPU provider, which publishes it
//! every tick, and the shell command `get_gpu_processes`, which reads it. Not sensors: no
//! ids, no history.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// Rows returned per device, busiest first.
const MAX_ROWS: usize = 20;

/// One process using one GPU during the last tick.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuProcess {
    pub pid: u32,
    /// Executable file name, e.g. "dwm.exe"; "Idle"/"System" for pids 0/4, "PID <n>" when
    /// the process ended before its name could be read.
    pub name: String,
    /// Busiest single engine of the process, 0..=100; `None` on the first tick after a
    /// (re)attach or for a process that just appeared (rate counter without two samples).
    pub load_percent: Option<f64>,
    /// Driver name of that engine ("3D", "VideoEncode", "Video Codec 0"...), only when the
    /// load is above 0.
    pub engine: Option<String>,
    pub dedicated_bytes: Option<u64>,
    pub shared_bytes: Option<u64>,
}

#[derive(Default)]
struct Inner {
    by_luid: HashMap<u64, Vec<GpuProcess>>,
    devices: HashMap<String, u64>,
}

/// Latest per-process GPU usage, keyed by adapter; cheap to clone (shared state).
#[derive(Clone, Default)]
pub struct GpuProcessTable(Arc<Mutex<Inner>>);

/// Busiest first: load (unknown last), then dedicated memory (unknown last), then pid.
fn busiest_first(a: &GpuProcess, b: &GpuProcess) -> Ordering {
    let load = |p: &GpuProcess| p.load_percent.unwrap_or(-1.0);
    load(b)
        .total_cmp(&load(a))
        .then_with(|| b.dedicated_bytes.cmp(&a.dedicated_bytes))
        .then_with(|| a.pid.cmp(&b.pid))
}

impl GpuProcessTable {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Processes of GPU `device_id`, busiest first, at most 20; empty for an unknown device.
    pub fn processes(&self, device_id: &str) -> Vec<GpuProcess> {
        let mut rows = {
            let inner = self.lock();
            let Some(luid) = inner.devices.get(device_id) else {
                return Vec::new();
            };
            inner.by_luid.get(luid).cloned().unwrap_or_default()
        };
        rows.sort_by(busiest_first);
        rows.truncate(MAX_ROWS);
        rows
    }

    /// Device id -> adapter LUID of the last discover (replaces the previous mapping).
    pub(crate) fn set_devices(&self, devices: Vec<(String, u64)>) {
        self.lock().devices = devices.into_iter().collect();
    }

    /// Replaces the whole table with the rows of the last tick.
    pub(crate) fn publish(&self, by_luid: HashMap<u64, Vec<GpuProcess>>) {
        self.lock().by_luid = by_luid;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, load: Option<f64>, dedicated: Option<u64>) -> GpuProcess {
        GpuProcess {
            pid,
            name: format!("p{pid}.exe"),
            load_percent: load,
            engine: None,
            dedicated_bytes: dedicated,
            shared_bytes: None,
        }
    }

    const RTX: u64 = 0x18036;
    const RADEON: u64 = 0x1AAD7;

    fn table() -> GpuProcessTable {
        let table = GpuProcessTable::new();
        table.set_devices(vec![
            ("gpu/pci-0000:01:00.0".to_owned(), RTX),
            ("gpu/pci-0000:11:00.0".to_owned(), RADEON),
        ]);
        table
    }

    #[test]
    fn rows_are_sorted_by_load_then_dedicated_memory() {
        let table = table();
        table.publish(HashMap::from([(
            RTX,
            vec![
                row(10, None, Some(900)),
                row(11, Some(5.0), Some(1)),
                row(12, Some(40.0), None),
                row(13, Some(5.0), Some(2_000)),
                row(14, None, None),
                row(15, None, Some(900)),
            ],
        )]));
        let pids: Vec<u32> = table
            .processes("gpu/pci-0000:01:00.0")
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(pids, [12, 13, 11, 10, 15, 14]);
    }

    #[test]
    fn at_most_twenty_rows_are_returned() {
        let table = table();
        let rows = (0..30)
            .map(|pid| row(pid, Some(f64::from(pid)), None))
            .collect();
        table.publish(HashMap::from([(RTX, rows)]));
        let top = table.processes("gpu/pci-0000:01:00.0");
        assert_eq!(top.len(), 20);
        assert_eq!(top[0].pid, 29);
        assert_eq!(top[19].pid, 10);
    }

    #[test]
    fn unknown_device_or_adapter_gives_an_empty_list() {
        let table = table();
        table.publish(HashMap::from([(RTX, vec![row(1, Some(1.0), None)])]));
        assert!(table.processes("gpu/pci-0000:11:00.0").is_empty());
        assert!(table.processes("gpu/unknown").is_empty());
        assert!(GpuProcessTable::new()
            .processes("gpu/pci-0000:01:00.0")
            .is_empty());
    }

    #[test]
    fn clones_share_the_table_and_publish_replaces_it() {
        let table = table();
        let reader = table.clone();
        table.publish(HashMap::from([(RTX, vec![row(1, Some(1.0), None)])]));
        assert_eq!(reader.processes("gpu/pci-0000:01:00.0").len(), 1);
        table.publish(HashMap::new());
        assert!(reader.processes("gpu/pci-0000:01:00.0").is_empty());
        // A rediscover that drops a device also drops its rows from the answers.
        table.publish(HashMap::from([(RADEON, vec![row(2, None, Some(5))])]));
        table.set_devices(vec![("gpu/pci-0000:01:00.0".to_owned(), RTX)]);
        assert!(reader.processes("gpu/pci-0000:11:00.0").is_empty());
    }

    #[test]
    fn serializes_with_the_ts_contract_keys() {
        let process = GpuProcess {
            pid: 2096,
            name: "dwm.exe".to_owned(),
            load_percent: Some(3.5),
            engine: Some("3D".to_owned()),
            dedicated_bytes: Some(2_000_000_000),
            shared_bytes: None,
        };
        assert_eq!(
            serde_json::to_value(&process).unwrap(),
            serde_json::json!({
                "pid": 2096,
                "name": "dwm.exe",
                "loadPercent": 3.5,
                "engine": "3D",
                "dedicatedBytes": 2_000_000_000u64,
                "sharedBytes": null
            })
        );
    }
}
```

(Per non lasciare il crate rotto fra uno step e l'altro, crea subito anche `procname.rs` con il contenuto dello Step 2; i suoi test si eseguono nello Step 3.)

- [ ] **Step 2: Scrivi `ProcessNames` con i suoi test**

Crea `crates/oma-win/src/gpu/procname.rs`:

```rust
//! pid -> executable name for the per-process GPU table (decision D6).
//!
//! A Toolhelp process snapshot needs no process handle, so it also names the processes a
//! normal user cannot open (dwm.exe, csrss.exe, services). It costs ~2.4 ms, so it is taken
//! at most once per tick and only when a pid without a cached name shows up.

use std::collections::{BTreeSet, HashMap};
use std::mem::size_of;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};

const _: () = assert!(size_of::<PROCESSENTRY32W>() == 568);

/// Lists (pid, executable name) of every running process.
type Snapshot = Box<dyn FnMut() -> Vec<(u32, String)> + Send>;

/// Names that never need a snapshot.
fn fixed_name(pid: u32) -> Option<&'static str> {
    match pid {
        0 => Some("Idle"),
        4 => Some("System"),
        _ => None,
    }
}

fn fallback_name(pid: u32) -> String {
    format!("PID {pid}")
}

/// Cache of process names for the pids seen in the GPU counters.
pub(crate) struct ProcessNames {
    names: HashMap<u32, String>,
    snapshot: Snapshot,
}

impl Default for ProcessNames {
    fn default() -> Self {
        Self::with_snapshot(Box::new(toolhelp_processes))
    }
}

impl ProcessNames {
    pub(crate) fn with_snapshot(snapshot: Snapshot) -> Self {
        Self {
            names: HashMap::new(),
            snapshot,
        }
    }

    /// Keeps names for exactly `pids` (the pids in this tick's counters): forgets the others
    /// and resolves the new ones with at most one snapshot. A pid missing from the snapshot
    /// (the process just ended) is cached as "PID <n>", so it never costs a second snapshot.
    pub(crate) fn update(&mut self, pids: &BTreeSet<u32>) {
        self.names.retain(|pid, _| pids.contains(pid));
        let missing: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|&pid| fixed_name(pid).is_none() && !self.names.contains_key(&pid))
            .collect();
        if missing.is_empty() {
            return;
        }
        let listed: HashMap<u32, String> = (self.snapshot)().into_iter().collect();
        for pid in missing {
            let name = listed
                .get(&pid)
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| fallback_name(pid));
            self.names.insert(pid, name);
        }
    }

    pub(crate) fn name(&self, pid: u32) -> String {
        fixed_name(pid)
            .map(str::to_owned)
            .or_else(|| self.names.get(&pid).cloned())
            .unwrap_or_else(|| fallback_name(pid))
    }
}

/// Every running process from one Toolhelp snapshot; empty if the snapshot fails.
fn toolhelp_processes() -> Vec<(u32, String)> {
    // SAFETY: plain flags; the returned handle is closed below.
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(handle) => handle,
        Err(e) => {
            tracing::debug!(error = %e, "process snapshot failed");
            return Vec::new();
        }
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut processes = Vec::new();
    // SAFETY: the snapshot handle is open and `entry` is writable with `dwSize` set.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        processes.push((
            entry.th32ProcessID,
            crate::network::wide_to_string(&entry.szExeFile),
        ));
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: the handle is open and not used afterwards.
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    processes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Names from a fixed process list; the counter counts snapshots.
    fn names(list: &[(u32, &str)]) -> (ProcessNames, Arc<AtomicUsize>) {
        let list: Vec<(u32, String)> = list.iter().map(|&(p, n)| (p, n.to_owned())).collect();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let names = ProcessNames::with_snapshot(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            list.clone()
        }));
        (names, calls)
    }

    #[test]
    fn new_pids_are_resolved_with_one_snapshot() {
        let (mut names, calls) = names(&[(2096, "dwm.exe"), (1712, "csrss.exe"), (9, "")]);
        names.update(&BTreeSet::from([2096, 1712]));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(names.name(2096), "dwm.exe");
        assert_eq!(names.name(1712), "csrss.exe");

        // Known pids only: no snapshot.
        names.update(&BTreeSet::from([2096]));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn idle_and_system_never_need_a_snapshot() {
        let (mut names, calls) = names(&[]);
        names.update(&BTreeSet::from([0, 4]));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(names.name(0), "Idle");
        assert_eq!(names.name(4), "System");
    }

    #[test]
    fn a_pid_absent_from_the_snapshot_is_cached_with_a_fallback() {
        let (mut names, calls) = names(&[(9, "")]);
        names.update(&BTreeSet::from([777, 9]));
        assert_eq!(names.name(777), "PID 777");
        assert_eq!(names.name(9), "PID 9", "empty names are not shown");
        names.update(&BTreeSet::from([777, 9]));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no second snapshot");
    }

    #[test]
    fn pids_gone_from_the_counters_are_forgotten() {
        let (mut names, calls) = names(&[(2096, "dwm.exe")]);
        names.update(&BTreeSet::from([2096]));
        names.update(&BTreeSet::new());
        assert_eq!(names.name(2096), "PID 2096");
        // Seen again (e.g. a reused pid): resolved again.
        names.update(&BTreeSet::from([2096]));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(names.name(2096), "dwm.exe");
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn toolhelp_names_the_desktop_window_manager() {
        let processes = toolhelp_processes();
        assert!(processes.len() > 20, "{} processes", processes.len());
        let own = std::process::id();
        let (_, name) = processes
            .iter()
            .find(|(pid, _)| *pid == own)
            .expect("the test process is listed");
        assert!(name.to_ascii_lowercase().ends_with(".exe"), "{name}");
        assert!(
            processes
                .iter()
                .any(|(_, name)| name.eq_ignore_ascii_case("dwm.exe")),
            "dwm.exe cannot be opened by a normal user but must be named"
        );
    }
}
```

- [ ] **Step 3: Esegui i test dei due moduli nuovi e verifica che passino**

Esegui: `cargo test -p oma-win --lib gpu::processes` e poi `cargo test -p oma-win --lib gpu::procname`
Risultato atteso: 5 test passati in `processes` (`rows_are_sorted_by_load_then_dedicated_memory`, `at_most_twenty_rows_are_returned`, `unknown_device_or_adapter_gives_an_empty_list`, `clones_share_the_table_and_publish_replaces_it`, `serializes_with_the_ts_contract_keys`); 4 passati e 1 ignorato in `procname`. Clippy può segnalare `set_devices`, `publish`, `ProcessNames` come codice morto finché gli Step 5–7 non li collegano: è atteso, e il lint si esegue solo allo Step 12.

(Questi due moduli sono nuovi e autonomi: i test si scrivono insieme al codice. Il fallimento atteso "prima" è banale, cioè il modulo non esiste; i test del comportamento nuovo del layer PDH seguono invece il ciclo rosso/verde negli Step 4–6.)

- [ ] **Step 4: Scrivi i test del carico e della memoria per processo in `pdh.rs` (falliscono)**

In `crates/oma-win/src/gpu/pdh.rs`, modulo `tests`, sostituisci l'helper `engine` con questi due helper:

```rust
    fn engine(luid: u64, engine: u32, engtype: &str) -> EngineInstance {
        process_engine(0, luid, engine, engtype)
    }

    fn process_engine(pid: u32, luid: u64, engine: u32, engtype: &str) -> EngineInstance {
        EngineInstance {
            pid,
            luid,
            engine,
            engtype: engtype.to_owned(),
        }
    }
```

Nel test `parses_engine_instances`, i quattro valori attesi portano ora il pid:

```rust
        assert_eq!(
            parse_engine("pid_15028_luid_0x00000000_0x00017DB6_phys_0_eng_0_engtype_3D"),
            Some(process_engine(15028, RTX, 0, "3D"))
        );
        assert_eq!(
            parse_engine("pid_6860_luid_0x00000000_0x0001A331_phys_0_eng_10_engtype_Video Codec 0"),
            Some(process_engine(6860, RADEON, 10, "Video Codec 0"))
        );
        assert_eq!(
            parse_engine("pid_4_luid_0x00000000_0x00017DB6_phys_0_eng_14_engtype_Security_1"),
            Some(process_engine(4, RTX, 14, "Security_1"))
        );
        assert_eq!(
            parse_engine("pid_4_luid_0x00000001_0x00000002_phys_1_eng_3_engtype_Copy"),
            Some(process_engine(4, 0x1_0000_0002, 3, "Copy"))
        );
```

Subito prima del test hardware `reads_engine_load_and_memory_on_this_machine` (con i suoi attributi), aggiungi:

```rust
    #[test]
    fn parses_process_memory_instances() {
        assert_eq!(
            parse_process_memory("pid_26328_luid_0x00000000_0x00018036_phys_0"),
            Some((26328, 0x18036))
        );
        assert_eq!(
            parse_process_memory("pid_4_luid_0x00000001_0x00000002_phys_1"),
            Some((4, 0x1_0000_0002))
        );
        for name in [
            "luid_0x00000000_0x00018036_phys_0",
            "pid_x_luid_0x00000000_0x00018036_phys_0",
            "pid_1_luid_0x00000000_0x00018036",
            "pid_1_luid_0x00000000_0x00018036_phys_0_eng_0_engtype_3D",
        ] {
            assert_eq!(parse_process_memory(name), None, "{name}");
        }
    }

    const FFMPEG: u32 = 18796;
    const DWM: u32 = 2096;

    /// hevc_nvenc ffmpeg on the RTX at one tick (spike): 3D 73.7, two NVENC engines ~50.
    fn nvenc_tick() -> Vec<(EngineInstance, f64)> {
        vec![
            (process_engine(FFMPEG, RTX, 0, "3D"), 73.7),
            (process_engine(FFMPEG, RTX, 6, "VideoEncode"), 50.1),
            (process_engine(FFMPEG, RTX, 7, "VideoEncode"), 49.0),
            (process_engine(FFMPEG, RTX, 3, "Copy"), 0.0),
            (process_engine(DWM, RTX, 0, "3D"), 0.0),
            (process_engine(DWM, RTX, 3, "Copy"), 0.0),
            (process_engine(DWM, RADEON, 0, "3D"), 2.5),
        ]
    }

    #[test]
    fn process_load_is_the_busiest_single_engine() {
        let usage = process_usage(Some(&nvenc_tick()), &[], &[]);
        let ffmpeg = &usage[&(RTX, FFMPEG)];
        // Not 99.1 (the two VideoEncode engines summed): consistent with LoadCore.
        assert_eq!(ffmpeg.load, Some(73.7));
        assert_eq!(ffmpeg.engine.as_deref(), Some("3D"));
        let dwm = &usage[&(RTX, DWM)];
        assert_eq!(dwm.load, Some(0.0));
        assert_eq!(dwm.engine, None, "no engine label at 0 %");
        // The same process on another adapter is a separate row.
        assert_eq!(usage[&(RADEON, DWM)].load, Some(2.5));
        assert_eq!(usage.len(), 3);
    }

    #[test]
    fn process_load_is_clamped_and_nan_is_unknown() {
        let rows = vec![
            (process_engine(1, RTX, 0, "3D"), 130.0),
            (process_engine(2, RTX, 0, "3D"), f64::NAN),
            (process_engine(2, RTX, 1, "Copy"), f64::NAN),
            (process_engine(3, RTX, 0, "3D"), f64::NAN),
            (process_engine(3, RTX, 1, "Copy"), 4.0),
        ];
        let usage = process_usage(Some(&rows), &[], &[]);
        assert_eq!(usage[&(RTX, 1)].load, Some(100.0));
        assert_eq!(usage[&(RTX, 2)].load, None, "just appeared: no rate yet");
        assert_eq!(usage[&(RTX, 3)].load, Some(4.0));
        assert_eq!(usage[&(RTX, 3)].engine.as_deref(), Some("Copy"));
    }

    #[test]
    fn process_memory_is_summed_per_process_and_adapter() {
        let dedicated = vec![
            (
                "pid_2096_luid_0x00000000_0x00017DB6_phys_0".to_owned(),
                1_900_000_000.0,
            ),
            (
                "pid_2096_luid_0x00000000_0x00017DB6_phys_1".to_owned(),
                100_000_000.0,
            ),
            (
                "pid_2096_luid_0x00000000_0x0001A331_phys_0".to_owned(),
                11_600_000.0,
            ),
            (
                "pid_7_luid_0x00000000_0x00017DB6_phys_0".to_owned(),
                f64::NAN,
            ),
            ("luid_0x00000000_0x00017DB6_phys_0".to_owned(), 5.0),
        ];
        let shared = vec![(
            "pid_2096_luid_0x00000000_0x00017DB6_phys_0".to_owned(),
            69_000_000.4,
        )];
        // First tick after attach: no engine rows, memory only.
        let usage = process_usage(None, &dedicated, &shared);
        assert_eq!(
            usage[&(RTX, DWM)],
            ProcessUsage {
                load: None,
                engine: None,
                dedicated: Some(2_000_000_000),
                shared: Some(69_000_000),
            }
        );
        assert_eq!(usage[&(RADEON, DWM)].dedicated, Some(11_600_000));
        assert_eq!(usage[&(RADEON, DWM)].shared, None);
        assert!(!usage.contains_key(&(RTX, 7)), "a NaN-only row is no row");
        assert_eq!(usage.len(), 2);
    }

    #[test]
    fn process_rows_are_grouped_by_adapter_and_named() {
        let usage = process_usage(Some(&nvenc_tick()), &[], &[]);
        let rows = process_rows(usage, |pid| match pid {
            DWM => "dwm.exe".to_owned(),
            _ => "ffmpeg.exe".to_owned(),
        });
        let mut rtx: Vec<_> = rows[&RTX]
            .iter()
            .map(|p| (p.pid, p.name.as_str(), p.load_percent))
            .collect();
        rtx.sort_by_key(|r| r.0);
        assert_eq!(
            rtx,
            [
                (DWM, "dwm.exe", Some(0.0)),
                (FFMPEG, "ffmpeg.exe", Some(73.7))
            ]
        );
        assert_eq!(rows[&RADEON].len(), 1);
    }

    #[test]
    fn layer_without_counters_publishes_an_empty_table() {
        let table = GpuProcessTable::new();
        table.set_devices(vec![("gpu/x".to_owned(), RTX)]);
        table.publish(process_rows(
            process_usage(Some(&nvenc_tick()), &[], &[]),
            |_| String::new(),
        ));
        assert!(!table.processes("gpu/x").is_empty());
        let mut layer = PdhLayer::new(table.clone());
        assert_eq!(layer.sample(), Ok(vec![]));
        assert!(table.processes("gpu/x").is_empty());
    }
```

In coda al modulo `tests`, dopo `reads_engine_load_and_memory_on_this_machine`, aggiungi il test hardware:

```rust
    #[test]
    #[ignore = "requires real Windows hardware"]
    fn publishes_per_process_rows_on_this_machine() {
        let adapters = super::super::enumerate::enumerate().expect("enumerate");
        let rtx = adapters
            .iter()
            .find(|a| a.vendor_id == 0x10DE)
            .expect("NVIDIA adapter");
        let table = GpuProcessTable::new();
        table.set_devices(vec![("rtx".to_owned(), rtx.luid)]);
        let mut layer = PdhLayer::new(table.clone());
        layer.attach(&adapters);

        layer.sample().expect("first sample");
        let first = table.processes("rtx");
        assert!(!first.is_empty(), "the desktop always uses the dGPU");
        assert!(
            first.iter().all(|p| p.load_percent.is_none()),
            "no load on the first tick after attach"
        );
        let dwm = first
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case("dwm.exe"))
            .expect("dwm.exe is named although OpenProcess fails for it");
        assert!(dwm.dedicated_bytes.is_some_and(|b| b > 0), "{dwm:?}");

        std::thread::sleep(std::time::Duration::from_millis(1_100));
        layer.sample().expect("second sample");
        let second = table.processes("rtx");
        assert!(second.len() <= 20);
        for p in &second {
            println!(
                "{:>6} {:<28} load {:?} {:?} ded {:?} shr {:?}",
                p.pid, p.name, p.load_percent, p.engine, p.dedicated_bytes, p.shared_bytes
            );
            if let Some(load) = p.load_percent {
                assert!((0.0..=100.0).contains(&load), "{p:?}");
                assert_eq!(p.engine.is_some(), load > 0.0, "{p:?}");
            }
        }
        assert!(second.iter().any(|p| p.load_percent.is_some()));
    }
```

- [ ] **Step 5: Esegui i test e verifica che falliscano**

Esegui: `cargo test -p oma-win --lib gpu::pdh`
Risultato atteso: errori di compilazione, tra cui `struct `EngineInstance` has no field named `pid``, `cannot find function `parse_process_memory``, `cannot find function `process_usage``, `cannot find struct `ProcessUsage``, `cannot find type `GpuProcessTable`` e `no function or associated item named `new` found for struct `PdhLayer``.

- [ ] **Step 6: Implementa la parte per processo del layer PDH**

In `crates/oma-win/src/gpu/pdh.rs`:

Commento del modulo e import (sostituiscono le righe esistenti):

```rust
//! GPU engine load and adapter memory from the PDH "GPU Engine" and
//! "GPU Adapter Memory" counters, aggregated the way Task Manager does, plus
//! the per-process table from the same engine rows and "GPU Process Memory".

use std::collections::{BTreeMap, BTreeSet, HashMap};
```

```rust
use super::layer::{GpuLayer, Readings};
use super::processes::{GpuProcess, GpuProcessTable};
use super::procname::ProcessNames;
use crate::pdh::{Counter, PdhError, Query};
```

Dopo la costante `SHARED`:

```rust
const PROCESS_DEDICATED: &str = r"\GPU Process Memory(*)\Dedicated Usage";
const PROCESS_SHARED: &str = r"\GPU Process Memory(*)\Shared Usage";
```

`EngineInstance` riceve il pid come primo campo:

```rust
pub(crate) struct EngineInstance {
    /// Process owning the instance (4 = System).
    pub pid: u32,
    pub luid: u64,
```

e `parse_engine` lo conserva invece di scartarlo:

```rust
    let (pid, rest) = head.strip_prefix("pid_")?.split_once("_luid_")?;
    let pid = pid.parse::<u32>().ok()?;
    let (luid, rest) = parse_luid(rest)?;
    let (phys, engine) = rest.strip_prefix("_phys_")?.split_once("_eng_")?;
    phys.parse::<u32>().ok()?;
    Some(EngineInstance {
        pid,
        luid,
        engine: engine.parse().ok()?,
        engtype: engtype.to_owned(),
    })
```

(`aggregate` e `supported_fields` ignorano il pid: il carico dell'adattatore non cambia.)

Subito prima di `/// Maps a driver engine name to its load field …` (`fn classify`):

```rust
/// `pid_26328_luid_0x00000000_0x00018036_phys_0` → (pid, LUID). Adapter-wide
/// instances (no `pid_`) are rejected.
pub(crate) fn parse_process_memory(instance: &str) -> Option<(u32, u64)> {
    let (pid, rest) = instance.strip_prefix("pid_")?.split_once("_luid_")?;
    let (luid, rest) = parse_luid(rest)?;
    rest.strip_prefix("_phys_")?.parse::<u32>().ok()?;
    Some((pid.parse().ok()?, luid))
}
```

Sostituisci `struct Counters { … }` con le funzioni per processo seguite dalla struct estesa:

```rust
/// GPU use of one process on one adapter during a tick.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ProcessUsage {
    pub load: Option<f64>,
    pub engine: Option<String>,
    pub dedicated: Option<u64>,
    pub shared: Option<u64>,
}

/// Adds per-process memory rows (bytes) to `usage`; `phys_N` rows of the same
/// process and adapter are summed, NaN and negative rows ignored.
fn add_memory(
    usage: &mut BTreeMap<(u64, u32), ProcessUsage>,
    rows: &[(String, f64)],
    slot: fn(&mut ProcessUsage) -> &mut Option<u64>,
) {
    for (instance, value) in rows {
        let Some((pid, luid)) = parse_process_memory(instance) else {
            continue;
        };
        if !value.is_finite() || *value < 0.0 {
            continue;
        }
        let bytes = slot(usage.entry((luid, pid)).or_default());
        *bytes = Some(bytes.unwrap_or(0) + value.round() as u64);
    }
}

/// Per (LUID, pid) usage for a tick (decision D5). The load of a process is its
/// busiest single engine (not a sum per engine type, which can exceed the
/// adapter's own LoadCore), clamped to 0..=100; `engine` names that engine only
/// when the load is above 0. `engines` is `None` when engine load is not
/// available this tick: every load is then unknown. A process whose engine rows
/// are all NaN (it just appeared) has an unknown load too.
pub(crate) fn process_usage(
    engines: Option<&[(EngineInstance, f64)]>,
    dedicated: &[(String, f64)],
    shared: &[(String, f64)],
) -> BTreeMap<(u64, u32), ProcessUsage> {
    let mut usage: BTreeMap<(u64, u32), ProcessUsage> = BTreeMap::new();
    for (instance, value) in engines.unwrap_or_default() {
        let entry = usage.entry((instance.luid, instance.pid)).or_default();
        if !value.is_finite() {
            continue;
        }
        let load = value.clamp(0.0, 100.0);
        if entry.load.is_none_or(|busiest| load > busiest) {
            entry.load = Some(load);
            entry.engine = (load > 0.0).then(|| instance.engtype.clone());
        }
    }
    add_memory(&mut usage, dedicated, |u| &mut u.dedicated);
    add_memory(&mut usage, shared, |u| &mut u.shared);
    usage
}

/// Table rows grouped by adapter LUID, named through `name`.
pub(crate) fn process_rows(
    usage: BTreeMap<(u64, u32), ProcessUsage>,
    name: impl Fn(u32) -> String,
) -> HashMap<u64, Vec<GpuProcess>> {
    let mut rows: HashMap<u64, Vec<GpuProcess>> = HashMap::new();
    for ((luid, pid), u) in usage {
        rows.entry(luid).or_default().push(GpuProcess {
            pid,
            name: name(pid),
            load_percent: u.load,
            engine: u.engine,
            dedicated_bytes: u.dedicated,
            shared_bytes: u.shared,
        });
    }
    rows
}

struct Counters {
    query: Query,
    engine: Option<Counter>,
    dedicated: Option<Counter>,
    shared: Option<Counter>,
    process_dedicated: Option<Counter>,
    process_shared: Option<Counter>,
}
```

In `Counters::open`, aggiungi i due contatori (se non si aggiungono, il layer registra un warning e la tabella resta senza memoria):

```rust
        let engine = add(ENGINE);
        let dedicated = add(DEDICATED);
        let shared = add(SHARED);
        let process_dedicated = add(PROCESS_DEDICATED);
        let process_shared = add(PROCESS_SHARED);
        Ok(Self {
            query,
            engine,
            dedicated,
            shared,
            process_dedicated,
            process_shared,
        })
```

Nell'`impl Counters`, subito prima di `fn memory`:

```rust
    /// Rows of an optional per-process counter. A failed read counts as no rows,
    /// so the process table never costs the adapter readings.
    fn process_rows(&self, counter: Option<Counter>) -> Vec<(String, f64)> {
        let Some(counter) = counter else {
            return Vec::new();
        };
        self.query.array(counter).unwrap_or_else(|e| {
            tracing::debug!(error = %e, "GPU process memory counter read failed");
            Vec::new()
        })
    }
```

La struct `PdhLayer` riceve due campi; aggiungi il costruttore e sposta il corpo di `sample` in `read`:

```rust
/// PDH layer. Owns its own query so a slow or failing GPU counter set never
/// affects the CPU provider's query.
#[derive(Default)]
pub(crate) struct PdhLayer {
    counters: Option<Counters>,
    /// (LUID, supported fields) per adapter of the last attach.
    adapters: Vec<(u64, BTreeSet<GpuField>)>,
    /// Set by `attach`, consumed by the next `sample` (CpuProvider's rule).
    fresh: bool,
    /// Where each tick's per-process rows are published.
    processes: GpuProcessTable,
    names: ProcessNames,
}

impl PdhLayer {
    pub(crate) fn new(processes: GpuProcessTable) -> Self {
        Self {
            processes,
            ..Self::default()
        }
    }

    /// One tick: adapter readings, and the per-process table published as a side effect.
    fn read(&mut self, fresh: bool) -> Result<Vec<Readings>, ProviderError> {
        let Some(counters) = self.counters.as_mut() else {
            self.processes.publish(HashMap::new());
            return Ok(vec![Readings::new(); self.adapters.len()]);
        };
        counters.query.collect()?;
        let engines: Option<Vec<(EngineInstance, f64)>> = match counters.engine {
            Some(counter) if !fresh => {
                let rows: Vec<_> = counters
                    .query
                    .array(counter)?
                    .into_iter()
                    .filter_map(|(name, value)| parse_engine(&name).map(|e| (e, value)))
                    .collect();
                (!rows.is_empty()).then_some(rows)
            }
            _ => None,
        };
        let dedicated = counters.memory(counters.dedicated)?;
        let shared = counters.memory(counters.shared)?;
        let usage = process_usage(
            engines.as_deref(),
            &counters.process_rows(counters.process_dedicated),
            &counters.process_rows(counters.process_shared),
        );
        self.names
            .update(&usage.keys().map(|&(_, pid)| pid).collect());
        let names = &self.names;
        self.processes
            .publish(process_rows(usage, |pid| names.name(pid)));
        Ok(self
            .adapters
            .iter()
            .map(|(luid, supported)| {
                adapter_readings(engines.as_deref(), &dedicated, &shared, *luid, supported)
            })
            .collect())
    }
```

(`fn open(adapters: &[Adapter])` resta com'è, dentro lo stesso `impl PdhLayer`, dopo `read`.)

Nell'`impl GpuLayer for PdhLayer`, `sample` diventa:

```rust
    fn sample(&mut self) -> Result<Vec<Readings>, ProviderError> {
        // Utilization Percentage is a rate counter: the collect right after
        // attach spans a few milliseconds and yields noise, so like
        // CpuProvider the first sample only primes it (memory is still read).
        let fresh = std::mem::replace(&mut self.fresh, false);
        let result = self.read(fresh);
        if result.is_err() {
            // No stale rows while the counters fail.
            self.processes.publish(HashMap::new());
        }
        result
    }
```

- [ ] **Step 7: Collega la tabella al provider e a `default_providers`**

In `crates/oma-win/src/gpu/mod.rs`:

Campo nuovo in fondo a `pub struct GpuProvider` (dopo `known_pci` del Task 6):

```rust
    /// PCI address last seen per LUID, kept across discovers (see `restore_pci`).
    known_pci: HashMap<u64, PciAddress>,
    /// Per-process table filled by the PDH layer; `discover` maps device ids to LUIDs.
    processes: GpuProcessTable,
}
```

In `with_layers`:

```rust
            known_pci: HashMap::new(),
            processes: GpuProcessTable::default(),
        }
```

`GpuProvider::new` (sostituisce la funzione intera):

```rust
    /// The real provider: DXGI/DXCore/D3DKMT enumeration, the base layers
    /// (D3DKMT, DXGI, PDH, PnP) always on, and the vendor libraries (NVML,
    /// NVAPI, ADL, IGCL) loaded on the first discover that sees `switch` on.
    /// The PDH layer publishes the per-process GPU usage into `processes`.
    pub fn new(switch: VendorSwitch, processes: GpuProcessTable) -> Self {
        let mut provider = Self::with_layers(
            Box::new(enumerate::enumerate),
            vec![
                Box::new(d3dkmt::D3dkmtLayer::default()),
                Box::new(dxgi::DxgiLayer::default()),
                Box::new(pdh::PdhLayer::new(processes.clone())),
                Box::new(pnp::PnpLayer::default()),
            ],
            Box::new(load_vendor_layers),
            switch,
        );
        provider.processes = processes;
        provider
    }
```

Prima di modificare `discover`, sposta il corpo attuale di `Provider::poll` in un metodo privato `GpuProvider::poll_inner(&mut self) -> Result<Vec<Option<f64>>, ProviderError>` dentro `impl GpuProvider`. Tutte le uscite anticipate (topologia, switch, errori dei layer) restano nel corpo spostato. Il metodo del trait diventa:

```rust
    fn poll(&mut self) -> Result<Vec<Option<f64>>, ProviderError> {
        let result = self.poll_inner();
        if result.is_err() {
            // Also clear on failures before PDH is reached, and on Rediscover.
            self.processes.publish(HashMap::new());
        }
        result
    }
```

All'inizio di `discover`, **prima** di `self.state = State::default()` e della chiamata fallibile a `enumerate`, aggiungi:

```rust
        self.processes.publish(HashMap::new());
        self.processes.set_devices(Vec::new());
```

Così durante il backoff dopo un errore di enumerazione non vengono esposti processi vecchi. La sola pulizia in `PdhLayer::sample` non copre queste uscite del provider.

In `discover`, raccogli le coppie (id, LUID) e consegnale alla tabella:

```rust
        let mut inventory = Inventory::default();
        let mut slots = Vec::new();
        let mut luids = Vec::new();
        let mut ordinal = 0;
```

```rust
            inventory.devices.push(gpu);
            luids.push((id.clone(), adapter.luid));
```

```rust
        let failing = vec![false; layers.len()];
        self.processes.set_devices(luids);
        self.state = State {
```

Nel modulo `tests` di `mod.rs`, subito prima di `fn restore_pci_records_and_restores_by_luid` (Task 6):

```rust
    #[test]
    fn discover_maps_device_ids_to_luids_for_the_process_table() {
        let (mut p, _) = provider(
            vec![nvidia(), amd_igpu()],
            Vec::new(),
            Vec::new(),
            &VendorSwitch::new(false),
        );
        let table = GpuProcessTable::new();
        p.processes = table.clone();
        p.discover().unwrap();
        let dwm = GpuProcess {
            pid: 2096,
            name: "dwm.exe".to_owned(),
            load_percent: Some(1.0),
            engine: Some("3D".to_owned()),
            dedicated_bytes: Some(1 << 30),
            shared_bytes: None,
        };
        table.publish(HashMap::from([(nvidia().luid, vec![dwm.clone()])]));
        assert_eq!(table.processes("gpu/pci-0000:01:00.0"), vec![dwm]);
        assert!(table.processes("gpu/pci-0000:11:00.0").is_empty());
        assert!(table.processes("gpu/pci-0000:02:00.0").is_empty());
    }

    #[test]
    fn provider_enumeration_failures_clear_process_rows() {
        for fail_during_discover in [false, true] {
            let (mut p, _) = provider(
                vec![nvidia()], Vec::new(), Vec::new(), &VendorSwitch::new(false),
            );
            let table = GpuProcessTable::new();
            p.processes = table.clone();
            p.discover().unwrap();
            table.publish(HashMap::from([(nvidia().luid, vec![GpuProcess {
                pid: 99, name: "old.exe".into(), load_percent: Some(50.0),
                engine: Some("3D".into()), dedicated_bytes: None, shared_bytes: None,
            }])]));
            assert_eq!(table.processes("gpu/pci-0000:01:00.0").len(), 1);
            p.enumerate = Box::new(|| Err(ProviderError::Failed("enumeration failed".into())));
            if fail_during_discover {
                assert!(p.discover().is_err());
            } else {
                p.state.topology_checked = None; // fail before the PDH layer runs
                assert!(p.poll().is_err());
            }
            assert!(table.processes("gpu/pci-0000:01:00.0").is_empty());
        }
    }
```

In `crates/oma-win/src/lib.rs`, `default_providers` diventa:

```rust
/// Every unprivileged Windows provider, in display order. `vendor` is the
/// safe-mode switch for the GPU vendor libraries (spec §8); `processes`
/// receives the per-process GPU usage (read by the shell's `get_gpu_processes`).
pub fn default_providers(
    vendor: gpu::VendorSwitch,
    processes: gpu::GpuProcessTable,
) -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(cpu::CpuProvider::new()),
        Box::new(gpu::GpuProvider::new(vendor, processes)),
```

(Le righe successive dell'elenco non cambiano.)

In `crates/oma-win/tests/providers.rs`, l'import diventa `use oma_win::gpu::{GpuProcessTable, GpuProvider, VendorSwitch};`. Poi sostituisci **ogni** `GpuProvider::new(X)` con `GpuProvider::new(X, GpuProcessTable::new())`: sono cinque chiamate, tre del M2 e due del Task 6 (`VendorSwitch::new(true)` due volte, `VendorSwitch::new(false)` due volte, `switch.clone()` una volta). Subito prima di `fn gpu_provider_loads_vendor_libraries_when_reenabled` (con i suoi attributi), aggiungi:

```rust
#[test]
#[ignore = "requires real Windows hardware"]
fn gpu_provider_publishes_per_process_usage() {
    let _serial = GPU_TESTS.lock().unwrap_or_else(PoisonError::into_inner);
    let processes = GpuProcessTable::new();
    let mut p = GpuProvider::new(VendorSwitch::new(true), processes.clone());
    discover_and_poll(&mut p);

    let rows = processes.processes(NVIDIA);
    assert!(!rows.is_empty() && rows.len() <= 20, "{} rows", rows.len());
    for pair in rows.windows(2) {
        let load = |i: usize| pair[i].load_percent.unwrap_or(-1.0);
        assert!(load(0) >= load(1), "sorted by load: {pair:?}");
    }
    let dwm = rows
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case("dwm.exe"))
        .expect("dwm.exe uses the primary GPU");
    assert!(dwm.dedicated_bytes.is_some_and(|b| b > 0), "{dwm:?}");
    assert!(dwm.load_percent.is_some(), "loads exist from the second poll");
    assert!(processes.processes("gpu/pci-0000:99:00.0").is_empty());
}
```

- [ ] **Step 8: Esegui i test di `oma-win` e verifica che passino**

Esegui: `cargo test -p oma-win`
Risultato atteso: tutti OK. In `gpu::pdh` passano i test M2 (con i pid negli attesi di `parses_engine_instances`) e i sei test nuovi (`parses_process_memory_instances`, `process_load_is_the_busiest_single_engine`, `process_load_is_clamped_and_nan_is_unknown`, `process_memory_is_summed_per_process_and_adapter`, `process_rows_are_grouped_by_adapter_and_named`, `layer_without_counters_publishes_an_empty_table`). In `gpu::tests` passano `discover_maps_device_ids_to_luids_for_the_process_table` e `provider_enumeration_failures_clear_process_rows`. `labels` passa; i test di `providers` sono ignorati. `app/src-tauri` non compila ancora con la nuova firma: lo sistema lo Step 9.

- [ ] **Step 9: Comando `get_gpu_processes` nella shell**

In `app/src-tauri/src/commands.rs`, le riesportazioni in testa al file diventano:

```rust
#[cfg(not(windows))]
pub use no_gpu_processes::{GpuProcess, GpuProcessTable};
#[cfg(not(windows))]
pub use no_vendor_libraries::VendorSwitch;
#[cfg(windows)]
pub use oma_win::gpu::{GpuProcess, GpuProcessTable, VendorSwitch};
```

Subito prima del modulo `no_vendor_libraries` (con il suo commento `/// Off Windows there are no GPU vendor libraries…`), aggiungi:

```rust
/// Per-process GPU usage, published by the GPU provider every tick (decision D5).
pub struct GpuProcessState(pub GpuProcessTable);

/// Processes using GPU `device_id` (JS argument `deviceId`): busiest first, at
/// most 20 rows; an unknown device gives an empty list.
#[tauri::command(async)]
pub fn get_gpu_processes(state: State<'_, GpuProcessState>, device_id: String) -> Vec<GpuProcess> {
    state.0.processes(&device_id)
}

/// Off Windows no provider publishes GPU processes: the table is always empty.
#[cfg(not(windows))]
mod no_gpu_processes {
    /// Never constructed off Windows; any serializable type fits the empty reply.
    pub type GpuProcess = serde_json::Value;

    #[derive(Clone, Default)]
    pub struct GpuProcessTable;

    impl GpuProcessTable {
        pub fn new() -> Self {
            Self
        }

        pub fn processes(&self, _device_id: &str) -> Vec<GpuProcess> {
            Vec::new()
        }
    }
}
```

In `app/src-tauri/src/main.rs`:

```rust
use crate::commands::{
    GpuProcessState, GpuProcessTable, StartupState, StartupStatus, VendorSwitch,
};
```

(sostituisce `use crate::commands::{StartupState, StartupStatus, VendorSwitch};`: il Task 3 non ha cambiato questo import.)

```rust
fn providers(vendor: VendorSwitch, processes: GpuProcessTable) -> Vec<Box<dyn Provider>> {
    #[cfg(windows)]
    {
        oma_win::default_providers(vendor, processes)
    }
    #[cfg(not(windows))]
    {
        let _ = (vendor, processes);
        Vec::new()
    }
}
```

In `main()`:

```rust
    let switch = VendorSwitch::new(!status.safe_mode);
    let processes = GpuProcessTable::new();
    let engine = Arc::new(Mutex::new(Engine::new(
        providers(switch.clone(), processes.clone()),
        history_capacity(SAMPLE_INTERVAL),
    )));
```

e, nel builder, dopo `.manage(StartupState::new(switch, status))`:

```rust
        .manage(StartupState::new(switch, status))
        .manage(GpuProcessState(processes))
```

Negli stessi tre elenchi del Task 3 (scritti un elemento per riga) aggiungi il comando nuovo **subito dopo** la voce di `get_session`; le altre righe non cambiano.

In `tauri::generate_handler![…]` di `main.rs` l'elenco risultante è:

```rust
        .invoke_handler(tauri::generate_handler![
            commands::get_schema,
            commands::get_history,
            commands::get_stats,
            commands::reset_stats,
            commands::get_session,
            commands::get_gpu_processes,
            commands::get_startup_status,
            commands::enable_vendor_libraries,
        ])
```

In `app/src-tauri/build.rs`, nell'array di `AppManifest::new().commands(&[…])`:

```rust
            "get_session",
            "get_gpu_processes",
            "get_startup_status",
```

In `app/src-tauri/capabilities/default.json`, nell'elenco `permissions`:

```json
    "allow-get-session",
    "allow-get-gpu-processes",
    "allow-get-startup-status",
```

- [ ] **Step 10: Compila la shell e rigenera il permesso**

Esegui: `cargo test --workspace`
Risultato atteso: tutti OK. La build di `oma-app` genera `app/src-tauri/permissions/autogenerated/get_gpu_processes.toml`, con `identifier = "allow-get-gpu-processes"` e `commands.allow = ["get_gpu_processes"]`: il file va nel commit. Senza la voce in `build.rs` il permesso `allow-get-gpu-processes` non esiste e la build di `oma-app` fallisce sulla capability.

- [ ] **Step 11: Verifica su hardware reale**

Esegui: `cargo test -p oma-win -- --include-ignored --nocapture`
Risultato atteso: tutti OK. Durante la stesura, su questa macchina:
- `gpu::procname::tests::toolhelp_names_the_desktop_window_manager`: il processo di test e `dwm.exe` compaiono nello snapshot;
- `gpu::pdh::tests::publishes_per_process_rows_on_this_machine`: al primo campione ci sono righe senza carico e `dwm.exe` ha memoria dedicata > 0. Il secondo campione stampa righe come:
  ```
   26328 chrome.exe                   load Some(12.87683210628946) Some("VideoDecode") ded Some(194756608) shr Some(37769216)
       4 System                       load Some(0.36702131979328995) Some("Copy") ded Some(4235264) shr Some(262144)
    2096 dwm.exe                      load Some(0.23438508092714525) Some("3D") ded Some(2458738688) shr Some(7995392)
  ```
  I pid e i valori cambiano da un'esecuzione all'altra; senza video in riproduzione chrome può mancare o stare a 0 %;
- `gpu_provider_publishes_per_process_usage` (in `tests/providers.rs`): con il provider reale e due poll, per `gpu/pci-0000:01:00.0` escono da 1 a 20 righe ordinate per carico, `dwm.exe` ha memoria dedicata > 0 e un carico; un id sconosciuto dà una lista vuota;
- i test GPU del Task 6 e del M2 continuano a passare con la nuova firma.

Controllo incrociato facoltativo: con un video in riproduzione in un browser, la riga del browser mostra "VideoDecode" con un carico vicino a quello della colonna "GPU" di Task Manager (scheda Dettagli, colonne "GPU" e "Motore GPU"). Non deve per forza coincidere: Task Manager somma per motore.

- [ ] **Step 12: Lint e commit**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git add Cargo.lock crates/oma-win/Cargo.toml crates/oma-win/src/lib.rs crates/oma-win/src/gpu/mod.rs crates/oma-win/src/gpu/pdh.rs crates/oma-win/src/gpu/processes.rs crates/oma-win/src/gpu/procname.rs crates/oma-win/tests/providers.rs app/src-tauri/src/commands.rs app/src-tauri/src/main.rs app/src-tauri/build.rs app/src-tauri/capabilities/default.json app/src-tauri/permissions/autogenerated/get_gpu_processes.toml
git commit -m "feat(win): per-process GPU usage table and get_gpu_processes command"
```

---

---

### Task 9: UI: fondamenta della vista Avanzata (uPlot, tipi, backend, `formatValue`, chiavi i18n di base, token delle serie)

**File:**
- Crea:
  - `app/src/lib/backend/decimate.ts` (inviluppo min/max, stessa regola di `History::window_decimated`)
  - `app/src/lib/backend/mockStats.ts` (min/max/media del backend mock)
- Modifica:
  - `app/package.json`, `app/pnpm-lock.yaml` (dipendenza `uplot` 1.6.32, versione esatta)
  - `app/src/lib/types.ts` (`SensorKind` `'link'`, `Unit` `'pcie_generation' | 'lanes'`, `Source` `'pnp'`, `SensorStats`, `StatsReply`, `Session`, `GpuProcess`)
  - `app/src/lib/backend/backend.ts`, `app/src/lib/backend/tauri.ts`, `app/src/lib/backend/mock.ts` (nuovi metodi, storico fino a 3600 s, decimazione, statistiche, sessione, processi GPU)
  - `app/src/lib/format.ts` (`formatValue` per ogni `Unit`; i formattatori accettano un `locale: string`)
  - `app/src/lib/i18n/en.json`, `app/src/lib/i18n/it.json` (`status.stale`, `flag.on/off`, `source.<nome>` per ogni `Source`, `property.<chiave>`)
  - `app/src/styles/theme.css` (token `--series-5` … `--series-8`)
- Test:
  - `app/src/test/fake-backend.ts` (onora `seconds`, registra le chiamate, `stats`/`session`/`gpuProcesses` impostabili)
  - Crea: `app/src/lib/backend/decimate.test.ts`, `app/src/lib/backend/tauri.test.ts`, `app/src/test/fake-backend.test.ts`
  - Modifica: `app/src/lib/format.test.ts`, `app/src/lib/backend/mock.test.ts`, `app/src/lib/i18n/i18n.test.ts`

**Interfacce:**
- Usa:
  - Task 3: comandi `get_history(ids, seconds, max_points: Option<u32>)` (argomento JS `maxPoints`; assente = finestra grezza; `Some(n)` = `window_decimated(.., n.clamp(2, 3600))`), `get_stats(ids) -> { revision, stats: (SensorStats | null)[] }`, `reset_stats(ids)`, `get_session() -> { startedAtMs, intervalMs }`.
  - Task 6: `SensorKind::Link` → `"link"`, `Unit::PcieGeneration` → `"pcie_generation"`, `Unit::Lanes` → `"lanes"`, `Source::Pnp` → `"pnp"`; chiavi delle proprietà dei device GPU (`pcieMaxGen`, `pcieMaxWidth`, `powerLimitMinW`, `powerLimitMaxW`, `powerLimitDefaultW`, `tempSlowdownC`, `tempShutdownC`, `tempMaxC`).
  - Task 5: proprietà dei dischi `tempWarningC`, `tempCriticalC`.
  - Task 8: comando `get_gpu_processes(device_id)` (argomento JS `deviceId`) → `GpuProcess[]` in camelCase, ordinati per carico e poi per memoria dedicata, al massimo 20 righe.
  - Task 2: regola dell'inviluppo di `History::window_decimated` (replicata dal mock).
- Produce (usati dai Task 10–13):
  - `types.ts`: `SensorStats { min, max, avg, count }`, `StatsReply { revision, stats: (SensorStats | null)[] }`, `Session { startedAtMs: number | null; intervalMs }`, `GpuProcess { pid, name, loadPercent: number | null, engine: string | null, dedicatedBytes: number | null, sharedBytes: number | null }`;
  - `Backend`:
    ```ts
    getHistory(ids: string[], seconds: number, maxPoints?: number): Promise<HistorySeed>;
    getStats(ids: string[]): Promise<StatsReply>;
    resetStats(ids: string[]): Promise<void>;
    getSession(): Promise<Session>;
    getGpuProcesses(deviceId: string): Promise<GpuProcess[]>;
    ```
  - `decimateWindow(window: HistoryWindow, maxPoints: number): HistoryWindow` (`lib/backend/decimate.ts`);
  - mock: `MOCK_HISTORY_SECONDS = 3600`, `mockGpuProcesses(t)`, `sortGpuProcesses(list)`; la GPU mock `gpu/pci-0000:01:00.0` ha 6 processi, gli altri device nessuno;
  - `FakeBackend` (per i test dei Task 10–13):
    - `history: HistoryWindow` (dal più vecchio); `getHistory(ids, seconds, maxPoints)` restituisce gli ultimi `seconds` campioni (1 campione = 1 s) e **non** decima;
    - `historyCalls: { ids: string[]; seconds: number; maxPoints: number | undefined }[]`;
    - `stats: Record<string, SensorStats>` (id senza voce → `null`); `statsCalls: string[][]`; `resetCalls: string[][]`; `resetStats(ids)` cancella anche quelle voci da `stats`;
    - `session: Session` (predefinita `{ startedAtMs: null, intervalMs: 1000 }`);
    - `gpuProcesses: GpuProcess[]`, restituita (copia) per qualsiasi device; `gpuProcessCalls: string[]`;
  - `formatValue(value: number | null, unit: Unit, locale: string, t: Translate, opts?: FormatOptions): string`, con `FormatOptions { rate?: 'bits' | 'bytes' }` (predefinito `'bytes'`; `'bits'` cambia solo `bytes_per_second`, che diventa `formatRate(value, 'bits', locale)`: le pagine di rete dei Task 12 e 13 lo passano per mostrare la stessa unità della vista Semplificata);
  - chiavi i18n `status.stale`, `flag.on`, `flag.off`, `source.<nome>` (tutte le 11 fonti), `property.<chiave>` (12 chiavi);
  - token CSS `--series-5` … `--series-8`.

**Comportamento:**
- **Storico nel mock.** `getHistory` rispetta `seconds` fino a 3600 (prima era limitato a 300 s), così `pnpm dev` mostra anche le finestre 30m e 1h. Con `maxPoints` restituisce l'inviluppo min/max di `decimateWindow`, con `maxPoints` limitato a 2..3600 come nella shell.
- **Regola dell'inviluppo** (identica al core, decisione D3): se i campioni sono al massimo `maxPoints`, oppure `maxPoints < 2`, la finestra resta invariata. Altrimenti:
  - i campioni si dividono in `floor(maxPoints / 2)` bucket consecutivi bilanciati, con confini `floor(b * n / buckets)` e `floor((b + 1) * n / buckets)`; le dimensioni differiscono al massimo di un campione;
  - ogni bucket produce due righe: (primo timestamp, minimo per serie) e (ultimo timestamp, massimo per serie);
  - una serie con anche un solo valore assente o non finito nel bucket produce `null` due volte: il buco si amplia al bucket; i picchi si conservano nei bucket interamente validi. I timestamp sono confini dell'inviluppo, non gli istanti reali degli estremi.
- **Statistiche nel mock.** Min/max/media si accumulano sui tick emessi, dall'avvio (decisione D1). `resetStats` azzera gli id indicati.
- **Sessione nel mock.** `startedAtMs` è il timestamp del primo tick (`null` prima); `intervalMs` è l'intervallo del mock.
- **`formatValue`**, con "—" per `null` e per i valori non finiti:

  | Unit | Esempio (`en`) |
  |---|---|
  | `celsius`, `percent`, `megahertz`, `watt`, `bytes` | come i formattatori esistenti: `54 °C`, `35%`, `2.52 GHz`, `148 W`, `17.9 GB` |
  | `volt` | 3 decimali: `1.075 V` (`it`: `1,075 V`) |
  | `ampere` | 1 decimale: `12.3 A` |
  | `rpm` | separatore delle migliaia della lingua: `1,650 RPM` (`it`: `12.000 RPM`; per 4 cifre l'italiano non raggruppa, `1650 RPM`) |
  | `bytes_per_second` | `120 MB/s`; con `{ rate: 'bits' }` in bit, come il riquadro di rete della vista Semplificata: `48 Mbit/s` |
  | `bits_per_second` | passi decimali in bit: `1.0 Gbit/s` |
  | `joule` | passi decimali J, kJ, MJ, GJ: `950 J`, `12.3 kJ` |
  | `boolean` | `t('flag.on')` se ≥ 0,5, altrimenti `t('flag.off')`: "Active"/"No", "Attivo"/"No" |
  | `pcie_generation` | `Gen 4` |
  | `lanes` | `x16` |

  Il `switch` è esaustivo: una nuova `Unit` senza caso non compila. Decisione del coordinatore: sulle pagine di rete della vista Avanzata il traffico usa la stessa unità della vista Semplificata (bit/s). Per questo `formatValue` accetta un quinto argomento facoltativo `{ rate: 'bits' }`, che i Task 12 e 13 passano quando la pagina è di tipo `network`.
- **Tauri.** Senza `maxPoints` la chiamata resta `{ ids, seconds }`, cioè senza chiave, e Rust riceve `None`.

- [ ] **Step 1: Scrivi i test (falliscono)**

`app/src/test/fake-backend.ts`, file completo. Implementa i nuovi metodi di `Backend`, onora `seconds` e registra le chiamate per i test dei Task 10–13:

```ts
import type { Backend, Unsubscribe } from '../lib/backend/backend';
import type {
  GpuProcess,
  HistorySeed,
  HistoryWindow,
  Schema,
  SensorStats,
  Session,
  Snapshot,
  StartupStatus,
  StatsReply,
} from '../lib/types';

export interface HistoryCall {
  ids: string[];
  seconds: number;
  maxPoints: number | undefined;
}

/** Hand-driven backend for tests: emit events explicitly. */
export class FakeBackend implements Backend {
  schema: Schema;
  /** Oldest first; `getHistory(ids, seconds)` returns the last `seconds` samples (1 sample = 1 s). */
  history: HistoryWindow = { timestampsMs: [], series: [] };
  historyCalls: HistoryCall[] = [];
  schemaCalls = 0;
  startup: StartupStatus = { safeMode: false, reason: null, crashModule: null };
  enableCalls = 0;
  /** Stats by sensor id; ids without an entry read as null. `resetStats` deletes entries. */
  stats: Record<string, SensorStats> = {};
  statsCalls: string[][] = [];
  resetCalls: string[][] = [];
  session: Session = { startedAtMs: null, intervalMs: 1000 };
  /** Returned (copied) for every device id; `gpuProcessCalls` records the ids asked for. */
  gpuProcesses: GpuProcess[] = [];
  gpuProcessCalls: string[] = [];
  #schemaListeners = new Set<(s: Schema) => void>();
  #snapshotListeners = new Set<(s: Snapshot) => void>();

  constructor(schema: Schema) {
    this.schema = schema;
  }

  async getSchema(): Promise<Schema> {
    this.schemaCalls++;
    return this.schema;
  }

  async getHistory(ids: string[], seconds: number, maxPoints?: number): Promise<HistorySeed> {
    this.historyCalls.push({ ids, seconds, maxPoints });
    const keep = Math.max(0, Math.floor(seconds));
    const from = Math.max(0, this.history.timestampsMs.length - keep);
    return {
      revision: this.schema.revision,
      seq: 0,
      timestampsMs: this.history.timestampsMs.slice(from),
      series: ids.map((_, i) => (this.history.series[i] ?? []).slice(from)),
    };
  }

  async onSchema(cb: (s: Schema) => void): Promise<Unsubscribe> {
    this.#schemaListeners.add(cb);
    return () => this.#schemaListeners.delete(cb);
  }

  async onSnapshot(cb: (s: Snapshot) => void): Promise<Unsubscribe> {
    this.#snapshotListeners.add(cb);
    return () => this.#snapshotListeners.delete(cb);
  }

  async getStartupStatus(): Promise<StartupStatus> {
    return this.startup;
  }

  async enableVendorLibraries(): Promise<StartupStatus> {
    this.enableCalls++;
    this.startup = { ...this.startup, safeMode: false };
    return this.startup;
  }

  async getStats(ids: string[]): Promise<StatsReply> {
    this.statsCalls.push(ids);
    return { revision: this.schema.revision, stats: ids.map((id) => this.stats[id] ?? null) };
  }

  async resetStats(ids: string[]): Promise<void> {
    this.resetCalls.push(ids);
    for (const id of ids) delete this.stats[id];
  }

  async getSession(): Promise<Session> {
    return this.session;
  }

  async getGpuProcesses(deviceId: string): Promise<GpuProcess[]> {
    this.gpuProcessCalls.push(deviceId);
    return [...this.gpuProcesses];
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

`app/src/test/fake-backend.test.ts` (nuovo):

```ts
import { MOCK_SCHEMA } from '../lib/backend/mock';
import { FakeBackend } from './fake-backend';

test('fake history returns the last `seconds` samples and records the call', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.history = { timestampsMs: [1000, 2000, 3000], series: [[1, 2, 3], [4, 5, 6]] };
  const h = await backend.getHistory(['a', 'b', 'c'], 2, 900);
  expect(h.timestampsMs).toEqual([2000, 3000]);
  expect(h.series).toEqual([[2, 3], [5, 6], []]);
  expect(backend.historyCalls).toEqual([{ ids: ['a', 'b', 'c'], seconds: 2, maxPoints: 900 }]);
  expect((await backend.getHistory(['a'], 60)).series).toEqual([[1, 2, 3]]);
  expect(backend.historyCalls[1].maxPoints).toBeUndefined();
});

test('fake stats are settable, recorded and cleared by a reset', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.stats = { a: { min: 1, max: 3, avg: 2, count: 3 } };
  expect(await backend.getStats(['a', 'b'])).toEqual({ revision: 1, stats: [{ min: 1, max: 3, avg: 2, count: 3 }, null] });
  await backend.resetStats(['a']);
  expect((await backend.getStats(['a'])).stats).toEqual([null]);
  expect(backend.statsCalls).toEqual([['a', 'b'], ['a']]);
  expect(backend.resetCalls).toEqual([['a']]);
});

test('fake session and gpu processes are settable', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  expect(await backend.getSession()).toEqual({ startedAtMs: null, intervalMs: 1000 });
  backend.session = { startedAtMs: 5, intervalMs: 2000 };
  expect(await backend.getSession()).toEqual({ startedAtMs: 5, intervalMs: 2000 });
  const row = { pid: 1, name: 'x.exe', loadPercent: 5, engine: '3D', dedicatedBytes: 1, sharedBytes: 2 };
  backend.gpuProcesses = [row];
  expect(await backend.getGpuProcesses('gpu/x')).toEqual([row]);
  expect(backend.gpuProcessCalls).toEqual(['gpu/x']);
});
```

`app/src/lib/backend/decimate.test.ts` (nuovo):

```ts
import type { HistoryWindow } from '../types';
import { decimateWindow } from './decimate';

const window = (n: number, series: ((i: number) => number | null)[]): HistoryWindow => ({
  timestampsMs: Array.from({ length: n }, (_, i) => 1000 * (i + 1)),
  series: series.map((f) => Array.from({ length: n }, (_, i) => f(i))),
});

test('windows that already fit are returned unchanged', () => {
  const w = window(10, [(i) => i]);
  expect(decimateWindow(w, 10)).toBe(w);
  expect(decimateWindow(w, 900)).toBe(w);
});

test('max points below two disable decimation', () => {
  const w = window(10, [(i) => i]);
  expect(decimateWindow(w, 1)).toBe(w);
  expect(decimateWindow(w, 0)).toBe(w);
});

test('each bucket emits its first timestamp with the min and its last with the max', () => {
  // 10 samples, 4 points -> 2 buckets of 5.
  const w = window(10, [(i) => [5, 1, 9, 3, 4, 7, 2, 8, 6, 0][i]]);
  expect(decimateWindow(w, 4)).toEqual({
    timestampsMs: [1000, 5000, 6000, 10000],
    series: [[1, 9, 0, 8]],
  });
});

test('balanced buckets distribute the remainder and odd max points round down', () => {
  // 11 samples, 5 points -> 2 buckets: 0..4 and 5..10.
  const w = window(11, [(i) => i]);
  const d = decimateWindow(w, 5);
  expect(d.timestampsMs).toEqual([1000, 5000, 6000, 11000]);
  expect(d.series).toEqual([[0, 4, 5, 10]]);
});

test('missing or non-finite samples make their bucket a gap, other series keep theirs', () => {
  // 6 samples, 4 points -> 2 buckets of 3; NaN counts as missing.
  const w = window(6, [(i) => (i < 3 ? null : i), (i) => (i === 1 ? Number.NaN : 10 + i)]);
  expect(decimateWindow(w, 4)).toEqual({
    timestampsMs: [1000, 3000, 4000, 6000],
    series: [
      [null, null, 3, 5],
      [null, null, 13, 15],
    ],
  });
});

test('a single missing sample preserves the gap conservatively', () => {
  const w = window(6, [(i) => (i === 1 ? null : i + 1)]);
  expect(decimateWindow(w, 4).series).toEqual([[null, null, 4, 6]]);
});

test('near one hour has no oversized final bucket', () => {
  const d = decimateWindow(window(3599, [(i) => i]), 900);
  expect(d.timestampsMs).toHaveLength(900);
  expect(d.timestampsMs[0]).toBe(1000);
  expect(d.timestampsMs.at(-1)).toBe(3_599_000);
  for (let i = 0; i < d.timestampsMs.length; i += 2) {
    const span = d.timestampsMs[i + 1] - d.timestampsMs[i];
    expect(span).toBeGreaterThanOrEqual(6000);
    expect(span).toBeLessThanOrEqual(7000);
  }
});

test('one hour at 1 s becomes 900 rows', () => {
  const d = decimateWindow(window(3600, [(i) => Math.sin(i)]), 900);
  expect(d.timestampsMs).toHaveLength(900);
  expect(d.series[0]).toHaveLength(900);
  expect(d.timestampsMs[0]).toBe(1000);
  expect(d.timestampsMs.at(-1)).toBe(3_600_000);
});
```

`app/src/lib/backend/tauri.test.ts` (nuovo). Controlla nomi dei comandi e degli argomenti senza Tauri, sostituendo `invoke` con un mock:

```ts
import { invoke } from '@tauri-apps/api/core';
import { createTauriBackend } from './tauri';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

// Names must match app/src-tauri/src/commands.rs; Tauri maps camelCase keys to snake_case arguments.
test('commands and argument names match the Rust shell', async () => {
  const backend = createTauriBackend();
  await backend.getHistory(['a'], 60);
  expect(invoke).toHaveBeenLastCalledWith('get_history', { ids: ['a'], seconds: 60 });
  await backend.getHistory(['a'], 3600, 900);
  expect(invoke).toHaveBeenLastCalledWith('get_history', { ids: ['a'], seconds: 3600, maxPoints: 900 });
  await backend.getStats(['a', 'b']);
  expect(invoke).toHaveBeenLastCalledWith('get_stats', { ids: ['a', 'b'] });
  await backend.resetStats(['a']);
  expect(invoke).toHaveBeenLastCalledWith('reset_stats', { ids: ['a'] });
  await backend.getSession();
  expect(invoke).toHaveBeenLastCalledWith('get_session');
  await backend.getGpuProcesses('gpu/pci-0000:01:00.0');
  expect(invoke).toHaveBeenLastCalledWith('get_gpu_processes', { deviceId: 'gpu/pci-0000:01:00.0' });
});
```

`app/src/lib/backend/mock.test.ts`, file completo:

```ts
import { catalogs } from '../i18n/index.svelte';
import { MOCK_HISTORY_SECONDS, MOCK_SCHEMA, createMockBackend, mockGpuProcesses, mockValues, sortGpuProcesses } from './mock';

const GPU = 'gpu/pci-0000:01:00.0';
const CPU_LOAD = 'cpu/0/load/total';

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

test('mock gpu is a discrete card with an experimental hotspot', () => {
  const gpu = MOCK_SCHEMA.devices.find((d) => d.kind === 'gpu');
  expect(gpu?.properties?.integrated).toBe('false');
  const hotspot = MOCK_SCHEMA.sensors.find((s) => s.id === 'gpu/pci-0000:01:00.0/temperature/hotspot');
  expect(hotspot?.experimental).toBe(true);
});

test('mock backend never starts in safe mode', async () => {
  const backend = createMockBackend();
  expect(await backend.getStartupStatus()).toEqual({ safeMode: false, reason: null, crashModule: null });
  expect((await backend.enableVendorLibraries()).safeMode).toBe(false);
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
  const h = await backend.getHistory([CPU_LOAD, 'unknown'], 10);
  expect(h.timestampsMs).toHaveLength(10);
  expect(h.series[0]).toHaveLength(10);
  expect(h.series[1].every((v) => v === null)).toBe(true);
});

test('mock history honors windows up to one hour', async () => {
  const backend = createMockBackend();
  expect((await backend.getHistory([CPU_LOAD], 1800)).timestampsMs).toHaveLength(1800);
  expect((await backend.getHistory([CPU_LOAD], 3600)).timestampsMs).toHaveLength(MOCK_HISTORY_SECONDS);
  expect((await backend.getHistory([CPU_LOAD], 7200)).timestampsMs).toHaveLength(MOCK_HISTORY_SECONDS);
});

test('mock history decimates to a min/max envelope when maxPoints is given', async () => {
  const backend = createMockBackend();
  const raw = await backend.getHistory([CPU_LOAD], 3600);
  const env = await backend.getHistory([CPU_LOAD], 3600, 900);
  expect(env.timestampsMs).toHaveLength(900);
  expect(env.series[0]).toHaveLength(900);
  // First bucket = raw samples 0..7 (3600 / 450 = 8 per bucket).
  const first = raw.series[0].slice(0, 8) as number[];
  expect(env.series[0][0]).toBe(Math.min(...first));
  expect(env.series[0][1]).toBe(Math.max(...first));
  // Short windows are returned raw even with maxPoints.
  expect((await backend.getHistory([CPU_LOAD], 60, 900)).timestampsMs).toHaveLength(60);
});

test('mock stats accumulate from the emitted ticks and reset per id', async () => {
  vi.useFakeTimers();
  try {
    const backend = createMockBackend(1000);
    expect((await backend.getStats([CPU_LOAD])).stats).toEqual([null]);
    const off = await backend.onSnapshot(() => {});
    vi.advanceTimersByTime(3000);
    const loads = [1, 2, 3].map((t) => mockValues(t)[0] as number);
    const reply = await backend.getStats([CPU_LOAD, 'unknown']);
    expect(reply.revision).toBe(MOCK_SCHEMA.revision);
    expect(reply.stats[0]?.min).toBe(Math.min(...loads));
    expect(reply.stats[0]?.max).toBe(Math.max(...loads));
    expect(reply.stats[0]?.avg).toBeCloseTo((loads[0] + loads[1] + loads[2]) / 3, 10);
    expect(reply.stats[0]?.count).toBe(3);
    expect(reply.stats[1]).toBeNull();

    await backend.resetStats([CPU_LOAD]);
    expect((await backend.getStats([CPU_LOAD])).stats).toEqual([null]);
    vi.advanceTimersByTime(1000);
    expect((await backend.getStats([CPU_LOAD])).stats[0]?.count).toBe(1);
    off();
  } finally {
    vi.useRealTimers();
  }
});

test('mock session starts at the first tick', async () => {
  vi.useFakeTimers();
  try {
    vi.setSystemTime(1_000_000);
    const backend = createMockBackend(500);
    expect(await backend.getSession()).toEqual({ startedAtMs: null, intervalMs: 500 });
    const off = await backend.onSnapshot(() => {});
    vi.advanceTimersByTime(1500);
    expect(await backend.getSession()).toEqual({ startedAtMs: 1_000_500, intervalMs: 500 });
    off();
  } finally {
    vi.useRealTimers();
  }
});

test('mock gpu processes are sorted by load then dedicated memory', async () => {
  const backend = createMockBackend();
  const list = await backend.getGpuProcesses(GPU);
  expect(list).toHaveLength(mockGpuProcesses(0).length);
  const loads = list.map((p) => p.loadPercent ?? -1);
  expect(loads).toEqual([...loads].sort((a, b) => b - a));
  expect(list.at(-1)?.loadPercent).toBeNull();
  expect(list.filter((p) => p.loadPercent === 0).map((p) => p.name)).toEqual(['explorer.exe', 'System']);
  expect(await backend.getGpuProcesses('gpu/unknown')).toEqual([]);
});

test('gpu process lists are capped at 20 rows', () => {
  const many = Array.from({ length: 30 }, (_, i) => ({
    pid: i,
    name: `p${i}.exe`,
    loadPercent: i,
    engine: '3D',
    dedicatedBytes: 0,
    sharedBytes: 0,
  }));
  const sorted = sortGpuProcesses(many);
  expect(sorted).toHaveLength(20);
  expect(sorted[0].pid).toBe(29);
});
```

`app/src/lib/format.test.ts`, file completo. La tabella `EXAMPLES` è un `Record<Unit, …>`: se in futuro si aggiunge una `Unit` senza esempio, `pnpm check` fallisce:

```ts
import {
  DASH,
  formatBytes,
  formatClock,
  formatDuration,
  formatPercent,
  formatPower,
  formatRate,
  formatTemperature,
  formatValue,
} from './format';
import { translate } from './i18n/index.svelte';
import type { Unit } from './types';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
const tIt = (key: string, params?: Record<string, string | number>) => translate('it', key, params);

test('null values render as a dash', () => {
  expect(formatPercent(null, 'en')).toBe(DASH);
  expect(formatBytes(null, 'en')).toBe(DASH);
  expect(formatRate(null, 'bits', 'en')).toBe(DASH);
  expect(formatClock(null, 'en')).toBe(DASH);
  expect(formatTemperature(null, 'en')).toBe(DASH);
  expect(formatPower(Number.NaN, 'en')).toBe(DASH);
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

test('temperature and power have no decimals', () => {
  expect(formatTemperature(54.4, 'en')).toBe('54 °C');
  expect(formatTemperature(99.6, 'it')).toBe('100 °C');
  expect(formatPower(147.8, 'en')).toBe('148 W');
  expect(formatPower(1234, 'en')).toBe('1,234 W');
});

test('durations', () => {
  expect(formatDuration(5 * 60_000, tEn)).toBe('5 min');
  expect(formatDuration(125 * 60_000, tEn)).toBe('2 h 5 min');
  expect(formatDuration(-1, tEn)).toBe('0 min');
});

// `Record<Unit, …>` makes this table fail to compile when a Unit is added without a case.
const EXAMPLES: Record<Unit, [number, string]> = {
  celsius: [54.4, '54 °C'],
  percent: [35.4, '35%'],
  megahertz: [2520, '2.52 GHz'],
  watt: [147.8, '148 W'],
  volt: [1.075, '1.075 V'],
  ampere: [12.34, '12.3 A'],
  rpm: [1650, '1,650 RPM'],
  bytes: [17.9 * 1024 ** 3, '17.9 GB'],
  bytes_per_second: [120 * 1024 ** 2, '120 MB/s'],
  bits_per_second: [1e9, '1.0 Gbit/s'],
  joule: [12_345, '12.3 kJ'],
  boolean: [1, 'Active'],
  pcie_generation: [4, 'Gen 4'],
  lanes: [16, 'x16'],
};

test('formatValue formats every unit', () => {
  for (const [unit, [value, expected]] of Object.entries(EXAMPLES) as [Unit, [number, string]][]) {
    expect(formatValue(value, unit, 'en', tEn), unit).toBe(expected);
  }
});

test('formatValue renders missing values as a dash for every unit', () => {
  for (const unit of Object.keys(EXAMPLES) as Unit[]) {
    expect(formatValue(null, unit, 'en', tEn), unit).toBe(DASH);
    expect(formatValue(Number.NaN, unit, 'en', tEn), unit).toBe(DASH);
  }
});

test('formatValue follows the locale', () => {
  expect(formatValue(1.075, 'volt', 'it', tIt)).toBe('1,075 V');
  expect(formatValue(12_000, 'rpm', 'it', tIt)).toBe('12.000 RPM');
  expect(formatValue(0, 'boolean', 'it', tIt)).toBe('No');
  expect(formatValue(1, 'boolean', 'it', tIt)).toBe('Attivo');
  expect(formatValue(0, 'boolean', 'en', tEn)).toBe('No');
});

test('formatValue rounds link values and scales energy', () => {
  expect(formatValue(3.9999, 'pcie_generation', 'en', tEn)).toBe('Gen 4');
  expect(formatValue(8, 'lanes', 'en', tEn)).toBe('x8');
  expect(formatValue(950, 'joule', 'en', tEn)).toBe('950 J');
  expect(formatValue(2.5e6, 'joule', 'en', tEn)).toBe('2.5 MJ');
});

test('formatValue shows byte rates in bits on request, like the Simple view network tile', () => {
  expect(formatValue(6_000_000, 'bytes_per_second', 'en', tEn, { rate: 'bits' })).toBe('48 Mbit/s');
  expect(formatValue(6_000_000, 'bytes_per_second', 'en', tEn, { rate: 'bytes' })).toBe('5.7 MB/s');
  expect(formatValue(6_000_000, 'bytes_per_second', 'en', tEn)).toBe('5.7 MB/s');
  // Units other than bytes_per_second ignore the option.
  expect(formatValue(1e9, 'bits_per_second', 'en', tEn, { rate: 'bytes' })).toBe('1.0 Gbit/s');
});
```

`app/src/lib/i18n/i18n.test.ts`, file completo. Come sopra, `Record<Source, true>` obbliga ad aggiornare il test quando si aggiunge una fonte:

```ts
import type { Source } from '../types';
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

// `Record<Source, true>` stops compiling when a Source is added to types.ts without an entry here.
const SOURCES: Record<Source, true> = {
  pdh: true,
  win32: true,
  ip_helper: true,
  dxgi: true,
  d3dkmt: true,
  nvml: true,
  nvapi: true,
  adl: true,
  igcl: true,
  pnp: true,
  mock: true,
};

test('every sensor source has a badge name', () => {
  for (const source of Object.keys(SOURCES)) {
    expect(catalogs.en[`source.${source}`], source).toBeDefined();
  }
});

// Device property keys produced by oma-win (GPU layers and storage).
const PROPERTIES = [
  'pciAddress',
  'integrated',
  'pcieMaxGen',
  'pcieMaxWidth',
  'powerLimitMinW',
  'powerLimitMaxW',
  'powerLimitDefaultW',
  'tempSlowdownC',
  'tempShutdownC',
  'tempMaxC',
  'tempWarningC',
  'tempCriticalC',
];

test('every device property has a label', () => {
  for (const key of PROPERTIES) {
    expect(catalogs.en[`property.${key}`], key).toBeDefined();
  }
});

test('flag values and the stale badge are translated', () => {
  expect(translate('en', 'flag.on')).toBe('Active');
  expect(translate('it', 'flag.on')).toBe('Attivo');
  expect(translate('en', 'flag.off')).toBe('No');
  expect(translate('it', 'status.stale')).toBe('Dati non aggiornati');
});
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

```bash
cd app && pnpm test
```

Risultato atteso: `Test Files  5 failed | 6 passed (11)` e `Tests  15 failed | 54 passed (69)`. Se i task precedenti hanno aggiunto test frontend, i totali crescono di conseguenza. Tra gli errori:
- `Failed to resolve import "./decimate"` (`decimate.test.ts`);
- `TypeError: formatValue is not a function` (5 test di `format.test.ts`);
- `expected [ Array(300) ] to have a length of 1800 but got 300`, `TypeError: backend.getStats is not a function`, `… getSession is not a function`, `… getGpuProcesses is not a function`, `TypeError: sortGpuProcesses is not a function` (`mock.test.ts`);
- `expected last "vi.fn()" call to have been called with [ 'get_history', …(1) ]` (`tauri.test.ts`: manca `maxPoints`);
- `pdh: expected undefined to be defined`, `pciAddress: expected undefined to be defined`, `expected 'flag.on' to be 'Active'` (`i18n.test.ts`).

I 3 test di `fake-backend.test.ts` passano già: il backend finto è stato scritto nello Step 1.

- [ ] **Step 3: Aggiungi uPlot**

```bash
cd app && pnpm add --save-exact uplot@1.6.32
```

`--save-exact` scrive `"uplot": "1.6.32"` e non `^1.6.32`, come le altre dipendenze. Risultato in `app/package.json`:

```json
  "dependencies": {
    "@tauri-apps/api": "2.11.1",
    "uplot": "1.6.32"
  },
```

`pnpm-lock.yaml` guadagna le voci `uplot@1.6.32` (licenza MIT, nessuna dipendenza). Il pacchetto include i tipi (`dist/uPlot.d.ts`) e il CSS (`dist/uPlot.min.css`); lo userà il Task 11. La CSP di `tauri.conf.json` non cambia: lo spike ha verificato che uPlot non usa `eval` né `<style>` iniettati.

- [ ] **Step 4: Implementa tipi, backend Tauri e mock**

`app/src/lib/types.ts`, file completo:

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
  | 'percent'
  | 'link';

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
  | 'boolean'
  | 'pcie_generation'
  | 'lanes';

export type Source =
  | 'pdh'
  | 'win32'
  | 'ip_helper'
  | 'dxgi'
  | 'd3dkmt'
  | 'nvml'
  | 'nvapi'
  | 'adl'
  | 'igcl'
  | 'pnp'
  | 'mock';

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
  /** Present (true) only for readings from undocumented vendor calls (spec §5.2). */
  experimental?: boolean;
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

/** GPU safe mode (spec §8): vendor libraries off after `--safe` or a crash. */
export interface StartupStatus {
  safeMode: boolean;
  reason: 'flag' | 'crash' | null;
  /** File name of the module that crashed the previous run, e.g. "nvml.dll". */
  crashModule: string | null;
}

/** Running statistics of one sensor since the app started or its last reset (spec §4.2). */
export interface SensorStats {
  min: number;
  max: number;
  avg: number;
  count: number;
}

/** `stats[i]` belongs to the i-th requested id; null = unknown id or no sample yet. */
export interface StatsReply {
  revision: number;
  stats: (SensorStats | null)[];
}

/** Sampling session of the core process (it outlives the window). */
export interface Session {
  /** Timestamp of the engine's first tick; null before it. */
  startedAtMs: number | null;
  intervalMs: number;
}

/** One process using a GPU (not a sensor: no id, no history). */
export interface GpuProcess {
  pid: number;
  name: string;
  /** Busiest engine of the process, 0..100; null on the first tick after (re)attach. */
  loadPercent: number | null;
  /** Type of that engine (e.g. "3D"), only while the load is above zero. */
  engine: string | null;
  dedicatedBytes: number | null;
  sharedBytes: number | null;
}
```

`app/src/lib/backend/backend.ts`, file completo:

```ts
import type { GpuProcess, HistorySeed, Schema, Session, Snapshot, StartupStatus, StatsReply } from '../types';

export type Unsubscribe = () => void;

/** Everything the UI needs from the sampling core (Tauri, or a mock in the browser). */
export interface Backend {
  getSchema(): Promise<Schema>;
  /**
   * Last `seconds` of history (at most 3600). With `maxPoints` the core returns a
   * min/max envelope of at most that many rows instead of the raw samples.
   */
  getHistory(ids: string[], seconds: number, maxPoints?: number): Promise<HistorySeed>;
  onSchema(cb: (schema: Schema) => void): Promise<Unsubscribe>;
  onSnapshot(cb: (snapshot: Snapshot) => void): Promise<Unsubscribe>;
  /** GPU safe-mode status of this session. */
  getStartupStatus(): Promise<StartupStatus>;
  /** Loads the GPU vendor libraries without a restart; returns the new status. */
  enableVendorLibraries(): Promise<StartupStatus>;
  /** Min/max/avg since the app started (or the last reset), one entry per id. */
  getStats(ids: string[]): Promise<StatsReply>;
  /** Restarts min/max/avg of these sensors; unknown ids are ignored. */
  resetStats(ids: string[]): Promise<void>;
  /** Start time and sampling interval of the core. */
  getSession(): Promise<Session>;
  /** Processes using this GPU, busiest first, at most 20; empty for an unknown device. */
  getGpuProcesses(deviceId: string): Promise<GpuProcess[]>;
}
```

`app/src/lib/backend/tauri.ts`, file completo:

```ts
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { GpuProcess, HistorySeed, Schema, Session, Snapshot, StartupStatus, StatsReply } from '../types';
import type { Backend } from './backend';

/** Command and event names are defined in app/src-tauri (commands.rs, main.rs). */
export function createTauriBackend(): Backend {
  return {
    getSchema: () => invoke<Schema>('get_schema'),
    // Tauri maps the camelCase `maxPoints` key to the `max_points` argument; a missing key is `None`.
    getHistory: (ids, seconds, maxPoints) =>
      invoke<HistorySeed>('get_history', maxPoints === undefined ? { ids, seconds } : { ids, seconds, maxPoints }),
    onSchema: (cb) => listen<Schema>('oma:schema', (e) => cb(e.payload)),
    onSnapshot: (cb) => listen<Snapshot>('oma:snapshot', (e) => cb(e.payload)),
    getStartupStatus: () => invoke<StartupStatus>('get_startup_status'),
    enableVendorLibraries: () => invoke<StartupStatus>('enable_vendor_libraries'),
    getStats: (ids) => invoke<StatsReply>('get_stats', { ids }),
    resetStats: (ids) => invoke<void>('reset_stats', { ids }),
    getSession: () => invoke<Session>('get_session'),
    getGpuProcesses: (deviceId) => invoke<GpuProcess[]>('get_gpu_processes', { deviceId }),
  };
}
```

`app/src/lib/backend/decimate.ts` (nuovo):

```ts
import type { HistoryWindow } from '../types';

/**
 * Min/max envelope, the same rule as `History::window_decimated` in oma-core:
 * with more samples than `maxPoints` (and `maxPoints >= 2`) the samples are split into
 * `floor(maxPoints / 2)` balanced buckets whose sizes differ by at most one;
 * each bucket emits (first timestamp, per-series min) then
 * (last timestamp, per-series max). These are envelope bounds, not extremum times.
 * Any missing or non-finite value makes that series emit null twice for the bucket.
 */
export function decimateWindow(window: HistoryWindow, maxPoints: number): HistoryWindow {
  const n = window.timestampsMs.length;
  if (n <= maxPoints || maxPoints < 2) return window;
  const buckets = Math.floor(maxPoints / 2);
  const timestampsMs: number[] = [];
  const series: (number | null)[][] = window.series.map(() => []);
  for (let b = 0; b < buckets; b++) {
    const start = Math.floor(b * n / buckets);
    const end = Math.floor((b + 1) * n / buckets);
    timestampsMs.push(window.timestampsMs[start], window.timestampsMs[end - 1]);
    window.series.forEach((values, k) => {
      if (values.slice(start, end).some((v) => v == null || !Number.isFinite(v))) {
        series[k].push(null, null);
        return;
      }
      let min = Infinity;
      let max = -Infinity;
      for (let i = start; i < end; i++) {
        const v = values[i];
        if (v === null || v === undefined || !Number.isFinite(v)) continue;
        if (v < min) min = v;
        if (v > max) max = v;
      }
      const found = min <= max;
      series[k].push(found ? min : null, found ? max : null);
    });
  }
  return { timestampsMs, series };
}
```

`app/src/lib/backend/mockStats.ts` (nuovo):

```ts
import type { SensorStats } from '../types';

interface Acc {
  min: number;
  max: number;
  sum: number;
  count: number;
}

/** Browser-side stand-in for oma-core's `Stats`: min/max/avg per sensor id. */
export class StatsAccumulator {
  #acc = new Map<string, Acc>();

  /** Adds one tick; `values[i]` belongs to `ids[i]`. Null and non-finite values are skipped. */
  push(ids: string[], values: (number | null)[]): void {
    ids.forEach((id, i) => {
      const v = values[i];
      if (v === null || v === undefined || !Number.isFinite(v)) return;
      const acc = this.#acc.get(id);
      if (!acc) {
        this.#acc.set(id, { min: v, max: v, sum: v, count: 1 });
        return;
      }
      acc.min = Math.min(acc.min, v);
      acc.max = Math.max(acc.max, v);
      acc.sum += v;
      acc.count++;
    });
  }

  get(ids: string[]): (SensorStats | null)[] {
    return ids.map((id) => {
      const acc = this.#acc.get(id);
      return acc ? { min: acc.min, max: acc.max, avg: acc.sum / acc.count, count: acc.count } : null;
    });
  }

  reset(ids: string[]): void {
    ids.forEach((id) => this.#acc.delete(id));
  }
}
```

`app/src/lib/backend/mock.ts`, file completo. `MOCK_SCHEMA` e `mockValues` non cambiano; `getHistory` calcola ogni riga di `mockValues` una volta sola, non una volta per serie, perché ora le righe possono essere 3600:

```ts
import type { GpuProcess, HistoryWindow, Label, Schema, Sensor, SensorKind, Snapshot, StartupStatus, Unit } from '../types';
import type { Backend } from './backend';
import { decimateWindow } from './decimate';
import { StatsAccumulator } from './mockStats';

const THREADS = 8;
const GIB = 1024 ** 3;
const MIB = 1024 ** 2;
const GPU = 'gpu/pci-0000:01:00.0';

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
    {
      id: GPU,
      kind: 'gpu',
      name: 'Mock GeForce RTX 4080',
      vendor: 'NVIDIA',
      properties: { pciAddress: '0000:01:00.0', integrated: 'false' },
    },
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
    sensor(`${GPU}/load/core`, GPU, 'load', 'percent', { key: 'gpu.load.core' }),
    sensor(`${GPU}/data/memory-dedicated-used`, GPU, 'data', 'bytes', { key: 'gpu.memory.dedicatedUsed' }),
    sensor(`${GPU}/data/memory-dedicated-total`, GPU, 'data', 'bytes', { key: 'gpu.memory.dedicatedTotal' }),
    sensor(`${GPU}/temperature/core`, GPU, 'temperature', 'celsius', { key: 'gpu.temperature.core' }),
    { ...sensor(`${GPU}/temperature/hotspot`, GPU, 'temperature', 'celsius', { key: 'gpu.temperature.hotspot' }), experimental: true },
    sensor(`${GPU}/clock/core`, GPU, 'clock', 'megahertz', { key: 'gpu.clock.core' }),
    sensor(`${GPU}/power/board`, GPU, 'power', 'watt', { key: 'gpu.power.board' }),
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
  const gpuLoad = 10 + 80 * wave(11, 3);
  return [
    total,
    ...threads,
    4200 + 400 * wave(7),
    gpuLoad,
    (2 + 6 * wave(40)) * GIB,
    16 * GIB,
    40 + gpuLoad * 0.3,
    52 + gpuLoad * 0.35,
    1500 + 12 * gpuLoad,
    25 + 2.9 * gpuLoad,
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

/** The mock never starts in GPU safe mode. */
export const MOCK_STARTUP: StartupStatus = { safeMode: false, reason: null, crashModule: null };

/** Same cap as the core (`MAX_HISTORY_SECONDS`): one hour at 1 s. */
export const MOCK_HISTORY_SECONDS = 3600;

/** Plausible processes on the mock GPU at tick `t`, unsorted. */
export function mockGpuProcesses(t: number): GpuProcess[] {
  const wave = (period: number, phase = 0) => (Math.sin((t + phase) / period) + 1) / 2;
  const game = 20 + 70 * wave(11, 3);
  const encoder = 5 + 10 * wave(7);
  return [
    { pid: 4, name: 'System', loadPercent: 0, engine: null, dedicatedBytes: 0, sharedBytes: 2 * MIB },
    { pid: 1188, name: 'dwm.exe', loadPercent: 1 + 3 * wave(5), engine: '3D', dedicatedBytes: 310 * MIB, sharedBytes: 48 * MIB },
    { pid: 6020, name: 'explorer.exe', loadPercent: 0, engine: null, dedicatedBytes: 42 * MIB, sharedBytes: 12 * MIB },
    { pid: 9412, name: 'game.exe', loadPercent: game, engine: '3D', dedicatedBytes: 5.5 * GIB, sharedBytes: 180 * MIB },
    { pid: 10764, name: 'obs64.exe', loadPercent: encoder, engine: 'VideoEncode', dedicatedBytes: 620 * MIB, sharedBytes: 64 * MIB },
    { pid: 12880, name: 'msedgewebview2.exe', loadPercent: null, engine: null, dedicatedBytes: null, sharedBytes: null },
  ];
}

const desc = (a: number | null, b: number | null) => (b ?? -1) - (a ?? -1);

/** Core order: busiest first, then most dedicated memory; at most 20 rows. */
export function sortGpuProcesses(list: GpuProcess[]): GpuProcess[] {
  return [...list]
    .sort((a, b) => desc(a.loadPercent, b.loadPercent) || desc(a.dedicatedBytes, b.dedicatedBytes))
    .slice(0, 20);
}

const MOCK_IDS = MOCK_SCHEMA.sensors.map((s) => s.id);

/** Browser-only backend used by `pnpm dev` and component tests. */
export function createMockBackend(intervalMs = 1000): Backend {
  let seq = 0;
  let startup = MOCK_STARTUP;
  let startedAtMs: number | null = null;
  let timer: ReturnType<typeof setInterval> | undefined;
  const stats = new StatsAccumulator();
  const listeners = new Set<(s: Snapshot) => void>();
  const emit = () => {
    seq++;
    const snapshot: Snapshot = { revision: MOCK_SCHEMA.revision, seq, timestampMs: Date.now(), values: mockValues(seq) };
    startedAtMs ??= snapshot.timestampMs;
    stats.push(MOCK_IDS, snapshot.values);
    listeners.forEach((cb) => cb(snapshot));
  };
  return {
    getSchema: async () => MOCK_SCHEMA,
    getHistory: async (ids, seconds, maxPoints) => {
      const n = Math.max(0, Math.min(Math.floor(seconds), MOCK_HISTORY_SECONDS));
      const now = Date.now();
      const rows = Array.from({ length: n }, (_, i) => mockValues(seq - n + 1 + i));
      const indices = ids.map((id) => MOCK_IDS.indexOf(id));
      const raw: HistoryWindow = {
        timestampsMs: rows.map((_, i) => now - (n - 1 - i) * intervalMs),
        series: indices.map((k) => rows.map((row) => (k < 0 ? null : row[k]))),
      };
      const window =
        maxPoints === undefined ? raw : decimateWindow(raw, Math.min(Math.max(Math.floor(maxPoints), 2), MOCK_HISTORY_SECONDS));
      return { revision: MOCK_SCHEMA.revision, seq, ...window };
    },
    onSchema: async () => () => {},
    getStartupStatus: async () => startup,
    enableVendorLibraries: async () => {
      startup = { ...startup, safeMode: false };
      return startup;
    },
    getStats: async (ids) => ({ revision: MOCK_SCHEMA.revision, stats: stats.get(ids) }),
    resetStats: async (ids) => stats.reset(ids),
    getSession: async () => ({ startedAtMs, intervalMs }),
    getGpuProcesses: async (deviceId) => (deviceId === GPU ? sortGpuProcesses(mockGpuProcesses(seq)) : []),
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

- [ ] **Step 5: Implementa `formatValue`, le traduzioni e i token delle serie**

`app/src/lib/format.ts`, file completo. I formattatori esistenti ora accettano `locale: string` invece di `Locale`: il tipo si allarga, e i chiamanti esistenti, che passano `i18n.locale`, non cambiano:

```ts
import type { Translate } from './i18n/index.svelte';
import type { Unit } from './types';

export const DASH = '—';

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];
const BIT_UNITS = ['bit/s', 'kbit/s', 'Mbit/s', 'Gbit/s', 'Tbit/s'];
const JOULE_UNITS = ['J', 'kJ', 'MJ', 'GJ'];
const formatters = new Map<string, Intl.NumberFormat>();

// `locale` is a BCP 47 tag ('en', 'it'); Intl accepts any tag, so callers may pass a plain string.
function num(value: number, digits: number, locale: string): string {
  const key = `${locale}:${digits}`;
  let formatter = formatters.get(key);
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits });
    formatters.set(key, formatter);
  }
  return formatter.format(value);
}

const missing = (v: number | null): v is null => v === null || !Number.isFinite(v);

export function formatPercent(value: number | null, locale: string): string {
  return missing(value) ? DASH : `${num(value, 0, locale)}%`;
}

/** Binary steps (1024) with the unit names Windows shows (KB, MB, GB). */
export function formatBytes(bytes: number | null, locale: string): string {
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
export function formatRate(bytesPerSecond: number | null, mode: 'bits' | 'bytes', locale: string): string {
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

export function formatClock(mhz: number | null, locale: string): string {
  if (missing(mhz)) return DASH;
  return mhz >= 1000 ? `${num(mhz / 1000, 2, locale)} GHz` : `${num(mhz, 0, locale)} MHz`;
}

export function formatTemperature(celsius: number | null, locale: string): string {
  return missing(celsius) ? DASH : `${num(celsius, 0, locale)} °C`;
}

export function formatPower(watt: number | null, locale: string): string {
  return missing(watt) ? DASH : `${num(watt, 0, locale)} W`;
}

/** Energy with decimal steps (J, kJ, MJ, GJ). */
function formatEnergy(joule: number, locale: string): string {
  let value = joule;
  let unit = 0;
  while (Math.abs(value) >= 1000 && unit < JOULE_UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${num(value, digits, locale)} ${JOULE_UNITS[unit]}`;
}

/** Options of `formatValue`. */
export interface FormatOptions {
  /**
   * How `bytes_per_second` is shown: 'bytes' (default: disks) or 'bits' (network pages,
   * the same unit as the Simple view's network tile).
   */
  rate?: 'bits' | 'bytes';
}

/** Any sensor value with its unit (Advanced view tables and KPIs). */
export function formatValue(
  value: number | null,
  unit: Unit,
  locale: string,
  t: Translate,
  opts: FormatOptions = {},
): string {
  if (missing(value)) return DASH;
  switch (unit) {
    case 'celsius':
      return formatTemperature(value, locale);
    case 'percent':
      return formatPercent(value, locale);
    case 'megahertz':
      return formatClock(value, locale);
    case 'watt':
      return formatPower(value, locale);
    case 'volt':
      return `${num(value, 3, locale)} V`;
    case 'ampere':
      return `${num(value, 1, locale)} A`;
    case 'rpm':
      return `${num(value, 0, locale)} RPM`;
    case 'bytes':
      return formatBytes(value, locale);
    case 'bytes_per_second':
      return formatRate(value, opts.rate ?? 'bytes', locale);
    case 'bits_per_second':
      return formatRate(value / 8, 'bits', locale);
    case 'joule':
      return formatEnergy(value, locale);
    case 'boolean':
      return t(value >= 0.5 ? 'flag.on' : 'flag.off');
    case 'pcie_generation':
      return `Gen ${Math.round(value)}`;
    case 'lanes':
      return `x${Math.round(value)}`;
    default: {
      const unknown: never = unit;
      return `${num(value, 1, locale)} ${String(unknown)}`;
    }
  }
}

export function formatDuration(ms: number, t: Translate): string {
  const minutes = Math.max(0, Math.floor(ms / 60_000));
  if (minutes < 60) return t('duration.minutes', { n: minutes });
  return t('duration.hoursMinutes', { h: Math.floor(minutes / 60), m: minutes % 60 });
}
```

`app/src/lib/i18n/en.json`: inserisci subito dopo la riga `"safe.enabling": "Re-enabling…",`:

```json
  "status.stale": "Data not updating",
  "flag.on": "Active",
  "flag.off": "No",
  "source.pdh": "Windows performance counters (PDH)",
  "source.win32": "Windows API",
  "source.ip_helper": "Windows IP Helper",
  "source.dxgi": "DirectX Graphics Infrastructure (DXGI)",
  "source.d3dkmt": "Windows display driver interface (D3DKMT)",
  "source.nvml": "NVIDIA Management Library (NVML)",
  "source.nvapi": "NVIDIA NVAPI",
  "source.adl": "AMD Display Library (ADL)",
  "source.igcl": "Intel Graphics Control Library (IGCL)",
  "source.pnp": "Windows Plug and Play",
  "source.mock": "Simulated data",
  "property.pciAddress": "PCI address",
  "property.integrated": "Integrated GPU",
  "property.pcieMaxGen": "Maximum PCIe generation",
  "property.pcieMaxWidth": "Maximum PCIe link width",
  "property.powerLimitMinW": "Minimum power limit (W)",
  "property.powerLimitMaxW": "Maximum power limit (W)",
  "property.powerLimitDefaultW": "Default power limit (W)",
  "property.tempSlowdownC": "Slowdown temperature (°C)",
  "property.tempShutdownC": "Shutdown temperature (°C)",
  "property.tempMaxC": "Maximum GPU temperature (°C)",
  "property.tempWarningC": "Warning temperature (°C)",
  "property.tempCriticalC": "Critical temperature (°C)",
```

`app/src/lib/i18n/it.json`: inserisci subito dopo la riga `"safe.enabling": "Riattivazione…",`:

```json
  "status.stale": "Dati non aggiornati",
  "flag.on": "Attivo",
  "flag.off": "No",
  "source.pdh": "Contatori di prestazioni di Windows (PDH)",
  "source.win32": "API di Windows",
  "source.ip_helper": "IP Helper di Windows",
  "source.dxgi": "DirectX Graphics Infrastructure (DXGI)",
  "source.d3dkmt": "Interfaccia del driver video di Windows (D3DKMT)",
  "source.nvml": "NVIDIA Management Library (NVML)",
  "source.nvapi": "NVIDIA NVAPI",
  "source.adl": "AMD Display Library (ADL)",
  "source.igcl": "Intel Graphics Control Library (IGCL)",
  "source.pnp": "Plug and Play di Windows",
  "source.mock": "Dati simulati",
  "property.pciAddress": "Indirizzo PCI",
  "property.integrated": "GPU integrata",
  "property.pcieMaxGen": "Generazione PCIe massima",
  "property.pcieMaxWidth": "Larghezza massima del link PCIe",
  "property.powerLimitMinW": "Limite di potenza minimo (W)",
  "property.powerLimitMaxW": "Limite di potenza massimo (W)",
  "property.powerLimitDefaultW": "Limite di potenza predefinito (W)",
  "property.tempSlowdownC": "Temperatura di rallentamento (°C)",
  "property.tempShutdownC": "Temperatura di spegnimento (°C)",
  "property.tempMaxC": "Temperatura massima della GPU (°C)",
  "property.tempWarningC": "Temperatura di avviso (°C)",
  "property.tempCriticalC": "Temperatura critica (°C)",
```

I nomi delle fonti compariranno nel badge della tabella dei sensori (Task 12, `title` = `source.<nome>`). Le etichette delle proprietà includono l'unità perché i valori di `Device.properties` sono numeri decimali senza unità (Task 13, `DeviceInfo`).

`app/src/styles/theme.css`: in `:root`, subito dopo la riga `--on-accent: #1a0616; /* text colour on --accent */`, aggiungi:

```css
  /* Chart series 5-8; series 1-4 use --accent, --accent-2, --ok, --warn. */
  --series-5: #b388ff;
  --series-6: #ff8c42;
  --series-7: #c6ff4a;
  --series-8: #6c8cff;
```

Viola, arancio, lime e pervinca: restano distinguibili dalle prime quattro serie (rosa, ciano, verde acqua, giallo) e da `--crit`, che non si usa per le serie perché i colori di stato devono restare distinti (spec §7.5).

- [ ] **Step 6: Esegui i test e verifica che passino**

```bash
cd app
pnpm test
pnpm check
pnpm build
```

Risultato atteso:
- `pnpm test`: `Test Files  11 passed (11)`, `Tests  77 passed (77)`;
- `pnpm check`: `0 ERRORS 0 WARNINGS`;
- `pnpm build`: OK. uPlot non è ancora importato, quindi il bundle non cambia.

- [ ] **Step 7: Verifica manuale (utente) con il backend mock**

Esegui `cd app && pnpm dev` e apri `http://localhost:1420` in un browser:
1. La vista Semplificata è invariata: banner "Monitoraggio attivo", riquadri che si aggiornano ogni secondo.
2. Nella console degli strumenti di sviluppo (F12) esegui:
   ```js
   const m = await import('/src/lib/backend/mock.ts');
   const b = m.createMockBackend();
   (await b.getHistory(['cpu/0/load/total'], 3600)).timestampsMs.length;       // 3600
   (await b.getHistory(['cpu/0/load/total'], 3600, 900)).timestampsMs.length;  // 900
   (await b.getGpuProcesses('gpu/pci-0000:01:00.0')).map((p) => p.name);       // game.exe per primo, msedgewebview2.exe per ultimo
   getComputedStyle(document.documentElement).getPropertyValue('--series-5');  // "#b388ff"
   ```
3. Chiudi il server con Ctrl+C.

- [ ] **Step 8: Commit**

```bash
git add app/package.json app/pnpm-lock.yaml app/src
git commit -m "feat(ui): Advanced view foundations: uPlot, stats/session/process API, formatValue"
```

---

### Task 10: UI: struttura della vista Avanzata (barra laterale, sezioni salvate, collegamento dalla vista Semplificata, indicatore di dati fermi, banner dalla sessione)

**File:**
- Crea:
  - `app/src/lib/advanced/nav.ts` (voci della barra laterale, sezione di un riquadro, sezione da aprire)
  - `app/src/lib/advanced/persist.ts` (stato della vista Avanzata in `localStorage`)
  - `app/src/lib/stale.ts` (soglia dei dati fermi)
  - `app/src/components/advanced/AdvancedView.svelte`, `app/src/components/advanced/Sidebar.svelte`
  - `app/src/components/advanced/DevicePage.svelte` (pagina minima, sostituita dal Task 13)
- Modifica:
  - `app/src/lib/live.svelte.ts` (`lastReceivedAtMs`)
  - `app/src/App.svelte` (vista Avanzata, sessione, controllo dei dati fermi ogni secondo, collegamento dai riquadri)
  - `app/src/components/TopBar.svelte` (prop `stale`, badge `status.stale`)
  - `app/src/components/simple/SimpleView.svelte` (`onOpenAdvanced(section)`, prop `startedAtMs`)
  - `app/src/lib/i18n/en.json`, `app/src/lib/i18n/it.json` (`advanced.sidebar`, `advanced.section.*`; rimossa `advanced.comingSoon`)
- Elimina: `app/src/components/advanced/AdvancedPlaceholder.svelte`
- Test:
  - Crea: `app/src/lib/advanced/nav.test.ts`, `app/src/lib/advanced/persist.test.ts`, `app/src/lib/stale.test.ts`, `app/src/components/advanced/AdvancedView.test.ts`
  - Modifica: `app/src/lib/live.test.ts`, `app/src/App.test.ts`

**Interfacce:**
- Usa:
  - Task 9: `Backend.getSession(): Promise<Session>` (`{ startedAtMs: number | null; intervalMs: number }`), `FakeBackend.session`, `formatValue(value, unit, locale, t)`, chiave `status.stale`.
  - Task 3: `get_session` restituisce `startedAtMs` = timestamp del primo tick del motore (decisione D2).
  - M1: `LiveStore`, `connect`, `HealthBanner` (`health`, `nowMs`), `monitoringHealth(startedAtMs)`, `TopBar`, `Tile`.
- Produce:
  - `lib/advanced/nav.ts`:
    ```ts
    export interface SidebarEntry { id: string; kind: DeviceKind; deviceIds: string[]; labelKey: string; labelArg?: string }
    export type SimpleTile = 'cpu' | 'gpu' | 'memory' | 'storage' | 'network';
    export function sidebarEntries(schema: Schema): SidebarEntry[];
    export function sectionForTile(tile: SimpleTile, deviceId?: string): string | null;
    export function resolveSection(entries: SidebarEntry[], wanted: string | null): string | null;
    ```
  - `lib/advanced/persist.ts` (ogni accesso a `localStorage` è protetto da try/catch):
    ```ts
    export const SECTION_KEY = 'oma.advanced.section';
    export const WINDOW_KEY = 'oma.advanced.window';
    export const seriesKey: (sectionId: string) => string;            // 'oma.advanced.series.<sectionId>'
    export const STORED_WINDOWS: readonly [60, 300, 1800, 3600];
    export type StoredWindow = 60 | 300 | 1800 | 3600;
    export function loadSection(): string | null;
    export function saveSection(sectionId: string): void;
    export function loadWindow(): StoredWindow | null;                 // null se assente o non valida
    export function saveWindow(seconds: StoredWindow): void;
    export function loadSeries(sectionId: string): string[] | null;    // null se assente o non un array di stringhe
    export function saveSeries(sectionId: string, ids: string[]): void;
    ```
    Il Task 11 usa `loadWindow`/`saveWindow` e `loadSeries`/`saveSeries`; `StoredWindow` coincide con il suo `WindowSeconds`. Se `loadWindow()` restituisce `null`, la finestra predefinita la sceglie il Task 11.
  - `lib/stale.ts`: `staleAfterMs(intervalMs): number` (= `max(5000, 5 × intervalMs)`), `isStale(lastSnapshotAtMs: number | null, nowMs: number, intervalMs: number): boolean` (`null` → `false`).
  - `LiveStore.lastReceivedAtMs: number | null` (`Date.now()` all'ultimo snapshot nuovo accettato).
  - `TopBar` prop `stale?: boolean` (predefinita `false`); `SimpleView` props `startedAtMs?: number | null` e `onOpenAdvanced: (section: string | null) => void`.
  - **Punto di sostituzione per il Task 13.** `AdvancedView.svelte` mostra, per la sezione aperta:
    ```svelte
    <header><h2>{t(current.labelKey)}</h2>{#if current.labelArg}<p class="device">{current.labelArg}</p>{/if}</header>
    {#key current.id}
      <DevicePage entry={current} {store} {backend} />
    {/key}
    ```
    Il Task 13 sovrascrive per intero `app/src/components/advanced/DevicePage.svelte` e mantiene le props `{ entry: SidebarEntry; store: LiveStore; backend: Backend }`. Non tocca `AdvancedView.svelte` e non ripete l'intestazione (titolo e nome del device restano in `AdvancedView`). Grazie a `{#key}`, a ogni cambio di sezione la pagina viene distrutta e ricreata, quindi timer e polling ripartono da zero. La pagina minima di questo task non ha test propri: i test di `AdvancedView.test.ts` controllano solo barra laterale e intestazione, e restano validi dopo il Task 13. I Task 11 e 12 possono montare temporaneamente i loro componenti in questa pagina minima per le verifiche manuali.

**Comportamento (spec §7.1, §7.2, §7.3; decisioni D2, D10, D12):**
- **Barra laterale**, nell'ordine:
  - CPU (una voce con id `cpu/0` per tutti i device `cpu`);
  - una voce per ogni GPU, iGPU compresa, nell'ordine dello schema;
  - RAM (una voce, id = id del device, `memory/0`);
  - una voce per disco;
  - una voce per scheda di rete;
  - poi gli altri tipi presenti: scheda madre, batteria, controller ventole, alimentatore.

  Le voci i cui device non hanno sensori non compaiono (spec §7.3), quindi in M3 non compare la batteria (decisione D10). Ogni voce mostra il tipo tradotto (`advanced.section.<kind>`) e sotto il nome del device; per la RAM il nome ("RAM") non si ripete.
- **Sezione aperta.** È l'ultima scelta, salvata in `oma.advanced.section`; se quella voce non esiste (più), si apre la prima, cioè la CPU (spec §7.3). La scelta salvata si cambia solo con un clic, non con il ripiego: se una GPU sparisce durante un ricaricamento del driver si vede la CPU, e quando la GPU ricompare torna la sua pagina.
- **Collegamento dai riquadri** (spec §7.2): il clic su un riquadro della vista Semplificata salva la sezione corrispondente e apre la vista Avanzata:
  - CPU → `cpu/0`;
  - GPU → l'id del device di quella GPU;
  - RAM → id del device della memoria;
  - "Rete · Dischi" → la prima scheda di rete, oppure il primo disco se non ci sono schede.

  Se la sezione è `null`, la vista si apre sull'ultima pagina visitata.
- **Dati fermi** (decisione D12). `App` controlla ogni secondo se l'ultimo snapshot è più vecchio di `max(5 s, 5 × intervalMs)`: in quel caso la barra superiore mostra il badge "Dati non aggiornati" (`status.stale`). Prima del primo snapshot il tempo si conta dall'apertura della finestra, così il badge compare anche se il campionatore è fermo già all'apertura. `intervalMs` viene da `getSession()` (1000 finché non arriva la risposta).
- **Banner** (seguito della M1, decisione D2): la durata "da …" parte da `session.startedAtMs`, cioè dal primo tick del core. Se è `null` si usa `store.firstTimestampMs`. Chiudere la finestra nella tray e riaprirla non azzera più la durata.
- `advanced.comingSoon` e `AdvancedPlaceholder.svelte` spariscono.

- [ ] **Step 1: Scrivi i test (falliscono)**

`app/src/lib/advanced/nav.test.ts` (nuovo):

```ts
import { MOCK_SCHEMA } from '../backend/mock';
import type { Schema, Sensor } from '../types';
import { resolveSection, sectionForTile, sidebarEntries } from './nav';

const GPU = 'gpu/pci-0000:01:00.0';
const IGPU = 'gpu/pci-0000:11:00.0';
const HDD = 'storage/device-hdd';

const loadSensor = (deviceId: string): Sensor => ({
  id: `${deviceId}/load/x`,
  deviceId,
  kind: 'load',
  unit: 'percent',
  label: { key: 'x' },
  source: 'mock',
  category: 'load',
});

/** MOCK_SCHEMA plus an iGPU, a second disk, a battery listed first and a NIC without sensors. */
const schema: Schema = {
  ...MOCK_SCHEMA,
  devices: [
    { id: 'battery/0', kind: 'battery', name: 'Battery' },
    ...MOCK_SCHEMA.devices,
    { id: IGPU, kind: 'gpu', name: 'AMD Radeon(TM) Graphics', properties: { integrated: 'true' } },
    { id: HDD, kind: 'storage', name: 'Disk 1 (D:)' },
    { id: 'network/wifi', kind: 'network', name: 'Wi-Fi' },
  ],
  sensors: [...MOCK_SCHEMA.sensors, loadSensor(IGPU), loadSensor(HDD), loadSensor('battery/0')],
};

test('entries follow the spec order and include integrated GPUs', () => {
  expect(sidebarEntries(schema).map((e) => e.id)).toEqual([
    'cpu/0',
    GPU,
    IGPU,
    'memory/0',
    'storage/device-mock-ssd',
    HDD,
    'network/mock-eth',
    'battery/0',
  ]);
});

test('entries carry kind, devices, label key and device name', () => {
  const [cpu, gpu, , memory] = sidebarEntries(schema);
  expect(cpu).toEqual({
    id: 'cpu/0',
    kind: 'cpu',
    deviceIds: ['cpu/0'],
    labelKey: 'advanced.section.cpu',
    labelArg: 'Mock Ryzen 7 7800X3D',
  });
  expect(gpu).toEqual({ id: GPU, kind: 'gpu', deviceIds: [GPU], labelKey: 'advanced.section.gpu', labelArg: 'Mock GeForce RTX 4080' });
  expect(memory).toEqual({ id: 'memory/0', kind: 'memory', deviceIds: ['memory/0'], labelKey: 'advanced.section.memory', labelArg: undefined });
});

test('devices without sensors have no entry', () => {
  expect(sidebarEntries(schema).some((e) => e.id === 'network/wifi')).toBe(false);
  expect(sidebarEntries({ revision: 1, devices: MOCK_SCHEMA.devices, sensors: [] })).toEqual([]);
});

test('simple view tiles map to sections', () => {
  expect(sectionForTile('cpu')).toBe('cpu/0');
  expect(sectionForTile('gpu', GPU)).toBe(GPU);
  expect(sectionForTile('memory')).toBe('memory/0');
  expect(sectionForTile('storage', HDD)).toBe(HDD);
  expect(sectionForTile('network', 'network/mock-eth')).toBe('network/mock-eth');
  expect(sectionForTile('gpu')).toBeNull();
  expect(sectionForTile('network')).toBeNull();
});

test('resolveSection keeps an existing section, else falls back to the first entry', () => {
  const entries = sidebarEntries(schema);
  expect(resolveSection(entries, IGPU)).toBe(IGPU);
  expect(resolveSection(entries, 'gpu/pci-gone')).toBe('cpu/0');
  expect(resolveSection(entries, null)).toBe('cpu/0');
  expect(resolveSection([], 'cpu/0')).toBeNull();
});
```

`app/src/lib/advanced/persist.test.ts` (nuovo):

```ts
import {
  SECTION_KEY,
  WINDOW_KEY,
  loadSection,
  loadSeries,
  loadWindow,
  saveSection,
  saveSeries,
  saveWindow,
  seriesKey,
} from './persist';

beforeEach(() => localStorage.clear());
afterEach(() => vi.restoreAllMocks());

test('keys follow the oma.advanced.* scheme', () => {
  expect(SECTION_KEY).toBe('oma.advanced.section');
  expect(WINDOW_KEY).toBe('oma.advanced.window');
  expect(seriesKey('gpu/pci-0000:01:00.0')).toBe('oma.advanced.series.gpu/pci-0000:01:00.0');
});

test('section round-trips; missing or empty reads as null', () => {
  expect(loadSection()).toBeNull();
  saveSection('gpu/pci-0000:01:00.0');
  expect(localStorage.getItem(SECTION_KEY)).toBe('gpu/pci-0000:01:00.0');
  expect(loadSection()).toBe('gpu/pci-0000:01:00.0');
  localStorage.setItem(SECTION_KEY, '');
  expect(loadSection()).toBeNull();
});

test('only the four chart windows are accepted', () => {
  expect(loadWindow()).toBeNull();
  saveWindow(1800);
  expect(loadWindow()).toBe(1800);
  for (const bad of ['120', 'abc', '']) {
    localStorage.setItem(WINDOW_KEY, bad);
    expect(loadWindow()).toBeNull();
  }
});

test('series are stored as JSON per section and validated on load', () => {
  expect(loadSeries('cpu/0')).toBeNull();
  saveSeries('cpu/0', ['cpu/0/load/total', 'cpu/0/clock/effective']);
  expect(localStorage.getItem('oma.advanced.series.cpu/0')).toBe('["cpu/0/load/total","cpu/0/clock/effective"]');
  expect(loadSeries('cpu/0')).toEqual(['cpu/0/load/total', 'cpu/0/clock/effective']);
  expect(loadSeries('memory/0')).toBeNull();
  for (const bad of ['{', '"x"', '[1,2]', '{"a":1}']) {
    localStorage.setItem(seriesKey('cpu/0'), bad);
    expect(loadSeries('cpu/0')).toBeNull();
  }
});

test('a throwing storage never breaks the view', () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('denied');
  });
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('quota');
  });
  expect(() => saveSection('cpu/0')).not.toThrow();
  expect(() => saveWindow(60)).not.toThrow();
  expect(() => saveSeries('cpu/0', [])).not.toThrow();
  expect(loadSection()).toBeNull();
  expect(loadWindow()).toBeNull();
  expect(loadSeries('cpu/0')).toBeNull();
});
```

`app/src/lib/stale.test.ts` (nuovo):

```ts
import { isStale, staleAfterMs } from './stale';

test('the threshold is five intervals, never under five seconds', () => {
  expect(staleAfterMs(500)).toBe(5000);
  expect(staleAfterMs(1000)).toBe(5000);
  expect(staleAfterMs(2000)).toBe(10_000);
  expect(staleAfterMs(5000)).toBe(25_000);
});

test('data turns stale only after the threshold has passed', () => {
  expect(isStale(10_000, 15_000, 1000)).toBe(false);
  expect(isStale(10_000, 15_001, 1000)).toBe(true);
  expect(isStale(10_000, 19_000, 2000)).toBe(false);
  expect(isStale(10_000, 20_001, 2000)).toBe(true);
});

test('an unknown last snapshot is not stale', () => {
  expect(isStale(null, 1e12, 1000)).toBe(false);
});
```

`app/src/lib/live.test.ts`: aggiungi in fondo al file:

```ts
test('applySnapshot records the local arrival time of new snapshots only', () => {
  const now = vi.spyOn(Date, 'now').mockReturnValue(50_000);
  try {
    const store = new LiveStore();
    store.applySchema(MOCK_SCHEMA);
    expect(store.lastReceivedAtMs).toBeNull();
    store.applySnapshot(snapshot(1));
    expect(store.lastReceivedAtMs).toBe(50_000);

    now.mockReturnValue(60_000);
    store.applySnapshot(snapshot(1)); // duplicate
    store.applySnapshot(snapshot(2, 9)); // other revision
    expect(store.lastReceivedAtMs).toBe(50_000);
    store.applySnapshot(snapshot(2));
    expect(store.lastReceivedAtMs).toBe(60_000);
  } finally {
    now.mockRestore();
  }
});
```

`app/src/components/advanced/AdvancedView.test.ts` (nuovo). Il componente si monta con uno store già popolato (`applySchema` + `applySnapshot`), senza `connect`:

```ts
import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { SECTION_KEY } from '../../lib/advanced/persist';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { i18n } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { Schema } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import AdvancedView from './AdvancedView.svelte';

beforeEach(() => {
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(cleanup);

const GPU = 'gpu/pci-0000:01:00.0';

function renderView(schema: Schema = MOCK_SCHEMA): LiveStore {
  const store = new LiveStore();
  store.applySchema(schema);
  store.applySnapshot({ revision: schema.revision, seq: 1, timestampMs: 1000, values: mockValues(1) });
  render(AdvancedView, { store, backend: new FakeBackend(schema) });
  return store;
}

const sidebar = () => screen.getByRole('navigation', { name: 'Components' });
const entryNames = () => [...sidebar().querySelectorAll('.name')].map((n) => n.textContent);
const title = () => screen.getByRole('heading', { level: 2 }).textContent;
const current = () => within(sidebar()).getByRole('button', { current: 'page' });

test('the sidebar lists the sections in spec order and opens on the CPU', () => {
  renderView();
  expect(entryNames()).toEqual(['CPU', 'GPU', 'RAM', 'Disk', 'Network']);
  expect(title()).toBe('CPU');
  expect(screen.getAllByText('Mock Ryzen 7 7800X3D').length).toBeGreaterThan(0);
  expect(current().textContent).toContain('CPU');
});

test('selecting an entry opens its page and remembers it', async () => {
  renderView();
  await fireEvent.click(within(sidebar()).getByRole('button', { name: /^GPU/ }));
  expect(title()).toBe('GPU');
  expect(current().textContent).toContain('Mock GeForce RTX 4080');
  expect(localStorage.getItem(SECTION_KEY)).toBe(GPU);
});

test('the last visited section is restored', () => {
  localStorage.setItem(SECTION_KEY, 'network/mock-eth');
  renderView();
  expect(title()).toBe('Network');
  expect(current().textContent).toContain('Ethernet');
});

test('a missing section falls back to the CPU without forgetting the choice', () => {
  localStorage.setItem(SECTION_KEY, GPU);
  const noGpu: Schema = {
    ...MOCK_SCHEMA,
    devices: MOCK_SCHEMA.devices.filter((d) => d.kind !== 'gpu'),
    sensors: MOCK_SCHEMA.sensors.filter((s) => !s.deviceId.startsWith('gpu/')),
  };
  const store = renderView(noGpu);
  expect(title()).toBe('CPU');
  expect(localStorage.getItem(SECTION_KEY)).toBe(GPU);

  // The GPU comes back (e.g. after a driver reload): so does its page.
  store.applySchema({ ...MOCK_SCHEMA, revision: 2 });
  flushSync();
  expect(title()).toBe('GPU');
});

test('section labels are translated', () => {
  i18n.locale = 'it';
  renderView();
  expect(screen.getByRole('navigation', { name: 'Componenti' })).toBeTruthy();
  expect([...screen.getByRole('navigation').querySelectorAll('.name')].map((n) => n.textContent)).toEqual([
    'CPU',
    'GPU',
    'RAM',
    'Disco',
    'Rete',
  ]);
});
```

`app/src/App.test.ts`, file completo. Il vecchio test `clicking a tile opens the advanced view, the toggle goes back`, che cercava il testo del segnaposto, è sostituito dal collegamento alla pagina GPU. I test del badge usano timer finti solo per `Date`, `setInterval` e `clearInterval`: le promesse e `vi.waitFor` restano reali.

```ts
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { i18n } from './lib/i18n/index.svelte';
beforeEach(() => { localStorage.clear(); i18n.locale = 'en'; });
afterEach(cleanup);
import { flushSync } from 'svelte';
import App from './App.svelte';
import { SECTION_KEY } from './lib/advanced/persist';
import { MOCK_SCHEMA, mockValues } from './lib/backend/mock';
import { LiveStore } from './lib/live.svelte';
import type { Schema } from './lib/types';
import { FakeBackend } from './test/fake-backend';

const IGPU = 'gpu/pci-0000:11:00.0';

/** MOCK_SCHEMA plus an integrated AMD GPU with a load sensor (value 12). */
const withIntegratedGpu = (schema: Schema): Schema => ({
  ...schema,
  devices: [
    ...schema.devices,
    { id: IGPU, kind: 'gpu', name: 'AMD Radeon(TM) Graphics', vendor: 'AMD', properties: { integrated: 'true' } },
  ],
  sensors: [
    ...schema.sensors,
    { id: `${IGPU}/load/core`, deviceId: IGPU, kind: 'load', unit: 'percent', label: { key: 'gpu.load.core' }, source: 'pdh', category: 'load' },
  ],
});

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
  expect(screen.getByText((text) => text.startsWith('C: 65% · '))).toBeTruthy();
  expect(screen.queryByText('Re-enable')).toBeNull();
});

test('simple view shows only discrete gpus', async () => {
  const backend = new FakeBackend(withIntegratedGpu(MOCK_SCHEMA));
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: Date.now(), values: [...mockValues(1), 12] });
  flushSync();

  expect(screen.getAllByText('GPU')).toHaveLength(1);
  expect(screen.getByText('Mock GeForce RTX 4080')).toBeTruthy();
  expect(screen.getByText((text) => text.startsWith('VRAM ') && text.endsWith(' / 16.0 GB'))).toBeTruthy();
  expect(screen.queryByText('AMD Radeon(TM) Graphics')).toBeNull();
});

test('simple view falls back to the integrated gpu', async () => {
  const noDiscrete: Schema = {
    ...MOCK_SCHEMA,
    devices: MOCK_SCHEMA.devices.filter((d) => d.kind !== 'gpu'),
    sensors: MOCK_SCHEMA.sensors.filter((s) => !s.deviceId.startsWith('gpu/')),
  };
  const backend = new FakeBackend(withIntegratedGpu(noDiscrete));
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  flushSync();

  expect(screen.getAllByText('GPU')).toHaveLength(1);
  expect(screen.getByText('AMD Radeon(TM) Graphics')).toBeTruthy();
});

test('safe mode notice reenables vendor libraries', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.startup = { safeMode: true, reason: 'crash', crashModule: 'nvml.dll' };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(screen.getByText('GPU safe mode')).toBeTruthy());
  expect(screen.getByText((text) => text.startsWith('The previous session crashed in nvml.dll.'))).toBeTruthy();
  await fireEvent.click(screen.getByRole('button', { name: 'Re-enable' }));

  expect(backend.enableCalls).toBe(1);
  await vi.waitFor(() => expect(screen.queryByText('GPU safe mode')).toBeNull());
});

test('safe mode notice explains the --safe flag in Italian', async () => {
  i18n.locale = 'it';
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.startup = { safeMode: true, reason: 'flag', crashModule: null };
  render(App, { backend, store: new LiveStore() });

  await vi.waitFor(() => expect(screen.getByText('Modalità sicura GPU')).toBeTruthy());
  expect(screen.getByText((text) => text.startsWith('Avvio con --safe:'))).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Riattiva' })).toBeTruthy();
});


test('clicking the GPU tile opens its Advanced page, the toggle goes back', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: 1000, values: mockValues(1) });
  flushSync();

  await fireEvent.click(screen.getByText('Mock GeForce RTX 4080').closest('button')!);
  expect(screen.getByRole('heading', { level: 2 }).textContent).toBe('GPU');
  expect(localStorage.getItem(SECTION_KEY)).toBe('gpu/pci-0000:01:00.0');
  expect(localStorage.getItem('oma.view')).toBe('advanced');

  await fireEvent.click(screen.getByRole('tab', { name: 'Simple' }));
  expect(screen.getByText('Monitoring active')).toBeTruthy();
});

test('the network tile opens the network page', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  flushSync();

  await fireEvent.click(screen.getByText('Network · Disks').closest('button')!);
  expect(screen.getByRole('heading', { level: 2 }).textContent).toBe('Network');
  expect(localStorage.getItem(SECTION_KEY)).toBe('network/mock-eth');
});

test('the health banner counts from the start of the core session', async () => {
  const now = 50_000_000;
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.session = { startedAtMs: now - 125 * 60_000, intervalMs: 1000 };
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: now, values: mockValues(1) });

  await vi.waitFor(() => expect(screen.getByText('for 2 h 5 min')).toBeTruthy());
});

test('without a session start the banner counts from the first snapshot', async () => {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const store = new LiveStore();
  render(App, { backend, store });
  await vi.waitFor(() => expect(store.schema).not.toBeNull());
  backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: 50_000_000, values: mockValues(1) });
  flushSync();

  expect(screen.getByText('for 0 min')).toBeTruthy();
});

test('the stale badge appears after five silent seconds and goes away with new data', async () => {
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  try {
    vi.setSystemTime(1_000_000);
    const backend = new FakeBackend(MOCK_SCHEMA);
    const store = new LiveStore();
    render(App, { backend, store });
    await vi.waitFor(() => expect(store.schema).not.toBeNull());
    backend.emitSnapshot({ revision: 1, seq: 1, timestampMs: Date.now(), values: mockValues(1) });

    vi.advanceTimersByTime(5000);
    flushSync();
    expect(screen.queryByText('Data not updating')).toBeNull();

    vi.advanceTimersByTime(1000);
    flushSync();
    expect(screen.getByText('Data not updating')).toBeTruthy();

    backend.emitSnapshot({ revision: 1, seq: 2, timestampMs: Date.now(), values: mockValues(2) });
    vi.advanceTimersByTime(1000);
    flushSync();
    expect(screen.queryByText('Data not updating')).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

test('the stale badge also appears when no snapshot ever arrives', async () => {
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  try {
    vi.setSystemTime(1_000_000);
    const backend = new FakeBackend(MOCK_SCHEMA);
    const store = new LiveStore();
    render(App, { backend, store });
    await vi.waitFor(() => expect(store.schema).not.toBeNull());

    vi.advanceTimersByTime(6000);
    flushSync();
    expect(screen.getByText('Data not updating')).toBeTruthy();
  } finally {
    vi.useRealTimers();
  }
});
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

```bash
cd app && pnpm test
```

Risultato atteso: `Test Files  6 failed | 9 passed (15)` e `Tests  1 failed | 71 passed (72)`. I file che non si caricano non contano i loro test. Errori:
- `Failed to resolve import "./nav"` (`nav.test.ts`), `"./persist"` (`persist.test.ts`), `"./stale"` (`stale.test.ts`);
- `Failed to resolve import "../../lib/advanced/persist"` (`AdvancedView.test.ts`) e `"./lib/advanced/persist"` (`App.test.ts`);
- `applySnapshot records the local arrival time of new snapshots only`: `expected undefined to be null`.

- [ ] **Step 3: Implementa navigazione, persistenza, soglia e `lastReceivedAtMs`**

`app/src/lib/advanced/nav.ts` (nuovo):

```ts
import type { DeviceKind, Schema } from '../types';

/** One sidebar entry of the Advanced view: a page over one or more devices. */
export interface SidebarEntry {
  /** Section id: the device id (the CPU entry is always 'cpu/0'). Persisted in localStorage. */
  id: string;
  kind: DeviceKind;
  deviceIds: string[];
  /** `advanced.section.<kind>`. */
  labelKey: string;
  /** Device name shown under the label (not translated); absent for memory. */
  labelArg?: string;
}

export type SimpleTile = 'cpu' | 'gpu' | 'memory' | 'storage' | 'network';

const CPU_SECTION = 'cpu/0';
const MEMORY_SECTION = 'memory/0';
/** Kinds with one entry for all their devices; every other kind gets one entry per device. */
const MERGED: readonly DeviceKind[] = ['cpu', 'memory'];
const ORDER: readonly DeviceKind[] = ['cpu', 'gpu', 'memory', 'storage', 'network', 'motherboard', 'battery', 'fan_controller', 'psu'];

/**
 * Sidebar order (spec §7.3): CPU, every GPU (integrated ones too), memory, one entry per
 * disk, one per network adapter, then any other kind. Devices without sensors are skipped.
 */
export function sidebarEntries(schema: Schema): SidebarEntry[] {
  const withSensors = new Set(schema.sensors.map((s) => s.deviceId));
  const entries: SidebarEntry[] = [];
  for (const kind of ORDER) {
    const devices = schema.devices.filter((d) => d.kind === kind && withSensors.has(d.id));
    if (devices.length === 0) continue;
    const labelKey = `advanced.section.${kind}`;
    if (MERGED.includes(kind)) {
      entries.push({
        id: kind === 'cpu' ? CPU_SECTION : devices[0].id,
        kind,
        deviceIds: devices.map((d) => d.id),
        labelKey,
        labelArg: kind === 'cpu' ? devices[0].name : undefined,
      });
    } else {
      for (const d of devices) entries.push({ id: d.id, kind, deviceIds: [d.id], labelKey, labelArg: d.name });
    }
  }
  return entries;
}

/** Section a Simple view tile opens; null lets the Advanced view keep its last page. */
export function sectionForTile(tile: SimpleTile, deviceId?: string): string | null {
  switch (tile) {
    case 'cpu':
      return CPU_SECTION;
    case 'memory':
      return deviceId ?? MEMORY_SECTION;
    default:
      return deviceId ?? null;
  }
}

/** The wanted section if it still exists, else the first entry (CPU), else null. */
export function resolveSection(entries: SidebarEntry[], wanted: string | null): string | null {
  if (wanted !== null && entries.some((e) => e.id === wanted)) return wanted;
  return entries[0]?.id ?? null;
}
```

`app/src/lib/advanced/persist.ts` (nuovo):

```ts
// Advanced view state in localStorage (settings.json arrives with milestone 5).
// Storage can be missing or throw (quota, privacy mode): every access is guarded.

export const SECTION_KEY = 'oma.advanced.section';
export const WINDOW_KEY = 'oma.advanced.window';
export const seriesKey = (sectionId: string) => `oma.advanced.series.${sectionId}`;

export const STORED_WINDOWS = [60, 300, 1800, 3600] as const;
export type StoredWindow = (typeof STORED_WINDOWS)[number];

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not persisted: the view still works for this session.
  }
}

export function loadSection(): string | null {
  const value = read(SECTION_KEY);
  return value ? value : null;
}

export function saveSection(sectionId: string): void {
  write(SECTION_KEY, sectionId);
}

/** The saved chart window, or null when absent or not one of 60/300/1800/3600. */
export function loadWindow(): StoredWindow | null {
  const value = Number(read(WINDOW_KEY));
  return STORED_WINDOWS.find((w) => w === value) ?? null;
}

export function saveWindow(seconds: StoredWindow): void {
  write(WINDOW_KEY, String(seconds));
}

/** Sensor ids charted on a section, or null when never saved or unreadable. */
export function loadSeries(sectionId: string): string[] | null {
  const raw = read(seriesKey(sectionId));
  if (raw === null) return null;
  try {
    const value: unknown = JSON.parse(raw);
    return Array.isArray(value) && value.every((v) => typeof v === 'string') ? value : null;
  } catch {
    return null;
  }
}

export function saveSeries(sectionId: string, ids: string[]): void {
  write(seriesKey(sectionId), JSON.stringify(ids));
}
```

`app/src/lib/stale.ts` (nuovo):

```ts
/** M3 decision D12: no snapshot for max(5 s, 5 intervals) means the core stopped sampling. */
export function staleAfterMs(intervalMs: number): number {
  return Math.max(5000, 5 * intervalMs);
}

/** True when the last snapshot is older than the stale threshold; false when unknown (null). */
export function isStale(lastSnapshotAtMs: number | null, nowMs: number, intervalMs: number): boolean {
  return lastSnapshotAtMs !== null && nowMs - lastSnapshotAtMs > staleAfterMs(intervalMs);
}
```

`app/src/lib/live.svelte.ts`, due modifiche nella classe `LiveStore`. Dopo `firstTimestampMs = $state(0);`:

```ts
  firstTimestampMs = $state(0);
  /** Local clock (Date.now()) when the last new snapshot arrived; drives the stale badge. */
  lastReceivedAtMs = $state<number | null>(null);
```

In `applySnapshot`, dopo `this.#lastSeq = snapshot.seq;`. Gli snapshot duplicati e quelli di un'altra revisione escono prima, quindi non aggiornano l'ora:

```ts
    this.#lastSeq = snapshot.seq;
    this.lastReceivedAtMs = Date.now();
    this.values = snapshot.values;
```

L'ora è quella locale (`Date.now()`) e non il `timestampMs` dello snapshot: così un orologio del core diverso da quello della WebView non fa scattare il badge.

- [ ] **Step 4: Implementa i componenti e le traduzioni**

`app/src/components/advanced/Sidebar.svelte` (nuovo):

```svelte
<script lang="ts">
  import type { SidebarEntry } from '../../lib/advanced/nav';
  import { t } from '../../lib/i18n/index.svelte';

  let {
    entries,
    selected,
    onSelect,
  }: { entries: SidebarEntry[]; selected: string | null; onSelect: (id: string) => void } = $props();
</script>

<nav class="sidebar" aria-label={t('advanced.sidebar')}>
  {#each entries as entry (entry.id)}
    <button
      type="button"
      class:on={entry.id === selected}
      aria-current={entry.id === selected ? 'page' : undefined}
      onclick={() => onSelect(entry.id)}
    >
      <span class="name">{t(entry.labelKey)}</span>
      {#if entry.labelArg}<span class="arg">{entry.labelArg}</span>{/if}
    </button>
  {/each}
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 4px;
    position: sticky;
    top: 76px;
  }
  button {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 8px 12px;
    text-align: left;
    cursor: pointer;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    border-radius: 0 8px 8px 0;
  }
  button:hover {
    background: var(--surface);
  }
  button.on {
    background: var(--surface-2);
    border-left-color: var(--accent);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .name {
    font-weight: 600;
  }
  .arg {
    overflow: hidden;
    font-size: 12px;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
```

`app/src/components/advanced/DevicePage.svelte` (nuovo). È la pagina minima che il Task 13 sostituisce per intero, con le stesse props. Mostra il valore attuale di ogni sensore della sezione, formattato con `formatValue`:

```svelte
<script lang="ts">
  import type { SidebarEntry } from '../../lib/advanced/nav';
  import type { Backend } from '../../lib/backend';
  import { formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';

  // Minimal page: the live value of every sensor of the section. The full device page
  // (KPIs, history chart, sensor table, device info) replaces this file with the same props.
  let { entry, store }: { entry: SidebarEntry; store: LiveStore; backend: Backend } = $props();

  const sensors = $derived(store.schema?.sensors.filter((s) => entry.deviceIds.includes(s.deviceId)) ?? []);
</script>

<ul class="sensors">
  {#each sensors as sensor (sensor.id)}
    <li>
      <span>{t(`sensor.${sensor.label.key}`, { arg: sensor.label.arg ?? '' })}</span>
      <span class="value">{formatValue(store.value(sensor.id), sensor.unit, i18n.locale, t)}</span>
    </li>
  {/each}
</ul>

<style>
  .sensors {
    margin: 0;
    padding: 0;
    list-style: none;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 14px;
    border-top: 1px solid var(--border);
  }
  li:first-child {
    border-top: 0;
  }
  .value {
    font-variant-numeric: tabular-nums;
  }
</style>
```

`app/src/components/advanced/AdvancedView.svelte` (nuovo):

```svelte
<script lang="ts">
  import { resolveSection, sidebarEntries } from '../../lib/advanced/nav';
  import { loadSection, saveSection } from '../../lib/advanced/persist';
  import type { Backend } from '../../lib/backend';
  import { t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import DevicePage from './DevicePage.svelte';
  import Sidebar from './Sidebar.svelte';

  let { store, backend }: { store: LiveStore; backend: Backend } = $props();

  // The section the user asked for. It is kept (and stays saved) while its device is
  // missing, so the page comes back when the device does.
  let wanted = $state(loadSection());
  const entries = $derived(store.schema ? sidebarEntries(store.schema) : []);
  const current = $derived(entries.find((e) => e.id === resolveSection(entries, wanted)) ?? null);

  function select(id: string) {
    wanted = id;
    saveSection(id);
  }
</script>

<div class="advanced">
  <Sidebar {entries} selected={current?.id ?? null} onSelect={select} />
  {#if current}
    <section class="page">
      <header>
        <h2>{t(current.labelKey)}</h2>
        {#if current.labelArg}<p class="device">{current.labelArg}</p>{/if}
      </header>
      {#key current.id}
        <DevicePage entry={current} {store} {backend} />
      {/key}
    </section>
  {/if}
</div>

<style>
  .advanced {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr);
    gap: 20px;
    align-items: start;
  }
  .page {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 12px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: 20px;
  }
  .device {
    margin: 0;
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
```

Elimina il segnaposto:

```bash
git rm app/src/components/advanced/AdvancedPlaceholder.svelte
```

`app/src/components/TopBar.svelte`, file completo:

```svelte
<script lang="ts">
  import { t } from '../lib/i18n/index.svelte';
  import type { View } from '../lib/view';

  let {
    view,
    onViewChange,
    serviceAvailable,
    stale = false,
  }: { view: View; onViewChange: (view: View) => void; serviceAvailable: boolean; stale?: boolean } = $props();
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
    {#if stale}
      <span class="stale" role="status">{t('status.stale')}</span>
    {/if}
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
    color: var(--on-accent);
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
  .stale {
    padding: 4px 10px;
    font-size: 12px;
    border-radius: 999px;
    color: var(--crit);
    border: 1px solid color-mix(in srgb, var(--crit) 45%, transparent);
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

`app/src/components/simple/SimpleView.svelte`, file completo:

```svelte
<script lang="ts">
  import { formatBytes, formatClock, formatPercent, formatPower, formatRate, formatTemperature } from '../../lib/format';
  import { sectionForTile } from '../../lib/advanced/nav';
  import { monitoringHealth } from '../../lib/health';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { DeviceKind } from '../../lib/types';
  import {
    cpuSummary,
    gpuSummaries,
    memorySummary,
    networkSummary,
    simpleViewGpus,
    storageSummary,
    sumSeries,
  } from '../../lib/select';
  import AnimatedNumber from '../common/AnimatedNumber.svelte';
  import Sparkline from '../common/Sparkline.svelte';
  import HealthBanner from './HealthBanner.svelte';
  import Tile from './Tile.svelte';

  let {
    store,
    startedAtMs = null,
    onOpenAdvanced,
  }: {
    store: LiveStore;
    /** Start of the core's sampling session (it outlives the window); null while unknown. */
    startedAtMs?: number | null;
    /** Opens the Advanced view on a section (null keeps its last page). */
    onOpenAdvanced: (section: string | null) => void;
  } = $props();

  const valueOf = (id: string) => store.value(id);
  const locale = $derived(i18n.locale);
  const cpu = $derived(store.schema ? cpuSummary(store.schema, valueOf) : null);
  const gpus = $derived(store.schema ? simpleViewGpus(gpuSummaries(store.schema, valueOf)) : []);
  const mem = $derived(store.schema ? memorySummary(store.schema, valueOf) : null);
  const disk = $derived(store.schema ? storageSummary(store.schema, valueOf) : null);
  const net = $derived(store.schema ? networkSummary(store.schema, valueOf) : null);
  const netSeries = $derived(net ? sumSeries(net.downIds.map((id) => store.series(id))) : []);
  const health = $derived(monitoringHealth(startedAtMs ?? store.firstTimestampMs));
  const firstDevice = (kind: DeviceKind) => store.schema?.devices.find((d) => d.kind === kind)?.id;
  const netDiskSection = $derived(
    net ? sectionForTile('network', firstDevice('network')) : sectionForTile('storage', firstDevice('storage')),
  );
</script>

<div class="simple">
  {#if store.timestampMs > 0}<HealthBanner {health} nowMs={store.timestampMs} />{/if}

  <div class="grid">
    {#if cpu}
      <Tile label={t('tile.cpu')} onclick={() => onOpenAdvanced(sectionForTile('cpu'))}>
        <div class="big"><AnimatedNumber value={cpu.load} format={(v) => formatPercent(v, locale)} /></div>
        <div class="sub">{cpu.name} · {formatClock(cpu.clockMhz, locale)}</div>
        {#if cpu.loadId}
          <Sparkline values={store.series(cpu.loadId)} capacity={store.capacity} max={100} />
        {/if}
      </Tile>
    {/if}

    {#each gpus as gpu (gpu.deviceId)}
      <Tile label={t('tile.gpu')} onclick={() => onOpenAdvanced(sectionForTile('gpu', gpu.deviceId))}>
        <div class="big"><AnimatedNumber value={gpu.load} format={(v) => formatPercent(v, locale)} /></div>
        <div class="sub">{gpu.name}</div>
        <div class="sub">
          {formatTemperature(gpu.temperatureC, locale)} · {formatClock(gpu.clockMhz, locale)} · {formatPower(gpu.powerW, locale)}
        </div>
        {#if gpu.memUsedBytes !== null && gpu.memTotalBytes !== null}
          <div class="sub">
            {t('tile.vram', { used: formatBytes(gpu.memUsedBytes, locale), total: formatBytes(gpu.memTotalBytes, locale) })}
          </div>
        {/if}
        {#if gpu.loadId}
          <Sparkline values={store.series(gpu.loadId)} capacity={store.capacity} max={100} />
        {/if}
      </Tile>
    {/each}

    {#if mem}
      <Tile label={t('tile.memory')} onclick={() => onOpenAdvanced(sectionForTile('memory', firstDevice('memory')))}>
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
      <Tile label={t('tile.netDisk')} onclick={() => onOpenAdvanced(netDiskSection)}>
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
            {#if disk.volume}{disk.volume.letter} {formatPercent(disk.volume.usedPct, locale)}{' · '}{/if}{t('tile.diskIo', {
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

`app/src/App.svelte`, file completo:

```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import AdvancedView from './components/advanced/AdvancedView.svelte';
  import SafeModeNotice from './components/SafeModeNotice.svelte';
  import SimpleView from './components/simple/SimpleView.svelte';
  import TopBar from './components/TopBar.svelte';
  import { saveSection } from './lib/advanced/persist';
  import { createBackend, type Backend } from './lib/backend';
  import { LiveStore, connect } from './lib/live.svelte';
  import { isStale } from './lib/stale';
  import type { Session, StartupStatus } from './lib/types';
  import type { View } from './lib/view';

  let { backend = createBackend(), store = new LiveStore() }: { backend?: Backend; store?: LiveStore } = $props();
  let view = $state<View>((localStorage.getItem('oma.view') === 'advanced') ? 'advanced' : 'simple');
  let visible = $state(!document.hidden);
  let startup = $state<StartupStatus | null>(null);
  let session = $state<Session | null>(null);
  // Until the first snapshot arrives, silence is measured from the moment the window opened.
  const openedAtMs = Date.now();
  let nowMs = $state(openedAtMs);
  const stale = $derived(isStale(store.lastReceivedAtMs ?? openedAtMs, nowMs, session?.intervalMs ?? 1000));
  $effect(() => { localStorage.setItem('oma.view', view); });

  onMount(() => {
    const visibility = () => { visible = !document.hidden; };
    document.addEventListener('visibilitychange', visibility);
    const clock = setInterval(() => { nowMs = Date.now(); }, 1000);
    let off: (() => void) | undefined;
    let cancelled = false;
    connect(store, backend)
      .then((unsubscribe) => {
        if (cancelled) unsubscribe();
        else off = unsubscribe;
      })
      .catch((error) => console.error('backend connection failed', error));
    backend
      .getStartupStatus()
      .then((status) => {
        if (!cancelled) startup = status;
      })
      .catch((error) => console.error('startup status unavailable', error));
    backend
      .getSession()
      .then((value) => {
        if (!cancelled) session = value;
      })
      .catch((error) => console.error('session unavailable', error));
    return () => {
      cancelled = true;
      clearInterval(clock);
      document.removeEventListener('visibilitychange', visibility);
      off?.();
    };
  });

  async function enableVendorLibraries() {
    try {
      startup = await backend.enableVendorLibraries();
    } catch (error) {
      console.error('cannot re-enable the GPU vendor libraries', error);
    }
  }

  function openAdvanced(section: string | null) {
    if (section !== null) saveSection(section);
    view = 'advanced';
  }
</script>

<TopBar {view} onViewChange={(v) => (view = v)} serviceAvailable={false} {stale} />
<main>
  {#if startup?.safeMode}
    <SafeModeNotice status={startup} onEnable={enableVendorLibraries} />
  {/if}
  {#if visible}
  {#if view === 'simple'}
    <SimpleView {store} startedAtMs={session?.startedAtMs ?? null} onOpenAdvanced={openAdvanced} />
  {:else}
    <AdvancedView {store} {backend} />
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

`app/src/lib/i18n/en.json`: sostituisci la riga `"advanced.comingSoon": "The Advanced view arrives in milestone 3.",` con:

```json
  "advanced.sidebar": "Components",
  "advanced.section.cpu": "CPU",
  "advanced.section.gpu": "GPU",
  "advanced.section.memory": "RAM",
  "advanced.section.storage": "Disk",
  "advanced.section.network": "Network",
  "advanced.section.motherboard": "Motherboard",
  "advanced.section.battery": "Battery",
  "advanced.section.fan_controller": "Fan controller",
  "advanced.section.psu": "Power supply",
```

`app/src/lib/i18n/it.json`: sostituisci la riga `"advanced.comingSoon": "La vista Avanzata arriva con la milestone 3.",` con:

```json
  "advanced.sidebar": "Componenti",
  "advanced.section.cpu": "CPU",
  "advanced.section.gpu": "GPU",
  "advanced.section.memory": "RAM",
  "advanced.section.storage": "Disco",
  "advanced.section.network": "Rete",
  "advanced.section.motherboard": "Scheda madre",
  "advanced.section.battery": "Batteria",
  "advanced.section.fan_controller": "Controller ventole",
  "advanced.section.psu": "Alimentatore",
```

Le chiavi `advanced.section.fan_controller` e `advanced.section.psu` coprono gli ultimi due `DeviceKind`: nessun provider M3 li produce, ma `sidebarEntries` li elenca se compaiono.

- [ ] **Step 5: Esegui i test e verifica che passino**

```bash
cd app
pnpm test
pnpm check
pnpm build
```

Risultato atteso:
- `pnpm test`: `Test Files  15 passed (15)`, `Tests  101 passed (101)`;
- `pnpm check`: `0 ERRORS 0 WARNINGS`;
- `pnpm build`: OK.

- [ ] **Step 6: Verifica nel browser con il backend mock (CDP, senza input sul desktop)**

1. In un terminale a parte avvia `cd app && pnpm dev` e attendi `Local: http://localhost:1420/`.
2. Salva questo script come `$env:TEMP\oma-advanced-cdp.mjs`. È un file di verifica usa e getta, da non aggiungere al repository. Pilota un Edge headless tramite CDP: il clic sul riquadro è un evento DOM dentro il browser headless, non un input sul desktop.

   ```js
   // Checks the Advanced view of `pnpm dev` in a headless Edge through CDP (DOM-level only,
   // no desktop input). Usage: node oma-advanced-cdp.mjs [port]
   const port = Number(process.argv[2] ?? 9333);
   const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

   async function connect(url) {
     const ws = new WebSocket(url);
     await new Promise((r) => ws.addEventListener('open', r));
     let id = 0;
     const pending = new Map();
     ws.addEventListener('message', (e) => {
       const msg = JSON.parse(e.data);
       if (msg.id && pending.has(msg.id)) pending.get(msg.id)(msg);
     });
     const send = (method, params = {}) =>
       new Promise((r) => {
         const n = ++id;
         pending.set(n, r);
         ws.send(JSON.stringify({ id: n, method, params }));
       });
     return { ws, send };
   }

   let targets = [];
   for (let i = 0; i < 40 && !targets.some((t) => t.type === 'page'); i++) {
     try {
       targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
     } catch {
       await sleep(500);
     }
   }
   const page = await connect(targets.find((t) => t.type === 'page').webSocketDebuggerUrl);
   const run = async (expression) =>
     (await page.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.result.value;

   await page.send('Page.navigate', { url: 'http://localhost:1420/' });
   await sleep(2500);
   await run(`localStorage.clear(); location.reload(); true`);
   await sleep(3000);
   console.log('LANG:', await run(`document.documentElement.lang`));
   console.log('BANNER:', await run(`document.querySelector('[role=status] .title')?.textContent`));
   // Click the GPU tile from page JS (a DOM event inside the headless browser).
   await run(`[...document.querySelectorAll('button.tile')].find((b) => b.textContent.includes('Mock GeForce'))?.click(); true`);
   await sleep(500);
   console.log('SIDEBAR:', await run(`[...document.querySelectorAll('nav .name')].map((n) => n.textContent).join(' | ')`));
   console.log('CURRENT:', await run(`document.querySelector('nav [aria-current=page]')?.textContent`));
   console.log('TITLE:', await run(`document.querySelector('h2')?.textContent + ' / ' + document.querySelector('.device')?.textContent`));
   console.log('SAVED:', await run(`localStorage.getItem('oma.view') + ' ' + localStorage.getItem('oma.advanced.section')`));
   console.log('ROWS:', await run(`[...document.querySelectorAll('.sensors li')].map((l) => l.textContent.replace(/\\s+/g, ' ').trim()).join(' | ')`));
   await run(`location.reload(); true`);
   await sleep(2500);
   console.log('AFTER RELOAD:', await run(`document.querySelector('h2')?.textContent`));
   console.log('STALE BADGE:', await run(`document.querySelector('.stale') !== null`));
   page.ws.close();

   const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
   const browser = await connect(version.webSocketDebuggerUrl);
   await browser.send('Browser.close');
   process.exit(0);
   ```
3. In PowerShell:
   ```powershell
   $edgeProfile = Join-Path $env:TEMP 'oma-cdp-profile'
   Start-Process "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe" -ArgumentList '--headless=new', '--disable-gpu', '--lang=en-US', "--user-data-dir=$edgeProfile", '--remote-debugging-port=9333', 'about:blank'
   node "$env:TEMP\oma-advanced-cdp.mjs" 9333
   ```
   Risultato atteso (verificato durante la stesura; i valori numerici cambiano a ogni esecuzione):
   ```
   LANG: en
   BANNER: Monitoring active
   SIDEBAR: CPU | GPU | RAM | Disk | Network
   CURRENT: GPU Mock GeForce RTX 4080
   TITLE: GPU / Mock GeForce RTX 4080
   SAVED: advanced gpu/pci-0000:01:00.0
   ROWS: GPU load 71% | Dedicated memory used 5.2 GB | Dedicated memory total 16.0 GB | Core temperature 61 °C | Hotspot temperature 77 °C | Core clock 2.35 GHz | Board power 230 W
   AFTER RELOAD: GPU
   STALE BADGE: false
   ```
   Lo script chiude Edge con `Browser.close`. Controlla che non restino processi: `Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" | Where-Object CommandLine -like '*oma-cdp-profile*'` non deve restituire nulla. Poi chiudi `pnpm dev` con Ctrl+C.

- [ ] **Step 7: Verifica manuale (utente)**

1. **Browser.** Con `pnpm dev` attivo apri `http://localhost:1420`:
   - un clic sul riquadro "Rete · Dischi" apre la pagina "Rete" con "Ethernet";
   - torna a "Semplice" e fai clic su "CPU": si apre la pagina "CPU", e nella barra laterale è evidenziata la voce CPU;
   - a 900 px di larghezza la barra laterale (200 px) e la pagina stanno affiancate senza scorrimento orizzontale.
2. **App vera** (richiede i Task 1–8). Esegui `cd app && pnpm tauri build --no-bundle && cd ..`, chiudi ogni istanza e avvia `.\target\release\oma-app.exe`:
   - la barra laterale elenca CPU, due voci GPU ("NVIDIA GeForce RTX 4080" e "AMD Radeon(TM) Graphics"), RAM, quattro dischi e le schede di rete con dati;
   - lascia l'app aperta almeno 2 minuti, chiudi la finestra (resta nella tray), attendi un minuto e riaprila dalla tray: il banner della vista Semplificata dice "da 3 min" circa e non riparte da "da 0 min". Era il seguito "banner sbagliato dopo la riapertura dalla tray" della M1;
   - nella vista Avanzata, dopo la riapertura, compare l'ultima pagina visitata;
   - il badge "Dati non aggiornati" non compare durante l'uso normale.

- [ ] **Step 8: Commit**

```bash
git add app/src
git commit -m "feat(ui): Advanced view shell with sidebar, deep links, stale badge and session-based banner"
```

---

### Task 11: UI: grafico storico uPlot (`chartData.ts`, `HistoryChart.svelte`)

**File:**
- Crea:
  - `app/src/lib/advanced/chartData.ts` (buffer del grafico, limiti di serie e di unità, scale, palette)
  - `app/src/lib/advanced/labels.ts` (nome tradotto di un sensore, usato anche dal Task 12)
  - `app/src/components/advanced/HistoryChart.svelte` (uPlot, finestra 1m/5m/30m/1h, scelta delle serie, pausa quando la finestra non è visibile)
- Modifica:
  - `app/src/test-setup.ts` (in tutti i test `uplot` è sostituito da uno stub)
  - `app/src/lib/i18n/en.json`, `app/src/lib/i18n/it.json` (chiavi `advanced.chart.*`)
- Test:
  - Crea: `app/src/test/uplot-stub.ts`, `app/src/lib/advanced/chartData.test.ts`, `app/src/components/advanced/HistoryChart.test.ts`

**Interfacce:**
- Usa:
  - Task 9:
    - la dipendenza `uplot` 1.6.32, con i tipi in `uplot/dist/uPlot.d.ts` e il CSS in `uplot/dist/uPlot.min.css`;
    - `Backend.getHistory(ids, seconds, maxPoints?)` e `HistorySeed`;
    - `formatValue(value, unit, locale, t)`;
    - i token `--series-5` … `--series-8` di `theme.css`;
    - `FakeBackend.history` (dal più vecchio; `getHistory` restituisce gli ultimi `seconds` campioni, senza sottocampionare) e `FakeBackend.historyCalls` (`{ ids, seconds, maxPoints }`).
  - Task 10:
    - `lib/advanced/persist.ts`: `loadWindow(): StoredWindow | null`, `saveWindow(seconds)`, `loadSeries(sectionId): string[] | null`, `saveSeries(sectionId, ids)`, `WINDOW_KEY`, `seriesKey(sectionId)`, `STORED_WINDOWS`;
    - `LiveStore`: `schema`, `timestampMs`, `value(id)`.
  - M1: `DASH`, `i18n`, `t`.
- Produce:
  - `lib/advanced/chartData.ts`:
    ```ts
    export const WINDOWS = [60, 300, 1800, 3600] as const;       // stessi valori di STORED_WINDOWS
    export type WindowSeconds = (typeof WINDOWS)[number];
    export const DEFAULT_WINDOW: WindowSeconds;                    // 300
    export const MAX_SERIES = 8;
    export const MAX_UNITS = 2;
    export const DECIMATE_FROM = 1800;
    export const MAX_POINTS = 900;
    export function maxPointsFor(windowSeconds: number): number | undefined;
    export class ChartBuffer {
      constructor(ids: string[], windowSeconds: number);
      readonly ids: string[];
      readonly windowSeconds: number;
      readonly length: number;
      readonly lastTimestampMs: number | null;
      seed(h: HistorySeed): void;
      append(timestampMs: number, values: (number | null)[]): void;
      trim(nowMs: number): void;
      data(): uPlot.AlignedData;                                   // [x in secondi, ...serie]
    }
    export function unitsOf(ids: string[], schema: Schema): Unit[];
    export function canAdd(selected: string[], candidate: string, schema: Schema): boolean;
    export function fitSelection(ids: string[], candidates: string[], schema: Schema): string[];
    export function initialSeries(saved: string[] | null, candidates: string[], defaults: string[], schema: Schema): string[];
    export const PALETTE_TOKENS: readonly ['--accent', '--accent-2', '--ok', '--warn', '--series-5', '--series-6', '--series-7', '--series-8'];
    export function seriesPalette(read: (token: string) => string): string[];
    export function scaleLayout(ids: string[], schema: Schema): { scales: Unit[]; seriesScale: Unit[] };
    export function scaleOptions(unit: Unit): uPlot.Scale;
    ```
  - `lib/advanced/labels.ts`: `sensorLabel(sensor: Sensor, t: Translate): string` (`sensor.<label.key>` con `{arg}`);
  - `HistoryChart.svelte`, props `{ sectionId: string; sensors: Sensor[]; defaults: string[]; schema: Schema; store: LiveStore; backend: Backend }`. Lo monta il Task 13 dentro `DevicePage`; `sensors` sono i sensori della pagina, cioè le serie offerte dal selettore;
  - `test/uplot-stub.ts`: la classe `FakeUplot` (`static instances`, `opts`, `data`, `setDataCalls`, `destroyed`). `test-setup.ts` la usa al posto di `uplot` in ogni file di test;
  - le chiavi i18n `advanced.chart.window.label`, `advanced.chart.window.<60|300|1800|3600>`, `advanced.chart.series`, `advanced.chart.maxSeries`, `advanced.chart.empty`.

**Comportamento (spec §7.3, §7.5; decisioni D3, D4, D10):**
- **Serie mostrate.**
  - All'apertura il grafico usa le serie salvate per la sezione (`oma.advanced.series.<id>`), senza gli id che non esistono più e senza i sensori che superano i limiti.
  - Se della lista salvata non resta nulla, oppure non c'è una lista salvata, valgono le serie predefinite passate dalla pagina (Task 13).
  - Una lista salvata vuota resta vuota, perché è una scelta dell'utente: il grafico mostra `advanced.chart.empty` e non chiede nulla al backend.
- **Limiti (D4).** Al massimo 8 serie e 2 unità di misura. Nel selettore le caselle che violerebbero un limite sono disabilitate e `advanced.chart.maxSeries` spiega perché. Ogni scelta si salva subito.
- **Finestra.** 1m / 5m / 30m / 1h, salvata in `oma.advanced.window`. Finché l'utente non ne sceglie una vale 5m.
- **Storico (D3).**
  - A ogni cambio di serie, di finestra o di revisione dello schema il grafico chiede `getHistory(ids, secondi, maxPoints)`.
  - `maxPoints` vale 900 per le finestre da 30 minuti in su; sotto i 30 minuti l'argomento manca e lo storico arriva grezzo.
  - Una risposta superata da una richiesta più recente si scarta.
- **Coda dal vivo.**
  - Ogni snapshot applicato allo `LiveStore` aggiunge un punto per serie. Lo store scarta le sequenze duplicate o fuori ordine. Un timestamp uguale all'ultimo si ignora; un timestamp inferiore su una nuova sequenza indica un arretramento dell'orologio e avvia un segmento nuovo, come lo storico del nucleo.
  - Dopo ogni punto si tolgono quelli più vecchi della finestra, misurata all'indietro dall'ultimo timestamp.
  - Uno snapshot arrivato mentre lo storico era in viaggio si aggiunge in coda al seme, così non si perde.
  - Da 30 minuti il seme è sottocampionato e la coda è a 1 Hz. Il limite di 900 righe vale per la risposta iniziale, non per il buffer vivo: dopo un'ora aperta la coda può contenere circa 3600 punti. Lo spike a 148 MB è un riferimento precedente; il Task 14 deve misurare anche la finestra mantenuta aperta per 61 minuti.
- **Pausa (spec §7.5).** Con `document.visibilityState === 'hidden'` il grafico non si aggiorna e non chiede nulla: invalida anche le risposte in viaggio e sospende i ridimensionamenti. Al ritorno a `visible` ricarica lo storico, che copre il buco.
- **Aspetto.**
  - I colori delle serie si leggono con `getComputedStyle(document.documentElement)` quando si costruisce il grafico: `--accent`, `--accent-2`, `--ok`, `--warn`, poi `--series-5` … `--series-8`. Assi e griglia usano `--text-muted` e `--border`.
  - La prima unità va sull'asse sinistro, la seconda sul destro, senza griglia propria.
  - Le percentuali mostrano almeno 0–100 e i flag 0–1, ma la scala si allarga se i dati escono da quei limiti: la potenza GPU in % del limite può superare 100.
  - I valori di assi e legenda passano da `formatValue`; nella legenda l'istante è nell'ora locale.
  - Niente zoom col trascinamento: lo zoom è la scelta della finestra.
  - La larghezza segue il contenitore (`ResizeObserver`); l'altezza è di 260 px.
- **Buffer.** Array normali e non `Float64Array` (spec §7.5): uPlot vuole `null` per i buchi e un array tipizzato non può contenerlo. `data()` restituisce copie, così uPlot non vede i punti aggiunti dopo.
- **Test.** jsdom non ha il canvas, quindi `test-setup.ts` sostituisce `uplot` con `FakeUplot` in tutti i file di test. Così anche i test dei Task 10 e 13 che montano la pagina completa non disegnano nulla, e i test del grafico controllano opzioni e dati passati a uPlot.

- [ ] **Step 1: Scrivi i test (falliscono)**

`app/src/test/uplot-stub.ts` (nuovo). Registra ogni grafico creato, così i test leggono opzioni e dati:

```ts
import type uPlot from 'uplot';

/**
 * Stand-in for uPlot in jsdom, which has no canvas: test-setup.ts mocks 'uplot' with
 * this class, and tests read `instances` to check what a component handed to uPlot.
 */
export class FakeUplot {
  static instances: FakeUplot[] = [];
  opts: uPlot.Options;
  data: uPlot.AlignedData;
  target: HTMLElement | undefined;
  destroyed = false;
  setDataCalls = 0;
  sizes: { width: number; height: number }[] = [];

  constructor(opts: uPlot.Options, data: uPlot.AlignedData, target?: HTMLElement) {
    this.opts = opts;
    this.data = data;
    this.target = target;
    FakeUplot.instances.push(this);
  }

  setData(data: uPlot.AlignedData): void {
    this.data = data;
    this.setDataCalls++;
  }

  setSize(size: { width: number; height: number }): void {
    this.sizes.push(size);
  }

  destroy(): void {
    this.destroyed = true;
  }
}
```

`app/src/test-setup.ts`: aggiungi in fondo al file. Il mock in un file di setup vale per ogni file di test:

```ts

// jsdom has no canvas, so uPlot cannot draw: every test file gets the recording stub.
vi.mock('uplot', async () => ({ default: (await import('./test/uplot-stub')).FakeUplot }));
```

`app/src/lib/advanced/chartData.test.ts` (nuovo). Il test del tema legge `theme.css` dal disco, perché Vitest non elabora il CSS; `pnpm test` gira in `app/`:

```ts
import { readFileSync } from 'node:fs';
import { MOCK_SCHEMA } from '../backend/mock';
import {
  ChartBuffer,
  MAX_SERIES,
  PALETTE_TOKENS,
  WINDOWS,
  canAdd,
  fitSelection,
  initialSeries,
  maxPointsFor,
  scaleLayout,
  scaleOptions,
  seriesPalette,
  unitsOf,
} from './chartData';
import { STORED_WINDOWS } from './persist';

const GPU = 'gpu/pci-0000:01:00.0';
const LOAD = `${GPU}/load/core`;
const TEMP = `${GPU}/temperature/core`;
const HOTSPOT = `${GPU}/temperature/hotspot`;
const VRAM = `${GPU}/data/memory-dedicated-used`;
const CPU_THREADS = Array.from({ length: 8 }, (_, i) => `cpu/0/load/thread-0-${i}`);
const ALL = MOCK_SCHEMA.sensors.map((s) => s.id);

const seed = (timestampsMs: number[], series: (number | null)[][]) => ({ revision: 1, seq: 7, timestampsMs, series });

test('the windows are the persisted ones and decimation starts at 30 minutes', () => {
  expect([...WINDOWS]).toEqual([...STORED_WINDOWS]);
  expect(maxPointsFor(60)).toBeUndefined();
  expect(maxPointsFor(300)).toBeUndefined();
  expect(maxPointsFor(1800)).toBe(900);
  expect(maxPointsFor(3600)).toBe(900);
});

test('seed keeps the history in id order and converts to seconds for uPlot', () => {
  const buffer = new ChartBuffer(['a', 'b'], 60);
  buffer.seed(seed([1000, 2000], [[1, 2], [null, 4]]));
  expect(buffer.data()).toEqual([[1, 2], [1, 2], [null, 4]]);
  expect(buffer.lastTimestampMs).toBe(2000);
});

test('seed pads short or missing columns with gaps', () => {
  const buffer = new ChartBuffer(['a', 'b'], 60);
  buffer.seed(seed([1000, 2000], [[1]]));
  expect(buffer.data()).toEqual([[1, 2], [1, null], [null, null]]);
});

test('append ignores samples that are not newer than the last one', () => {
  const buffer = new ChartBuffer(['a'], 60);
  buffer.seed(seed([1000, 2000], [[1, 2]]));
  buffer.append(2000, [99]);
  buffer.append(1500, [98]);
  buffer.append(3000, [3]);
  expect(buffer.data()).toEqual([[1, 2, 3], [1, 2, 3]]);
});

test('append turns missing and non-finite values into gaps', () => {
  const buffer = new ChartBuffer(['a', 'b'], 60);
  buffer.append(1000, [Number.NaN, Number.POSITIVE_INFINITY]);
  buffer.append(2000, [5]);
  expect(buffer.data()).toEqual([[1, 2], [null, 5], [null, null]]);
});

test('trim keeps exactly the window measured back from now', () => {
  const buffer = new ChartBuffer(['a'], 60);
  for (let t = 0; t <= 120; t += 10) buffer.append(t * 1000, [t]);
  buffer.trim(120_000);
  expect(buffer.data()[1]).toEqual([60, 70, 80, 90, 100, 110, 120]);
  buffer.trim(120_000);
  expect(buffer.length).toBe(7);
});

test('data returns copies, so uPlot never sees later appends', () => {
  const buffer = new ChartBuffer(['a'], 60);
  buffer.append(1000, [1]);
  const before = buffer.data();
  buffer.append(2000, [2]);
  expect(before).toEqual([[1], [1]]);
});

test('unitsOf lists distinct units in order and skips unknown ids', () => {
  expect(unitsOf([LOAD, TEMP, 'nope', HOTSPOT], MOCK_SCHEMA)).toEqual(['percent', 'celsius']);
  expect(unitsOf([], MOCK_SCHEMA)).toEqual([]);
});

test('canAdd allows a second unit but not a third', () => {
  expect(canAdd([LOAD], TEMP, MOCK_SCHEMA)).toBe(true);
  expect(canAdd([LOAD, TEMP], HOTSPOT, MOCK_SCHEMA)).toBe(true);
  expect(canAdd([LOAD, TEMP], VRAM, MOCK_SCHEMA)).toBe(false);
});

test('canAdd refuses the ninth series, duplicates and unknown ids', () => {
  const eight = ['cpu/0/load/total', ...CPU_THREADS.slice(0, 7)];
  expect(eight).toHaveLength(MAX_SERIES);
  expect(canAdd(eight.slice(0, 7), eight[7], MOCK_SCHEMA)).toBe(true);
  expect(canAdd(eight, CPU_THREADS[7], MOCK_SCHEMA)).toBe(false);
  expect(canAdd([LOAD], LOAD, MOCK_SCHEMA)).toBe(false);
  expect(canAdd([], 'nope', MOCK_SCHEMA)).toBe(false);
});

test('fitSelection drops unknown ids and whatever breaks the limits', () => {
  expect(fitSelection([LOAD, 'gone', TEMP, VRAM], ALL, MOCK_SCHEMA)).toEqual([LOAD, TEMP]);
  const ten = ['cpu/0/load/total', ...CPU_THREADS, 'cpu/0/clock/effective'];
  expect(fitSelection(ten, ALL, MOCK_SCHEMA)).toHaveLength(MAX_SERIES);
  expect(fitSelection([LOAD], ['other'], MOCK_SCHEMA)).toEqual([]);
});

test('initial series: saved ones cleaned, else the defaults; an empty choice stays empty', () => {
  const gpu = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU).map((s) => s.id);
  expect(initialSeries(null, gpu, [LOAD, TEMP], MOCK_SCHEMA)).toEqual([LOAD, TEMP]);
  expect(initialSeries([VRAM, 'gone'], gpu, [LOAD, TEMP], MOCK_SCHEMA)).toEqual([VRAM]);
  expect(initialSeries(['gone'], gpu, [LOAD, TEMP], MOCK_SCHEMA)).toEqual([LOAD, TEMP]);
  expect(initialSeries([], gpu, [LOAD, TEMP], MOCK_SCHEMA)).toEqual([]);
  expect(initialSeries(null, gpu, [LOAD, 'cpu/0/load/total'], MOCK_SCHEMA)).toEqual([LOAD]);
});

test('scale layout puts the first unit left and the second right', () => {
  expect(scaleLayout([TEMP, LOAD, HOTSPOT], MOCK_SCHEMA)).toEqual({
    scales: ['celsius', 'percent'],
    seriesScale: ['celsius', 'percent', 'celsius'],
  });
});

test('percent and flag scales include their natural bounds, others auto-range', () => {
  expect(scaleOptions('percent')).toEqual({ range: { min: { soft: 0, mode: 1, pad: 0 }, max: { soft: 100, mode: 1, pad: 0 } } });
  expect(scaleOptions('boolean')).toEqual({ range: { min: { soft: 0, mode: 1, pad: 0 }, max: { soft: 1, mode: 1, pad: 0 } } });
  expect(scaleOptions('celsius')).toEqual({});
});

test('palette reads the eight tokens in order', () => {
  expect(seriesPalette((token) => ` ${token}-value `)).toEqual(PALETTE_TOKENS.map((t) => `${t}-value`));
  expect(PALETTE_TOKENS).toHaveLength(MAX_SERIES);
});

test('theme.css defines every palette token', () => {
  const theme = readFileSync('src/styles/theme.css', 'utf8'); // vitest runs from app/
  for (const token of PALETTE_TOKENS) expect(theme).toMatch(new RegExp(`${token}\\s*:`));
});
```

`app/src/components/advanced/HistoryChart.test.ts` (nuovo). `document.visibilityState` viene ridefinito con un getter per simulare la finestra nascosta:

```ts
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { WINDOW_KEY, seriesKey } from '../../lib/advanced/persist';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { HistorySeed, Sensor } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { FakeUplot } from '../../test/uplot-stub';
import HistoryChart from './HistoryChart.svelte';

const plots = FakeUplot.instances;
const GPU = 'gpu/pci-0000:01:00.0';
const LOAD = `${GPU}/load/core`;
const TEMP = `${GPU}/temperature/core`;
const HOTSPOT = `${GPU}/temperature/hotspot`;
const VRAM = `${GPU}/data/memory-dedicated-used`;
const gpuSensors = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU);
const cpuSensors = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === 'cpu/0');
const byId = (id: string) => MOCK_SCHEMA.sensors.find((s) => s.id === id)!;
const index = (id: string) => MOCK_SCHEMA.sensors.findIndex((s) => s.id === id);
const labelOf = (s: Sensor) => t(`sensor.${s.label.key}`, s.label.arg === undefined ? {} : { arg: s.label.arg });
const checkbox = (label: string) => screen.getByLabelText(label) as HTMLInputElement;

let visibility: DocumentVisibilityState = 'visible';
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
const setVisibility = (state: DocumentVisibilityState) => {
  visibility = state;
  document.dispatchEvent(new Event('visibilitychange'));
};

beforeEach(() => {
  plots.length = 0;
  localStorage.clear();
  i18n.locale = 'en';
  visibility = 'visible';
});
afterEach(cleanup);

/** FakeBackend with two history samples (1 s and 2 s): column i holds [10 + i, 20 + i]. */
function fakeBackend(): FakeBackend {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.history = { timestampsMs: [1000, 2000], series: Array.from({ length: 8 }, (_, i) => [10 + i, 20 + i]) };
  return backend;
}

function renderChart(backend: FakeBackend, store = new LiveStore(), sensors = gpuSensors, defaults = [LOAD, TEMP], sectionId = GPU) {
  if (!store.schema) store.applySchema(MOCK_SCHEMA);
  return render(HistoryChart, { sectionId, sensors, defaults, schema: MOCK_SCHEMA, store, backend });
}

test('seeds the default series and draws them on two unit scales', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  expect(backend.historyCalls).toEqual([{ ids: [LOAD, TEMP], seconds: 300, maxPoints: undefined }]);
  const [plot] = plots;
  expect(plot.opts.series.slice(1).map((s) => s.label)).toEqual([labelOf(byId(LOAD)), labelOf(byId(TEMP))]);
  expect(plot.opts.series.slice(1).map((s) => s.scale)).toEqual(['percent', 'celsius']);
  expect(plot.opts.axes?.map((a) => [a.scale, a.side])).toEqual([[undefined, undefined], ['percent', 3], ['celsius', 1]]);
  expect(plot.data).toEqual([[1, 2], [10, 20], [11, 21]]);
  expect(screen.getByRole('button', { name: t('advanced.chart.window.300') }).getAttribute('aria-pressed')).toBe('true');
});

test('long windows ask for decimated history and the choice persists', async () => {
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.3600') }));
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(backend.historyCalls.at(-1)).toEqual({ ids: [LOAD, TEMP], seconds: 3600, maxPoints: 900 });
  expect(localStorage.getItem(WINDOW_KEY)).toBe('3600');
  expect(plots[0].destroyed).toBe(true);

  await fireEvent.click(screen.getByRole('button', { name: t('advanced.chart.window.60') }));
  await vi.waitFor(() => expect(plots).toHaveLength(3));
  expect(backend.historyCalls.at(-1)).toEqual({ ids: [LOAD, TEMP], seconds: 60, maxPoints: undefined });
});

test('the saved window is used on mount', async () => {
  localStorage.setItem(WINDOW_KEY, '1800');
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(backend.historyCalls).toEqual([{ ids: [LOAD, TEMP], seconds: 1800, maxPoints: 900 }]);
});

test('live snapshots extend the chart and old points leave the window', async () => {
  localStorage.setItem(WINDOW_KEY, '60');
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 62_000, values: mockValues(1) });
  flushSync();
  const plot = plots[0];
  expect(plot.setDataCalls).toBe(1);
  // 1000 ms is older than 62 s - 60 s and leaves; 2000 ms is exactly on the edge and stays.
  expect(plot.data).toEqual([
    [2, 62],
    [20, mockValues(1)[index(LOAD)]],
    [21, mockValues(1)[index(TEMP)]],
  ]);
});

test('a snapshot that arrives while history loads is not lost', async () => {
  const backend = fakeBackend();
  let resolve!: (h: HistorySeed) => void;
  backend.getHistory = () => new Promise((done) => (resolve = done));
  const store = new LiveStore();
  renderChart(backend, store);
  await vi.waitFor(() => expect(resolve).toBeDefined());

  store.applySnapshot({ revision: 1, seq: 5, timestampMs: 3000, values: mockValues(5) });
  flushSync();
  resolve({ revision: 1, seq: 4, timestampsMs: [1000, 2000], series: [[1, 2], [3, 4]] });
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(plots[0].data[0]).toEqual([1, 2, 3]);
});

test('the picker allows at most 8 series and saves the choice per section', async () => {
  const backend = fakeBackend();
  renderChart(backend, new LiveStore(), cpuSensors, ['cpu/0/load/total', 'cpu/0/clock/effective'], 'cpu/0');
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  const thread = (i: number) => checkbox(t('sensor.cpu.load.thread', { arg: i }));
  for (let i = 0; i < 6; i++) await fireEvent.click(thread(i));
  expect(thread(6).disabled).toBe(true);
  expect(screen.getByText(`${t('advanced.chart.series')} · 8/8`)).toBeTruthy();
  await vi.waitFor(() => expect(backend.historyCalls.at(-1)?.ids).toHaveLength(8));
  expect(JSON.parse(localStorage.getItem(seriesKey('cpu/0'))!)).toHaveLength(8);

  await fireEvent.click(thread(0));
  expect(thread(6).disabled).toBe(false);
});

test('the picker refuses a third unit', async () => {
  renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  expect(checkbox(labelOf(byId(VRAM))).disabled).toBe(true);
  expect(checkbox(labelOf(byId(HOTSPOT))).disabled).toBe(false);
  await fireEvent.click(checkbox(labelOf(byId(TEMP))));
  expect(checkbox(labelOf(byId(VRAM))).disabled).toBe(false);
});

test('the saved selection of the section wins over the defaults', async () => {
  localStorage.setItem(seriesKey(GPU), JSON.stringify([VRAM, 'gone']));
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(backend.historyCalls[0].ids).toEqual([VRAM]);
});

test('an empty selection shows a hint and fetches nothing', async () => {
  localStorage.setItem(seriesKey(GPU), '[]');
  const backend = fakeBackend();
  renderChart(backend);
  await vi.waitFor(() => expect(screen.getByText(t('advanced.chart.empty'))).toBeTruthy());
  expect(backend.historyCalls).toEqual([]);
  expect(plots).toHaveLength(0);
});

test('rendering pauses while hidden and history is reloaded when visible', async () => {
  const backend = fakeBackend();
  const store = new LiveStore();
  renderChart(backend, store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));

  setVisibility('hidden');
  flushSync();
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 3000, values: mockValues(1) });
  flushSync();
  expect(plots[0].setDataCalls).toBe(0);
  expect(backend.historyCalls).toHaveLength(1);

  setVisibility('visible');
  await vi.waitFor(() => expect(plots).toHaveLength(2));
  expect(backend.historyCalls).toHaveLength(2);
});

test('series colours come from the theme tokens', async () => {
  document.documentElement.style.setProperty('--accent', '#ff4fd8');
  document.documentElement.style.setProperty('--accent-2', '#4cc9f0');
  try {
    renderChart(fakeBackend());
    await vi.waitFor(() => expect(plots).toHaveLength(1));
    expect(plots[0].opts.series.slice(1).map((s) => s.stroke)).toEqual(['#ff4fd8', '#4cc9f0']);
  } finally {
    document.documentElement.removeAttribute('style');
  }
});

test('a history reply arriving while hidden cannot create a plot', async () => {
  const backend = fakeBackend();
  let resolve!: (h: HistorySeed) => void;
  backend.getHistory = () => new Promise((done) => (resolve = done));
  renderChart(backend);
  await vi.waitFor(() => expect(resolve).toBeDefined());
  setVisibility('hidden');
  flushSync();
  resolve({ revision: 1, seq: 0, timestampsMs: [1000], series: [[1], [2]] });
  await new Promise((done) => setTimeout(done, 0));
  expect(plots).toHaveLength(0);
});

test('history from another schema revision is never plotted', async () => {
  const backend = fakeBackend();
  backend.getHistory = async () => ({ revision: 2, seq: 0, timestampsMs: [1000], series: [[1], [2]] });
  renderChart(backend);
  await new Promise((done) => setTimeout(done, 0));
  expect(plots).toHaveLength(0);
});

test('a newer snapshot after a clock rollback starts a new chart segment', async () => {
  const store = new LiveStore();
  renderChart(fakeBackend(), store);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 10_000, values: mockValues(1) });
  flushSync();
  store.applySnapshot({ revision: 1, seq: 2, timestampMs: 5000, values: mockValues(2) });
  flushSync();
  expect(plots[0].data[0]).toEqual([5]);
  expect(plots[0].data[1]).toEqual([mockValues(2)[index(LOAD)]]);
});

test('unmounting destroys the plot', async () => {
  const { unmount } = renderChart(fakeBackend());
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  unmount();
  expect(plots[0].destroyed).toBe(true);
});
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

```bash
cd app && pnpm test
```

Risultato atteso: `Test Files  2 failed | 15 passed (17)`, `Tests  101 passed (101)`. I conteggi partono dai 101 test frontend dopo il Task 10. I due file nuovi falliscono all'import:
- `Failed to resolve import "./chartData" from "src/lib/advanced/chartData.test.ts"`;
- `Failed to resolve import "./HistoryChart.svelte" from "src/components/advanced/HistoryChart.test.ts"`.

- [ ] **Step 3: Implementa `chartData.ts` e `labels.ts`**

`app/src/lib/advanced/chartData.ts` (nuovo):

```ts
import type uPlot from 'uplot';
import type { HistorySeed, Schema, Unit } from '../types';

/** Chart windows in seconds: 1m, 5m, 30m, 1h (spec §7.3). Same values as persist.ts `StoredWindow`. */
export const WINDOWS = [60, 300, 1800, 3600] as const;
export type WindowSeconds = (typeof WINDOWS)[number];
/** Window used until the user picks one. */
export const DEFAULT_WINDOW: WindowSeconds = 300;
/** At most 8 series and 2 units per chart (decision D4). */
export const MAX_SERIES = 8;
export const MAX_UNITS = 2;
/** Windows of at least this many seconds are requested decimated (decision D3). */
export const DECIMATE_FROM = 1800;
export const MAX_POINTS = 900;

/** `maxPoints` argument of `Backend.getHistory`: raw below 30 min, decimated from 30 min. */
export function maxPointsFor(windowSeconds: number): number | undefined {
  return windowSeconds >= DECIMATE_FROM ? MAX_POINTS : undefined;
}

const clean = (v: number | null | undefined): number | null =>
  v === null || v === undefined || !Number.isFinite(v) ? null : v;

/**
 * Time-aligned chart data for a fixed list of sensor ids: seeded from the core history,
 * extended by live snapshots, trimmed to the window. uPlot needs `null` for gaps, which a
 * Float64Array cannot hold, so the columns are plain arrays.
 */
export class ChartBuffer {
  readonly ids: string[];
  readonly windowSeconds: number;
  #timestampsMs: number[] = [];
  #series: (number | null)[][];

  constructor(ids: string[], windowSeconds: number) {
    this.ids = [...ids];
    this.windowSeconds = windowSeconds;
    this.#series = this.ids.map(() => []);
  }

  get length(): number {
    return this.#timestampsMs.length;
  }

  get lastTimestampMs(): number | null {
    return this.#timestampsMs.at(-1) ?? null;
  }

  /** Replaces the content with a history window whose series follow `ids` order. */
  seed(h: HistorySeed): void {
    this.#timestampsMs = [...h.timestampsMs];
    this.#series = this.ids.map((_, i) => {
      const column = h.series[i] ?? [];
      return this.#timestampsMs.map((_, k) => clean(column[k]));
    });
  }

  /** Adds one sample (values in `ids` order); a sample not newer than the last one is ignored. */
  append(timestampMs: number, values: (number | null)[]): void {
    const last = this.lastTimestampMs;
    if (last !== null && timestampMs <= last) return;
    this.#timestampsMs.push(timestampMs);
    this.#series.forEach((column, i) => column.push(clean(values[i])));
  }

  /** Drops the samples older than the window, measured back from `nowMs`. */
  trim(nowMs: number): void {
    const since = nowMs - this.windowSeconds * 1000;
    let drop = 0;
    while (drop < this.#timestampsMs.length && this.#timestampsMs[drop] < since) drop++;
    if (drop === 0) return;
    this.#timestampsMs.splice(0, drop);
    for (const column of this.#series) column.splice(0, drop);
  }

  /** uPlot layout, copied: x in seconds, then one column per id. */
  data(): uPlot.AlignedData {
    return [this.#timestampsMs.map((ms) => ms / 1000), ...this.#series.map((column) => [...column])];
  }
}

/** Distinct units of `ids`, in order of first appearance; unknown ids are skipped. */
export function unitsOf(ids: string[], schema: Schema): Unit[] {
  const units: Unit[] = [];
  for (const id of ids) {
    const unit = schema.sensors.find((s) => s.id === id)?.unit;
    if (unit && !units.includes(unit)) units.push(unit);
  }
  return units;
}

/** True when `candidate` fits: fewer than 8 series and at most a second unit. */
export function canAdd(selected: string[], candidate: string, schema: Schema): boolean {
  if (selected.includes(candidate) || selected.length >= MAX_SERIES) return false;
  const sensor = schema.sensors.find((s) => s.id === candidate);
  if (!sensor) return false;
  const units = unitsOf(selected, schema);
  return units.includes(sensor.unit) || units.length < MAX_UNITS;
}

/** The ids present in `candidates`, in order, as long as they respect the series and unit limits. */
export function fitSelection(ids: string[], candidates: string[], schema: Schema): string[] {
  const out: string[] = [];
  for (const id of ids) {
    if (candidates.includes(id) && canAdd(out, id, schema)) out.push(id);
  }
  return out;
}

/**
 * Series to chart on mount. `saved` is persist.ts `loadSeries` (null = never saved): it is
 * cleaned against the current sensors, and the defaults apply when nothing of it survives.
 * An explicitly saved empty list stays empty.
 */
export function initialSeries(saved: string[] | null, candidates: string[], defaults: string[], schema: Schema): string[] {
  const fallback = fitSelection(defaults, candidates, schema);
  if (saved === null) return fallback;
  if (saved.length === 0) return [];
  const kept = fitSelection(saved, candidates, schema);
  return kept.length > 0 ? kept : fallback;
}

/** Series colours: accent tokens first, then the four chart-only tokens of theme.css. */
export const PALETTE_TOKENS = ['--accent', '--accent-2', '--ok', '--warn', '--series-5', '--series-6', '--series-7', '--series-8'] as const;

/** Resolves the palette through `read`, e.g. `getComputedStyle(root).getPropertyValue`. */
export function seriesPalette(read: (token: string) => string): string[] {
  return PALETTE_TOKENS.map((token) => read(token).trim());
}

/** Scale key per series (its unit): the first unit is drawn on the left axis, the second on the right. */
export function scaleLayout(ids: string[], schema: Schema): { scales: Unit[]; seriesScale: Unit[] } {
  const scales = unitsOf(ids, schema);
  const seriesScale = ids.map((id) => schema.sensors.find((s) => s.id === id)?.unit ?? scales[0]);
  return { scales, seriesScale };
}

/**
 * Scale options per unit: percent shows at least 0..100 and flags 0..1, but the range still
 * grows past the bound when the data does (the GPU power ratio can exceed 100 %).
 */
export function scaleOptions(unit: Unit): uPlot.Scale {
  const bounds = unit === 'percent' ? [0, 100] : unit === 'boolean' ? [0, 1] : null;
  if (!bounds) return {};
  return { range: { min: { soft: bounds[0], mode: 1, pad: 0 }, max: { soft: bounds[1], mode: 1, pad: 0 } } };
}
```

`app/src/lib/advanced/labels.ts` (nuovo):

```ts
import type { Translate } from '../i18n/index.svelte';
import type { Sensor } from '../types';

/** Translated sensor name: `sensor.<label.key>` with the optional `{arg}`. */
export function sensorLabel(sensor: Sensor, t: Translate): string {
  return t(`sensor.${sensor.label.key}`, sensor.label.arg === undefined ? {} : { arg: sensor.label.arg });
}
```

- [ ] **Step 4: Implementa `HistoryChart.svelte` e le traduzioni**

`app/src/components/advanced/HistoryChart.svelte` (nuovo). Punti da rispettare:
- `sectionId` e la selezione iniziale si leggono una volta sola (`untrack`): il Task 10 ricrea la pagina a ogni cambio di sezione con `{#key}`;
- i due `$effect` leggono le dipendenze e poi lavorano dentro `untrack`, così scrivere `buffer` e `plot` non li fa ripartire;
- `generation` scarta le risposte superate e quelle che arrivano dopo lo smontaggio.

```svelte
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import uPlot from 'uplot';
  import 'uplot/dist/uPlot.min.css';
  import {
    ChartBuffer,
    DEFAULT_WINDOW,
    MAX_SERIES,
    PALETTE_TOKENS,
    WINDOWS,
    canAdd,
    fitSelection,
    initialSeries,
    maxPointsFor,
    scaleLayout,
    scaleOptions,
    seriesPalette,
    type WindowSeconds,
  } from '../../lib/advanced/chartData';
  import { sensorLabel } from '../../lib/advanced/labels';
  import { loadSeries, loadWindow, saveSeries, saveWindow } from '../../lib/advanced/persist';
  import type { Backend } from '../../lib/backend/backend';
  import { DASH, formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { LiveStore } from '../../lib/live.svelte';
  import type { HistorySeed, Schema, Sensor } from '../../lib/types';

  let {
    sectionId,
    sensors,
    defaults,
    schema,
    store,
    backend,
  }: {
    /** Section id; parents re-key the component when it changes. */
    sectionId: string;
    /** Sensors of the page: the series the picker offers. */
    sensors: Sensor[];
    defaults: string[];
    schema: Schema;
    store: LiveStore;
    backend: Backend;
  } = $props();

  const HEIGHT = 260;

  const candidateIds = $derived(sensors.map((s) => s.id));
  let windowSeconds = $state<WindowSeconds>(loadWindow() ?? DEFAULT_WINDOW);
  let chosen = $state<string[]>(untrack(() => initialSeries(loadSeries(sectionId), candidateIds, defaults, schema)));
  const selected = $derived(fitSelection(chosen, candidateIds, schema));
  const selectionKey = $derived(selected.join('\n'));
  let paused = $state(document.visibilityState === 'hidden');

  let container: HTMLDivElement;
  let plot: uPlot | undefined;
  let buffer: ChartBuffer | undefined;
  let generation = 0;
  let destroyed = false;

  function chooseWindow(w: WindowSeconds) {
    windowSeconds = w;
    saveWindow(w);
  }

  function toggle(id: string) {
    if (selected.includes(id)) chosen = selected.filter((x) => x !== id);
    else if (canAdd(selected, id, schema)) chosen = [...selected, id];
    else return;
    saveSeries(sectionId, chosen);
  }

  async function reseed(ids: string[], seconds: WindowSeconds) {
    const token = ++generation;
    const revision = schema.revision;
    // Never append values from a new schema to a plot of the previous source/unit.
    buffer = undefined;
    plot?.destroy();
    plot = undefined;
    let history: HistorySeed = { revision, seq: 0, timestampsMs: [], series: [] };
    if (ids.length > 0) {
      try {
        history = await backend.getHistory(ids, seconds, maxPointsFor(seconds));
      } catch (error) {
        console.error('chart history unavailable', error);
      }
    }
    if (token !== generation || destroyed || paused || schema.revision !== revision) return;
    if (ids.length > 0 && history.revision !== revision) return;
    const next = new ChartBuffer(ids, seconds);
    next.seed(history);
    // A snapshot applied while the request was in flight is newer than the seed.
    if (store.timestampMs > (next.lastTimestampMs ?? 0)) next.append(store.timestampMs, ids.map((id) => store.value(id)));
    next.trim(next.lastTimestampMs ?? 0);
    buffer = next;
    build(ids);
  }

  function build(ids: string[]) {
    plot?.destroy();
    plot = undefined;
    if (!buffer || ids.length === 0) return;
    const css = getComputedStyle(document.documentElement);
    const read = (token: string) => css.getPropertyValue(token).trim();
    const palette = seriesPalette(read);
    const muted = read('--text-muted');
    const border = read('--border');
    const { scales, seriesScale } = scaleLayout(ids, schema);
    const byId = new Map(sensors.map((s) => [s.id, s]));
    const axis = (unit: (typeof scales)[number], side: 1 | 3): uPlot.Axis => ({
      scale: unit,
      side,
      size: 72,
      stroke: muted,
      grid: { show: side === 3, stroke: border, width: 1 },
      ticks: { stroke: border, width: 1 },
      values: (_u, splits) => splits.map((v) => formatValue(v, unit, i18n.locale, t)),
    });
    const opts: uPlot.Options = {
      width: Math.max(320, container.clientWidth || 800),
      height: HEIGHT,
      cursor: { drag: { x: false, y: false, setScale: false } },
      scales: Object.fromEntries([['x', { time: true }], ...scales.map((unit) => [unit, scaleOptions(unit)])]),
      series: [
        { label: '', value: (_u, v) => (v == null ? DASH : new Date(v * 1000).toLocaleTimeString(i18n.locale)) },
        ...ids.map((id, i) => {
          const sensor = byId.get(id);
          const unit = seriesScale[i];
          return {
            label: sensor ? sensorLabel(sensor, t) : id,
            scale: unit,
            stroke: palette[i],
            width: 1.5,
            points: { show: false },
            value: (_u: uPlot, v: number | null) => formatValue(v ?? null, unit, i18n.locale, t),
          };
        }),
      ],
      axes: [
        { stroke: muted, grid: { stroke: border, width: 1 }, ticks: { stroke: border, width: 1 } },
        axis(scales[0], 3),
        ...(scales[1] ? [axis(scales[1], 1)] : []),
      ],
    };
    plot = new uPlot(opts, buffer.data(), container);
  }

  function tail(timestampMs: number) {
    if (paused || !buffer || !plot || timestampMs <= 0) return;
    // LiveStore has already rejected duplicate/out-of-order sequences. A lower
    // timestamp here is a wall-clock rollback, so begin a new chart segment.
    if (buffer.lastTimestampMs !== null && timestampMs < buffer.lastTimestampMs) {
      buffer = new ChartBuffer(buffer.ids, buffer.windowSeconds);
    }
    buffer.append(timestampMs, buffer.ids.map((id) => store.value(id)));
    buffer.trim(timestampMs);
    plot.setData(buffer.data());
  }

  // Reseed on selection, window or schema change, and when the window becomes visible again.
  $effect(() => {
    const ids = selectionKey ? selectionKey.split('\n') : [];
    const seconds = windowSeconds;
    void schema.revision;
    if (paused) return;
    untrack(() => void reseed(ids, seconds));
  });

  // Live tail: one point per snapshot applied to the store.
  $effect(() => {
    const timestampMs = store.timestampMs;
    untrack(() => tail(timestampMs));
  });

  onMount(() => {
    const onVisibility = () => {
      paused = document.visibilityState === 'hidden';
      if (paused) generation++; // Invalidate history that is still in flight.
    };
    document.addEventListener('visibilitychange', onVisibility);
    const observer =
      typeof ResizeObserver === 'undefined'
        ? undefined
        : new ResizeObserver(() => {
            if (!paused) plot?.setSize({ width: Math.max(320, container.clientWidth), height: HEIGHT });
          });
    observer?.observe(container);
    return () => {
      destroyed = true;
      generation++;
      document.removeEventListener('visibilitychange', onVisibility);
      observer?.disconnect();
      plot?.destroy();
      plot = undefined;
    };
  });
</script>

<section class="chart">
  <div class="controls">
    <div class="windows" role="group" aria-label={t('advanced.chart.window.label')}>
      {#each WINDOWS as w (w)}
        <button type="button" aria-pressed={windowSeconds === w} class:on={windowSeconds === w} onclick={() => chooseWindow(w)}>
          {t(`advanced.chart.window.${w}`)}
        </button>
      {/each}
    </div>
    <details class="picker">
      <summary>{t('advanced.chart.series')} · {selected.length}/{MAX_SERIES}</summary>
      <p class="hint">{t('advanced.chart.maxSeries')}</p>
      <ul>
        {#each sensors as sensor (sensor.id)}
          {@const index = selected.indexOf(sensor.id)}
          <li>
            <label>
              <input
                type="checkbox"
                checked={index >= 0}
                disabled={index < 0 && !canAdd(selected, sensor.id, schema)}
                onchange={() => toggle(sensor.id)}
              />
              <i class="swatch" style:background={index >= 0 ? `var(${PALETTE_TOKENS[index]})` : 'transparent'}></i>
              {sensorLabel(sensor, t)}
            </label>
          </li>
        {/each}
      </ul>
    </details>
  </div>
  <div class="plot" bind:this={container}></div>
  {#if selected.length === 0}
    <p class="empty">{t('advanced.chart.empty')}</p>
  {/if}
</section>

<style>
  .chart {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }
  .windows {
    display: flex;
    padding: 3px;
    border-radius: 10px;
    background: var(--surface-2);
  }
  .windows button {
    padding: 4px 12px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .windows button.on {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
  }
  .picker {
    min-width: 220px;
    font-size: 13px;
  }
  .picker summary {
    cursor: pointer;
    color: var(--text-muted);
    text-align: right;
  }
  .picker ul {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 2px 12px;
    margin: 6px 0 0;
    padding: 8px;
    list-style: none;
    background: var(--surface-2);
    border-radius: 8px;
  }
  .picker label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .picker label:has(input:disabled) {
    opacity: 0.45;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    border: 1px solid var(--border);
  }
  .hint {
    margin: 6px 0 0;
    color: var(--text-muted);
    font-size: 12px;
  }
  .plot {
    min-height: 260px;
  }
  .empty {
    position: absolute;
    inset: 50% 0 auto;
    margin: 0;
    text-align: center;
    color: var(--text-muted);
  }
  .plot :global(.u-legend) {
    color: var(--text);
    font-size: 12px;
  }
</style>
```

`app/src/lib/i18n/en.json`: inserisci subito dopo la riga `"advanced.section.psu": "Power supply",` (Task 10):

```json
  "advanced.chart.window.label": "Time window",
  "advanced.chart.window.60": "1m",
  "advanced.chart.window.300": "5m",
  "advanced.chart.window.1800": "30m",
  "advanced.chart.window.3600": "1h",
  "advanced.chart.series": "Series",
  "advanced.chart.maxSeries": "Up to 8 series with at most 2 units.",
  "advanced.chart.empty": "Choose at least one series.",
```

`app/src/lib/i18n/it.json`: inserisci subito dopo la riga `"advanced.section.psu": "Alimentatore",`:

```json
  "advanced.chart.window.label": "Finestra temporale",
  "advanced.chart.window.60": "1m",
  "advanced.chart.window.300": "5m",
  "advanced.chart.window.1800": "30m",
  "advanced.chart.window.3600": "1h",
  "advanced.chart.series": "Serie",
  "advanced.chart.maxSeries": "Al massimo 8 serie con al più 2 unità di misura.",
  "advanced.chart.empty": "Scegli almeno una serie.",
```

- [ ] **Step 5: Esegui i test e verifica che passino**

```bash
cd app
pnpm test
pnpm check
pnpm build
```

Risultato atteso:
- `pnpm test`: `Test Files  17 passed (17)`, `Tests  132 passed (132)` (16 in `chartData.test.ts`, 15 in `HistoryChart.test.ts`);
- `pnpm check`: `0 ERRORS 0 WARNINGS`;
- `pnpm build`: OK. `HistoryChart` non è ancora montato, quindi uPlot non entra ancora nel bundle.

Il test `both catalogs define the same keys` (`i18n.test.ts`) conferma che le chiavi nuove sono in entrambe le lingue.

- [ ] **Step 6: Verifica dal vivo**

In questo task non c'è: il grafico si monta nella pagina con il Task 13, che lo verifica nel browser con il backend mock e nell'app vera (Task 13, Step 6–8).

- [ ] **Step 7: Commit**

```bash
git add app/src
git commit -m "feat(ui): uPlot history chart with window selector, series picker and visibility pause"
```

---

### Task 12: UI: tabella dei sensori (min/max/media dal core, azzeramento, badge della fonte)

**File:**
- Crea:
  - `app/src/lib/advanced/pages.ts` (per ora: raggruppamento per categoria, `StatsOf` e i testi della tabella; il Task 13 aggiunge KPI, serie predefinite e proprietà)
  - `app/src/lib/advanced/statsPoller.svelte.ts` (lettura periodica di `getStats` e azzeramento)
  - `app/src/components/advanced/SensorTable.svelte`
- Modifica: `app/src/lib/i18n/en.json`, `app/src/lib/i18n/it.json` (chiavi `advanced.table.*`, `advanced.experimental`, `advanced.category.*`)
- Test:
  - Crea: `app/src/lib/advanced/pages.test.ts`, `app/src/lib/advanced/statsPoller.test.ts`, `app/src/components/advanced/SensorTable.test.ts`

**Interfacce:**
- Usa:
  - Task 9:
    - `Backend.getStats(ids): Promise<StatsReply>` (statistiche nell'ordine degli id, `null` per gli id sconosciuti o ancora senza campioni) e `Backend.resetStats(ids)`;
    - `SensorStats`, `formatValue(value, unit, locale, t, opts?: FormatOptions)` con `FormatOptions { rate?: 'bits' | 'bytes' }`, `formatPercent(value, locale: string)`;
    - le chiavi `source.<nome>` (11 fonti) e `flag.on`/`flag.off`;
    - `FakeBackend.stats` (per id; `resetStats` cancella le voci), `FakeBackend.statsCalls`, `FakeBackend.resetCalls`.
  - Task 3: `get_stats` e `reset_stats`, cioè le statistiche del core dall'avvio dell'app (decisione D1).
  - Task 11: `sensorLabel(sensor, t)` (`lib/advanced/labels.ts`).
  - M1: `ValueOf` (`lib/select.ts`), `catalogs`, `i18n`, `t`.
- Produce:
  - `lib/advanced/pages.ts`:
    ```ts
    export type StatsOf = (id: string) => SensorStats | null;
    export interface SensorGroup { category: string; sensors: Sensor[] }
    export const CATEGORY_ORDER = ['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'throughput', 'link', 'energy', 'flag'];
    export function groupSensors(sensors: Sensor[]): SensorGroup[];
    export function categoryLabel(category: string, t: Translate): string;
    export function sourceCode(source: Source): string;                 // "NVML", "IP HELPER"
    export function formatAverage(stats: SensorStats | null, unit: Unit, locale: string, t: Translate, opts?: FormatOptions): string;
    ```
  - `lib/advanced/statsPoller.svelte.ts`:
    ```ts
    export const STATS_INTERVAL_MS = 1000;
    export class StatsPoller {
      constructor(backend: Backend, ids: () => string[], revision: () => number | null, intervalMs?: number);
      byId: ReadonlyMap<string, SensorStats>;   // $state.raw
      readonly statsOf: StatsOf;
      start(): () => void;                      // legge subito, poi ogni intervallo; restituisce stop
      stop(): void;
      poll(): Promise<void>;
      reset(): Promise<void>;                   // resetStats(ids), poi una nuova lettura
    }
    ```
  - `SensorTable.svelte`, props `{ sensors: Sensor[]; valueOf: ValueOf; stats: StatsPoller; rate?: 'bits' | 'bytes' }` (`rate` predefinito `'bytes'`; il Task 13 passa `'bits'` sulle pagine di rete);
  - le chiavi i18n `advanced.table.sensor/current/min/max/avg/reset`, `advanced.experimental`, `advanced.category.<categoria>` per le 13 categorie.

  Il Task 13 crea un solo `StatsPoller` in `DevicePage`, sugli id di tutti i sensori della pagina, e lo passa alla tabella e ai KPI: il carico massimo della CPU e il picco di download leggono le stesse statistiche. Così vale il contratto "DevicePage legge `getStats(pageSensorIds)` ogni secondo".

  La callback `revision` legge la revisione dello schema visualizzato. `statsOf` nasconde immediatamente la cache di una revisione diversa; una risposta si pubblica solo se la revisione richiesta, quella restituita e quella ancora visualizzata coincidono. Un cambio di fonte o unità a parità di id non mostra quindi statistiche della fonte precedente.

**Comportamento (spec §4.2, §7.3; decisione D1):**
- **Gruppi.**
  - I sensori si raggruppano per `sensor.category`, nell'ordine di `CATEGORY_ORDER`. Le categorie sconosciute vengono dopo, in ordine alfabetico.
  - Dentro un gruppo resta l'ordine dello schema.
  - L'intestazione del gruppo è `advanced.category.<categoria>`; se la chiave manca si mostra la categoria così com'è.
- **Colonne.**
  - Sensore;
  - Attuale: il valore dal vivo dello store;
  - Min, Max e Media: le statistiche del core dall'avvio dell'app o dall'ultimo azzeramento (D1).

  Tutte passano da `formatValue` nell'unità del sensore. "—" quando il valore non è noto: nessun campione ancora, oppure subito dopo un azzeramento, fino al campione successivo.
- **Traffico di rete in bit/s** (decisione del coordinatore). Sulle pagine di rete il traffico usa la stessa unità della vista Semplificata: la tabella riceve `rate="bits"` e lo passa a `formatValue(…, { rate })` e a `formatAverage`, quindi `bytes_per_second` si legge in `Mbit/s`. Sulle altre pagine (dischi) resta in `MB/s`.
- **Flag** (`boolean`, per esempio la limitazione per potenza). Attuale, Min e Max si leggono come `flag.on`/`flag.off`. La Media è la quota di tempo in cui il flag era attivo (0,25 → "25%"), perché un "Attivo" medio non avrebbe senso.
- **Lettura delle statistiche.**
  - `getStats` con gli id di tutti i sensori della pagina: subito e poi ogni secondo, solo mentre `document.visibilityState` è `visible`;
  - al ritorno a `visible` si legge subito;
  - una risposta lenta non accumula richieste; una risposta partita prima di un azzeramento si scarta;
  - un errore finisce in console (`sensor statistics unavailable`) e la lettura continua al giro successivo.
- **Azzeramento.** Il pulsante `advanced.table.reset` ("Azzera min/max", spec §7.3):
  - chiama `resetStats` con gli id della pagina, cioè azzera min, max e media nel core (D1);
  - svuota subito quelle righe e rilegge;
  - resta disabilitato mentre la chiamata è in corso.
- **Sperimentale.** Solo i sensori con `experimental: true` hanno accanto al nome l'etichetta `advanced.experimental`, nel colore `--warn`.
- **Badge della fonte.**
  - Testo breve: `NVML`, `D3DKMT`, `IP HELPER`.
  - `title` = `t('source.<nome>')`, per esempio "NVIDIA Management Library (NVML)".
  - È nel DOM su ogni riga, ma compare solo passando sopra la riga o col focus (spec §7.3: "visibile passandoci sopra").

- [ ] **Step 1: Scrivi i test (falliscono)**

`app/src/lib/advanced/pages.test.ts` (nuovo). Il Task 13 lo estende:

```ts
import { MOCK_SCHEMA } from '../backend/mock';
import { DASH } from '../format';
import { catalogs, translate } from '../i18n/index.svelte';
import type { Sensor } from '../types';
import { CATEGORY_ORDER, categoryLabel, formatAverage, groupSensors, sourceCode } from './pages';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
const GPU = 'gpu/pci-0000:01:00.0';

const sensor = (id: string, category: string): Sensor => ({
  id,
  deviceId: 'd',
  kind: 'load',
  unit: 'percent',
  label: { key: 'k' },
  source: 'mock',
  category,
});

test('categories follow the table order, unknown ones last and alphabetical', () => {
  const groups = groupSensors([
    sensor('a', 'zeta'),
    sensor('b', 'load'),
    sensor('c', 'alpha'),
    sensor('d', 'temperature'),
    sensor('e', 'load'),
    sensor('f', 'flag'),
  ]);
  expect(groups.map((g) => g.category)).toEqual(['temperature', 'load', 'flag', 'alpha', 'zeta']);
  expect(groups[1].sensors.map((s) => s.id)).toEqual(['b', 'e']);
});

test('gpu sensors of the mock group by category in table order', () => {
  const groups = groupSensors(MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU));
  expect(groups.map((g) => g.category)).toEqual(['temperature', 'load', 'clock', 'power', 'data']);
  expect(groups[0].sensors.map((s) => s.id)).toEqual([`${GPU}/temperature/core`, `${GPU}/temperature/hotspot`]);
  expect(groupSensors([])).toEqual([]);
});

test('every known category has a heading in both languages', () => {
  expect(CATEGORY_ORDER).toEqual(['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'throughput', 'link', 'energy', 'flag']);
  for (const category of CATEGORY_ORDER) {
    expect(catalogs.en[`advanced.category.${category}`], category).toBeDefined();
    expect(catalogs.it[`advanced.category.${category}`], category).toBeDefined();
  }
  expect(categoryLabel('temperature', tEn)).toBe('Temperatures');
  expect(categoryLabel('mystery', tEn)).toBe('mystery');
});

test('every source has a badge description', () => {
  for (const source of ['pdh', 'win32', 'ip_helper', 'dxgi', 'd3dkmt', 'nvml', 'nvapi', 'adl', 'igcl', 'pnp', 'mock']) {
    expect(catalogs.en[`source.${source}`], source).toBeDefined();
  }
  expect(sourceCode('ip_helper')).toBe('IP HELPER');
  expect(sourceCode('nvml')).toBe('NVML');
});

test('the average of a flag is the share of time it was on', () => {
  expect(formatAverage({ min: 0, max: 1, avg: 0.25, count: 8 }, 'boolean', 'en', tEn)).toBe('25%');
  expect(formatAverage({ min: 30, max: 60, avg: 44.6, count: 8 }, 'percent', 'en', tEn)).toBe('45%');
  expect(formatAverage(null, 'celsius', 'en', tEn)).toBe(DASH);
  expect(formatAverage({ min: 0, max: 2e6, avg: 1e6, count: 4 }, 'bytes_per_second', 'en', tEn, { rate: 'bits' })).toBe('8.0 Mbit/s');
});
```

`app/src/lib/advanced/statsPoller.test.ts` (nuovo). Usa timer finti; `getStats` e `resetStats` sono `vi.fn` per controllare risposte lente e fallite:

```ts
import { MOCK_SCHEMA } from '../backend/mock';
import type { SensorStats, StatsReply } from '../types';
import { FakeBackend } from '../../test/fake-backend';
import { StatsPoller } from './statsPoller.svelte';

const A = 'cpu/0/load/total';
const B = 'cpu/0/clock/effective';
const STATS: SensorStats = { min: 1, max: 9, avg: 5, count: 3 };

let visibility: DocumentVisibilityState = 'visible';
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
const setVisibility = (state: DocumentVisibilityState) => {
  visibility = state;
  document.dispatchEvent(new Event('visibilitychange'));
};

function setup(ids = [A, B]) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  const getStats = vi.fn(async (req: string[]): Promise<StatsReply> => ({ revision: 1, stats: req.map((id) => (id === A ? STATS : null)) }));
  const resetStats = vi.fn(async () => {});
  backend.getStats = getStats;
  backend.resetStats = resetStats;
  let revision = 1;
  return { poller: new StatsPoller(backend, () => ids, () => revision), getStats, resetStats, setRevision: (value: number) => { revision = value; } };
}

beforeEach(() => {
  visibility = 'visible';
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

test('polls at start and then every second', async () => {
  const { poller, getStats } = setup();
  const stop = poller.start();
  await vi.advanceTimersByTimeAsync(0);
  expect(getStats).toHaveBeenCalledTimes(1);
  expect(getStats).toHaveBeenCalledWith([A, B]);
  expect(poller.statsOf(A)).toEqual(STATS);
  expect(poller.statsOf(B)).toBeNull();
  await vi.advanceTimersByTimeAsync(3000);
  expect(getStats).toHaveBeenCalledTimes(4);
  stop();
  await vi.advanceTimersByTimeAsync(5000);
  expect(getStats).toHaveBeenCalledTimes(4);
});

test('stops polling while hidden and polls at once when visible again', async () => {
  const { poller, getStats } = setup();
  const stop = poller.start();
  await vi.advanceTimersByTimeAsync(0);
  setVisibility('hidden');
  await vi.advanceTimersByTimeAsync(5000);
  expect(getStats).toHaveBeenCalledTimes(1);
  setVisibility('visible');
  await vi.advanceTimersByTimeAsync(0);
  expect(getStats).toHaveBeenCalledTimes(2);
  stop();
});

test('a slow reply does not pile up requests', async () => {
  const { poller, getStats } = setup();
  let release!: () => void;
  getStats.mockImplementationOnce(
    (req) => new Promise((done) => (release = () => done({ revision: 1, stats: req.map(() => null) }))),
  );
  const stop = poller.start();
  await vi.advanceTimersByTimeAsync(3000);
  expect(getStats).toHaveBeenCalledTimes(1);
  release();
  await vi.advanceTimersByTimeAsync(1000);
  expect(getStats).toHaveBeenCalledTimes(2);
  stop();
});

test('reset clears the page sensors in the core and reads them again', async () => {
  const { poller, getStats, resetStats } = setup();
  await poller.poll();
  expect(poller.statsOf(A)).toEqual(STATS);
  getStats.mockImplementationOnce(async (req) => ({ revision: 1, stats: req.map(() => null) }));
  await poller.reset();
  expect(resetStats).toHaveBeenCalledWith([A, B]);
  expect(getStats).toHaveBeenCalledTimes(2);
  expect(poller.statsOf(A)).toBeNull();
});

test('a reply that started before a reset is dropped', async () => {
  const { poller, getStats } = setup();
  let release!: () => void;
  getStats.mockImplementationOnce(
    (req) => new Promise((done) => (release = () => done({ revision: 1, stats: req.map(() => STATS) }))),
  );
  const stale = poller.poll();
  getStats.mockImplementationOnce(async (req) => ({ revision: 1, stats: req.map(() => null) }));
  await poller.reset();
  release();
  await stale;
  expect(poller.statsOf(A)).toBeNull();
});

test('errors are logged and polling goes on', async () => {
  const { poller, getStats, resetStats } = setup();
  const error = vi.spyOn(console, 'error').mockImplementation(() => {});
  getStats.mockRejectedValueOnce(new Error('offline'));
  resetStats.mockRejectedValueOnce(new Error('offline'));
  await poller.poll();
  await poller.reset();
  expect(error).toHaveBeenCalledWith('sensor statistics unavailable', expect.any(Error));
  expect(error).toHaveBeenCalledWith('cannot reset the sensor statistics', expect.any(Error));
  await poller.poll();
  expect(poller.statsOf(A)).toEqual(STATS);
  error.mockRestore();
});

test('a page without sensors asks nothing', async () => {
  const { poller, getStats } = setup([]);
  await poller.poll();
  expect(getStats).not.toHaveBeenCalled();
});

test('cached statistics disappear immediately when the schema changes', async () => {
  const { poller, setRevision } = setup();
  await poller.poll();
  expect(poller.statsOf(A)).toEqual(STATS);
  setRevision(2);
  expect(poller.statsOf(A)).toBeNull();
});

test('a late reply from the previous schema is dropped and the new schema retries', async () => {
  const { poller, getStats, setRevision } = setup();
  let release!: () => void;
  getStats.mockImplementationOnce(
    (req) => new Promise((done) => (release = () => done({ revision: 1, stats: req.map(() => STATS) }))),
  );
  const old = poller.poll();
  setRevision(2);
  release();
  await old;
  expect(poller.statsOf(A)).toBeNull();
  getStats.mockImplementationOnce(async (req) => ({ revision: 2, stats: req.map(() => STATS) }));
  await poller.poll();
  expect(poller.statsOf(A)).toEqual(STATS);
});

test('a reply newer than the displayed schema is not published', async () => {
  const { poller, getStats } = setup();
  getStats.mockImplementationOnce(async (req) => ({ revision: 2, stats: req.map(() => STATS) }));
  await poller.poll();
  expect(poller.statsOf(A)).toBeNull();
});
```

`app/src/components/advanced/SensorTable.test.ts` (nuovo). Usa le statistiche di `FakeBackend`. Il sensore di limitazione con fonte `nvml` si aggiunge ai sensori della GPU mock per provare i flag e il badge:

```ts
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { StatsPoller } from '../../lib/advanced/statsPoller.svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { DASH, formatValue } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import type { Sensor } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import SensorTable from './SensorTable.svelte';

const GPU = 'gpu/pci-0000:01:00.0';
const LOAD = `${GPU}/load/core`;
const THROTTLE: Sensor = {
  id: `${GPU}/flag/throttle-power`,
  deviceId: GPU,
  kind: 'flag',
  unit: 'boolean',
  label: { key: 'gpu.throttle.power' },
  source: 'nvml',
  category: 'flag',
};
const sensors = [...MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU), THROTTLE];
const ids = sensors.map((s) => s.id);
const current: Record<string, number> = { [LOAD]: 63, [THROTTLE.id]: 1 };
const valueOf = (id: string) => current[id] ?? null;

beforeEach(() => {
  i18n.locale = 'en';
});
afterEach(cleanup);

function setup() {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.stats = {
    [LOAD]: { min: 5, max: 95, avg: 40.4, count: 10 },
    [THROTTLE.id]: { min: 0, max: 1, avg: 0.25, count: 8 },
  };
  const stats = new StatsPoller(backend, () => ids, () => MOCK_SCHEMA.revision);
  render(SensorTable, { sensors, valueOf, stats });
  return { backend, stats };
}

/** Text of the value cells in the row of the sensor labelled `label`. */
const cells = (label: string) => [...screen.getByText(label).closest('tr')!.querySelectorAll('td')].map((td) => td.textContent);

test('groups follow the category order', () => {
  setup();
  const headings = screen.getAllByRole('columnheader').filter((th) => th.getAttribute('scope') === 'colgroup');
  expect(headings.map((h) => h.textContent)).toEqual(
    ['temperature', 'load', 'clock', 'power', 'data', 'flag'].map((c) => t(`advanced.category.${c}`)),
  );
});

test('rows show current, min, max and average in the sensor unit', async () => {
  const { stats } = setup();
  await stats.poll();
  flushSync();
  expect(cells(t('sensor.gpu.load.core'))).toEqual(['63%', '5%', '95%', '40%']);
  expect(cells(t('sensor.gpu.temperature.core'))).toEqual([DASH, DASH, DASH, DASH]);
});

test('flags read as on/off and their average as the share of time on', async () => {
  const { stats } = setup();
  await stats.poll();
  flushSync();
  expect(cells(t('sensor.gpu.throttle.power'))).toEqual([t('flag.on'), t('flag.off'), t('flag.on'), '25%']);
  expect(t('flag.on')).toBe(formatValue(1, 'boolean', 'en', t));
});

test('experimental sensors are marked and every row has its source badge', () => {
  setup();
  const hotspotRow = screen.getByText(t('sensor.gpu.temperature.hotspot')).closest('tr')!;
  expect(hotspotRow.textContent).toContain(t('advanced.experimental'));
  expect(screen.getAllByText(t('advanced.experimental'))).toHaveLength(1);
  expect(screen.getByText('NVML').getAttribute('title')).toBe(t('source.nvml'));
  expect(screen.getAllByTitle(t('source.mock'))).toHaveLength(sensors.length - 1);
});

test('reset clears the page sensors in the core and reads them again', async () => {
  const { backend, stats } = setup();
  await stats.poll();
  flushSync();
  expect(cells(t('sensor.gpu.load.core'))[1]).toBe('5%');

  await fireEvent.click(screen.getByRole('button', { name: t('advanced.table.reset') }));
  await vi.waitFor(() => expect(backend.statsCalls).toHaveLength(2));
  expect(backend.resetCalls).toEqual([ids]);
  flushSync();
  // FakeBackend drops the reset entries, as the core does until the next sample.
  expect(cells(t('sensor.gpu.load.core'))).toEqual(['63%', DASH, DASH, DASH]);
});

test('labels follow the language', async () => {
  i18n.locale = 'it';
  const { stats } = setup();
  await stats.poll();
  flushSync();
  expect(screen.getByRole('button', { name: 'Azzera min/max' })).toBeTruthy();
  expect(screen.getByRole('columnheader', { name: 'Media' })).toBeTruthy();
  expect(cells(t('sensor.gpu.load.core'))).toEqual(['63%', '5%', '95%', '40%']);
  expect(screen.getByText('Indicatori di stato')).toBeTruthy();
});

test('network pages show byte rates in bits, like the Simple view', async () => {
  const DOWN = 'network/mock-eth/throughput/down';
  const netSensors = MOCK_SCHEMA.sensors.filter((s) => s.deviceId === 'network/mock-eth');
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.stats = { [DOWN]: { min: 125_000, max: 6_000_000, avg: 1_000_000, count: 4 } };
  const stats = new StatsPoller(backend, () => netSensors.map((s) => s.id), () => MOCK_SCHEMA.revision);
  render(SensorTable, { sensors: netSensors, valueOf: (id: string) => (id === DOWN ? 6_000_000 : null), stats, rate: 'bits' });
  await stats.poll();
  flushSync();
  expect(cells(t('sensor.network.down'))).toEqual(['48 Mbit/s', '1.0 Mbit/s', '48 Mbit/s', '8.0 Mbit/s']);
});
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

```bash
cd app && pnpm test
```

Risultato atteso: `Test Files  3 failed | 17 passed (20)`, `Tests  132 passed (132)`. I tre file nuovi falliscono all'import:
- `Failed to resolve import "./pages" from "src/lib/advanced/pages.test.ts"`;
- `Failed to resolve import "./statsPoller.svelte" from "src/lib/advanced/statsPoller.test.ts"`;
- `Failed to resolve import "./SensorTable.svelte" from "src/components/advanced/SensorTable.test.ts"`.

- [ ] **Step 3: Implementa `pages.ts` e `StatsPoller`**

`app/src/lib/advanced/pages.ts` (nuovo):

```ts
import { formatPercent, formatValue, type FormatOptions } from '../format';
import { catalogs, type Translate } from '../i18n/index.svelte';
import type { Sensor, SensorStats, Source, Unit } from '../types';

/** Statistics since app start (or the last reset) of one sensor; null while unknown. */
export type StatsOf = (id: string) => SensorStats | null;

export interface SensorGroup {
  category: string;
  sensors: Sensor[];
}

/** Table order of the sensor categories (spec §7.3); unknown categories follow alphabetically. */
export const CATEGORY_ORDER = ['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'throughput', 'link', 'energy', 'flag'];

/** Groups by `category`, keeping the schema order inside each group. */
export function groupSensors(sensors: Sensor[]): SensorGroup[] {
  const groups = new Map<string, Sensor[]>();
  for (const sensor of sensors) {
    const list = groups.get(sensor.category);
    if (list) list.push(sensor);
    else groups.set(sensor.category, [sensor]);
  }
  const rank = (category: string) => {
    const i = CATEGORY_ORDER.indexOf(category);
    return i < 0 ? CATEGORY_ORDER.length : i;
  };
  return [...groups.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
    .map(([category, list]) => ({ category, sensors: list }));
}

/** Group heading: `advanced.category.<name>` when translated, else the raw category. */
export function categoryLabel(category: string, t: Translate): string {
  const key = `advanced.category.${category}`;
  return key in catalogs.en ? t(key) : category;
}

/** Short badge text for a source, e.g. "NVML", "IP HELPER". */
export function sourceCode(source: Source): string {
  return source.replace(/_/g, ' ').toUpperCase();
}

/** Average column: for on/off flags it is the share of time the flag was on. */
export function formatAverage(
  stats: SensorStats | null,
  unit: Unit,
  locale: string,
  t: Translate,
  opts: FormatOptions = {},
): string {
  if (!stats) return formatValue(null, unit, locale, t, opts);
  return unit === 'boolean' ? formatPercent(stats.avg * 100, locale) : formatValue(stats.avg, unit, locale, t, opts);
}
```

`app/src/lib/advanced/statsPoller.svelte.ts` (nuovo). Il file è `.svelte.ts` perché `byId` è uno `$state.raw`: i componenti che leggono `statsOf` si aggiornano a ogni risposta.

```ts
import type { Backend } from '../backend/backend';
import type { SensorStats } from '../types';
import type { StatsOf } from './pages';

export const STATS_INTERVAL_MS = 1000;

/**
 * Reads min/max/avg from the core (decision D1) for the sensors of a page, once per
 * interval and only while the document is visible; `reset` clears them in the core.
 */
export class StatsPoller {
  /** Latest statistics by sensor id. */
  byId = $state.raw<ReadonlyMap<string, SensorStats>>(new Map());
  readonly #backend: Backend;
  readonly #ids: () => string[];
  readonly #revision: () => number | null;
  #byRevision = $state<number | null>(null);
  readonly #intervalMs: number;
  #timer: ReturnType<typeof setInterval> | undefined;
  #inFlight = false;
  /** Number of the newest request; older replies are dropped. */
  #latest = 0;

  constructor(backend: Backend, ids: () => string[], revision: () => number | null, intervalMs = STATS_INTERVAL_MS) {
    this.#backend = backend;
    this.#ids = ids;
    this.#revision = revision;
    this.#intervalMs = intervalMs;
  }

  readonly statsOf: StatsOf = (id) =>
    this.#byRevision === this.#revision() ? this.byId.get(id) ?? null : null;

  /** Polls now and then every interval; returns the stop function. */
  start(): () => void {
    if (this.#timer === undefined) {
      this.#timer = setInterval(() => {
        if (!this.#inFlight) void this.poll();
      }, this.#intervalMs);
      document.addEventListener('visibilitychange', this.#onVisibility);
      void this.poll();
    }
    return () => this.stop();
  }

  stop(): void {
    clearInterval(this.#timer);
    this.#timer = undefined;
    document.removeEventListener('visibilitychange', this.#onVisibility);
  }

  readonly #onVisibility = () => {
    if (document.visibilityState === 'visible') void this.poll();
  };

  async poll(): Promise<void> {
    if (document.visibilityState === 'hidden') return;
    const ids = this.#ids();
    const revision = this.#revision();
    const request = ++this.#latest;
    if (ids.length === 0) {
      this.#inFlight = false;
      this.byId = new Map();
      return;
    }
    this.#inFlight = true;
    try {
      const reply = await this.#backend.getStats(ids);
      if (request !== this.#latest || revision !== this.#revision() || reply.revision !== revision) return;
      const next = new Map<string, SensorStats>();
      ids.forEach((id, i) => {
        const stats = reply.stats[i];
        if (stats) next.set(id, stats);
      });
      this.byId = next;
      this.#byRevision = revision;
    } catch (error) {
      console.error('sensor statistics unavailable', error);
    } finally {
      if (request === this.#latest) this.#inFlight = false;
    }
  }

  /** Clears min/max/avg of every sensor of the page in the core, then reads them again. */
  async reset(): Promise<void> {
    const ids = this.#ids();
    try {
      await this.#backend.resetStats(ids);
    } catch (error) {
      console.error('cannot reset the sensor statistics', error);
      return;
    }
    const cleared = new Map(this.byId);
    for (const id of ids) cleared.delete(id);
    this.byId = cleared;
    await this.poll();
  }
}
```

- [ ] **Step 4: Implementa `SensorTable.svelte` e le traduzioni**

`app/src/components/advanced/SensorTable.svelte` (nuovo):

```svelte
<script lang="ts">
  import { sensorLabel } from '../../lib/advanced/labels';
  import { categoryLabel, formatAverage, groupSensors, sourceCode } from '../../lib/advanced/pages';
  import type { StatsPoller } from '../../lib/advanced/statsPoller.svelte';
  import { formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { ValueOf } from '../../lib/select';
  import type { Sensor } from '../../lib/types';

  let {
    sensors,
    valueOf,
    stats,
    rate = 'bytes',
  }: {
    sensors: Sensor[];
    valueOf: ValueOf;
    stats: StatsPoller;
    /** 'bits' on network pages: traffic in bit/s, like the Simple view. */
    rate?: 'bits' | 'bytes';
  } = $props();

  const groups = $derived(groupSensors(sensors));
  const locale = $derived(i18n.locale);
  const opts = $derived({ rate });
  let resetting = $state(false);

  async function reset() {
    resetting = true;
    try {
      await stats.reset();
    } finally {
      resetting = false;
    }
  }
</script>

<section class="sensors">
  <div class="bar">
    <button type="button" class="reset" disabled={resetting} onclick={reset}>{t('advanced.table.reset')}</button>
  </div>
  <table>
    <thead>
      <tr>
        <th scope="col">{t('advanced.table.sensor')}</th>
        <th scope="col" class="num">{t('advanced.table.current')}</th>
        <th scope="col" class="num">{t('advanced.table.min')}</th>
        <th scope="col" class="num">{t('advanced.table.max')}</th>
        <th scope="col" class="num">{t('advanced.table.avg')}</th>
      </tr>
    </thead>
    {#each groups as group (group.category)}
      <tbody>
        <tr class="group">
          <th scope="colgroup" colspan="5">{categoryLabel(group.category, t)}</th>
        </tr>
        {#each group.sensors as sensor (sensor.id)}
          {@const s = stats.statsOf(sensor.id)}
          <tr>
            <th scope="row">
              <span class="name">{sensorLabel(sensor, t)}</span>
              {#if sensor.experimental}<span class="tag exp">{t('advanced.experimental')}</span>{/if}
              <span class="tag source" title={t(`source.${sensor.source}`)}>{sourceCode(sensor.source)}</span>
            </th>
            <td class="num">{formatValue(valueOf(sensor.id), sensor.unit, locale, t, opts)}</td>
            <td class="num">{formatValue(s?.min ?? null, sensor.unit, locale, t, opts)}</td>
            <td class="num">{formatValue(s?.max ?? null, sensor.unit, locale, t, opts)}</td>
            <td class="num">{formatAverage(s, sensor.unit, locale, t, opts)}</td>
          </tr>
        {/each}
      </tbody>
    {/each}
  </table>
</section>

<style>
  .sensors {
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .bar {
    display: flex;
    justify-content: flex-end;
    margin-bottom: 8px;
  }
  .reset {
    padding: 5px 12px;
    font-size: 13px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    cursor: pointer;
  }
  .reset:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .reset:disabled {
    opacity: 0.5;
    cursor: default;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 5px 8px;
    text-align: left;
    font-weight: 400;
    border-bottom: 1px solid var(--border);
  }
  thead th {
    color: var(--text-muted);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .group th {
    padding-top: 14px;
    color: var(--accent-2);
    font-weight: 600;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .tag {
    margin-left: 6px;
    padding: 1px 6px;
    font-size: 10px;
    letter-spacing: 0.06em;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-muted);
    vertical-align: middle;
  }
  .exp {
    color: var(--warn);
    border-color: color-mix(in srgb, var(--warn) 45%, transparent);
  }
  /* Spec §7.3: the source badge shows on hover. */
  .source {
    opacity: 0;
    cursor: help;
    transition: opacity 0.15s;
  }
  tr:hover .source,
  tr:focus-within .source {
    opacity: 1;
  }
</style>
```

`app/src/lib/i18n/en.json`: inserisci subito dopo la riga `"advanced.chart.empty": "Choose at least one series.",` (Task 11):

```json
  "advanced.table.sensor": "Sensor",
  "advanced.table.current": "Current",
  "advanced.table.min": "Min",
  "advanced.table.max": "Max",
  "advanced.table.avg": "Average",
  "advanced.table.reset": "Reset min/max",
  "advanced.experimental": "Experimental",
  "advanced.category.temperature": "Temperatures",
  "advanced.category.load": "Load",
  "advanced.category.clock": "Clocks",
  "advanced.category.power": "Power",
  "advanced.category.percent": "Percentages",
  "advanced.category.voltage": "Voltages",
  "advanced.category.current": "Currents",
  "advanced.category.fan": "Fans",
  "advanced.category.data": "Data",
  "advanced.category.throughput": "Throughput",
  "advanced.category.link": "Link",
  "advanced.category.energy": "Energy",
  "advanced.category.flag": "Status flags",
```

`app/src/lib/i18n/it.json`: inserisci subito dopo la riga `"advanced.chart.empty": "Scegli almeno una serie.",`:

```json
  "advanced.table.sensor": "Sensore",
  "advanced.table.current": "Attuale",
  "advanced.table.min": "Min",
  "advanced.table.max": "Max",
  "advanced.table.avg": "Media",
  "advanced.table.reset": "Azzera min/max",
  "advanced.experimental": "Sperimentale",
  "advanced.category.temperature": "Temperature",
  "advanced.category.load": "Carico",
  "advanced.category.clock": "Clock",
  "advanced.category.power": "Potenza",
  "advanced.category.percent": "Percentuali",
  "advanced.category.voltage": "Tensioni",
  "advanced.category.current": "Correnti",
  "advanced.category.fan": "Ventole",
  "advanced.category.data": "Dati",
  "advanced.category.throughput": "Velocità di trasferimento",
  "advanced.category.link": "Collegamento",
  "advanced.category.energy": "Energia",
  "advanced.category.flag": "Indicatori di stato",
```

- [ ] **Step 5: Esegui i test e verifica che passino**

```bash
cd app
pnpm test
pnpm check
pnpm build
```

Risultato atteso:
- `pnpm test`: `Test Files  20 passed (20)`, `Tests  154 passed (154)` (5 in `pages.test.ts`, 10 in `statsPoller.test.ts`, 7 in `SensorTable.test.ts`);
- `pnpm check`: `0 ERRORS 0 WARNINGS`;
- `pnpm build`: OK, bundle invariato: la tabella si monta nel Task 13.

- [ ] **Step 6: Verifica dal vivo**

In questo task non c'è: la tabella si monta nella pagina con il Task 13, che la verifica nel browser e nell'app vera (Task 13, Step 6–8).

- [ ] **Step 7: Commit**

```bash
git add app/src
git commit -m "feat(ui): sensor table with core min/max/avg, reset and source badges"
```

---

### Task 13: UI: pagine della vista Avanzata (KPI, `DevicePage`, proprietà del dispositivo, processi GPU)

> **Aggancio al Task 10 (presupposto da riconciliare in integrazione).** Questo task presume ciò che il Task 10 produce:
> - `app/src/components/advanced/DevicePage.svelte` esiste come pagina minima, con props `{ entry: SidebarEntry; store: LiveStore; backend: Backend }`;
> - `AdvancedView.svelte` la monta dentro `{#key current.id}`, sotto il proprio `<header>` (`h2` con il tipo, `.device` con il nome del device).
>
> Il Task 13 **sovrascrive per intero** `DevicePage.svelte` e ne mantiene le props, **non tocca** `AdvancedView.svelte` e non ripete titolo né nome del device. Lo schema si legge da `store.schema`. Una bozza precedente prevedeva un `SectionPlaceholder` dentro `AdvancedView`; il Task 10 finale non lo crea, quindi lì non c'è nulla da sostituire. Se in integrazione `AdvancedView` monta la pagina in un altro modo, basta che passi le stesse tre props.

**File:**
- Crea:
  - `app/src/components/advanced/KpiRow.svelte`
  - `app/src/components/advanced/DeviceInfo.svelte`
  - `app/src/components/advanced/GpuProcesses.svelte`
- Modifica:
  - `app/src/lib/advanced/pages.ts` (KPI, serie predefinite, righe delle proprietà; file completo)
  - `app/src/components/advanced/DevicePage.svelte` (sostituisce per intero la pagina minima del Task 10)
  - `app/src/lib/i18n/en.json`, `app/src/lib/i18n/it.json` (chiavi `advanced.kpi.*`, `advanced.processes.*`, `advanced.info.*`)
- Test:
  - Modifica: `app/src/lib/advanced/pages.test.ts` (file completo)
  - Crea: `app/src/components/advanced/DevicePage.test.ts`, `app/src/components/advanced/GpuProcesses.test.ts`

**Interfacce:**
- Usa:
  - Task 9:
    - `Backend.getGpuProcesses(deviceId)` e `GpuProcess`;
    - `formatValue(value, unit, locale, t, opts?)` con `opts.rate` (`'bits'` sulle pagine di rete), `formatBytes`, `formatPercent`;
    - le chiavi `property.<chiave>`, la cui etichetta contiene già l'unità ("Default power limit (W)");
    - `FakeBackend.stats`, `statsCalls`, `gpuProcesses`, `gpuProcessCalls`, `historyCalls`.
  - Task 10: `SidebarEntry` (`id`, `kind`, `deviceIds`, `labelKey`, `labelArg`), `AdvancedView` (monta la pagina con `{#key}`), `SECTION_KEY`.
  - Task 11: `HistoryChart` (props `sectionId`, `sensors`, `defaults`, `schema`, `store`, `backend`) e `FakeUplot`.
  - Task 12: `StatsPoller`, `SensorTable` (con la prop `rate?: 'bits' | 'bytes'`), `StatsOf` e il resto di `pages.ts` (`formatAverage` accetta `opts`).
  - Task 5, 6 e 8: il sensore `storage.temperature`; le proprietà `pcieMaxGen`, `pcieMaxWidth`, `powerLimit*W`, `temp*C` dei device; `get_gpu_processes`, che restituisce al massimo 20 righe ordinate per carico.
  - M1: `ValueOf`, `AnimatedNumber`.
- Produce:
  - in `lib/advanced/pages.ts`, oltre a quanto già fatto dal Task 12:
    ```ts
    export interface KpiDef {
      id: string;
      labelKey: string;                                              // advanced.kpi.<id>
      value: (valueOf: ValueOf, stats: StatsOf) => number | null;
      unit: Unit;
      secondary?: (valueOf: ValueOf, stats: StatsOf) => string | null;
    }
    export function kpisFor(kind: DeviceKind, schema: Schema, deviceIds: string[]): KpiDef[];   // i primi 4 i cui sensori esistono
    export function defaultSeries(kind: DeviceKind, schema: Schema, deviceIds: string[]): string[];
    export const PROPERTY_ORDER: string[];
    export interface PropertyRow { key: string; label: string; value: string }
    export function propertyRows(device: Device, locale: string, t: Translate): PropertyRow[];
    ```
  - `KpiRow.svelte` (`{ kpis: KpiDef[]; valueOf: ValueOf; statsOf: StatsOf; rate?: 'bits' | 'bytes' }`), `DeviceInfo.svelte` (`{ devices: Device[] }`), `GpuProcesses.svelte` (`{ deviceId: string; backend: Backend }`);
  - `DevicePage.svelte`, con le stesse props del Task 10: `{ entry: SidebarEntry; store: LiveStore; backend: Backend }`;
  - le chiavi i18n `advanced.kpi.<id>` (18 KPI più `advanced.kpi.vramOf`), `advanced.processes.title/name/load/dedicated/shared/empty`, `advanced.info.title/yes/no`.

**Comportamento (spec §7.3; decisioni D1, D5, D9):**
- **Struttura della pagina**, dall'alto:
  1. riga dei KPI;
  2. grafico storico;
  3. tabella dei sensori;
  4. se servono, affiancati: proprietà del device e processi GPU.

  I titoli di sezione interni sono `h3` con lo stile `.label`: l'`h2` resta quello di `AdvancedView`.
- **KPI** (spec §7.3: 4 per tipo). Si prendono i primi 4, nell'ordine della tabella, i cui sensori esistono:

  | Tipo | KPI, in ordine |
  |---|---|
  | CPU | carico (`cpu.load.total`); clock (`cpu.clock.effective`); thread più carico (massimo dei `cpu.load.thread`); carico massimo (max delle statistiche di `cpu.load.total`) |
  | GPU | carico (`gpu.load.core`); temperatura (`gpu.temperature.core`, altrimenti `gpu.temperature.hotspot`); potenza (`gpu.power.board` in W, altrimenti `gpu.power.limitPercent` in %); VRAM (`gpu.memory.dedicatedUsed`, sotto "di <totale>" se esiste `gpu.memory.dedicatedTotal`); clock (`gpu.clock.core`) |
  | RAM | carico (`memory.load`); in uso (`memory.used`); totale (`memory.total`); disponibile (totale − in uso, mai sotto 0) |
  | Disco | tempo attivo (`storage.active`); lettura (`storage.read`); scrittura (`storage.write`); temperatura (`storage.temperature`, Task 5); spazio libero (il primo `storage.volumeFree`, sotto la lettera del volume) |
  | Rete | download (`network.down`); upload (`network.up`); velocità del collegamento (`network.linkSpeed`); picco di download (max delle statistiche di `network.down`) |
  | Altri tipi | nessuno |

  - I valori passano da `formatValue` nell'unità del sensore, con `AnimatedNumber` (≤ 300 ms, rispetta `prefers-reduced-motion`).
  - I KPI "massimo" e "picco" leggono le statistiche del core (D1), quindi il pulsante "Azzera min/max" della tabella azzera anche quelli.
  - **Traffico di rete in bit/s** (decisione del coordinatore). Sulle pagine di tipo `network` `DevicePage` passa `rate="bits"` a `KpiRow` e a `SensorTable`: download, upload e picco di download si leggono in `Mbit/s`, come nel riquadro della vista Semplificata. Sulle altre pagine (dischi) i byte al secondo restano in `MB/s`.
  - Una GPU senza NVML (per esempio l'iGPU) mostra solo i KPI dei sensori che ha, compreso il clock come quinto candidato.
- **Serie predefinite del grafico:**
  - CPU: carico totale e clock;
  - GPU: carico core e temperatura core, oppure hotspot;
  - RAM: `memory.load`;
  - disco: lettura e scrittura;
  - rete: download e upload;
  - altri tipi: il primo sensore della pagina.
- **Statistiche.** `DevicePage` crea un solo `StatsPoller` sugli id di tutti i sensori della pagina, lo avvia al montaggio (ogni secondo, solo con la finestra visibile) e lo ferma allo smontaggio. Con il `{#key}` del Task 10 ogni sezione riparte da capo.
- **Proprietà del device** (D9). Il riquadro compare solo se un device della pagina ha proprietà. Per ogni proprietà:
  - l'etichetta è `property.<chiave>`, oppure la chiave stessa se manca la traduzione;
  - l'ordine è `PROPERTY_ORDER`, poi le chiavi sconosciute in ordine alfabetico;
  - `pcieMaxGen` diventa "Gen 4" e `pcieMaxWidth` "x16";
  - i limiti in W e in °C sono numeri nel formato della lingua: l'unità è già nell'etichetta (Task 9);
  - `integrated` diventa `advanced.info.yes`/`advanced.info.no`;
  - gli altri valori, come `pciAddress`, restano come sono.
- **Processi GPU** (D5), solo sulle pagine GPU, per il primo device della voce:
  - `getGpuProcesses` subito, poi ogni 2 s, solo con la finestra visibile; al ritorno a `visible` si legge subito; niente richieste sovrapposte;
  - colonne:
    - processo, con il pid in piccolo;
    - carico, per esempio "87% · 3D" (il motore c'è solo quando il carico è > 0; "—" se il carico è `null`);
    - memoria dedicata e condivisa (`formatBytes`, "—" se `null`);
  - lista vuota → `advanced.processes.empty`; prima della prima risposta non si mostra nulla;
  - un errore finisce in console (`GPU process list unavailable`) e si riprova al giro successivo.

- [ ] **Step 1: Scrivi i test (falliscono)**

`app/src/lib/advanced/pages.test.ts`, file completo. I primi cinque test sono quelli del Task 12; gli altri coprono KPI, serie predefinite e proprietà:

```ts
import { MOCK_SCHEMA, mockValues } from '../backend/mock';
import { DASH, formatValue } from '../format';
import { catalogs, i18n, translate } from '../i18n/index.svelte';
import type { DeviceKind, Schema, Sensor, SensorStats } from '../types';
import {
  CATEGORY_ORDER,
  PROPERTY_ORDER,
  categoryLabel,
  defaultSeries,
  formatAverage,
  groupSensors,
  kpisFor,
  propertyRows,
  sourceCode,
  type StatsOf,
} from './pages';

const tEn = (key: string, params?: Record<string, string | number>) => translate('en', key, params);
const GPU = 'gpu/pci-0000:01:00.0';

const sensor = (id: string, category: string): Sensor => ({
  id,
  deviceId: 'd',
  kind: 'load',
  unit: 'percent',
  label: { key: 'k' },
  source: 'mock',
  category,
});

test('categories follow the table order, unknown ones last and alphabetical', () => {
  const groups = groupSensors([
    sensor('a', 'zeta'),
    sensor('b', 'load'),
    sensor('c', 'alpha'),
    sensor('d', 'temperature'),
    sensor('e', 'load'),
    sensor('f', 'flag'),
  ]);
  expect(groups.map((g) => g.category)).toEqual(['temperature', 'load', 'flag', 'alpha', 'zeta']);
  expect(groups[1].sensors.map((s) => s.id)).toEqual(['b', 'e']);
});

test('gpu sensors of the mock group by category in table order', () => {
  const groups = groupSensors(MOCK_SCHEMA.sensors.filter((s) => s.deviceId === GPU));
  expect(groups.map((g) => g.category)).toEqual(['temperature', 'load', 'clock', 'power', 'data']);
  expect(groups[0].sensors.map((s) => s.id)).toEqual([`${GPU}/temperature/core`, `${GPU}/temperature/hotspot`]);
  expect(groupSensors([])).toEqual([]);
});

test('every known category has a heading in both languages', () => {
  expect(CATEGORY_ORDER).toEqual(['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'throughput', 'link', 'energy', 'flag']);
  for (const category of CATEGORY_ORDER) {
    expect(catalogs.en[`advanced.category.${category}`], category).toBeDefined();
    expect(catalogs.it[`advanced.category.${category}`], category).toBeDefined();
  }
  expect(categoryLabel('temperature', tEn)).toBe('Temperatures');
  expect(categoryLabel('mystery', tEn)).toBe('mystery');
});

test('every source has a badge description', () => {
  for (const source of ['pdh', 'win32', 'ip_helper', 'dxgi', 'd3dkmt', 'nvml', 'nvapi', 'adl', 'igcl', 'pnp', 'mock']) {
    expect(catalogs.en[`source.${source}`], source).toBeDefined();
  }
  expect(sourceCode('ip_helper')).toBe('IP HELPER');
  expect(sourceCode('nvml')).toBe('NVML');
});

test('the average of a flag is the share of time it was on', () => {
  expect(formatAverage({ min: 0, max: 1, avg: 0.25, count: 8 }, 'boolean', 'en', tEn)).toBe('25%');
  expect(formatAverage({ min: 30, max: 60, avg: 44.6, count: 8 }, 'percent', 'en', tEn)).toBe('45%');
  expect(formatAverage(null, 'celsius', 'en', tEn)).toBe(DASH);
  expect(formatAverage({ min: 0, max: 2e6, avg: 1e6, count: 4 }, 'bytes_per_second', 'en', tEn, { rate: 'bits' })).toBe('8.0 Mbit/s');
});

const values = mockValues(3);
const valueOf = (id: string) => {
  const i = MOCK_SCHEMA.sensors.findIndex((s) => s.id === id);
  return i < 0 ? null : values[i];
};
const noStats: StatsOf = () => null;
const DISK = 'storage/device-mock-ssd';
const NIC = 'network/mock-eth';
const ids = (kind: DeviceKind, deviceId: string, schema: Schema = MOCK_SCHEMA) => kpisFor(kind, schema, [deviceId]).map((k) => k.id);
const kpi = (kind: DeviceKind, deviceId: string, id: string, schema: Schema = MOCK_SCHEMA) =>
  kpisFor(kind, schema, [deviceId]).find((k) => k.id === id)!;
const without = (...keys: string[]): Schema => ({ ...MOCK_SCHEMA, sensors: MOCK_SCHEMA.sensors.filter((s) => !keys.includes(s.label.key)) });
const plus = (schema: Schema, extra: Sensor): Schema => ({ ...schema, sensors: [...schema.sensors, extra] });

test('cpu kpis: load, clock, busiest thread and peak load', () => {
  expect(ids('cpu', 'cpu/0')).toEqual(['load', 'clock', 'busiestThread', 'peakLoad']);
  expect(kpi('cpu', 'cpu/0', 'load').value(valueOf, noStats)).toBe(valueOf('cpu/0/load/total'));
  const threads = MOCK_SCHEMA.sensors.filter((s) => s.label.key === 'cpu.load.thread').map((s) => valueOf(s.id)!);
  expect(kpi('cpu', 'cpu/0', 'busiestThread').value(valueOf, noStats)).toBe(Math.max(...threads));
  expect(kpi('cpu', 'cpu/0', 'busiestThread').value(() => null, noStats)).toBeNull();
  const peak: SensorStats = { min: 1, max: 97, avg: 30, count: 5 };
  const stats: StatsOf = (id) => (id === 'cpu/0/load/total' ? peak : null);
  expect(kpi('cpu', 'cpu/0', 'peakLoad').value(valueOf, stats)).toBe(97);
  expect(kpi('cpu', 'cpu/0', 'peakLoad').value(valueOf, noStats)).toBeNull();
});

test('gpu kpis: load, temperature, power and vram with its total', () => {
  i18n.locale = 'en';
  expect(ids('gpu', GPU)).toEqual(['load', 'temperature', 'power', 'vram']);
  const vram = kpi('gpu', GPU, 'vram');
  expect(vram.unit).toBe('bytes');
  expect(vram.secondary?.(valueOf, noStats)).toBe(tEn('advanced.kpi.vramOf', { total: '16.0 GB' }));
  expect(vram.secondary?.(() => null, noStats)).toBeNull();
  expect(kpi('gpu', GPU, 'power').unit).toBe('watt');
});

test('gpu kpis fall back to hotspot and power percentage', () => {
  const powerPct: Sensor = {
    id: `${GPU}/percent/power-limit`,
    deviceId: GPU,
    kind: 'percent',
    unit: 'percent',
    label: { key: 'gpu.power.limitPercent' },
    source: 'd3dkmt',
    category: 'percent',
  };
  const schema = plus(without('gpu.temperature.core', 'gpu.power.board'), powerPct);
  expect(kpi('gpu', GPU, 'temperature', schema).value(valueOf, noStats)).toBe(valueOf(`${GPU}/temperature/hotspot`));
  expect(kpi('gpu', GPU, 'power', schema).unit).toBe('percent');
});

test('a gpu with few sensors shows only what exists, clock included', () => {
  const schema = without('gpu.temperature.core', 'gpu.temperature.hotspot', 'gpu.power.board', 'gpu.memory.dedicatedUsed');
  expect(ids('gpu', GPU, schema)).toEqual(['load', 'clock']);
});

test('memory kpis: load, used, total and available', () => {
  expect(ids('memory', 'memory/0')).toEqual(['load', 'used', 'total', 'available']);
  const available = kpi('memory', 'memory/0', 'available').value(valueOf, noStats);
  expect(available).toBe(valueOf('memory/0/data/total')! - valueOf('memory/0/data/used')!);
  expect(kpi('memory', 'memory/0', 'available').value(() => null, noStats)).toBeNull();
});

test('storage kpis use the temperature when the disk reports it, else the free space', () => {
  expect(ids('storage', DISK)).toEqual(['active', 'read', 'write', 'freeSpace']);
  expect(kpi('storage', DISK, 'freeSpace').secondary?.(valueOf, noStats)).toBe('C:');
  const temperature: Sensor = {
    id: `${DISK}/temperature/drive`,
    deviceId: DISK,
    kind: 'temperature',
    unit: 'celsius',
    label: { key: 'storage.temperature' },
    source: 'win32',
    category: 'temperature',
  };
  expect(ids('storage', DISK, plus(MOCK_SCHEMA, temperature))).toEqual(['active', 'read', 'write', 'temperature']);
});

test('network kpis: down, up, link speed and peak download', () => {
  expect(ids('network', NIC)).toEqual(['down', 'up', 'linkSpeed', 'peakDown']);
  expect(kpi('network', NIC, 'linkSpeed').unit).toBe('bits_per_second');
  const stats: StatsOf = (id) => (id === `${NIC}/throughput/down` ? { min: 0, max: 5e6, avg: 1e6, count: 9 } : null);
  expect(kpi('network', NIC, 'peakDown').value(valueOf, stats)).toBe(5e6);
});

test('kinds without kpi definitions get none', () => {
  expect(kpisFor('motherboard', MOCK_SCHEMA, ['cpu/0'])).toEqual([]);
});

test('every kpi label is translated in both languages', () => {
  const pages: [DeviceKind, string][] = [['cpu', 'cpu/0'], ['gpu', GPU], ['memory', 'memory/0'], ['storage', DISK], ['network', NIC]];
  const keys = ['advanced.kpi.vramOf', 'advanced.kpi.temperature', 'advanced.kpi.clock', 'advanced.info.yes', 'advanced.info.no'];
  for (const [kind, id] of pages) keys.push(...kpisFor(kind, MOCK_SCHEMA, [id]).map((k) => k.labelKey));
  for (const key of keys) {
    expect(catalogs.en[key], key).toBeDefined();
    expect(catalogs.it[key], key).toBeDefined();
  }
});

test('default series per kind', () => {
  expect(defaultSeries('cpu', MOCK_SCHEMA, ['cpu/0'])).toEqual(['cpu/0/load/total', 'cpu/0/clock/effective']);
  expect(defaultSeries('gpu', MOCK_SCHEMA, [GPU])).toEqual([`${GPU}/load/core`, `${GPU}/temperature/core`]);
  expect(defaultSeries('gpu', without('gpu.temperature.core'), [GPU])).toEqual([`${GPU}/load/core`, `${GPU}/temperature/hotspot`]);
  expect(defaultSeries('memory', MOCK_SCHEMA, ['memory/0'])).toEqual(['memory/0/load/used']);
  expect(defaultSeries('storage', MOCK_SCHEMA, [DISK])).toEqual([`${DISK}/throughput/read`, `${DISK}/throughput/write`]);
  expect(defaultSeries('network', MOCK_SCHEMA, [NIC])).toEqual([`${NIC}/throughput/down`, `${NIC}/throughput/up`]);
  expect(defaultSeries('motherboard', MOCK_SCHEMA, ['cpu/0'])).toEqual(['cpu/0/load/total']);
  expect(defaultSeries('battery', MOCK_SCHEMA, ['none'])).toEqual([]);
});

test('device properties are translated, formatted and ordered', () => {
  const rows = propertyRows(
    {
      id: GPU,
      kind: 'gpu',
      name: 'GPU',
      properties: {
        tempSlowdownC: '94',
        zeta: 'abc',
        pcieMaxWidth: '16',
        integrated: 'false',
        pcieMaxGen: '4',
        pciAddress: '0000:01:00.0',
        powerLimitDefaultW: '320',
        tempMaxC: 'n/a',
      },
    },
    'en',
    tEn,
  );
  expect(rows.map((r) => r.key)).toEqual(['pciAddress', 'integrated', 'pcieMaxGen', 'pcieMaxWidth', 'powerLimitDefaultW', 'tempSlowdownC', 'tempMaxC', 'zeta']);
  expect(rows.map((r) => r.value)).toEqual([
    '0000:01:00.0',
    tEn('advanced.info.no'),
    formatValue(4, 'pcie_generation', 'en', tEn),
    formatValue(16, 'lanes', 'en', tEn),
    '320',
    '94',
    'n/a',
    'abc',
  ]);
  expect(rows[0].label).toBe(tEn('property.pciAddress'));
  expect(rows.at(-1)?.label).toBe('zeta');
  expect(propertyRows({ id: GPU, kind: 'gpu', name: 'GPU', properties: { powerLimitMaxW: '12345.5', integrated: 'true' } }, 'it', (k) => translate('it', k)).map((r) => r.value)).toEqual(['Sì', '12.345,5']);
  expect(propertyRows({ id: 'x', kind: 'cpu', name: 'x' }, 'en', tEn)).toEqual([]);
});

test('every known property has a label in both languages', () => {
  for (const key of PROPERTY_ORDER) {
    expect(catalogs.en[`property.${key}`], key).toBeDefined();
    expect(catalogs.it[`property.${key}`], key).toBeDefined();
  }
});
```

`app/src/components/advanced/DevicePage.test.ts` (nuovo). L'ultimo test monta `AdvancedView` del Task 10 e controlla che la pagina completa stia sotto il suo `h2`:

```ts
import { cleanup, render, screen } from '@testing-library/svelte';
import type { SidebarEntry } from '../../lib/advanced/nav';
import { SECTION_KEY } from '../../lib/advanced/persist';
import { MOCK_SCHEMA, mockValues } from '../../lib/backend/mock';
import { formatValue } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import { LiveStore } from '../../lib/live.svelte';
import type { GpuProcess, SensorStats } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import { FakeUplot } from '../../test/uplot-stub';
import AdvancedView from './AdvancedView.svelte';
import DevicePage from './DevicePage.svelte';

const plots = FakeUplot.instances;
const GPU = 'gpu/pci-0000:01:00.0';
const GPU_ENTRY: SidebarEntry = { id: GPU, kind: 'gpu', deviceIds: [GPU], labelKey: 'advanced.section.gpu', labelArg: 'Mock GeForce RTX 4080' };
const CPU_ENTRY: SidebarEntry = { id: 'cpu/0', kind: 'cpu', deviceIds: ['cpu/0'], labelKey: 'advanced.section.cpu', labelArg: 'Mock Ryzen 7 7800X3D' };
const GAME: GpuProcess = { pid: 4242, name: 'game.exe', loadPercent: 87, engine: '3D', dedicatedBytes: 1024 ** 3, sharedBytes: 0 };
const STATS: SensorStats = { min: 1, max: 2, avg: 1.5, count: 2 };
const idsOf = (deviceId: string) => MOCK_SCHEMA.sensors.filter((s) => s.deviceId === deviceId).map((s) => s.id);

beforeEach(() => {
  plots.length = 0;
  localStorage.clear();
  i18n.locale = 'en';
});
afterEach(cleanup);

function setup() {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.stats = Object.fromEntries(MOCK_SCHEMA.sensors.map((s) => [s.id, STATS]));
  backend.gpuProcesses = [GAME];
  const store = new LiveStore();
  store.applySchema(MOCK_SCHEMA);
  store.applySnapshot({ revision: 1, seq: 1, timestampMs: 2000, values: mockValues(1) });
  return { backend, store };
}

const kpiLabels = () => [...document.querySelectorAll('.kpi .label')].map((e) => e.textContent);
const kpiValues = () => [...document.querySelectorAll('.kpi .value')].map((e) => e.textContent);

test('gpu page: kpis, chart, sensor table, properties and processes', async () => {
  const { backend, store } = setup();
  render(DevicePage, { entry: GPU_ENTRY, store, backend });

  expect(kpiLabels()).toEqual(['load', 'temperature', 'power', 'vram'].map((id) => t(`advanced.kpi.${id}`)));
  const load = mockValues(1)[MOCK_SCHEMA.sensors.findIndex((s) => s.id === `${GPU}/load/core`)];
  expect(kpiValues()[0]).toBe(formatValue(load, 'percent', 'en', t));
  expect(screen.getByText(t('advanced.kpi.vramOf', { total: '16.0 GB' }))).toBeTruthy();

  await vi.waitFor(() => expect(backend.statsCalls[0]).toEqual(idsOf(GPU)));
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  expect(backend.historyCalls[0]).toEqual({ ids: [`${GPU}/load/core`, `${GPU}/temperature/core`], seconds: 300, maxPoints: undefined });
  expect(screen.getByRole('button', { name: t('advanced.table.reset') })).toBeTruthy();

  expect(screen.getByText(t('advanced.info.title'))).toBeTruthy();
  expect(screen.getByText('0000:01:00.0')).toBeTruthy();
  expect(backend.gpuProcessCalls[0]).toBe(GPU);
  await vi.waitFor(() => expect(screen.getByText('game.exe')).toBeTruthy());
  expect(screen.queryByRole('heading', { level: 2 })).toBeNull();
});

test('cpu page: peak load from the core statistics, no process list, no empty info box', async () => {
  const { backend, store } = setup();
  render(DevicePage, { entry: CPU_ENTRY, store, backend });

  expect(kpiLabels()).toEqual(['load', 'clock', 'busiestThread', 'peakLoad'].map((id) => t(`advanced.kpi.${id}`)));
  await vi.waitFor(() => expect(kpiValues()[3]).toBe(formatValue(STATS.max, 'percent', 'en', t)));
  expect(backend.statsCalls[0]).toEqual(idsOf('cpu/0'));
  expect(backend.gpuProcessCalls).toEqual([]);
  expect(screen.queryByText(t('advanced.processes.title'))).toBeNull();
  expect(screen.queryByText(t('advanced.info.title'))).toBeNull();
});

test('network page: traffic in bits per second, like the Simple view', async () => {
  const { backend, store } = setup();
  const NIC = 'network/mock-eth';
  const entry: SidebarEntry = { id: NIC, kind: 'network', deviceIds: [NIC], labelKey: 'advanced.section.network', labelArg: 'Ethernet' };
  render(DevicePage, { entry, store, backend });

  const down = mockValues(1)[MOCK_SCHEMA.sensors.findIndex((s) => s.id === `${NIC}/throughput/down`)];
  expect(kpiValues()[0]).toBe(formatValue(down, 'bytes_per_second', 'en', t, { rate: 'bits' }));
  expect(kpiValues()[0]).toMatch(/bit\/s$/);
  await vi.waitFor(() => expect(kpiValues()[3]).toBe(formatValue(STATS.max, 'bytes_per_second', 'en', t, { rate: 'bits' })));
  // The label also appears in the KPI row and in the series picker: take the table row.
  const name = [...document.querySelectorAll('.sensors th .name')].find((n) => n.textContent === t('sensor.network.down'))!;
  const cells = () => [...name.closest('tr')!.querySelectorAll('td')].map((td) => td.textContent!);
  await vi.waitFor(() => expect(cells().every((c) => c.endsWith('bit/s'))).toBe(true));
});

test('leaving the page stops the statistics polling and destroys the chart', async () => {
  vi.useFakeTimers();
  try {
    const { backend, store } = setup();
    const { unmount } = render(DevicePage, { entry: CPU_ENTRY, store, backend });
    await vi.advanceTimersByTimeAsync(2000);
    const calls = backend.statsCalls.length;
    expect(calls).toBeGreaterThanOrEqual(2);
    unmount();
    await vi.advanceTimersByTimeAsync(5000);
    expect(backend.statsCalls).toHaveLength(calls);
    expect(plots.every((p) => p.destroyed)).toBe(true);
  } finally {
    vi.useRealTimers();
  }
});

test('the advanced view mounts the full page under its heading', async () => {
  localStorage.setItem(SECTION_KEY, GPU);
  const { backend, store } = setup();
  render(AdvancedView, { store, backend });

  expect(screen.getByRole('heading', { level: 2 }).textContent).toBe(t('advanced.section.gpu'));
  expect(kpiLabels()).toHaveLength(4);
  await vi.waitFor(() => expect(plots).toHaveLength(1));
  await vi.waitFor(() => expect(screen.getByText('game.exe')).toBeTruthy());
});
```

`app/src/components/advanced/GpuProcesses.test.ts` (nuovo). Usa timer finti; `vi.spyOn` fa fallire solo la prima richiesta:

```ts
import { cleanup, render, screen } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { MOCK_SCHEMA } from '../../lib/backend/mock';
import { DASH } from '../../lib/format';
import { i18n, t } from '../../lib/i18n/index.svelte';
import type { GpuProcess } from '../../lib/types';
import { FakeBackend } from '../../test/fake-backend';
import GpuProcesses from './GpuProcesses.svelte';

const GPU = 'gpu/pci-0000:01:00.0';
const MIB = 1024 ** 2;
const GAME: GpuProcess = { pid: 4242, name: 'game.exe', loadPercent: 87.4, engine: '3D', dedicatedBytes: 3 * 1024 * MIB, sharedBytes: 120 * MIB };
const DWM: GpuProcess = { pid: 1480, name: 'dwm.exe', loadPercent: null, engine: null, dedicatedBytes: 250 * MIB, sharedBytes: null };

let visibility: DocumentVisibilityState = 'visible';
Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => visibility });
const setVisibility = (state: DocumentVisibilityState) => {
  visibility = state;
  document.dispatchEvent(new Event('visibilitychange'));
};

beforeEach(() => {
  i18n.locale = 'en';
  visibility = 'visible';
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

function setup(rows: GpuProcess[] = [GAME, DWM], failFirst = false) {
  const backend = new FakeBackend(MOCK_SCHEMA);
  backend.gpuProcesses = rows;
  if (failFirst) vi.spyOn(backend, 'getGpuProcesses').mockRejectedValueOnce(new Error('offline'));
  const view = render(GpuProcesses, { deviceId: GPU, backend });
  return { backend, ...view };
}

const cells = (name: string) => [...screen.getByText(name).closest('tr')!.querySelectorAll('td')].map((td) => td.textContent);

test('lists the processes of the gpu with load, engine and memory', async () => {
  const { backend } = setup();
  await vi.advanceTimersByTimeAsync(0);
  flushSync();
  expect(backend.gpuProcessCalls).toEqual([GPU]);
  expect(screen.getByText('4242')).toBeTruthy();
  expect(cells('game.exe')).toEqual(['87% · 3D', '3.0 GB', '120 MB']);
  expect(cells('dwm.exe')).toEqual([DASH, '250 MB', DASH]);
});

test('refreshes every 2 s only while visible', async () => {
  const { backend } = setup();
  await vi.advanceTimersByTimeAsync(0);
  await vi.advanceTimersByTimeAsync(4000);
  expect(backend.gpuProcessCalls).toHaveLength(3);
  setVisibility('hidden');
  await vi.advanceTimersByTimeAsync(6000);
  expect(backend.gpuProcessCalls).toHaveLength(3);
  setVisibility('visible');
  await vi.advanceTimersByTimeAsync(0);
  expect(backend.gpuProcessCalls).toHaveLength(4);
});

test('stops refreshing when unmounted', async () => {
  const { backend, unmount } = setup();
  await vi.advanceTimersByTimeAsync(0);
  unmount();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(backend.gpuProcessCalls).toHaveLength(1);
});

test('says so when no process uses the gpu', async () => {
  setup([]);
  await vi.advanceTimersByTimeAsync(0);
  flushSync();
  expect(screen.getByText(t('advanced.processes.empty'))).toBeTruthy();
});

test('a failed request is logged and the next one retries', async () => {
  const error = vi.spyOn(console, 'error').mockImplementation(() => {});
  setup([GAME, DWM], true);
  await vi.advanceTimersByTimeAsync(0);
  expect(screen.queryByText('game.exe')).toBeNull();
  expect(error).toHaveBeenCalledWith('GPU process list unavailable', expect.any(Error));
  await vi.advanceTimersByTimeAsync(2000);
  flushSync();
  expect(screen.getByText('game.exe')).toBeTruthy();
  error.mockRestore();
});
```

- [ ] **Step 2: Esegui i test e verifica che falliscano**

```bash
cd app && pnpm test
```

Risultato atteso: `Test Files  3 failed | 19 passed (22)`, `Tests  17 failed | 154 passed (171)`:
- `pages.test.ts`: 12 test falliscono con `TypeError: kpisFor is not a function` (e simili per `defaultSeries` e `propertyRows`);
- `DevicePage.test.ts`: 5 test falliscono, perché la pagina minima del Task 10 non ha KPI, grafico, tabella né processi;
- `GpuProcesses.test.ts` fallisce all'import: `Failed to resolve import "./GpuProcesses.svelte"`.

- [ ] **Step 3: Implementa KPI, serie predefinite e proprietà in `pages.ts`**

`app/src/lib/advanced/pages.ts`, file completo. La parte del Task 12 non cambia. `translateNow` è il `t` del modulo i18n, che legge la lingua corrente: il testo secondario della VRAM si ricalcola nel template, quindi segue il cambio di lingua.

```ts
import { formatBytes, formatPercent, formatValue, type FormatOptions } from '../format';
import { catalogs, i18n, t as translateNow, type Translate } from '../i18n/index.svelte';
import type { ValueOf } from '../select';
import type { Device, DeviceKind, Schema, Sensor, SensorStats, Source, Unit } from '../types';

/** Statistics since app start (or the last reset) of one sensor; null while unknown. */
export type StatsOf = (id: string) => SensorStats | null;

export interface SensorGroup {
  category: string;
  sensors: Sensor[];
}

/** Table order of the sensor categories (spec §7.3); unknown categories follow alphabetically. */
export const CATEGORY_ORDER = ['temperature', 'load', 'clock', 'power', 'percent', 'voltage', 'current', 'fan', 'data', 'throughput', 'link', 'energy', 'flag'];

/** Groups by `category`, keeping the schema order inside each group. */
export function groupSensors(sensors: Sensor[]): SensorGroup[] {
  const groups = new Map<string, Sensor[]>();
  for (const sensor of sensors) {
    const list = groups.get(sensor.category);
    if (list) list.push(sensor);
    else groups.set(sensor.category, [sensor]);
  }
  const rank = (category: string) => {
    const i = CATEGORY_ORDER.indexOf(category);
    return i < 0 ? CATEGORY_ORDER.length : i;
  };
  return [...groups.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
    .map(([category, list]) => ({ category, sensors: list }));
}

/** Group heading: `advanced.category.<name>` when translated, else the raw category. */
export function categoryLabel(category: string, t: Translate): string {
  const key = `advanced.category.${category}`;
  return key in catalogs.en ? t(key) : category;
}

/** Short badge text for a source, e.g. "NVML", "IP HELPER". */
export function sourceCode(source: Source): string {
  return source.replace(/_/g, ' ').toUpperCase();
}

/** Average column: for on/off flags it is the share of time the flag was on. */
export function formatAverage(
  stats: SensorStats | null,
  unit: Unit,
  locale: string,
  t: Translate,
  opts: FormatOptions = {},
): string {
  if (!stats) return formatValue(null, unit, locale, t, opts);
  return unit === 'boolean' ? formatPercent(stats.avg * 100, locale) : formatValue(stats.avg, unit, locale, t, opts);
}

/** One headline figure of a page (spec §7.3: 4 KPIs per component). */
export interface KpiDef {
  id: string;
  labelKey: string;
  value: (valueOf: ValueOf, stats: StatsOf) => number | null;
  unit: Unit;
  /** Optional small text under the value, already translated. */
  secondary?: (valueOf: ValueOf, stats: StatsOf) => string | null;
}

const KPI_COUNT = 4;

const sensorsWith = (schema: Schema, deviceIds: string[], key: string): Sensor[] =>
  schema.sensors.filter((s) => deviceIds.includes(s.deviceId) && s.label.key === key);

/** First sensor of the page with any of `keys`, in the order of the keys (fallbacks). */
function firstOf(schema: Schema, deviceIds: string[], ...keys: string[]): Sensor | undefined {
  for (const key of keys) {
    const found = sensorsWith(schema, deviceIds, key)[0];
    if (found) return found;
  }
  return undefined;
}

const kpi = (id: string, unit: Unit, value: KpiDef['value'], secondary?: KpiDef['secondary']): KpiDef => ({
  id,
  labelKey: `advanced.kpi.${id}`,
  unit,
  value,
  ...(secondary ? { secondary } : {}),
});

/** Live value of a sensor. */
const live = (id: string, sensor: Sensor | undefined): KpiDef | null =>
  sensor ? kpi(id, sensor.unit, (valueOf) => valueOf(sensor.id)) : null;

/** Highest value since start (or reset), from the core statistics. */
const peak = (id: string, sensor: Sensor | undefined): KpiDef | null =>
  sensor ? kpi(id, sensor.unit, (_valueOf, stats) => stats(sensor.id)?.max ?? null) : null;

function maxOf(valueOf: ValueOf, sensors: Sensor[]): number | null {
  const values = sensors.map((s) => valueOf(s.id)).filter((v): v is number => v !== null);
  return values.length ? Math.max(...values) : null;
}

function candidates(kind: DeviceKind, schema: Schema, ids: string[]): (KpiDef | null)[] {
  const find = (...keys: string[]) => firstOf(schema, ids, ...keys);
  switch (kind) {
    case 'cpu': {
      const threads = sensorsWith(schema, ids, 'cpu.load.thread');
      return [
        live('load', find('cpu.load.total')),
        live('clock', find('cpu.clock.effective')),
        threads.length ? kpi('busiestThread', threads[0].unit, (valueOf) => maxOf(valueOf, threads)) : null,
        peak('peakLoad', find('cpu.load.total')),
      ];
    }
    case 'gpu': {
      const used = find('gpu.memory.dedicatedUsed');
      const total = find('gpu.memory.dedicatedTotal');
      return [
        live('load', find('gpu.load.core')),
        live('temperature', find('gpu.temperature.core', 'gpu.temperature.hotspot')),
        live('power', find('gpu.power.board', 'gpu.power.limitPercent')),
        used
          ? kpi(
              'vram',
              used.unit,
              (valueOf) => valueOf(used.id),
              total
                ? (valueOf) => {
                    const bytes = valueOf(total.id);
                    return bytes === null ? null : translateNow('advanced.kpi.vramOf', { total: formatBytes(bytes, i18n.locale) });
                  }
                : undefined,
            )
          : null,
        live('clock', find('gpu.clock.core')),
      ];
    }
    case 'memory': {
      const used = find('memory.used');
      const total = find('memory.total');
      return [
        live('load', find('memory.load')),
        live('used', used),
        live('total', total),
        used && total
          ? kpi('available', total.unit, (valueOf) => {
              const all = valueOf(total.id);
              const inUse = valueOf(used.id);
              return all === null || inUse === null ? null : Math.max(0, all - inUse);
            })
          : null,
      ];
    }
    case 'storage': {
      const free = find('storage.volumeFree');
      return [
        live('active', find('storage.active')),
        live('read', find('storage.read')),
        live('write', find('storage.write')),
        live('temperature', find('storage.temperature')),
        free ? kpi('freeSpace', free.unit, (valueOf) => valueOf(free.id), () => free.label.arg ?? null) : null,
      ];
    }
    case 'network':
      return [
        live('down', find('network.down')),
        live('up', find('network.up')),
        live('linkSpeed', find('network.linkSpeed')),
        peak('peakDown', find('network.down')),
      ];
    default:
      return [];
  }
}

/** The KPIs of a page: the first 4 of its kind whose sensors exist, in definition order. */
export function kpisFor(kind: DeviceKind, schema: Schema, deviceIds: string[]): KpiDef[] {
  return candidates(kind, schema, deviceIds)
    .filter((k): k is KpiDef => k !== null)
    .slice(0, KPI_COUNT);
}

const DEFAULT_SERIES: Partial<Record<DeviceKind, string[][]>> = {
  cpu: [['cpu.load.total'], ['cpu.clock.effective']],
  gpu: [['gpu.load.core'], ['gpu.temperature.core', 'gpu.temperature.hotspot']],
  memory: [['memory.load']],
  storage: [['storage.read'], ['storage.write']],
  network: [['network.down'], ['network.up']],
};

/** Series charted when the user has not chosen any; the first sensor for other kinds. */
export function defaultSeries(kind: DeviceKind, schema: Schema, deviceIds: string[]): string[] {
  const ids = (DEFAULT_SERIES[kind] ?? [])
    .map((keys) => firstOf(schema, deviceIds, ...keys)?.id)
    .filter((id): id is string => id !== undefined);
  if (ids.length > 0) return ids;
  const first = schema.sensors.find((s) => deviceIds.includes(s.deviceId));
  return first ? [first.id] : [];
}

/** Display order of the static device properties; unknown keys follow alphabetically. */
export const PROPERTY_ORDER = [
  'pciAddress',
  'integrated',
  'pcieMaxGen',
  'pcieMaxWidth',
  'powerLimitDefaultW',
  'powerLimitMinW',
  'powerLimitMaxW',
  'tempSlowdownC',
  'tempShutdownC',
  'tempMaxC',
  'tempWarningC',
  'tempCriticalC',
];

/**
 * How numeric properties are shown. The labels of the W and °C limits already carry the
 * unit (Task 9), so those values are plain numbers; the PCIe ones read "Gen 4" and "x16".
 */
const PROPERTY_FORMATS: Record<string, Unit | 'number'> = {
  pcieMaxGen: 'pcie_generation',
  pcieMaxWidth: 'lanes',
  powerLimitDefaultW: 'number',
  powerLimitMinW: 'number',
  powerLimitMaxW: 'number',
  tempSlowdownC: 'number',
  tempShutdownC: 'number',
  tempMaxC: 'number',
  tempWarningC: 'number',
  tempCriticalC: 'number',
};

export interface PropertyRow {
  key: string;
  label: string;
  value: string;
}

/** Device properties as translated label/value rows (`property.<key>`), in display order. */
export function propertyRows(device: Device, locale: string, t: Translate): PropertyRow[] {
  const rank = (key: string) => {
    const i = PROPERTY_ORDER.indexOf(key);
    return i < 0 ? PROPERTY_ORDER.length : i;
  };
  return Object.entries(device.properties ?? {})
    .sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b))
    .map(([key, raw]) => {
      const labelKey = `property.${key}`;
      const label = labelKey in catalogs.en ? t(labelKey) : key;
      const format = PROPERTY_FORMATS[key];
      const number = Number(raw);
      let value = raw;
      if (key === 'integrated') value = t(raw === 'true' ? 'advanced.info.yes' : 'advanced.info.no');
      else if (format && raw.trim() !== '' && Number.isFinite(number)) {
        value = format === 'number' ? new Intl.NumberFormat(locale).format(number) : formatValue(number, format, locale, t);
      }
      return { key, label, value };
    });
}
```

- [ ] **Step 4: Implementa i componenti e le traduzioni**

`app/src/components/advanced/KpiRow.svelte` (nuovo):

```svelte
<script lang="ts">
  import type { KpiDef, StatsOf } from '../../lib/advanced/pages';
  import { formatValue } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { ValueOf } from '../../lib/select';
  import AnimatedNumber from '../common/AnimatedNumber.svelte';

  let {
    kpis,
    valueOf,
    statsOf,
    rate = 'bytes',
  }: {
    kpis: KpiDef[];
    valueOf: ValueOf;
    statsOf: StatsOf;
    /** 'bits' on network pages: traffic in bit/s, like the Simple view. */
    rate?: 'bits' | 'bytes';
  } = $props();
  const locale = $derived(i18n.locale);
</script>

<div class="kpis">
  {#each kpis as kpi (kpi.id)}
    {@const secondary = kpi.secondary?.(valueOf, statsOf) ?? null}
    <div class="kpi">
      <div class="label">{t(kpi.labelKey)}</div>
      <div class="value">
        <AnimatedNumber value={kpi.value(valueOf, statsOf)} format={(v) => formatValue(v, kpi.unit, locale, t, { rate })} />
      </div>
      {#if secondary}<div class="secondary">{secondary}</div>{/if}
    </div>
  {/each}
</div>

<style>
  .kpis {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: 12px;
  }
  .kpi {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    padding: 12px 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .value {
    font-size: 24px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .secondary {
    font-size: 12px;
    color: var(--text-muted);
  }
</style>
```

`app/src/components/advanced/DeviceInfo.svelte` (nuovo):

```svelte
<script lang="ts">
  import { propertyRows } from '../../lib/advanced/pages';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { Device } from '../../lib/types';

  let { devices }: { devices: Device[] } = $props();
</script>

<section class="info">
  <h3 class="label">{t('advanced.info.title')}</h3>
  {#each devices as device (device.id)}
    <dl>
      {#each propertyRows(device, i18n.locale, t) as row (row.key)}
        <dt>{row.label}</dt>
        <dd>{row.value}</dd>
      {/each}
    </dl>
  {/each}
</section>

<style>
  .info {
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  h3 {
    margin: 0 0 8px;
    font-weight: 400;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    margin: 0;
    font-size: 13px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
</style>
```

`app/src/components/advanced/GpuProcesses.svelte` (nuovo):

```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Backend } from '../../lib/backend/backend';
  import { DASH, formatBytes, formatPercent } from '../../lib/format';
  import { i18n, t } from '../../lib/i18n/index.svelte';
  import type { GpuProcess } from '../../lib/types';

  let { deviceId, backend }: { deviceId: string; backend: Backend } = $props();

  const INTERVAL_MS = 2000;
  const locale = $derived(i18n.locale);
  let rows = $state.raw<GpuProcess[] | null>(null);
  let inFlight = false;

  async function refresh() {
    if (document.visibilityState === 'hidden' || inFlight) return;
    inFlight = true;
    try {
      rows = await backend.getGpuProcesses(deviceId);
    } catch (error) {
      console.error('GPU process list unavailable', error);
    } finally {
      inFlight = false;
    }
  }

  function loadText(p: GpuProcess): string {
    if (p.loadPercent === null) return DASH;
    const load = formatPercent(p.loadPercent, locale);
    return p.engine ? `${load} · ${p.engine}` : load;
  }

  onMount(() => {
    const onVisibility = () => {
      if (document.visibilityState === 'visible') void refresh();
    };
    document.addEventListener('visibilitychange', onVisibility);
    const timer = setInterval(() => void refresh(), INTERVAL_MS);
    void refresh();
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  });
</script>

<section class="processes">
  <h3 class="label">{t('advanced.processes.title')}</h3>
  {#if rows !== null && rows.length === 0}
    <p class="empty">{t('advanced.processes.empty')}</p>
  {:else if rows !== null}
    <table>
      <thead>
        <tr>
          <th scope="col">{t('advanced.processes.name')}</th>
          <th scope="col" class="num">{t('advanced.processes.load')}</th>
          <th scope="col" class="num">{t('advanced.processes.dedicated')}</th>
          <th scope="col" class="num">{t('advanced.processes.shared')}</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as p (p.pid)}
          <tr>
            <th scope="row">{p.name} <span class="pid">{p.pid}</span></th>
            <td class="num">{loadText(p)}</td>
            <td class="num">{formatBytes(p.dedicatedBytes, locale)}</td>
            <td class="num">{formatBytes(p.sharedBytes, locale)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  .processes {
    /* The table (nowrap numbers, long engine names) may be wider than its grid cell:
       it scrolls inside the box instead of widening the whole page. */
    min-width: 0;
    overflow-x: auto;
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  h3 {
    margin: 0 0 8px;
    font-weight: 400;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th,
  td {
    padding: 4px 8px;
    text-align: left;
    font-weight: 400;
    border-bottom: 1px solid var(--border);
  }
  thead th {
    color: var(--text-muted);
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .pid {
    margin-left: 4px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .empty {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
```

`app/src/components/advanced/DevicePage.svelte`, file completo: sostituisce la pagina minima del Task 10, con le stesse props. `untrack` evita l'avviso `state_referenced_locally`: la pagina viene ricreata a ogni cambio di sezione, quindi il `backend` letto una volta resta valido.

```svelte
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { SidebarEntry } from '../../lib/advanced/nav';
  import { defaultSeries, kpisFor } from '../../lib/advanced/pages';
  import { StatsPoller } from '../../lib/advanced/statsPoller.svelte';
  import type { Backend } from '../../lib/backend';
  import type { LiveStore } from '../../lib/live.svelte';
  import DeviceInfo from './DeviceInfo.svelte';
  import GpuProcesses from './GpuProcesses.svelte';
  import HistoryChart from './HistoryChart.svelte';
  import KpiRow from './KpiRow.svelte';
  import SensorTable from './SensorTable.svelte';

  // One Advanced page (spec §7.3). AdvancedView renders the heading and re-keys this
  // component per section, so timers and polling start from scratch on every page.
  let { entry, store, backend }: { entry: SidebarEntry; store: LiveStore; backend: Backend } = $props();

  const schema = $derived(store.schema);
  const devices = $derived(schema?.devices.filter((d) => entry.deviceIds.includes(d.id)) ?? []);
  const sensors = $derived(schema?.sensors.filter((s) => entry.deviceIds.includes(s.deviceId)) ?? []);
  const kpis = $derived(schema ? kpisFor(entry.kind, schema, entry.deviceIds) : []);
  const defaults = $derived(schema ? defaultSeries(entry.kind, schema, entry.deviceIds) : []);
  const hasProperties = $derived(devices.some((d) => Object.keys(d.properties ?? {}).length > 0));
  // Network traffic in bit/s, the unit of the Simple view's network tile.
  const rate = $derived(entry.kind === 'network' ? 'bits' : 'bytes');
  const valueOf = (id: string) => store.value(id);
  // The backend of a mounted page never changes.
  const stats = new StatsPoller(untrack(() => backend), () => sensors.map((s) => s.id), () => schema?.revision ?? null);

  onMount(() => stats.start());
</script>

{#if schema}
  <div class="page">
    <KpiRow {kpis} {valueOf} statsOf={stats.statsOf} {rate} />
    <HistoryChart sectionId={entry.id} {sensors} {defaults} {schema} {store} {backend} />
    <SensorTable {sensors} {valueOf} {stats} {rate} />
    {#if hasProperties || entry.kind === 'gpu'}
      <div class="extra">
        {#if hasProperties}<DeviceInfo {devices} />{/if}
        {#if entry.kind === 'gpu'}<GpuProcesses deviceId={entry.deviceIds[0]} {backend} />{/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  .extra {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 14px;
    align-items: start;
  }
</style>
```

`app/src/lib/i18n/en.json`: inserisci subito dopo la riga `"advanced.category.flag": "Status flags",` (Task 12):

```json
  "advanced.kpi.load": "Load",
  "advanced.kpi.clock": "Clock",
  "advanced.kpi.busiestThread": "Busiest thread",
  "advanced.kpi.peakLoad": "Peak load",
  "advanced.kpi.temperature": "Temperature",
  "advanced.kpi.power": "Power",
  "advanced.kpi.vram": "VRAM used",
  "advanced.kpi.vramOf": "of {total}",
  "advanced.kpi.used": "In use",
  "advanced.kpi.total": "Total",
  "advanced.kpi.available": "Available",
  "advanced.kpi.active": "Active time",
  "advanced.kpi.read": "Read",
  "advanced.kpi.write": "Write",
  "advanced.kpi.freeSpace": "Free space",
  "advanced.kpi.down": "Download",
  "advanced.kpi.up": "Upload",
  "advanced.kpi.linkSpeed": "Link speed",
  "advanced.kpi.peakDown": "Peak download",
  "advanced.processes.title": "Processes using this GPU",
  "advanced.processes.name": "Process",
  "advanced.processes.load": "Load",
  "advanced.processes.dedicated": "Dedicated memory",
  "advanced.processes.shared": "Shared memory",
  "advanced.processes.empty": "No process is using this GPU.",
  "advanced.info.title": "Device properties",
  "advanced.info.yes": "Yes",
  "advanced.info.no": "No",
```

`app/src/lib/i18n/it.json`: inserisci subito dopo la riga `"advanced.category.flag": "Indicatori di stato",`:

```json
  "advanced.kpi.load": "Carico",
  "advanced.kpi.clock": "Clock",
  "advanced.kpi.busiestThread": "Thread più carico",
  "advanced.kpi.peakLoad": "Carico massimo",
  "advanced.kpi.temperature": "Temperatura",
  "advanced.kpi.power": "Potenza",
  "advanced.kpi.vram": "VRAM usata",
  "advanced.kpi.vramOf": "di {total}",
  "advanced.kpi.used": "In uso",
  "advanced.kpi.total": "Totale",
  "advanced.kpi.available": "Disponibile",
  "advanced.kpi.active": "Tempo attivo",
  "advanced.kpi.read": "Lettura",
  "advanced.kpi.write": "Scrittura",
  "advanced.kpi.freeSpace": "Spazio libero",
  "advanced.kpi.down": "Download",
  "advanced.kpi.up": "Upload",
  "advanced.kpi.linkSpeed": "Velocità del collegamento",
  "advanced.kpi.peakDown": "Download massimo",
  "advanced.processes.title": "Processi che usano questa GPU",
  "advanced.processes.name": "Processo",
  "advanced.processes.load": "Carico",
  "advanced.processes.dedicated": "Memoria dedicata",
  "advanced.processes.shared": "Memoria condivisa",
  "advanced.processes.empty": "Nessun processo sta usando questa GPU.",
  "advanced.info.title": "Proprietà del dispositivo",
  "advanced.info.yes": "Sì",
  "advanced.info.no": "No",
```

- [ ] **Step 5: Esegui i test e verifica che passino**

```bash
cd app
pnpm test
pnpm check
pnpm build
```

Risultato atteso:
- `pnpm test`: `Test Files  22 passed (22)`, `Tests  176 passed (176)` (17 in `pages.test.ts`, 5 in `DevicePage.test.ts`, 5 in `GpuProcesses.test.ts`). Anche `AdvancedView.test.ts` e `App.test.ts` del Task 10 passano senza modifiche: ora montano la pagina completa, con `FakeUplot` al posto di uPlot;
- `pnpm check`: `0 ERRORS 0 WARNINGS`;
- `pnpm build`: OK. Il JS passa da circa 79 kB (28 kB gzip) a circa 153 kB (58 kB gzip): uPlot e le pagine, come stimato dallo spike. Il CSS, con `uPlot.min.css`, sale a circa 12,5 kB.

- [ ] **Step 6: Verifica nel browser con il backend mock (CDP, senza input sul desktop)**

1. In un terminale a parte avvia `cd app && pnpm dev` e attendi `Local: http://localhost:1420/`.
2. Salva questo script come `$env:TEMP\oma-pages-cdp.mjs`. È un file di verifica usa e getta, da non aggiungere al repository. Legge ogni pagina della vista Avanzata tramite CDP: i clic sono eventi DOM eseguiti dal JS della pagina, non input sul desktop. Alla fine rimette `localStorage` com'era.

   ```js
   // Reads every Advanced page through CDP: page JS only (DOM clicks inside the page), no
   // desktop input. localStorage is restored at the end, so the app keeps its state.
   // Usage: node oma-pages-cdp.mjs <cdp-port> [url]
   //   headless Edge on `pnpm dev`: node oma-pages-cdp.mjs 9333 http://localhost:1420/
   //   the real app (WebView2):     node oma-pages-cdp.mjs 9222
   const port = Number(process.argv[2] ?? 9333);
   const url = process.argv[3] ?? null;
   const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
   const errors = [];

   async function connect(wsUrl) {
     const ws = new WebSocket(wsUrl);
     await new Promise((r) => ws.addEventListener('open', r));
     let id = 0;
     const pending = new Map();
     ws.addEventListener('message', (e) => {
       const msg = JSON.parse(e.data);
       if (msg.id && pending.has(msg.id)) pending.get(msg.id)(msg);
       if (msg.method === 'Runtime.exceptionThrown') errors.push(msg.params.exceptionDetails.text);
       if (msg.method === 'Runtime.consoleAPICalled' && msg.params.type === 'error') {
         errors.push(msg.params.args.map((a) => a.value ?? a.description).join(' '));
       }
     });
     const send = (method, params = {}) =>
       new Promise((r) => {
         const n = ++id;
         pending.set(n, r);
         ws.send(JSON.stringify({ id: n, method, params }));
       });
     return { ws, send };
   }

   let targets = [];
   for (let i = 0; i < 60 && !targets.some((t) => t.type === 'page'); i++) {
     try {
       targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
     } catch {
       await sleep(500);
     }
   }
   const page = await connect(targets.find((t) => t.type === 'page').webSocketDebuggerUrl);
   await page.send('Runtime.enable');
   const run = async (expression) =>
     (await page.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.result.value;
   const texts = (selector) => `[...document.querySelectorAll('${selector}')].map((e) => e.textContent.replace(/\\s+/g, ' ').trim())`;

   if (url) {
     await page.send('Page.navigate', { url });
     await sleep(2500);
   }
   const KEYS = ['oma.view', 'oma.advanced.section', 'oma.advanced.window'];
   const saved = await run(`JSON.stringify(${JSON.stringify(KEYS)}.map((k) => localStorage.getItem(k)))`);
   await run(`localStorage.setItem('oma.view', 'advanced'); localStorage.removeItem('oma.advanced.window'); location.reload(); true`);
   await sleep(3500);

   const count = await run(`document.querySelectorAll('nav button').length`);
   for (let i = 0; i < count; i++) {
     await run(`document.querySelectorAll('nav button')[${i}].click(); true`);
     await sleep(2500);
     console.log(`== ${await run(`document.querySelector('h2')?.textContent + ' / ' + (document.querySelector('.device')?.textContent ?? '')`)}`);
     console.log('  KPIS:', await run(`${texts('.kpi')}.join(' | ')`));
     console.log('  CHART:', await run(`document.querySelectorAll('.plot canvas').length + ' canvas; ' + ${texts('.u-legend .u-series:not(:first-child) .u-label')}.join(', ')`));
     console.log('  GROUPS:', await run(`${texts('.sensors tr.group th')}.join(' | ')`));
     console.log('  FIRST ROW:', await run(`document.querySelector('.sensors tbody tr:not(.group)')?.textContent.replace(/\\s+/g, ' ').trim()`));
     console.log('  INFO:', await run(`(() => { const v = ${texts('.info dd')}; return ${texts('.info dt')}.map((d, k) => d + ' = ' + v[k]).join(' | '); })()`));
     console.log('  PROCESSES:', await run(`${texts('.processes tbody th')}.slice(0, 5).join(' | ') || (document.querySelector('.processes .empty')?.textContent ?? '-')`));
   }
   await run(`[...document.querySelectorAll('.windows button')].find((b) => b.textContent.trim() === '1h')?.click(); true`);
   await sleep(2000);
   console.log('AFTER 1h:', await run(`document.querySelector('.windows [aria-pressed=true]')?.textContent.trim() + ', saved ' + localStorage.getItem('oma.advanced.window') + ', ' + document.querySelectorAll('.plot canvas').length + ' canvas'`));
   console.log('CONSOLE ERRORS:', errors.length ? errors.join(' || ') : 'none');

   await run(`${JSON.stringify(KEYS)}.forEach((k, i) => { const v = ${saved}[i]; if (v === null) localStorage.removeItem(k); else localStorage.setItem(k, v); }); location.reload(); true`);
   await sleep(500);
   page.ws.close();
   if (url) {
     const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
     const browser = await connect(version.webSocketDebuggerUrl);
     await browser.send('Browser.close');
   }
   process.exit(0);
   ```
3. In PowerShell:
   ```powershell
   $edgeProfile = Join-Path $env:TEMP 'oma-cdp-profile'
   Remove-Item -Recurse -Force $edgeProfile -ErrorAction SilentlyContinue
   Start-Process "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe" -ArgumentList '--headless=new', '--disable-gpu', '--lang=en-US', "--user-data-dir=$edgeProfile", '--remote-debugging-port=9333', 'about:blank'
   node "$env:TEMP\oma-pages-cdp.mjs" 9333 http://localhost:1420/
   ```
   Risultato atteso (verificato durante la stesura; i numeri cambiano a ogni esecuzione):
   ```
   == CPU / Mock Ryzen 7 7800X3D
     KPIS: Load 58% | Clock 4.53 GHz | Busiest thread 76% | Peak load 58%
     CHART: 1 canvas; Total load, Estimated clock
     GROUPS: Load | Clocks
     FIRST ROW: Total load MOCK58%48%58%53%
     INFO:
     PROCESSES: -
   == GPU / Mock GeForce RTX 4080
     KPIS: Load 84% | Temperature 65 °C | Power 268 W | VRAM used 5.6 GB of 16.0 GB
     CHART: 1 canvas; GPU load, Core temperature
     GROUPS: Temperatures | Load | Clocks | Power | Data
     FIRST ROW: Core temperature MOCK65 °C59 °C64 °C62 °C
     INFO: PCI address = 0000:01:00.0 | Integrated GPU = No
     PROCESSES: game.exe 9412 | obs64.exe 10764 | dwm.exe 1188 | explorer.exe 6020 | System 4
   == RAM /
     KPIS: Load 57% | In use 18.1 GB | Total 32.0 GB | Available 13.9 GB
     CHART: 1 canvas; Memory load
     GROUPS: Load | Data
     FIRST ROW: Memory load MOCK57%55%57%56%
     INFO:
     PROCESSES: -
   == Disk / Disk 0 (C:)
     KPIS: Active time 45% | Read 86.7 MB/s | Write 22.9 MB/s | Free space 700 GB C:
     CHART: 1 canvas; Read rate, Write rate
     GROUPS: Load | Percentages | Data | Throughput
     FIRST ROW: Active time MOCK45%36%60%52%
     INFO:
     PROCESSES: -
   == Network / Ethernet
     KPIS: Download 9.6 Mbit/s | Upload 356 kbit/s | Link speed 1.0 Gbit/s | Peak download 48 Mbit/s
     CHART: 1 canvas; Download, Upload
     GROUPS: Throughput
     FIRST ROW: Download MOCK5.8 Mbit/s10 Mbit/s48 Mbit/s35 Mbit/s
     INFO:
     PROCESSES: -
   AFTER 1h: 1h, saved 3600, 1 canvas
   CONSOLE ERRORS: none
   ```
   In `FIRST ROW` le quattro celle dei valori sono attaccate, perché lo script legge il `textContent` della riga: attuale, min, max, media. `MOCK` è il badge della fonte, presente nel DOM anche quando è invisibile.
4. Lo script chiude Edge con `Browser.close`. Controlla che non restino processi: `Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" | Where-Object CommandLine -like '*oma-cdp-profile*'` non deve restituire nulla. Poi chiudi `pnpm dev` con Ctrl+C.

- [ ] **Step 7: Verifica nell'app vera (CDP su WebView2, senza input sul desktop)**

Richiede i Task 1–8: i comandi `get_stats`, `get_session` e `get_gpu_processes`, la temperatura dei dischi e le proprietà delle GPU. La porta di debug di WebView2 si apre con `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, come nello spike di uPlot. Lo script legge le pagine e rimette `localStorage` com'era.

L'utente può avere in esecuzione `target\release\oma-app.exe`: la verifica **non lo chiude e non lo riavvia**. Usa l'isolamento del Task 3 (Step 6): cartella target separata, identificatore `io.github.openmonitoradvanced.verify` (istanza singola e profilo WebView2 separati) e `LOCALAPPDATA` reindirizzata per log e crash marker. Dalla radice del repository, in un'unica sessione PowerShell:

```powershell
$verify = "$env:TEMP\oma-m3-verify"
New-Item -ItemType Directory -Force "$verify\localappdata" | Out-Null
Set-Content "$verify\tauri.verify.json" '{"identifier":"io.github.openmonitoradvanced.verify"}'
$env:CARGO_TARGET_DIR = "$verify\target"
Push-Location app
pnpm tauri build --no-bundle --config "$verify\tauri.verify.json"
Pop-Location
Remove-Item Env:CARGO_TARGET_DIR
$realLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = "$verify\localappdata"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
$app = Start-Process "$verify\target\release\oma-app.exe" -PassThru
Start-Sleep -Seconds 5
node "$env:TEMP\oma-pages-cdp.mjs" 9222
Stop-Process -Id $app.Id -Confirm:$false
$env:LOCALAPPDATA = $realLocalAppData
Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
```

La WebView2 segue la lingua di Windows: su questa macchina (Windows in italiano) le etichette escono in italiano. Risultato atteso su questa macchina (i numeri variano):
- **Barra laterale.** Una sezione `==` per ogni voce: CPU, "NVIDIA GeForce RTX 4080", "AMD Radeon(TM) Graphics", RAM, quattro dischi, le schede di rete.
- **RTX 4080:**
  - `KPIS: Carico …% | Temperatura … °C | Potenza … W | VRAM usata … GB di 16,0 GB`;
  - `CHART: 1 canvas; Carico GPU, Temperatura core`;
  - `GROUPS: Temperature | Carico | Clock | Potenza | Percentuali | Tensioni | Ventole | Dati | Collegamento | Indicatori di stato`;
  - `FIRST ROW` è la temperatura core, con badge `NVML` e min/max/media valorizzati;
  - `INFO: Indirizzo PCI = 0000:01:00.0 | GPU integrata = No | Generazione PCIe massima = Gen 4 | Larghezza massima del link PCIe = x16 | Limite di potenza predefinito (W) = 320 | Limite di potenza minimo (W) = 150 | Limite di potenza massimo (W) = 370 | Temperatura di rallentamento (°C) = 94 | Temperatura di spegnimento (°C) = 99 | Temperatura massima della GPU (°C) = 90`;
  - `PROCESSES` elenca, per carico decrescente, tra gli altri `dwm.exe` e `msedgewebview2.exe`, cioè la WebView2 dell'app stessa.
- **iGPU AMD:** `INFO: Indirizzo PCI = 0000:11:00.0 | GPU integrata = Sì | Generazione PCIe massima = Gen 4 | Larghezza massima del link PCIe = x16` (da PnP); solo i KPI dei sensori disponibili (`Carico | Temperatura | VRAM usata | Clock`).
- **Dischi.** Gli NVMe e l'HDD hanno `Temperatura` come quarto KPI e le soglie `Temperatura di avviso (°C)` / `Temperatura critica (°C)` in `INFO`, se il disco le riporta (l'HDD riporta solo quella di avviso, 60). L'SSD SATA, senza sensore, mostra invece `Spazio libero`.
- **CPU e rete** come nel mock: `Carico | Clock | Thread più carico | Carico massimo` e `Download | Upload | Velocità del collegamento | Download massimo`, con il traffico in `kbit/s`/`Mbit/s`.
- In fondo: `AFTER 1h: 1h, saved 3600, 1 canvas` e `CONSOLE ERRORS: none`.

L'istanza dell'utente, se c'era, è ancora in esecuzione: `Get-Process oma-app` mostra il suo `Path` in `target\release`. Il budget di memoria con la pagina GPU aperta e il grafico a 1h si misura nel Task 14.

- [ ] **Step 8: Verifica manuale (utente)**

1. **Browser.** Con `pnpm dev` attivo apri `http://localhost:1420`, vista Avanzata:
   - passando sopra una riga della tabella compare il badge della fonte; tenendo fermo il puntatore sul badge si legge la descrizione ("Simulated data");
   - "Azzera min/max" fa passare min, max e media a "—"; entro 1–2 s tornano valori, perché il mock riparte da capo; il carico massimo della CPU si azzera insieme alla tabella;
   - nel selettore delle serie, con 8 serie scelte le altre caselle sono disabilitate; con due unità scelte sono disabilitate quelle di una terza unità;
   - con 1h la curva copre tutta la storia del mock e la legenda mostra i valori con l'unità;
   - a 900 px di larghezza barra laterale, KPI, grafico e tabella stanno senza scorrimento orizzontale;
   - con la lingua del browser in italiano, KPI, colonne e gruppi sono tradotti.
2. **App vera.**
   - Sulla pagina della RTX 4080 avvia un video o un gioco: entro circa 2 s il processo compare in cima alla lista, con carico e motore (per esempio "· 3D" o "· VideoDecode").
   - Riduci a icona la finestra per circa 30 s e ripristinala: il grafico riprende senza buchi, perché lo storico viene ricaricato.
   - "Azzera min/max" sulla pagina GPU non tocca le statistiche della CPU: il carico massimo della pagina CPU resta invariato.

- [ ] **Step 9: Commit**

```bash
git add app/src
git commit -m "feat(ui): Advanced device pages with KPIs, history chart, sensor table, properties and GPU processes"
```

---

### Task 14: misura del budget M3, emendamenti alla spec, README e seguiti

**File:**
- Crea:
  - `scripts/seed-advanced-view.ps1` (prepara la misura in finestra: apre la vista Avanzata sulla pagina GPU con il grafico a 1 h, via CDP)
  - `docs/follow-ups.md` (seguiti rimasti aperti dopo M1, M2 e M3)
- Modifica:
  - `scripts/measure-footprint.ps1` (opzione `-FillHistoryMinutes`: misura con lo storico di 1 ora pieno)
  - `docs/perf-budget.md` (righe M3 e dettagli)
  - `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md` (§3, §4.2, §5.1, §5.2, §7.3, §8, §13.6)
  - `README.md` (stato, sezione "Advanced view")

**Interfacce:**
- Usa:
  - tutto quanto prodotto dai Task 1–13;
  - lo stato della vista Avanzata in `localStorage` (Task 10 e 11): `oma.view` = `advanced`, `oma.advanced.section` = id della sezione (per una GPU è l'id del device), `oma.advanced.window` = `60` | `300` | `1800` | `3600`, `oma.advanced.series.<sectionId>` = array JSON di id di sensori;
  - la RTX 4080 di questa macchina: device `gpu/pci-0000:01:00.0`, nome `NVIDIA GeForce RTX 4080`;
  - l'istanza singola di M1: un secondo avvio dell'eseguibile chiede all'istanza in esecuzione di aprire la finestra (`window::show_main`) ed esce;
  - la radice del grafico uPlot è un elemento con classe `uplot`; la legenda, se visibile, ha una riga `.u-series` per l'asse x più una per serie;
  - `scripts/measure-footprint.ps1` della M2.
- Produce:
  - i risultati del budget M3 (finestra aperta sulla vista Avanzata, pagina GPU, grafico a 1 h con 8 serie; tray), sia a pochi secondi dall'avvio sia con lo storico di 1 ora pieno;
  - la spec allineata a quanto fatto in M3, le esclusioni D10 comprese;
  - un README con la vista Avanzata;
  - `docs/follow-ups.md`.

**Come si apre la vista Avanzata senza input sintetico.** La finestra da misurare deve mostrare la vista Avanzata sulla pagina della RTX 4080 con il grafico a 1 h. Niente clic simulati né UI Automation: l'utente lavora su questo PC. Lo stato della vista vive nel `localStorage` del profilo WebView2 (`%LOCALAPPDATA%\io.github.openmonitoradvanced\EBWebView`) e sopravvive alla chiusura dell'app. Il nuovo script `seed-advanced-view.ps1`:
1. avvia l'app con la porta DevTools di WebView2 (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`, impostata solo per quel processo);
2. scrive le chiavi con `Runtime.evaluate` del Chrome DevTools Protocol e ricarica la pagina;
3. chiude la finestra con `WM_CLOSE` (`Process.CloseMainWindow`): WebView2 si chiude normalmente e scrive il `localStorage` su disco. Poi termina il processo, che era rimasto nella tray;
4. riavvia l'app, controlla che si apra sullo stato scritto (chiavi, grafico presente, testo atteso nella pagina), salva uno screenshot (`Page.captureScreenshot`) e chiude di nuovo.

Le misure partono poi senza porta DevTools, quindi il processo misurato è identico a quello dell'utente, e l'app si apre sulla pagina preparata. Il profilo è quello dell'istanza dell'utente: lo Step 1 ne salva il `localStorage` e lo Step 8 lo ripristina. La procedura è stata provata durante la stesura del piano, su questa macchina, con una build di M2 (identificatore diverso, per non toccare l'istanza dell'utente): scrittura, chiusura, riavvio e rilettura in 62 s, stato conservato, screenshot corretto; anche `-CheckOnly` e le misure successive lo hanno ritrovato. La porta resta aperta solo nei due avvii di preparazione. Se CDP non fosse disponibile, lo Step 5 descrive il ripiego con un passaggio manuale dell'utente.

**Serie del grafico.** La misura usa il caso peggiore permesso da D4: 8 serie e 2 unità. Serie: `load/core`, `load/3d`, `load/copy`, `load/video-decode`, `load/video-encode`, `fan/percent`, `percent/power-limit` (%) e `temperature/core` (°C) della RTX 4080, tutte presenti su questa macchina dalla M2 (PDH e NVML).

**Storico pieno.** A pochi secondi dall'avvio il grafico a 1 h ha solo qualche decina di punti. Con `-FillHistoryMinutes 61` lo script avvia l'app nella tray, aspetta 61 minuti, misura la tray, poi apre la finestra con un secondo avvio (istanza singola) e misura la finestra: il nucleo ha 3600 campioni per sensore e il grafico parte da un seed decimato di al massimo 900 punti. Questa prova non misura la memoria della UI rimasta aperta a lungo: il Task 11 aggiunge una coda di campioni live non decimati, che può raggiungere circa 3600 punti per serie. Una misura separata tiene perciò la pagina visibile per 61 minuti prima del campionamento. Lo spike 8 × 3600 è solo un riferimento, non sostituisce questa misura della build finale. Le righe senza storico pieno restano per il confronto con M1 e M2.

- [ ] **Step 1: Prerequisiti — verifica manuale (utente)**

Chiedi all'utente di:
- chiudere la propria istanza di OpenMonitor Advanced (icona nella tray → **Esci**): con l'istanza singola, ogni avvio degli script passerebbe il controllo a quella istanza e la misura non sarebbe valida;
- non avviare OpenMonitor Advanced per circa 140 minuti (Step 5–8); nella seconda prova lunga la finestra deve restare visibile senza essere minimizzata o completamente coperta.

Poi controlla, in PowerShell dalla radice del repository:

```powershell
@(Get-Process oma-app -ErrorAction SilentlyContinue).Count
Test-Path "$env:LOCALAPPDATA\OpenMonitorAdvanced\crash.txt"
```

Risultato atteso: `0` e `False`. Con un crash marker l'app partirebbe in modalità sicura, senza le librerie dei vendor, e la misura sembrerebbe migliore del vero.

Il profilo WebView2 è lo stesso dell'istanza dell'utente: la preparazione dello Step 5 cambia anche la sua vista. Salva il `localStorage` dell'utente per ripristinarlo nello Step 8 (con l'app chiusa i file non sono in uso):

**Cleanup obbligatorio:** dopo un backup riuscito, il ripristino dello Step 8 è una clausola `finally` dell'intera procedura: eseguilo anche se preparazione, CDP, compilazione o misura falliscono, se l'utente interrompe, o se il budget è superato. Conserva il backup finché il ripristino non è verificato. Un errore impedisce di dichiarare la milestone conclusa, non autorizza a saltare il ripristino; termina prima i processi avviati per la prova e verifica che nessuno usi il profilo.

```powershell
$webProfile = "$env:LOCALAPPDATA\io.github.openmonitoradvanced\EBWebView"
@(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
    Where-Object { $_.CommandLine -like "*$webProfile*" }).Count
$backup = "$env:TEMP\oma-m3-localstorage"
Remove-Item -Recurse -Force $backup -ErrorAction SilentlyContinue
if (Test-Path "$webProfile\Default\Local Storage") { Copy-Item -Recurse "$webProfile\Default\Local Storage" $backup }
Test-Path $backup
```

Risultato atteso: `0` (nessun processo WebView2 usa ancora il profilo) e `True`. Se la cartella `Local Storage` non esiste, l'utente non ha mai aperto la finestra: `False` va bene, e nello Step 8 basta cancellare la cartella creata dalle misure.

Controlla anche il formato delle chiavi scritto dai Task 10 e 11:

```bash
grep -rnE "oma\.view|oma\.advanced\." app/src --include='*.ts' --include='*.svelte' --exclude='*.test.ts'
grep -n "legend" app/src/components/advanced/HistoryChart.svelte
```

Risultato atteso: `oma.advanced.section` e `oma.advanced.window` salvati come testo semplice (`setItem(key, section)` e `setItem(key, String(window))`, oppure `JSON.stringify` di un numero, che dà lo stesso testo), `oma.advanced.series.<sectionId>` come `JSON.stringify` di un array di stringhe. Se il Task 10 salvasse la sezione con `JSON.stringify` (testo tra virgolette), adegua le due righe `localStorage.setItem('oma.advanced.section', section)` e il confronto finale dello script dello Step 2 prima di proseguire. Il secondo comando dice se la legenda di uPlot è visibile (`legend: { show: false }` → `legendSeries` sarà 0; altrimenti 9, l'asse x più 8 serie).

- [ ] **Step 2: Crea lo script di preparazione**

`scripts/seed-advanced-view.ps1`, file completo:

```powershell
<#
.SYNOPSIS
  Prepares the window-mode budget measurement: makes the next start of
  OpenMonitor Advanced open the Advanced view on a given page and chart window.
.DESCRIPTION
  Starts the app with the WebView2 DevTools port enabled, writes the Advanced
  view state into the page's localStorage through the Chrome DevTools Protocol
  (Runtime.evaluate: no synthetic input, no UI Automation), and closes the app.
  It then starts the app a second time and checks that the page opens on the
  requested section, window and series, optionally saving a screenshot. The
  state lives in the WebView2 profile, so the next normal start (without the
  DevTools port, as in measure-footprint.ps1) opens on the same page.
  -CheckOnly skips the seeding and only checks what the next start opens.
  Every instance of the same executable must be closed first: with the single
  instance lock, a launch would only hand over to the running one.
.EXAMPLE
  ./scripts/seed-advanced-view.ps1 -Section 'gpu/pci-0000:01:00.0' -Window 3600 -Screenshot "$env:TEMP\oma-seed.png"
.EXAMPLE
  ./scripts/seed-advanced-view.ps1 -Section 'gpu/pci-0000:01:00.0' -Window 3600 -CheckOnly
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\oma-app.exe'),
    [Parameter(Mandatory = $true)][string]$Section,
    [ValidateSet(60, 300, 1800, 3600)][int]$Window = 3600,
    # Sensor ids for the chart; empty keeps the page's default series.
    [string[]]$Series = @(),
    # Text that must appear on the page (for example the GPU name).
    [string]$ExpectText = '',
    [int]$Port = 9223,
    [string]$Screenshot = '',
    [switch]$CheckOnly
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path
$exeName = [IO.Path]::GetFileNameWithoutExtension($exePath)

if (Get-Process -Name $exeName -ErrorAction SilentlyContinue) {
    throw "Close every running $exeName.exe first (a new launch would hand over to it)."
}

function Get-Descendants([int]$RootId) {
    $processes = @(Get-CimInstance Win32_Process)
    $tree = [Collections.Generic.HashSet[int]]::new()
    [void]$tree.Add($RootId)
    do {
        $changed = $false
        foreach ($p in $processes) {
            if ($tree.Contains([int]$p.ParentProcessId) -and $tree.Add([int]$p.ProcessId)) { $changed = $true }
        }
    } while ($changed)
    return @($tree)
}

function Start-WithDevTools {
    $previous = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
    try {
        $proc = Start-Process -FilePath $exePath -WindowStyle Normal -PassThru
    }
    finally {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $previous
    }
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 500
        $proc.Refresh()
        if ($proc.HasExited) { throw "$exeName.exe exited (another instance may be running)." }
        try {
            $page = @(Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/list" -TimeoutSec 2 |
                Where-Object { $_.type -eq 'page' }) | Select-Object -First 1
        }
        catch { $page = $null }
        if ($page) {
            $state = @{ Process = $proc; Socket = $page.webSocketDebuggerUrl }
            while ((Get-Date) -lt $deadline) {
                if ((Invoke-Page $state 'document.readyState') -eq 'complete') { return $state }
                Start-Sleep -Milliseconds 500
            }
        }
    }
    throw "No WebView2 page on DevTools port $Port within 60 s."
}

function Invoke-Cdp($State, [string]$Method, [hashtable]$Params) {
    $socket = [Net.WebSockets.ClientWebSocket]::new()
    $none = [Threading.CancellationToken]::None
    $socket.ConnectAsync([Uri]$State.Socket, $none).GetAwaiter().GetResult()
    try {
        $request = @{ id = 1; method = $Method; params = $Params } | ConvertTo-Json -Depth 10 -Compress
        $bytes = [Text.Encoding]::UTF8.GetBytes($request)
        $socket.SendAsync([ArraySegment[byte]]::new($bytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, $none).GetAwaiter().GetResult()
        $buffer = [byte[]]::new(65536)
        while ($true) {
            $message = [IO.MemoryStream]::new()
            do {
                $received = $socket.ReceiveAsync([ArraySegment[byte]]::new($buffer), $none).GetAwaiter().GetResult()
                $message.Write($buffer, 0, $received.Count)
            } until ($received.EndOfMessage)
            $reply = [Text.Encoding]::UTF8.GetString($message.ToArray()) | ConvertFrom-Json
            if ($reply.id -eq 1) {
                if ($reply.error) { throw "CDP $Method failed: $($reply.error.message)" }
                return $reply.result
            }
        }
    }
    finally {
        $socket.Dispose()
    }
}

function Invoke-Page($State, [string]$Expression) {
    $result = Invoke-Cdp $State 'Runtime.evaluate' @{ expression = $Expression; returnByValue = $true }
    if ($result.exceptionDetails) { throw "Page script failed: $($result.exceptionDetails.text)" }
    return $result.result.value
}

function Stop-Gracefully($State) {
    $proc = $State.Process
    $tree = Get-Descendants $proc.Id
    $children = @($tree | Where-Object { $_ -ne $proc.Id })
    # WM_CLOSE destroys the window (the app stays in the tray), so WebView2
    # shuts down normally and flushes localStorage to the profile on disk.
    [void]$proc.CloseMainWindow()
    $deadline = (Get-Date).AddSeconds(20)
    while ($children.Count -and (Get-Date) -lt $deadline -and
        @(Get-Process -Id $children -ErrorAction SilentlyContinue).Count) {
        Start-Sleep -Milliseconds 500
    }
    Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline -and @(Get-Process -Id $tree -ErrorAction SilentlyContinue).Count) {
        Start-Sleep -Milliseconds 500
    }
}

$sectionJs = ConvertTo-Json -InputObject $Section -Compress
$seriesJs = ConvertTo-Json -InputObject @($Series) -Compress
$expectJs = ConvertTo-Json -InputObject $ExpectText -Compress
$seed = @"
(() => {
  const section = $sectionJs;
  const series = $seriesJs;
  localStorage.setItem('oma.view', 'advanced');
  localStorage.setItem('oma.advanced.section', section);
  localStorage.setItem('oma.advanced.window', '$Window');
  if (series.length) localStorage.setItem('oma.advanced.series.' + section, JSON.stringify(series));
  else localStorage.removeItem('oma.advanced.series.' + section);
  location.reload();
  return 'seeded';
})()
"@
$check = @"
JSON.stringify({
  view: localStorage.getItem('oma.view'),
  section: localStorage.getItem('oma.advanced.section'),
  window: localStorage.getItem('oma.advanced.window'),
  series: localStorage.getItem('oma.advanced.series.' + $sectionJs),
  charts: document.querySelectorAll('.uplot').length,
  legendSeries: document.querySelectorAll('.uplot .u-legend .u-series').length,
  expectedText: $expectJs === '' || document.body.innerText.includes($expectJs)
})
"@

# 1. Seed the state, then close the app so WebView2 writes it to disk.
if (-not $CheckOnly) {
    $app = Start-WithDevTools
    try {
        [void](Invoke-Page $app $seed)
        Start-Sleep -Seconds 10
    }
    finally {
        Stop-Gracefully $app
    }
}

# 2. Start again and check that the page opens on the seeded state.
$app = Start-WithDevTools
try {
    Start-Sleep -Seconds 10
    $state = Invoke-Page $app $check | ConvertFrom-Json
    if ($Screenshot) {
        $image = Invoke-Cdp $app 'Page.captureScreenshot' @{ format = 'png' }
        [IO.File]::WriteAllBytes($Screenshot, [Convert]::FromBase64String($image.data))
    }
}
finally {
    Stop-Gracefully $app
}

$state | Format-List
$problems = @()
if ($state.view -ne 'advanced') { $problems += "view is '$($state.view)'" }
if ($state.section -ne $Section) { $problems += "section is '$($state.section)'" }
if ($state.window -ne "$Window") { $problems += "window is '$($state.window)'" }
if ($state.charts -lt 1) { $problems += 'no uPlot chart on the page' }
if (-not $state.expectedText) { $problems += "text '$ExpectText' not on the page" }
if ($problems.Count) { throw "The Advanced view did not open as seeded: $($problems -join '; ')" }
'Seeded: the next start opens the Advanced view on the requested page.'
```

Controlla la sintassi senza avviare nulla (in PowerShell, dalla radice del repository):

```powershell
$errors = $null
[void][System.Management.Automation.Language.Parser]::ParseFile("$PWD\scripts\seed-advanced-view.ps1", [ref]$null, [ref]$errors)
$errors.Count
```

Risultato atteso: `0`.

- [ ] **Step 3: Aggiungi allo script di misura lo storico pieno**

`scripts/measure-footprint.ps1`, file completo (la misura resta quella della M2, spostata in `Measure-Process`; si aggiungono `-FillHistoryMinutes` e il campo `HistoryMinutes` nell'output):

```powershell
<#
.SYNOPSIS
  Measures OpenMonitor Advanced against the performance budget (spec §1.2).
.DESCRIPTION
  Starts the release build, waits for warm-up, then reports the app's CPU
  usage and the private working set (Task Manager "Memory" column) of the app
  and of its WebView2 child processes. VendorModules lists the GPU vendor
  libraries loaded in the app, so a measurement taken in safe mode (or on a
  machine without a vendor driver) is recognisable.
  With -FillHistoryMinutes the app first runs in the tray for that long, so
  the one-hour history is full; the tray is measured, then (unless
  -Minimized) a second launch hands over to the running instance, which opens
  its window, and the window is measured. The window opens on the view saved
  in the WebView2 profile (see seed-advanced-view.ps1).
.EXAMPLE
  ./scripts/measure-footprint.ps1                            # window open
  ./scripts/measure-footprint.ps1 -Minimized                 # tray only
  ./scripts/measure-footprint.ps1 -FillHistoryMinutes 61     # full history: tray, then window
#>
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\oma-app.exe'),
    [int]$WarmupSeconds = 15,
    [int]$SampleSeconds = 30,
    [switch]$Minimized,
    [int]$FillHistoryMinutes = 0
)

$ErrorActionPreference = 'Stop'
$exePath = (Resolve-Path $Exe).Path

function Measure-Process([Diagnostics.Process]$Proc, [string]$Mode) {
    Start-Sleep -Seconds $WarmupSeconds
    $Proc.Refresh()
    if ($Proc.HasExited) { throw "The measured instance exited (another instance may already be running)." }
    $cpuStart = $Proc.TotalProcessorTime
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $SampleSeconds
    $Proc.Refresh()
    $cpuEnd = $Proc.TotalProcessorTime
    $cpuPercent = ($cpuEnd - $cpuStart).TotalMilliseconds / $elapsed.Elapsed.TotalMilliseconds / [Environment]::ProcessorCount * 100

    # Follow the actual process tree, including renderer grandchildren.
    $processes = @(Get-CimInstance Win32_Process)
    $descendants = [Collections.Generic.HashSet[int]]::new()
    [void]$descendants.Add($Proc.Id)
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
    $ids = @($Proc.Id) + @($webviews | ForEach-Object { [int]$_.ProcessId })
    $perf = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
        Where-Object { $ids -contains [int]$_.IDProcess })
    if ($perf.Count -ne $ids.Count) { throw "Missing process memory counters; measurement is invalid." }
    $appPrivate = ($perf | Where-Object { [int]$_.IDProcess -eq $Proc.Id }).WorkingSetPrivate
    $vendorDlls = @('nvml.dll', 'nvapi64.dll', 'atiadlxx.dll', 'ControlLib.dll')
    $vendorModules = @($Proc.Modules | Where-Object { $vendorDlls -contains $_.ModuleName } |
        ForEach-Object { $_.ModuleName } | Sort-Object -Unique)
    $totalPrivate = ($perf | Measure-Object -Property WorkingSetPrivate -Sum).Sum

    [pscustomobject]@{
        Mode              = $Mode
        HistoryMinutes    = $FillHistoryMinutes
        CorePercentCpu    = [math]::Round($cpuPercent, 2)
        AppPrivateMB      = [math]::Round($appPrivate / 1MB, 1)
        WebView2Processes = $webviews.Count
        TotalPrivateMB    = [math]::Round($totalPrivate / 1MB, 1)
        VendorModules     = if ($vendorModules.Count) { $vendorModules -join ', ' } else { '(none)' }
    }
}

$fill = $FillHistoryMinutes -gt 0
$proc = if ($Minimized -or $fill) {
    Start-Process -FilePath $exePath -ArgumentList '--minimized' -WindowStyle Hidden -PassThru
} else {
    # Controller ruling: a hidden window would likely stop WebView rendering,
    # invalidating the "window open" measurement, so window mode launches
    # normally. -WindowStyle Hidden is kept only for -Minimized.
    Start-Process -FilePath $exePath -WindowStyle Normal -PassThru
}

try {
    if ($fill) {
        Start-Sleep -Seconds 5
        $proc.Refresh()
        if ($proc.HasExited) { throw "The measured instance exited (another instance may already be running)." }
        Start-Sleep -Seconds ($FillHistoryMinutes * 60)
        Measure-Process $proc 'tray' | Format-List
        if (-not $Minimized) {
            # Single instance: the second launch asks the running app to open its window, then exits.
            $second = Start-Process -FilePath $exePath -WindowStyle Normal -PassThru
            if (-not $second.WaitForExit(15000)) { throw "The second launch did not hand over to the running instance." }
            Measure-Process $proc 'window' | Format-List
        }
    } elseif ($Minimized) {
        Measure-Process $proc 'tray' | Format-List
    } else {
        Measure-Process $proc 'window' | Format-List
    }
}
finally {
    Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
}
```

Controlla la sintassi come nello Step 2, con `scripts\measure-footprint.ps1`. Risultato atteso: `0`.

- [ ] **Step 4: Compila in release**

```bash
cd app && pnpm tauri build --no-bundle && cd ..
git rev-parse --short HEAD
```

Annota la revisione: va nel registro dello Step 9.

- [ ] **Step 5: Prepara la vista Avanzata e verificala**

In PowerShell, dalla radice del repository:

```powershell
$gpu = 'gpu/pci-0000:01:00.0'
$series = @('load/core', 'load/3d', 'load/copy', 'load/video-decode', 'load/video-encode',
    'fan/percent', 'percent/power-limit', 'temperature/core') | ForEach-Object { "$gpu/$_" }
./scripts/seed-advanced-view.ps1 -Section $gpu -Window 3600 -Series $series -ExpectText 'RTX 4080' -Screenshot "$env:TEMP\oma-m3-seed.png"
```

Dura circa un minuto e apre due volte la finestra dell'app. Risultato atteso:

```
view         : advanced
section      : gpu/pci-0000:01:00.0
window       : 3600
series       : ["gpu/pci-0000:01:00.0/load/core", … ,"gpu/pci-0000:01:00.0/temperature/core"]
charts       : 1
legendSeries : 9            (0 se la legenda è nascosta, vedi Step 1)
expectedText : True

Seeded: the next start opens the Advanced view on the requested page.
```

`series` deve contenere le 8 serie. Se la UI riscrive la chiave, il valore mostrato è quello che l'app usa davvero. Apri lo screenshot `%TEMP%\oma-m3-seed.png` (con lo strumento Read) e verifica:
- vista Avanzata;
- voce della RTX 4080 selezionata nella barra laterale;
- finestra 1h attiva;
- 8 serie nel grafico, con due assi (% e °C).

Se qualcosa non corrisponde, correggi (lo script o la UI) e ripeti: non misurare uno stato diverso.

Se lo script si ferma con `No WebView2 page on DevTools port 9223 within 60 s`:
- ripeti con `-Port 9333`, nel caso la porta sia occupata;
- se CDP resta indisponibile, **verifica manuale (utente)**: avvia `.\target\release\oma-app.exe` e chiedi all'utente di aprire **Avanzata** → la voce della RTX 4080, scegliere la finestra **1h**, selezionare le 8 serie elencate sopra e poi chiudere la finestra con la X e l'app dalla tray (**Esci**). Durante le misure dello Step 6 e dello Step 7 chiedi all'utente di confermare a vista che la finestra si è aperta su quella pagina.

- [ ] **Step 6: Misura a pochi secondi dall'avvio (confronto con M1 e M2)**

```bash
pwsh -File scripts/measure-footprint.ps1
pwsh -File scripts/measure-footprint.ps1 -Minimized
```

La prima misura apre la finestra, che mostra la pagina preparata; la seconda resta nella tray. Conserva l'output completo.

- [ ] **Step 7: Misura con lo storico di 1 ora pieno**

Dura circa 63 minuti: eseguila in background (con lo strumento Bash, `run_in_background`) e aspetta la notifica di fine; non interrompere l'utente.

```bash
pwsh -File scripts/measure-footprint.ps1 -FillHistoryMinutes 61 > "$TEMP/oma-m3-fill.txt" 2>&1
cat "$TEMP/oma-m3-fill.txt"
```

Risultato atteso: due blocchi, `Mode : tray` e `Mode : window`, entrambi con `HistoryMinutes : 61`. La finestra si apre dopo circa 62 minuti, per il secondo avvio. Se compare `The second launch did not hand over to the running instance.` oppure `The measured instance exited`, un'altra istanza era in esecuzione: ripeti lo Step 1.

Poi verifica che lo stato misurato fosse ancora quello preparato:

```powershell
./scripts/seed-advanced-view.ps1 -Section 'gpu/pci-0000:01:00.0' -Window 3600 -ExpectText 'RTX 4080' -CheckOnly
```

Risultato atteso: come nello Step 5, con le stesse 8 serie.

Misura poi anche il caso di **pagina continuamente visibile per 61 minuti**, senza `-FillHistoryMinutes`, senza cambiare pagina, finestra temporale o serie. Il parametro esistente `-WarmupSeconds` ritarda la misura mentre la finestra è già aperta:

```powershell
pwsh -File scripts/measure-footprint.ps1 -WarmupSeconds 3660 -SampleSeconds 30 > "$env:TEMP\oma-m3-visible.txt" 2>&1
Get-Content "$env:TEMP\oma-m3-visible.txt"
```

Esegui anche questa prova in background e attendine la conclusione. L'output ha `Mode : window` e `HistoryMinutes : 0`: quest'ultimo campo conta solo il riempimento preliminare nella tray, quindi registra esplicitamente nel risultato i 3660 s di warmup visibile. Se la pagina viene nascosta, minimizzata o ricreata durante l'attesa, la prova non è valida: alla ricomparsa il grafico riparte dal seed decimato. Richiedi conferma manuale all'utente che la finestra sia rimasta visibile. Questa riga deve rispettare lo stesso budget di 200 MB.

**Budget (spec §1.2)** e riferimenti precedenti. I riferimenti non sono risultati da riprodurre: servono solo a riconoscere un'anomalia.

| Modalità | Voce | Budget | Riferimenti |
|---|---|---|---|
| `window` | `CorePercentCpu` | < 1 | M2: 0,05 |
| `window` | `TotalPrivateMB` | < 200 | M2 (vista Semplificata): 113,3. Spike uPlot con una build M2: 148,0 con 8 serie × 3600 punti non decimati, 219,0 con 20 serie |
| `window` | `WebView2Processes` | — | 6 |
| `tray` | `AppPrivateMB` | < 30 | M2: 16,7 (storico quasi vuoto) |
| `tray` | `WebView2Processes` | 0 | 0 |
| `tray` | `CorePercentCpu` | < 1 | M2: 0,05 |

In tutte le righe `VendorModules` deve essere `atiadlxx.dll, nvapi64.dll, nvml.dll`: altrimenti la misura è in modalità sicura, quindi non valida.

Se una voce supera il budget, **fermati e segnala l'esito DONE_WITH_CONCERNS**: non allentare il budget e non proseguire con gli step di consegna o commit. Esegui comunque il ripristino obbligatorio dello Step 8 prima di restituire l'esito. Prima di segnalarlo, isola la causa:
- **Finestra oltre 200 MB:** ripeti lo Step 5 senza `-Series` (serie predefinite della pagina GPU, 2 serie) e poi lo Step 6 in modalità finestra. La differenza è il costo delle 6 serie in più. Controlla anche che per le finestre di 30 e 60 minuti la UI chieda lo storico decimato: `grep -rnE "maxPoints|MAX_POINTS|DECIMATE_FROM" app/src/components/advanced app/src/lib/advanced` deve mostrare la chiamata `backend.getHistory(ids, seconds, maxPointsFor(seconds))` in `HistoryChart.svelte` e `maxPointsFor`, che restituisce `MAX_POINTS` (900) per `windowSeconds >= DECIMATE_FROM` (1800).
- **Tray oltre 30 MB:** confronta la riga `tray` dello Step 6 (storico quasi vuoto) con quella dello Step 7 (storico pieno). Poi misura in modalità sicura (`.\target\release\oma-app.exe --minimized --safe`, 15 s di attesa, `(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$((Get-Process oma-app).Id)").WorkingSetPrivate / 1MB`) per isolare il costo delle librerie dei vendor, come in M2.

- [ ] **Step 8: Ripristina la vista dell'utente**

Le misure hanno lasciato nel profilo la vista Avanzata con 8 serie. Ripristina il `localStorage` salvato nello Step 1, con l'app chiusa:

```powershell
$webProfile = "$env:LOCALAPPDATA\io.github.openmonitoradvanced\EBWebView"
$backup = "$env:TEMP\oma-m3-localstorage"
@(Get-Process oma-app -ErrorAction SilentlyContinue).Count
@(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
    Where-Object { $_.CommandLine -like "*$webProfile*" }).Count
Remove-Item -Recurse -Force "$webProfile\Default\Local Storage"
if (Test-Path $backup) { Copy-Item -Recurse $backup "$webProfile\Default\Local Storage" }
```

Risultato atteso: `0` e `0` prima della copia (se un processo è ancora vivo, aspetta qualche secondo e ripeti). Poi informa l'utente che può riavviare la propria istanza: si riaprirà sulla vista che aveva prima.

- [ ] **Step 9: Registra i risultati**

In `docs/perf-budget.md`, nella tabella, aggiungi cinque righe M3 dopo quelle M2, usando **esclusivamente** l'output degli Step 6 e 7:
- `M3`, `same machine, …` con le versioni dei driver lette da `Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion` e la revisione dello Step 4, modalità `window (Advanced view, GPU page, 1 h chart, 8 series)`;
- `M3`, `same machine, same drivers, build …`, modalità `tray`;
- `M3`, `same machine, same drivers, build …`, modalità `window, after 61 min in the tray (full 1 h history; Advanced view as above)`;
- `M3`, `same machine, same drivers, build …`, modalità `tray, after 61 min (full 1 h history)`.
- `M3`, `same machine, same drivers, build …`, modalità `window, continuously visible for 61 min (Advanced GPU, 1 h, 8 series; raw live tail)`; annota `WarmupSeconds=3660`, senza interpretare `HistoryMinutes=0` come storico vuoto.

Per ogni riga riporta App CPU %, App private MB, WebView2 procs, Total private MB e l'esito del budget (`yes` solo se tutte le soglie sono rispettate).

Dopo la sezione `## M2 measurement details` aggiungi una sezione `## M3 measurement details` con:
- data, revisione e comando di build;
- protocollo: 15 s di riscaldamento e 30 s di campionamento; le righe con storico pieno dopo 61 minuti nella tray;
- come è stata preparata la vista (`scripts/seed-advanced-view.ps1`, sezione `gpu/pci-0000:01:00.0`, finestra 3600, le 8 serie) e l'output del controllo `-CheckOnly` dello Step 7;
- l'output grezzo delle quattro misure, `VendorModules` compreso;
- una nota: la decimazione a 900 punti per le finestre di 30 e 60 minuti (D3) e il limite di 8 serie (D4) servono a stare sotto i 200 MB. Il valore di 148 MB dello spike uPlot (8 serie × 3600 punti non decimati, build M2) è evidenza precedente, non riprodotta su questa build.

Non riportare i valori della tabella dei riferimenti come risultati.

- [ ] **Step 10: Emenda la spec**

Tutte le modifiche sono in `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`. Il testo nuovo è in italiano, nello stile del resto della spec.

**§3 Modello dati.** Sostituisci la riga

```markdown
- **ID stabili tra riavvii**, costruiti dall'identità hardware. Esempi: `gpu/pci-0000:01:00.0/temperature/hotspot`, `storage/nvme-<seriale-hash>/temperature/composite`. Regole, impostazioni e selezione dei sensori per il log vi fanno riferimento.
```

con

```markdown
- **ID stabili tra riavvii**, costruiti dall'identità hardware. Esempi: `gpu/pci-0000:01:00.0/temperature/hotspot`, `storage/device-<hash>/temperature/drive`. Regole, impostazioni e selezione dei sensori per il log vi fanno riferimento. Per i dischi l'identità segue una catena di ripiego (§5.1).
```

**§4.2 Storico.** Sostituisci le righe

```markdown
- Un ring buffer per sensore, **1 ora di campioni** alla frequenza corrente, con i timestamp condivisi. Con circa 300 sensori a 1 s sono circa 5 MB.
- Min, max e media partono dall'avvio dell'app e si possono azzerare dall'interfaccia.
- I periodi in cui una fonte non è disponibile sono registrati come valori assenti e appaiono come buchi nei grafici.
```

con

```markdown
- Un ring buffer per sensore, **1 ora di campioni** alla frequenza corrente, con i timestamp condivisi. Con circa 300 sensori a 1 s sono circa 5 MB.
- **Min, max e media** si calcolano nel nucleo (`oma-core`), per ogni sensore, dal primo ciclo dopo l'avvio dell'app, e non nell'interfaccia: la WebView viene distrutta quando si chiude la finestra (§2.2), mentre le statistiche devono coprire anche il tempo passato nella tray. I valori assenti non contano.
  - Il pulsante "azzera" di una pagina della vista Avanzata azzera le statistiche dei soli sensori di quella pagina.
  - Quando cambia lo schema, le statistiche di un sensore restano solo se ID, fonte e unità sono invariati, come lo storico.
- L'istante del primo ciclo (`startedAtMs`) è esposto all'interfaccia: il banner "monitoraggio attivo da…" conta da lì anche dopo aver riaperto la finestra dalla tray.
- **Storico inviato all'interfaccia:** le finestre da 1 e 5 minuti arrivano con tutti i campioni. Per 30 minuti e 1 ora lo storico arriva decimato ad al massimo 900 punti per serie, in intervalli bilanciati le cui dimensioni differiscono al massimo di un campione. Ogni intervallo interamente valido dà due punti, minimo e massimo, così i picchi restano visibili; i timestamp sono i confini dell'inviluppo, non gli istanti reali degli estremi. Se un intervallo contiene un valore assente, per quella serie entrambi i punti sono assenti: i buchi si ampliano conservativamente all'intervallo. La decimazione serve al budget di memoria della finestra: in una prova, 20 serie da 3600 punti portavano il totale a 219 MB.
- I periodi in cui una fonte non è disponibile sono registrati come valori assenti e appaiono come buchi nei grafici.
```

**§5.1 Provider `sys`.** Sostituisci la riga della tabella

```markdown
| Temperatura NVMe (fallback) | `IOCTL_STORAGE_QUERY_PROPERTY` con `StorageDeviceTemperatureProperty`, se accessibile senza privilegi |
```

con

```markdown
| Dischi: identità | Catena di ripiego, dal livello più forte: numero di serie (`storage/device-…`, invariato dalla M1; un seriale non UTF-8 si usa come byte grezzi) → GUID del disco GPT (`storage/gpt-…`, da `IOCTL_DISK_GET_DRIVE_LAYOUT_EX`) → firma MBR più dimensione del disco (`storage/mbr-…`) → instance id PnP (`storage/pnp-…`, da SetupAPI; legato alla porta, cambia se il disco viene spostato). Ogni valore entra nell'ID solo come hash SHA-256. Un livello si usa solo se il suo valore è unico tra i dischi della macchina, perché i cloni copiano GUID e firma. Un disco si omette solo se falliscono tutti i livelli, con un avviso nel log che indica il motivo per ciascuno. Tutte le chiamate usano `\\.\PhysicalDriveN` aperto con accesso 0, senza privilegi. |
| Temperatura dei dischi | `IOCTL_STORAGE_QUERY_PROPERTY` con `StorageDeviceTemperatureProperty`, senza privilegi (verificato in M3 su NVMe e SATA). Il supporto dipende da disco e driver: un disco senza supporto risponde `ERROR_INVALID_FUNCTION` e non ha sensori. Il sensore 0 è la temperatura del disco (la "composite" degli NVMe, `…/temperature/drive`), gli altri sono sensori aggiuntivi (`…/temperature/sensor-<n>`); `0x8000` significa "non riportato". Le soglie warning e critical diventano proprietà del device (`tempWarningC`, `tempCriticalC`) per le regole della M5. Lettura ogni 30 s (§4.1), al massimo un disco per ciclo (fino a circa 140 ms per un NVMe che esce da uno stato a basso consumo). Un disco in standby (`GetDevicePowerState`) non viene interrogato, per non risvegliarlo. |
```

**§5.2 Provider `gpu`.**

1. Sostituisci la riga

```markdown
     - L'utilizzo per processo arriva con la vista Avanzata (M3).
```

con

```markdown
     - Lo stesso contatore dà l'utilizzo per processo (M3, punto 5).
```

2. Sostituisci la riga

```markdown
   - **PDH `GPU Adapter Memory(*)`:** memoria dedicata e condivisa in uso. `GPU Process Memory` (per processo) arriva in M3.
```

con

```markdown
   - **PDH `GPU Adapter Memory(*)`:** memoria dedicata e condivisa in uso. `GPU Process Memory(*)` dà la memoria per processo (M3, punto 5).
```

3. Sostituisci la riga

```markdown
     - **Stato:** implementato e coperto da test con funzioni finte, **non ancora verificato su hardware Intel**. `ctlPciGetState` arriva in M3.
```

con

```markdown
     - **Stato:** implementato e coperto da test con funzioni finte, **non ancora verificato su hardware Intel**. Dalla M3 legge anche il link PCIe (`ctlPciGetState` per generazione e larghezza correnti, `ctlPciGetProperties` per i massimi), verificato anch'esso solo con funzioni finte.
```

4. Dopo la riga

```markdown
   - Il carico per motore viene sempre da PDH.
```

aggiungi

```markdown
   - Le proprietà statiche del device (punto 5) seguono la stessa priorità: per ogni chiave vince il livello più prioritario che la fornisce. PnP fornisce solo proprietà, nessun sensore. `pciAddress` e `integrated` vengono dall'enumerazione e nessun livello li sovrascrive.
```

5. Sostituisci la riga

```markdown
5. **Fuori dalla M2, rimandati alla M3:** generazione e larghezza PCIe; utilizzo GPU per processo; utilizzo di encoder e decoder da NVML; contatori di energia; soglie del limite di potenza per le regole.
```

con

```markdown
5. **Aggiunte della M3** (vista Avanzata):
   - **Utilizzo per processo**, dal livello PDH, che già legge `GPU Engine(*)`, più `GPU Process Memory(*)` (`Dedicated Usage`, `Shared Usage`; circa +50 µs per ciclo).
     - Il carico di un processo è quello del suo motore più occupato (massimo tra i motori, limitato a 0–100), coerente con il carico core dell'adattatore. Il tipo di motore si indica solo se il carico è maggiore di zero.
     - I nomi dei processi vengono da `CreateToolhelp32Snapshot` (documentata e senza aprire i processi; circa 2,4 ms), al massimo una volta per ciclo e solo quando compare un pid sconosciuto, con una cache che scarta i pid spariti. Non si usano `OpenProcess` (fallisce per dwm, System e i servizi) né la chiamata non documentata `NtQuerySystemInformation` (88). Pid 0 = "Idle", pid 4 = "System".
     - Non sono sensori: niente ID né storico. La vista Avanzata legge l'elenco con il comando `get_gpu_processes` (al massimo 20 righe, per carico e poi per memoria dedicata). Il primo campione dopo un attach non ha il carico, perché il contatore è un tasso.
   - **Encoder e decoder da NVML** (`EncoderUtilization`, `DecoderUtilization`): campi nuovi, solo NVML, distinti dal carico dei motori video di PDH, che resta solo PDH.
   - **Link PCIe:** generazione e larghezza correnti come sensori (tipo `link`, unità `pcie_generation` e `lanes`) da NVML e IGCL. Generazione e larghezza massime sono proprietà del device (`pcieMaxGen`, `pcieMaxWidth`), da NVML oppure, per ogni vendor, dal nuovo livello base **PnP** (`CM_Get_DevNode_PropertyW`, proprietà PCI del device, letta alla discovery). Il valore "corrente" di PnP non si usa: viene fissato all'avvio del device e non si aggiorna.
   - **Limiti statici come proprietà** del device, letti una volta all'attach da NVML: limite di potenza minimo, massimo e predefinito (`powerLimitMinW`, `powerLimitMaxW`, `powerLimitDefaultW`) e soglie di temperatura slowdown, shutdown e massima (`tempSlowdownC`, `tempShutdownC`, `tempMaxC`). Servono alla vista Avanzata e alle regole della M5.
   - **Indirizzo PCI conservato per LUID:** se un'enumerazione successiva non riporta l'indirizzo di un adattatore, resta quello già noto, così l'ID del device non cambia.

   **Esclusi (decisione D10 del piano M3):**
   - NVML `TotalEnergyConsumption` (p95 circa 9 ms, e nessun uso prima del log CSV e delle regole della M5) e `PcieThroughput` (blocca per 31 ms);
   - gli elenchi dei processi di NVML: `usedGpuMemory` non è disponibile sotto WDDM;
   - i sensori ADL 40 e 41 (valori costanti, unità non documentata).
```

**§7.3 Vista Avanzata.** Sostituisci le righe

```markdown
- **Ogni pagina ha:**
  - **4 KPI** in alto, definiti per tipo di componente;
  - un **grafico storico** uPlot, con finestra 1m / 5m / 30m / 1h e con le serie da mostrare selezionabili;
  - una **tabella dei sensori** raggruppata per categoria (temperature, carico, clock, potenza, tensioni, ventole…), con colonne attuale, min, max e media, e un pulsante "azzera min/max";
  - un **badge della fonte** su ogni sensore, visibile passandoci sopra.
- **I sensori che richiedono il servizio**, quando questo non è attivo, non compaiono uno per uno. Al loro posto c'è un solo avviso: "N sensori in più disponibili con il servizio".
```

con

```markdown
- **Ogni pagina ha:**
  - **4 KPI** in alto, definiti per tipo di componente: i primi quattro disponibili di un elenco per tipo (per la GPU: carico, temperatura, potenza, VRAM, poi clock);
  - un **grafico storico** uPlot, con finestra 1m / 5m / 30m / 1h e con le serie da mostrare selezionabili:
    - **al massimo 8 serie e 2 unità di misura** insieme, con due assi verticali (sinistro e destro); oltre questi limiti il selettore non aggiunge serie. È il limite che tiene la finestra nel budget di memoria;
    - le finestre da 30 minuti e 1 ora usano lo storico decimato (§4.2), quelle da 1 e 5 minuti tutti i campioni;
    - il grafico si aggiorna al ritmo dei dati e si ferma quando la finestra non è visibile;
  - una **tabella dei sensori** raggruppata per categoria (temperature, carico, clock, potenza, tensioni, ventole…), con colonne attuale, min, max e media, e un pulsante "azzera min/max", che azzera le statistiche dei sensori della pagina (§4.2). I sensori sperimentali sono segnati come tali;
  - un **badge della fonte** su ogni sensore, visibile passandoci sopra;
  - le **informazioni del device**: le proprietà statiche, per esempio indirizzo PCI, link PCIe massimo, limiti di potenza e soglie di temperatura;
  - per le GPU, la **tabella dei processi** che usano la GPU, con carico, motore, memoria dedicata e condivisa: al massimo 20 righe, aggiornate ogni 2 s mentre la pagina è visibile.
- Sezione, finestra del grafico e serie scelte per ogni pagina restano salvate nella WebView (`localStorage`) finché le impostazioni della M5 non le sostituiscono. Un clic su un riquadro della vista Semplificata apre la pagina corrispondente.
- **I sensori che richiedono il servizio**, quando questo non è attivo, non compaiono uno per uno. Al loro posto c'è un solo avviso: "N sensori in più disponibili con il servizio". L'avviso arriva con il servizio (M4): in M3 non esistono ancora sensori del servizio.
- La voce **Batteria** compare solo quando esiste un device batteria: in M3 nessun provider lo crea ancora.
```

**§7.5 Buffer dei grafici.** Il Task 11 introduce un'eccezione esplicita al vincolo dei buffer tipizzati. Sostituisci la riga

```markdown
  - buffer tipizzati (`Float64Array`).
```

con

```markdown
  - buffer tipizzati (`Float64Array`) per le serie interne della vista Semplificata; per il grafico uPlot della vista Avanzata, array di `number | null`, perché `null` rappresenta i buchi. Il costo delle copie e della coda dal vivo rientra nella misura del budget con la finestra aperta per almeno un'ora.
```

**§8 Gestione errori.** Sostituisci la riga

```markdown
- **Dati anomali:** valori fuori dall'intervallo fisico plausibile vengono scartati come assenti e registrati nel log di diagnostica. Esempi: temperature < −50 °C o > 150 °C, percentuali < 0 o > 100 dove non ha senso.
```

con

```markdown
- **Dati anomali:** valori fuori dall'intervallo fisico plausibile vengono scartati come assenti e registrati nel log di diagnostica, al massimo una riga al minuto per sensore. Esempi: temperature < −50 °C o > 150 °C, percentuali < 0 o > 100 dove non ha senso.
- **Nucleo:** un panic durante un ciclo di campionamento viene registrato nel log e il ciclo successivo parte regolarmente. Un disallineamento tra valori e sensori non ferma lo storico: i valori mancanti diventano assenti e quelli in più si scartano. Se l'interfaccia non riceve dati per più di max(5 s, 5 intervalli), la barra superiore mostra "Dati non aggiornati".
```

**§13 Punti aperti.** Sostituisci il punto 6:

```markdown
6. **NVMe via `IOCTL_STORAGE_QUERY_PROPERTY` senza privilegi:** funziona?
```

con

```markdown
6. **NVMe via `IOCTL_STORAGE_QUERY_PROPERTY` senza privilegi:** funziona? **Risolto in M3:** sì. `StorageDeviceTemperatureProperty` su `\\.\PhysicalDriveN` aperto con accesso 0 funziona da utente normale su Windows 11 (build 26200), sia per NVMe sia per SATA. Il supporto dipende dal disco: un SSD SATA risponde `ERROR_INVALID_FUNCTION` (non supportato, non un problema di permessi). Vedi §5.1.
```

Controlla che nel file non restino riferimenti superati:

```bash
grep -nE "arriva in M3|arriva con la vista Avanzata|Fuori dalla M2|nvme-<seriale|Temperatura NVMe \(fallback\)" docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md
grep -n "Risolto in M3" docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md
```

Risultato atteso: il primo comando non stampa nulla; il secondo stampa una sola riga, il punto 6 di §13.

- [ ] **Step 11: Aggiorna il README**

In `README.md`:

1. Sostituisci la riga di stato

```markdown
**Status:** milestone 2 (GPU) — CPU, RAM, disks, network and GPUs (NVIDIA, AMD, Intel) without admin rights.
```

con

```markdown
**Status:** milestone 3 (Advanced view) — CPU, RAM, disks, network and GPUs (NVIDIA, AMD, Intel) without admin rights, with a page per component in the Advanced view.
```

2. Aggiungi prima di `## GPU support`:

```markdown
## Advanced view

A sidebar lists every component: the CPU, each GPU (the integrated one too), RAM, each disk and each
network adapter. The view reopens on the last page you visited. Each page shows:

- four key figures for the component;
- a history chart for the last 1 minute, 5 minutes, 30 minutes or 1 hour, with up to 8 series and
  2 units at once; the 30-minute and 1-hour windows draw a min/max envelope, so peaks stay visible;
- a table of every sensor, grouped by category, with current, minimum, maximum and average values.
  The monitor keeps these statistics from the moment the app starts, also while it sits in the tray;
  the reset button clears them for the sensors of that page. Hover a sensor to see where its value
  comes from; *experimental* marks readings from undocumented calls;
- the device's static details, such as PCIe link, power limits and temperature thresholds;
- for GPUs, the processes using the GPU, with their load and dedicated/shared memory.

Clicking a tile in the Simple view opens the matching page. If no data arrives for a few seconds, the
top bar shows *Data not updating*.

**Disks.** Drive temperatures come from the drive itself where it reports them (most NVMe drives,
some SATA drives), refreshed every 30 seconds; a spun-down disk is not woken up to read it. Disks
without a readable serial number (virtual machines, some RAID or USB enclosures) are still shown:
they are identified by their GPT disk GUID, their MBR signature or, as a last resort, the port they
are connected to.
```

- [ ] **Step 12: Registra i seguiti rimasti aperti**

Prima di scriverli, controlla che ognuno sia ancora aperto:

```bash
grep -nE "app\.run\(|run_return" app/src-tauri/src/main.rs
grep -n "single_instance::init" -A2 app/src-tauri/src/main.rs
grep -rn "fn used_pct" crates/oma-win/src
grep -n "from_raw_parts(items, count" crates/oma-win/src/pdh.rs
grep -nE "devCsp|ws://localhost:1420" app/src-tauri/tauri.conf.json
```

Risultato atteso (situazione alla stesura):
- `app.run(` presente, `run_return` assente;
- la callback di `single_instance::init` ignora gli argomenti (`_args`);
- due definizioni di `used_pct` (`memory.rs`, `storage.rs`);
- due `from_raw_parts(items, count as usize)` in `pdh.rs`;
- nessuna riga per `devCsp`.

Una voce già risolta da un task di M3 va in "Closed in M3", non tra quelle aperte.

Crea `docs/follow-ups.md`:

```markdown
# Follow-ups

Items consciously left open, with where they live and when they are expected to be picked up.
Updated at the end of every milestone (last update: M3).

## Open: code

| Item | Where | Pick up |
|---|---|---|
| The log guard is never dropped: `App::run` ends the process with `process::exit`, so the last buffered log lines can be lost. Use `run_return`, or flush on `RunEvent::Exit`. | `app/src-tauri/src/main.rs` | M5 (tray and settings) |
| The single-instance callback ignores the second launch's arguments, so a second `--minimized` launch opens the window. `scripts/measure-footprint.ps1 -FillHistoryMinutes` relies on a second launch *without* arguments opening the window: keep that working. | `app/src-tauri/src/main.rs` | M5 (tray) |
| `used_pct` is duplicated in the memory and storage providers. | `crates/oma-win/src/memory.rs`, `crates/oma-win/src/storage.rs` | when touched |
| PDH: the item count returned by the API goes unchecked into `from_raw_parts`, and a null `szName` is not guarded. | `crates/oma-win/src/pdh.rs` | when touched |
| The label-key test keeps a hand-written list: only GPU keys are cross-checked against the code (`GpuField` self-test); CPU, memory, storage and network keys are not. | `crates/oma-win/tests/labels.rs` | when touched |
| The CSP has no `devCsp` with `ws://localhost:1420`, so Vite hot reload inside `pnpm tauri dev` may be blocked. | `app/src-tauri/tauri.conf.json` | when touched |
| NVML is not initialised again after the NVIDIA driver is updated or unloaded while the app runs; its fields fall back to D3DKMT until a restart (README, "Known limits"). | `crates/oma-win/src/gpu/nvml.rs` | M6 |
| Disk temperature probes retry every 30 s, including disks asleep at startup; new driver sensor indices request rediscovery without waking a sleeping disk. Verify real standby/wake behavior before using these readings in rules. | `crates/oma-win/src/storage.rs` | M5 (disk rules; retry and index identity already covered in M3) |
| A disk identified only by its PnP instance id (no serial, no unique GPT or MBR id) gets a new id when it is moved to another port: its history and statistics restart. | `crates/oma-win/src/storage_identity.rs` | accepted |

## Open: deferred features (spec §5.2 point 5, §7.3; M3 decision D10)

- NVML `TotalEnergyConsumption` (p95 about 9 ms) and `PcieThroughput` (blocks 31 ms): only with sampling outside the tick, when the CSV log or the rules need them (M5).
- Battery page: appears when a battery provider exists.
- "N more sensors available with the service": M4.
- Advanced view state (section, chart window, series) lives in the WebView `localStorage` until `settings.json` (M5).

## Manual checks owed by a human

- Tray left click and the "Open" menu item re-create the window after it was closed (M1).
- USB disk hot-plug keeps or changes disk ids correctly (M1). Since M3 also: a disk without a serial number (for example a VHDX mounted by an administrator) appears with a `storage/gpt-…` id and keeps it across a restart.
- IGCL telemetry and PCIe link on Intel hardware; ADL on a dedicated Radeon (hardware matrix, spec §12).

## Closed in M3

- Disks without a unique readable serial are no longer dropped (identity fallback chain, Task 4).
- The "monitoring for N min" banner counts from the core's first tick, also after reopening from the tray (`startedAtMs`, Tasks 1 and 10).
- A panic in `Engine::tick` no longer stops sampling; `History::push` no longer panics on a length mismatch; the UI shows *Data not updating* (Tasks 2 and 10).
- Implausible-value debug logs are rate-limited to one line per sensor per minute (Task 2).
- The GPU PCI address is kept per LUID across enumerations (Task 6).
- A second launch while the window is closed re-creates the window: exercised by the full-history measurement (Task 14, Step 7).
```

- [ ] **Step 13: Verifica finale completa**

```bash
cd app && pnpm build && cd ..
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p oma-win -- --include-ignored
cd app && pnpm check && pnpm test && cd ..
```

Risultato atteso: tutto OK e senza avvisi. `cargo test -p oma-win -- --include-ignored` comprende i test hardware dei Task 4–8 e di `providers.rs`; `igcl_reads_an_intel_gpu` su questa macchina termina subito (`no Intel adapter: skipped`).

- [ ] **Step 14: Commit**

```bash
git add scripts/seed-advanced-view.ps1 scripts/measure-footprint.ps1 docs/perf-budget.md docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md README.md docs/follow-ups.md
git commit -m "docs: M3 budget results, Advanced view spec amendments, README and follow-ups"
```

---

---

## Copertura della spec (auto-revisione)

| Requisito della spec | Task |
|---|---|
| §4.2 min, max e media dall'avvio, azzerabili dall'interfaccia | 1, 3, 12 |
| §4.2 storico di 1 ora per sensore, buchi quando la fonte manca; finestre lunghe sottocampionate | 2, 3, 11 |
| §4.1 un tick in panic non ferma il campionamento; log dei valori scartati limitato | 2 |
| §5.1 temperatura dei dischi senza privilegi (NVMe e SATA dove supportata), soglie del disco | 5 |
| Seguito M1: disco senza numero di serie non più scartato (GPT, MBR, PnP) | 4 |
| §5.2 punto 5: link PCIe (attuale NVML/IGCL, massimo PnP/NVML), encoder/decoder NVML, soglie e limiti di potenza come proprietà | 6, 7 |
| §5.2 punto 5: uso GPU per processo (PDH) | 8, 13 |
| §5.2 punto 5: contatori di energia, throughput PCIe | esclusi (D10), emendamento nel Task 14 |
| Seguito M2: indirizzo PCI conservato per LUID | 6 |
| §7.1 selettore Semplice/Avanzata (esistente), badge dati non aggiornati | 10 |
| §7.2 clic su un riquadro apre la pagina corrispondente | 10 |
| §7.3 barra laterale per componente, tutte le GPU, voci senza dati nascoste, apertura sull'ultima sezione o sulla CPU | 10 |
| §7.3 4 KPI per tipo di componente | 13 |
| §7.3 grafico uPlot, finestra 1m/5m/30m/1h, serie selezionabili | 11 |
| §7.3 tabella per categoria con attuale/min/max/media e "azzera min/max" | 12 |
| §7.3 badge della fonte al passaggio del mouse; sensori sperimentali segnati | 12 |
| §7.3 "N sensori in più con il servizio"; Scheda madre e Batteria | rimandati (D10): M4 servizio, provider batteria |
| §7.5 token dei colori, niente animazioni continue, rendering sospeso quando la finestra non è visibile, un solo ciclo di rendering | 9, 11, 12, 13 |
| §7.6 stringhe in en/it con le stesse chiavi | 5, 6, 9–13 |
| Seguito M1: `startedAtMs` dal nucleo per il banner | 1, 3, 10 |
| §1.2 budget misurato a fine milestone; spec e README aggiornati; §13.6 risolto | 14 |
