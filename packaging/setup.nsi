Unicode true
Name "KohdaLab SLM ${VERSION}"
OutFile "${OUTPUT}"
InstallDir "$PROGRAMFILES64\Kohdalab\KohdaLab SLM"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
InstType "KohdaLab SLM"
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\KohdaLab SLM"

Function .onInit
  SetRegView 64
  SetShellVarContext all
  StrCpy $INSTDIR "$PROGRAMFILES64\Kohdalab\KohdaLab SLM"
FunctionEnd

Section
  SetOutPath "$INSTDIR"
  File /r "${PAYLOAD}\*"
  WriteUninstaller "$INSTDIR\Uninstall KohdaLab SLM.exe"
  CreateShortcut "$SMPROGRAMS\KohdaLab SLM.lnk" "$INSTDIR\kohdalab-slm.exe"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "KohdaLab SLM"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "KohdaLab"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall KohdaLab SLM.exe"'
SectionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext all
  StrCmp $INSTDIR "$PROGRAMFILES64\Kohdalab\KohdaLab SLM" expected
    Abort
  expected:
FunctionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\KohdaLab SLM.lnk"
  DeleteRegKey HKLM "${UNINSTALL_KEY}"
  Delete "$INSTDIR\kohdalab-slm.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  Delete "$INSTDIR\bindings\control\LICENSE"
  Delete "$INSTDIR\bindings\control\README.md"
  Delete "$INSTDIR\bindings\control\kohdalab_control.py"
  Delete "$INSTDIR\bindings\control\pyproject.toml"
  Delete "$INSTDIR\bindings\control\GUI-API.md"
  RMDir "$INSTDIR\bindings\control"
  RMDir "$INSTDIR\bindings"
  RMDir /r "$INSTDIR\third-party-licenses"
  Delete "$INSTDIR\Uninstall KohdaLab SLM.exe"
  RMDir "$INSTDIR"
SectionEnd
