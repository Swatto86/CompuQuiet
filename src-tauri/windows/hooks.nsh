; Extra steps for the Windows setup (bundle.windows.nsis.installerHooks).
;
; The setup installs for all users, into Program Files, so that an ordinary
; process cannot replace the program that the sign-in task starts with
; administrator rights. Releases up to 1.1.7 installed per user, into
; %LOCALAPPDATA%. A setup that reads only the all-users registry keys neither
; sees nor replaces that copy, so the hooks below move it out of the way.
; The data folder (%APPDATA%\CompuQuiet) is never touched.

Var PerUserDir
Var MovedFromPerUser
Var HadDesktopShortcut

!macro NSIS_HOOK_PREINSTALL
  StrCpy $MovedFromPerUser 0
  StrCpy $HadDesktopShortcut 0

  ; A per-user install records its folder under HKCU; this setup writes HKLM,
  ; so what is here belongs to the old copy.
  ReadRegStr $PerUserDir HKCU "${MANUPRODUCTKEY}" ""
  ${If} $PerUserDir != ""
  ${AndIf} $PerUserDir != $INSTDIR
  ${AndIf} ${FileExists} "$PerUserDir\uninstall.exe"
    DetailPrint "Removing the per-user copy in $PerUserDir"

    ; Its desktop shortcut, if it had one, is replaced by an all-users one
    ; after the install.
    SetShellVarContext current
    !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$PerUserDir\${MAINBINARYNAME}.exe"
    Pop $HadDesktopShortcut
    SetShellVarContext all

    ; Silent, in place (_?= keeps it from copying itself away, so ExecWait
    ; waits for it). It is not told /UPDATE, so it removes its shortcuts.
    ExecWait '"$PerUserDir\uninstall.exe" /S _?=$PerUserDir' $0
    ${If} $0 = 0
      Delete "$PerUserDir\uninstall.exe"
      RMDir "$PerUserDir"
      DeleteRegKey HKCU "${MANUPRODUCTKEY}"
      DeleteRegKey /ifempty HKCU "${MANUKEY}"
      StrCpy $MovedFromPerUser 1
    ${Else}
      DetailPrint "The per-user copy could not be removed (exit code $0) and is left in place."
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${If} $MovedFromPerUser = 1
    ; A silent update makes no shortcuts, and the removal above took the old
    ; ones with it.
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    ${If} $HadDesktopShortcut = 1
      CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
      !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
    ${EndIf}

    ; The sign-in task still names the old copy. This setup has
    ; administrator rights, so it can change even a task made with them, which
    ; the app started afterwards (without them) cannot. /Change keeps the
    ; task's run level.
    nsExec::Exec 'schtasks /Query /TN "${PRODUCTNAME}"'
    Pop $0
    ${If} $0 = 0
      nsExec::Exec 'schtasks /Change /TN "${PRODUCTNAME}" /TR "\"$INSTDIR\${MAINBINARYNAME}.exe\" --hidden"'
      Pop $0
    ${EndIf}
    nsExec::Exec 'schtasks /Query /TN "ComputeQuiet"'
    Pop $0
    ${If} $0 = 0
      nsExec::Exec 'schtasks /Change /TN "ComputeQuiet" /TR "\"$INSTDIR\${MAINBINARYNAME}.exe\" --hidden"'
      Pop $0
    ${EndIf}
  ${EndIf}

  ; A copy that was running as administrator asked for this update with
  ; /ELEVATED and without /R (update.rs). The setup's own restart starts the
  ; program as the desktop user, which would bring it back without those
  ; rights until the next sign-in. This setup is elevated, so what it starts
  ; is too. Hidden, as every restart is (cli::restart_args).
  ClearErrors
  ${GetOptions} $CMDLINE "/ELEVATED" $0
  ${IfNot} ${Errors}
  ${AndIf} $UpdateMode = 1
    Exec '"$INSTDIR\${MAINBINARYNAME}.exe" --hidden'
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; The sign-in task is made by the app, not by this setup, so it would
  ; outlive it and start nothing at every sign-in. An update keeps it, and so
  ; does a reinstall: a setup run by hand over an installed copy uninstalls it
  ; first ("uninstall before installing") by running this uninstaller in place,
  ; from the install folder, and then installs again. Someone who uninstalls
  ; runs a copy of it that NSIS makes in a temporary folder.
  ${If} $UpdateMode <> 1
  ${AndIf} $EXEDIR != $INSTDIR
    nsExec::Exec 'schtasks /Delete /F /TN "${PRODUCTNAME}"'
    Pop $0
    nsExec::Exec 'schtasks /Delete /F /TN "ComputeQuiet"'
    Pop $0
  ${EndIf}
!macroend
