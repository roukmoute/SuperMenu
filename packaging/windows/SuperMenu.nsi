; Native per-user Windows installation. The launcher remains a standalone exe.
Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 8
CRCCheck on
Name "Super Menu"
Caption "Super Menu"
OutFile "${OUTPUT_EXE}"
InstallDir "${DEFAULT_INSTALL_DIR}"
ShowInstDetails nevershow
ShowUninstDetails nevershow
VIProductVersion "${PRODUCT_VERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "Super Menu"
VIAddVersionKey /LANG=1033 "FileDescription" "Super Menu Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Copyright roukmoute"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "nsDialogs.nsh"
!include "x64.nsh"
!include "Win\COM.nsh"

!define ARP_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${INSTALL_ID}"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define MUI_ICON "${BRAND_ICON}"
!define MUI_UNICON "${BRAND_ICON}"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TITLE "$(WelcomeTitle)"
!define MUI_WELCOMEPAGE_TEXT "$(WelcomeText)"
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION "LaunchApplication"
!define MUI_FINISHPAGE_RUN_TEXT "$(LaunchText)"
!define MUI_FINISHPAGE_NOAUTOCLOSE

!insertmacro MUI_PAGE_WELCOME
Page custom OptionsCreate OptionsLeave
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "French"

LangString WelcomeTitle ${LANG_ENGLISH} "Install Super Menu"
LangString WelcomeTitle ${LANG_FRENCH} "Installer Super Menu"
LangString WelcomeText ${LANG_ENGLISH} "A fast, lightweight Windows launcher, available with Win + Space.$\r$\n$\r$\nSetup installs Super Menu for your Windows account. Your existing settings and personal data are preserved."
LangString WelcomeText ${LANG_FRENCH} "Un lanceur Windows rapide et léger, disponible avec Win + Espace.$\r$\n$\r$\nSuper Menu sera installé pour votre compte Windows. Vos réglages et données personnelles sont conservés."
LangString OptionsTitle ${LANG_ENGLISH} "Choose your shortcuts"
LangString OptionsTitle ${LANG_FRENCH} "Choisir les raccourcis"
LangString OptionsSubtitle ${LANG_ENGLISH} "Super Menu will be available from the Start menu."
LangString OptionsSubtitle ${LANG_FRENCH} "Super Menu sera disponible dans le menu Démarrer."
LangString OptionsBody ${LANG_ENGLISH} "Install for your account, without administrator permissions. Choose the optional shortcuts below."
LangString OptionsBody ${LANG_FRENCH} "Installation pour votre compte, sans droits administrateur. Choisissez les raccourcis facultatifs ci-dessous."
LangString StartupText ${LANG_ENGLISH} "Start Super Menu when I sign in to Windows"
LangString StartupText ${LANG_FRENCH} "Démarrer Super Menu à la connexion Windows"
LangString DesktopText ${LANG_ENGLISH} "Create a desktop shortcut"
LangString DesktopText ${LANG_FRENCH} "Créer un raccourci sur le Bureau"
LangString PreserveText ${LANG_ENGLISH} "On updates, your existing shortcut choices are selected automatically."
LangString PreserveText ${LANG_FRENCH} "Lors d'une mise à jour, les choix de raccourcis existants sont repris automatiquement."
LangString LaunchText ${LANG_ENGLISH} "Open Super Menu"
LangString LaunchText ${LANG_FRENCH} "Ouvrir Super Menu"
LangString ShutdownError ${LANG_ENGLISH} "Super Menu could not be safely closed. Finish the running software operation or close the existing application, then try again. No files were replaced or removed."
LangString ShutdownError ${LANG_FRENCH} "Super Menu n'a pas pu être fermé correctement. Terminez l'opération de logiciel en cours ou fermez l'application existante, puis réessayez. Aucun fichier n'a été remplacé ou supprimé."
LangString ShutdownExecError ${LANG_ENGLISH} "Setup could not run the Super Menu shutdown command. No files were replaced or removed."
LangString ShutdownExecError ${LANG_FRENCH} "Le programme d'installation ne peut pas exécuter la commande d'arrêt de Super Menu. Aucun fichier n'a été remplacé ou supprimé."
LangString InstallError ${LANG_ENGLISH} "Super Menu could not be installed in the selected folder."
LangString InstallError ${LANG_FRENCH} "Super Menu ne peut pas être installé dans le dossier prévu."
LangString RemoveError ${LANG_ENGLISH} "An installed file could not be removed. Close applications using this folder, then try uninstalling again. The uninstall entry has been preserved."
LangString RemoveError ${LANG_FRENCH} "Un fichier installé ne peut pas être supprimé. Fermez les applications utilisant ce dossier, puis réessayez. L'entrée de désinstallation a été conservée."
LangString BadOptions ${LANG_ENGLISH} "Use /STARTUP=0 or 1 and /DESKTOP=0 or 1."
LangString BadOptions ${LANG_FRENCH} "Utilisez /STARTUP=0 ou 1 et /DESKTOP=0 ou 1."
LangString RequireX64 ${LANG_ENGLISH} "Super Menu requires 64-bit Windows."
LangString RequireX64 ${LANG_FRENCH} "Super Menu nécessite Windows 64 bits."

