; OpenMonitor Advanced additions to the Tauri NSIS template (spec §10).
;
; Included by installer.nsi through `bundle.windows.nsis.installerHooks`, i.e.
; before any !define of the template, so only macros, variables, functions that
; need no template define and path defines live at top level. `${__FILEDIR__}` is
; this file's directory (app/src-tauri/nsis), which is how we locate the payload
; no matter where CARGO_TARGET_DIR points.
;
; Failure policy: a failed STOP, helper verb or PawnIO setup ends the (un)installer
; through OMA_FAIL (non-zero exit code, Abort). Abort skips every later section and
; .onInstSuccess, so nothing is deleted or recorded after a failure and the reboot
; exit code 3010 can never hide it. Checked statically by
; app/src/test/nsis-template.test.ts; fault injection happens in a VM (Task 15).

!include LogicLib.nsh
!include Sections.nsh
!include WordFunc.nsh
!include FileFunc.nsh

!define OMA_PAYLOAD "${__FILEDIR__}\..\..\..\target\installer-payload"
!define OMA_SERVICE_NAME "oma-service"
!define OMA_SERVICE_EXE "oma-service.exe"
!define OMA_REGKEY "Software\OpenMonitorAdvanced\Installer"
!define OMA_REGVALUE "AdvancedSensors"
!define OMA_PAWNIO_MIN "2.2.0"
!define OMA_PAWNIO_KEY "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO"
!define OMA_STOP_TIMEOUT_TICKS 60 ; x 500 ms = 30 s
!define OMA_EXIT_TIMEOUT_MS 10000 ; after SERVICE_STOPPED, for the process to exit
!define OMA_FAILED_EXIT_CODE 2 ; NSIS "aborted by script"

!if /FileExists "${OMA_PAYLOAD}\service\${OMA_SERVICE_EXE}"
!else
  !error "Missing ${OMA_PAYLOAD}\service\${OMA_SERVICE_EXE}: run scripts/build-installer-payload.ps1 first"
!endif
!if /FileExists "${OMA_PAYLOAD}\PawnIO_setup.exe"
!else
  !error "Missing ${OMA_PAYLOAD}\PawnIO_setup.exe: run scripts/build-installer-payload.ps1 first"
!endif
; Never embed a PawnIO setup that the payload script did not verify: compare it with the
; pinned hash (pawnio.sha256, the same file the payload script checks against). A mismatch
; stops the compilation. pwsh (PowerShell 7), which the payload script requires anyway:
; Windows PowerShell started from pwsh inherits its PSModulePath and cannot load Get-FileHash.
!system `pwsh.exe -NoProfile -NonInteractive -Command "if ((Get-FileHash -Algorithm SHA256 -LiteralPath '${OMA_PAYLOAD}\PawnIO_setup.exe').Hash -ne (Get-Content -Raw -LiteralPath '${__FILEDIR__}\pawnio.sha256').Trim()) { Write-Host 'PawnIO_setup.exe does not match pawnio.sha256: run scripts/build-installer-payload.ps1'; exit 1 }"` = 0

; "0" on success, otherwise a short English description (OmaStopService).
Var OmaResult
; What failed, shown inside the omaSensorsFailed / omaServiceRemoveFailed texts.
Var OmaDetail
; OmaPathAttributes: in = path, out = attributes or "absent".
Var OmaPath
Var OmaAttr
; OmaCheckDirInside: the directory $INSTDIR must be strictly inside.
Var OmaBase

