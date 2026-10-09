# M9a: grafico a strisce e layout a tutta finestra. Piano di implementazione

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:**
- il grafico della vista Avanzata diventa a strisce, una per unità, con la legenda dei valori e il cassetto «Serie» che copre solo le linee;
- Semplice e Avanzata usano le finestre grandi;
- le tessere della Semplice mostrano più dati.

**Architecture:**
- **Solo interfaccia,** in `app/src/`. Rust, servizio e protocollo restano invariati.
- **`StripChart.svelte`** prende il posto di `HistoryChart.svelte` con le **stesse props**: `sectionId`, `sensors`, `defaults`, `schema`, `store`, `backend`, `rate` e `fixedWindow`. Lo usa `DevicePage`. `StressRun` tiene `HistoryChart`, con le serie scelte dal test (M6, spec §5.4).
- **Cosa tiene `StripChart`:**
  - la selezione;
  - un solo `ChartBuffer`;
  - lo storico;
  - il viewport;
  - l'orologio dei frame;
  - la legenda (`StripLegend.svelte`);
  - il cassetto (`SeriesDrawer.svelte`).
- **`ChartStrip.svelte`** è la striscia: uno uPlot con il canvas composito estratto da `HistoryChart`.
- **Funzioni pure:**
  - `app/src/lib/advanced/chartData.ts` (strisce, palette, estremi);
  - `app/src/lib/advanced/drawer.ts` (gruppi del cassetto);
  - `app/src/lib/select.ts` (dati delle tessere);
  - `app/src/lib/simpleGrid.ts` (griglia della Semplice).
- **Ordine dei task:**
  - M1: funzioni pure del grafico;
  - M2: strisce e legenda;
  - M3: cassetto;
  - M4: larghezze e griglia della Semplice;
  - M5: dati delle tessere;
  - M6: grafico dello stress test;
  - M7: misure, documenti e prove dal vivo.

**Tech Stack:** quelli già in uso, cioè Svelte 5, TypeScript 6, uPlot 1.6.32 e Vitest con `@testing-library/svelte`. Nessuna dipendenza nuova.

**Spec:** `docs/superpowers/specs/2026-10-09-m9a-grafico-layout-design.md`. I mockup approvati stanno in `.superpowers/brainstorm/*/content/`:
- `picker-v2.html` (cassetto);
- `simple-2x2-data.html` (Semplice).

## Global Constraints

- **Lingua e formato:**
  - codice, commenti e commit in inglese (conventional commits);
  - documentazione e testi italiani con gli accenti corretti e gli apostrofi dritti;
  - fine riga LF;
  - ogni commit termina con le due righe:
    - `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`;
    - `Claude-Session: https://claude.ai/code/session_01B3yzfCRMU6TL8scsVZQ32s`.
- **Divieti per gli agenti:**
  - mai push, merge in `main` o release;
  - mai ricerche a tutto il disco;
  - mai comandi elevati;
  - mai clic sintetici o UI Automation sul desktop;
  - mai build release o misure di footprint senza il via dell'utente (M7), per non caricare la CPU senza avviso.
- **Prestazioni:**
  - fra due dati, un frame cambia solo i `transform` dei canvas (uno per striscia) e niente altro;
  - i percorsi si ricalcolano solo a ogni dato e solo per le strisce visibili.
- **i18n:** ogni chiave nuova c'è in `en.json` e in `it.json`, con le stesse chiavi.
- **Tooltip:** ogni termine tecnico nuovo ha la voce `glossary.<term>` e passa da `Term`. `Term` non va mai dentro un `<button>` (vedi DM10).
- **Sicurezza dell'interfaccia:** i nomi di sensori, dischi e adattatori si mostrano solo come testo, mai con `{@html}`.
- **TDD e debug:** prima il test che fallisce (`superpowers:test-driven-development`); davanti a un test che fallisce senza un motivo chiaro, `superpowers:systematic-debugging`.
- **Interfaccia:** M3 e M4 usano la skill `frontend-design:frontend-design`.
- **graphify:** ogni brief riporta `graphify query "<domanda>"`, `graphify explain "<simbolo>"` e `graphify path "<A>" "<B>"`. Dopo le modifiche al codice: `PYTHONHASHSEED=0 graphify update .`.
- **Comandi di verifica:** `cd app && pnpm test && pnpm check && pnpm build`. Atteso: tutto verde.

## Decisioni del piano

