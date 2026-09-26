# Riferimenti M4 — spike pre-esecuzione

Questa cartella raccoglie, come richiesto dal piano
(`docs/superpowers/plans/2026-09-26-m4-servizio.md`), i contratti, gli
estratti di codice funzionante e il riepilogo degli avvisi di trimming
prodotti dagli spike usa-e-getta di M4, ripuliti da seriali e percorsi
personali, così da essere riproducibili in un checkout pulito.

**Provenienza:** spike eseguiti il 26 settembre 2026 su una macchina Ryzen 7
7800X3D, scheda B650, 1 disco HDD SATA, 1 SSD SATA, 2 NVMe, Windows 11 Pro
26200, .NET SDK 10.0.303, Rust 1.90.0, `windows` 0.62, `@tauri-apps/cli`
2.11.5, NSIS 3.11.

**Regola:** le cartelle temporanee citate nei report (lo spike originale sotto
`.superpowers/plan-m4/spikes/`, ignorato da git, e lo scratchpad di sessione
con i sorgenti e i dump usa-e-getta) non esistono in un checkout pulito e non
sono garantite: **fa fede questa cartella**. Dove un report cita una riga di
sorgente di terze parti (LibreHardwareMonitor, DiskInfoToolkit,
RAMSPDToolkit, BlackSharp), il codice di terze parti stesso non è copiato
qui: la citazione è per tag/commit e percorso, e va confrontata con il
proprio checkout di quella libreria.

## File

| File | Contenuto | Task del piano supportati |
|---|---|---|
| [`s1-lhm.md`](s1-lhm.md) | LibreHardwareMonitorLib 0.9.6: sensori con/senza privilegi, tempi e memoria di `Open`/`Update`/`Close`, verdetto sul trimming (`TrimMode=full` OK), mappa disco ↔ `PhysicalDriveN`, identità dei dischi (seriale del descrittore vs IDENTIFY), sequenza di chiusura PawnIO, §9.7 mappatura canonica LHM → id del nucleo | Task 5 (schema dei sensori del servizio), Task 6 (gate di risveglio dischi/storage) |
| [`trim-warnings.md`](trim-warnings.md) | Elenco versionato dei 20 avvisi di trimming, con codice/origine/motivazione; separazione tra quelli di libreria (accettati) e i 6 del codice proprio dello spike (non applicabili a `oma-service`); regola: un nuovo avviso deve far fallire la build | Task 5, Task 6 |
| [`s2-msgpack.md`](s2-msgpack.md) | Busta MessagePack adiacente `{"type","body"}` e unione `{"kind","value"}`, byte-identità Rust/.NET verificata su 6 messaggi, insidie (`skip_serializing_if`, `MessagePackWriter` per valore, bit di NaN), raccomandazioni per `oma-core::protocol` e i formatter .NET | Task 2 (protocollo `oma-core::protocol`), Task 3 (formatter MessagePack lato .NET) |
| [`s3-pipe-scm.md`](s3-pipe-scm.md) | DACL della pipe (`0x0012019b`, niente `CreateNewInstance`), ciclo di accettazione senza finestra a zero istanze, client Rust overlapped con stop event, mappatura dei codici di errore, chiamate SCM non elevate (`OpenServiceW`, `StartServiceW`, codici 1060/5/1056) | Task 7 (client della pipe in `oma-win`), Task 8 (controllo del servizio via SCM) |
| [`s4-nsis.md`](s4-nsis.md) e [`s4-nsis/`](s4-nsis/) | Template NSIS personalizzato (Tauri CLI 2.11.5, diff di 8 righe), `oma.nsh` con tutta la logica (pagina componenti, arresto del servizio via SCM, installazione PawnIO), test di deriva dal template a monte, conflitti noti con il template di Tauri | Task 13 (installer NSIS e servizio d'installazione) |
| [`code/`](code/) | Estratti di codice sorgente funzionante citati dai report (vedi sotto) | Task 2, 3, 7, 8 |

## `code/` — estratti citati dai report

- [`code/s3-dotnet-server/`](code/s3-dotnet-server/) — `Program.cs` +
  `PipeServer.csproj`: il server a named pipe .NET di `s3-pipe-scm.md` §1
  (ACL, `FirstPipeInstance`, ciclo di accettazione a 8 istanze, framing a 4
  byte).
- [`code/s3-rust-client/`](code/s3-rust-client/) — `main.rs`, `pipe.rs`
  (client overlapped con stop event), `scm.rs` (chiamate SCM), `fake.rs`
  (server finto per i test in CI), `synctest.rs` (confronto con handle
  sincroni), `Cargo.toml`: il client Rust e le chiamate SCM di
  `s3-pipe-scm.md` §2–3.
- [`code/s2-msgpack-rust/`](code/s2-msgpack-rust/) — `gen_fixtures.rs`,
  `check_decode.rs`, `Cargo.toml`: l'encoder Rust (`rmp_serde::to_vec_named`)
  e le verifiche di decodifica di `s2-msgpack.md`.
- [`code/s2-msgpack-dotnet/`](code/s2-msgpack-dotnet/) — `Program.cs`,
  `MsgpackSpike.csproj`: l'encoder .NET a basso livello
  (`MessagePackWriter`/`MessagePackReader`) di `s2-msgpack.md`.

Non sono stati copiati i sorgenti di LibreHardwareMonitor, DiskInfoToolkit,
RAMSPDToolkit e BlackSharp (cartelle `lhm-src`, `dit-src`, `spd-src`,
`bs-src` dello spike S1): sono citati in `s1-lhm.md` per tag/commit e URL
GitHub. Non sono stati copiati i fixture binari `.msgpack` di S2 (dati
generati, non sorgente) né i dump JSON/log di S1 (contengono identificatori
hardware e non sono necessari per riprodurre il lavoro: la tabella §9 di
`s1-lhm.md` riporta già i valori rilevanti, con i seriali oscurati).

## Nota sulla dicitura degli avvisi di trimming

Il piano richiede di trattare ogni **nuovo** avviso di trimming come un
fallimento della build. `trim-warnings.md` è la base di confronto: un avviso
nuovo, non presente in quell'elenco con la stessa origine, deve bloccare
`dotnet publish` finché non è stato valutato con lo stesso criterio usato qui
(percorso solo Unix/STA/modulo disattivato, oppure generico su struct
sequenziali senza costruttore) e aggiunto alla tabella.