; Strings for our section. Inserted after the MUI_LANGUAGE macros.
!macro OMA_LANGSTRINGS
  LangString omaSensorsSection ${LANG_ENGLISH} "Advanced sensors"
  LangString omaSensorsDesc ${LANG_ENGLISH} "Windows service and PawnIO driver for CPU temperatures, voltages, fans and SMART data. Requires administrator rights only now, during setup."
  LangString omaSensorsFailed ${LANG_ENGLISH} "Advanced sensors could not be set up ($OmaDetail).$\r$\n$\r$\nSetup stopped before finishing. Run it again, or clear the Advanced sensors option to install without them."
  LangString omaSensorsNeedProgramFiles ${LANG_ENGLISH} "Advanced sensors can only be installed inside $PROGRAMFILES64, which only administrators can change ($OmaDetail).$\r$\n$\r$\nChoose a folder there, or go back and clear the Advanced sensors option (/NOSENSORS when silent)."
  LangString omaServiceRemoveFailed ${LANG_ENGLISH} "The Advanced sensors service could not be removed ($OmaDetail).$\r$\n$\r$\nNo files were deleted. Close OpenMonitor Advanced and try again."
  !ifdef LANG_ITALIAN
    LangString omaSensorsSection ${LANG_ITALIAN} "Sensori avanzati"
    LangString omaSensorsDesc ${LANG_ITALIAN} "Servizio Windows e driver PawnIO per temperature, tensioni e ventole della CPU e dati SMART. Servono i privilegi di amministratore solo ora, durante l'installazione."
    LangString omaSensorsFailed ${LANG_ITALIAN} "Non è stato possibile configurare i sensori avanzati ($OmaDetail).$\r$\n$\r$\nL'installazione si è fermata prima della fine. Riprova, oppure togli l'opzione Sensori avanzati per installare senza."
    LangString omaSensorsNeedProgramFiles ${LANG_ITALIAN} "I sensori avanzati si possono installare solo dentro $PROGRAMFILES64, che solo gli amministratori possono modificare ($OmaDetail).$\r$\n$\r$\nScegli una cartella lì, oppure torna indietro e togli l'opzione Sensori avanzati (/NOSENSORS in modalità silenziosa)."
    LangString omaServiceRemoveFailed ${LANG_ITALIAN} "Non è stato possibile rimuovere il servizio dei sensori avanzati ($OmaDetail).$\r$\n$\r$\nNessun file è stato eliminato. Chiudi OpenMonitor Advanced e riprova."
  !endif
!macroend

; Ends the (un)installer as failed: logs the detail, tells the user (OK when
; silent), sets a non-zero exit code and aborts. After an Abort no later section
; and no .onInstSuccess runs.
!macro OMA_FAIL message detail
  StrCpy $OmaDetail "${detail}"
  DetailPrint "OpenMonitor Advanced: $OmaDetail"
  MessageBox MB_ICONSTOP|MB_OK "${message}" /SD IDOK
  SetErrorLevel ${OMA_FAILED_EXIT_CODE}
  Abort "$OmaDetail"
!macroend

; Stops oma-service if it exists, waits until SERVICE_STOPPED and then until its
; process has exited (so the exe is no longer locked), using the SCM API
; directly: no child process, no dependency on the previously installed helper,
; no System32/SysWOW64 redirection issue (the SCM is not bitness-bound).
; Sets $OmaResult to "0" on success, also when the service does not exist;
; otherwise to a description (SCM error, timeout).
!macro OMA_STOP_SERVICE un
Function ${un}OmaStopService
  Push $0 ; SCM handle
  Push $1 ; service handle
  Push $2 ; SERVICE_STATUS_PROCESS buffer
  Push $3 ; call result / dwCurrentState
  Push $4 ; ticks
  Push $5 ; last error
  Push $6 ; process id
  Push $7 ; process handle
  Push $8 ; bytes needed
  StrCpy $OmaResult "0"
  StrCpy $7 0
  ; SC_MANAGER_CONNECT
  System::Call 'advapi32::OpenSCManagerW(p 0, p 0, i 0x0001) p.r0 ?e'
  Pop $5
  ${If} $0 P= 0
    StrCpy $OmaResult "OpenSCManager failed, error $5"
  ${Else}
    ; SERVICE_STOP | SERVICE_QUERY_STATUS
    System::Call 'advapi32::OpenServiceW(p r0, w "${OMA_SERVICE_NAME}", i 0x0024) p.r1 ?e'
    Pop $5
    ${If} $1 P= 0
      ${If} $5 <> 1060 ; ERROR_SERVICE_DOES_NOT_EXIST: nothing to stop
        StrCpy $OmaResult "OpenService failed, error $5"
      ${EndIf}
    ${Else}
      DetailPrint "Stopping ${OMA_SERVICE_NAME}..."
      System::Call '*(i, i, i, i, i, i, i, i, i) p.r2' ; SERVICE_STATUS_PROCESS, 36 bytes
      StrCpy $4 0
      ${Do}
        ; SC_STATUS_PROCESS_INFO
        System::Call 'advapi32::QueryServiceStatusEx(p r1, i 0, p r2, i 36, *i .r8) i.r3 ?e'
        Pop $5
        ${If} $3 = 0
          StrCpy $OmaResult "QueryServiceStatusEx failed, error $5"
          ${Break}
        ${EndIf}
        System::Call '*$2(i, i .r3, i, i, i, i, i, i .r6)' ; dwCurrentState, dwProcessId
        ; Hold the process open from the first sighting: waiting on it later can
        ; then never hit a reused process id.
        ${If} $7 = 0
        ${AndIf} $6 <> 0
          System::Call 'kernel32::OpenProcess(i 0x00100000, i 0, i r6) p.r7' ; SYNCHRONIZE
        ${EndIf}
        ${If} $3 = 1 ; SERVICE_STOPPED
          ${Break}
        ${EndIf}
        ${If} $4 >= ${OMA_STOP_TIMEOUT_TICKS}
          StrCpy $OmaResult "${OMA_SERVICE_NAME} did not stop within 30 s, state $3"
          ${Break}
        ${EndIf}
        ${If} $3 <> 3 ; not STOP_PENDING yet: (re)send STOP. 1061 while starting is retried on the next tick.
          System::Call 'advapi32::ControlService(p r1, i 1, p r2) i'
        ${EndIf}
        Sleep 500
        IntOp $4 $4 + 1
      ${Loop}
      System::Free $2
      System::Call 'advapi32::CloseServiceHandle(p r1)'
      ${If} $7 P<> 0
        ${If} $OmaResult == "0"
          System::Call 'kernel32::WaitForSingleObject(p r7, i ${OMA_EXIT_TIMEOUT_MS}) i.r3'
          ${If} $3 <> 0 ; WAIT_OBJECT_0
            StrCpy $OmaResult "the ${OMA_SERVICE_NAME} process did not exit, wait result $3"
          ${EndIf}
        ${EndIf}
        System::Call 'kernel32::CloseHandle(p r7)'
      ${EndIf}
    ${EndIf}
    System::Call 'advapi32::CloseServiceHandle(p r0)'
  ${EndIf}
  ${If} $OmaResult != "0"
    DetailPrint "$OmaResult"
  ${EndIf}
  Pop $8
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd
!macroend
!insertmacro OMA_STOP_SERVICE ""
!insertmacro OMA_STOP_SERVICE "un."