Var Startup
Var DesktopChoice
Var StartupCheckbox
Var DesktopChoiceCheckbox
Var Parameters
Var Dialog
Var StopExit
Var StagedExecutable

; Never delete or replace a shortcut repointed to a different program. Use the
; native ShellLink interface without resolving targets or starting applications.
!macro DefineIsOwnedShortcut Prefix
Function ${Prefix}IsOwnedShortcut
  Exch $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  StrCpy $1 0
  StrCpy $2 0
  StrCpy $5 0
  IfFileExists "$0" 0 shortcut_done
  System::Call 'OLE32::CoInitializeEx(p0,i2)i.r4'
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r1 ".r3"
  ${If} $1 P<> 0
    ${IUnknown::QueryInterface} $1 '("${IID_IPersistFile}",.r2).r3'
    ${If} $2 P<> 0
      ${IPersistFile::Load} $2 '("$0",${STGM_READ}).r3'
      ${If} $3 == 0
        ${IShellLink::GetPath} $1 '(.r6,${NSIS_MAX_STRLEN},0,4).r3'
        ${If} $3 == 0
          StrCmp $6 "$INSTDIR\supermenu.exe" 0 +2
            StrCpy $5 1
        ${EndIf}
      ${EndIf}
      ${IUnknown::Release} $2 ""
    ${EndIf}
    ${IUnknown::Release} $1 ""
  ${EndIf}
  ${If} $4 == 0
  ${OrIf} $4 == 1
    System::Call 'OLE32::CoUninitialize()'
  ${EndIf}
shortcut_done:
  StrCpy $0 $5
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Exch $0
FunctionEnd
!macroend
!insertmacro DefineIsOwnedShortcut ""
!insertmacro DefineIsOwnedShortcut "un."

Function .onInit
  SetShellVarContext current
  SetRegView 64
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "$(RequireX64)" /SD IDOK
    SetErrorLevel 64
    Abort
  ${EndIf}
  ReadRegStr $0 HKCU "${ARP_KEY}" "InstallLocation"
  StrCmp $0 "" +2
    StrCpy $INSTDIR $0
  StrCpy $Startup 0
  StrCpy $DesktopChoice 0
  ReadRegStr $0 HKCU "${RUN_KEY}" "${INSTALL_ID}"
  StrCmp $0 '$\"$INSTDIR\supermenu.exe$\" --background' 0 +2
    StrCpy $Startup 1
  Push "$DESKTOP\${SHORTCUT_NAME}.lnk"
  Call IsOwnedShortcut
  Pop $DesktopChoice
  ${GetParameters} $Parameters
  ClearErrors
  ${GetOptions} $Parameters "/STARTUP=" $0
  IfErrors startup_done
  StrCmp $0 "0" startup_off
  StrCmp $0 "1" startup_on invalid_options
startup_off:
  StrCpy $Startup 0
  Goto startup_done
startup_on:
  StrCpy $Startup 1
startup_done:
  ClearErrors
  ${GetOptions} $Parameters "/DESKTOP=" $0
  IfErrors options_done
  StrCmp $0 "0" desktop_off
  StrCmp $0 "1" desktop_on invalid_options
desktop_off:
  StrCpy $DesktopChoice 0
  Goto options_done
desktop_on:
  StrCpy $DesktopChoice 1
options_done:
  Return
invalid_options:
  MessageBox MB_OK|MB_ICONSTOP "$(BadOptions)" /SD IDOK
  SetErrorLevel 64
  Abort
FunctionEnd

Function OptionsCreate
  !insertmacro MUI_HEADER_TEXT "$(OptionsTitle)" "$(OptionsSubtitle)"
  nsDialogs::Create 1018
  Pop $Dialog
  ${If} $Dialog == error
    Abort
  ${EndIf}
  ${NSD_CreateLabel} 0 0 100% 32u "$(OptionsBody)"
  Pop $0
  ${NSD_CreateCheckbox} 0 44u 100% 12u "$(StartupText)"
  Pop $StartupCheckbox
  ${NSD_SetState} $StartupCheckbox $Startup
  ${NSD_CreateCheckbox} 0 68u 100% 12u "$(DesktopText)"
  Pop $DesktopChoiceCheckbox
  ${NSD_SetState} $DesktopChoiceCheckbox $DesktopChoice
  ${NSD_CreateLabel} 0 102u 100% 30u "$(PreserveText)"
  Pop $0
  nsDialogs::Show
