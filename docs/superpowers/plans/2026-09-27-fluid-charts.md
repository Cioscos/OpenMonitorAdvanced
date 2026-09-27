# Grafici fluidi — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Far scorrere i grafici delle viste Semplificata e Avanzata a circa 60 FPS, con curve senza overshoot, punto finale bianco e glow leggero.

**Architecture:** Un coordinatore `requestAnimationFrame` serve tutti i grafici visibili. uPlot resta il renderer della vista Avanzata: i dati cambiano solo ai nuovi snapshot, la scala X avanza tra gli snapshot. I minigrafici SVG usano timestamp condivisi da `LiveStore` per la stessa finestra temporale continua.

**Tech Stack:** Svelte 5.57, TypeScript 6.0, uPlot 1.6.32, Vitest 5, WebView2/Tauri 2.11.

**Spec:** `docs/superpowers/specs/2026-09-27-fluid-charts-design.md`; leggere anche §7.3, §7.5 e §14 di `docs/superpowers/specs/2026-09-24-openmonitor-advanced-design.md`.

## Global Constraints

- Frequenza visiva iniziale massima 60 FPS; il contratto del coordinatore ammette 30 e 15 FPS, senza impostazione utente in questa modifica.
- Nessun campione sintetico: KPI, legenda, statistiche e log conservano i valori reali.
- Massimo 8 serie e 2 unità nel grafico Avanzata; storico decimato per finestre da 30 minuti e 1 ora, con `null` come buco.
- Un solo ciclo di rendering, fermo a finestra nascosta e con `prefers-reduced-motion: reduce`; ripresa riallineata allo storico.
- Il glow non sfoca griglia, assi, etichette o testo. Il punto finale è bianco e manca quando l'ultimo campione è assente.
- Nel build release su display a 60 Hz: mediana ≥ 55 FPS e p95 del frame ≤ 20 ms; CPU app a riposo < 1%, finestra < 200 MB, tray < 30 MB. Mai degradare silenziosamente a 30/15 FPS.
- Codice, commenti e commit in inglese; documentazione in italiano; fine riga LF. Non usare input desktop sintetici nelle verifiche dal vivo.

## Review Focus

1. Timestamp irregolari o snapshot in ritardo: scorrimento continuo senza salti all'arrivo del dato (Task 2 e 3).
2. Campione finale `null` dopo valori validi: curva interrotta e nessun punto bianco su un valore vecchio (Task 4 e 5).
3. Ritorno dell'orologio di sistema o cambio schema: nessun segmento collega epoche incompatibili (Task 2 e 3).
4. Finestra nascosta o movimento ridotto: nessun ciclo attivo, ripartenza senza recupero accelerato (Task 1 e 2).
5. Serie piatta, picco stretto o timestamp coincidenti: curva finita senza overshoot, punto dentro il riquadro (Task 4 e 5).

---

## File structure

- `app/src/lib/chartFrameClock.ts`: unico rAF, limite FPS, visibilità e movimento ridotto.
- `app/src/lib/advanced/chartViewport.ts`: conversione monotona da tempo rAF a finestra X, con riallineamento ai campioni.
- `app/src/lib/live.svelte.ts`: buffer circolare condiviso dei timestamp dei minigrafici.
- `app/src/lib/sparkline.ts`: geometria SVG della finestra temporale, curva monotona e ultimo punto.
- `app/src/components/common/Sparkline.svelte`: sottoscrizione al clock, path, glow e punto.
- `app/src/components/advanced/HistoryChart.svelte`: scorrimento uPlot, spline, passata glow e punto via API native di disegno.
- Test omonimi in `app/src/lib/` e `app/src/components/`; `app/src/test/uplot-stub.ts` registra le chiamate di scala/disegno.
- `docs/perf-budget.md`: protocollo e risultati misurati nella build finale.

### Task 1: Coordinatore dei frame

**Files:** Create `app/src/lib/chartFrameClock.ts`, `app/src/lib/chartFrameClock.test.ts`.

**Interfaces:** Produce `type ChartFps = 60 | 30 | 15`, `subscribeChartFrame(callback: (monotonicMs: number) => void, fps?: ChartFps): () => void`. Un singleton è l'unico proprietario di `requestAnimationFrame`; la sottoscrizione restituisce la funzione di cleanup.