; Deletes the oma-service registration through the SCM, without running any exe.
; Used when the service may still be registered but oma-service.exe is gone, so
; the helper cannot run. Call OmaStopService first. Sets $OmaResult like
; OmaStopService: "0" also when the service does not exist or is already marked
; for deletion.
!macro OMA_DELETE_SERVICE un
Function ${un}OmaDeleteService
  Push $0 ; SCM handle
  Push $1 ; service handle
  Push $2 ; call result
  Push $5 ; last error
  StrCpy $OmaResult "0"
  System::Call 'advapi32::OpenSCManagerW(p 0, p 0, i 0x0001) p.r0 ?e' ; SC_MANAGER_CONNECT
  Pop $5
  ${If} $0 P= 0
    StrCpy $OmaResult "OpenSCManager failed, error $5"
  ${Else}
    System::Call 'advapi32::OpenServiceW(p r0, w "${OMA_SERVICE_NAME}", i 0x00010000) p.r1 ?e' ; DELETE
    Pop $5
    ${If} $1 P= 0
      ${If} $5 <> 1060 ; ERROR_SERVICE_DOES_NOT_EXIST: nothing to delete
        StrCpy $OmaResult "OpenService for delete failed, error $5"
      ${EndIf}
    ${Else}
      DetailPrint "${OMA_SERVICE_NAME} is registered without ${OMA_SERVICE_EXE}: removing the orphaned service"
      System::Call 'advapi32::DeleteService(p r1) i.r2 ?e'
      Pop $5
      ${If} $2 = 0
      ${AndIf} $5 <> 1072 ; ERROR_SERVICE_MARKED_FOR_DELETE: already going away
        StrCpy $OmaResult "DeleteService failed, error $5"
      ${EndIf}
      System::Call 'advapi32::CloseServiceHandle(p r1)'
    ${EndIf}
    System::Call 'advapi32::CloseServiceHandle(p r0)'
  ${EndIf}
  ${If} $OmaResult != "0"
    DetailPrint "$OmaResult"
  ${EndIf}
  Pop $5
  Pop $2
  Pop $1
  Pop $0
FunctionEnd
!macroend
!insertmacro OMA_DELETE_SERVICE ""
!insertmacro OMA_DELETE_SERVICE "un."

