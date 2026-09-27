# Grafici fluidi compositati Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mantenere lo scorrimento di linee e asse X vicino a 60 FPS, con punto bianco fisso a destra, glow leggero e budget release rispettati in entrambe le viste.

**Architecture:** uPlot conserva dati, legenda e scala Y; un canvas statico per campione contiene curve e asse X e viene traslato dal compositor tra i campioni. I minigrafici ricalcolano i path SVG solo ai campioni e traslano la geometria; un piccolo livello fisso mostra tratto mantenuto e punto.

**Tech Stack:** Svelte 5, TypeScript 6, uPlot 1.6.32, Canvas 2D, SVG/CSS transforms, Vitest, Tauri 2.11/WebView2.

**Spec:** `docs/superpowers/specs/2026-09-27-fluid-charts-design.md` (revisione approvata, commit `7f8f4fb`).

## Global Constraints

- Sensori e storico restano ai timestamp reali; il tratto piatto fino al bordo è solo visivo e non entra in KPI, legenda, statistiche o log.
- Frequenza iniziale vicina a 60 FPS, clock condiviso già predisposto per 30/15; non abbassare il limite in modo silenzioso.
- Curve monotone senza overshoot; glow solo sulle linee; punto bianco al bordo destro per ultimo valore valido ancora nella finestra; ultimo valore assente elimina tratto e punto.
- Linee, griglia verticale, tacche ed etichette X scorrono insieme; scala Y cambia solo ai campioni reali e, quando cambia intervallo, transisce per 180 ms (istantanea con movimento ridotto).
- Finestra nascosta, tray e movimento ridotto fermano il movimento; resize, DPR, lingua, tema e cambio serie/range ricostruiscono le geometrie senza saltare nel tempo.
- Verifica release su Windows/WebView2: storico pieno, 8 serie Avanzata, almeno 60 s di traccia, poi almeno un'ora visibile; FPS mediano ≥55 e p95 ≤20 ms sul display a 60 Hz, CPU app+WebView2 <1%, memoria finestra <200 MB, tray <30 MB. Sul display attuale a 164 Hz riportare anche il refresh e non spacciare il risultato per una prova a 60 Hz.
- Non usare clic sintetici o UI Automation sul desktop; concordare con l'utente l'osservazione delle finestre. Le modifiche sono nel worktree `feat/fluid-charts` e non vanno integrate finché i gate non sono soddisfatti.

## Review Focus

1. Snapshot in ritardo rispetto alla posizione animata: il canvas nuovo deve iniziare dalla stessa posizione visibile, senza arretrare (Task 1/4).
2. Ultimo valore `null`, `NaN` o campione ormai fuori finestra: nessun tratto mantenuto né punto; i buchi delle linee restano separati (Task 2/3/4).
3. Più serie con due scale Y e valori sovrapposti: ogni punto usa la propria scala e conserva colore e dimensione, senza glow su griglia/testo (Task 3/4).
4. Resize/DPR/lingua/tema durante il movimento: canvas e tacche si riallineano e le risorse precedenti vengono rilasciate (Task 4/5).
5. Visibilità/movimento ridotto o ritorno dalla tray con snapshot nel frattempo: nessun rAF nascosto e nessun recupero accelerato al ritorno (Task 2/5).

---

### Task 1: Modello di traslazione e tacche temporali

**Files:** Create `app/src/lib/chartCompositor.ts`, `app/src/lib/chartCompositor.test.ts`; modify `app/src/lib/advanced/chartViewport.ts` and `app/src/lib/advanced/chartViewport.test.ts` only if its public time anchor cannot supply the base or needs another delayed-snapshot assertion.

**Interfaces:** Produce `scrollOffsetPx(baseRightMs: number, visibleRightMs: number, windowMs: number, plotWidthPx: number): number`, `heldLengthPx(lastSampleMs: number | null, visibleRightMs: number, windowMs: number, plotWidthPx: number): number | null`, and `timeTicks(minSeconds: number, maxSeconds: number, incrementSeconds: number): number[]`. Pure functions; later tasks own DOM and canvas.

- [ ] **Step 1: Write failing tests** asserting `scrollOffsetPx(base, base+250, 60_000, 600) === 2.5`, `heldLengthPx(base, base+250, 60_000, 600) === 2.5`, null/out-of-window/nonfinite endpoint returns `null`, and `timeTicks(60, 120, 30)` returns `[30, 60, 90, 120, 150]` for the one-tick overscan on both sides. Test the delayed-snapshot rebase with `createChartViewport.sample()` in its existing test file.
- [ ] **Step 2: Run** `cd app; pnpm test -- chartCompositor.test.ts` and confirm failures are the missing functions/expected behavior.
- [ ] **Step 3: Implement** the three pure functions above. Clamp nonfinite dimensions and negative elapsed offset; use the `createChartViewport` range as the source of `visibleRightMs`, not `Date.now()`.
- [ ] **Step 4: Run** the focused test, `pnpm check`, then commit `feat: add composited chart timing geometry`.

