# M7d — Editor, profili e benchmark: piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** comporre i profili dell'overlay in una finestra editor a griglia, con un'anteprima reale disegnata da `oma-overlay`, gestirli (salva, duplica, rinomina, elimina, importa, esporta), catturare i benchmark per frame con riepilogo e storico, e preparare la release 0.5.0.

**Architecture:**
- **`oma-core` (puro):**
  - `overlay::write`: scrittura minima dei profili (solo ciò che differisce dal predefinito) e nomi unici (D1);
  - `frames::session`: accumulatore della sessione di benchmark e riepilogo; `StutterCounter` incrementale condiviso con `stutter` (D2);
  - `overlay::geometry`: posizione dell'overlay con il riquadro del benchmark, più la fixture di geometria condivisa con l'editor (D3).
- **`oma-ipc::overlay`:** protocollo v2, con il messaggio `Benchmark` (D4).
- **`oma-win`:** elenco dei font DirectWrite e UUID v4 pubblico (D5).
- **`oma-overlay`:** modo anteprima `--preview` in una finestra normale (D6); badge `● REC`, riquadro di riepilogo e scala automatica che dimentica i picchi (D7).
- **App (Rust):**
  - archivio dei profili su disco e comandi dell'editor (D8);
  - controller: profilo in anteprima, frame sintetici, dati per la tela (D9);
  - anteprima e finestra dell'editor collegate: secondo `OverlayHost`, finestra Tauri, tray, eventi (D10);
  - benchmark: registratore puro e file (D11); collegamento, scorciatoia e comandi (D12).
- **UI (Svelte):** fondamenta dell'editor (D13), tela e tavolozza (D14), barra, proprietà, anteprima e uscita (D15), Impostazioni › Benchmark e ritocchi di Impostazioni › Overlay (D16).
- **Chiusura:** documenti, misure e bump alla 0.5.0 (D17); prove dal vivo con l'utente (D18).

**Tech Stack:** Rust 1.90 (workspace `rust-version` 1.85), crate `windows` 0.62, Tauri 2.11 (con `tauri-plugin-dialog` 2.7 già presente) + Svelte 5 + TypeScript 6 + Vitest, Canvas2D, NSIS, PowerShell 7 + Pester 5.7.1.

**Spec:** `docs/superpowers/specs/2026-10-04-m7-manutenzione-overlay-design.md`:
- §6.5, §6.6, §7 e §8 interi; dal §5.4 i messaggi `Preview`, `Benchmark` e `Fonts` (ripensati da DD1–DD3);
- §9, §10, §11, §12, §13 e §15 per la M7d.

Base: la M7c in `main` (piano `2026-10-05-m7c-overlay.md`, decisioni DP1–DP17, che restano valide dove questo piano non le cambia).

**Branch:** `feat/m7d-editor-benchmark` da `main` (D1 passo 1); merge in `main` in locale alla fine. Push, tag e pubblicazione della 0.5.0 solo su richiesta dell'utente (D11 della spec).

**Esecuzione:**
- **Subagent-driven:** un implementer e una revisione per task, poi la revisione dell'intero branch.
- **Revisioni in più:** `ffi-safety-reviewer` dopo D5 e D6; `security-review` dopo D8 e D12 (percorsi da id ricevuti dalla UI, file importati); `frontend-design:frontend-design` prima di D13–D16.
- **Prove dal vivo:** D18, con l'utente.

## Global Constraints

- **Lingua e formato:** codice, commenti e messaggi di commit in inglese (conventional commits); documentazione e prosa in italiano con gli accenti corretti. Fine riga LF ovunque. Ogni commit termina con `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.
- **Il gioco non si tocca (§10):** nessuna iniezione, nessun hook, nessun handle sul processo del gioco, nessun input sintetico. Il nome del gioco arriva da `PresentingProcesses`.
- **Divieti per gli agenti:**
  - mai clic sintetici, UI Automation o tasti inviati al desktop, al tray, alle finestre dell'app o ai giochi;
  - mai installer eseguiti, mai test Pester `Integration`, mai comandi elevati;
  - mai catture dal vivo con PresentMon né sessioni ETW: i test usano frame finti o sintetici;
  - mai ricerche a tutto il disco;
  - niente loop di test ripetuti né generatori di carico: una suite completa per verifica va bene; prima di un carico pesante si avvisa l'utente.
- **Risparmio (preferenza dell'utente):** a parità di risultato si sceglie l'opzione che costa meno CPU e memoria. In particolare:
  - il processo dell'anteprima vive solo mentre l'anteprima è aperta;
  - i frame sintetici si generano solo con l'editor aperto e senza bersaglio;
  - i dati per la tela partono solo con l'editor aperto;
  - il benchmark tiene in memoria solo i frametime mostrati (`f32`) e contatori, con un tetto.
- **TDD e debug:** prima il test che fallisce (`superpowers:test-driven-development`); davanti a un test che fallisce senza un motivo chiaro, `superpowers:systematic-debugging`.
- **graphify:** ogni brief riporta `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"` per orientarsi. Dopo le modifiche al codice: `PYTHONHASHSEED=0 graphify update .`.
- **FFI Rust:** un commento `// SAFETY:` per ogni blocco `unsafe`; un assert di dimensione a compile time per ogni struct FFI scritta a mano; helper puri testati senza hardware; i test che chiedono GPU, desktop o un binario compilato sono `#[ignore = "requires real Windows hardware"]`.
- **Protocollo dell'overlay:** mai `skip_serializing_if`, `nil` per gli assenti, enumerati come stringhe, niente `deny_unknown_fields` sui tipi del protocollo. I profili restano `deny_unknown_fields` (§6.5).
- **Dipendenze:** nessun nuovo crate e nessun nuovo pacchetto npm. Il trascinamento è fatto a mano con gli eventi pointer, la tela è Canvas2D (DD4). L'unica feature nuova del crate `windows` è `Win32_Graphics_DirectWrite` in `oma-win` (D5). `pwsh scripts/generate-licenses.ps1 -Check` deve passare.
- **Nomi fissi:**
  - finestra Tauri `overlay-editor`, caricata da `index.html?window=overlay-editor`;
  - capability `app/src-tauri/capabilities/editor.json` per la sola finestra `overlay-editor`;
  - argomento `--preview` di `oma-overlay.exe`, codice d'uscita `5` (`EXIT_CLOSED`) quando l'utente chiude l'anteprima;
  - estensione d'export `.omaoverlay.json`;
  - sottocartella `benchmarks` della cartella del log CSV (§8);
  - eventi `overlay-editor-data` e `overlay-preview`.
- **Budget (§11):** l'editor aperto sta come la finestra principale (< 200 MB con WebView2); i limiti della M7c restano (in gioco `oma-overlay` < 70 MB). L'anteprima è un secondo processo `oma-overlay` aperto solo su richiesta: si misura e si annota, senza un limite nuovo.
- **i18n:** stesse chiavi in `app/src/lib/i18n/en.json` e `it.json`; ogni chiave letta da Rust in `RUST_KEYS` (`app/src-tauri/src/i18n.rs`); i testi esatti sono nelle tabelle dei task D10, D12, D13, D15 e D16.

## Decisioni del piano

Fissano i punti del §15 per la M7d e le scelte che la spec lascia aperte. Il revisore le tratta come requisiti.