; Removes $INSTDIR\service\logs, the service's logs (ruling R30: they go with
; the service on uninstall or deselection; an upgrade keeps them). A junction or
; link in its place is removed as a link, never followed; RMDir /r only runs on
; a real folder, which only SYSTEM and Administrators can fill.
!macro OMA_REMOVE_SERVICE_LOGS un
Function ${un}OmaRemoveServiceLogs
  Push $0
  System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR\service\logs") i.r0'
  ${If} $0 <> -1
    IntOp $0 $0 & 0x400
    ${If} $0 <> 0
      RMDir "$INSTDIR\service\logs"
    ${Else}
      RMDir /r "$INSTDIR\service\logs"
    ${EndIf}
  ${EndIf}
  Pop $0
FunctionEnd
!macroend
!insertmacro OMA_REMOVE_SERVICE_LOGS ""
!insertmacro OMA_REMOVE_SERVICE_LOGS "un."

; One icacls call on $INSTDIR\service, skipped once $OmaResult holds a failure.
; Exit code compared as a string ("error" if icacls could not start).
!macro OMA_ICACLS args
  ${If} $OmaResult == "0"
    nsExec::ExecToLog '"$SYSDIR\icacls.exe" "$INSTDIR\service" ${args}'
    Pop $0
    ${If} $0 != "0"
      StrCpy $OmaResult "icacls ${args} exited with $0"
    ${EndIf}
  ${EndIf}
!macroend

; One icacls call on a path inside the service folder, skipped once $OmaResult
; holds a failure. /L: on the entry itself, never on a link's target. Exit code
; compared as a string ("error" if icacls could not start).
!macro OMA_ICACLS_PATH path args
  ${If} $OmaResult == "0"
    nsExec::ExecToLog '"$SYSDIR\icacls.exe" "${path}" ${args} /L'
    Pop $0
    ${If} $0 != "0"
      StrCpy $OmaResult "icacls ${path} ${args} exited with $0"
    ${EndIf}
  ${EndIf}
!macroend

; Attributes of $OmaPath into $OmaAttr, or "absent". INVALID_FILE_ATTRIBUTES
; means "absent" only for ERROR_FILE_NOT_FOUND (2) and ERROR_PATH_NOT_FOUND (3);
; any other error (access denied, bad name...) sets $OmaResult.
Function OmaPathAttributes
  Push $0
  Push $1
  System::Call 'kernel32::GetFileAttributesW(w "$OmaPath") i.r0 ?e'
  Pop $1
  ${If} $0 = -1
    ${If} $1 = 2
    ${OrIf} $1 = 3
      StrCpy $OmaAttr "absent"
    ${Else}
      StrCpy $OmaAttr "error"
      StrCpy $OmaResult "cannot read the attributes of $OmaPath, error $1"
    ${EndIf}
  ${Else}
    StrCpy $OmaAttr $0
  ${EndIf}
  Pop $1
  Pop $0
FunctionEnd

; Drops one trailing backslash from a variable (scratch: $6).
!macro OMA_TRIM_BACKSLASH var
  StrCpy $6 ${var} "" -1
  ${If} $6 == "\"
    StrCpy ${var} ${var} -1
  ${EndIf}
!macroend

; Controller ruling R25 (spec §9): with Advanced sensors, $INSTDIR must be inside
; $PROGRAMFILES64, which only administrators can write. A custom folder under a
; user-writable parent (C:\Apps, D:\) would let a user rename or replace folders
; around the LocalSystem service binary, the elevated PawnIO setup and
; uninstall.exe, and no ACL on a child can fix a parent that grants delete-child.
; Sets $OmaResult to "0" or to the reason.
Function OmaCheckInstallDir
  StrCpy $OmaBase "$PROGRAMFILES64"
  Call OmaCheckDirInside
FunctionEnd