### Task 2: Minigrafici SVG traslati

**Files:** Modify `app/src/components/common/Sparkline.svelte`, `app/src/components/common/Sparkline.test.ts`, `app/src/lib/sparkline.ts`, `app/src/lib/sparkline.test.ts`.

**Interfaces:** Consume Task 1 functions and existing `subscribeChartFrame()`/`sparklineGeometry()`. A snapshot rebuilds `geometry.path` once; rAF updates only the DOM transform of the path group and la lunghezza del tratto mantenuto. `sparklineGeometry()` still returns the last real endpoint for Y and gap decisions.

- [ ] **Step 1: Write failing tests**: path `d` stays byte-identical across two rAF ticks while transform advances; endpoint remains at `left:100%` with a connecting flat segment; `NaN` final or last sample outside 5 min removes both; delayed snapshot and clock rollback do not jump; visibility/reduced motion stop frames and unmount releases listeners.
- [ ] **Step 2: Run** `cd app; pnpm test -- Sparkline.test.ts sparkline.test.ts`; verify the new tests fail on the current per-frame path renderer.
- [ ] **Step 3: Implement** a clipped `<g>` containing both colored paths, set its CSS transform imperatively from the shared clock, and keep segment/white marker outside that group. Rebase on snapshot using `createChartViewport`; never append a held value to the prop arrays. Convert the offset from `scrollOffsetPx()` to the rendered SVG's CSS pixel width while retaining the 150×34 SVG viewBox.
- [ ] **Step 4: Run** focused tests, `pnpm check`, `pnpm build`; commit `feat: composite simple sparklines between samples`.

### Task 3: Disegno statico di serie e asse X Avanzata

**Files:** Create `app/src/lib/advanced/chartCanvas.ts`, `app/src/lib/advanced/chartCanvas.test.ts`; modify `app/src/test/uplot-canvas.ts` only for realistic Canvas API test fixtures.

**Interfaces:** Produce `drawChartCanvas(ctx: CanvasRenderingContext2D, plot: uPlot, paths: ReadonlyArray<{stroke: Path2D | null; gapsClip: Path2D | null; color: string}>, ticks: readonly number[], locale: string, overscanPx: number, theme: {gridColor: string; textColor: string}): void`. Input paths come from public `uPlot.paths.spline()` results captured by Task 4; input ticks from Task 1. Draw in canvas pixel coordinates with `plot.bbox` clipping.

- [ ] **Step 1: Write failing tests**: 8 series with two Y units draw colored lines and translucent glow only once per snapshot; `null` gaps keep their clip; vertical grid/tick labels are not blurred; one future tick lands inside right overscan; empty paths draw nothing; `uPlot.pxRatio=2` preserves size/position.
- [ ] **Step 2: Run** `cd app; pnpm test -- chartCanvas.test.ts`; confirm expected failures.
- [ ] **Step 3: Implement** the static painter. Reuse the captured uPlot spline Path2D so the current monotone fixture remains valid; draw X labels with `formatTimeTick()` and the `theme` argument supplied at rebuild. Save/restore context around clipping and glow.
- [ ] **Step 4: Run** focused tests, `pnpm check`; commit `feat: draw advanced chart on composited canvas`.

### Task 4: Integrazione uPlot e canvas compositato

**Files:** Modify `app/src/components/advanced/HistoryChart.svelte`, `app/src/components/advanced/HistoryChart.test.ts`, `app/src/test/uplot-stub.ts`; remove `app/src/lib/advanced/chartDecoration.ts` and its test after equivalent coverage in Tasks 3/4.

**Interfaces:** Consume `scrollOffsetPx()`, `heldLengthPx()`, `timeTicks()` from Task 1 and `drawChartCanvas()` from Task 3. uPlot owns real data, Y scale, legend and plot bounds; its series path factory captures Path2D but returns no native stroke. Its X grid/ticks/values are hidden while preserving axis layout; use uPlot's X-axis split callback to capture its selected tick increment for `timeTicks()` so the canvas and axis spacing agree. A transparent canvas with right overscan paints line/glow and X elements on each snapshot/reseed/resize; a shared rAF changes only its `translateX`. Fixed marker elements draw one held segment and white point per valid series at the right plot boundary.

