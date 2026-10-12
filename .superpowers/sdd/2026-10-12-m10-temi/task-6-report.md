# Task 6 report

## Substitutions (app/src/components)
- advanced/ChartStrip.svelte:586 `background: white` -> `var(--text)` (endpoint dot; stays visible on light themes)
- advanced/HistoryChart.svelte:809 same
- advanced/ChartStrip.svelte:388,438,466 and HistoryChart.svelte:458,475,521: grid colour read from `--border` -> `--chart-grid` (variable `border` -> `grid`)
- advanced/SeriesDrawer.svelte:133 `rgb(0 0 0 / 0.6)` -> `var(--shadow)` (Synthwave shadow is 50%, was 60%: negligible)
- simple/HealthBanner.svelte:190 `rgb(0 0 0 / 45%)` -> `var(--shadow)` (50% vs 45%)
Only 4 spots existed in `<style>` blocks; earlier tasks had already moved most colours. No backdrop use found in styles (no `--backdrop` consumer needed).

## Exceptions (in test, each with reason)
- performance/Gauge.svelte hex colours in style: physical dial with fixed dark face.
- editor/Canvas.svelte `#140F1E`: simulated game background (script only, none in `<style>`; entry is defensive).
- ChartStrip/HistoryChart comments quoting uPlot defaults `#607D8B`, `rgba(0,0,0,.07)`.
Not scanned (markup, not `<style>`): Gauge.svelte SVG attributes, Sparkline.svelte inline white dot (`style="...background: white"` in markup, left as is; brief scope is `<style>` blocks; flag for later), editor profile default colours (data).

## Tests / TDD
- New `app/src/styles/noFixedColors.test.ts` (scans `<style>` blocks of every .svelte under app/src for hex, rgb(a), hsl(a), black/white).
- RED: failed listing the 4 spots above. GREEN: after edits. Full: 93 files, 1238 tests pass; `pnpm check` 0 errors; `pnpm build` OK.
- Existing chart tests needed no change (they pass gridColor explicitly).

## Screenshots
`.superpowers/sdd/2026-10-12-m10-temi/task-6-shots/{before,after}-{simple,advanced}.png` (dev browser, mock backend). Dev server stopped.

## Concerns
- Synthwave chart grid is now `--chart-grid` (#2d204280, border at 50% alpha): slightly dimmer than before by design.
- Sparkline inline white dot (see above).

## Fix round 1
- Gauge.svelte: dial face derived from the theme (classes in `<style>`, SVG attributes removed): bezel `color-mix(--text 35%/22%, --surface-2)` and `--surface-2`; well/hub ring/LCD `--bg`; carbon `--surface`/`--surface-2`; vignette `--bg`; gloss and matrix dots `--text` (mask now `mask-type: alpha`, so dots work on light themes); hub `color-mix(--text 45%, --surface-2)` to `--surface`; ticks/title `--text`, tick labels `--text-muted`. Gauge exception removed from the test.
- Sparkline.svelte:176 `background: white` -> `var(--text)`; Sparkline.test.ts:250 now asserts the inline `background: var(--text)` (jsdom does not resolve var()).
- noFixedColors.test.ts: second test over markup outside `<style>`/`<script>` (inline `style=`, `style:x=`, SVG `fill|stroke|stop-color|flood-color` attributes); exceptions list is empty.
- RED (Gauge+Sparkline stashed): both tests failed, 4 Gauge style lines, 1 Sparkline + 14 Gauge attribute lines. GREEN: `cd app && pnpm test` 93 files / 1239 tests pass; `pnpm check` 0 errors.
- Screenshots (dev browser, mock; daylight = daylight variables set on `<html>`): task-6-shots/fix-gauge-synthwave.png, fix-gauge-daylight.png, fix-simple-synthwave.png, fix-simple-daylight.png. Synthwave gauge looks close to before; daylight gauge is legible. Dev server stopped.
