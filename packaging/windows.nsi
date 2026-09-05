Unicode True
!include "MUI2.nsh"
Name "Emulator Hub"
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\Emulator Hub"
RequestExecutionLevel user
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "Emulator Hub"
  SetOutPath "$INSTDIR"
  File /r "${SOURCE}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Emulator Hub"
  CreateShortcut "$SMPROGRAMS\Emulator Hub\Emulator Hub.lnk" "$INSTDIR\emulator-hub.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EmulatorHub" "DisplayName" "Emulator Hub"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EmulatorHub" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EmulatorHub" "UninstallString" '$"$INSTDIR\Uninstall.exe$"'
SectionEnd
Section "Uninstall"
  Delete "$SMPROGRAMS\Emulator Hub\Emulator Hub.lnk"
  RMDir "$SMPROGRAMS\Emulator Hub"
  Delete "$INSTDIR\emulator-hub.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.txt"
  Delete "$INSTDIR\FONT-OFL-1.1.txt"
  Delete "$INSTDIR\WINDOWS-RUNTIME-IMPORTS.json"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\EmulatorHub"
SectionEnd
