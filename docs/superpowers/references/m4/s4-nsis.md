# Spike S4: custom NSIS template (Tauri CLI 2.11.5) and service install

Date 2026-09-26, Windows 11 Pro 26200, not elevated, `@tauri-apps/cli` 2.11.5, NSIS 3.11 (Tauri's bundle), .NET SDK 10.0.303.
The throwaway worktree `spike/m4-nsis` has been removed and the branch deleted. The surviving files are in [`s4-nsis/`](s4-nsis/):

- `installer.nsi`: our template, the one that compiled;
- `installer.diff`: the full diff against upstream;
- `oma.nsh`: all of our NSIS logic;
- `nsis-template.test.ts`: the drift test;
- `nsis-template-drift.ps1`: the upstream diff and merge script;
- `tauri.conf.diff`: the config changes.

The installer produced was **never run**. The only thing executed was a separate, non-elevated `/S` harness (`harness.nsi`, `RequestExecutionLevel user`) that tests NSIS semantics.

## Verdict

**It works, and the diff can stay tiny: 8 changed lines in `installer.nsi`.**

- All of the logic lives in a second file, `nsis/oma.nsh`, which Tauri pulls in through `bundle.windows.nsis.installerHooks`.
- makensis 3.11 compiled it with **0 errors and 0 warnings**, also at `-V4`.
- The payload layout is as intended (7-Zip listing below).
- A 3-way merge of our template onto Tauri `dev`, which already changes `CheckIfAppIsRunning`, applies with **0 conflicts**.

## 1. Upstream files (tag `tauri-cli-v2.11.5`, commit `9452ddee`)

The directory `crates/tauri-bundler/src/bundle/windows/nsis/` contains `installer.nsi`, `utils.nsh`, `FileAssociation.nsh`, `languages/*.nsh` and `mod.rs`.

- **Only `installer.nsi` can be replaced** (`bundle.windows.nsis.template`, read with `fs::read_to_string` and rendered with Handlebars).
- On every build `mod.rs` writes the rest **from its own embedded copies** into `<target>/release/nsis/x64/`:
  - `utils.nsh`, which holds `SetContext`, `CheckIfAppIsRunning`, the shortcut and COM macros;
  - `FileAssociation.nsh`;
  - `<Lang>.nsh` for each language, which holds the `$(appRunning)`, `$(older)`… strings;
  - the rendered `installer.nsi`.

  makensis then runs with `-INPUTCHARSET UTF8 -OUTPUTCHARSET UTF8 -V3` and cwd = that directory.
- ⇒ **Our template calls macros and strings that the bundler owns.** A CLI bump can change them under us. On `dev`, `CheckIfAppIsRunning executableName` has already become `executablePath` (restart manager, #14479). So the drift guard has to pin the CLI version exactly.
- Handlebars helpers available: `or`, `association-description`, `no-escape`, plus the built-in ones. The Handlebars data has no path to `src-tauri`, **but `{{installer_hooks}}` is canonicalised to an absolute path.** Inside the hooks file, `${__FILEDIR__}` is therefore `app/src-tauri/nsis`, and that is how we locate the payload wherever `CARGO_TARGET_DIR` is.

## 2. Design as built

### `tauri.conf.json` (`bundle`)

```diff
-    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.ico"]
+    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.ico"],
+    "publisher": "OpenMonitorAdvanced",
+    "windows": {
+      "nsis": {
+        "template": "nsis/installer.nsi",
+        "installerHooks": "nsis/oma.nsh",
+        "installMode": "perMachine",
+        "languages": ["English", "Italian"]
+      }
+    }
```

- **`publisher` is needed.** Without it, `MANUFACTURER` becomes the *second* segment of the identifier (`io.github.openmonitoradvanced` → `github`), so `MANUKEY` = `HKLM\Software\github`.
- `languages` was added to test the `LANG_ITALIAN` strings. It compiles, with 2 language tables.

Full diff: [`s4-nsis/tauri.conf.diff`](s4-nsis/tauri.conf.diff).

### Template diff (full, compiled)

Full diff: [`s4-nsis/installer.diff`](s4-nsis/installer.diff). Full compiled template: [`s4-nsis/installer.nsi`](s4-nsis/installer.nsi).

```diff
--- upstream-2.11.5.nsi
+++ installer.nsi
@@ -384,6 +384,8 @@
   reinst_done:
 FunctionEnd
 
+!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive ; OMA
+!insertmacro MUI_PAGE_COMPONENTS ; OMA "Advanced sensors" section
 ; 5. Choose install directory page
 !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
 !insertmacro MUI_PAGE_DIRECTORY
@@ -473,6 +475,7 @@
 {{#each language_files}}
   !include "{{this}}"
 {{/each}}
+!insertmacro OMA_LANGSTRINGS ; OMA
 
 Function .onInit
   ${GetOptions} $CMDLINE "/P" $PassiveMode
@@ -495,6 +498,7 @@
   !endif
 
   !insertmacro SetContext
+  !insertmacro OMA_ONINIT ; OMA
 
   ${If} $INSTDIR == "${PLACEHOLDER_INSTALL_DIR}"
     ; Set default install location
@@ -524,7 +528,7 @@
 FunctionEnd
 
 
-Section EarlyChecks
+Section -EarlyChecks ; OMA hidden
   ; Abort silent installer if downgrades is disabled
   !if "${ALLOWDOWNGRADES}" == "false"
   ${If} ${Silent}
@@ -543,7 +547,7 @@
 
 SectionEnd
 
-Section WebView2
+Section -WebView2 ; OMA hidden
   ; Check if Webview2 is already installed and skip this section
   ${If} ${RunningX64}
     ReadRegStr $4 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
@@ -635,7 +639,7 @@
   ${EndIf}
 SectionEnd
 
-Section Install
+Section -Install ; OMA hidden
   SetOutPath $INSTDIR
 
   !ifmacrodef NSIS_HOOK_PREINSTALL
@@ -740,6 +744,7 @@
   ${EndIf}
 SectionEnd
 
+!insertmacro OMA_SECTIONS ; OMA
 Function .onInstSuccess
   ; Check for `/R` flag only in silent and passive installers because
   ; GUI installer has a toggle for the user to (re)start the app
```

Why each edit is needed:

- **Components page:** hooks cannot insert a page.
- **Hidden `-` sections:** the template's sections are *named*, so they would show as checkboxes. The `-` prefix hides them, and NSIS counts hidden sections as required: "5 sections (4 required)".
- **`OMA_ONINIT`:** it has to run after `SetContext`, which sets the 64-bit registry view.
- **`OMA_LANGSTRINGS`:** `LangString` must come after `MUI_LANGUAGE`, and hooks are included before it.
- **`OMA_SECTIONS` after `-Install`:** the app has already been closed by `CheckIfAppIsRunning` when we stop the service. The section index `${SecSensors}` exists only after the `Section` line, so `.onInit` calls a function, `OmaInitComponents`, defined inside `OMA_SECTIONS`.
- **The uninstaller needs no template edit.** Our logic is `!macro NSIS_HOOK_PREUNINSTALL` in `oma.nsh`. A first version inserted one line after `CheckIfAppIsRunning` in `Section Uninstall`, and that line produced the only merge conflict against `dev`. Moving it into the hook removed the conflict.

### `oma.nsh` (the logic; full file: [`s4-nsis/oma.nsh`](s4-nsis/oma.nsh))

- **Payload:**
  - `!define OMA_PAYLOAD "${__FILEDIR__}\..\..\..\target\installer-payload"`, containing `service\oma-service.exe` and `PawnIO_setup.exe`;
  - `!if /FileExists … !else !error "run the payload script first"` gives a clear compile error if the payload script was not run.
  - The payload sits in the repo's `target/` (already gitignored), **not** in `src-tauri/`, where `tauri dev` would watch 70+ MB.
  - We do **not** use `bundle.resources`: `tauri-build` checks that resources exist at *every* `cargo build`/`clippy`/`test` of `oma-app`, which would make the whole Rust workflow depend on `dotnet publish`.
- **`OmaInitComponents`** (from `.onInit`):
  - the initial state is the stored `HKLM\Software\OpenMonitorAdvanced\Installer!AdvancedSensors` (DWORD) if it exists, otherwise checked;
  - `/NOSENSORS` always forces it off.

  This single rule covers GUI, `/P`, `/S` and `/UPDATE`. Fresh `/S` → default on; any update → stored choice.
- **`Section "$(omaSensorsSection)" SecSensors`** (visible, checked by default):
  1. `Call OmaStopService`;
  2. `File` the exe to `$INSTDIR\service\`;
  3. `nsExec::ExecToLog '"$INSTDIR\service\oma-service.exe" install'`;
  4. the PawnIO check. `SetRegView 64`, read `Uninstall\PawnIO!DisplayVersion`, then `${VersionCompare}` against `2.2.0`. If the value is missing or older, `File /oname=$PLUGINSDIR\PawnIO_setup.exe`, `ExecWait '"…" -install -silent' $3`, and exit code 3010 → `SetRebootFlag true`.
- **`Section -OmaSensorsBookkeeping`:**
  - writes `AdvancedSensors` = 1/0;
  - if the section was deselected on a reinstall and `$INSTDIR\service\oma-service.exe` exists: stop, then helper `uninstall`, then delete the file and the directory.
- **`OmaStopService` / `un.OmaStopService`** calls the SCM directly through `System::Call`:
  - `OpenSCManagerW(SC_MANAGER_CONNECT)` → `OpenServiceW(STOP|QUERY_STATUS)` → `ControlService(STOP)`;
  - then polls `QueryServiceStatus` every 500 ms until `SERVICE_STOPPED`, for up to 30 s;
  - a missing service is a no-op.

  This needs no child process and does not depend on the *previous* version's helper. The System32/SysWOW64 redirection of 32-bit NSIS does not matter: the SCM does not depend on bitness. That also answers the question about `$WINDIR\Sysnative\sc.exe`: it is not needed, and **`sc.exe` output is localised** (on this PC: "OPERAZIONI NON RIUSCITE 1060"), so parsing it would be fragile.
- **`NSIS_HOOK_PREUNINSTALL`** (a Tauri hook):
  - if the service exe exists: stop it; unless `$UpdateMode`, run the helper `uninstall`; delete `service\`;
  - unless `$UpdateMode`, delete our registry value and the key if empty;
  - PawnIO is not touched.
- **`nsExec::ExecToLog`**, not `ExecWait`: no console window flashes and the helper output goes to the details log. Pitfall: when the process cannot start, the pushed value is the string `error`. `${If} $0 <> 0` reads it as 0, i.e. success, so we compare with `!= "0"`.

## 3. Build results

`pnpm tauri build --bundles nsis`, `CARGO_TARGET_DIR` in the scratchpad.

| | |
|---|---|
| First build (cold target, NSIS download) | **98 s** (cargo release 69 s) |
| Rebuilds (cargo cached) | 37 s |
| makensis alone (76 MB payload, solid LZMA) | **17.4 s** |
| Tauri NSIS download | `nsis-3.11.zip` (SHA-1 pinned) + `nsis_tauri_utils.dll` v0.5.3 → `%LOCALAPPDATA%\tauri\NSIS` (7.9 MB, left in place as Tauri's cache; `nsExec.dll` is included) |
| Errors / warnings | 0 / 0 (checked again at `-V4`) |
| Installer | **26.79 MB** (26 792 893 B), `LZMA:23`, solid |
| `oma-app.exe` | 11.5 MB raw |
| Stand-in service | `dotnet new worker` + `Microsoft.Extensions.Hosting.WindowsServices`, `-r win-x64 --self-contained -p:PublishSingleFile=true` → **75.9 MB** raw, ≈ 24 MB compressed (the app part of the installer ≈ 2.7 MB) |

makensis summary:

```
Install: 7 pages (448 bytes), 5 sections (4 required) (10360 bytes), 2132 instructions, 537 strings, 2 language tables
Uninstall: 2 pages (192 bytes), 1 section (2072 bytes), 706 instructions, 259 strings, 2 language tables
Using lzma (compress whole) compression.
Total size: 26792893 / 87930786 bytes (30.4%)
```

`7z l` of the installer:

```
$PLUGINSDIR\System.dll, modern-wizard.bmp, nsDialogs.dll, nsis_tauri_utils.dll, StartMenu.dll, NSISdl.dll
oma-app.exe                    11532288
service\oma-service.exe        75864732
$PLUGINSDIR\nsExec.dll             7168
$PLUGINSDIR\PawnIO_setup.exe   (dummy, 168974)
```

**Note for the payload script:** `dotnet publish` also writes `oma-service.pdb` and `appsettings*.json`. Stage only what we ship (`-p:DebugType=none`, and no appsettings unless needed).

### Semantics checked with a non-elevated `/S` harness

| Check | Result |
|---|---|
| `${VersionCompare}` (0 equal, 1 first newer, 2 older) | `2.2.0.0` vs `2.2.0` → **0**; `2.2` vs `2.2.0` → 0; `2.1.9.0` → 2; `2.10.0.0` → 1 (numeric, not lexicographic); `2.2.0-beta` → **1** (suffixes are not understood: fine for PawnIO, which is numeric) |
| PawnIO key from 32-bit NSIS | view 32 → **empty**; `SetRegView 64` → `2.2.0.0` (installed on this PC; `VersionMajor`/`VersionMinor` DWORDs exist too) |
| `/NOSENSORS`, `/nosensors`, `/S /NOSENSORS /P` | the section is unselected; `/P` is still seen |
| `/NS`, `/S /UPDATE` | the section stays selected (no collision with `/NS`) |
| `/NOSENSORSX`, `/NOSENSORS=0` | **also match**: `GetOptions` is a case-insensitive *prefix* match. Consequence: never add a switch that starts with `/P`, `/R`, `/NS`, `/ARGS` or `/UPDATE`; for example `/PAWNIO…` would turn on passive mode. |
| `OmaStopService` with no service | returns silently |

## 4. Drift check: recommendation

**A Vitest test in `app/`, plus a PowerShell script for the upstream diff and merge.** Vitest fits because the version to pin lives in `app/package.json` and `pnpm test` already runs in the checklist. The draft is in [`s4-nsis/nsis-template.test.ts`](s4-nsis/nsis-template.test.ts); it runs in about 10 ms. It checks three things:

1. exactly one `app/src-tauri/nsis/upstream-<ver>.nsi` exists, and the file name carries the base version;
2. `devDependencies['@tauri-apps/cli']` **and** the installed `@tauri-apps/cli/package.json` version both equal `<ver>`. The installed version catches a lockfile drift;
3. `installer.nsi` minus the lines ending with `; OMA …`, with `Section -X ; OMA hidden` mapped back to `Section X`, is **byte-identical** to the reference copy. This proves that the reference copy really is the base and that every edit is marked.

Verified in both directions:

- green as is;
- with the CLI set to `2.11.6` → *"template derived from 2.11.5: run scripts/nsis-template-drift.ps1 -To 2.11.6"*;
- with one unmarked line added to the template → the diff failure.

[`s4-nsis/nsis-template-drift.ps1`](s4-nsis/nsis-template-drift.ps1) `-To <ver> [-Ref <git ref>] [-Apply]`:

- downloads `installer.nsi`, `utils.nsh`, `FileAssociation.nsh`, `languages/English.nsh` and `languages/Italian.nsh` for both tags;
- prints `git diff --no-index` for each file. The bundler-owned files matter because our template calls into them;
- runs `git merge-file` (ours = template, base = old upstream, theirs = new upstream);
- with `-Apply` and no conflicts, it writes the merged template plus `upstream-<new>.nsi` and deletes the old reference copy.

Tried with `-Ref dev`: the diff shows `RestartManager.nsh`, the `CheckIfAppIsRunning` change to `$INSTDIR\…` and the `utils.nsh` macro rewrite. The merge had **0 conflicts** (1 conflict before the uninstaller moved into the hook).

Rejected alternatives:

- a Rust test: it would have to parse `package.json` from `oma-app`;
- a CI-only download check: it needs network access in the tests.

Local caveat: inside the worktree under the session's own scratchpad, Vitest would not start (`ERR_PACKAGE_IMPORT_NOT_DEFINED` for `#module-evaluator`). The likely cause is the `node_modules\.pnpm\vitest@…_<hash>\…\package.json` path exceeding MAX_PATH under a long temporary path. The test ran with the main checkout's Vitest (`--root <worktree>/app`). This has no effect on the real repo path.

## 5. The helper verbs in C# (instead of SDDL in NSIS): yes

Reasons:

- **Testability:** the DACL splice is a pure function (SDDL in → SDDL out, via `RawSecurityDescriptor`/`CommonSecurityDescriptor`). xUnit can cover:
  - the SDDL of a real service (`sc sdshow`);
  - idempotence: an IU ACE that already exists is not added again;
  - canonical order: deny ACEs first;
  - the absence of `DC`/`WD`/`WO`;
  - a malformed input.

  NSIS has no tests and only string operations.
- **Robustness:** Win32 calls with real error codes (`QueryServiceObjectSecurity`/`SetServiceObjectSecurity`, `CreateService`/`ChangeServiceConfig`, `ChangeServiceConfig2(SERVICE_CONFIG_FAILURE_ACTIONS)`) instead of `sc.exe`, whose output is localised.
- **Security:** the helper runs from `Program Files`, which only administrators can write, and its verbs need admin rights on the SCM anyway. A non-admin who invokes them gets `ERROR_ACCESS_DENIED`.

Requirements for the helper, to put in the plan:

- **Exit codes:** documented and stable, 0 = OK. The installer treats any non-zero code or `error` as "sensors not configured". We chose to warn and continue (`MessageBox … /SD IDOK`) and **not to abort**, because the app works in base mode anyway.
- **`install` is idempotent:** it creates or updates the config (binPath quoted, `SERVICE_DEMAND_START`, failure actions) and splices the ACE only when it is missing. It must handle **`ERROR_SERVICE_MARKED_FOR_DELETE` (1072)**, i.e. retry for a few seconds. This state is common right after the old uninstaller has deleted the service during a GUI upgrade: any open handle (services.msc, our own query) delays the deletion.
- **`uninstall`:** stop and wait, then `DeleteService`. A missing service returns 0.
- **`stop`:** optional. The installer does not use it, because at install time the new exe is not in place yet and the old one could be any version; the SCM `System::Call` covers the case.
- **Console window:** the helper should not create one. `nsExec` hides it anyway.
- **Startup cost:** a single-file self-contained exe starts in roughly 100–300 ms per verb, which is negligible.

## 6. Conflicts with the Tauri template

- **`CheckIfAppIsRunning`:**
  - `nsis_tauri_utils::FindProcess "oma-app.exe"` + `KillProcess` in per-machine mode matches **by name, in all sessions**. On this dev PC, a test install would kill the `oma-app.exe` from `target\debug` and the user's running one: **never run the installer on the dev machine while the app is running**;
  - it only closes the app, never the service: hence `OmaStopService`;
  - in 2.11.5 it is a name match. `dev` switches to a path and the Restart Manager, and the signature changes (drift!).
- **Reinstall/upgrade page (`PageReinstall`)**, shown in GUI mode when an install exists:
  - Upgrade: the default is "Uninstall before installing", which runs the **old uninstaller in full, without `/UPDATE`**. That uninstaller then deletes the service (via our hook) and `AdvancedSensors`.
  - It is harmless: `.onInit` has already read the choice, the bookkeeping section writes it back, and our section re-creates the service. Error 1072 is the one to handle (§5).
  - In `/P` mode the page calls `PageLeaveReinstall` with no radio buttons (`$R2` is a string, so `NSD_GetState` → 0). The outcome:
    - same version → **the uninstaller runs**;
    - upgrade → no uninstall, overwrite.
  - In `/S` mode the page never runs: plain overwrite.
- **Bug #16012** (closed 2026-09-13 with no PR; `dev` still has no `Quit`). Choosing "Uninstall" on the same-version page uninstalls and then **reinstalls**. For us it is a cosmetic UX issue: the user sees the components page again. We could add `Quit` after a successful same-version uninstall, but that is a diff inside `PageLeaveReinstall`, where upstream is volatile. **Leave it as is.**
- **`$UpdateMode`** is set **only** by `/UPDATE`, which is passed only by `tauri-plugin-updater` (`/P` or `/S` + `/UPDATE` [+ `/R`]).
  - In the installer it skips the WebView2 install, the shortcuts and the uninstall-first step.
  - The uninstaller gets `/UPDATE` only through `reinst_uninstall` when `$UpdateMode=1`, which the code skips except for the WiX migration. **In practice our uninstaller never sees `$UpdateMode=1`.** The `$UpdateMode` branch in our hook is defensive.
  - A real update therefore **never removes the service**: our section's `OmaStopService` plus the idempotent `install` must handle an in-place update.
- **Per-machine + updater:** the installer has `RequestExecutionLevel admin`, so **every** update shows UAC, even in `/S`.
  - `passive` (the default) is the only sensible mode: it shows a progress bar and the UAC prompt;
  - `quiet` "requires admin privileges if the installer does" and is unusable for unattended per-machine updates.
  - Irrelevant for v1, which has no updater (§9 spec: link only). Revisit only if the updater is adopted.
- **Silent downgrades:** `EarlyChecks` relies on `$R0` from `PageReinstall`, which never runs under `/S`, so silent downgrades are *not* blocked. This is an upstream bug; we do not touch `$R0`.
- **`SetRebootFlag` (PawnIO 3010):**
  - in GUI mode, MUI's finish page offers reboot now/later;
  - in `/P` and `/S` the finish page is skipped and nothing signals the reboot. **Recommendation:** in `.onInstSuccess`, `${If} ${RebootFlag}` → `SetErrorLevel 3010`, so that winget and other deployment tools see it. This needs a hook or a template edit, because `.onInstSuccess` is a template function: an `OMA_ONINSTSUCCESS` line.
- **`$PLUGINSDIR` for `PawnIO_setup.exe`** (and for WebView2 in the template's `$TEMP`): the elevated installer runs an exe from a directory derived from the user's `%TEMP%`. The risk of DLL planting or a race is low (PawnIO's setup is itself an NSIS installer, which hardens its DLL search), but a better choice is `$INSTDIR\service\PawnIO_setup.exe`, which only administrators can write, followed by `Delete`. The same applies if the helper is ever extracted before copying.
- **Signing (SignPath):** when signing is configured, Tauri signs and embeds its own copies of only these plugins: `NSISdl`, `StartMenu`, `System`, `nsDialogs`, `nsis_tauri_utils`. **`nsExec.dll` would be embedded unsigned.** Either sign it in `signCommand` or accept it. Worth checking at the SignPath step.
- **Hook timing:** `NSIS_HOOK_PREUNINSTALL` runs *before* `CheckIfAppIsRunning`. If the user then cancels the prompt that the app is running, the service is already gone: the app stays installed in base mode, and the next install re-creates the service because the stored choice was removed, so the default is on. This is accepted in exchange for a zero diff in the uninstaller. It also requires that **the app does not restart the service on its own after a disconnect** (spec §2.2 says "all'apertura", i.e. only at startup, and that is enough).

## 7. Recommendations for the M4 plan

1. Adopt the structure as is:
   - `app/src-tauri/nsis/installer.nsi` (8 marked lines);
   - `app/src-tauri/nsis/upstream-2.11.5.nsi`;
   - `app/src-tauri/nsis/oma.nsh` (all of the logic);
   - `bundle.publisher` + `windows.nsis.{template, installerHooks, installMode: perMachine, languages}`.
2. The payload script (`scripts/build-installer-payload.ps1`):
   - `dotnet publish` of the service to `target/installer-payload/service/` (exe only);
   - download the PawnIO setup at a fixed version with a SHA-256 check to `target/installer-payload/PawnIO_setup.exe`.

   Then `pnpm tauri build`. The `!error` guard enforces the order. Optionally wire the script as `build.beforeBundleCommand`.
3. Add the Vitest drift test and `scripts/nsis-template-drift.ps1`. Every `@tauri-apps/cli` bump then goes through `-To <ver>`, a review of the `utils.nsh` and `languages` diffs, `-Apply`, a rebuild and a manual install test.
4. Implement `install`/`uninstall` (+ optional `stop`) in C# with xUnit tests for the SDDL splice and the 1072 retry. Treat a non-zero code as "base mode", never as an aborted install.
5. Small fixes to include:
   - `SetErrorLevel 3010` when `${RebootFlag}` is set (silent/passive);
   - PawnIO extracted under `$INSTDIR` instead of `$PLUGINSDIR`;
   - no new switch whose prefix collides with `/P`, `/R`, `/NS`, `/ARGS` or `/UPDATE`.
6. Manual checks for the user (they need UAC and must not run on the dev PC with the app open, or in a VM):
   - GUI install: the components page shows only "Advanced sensors", with the description on hover in EN and IT;
   - `/S`, `/S /NOSENSORS`, a GUI upgrade (the default "uninstall first" path), `/P /UPDATE` with the stored choice 0 and 1;
   - deselecting on a reinstall removes the service;
   - uninstalling removes the service and keeps PawnIO;
   - the reboot prompt when PawnIO returns 3010.