| # | Decisione | Perché |
|---|---|---|
| DD1 | **Font (§7.3, §15): un comando DirectWrite in `oma-win`** (`system_font_families`), chiamato dall'app su richiesta dell'editor e tenuto in cache per la sessione. Il messaggio `Fonts` del §5.4 non si fa. | Non serve avviare `oma-overlay` per aprire l'editor, e niente richiesta e risposta sulla pipe. |
| DD2 | **Anteprima (§7.4) = un secondo processo `oma-overlay.exe --pipe <nome> --preview`**, con un suo `OverlayHost`, invece di una seconda finestra nello stesso processo. In modo anteprima la finestra è normale (non in primo piano, non trasparente ai clic, sfondo scuro), l'area è la sua area client e `SetPlacement` si ignora. Chiusa dall'utente, il processo esce con `5` e l'host la tratta come «non voluta», non come una caduta. Il messaggio `Preview` del §5.4 non si fa. | Il ciclo della finestra di `oma-overlay` resta a una finestra sola; l'host, il controllo del PID e il riavvio si riusano. Il costo (un dispositivo D3D11 in più) c'è solo mentre l'anteprima è aperta. |
| DD3 | **Protocollo dell'overlay v2:** `OVERLAY_PROTOCOL_VERSION = 2`, con il solo messaggio nuovo `Benchmark`. | App e overlay escono nello stesso setup, ma un `oma-overlay.exe` vecchio in `target\debug\` deve dare «Componente overlay di un'altra versione», non messaggi scartati. |
| DD4 | **Tela (§15): Canvas2D; trascinamento con gli eventi pointer scritti a mano** (`setPointerCapture`), nessuna libreria. La geometria (`cell_px`, `footprint`, `place`, `block_px`) è portata in TypeScript e tenuta allineata da una fixture condivisa (`testdata/overlay/geometry-cases.json`) letta dai test Rust e Vitest. | Nessuna dipendenza nuova; la fixture rende «la geometria di `oma-core::overlay`» verificabile. |
| DD5 | **Il riquadro del benchmark** (`● REC mm:ss` e riepilogo finale, §8) sta nella finestra dell'overlay, attaccato all'ingombro del profilo: sotto per le ancore in alto e al centro, sopra per quelle in basso, allineato al lato orizzontale dell'ancora. La finestra diventa l'unione dei due rettangoli, tenuta nell'area. Il REC è largo 10 celle e alto 2; il riepilogo 18×8. | La finestra resta piccola (costo e piani MPO); un angolo dell'area lontano dal profilo renderebbe la finestra grande quanto lo schermo. |
| DD6 | **Il benchmark richiede l'overlay acceso**, perché senza l'overlay il motore dei frame è spento e non c'è un bersaglio. Con l'overlay spento il pulsante è disattivato con una spiegazione; la scorciatoia dà l'avviso «nessun gioco in primo piano». Un gioco in `blockedGames` si misura comunque (§9), senza badge. | Il §8 chiede un bersaglio; accendere il motore di nascosto cambierebbe il costo dell'overlay spento (§11). |
| DD7 | **Dati della sessione di benchmark:** solo la swapchain principale del bersaglio (quella del controller); in memoria i frametime mostrati come `f32` e contatori. Tetto: 60 minuti (§8) e `MAX_SESSION_FRAMES = 3_600_000` frame mostrati; oltre, la cattura si ferma con il motivo `limit`. Il CSV si scrive dal thread del controller con un `BufWriter` di 64 KiB. | 3,6 M di `f32` sono circa 14 MB nel caso peggiore (1000 FPS per un'ora). Le scritture sono piccole (10 Hz); un thread di scrittura dedicato si aggiunge solo se le prove mostrano ritardi. |
| DD8 | **Rendered FPS della sessione:** la cascata del §4.5 sull'intera sessione: tipo di frame del driver se c'è almeno un frame generato con tipo noto; altrimenti i marcatori PCL (`(ultimo − primo id) / tempo`); altrimenti nil. L'euristica «FG?» non entra nel riepilogo. | Il riepilogo riporta numeri, mai stime (§4.5). |
| DD9 | **«Usa ora» (§7.4):** salva il profilo e lo rende il profilo attivo fino al prossimo cambio di bersaglio, con lo stesso meccanismo di «Profilo successivo». Per renderlo stabile restano «Associa» e il profilo predefinito. | Non cambia le impostazioni dell'utente di nascosto; è coerente con la correzione di W3 (un cambio delle impostazioni annulla la scelta). |
| DD10 | **Profili integrati nell'editor:** si aprono in sola lettura; «Duplica» li lega per ruolo allo schema di questo PC (§6.6) e crea un profilo utente con il nome tradotto più « (2)» se serve. | §6.6. |
| DD11 | **Nomi unici:** `unique_name(existing, wanted)` aggiunge « (2)», « (3)»… al primo nome libero, senza distinguere maiuscole e minuscole; vale per import, duplica e salva come. | §6.5 lo chiede per l'import; lo stesso per gli altri casi evita due profili indistinguibili nel selettore. |
| DD12 | **Posizione dell'editor:** `overlay.editorBounds { x, y, width, height }` (pixel fisici, `null` di predefinito) in `settings.json`. Si aggiorna in memoria a ogni `Moved` e `Resized` e si salva una volta sola a `Destroyed`. All'apertura si usa se interseca un monitor attuale, altrimenti la finestra si centra. | Nessun plugin nuovo; una sola scrittura per apertura. |
| DD13 | **Uscita con modifiche non salvate (§7.2):** chiudere la finestra dell'editor e «Esci» dal tray chiedono salva, scarta o annulla. `--quit` (installer, `oma-app.exe --quit`) non chiede: l'installer deve poter chiudere l'app. | Il tray è l'uscita normale dell'utente; l'installer non può aspettare una risposta. |
| DD14 | **Dati della tela (§7.3):** con l'editor aperto il controller emette `overlay-editor-data` con le metriche (a `textHz`) e i frametime nuovi (a 10 Hz): del bersaglio se c'è, altrimenti dei frame sintetici (DD15). I valori dei sensori la tela li prende dalla `LiveStore` e dallo storico che la UI ha già. | Nessun calcolo delle metriche in TypeScript. |
| DD15 | **Frame sintetici:** con l'editor aperto e senza bersaglio, il controller genera un secondo alla volta `synthetic(seed, &EDITOR_SYNTHETIC, 1.0)` traslato nel tempo, con `EDITOR_SYNTHETIC = { base_fps: 72, fg_factor: 2, jitter_ms: 0.8, stutter_every: Some(90), pcl: true, gpu_busy_ratio: Some(0.9) }`, in una `FrameWindow` a parte di 10 s; lo stato dei frame verso tela e anteprima vale `running`. | Grafici, moltiplicatore, soglie e `visibleIf` si vedono in movimento (§7.3) senza toccare la finestra dei frame reali. |
| DD16 | **Scala automatica di `meter` e `gauge`** (voce aperta della M7c): il massimo automatico è il massimo degli ultimi 60 s, non il picco di sempre. Vale nell'overlay e nella tela. | Un picco isolato non lascia la scala larga per sempre. |

## Review Focus

1. **Modifiche perse.** Chiudere l'editor, o uscire dal tray, con un profilo modificato e non salvato. Atteso: una domanda con salva, scarta e annulla; «annulla» lascia tutto com'era; `--quit` non chiede. Test: D15, `closing_with_changes_asks`, `cancel_keeps_the_editor_open`; D10, `tray_quit_with_a_dirty_editor_asks_first`, `quit_flag_never_asks`.
2. **Import di un file rotto od ostile:** 50 MB, 10.000 blocchi, chiavi sconosciute, `NaN`, un nome già usato, un file che si chiama `builtin-gaming.omaoverlay.json` o che contiene un id. Atteso: un messaggio d'errore, nessun file scritto, oppure un profilo nuovo con id nuovo e nome unico. Test: D8, `import_rejects_oversized_and_invalid_files_without_writing`, `import_assigns_a_new_id_and_a_unique_name`.
3. **Id costruiti dalla UI** per aprire o eliminare file (`..\..\Windows\x`, percorsi assoluti, nomi con `:` o `/`): rifiutati, nessun file toccato fuori dalla cartella dei profili o dei benchmark. Test: D8, `profile_ids_outside_the_uuid_form_are_rejected`; D11, `benchmark_ids_with_separators_are_rejected`.
4. **Benchmark che finisce male:** il gioco si chiude, la cartella del log non esiste o è piena, la sessione supera 60 minuti o il tetto dei frame, l'app esce durante la cattura. Atteso: CSV chiuso e leggibile, `.json` del riepilogo scritto quando ci sono frame, stato `error` con il motivo, nessun file vuoto lasciato. Test: D11, `stops_after_ten_seconds_without_target`, `stops_at_sixty_minutes`, `stops_at_the_frame_cap`, `write_failure_ends_with_an_error_and_no_summary_if_empty`; D12, `shutdown_finishes_a_running_capture`.
5. **Anteprima chiusa, caduta o lasciata aperta:** l'utente chiude la finestra dell'anteprima, il processo cade più volte, l'editor si chiude con l'anteprima aperta, l'overlay è spento. Atteso: nessun riavvio dopo una chiusura dell'utente, il pulsante dell'editor torna «Anteprima», nessun processo d'anteprima senza editor, l'overlay in gioco non ne risente. Test: D10, `user_closing_the_preview_is_not_a_crash`, `closing_the_editor_stops_the_preview`; D9, `preview_does_not_change_the_in_game_overlay`.

---

### Task D1: `oma-core::overlay`, scrittura minima e nomi unici

**Files:**
- Create: `crates/oma-core/src/overlay/write.rs`
- Modify: `crates/oma-core/src/overlay/mod.rs`

**Interfaces:**
- Consumes: `Profile`, `Block` e i predefiniti serde di `crates/oma-core/src/overlay/profile.rs`; `parse_profile`.
- Produces:
  - **`pub fn profile_to_json(profile: &Profile) -> String`:** JSON compatto con solo ciò che differisce dal predefinito (§6.3). Sempre presenti: `format`, `name`, `blocks` e, per ogni blocco, `id`, `rect`, `source`, `kind`. Il resto si toglie quando è uguale al valore che il deserializzatore darebbe in sua assenza, a ogni livello di annidamento (per esempio `style.valueStyle.size` resta, `style.valueStyle.font` predefinito no). Approccio: `serde_json::to_value` del profilo e, per confronto, del valore predefinito di ogni struct; si tolgono le chiavi uguali, ricorsivamente sugli oggetti; gli array (`blocks`, `thresholds`) non si confrontano come un tutto ma elemento per elemento con il predefinito del loro tipo.
  - **`pub fn unique_name(existing: &[&str], wanted: &str) -> String`:** `wanted` se libero, altrimenti `wanted (2)`, `wanted (3)`…; il confronto ignora maiuscole e minuscole; un `wanted` che finisce già con ` (n)` parte da quel numero più uno.
  - **`pub fn new_block_id(existing: &[&str]) -> String`:** `b1`, `b2`… il primo libero.

- [ ] **Step 1: committare il piano su `main` e creare il branch**

```bash
git add docs/superpowers/plans/2026-10-05-m7d-editor-benchmark.md
git commit -m "docs: add the M7d editor and benchmark plan"
git switch -c feat/m7d-editor-benchmark
```

- [ ] **Step 2: test che falliscono:**
  - `minimal_json_round_trips_every_builtin`: per i quattro `builtin_profile(..)` sulla fixture `this-machine-schema.json`, `parse_profile(&profile_to_json(&p)) == Ok(p)`;
  - `minimal_json_omits_defaults`: un profilo con un solo blocco testo tutto predefinito dà esattamente `{"format":1,"name":"x","blocks":[{"id":"a","rect":{"x":0,"y":0,"w":10,"h":2},"source":{"frames":"fps-displayed"},"kind":"text"}]}`;
  - `minimal_json_keeps_changed_nested_fields`: `style.valueStyle.size = 18.0` compare, `style.valueStyle.font` no;
  - `minimal_json_keeps_non_default_thresholds_in_order`;
  - `unique_name_appends_the_first_free_number`: `["Gaming", "gaming (2)"]` e `"Gaming"` danno `"Gaming (3)"`; `"Nuovo"` libero resta `"Nuovo"`; `"Gaming (2)"` con `["Gaming (2)"]` dà `"Gaming (3)"`;
  - `new_block_id_skips_used_ids`.
- [ ] **Step 3:** `cargo test -p oma-core overlay::write`. Atteso: FAIL.
- [ ] **Step 4:** implementare.
- [ ] **Step 5:** `cargo test -p oma-core`, `cargo clippy -p oma-core --all-targets -- -D warnings`. Atteso: PASS.
- [ ] **Step 6: commit** `feat(core): write overlay profiles with only non-default values`.

### Task D2: `oma-core::frames`, sessione del benchmark e stutter incrementale

**Files:**
- Create: `crates/oma-core/src/frames/session.rs`
- Modify: `crates/oma-core/src/frames/metrics.rs`, `crates/oma-core/src/frames/mod.rs`

**Interfaces:**
- Consumes: `FrameSample`, `FrameKind`, `lows`, `LowDefinition`, `Stutter` (`frames`).
- Produces:
  - **`pub struct StutterCounter`** in `metrics.rs`: `new()`, `push(&mut self, t_s: f64, ft_ms: f64)` per i soli frame mostrati, `result(&self) -> Stutter`. Stessa regola di oggi (2,5 × mediana dei 2 s precedenti e più di 8 ms sopra, almeno 10 frame di storia). `stutter(frames)` diventa un ciclo su `StutterCounter`, con lo stesso risultato.
  - **`pub const MAX_SESSION_FRAMES: usize = 3_600_000`.**
  - **`pub struct SessionAccumulator`:**
    - `new()`;
    - `push(&mut self, f: &FrameSample) -> bool`: falso quando i frame mostrati hanno raggiunto `MAX_SESSION_FRAMES` (il frame non entra);
    - `frames(&self) -> u64`, `duration_s(&self) -> f64` (dal primo all'ultimo `t_s`);
    - `summary(&self) -> Option<SessionSummary>`: `None` senza frame mostrati.
    
    Tiene: frametime mostrati (`Vec<f32>`), somma dei frametime mostrati, conteggi (totali, mostrati, generati, dell'app), primo e ultimo `pcl_frame_id` con i loro `t_s`, somme e conteggi delle due latenze, minimo e massimo del frametime mostrato, uno `StutterCounter`.
  - **`pub struct SessionSummary`** (serde camelCase, `Serialize` e `Deserialize`, tutte le chiavi presenti):
    - `duration_s: f64`;
    - `frames_total: u64`, `frames_displayed: u64`, `frames_generated: u64`;
    - `fps_displayed: f64` (`1000 · N / Σft`, §4.4);
    - `fps_rendered: Option<f64>`, `rendered_source: Option<String>` (`XeSS-FG`, `AFMF`, `FG` o `Reflex`, DD8);
    - `lows_integral: SummaryLows`, `lows_percentile: SummaryLows`, con `SummaryLows { one_percent: f64, point_one_percent: f64 }`;
    - `frametime_min_ms: f64`, `frametime_max_ms: f64`;
    - `stutter_count: u32`, `stutter_percent: f64`;
    - `fg_multiplier: Option<f64>` (mostrati ÷ renderizzati);
    - `latency_pc_ms: Option<f64>`, `latency_display_ms: Option<f64>`.

- [ ] **Step 1: test che falliscono:**
  - `stutter_counter_matches_stutter_on_fixtures`: sulle nove fixture di `testdata/presentmon/` e su `synthetic(7, …)` con stutter, stesso conteggio e stessa percentuale;
  - `summary_matches_window_metrics_on_a_fixture`: sulla fixture senza FG, `fps_displayed` uguale a `displayed_fps(&tutti)` entro 1e-9, i low uguali a `lows(&ft, def)` per le due definizioni;
  - `summary_rendered_from_pcl_ids`: sulla fixture DLSS FG con PCL, `rendered_source = "Reflex"` e `fg_multiplier` entro ±0,05 dal valore del README;
  - `summary_rendered_none_without_evidence`: fixture FSR FG senza PCL, `fps_rendered = None`;
  - `summary_none_without_displayed_frames`;
  - `push_refuses_beyond_the_cap`: con un tetto di prova (funzione `with_cap(n)` solo per i test) il frame `n+1` dà falso;
  - `summary_serializes_every_key`: `serde_json::to_value` ha tutte le chiavi, `null` compresi.
- [ ] **Step 2:** `cargo test -p oma-core frames`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): benchmark session summary and an incremental stutter counter`.

### Task D3: geometria del riquadro del benchmark e fixture condivisa

**Files:**
- Modify: `crates/oma-core/src/overlay/geometry.rs`
- Create: `testdata/overlay/geometry-cases.json`, `testdata/overlay/README.md`
- Test: in `geometry.rs`, più un test che legge la fixture.

**Interfaces:**
- Consumes: `place`, `footprint`, `cell_px`, `PxRect`, `Anchor` (C3).
- Produces:
  - **`pub struct Placed { pub window: PxRect, pub profile: Option<PxRect>, pub extra: Option<PxRect> }`.**
  - **`pub fn place_with_extra(profile: &Profile, area: PxRect, dpi: u32, extra_cells: Option<(u32, u32)>) -> Option<Placed>`:**
    - senza `extra_cells` vale `place(...)` con `window == profile`;
    - con `extra_cells` (DD5) il riquadro è largo e alto `extra_cells × cell_px`, sotto l'ingombro per le ancore `top-*`, `left`, `center`, `right`, sopra per le `bottom-*`, allineato a sinistra, al centro o a destra come l'ancora; `window` è l'unione, poi tenuta dentro `area` spostando tutto insieme;
    - un profilo senza blocchi con `extra_cells` mette il solo riquadro al punto d'ancora con lo scostamento;
    - `None` senza blocchi e senza `extra_cells`.
  - **`testdata/overlay/geometry-cases.json`:** un array di casi `{ "name", "profile": <profilo JSON>, "area": {x,y,w,h}, "dpi", "extraCells": [w,h] | null, "expected": { "window", "profile", "extra" } }`, almeno: le nove ancore con area 1920×1080 e DPI 96; `scale` 1,5 con DPI 144; un profilo che sborda e viene tenuto dentro; i due casi del riquadro (ancora in alto a sinistra e in basso a destra); un profilo vuoto con riquadro. Il README spiega il formato e che la fixture la leggono `oma-core` e Vitest (D13).

- [ ] **Step 1: test che falliscono:**
  - `extra_box_goes_below_top_anchors_and_above_bottom_anchors`;
  - `extra_box_aligns_with_the_anchor_side`;
  - `window_is_the_union_kept_inside_the_area`;
  - `empty_profile_with_extra_places_the_box_alone`;
  - `geometry_cases_fixture_matches`: legge `testdata/overlay/geometry-cases.json` (percorso da `CARGO_MANIFEST_DIR`) e confronta ogni caso.
- [ ] **Step 2:** `cargo test -p oma-core overlay::geometry`. Atteso: FAIL.
- [ ] **Step 3:** implementare; i valori attesi della fixture si scrivono a mano dai casi già fissati dai test di `place` della M7c, più quelli nuovi calcolati sul foglio.
- [ ] **Step 4:** `cargo test -p oma-core`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(core): place the benchmark box next to the profile; shared geometry fixture`.

### Task D4: protocollo dell'overlay v2, il messaggio `Benchmark`

**Files:**
- Modify: `crates/oma-ipc/src/overlay.rs`

**Interfaces:**
- Consumes: `SessionSummary` (D2) solo come forma: il protocollo ha un suo tipo, senza dipendere da `oma-core`.
- Produces:
  - `OVERLAY_PROTOCOL_VERSION = 2` (DD3).
  - **`OverlayMessage::Benchmark(BenchmarkOverlay)`**, con:
    - `recording_s: Option<u32>`: secondi di cattura, nil se non si registra;
    - `summary: Option<WireBenchmarkSummary>`: il riepilogo da mostrare, nil per toglierlo;
    - `WireBenchmarkSummary { fps_displayed: f64, low_one_percent: f64, low_point_one_percent: f64, stutter_count: u32, stutter_percent: f64 }`, con i low della definizione integrale.
  - `validate()` rifiuta i `f64` non finiti di `WireBenchmarkSummary`.

- [ ] **Step 1: test che falliscono:**
  - `benchmark_round_trips`, nei due casi (registrazione e riepilogo);
  - `benchmark_absent_fields_are_nil_on_the_wire`;
  - `validate_rejects_a_non_finite_summary`;
  - `version_one_hello_is_incompatible`.
- [ ] **Step 2:** `cargo test -p oma-ipc`. Atteso: FAIL.
- [ ] **Step 3:** implementare. `oma-overlay` (`state.rs`) ignora `Benchmark` fino a D7 con un ramo vuoto, così il workspace compila.
- [ ] **Step 4:** `cargo test --workspace`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ipc): overlay protocol v2 with the benchmark message`.

### Task D5: `oma-win`, font di sistema e UUID v4

**Files:**
- Create: `crates/oma-win/src/fonts.rs`
- Modify: `crates/oma-win/src/lib.rs`, `crates/oma-win/Cargo.toml` (feature `Win32_Graphics_DirectWrite`), `crates/oma-win/src/overlay_pipe.rs`

**Interfaces:**
- Produces:
  - **`pub fn system_font_families() -> windows::core::Result<Vec<String>>`:** `DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)`, `GetSystemFontCollection(false)`, per ogni famiglia `GetFamilyNames` con il nome `en-us` se c'è, altrimenti il primo; elenco ordinato senza distinzione di maiuscole, senza doppioni, al massimo 2048 voci, nomi oltre 64 byte scartati (il limite di `TextStyle.font`).
  - **Puro:** `pub(crate) fn pick_family_name(names: &[(String, String)]) -> Option<String>` (coppie locale-nome): `en-us` senza distinzione di maiuscole, altrimenti il primo.
  - **`pub fn random_uuid_v4() -> std::io::Result<String>`** in `overlay_pipe.rs`, con lo stesso `BCryptGenRandom` di `random_pipe_name` (che la usa).

- [ ] **Step 1: test che falliscono:**
  - `pick_family_name_prefers_en_us`;
  - `pick_family_name_falls_back_to_the_first`;
  - `random_uuid_v4_is_lowercase_8_4_4_4_12` e due chiamate diverse;
  - hardware `#[ignore]`: `system_fonts_include_segoe_ui`.