- [ ] **Step 1: Write the failing tests.** `chartFrameClock.test.ts`: due iscritti condividono un solo rAF; 60/30/15 limitano i callback secondo `performance.now`; ultimo unsubscribe cancella il rAF; `visibilitychange` e `matchMedia('(prefers-reduced-motion: reduce)')` lo fermano; alla ripresa arriva il tempo corrente senza replay dei frame mancati.
- [ ] **Step 2: Run `cd app && pnpm test -- chartFrameClock.test.ts`.** Expected: FAIL perché il modulo non esiste.
- [ ] **Step 3: Implement `subscribeChartFrame` in `chartFrameClock.ts`.** Registra i listener DOM solo mentre esiste almeno un iscritto; per display sotto 60 Hz non programmare timer aggiuntivi né cambiare la risoluzione del timer Windows.
- [ ] **Step 4: Run `cd app && pnpm test -- chartFrameClock.test.ts && pnpm check`.** Expected: PASS e zero errori TypeScript/Svelte.
- [ ] **Step 5: Commit.** `git add app/src/lib/chartFrameClock.ts app/src/lib/chartFrameClock.test.ts && git commit -m "feat(charts): share a visibility-aware frame clock"`.

### Task 2: Scorrimento uPlot e prima prova prestazionale

**Files:** Create `app/src/lib/advanced/chartViewport.ts`, `app/src/lib/advanced/chartViewport.test.ts`; modify `app/src/components/advanced/HistoryChart.svelte`, `app/src/components/advanced/HistoryChart.test.ts`, `app/src/test/uplot-stub.ts`.

**Interfaces:** Consumes `subscribeChartFrame`. Produces `createChartViewport(windowSeconds: number): { sample(timestampMs: number, receivedMonoMs: number): void; range(nowMonoMs: number): { min: number; max: number } | null; reset(): void }`, con range uPlot in secondi Unix. `sample` non torna indietro per ritardi normali; un rollback esplicito chiama `reset` prima del nuovo campione.

- [ ] **Step 1: Write failing tests.** `chartViewport.test.ts`: al tempo monotono +250 ms la finestra avanza di 0,25 s; un nuovo campione tardivo non sposta X all'indietro; timestamp inferiore dopo `reset` apre una nuova epoca; `range` prima del primo campione è `null`. In `HistoryChart.test.ts`: `setData` è chiamato solo allo snapshot, `setScale('x', …)` sui frame; dopo hide/unmount non cambia scala; alla ripresa non recupera molti secondi in un frame; con movimento ridotto un nuovo snapshot è visibile pur senza rAF. Estendere `FakeUplot` per registrare `setScale`.
- [ ] **Step 2: Run `cd app && pnpm test -- chartViewport.test.ts HistoryChart.test.ts`.** Expected: FAIL nei nuovi casi.
- [ ] **Step 3: Implement `createChartViewport` e integrare il clock in `HistoryChart.svelte`.** Ancorare la scala al timestamp dell'ultimo campione e a `performance.now`; usare `plot.setData(buffer.data(), false)` soltanto allo snapshot e `plot.setScale('x', range)` sui frame. Quando il clock è sospeso per movimento ridotto, ridisegnare direttamente allo snapshot. Preservare reseed, null, due scale Y, resize e cleanup esistenti. Limare solo la correzione dell'ancora temporale, non il valore del sensore.
- [ ] **Step 4: Run `cd app && pnpm test -- chartViewport.test.ts HistoryChart.test.ts && pnpm check && pnpm build`.** Expected: PASS, check pulito, build riuscita.
- [ ] **Step 5: Misurare presto uPlot nella release WebView2.** Con `scripts/seed-advanced-view.ps1` predisporre la pagina GPU a 8 serie e 1 h; tenere la finestra visibile secondo la regola di `CLAUDE.md`, raccogliere una traccia WebView2 di rAF/disegno e `scripts/measure-footprint.ps1` per almeno 60 s dopo il riempimento dello storico. Annotare hardware, refresh del display, FPS mediano, p95, CPU e memoria in `docs/perf-budget.md`. Se fallisce il target, ottimizzare questo percorso e ripetere; se uPlot resta insufficiente, fermarsi e aggiornare il design per la scelta del renderer, senza passare a 30/15 FPS.
- [ ] **Step 6: Commit.** Includere solo codice, test e risultati osservati: `git commit -m "feat(charts): scroll uPlot on the shared frame clock"`.

