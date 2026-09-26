// Guards for the per-machine NSIS installer (spec §10, Task 13).
//
// 1. Drift: our template (app/src-tauri/nsis/installer.nsi) is Tauri's installer.nsi from the
//    pinned tauri-cli version plus lines marked `; OMA`. A CLI bump fails here until the
//    template is rebased with scripts/nsis-template-drift.ps1.
// 2. Payload script: a failed service publish never leaves a stale payload behind.
// 3. Static checks of the failure paths in nsis/oma.nsh: a failed STOP, helper verb or PawnIO
//    setup aborts before any file is replaced or deleted and before the choice is recorded.
//    These read the script, they do not run it: the same paths are exercised with real fault
//    injection in a VM in Task 15.
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

const appDir = resolve(__dirname, '../..');
const repoDir = resolve(appDir, '..');
const nsisDir = resolve(appDir, 'src-tauri/nsis');
const read = (p: string) => readFileSync(p, 'utf8').replace(/\r\n/g, '\n');

// The reference copy is named after the tauri-cli version it was taken from.
const upstreamFiles = existsSync(nsisDir) ? readdirSync(nsisDir).filter((f) => /^upstream-.+\.nsi$/.test(f)) : [];
const baseVersion = upstreamFiles[0]?.match(/^upstream-(.+)\.nsi$/)?.[1];

describe('custom NSIS template', () => {
  it('has exactly one upstream reference copy', () => {
    expect(upstreamFiles).toHaveLength(1);
  });

  it('the Tauri CLI version matches the upstream template copy', () => {
    const pkg = JSON.parse(read(resolve(appDir, 'package.json')));
    const installed = createRequire(resolve(appDir, 'package.json'))('@tauri-apps/cli/package.json')
      .version as string;
    const pinned = pkg.devDependencies['@tauri-apps/cli'] as string;
    const hint = `template derived from ${baseVersion}: run scripts/nsis-template-drift.ps1 -To ${pinned}`;
    expect(pinned, hint).toBe(baseVersion);
    expect(installed, hint).toBe(baseVersion);
  });

  it('the template differs from upstream only on lines marked OMA', () => {
    // Added lines end with "; OMA ..."; template sections are hidden with "-".
    const ours = read(resolve(nsisDir, 'installer.nsi'))
      .split('\n')
      .filter((l) => !/ ; OMA( .*)?$/.test(l) || / ; OMA hidden$/.test(l))
      .map((l) => l.replace(/^Section -(\w+) ; OMA hidden$/, 'Section $1'))
      .join('\n');
    expect(ours).toBe(read(resolve(nsisDir, upstreamFiles[0])));
  });

  it('calls the reboot exit code hook only from .onInstSuccess', () => {
    const lines = read(resolve(nsisDir, 'installer.nsi')).split('\n');
    const at = lines.findIndex((l) => l.trim() === '!insertmacro OMA_ONINSTSUCCESS ; OMA');
    expect(at).toBeGreaterThan(0);
    const enclosing = lines
      .slice(0, at)
      .reverse()
      .find((l) => /^Function /.test(l));
    expect(enclosing).toBe('Function .onInstSuccess');
    expect(lines.filter((l) => l.includes('OMA_ONINSTSUCCESS'))).toHaveLength(1);
  });
});

// ---------------------------------------------------------------------------------------------
// Payload script

const payloadScript = resolve(repoDir, 'scripts/build-installer-payload.ps1');
const scratch: string[] = [];
afterEach(() => {
  for (const dir of scratch.splice(0)) rmSync(dir, { recursive: true, force: true });
});

/**
 * Runs the payload script against a fake `dotnet` (a .cmd with the given body, where `{OUT}` stands
 * for the payload directory) in a temp dir.
 */
function runPayload(fakeDotnetBody: string) {
  const dir = mkdtempSync(join(tmpdir(), 'oma-payload-'));
  scratch.push(dir);
  const project = join(dir, 'project');
  const out = join(dir, 'payload');
  mkdirSync(join(project, 'obj'), { recursive: true });
  mkdirSync(join(out, 'service'), { recursive: true });
  // A stale payload from an earlier, successful build.
  const staleExe = join(out, 'service', 'oma-service.exe');
  writeFileSync(staleExe, 'stale');
  const fake = join(dir, 'fake-dotnet.cmd');
  writeFileSync(fake, `@echo off\r\n${fakeDotnetBody.replaceAll('{OUT}', out)}\r\n`);
  const run = spawnSync(
    'pwsh',
    ['-NoProfile', '-NonInteractive', '-File', payloadScript, '-DotnetExe', fake, '-ServiceProject', project, '-OutputRoot', out],
    { encoding: 'utf8', timeout: 120_000 },
  );
  return { run, staleExe, out, output: `${run.stdout}\n${run.stderr}` };
}

