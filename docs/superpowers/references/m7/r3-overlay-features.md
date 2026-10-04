# Research: in-game overlay (OSD) with layout editor — feature landscape

Date: 2026-10-04. Purpose: input for the OpenMonitor Advanced overlay design spec. No code written.
Confidence tags: [V] = verified in a fetched page; [S] = from a search snippet only; [I] = inference/synthesis of mine.
Caveat: several vendor pages (MSI, Intel, TechRadar, NotebookCheck, EA forums, HWiNFO forum) returned 401/402/403 to the fetcher, and Reddit is poorly indexed by the search tool. The "what users value" ranking in section 3 is therefore a synthesis of feature sets that recur across products, release notes, and forum threads, not a poll.

---------------------------------------------------------------------------
## 0. The single most important architectural finding

There are two ways to get pixels on top of a game, and every product in this space is one of them:

| Approach | Used by | Pros | Cons |
|---|---|---|---|
| A. Hook/injection into the game's graphics API (draw into the back buffer) | RTSS (so MSI Afterburner, HWiNFO-OSD, CapFrameX OSD), MangoHud (Vulkan/GL layer), Special K, Steam/Discord/NVIDIA legacy overlays | Works in exclusive fullscreen; appears in game-capture (OBS "Game Capture", ShadowPlay) | Anti-cheat blocks/bans risk, per-API support matrix (DX9/10/11/12/Vulkan/OpenGL), crashes, FPS cost; needs admin-ish/service-level injection |
| B. Independent topmost window composited by DWM (+ MPO to keep Independent Flip) | NVIDIA FrameView 1.9 ("renders as an independent window rather than drawing into the application's back buffer"), Intel PresentMon 2.x, Xbox Game Bar widgets | No injection, anti-cheat friendly, API-agnostic | Not visible over true exclusive fullscreen unless a high Z-band is used; may break Independent Flip without MPO; visible in desktop capture unless excluded |

Key facts:
- Microsoft: in classic Fullscreen Exclusive the OS cannot draw overlays; overlay vendors "would have to step into and intercept the rendering process, ... performance regressions, instability and issues with anti-cheat". Fullscreen Optimizations run games as borderless windowed behind the scenes, so DWM can composite an overlay on top with a small overhead. https://devblogs.microsoft.com/directx/demystifying-full-screen-optimizations/ [V]
- FrameView 1.9: overlay is an independent window; "won't appear" for OpenGL/Vulkan apps in legacy fullscreen that take ownership of the screen, and on systems without Multi-Plane Overlay (MPO) support; a legacy overlay can be restored via settings.ini. https://www.nvidia.com/en-ph/geforce/technologies/frameview/release-notes/ [V]
- PresentMon places its overlay in a higher Z-band using the undocumented `CreateWindowInBand`, which requires manifest `uiAccess=true`, a **cryptographically signed** executable, **run from a trusted location (e.g. C:\Program Files)**. Its UI is CEF (Chromium Embedded Framework) — i.e. a web UI, same family as our WebView2. https://docsearch.algolia.com/mcp/docs/repo/gametechdev/presentmon [V]; anti-cheat/separate-window rationale and MPO reliance [S] (search snippet, same query).
- Implication for OMA [I]: our project already signs (SignPath planned, installer in Program Files). Approach B with a Tauri/WebView2 transparent click-through topmost window is the natural fit, with uiAccess/Z-band as a later (signed-only) enhancement. Tauri supports `set_ignore_cursor_events` for click-through, though there are open issues of click-through on WebView windows: https://github.com/tauri-apps/tauri/issues/9250 [S]. WebView2 renders in a separate GPU process (msedgewebview2.exe), so an always-on-screen web overlay has a real GPU/CPU cost that must be budgeted against `docs/perf-budget.md`. [I]
- Capture exclusion for approach B: `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` (Windows 10 2004+; earlier builds behave as WDA_MONITOR = black rectangle). Window shows on the monitor, absent from OBS display capture/screenshots/Recall; flag can be toggled dynamically. https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity [S], https://www.meziantou.net/how-to-exclude-your-windows-app-from-screen-capture-and-recall.htm [V]. This gives OMA a clean "hide from stream/recording" switch that injected overlays cannot offer (and the reverse option "show in capture" is the default for injected ones). Note OBS "Game Capture" (hook-based) never sees window overlays anyway, only "Display Capture"/"Window capture" does. [I]

---------------------------------------------------------------------------
## 1. RTSS OverlayEditor plugin and MSI Afterburner OSD

### 1.1 OverlayEditor concepts
Introduced with Afterburner 4.6.3 beta / RTSS 7.3.0 beta 6 (plugin `OverlayEditor.dll`). https://videocardz.com/newz/msi-afterburner-4-6-3-beta-and-rtss-7-3-0-beta-6-bring-overlay-editor-plugin (title only; 402 on body) [S]. Master thread: https://forums.guru3d.com/threads/rtss-overlay-editor-megathread.436443/