- [ ] **Step 2:** `cargo test -p oma-win fonts overlay_pipe`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-win`, `cargo test -p oma-win fonts -- --ignored`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(win): list the system font families and expose a random UUID`.
- [ ] **Step 6:** revisione `ffi-safety-reviewer`.

### Task D6: `oma-overlay`, il modo anteprima

**Files:**
- Modify: in `crates/oma-overlay/src/`: `args.rs`, `main.rs`, `window.rs`, `link.rs` (costante `EXIT_CLOSED = 5`), `render/mod.rs` (traslazione e sfondo).

**Interfaces:**
- Consumes: `place` (C3), `Compositor` (C11), `render::draw` (C12).
- Produces:
  - **`Args { pipe: String, preview: bool }`:** `--preview` facoltativo, una volta sola; ogni altro argomento resta un errore.
  - **`window::create_preview(title: &str) -> Result<HWND>`:** classe `OmaOverlayPreview`, `WS_OVERLAPPEDWINDOW`, `WS_EX_NOREDIRECTIONBITMAP`, niente `TOPMOST` né `TRANSPARENT`, area client iniziale 1280×720 logici al DPI del monitor primario, centrata, mostrata con `SW_SHOWNOACTIVATE`. `WM_CLOSE` posta `WM_QUIT` con il codice `EXIT_CLOSED`; `WM_SIZE` e `WM_DPICHANGED` (con il rettangolo suggerito) svegliano il ciclo.
  - **Puro:** `pub(crate) fn preview_ex_style() -> WINDOW_EX_STYLE` e `pub(crate) fn preview_style() -> WINDOW_STYLE`.
  - **Ciclo in modo anteprima** (`main.rs`):
    - il titolo arriva da `strings["previewTitle"]` del `SetProfile` (`SetWindowTextW` quando cambia); prima del primo profilo vale `OpenMonitor Advanced`;
    - `SetPlacement` si ignora; l'area è l'area client (`GetClientRect`) e il DPI `GetDpiForWindow`;
    - la superficie è grande quanto l'area client; ogni frame si pulisce con lo sfondo opaco `#140F1E`, poi il profilo si disegna a `place(profile, client, dpi)` con una traslazione del contesto D2D (`SetTransform`);
    - `hide_from_capture` si ignora;
    - la superficie non si rilascia mai per inattività (la finestra è visibile finché esiste).
  - **Puro:** `pub(crate) fn preview_origin(client: PxRect, placed: PxRect) -> (f32, f32)`.