describe('installer payload script', { timeout: 120_000 }, () => {
  it('fails on a failed publish and does not keep the old service exe', () => {
    const { run, staleExe, out, output } = runPayload('echo publish failed & exit /b 1');
    expect(run.status, output).not.toBe(0);
    expect(output).toMatch(/dotnet publish failed with exit code 1/);
    expect(existsSync(staleExe), 'stale oma-service.exe must be gone').toBe(false);
    // It stops before the PawnIO step: nothing downloaded.
    expect(existsSync(join(out, 'PawnIO_setup.exe'))).toBe(false);
  });

  it('fails when publish "succeeds" without producing oma-service.exe', () => {
    const { run, staleExe, output } = runPayload('echo nothing published & exit /b 0');
    expect(run.status, output).not.toBe(0);
    expect(output).toMatch(/oma-service\.exe is missing/);
    expect(existsSync(staleExe)).toBe(false);
  });

  it('fails when publish leaves anything but oma-service.exe and its .pdb (the installer copies only the exe)', () => {
    const { run, out, output } = runPayload(
      'echo publishing & mkdir "{OUT}\\service" & echo exe> "{OUT}\\service\\oma-service.exe" & echo pdb> "{OUT}\\service\\oma-service.pdb" & echo dll> "{OUT}\\service\\native.dll" & exit /b 0',
    );
    expect(run.status, output).not.toBe(0);
    expect(output).toMatch(/unexpected files? in the publish output: native\.dll/);
    expect(existsSync(join(out, 'service')), 'the incomplete payload must be gone').toBe(false);
  });

  it('publishes without native libraries extracted at run time', () => {
    const pubxml = read(resolve(repoDir, 'service/OpenMonitorAdvanced.Service/Properties/PublishProfiles/Service.pubxml'));
    expect(pubxml).toMatch(/<IncludeNativeLibrariesForSelfExtract>false<\/IncludeNativeLibrariesForSelfExtract>/);
    expect(pubxml).not.toMatch(/<IncludeNativeLibrariesForSelfExtract>true/);
  });

  it('fails on a trim warning that is not on the allowlist', () => {
    const { run, output } = runPayload(
      'echo ILLink : Trim analysis warning IL2026: Oma.Fake.Unlisted(): not reviewed [x.csproj] & exit /b 0',
    );
    expect(run.status, output).not.toBe(0);
    expect(output).toMatch(/IL2026\|Oma\.Fake\.Unlisted\(\)/);
    expect(output).toMatch(/trim warning gate failed/);
  });

  it('rejects a cached PawnIO setup with the wrong hash and never uses it (no network)', () => {
    const dir = mkdtempSync(join(tmpdir(), 'oma-pawnio-'));
    scratch.push(dir);
    const out = join(dir, 'payload');
    mkdirSync(out, { recursive: true });
    const cached = join(out, 'PawnIO_setup.exe');
    writeFileSync(cached, 'not the pinned PawnIO setup');
    const source = join(dir, 'PawnIO_setup.exe');
    writeFileSync(source, 'also not the pinned PawnIO setup');
    const run = spawnSync(
      'pwsh',
      ['-NoProfile', '-NonInteractive', '-File', payloadScript, '-PawnIoOnly', '-PawnIoSource', source, '-OutputRoot', out],
      { encoding: 'utf8', timeout: 120_000 },
    );
    const output = `${run.stdout}\n${run.stderr}`;
    expect(run.status, output).not.toBe(0);
    expect(output).toMatch(/cached copy rejected \(SHA-256 /);
    expect(output).toMatch(/PawnIO_setup\.exe rejected: SHA-256 /);
    expect(existsSync(cached), 'the rejected file must not stay in the payload').toBe(false);
    expect(existsSync(`${cached}.partial`)).toBe(false);
  });

  it('reads the PawnIO hash from the single pinned file and requires PowerShell 7', () => {
    const script = read(payloadScript);
    expect(script).toMatch(/^#Requires -Version 7$/m);
    expect(read(resolve(repoDir, 'scripts/check-trim-warnings.ps1'))).toMatch(/^#Requires -Version 7$/m);
    expect(script).toMatch(/pawnio\.sha256/);
    // No second copy of the hash in the script.
    expect(script).not.toMatch(/[0-9A-Fa-f]{64}/);
    expect(read(resolve(nsisDir, 'pawnio.sha256'))).toMatch(/^[0-9A-F]{64}\n?$/);
  });
});

// ---------------------------------------------------------------------------------------------
// Static checks of oma.nsh failure paths

/** Significant statements of a block: comments and blank lines dropped, trimmed. */
function statements(text: string): string[] {
  return text
    .split('\n')
    .map((l) => l.replace(/^\s*;.*$/, '').trim())
    .filter((l) => l !== '');
}

/** The statements between the first line matching `start` and the next line matching `end`. */
function block(stmts: string[], start: RegExp, end: RegExp): string[] {
  const from = stmts.findIndex((s) => start.test(s));
  expect(from, `block ${start}`).toBeGreaterThanOrEqual(0);
  const to = stmts.findIndex((s, i) => i > from && end.test(s));
  expect(to, `end of block ${start}`).toBeGreaterThan(from);
  return stmts.slice(from, to + 1);
}

/** Index of the first statement matching `re` at or after `from`, or -1. */
const indexOf = (stmts: string[], re: RegExp, from = 0) => stmts.findIndex((s, i) => i >= from && re.test(s));

const STOP = /^Call (un\.)?OmaStopService$/;
const STOP_CHECK = /^\$\{If\} \$OmaResult != "0"$/;
const HELPER = /^!insertmacro OMA_HELPER "(install|uninstall)"$/;
const HELPER_CHECK = /^\$\{If\} \$0 != "0"$/;
const FAIL = /^!insertmacro OMA_FAIL /;
const TOUCHES_SERVICE_FILES = /^(File|Delete|RMDir)\b.*(oma-service|OMA_SERVICE_EXE|\\service)/;

/** Every `call` is immediately followed by `check` and then by OMA_FAIL. */
function expectCheckedFailures(stmts: string[], call: RegExp, check: RegExp) {
  const calls = stmts.flatMap((s, i) => (call.test(s) ? [i] : []));
  expect(calls.length, `${call} is used`).toBeGreaterThan(0);
  for (const i of calls) {
    expect(stmts[i + 1], `check after "${stmts[i]}"`).toMatch(check);
    expect(stmts[i + 2], `failure after "${stmts[i]}"`).toMatch(FAIL);
  }
}

describe('oma.nsh failure paths', () => {
  const nsh = existsSync(resolve(nsisDir, 'oma.nsh')) ? read(resolve(nsisDir, 'oma.nsh')) : '';
  const all = statements(nsh);

  it('OMA_FAIL sets a non-zero exit code and aborts', () => {
    const fail = block(all, /^!macro OMA_FAIL /, /^!macroend$/);
    const level = fail.find((s) => /^SetErrorLevel /.test(s));
    expect(level).toBeDefined();
    expect(level).not.toMatch(/^SetErrorLevel (0|3010)$/);
    expect(indexOf(fail, /^Abort\b/)).toBeGreaterThan(fail.indexOf(level!));
  });

  it('checks every STOP and helper result before going on', () => {
    expectCheckedFailures(all, STOP, STOP_CHECK);
    expectCheckedFailures(all, HELPER, HELPER_CHECK);
  });

  it('the stop function reports a timeout or an SCM error as a failure', () => {
    const fn = block(all, /^Function \$\{un\}OmaStopService$/, /^FunctionEnd$/);
    expect(fn[indexOf(fn, /^StrCpy \$OmaResult "0"$/)]).toBeDefined();
    // Timeout, OpenSCManager and QueryServiceStatusEx failures set a non-"0" result;
    // only a missing service (1060) is success.
    const failure = /^StrCpy \$OmaResult "(?!0")/;
    const timeout = indexOf(fn, /^\$\{If\} \$4 >= \$\{OMA_STOP_TIMEOUT_TICKS\}$/);
    expect(fn[timeout + 1], 'timeout').toMatch(failure);
    const query = indexOf(fn, /QueryServiceStatusEx\(/);
    expect(fn.slice(query + 2, query + 4), 'query failure').toEqual(['${If} $3 = 0', expect.stringMatching(failure)]);
    expect(fn[indexOf(fn, /OpenSCManagerW\(/) + 3], 'SCM failure').toMatch(failure);
    expect(fn[indexOf(fn, /WaitForSingleObject\(/) + 2], 'process exit').toMatch(failure);
    expect(fn.slice(indexOf(fn, /OpenServiceW\(/) + 3, indexOf(fn, /OpenServiceW\(/) + 5)).toEqual([
      expect.stringMatching(/^\$\{If\} \$5 <> 1060\b/),
      expect.stringMatching(failure),
    ]);
  });

  it('the component stops the service before copying and records nothing itself', () => {
    const section = block(all, /^Section "\$\(omaSensorsSection\)" SecSensors$/, /^SectionEnd$/);
    const stop = indexOf(section, STOP);
    const firstFile = indexOf(section, TOUCHES_SERVICE_FILES);
    expect(stop).toBeGreaterThanOrEqual(0);
    expect(firstFile).toBeGreaterThan(stop + 2);
    // The exe copy is checked, then the helper runs.
    const copy = indexOf(section, /^File .*OMA_SERVICE_EXE/);
    expect(section[copy + 1]).toBe('${If} ${Errors}');
    expect(section[copy + 2]).toMatch(FAIL);
    expect(indexOf(section, HELPER)).toBeGreaterThan(copy);
    expect(section.some((s) => /^WriteReg/.test(s))).toBe(false);
  });

  it('PawnIO runs from the install dir, is deleted, and a bad exit code fails the install', () => {
    expect(nsh).not.toMatch(/PLUGINSDIR/);
    const section = block(all, /^Section "\$\(omaSensorsSection\)" SecSensors$/, /^SectionEnd$/);
    const setup = '"$INSTDIR\\service\\PawnIO_setup.exe"';
    const extract = indexOf(section, /^File "\/oname=\$INSTDIR\\service\\PawnIO_setup\.exe"/);
    const exec = indexOf(section, /^ExecWait /);
    const del = indexOf(section, /^Delete "\$INSTDIR\\service\\PawnIO_setup\.exe"$/, exec);
    expect(extract).toBeGreaterThanOrEqual(0);
    // A failed extraction is checked too (and cleans up) before the setup is run.
    expect(section[extract + 1]).toBe('${If} ${Errors}');
    expect(section.slice(extract + 2, extract + 4)).toEqual([
      'Delete "$INSTDIR\\service\\PawnIO_setup.exe"',
      expect.stringMatching(FAIL),
    ]);
    expect(section[exec]).toBe(`ExecWait '${setup} -install -silent' $3`);
    expect(del).toBeGreaterThan(exec);
    const after = section.slice(del);
    // 3010 = reboot needed; any other non-zero code, or a setup that could not start, fails.
    expect(after.join('\n')).toMatch(/\$\{ElseIf\} \$3 == "3010"\nSetRebootFlag true/);
    expect(after.join('\n')).toMatch(/\$\{ElseIf\} \$3 != "0"\n!insertmacro OMA_FAIL /);
    expect(after.join('\n')).toMatch(/\$\{If\} \$3 == "error"\n!insertmacro OMA_FAIL /);
  });

  it('records the choice only in the bookkeeping section, after the component or its removal', () => {
    const book = block(all, /^Section -OmaSensorsBookkeeping$/, /^SectionEnd$/);
    const writes = all.filter((s) => /^WriteReg/.test(s));
    expect(writes).toHaveLength(2);
    for (const w of writes) expect(book).toContain(w);
    // Deselected: stop, helper uninstall, then delete; the 0 is written after all of that.
    const stop = indexOf(book, STOP);
    const helper = indexOf(book, HELPER);
    const del = indexOf(book, /^Delete /);
    const zero = indexOf(book, /^WriteRegDWORD .* 0$/);
    expect(stop).toBeGreaterThan(0);
    expect(helper).toBeGreaterThan(stop);
    expect(del).toBeGreaterThan(helper);
    expect(zero).toBeGreaterThan(del);
  });

  it('the uninstall hook checks STOP and helper before deleting and leaves PawnIO alone', () => {
    const hook = block(all, /^!macro NSIS_HOOK_PREUNINSTALL$/, /^!macroend$/);
    const stop = indexOf(hook, STOP);
    const helper = indexOf(hook, HELPER);
    const del = indexOf(hook, TOUCHES_SERVICE_FILES);
    expect(stop).toBeGreaterThanOrEqual(0);
    expect(helper).toBeGreaterThan(stop);
    expect(del).toBeGreaterThan(helper + 2);
    expect(hook.join('\n')).not.toMatch(/pawnio/i);
  });

  it('locks $INSTDIR\\service down with icacls, by SID, before anything is written into it', () => {
    const section = block(all, /^Section "\$\(omaSensorsSection\)" SecSensors$/, /^SectionEnd$/);
    const stop = indexOf(section, STOP);
    const protect = indexOf(section, /^Call OmaProtectServiceDir$/);
    expect(protect).toBeGreaterThan(stop + 2);
    expect(section[protect + 1]).toMatch(STOP_CHECK);
    expect(section[protect + 2]).toMatch(FAIL);
    // Nothing is copied, extracted or deleted in the service dir before it is protected.
    expect(indexOf(section, /^(File|Delete|RMDir|ExecWait|nsExec)\b/)).toBeGreaterThan(protect + 2);
    expect(indexOf(section, /^File /)).toBeGreaterThan(protect + 2);

    const icacls = block(all, /^!macro OMA_ICACLS args$/, /^!macroend$/);
    expect(icacls).toContain(`nsExec::ExecToLog '"$SYSDIR\\icacls.exe" "$INSTDIR\\service" \${args}'`);
    const run = indexOf(icacls, /^nsExec::ExecToLog /);
    expect(icacls.slice(run + 1, run + 4)).toEqual([
      'Pop $0',
      '${If} $0 != "0"',
      expect.stringMatching(/^StrCpy \$OmaResult "(?!0")/),
    ]);

    const fn = block(all, /^Function OmaProtectServiceDir$/, /^FunctionEnd$/);
    const owner = indexOf(fn, /^!insertmacro OMA_ICACLS "\/setowner \*S-1-5-32-544"$/);
    const reset = indexOf(fn, /^!insertmacro OMA_ICACLS "\/reset"$/);
    const grant = indexOf(
      fn,
      /^!insertmacro OMA_ICACLS "\/inheritance:r \/grant:r \*S-1-5-18:\(OI\)\(CI\)F \*S-1-5-32-544:\(OI\)\(CI\)F \*S-1-5-32-545:\(OI\)\(CI\)RX"$/,
    );
    const reparse = indexOf(fn, /0x400/);
    expect(reparse).toBeGreaterThan(0);
    expect(owner).toBeGreaterThan(reparse);
    expect(reset).toBeGreaterThan(owner);
    expect(grant).toBeGreaterThan(reset);
    // Leftovers (a planted DLL, a file with its own ACL) are removed after the lock-down.
    expect(indexOf(fn, /^Delete "\$1\\\*\.\*"$/)).toBeGreaterThan(grant);
    // Locale-independent: SIDs only, never account names.
    expect(fn.filter((s) => /OMA_ICACLS/.test(s)).join('\n')).not.toMatch(/SYSTEM|Administrators|Users|Everyone/i);
  });

  it('checks the protection and orphan-removal results too', () => {
    expectCheckedFailures(all, /^Call OmaProtectServiceDir$/, STOP_CHECK);
    expectCheckedFailures(all, /^Call (un\.)?OmaDeleteService$/, STOP_CHECK);
    const fn = block(all, /^Function \$\{un\}OmaDeleteService$/, /^FunctionEnd$/);
    expect(fn.join('\n')).toMatch(/DeleteService\(/);
    expect(fn.join('\n')).toMatch(/1060/); // not installed = nothing to do
    expect(fn.join('\n')).toMatch(/1072/); // already marked for deletion = done
    expect(fn.filter((s) => /^StrCpy \$OmaResult "(?!0")/.test(s)).length).toBeGreaterThanOrEqual(3);
  });

  it('removes a service left registered without its exe (deselection and uninstall)', () => {
    const book = block(all, /^Section -OmaSensorsBookkeeping$/, /^SectionEnd$/);
    const hook = block(all, /^!macro NSIS_HOOK_PREUNINSTALL$/, /^!macroend$/);
    expect(indexOf(book, /^Call OmaDeleteService$/)).toBeGreaterThan(0);
    expect(indexOf(hook, /^Call un\.OmaDeleteService$/)).toBeGreaterThan(0);
  });

  it('the uninstall hook closes the app before touching the service', () => {
    const hook = block(all, /^!macro NSIS_HOOK_PREUNINSTALL$/, /^!macroend$/);
    expect(hook[1]).toBe('!insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"');
  });

  it('refuses to compile with a PawnIO setup that does not match the pinned hash', () => {
    const check = all.find((s) => /^!system /.test(s));
    expect(check).toBeDefined();
    // pwsh, not Windows PowerShell: launched from pwsh it cannot load Get-FileHash.
    expect(check).toMatch(/^!system `pwsh\.exe /);
    expect(check).toMatch(/Get-FileHash/);
    expect(check).toMatch(/\$\{OMA_PAYLOAD\}\\PawnIO_setup\.exe/);
    expect(check).toMatch(/\$\{__FILEDIR__\}\\pawnio\.sha256/);
    expect(check).toMatch(/ = 0$/);
  });

  it('requires $INSTDIR strictly inside $PROGRAMFILES64, by normalized path and trailing backslash', () => {
    const outer = block(all, /^Function OmaCheckInstallDir$/, /^FunctionEnd$/);
    expect(outer).toContain('StrCpy $OmaBase "$PROGRAMFILES64"');
    expect(outer).toContain('Call OmaCheckDirInside');
    const fn = block(all, /^Function OmaCheckDirInside$/, /^FunctionEnd$/);
    const base = indexOf(fn, /^System::Call 'kernel32::GetFullPathNameW\(w "\$OmaBase", /);
    const inst = indexOf(fn, /^System::Call 'kernel32::GetFullPathNameW\(w "\$INSTDIR", /);
    expect(base).toBeGreaterThan(0);
    expect(inst).toBeGreaterThan(base);
    // Both normalized, trailing backslash trimmed, then compared as "<base>\" prefix of "<dir>\",
    // so C:\Program FilesX never matches C:\Program Files and the root itself is refused.
    const withSlash = indexOf(fn, /^StrCpy \$0 "\$0\\"$/, inst);
    const len = indexOf(fn, /^StrLen \$2 \$0$/, withSlash);
    const prefix = indexOf(fn, /^StrCpy \$3 "\$1\\" \$2$/, len);
    const dirLen = indexOf(fn, /^StrLen \$4 "\$1\\"$/, prefix);
    expect(withSlash).toBeGreaterThan(inst);
    expect(len).toBe(withSlash + 1);
    expect(prefix).toBe(len + 1);
    expect(dirLen).toBe(prefix + 1);
    expect(fn.slice(dirLen + 1, dirLen + 4)).toEqual([
      '${If} $3 != $0',
      '${OrIf} $4 <= $2',
      expect.stringMatching(/^StrCpy \$OmaResult "(?!0")/),
    ]);
    expect(fn.filter((s) => s === '!insertmacro OMA_TRIM_BACKSLASH $0' || s === '!insertmacro OMA_TRIM_BACKSLASH $1')).toHaveLength(2);
  });

  it('refuses a junction or link anywhere from $PROGRAMFILES64 down to $INSTDIR\\service', () => {
    const fn = block(all, /^Function OmaCheckDirInside$/, /^FunctionEnd$/);
    expect(fn).toContain('StrCpy $5 "$1\\service"');
    const walk = indexOf(fn, /^\$\{Do\}$/);
    expect(walk).toBeGreaterThan(indexOf(fn, /^StrCpy \$5 /));
    // The walk starts at the backslash right after the base, i.e. with the base itself.
    expect(fn[walk - 1]).toBe('IntOp $3 $2 - 1');
    const body = fn.slice(walk);
    const attrs = indexOf(body, /^Call OmaPathAttributes$/);
    expect(attrs).toBeGreaterThan(0);
    // An error stops with a failure, "absent" ends the walk (nothing deeper exists), anything
    // else is checked for a reparse point before going one level deeper.
    expect(body.slice(attrs + 1, attrs + 11)).toEqual([
      '${If} $OmaResult != "0"',
      '${Break}',
      '${EndIf}',
      '${If} $OmaAttr == "absent"',
      '${Break}',
      '${EndIf}',
      'IntOp $6 $OmaAttr & 0x400',
      '${If} $6 <> 0',
      expect.stringMatching(/^StrCpy \$OmaResult "(?!0")/),
      '${Break}',
    ]);
  });

  it('treats a path as absent only for FILE_NOT_FOUND or PATH_NOT_FOUND', () => {
    const fn = block(all, /^Function OmaPathAttributes$/, /^FunctionEnd$/);
    expect(fn.join('\n')).toMatch(
      /\$\{If\} \$0 = -1\n\$\{If\} \$1 = 2\n\$\{OrIf\} \$1 = 3\nStrCpy \$OmaAttr "absent"\n\$\{Else\}\nStrCpy \$OmaAttr "error"\nStrCpy \$OmaResult "(?!0")/,
    );
    // The service folder lock-down goes through it too, not through a bare GetFileAttributesW.
    const protect = block(all, /^Function OmaProtectServiceDir$/, /^FunctionEnd$/);
    expect(protect.join('\n')).not.toMatch(/GetFileAttributesW/);
    const call = indexOf(protect, /^Call OmaPathAttributes$/);
    expect(protect.slice(call + 1, call + 3)).toEqual(['${If} $OmaResult != "0"', expect.stringMatching(/^Goto |^\$\{/)]);
  });

  it('checks the install folder before the app files, again before the sensors, and on the directory page', () => {
    // Before the template copies anything (silent: exit 2 via OMA_FAIL unless /NOSENSORS unselected it).
    const pre = block(all, /^!macro NSIS_HOOK_PREINSTALL$/, /^!macroend$/);
    expect(pre.slice(1, 4)).toEqual([
      'Call OmaCheckSensorsInstallDir',
      '${If} $OmaResult != "0"',
      expect.stringMatching(/^!insertmacro OMA_FAIL "\$\(omaSensorsNeedProgramFiles\)" /),
    ]);
    const only = block(all, /^Function OmaCheckSensorsInstallDir$/, /^FunctionEnd$/);
    expect(only).toEqual([
      'Function OmaCheckSensorsInstallDir',
      'StrCpy $OmaResult "0"',
      '${If} ${SectionIsSelected} ${SecSensors}',
      'Call OmaCheckInstallDir',
      '${EndIf}',
      'FunctionEnd',
    ]);
    // /NOSENSORS unselects the section, so the check does not apply then.
    const init = block(all, /^Function OmaInitComponents$/, /^FunctionEnd$/);
    expect(init.join('\n')).toMatch(/\$\{GetOptions\} \$CMDLINE "\/NOSENSORS" \$0\n\$\{IfNot\} \$\{Errors\}\n!insertmacro UnselectSection \$\{SecSensors\}/);

    // First thing in the sensors section, before STOP and any File/ExecWait/nsExec.
    const section = block(all, /^Section "\$\(omaSensorsSection\)" SecSensors$/, /^SectionEnd$/);
    expect(section.slice(1, 4)).toEqual([
      'Call OmaCheckInstallDir',
      '${If} $OmaResult != "0"',
      expect.stringMatching(/^!insertmacro OMA_FAIL "\$\(omaSensorsNeedProgramFiles\)" /),
    ]);
    expect(indexOf(section, /^(File|ExecWait|nsExec|Call OmaStopService)\b/)).toBeGreaterThan(3);

    // Directory page: stay on the page with a message.
    const leave = block(all, /^Function OmaDirectoryLeave$/, /^FunctionEnd$/);
    expect(leave.slice(1)).toEqual([
      'Call OmaCheckSensorsInstallDir',
      '${If} $OmaResult != "0"',
      'StrCpy $OmaDetail $OmaResult',
      'MessageBox MB_ICONEXCLAMATION|MB_OK "$(omaSensorsNeedProgramFiles)"',
      'Abort',
      '${EndIf}',
      'FunctionEnd',
    ]);
    const tpl = read(resolve(nsisDir, 'installer.nsi')).split('\n');
    const leaveAt = tpl.indexOf('!define MUI_PAGE_CUSTOMFUNCTION_LEAVE OmaDirectoryLeave ; OMA');
    expect(leaveAt).toBeGreaterThan(0);
    expect(tpl[leaveAt + 1]).toBe('!insertmacro MUI_PAGE_DIRECTORY');
    expect(nsh.match(/LangString omaSensorsNeedProgramFiles \$\{LANG_(ENGLISH|ITALIAN)\} /g)).toHaveLength(2);
  });

  it('keeps the logs in the service folder, with no ProgramData folder to create first (ruling R30)', () => {
    expect(nsh).not.toMatch(/ProgramData|\$APPDATA|SetShellVarContext|OmaProtectLogDir|OmaLockLogDir|OmaCleanLogDir/);
    const section = block(all, /^Section "\$\(omaSensorsSection\)" SecSensors$/, /^SectionEnd$/);
    expect(section.join('\n')).not.toMatch(/\blogs\b/);
  });

  it('keeps a real logs folder on an upgrade and removes a link in its place, never following it', () => {
    const fn = block(all, /^Function OmaProtectServiceDir$/, /^FunctionEnd$/);
    const text = fn.join('\n');
    expect(text).not.toMatch(/RMDir \/r/i);
    const grant = indexOf(fn, /^!insertmacro OMA_ICACLS "\/inheritance:r \/grant:r /);
    const logs = indexOf(fn, /^StrCpy \$OmaPath "\$1\\logs"$/);
    const empty = indexOf(fn, /^Delete "\$1\\\*\.\*"$/);
    expect(logs).toBeGreaterThan(grant);
    expect(empty).toBeGreaterThan(logs);
    expect(fn[logs + 1]).toBe('Call OmaPathAttributes');
    // A junction or link: removed as a link (RMDir without /r, or Delete), then checked gone.
    expect(text).toMatch(
      /IntOp \$2 \$OmaAttr & 0x400\n\$\{If\} \$2 <> 0\nIntOp \$2 \$OmaAttr & 0x10\n\$\{If\} \$2 <> 0\nRMDir "\$1\\logs"\n\$\{Else\}\nDelete "\$1\\logs"\n\$\{EndIf\}\nCall OmaPathAttributes\n\$\{If\} \$OmaResult == "0"\n\$\{AndIf\} \$OmaAttr != "absent"\nStrCpy \$OmaResult "(?!0")/,
    );
    // A real folder is kept with its files and reset to inherit the protected (OI)(CI) ACL just
    // granted to the service folder, never through a link (/L).
    expect(text).toMatch(/\$\{Else\}\nIntOp \$2 \$OmaAttr & 0x10\n\$\{If\} \$2 <> 0\n!insertmacro OMA_ICACLS_PATH "\$1\\logs" "\/reset \/T"\n\$\{EndIf\}/);
    const icacls = block(all, /^!macro OMA_ICACLS_PATH path args$/, /^!macroend$/);
    const run = indexOf(icacls, /^nsExec::ExecToLog /);
    expect(icacls[run]).toBe(`nsExec::ExecToLog '"$SYSDIR\\icacls.exe" "\${path}" \${args} /L'`);
    expect(icacls.slice(run + 1, run + 4)).toEqual([
      'Pop $0',
      '${If} $0 != "0"',
      expect.stringMatching(/^StrCpy \$OmaResult "(?!0")/),
    ]);
    // The emptiness check lets logs (by then a real folder) stay, and nothing else.
    expect(text).toMatch(/\$\{If\} \$2 != "\."\n\$\{AndIf\} \$2 != "\.\."\n\$\{AndIf\} \$2 != "logs"\nStrCpy \$OmaResult "(?!0")/);
  });

  it('removes the service logs on uninstall and on deselection, but not on an upgrade', () => {
    const fn = block(all, /^Function \$\{un\}OmaRemoveServiceLogs$/, /^FunctionEnd$/);
    // A link is removed as a link; RMDir /r only on a real folder.
    expect(fn.join('\n')).toMatch(
      /System::Call 'kernel32::GetFileAttributesW\(w "\$INSTDIR\\service\\logs"\) i\.r0'\n\$\{If\} \$0 <> -1\nIntOp \$0 \$0 & 0x400\n\$\{If\} \$0 <> 0\nRMDir "\$INSTDIR\\service\\logs"\n\$\{Else\}\nRMDir \/r "\$INSTDIR\\service\\logs"\n\$\{EndIf\}/,
    );

    const hook = block(all, /^!macro NSIS_HOOK_PREUNINSTALL$/, /^!macroend$/);
    const helper = indexOf(hook, HELPER);
    const exe = indexOf(hook, /^Delete "\$INSTDIR\\service\\\$\{OMA_SERVICE_EXE\}"$/);
    const remove = indexOf(hook, /^Call un\.OmaRemoveServiceLogs$/);
    const rmdir = indexOf(hook, /^RMDir "\$INSTDIR\\service"$/);
    expect(remove).toBeGreaterThan(helper);
    expect(remove).toBeGreaterThan(exe);
    expect(rmdir).toBeGreaterThan(remove);
    expect(hook[remove - 1]).toBe('${If} $UpdateMode <> 1');

    const book = block(all, /^Section -OmaSensorsBookkeeping$/, /^SectionEnd$/);
    const calls = book.flatMap((s, i) => (s === 'Call OmaRemoveServiceLogs' ? [i] : []));
    expect(calls).toHaveLength(2); // with the exe, and for an orphaned service
    expect(calls[0]).toBeGreaterThan(indexOf(book, HELPER));
    for (const at of calls) expect(book[at + 1]).toBe('RMDir "$INSTDIR\\service"');
  });

  it('turns the reboot flag into exit code 3010 only on success', () => {
    const lines = all.filter((s) => /SetErrorLevel 3010/.test(s));
    expect(lines).toHaveLength(1);
    const hook = block(all, /^!macro OMA_ONINSTSUCCESS$/, /^!macroend$/);
    expect(hook).toEqual(['!macro OMA_ONINSTSUCCESS', '${If} ${RebootFlag}', 'SetErrorLevel 3010', '${EndIf}', '!macroend']);
  });
});