; $INSTDIR must be strictly inside $OmaBase, both as normalized full paths
; (GetFullPathNameW: "..", "/" and relative parts resolved), compared without
; case as "<base>\" against the start of "<dir>\", so a sibling such as
; "C:\Program FilesX" never matches and the base itself is refused. Then every
; directory that already exists from $OmaBase down to $INSTDIR\service must not
; be a reparse point (junction, symlink, mount point). 8.3 short names are not
; expanded: such a path is refused, which is safe. Sets $OmaResult.
Function OmaCheckDirInside
  Push $0 ; base
  Push $1 ; dir
  Push $2 ; call result, then length of "<base>\"
  Push $3 ; prefix of "<dir>\", then walk index
  Push $4 ; length of "<dir>\", then length of the walked path
  Push $5 ; walked path
  Push $6 ; scratch
  StrCpy $OmaResult "0"
  System::Call 'kernel32::GetFullPathNameW(w "$OmaBase", i ${NSIS_MAX_STRLEN}, w .r0, p 0) i.r2'
  ${If} $2 = 0
  ${OrIf} $2 >= ${NSIS_MAX_STRLEN}
    StrCpy $OmaResult "cannot resolve $OmaBase"
  ${EndIf}
  System::Call 'kernel32::GetFullPathNameW(w "$INSTDIR", i ${NSIS_MAX_STRLEN}, w .r1, p 0) i.r2'
  ${If} $2 = 0
  ${OrIf} $2 >= ${NSIS_MAX_STRLEN}
    StrCpy $OmaResult "cannot resolve $INSTDIR"
  ${EndIf}
  ${If} $OmaResult == "0"
    !insertmacro OMA_TRIM_BACKSLASH $0
    !insertmacro OMA_TRIM_BACKSLASH $1
    StrCpy $0 "$0\"
    StrLen $2 $0
    StrCpy $3 "$1\" $2
    StrLen $4 "$1\"
    ${If} $3 != $0
    ${OrIf} $4 <= $2
      StrCpy $OmaResult "$1 is not inside $0"
    ${EndIf}
  ${EndIf}
  ${If} $OmaResult == "0"
    ; Walk "<dir>\service" from the backslash after the base: check the base,
    ; then each deeper folder, until one does not exist yet.
    StrCpy $5 "$1\service"
    StrLen $4 $5
    IntOp $3 $2 - 1
    ${Do}
      ${If} $3 < $4
        StrCpy $6 $5 1 $3
        ${If} $6 != "\"
          IntOp $3 $3 + 1
          ${Continue}
        ${EndIf}
        StrCpy $OmaPath $5 $3
      ${Else}
        StrCpy $OmaPath $5
      ${EndIf}
      Call OmaPathAttributes
      ${If} $OmaResult != "0"
        ${Break}
      ${EndIf}
      ${If} $OmaAttr == "absent"
        ${Break}
      ${EndIf}
      ; FILE_ATTRIBUTE_REPARSE_POINT
      IntOp $6 $OmaAttr & 0x400
      ${If} $6 <> 0
        StrCpy $OmaResult "$OmaPath is a junction or link"
        ${Break}
      ${EndIf}
      ${If} $3 >= $4
        ${Break}
      ${EndIf}
      IntOp $3 $3 + 1
    ${Loop}
  ${EndIf}
  ${If} $OmaResult != "0"
    DetailPrint "$OmaResult"
  ${EndIf}
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; Tauri hook: first thing in the template's Install section, before any app file
; is copied. With Advanced sensors selected, a folder outside Program Files stops
; the install here (exit code 2 when silent; /NOSENSORS unselects the section).
!macro NSIS_HOOK_PREINSTALL
  Call OmaCheckSensorsInstallDir
  ${If} $OmaResult != "0"
    !insertmacro OMA_FAIL "$(omaSensorsNeedProgramFiles)" "$OmaResult"
  ${EndIf}
!macroend

