; Per-user installer: no administrator rights, a shortcut on the desktop and in the Start menu.
; Built by CI: makensis -DVERSION=x.y.z -DOUTFILE=<path> installer/collector.nsi

Unicode true
ManifestDPIAware true
!include "MUI2.nsh"

!ifndef VERSION
  !error "VERSION is required, for example -DVERSION=0.1.0"
!endif
!ifndef OUTFILE
  !define OUTFILE "BigRocketCampCollector-setup.exe"
!endif

!define NAME "Big Rocket Camp Collector"
!define APP_ID "BigRocketCampCollector"
!define EXE "bigrocketcamp-collector.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
; WM_APP + 16 in src/win/ui.rs: the collector quits instead of hiding.
!define WM_QUIT_COLLECTOR 0x8010

Name "${NAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APP_ID}"
RequestExecutionLevel user
SetCompressor /SOLID lzma

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1036 "ProductName" "${NAME}"
VIAddVersionKey /LANG=1036 "FileDescription" "Installation de ${NAME}"
VIAddVersionKey /LANG=1036 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1036 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1036 "LegalCopyright" "MIT"

!define MUI_ICON "..\assets\icons\app.ico"
!define MUI_UNICON "..\assets\icons\app.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Lancer ${NAME}"
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "French"

; A running collector holds its executable: ask it to quit before replacing or removing it.
!macro CloseCollector
  FindWindow $0 "${APP_ID}"
  IntCmp $0 0 +3
    SendMessage $0 ${WM_QUIT_COLLECTOR} 0 0
    Sleep 1000
!macroend

Section
  !insertmacro CloseCollector
  SetOutPath "$INSTDIR"
  File "..\target\release\${EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$DESKTOP\${NAME}.lnk" "$INSTDIR\${EXE}"
  CreateShortcut "$SMPROGRAMS\${NAME}.lnk" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Big Rocket Camp"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  !insertmacro CloseCollector
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$DESKTOP\${NAME}.lnk"
  Delete "$SMPROGRAMS\${NAME}.lnk"
  DeleteRegValue HKCU "${RUN_KEY}" "${APP_ID}"
  ; The token, the queue and the settings leave with the collector.
  RMDir /r "$LOCALAPPDATA\${APP_ID}"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