| # | Decisione | Perché |
|---|---|---|
| DM1 | **Strisce:**<br>• `export interface Strip { unit: Unit; ids: string[]; indices: number[] }`, dove `indices` sono le posizioni di `ids` nella lista passata;<br>• `stripLayout(ids: string[], schema: Schema): Strip[]`: un'unità per striscia, nell'ordine della prima serie di ogni unità; gli id assenti dallo schema si saltano. | Spec §2.1. |
| DM2 | **Palette:**<br>• `STRIP_DASHES = [[], [6, 4], [2, 3]]`;<br>• `stripPalette(count: number): Array<{ token: (typeof PALETTE_TOKENS)[number]; dash: readonly number[] }>`, con `token = PALETTE_TOKENS[i % 8]` e `dash = STRIP_DASHES[Math.floor(i / 8) % 3]`.<br>Il colore si risolve con `seriesPalette` come oggi. | Spec §2.3. |
| DM3 | **Dati di una striscia:**<br>• `ChartBuffer.columns(indices: number[], convert: ReadonlyArray<((v: number) => number) \| undefined> = []): uPlot.AlignedData`, cioè le x e le sole colonne di `indices`, convertite (`convert[k]` vale per `indices[k]`);<br>• `windowExtent(xs: number[], column: (number \| null)[], sinceSeconds: number): { min: number; max: number } \| null`, che conta solo le x ≥ `sinceSeconds` e i valori non `null`.<br>Gli estremi della legenda si calcolano sui dati già convertiti della striscia, a ogni dato. | Spec §2.4 e §6. |
| DM4 | **Disegno:**<br>• `ChartCanvasPath` e `ChartHeldSegment` ricevono `dash: readonly number[]`, applicato con `ctx.setLineDash` (e azzerato dopo);<br>• `drawChartCanvas` riceve un decimo argomento `options: { timeLabels?: boolean } = {}`: con `timeLabels: false` disegna le righe verticali ma non le etichette del tempo. | Spec §2.1 e §2.3. Il segmento tenuto ha lo stesso tratteggio della sua linea. |
| DM5 | **Altezze e larghezze:**<br>• `STRIP_SINGLE_HEIGHT = 260` (uPlot con l'asse del tempo compreso, come oggi);<br>• `STRIP_HEIGHT = 130` per l'area delle linee di ogni striscia;<br>• l'ultima striscia aggiunge `TIME_AXIS_HEIGHT = 40` per le etichette, mentre le altre hanno l'asse X di misura 0;<br>• `Y_AXIS_PX = 72` (la `size` di oggi);<br>• `LEGEND_PX = 220`;<br>• lo spazio fra il grafico e la legenda è 12 px. | Spec §2.1. Il cassetto si posiziona con queste costanti. |
| DM6 | **`ChartStrip.svelte`:** uno uPlot con una sola scala Y (`unit`), `legend: { show: false }` e `cursor.sync: { key: <chiave del grafico> }`. Tiene ciò che oggi è per grafico:<br>• il canvas composito;<br>• i punti bianchi;<br>• il segmento tenuto;<br>• le transizioni Y (`yRange`, `stepY`, `finishY`);<br>• `timeSpace` e lo spostamento del cursore.<br>**Props:** `unit`, `colors: Array<{ color: string; dash: readonly number[] }>`, `labels: string[]`, `hidden: boolean[]`, `format` (di `displayScale`), `viewport`, `windowSeconds`, `height`, `timeAxis: boolean`, `syncKey`, `reducedMotion`, `oncursor: (idx: number \| null) => void`.<br>**Metodi esportati**, chiamati dal padre:<br>• `build(data)`: crea o ricrea il plot;<br>• `setData(data, nowMonoMs)`: nuovo dato, come oggi `tail`;<br>• `frame(nowMonoMs)`: Y e spostamento;<br>• `finishY()`;<br>• `reset()`: finestra nascosta;<br>• `destroy()`.<br>**Fuori dallo schermo:** un `IntersectionObserver` sul contenitore; da nascosta la striscia tiene l'ultimo `data`, salta `setData` e `frame`, e quando torna visibile rifà `setData` più rebase. Senza `IntersectionObserver` (jsdom) è sempre visibile. | Spec §4 e §6. Il padre non conosce uPlot. |
| DM7 | **`StripChart.svelte`:** tiene quello che `HistoryChart` fa oggi per tutto il grafico:<br>• selezione, persistenza e finestra;<br>• `reseed` e `getHistory` (una richiesta per tutte le serie);<br>• `tail`;<br>• visibilità e `windowAllSuspended` (per striscia);<br>• `subscribeChartFrame`: una sola iscrizione, che chiama `frame` su ogni striscia;<br>• unità e locale, che a ogni cambio ricostruiscono le strisce dal buffer.<br>**Cursore:** `cursorIdx: number \| null`, l'ultimo indice riportato da una striscia (`null` all'uscita).<br>**Fra un rebuild e l'altro restano:** le serie nascoste (`hiddenIds: Set<string>`) e la chiave di sincronizzazione `chart-<sectionId>`. | Spec §4. Un solo viewport, quindi le strisce scorrono insieme. |
| DM8 | **`StripLegend.svelte`:** una per striscia.<br>**Props:** `title: string`, `rows: Array<{ id; label; color; dash; now: string; min: string; max: string; hidden: boolean }>`, `ontoggle(id)` e `onremove(id)`.<br>**Valore `now`:**<br>• senza cursore: `format(convert(store.measured(id)))`;<br>• col cursore: il valore della colonna in `cursorIdx`.<br>`—` (`DASH`) se assente.<br>**Righe:**<br>• il nome è un `<button>` che nasconde o mostra la serie, con `aria-pressed` = visibile;<br>• la × è un `<button>` con `aria-label` = `advanced.chart.remove` (`{name}`).<br>**Titolo:** `advanced.chart.unit.<unit>`, con una voce per ognuno dei 16 valori di `Unit`. | Spec §2.4. |
| DM9 | **Limiti tolti solo nel grafico a strisce:**<br>• `MAX_SERIES`, `MAX_UNITS`, `canAdd`, `fitSelection`, `initialSeries`, `scaleLayout` e la chiave `advanced.chart.maxSeries` restano com'erano, per `HistoryChart`;<br>• per `StripChart`, `knownSelection(ids: string[], candidates: string[]): string[]` tiene gli id presenti in `candidates`, nell'ordine e senza duplicati;<br>• `initialStripSeries(saved: string[] \| null, candidates: string[], defaults: string[]): string[]` ha le regole di `initialSeries`, ma sopra `knownSelection`.<br>`HistoryChart.svelte` e `HistoryChart.test.ts` restano. | Spec §2.2 e §5.4. |
| DM10 | **Tessera cliccabile con i tooltip dentro:** `Tile.svelte` diventa un `<div class="tile">`. L'etichetta è un `<button class="open">` il cui `::after` copre tutta la tessera (`position: absolute; inset: 0`), così il clic su qualunque punto apre la vista Avanzata. Il contenuto con un `Term` ha `position: relative; z-index: 1`. | `Term` è focalizzabile e non può stare dentro un `<button>` (commento in `Term.svelte`). Lo schema della «scheda con link esteso» tiene il clic su tutta la tessera. |
| DM11 | **Larghezze:** in `App.svelte`, `main` ha `class:wide={view === 'simple' \|\| view === 'advanced'}`, con `.wide { max-width: 1600px }`. Le altre viste restano a 1100 px. | Spec §5.1. |
| DM12 | **Griglia della Semplice** (`app/src/lib/simpleGrid.ts`): `simpleGrid(count: number): { columns: number; spans: number[]; rows: number }`.<br>• 1 → `{1, [1], 1}`;<br>• 2 → `{2, [1,1], 1}`;<br>• 3 → `{2, [1,1,2], 2}`;<br>• 4 → `{2, [1,1,1,1], 2}`;<br>• 5 → `{6, [2,2,2,3,3], 2}`;<br>• 6 → `{3, [1×6], 2}`;<br>• ≥ 7 → `{3, [1×count], ceil(count/3)}`.<br>**Riempimento:** il modo «riempie» vale con larghezza del contenuto ≥ 900 px (container query su `.simple`) e `window.innerHeight` ≥ 720, solo per `rows ≤ 2`. In questo modo:<br>• `.simple` è alta `calc(100dvh - var(--simple-top) - 20px)`, con `--simple-top` = il `getBoundingClientRect().top` dell'elemento, misurato al mount e a ogni `resize`;<br>• le righe sono `repeat(rows, 1fr)`.<br>Sotto, la griglia di oggi. | Spec §5.2. Il conteggio delle tessere varia con le GPU visibili. |
| DM13 | **Sparkline e numeri:**<br>• `Sparkline.svelte`: `height: 100%` con `min-height: 34px` (oggi 34 px fissi); il contenitore nella tessera ha `flex: 1`;<br>• numeri grandi: `.big { font-size: clamp(28px, 2.2vw, 40px) }`, e `.big.rate` resta proporzionato con `clamp(20px, 1.6vw, 30px)`. | Spec §5.2. |
| DM14 | **Dati delle tessere** (`select.ts`), con i campi nuovi accanto a quelli di oggi:<br>• `CpuSummary`: `temperatureC` (`cpu.temperature.package`, poi `cpu.temperature.tctl`), `powerW` (`cpu.power.package`), `busiestThread: number \| null` (massimo di `cpu.load.thread`) e `threads: Array<{ id: string; value: number \| null }>` (`cpu.load.thread`, in ordine di schema);<br>• `GpuSummary`: `hotspotC` (`gpu.temperature.hotspot`), `powerLimitW` (`gpu.power.limit`), `fanPct` (`gpu.fan.percent`), `fanRpm` (`gpu.fan.rpm`) e `throttle: 'power' \| 'thermal' \| null` (`gpu.throttle.power`, poi `gpu.throttle.thermal`, se il valore è 1);<br>• `MemorySummary`: `availableBytes` (totale meno usata, mai sotto 0) e `dimmTempsC: number[]` (`memory.temperature.dimm` con un valore);<br>• `NetworkSummary`: `adapterName` (il nome del primo dispositivo di rete) e `linkBps` (`network.linkSpeed` del primo);<br>• nuova `diskRows(schema, valueOf, powerOf: (deviceId: string) => string \| undefined): DiskRow[]`, con `DiskRow { deviceId; name; letter: string \| null; usedPct; activePct; readBps; writeBps; temperatureC; standby: boolean }`, nell'ordine dei dischi nello schema (lo stesso di `nav.ts`).<br>`letter` è l'`arg` del primo `storage.volumeUsed` del disco; `standby` vale `powerOf(id) === 'standby'`. | Spec §5.3. Le chiavi sono quelle che usano già `pages.ts` e lo schema. |
| DM15 | **Glossario:** `MONITOR_TERMS = ['hotspot', 'busiestThread', 'threadLoad', 'powerLimit', 'linkSpeed', 'diskActivity', 'dimm']` in `app/src/lib/performance/glossary.ts`. Usano anche le voci esistenti `vram` e `throttling`.<br>Il test del glossario controlla che ogni termine abbia `glossary.<term>` e `glossary.<term>.name` in tutte e due le lingue. | Regola dei tooltip. |
| DM16 | **Pagina dello stress** (`StressRun.svelte`), con `HistoryChart` e `fixedWindow` come oggi:<br>• `stressChartSensors(schema: Schema \| null, component: StressComponent, objective: Objective, gpuDeviceId: string \| null, diskDeviceId: string \| null): Sensor[]`, in `app/src/lib/performance/format.ts`, al posto della scelta di oggi in `StressRun`; segue la tabella del §5.4 della spec;<br>• **CPU:** temperatura (`CPU_TEMPERATURES`), poi `cpu/0/power/package` o, in overclock, il primo `cpu.clock.effective`;<br>• **RAM:** il primo `memory.load`, poi ogni `memory.temperature.dimm`; senza moduli, la temperatura della CPU;<br>• **GPU:** `<gpu>/temperature/core` e `<gpu>/temperature/hotspot`, poi `<gpu>/power/board` o, in overclock, `<gpu>/clock/core`;<br>• **disco:** `diskChartSensors` di oggi.<br>Le funzioni `cpuChartSensors`, `gpuChartSensors` e `diskChartSensors` restano, usate da `stressChartSensors`.<br>**Prop nuova di `HistoryChart`:** `fixedSeries?: boolean`. Se vale `true`, nasconde il `<details>` «Serie», usa sempre `defaults` (filtrati da `fitSelection`), e non legge né salva la selezione. `StressRun` la passa a `true`. | Scelta dell'utente del 2026-10-09: il vecchio grafico resta allo stress, con le serie scelte dal test. |