; Makes $INSTDIR\service a directory that only SYSTEM and Administrators can
; write. Defence in depth: OmaCheckInstallDir already requires Program Files,
; where no user can race us; this also covers a folder left behind with a
; different ACL. The service folder holds a LocalSystem service binary and the
; PawnIO setup, both run elevated. SIDs, not names, so it works in every language:
;   owner Administrators; explicit ACEs dropped (/reset); inheritance removed and
;   exactly SYSTEM (S-1-5-18) F, Administrators (S-1-5-32-544) F,
;   Users (S-1-5-32-545) RX granted, inherited by files and subfolders.
; Then empties the directory, so no file planted before the lock-down (a DLL
; next to the exe, an exe with its own ACL) survives: everything written
; afterwards inherits the protected ACL. The one exception is the service's own
; logs folder (ruling R30: the logs live here, where no user can create a folder
; first): on an upgrade a real logs folder is kept with its files and reset to
; inherit the ACL just granted, while a junction or link in its place is removed
; as a link, never followed. Called after the service is stopped.
; Sets $OmaResult to "0" or to a description of the failure.
Function OmaProtectServiceDir
  Push $0
  Push $1
  Push $2
  StrCpy $OmaResult "0"
  StrCpy $1 "$INSTDIR\service"
  StrCpy $OmaPath $1
  Call OmaPathAttributes
  ${If} $OmaResult != "0"
    ; attributes unreadable for another reason than "not found": $OmaResult says why
  ${ElseIf} $OmaAttr == "absent"
    ClearErrors
    CreateDirectory "$1"
    ${If} ${Errors}
      StrCpy $OmaResult "cannot create $1"
    ${EndIf}
  ${Else}
    IntOp $2 $OmaAttr & 0x400 ; FILE_ATTRIBUTE_REPARSE_POINT: never follow a planted junction or link
    ${If} $2 <> 0
      StrCpy $OmaResult "$1 is a junction or link"
    ${EndIf}
    IntOp $2 $OmaAttr & 0x10 ; FILE_ATTRIBUTE_DIRECTORY
    ${If} $2 = 0
      StrCpy $OmaResult "$1 is not a directory"
    ${EndIf}
  ${EndIf}
  !insertmacro OMA_ICACLS "/setowner *S-1-5-32-544"
  !insertmacro OMA_ICACLS "/reset"
  !insertmacro OMA_ICACLS "/inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX"
  ${If} $OmaResult == "0"
    StrCpy $OmaPath "$1\logs"
    Call OmaPathAttributes
    ${If} $OmaResult == "0"
    ${AndIf} $OmaAttr != "absent"
      IntOp $2 $OmaAttr & 0x400
      ${If} $2 <> 0
        IntOp $2 $OmaAttr & 0x10
        ${If} $2 <> 0
          RMDir "$1\logs"
        ${Else}
          Delete "$1\logs"
        ${EndIf}
        Call OmaPathAttributes
        ${If} $OmaResult == "0"
        ${AndIf} $OmaAttr != "absent"
          StrCpy $OmaResult "cannot remove the junction or link $1\logs"
        ${EndIf}
      ${Else}
        IntOp $2 $OmaAttr & 0x10
        ${If} $2 <> 0
          !insertmacro OMA_ICACLS_PATH "$1\logs" "/reset /T"
        ${EndIf}
      ${EndIf}
    ${EndIf}
  ${EndIf}
  ${If} $OmaResult == "0"
    Delete "$1\*.*"
    FindFirst $0 $2 "$1\*.*"
    ${DoWhile} $2 != ""
      ${If} $2 != "."
      ${AndIf} $2 != ".."
      ${AndIf} $2 != "logs"
        StrCpy $OmaResult "$1 still contains $2"
        ${Break}
      ${EndIf}
      FindNext $0 $2
    ${Loop}
    FindClose $0
  ${EndIf}
  ${If} $OmaResult != "0"
    DetailPrint "$OmaResult"
  ${EndIf}
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; Runs the service helper verb ($INSTDIR\service\oma-service.exe <verb>),
; hidden console, output in the details log. Exit code left in $0, or the
; string "error" if the process could not be started: compare with != "0",
; never with <> 0 (IntCmp would read "error" as 0).
!macro OMA_HELPER verb
  nsExec::ExecToLog '"$INSTDIR\service\${OMA_SERVICE_EXE}" ${verb}'
  Pop $0
!macroend

; Called from .onInit after SetContext (64-bit registry view).
; Initial state = stored choice if any, otherwise checked; /NOSENSORS forces off.
!macro OMA_ONINIT
  Call OmaInitComponents
!macroend

; Called first thing in .onInstSuccess, which only runs when no section aborted:
; the reboot exit code never replaces a failure code. Lets winget and other
; deployment tools see that PawnIO needs a restart in /S and /P mode too.
!macro OMA_ONINSTSUCCESS
  ${If} ${RebootFlag}
    SetErrorLevel 3010
  ${EndIf}
!macroend