### Task 3: Timestamp condivisi per i minigrafici

**Files:** Modify `app/src/lib/live.svelte.ts`, `app/src/lib/live.test.ts`.

**Interfaces:** Produces `LiveStore.seriesTimestampsMs(): number[]`, con lunghezza e ordine pari a `series(id)`; i valori storici mancanti hanno comunque un timestamp. Il buffer è condiviso tra sensori, massimo `capacity` elementi.

- [ ] **Step 1: Write failing tests.** `live.test.ts`: snapshot irregolari conservano gli istanti esatti; `seedHistory` allinea timestamp e serie; cambio schema riempie i nuovi sensori con assenti senza disallineare; rollback azzera valori e timestamp insieme; superata `capacity`, entrambi espellono lo stesso campione.
- [ ] **Step 2: Run `cd app && pnpm test -- live.test.ts`.** Expected: FAIL nei nuovi casi.
- [ ] **Step 3: Implement `seriesTimestampsMs()` in `LiveStore`.** Usare `SeriesBuffer` o un ring `Float64Array` di pari capacità; aggiornare `applySnapshot`, `seedHistory` e il reset per rollback senza modificare il contratto del backend.
- [ ] **Step 4: Run `cd app && pnpm test -- live.test.ts && pnpm check`.** Expected: PASS e check pulito.
- [ ] **Step 5: Commit.** `git commit -m "feat(charts): retain shared sparkline timestamps"`.

### Task 4: Scorrimento e resa dei minigrafici SVG

**Files:** Modify `app/src/lib/sparkline.ts`, `app/src/lib/sparkline.test.ts`, `app/src/components/common/Sparkline.svelte`, `app/src/components/simple/SimpleView.svelte`; create `app/src/components/common/Sparkline.test.ts`.

**Interfaces:** Consumes `seriesTimestampsMs()` e `subscribeChartFrame`. Produce `sparklineGeometry(values: number[], timestampsMs: number[], rightEdgeMs: number, windowMs: number, width: number, height: number, min: number, max?: number): { path: string; endpoint: { x: number; y: number } | null }`. La funzione attuale `sparklinePath` può restare come wrapper per i chiamanti/test esistenti.

- [ ] **Step 1: Write failing tests.** `sparkline.test.ts`: X avanza fra snapshot, campioni irregolari non saltano, `NaN` spezza il path e rimuove il punto se è l'ultimo campione, dati piatti/uno solo/timestamp coincidenti producono coordinate finite. I segmenti cubici non superano gli estremi dei campioni; i valori aggregati rete/disco seguono gli stessi timestamp. `Sparkline.test.ts`: un solo punto bianco, path colorato e glow semitrasparente; nessun rAF dopo unmount o movimento ridotto.
- [ ] **Step 2: Run `cd app && pnpm test -- sparkline.test.ts Sparkline.test.ts`.** Expected: FAIL nei nuovi casi.
- [ ] **Step 3: Implement `sparklineGeometry` e aggiornare i componenti.** Usare una spline cubica monotona per ogni tratto contiguo, clippare la finestra di cinque minuti, passare i timestamp da `SimpleView`. Disegnare il glow SVG con una seconda path più larga e opaca solo in parte; cerchio finale bianco con raggio costante e clipping nel viewport. La sottoscrizione usa il tempo monotono per far avanzare `rightEdgeMs` dall'ultimo snapshot.
- [ ] **Step 4: Run `cd app && pnpm test -- sparkline.test.ts Sparkline.test.ts && pnpm check && pnpm build`.** Expected: PASS, check pulito, build riuscita.
- [ ] **Step 5: Commit.** `git commit -m "feat(charts): scroll smooth sparklines with endpoints"`.

### Task 5: Curve, punto e glow in uPlot

**Files:** Modify `app/src/components/advanced/HistoryChart.svelte`, `app/src/components/advanced/HistoryChart.test.ts`; create `app/src/lib/advanced/chartDecoration.ts`, `app/src/lib/advanced/chartDecoration.test.ts`.