- [ ] **Step 1: test che falliscono:**
  - `preview_flag_parses`, `preview_flag_twice_is_an_error`;
  - `preview_window_is_not_topmost_nor_click_through`;
  - `preview_origin_is_the_placed_offset`;
  - `exit_closed_is_five`;
  - hardware `#[ignore]`: `preview_window_draws_the_gaming_profile` (finestra d'anteprima creata dal test, un frame presentato senza errori, poi `DestroyWindow`: è una finestra del test, non input sul desktop).
- [ ] **Step 2:** `cargo test -p oma-overlay`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-overlay` e `-- --ignored`, clippy, `cargo build -p oma-overlay`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(overlay): preview mode in a normal window`.
- [ ] **Step 6:** revisione `ffi-safety-reviewer`.

### Task D7: `oma-overlay`, badge del benchmark e scala che dimentica i picchi

**Files:**
- Modify: in `crates/oma-overlay/src/`: `state.rs`, `main.rs` (`placement_rect` con `place_with_extra`), `render/mod.rs`, `render/layout.rs`, `render/shapes.rs`.

**Interfaces:**
- Consumes: `Benchmark` (D4), `place_with_extra` e `Placed` (D3).
- Produces:
  - **`OverlayState.benchmark: Option<BenchmarkOverlay>`**, aggiornato da `Benchmark`; `Changes { layout: true, text: true }` quando cambia la presenza del riquadro, `text: true` quando cambiano solo i secondi.
  - **`pub(crate) fn extra_cells(b: Option<&BenchmarkOverlay>) -> Option<(u32, u32)>`:** `(18, 8)` con un riepilogo, `(10, 2)` durante la registrazione, `None` altrimenti (DD5).
  - **Disegno:** il riquadro usa il pannello del profilo; il REC è `● REC mm:ss` con il punto rosso `#FF3B5C`; il riepilogo ha quattro righe, con le etichette da `strings`: `bench.avg`, `bench.low1`, `bench.low01`, `bench.stutter` e i valori formattati con `format_frame_metric`. Un tempo oltre 59:59 si scrive `60:00`.
  - **Puro:** `pub(crate) fn rec_text(seconds: u32) -> String` («● REC 03:07»).
  - **Scala automatica (DD16):** `value_range` usa il massimo degli ultimi 60 s del valore del blocco (un `StatRing` di 60 s per ogni `meter` e `gauge` con un estremo `auto` che non sia una percentuale), non più `peak`.
  - In modo anteprima il riquadro del benchmark non si disegna.

- [ ] **Step 1: test che falliscono:**
  - `benchmark_message_sets_the_box`, `benchmark_seconds_only_mark_text`;
  - `extra_cells_by_state`;
  - `rec_text_formats_minutes_and_seconds` (0 → «● REC 00:00», 187 → «● REC 03:07», 3600 → «● REC 60:00»);
  - `placement_includes_the_benchmark_box`;
  - `auto_range_forgets_a_spike_after_sixty_seconds`;
  - hardware `#[ignore]`: `renders_the_summary_box_to_a_bitmap`.
- [ ] **Step 2:** `cargo test -p oma-overlay`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-overlay` e `-- --ignored`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(overlay): benchmark badge and summary box; auto range over the last minute`.

### Task D8: app, archivio dei profili e comandi dell'editor

**Files:**
- Create: `app/src-tauri/src/overlay/store.rs` (file dei profili), `app/src-tauri/src/overlay/editor.rs` (comandi Tauri)
- Modify: `app/src-tauri/src/overlay/mod.rs`, `app/src-tauri/src/overlay/profiles.rs` (lettura di un singolo file riusata), `app/src-tauri/src/main.rs` (registrazione dei comandi), `app/src-tauri/build.rs`, `app/src-tauri/capabilities/default.json`, `app/src-tauri/src/i18n.rs`

**Interfaces:**
- Consumes: `profile_to_json`, `unique_name` (D1); `parse_profile`, `MAX_PROFILE_BYTES`, `builtin_profile`, `BuiltinId`, `is_profile_id`; `RealFs::write_atomic` (`settings/mod.rs`); `random_uuid_v4` (D5); `tauri_plugin_dialog`.
- Produces:
  - **`pub struct ProfileStore { dir: PathBuf }`**, con:
    - `load(&self, id: &str, schema: &Schema, lang: Lang) -> Result<EditableProfile, StoreError>`: un integrato legato allo schema con il nome tradotto e `builtin: true`, oppure il file;
    - `save(&self, id: Option<&str>, json: &str) -> Result<String, StoreError>`: valida con `parse_profile`, scrive `profile_to_json` in modo atomico in `<dir>\<id>.json`; senza `id`, un id nuovo e il nome reso unico (DD11); un id integrato è `StoreError::ReadOnly`;
    - `delete(&self, id: &str) -> Result<(), StoreError>`;
    - `duplicate(&self, id: &str, schema: &Schema, lang: Lang) -> Result<String, StoreError>` (DD10);
    - `import(&self, path: &Path) -> Result<String, StoreError>`: legge al massimo `MAX_PROFILE_BYTES + 1` byte, valida, assegna un id nuovo e un nome unico, salva;
    - `export(&self, id: &str, schema: &Schema, lang: Lang, path: &Path) -> Result<(), StoreError>`: scrive `profile_to_json`.
  - **`EditableProfile { id, builtin: bool, json: String }`** (serde camelCase): `json` è il profilo completo, predefiniti compresi, così la UI non deve conoscere i predefiniti per leggere.
  - **`StoreError`:** `InvalidId`, `ReadOnly`, `NotFound`, `Invalid(String)` (il testo di `ProfileError`), `Io(String)`; `key()` dà la chiave i18n e `detail()` il testo.
  - **Id:** ogni comando accetta solo un id integrato conosciuto o un UUID minuscolo (`is_profile_id`); il percorso si costruisce sempre come `dir.join(format!("{id}.json"))`.
  - **Comandi Tauri** (in `editor.rs`, `State<'_, OverlayHandle>` per ricaricare il catalogo dopo ogni scrittura con `Input::ReloadProfiles`):
    - `overlay_load_profile(id) -> Result<EditableProfile, CommandError>`;
    - `overlay_save_profile(id: Option<String>, json: String) -> Result<String, CommandError>`;
    - `overlay_delete_profile(id)`, `overlay_duplicate_profile(id) -> Result<String, CommandError>`;
    - `overlay_import_profile() -> Result<Option<String>, CommandError>`: finestra di dialogo «Apri» con il filtro `*.omaoverlay.json` (e «Tutti i file»), `None` se annullata;
    - `overlay_export_profile(id) -> Result<bool, CommandError>`: dialogo «Salva» con il nome `<nome>.omaoverlay.json`, falso se annullata;
    - `overlay_font_families() -> Vec<String>`: `system_font_families` su un thread bloccante, in cache per la sessione (`OnceLock`); in errore, `["Segoe UI"]` e una riga warn nel log.
  - **`CommandError { key: String, detail: Option<String> }`** (serde camelCase).
  - **Testi:**

    | Chiave | Inglese | Italiano |
    |---|---|---|
    | `editor.error.invalidId` | `Unknown profile.` | `Profilo sconosciuto.` |
    | `editor.error.readOnly` | `Built-in profiles are read-only: duplicate it to edit.` | `I profili integrati sono in sola lettura: duplicalo per modificarlo.` |
    | `editor.error.notFound` | `The profile file no longer exists.` | `Il file del profilo non esiste più.` |
    | `editor.error.invalid` | `Invalid profile: {detail}` | `Profilo non valido: {detail}` |
    | `editor.error.io` | `File error: {detail}` | `Errore del file: {detail}` |
    | `editor.dialog.filter` | `Overlay profile` | `Profilo dell'overlay` |

- [ ] **Step 1: test che falliscono** (cartella temporanea unica per test sotto `std::env::temp_dir()`):
  - `save_writes_minimal_json_atomically_and_reloads`;
  - `save_without_id_assigns_a_uuid_and_a_unique_name`;
  - `builtins_are_read_only`;
  - `profile_ids_outside_the_uuid_form_are_rejected`: `..\..\x`, `C:\x`, `a/b`, `builtin-gaming.json`, stringa vuota, UUID maiuscolo;
  - `import_rejects_oversized_and_invalid_files_without_writing`: 1 MiB + 1 byte, chiavi sconosciute, `1e999`, 257 blocchi; la cartella resta com'era;
  - `import_assigns_a_new_id_and_a_unique_name`;
  - `duplicate_binds_a_builtin_by_role`;
  - `delete_removes_only_that_file`;
  - `export_writes_minimal_json`;
  - `every_registered_command_is_in_the_manifest_and_the_capability`: il test esistente, con i comandi nuovi.
- [ ] **Step 2:** `cargo test -p oma-app overlay::store`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-app`, clippy, `cd app && pnpm test i18n`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): save, duplicate, import and export overlay profiles`.
- [ ] **Step 6:** revisione `security-review` sul diff del task (id dalla UI, file importati).

### Task D9: app, controller con anteprima, frame sintetici e dati per la tela

**Files:**
- Create: `app/src-tauri/src/overlay/editor_feed.rs` (puro: frame sintetici e dati per la tela)
- Modify: `app/src-tauri/src/overlay/controller.rs`, `app/src-tauri/src/overlay/forward.rs`

**Interfaces:**
- Consumes: `synthetic`, `SyntheticProfile`, `FrameWindow`, `read` (`oma-core::frames`); `set_profile`, `used_sensors`, `metrics_message`, `frame_times_since` (C14).
- Produces:
  - **`editor_feed`:**
    - `pub const EDITOR_SYNTHETIC: SyntheticProfile` (DD15);
    - `pub struct SyntheticFeed { window: FrameWindow, generated_to_s: f64, seed: u64 }` con `new()` e `advance(&mut self, now_s: f64)`, che genera i secondi mancanti fino a `now_s`, al massimo 2 s per chiamata (dopo una pausa lunga non recupera tutto);
    - `pub fn window(&self) -> &FrameWindow`.
  - **Nuovi ingressi del `Controller`:**
    - `on_editor(&mut self, open: bool)`;
    - `set_preview(&mut self, profile: Option<Profile>)`: `None` chiude l'anteprima;
    - `on_preview_host(&mut self, state: HostState)`;
    - `use_now(&mut self, id: String)`: come `next_profile`, ma con l'id dato (DD9).
  - **Nuovi campi di `Outputs`:**
    - `want_preview: bool`: vero con un profilo d'anteprima;
    - `preview: Vec<OverlayMessage>`: `SetProfile` del profilo in anteprima (id `preview`, `strings["previewTitle"]` compresa) e gli stessi `Values`, `FrameMetrics` e `FrameTimes` dell'overlay, mentre il processo d'anteprima è `Running`;
    - `editor_data: Option<EditorData>`: con l'editor aperto, a `textHz` le metriche e a 10 Hz i frametime nuovi.
  - **`EditorData { metrics: FrameMetrics, frame_times: Vec<WireFrameTime> }`** (serde camelCase all'esterno; `FrameMetrics` e `WireFrameTime` restano come nel protocollo).
  - **Regole:**
    1. **Sorgente dei frame per tela e anteprima:** il bersaglio, se c'è; altrimenti, con l'editor aperto, il `SyntheticFeed` (stato `running`, DD15). Senza editor il `SyntheticFeed` non esiste.
    2. **Overlay in gioco:** invariato; i frame sintetici non vi arrivano mai.
    3. **`ValuesPlan`:** `used` è l'unione dei sensori del profilo attivo e di quello in anteprima; `wanted` vale `shown || preview_running || editor_open`.
    4. **Dati:** `FrameMetrics` e `FrameTimes` partono quando servono all'overlay, all'anteprima o all'editor, con le stesse cadenze; il calcolo si fa una volta per passo e si inoltra a chi serve.
    5. **Anteprima chiusa dall'utente** (`on_preview_host(Off)` mentre `want_preview`): il profilo d'anteprima si toglie e lo stato lo dice (`OverlayStatus.preview = false`).
  - **`OverlayStatus.preview: bool`** (anteprima aperta), per la UI.
  - **Testo** (in `overlay_strings` come `previewTitle` e in `RUST_KEYS`):

    | Chiave | Inglese | Italiano |
    |---|---|---|
    | `overlay.text.previewTitle` | `Overlay preview` | `Anteprima overlay` |

- [ ] **Step 1: test che falliscono:**
  - `synthetic_feed_is_deterministic_and_bounded`: stessi frame per lo stesso seme; `advance` dopo 30 s di pausa genera al massimo 2 s;
  - `editor_without_target_gets_synthetic_running_metrics`;
  - `editor_with_target_gets_the_target_metrics`;
  - `editor_closed_sends_no_editor_data`;
  - `preview_does_not_change_the_in_game_overlay`: con un gioco in primo piano, aprire l'anteprima non cambia i messaggi `overlay`;
  - `preview_gets_its_own_profile_and_shared_data`;
  - `values_plan_is_the_union_of_active_and_preview`;
  - `preview_closed_by_the_user_is_dropped`;
  - `use_now_lasts_until_target_change`.
- [ ] **Step 2:** `cargo test -p oma-app overlay::controller overlay::editor_feed`. Atteso: FAIL.
- [ ] **Step 3:** implementare. Il controller delega a `editor_feed` e a funzioni piccole: non deve crescere con logica duplicata.
- [ ] **Step 4:** `cargo test -p oma-app`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): preview profile, synthetic frames and canvas data in the overlay controller`.

### Task D10: app, finestra dell'editor, anteprima, tray e uscita

**Files:**
- Modify:
  - in `app/src-tauri/src/`: `window.rs` (`show_editor`), `overlay/host.rs` (modo anteprima ed `EXIT_CLOSED`), `overlay/runner.rs`, `tray.rs`, `main.rs` (sampler e impostazioni anche con l'editor aperto, `--quit`, uscita dal tray), `i18n.rs`;
  - `crates/oma-core/src/settings/overlay.rs`, `decode.rs`, `patch.rs` (`editorBounds`, DD12);
  - `app/src-tauri/build.rs`; nuova `app/src-tauri/capabilities/editor.json`; `app/src/lib/types.ts` e `app/src/lib/backend/mockSettings.ts` (`editorBounds`);
  - `app/src/lib/i18n/en.json` e `it.json`.

**Interfaces:**
- Consumes: D8, D9; `OverlayHost` (C13).
- Produces:
  - **Host:** `OverlayHost::start(exe, mode: OverlayMode, on_state)` con `OverlayMode { Overlay, Preview }`; in `Preview` il figlio riceve anche `--preview`. `Supervisor::on_closed()` (uscita `EXIT_CLOSED`): stato `Off`, voluto falso, nessuna caduta contata.
  - **Runner:** un secondo `OverlayHost` in modo anteprima, creato alla prima anteprima e fermato quando non è più voluto; gli `Outputs.preview` vanno a lui; `want_preview` lo accende e lo spegne. Nuovi `Input`: `Editor(bool)`, `Preview(Option<Profile>)`, `PreviewHost(HostState)`, `UseNow(String)`, `EditorDirty(bool)`. `EditorData` va all'evento `overlay-editor-data`; lo stato dell'anteprima all'evento `overlay-preview` (`{ open: bool }`).
  - **Finestra:** `window::show_editor(app)`:
    - una sola istanza (`get_webview_window("overlay-editor")`: `unminimize`, `show`, `set_focus`);
    - `WebviewUrl::App("index.html?window=overlay-editor")`, titolo da `editor.title`, 1280×800 logici, minimo 1100×700;
    - posizione da `overlay.editorBounds` se interseca un monitor di `available_monitors()`, altrimenti centrata (DD12; puro e testato: `pub(crate) fn bounds_visible(bounds: WindowBounds, monitors: &[PxRect]) -> bool`);
    - `Moved` e `Resized` aggiornano i limiti in memoria, `Destroyed` li salva con `update_with` e manda `Input::Editor(false)` e `Input::Preview(None)`;
    - all'apertura `Input::Editor(true)`.
  - **Uscita (DD13):** «Esci» dal tray con l'editor aperto e `EditorDirty(true)` mostra l'editor ed emette `overlay-editor-quit`; l'editor risponde con `app_quit_confirmed()` dopo «salva» o «scarta». `--quit` esce sempre senza chiedere.
  - **Comandi Tauri nuovi:** `open_overlay_editor()`, `overlay_preview(json: Option<String>) -> Result<(), CommandError>` (valida con `parse_profile`; `None` chiude), `overlay_use_now(id) -> Result<(), CommandError>`, `overlay_editor_dirty(dirty: bool)`, `app_quit_confirmed()`.
  - **Sampler ed eventi:** il tick emette schema e snapshot, e le impostazioni si emettono, quando esiste la finestra `main` **o** `overlay-editor`.
  - **Capability `editor.json`:** `"windows": ["overlay-editor"]`, `core:event:allow-listen`, `core:event:allow-unlisten`, `core:window:allow-destroy`, `core:window:allow-set-focus` e i comandi che l'editor usa (quelli di D8, i quattro qui sopra tranne `open_overlay_editor`, più `get_schema`, `get_settings`, `get_history`, `get_overlay_status`, `update_settings` e gli altri che la `LiveStore` chiama). Il test `every_registered_command_is_in_the_manifest_and_the_capability` legge anche `editor.json` e controlla che ogni permesso di `editor.json` sia un comando registrato.
  - **Impostazioni:** `OverlaySettings.editor_bounds: Option<WindowBounds { x: i32, y: i32, width: u32, height: u32 }>`, predefinito `None`, chiave `editorBounds`; il decoder porta a `null` (con una diagnostica) un `width < 1100`, un `height < 700` o valori oltre ±32768; la patch rifiuta gli stessi casi con `settings.error.range`.
  - **Tray:** voce `overlay-editor` con il testo `tray.overlay.editor`, sotto «Mostra/nascondi overlay», sempre attiva; apre l'editor.
  - **Testi:**

    | Chiave | Inglese | Italiano |
    |---|---|---|
    | `editor.title` | `Overlay editor` | `Editor overlay` |
    | `tray.overlay.editor` | `Overlay editor` | `Editor overlay` |

- [ ] **Step 1: test che falliscono:**
  - `user_closing_the_preview_is_not_a_crash`: `on_closed` cinque volte non porta a `Failed` e non riavvia;
  - `preview_mode_passes_the_preview_flag`;
  - `closing_the_editor_stops_the_preview`;
  - `bounds_visible_needs_an_intersecting_monitor`;
  - `editor_bounds_decode_and_patch_rules`;
  - `tray_editor_item_opens_the_editor`;
  - `tray_quit_with_a_dirty_editor_asks_first`, `quit_flag_never_asks` (logica pura dell'uscita estratta in una funzione);
  - `every_registered_command_is_in_the_manifest_and_the_capability`, esteso a `editor.json`;
  - `rust_keys_exist_in_both_catalogs`.
- [ ] **Step 2:** `cargo test -p oma-app -p oma-core`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo build -p oma-overlay`, poi `cargo test --workspace`, clippy e `cd app && pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: verifica d'avvio dell'agente, senza input sul desktop:** `pnpm tauri dev`; nel log nessun processo d'anteprima e nessun errore; chiusura con `oma-app.exe --quit`.
- [ ] **Step 6: commit** `feat(app): overlay editor window, preview process and tray item`.

### Task D11: app, registratore del benchmark (puro) e file

**Files:**
- Create: `app/src-tauri/src/overlay/benchmark.rs`
- Modify: `app/src-tauri/src/overlay/mod.rs`, `app/src-tauri/src/log/session.rs` (`configured_dir` diventa `pub(crate)`)

**Interfaces:**
- Consumes: `SessionAccumulator`, `SessionSummary` (D2); `oma_core::csv::{escape_field, format_number, local_time, BOM}`; `LogFs` e `LogFile` (`log/fs.rs`), `WriteFailure` (`log/writer.rs`).
- Produces:
  - **Costanti:** `MAX_DURATION_MS = 3_600_000`, `NO_TARGET_STOP_MS = 10_000`, `SUMMARY_SHOW_MS = 10_000`, `MAX_HISTORY = 500`, `MAX_SUMMARY_BYTES = 65_536`, `FLUSH_BYTES = 65_536`.
  - **`pub struct Recorder` (puro):**
    - `start(&mut self, exe: &str, now_ms: u64) -> Result<(), StartError>` (`StartError::NoTarget`, `AlreadyRecording`);
    - `on_frames(&mut self, frames: &[FrameSample], now_ms: u64) -> Vec<String>`: le righe CSV dei frame accettati;
    - `on_target(&mut self, present: bool, now_ms: u64)`;
    - `tick(&mut self, now_ms: u64) -> Option<EndReason>`: `NoTarget` dopo 10 s senza bersaglio, `Limit` a 60 minuti o al tetto dei frame;
    - `stop(&mut self, reason: EndReason) -> Option<SessionSummary>`;
    - `elapsed_s(&self, now_ms: u64) -> Option<u32>`.
  - **`EndReason`:** `User`, `NoTarget`, `Limit`, `Error`, `Shutdown` (serde camelCase).
  - **CSV (§8):** intestazione `qpc_ms,frametime_displayed_ms,frametime_app_ms,frame_type,displayed,pc_latency_ms,gpu_busy_ms`, BOM, righe CRLF; `qpc_ms` relativo al primo frame; numeri con `format_number`, vuoto per nil; `frame_type` con la stringa del protocollo passata da `escape_field`; `displayed` `1` o `0`.
  - **Nomi:** `pub fn file_stem(exe: &str, start: LocalTime) -> String`, che vale `<exe>-<AAAAMMGG-hhmmss>`: dal nome dell'eseguibile si toglie `.exe`, si tengono `[A-Za-z0-9._-]`, il resto diventa `_`, al massimo 64 caratteri, `game` se resta vuoto. In collisione `-2`…`-99` con `create_new`.
  - **`pub struct BenchmarkFiles { dir: PathBuf, fs: Arc<dyn LogFs> }`:**
    - `begin(&self, stem: &str) -> Result<BenchmarkWriter, WriteFailure>`: crea la cartella e il CSV con l'intestazione;
    - `BenchmarkWriter::append(&mut self, lines: &[String]) -> Result<(), WriteFailure>` con il buffer di `FLUSH_BYTES`; `finish(self, summary: Option<&BenchmarkRecord>) -> Result<(), WriteFailure>`: flush e, con un riepilogo, il `.json` accanto scritto in modo atomico; senza frame il CSV vuoto si cancella;
    - `list(&self) -> Vec<BenchmarkEntry>`: i `.json` della cartella, al massimo `MAX_HISTORY` dal più recente (per nome), ognuno al massimo `MAX_SUMMARY_BYTES`; i file illeggibili si saltano;
    - `csv_path(&self, id: &str) -> Result<PathBuf, BenchmarkError>` e `delete(&self, id: &str) -> Result<(), BenchmarkError>` (cancella `.csv` e `.json`).
  - **`BenchmarkRecord`** (il `.json`, camelCase): `{ "format": 1, "game": <exe>, "startedAt": <ISO locale>, "endReason", "summary": SessionSummary }`. **`BenchmarkEntry { id, record }`** dove `id` è il nome senza estensione.
  - **Id (Review Focus 3):** `pub fn is_benchmark_id(id: &str) -> bool`: solo `[A-Za-z0-9._-]`, al massimo 96 caratteri, nessun `..`, deve finire con `-AAAAMMGG-hhmmss` o con quello seguito da `-n`.

- [ ] **Step 1: test che falliscono** (con `fake::MemFs` di `log/fs.rs` e un orologio finto):
  - `start_needs_a_target`;
  - `stops_after_ten_seconds_without_target`, e un ritorno a 9 s non lo ferma;
  - `stops_at_sixty_minutes`;
  - `stops_at_the_frame_cap`;
  - `csv_rows_match_the_spec_columns`: header esatto e una riga con nil;
  - `formula_guard_on_frame_type`;
  - `file_stem_sanitizes_the_exe`: `Control_DX12.exe` → `Control_DX12-20261005-213000`; `../a:b.exe` → `.._a_b-…` non contiene separatori;
  - `name_collision_adds_a_suffix`;
  - `write_failure_ends_with_an_error_and_no_summary_if_empty`;
  - `empty_capture_leaves_no_files`;
  - `history_lists_newest_first_and_skips_bad_files`;
  - `benchmark_ids_with_separators_are_rejected`;
  - `delete_removes_csv_and_json`.
- [ ] **Step 2:** `cargo test -p oma-app overlay::benchmark`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test -p oma-app`, clippy. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): benchmark recorder with per-frame CSV and summary files`.

### Task D12: app, benchmark collegato, scorciatoia e comandi

**Files:**
- Modify: in `app/src-tauri/src/`: `overlay/controller.rs`, `overlay/runner.rs`, `hotkeys.rs`, `main.rs`, `i18n.rs`, `build.rs`, `capabilities/default.json`; `app/src/lib/i18n/en.json` e `it.json`.

**Interfaces:**
- Consumes: D11; `BenchmarkOverlay` (D4).
- Produces:
  - **Controller:**
    - `toggle_benchmark(&mut self, now_ms)`: avvia con il bersaglio presente, altrimenti `ToastRequest::BenchmarkNoTarget`; con una cattura in corso la ferma (`User`);
    - i frame della swapchain principale del bersaglio vanno anche al `Recorder`, dopo il filtro del PID (DD7); il cambio di bersaglio conta come «senza bersaglio»;
    - `Outputs.benchmark: Vec<BenchmarkCommand>` con `Begin { stem }`, `Rows(Vec<String>)`, `Finish { record: Option<BenchmarkRecord> }`;
    - verso l'overlay, `Benchmark { recording_s }` a ogni cambio di secondo e, alla fine, il riepilogo per `SUMMARY_SHOW_MS`, poi `Benchmark { None, None }`;
    - `on_benchmark_error(&mut self, failure: WriteFailure)`: ferma con `Error` e tiene il motivo nello stato;
    - **`OverlayStatus.benchmark: BenchmarkStatus { state: "idle" | "recording" | "error", game: Option<String>, elapsed_s: Option<u32>, error: Option<LogError> }`.**
  - **Runner:** esegue i `BenchmarkCommand` con `BenchmarkFiles` nella cartella `configured_dir(settings).join("benchmarks")`; un errore di scrittura torna al controller; all'uscita dell'app una cattura in corso si chiude con `Shutdown` prima dell'host. Emette `overlay-status` alla fine, così lo storico si aggiorna.
  - **Scorciatoia:** `HotkeyAction::OverlayBenchmark` (quinta, ultima per precedenza, DP14), `ACTIONS = 5`; `OverlayActions::toggle_benchmark()`; `OverlayHotkeys.benchmark`.
  - **Comandi Tauri:** `benchmark_toggle()`, `benchmark_list() -> Vec<BenchmarkEntry>`, `benchmark_open_csv(id) -> Result<(), String>`, `benchmark_open_folder() -> Result<(), String>` (con `log.error.folderMissing` come per il log), `benchmark_delete(id) -> Result<(), String>`. Nessun comando riceve un percorso.
  - **Notifiche:**

    | Chiave | Inglese | Italiano |
    |---|---|---|
    | `benchmark.toast.title` | `Benchmark` | `Benchmark` |
    | `benchmark.noTarget` | `No game in the foreground` | `Nessun gioco in primo piano` |
    | `benchmark.toast.error` | `The capture stopped: {detail}` | `La cattura si è fermata: {detail}` |
    | `overlay.text.bench.avg` | `Avg FPS` | `FPS medi` |
    | `overlay.text.bench.low1` | `1% low` | `1% low` |
    | `overlay.text.bench.low01` | `0.1% low` | `0,1% low` |
    | `overlay.text.bench.stutter` | `Stutter` | `Stutter` |

    L'errore di scrittura usa le chiavi esistenti `log.error.*` per `{detail}`. Le quattro `overlay.text.bench.*` entrano in `overlay_strings` (come `bench.avg`, `bench.low1`, `bench.low01`, `bench.stutter`) e, con le tre `benchmark.*`, in `RUST_KEYS`.

- [ ] **Step 1: test che falliscono:**
  - `benchmark_without_target_toasts`;
  - `benchmark_records_only_the_target_main_swapchain`;
  - `benchmark_rec_seconds_go_to_the_overlay`;
  - `summary_shows_for_ten_seconds_then_clears`;
  - `blocked_game_is_recorded_without_badge`;
  - `write_error_stops_with_the_reason`;
  - `shutdown_finishes_a_running_capture`;
  - `five_actions_register_and_dispatch`, `benchmark_press_goes_to_the_controller`, `log_hotkeys_unchanged`.
- [ ] **Step 2:** `cargo test -p oma-app`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cargo test --workspace`, clippy, `cd app && pnpm test i18n`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(app): benchmark capture with hotkey, overlay badge and history commands`.
- [ ] **Step 6:** revisione `security-review` sui comandi che aprono e cancellano file.

### Task D13: UI, fondamenta dell'editor

Prima di scrivere i componenti di D13–D16: skill `frontend-design:frontend-design`, con la palette Synthwave e i controlli di `components/settings/controls`.

**Files:**
- Create:
  - `app/src/editor/EditorApp.svelte` (radice della finestra);
  - in `app/src/lib/editor/`: `profile.ts` (tipi, predefiniti, limiti), `geometry.ts`, `history.svelte.ts`, `ops.ts`, `editor.svelte.ts`, e i test `geometry.test.ts`, `history.test.ts`, `ops.test.ts`, `editor.test.ts`.
- Modify: `app/src/main.ts`; in `app/src/lib/backend/`: `backend.ts`, `tauri.ts`, `mock.ts`; `app/src/test/fake-backend.ts`; `app/src/lib/types.ts`; `app/src/lib/i18n/en.json` e `it.json`.

**Interfaces:**
- Consumes: i comandi e gli eventi di D8, D9, D10.
- Produces:
  - **`main.ts`:** con `?window=overlay-editor` monta `EditorApp`, altrimenti `App`; la lingua segue `settings.general.language` come nella finestra principale.
  - **`profile.ts`:** i tipi TypeScript di `Profile`, `Block`, `Style`… con gli stessi nomi camelCase di serde; `PROFILE_DEFAULTS`, `BLOCK_DEFAULTS` e `LIMITS` copiati dai predefiniti e dai limiti di `profile.rs` (C3), con un commento che rimanda a quel file; `newBlock(source, cell: {x, y}, existing): Block` (tipo `text`, `graph` per `frametime-*`, §7.2; dimensioni predefinite 12×2 per `text`, 20×4 per `graph`).
  - **`geometry.ts`:** `cellPx(scale, dpi)`, `footprint(blocks)`, `place(profile, area, dpi)`, `placeWithExtra(...)`, `blockPx(block, origin, cell)`: le stesse formule di `geometry.rs` e di `render::layout::block_px`.
  - **`history.svelte.ts`:** `class History<T>` con `push(state)`, `undo()`, `redo()`, `canUndo`, `canRedo`, 100 passi (§7.2); un `push` dopo un `undo` toglie i passi rifatti. Gli spostamenti continui (trascinamento) entrano come un solo passo, con `begin()` e `commit()`.
  - **`ops.ts` (puro):** `moveBlocks(profile, ids, dx, dy)`, `resizeBlocks(profile, ids, dw, dh)`, `snap(px, cell)`, `duplicateBlocks`, `pasteBlocks(profile, clip, at)`, `deleteBlocks`, `bringForward`, `sendBackward`, `commonValue(blocks, path)` (valore comune o `MIXED`), `setPath(blocks, ids, path, value)`. Rettangoli tenuti nei limiti di C3 (`x`, `y` 0–400; `w`, `h` 1–200).
  - **`editor.svelte.ts`:** `class EditorStore` con `profileId`, `builtin`, `profile`, `saved` (per `dirty`), `selection: Set<string>`, `history`, `clipboard`, e le azioni `load(id)`, `save()`, `saveAs(name)`, `rename(name)`, `duplicate()`, `remove()`, `importFile()`, `exportFile()`. `dirty` si comunica a Rust con `overlayEditorDirty` a ogni cambio.
  - **Backend:** `overlayLoadProfile`, `overlaySaveProfile`, `overlayDeleteProfile`, `overlayDuplicateProfile`, `overlayImportProfile`, `overlayExportProfile`, `overlayFontFamilies`, `overlayPreview`, `overlayUseNow`, `overlayEditorDirty`, `openOverlayEditor`, `appQuitConfirmed`, `onOverlayEditorData`, `onOverlayPreview`, `onOverlayEditorQuit`, più `benchmarkToggle`, `benchmarkList`, `benchmarkOpenCsv`, `benchmarkOpenFolder`, `benchmarkDelete` (usati da D16). Il backend finto ha un archivio in memoria dei profili e frame finti, così `pnpm dev` mostra l'editor con `?window=overlay-editor`.
  - **Testi:**

    | Chiave | Inglese | Italiano |
    |---|---|---|
    | `editor.newProfileName` | `New profile` | `Nuovo profilo` |

- [ ] **Step 1: test che falliscono (Vitest):**
  - `geometry matches the shared fixture`: legge `../../../../testdata/overlay/geometry-cases.json` (import JSON di Vite) e confronta ogni caso;
  - `history keeps 100 steps`, `push after undo drops the redo branch`, `a drag is one step`;
  - `move snaps to whole cells and stays in limits`, `resize keeps w and h in 1..200`;
  - `duplicate gives new ids`, `paste offsets by one cell`;
  - `commonValue reports MIXED`, `setPath applies to every selected block`;
  - `new graph block for frametime sources`;
  - `dirty after an edit and clean after save`;
  - `i18n keys are identical in en and it`: il test esistente.
- [ ] **Step 2:** `cd app && pnpm test editor`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): overlay editor model, history and geometry`.