## Review Focus

1. **Molte serie con la finestra di 1 h.**
   - **Atteso:** frame fatti solo di `transform`; una striscia fuori dallo schermo non ricalcola.
   - **Test:**
     - M2: `offscreen_strips_skip_set_data_until_visible`, `frames_only_translate_every_strip`;
     - M7: la misura.
2. **Cambio di unità o di schema col grafico aperto** (°C ↔ °F, una nuova revisione in cui un sensore sparisce).
   - **Atteso:** le strisce si rifanno dal buffer, °C e °F restano una sola striscia, e la serie sparita esce senza lasciare una striscia vuota.
   - **Test (M2):** `temperature_unit_change_keeps_one_strip`, `schema_revision_regroups_strips`.
3. **Si toglie l'ultima serie di un'unità con il cursore sopra o col cassetto aperto.**
   - **Atteso:** la striscia sparisce, l'indice del cursore torna `null`, nessuna eccezione.
   - **Test:**
     - M2: `removing_the_last_series_of_a_unit_drops_its_strip`;
     - M3: `unticking_in_the_drawer_drops_an_empty_strip`.
4. **Ricerca nel cassetto con accenti e maiuscole, e «tutti» con un filtro attivo.**
   - **Atteso:** «temperatúra» e «TEMPERATURA» trovano «Temperatura»; «tutti» spunta solo le righe visibili.
   - **Test (M3):** `search_ignores_case_and_accents`, `all_ticks_only_filtered_rows`.