FunctionEnd

Function OptionsLeave
  ${NSD_GetState} $StartupCheckbox $Startup
  ${NSD_GetState} $DesktopChoiceCheckbox $DesktopChoice
FunctionEnd

Function LaunchApplication
  IfSilent launch_done
  Exec '$\"$INSTDIR\supermenu.exe$\"'
launch_done:
FunctionEnd

; The NEW payload is the controller for upgrades, including older versions
; which do not yet understand --shutdown-for-install themselves.
Function StopBeforeInstall
  StrCpy $StopExit 1
  ClearErrors
  ExecWait '$\"$PLUGINSDIR\supermenu-setup-helper.exe$\" --shutdown-for-install $\"$INSTDIR\supermenu.exe$\"' $StopExit
  IfErrors shutdown_exec_failed
  StrCmp $StopExit 0 shutdown_done
  MessageBox MB_OK|MB_ICONSTOP "$(ShutdownError)" /SD IDOK
  SetErrorLevel 2
  Abort
shutdown_exec_failed:
  MessageBox MB_OK|MB_ICONSTOP "$(ShutdownExecError)" /SD IDOK
  SetErrorLevel 3
  Abort
shutdown_done:
FunctionEnd

Section "Super Menu"
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=supermenu-setup-helper.exe "${SOURCE_EXE}"
  Call StopBeforeInstall
  SetOutPath "$INSTDIR"
  ClearErrors
  ; Complete the copy before replacing an earlier installation. A failed
  ; replacement leaves the old payload and its uninstall metadata intact.
  GetTempFileName $StagedExecutable "$INSTDIR"
  IfErrors install_failed
  CopyFiles /SILENT "$PLUGINSDIR\supermenu-setup-helper.exe" "$StagedExecutable"
  IfErrors install_failed
  System::Call 'kernel32::MoveFileExW(w"$StagedExecutable",w"$INSTDIR\supermenu.exe",i9)i.r0'
  StrCmp $0 0 install_failed
  StrCpy $StagedExecutable ""
  File /oname=LICENSE "${LICENSE_FILE}"
  File /oname=THIRD_PARTY_NOTICES.md "${NOTICES_FILE}"
  File /oname=brand.ico "${BRAND_ICON}"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  IfErrors install_failed
  CreateDirectory "$SMPROGRAMS\${SHORTCUT_NAME}"
  IfFileExists "$SMPROGRAMS\${SHORTCUT_NAME}\Super Menu.lnk" 0 create_start_shortcut
  Push "$SMPROGRAMS\${SHORTCUT_NAME}\Super Menu.lnk"
  Call IsOwnedShortcut
  Pop $0
  StrCmp $0 1 create_start_shortcut start_shortcut_done
create_start_shortcut:
  CreateShortCut "$SMPROGRAMS\${SHORTCUT_NAME}\Super Menu.lnk" "$INSTDIR\supermenu.exe" "" "$INSTDIR\brand.ico"
start_shortcut_done:
  ${If} $DesktopChoice == 1
    IfFileExists "$DESKTOP\${SHORTCUT_NAME}.lnk" 0 create_desktop_shortcut
    Push "$DESKTOP\${SHORTCUT_NAME}.lnk"
    Call IsOwnedShortcut
    Pop $0
    StrCmp $0 1 create_desktop_shortcut desktop_shortcut_done
create_desktop_shortcut:
    CreateShortCut "$DESKTOP\${SHORTCUT_NAME}.lnk" "$INSTDIR\supermenu.exe" "" "$INSTDIR\brand.ico"
  ${Else}
    Push "$DESKTOP\${SHORTCUT_NAME}.lnk"
    Call IsOwnedShortcut
    Pop $0
    StrCmp $0 1 0 desktop_shortcut_done
      Delete "$DESKTOP\${SHORTCUT_NAME}.lnk"
  ${EndIf}