### Task D14: UI, tela e tavolozza

**Files:**
- Create: in `app/src/editor/`: `Canvas.svelte`, `Palette.svelte`, `draw.ts` (Canvas2D) e i test `Canvas.test.ts`, `Palette.test.ts`, `draw.test.ts`.
- Modify: `app/src/editor/EditorApp.svelte`, `app/src/lib/i18n/en.json` e `it.json`.

**Interfaces:**
- Consumes: D13; `LiveStore`, `getHistory`, `sidebarEntries`, `groupSensors`, `categoryLabel`, `sensorLabel`; `formatValue` di `lib/format.ts`; `onOverlayEditorData`.
- Produces:
  - **Tavolozza (§7.1):** ricerca come `SensorTree` (stessa sottostringa senza distinzione di maiuscole su `sensorLabel`), dispositivi e categorie da `sidebarEntries` e `groupSensors`; il gruppo «Frame» con le 12 metriche (etichette `overlay.text.metric.*`); il gruppo «Testo» con «Nuovo testo». Ogni voce è trascinabile con gli eventi pointer e attivabile con Invio (crea il blocco nella prima cella libera in alto a sinistra).
  - **Tela (§7.1, §7.3):**
    - area simulata in scala dentro lo spazio disponibile, con la griglia delle celle e l'ingombro del profilo (`placeWithExtra` senza riquadro);
    - risoluzione simulata: monitor attuale (dalla finestra), 1920×1080, 2560×1440, 3840×2160;
    - i blocchi si disegnano con `draw.ts`: pannello, testo (etichetta, valore, unità con i tre stili, contorno con `strokeText`, ombra con `shadowOffsetX/Y`), `graph` (line, area, bars, frametime), `meter`, `sparkline`, `gauge`; soglie per `target`, `visibleIf` (i blocchi nascosti si disegnano al 30% di opacità con un bordo tratteggiato, così restano modificabili); «sensore assente» per un sensore che manca dallo schema;
    - valori: sensori dalla `LiveStore`, grafici dei sensori dallo storico (`getHistory` all'apertura, poi la `LiveStore`), metriche e frametime da `overlay-editor-data`;
    - una riga sotto la tela con `editor.canvas.note`.
  - **Interazioni (§7.2):** trascinare dalla tavolozza crea un blocco; trascinare un blocco lo sposta, le maniglie lo ridimensionano, sempre agganciati alle celle; clic seleziona, Maiusc o Ctrl+clic aggiunge o toglie; frecce spostano di una cella, Maiusc+frecce ridimensionano; Ctrl+C, Ctrl+V, Ctrl+D, Canc, Ctrl+Z, Ctrl+Y; ogni azione passa da `ops.ts` e dalla `History`.
  - **Accessibilità (§7.2):** la tela ha `role="application"`, `aria-label` `editor.canvas.label` e il focus visibile; un elenco dei blocchi accanto alla tela (`role="listbox"`) seleziona i blocchi da tastiera; ogni pulsante ha un testo.
  - **Testi:**

    | Chiave | Inglese | Italiano |
    |---|---|---|
    | `editor.palette.search` | `Search sensors` | `Cerca sensori` |
    | `editor.palette.frames` | `Frames` | `Frame` |
    | `editor.palette.text` | `Text` | `Testo` |
    | `editor.palette.newText` | `New text` | `Nuovo testo` |
    | `editor.canvas.label` | `Profile canvas` | `Tela del profilo` |
    | `editor.canvas.blocks` | `Blocks` | `Blocchi` |
    | `editor.canvas.note` | `The canvas uses the browser's fonts: it is close to the overlay, not identical. Use Preview for the exact result.` | `La tela usa i font del browser: è vicina all'overlay, non identica. Per il risultato esatto usa «Anteprima».` |
    | `editor.resolution` | `Simulated screen` | `Schermo simulato` |
    | `editor.resolution.current` | `Current monitor` | `Monitor attuale` |

- [ ] **Step 1: test che falliscono (Vitest, jsdom con un contesto 2D finto):**
  - `dropping a sensor creates a text block`, `dropping frametime creates a graph block`;
  - `drag moves by whole cells`, `handles resize`;
  - `shift click adds to the selection`;
  - `arrows move and shift arrows resize`;
  - `ctrl z undoes a drag in one step`;
  - `palette search filters sensors`, `enter on a palette item adds a block`;
  - `hidden blocks are drawn translucent`;
  - `threshold colour applies to its target`;
  - `missing sensor shows the absent text`.
- [ ] **Step 2:** `cd app && pnpm test editor`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): overlay editor canvas and source palette`.

### Task D15: UI, barra, proprietà, anteprima e uscita

**Files:**
- Create: in `app/src/editor/`: `Toolbar.svelte`, `Properties.svelte`, `ThresholdsEditor.svelte`, `VisibleIfEditor.svelte`, `UnsavedDialog.svelte` e i test.
- Modify: `app/src/editor/EditorApp.svelte`, `app/src/lib/i18n/en.json` e `it.json`.

**Interfaces:**
- Consumes: D13, D14; `overlayFontFamilies`, `overlayPreview`, `overlayUseNow`, `onOverlayPreview`, `onOverlayEditorQuit`, `appQuitConfirmed`; `getCurrentWindow().onCloseRequested` e `destroy()`.
- Produces:
  - **Barra (§7.1):** selettore del profilo (integrati con il nome tradotto); salva, salva come, rinomina, duplica, elimina (con conferma), importa, esporta; ancoraggio (nove posizioni), scostamento, scala (0,5–3,0, passo 0,05), pannello (colore, opacità, raggio, margine); schermo simulato; «Anteprima»/«Chiudi anteprima»; «Usa ora»; annulla e ripeti. Con un integrato aperto i comandi di modifica sono disattivati e una riga dice `editor.error.readOnly`.
  - **Proprietà (§6.2, §6.3, §7.1):** tipo, posizione e dimensione, livello, statistica (con finestra e definizione per `low-*`), etichetta, i tre stili di testo (font da `overlayFontFamilies`, dimensione, spessore, corsivo, colore, contorno, ombra), allineamento, decimali, unità; per i grafici modo, intervallo, asse Y, linea, riempimento, griglia, min/media/max, valore; per `meter` e `gauge` orientamento e estremi; soglie (fino a 8, prima vera, per target); `visibleIf` (sempre, frame generation attiva, un valore con sorgente, statistica, operatore e soglia); pannello proprio. Con più blocchi selezionati solo le proprietà comuni, con «—» per i valori diversi, e la modifica vale per tutti (§7.1).
  - **Anteprima (§7.4):** ogni modifica, con un ritardo di 100 ms, manda il profilo in corso (`overlayPreview(json)`); «Chiudi anteprima» manda `null`; il pulsante segue `overlay-preview`. Un errore mostra `editor.error.preview`.
  - **«Usa ora»:** salva (o salva come per un profilo nuovo) e chiama `overlayUseNow(id)` (DD9).
  - **Uscita (§7.2, DD13):** `onCloseRequested` con `dirty` impedisce la chiusura e apre `UnsavedDialog` (salva, scarta, annulla); salva o scarta chiudono con `destroy()`. Lo stesso dialogo risponde a `overlay-editor-quit` e poi chiama `appQuitConfirmed()`. Cambiare profilo con modifiche chiede lo stesso.
  - **Testi:**

    | Chiave | Inglese | Italiano |
    |---|---|---|
    | `editor.profile` | `Profile` | `Profilo` |
    | `editor.save` | `Save` | `Salva` |
    | `editor.saveAs` | `Save as…` | `Salva come…` |
    | `editor.rename` | `Rename` | `Rinomina` |
    | `editor.duplicate` | `Duplicate` | `Duplica` |
    | `editor.delete` | `Delete` | `Elimina` |
    | `editor.delete.confirm` | `Delete the profile «{name}»?` | `Eliminare il profilo «{name}»?` |
    | `editor.import` | `Import…` | `Importa…` |
    | `editor.export` | `Export…` | `Esporta…` |
    | `editor.name` | `Name` | `Nome` |
    | `editor.undo` | `Undo` | `Annulla` |
    | `editor.redo` | `Redo` | `Ripeti` |
    | `editor.anchor` | `Anchor` | `Ancoraggio` |
    | `editor.anchor.top-left` | `Top left` | `In alto a sinistra` |
    | `editor.anchor.top` | `Top` | `In alto` |
    | `editor.anchor.top-right` | `Top right` | `In alto a destra` |
    | `editor.anchor.left` | `Left` | `A sinistra` |
    | `editor.anchor.center` | `Center` | `Al centro` |
    | `editor.anchor.right` | `Right` | `A destra` |
    | `editor.anchor.bottom-left` | `Bottom left` | `In basso a sinistra` |
    | `editor.anchor.bottom` | `Bottom` | `In basso` |
    | `editor.anchor.bottom-right` | `Bottom right` | `In basso a destra` |
    | `editor.offset` | `Offset (cells)` | `Scostamento (celle)` |
    | `editor.scale` | `Scale` | `Scala` |
    | `editor.panel` | `Panel` | `Pannello` |
    | `editor.panel.color` | `Color` | `Colore` |
    | `editor.panel.opacity` | `Opacity` | `Opacità` |
    | `editor.panel.radius` | `Corner radius` | `Raggio degli angoli` |
    | `editor.panel.padding` | `Padding (cells)` | `Margine interno (celle)` |
    | `editor.panel.own` | `Own panel` | `Pannello proprio` |
    | `editor.preview` | `Preview` | `Anteprima` |
    | `editor.preview.close` | `Close preview` | `Chiudi anteprima` |
    | `editor.useNow` | `Use now` | `Usa ora` |
    | `editor.useNow.hint` | `Saves the profile and shows it in game until the game changes.` | `Salva il profilo e lo mostra in gioco fino al cambio di gioco.` |
    | `editor.error.preview` | `Preview not available: {detail}` | `Anteprima non disponibile: {detail}` |
    | `editor.unsaved.title` | `Unsaved changes` | `Modifiche non salvate` |
    | `editor.unsaved.body` | `Save the changes to «{name}»?` | `Salvare le modifiche a «{name}»?` |
    | `editor.unsaved.save` | `Save` | `Salva` |
    | `editor.unsaved.discard` | `Discard` | `Scarta` |
    | `editor.unsaved.cancel` | `Cancel` | `Annulla` |
    | `editor.props.title` | `Properties` | `Proprietà` |
    | `editor.props.none` | `Select a block to edit it.` | `Seleziona un blocco per modificarlo.` |
    | `editor.props.multi` | `{n} blocks selected: common properties only.` | `{n} blocchi selezionati: solo le proprietà comuni.` |
    | `editor.props.mixed` | `—` | `—` |
    | `editor.props.kind` | `Type` | `Tipo` |
    | `editor.props.kind.text` | `Text` | `Testo` |
    | `editor.props.kind.graph` | `Graph` | `Grafico` |
    | `editor.props.kind.meter` | `Bar` | `Barra` |
    | `editor.props.kind.sparkline` | `Sparkline` | `Sparkline` |
    | `editor.props.kind.gauge` | `Gauge` | `Indicatore` |
    | `editor.props.x` | `X (cells)` | `X (celle)` |
    | `editor.props.y` | `Y (cells)` | `Y (celle)` |
    | `editor.props.w` | `Width (cells)` | `Larghezza (celle)` |
    | `editor.props.h` | `Height (cells)` | `Altezza (celle)` |
    | `editor.props.z` | `Layer` | `Livello` |
    | `editor.props.forward` | `Bring forward` | `Porta avanti` |
    | `editor.props.backward` | `Send backward` | `Porta indietro` |
    | `editor.props.copy` | `Copy` | `Copia` |
    | `editor.props.paste` | `Paste` | `Incolla` |
    | `editor.props.remove` | `Delete block` | `Elimina il blocco` |
    | `editor.props.stat` | `Statistic` | `Statistica` |
    | `editor.props.stat.current` | `Current` | `Attuale` |
    | `editor.props.stat.min` | `Minimum` | `Minimo` |
    | `editor.props.stat.avg` | `Average` | `Media` |
    | `editor.props.stat.max` | `Maximum` | `Massimo` |
    | `editor.props.window` | `Window (s)` | `Finestra (s)` |
    | `editor.props.definition` | `Low definition` | `Definizione dei low` |
    | `editor.props.definition.integral` | `Integral` | `Integrale` |
    | `editor.props.definition.percentile` | `Percentile` | `Percentile` |
    | `editor.props.text` | `Text` | `Testo` |
    | `editor.props.label` | `Label` | `Etichetta` |
    | `editor.props.auto` | `Automatic` | `Automatica` |
    | `editor.props.labelStyle` | `Label style` | `Stile dell'etichetta` |
    | `editor.props.valueStyle` | `Value style` | `Stile del valore` |
    | `editor.props.unitStyle` | `Unit style` | `Stile dell'unità` |
    | `editor.props.font` | `Font` | `Carattere` |
    | `editor.props.size` | `Size (pt)` | `Dimensione (pt)` |
    | `editor.props.weight` | `Weight` | `Spessore` |
    | `editor.props.italic` | `Italic` | `Corsivo` |
    | `editor.props.color` | `Color` | `Colore` |
    | `editor.props.outline` | `Outline` | `Contorno` |
    | `editor.props.shadow` | `Shadow` | `Ombra` |
    | `editor.props.align` | `Alignment` | `Allineamento` |
    | `editor.props.align.left` | `Left` | `A sinistra` |
    | `editor.props.align.center` | `Center` | `Al centro` |
    | `editor.props.align.right` | `Right` | `A destra` |
    | `editor.props.decimals` | `Decimals` | `Decimali` |
    | `editor.props.unit` | `Unit` | `Unità` |
    | `editor.props.graph` | `Chart` | `Grafico` |
    | `editor.props.graph.line` | `Line` | `Linea` |
    | `editor.props.graph.area` | `Area` | `Area` |
    | `editor.props.graph.bars` | `Bars` | `Barre` |
    | `editor.props.graph.frametime` | `Frametime` | `Frametime` |
    | `editor.props.range` | `Range (s)` | `Intervallo (s)` |
    | `editor.props.yAuto` | `Automatic scale` | `Scala automatica` |
    | `editor.props.min` | `Minimum` | `Minimo` |
    | `editor.props.max` | `Maximum` | `Massimo` |
    | `editor.props.lineColor` | `Line color` | `Colore della linea` |
    | `editor.props.lineWidth` | `Line width` | `Spessore della linea` |
    | `editor.props.fill` | `Fill` | `Riempimento` |
    | `editor.props.gridLines` | `Grid lines` | `Linee della griglia` |
    | `editor.props.showMinAvgMax` | `Show min/avg/max` | `Mostra min/media/max` |
    | `editor.props.showValue` | `Show the value` | `Mostra il valore` |
    | `editor.props.orientation` | `Orientation` | `Orientamento` |
    | `editor.props.orientation.horizontal` | `Horizontal` | `Orizzontale` |
    | `editor.props.orientation.vertical` | `Vertical` | `Verticale` |
    | `editor.props.rangeMin` | `Start` | `Inizio` |
    | `editor.props.rangeMax` | `End` | `Fine` |
    | `editor.thresholds` | `Thresholds` | `Soglie` |
    | `editor.thresholds.add` | `Add threshold` | `Aggiungi soglia` |
    | `editor.thresholds.hint` | `The first true rule wins, for each target.` | `Vince la prima regola vera, per ogni destinazione.` |
    | `editor.thresholds.target.value` | `Value` | `Valore` |
    | `editor.thresholds.target.graph` | `Graph` | `Grafico` |
    | `editor.thresholds.target.panel` | `Panel` | `Pannello` |
    | `editor.thresholds.remove` | `Remove` | `Rimuovi` |
    | `editor.visibleIf` | `Show` | `Mostra` |
    | `editor.visibleIf.always` | `Always` | `Sempre` |
    | `editor.visibleIf.fg` | `When frame generation is active` | `Quando la frame generation è attiva` |
    | `editor.visibleIf.value` | `When a value meets a condition` | `Quando un valore rispetta una condizione` |