- [ ] **Step 1: Write failing integration tests**: `setScale('x')` is not called on ordinary rAF frames; `setData` is still called at most once per snapshot; canvas and X grid share the Task 1 offset; series colors/dual Y scales are preserved; right-edge point and held segment disappear for null/out-of-window final values; delayed snapshot, range/series change and rollback rebase without a visible X jump.
- [ ] **Step 2: Run** `cd app; pnpm test -- HistoryChart.test.ts`; confirm failures against the existing uPlot-per-frame path.
- [ ] **Step 3: Integrate** static painter and compositor in `HistoryChart.svelte`. Keep last offscreen predecessor (commit `5287167`); size overlay to the plot width plus the overscan on both sides, clip it at the plot/X-axis boundary, and use `ResizeObserver` for resize. On every rebuild, derive the canvas base from `viewport.range(now)` so a delayed snapshot starts at the previous visible edge. Remove old per-frame `drawScale()` and decoration hook only after the replacement tests are green.
- [ ] **Step 4: Run** focused tests, entire `pnpm test`, `pnpm check`, `pnpm build`; commit `feat: composite advanced chart and time axis`.

### Task 5: Gate prestazionale anticipato su WebView2

**Files:** Modify `docs/perf-budget.md` with raw results; no product code unless profiling identifies a specific defect.

**Interfaces:** Depends on Tasks 2/4. This gate decides whether to proceed to the Y transition/long soak or return to the design's WebGL comparison.

- [ ] **Step 1: Build** `cd app; pnpm tauri build --no-bundle`, record commit and SHA-256; ensure no other `oma-app.exe` is running and seed Avanzata GPU, 1 min, 8 series with `scripts/seed-advanced-view.ps1`.
- [ ] **Step 2: With the user's visible-window coordination**, run clean 30 s warm-up + 60 s `scripts/measure-footprint.ps1 -Service` for Avanzata and Semplificata, plus separate 60 s WebView2 traces. Report CPU validity, total CPU/memory, actual 164 Hz refresh, frame rate and p95. The tracing run does not substitute for a clean footprint sample.
- [ ] **Step 3: If a budget fails**, profile the offending view, fix a proven cause with red/green tests and rerun its gate. If the compositor architecture remains insufficient, compare a WebGL candidate and return to the user for a revised spec before migration. Record failure and evidence; do not silently lower FPS.
- [ ] **Step 4: Commit** verified interim measurements to `docs/perf-budget.md`; keep the branch isolated.

### Task 6: Transizione Y e lifecycle

**Files:** Modify `app/src/components/advanced/HistoryChart.svelte`, `app/src/components/advanced/HistoryChart.test.ts`, `app/src/lib/advanced/chartViewport.ts` only if needed, and `app/src/components/common/Sparkline.test.ts` for pause/resize regressions.

**Interfaces:** Consume Task 4 overlay. A changed automatic Y range interpolates uPlot's Y scale and overlay mapping together for 180 ms, cancels/rebases on the next snapshot, and jumps immediately under reduced motion. The shared clock and one Y transition reuse one rAF loop.

- [ ] **Step 1: Write failing tests**: Y interval change moves paths and Y tick values together during 180 ms; a second snapshot cancels and starts from current displayed Y; reduced motion applies the final range at once; hide/tray/unmount cancel rAF; resize, DPR, locale and theme rebuild canvas without duplicate listeners or stale point positions.
- [ ] **Step 2: Run** `cd app; pnpm test -- HistoryChart.test.ts Sparkline.test.ts`; verify failures.
- [ ] **Step 3: Implement** the bounded transition and lifecycle rebuild. Avoid rerendering the full plot on normal X frames; restrict any temporary uPlot redraw to the 180 ms Y transition.
- [ ] **Step 4: Run** focused tests, entire `pnpm test`, `pnpm check`, `pnpm build`; commit `feat: synchronize y transitions and chart lifecycle`.

### Task 7: Verifica finale release e integrazione

**Files:** Modify `docs/perf-budget.md`; update `docs/superpowers/specs/2026-09-27-fluid-charts-design.md` only if a measured design change is approved.

**Interfaces:** Uses `scripts/measure-footprint.ps1`, `scripts/seed-advanced-view.ps1`, the release binary, WebView2 trace helper and user observation. No synthetic desktop input.

- [ ] **Step 1: Verify** full `pnpm test`, `pnpm check`, `pnpm build`, release `pnpm tauri build --no-bundle`, `git diff --check`, and update `PYTHONHASHSEED=0 graphify update .`. Record exact outputs and executable SHA-256.
- [ ] **Step 2: Coordinate** the visual check of Semplificata and Avanzata at 1/5/30/60 min, 8 series, null values, held dots, X grid, pause/reduced motion and resize on the release.
- [ ] **Step 3: Fill history** for 61 min and trace at least 60 s per view; separately leave each window continuously visible for ≥1 h and measure the final 60 s. Run the tray sample. Record process-tree validity and raw CPU/memory, FPS median/p95, frame counts, screenshot and any visual defect. Re-test a failed row after a targeted fix.
- [ ] **Step 4: Request** a whole-branch code review, fix findings with tests, repeat affected gates, and commit the final `docs/perf-budget.md` results. Use `superpowers:verification-before-completion` and `superpowers:finishing-a-development-branch`; integrate locally only after the approved criteria are met.
