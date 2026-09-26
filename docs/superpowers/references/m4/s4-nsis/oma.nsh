; OpenMonitor Advanced additions to the Tauri NSIS template.
;
; Included by installer.nsi through `bundle.windows.nsis.installerHooks`, i.e.
; before any !define of the template, so only macros and path defines live at
; top level. `${__FILEDIR__}` is this file's directory (app/src-tauri/nsis), which
; is how we locate the payload no matter where CARGO_TARGET_DIR points.

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

!if /FileExists "${OMA_PAYLOAD}\service\${OMA_SERVICE_EXE}"
!else
  !error "Missing ${OMA_PAYLOAD}\service\${OMA_SERVICE_EXE}: run the payload script first"
!endif
!if /FileExists "${OMA_PAYLOAD}\PawnIO_setup.exe"
!else
  !error "Missing ${OMA_PAYLOAD}\PawnIO_setup.exe: run the payload script first"
!endif

; Strings for our section. Inserted after the MUI_LANGUAGE macros.
!macro OMA_LANGSTRINGS
  LangString omaSensorsSection ${LANG_ENGLISH} "Advanced sensors"
  LangString omaSensorsDesc ${LANG_ENGLISH} "Windows service and PawnIO driver for CPU temperatures, voltages, fans and SMART data. Requires administrator rights only now, during setup."
  LangString omaHelperFailed ${LANG_ENGLISH} "Advanced sensors could not be configured (code $0). The app will run in basic mode."
  !ifdef LANG_ITALIAN
    LangString omaSensorsSection ${LANG_ITALIAN} "Sensori avanzati"
    LangString omaSensorsDesc ${LANG_ITALIAN} "Servizio Windows e driver PawnIO per temperature, tensioni e ventole della CPU e dati SMART. Servono i privilegi di amministratore solo ora, durante l'installazione."
    LangString omaHelperFailed ${LANG_ITALIAN} "Non è stato possibile configurare i sensori avanzati (codice $0). L'app funzionerà in modalità base."
  !endif
!macroend

; Stops oma-service if it exists and waits until SERVICE_STOPPED, using the SCM
; API directly: no child process, no dependency on the previously installed
; helper, no System32/SysWOW64 redirection issue (the SCM is not bitness-bound).
!macro OMA_STOP_SERVICE un
Function ${un}OmaStopService
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  ; SC_MANAGER_CONNECT
  System::Call 'advapi32::OpenSCManagerW(p 0, p 0, i 0x0001) p.r0'
  ${If} $0 P<> 0
    ; SERVICE_STOP | SERVICE_QUERY_STATUS
    System::Call 'advapi32::OpenServiceW(p r0, w "${OMA_SERVICE_NAME}", i 0x0024) p.r1'
    ${If} $1 P<> 0
      DetailPrint "Stopping ${OMA_SERVICE_NAME}..."
      System::Call '*(i, i, i, i, i, i, i) p.r2' ; SERVICE_STATUS
      ; SERVICE_CONTROL_STOP; fails with 1062 when already stopped, which is fine
      System::Call 'advapi32::ControlService(p r1, i 1, p r2) i'
      StrCpy $4 0
      ${Do}
        System::Call 'advapi32::QueryServiceStatus(p r1, p r2) i.r3'
        ${If} $3 = 0
          ${Break}
        ${EndIf}
        System::Call '*$2(i, i .r3)' ; dwCurrentState
        ${If} $3 = 1 ; SERVICE_STOPPED
          ${Break}
        ${EndIf}
        ${If} $4 >= ${OMA_STOP_TIMEOUT_TICKS}
          DetailPrint "${OMA_SERVICE_NAME} did not stop in time (state $3)"
          ${Break}
        ${EndIf}
        Sleep 500
        IntOp $4 $4 + 1
      ${Loop}
      System::Free $2
      System::Call 'advapi32::CloseServiceHandle(p r1)'
    ${EndIf}
    System::Call 'advapi32::CloseServiceHandle(p r0)'
  ${EndIf}
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

; Declares the optional section, the hidden bookkeeping section and the init
; function. Inserted after the template's (hidden) Install section so that the
; app has already been closed by CheckIfAppIsRunning when we stop the service.
!macro OMA_SECTIONS
Section "$(omaSensorsSection)" SecSensors
  Call OmaStopService
  SetOutPath "$INSTDIR\service"
  File "${OMA_PAYLOAD}\service\${OMA_SERVICE_EXE}"
  SetOutPath $INSTDIR

  ; install: create or update the service (demand start, quoted path, failure
  ; actions) and splice the IU start/stop ACE into the live DACL. Idempotent.
  !insertmacro OMA_HELPER "install"
  ${If} $0 != "0"
    DetailPrint "oma-service install failed with $0"
    MessageBox MB_ICONEXCLAMATION|MB_OK "$(omaHelperFailed)" /SD IDOK
  ${EndIf}

  ; PawnIO >= ${OMA_PAWNIO_MIN}. The key only exists in the 64-bit view.
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
    InitPluginsDir
    File "/oname=$PLUGINSDIR\PawnIO_setup.exe" "${OMA_PAYLOAD}\PawnIO_setup.exe"
    ClearErrors
    ExecWait '"$PLUGINSDIR\PawnIO_setup.exe" -install -silent' $3
    ${If} ${Errors}
      DetailPrint "PawnIO setup could not be started"
    ${ElseIf} $3 = 3010
      SetRebootFlag true
    ${ElseIf} $3 <> 0
      DetailPrint "PawnIO setup failed with $3"
    ${EndIf}
    Delete "$PLUGINSDIR\PawnIO_setup.exe"
  ${Else}
    DetailPrint "PawnIO $1 already installed"
  ${EndIf}
SectionEnd

Section -OmaSensorsBookkeeping
  SetRegView 64
  ${If} ${SectionIsSelected} ${SecSensors}
    WriteRegDWORD HKLM "${OMA_REGKEY}" "${OMA_REGVALUE}" 1
  ${Else}
    ; Deselected on a reinstall: remove what a previous install created.
    ${If} ${FileExists} "$INSTDIR\service\${OMA_SERVICE_EXE}"
      Call OmaStopService
      !insertmacro OMA_HELPER "uninstall"
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
; CheckIfAppIsRunning. Stopping the service while the app is still open is
; harmless: the app only starts it at launch. Keeps the uninstaller diff at zero.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} ${FileExists} "$INSTDIR\service\${OMA_SERVICE_EXE}"
    Call un.OmaStopService
    ${If} $UpdateMode <> 1
      ; stop + delete the service. PawnIO stays (shared with other programs).
      !insertmacro OMA_HELPER "uninstall"
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