**Interfaces:** Consumes il clock e il viewport del Task 2. Produce `drawChartSeriesDecoration(plot: uPlot, seriesIdx: number, strokePath: Path2D | null, gapsClip: Path2D | null, color: string): void`, invocato dal hook `drawSeries` uPlot. Il tracciato e la clip provengono da una wrapper della funzione pubblica `uPlot.paths.spline()` configurata per ciascuna serie; il punto si legge dall'ultima cella della colonna corrispondente in `plot.data`.

- [ ] **Step 1: Write failing tests.** `HistoryChart.test.ts`: ogni serie usa `uPlot.paths.spline()` senza unire i `null`; colori e scale restano invariati; al cambio serie/reseed il punto segue il sensore giusto. `chartDecoration.test.ts` con canvas finto: un `null` finale non disegna punto; punti validi usano bianco, clipping al plot e ai buchi, glow solo sul tratto; picco/serie piatta restano nei limiti Y. Verificare la spline monotona inclusa in uPlot 1.6.32 con un fixture di tre campioni prima di usarla.
- [ ] **Step 2: Run `cd app && pnpm test -- HistoryChart.test.ts chartDecoration.test.ts`.** Expected: FAIL nei nuovi casi.
- [ ] **Step 3: Implement `drawChartSeriesDecoration` e configurare `HistoryChart.svelte`.** La wrapper di `uPlot.paths.spline()` conserva il `stroke` e il `clip` del disegno corrente in una closure per serie; il hook `drawSeries` li passa alla funzione, senza leggere proprietà private di uPlot. `plot.ctx` disegna soltanto la passata tenue colorata e il punto bianco, con `save`/`restore` e `bbox` per non sfocare griglia/assi/testo. Se il fixture smentisce il limite di overshoot, sostituire il tracciato con una spline monotona equivalente prima di proseguire.
- [ ] **Step 4: Run `cd app && pnpm test -- HistoryChart.test.ts chartDecoration.test.ts && pnpm check && pnpm build`.** Expected: PASS, check pulito, build riuscita.
- [ ] **Step 5: Commit.** `git commit -m "feat(charts): add smooth uPlot lines and live markers"`.

### Task 6: Verifica integrata e budget finale

**Files:** Modify `docs/perf-budget.md`; modify eventuali test o file dei Task 1–5 soltanto per difetti trovati.

**Interfaces:** Consumes le due viste complete. Produces evidenza riproducibile dei criteri del design, senza nuova API di prodotto.

- [ ] **Step 1: Run `cd app && pnpm test && pnpm check && pnpm build`.** Expected: tutti i test verdi, zero errori, build riuscita.
- [ ] **Step 2: Verificare la resa delle due viste nel build release WebView2.** Osservare curva, bianco e glow rispetto allo screenshot; verificare visibilità, movimento ridotto, assenza di dati, 1/5/30/60 min e 8 serie. Non usare clic sintetici sul desktop; predisporre la pagina con lo script esistente e chiedere all'utente solo le azioni manuali necessarie.
- [ ] **Step 3: Ripetere la misura completa.** Una misura da almeno 60 s con storia piena, poi una dopo almeno un'ora di finestra visibile. Registrare FPS mediano, p95, CPU e memoria per vista Semplificata e Avanzata in `docs/perf-budget.md`, con data, hardware, build e procedura. Eseguire anche la misura tray. Se i criteri falliscono, correggere e ripetere soltanto la misura interessata; una migrazione di renderer richiede revisione del design approvato.
- [ ] **Step 4: Aggiornare il grafo locale.** `PYTHONHASHSEED=0 graphify update .` secondo `CLAUDE.md`; se il launcher resta indisponibile, registrarlo nel report senza bloccare la verifica del codice.
- [ ] **Step 5: Run `git diff --check` and review the branch against the spec.** Expected: nessun errore di whitespace, tutti i requisiti coperti; ottenere la revisione finale del branch prima del merge locale.
- [ ] **Step 6: Commit.** `git commit -m "docs: record fluid chart performance"`; integrare il branch secondo `CLAUDE.md` dopo la revisione.