; Declares the optional section, the hidden bookkeeping section and the init
; function. Inserted after the template's (hidden) Install section so that the
; app has already been closed by CheckIfAppIsRunning when we stop the service.
!macro OMA_SECTIONS
Section "$(omaSensorsSection)" SecSensors
  ; 0. Program Files only (checked in NSIS_HOOK_PREINSTALL too; again here in
  ;    case anything changed $INSTDIR since).
  Call OmaCheckInstallDir
  ${If} $OmaResult != "0"
    !insertmacro OMA_FAIL "$(omaSensorsNeedProgramFiles)" "$OmaResult"
  ${EndIf}

  ; 1. Stop the running service before replacing its exe. On failure we abort
  ;    with the old exe and the old service untouched.
  Call OmaStopService
  ${If} $OmaResult != "0"
    !insertmacro OMA_FAIL "$(omaSensorsFailed)" "stop: $OmaResult"
  ${EndIf}

  ; 2. Lock $INSTDIR\service down (SYSTEM/Administrators only) and empty it,
  ;    before anything is written or run from it.
  Call OmaProtectServiceDir
  ${If} $OmaResult != "0"
    !insertmacro OMA_FAIL "$(omaSensorsFailed)" "protecting the service folder: $OmaResult"
  ${EndIf}

  ; 3. Copy the exe. A locked or unwritable file sets the error flag (the
  ;    Retry/Ignore prompt answers Ignore when silent).
  SetOutPath "$INSTDIR\service"
  ClearErrors
  File "${OMA_PAYLOAD}\service\${OMA_SERVICE_EXE}"
  ${If} ${Errors}
    !insertmacro OMA_FAIL "$(omaSensorsFailed)" "copy of ${OMA_SERVICE_EXE} failed"
  ${EndIf}
  SetOutPath $INSTDIR

  ; 4. install: create or update the service (demand start, quoted path,
  ;    failure actions) and add the IU start/stop rights. Idempotent.
  !insertmacro OMA_HELPER "install"
  ${If} $0 != "0"
    !insertmacro OMA_FAIL "$(omaSensorsFailed)" "${OMA_SERVICE_EXE} install exited with $0"
  ${EndIf}

  ; 5. PawnIO >= ${OMA_PAWNIO_MIN}. The key only exists in the 64-bit view.
  ;    The setup runs from $INSTDIR\service (writable by administrators only),
  ;    never from the user-writable temp directory, and is deleted after use.
  SetRegView 64
  ReadRegStr $1 HKLM "${OMA_PAWNIO_KEY}" "DisplayVersion"
  StrCpy $2 1 ; 1 = install needed
  ${If} $1 != ""
    ${VersionCompare} "$1" "${OMA_PAWNIO_MIN}" $3 ; 0 equal, 1 newer, 2 older
    ${If} $3 <> 2
      StrCpy $2 0
    ${EndIf}
  ${EndIf}
  ${If} $2 = 1
    DetailPrint "Installing PawnIO (found: '$1')"
    SetOutPath "$INSTDIR\service"
    ClearErrors
    File "/oname=$INSTDIR\service\PawnIO_setup.exe" "${OMA_PAYLOAD}\PawnIO_setup.exe"
    ${If} ${Errors}
      Delete "$INSTDIR\service\PawnIO_setup.exe"
      !insertmacro OMA_FAIL "$(omaSensorsFailed)" "copy of PawnIO_setup.exe failed"
    ${EndIf}
    SetOutPath $INSTDIR
    ClearErrors
    ExecWait '"$INSTDIR\service\PawnIO_setup.exe" -install -silent' $3
    ${If} ${Errors}
      StrCpy $3 "error"
    ${EndIf}
    Delete "$INSTDIR\service\PawnIO_setup.exe"
    ${If} $3 == "error"
      !insertmacro OMA_FAIL "$(omaSensorsFailed)" "PawnIO setup could not be started"
    ${ElseIf} $3 == "3010"
      SetRebootFlag true
    ${ElseIf} $3 != "0"
      !insertmacro OMA_FAIL "$(omaSensorsFailed)" "PawnIO setup exited with $3"
    ${EndIf}
  ${Else}
    DetailPrint "PawnIO $1 already installed"
  ${EndIf}
SectionEnd