- [ ] **Step 1: test che falliscono (Vitest):**
  - `builtin profiles are read only until duplicated`;
  - `save as asks a name and selects the new profile`;
  - `delete asks for confirmation`;
  - `multi selection shows common properties and edits all`;
  - `font list comes from the backend`;
  - `threshold editor keeps order and caps at eight`;
  - `visibleIf editor writes each form`;
  - `preview is sent 100 ms after the last edit`;
  - `use now saves then activates`;
  - `closing_with_changes_asks`, `cancel_keeps_the_editor_open`, `discard_closes_without_saving`;
  - `quit request from the tray asks and confirms`.
- [ ] **Step 2:** `cd app && pnpm test editor`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): overlay editor toolbar, properties, preview and unsaved changes`.

### Task D16: UI, Impostazioni › Benchmark e ritocchi di Impostazioni › Overlay

**Files:**
- Create: `app/src/components/settings/BenchmarkSection.svelte` e `BenchmarkSection.test.ts`
- Modify: `app/src/components/settings/SettingsView.svelte` (sezione `benchmark` fra `overlay` e `sources`), `app/src/components/settings/OverlaySection.svelte`, `app/src/lib/view.ts` (`SettingsTarget.section` con `benchmark`), `app/src/lib/i18n/en.json` e `it.json`.

**Interfaces:**
- Consumes: `OverlayStatus.benchmark` (D12), i comandi del benchmark (D12), `openOverlayEditor` (D10).
- Produces:
  - **Benchmark (§8):** stato (`benchmark.recording` con il tempo `mm:ss` che avanza ogni secondo dalla UI, ripartendo da `elapsedS`), «Avvia la cattura»/«Ferma la cattura» (disattivato con l'overlay spento, DD6, con `benchmark.needsOverlay`); poi lo storico dal più recente: gioco, data, durata, i valori del riepilogo, «Apri CSV», «Apri cartella», «Elimina» con conferma. Lo storico si rilegge all'apertura e quando lo stato passa da `recording` a un altro.
  - **Overlay:** pulsante «Apri editor» nel gruppo dei profili, sempre attivo; `HotkeyInput` per `hotkeyBenchmark` accanto alle altre due; gli errori di «Riprova» e «Ricarica i profili» compaiono in una riga sotto i pulsanti (voce aperta della M7c) con `overlay.error`.
  - **Testi** (`{game}`, `{time}`, `{date}`, `{total}`, `{displayed}`, `{generated}`, `{count}`, `{percent}`, `{detail}` sono parametri):

    | Chiave | Inglese | Italiano |
    |---|---|---|
    | `settings.section.benchmark` | `Benchmark` | `Benchmark` |
    | `benchmark.start` | `Start capture` | `Avvia la cattura` |
    | `benchmark.stop` | `Stop capture` | `Ferma la cattura` |
    | `benchmark.recording` | `Recording {game}: {time}` | `Registrazione di {game}: {time}` |
    | `benchmark.needsOverlay` | `Turn the overlay on to measure games.` | `Accendi l'overlay per misurare i giochi.` |
    | `benchmark.hint` | `One row per frame in a CSV file, plus a summary, in the benchmarks folder of the CSV log.` | `Una riga per frame in un file CSV, più un riepilogo, nella cartella benchmarks del log CSV.` |
    | `benchmark.history` | `Sessions` | `Sessioni` |
    | `benchmark.history.empty` | `No sessions yet.` | `Ancora nessuna sessione.` |
    | `benchmark.openCsv` | `Open CSV` | `Apri CSV` |
    | `benchmark.openFolder` | `Open folder` | `Apri cartella` |
    | `benchmark.delete` | `Delete` | `Elimina` |
    | `benchmark.delete.confirm` | `Delete the session of {game} from {date}?` | `Eliminare la sessione di {game} del {date}?` |
    | `benchmark.duration` | `Duration` | `Durata` |
    | `benchmark.frames` | `Frames: {total} ({displayed} shown, {generated} generated)` | `Frame: {total} ({displayed} mostrati, {generated} generati)` |
    | `benchmark.fpsDisplayed` | `Average FPS shown` | `FPS medi mostrati` |
    | `benchmark.fpsRendered` | `Average FPS rendered` | `FPS medi renderizzati` |
    | `benchmark.lowsIntegral` | `1% / 0.1% low (integral)` | `1% / 0,1% low (integrale)` |
    | `benchmark.lowsPercentile` | `1% / 0.1% low (percentile)` | `1% / 0,1% low (percentile)` |
    | `benchmark.frametime` | `Frametime min / max` | `Frametime min / max` |
    | `benchmark.stutter` | `Stutter: {count} ({percent} of the time)` | `Stutter: {count} ({percent} del tempo)` |
    | `benchmark.fgMultiplier` | `Average FG multiplier` | `Moltiplicatore FG medio` |
    | `benchmark.latencyPc` | `Average PC latency` | `Latenza PC media` |
    | `benchmark.latencyDisplay` | `Average display latency` | `Latenza di visualizzazione media` |
    | `benchmark.end.user` | `Stopped` | `Fermata` |
    | `benchmark.end.noTarget` | `Game gone for 10 s` | `Gioco assente da 10 s` |
    | `benchmark.end.limit` | `Limit reached` | `Limite raggiunto` |
    | `benchmark.end.error` | `Write error` | `Errore di scrittura` |
    | `benchmark.end.shutdown` | `App closed` | `App chiusa` |
    | `overlay.openEditor` | `Open editor` | `Apri editor` |
    | `overlay.hotkeyBenchmark` | `Benchmark capture` | `Cattura del benchmark` |
    | `overlay.error` | `It did not work: {detail}` | `Non ha funzionato: {detail}` |

