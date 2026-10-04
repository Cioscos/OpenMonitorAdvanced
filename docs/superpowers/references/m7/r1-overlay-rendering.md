# In-game overlay (OSD) for OpenMonitor Advanced: research on rendering approaches

Research date: 2026-10-04. Scope: how to draw live sensor values and FPS/frametime over games
(D3D9/10/11/12, OpenGL, Vulkan) on Windows 10/11, for a legitimate, open-source performance
monitor (same category as MSI Afterburner/RTSS, the Steam and Discord overlays, Intel PresentMon).
Constraints: GPL-3.0-or-later, UI app without admin rights, LocalSystem .NET service, currently
unsigned releases (SignPath Foundation planned), perf budget (core < 1 % CPU idle, window < 200 MB).

Confidence tags: **[fact]** verified in a cited source; **[inference]** reasoning from cited facts;
**[verify]** must be checked on real hardware before the spec commits.

---

## 0. Executive summary

Five families of approach. They differ less in "can it draw text" than in three hard constraints:
**(a)** does it work in exclusive fullscreen, **(b)** does it run code inside the game process
(anti-cheat compatibility, antivirus false positives, crash liability), **(c)** what it costs the
game's presentation path (latency / VRR).

| # | Approach | Code in game process? | Works over exclusive FS | Main cost |
|---|----------|----------------------|-------------------------|-----------|
| A | Own injection + Present/SwapBuffers hooks | yes | yes | AC incompatibility, AV flags, large compat surface, two DLLs (x86/x64) |
| B | Own Vulkan implicit layer | yes (Vulkan only) | yes | same AC issues, Vulkan-only |
| C | RTSS as render backend (shared memory) | no (RTSS injects) | yes | user must install RTSS; RTSS itself blocked by some ACs |
| D | Hook-free topmost click-through window | no | borderless/FSO yes; true FSE only with uiAccess (signed) | forces DWM composition (loses independent flip/VRR unless MPO); ~1 frame latency |
| E | Xbox Game Bar widget (UWP) | no | Game Bar's band | UWP/MSIX + Store publication; separate C#/C++ XAML stack |

FPS/frametime does **not** require injection: the PresentMon/ETW technique measures presents of
every graphics API from outside the game, and OMA's LocalSystem service can consume it.

**Recommendation (see §11):** Phase 1 = hook-free native overlay window (D) + FPS/frametime via ETW
in `oma-service` + optional RTSS backend (C) for users who already run RTSS (that path covers true
exclusive fullscreen). Phase 2 (after SignPath signing) = a `uiAccess` overlay process installed in
Program Files (Intel PresentMon's technique) to sit above exclusive-fullscreen games without
injecting. Own injection (A/B) is not recommended for 1.x; if ever added, make it opt-in and
single-player-only, built on hudhook + a hand-written Vulkan layer.

---

## 1. Approach A — own injection + graphics API hooks

### 1.1 Hook points per API

| API | Hook points | Notes |
|-----|-------------|-------|
| D3D9 / D3D9Ex | `IDirect3DDevice9::Present`, swapchain `Present`, `PresentEx`, `Reset`/`ResetEx` | Device-lost/reset must release & recreate resources. `EndScene` (classic tutorial hook) can fire several times/frame — avoid. |
| D3D10/11 (DXGI) | `IDXGISwapChain::Present`, `Present1`, `ResizeBuffers`, `ResizeBuffers1` | Drop the RTV before `ResizeBuffers`, recreate after. |
| D3D12 (DXGI) | DXGI hooks **plus** capturing the `ID3D12CommandQueue` the swapchain presents on | DXGI exposes no "get queue from swapchain" API. hudhook 0.9.x hooks `CreateSwapChain`, `CreateSwapChainForHwnd`, `ExecuteCommandLists`, `Present`, `Present1`, `ResizeBuffers`, `ResizeBuffers1` to recover the queue [fact, hudhook `src/hooks/dx12.rs`]. Needs own descriptor heaps + per-backbuffer fences. |
| OpenGL | `wglSwapBuffers` (opengl32) / `SwapBuffers` (gdi32) | Save/restore GL state or use a shared context. RTSS 7.3.7 switched its GL fallback from in-context to separate-context rendering [fact, Guru3D RTSS 7.3.7 notes]. |
| Vulkan | `vkQueuePresentKHR` + `vkCreateSwapchainKHR`/`vkDestroySwapchainKHR` + `vkCreateDevice` | Inline-hooking `vulkan-1.dll` is fragile; the supported path is a layer (§2). DXVK / vkd3d-proton titles present via Vulkan on Windows. |

### 1.2 Getting the DLL into the process (the standard mechanisms, and their gates)

- **`SetWindowsHookEx` global hook.** A 32-bit DLL loads only into 32-bit processes and a 64-bit DLL
  only into 64-bit; the two DLLs must have different names; you need a hook-owner process of each
  bitness that keeps pumping messages [fact, MS Learn WOW64 Implementation Details + SetWindowsHookExW
  remarks]. This is exactly how RTSS works: a global hook injects `RTSSHooks64.dll`/`RTSSHooks.dll`
  into every process; "detection level = NONE" does not stop the DLL loading, only stops it installing
  the 3D-API hooks there [fact, Unwinder, Guru3D]. Not admin-only, but see §6/§9.
- **`CreateRemoteThread` + `LoadLibrary`.** `OpenProcess` → `VirtualAllocEx` → write the DLL path →
  `CreateRemoteThread(LoadLibraryW)`. This is hudhook's injector [fact, hudhook `src/inject.rs`:
  `VirtualAllocEx`, `CreateRemoteThread`, `OpenProcess(PROCESS_ALL_ACCESS)`]. Needs a handle with
  `PROCESS_ALL_ACCESS`; cross-bitness is impossible; getting a handle to a higher-integrity or
  protected game fails without matching privilege.
