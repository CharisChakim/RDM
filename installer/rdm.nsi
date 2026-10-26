; RDM installer: per-user (no admin rights), Start menu shortcut, uninstaller.
; Build the app first (cargo build --release -p rdm-desktop), then: makensis installer\rdm.nsi

Unicode true
!define APP "RDM"
!define VERSION "0.1.0"
!define UNINST "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"

Name "RDM - Rust Download Manager"
OutFile "..\target\release\RDM_${VERSION}_x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!define MUI_ICON "..\desktop\ui\icons\app.ico"
!define MUI_UNICON "..\desktop\ui\icons\app.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\RDM.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start RDM"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section
  ; A running copy (it lives in the tray) would keep the exe locked.
  nsExec::Exec 'taskkill /IM RDM.exe /F'
  Sleep 500
  SetOutPath "$INSTDIR"
  File "..\target\release\RDM.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\RDM.exe"
  WriteRegStr HKCU "${UNINST}" "DisplayName" "RDM - Rust Download Manager"
  WriteRegStr HKCU "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST}" "DisplayIcon" "$INSTDIR\RDM.exe"
  WriteRegStr HKCU "${UNINST}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  nsExec::Exec 'taskkill /IM RDM.exe /F'
  Sleep 500
  Delete "$INSTDIR\RDM.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  ; RDM writes this itself when "Start with Windows" is on.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${APP}"
  DeleteRegKey HKCU "${UNINST}"
  ; The download list in %APPDATA%\dev.rdm.app is kept, like IDM keeps its list.
SectionEnd