; Runs only if SecSensors completed (any failure there aborted the install), so
; AdvancedSensors=1 is recorded only for a working component and a failure keeps
; the previous choice.
Section -OmaSensorsBookkeeping
  SetRegView 64
  ${If} ${SectionIsSelected} ${SecSensors}
    WriteRegDWORD HKLM "${OMA_REGKEY}" "${OMA_REGVALUE}" 1
  ${Else}
    ; Deselected on a reinstall: remove what a previous install created, and
    ; delete the exe only once the service is really gone.
    ${If} ${FileExists} "$INSTDIR\service\${OMA_SERVICE_EXE}"
      Call OmaStopService
      ${If} $OmaResult != "0"
        !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "stop: $OmaResult"
      ${EndIf}
      !insertmacro OMA_HELPER "uninstall"
      ${If} $0 != "0"
        !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "${OMA_SERVICE_EXE} uninstall exited with $0"
      ${EndIf}
      Delete "$INSTDIR\service\${OMA_SERVICE_EXE}"
      Call OmaRemoveServiceLogs
      RMDir "$INSTDIR\service"
    ${Else}
      ; No exe, but the service may still be registered (orphan): stop it and
      ; delete it through the SCM. Both are no-ops when there is no service.
      Call OmaStopService
      ${If} $OmaResult != "0"
        !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "stop: $OmaResult"
      ${EndIf}
      Call OmaDeleteService
      ${If} $OmaResult != "0"
        !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "delete: $OmaResult"
      ${EndIf}
      Call OmaRemoveServiceLogs
      RMDir "$INSTDIR\service"
    ${EndIf}
    WriteRegDWORD HKLM "${OMA_REGKEY}" "${OMA_REGVALUE}" 0
  ${EndIf}
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecSensors} "$(omaSensorsDesc)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; "0" when Advanced sensors is not selected, otherwise OmaCheckInstallDir's result.
Function OmaCheckSensorsInstallDir
  StrCpy $OmaResult "0"
  ${If} ${SectionIsSelected} ${SecSensors}
    Call OmaCheckInstallDir
  ${EndIf}
FunctionEnd

; Leave callback of the directory page (template line marked OMA). Simplest
; robust UX: the user stays on the page and either picks a folder inside
; Program Files or goes back to the components page to clear the option.
Function OmaDirectoryLeave
  Call OmaCheckSensorsInstallDir
  ${If} $OmaResult != "0"
    StrCpy $OmaDetail $OmaResult
    MessageBox MB_ICONEXCLAMATION|MB_OK "$(omaSensorsNeedProgramFiles)"
    Abort
  ${EndIf}
FunctionEnd

Function OmaInitComponents
  Push $0
  SetRegView 64
  ClearErrors
  ReadRegDWORD $0 HKLM "${OMA_REGKEY}" "${OMA_REGVALUE}"
  ${IfNot} ${Errors}
    ${If} $0 = 0
      !insertmacro UnselectSection ${SecSensors}
    ${EndIf}
  ${EndIf}
  ${GetOptions} $CMDLINE "/NOSENSORS" $0
  ${IfNot} ${Errors}
    !insertmacro UnselectSection ${SecSensors}
  ${EndIf}
  Pop $0
FunctionEnd
!macroend

; Tauri hook: runs at the top of the Uninstall section, before any file is
; deleted, so a failure here leaves the installation intact. It closes the app
; first (the template's own check, repeated later, then finds nothing): if the
; user cancels the "app is running" prompt, the uninstall stops before the
; service is touched. Keeps the uninstaller diff at zero.
; PawnIO is never touched: it can be shared with other programs. The service's
; logs go with the service, except on an upgrade (ruling R30).
!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  Call un.OmaStopService
  ${If} $OmaResult != "0"
    !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "stop: $OmaResult"
  ${EndIf}
  ${If} $UpdateMode <> 1
    ${If} ${FileExists} "$INSTDIR\service\${OMA_SERVICE_EXE}"
      ; stop + delete the service
      !insertmacro OMA_HELPER "uninstall"
      ${If} $0 != "0"
        !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "${OMA_SERVICE_EXE} uninstall exited with $0"
      ${EndIf}
    ${Else}
      ; No exe, but the service may still be registered (orphan).
      Call un.OmaDeleteService
      ${If} $OmaResult != "0"
        !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "delete: $OmaResult"
      ${EndIf}
    ${EndIf}
  ${EndIf}
  Delete "$INSTDIR\service\${OMA_SERVICE_EXE}"
  ${If} $UpdateMode <> 1
    Call un.OmaRemoveServiceLogs
  ${EndIf}
  RMDir "$INSTDIR\service"
  ${If} $UpdateMode <> 1
    SetRegView 64
    DeleteRegValue HKLM "${OMA_REGKEY}" "${OMA_REGVALUE}"
    DeleteRegKey /ifempty HKLM "${OMA_REGKEY}"
    DeleteRegKey /ifempty HKLM "Software\OpenMonitorAdvanced"
  ${EndIf}
  ; The app's start-with-Windows entry of the user who uninstalls. Other users'
  ; entries stay: they point at an exe that is gone and Windows skips them.
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "OpenMonitor Advanced"
  ${EndIf}
!macroend
