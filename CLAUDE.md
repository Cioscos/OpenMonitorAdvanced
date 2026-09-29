# OpenMonitor Advanced

Monitor hardware open source per Windows 10/11 (GPL-3.0-or-later): vista Semplificata e vista Avanzata, palette Synthwave, nessun privilegio amministrativo per CPU/RAM/dischi/rete/GPU.

- **Spec (fonte di verità):** `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`
- **Piani per milestone:** `docs/superpowers/plans/` (M1 Fondamenta, M2 GPU, M3 vista Avanzata, M4 servizio e M5a impostazioni e tray completate; poi M5b regole e M5c log CSV, che con M5a formano M5 regole e integrazione; infine M6 rifinitura)
- **Budget prestazioni:** `docs/perf-budget.md` (nucleo a riposo < 1% CPU, tray < 30 MB, finestra < 200 MB WebView2 compresa); si misura a ogni milestone con `scripts/measure-footprint.ps1`

## Struttura

- `crates/oma-core`: modello dati, scheduler/worker, merge per fonte, storico. Niente codice Windows.
- `crates/oma-win`: provider Windows (PDH, D3DKMT, DXGI, NVML, NVAPI, ADL, IGCL, dischi, rete). Tutto il codice specifico di Windows sta qui.
- `crates/oma-ipc`: tipi del protocollo, codifica MessagePack e framing verso `oma-service`; portabile, senza codice Windows.
- `app/src-tauri` (crate `oma-app`): shell Tauri 2.11 (comandi, tray, finestra, modalità sicura).
- `app/src-tauri/nsis`: template NSIS proprio (`installer.nsi`, copiato da `upstream-2.11.5.nsi` di tauri-cli 2.11.5 e modificato solo sulle righe marcate `; OMA`), i nostri hook e le sezioni dei sensori avanzati in `oma.nsh`, e lo SHA-256 fissato di PawnIO (`pawnio.sha256`).
- `app/`: UI Svelte 5 + TypeScript 6, test Vitest, i18n `en.json`/`it.json` con le stesse chiavi.
- `service/`: servizio Windows `oma-service` (.NET 10) con LibreHardwareMonitorLib e i suoi test (`OpenMonitorAdvanced.Service`, `OpenMonitorAdvanced.Service.Tests`).
- `protocol/fixtures/`: messaggi MessagePack di riferimento condivisi tra i test Rust e .NET del protocollo.

## Comandi

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p oma-win -- --include-ignored   # test hardware (RTX 4080 + iGPU AMD su questa macchina)
cd app && pnpm test && pnpm check && pnpm build
cd app && pnpm tauri dev                     # app in sviluppo; pnpm dev = solo UI nel browser con backend finto
dotnet test service/OpenMonitorAdvanced.slnx # test del servizio, dalla radice del repository
pwsh scripts/build-installer-payload.ps1     # pubblica oma-service e mette in staging il setup di PawnIO
pwsh scripts/check-trim-warnings.ps1         # confronta gli avvisi di trimming del servizio con service/trim-allowlist.txt
cd app && pnpm tauri build --bundles nsis    # installer NSIS con app, servizio e PawnIO
```

## Tecniche e convenzioni

- **Flusso di lavoro:** skill superpowers. Brainstorming, poi spec, poi un piano per milestone, poi esecuzione subagent-driven: ogni task ha un implementer e una revisione dedicata, e alla fine una revisione dell'intero branch. Si lavora su un branch `feat/<milestone>` e si fa il merge in `main` in locale; non c'è un remote.
- **TDD:** prima i test che falliscono, poi l'implementazione. Dal codice FFI si estraggono helper puri, testabili senza hardware. I test hardware sono marcati `#[ignore = "requires real Windows hardware"]`.
- **FFI:** binding scritti a mano. Un commento `// SAFETY:` su ogni blocco `unsafe`; un assert di dimensione a compile time per ogni struct FFI. Le DLL dei vendor si caricano solo da System32 (`dynlib::Library`) e non si scaricano mai.
- **Licenze:** nessun header proprietario (NVML, ADL, IGCL) e nessun testo copiato da essi. Le attribuzioni vanno in `THIRD_PARTY_NOTICES.md`; nei nostri sorgenti niente tag SPDX di terzi.
- **Contratto Rust↔UI:** id dei sensori nella forma `<device_id>/<kind>/<name>`, etichette come chiavi i18n.
- **Protocollo IPC (`crates/oma-ipc`, `service/OpenMonitorAdvanced.Service/Protocol/`):** mai `skip_serializing_if` sui tipi del protocollo, perché le chiavi devono essere sempre presenti (`nil` per gli assenti, §6 della spec); le fixture di `protocol/fixtures/` si rigenerano solo con `OMA_WRITE_FIXTURES=1`, a thread singolo.
- **Stile:** codice, commenti e commit in inglese (conventional commits); documentazione e prosa dei piani in italiano con gli accenti corretti.
- Fine riga LF ovunque (`.gitattributes`).
- **Verifiche dal vivo:** mai clic sintetici o UI Automation sul desktop, perché l'utente usa il PC mentre gli agenti lavorano. Le azioni su tray e finestre si chiedono all'utente.
- **Ricerche nel filesystem:** mai una ricerca a tutto il disco (`find /`, `Get-ChildItem C:\ -Recurse`): può restare bloccata per ore. Restringere sempre l'ambito: pacchetti NuGet in `$USERPROFILE/.nuget/packages/<id>/<versione>/`, crate Rust in `$USERPROFILE/.cargo/registry/src/`, il repository o `node_modules`.