- [ ] **Step 1: test che falliscono (Vitest):**
  - `start disabled with the overlay off`;
  - `recording shows the game and the time`;
  - `history lists newest first with the summary`;
  - `delete asks then removes the entry`;
  - `history reloads when a capture ends`;
  - `open editor calls the backend`;
  - `benchmark hotkey capture suspends hotkeys`;
  - `retry and reload errors are shown`.
- [ ] **Step 2:** `cd app && pnpm test settings`. Atteso: FAIL.
- [ ] **Step 3:** implementare.
- [ ] **Step 4:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): benchmark settings page, open editor button and overlay errors`.

### Task D17: documenti, misure e bump alla 0.5.0

**Files:**
- Modify:
  - `README.md`: nella sezione «Overlay in-game», l'editor (apertura, tela, anteprima, import ed export, cartella dei profili) e il benchmark (scorciatoia, file CSV e `.json`, cartella); «Known limits»: la tela non è fedele al pixel per i font, il benchmark richiede l'overlay acceso;
  - `CLAUDE.md`: stato della M7d, finestra `overlay-editor` e capability `editor.json`, `--preview` e il codice d'uscita 5, protocollo dell'overlay v2, `overlay::store`, `overlay::benchmark`, `frames::session`;
  - `docs/perf-budget.md`: i numeri di D18 (editor aperto, anteprima, `FrameReadout` sulla finestra di 300 s);
  - `docs/follow-ups.md`: «Closed in M7d» (le voci della M7c chiuse qui: scala automatica, errori nascosti della pagina Overlay, costo dei low lunghi se misurato) e le voci aperte; la sezione «Draft: release notes for 0.5.0»;
  - i file di versione con `pwsh scripts/bump-version.ps1 0.5.0`.
- Create: un test `#[ignore = "measurement"]` in `crates/oma-core/src/frames/readout.rs`, `readout_cost_on_a_300_s_window`, che stampa il tempo di `read` su 300 s a 240 FPS (72.000 frame) con una finestra low di 300 s.

