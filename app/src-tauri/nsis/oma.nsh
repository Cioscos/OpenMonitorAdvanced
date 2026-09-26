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

; "0" on success, otherwise a short English description (OmaStopService).
Var OmaResult
; What failed, shown inside the omaSensorsFailed / omaServiceRemoveFailed texts.
Var OmaDetail

; Strings for our section. Inserted after the MUI_LANGUAGE macros.
!macro OMA_LANGSTRINGS
  LangString omaSensorsSection ${LANG_ENGLISH} "Advanced sensors"
  LangString omaSensorsDesc ${LANG_ENGLISH} "Windows service and PawnIO driver for CPU temperatures, voltages, fans and SMART data. Requires administrator rights only now, during setup."
  LangString omaSensorsFailed ${LANG_ENGLISH} "Advanced sensors could not be set up ($OmaDetail).$\r$\n$\r$\nSetup stopped before finishing. Run it again, or clear the Advanced sensors option to install without them."
  LangString omaServiceRemoveFailed ${LANG_ENGLISH} "The Advanced sensors service could not be removed ($OmaDetail).$\r$\n$\r$\nNo files were deleted. Close OpenMonitor Advanced and try again."
  !ifdef LANG_ITALIAN
    LangString omaSensorsSection ${LANG_ITALIAN} "Sensori avanzati"
    LangString omaSensorsDesc ${LANG_ITALIAN} "Servizio Windows e driver PawnIO per temperature, tensioni e ventole della CPU e dati SMART. Servono i privilegi di amministratore solo ora, durante l'installazione."
    LangString omaSensorsFailed ${LANG_ITALIAN} "Non è stato possibile configurare i sensori avanzati ($OmaDetail).$\r$\n$\r$\nL'installazione si è fermata prima della fine. Riprova, oppure togli l'opzione Sensori avanzati per installare senza."
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
  ; 1. Stop the running service before replacing its exe. On failure we abort
  ;    with the old exe and the old service untouched.
  Call OmaStopService
  ${If} $OmaResult != "0"
    !insertmacro OMA_FAIL "$(omaSensorsFailed)" "stop: $OmaResult"
  ${EndIf}

  ; 2. Copy the exe. A locked or unwritable file sets the error flag (the
  ;    Retry/Ignore prompt answers Ignore when silent).
  SetOutPath "$INSTDIR\service"
  ClearErrors
  File "${OMA_PAYLOAD}\service\${OMA_SERVICE_EXE}"
  ${If} ${Errors}
    !insertmacro OMA_FAIL "$(omaSensorsFailed)" "copy of ${OMA_SERVICE_EXE} failed"
  ${EndIf}
  SetOutPath $INSTDIR

  ; 3. install: create or update the service (demand start, quoted path,
  ;    failure actions) and add the IU start/stop rights. Idempotent.
  !insertmacro OMA_HELPER "install"
  ${If} $0 != "0"
    !insertmacro OMA_FAIL "$(omaSensorsFailed)" "${OMA_SERVICE_EXE} install exited with $0"
  ${EndIf}

  ; 4. PawnIO >= ${OMA_PAWNIO_MIN}. The key only exists in the 64-bit view.
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
      RMDir "$INSTDIR\service"
    ${EndIf}
    WriteRegDWORD HKLM "${OMA_REGKEY}" "${OMA_REGVALUE}" 0
  ${EndIf}
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecSensors} "$(omaSensorsDesc)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

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

; Tauri hook: runs at the top of the Uninstall section, before the template's
; CheckIfAppIsRunning and before any file is deleted, so a failure here leaves
; the installation intact. Stopping the service while the app is still open is
; harmless: the app only starts it at launch. Keeps the uninstaller diff at zero.
; PawnIO is never touched: it can be shared with other programs.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\service\${OMA_SERVICE_EXE}"
    Call un.OmaStopService
    ${If} $OmaResult != "0"
      !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "stop: $OmaResult"
    ${EndIf}
    ${If} $UpdateMode <> 1
      ; stop + delete the service
      !insertmacro OMA_HELPER "uninstall"
      ${If} $0 != "0"
        !insertmacro OMA_FAIL "$(omaServiceRemoveFailed)" "${OMA_SERVICE_EXE} uninstall exited with $0"
      ${EndIf}
    ${EndIf}
    Delete "$INSTDIR\service\${OMA_SERVICE_EXE}"
    RMDir "$INSTDIR\service"
  ${EndIf}
  ${If} $UpdateMode <> 1
    SetRegView 64
    DeleteRegValue HKLM "${OMA_REGKEY}" "${OMA_REGVALUE}"
    DeleteRegKey /ifempty HKLM "${OMA_REGKEY}"
    DeleteRegKey /ifempty HKLM "Software\OpenMonitorAdvanced"
  ${EndIf}
!macroend