desktop_shortcut_done:
  ${If} $Startup == 1
    ReadRegStr $0 HKCU "${RUN_KEY}" "${INSTALL_ID}"
    ${If} $0 == ""
    ${OrIf} $0 == '$\"$INSTDIR\supermenu.exe$\" --background'
      WriteRegStr HKCU "${RUN_KEY}" "${INSTALL_ID}" '$\"$INSTDIR\supermenu.exe$\" --background'
    ${EndIf}
  ${Else}
    ReadRegStr $0 HKCU "${RUN_KEY}" "${INSTALL_ID}"
    StrCmp $0 '$\"$INSTDIR\supermenu.exe$\" --background' 0 +2
      DeleteRegValue HKCU "${RUN_KEY}" "${INSTALL_ID}"
  ${EndIf}
  WriteRegStr HKCU "${ARP_KEY}" "DisplayName" "${DISPLAY_NAME}"
  WriteRegStr HKCU "${ARP_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${ARP_KEY}" "Publisher" "roukmoute"
  WriteRegStr HKCU "${ARP_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${ARP_KEY}" "DisplayIcon" '$\"$INSTDIR\brand.ico$\"'
  WriteRegStr HKCU "${ARP_KEY}" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "${ARP_KEY}" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegStr HKCU "${ARP_KEY}" "URLInfoAbout" "https://github.com/roukmoute/SuperMenu"
  WriteRegStr HKCU "${ARP_KEY}" "HelpLink" "https://github.com/roukmoute/SuperMenu/issues"
  WriteRegDWORD HKCU "${ARP_KEY}" "EstimatedSize" ${INSTALL_SIZE_KB}
  WriteRegDWORD HKCU "${ARP_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${ARP_KEY}" "NoRepair" 1
  ${GetTime} "" "L" $0 $1 $2 $3 $4 $5 $6
  WriteRegStr HKCU "${ARP_KEY}" "InstallDate" "$2$1$0"
  SetErrorLevel 0
  Goto install_done
install_failed:
  StrCmp $StagedExecutable "" +2
    Delete "$StagedExecutable"
  MessageBox MB_OK|MB_ICONSTOP "$(InstallError)" /SD IDOK
  SetErrorLevel 4
  Abort
install_done:
SectionEnd

Function un.onInit
  SetShellVarContext current
  SetRegView 64
FunctionEnd

Function un.StopBeforeRemove
  IfFileExists "$INSTDIR\supermenu.exe" 0 shutdown_done
  StrCpy $StopExit 1
  ClearErrors
  ExecWait '$\"$INSTDIR\supermenu.exe$\" --shutdown-for-install $\"$INSTDIR\supermenu.exe$\"' $StopExit
  IfErrors shutdown_exec_failed
  StrCmp $StopExit 0 shutdown_done
  MessageBox MB_OK|MB_ICONSTOP "$(ShutdownError)" /SD IDOK
  SetErrorLevel 2
  Abort
shutdown_exec_failed:
  MessageBox MB_OK|MB_ICONSTOP "$(ShutdownExecError)" /SD IDOK
  SetErrorLevel 3
  Abort
shutdown_done:
FunctionEnd

Section "Uninstall"
  Call un.StopBeforeRemove
  ; Keep the uninstall entry and shortcuts if a file is still locked. Ignore
  ; missing files, but report actual deletion failures before deregistering.
  ClearErrors
  IfFileExists "$INSTDIR\supermenu.exe" 0 +3
    Delete "$INSTDIR\supermenu.exe"
    IfErrors remove_failed
  IfFileExists "$INSTDIR\LICENSE" 0 +3
    Delete "$INSTDIR\LICENSE"
    IfErrors remove_failed
  IfFileExists "$INSTDIR\THIRD_PARTY_NOTICES.md" 0 +3
    Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
    IfErrors remove_failed
  IfFileExists "$INSTDIR\brand.ico" 0 +3
    Delete "$INSTDIR\brand.ico"
    IfErrors remove_failed
  IfFileExists "$INSTDIR\uninstall.exe" 0 +3
    Delete "$INSTDIR\uninstall.exe"
    IfErrors remove_failed
  ReadRegStr $0 HKCU "${RUN_KEY}" "${INSTALL_ID}"
  StrCmp $0 '$\"$INSTDIR\supermenu.exe$\" --background' 0 +2
    DeleteRegValue HKCU "${RUN_KEY}" "${INSTALL_ID}"
  Push "$DESKTOP\${SHORTCUT_NAME}.lnk"
  Call un.IsOwnedShortcut
  Pop $0
  StrCmp $0 1 0 +2
    Delete "$DESKTOP\${SHORTCUT_NAME}.lnk"
  Push "$SMPROGRAMS\${SHORTCUT_NAME}\Super Menu.lnk"
  Call un.IsOwnedShortcut
  Pop $0
  StrCmp $0 1 0 +2
    Delete "$SMPROGRAMS\${SHORTCUT_NAME}\Super Menu.lnk"
  RMDir "$SMPROGRAMS\${SHORTCUT_NAME}"
  DeleteRegKey HKCU "${ARP_KEY}"
  ; No recursive deletion: foreign files and all %LOCALAPPDATA%\SuperMenu data stay.
  RMDir "$INSTDIR"
  SetErrorLevel 0
  Goto remove_done
remove_failed:
  MessageBox MB_OK|MB_ICONSTOP "$(RemoveError)" /SD IDOK
  SetErrorLevel 5
  Abort
remove_done:
SectionEnd