5. **Semplice senza alcuni sensori** (niente PawnIO, quindi niente DIMM; GPU senza ventola) **o con un disco in standby.**
   - **Atteso:** i chip mancanti non ci sono, senza «—»; il disco in standby mostra «In standby» e nessun valore.
   - **Test (M5):** `missing_sensors_hide_their_chips`, `standby_disk_shows_standby_only`.

## Task

### Task M1: funzioni pure del grafico

**Files:**
- Modify:
  - `app/src/lib/advanced/chartData.ts`, `chartData.test.ts`;
  - `app/src/lib/advanced/chartCanvas.ts`, `chartCanvas.test.ts`;
  - `app/src/lib/i18n/en.json`, `it.json`.
- Create: `app/src/lib/advanced/drawer.ts`, `drawer.test.ts`.

**Interfaces:**
- Produces:
  - `Strip`, `stripLayout`, `STRIP_DASHES`, `stripPalette`, `windowExtent` e `ChartBuffer.columns` (DM1–DM3);
  - `dash` su `ChartCanvasPath` e `ChartHeldSegment`, e l'argomento `options` di `drawChartCanvas` (DM4);
  - `DrawerGroup { unit: Unit; sensors: Sensor[] }`;
  - `foldText(text: string): string`: NFD senza i segni combinanti, poi `toLowerCase()`;
  - `drawerGroups(sensors: Sensor[], stripUnits: Unit[], filter: string, label: (s: Sensor) => string): DrawerGroup[]`: prima le unità di `stripUnits` nel loro ordine, poi le altre nell'ordine della prima apparizione in `sensors`; un sensore entra se `foldText(label(s))` contiene `foldText(filter.trim())`; i gruppi vuoti non ci sono;
  - le chiavi i18n `advanced.chart.unit.<unit>` per i 16 valori di `Unit`. In italiano:
    - Temperatura, Percentuale, Frequenza, Potenza, Tensione, Corrente;
    - Velocità di rotazione, Dati, Velocità di trasferimento, Velocità in bit, Energia;
    - Stato, Generazione PCIe, Linee, Ore, Conteggio.

  `MAX_SERIES` e gli altri restano fino a M2.