- **Layout** = a set of **layers** + **data sources** + **text tables** + global properties (refresh rate is global in layout properties, not per layer). Files: `.ovl` (legacy) and `.ovx` (current), in `...\RivaTuner Statistics Server\Plugins\Client\Overlays\`. Layouts are loaded via Layouts > Load; **Merge** (7.3.7) combines layouts or copies data sources/layers/text tables between them. https://www.guru3d.com/files-details/rtss-rivatuner-statistics-server-download.html [V], https://www.cloudspress.com/rtss-overlay-editor-megathread-setup-layouts-formulas-and-troubleshooting/ [V]
- **Layers**: Layers > Add (or Insert) creates a "Text layer"; double-click opens Layer Properties; bring-to-front / send-to-back exist. Layer kinds mentioned: text (with hypertext), graph, bar, table, image/sprite/gauge (via `<I>`, `<AI>` tags), background. Position is explicit per layer (`<P=x,y>`). There is **no free-form drag canvas with grid snap** documented in what I could fetch; layout is largely coordinate/text-flow based. (This is the opening for OMA's visual grid editor — see 1.5.) https://mybyways.com/blog/cpu-and-gpu-monitoring-overlay-for-gaming [V]
- **Render-order quirk**: graphs are batched and drawn before/after text regardless of layer order ("geometry batching"); 7.3.2 beta 2 allowed proper Z-ordering. Source: megathread pages above [V]. Lesson: make z-order explicit and predictable in our editor.
- **Data sources**: internal HAL (CPU/GPU/RAM), external HWiNFO64 via shared memory, framerate/frametime (always available), PresentMon-derived (GPU busy, 7.3.5). Per source: **correction formula** (variable `x`, operators + - * / % ^, parentheses, references to other sources; e.g. `x/1024` for MB→GB), **custom unit label**, **format string** (`%0.1f`). Statistic functions: `statmax("src")`, `swmax("src", N)` (sliding-window max up to 1024 samples); 7.3.7 adds `reflexlatency()`, `presentmonlatency()`. https://www.cloudspress.com/...troubleshooting/ [V]
- **Current value vs graph per layer**: in Layer Properties tick "Add current value macro" for a number, or "Add embedded graph" (optionally "use custom template" → graph properties). Embedded graph styles: **line, filled area, bar**; several graphs can be stacked on the same area with semi-transparent backgrounds. https://mybyways.com/blog/... [V]
- **Hypertext tags** (the real formatting engine, authored as text): `<S=n>` font size % (positive superscript, negative subscript), `<C=aarrggbb>` colour and **`<C=formula>` dynamic colour by value ranges** (and recomputed per graph sample when placed before an embedded object), `<A=n>` alignment boxes (positive left / negative right), `<P=x,y>` / `<P0..P8>` absolute position or screen anchors, `<B=x,y[,radius]>` filled rectangle (7.3.7 adds rounded corners), `<G=...>` graph, `<I=...>` image, `<AI=...>` sprite animation/gauge, `<IF><ELSE>` and `<SWITCH><CASE>` conditionals, `<TT=name>` text tables, `<FNT>` font override (7.3.7), slots 0-249 to store/recall size/colour/alignment, macros `%CPU% %GPU% %RAM% %VRAM% %Driver% %Time24% %Date% %Timer%`, static tags `<API> <EXE> <RES> <ARCH>`, frame tags `<FR> <FT> <FRMIN> <FRMAX> <FRAVG> <FR01L> <FR10L> <BTIME>`. https://forums.guru3d.com/threads/list-of-all-hypertext-tags.437486/ [V]. Font *typeface* is global, not changeable via tags "for performance reasons". [V]
- **Conditional layers** (RTSS 7.3.5): layers shown/transformed by conditions on sensor values; sample use is a GPU-bound / CPU-bound indicator ("GPU limited when GPU busy / frametime >= 0.75") with PresentMon integration. https://forums.guru3d.com/threads/download-rtss-7-3-5-beta-5.449364/ [V]
- **Hotkeys / layout switching**: HotkeyHandler plugin loads different `.ovl` layouts by hotkey (minimal vs benchmarking). [V] Process-specific overlays are not possible because one client formats the layout for all running 3D processes; workaround is manual hotkey switching. https://forums.guru3d.com/threads/rtss-overlay-editor-megathread.436443/ [V]
- **Benchmark stats**: Min/Avg/Max/1%/0.1% only populate while an RTSS benchmark is running, started/stopped by a manually assigned hotkey: "You need to manually set a Hotkey for Benchmarking and Start/Stop your Benchmark for them to show in the OSD." [V]. HWiNFO can show "Framerate 1% Low" and "0.1% Low" only if RTSS benchmark mode is running. https://www.hwinfo.com/forum/threads/is-there-a-way-to-show-the-1-lows-in-the-osd-with-hwinfo64-rtss-or-is-only-fps-frametime-available-thanks.8152/ [S]
- **Documented pain**: tag syntax has no end-user documentation ("closest thing is the RTSS SDK and the commented RTSSSharedMemorySample source", Unwinder). https://forums.guru3d.com/threads/afterburner-rtss-osd-custom-layout-tag-syntax-documentation.427545/ [V]. Users report hypertext editing needs SDK knowledge; dynamic value-based colour was initially refused by the author as "annoying" before formulas arrived; multi-client OSD stacking conflicts need manual `<P>`; ".ovl overlays only show while the Overlay Editor is open" when the plugin isn't enabled; duplicate data if HWiNFO "Show value in OSD" is also on. [V]
- Community "skin" ecosystem exists (GitHub repos of `.ovl` files, e.g. https://github.com/BreadPitch/THE-RTSS-Overlay, https://github.com/PeterKelemen2/RTSS-Overlay), proving demand for **shareable layout files**. [S]

### 1.2 MSI Afterburner OSD settings (the "classic" model)
- Per-sensor in Monitoring tab: tick "Show in On-Screen Display"; dropdown chooses **Text / Graph** (and "in Graph" variants; text+graph both possible). Per-sensor graph properties: width, height, min/max (graph limits), colour, plus **group name** (sensors with the same group name share one line), **group colour, value colour ("System color 0" in a Colors library), units colour**, "Alignments library" to tighten column spacing; drag to reorder; override name; text size for graph captions. https://www.gamingpcbuilder.com/msi-afterburner-overlay-customization-guide [V]; MSI blog (403) https://www.msi.com/blog/msi-afterburner-on-screen-display [S]
- OSD tab: hotkey "Toggle On-Screen Display", plus show/hide hotkeys; **benchmark** begin/end/clear-history hotkeys, benchmark results file (min/avg/max, 1% and 0.1% low). [S]
- RTSS side: position corner or X/Y coordinates, **On-Screen Display Zoom** (integer scale), **Raster 3D** font picker (raster vs vector font), shadow (colour/offset), "On-Screen Display fill" (translucent black background), per-application profiles (copied from Global *at creation*, not inherited afterwards), application detection level (None/Low/Medium/High), **Stealth mode** (anti-cheat), Show own statistics. https://forums.guru3d.com/goto/post?id=5414336 [S]. Text outline lives in the Global profile `[Font]` section, not in the editor [V].
- Complaints specific to the model: Zoom is integer-only ("2 is already a lot too big"), so tuning 4K/1440p readability is coarse; the fix is picking a larger Raster 3D font since "everything else is scaled" from it. https://forums.guru3d.com/threads/rtss-has-scaling-issues-with-nvidia-dsr-enabled.446229/latest (DSR scaling bug) [S]

### 1.3 CapFrameX overlay (RTSS front-end, open source)
- Entries list with drag-to-reorder; per entry: **group name** (same group = same line), colour, **threshold colours** (colour changes when a configurable value is exceeded/not reached), text size, separator line after entry; "Apply to multiple entries" copies formatting across entries of the same sensor type or group. **Three overlay configurations** persisted as JSON `OverlayEntryConfiguration_0/1/2` in `Documents\CapFrameX\Configuration`, selectable at runtime; four downloadable CPU/GPU templates. Sensor data from a customized Open Hardware Monitor library, rendered through RTSS. https://capframex.com/blog/post/How to configure the CapFrameX game overlay [V]
- **Run history** in OSD: 1-20 past benchmark runs (3-5 typically shown) with selectable metrics, oldest replaced when full; **aggregation** merges history into one record file and flags outliers. https://www.pcgameshardware.de/... (403) and search snippets [S]. Good example of a differentiator for reviewers.
- Best-practice advice: "Overlays that are too large or too busy can obscure gameplay"; build separate profiles for benchmarking vs casual. https://wccftech.com/how-to-set-up-high-quality-performance-overlays-with-rtss/amp/ [V]

### 1.4 HWiNFO + RTSS
HWiNFO publishes selected sensors to RTSS ("Show" in the RivaTuner OSD section of sensor Configure; needs "Shared Memory Support"; free edition's shared memory may turn off after 12 h / "8 hours" per forum). One-way push to an external renderer; users still need RTSS for drawing. https://www.hwinfo.com/forum/threads/support-of-rtss-displaying-hwinfo-sensor-values-in-games.45/post-821 [S]. Opportunity for OMA: it *is* the sensor source and the renderer, no double configuration.

### 1.5 What the RTSS model leaves open (OMA's differentiator)
Visual grid canvas with snap, WYSIWYG block selection + property panel, per-block "text vs graph" switch, z-order, and profile files — i.e. exactly what OverlayEditor makes hard (text-tag authoring, no snap grid, no docs). [I]

---------------------------------------------------------------------------
## 2. Other overlays

### 2.1 Steam in-game Performance Monitor
Beta 17 June (2025), Settings > In Game > Performance Monitor. Four detail levels: (1) FPS single value, (2) FPS detail + graph, (3) FPS detail + CPU and GPU utilization, (4) full FPS + CPU, GPU, RAM details. Metrics: avg FPS with and without frame generation, min/max FPS, frame-rate graph, avg/max CPU utilization and clock, GPU utilization, GPU temperature, VRAM, system RAM, per-core CPU graph option; detects DLSS/FSR frame generation and splits "real" FPS from generated FPS over 1 s intervals. Customization: detail level, screen position, contrast, saturation, background opacity. https://www.techspot.com/community/topics/steam-beta-adds-new-in-game-performance-metrics-overlay-can-show-dlss-frames.292994/ [S], https://www.3dtested.com/...steam-just-got-a-new-performance-overlay... [S] (523 on body). Take-away: **tiered presets (4 levels) are what mainstream users actually use**; frame-gen split is now expected.

### 2.2 NVIDIA app statistics overlay
Alt+Z opens overlay, Statistics > Custom to choose metrics; Alt+R toggles statistics. Layouts: horizontal 1-line, horizontal 2-line, vertical; text size and colour; position. Newer builds add CPU temp, clock, power. Missing: per-core stats, RAM, VRAM (per Guru3D). https://www.guru3d.com/story/nvidia-statistics-overlay-adds-cpu-temperature-power-and-clock-monitoring/ [V]
NVIDIA FrameView 1.7+: preset-based overlay (FPS only ... FPS + 1% low + PC latency + clocks), font size, black background for readability, mode tags (Full Screen Status, Tearing, VSync); accurate at 800+ FPS. https://www.nvidia.com/en-gb/geforce/technologies/frameview/release-notes [S]; 1.9 moved to independent-window overlay with "Drag Overlay" checkbox for repositioning, and acts as a system monitor when no game is running. [V]

### 2.3 AMD Adrenalin Metrics overlay
Ctrl+Shift+O toggles; Performance > Metrics > Overlay; visual customization: colours, **columns**, position, transparency, size; metrics: FPS, frametime, GPU utilization/temp/power/memory etc. https://amd.com/en/resources/support-articles/faqs/DH3-038.html [S]; https://community.amd.com:443/t5/pc-drivers-software/adrenalin-2019-18-12-2-amd-performance-overlay-improvements/td-p/150696 [S]

### 2.4 Intel PresentMon 2.x overlay (closest conceptual peer)
- Built-in overlay, own renderer (custom D3D11), CEF-based settings UI, Z-band window (see §0). https://github.com/GameTechDev/PresentMon [V]
- **Presets / "loadouts"**: Basic, GPU Focus, Power/Temp (3 built-ins, cycle-able, not editable); user creates custom loadout via Custom/Edit: add/remove **widgets**, choose chart type (**numeric readout, multi-line timeline graph, histogram**), colour, text size, position, window mode, width, time scale, graph scale, and the **statistic** per widget (average, 99th/95th/90th/10th/5th/1st percentile, min, max). Hotkeys for main functions are all editable. 2.0 added histograms; 2.3.1 added FPS-Presents/Display/App and GPU telemetry (freq, voltage, bandwidth, temp, power, fan); 2.6.0 (Sept 2026) adds multi-device per-metric selection, "Game Experience" preset, and keeps unavailable metrics visible but dimmed in pickers. https://www.neoteo.com/en/how-to-analyze-your-graphics-card-performance-with-intel-presentmon [V], https://www.pchardwarepro.com/en/How-to-optimize-performance-with-Presentmon-step-by-step/ [V], https://github.com/GameTechDev/PresentMon/releases [V], https://www.geeks3d.com/20230819/intel-presentmon-new-gaming-performance-overlay-and-telemetry-application/ [V]
- **GPU Busy** metric (GPU time vs frametime) to show CPU- vs GPU-bound; capture without visible overlay; hotkey capture to CSV (default 10 s); CLI option to disable overlay alpha blending (perf). [V]
- Overlay latency: ETW pipeline improved from ~1000 ms to ~30 ms in 2.2.0. [V] (shows how data latency matters)

### 2.5 MangoHud (Linux)
Config file `MangoHud.conf`; priority: app dir `MangoHud.conf` > per-app `~/.config/MangoHud/<app>.conf` or `wine-<exe>.conf` > global. https://git.blob42.xyz/... and https://www.gamingonlinux.com/2023/09/mangohud-v07-out-now-adding-presets-support-like-on-steam-deck/ [S]. Option families (from the shipped sample config) [V: https://raw.githubusercontent.com/flightlessmango/MangoHud/master/data/MangoHud.conf]:
- Layout: `position` (top-left, top-right, middle-left/right, bottom-left/right, top-center, bottom-center), `offset_x/y`, `width/height`, `horizontal`, `horizontal_stretch`, `hud_compact`, `hud_no_margin`, `table_columns`, `cellpadding_y`, `round_corners`, `legacy_layout`.
- Graphs: `graphs=gpu_load,cpu_load,gpu_core_clock,gpu_mem_clock,vram,ram,cpu_temp,gpu_temp`, `frame_timing` (frametime line), `frame_timing_detailed`, `dynamic_frame_timing`, `histogram` (switch FPS graph to histogram), `throttling_status_graph`.
- Colours (per metric: `cpu_color, gpu_color, vram_color, ram_color, frametime_color, text_color, background_color, text_outline_color`...), `alpha`, `background_alpha`.
- **Threshold colouring**: `gpu_load_value=60,90` + `gpu_load_color=...`, same for CPU; `fps_value` + `fps_color` (low/mid/high colours).
- Presets: `preset=` 0 no HUD, 1 FPS only, 2 horizontal, 3 extended, 4 detailed; Steam Deck-style performance levels; user presets in `presets.conf`; hotkey `toggle_preset` (default RShift+F10).
- Keys: `toggle_hud` (RShift+F12), `toggle_hud_position` (F11), `toggle_fps_limit` (LShift+F1), `toggle_logging` (LShift+F2), `reload_cfg` (LShift+F4), `reset_fps_metrics` (RShift+F9).
- `fps_limit=0,30,60` (comma list cycled by hotkey), `fps_limit_method=early|late`, `vsync`.
- Logging: `autostart_log`, `log_duration`, `log_interval`, `output_folder`, `benchmark_percentiles`.
- `blacklist`, `control` (socket remote control), `reload_cfg` (live reload).

### 2.6 Special K
Text OSD (colour, position, content selection), plus **widgets** (frame pacing graph, GPU, CPU) that are right-click-configurable with per-widget toggle keybinds; default OSD toggle Ctrl+Shift+O, control panel Ctrl+Shift+Backspace; very detailed GPU/CPU panels (temp, clocks, fan, VRAM, PCIe link, core parking); also a frame limiter. https://wiki.special-k.info/ ... and https://discourse.special-k.info/t/is-it-possible-to-change-the-style-of-the-performance-overlay/3038 [S]. Heavy injection; known compatibility friction.

### 2.7 FPS Monitor (commercial, Steam, ~$12.99)
Closest in spirit to the feature OMA wants: in-app **scene editor** (add/remove sensors, limit values, colours, sizes, fonts, alignment, position of any element), **several scenes switchable even during gameplay**, per-game profiles, desktop widgets, 2D graphs, hardware **alerts** at dangerous/critical values, FPS min/max/avg, frametime, 0.1%/1% lows, FPS lock, screenshots, hotkeys, DX9-12/OpenGL/Vulkan, claims to work with major anti-cheats. Steam reviews: "Mixed" last 30 days (54%), "Mostly Positive" overall (75%) — recent stability/regression complaints. https://store.steampowered.com/app/966610/FPS_Monitor/ [V]

### 2.8 Xbox Game Bar Performance widget
Win+G; CPU, GPU, VRAM, RAM, FPS; click category to see graph; **pin** keeps it always on top over games; options: graph position (e.g. bottom), hide metrics, accent colour, transparency. Free and zero-install; limited detail (no temps, no frametime, no lows). https://www.xda-developers.com/xbox-game-bar-resources-widget-track-cpu-gpu-ram-usage/ [S], https://www.howtogeek.com/706162/how-to-see-fps-in-any-windows-10-game-without-extra-software/ [S]. Game Bar widgets sit in a higher z-band than fullscreen optimizations (hence visible over games). [S via Microsoft/Special K wiki search]

---------------------------------------------------------------------------
## 3. Features users value most (synthesis)

Evidence base: product feature convergence (what every mature tool ships), release notes, and forum complaints. Not a survey. Ranking: **T** = table stakes (users assume it; absence is a complaint), **D** = differentiator (few tools do it well), **N** = nice-to-have.

| # | Feature | Class | Evidence / note |
|---|---|---|---|
| 1 | Toggle show/hide hotkey (global, rebindable) | T | Every product: MangoHud RShift+F12, AMD Ctrl+Shift+O, NVIDIA Alt+R, Special K Ctrl+Shift+O, Afterburner toggle |
| 2 | Choice of metrics per block + text vs graph | T | Afterburner per-sensor Text/Graph; PresentMon widgets; MangoHud `graphs=` |
| 3 | Position (corners + offset) and background opacity | T | Steam (position/contrast/saturation/opacity), AMD (position/transparency/size), FrameView (draggable) |
| 4 | Font size / overall scale | T | NVIDIA, FrameView 1.7, AMD "size", RTSS zoom |
| 5 | FPS + frametime numbers; frametime graph | T | RTSS, PresentMon, MangoHud, CapFrameX; "graph shows spikes that the FPS number hides" |
| 6 | Presets / quick levels (FPS only -> full) and hotkey to cycle | T (mainstream) / D (user-editable cycle) | Steam 4 levels, MangoHud 5 presets + `toggle_preset`, PresentMon 3 loadouts, CapFrameX 3 configs, FPS Monitor scenes, HotkeyHandler |
| 7 | Threshold colouring (green/amber/red on temp, load, FPS) | T for temps/FPS, D for editor UI with user thresholds | MangoHud value/color pairs, CapFrameX thresholds, FPS Monitor limits/alerts, RTSS dynamic `<C=formula>` |
| 8 | Min/Avg/Max and 1% / 0.1% lows (live, resettable) | T for enthusiasts | Steam min/max/avg, FrameView 1%, FPS Monitor, RTSS `<FR01L>`; RTSS needs a benchmark hotkey (a noted irritation), PresentMon percentiles per widget |
| 9 | Compact horizontal bar mode / multi-column | D | MangoHud `horizontal`, NVIDIA 1-line/2-line, AMD columns |
| 10 | Per-game profile auto-switch | D (RTSS cannot per-process in OverlayEditor; FPS Monitor does) | RTSS limitation quoted in §1.1 |
| 11 | Benchmark/logging capture (hotkey start/stop, CSV, summary) | D (T for reviewers) | MangoHud logging, PresentMon capture hotkey (10 s default), Afterburner benchmark, CapFrameX run history/aggregation |
| 12 | Import/export/share profiles as a file | D | RTSS `.ovl/.ovx` skin ecosystem, CapFrameX JSON + downloadable templates, Merge in RTSS 7.3.7 |
| 13 | Scaling with resolution/DPI (and per-monitor) | T-in-practice, but often broken | 4K readability complaints (§5); integer-only RTSS zoom |
| 14 | Hide from capture/stream (OBS) per option | D | Only achievable cleanly with a window overlay + WDA_EXCLUDEFROMCAPTURE |
| 15 | Frame-gen aware FPS (real vs generated) | D (rising to T) | Steam splits it; PresentMon FPS-Presents/Display/App |
| 16 | CPU/GPU-bound indicator (GPU Busy) | D | PresentMon GPU Busy; RTSS conditional-layer sample |
| 17 | Frame limiter | N for OMA (and a large risk surface: driver/hook level) | MangoHud, RTSS, Special K, FPS Monitor "FPS Lock"; arguably out of scope for a no-admin monitor |
| 18 | Run history comparison in OSD | D (niche) | CapFrameX |
| 19 | Multiple monitors (choose which display) | N->T for multi-monitor users | PresentMon window mode; FrameView draggable |
| 20 | Live config reload / live preview while editing | D | MangoHud `reload_cfg`; FPS Monitor "configure before entering the game" (a WYSIWYG gap for RTSS) |
| 21 | Alerts (temp/limit) with colour or flashing | N/D | FPS Monitor alerts |

Design guidance that recurs: show *fewer* metrics by default; large or busy overlays "make performance harder to interpret" (wccftech guide); separate benchmark layout vs casual layout. [V]

---------------------------------------------------------------------------
## 4. Graph types and how they are configured

| Type | Where seen | Config knobs |
|---|---|---|
| **Line (time series)** | RTSS embedded graph, Afterburner graph, PresentMon multi-line timeline, MangoHud `graphs=`, Steam FPS graph, Game Bar | width, height, time window/sample count (PresentMon "time scale", RTSS graph width = samples), min/max (fixed or autoscale; RTSS "graph max"), line colour, background colour/alpha, optional threshold line, margin |
| **Filled area** | RTSS graph style, Afterburner (graph fill), Game Bar CPU/GPU | same as line + fill colour/alpha |
| **Bar / bar-range** | RTSS ("bar range graphs", renders under 1 px width in 7.3.7) | bar width, range, colour per sample (dynamic colour formula) |
| **Frametime plot** | RTSS (classic frametime graph, scale max), MangoHud `frame_timing`, `dynamic_frame_timing`, `frame_timing_detailed`, Special K frame-pacing widget, PresentMon frametime + GPU busy overlay | ms scale (fixed or dynamic), reference lines (16.7/33.3 ms), colour; stacked comparison frametime vs GPU time |
| **Histogram** | PresentMon 2.0, MangoHud `histogram` | bin count, range, statistic markers (avg/p99) |
| **Mini gauge / ring / bar meter** | RTSS `<AI>` sprite gauges & `<B>` bars (rounded), NVIDIA/AMD none; Steam none | min/max, colour thresholds |
| **Sparkline in-line with a number** | Afterburner "text + graph", MangoHud per-metric graph, Game Bar | small fixed width next to the value |
| **Stat readout (not a graph)** | PresentMon per-widget statistic: avg, 1%, 5%, 10%, 90%, 95%, 99%, min, max | rolling window length |

Common configuration vocabulary to adopt [I]: *time range* (seconds or samples), *y-range* (auto / fixed min-max / "nice" autoscale with decay), *colour mode* (solid / by-threshold / gradient), *line width*, *fill alpha*, *background alpha*, *show current value overlay*, *reference lines*. RTSS's dynamic colour "recomputed independently for each sample" is a good model for threshold-coloured graphs.

---------------------------------------------------------------------------
## 5. Pitfalls users complain about

1. **Overlay causes stutter / FPS loss.** Causes: hook overhead (worst in DX12/Vulkan, anti-cheat games), aggressive polling, too many sensors, certain sensors being costly (e.g. polling GPU Power % via the NVIDIA driver causing frametime spikes), conflicts among overlays. One Anandtech thread: Afterburner OSD cost 9-16% FPS in some games. Mitigations users apply: fewer sensors, longer monitoring refresh period, lowering RTSS detection level, disabling other overlays. https://usekudu.com/guides/gaming/fix-msi-afterburner-osd-causing-fps-drop [V], https://forums.anandtech.com/threads/in-some-games-msi-afterburner-overlay-can-negatively-impact-fps-by-9-16.2457508/post-37899595 [S]. PresentMon even ships "disable overlay alpha blending" for perf. [V]
   -> For OMA: poll cost belongs to the existing scheduler; the overlay should use a separate, low refresh rate (user-selectable, 1-2 Hz for text, higher only for graphs) and render only on change.
2. **Anti-cheat.** EasyAntiCheat has blocked RTSS builds (whitelist lag), Watch Dogs 2 blocked it, bans "rare" but never guaranteed; RTSS "Stealth mode" exists. https://www.guru3d.com/story/update-watch-dogs-2-anti-cheat-system-blocks-rtss-overlay-software/ [S], https://forums.ea.com/discussions/apex-legends-technical-issues-en/about-stealth-mode-in-rtss/5692592 [S]. Window-based overlays (FrameView, PresentMon) are explicitly positioned as the anti-cheat-friendly path. [S] Advice for OMA: document "no injection" prominently; do not offer injection; never read game memory.
3. **Overlay not visible in some modes/APIs.** Exclusive fullscreen (needs Z-band/uiAccess or borderless), OpenGL/Vulkan legacy fullscreen, no MPO support, HDR/frame-generation oddities, UWP/Store games (RTSS `<ARCH>` UWP). [V from Microsoft + FrameView notes]. RTSS 7.3.7 added an alternate DX12 queue detection for NVIDIA Smooth Motion; Vulkan late device-pointer init. [V]
4. **Conflicts between overlays** (Discord, Steam, GeForce/NVIDIA app, AMD, Game Bar, Afterburner): double hooks, frametime spikes, overlapping positions (RTSS multi-client stacking needs manual `<P>`). [V/S]
5. **Unreadable at 4K / high DPI / odd scaling.** RTSS integer-only zoom; DSR scaling bug in RTSS; Overwolf notes DPI > 100 % misrenders overlays. https://support.overwolf.com/support/solutions/articles/9000176964-how-to-scale-down-your-dpi [S]. Expect users to want: continuous scale (e.g. 50-300 %), per-monitor DPI awareness, outline/shadow/background for legibility over bright scenes (RTSS fill, FrameView black background, Steam contrast/saturation).
6. **Editor complexity.** RTSS hypertext requires SDK knowledge, no docs, order-of-render surprises, forum threads full of "how do I make it look like X" -> community skins as workaround. [V]. FPS Monitor and PresentMon win on approachability with GUI editors and presets. Lesson: ship good built-in layouts and make the editor optional.
7. **Benchmarking stats that don't show without a ritual** (RTSS: set a hotkey, start/stop manually; Min/Avg/Max not live). [V]
8. **Data latency / staleness.** PresentMon ETW latency 1000 ms -> 30 ms fix shows users notice. [V]
9. **Sensor source conflicts**: two programs reading the same sensors (HWiNFO + Afterburner + OMA) can duplicate or fight over drivers (e.g. PawnIO/WinRing0-style); also HWiNFO shared-memory limits on the free edition. [S/I]
10. **Overlay in captures** is wanted off by streamers (OBS) while some users *want* it in recordings; injected overlays force the latter, window overlays can choose. [I]
11. **Mixed Steam reviews of FPS Monitor** show commercial overlays also regress on updates -> keep the overlay renderer simple and well tested. [V]

---------------------------------------------------------------------------
## 6. Suggested scope for OpenMonitor Advanced (prioritized)

Ground rules derived from project constraints: no admin, no injection, GPL, WebView2/Tauri, synthwave palette, perf budget (idle < 1 % CPU).

### MVP
1. Overlay as a separate frameless, transparent, always-on-top, click-through Tauri window (borderless/windowed + Fullscreen-Optimizations games). Clear in-UI note about exclusive fullscreen limits.
2. Layout editor: grid canvas (configurable cell size, snap on/off), add/delete/duplicate/move/resize blocks, multi-select, z-order (front/back), undo/redo, live preview of the real overlay.
3. Blocks bound to existing sensor ids (`<device_id>/<kind>/<name>`): GPU load, VRAM, temps, CPU, RAM, network, disks; FPS and frametime need a frame source (decision point — see "Open questions").
4. Per-block properties panel: display mode **Text / Graph / Text+Graph**; label (i18n key or custom), unit, decimals; font family (limited list), size, weight; text colour, label colour, background colour/alpha, border, padding, text outline/shadow; alignment.
5. Graph types in MVP: **line** and **filled area**, plus **bar meter** (horizontal progress bar). Config: time range (10/30/60 s), y-range auto/fixed, colour, fill alpha.
6. Threshold colouring: up to 2-3 user thresholds per block (colour by value), with temp/load defaults pre-filled.
7. Layout profiles: save/load/rename/delete as JSON files; 3-4 built-in templates (FPS only, Minimal, Standard, Full) — mirroring Steam's levels / MangoHud presets.
8. Global overlay settings: position/offset or drag mode, global scale (continuous), global opacity, monitor selector, refresh rate (1/2/5 Hz text; graphs smoothed), per-DPI scaling.
9. Global hotkeys: show/hide overlay, cycle profile; rebindable; conflict check.
10. "Hide from capture" toggle (WDA_EXCLUDEFROMCAPTURE), default on or off per decision.
11. Import/export of a profile as a single `.json`/`.omaoverlay` file with version field.

### Next
- Auto-switch profile per executable (process watcher by exe name; fallback profile) — a clear win over RTSS OverlayEditor.
- Min/avg/max and 1%/0.1% lows with live reset hotkey; rolling window selection (as PresentMon percentiles per widget).
- Frametime plot block with fixed/dynamic ms scale, 16.7/33.3 ms reference lines; histogram block.
- Compact horizontal bar mode and multi-column auto-layout; group-name style lines (Afterburner/CapFrameX concept) as "row groups".
- Benchmark/capture: hotkey start/stop, CSV (reuse the existing M5c CSV log), end-of-run summary in overlay and in app.
- Mini gauge block; sparkline next to text; conditional visibility (show block only above threshold, e.g. temperature warning).
- Template gallery in the app, drag-drop import, copy formatting between blocks ("apply to multiple", as CapFrameX).
- Frame-generation-aware FPS and GPU-busy/bound indicator if the frame source provides it.
- Opt-in elevated-trust display path: signed uiAccess + Z-band (like PresentMon) for exclusive fullscreen, only after SignPath signing exists and installed under Program Files.

### Later
- Expressions/formulas in blocks (`x/1024`, sums, sliding-window max) — RTSS-style data-source corrections, but via a safe expression evaluator.
- Run history in OSD and aggregation of runs.
- Layout "merge", versioned community profile gallery, online sharing.
- Alerts (flash/sound/notification) tied to the existing M5b rules engine.
- Conditional layers, animations, image/gauge sprites, custom fonts.
- Per-monitor multiple overlays; VR/desktop widget mode; frame limiter (probably never: needs injection/driver level).

### Open questions the spec must settle (from this research)
1. **FPS/frametime source**: the repo's current providers cover PDH/D3DKMT/DXGI/NVML etc.; none gives per-process present timing. Options: ETW (the PresentMon route: `Microsoft-Windows-DxgKrnl`/`DXGI` present events; needs admin or the "Performance Log Users" group/ a service), or consume PresentMon service. The existing `oma-service` could host ETW. Without this, "FPS/frametime" blocks (table stakes) are impossible. [I]
2. Exclusive fullscreen: ship borderless-only at first, or sign + uiAccess later?
3. Overlay rendering tech: WebView2 window (reuse Svelte renderer, higher RAM/GPU than native) vs a native Win32/Direct2D renderer for the overlay only (lower cost, but forks the rendering stack). Our budget (< 200 MB incl. WebView2) argues for measuring early.
4. Capture visibility default.
5. Profile format and versioning (JSON with schema version; share-safe, no paths).

---------------------------------------------------------------------------
## Sources (consolidated)
- RTSS/OverlayEditor: https://forums.guru3d.com/threads/rtss-overlay-editor-megathread.436443/ ; https://forums.guru3d.com/threads/list-of-all-hypertext-tags.437486/ ; https://forums.guru3d.com/threads/afterburner-rtss-osd-custom-layout-tag-syntax-documentation.427545/ ; https://forums.guru3d.com/threads/download-rtss-7-3-5-beta-5.449364/ ; https://www.guru3d.com/files-details/rtss-rivatuner-statistics-server-download.html ; https://www.cloudspress.com/rtss-overlay-editor-megathread-setup-layouts-formulas-and-troubleshooting/ ; https://mybyways.com/blog/cpu-and-gpu-monitoring-overlay-for-gaming ; https://videocardz.com/newz/msi-afterburner-4-6-3-beta-and-rtss-7-3-0-beta-6-bring-overlay-editor-plugin
- Afterburner: https://www.msi.com/blog/msi-afterburner-on-screen-display ; https://www.gamingpcbuilder.com/msi-afterburner-overlay-customization-guide ; https://usekudu.com/guides/gaming/fix-msi-afterburner-osd-causing-fps-drop
- CapFrameX: https://capframex.com/blog/post/How to configure the CapFrameX game overlay ; https://wccftech.com/how-to-set-up-high-quality-performance-overlays-with-rtss/amp/
- HWiNFO: https://www.hwinfo.com/forum/threads/support-of-rtss-displaying-hwinfo-sensor-values-in-games.45/post-821
- PresentMon: https://github.com/GameTechDev/PresentMon ; https://github.com/GameTechDev/PresentMon/releases ; https://docsearch.algolia.com/mcp/docs/repo/gametechdev/presentmon ; https://game.intel.com/us/intel-presentmon/ ; https://www.neoteo.com/en/how-to-analyze-your-graphics-card-performance-with-intel-presentmon ; https://www.pchardwarepro.com/en/How-to-optimize-performance-with-Presentmon-step-by-step/ ; https://www.geeks3d.com/20230819/intel-presentmon-new-gaming-performance-overlay-and-telemetry-application/ ; https://www.xda-developers.com/5-reasons-intel-presentmon-is-the-best-tool-for-checking-exactly-what-your-gaming-pcs-bottleneck-is/
- NVIDIA: https://www.guru3d.com/story/nvidia-statistics-overlay-adds-cpu-temperature-power-and-clock-monitoring/ ; https://www.nvidia.com/en-ph/geforce/technologies/frameview/release-notes/ ; https://www.nvidia.com/en-gb/geforce/technologies/frameview/release-notes
- AMD: https://amd.com/en/resources/support-articles/faqs/DH3-038.html
- Steam: https://www.techspot.com/community/topics/steam-beta-adds-new-in-game-performance-metrics-overlay-can-show-dlss-frames.292994/ ; https://www.3dtested.com/video-games/pc-gaming/steam-just-got-a-new-performance-overlay-you-can-now-view-real-fps-alongside-dlss-frs-generated-frames
- MangoHud: https://raw.githubusercontent.com/flightlessmango/MangoHud/master/data/MangoHud.conf ; https://www.gamingonlinux.com/2023/09/mangohud-v07-out-now-adding-presets-support-like-on-steam-deck/
- Special K: https://discourse.special-k.info/t/is-it-possible-to-change-the-style-of-the-performance-overlay/3038 ; https://wiki.special-k.info/SwapChain
- FPS Monitor: https://store.steampowered.com/app/966610/FPS_Monitor/
- Game Bar: https://www.xda-developers.com/xbox-game-bar-resources-widget-track-cpu-gpu-ram-usage/
- Windows platform: https://devblogs.microsoft.com/directx/demystifying-full-screen-optimizations/ ; https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity ; https://www.meziantou.net/how-to-exclude-your-windows-app-from-screen-capture-and-recall.htm ; https://dev.overwolf.com/ow-electron/guides/product-guidelines/app-screen-behavior/in-game-overlays/ ; https://github.com/tauri-apps/tauri/issues/9250
- Anti-cheat/stutter: https://www.guru3d.com/story/update-watch-dogs-2-anti-cheat-system-blocks-rtss-overlay-software/ ; https://forums.ea.com/discussions/apex-legends-technical-issues-en/about-stealth-mode-in-rtss/5692592 ; https://forums.anandtech.com/threads/in-some-games-msi-afterburner-overlay-can-negatively-impact-fps-by-9-16.2457508/post-37899595