- **AppInit_DLLs.** Dead on modern systems: disabled whenever Secure Boot is enabled (Windows 8+),
  and loading arbitrary DLLs this way is a desktop-app certification failure [fact, MS Learn
  "AppInit DLLs and Secure Boot"]. Do not consider.
- **DXGI/opengl/dxgi proxy DLL** (ship `dxgi.dll`/`opengl32.dll` next to the game that forwards to the
  real system DLL and hooks Present). MangoHud-Windows uses a `dxgi.dll` proxy for D3D11/12 [fact,
  MangoHud-Windows README]. Per-game, user-copied, no system-wide injection — lowest AC footprint of
  the injection family, but manual.

### 1.3 Integrity / privileges

The overlay DLL runs at the game's integrity level. OMA's UI runs non-elevated; injecting into a
normal game is fine, but any game running elevated or as a protected process needs equal/higher
privilege [inference from MS Learn integrity-level docs]. RTSS sidesteps this by running elevated
(`requireAdministrator`) [fact, Guru3D forum]. OMA wants to stay non-admin, which argues against
being an injector at all.

### 1.4 Resize / fullscreen / HDR / multiple swapchains

- **Resize/fullscreen transitions:** must hook `ResizeBuffers`/`Reset` and rebuild all per-backbuffer
  resources; hudhook had repeated DX12 resize bugs fixed through 0.9.2 ("stabilize swap-chain resize
  handling") and DX11 fullscreen fixes in 0.9.1 [fact, hudhook releases]. This is the single most
  bug-prone area.
- **HDR swapchains:** back buffer may be `R10G10B10A2_UNORM` or `R16G16B16A16_FLOAT` (scRGB). An
  overlay that assumes 8-bit sRGB (e.g. hudhook forces `R8G8B8A8_UNORM` in places [fact, hudhook
  `src/hooks/dx11.rs`]) renders with wrong brightness on HDR. Must read the actual swapchain format
  and color space and author colors accordingly [verify].
- **Multiple swapchains / windows:** editors, multi-monitor, VR companion windows present several
  swapchains; the hook must pick the game's main one (largest/foreground) and ignore the others.

### 1.5 Coexistence with other overlays (hook chaining)

Multiple overlays all hook `Present`; order is whoever hooked last wins the outer frame. Known
conflicts: hudhook's own issue #196 "RivaTuner overlay conflict" — with RTSS present, a DX9 game's
`Present` hook fired only once and ImGui never showed; author's workaround was "tell users not to run
other overlay software," since RTSS "hijacks Present" [fact, hudhook issue #196]. RTSS, Steam,
Discord (old), ReShade, Special K, OBS game-capture all live in this same contested space. A robust
injector must chain correctly (call the original) and tolerate being wrapped by others — a large,
open-ended QA burden.

---

## 2. Approach B — own Vulkan implicit layer

The sanctioned way to draw in Vulkan is an **implicit layer**, the mechanism Steam, OBS and MangoHud
use.

- **Registration (Windows):** add a `REG_DWORD = 0` value whose *name* is the absolute path of the
  layer's JSON manifest, under:
  - `HKLM\SOFTWARE\Khronos\Vulkan\ImplicitLayers` and `HKCU\...` (64-bit),
  - `HKLM\SOFTWARE\WOW6432Node\Khronos\Vulkan\ImplicitLayers` and `HKCU\...` (32-bit),
  - optionally the driver adapter keys `...\Control\Class\{GUID}\000X\VulkanImplicitLayers`.
  Setting the value to non-zero disables it [fact, Khronos LoaderLayerInterface].
- **Elevation rule:** the loader ignores **HKCU** implicit layers when the app runs with admin
  privileges (anti-escalation) [fact, LoaderLayerInterface]. So a per-user (non-admin) install under
  HKCU works for normal games but is skipped for elevated games — for those you'd need an HKLM entry,
  which needs admin to write.
- **Manifest:** `vkNegotiateLoaderLayerInterfaceVersion` entry (interface v2), and a **required**
  `disable_environment` var so users/tools can turn it off. OBS's manifest is the canonical shape
  [fact]:
  ```json
  { "file_format_version":"1.1.2","layer":{"name":"VK_LAYER_OBS_HOOK","type":"GLOBAL",
    "library_path":".\\graphics-hook64.dll","api_version":"1.3.216","implementation_version":"1",
    "description":"Open Broadcaster Software hook",
    "functions":{"vkNegotiateLoaderLayerInterfaceVersion":"OBS_Negotiate"},
    "disable_environment":{"DISABLE_VULKAN_OBS_CAPTURE":"1"}}}
  ```
- **32- vs 64-bit:** need separate 32-bit and 64-bit layer DLLs; MangoHud/MangoHud-Windows give the
  two layers **distinct names** to avoid the loader skipping the 64-bit one after seeing a wrong-bitness
  32-bit layer with the same name [fact, Arch bug 67947 discussion]. 
- **Enable/disable env vars:** `VK_INSTANCE_LAYERS`, `VK_LOADER_LAYERS_DISABLE` (globs, `~implicit~`
  disables all implicit layers), plus each layer's own `enable_environment`/`disable_environment`
  [fact].

This is cleaner than inline Vulkan hooking but still runs our code in-process → same anti-cheat
exposure as A, and only covers Vulkan/DXVK titles.

---

## 3. Approach C — RTSS (RivaTuner Statistics Server) as rendering backend via shared memory

RTSS already solves injection, all APIs (D3D9–12, OpenGL, Vulkan, 32/64-bit, down to XP, exclusive
fullscreen) and coexistence — many tools (HWiNFO, AIDA64, CapFrameX, HandheldCompanion, steam-deck-tools)
just push text/graphs into it [fact, AIDA64/HWiNFO docs; GitHub].

### 3.1 The interface (documented enough, widely cloned)

- RTSS exposes a memory-mapped file **`RTSSSharedMemoryV2`**. Layout is in `RTSSSharedMemory.h`,
  shipped in RTSS's `SDK` folder and mirrored in dozens of GitHub repos [fact, GitHub code search].
  Structure: header with `dwSignature`('RTSS'/0xDEAD), `dwVersion`, offsets/sizes for the OSD-entry and
  app-entry arrays, `dwOSDFrame`, and a `dwBusy` interlock bit you set while writing [fact, header v2.x
  in CapFrameX/obs-rtss]. There are **8 OSD slots** (`arrOSD[8]`), each with `szOSD[256]`,
  `szOSDOwner[256]`, `szOSDEx[4096]`, a 256 KB `buffer`, and `szOSDEx2[32768]` [fact, header].
- A client claims a free slot by writing its owner id into `szOSDOwner`, writes formatted text into
  `szOSDEx`, guarded by `dwBusy` interlock [fact, Unwinder's `RTSSSharedMemoryInterface.cpp` in
  obs-rtss, MIT].
- **Text formatting tags** (hypertext parser): `<C=RRGGBB>`/`<C0..>` color, `<S=..>`/`<S0..>` size,
  `<A=..>`/`<A0..>` alignment columns, `<P=x,y>` absolute position, `<L0..>` layers, `<FR>` framerate,
  `<FT>` frametime, `<APP>` app name, `<FNT>` font (new in 7.3.7), and `<OBJ=offset>` to place an
  **embedded object** [fact, Unwinder's `RTSSSharedMemorySampleDlg.cpp`; Guru3D 7.3.7 notes].
- **Graphs:** embedded object `RTSS_EMBEDDED_OBJECT_GRAPH` with flags
  `..._FILLED | ..._BAR | ..._BGND | ..._VERTICAL | ..._AUTOSCALE | ..._FRAMERATE/..._FRAMETIME | ..._BAR_RANGE`,
  a min/max and a float sample array; placed in the OSD `buffer` and referenced with `<OBJ=...>`
  [fact, header + `EmbedGraph(...)` in CapFrameX/obs-rtss/tickmeter]. So RTSS can draw line/filled/bar
  graphs for us — we feed samples, RTSS rasterizes.
- Signature so RTSS v3-vector-mode ignores unsupported tags; "format tags fully supported for raster
  3D OSD only" [fact, search result].

### 3.2 Access / privileges

RTSS runs elevated and creates the mapping; a non-admin client opening `RTSSSharedMemoryV2` can hit
`ERROR_ACCESS_DENIED` depending on the ACL/elevation of the creator [inference from the general
`OpenFileMapping` elevation rule — [verify] whether RTSS sets a permissive DACL; HWiNFO/AIDA64 as
non-admin read it, which suggests it does]. Many .NET clients map it read/write successfully without
elevation.

### 3.3 Licensing (the blocker for bundling)

- RTSS is **freeware, not open source**, distributed only via Guru3D ("do not download anywhere else");
  bundled with MSI Afterburner [fact, Guru3D]. No redistribution grant found; the EULA text was not
  locatable online [verify]. **Assume we may NOT ship RTSS** — the user installs it.
- The **SDK sample** (`RTSSSharedMemorySample`, by Unwinder) is described as "open source" but with no
  SPDX license in the folder; third-party ports carry their own licenses (RTSSSharedMemoryNET is
  LGPL-3.0; obs-rtss re-implementation is MIT) [fact, GitHub]. For a GPL-3 app, **do not copy
  Unwinder's sample**; instead re-implement the struct layout from the header (layout facts are not
  copyrightable) and use an MIT/LGPL client, or write our own in Rust/C#. GPL-3 projects already do
  this: HandheldCompanion (GPL-ish), epinter/rainmeter-lhws (MPL-2.0) read `RTSSSharedMemoryV2`
  directly [fact].
- Interop is at arm's length (a memory-mapped file we read/write), so using RTSS at runtime does not
  make OMA a derivative of RTSS. Shipping our own reader under GPL-3 is fine.

### 3.4 Verdict

Best "works everywhere incl. exclusive fullscreen, zero injection code from us" option — but gated on
the user already running RTSS, and RTSS is itself blocked by strict anti-cheats (§6). Excellent as an
*optional* backend; cannot be the only one.

---

## 4. Approach D — hook-free topmost, click-through, layered/DirectComposition window

A normal top-level window (`WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE`),
per-pixel alpha (UpdateLayeredWindow or DirectComposition + a flip swapchain), click-through, sized to
the game's client rect. **No injection, no game memory, no synthetic input** — the OS compositor draws
it above the game.

### 4.1 Fullscreen behaviour

- **Borderless / windowed / Windows-11 "Optimizations for windowed games" (FSO):** works. FSO runs
  "fullscreen exclusive games instead in a highly optimized borderless windowed format"; when an
  overlay like Game Bar appears "the DWM reassumes control of the display" and composites it safely
  [fact, DirectX devblog "Demystifying Fullscreen Optimizations"]. This is how the **new Discord
  overlay (Mar 2025)** works: "a permanent `HWND_TOPMOST` window," "no longer uses DLL injection,"
  works in windowed & borderless, **not** in true fullscreen [fact, Discord blog; erikmcclure.com].
- **True exclusive fullscreen (FSE):** a plain `ZBID_DESKTOP` topmost window cannot sit above it, and
  touching a desktop window while a fullscreen app runs can minimize it [fact, adeltax z-order blog;
  alia5 dev.to]. You need a higher **Z-band** (see §4.3).

### 4.2 Latency / VRR cost (the real price of D)

- The DWM uses **independent flip** (≈FSE efficiency) when the game's flip-model swapchain covers the
  screen and nothing is composited on top. If even one pixel of another window overlaps, the DWM
  either reverts to composed mode, "reverse-composes," or **uses MPO (multiplane overlay) to keep
  independent flip** [fact, DXGI flip-model devblog].
- In practice a topmost overlay tends to force **Composed Flip**, which is what ForceComposedFlip
  exploits deliberately: a 1×1 click-through topmost window makes DWM composite every frame [fact,
  ForceComposedFlip]. Consequence: **G-Sync/FreeSync (VRR) can break and ~1 frame of latency is added**
  whenever the overlay is visible [fact, erikmcclure.com on Discord's overlay]. On hardware with free
  MPO planes the hit can be avoided ("Hardware Composed: Independent Flip") but MPO availability is
  driver/config dependent and overlay planes are scarce [fact, PresentMon/blurbusters]. → **Design
  rule:** draw nothing (truly empty/hidden window) when the OSD is toggled off, so VRR is intact when
  the user isn't looking [inference]. [verify] the exact VRR/latency behaviour on the RTX 4080 + AMD
  iGPU test rig with MPO on/off.

### 4.3 Getting above exclusive fullscreen without injecting: `uiAccess` + Z-bands

- Windows z-order has **bands** (`ZBID_DESKTOP` … `ZBID_UIACCESS`); higher bands always sit above lower
  ones. Normal apps are stuck in `ZBID_DESKTOP`; the undocumented `CreateWindowInBand` lets you create
  in a higher band but checks privilege [fact, adeltax blog; Intel PresentMon README].
- A process with **`uiAccess="true"`** in its manifest is granted `ZBID_UIACCESS` (top band) — and
  Intel's PresentMon capture app reports it "can remain on top (even above fullscreen exclusive games)
  when uiAccess is set to true, even when `CreateWindowInBand` is not used" [fact, PresentMon
  README-CaptureApplication]. This is the clean, injection-free way to overlay FSE games.
- **But `uiAccess` has hard requirements** [fact, MS Learn "Security Considerations for Assistive
  Technologies" + UAC settings]:
  1. the exe must be **Authenticode-signed**, signature checked regardless of policy;
  2. it must run from a **secure location** (`%ProgramFiles%`, `%ProgramFiles(x86)%`, or
     `%SystemRoot%\system32`);
  3. manifest `requestedExecutionLevel ... uiAccess="true"`.
  UWP cannot use uiAccess. A non-admin process can spawn it but there are known issues spawning a
  uiAccess process from a non-admin one, and with PostMessage across the boundary [fact, PresentMon
  README]. Xbox Game Bar itself is the Microsoft-blessed example of a window above FSE games.
- **Implication for OMA:** this requires the SignPath code-signing cert (planned) and installing the
  overlay exe under Program Files (our NSIS installer already puts the app there). Until signed, D is
  limited to borderless/FSO games (which, with FSO default-on in Win11, is the large majority).

### 4.4 Precedents

- Discord new overlay (topmost window, no injection) [fact].
- Intel PresentMon capture app (uiAccess + CreateWindowInBand, D3D11 renderer) [fact].
- Game Bar (immersive band) [fact].

### 4.5 Drawing tech for D

We own the window, so we render with our own device: Direct2D/DirectWrite (cheap text, system fonts),
or a tiny D3D11 + imgui, or even a WebView2 for a fancy HUD (but WebView2 is heavy for an always-on
overlay — stay native). DirectComposition gives a flip swapchain with per-pixel alpha without
UpdateLayeredWindow's CPU blit. [inference]

---

## 5. Approach E — Xbox Game Bar widget (UWP)

- A Game Bar widget is a **UWP XAML app** with a view rendered into Game Bar's overlay; it can be a
  normal app and a widget simultaneously, and communicates with a separate (full-trust/win32) process
  for driver-level access [fact, MS Learn Game Bar overview]. SDK = NuGet `Microsoft.Gaming.XboxGameBar`;
  latest 7.3.2607010 (2026-07-01), actively updated [fact, nuget.org].
- **Pinned** widgets stay on screen over games even when Game Bar is dismissed, with user-controllable
  transparency and click-through [fact, MS Learn + elevenforum]. Game Bar's band handles the FSE
  problem for us.
- **Distribution is the catch:** "In all cases you will need to upload and publish your Game Bar widget
  app to the Microsoft Store." Side-loading the MSIX alongside the win32 app is allowed on Win10 build
  ≥18956 [fact, MS Learn distribution]. So we'd need MSIX packaging, a Store listing, and a second UI
  stack (XAML) distinct from our Svelte/Tauri app.
- Good as a *secondary channel* for users who live in Game Bar; poor as the primary overlay given the
  Store dependency and the duplicate UI. Our LocalSystem service could feed it the same sensor data.

---

## 6. Anti-cheat implications (compatibility, not evasion)

The goal is: **do not get legitimate users' games blocked or their accounts flagged**. The pattern
across vendors is consistent — *injection is the trigger; a separate OS window is not*.

- **Injection is what's scanned hardest.** "DLL injection (loading code into the game process) is what
  anti-cheat scans hardest… A plain always-on-top window is a separate window the OS draws above the
  game — no hook, no injection" is explicitly the safe approach [fact, edgedrop.app]. Present-hooking
  and swapchain-handle grabbing are the riskiest [fact, edgedrop.app].
- **Per-vendor:**
  - **VAC / CS2 "Trusted Mode":** blocks third-party files interacting with the game by default; to
    inject in normal mode the DLL must be **digitally signed**, else the game drops to insecure mode;
    `-allow_third_party_software` opts out. Signed overlays (Steam, and signed Afterburner/RTSS builds)
    are tolerated [fact, Valve/Steam support, esports.net].
  - **Riot Vanguard (Valorant/LoL):** most aggressive; kernel-level; blocks RTSS/Afterburner because
    their `RTCore64.sys` driver is blocked, and flags many overlay tools. Discord/Steam overlays
    generally coexist; some overlays cause black screens [fact, thespike/thespike-type sources].
  - **EAC:** "tolerant of legitimate overlay tools but flags low-level hooks"; present-hooks and
    injected wrapper DLLs are flagged; refuses to go online if a loaded module looks injected/modified
    [fact, edgedrop.app; EAC "untrusted system file" reports].
  - **BattlEye:** "topmost windows are fine; injection is not"; actively blocks RTSS on startup in some
    titles; completely blocks Special K in Escape from Tarkov [fact, edgedrop.app; Special K wiki].
  - **Ricochet (CoD):** no official overlay policy; bans for RTSS/Afterburner are "rare" per EA-style
    responses; stealth mode used as precaution [fact, search].
  - **FACEIT:** kernel AC; blocks RTSS unless whitelisted [fact, search].
- **How RTSS/others survive:** signed binaries + reputation + per-vendor whitelisting; RTSS offers a
  "Low detection + Stealth mode" compatibility toggle [fact]. OBS game-capture has an explicit
  "anti-cheat compatibility hook" path that uses the safer injection route and avoids it for UWP
  [fact, OBS `game-capture.c`: `SETTING_ANTI_CHEAT_HOOK`, `use_anticheat`]. ReShade ships a **signed
  build with add-ons disabled** that is whitelisted and **auto-disables** itself on multiplayer/network
  activity, plus a separate full-add-on build that is **not** whitelisted and is single-player-only
  [fact, reshade.me 5.0 notes]. Special K explicitly tells users to close it before multiplayer
  [fact, Special K wiki].
- **Defender / SmartScreen:** unsigned injector DLLs are routinely flagged `HackTool:Win64/GameHack`
  or generic trojan heuristics; ReShade trips this on each new release "partly because it's not
  digitally signed," clearing after definitions update [fact, gridinsoft; reshade.me forums].
  → OMA's planned Authenticode signing is essential if we ever inject; even then, injectors draw AV
  heuristic attention.
- **SignPath Foundation caveat:** their policy won't sign tools "designed to… circumvent security
  measures" and flags system modification without warning [fact, signpath.org/terms]. A benign
  monitoring overlay should qualify, but an **injector** that hooks arbitrary games is at real risk of
  being refused by SignPath [inference] — another reason to keep the signed product hook-free and gate
  any injector behind an explicit, clearly-labelled opt-in.

**Net:** approaches D and E (and C only where the user's RTSS is already whitelisted) are
anti-cheat-safe. Approaches A and B are not, for online games, without per-vendor whitelisting that a
small project won't get.

---

## 7. Open-source building blocks and licenses (GPL-3.0-or-later compatibility)

| Component | Latest / activity (2026) | License | GPL-3 compatible? | Role |
|-----------|--------------------------|---------|-------------------|------|
| **hudhook** (veeenu) | 0.9.3, 2026-09-08; 358★; active; targets x86_64 + i686 (both msvc/gnu) | MIT | yes | imgui overlay hooks for DX9/11/12 + OpenGL3; **no Vulkan**; own CreateRemoteThread injector; vendors MinHook (hde32/hde64) |
| **MinHook** | v1.3.4 (2025-03); orig TsudaKageyu BSD-2 | BSD-2 | yes | inline x86/x64 hooking |
| **minhook (Rust)** (Jakobzs) | 0.9.0, 2025-12 | MIT | yes | Rust bindings |
| **retour-rs** | 0.3.1 crate (2025-09); repo 0.4.0-alpha | BSD-2 | yes | detours/inline + static detour in Rust |
| **ilhook-rs** | 2.3.0, 2025-10 | MIT | yes | x86/x64 inline hooks (Rust) |
| **Microsoft Detours** | v4.0.1 (repo active 2026) | MIT | yes | trampoline hooking (C/C++) |
| **detours-sys (Rust)** | 0.1.2 (old) | MIT/Apache | yes | stale |
| **kiero** | archived; 1.2.12 (2021) | MIT | yes | vtable hook finder DX9-12/GL/Vulkan; unmaintained |
| **Dear ImGui** | v1.92.9b (2026-07); 76k★ | MIT | yes | overlay widgets; freetype optional |
| **imgui-rs** | 0.12 (2024) | MIT/Apache | yes | hudhook's imgui bindings (slightly behind) |
| **dear-imgui-rs** | 0.18.0 (2026-09) | MIT/Apache | yes | newer Rust imgui alt |
| **ReShade** | 6.8.0; BSD-3; add-on API covers DX9-12/GL/Vulkan | BSD-3 | yes (but see §6 online policy) | generic post-fx + add-on overlays via `register_overlay` |
| **Special K** | SK_26_9_25 (2026-09); 2093★ | **GPL-3.0** | yes (same license) | mature global injection + overlay; uses `SetWindowsHookEx(WH_CBT/WH_SHELL)` for global inject |
| **MangoHud** | v0.8.4 (2026-05); 9k★; MIT | MIT | yes | **Linux-only** officially |
| **MangoHud-Windows** (Leclowndu93150) | tiny fork (1★); Vulkan implicit layer + dxgi proxy for D3D11/12; 32+64-bit | (LICENSE present; MIT-derived) | likely yes | reference for a Windows MangoHud-style layer |
| **OBS graphics-hook** | OBS 32.2.2 (2026-08) | GPL-2.0 **only** | **NO — GPL-2-only is incompatible with GPL-3** | reference only; cannot copy code into a GPL-3 app |
| **Intel PresentMon** | v2.6.0 (2026-09); 2612★ | MIT | yes | ETW FPS/frametime/telemetry service + uiAccess D3D11 overlay — closest architectural match to OMA |

Notes:
- **OBS graphics-hook is GPL-2.0-only**, so OMA (GPL-3.0-or-later) **cannot** incorporate its code,
  only study it [fact, OBS COPYING = GPLv2; README "GPL v2 or any later" applies to OBS as a whole but
  the hook files are GPLv2 — [verify] per-file headers before any reuse; safest: don't reuse].
- **Special K is GPL-3** — fully license-compatible and the most mature reference for global injection;
  but it's a huge C++ codebase and an injector (see §6 AC caveats).
- hudhook has **no Vulkan backend** [fact, Cargo.toml features = dx9/dx11/dx12/opengl3 only]; a Vulkan
  OSD needs our own layer (§2) or RTSS (§3).

---

## 8. Rendering text and simple graphs cheaply in-game

- **Dear ImGui:** immediate-mode, single vertex/index buffer, typically 1 draw call for a small HUD;
  reported "almost 0 CPU" for complex editor UIs at 60+ fps, ~2–7 % CPU only when the whole demo is
  maxed [fact, forrestthewoods; imgui issue 3483]. For a few text lines + a couple of sparkline graphs
  the per-frame cost is negligible (tens of µs CPU) [inference]. Font atlas rasterized once (stb_truetype,
  or freetype feature). Good default for an **injected** overlay.
- **Direct2D / DirectWrite:** for the **hook-free window (D)** we own the device, so DirectWrite gives
  high-quality text with **system fonts** (enumerate via DirectWrite font collection) at trivial cost,
  and Direct2D draws lines/filled areas/bars for graphs. No third-party font needed; for a bundled font
  prefer **SIL OFL** (FSF-approved, explicitly allows bundling/aggregation with GPL software; keep the
  font a separate, clearly-licensed file) [fact, OFL FAQ / Wikipedia]. Avoid fonts that are GPL-without-font-exception.
- **Custom quad batching:** only worth it if we want zero dependencies; one dynamic vertex buffer +
  a texture atlas for glyphs + instanced/triangle-list quads. More code than ImGui/D2D for the same
  result; recommend only if ImGui's MIT dep is unwanted (it isn't — MIT is fine).
- **Graphs:** line/area/bar from a ring buffer of samples; with RTSS backend we instead fill
  `RTSS_EMBEDDED_OBJECT_GRAPH` and let RTSS draw (§3.1).
- **Cost target:** the OSD must respect OMA's budget; sampling is already done by the service, so the
  overlay only formats + draws. Keep redraw to the chosen chart FPS (60/30/15 per the existing
  chart-FPS setting memory) rather than every game frame when using window-D [inference].

---

## 9. FPS / frametime without injection (important)

FPS and frametime — the one thing people assume needs a Present hook — can be measured **from outside**
via **ETW** (the Microsoft-DirectX "Microsoft-Windows-DxgKrnl"/present events), which is exactly what
**Intel PresentMon** does for every API (DX, GL, Vulkan) and every process [fact, PresentMon].
PresentMon needs the user in the **"Performance Log Users"** group or admin for full process info
[fact, PresentMon README] — and OMA already has a **LocalSystem service**, which satisfies that without
elevating the UI. So OMA can show real FPS/frametime/latency with **no code in the game at all**,
feeding the hook-free overlay (D) or Game Bar widget (E). This is the single most important finding for
staying anti-cheat-safe.

---

## 10. Comparison matrix (approach × APIs × FSE × AC risk × effort × license)

| Approach | D3D9 | D3D11 | D3D12 | OpenGL | Vulkan | Exclusive FS | Anti-cheat risk | Effort | License of building blocks |
|----------|:----:|:-----:|:-----:|:------:|:------:|:------------:|-----------------|--------|-----------------------------|
| A. Own injection + hooks (hudhook + own Vk) | ✓ | ✓ | ✓ | ✓ | ✗ (hudhook) / ✓ via B | ✓ | **High** (injection flagged; bans/blocks online; AV flags; SignPath may refuse) | **Very high** (resize/HDR/multi-swapchain/hook-chaining; two bitnesses) | hudhook MIT, MinHook BSD-2, ImGui MIT — all GPL-3 OK |
| B. Own Vulkan implicit layer | – | – | – | – | ✓ | ✓ | **High** (in-process; AC must whitelist) | Medium-high (two layer DLLs, registry, loader quirks) | MangoHud MIT ref; ash MIT/Apache; ImGui MIT |
| C. RTSS backend (shared memory) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | **Medium** (RTSS itself blocked by Vanguard/BattlEye/FACEIT; fine elsewhere) | **Low** (write a mapped-file client; we don't inject) | Our own GPL-3 reader; RTSS freeware, **not redistributable** — user installs |
| D. Hook-free topmost window | ✓ | ✓ | ✓ | ✓ | ✓ | ✗ plain; ✓ with uiAccess (signed, ProgramFiles) | **Low** (no injection; the sanctioned safe pattern) | Low-medium (window + D2D/DWrite; uiAccess needs signing) | D2D/DWrite (OS), ImGui MIT optional, OFL font |
| E. Xbox Game Bar widget | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ (Game Bar band) | **Low** | Medium-high (UWP/MSIX, Store publish, 2nd UI stack) | MS SDK (proprietary NuGet, runtime use) |
| (FPS source) ETW / PresentMon | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | **None** (out-of-process) | Low-medium (ETW consumer in service) | PresentMon MIT |

APIs row for C/D/E means "can display the OSD over a game using that API," since those approaches are
API-agnostic (RTSS injects; D/E composite at the OS level).

---

## 11. Recommendation for OpenMonitor Advanced

Overlay is explicitly out of v1 scope in OMA's spec ("Overlay in-game (OSD)… own sub-project with its
own spec") [fact, design spec line 18], so this feeds a future sub-project. Phased plan:

**Phase 1 — anti-cheat-safe, no injection, ships unsigned-capable:**
1. **FPS/frametime via ETW in `oma-service`** (LocalSystem already has the rights) — PresentMon-style,
   MIT, no game-process code. Extend the IPC protocol with an OSD/frame-stats message.
2. **Hook-free topmost click-through overlay window** (approach D) rendered with Direct2D/DirectWrite
   (system fonts) or a small D3D11+ImGui, driven by the service's sensor + FPS data. Works over the
   large majority of modern games (borderless + Win11 FSO default-on). Draw nothing when toggled off to
   preserve VRR.
3. **Optional RTSS backend** (approach C): if `RTSSSharedMemoryV2` is present, let the user route the
   OSD through RTSS (covers true exclusive fullscreen today, for users who already run it). Ship our own
   GPL-3 Rust/C# reader of the documented layout — **do not** bundle RTSS, **do not** copy Unwinder's
   sample.

**Phase 2 — once SignPath Authenticode signing lands:**
4. Make the overlay exe `uiAccess="true"`, installed under Program Files (installer already does), so it
   sits above exclusive-fullscreen games without injecting (Intel PresentMon's proven technique).

**Phase 3 (only if strongly demanded) — injection, opt-in, single-player only:**
5. hudhook (MIT) for DX9/11/12/GL + a hand-written Vulkan implicit layer (MangoHud-Windows as reference)
   for Vulkan/DXVK. Clearly label it "single-player / may conflict with anti-cheat," auto-disable like
   ReShade when online/AC is detected, and expect SignPath scrutiny. This is a big QA surface
   (resize/HDR/multi-swapchain/hook-chaining) — defer unless users need an overlay in genuinely
   exclusive-fullscreen, non-RTSS, single-player titles.

**Hard "don't":** no OBS graphics-hook code (GPL-2-only, incompatible); no AppInit_DLLs; no RTSS
redistribution; no synthetic input / no sitting silently in online competitive games' processes.

---

## 12. Open items to verify on the test rig / before spec freeze
- [verify] VRR + latency behaviour of a topmost overlay on RTX 4080 and the AMD iGPU with MPO on/off.
- [verify] Whether a non-admin OMA process can open `RTSSSharedMemoryV2` read/write (DACL check).
- [verify] RTSS EULA redistribution clause (text not found online).
- [verify] uiAccess process spawned from OMA's non-admin UI (known spawn/PostMessage quirks).
- [verify] HDR swapchain color handling for any injected renderer.
- [verify] Per-file license headers in OBS graphics-hook before even reading for reference.