**Interfaces:** nessuna nuova.

- [ ] **Step 1:** `cargo test -p oma-core readout_cost_on_a_300_s_window -- --ignored --nocapture`; annotare il tempo per chiamata. Se supera 2 ms, aprire una voce in `docs/follow-ups.md` (il controller la chiama a `textHz`).
- [ ] **Step 2: bozza delle note di rilascio della 0.5.0** in `docs/follow-ups.md`, nello stile della 0.4.1: overlay in-game (M7b, M7c), editor, profili, benchmark; i limiti principali (fullscreen esclusivo, FG senza Reflex, memoria dell'overlay in gioco).
- [ ] **Step 3:** `pwsh scripts/bump-version.ps1 0.5.0`, poi `pwsh scripts/check-version.ps1`. Atteso: le cinque versioni e `Cargo.lock` a 0.5.0.
- [ ] **Step 4: verifiche:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cd app && pnpm test && pnpm check && pnpm build`, `dotnet test service/OpenMonitorAdvanced.slnx`, Pester senza `Integration`, `pwsh scripts/generate-licenses.ps1 -Check`. Atteso: PASS.
- [ ] **Step 5: commit:** prima `docs: document the overlay editor and the benchmark`, poi `chore: release 0.5.0`. Nessun tag e nessun push.

### Task D18: prove dal vivo con l'utente

**Regole:**
- l'agente prepara, compila e legge i log;
- l'utente usa editor, tray, scorciatoie e giochi, e installa il setup;
- i comandi per l'utente si danno un blocco alla volta, per PowerShell 5.1 amministratore solo quando servono i diritti.

- [ ] **Step 1 (agente):** `pwsh scripts/build-installer-payload.ps1`, poi `cd app && pnpm tauri build --bundles nsis`; annotare lo SHA-256 del setup.
- [ ] **Step 2 (utente):** installare il setup 0.5.0 sopra quello della M7c, con l'overlay acceso.
- [ ] **Step 3: prove**, ciascuna con l'esito annotato nella sezione «Esito dell'esecuzione» di questo piano:

| # | Prova | Atteso |
|---|---|---|
| X1 | Editor dal tray e da Impostazioni › Overlay; duplicare «Gaming», aggiungere sensori e metriche trascinandoli, spostare e ridimensionare con mouse e tastiera, selezione multipla, annulla e ripeti | tutto funziona da mouse e da tastiera; la posizione della finestra si ricorda alla riapertura |
| X2 | Proprietà: font, contorno, ombra, soglie, `visibleIf` sulla FG, grafico del frametime, `meter` e `gauge` | la tela segue; senza gioco i frame sintetici muovono grafici e soglie |
| X3 | Anteprima: aprirla, modificare, ridimensionarla, chiuderla dalla X, riaprirla | si aggiorna entro un attimo; chiusa dall'utente non si riapre da sola; «Usa ora» mostra il profilo in gioco |
| X4 | Import ed export: esportare un profilo, reimportarlo (nome « (2)»), importare un file rotto | il file rotto dà un messaggio e non crea niente |
| X5 | Uscita con modifiche: chiudere l'editor, poi «Esci» dal tray | la domanda c'è; «Annulla» lascia l'editor aperto |
| X6 | Benchmark in Control (con la scorciatoia e con il pulsante), uscire dal gioco durante una cattura, una cattura senza gioco | `● REC` nell'overlay, riepilogo per 10 s, file CSV e `.json` nella cartella; fine dopo 10 s senza gioco; avviso senza gioco; lo storico apre, mostra ed elimina |
| X7 | Budget: editor aperto con la finestra principale chiusa, poi con l'anteprima aperta (`measure-footprint.ps1` e Gestione attività) | editor come la finestra principale (< 200 MB con WebView2); i numeri dell'anteprima annotati in `docs/perf-budget.md` |
| X8 | Voci della M7c rimaste: costo di `EVENT_OBJECT_LOCATIONCHANGE` muovendo la finestra di un gioco in finestra; stato `starting` sul desktop senza giochi | numeri e comportamento annotati in `docs/follow-ups.md` |

- [ ] **Step 4:** le correzioni nate dalle prove seguono il ciclo normale: test, fix, revisione. Poi `docs/follow-ups.md`, la memoria `m7d-followups.md` e `CLAUDE.md` (stato della M7d).
- [ ] **Step 5: commit** `docs: record the M7d live checks`.

## Esito dell'esecuzione

D1–D17 eseguiti subagent-driven il 2026-10-05, ognuno con la sua revisione, poi la revisione dell'intero branch con un giro di correzioni e la riscrittura del trailer dei commit. La versione è la 0.5.0 (non taggata, non pubblicata). Le prove dal vivo D18 si sono fatte con l'utente il 2026-10-05 e il 2026-10-06 su questo PC (God of War in finestra, Windows Terminal), con il setup locale 0.5.0, SHA-256 `14d154241266ae4358818897d93b435e3d40b59ffa10a551a73a2e1d07f1f2f8`.

| # | Esito |
|---|---|
| X1 | Superata (2026-10-05), compresi i limiti della finestra dell'editor da massimizzata: si riapre alla dimensione e alla posizione normali. |
| X2 | Superata (2026-10-06). |
| X3 | Superata (2026-10-06), compreso modificare e chiudere l'editor con l'anteprima aperta. |
| X4 | Superata (2026-10-06), dopo una correzione richiesta dall'utente: gli errori dell'editor stanno in un banner rosso che si chiude, non in testo rosso semplice (`2101dbf`). |
| X5 | Superata (2026-10-06). |
| X6 | Superata (2026-10-06), dopo una correzione richiesta dall'utente: spazio sotto «Apri cartella» nella lista delle sessioni del benchmark (`4b9bea6`, un solo valore CSS). |
| X7 | Superata (2026-10-06). Solo editor (finestra principale chiusa): app con WebView2 154 MB, CPU circa 0,15%. Editor e anteprima: `oma-overlay --preview` 21 MB e 0,08% di CPU, app 154 MB. Nel tray: app 26,7 MB, WebView2 e anteprima spariti; overlay in gioco nascosto 9,9 MB. Numeri in `docs/perf-budget.md`. |
| X8 | Superata (2026-10-06). (a) Trascinare per 20 s la finestra di God of War: `oma-app` 0,26% di CPU contro 0,07% a riposo nel tray. (b) Desktop senza giochi: Impostazioni › Overlay mostra «Misura attiva» e non resta su «Avvio…»; il motore resta acceso finché l'overlay è abilitato (progetto della M7c). |

**Correzioni nate dalle prove.** `2101dbf` banner di errore chiudibile nell'editor (X4); `4b9bea6` spazio sotto «Apri cartella» nelle sessioni del benchmark (X6). Entrambe riviste.

**Voci aperte nuove.** Il log scrive un WARN «stopped unexpectedly code=5» prima dell'INFO «the preview was closed» quando si chiude l'anteprima (rumore). L'overlay si nasconde dopo circa 3 s in un trascinamento della finestra del gioco, perché il gioco smette di presentare nel ciclo modale di spostamento di Windows: l'utente lo ritiene accettabile. Entrambe in `docs/follow-ups.md`.

**Prove aggiunte il 2026-10-06.** Chiusura forzata del processo dell'anteprima cinque volte: riavvii dopo 1, 2, 4 e 8 s, nessun riavvio dopo la quinta caduta, «Anteprima non disponibile: overlay interrotto» nell'editor, overlay in gioco intatto, riapertura manuale riuscita. Anteprima spostata su un secondo monitor a 1920×1080 con scala 150%: il cambio di DPI ridimensiona e ridisegna correttamente.

**Non provate dal vivo (restano dovute).** Apertura dell'anteprima su un monitor principale piccolo (saltata dall'utente; coperta dal test `preview_bounds`); i percorsi dell'installer (VM).