## Skill e strumenti installati (usali quando servono)

Prima di agire controlla se una skill copre il lavoro e, se sì, invocala con lo strumento `Skill`: le skill hanno la precedenza sul comportamento predefinito, le istruzioni dell'utente sulle skill.

- **Processo (superpowers):**
  - `superpowers:brainstorming` prima di progettare una funzionalità o cambiare un comportamento;
  - `superpowers:writing-plans` per il piano di una milestone;
  - `superpowers:subagent-driven-development` (o `superpowers:executing-plans`) per eseguirlo;
  - `superpowers:using-git-worktrees` e `superpowers:finishing-a-development-branch` per aprire e chiudere il branch;
  - `superpowers:test-driven-development` per scrivere codice;
  - `superpowers:systematic-debugging` davanti a un bug o a un test che fallisce, prima di proporre correzioni;
  - `superpowers:verification-before-completion` prima di dichiarare qualcosa finito o di fare commit;
  - `superpowers:requesting-code-review` e `superpowers:receiving-code-review` per le revisioni.
- **Codice:**
  - `graphify` per orientarsi nel codice (vedi sotto);
  - il plugin LSP `rust-analyzer` per definizioni, riferimenti e diagnostiche nel codice Rust; richiede il componente `rust-analyzer` della toolchain 1.90.0 (`rustup component add rust-analyzer --toolchain 1.90.0`);
  - `code-review`, `simplify` e `security-review` per rivedere le modifiche.
- **Interfaccia:** `frontend-design:frontend-design` per nuove schermate o componenti Svelte; `dataviz` per grafici, KPI e palette delle serie.
- **Documentazione delle librerie:** il server MCP `context7` (Tauri, Svelte, uPlot, crate `windows`, Vitest…) invece di andare a memoria.
- **App dal vivo:** `run` per avviarla e controllare una modifica, rispettando la regola sulle verifiche dal vivo (niente input sintetico).
- **Subagent:** non ereditano questa sezione. Nei brief nomina le skill che devono usare (per esempio TDD, systematic-debugging) e i comandi graphify.

## graphify (obbligatorio)

Questo progetto ha un knowledge graph in `graphify-out/`, installato solo per questo PC: skill in `.claude/skills/graphify`, hook in `.claude/settings.local.json`, file esclusi da git.

- **Per domande sul codice** (architettura, "dove sta X", "chi chiama Y", "cosa si rompe se cambio Z"), usa il grafo **prima** di grep o di letture a tappeto:
  - `graphify query "<domanda>"`;
  - `graphify explain "<simbolo>"`;
  - `graphify path "<A>" "<B>"`;
  - aggiungi `--budget 4000` se la risposta è troncata.

  Il grafo individua i file, non risponde: dopo leggi il codice.
- **`graphify-out/GRAPH_REPORT.md`:** solo per una revisione ampia dell'architettura.
- **Dopo modifiche al codice**, aggiorna il grafo con `PYTHONHASHSEED=0 graphify update .` (solo AST, nessun costo di API). La variabile serve su questo PC: senza, `graphify update` va in crash (access violation nella ri-esecuzione con `os.execvpe`).
- **I subagent non ereditano queste istruzioni:** metti i comandi `graphify query`/`explain`/`path` in ogni brief che richiede di trovare o capire codice.