- [ ] **Step 1: test che falliscono.**
  - In `chartData.test.ts`:
    - `strip_layout_groups_by_unit_in_first_seen_order`: carico, temperatura, carico2 → due strisce, `percent` con `[0, 2]`, poi `celsius` con `[1]`;
    - `strip_layout_skips_unknown_ids`;
    - `strip_palette_repeats_colours_with_dashes`: indice 0 → `--accent`, `[]`; 8 → `--accent`, `[6, 4]`; 16 → `[2, 3]`; 24 → `[]`;
    - `window_extent_ignores_nulls_and_samples_before_the_window`;
    - `window_extent_of_an_empty_column_is_null`;
    - `columns_copies_only_the_chosen_series_and_converts_them`.
  - In `chartCanvas.test.ts`:
    - `dashed_paths_set_and_reset_the_line_dash`;
    - `time_labels_false_draws_grid_lines_without_labels`.
  - In `drawer.test.ts`:
    - `groups_follow_strip_units_then_schema_order`;
    - `search_ignores_case_and_accents` (Review Focus 4): «temperatúra» e «TEMPERATURA» trovano «Temperatura»;
    - `empty_groups_are_dropped`.
- [ ] **Step 2:** `cd app && pnpm test chartData chartCanvas drawer`. Atteso: FAIL.
- [ ] **Step 3: implementare** DM1–DM4 e le funzioni del cassetto.
- [ ] **Step 4:** `pnpm test && pnpm check`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): pure helpers for the strip chart and the series drawer`.

### Task M2: grafico a strisce e legenda con i valori

È il task più delicato: usare il modello più capace. Gli invarianti del canvas composito vanno tenuti tutti, e sono quelli che i test di `HistoryChart.test.ts` già fissano.

**Files:**
- Create:
  - `app/src/components/advanced/StripChart.svelte`, `StripChart.test.ts` (dai test di `HistoryChart.test.ts`);
  - `app/src/components/advanced/ChartStrip.svelte`;
  - `app/src/components/advanced/StripLegend.svelte`, `StripLegend.test.ts`.
- Modify:
  - `app/src/components/advanced/DevicePage.svelte`;
  - `app/src/lib/advanced/chartData.ts`, `chartData.test.ts` (DM9);
  - `app/src/lib/i18n/en.json`, `it.json`;
  - `app/src/test/uplot-stub.ts`, se serve `cursor.sync`.

`HistoryChart.svelte` e `HistoryChart.test.ts` non si toccano.

**Interfaces:**
- Consumes: M1.
- Produces:
  - `StripChart` con le props di `HistoryChart`;
  - `ChartStrip` (DM6);
  - `StripLegend` (DM8);
  - `knownSelection` e `initialStripSeries` (DM9).

  Per M3, `StripChart` espone al cassetto:
  - `chosen`;
  - `toggle(id: string)`;
  - `setMany(ids: string[], on: boolean)`;
  - le unità delle strisce (`strips.map((s) => s.unit)`).

- [ ] **Step 1: test che falliscono.**
  - **Copia:** `StripChart.test.ts` nasce da una copia di `HistoryChart.test.ts`, con `renderChart` che monta `StripChart`. Ogni test di oggi resta, con lo stesso nome e le stesse asserzioni, **tranne** questi, riscritti come indicato:
    - `seeds the default series and draws them on two unit scales` → `one_plot_per_unit_with_its_own_y_axis`: carico e temperatura danno due plot; `plots[0]` ha la serie `percent` e l'asse `percent` a sinistra, `plots[1]` `celsius`; una sola richiesta di storico per le due serie;
    - `the picker allows at most 8 series…` → `the_picker_has_no_series_limit_and_saves_per_section`: 12 serie spuntabili, salvate;
    - `the picker refuses a third unit` → `a_third_unit_adds_a_third_strip`;
    - `the secondary axis never repeats a label…` → la stessa verifica sull'asse Y della striscia `celsius`, senza la parte dell'asse secondario;
    - `hides the composited path and marker when the legend hides a series` e i test sulle serie nascoste → il clic sul nome in `StripLegend` al posto della legenda di uPlot;
    - i test che contano «il plot» → contano il plot della striscia interessata.
  - **Test nuovi in `StripChart.test.ts`:**
    - `only_the_last_strip_draws_time_labels`: `drawChartCanvas` riceve `{ timeLabels: false }` per le altre;
    - `strip_heights_are_260_alone_and_130_plus_axis_when_stacked`;
    - `frames_only_translate_every_strip` (Review Focus 1): fra due dati un frame cambia solo `style.transform` dei canvas;
    - `offscreen_strips_skip_set_data_until_visible` (Review Focus 1), con un `IntersectionObserver` finto;
    - `legend_shows_latest_values_without_cursor_and_cursor_values_on_hover`;
    - `legend_min_max_follow_the_visible_window`;
    - `remove_button_drops_the_series_and_saves`;
    - `removing_the_last_series_of_a_unit_drops_its_strip` (Review Focus 3): con il cursore attivo, `cursorIdx` torna `null`;
    - `temperature_unit_change_keeps_one_strip` (Review Focus 2);
    - `schema_revision_regroups_strips` (Review Focus 2);
    - `cursor_sync_key_is_shared_by_the_strips_of_a_chart`;
    - `known_selection_keeps_order_and_drops_unknown_and_duplicates` (in `chartData.test.ts`).
  - **In `StripLegend.test.ts`:**
    - `name_button_toggles_and_reports_pressed`;
    - `remove_has_an_accessible_name`;
    - `dashed_swatch_matches_the_series`.
- [ ] **Step 2:** `cd app && pnpm test StripChart StripLegend chartData`. Atteso: FAIL.
- [ ] **Step 3: implementare** DM5–DM9 e DM16.
  - **Fonte del codice:** si copia da `HistoryChart.svelte` (stessi commenti, stessi nomi), senza riscriverlo. `HistoryChart.svelte` resta com'è per lo stress test.
  - **Picker:** resta il `<details>` di oggi, senza limiti e senza il testo `maxSeries`, finché M3 non lo sostituisce.
  - **Layout di ogni striscia:** una griglia `1fr 220px` con 12 px di spazio; a destra la legenda.
- [ ] **Step 4:** `pnpm test && pnpm check && pnpm build`. Atteso: PASS. `HistoryChart` resta solo in `StressRun.svelte` e nei suoi test (`git grep -l HistoryChart app/src`).
- [ ] **Step 5: commit** `feat(ui): strip chart, one plot per unit, with a value legend`.

### Task M3: cassetto «Serie»

Usare `frontend-design:frontend-design`, con il mockup `picker-v2.html` come riferimento.

**Files:**
- Create: `app/src/components/advanced/SeriesDrawer.svelte`, `SeriesDrawer.test.ts`.
- Modify:
  - `app/src/components/advanced/StripChart.svelte`, `StripChart.test.ts`;
  - `app/src/lib/i18n/en.json`, `it.json`.

**Interfaces:**
- Consumes: `drawerGroups` e `foldText` (M1); `chosen`, `toggle` e `setMany` (M2).
- Produces: `SeriesDrawer`, con le props:
  - `groups: DrawerGroup[]`;
  - `chosen: string[]`;
  - `filter: string` (bindable);
  - `valueText: (id: string) => string`;
  - `unitTitle: (unit: Unit) => string`;
  - `ontoggle(id)`;
  - `onsetmany(ids, on)`;
  - `onclose()`.

  Chiavi i18n nuove:
  - `advanced.chart.search`: «Cerca sensore…»;
  - `advanced.chart.all`: «tutti»;
  - `advanced.chart.none`: «nessuno».

- [ ] **Step 1: test che falliscono.**
  - In `SeriesDrawer.test.ts`:
    - `rows_show_the_current_value`;
    - `all_ticks_only_filtered_rows` (Review Focus 4);
    - `none_unticks_the_filtered_rows_of_the_group`;
    - `two_columns_from_520px`.
  - In `StripChart.test.ts`:
    - `series_button_opens_the_drawer_over_the_lines_only`: il cassetto ha `left` = 72 px e `right` = 232 px rispetto al contenitore delle strisce;
    - `drawer_closes_on_escape_outside_click_and_button_and_returns_focus`;
    - `search_has_focus_on_open`;
    - `ticking_a_new_unit_adds_a_strip_while_open`;
    - `unticking_in_the_drawer_drops_an_empty_strip` (Review Focus 3);
    - `chart_grows_to_360px_while_the_drawer_is_open`;
    - `button_reports_aria_expanded_and_count`.
- [ ] **Step 2:** `cd app && pnpm test SeriesDrawer StripChart`. Atteso: FAIL.
- [ ] **Step 3: implementare** il §3 della spec:
  - posizione da DM5;
  - sfondo `color-mix(in srgb, var(--surface) 88%, transparent)`, bordo `--accent` e ombra, senza `backdrop-filter`;
  - `role="dialog"` non modale;
  - entrata di 150 ms (`transform: translateX`), tolta con `prefers-reduced-motion`.

  Il `<details>` si cancella.
- [ ] **Step 4:** `pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): series drawer over the plot lines, with live values`.

### Task M4: larghezze e griglia della Semplice

Usare `frontend-design:frontend-design`, con il mockup `simple-2x2-data.html` come riferimento per proporzioni e spazi.

**Files:**
- Create: `app/src/lib/simpleGrid.ts`, `simpleGrid.test.ts`.
- Modify:
  - `app/src/App.svelte`;
  - `app/src/components/simple/SimpleView.svelte`, `SimpleView.test.ts`;
  - `app/src/components/simple/Tile.svelte`;
  - `app/src/components/common/Sparkline.svelte`.

**Interfaces:**
- Produces:
  - `simpleGrid` (DM12);
  - `Tile` con le stesse props (`label`, `onclick`, `children`) e il markup di DM10.

- [ ] **Step 1: test che falliscono.**
  - In `simpleGrid.test.ts`, `grid_for_1_to_7_tiles`, con i valori di DM12.
  - In `SimpleView.test.ts`:
    - `the_whole_tile_opens_the_advanced_page`: il `<button class="open">` ha il nome della tessera, e il clic chiama `onOpenAdvanced`;
    - `five_tiles_span_two_and_three_columns`;
    - `fill_mode_needs_720px_of_height`.
  - In un test di `App`, se c'è (altrimenti in `SimpleView.test.ts` sulla classe di `main`): `simple_and_advanced_are_wide_performance_is_not`.
- [ ] **Step 2:** `cd app && pnpm test simpleGrid SimpleView`. Atteso: FAIL.
- [ ] **Step 3: implementare** DM10–DM13.
- [ ] **Step 4:** `pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): wide layout and a filling grid for the Simple view`.

### Task M5: dati in più nelle tessere

**Files:**
- Modify:
  - `app/src/lib/select.ts`, `select.test.ts`;
  - `app/src/components/simple/SimpleView.svelte`, `SimpleView.test.ts`;
  - `app/src/lib/performance/glossary.ts` e il suo test;
  - `app/src/lib/i18n/en.json`, `it.json`.

**Interfaces:**
- Consumes:
  - `Tile` (M4);
  - `store.diskPower(deviceId)`, come in `DevicePage`.
- Produces:
  - i campi e `diskRows` di DM14;
  - `MONITOR_TERMS` (DM15);
  - le chiavi `tile.*` nuove:
    - `tile.temperature`, `tile.power`, `tile.clock`, `tile.busiestThread`, `tile.threadLoad`;
    - `tile.hotspot`, `tile.fan`, `tile.throttle.power` («Limitata dalla potenza»), `tile.throttle.thermal` («Limitata dal calore»);
    - `tile.available`, `tile.dimm`, `tile.link`;
    - `tile.disk.activity`, `tile.disk.read`, `tile.disk.write`, `tile.disk.temperature`.

- [ ] **Step 1: test che falliscono.**
  - In `select.test.ts`:
    - `cpu_summary_has_temperature_power_and_threads`;
    - `gpu_summary_reports_throttle_only_when_set`;
    - `memory_available_never_negative`;
    - `disk_rows_follow_schema_order_with_letters`;
    - `network_summary_has_adapter_and_link`.
  - In `SimpleView.test.ts`:
    - `missing_sensors_hide_their_chips` (Review Focus 5): senza `gpu.fan.*` e `memory.temperature.dimm` quei chip non ci sono, e nessun chip mostra `—`;
    - `standby_disk_shows_standby_only` (Review Focus 5);
    - `thread_bars_have_their_value_in_a_tooltip`;
    - `technical_labels_use_term`.
  - Nel test del glossario: `monitor_terms_have_texts`.
- [ ] **Step 2:** `cd app && pnpm test select SimpleView glossary`. Atteso: FAIL.
- [ ] **Step 3: implementare** DM14, DM15 e il §5.3 della spec.
  - **Chip:** un elenco `.chips` con etichetta muta e valore, come nel mockup.
  - **Barre dei thread:** CSS, con `height: <valore>%` e il valore nel `title`.
  - **Tabella dei dischi:** una griglia a 5 colonne.
  - **Testi del glossario** (`it.json`; `en.json` con lo stesso senso):
    - `hotspot`: «Il punto più caldo del chip della GPU, di solito qualche grado sopra la temperatura media.»;
    - `busiestThread`: «Il carico del thread più impegnato in questo momento: dice se un programma usa un solo core al massimo.»;
    - `threadLoad`: «Una barra per ogni thread della CPU: più è alta, più quel thread lavora.»;
    - `powerLimit`: «Quanto la GPU consuma rispetto al massimo che la scheda le permette.»;
    - `linkSpeed`: «La velocità massima concordata con la rete, non quella che stai usando.»;
    - `diskActivity`: «La parte di tempo in cui il disco sta lavorando: al 100% è sempre occupato.»;
    - `dimm`: «I moduli di RAM montati sulla scheda madre; la temperatura c'è solo con i sensori avanzati.».
- [ ] **Step 4:** `pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): more readings in the Simple view tiles`.

### Task M6: grafico dello stress test

**Files:**
- Modify:
  - `app/src/lib/performance/format.ts`, `format.test.ts`;
  - `app/src/components/advanced/HistoryChart.svelte`, `HistoryChart.test.ts`;
  - `app/src/components/performance/StressRun.svelte` e il suo test, se c'è.

**Interfaces:**
- Produces: `stressChartSensors` e la prop `fixedSeries` (DM16).

- [ ] **Step 1: test che falliscono.**
  - In `format.test.ts`, una riga per caso della tabella del §5.4:
    - `stress_chart_cpu_normal_is_temperature_and_power`;
    - `stress_chart_cpu_overclock_is_temperature_and_clock`;
    - `stress_chart_ram_is_used_and_dimm_temperatures`;
    - `stress_chart_ram_without_dimms_falls_back_to_cpu_temperature`;
    - `stress_chart_gpu_normal_is_core_hotspot_and_power`;
    - `stress_chart_gpu_overclock_is_core_hotspot_and_clock`;
    - `stress_chart_disk_is_temperature_read_and_write`;
    - `stress_chart_skips_missing_sensors`.
  - In `HistoryChart.test.ts`:
    - `fixed_series_hides_the_picker_and_ignores_the_saved_selection`: con una selezione salvata per `performance`, il grafico mostra i `defaults`;
    - `fixed_series_saves_nothing`.
- [ ] **Step 2:** `cd app && pnpm test format HistoryChart`. Atteso: FAIL.
- [ ] **Step 3: implementare** DM16. `StressRun` passa `fixedSeries` e prende le serie da `stressChartSensors(store.schema, status.component, status.objective, status.gpuDeviceId, status.disk?.deviceId || performanceStore.diskDeviceId)`.
- [ ] **Step 4:** `pnpm test && pnpm check && pnpm build`. Atteso: PASS.
- [ ] **Step 5: commit** `feat(ui): stress chart series follow the test`.

### Task M7: misure, documenti e prove dal vivo con l'utente

**Files:**
- Modify:
  - `README.md`;
  - `CLAUDE.md`;
  - `docs/follow-ups.md`;
  - `docs/perf-budget.md` (risultati).

- [ ] **Step 1: via dell'utente.** Si chiede all'utente prima della build release e delle misure, che caricano la CPU. Poi `pwsh scripts/measure-footprint.ps1` (vedi `docs/perf-budget.md` per i parametri) sui casi (a), (b) e (c) del §6 della spec. Il caso (b) a 1h si misura con la cronologia piena.
- [ ] **Step 2: se il caso (b) supera il budget,** si abbassa la cadenza dei frame delle strisce con più di 16 serie (spec §6), con un test, e si misura di nuovo. Il risultato va in `docs/follow-ups.md`, «Open: charts (M9a)».
- [ ] **Step 3: documenti** (spec §8). In `CLAUDE.md`:
  - la struttura `StripChart`, `ChartStrip`, `StripLegend` e `SeriesDrawer`;
  - la M9a nell'elenco delle milestone.
- [ ] **Step 4: prove dal vivo**, fatte dall'utente, una alla volta, senza input sintetico:
  - **Z1:** pagina CPU con carico, temperatura, potenza e clock. Ci sono 4 strisce allineate, il cursore è condiviso, la legenda ha valori, min e max.
  - **Z2:** il cassetto. Le linee si vedono sotto, mentre assi e legenda restano scoperti; la ricerca e «tutti» funzionano; Esc e il clic fuori lo chiudono.
  - **Z3:** la × e il clic sul nome nella legenda.
  - **Z4:** le pagine GPU, RAM, Disco e Rete, e la pagina dello stress con le strisce.
  - **Z5:** la Semplice a schermo intero, 2 × 2, con i dati nuovi e i tooltip. La finestra piccola torna come oggi.
  - **Z6:** con la GPU integrata visibile (se l'utente lo può fare), 5 tessere in 3 + 2.
  - **Z7:** uno stress test breve della CPU e uno della RAM (preset «Rapido»). Il grafico mostra le serie della tabella del §5.4, senza il pannello «Serie».
- [ ] **Step 5: commit** `docs: M9a strip chart and full-window layout`.
