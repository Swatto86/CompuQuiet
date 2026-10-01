; Extra steps for the Windows setup (bundle.windows.nsis.installerHooks).
;
; The setup installs for all users, into Program Files, so that an ordinary
; process cannot replace the program that the sign-in task starts with
; administrator rights. Releases up to 1.1.7 installed per user, into
; %LOCALAPPDATA%. A setup that reads only the all-users registry keys neither
; sees nor replaces that copy, so the hooks below move it out of the way.
; The data folder (%APPDATA%\CompuQuiet) is never touched.
;
; The setup has administrator rights, while the old copy, its registry key and
; its uninstaller can all be rewritten by an ordinary process. So nothing found
; there is ever run: the copy is deleted file by file, and only from the folder
; a per-user setup picks by default.

Var PerUserDir
Var MovedFromPerUser
Var HadDesktopShortcut
Var ProgramFilesLength
Var InstallHead

!macro NSIS_HOOK_PREINSTALL
  StrCpy $MovedFromPerUser 0
  StrCpy $HadDesktopShortcut 0

  ; A per-user install records its folder under HKCU; this setup writes HKLM,
  ; so what is here belongs to the old copy.
  ReadRegStr $PerUserDir HKCU "${MANUPRODUCTKEY}" ""
  ${If} $PerUserDir != ""
  ${AndIf} $PerUserDir != $INSTDIR
  ${AndIf} ${FileExists} "$PerUserDir\uninstall.exe"
    SetShellVarContext current
    ${If} $PerUserDir == "$LOCALAPPDATA\${PRODUCTNAME}"
      DetailPrint "Removing the per-user copy in $PerUserDir"

      ; It may be running, and the setup's own check comes after this hook.
      !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

      ; The folder stays if anything else is in it, as the old uninstaller
      ; left it.
      Delete "$PerUserDir\${MAINBINARYNAME}.exe"
      Delete "$PerUserDir\uninstall.exe"
      ${IfNot} ${FileExists} "$PerUserDir\${MAINBINARYNAME}.exe"
      ${AndIfNot} ${FileExists} "$PerUserDir\uninstall.exe"
        RMDir "$PerUserDir"

        ; Its shortcuts go too. The desktop one, if it had one, is replaced by
        ; an all-users one after the install.
        !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$PerUserDir\${MAINBINARYNAME}.exe"
        Pop $0
        ${If} $0 = 1
          !insertmacro UnpinShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk"
          Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
        ${EndIf}
        !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$PerUserDir\${MAINBINARYNAME}.exe"
        Pop $HadDesktopShortcut
        ${If} $HadDesktopShortcut = 1
          !insertmacro UnpinShortcut "$DESKTOP\${PRODUCTNAME}.lnk"
          Delete "$DESKTOP\${PRODUCTNAME}.lnk"
        ${EndIf}

        DeleteRegKey HKCU "${UNINSTKEY}"
        DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
        DeleteRegKey HKCU "${MANUPRODUCTKEY}"
        DeleteRegKey /ifempty HKCU "${MANUKEY}"
        StrCpy $MovedFromPerUser 1
      ${Else}
        DetailPrint "The per-user copy could not be removed and is left in place."
      ${EndIf}
    ${Else}
      DetailPrint "The per-user copy in $PerUserDir is left in place."
      ${If} $UpdateMode <> 1
        MessageBox MB_OK|MB_ICONINFORMATION "An older per-user copy of ${PRODUCTNAME} is installed in $PerUserDir. This setup does not run programs from there, so it is left as it is.$\r$\n$\r$\nUninstall it from Settings, Apps when this setup has finished." /SD IDOK
      ${EndIf}
    ${EndIf}
    SetShellVarContext all
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
  ${EndIf}

  ; A sign-in task that names another copy (the per-user one removed above,
  ; or one removed by an earlier run of a setup) starts nothing. This setup
  ; has administrator rights, so it can change even a task made with them,
  ; which the app started afterwards (without them) cannot. Only from Program
  ; Files, where no ordinary process can replace what such a task starts; the
  ; app itself re-points one elsewhere, without those rights. Set-ScheduledTask
  ; keeps the task's account, sign-in type and run level; `schtasks /Change`
  ; would ask for the account's password, which nobody is there to type.
  StrLen $ProgramFilesLength "$PROGRAMFILES64\"
  StrCpy $InstallHead "$INSTDIR\" $ProgramFilesLength
  ${If} $InstallHead == "$PROGRAMFILES64\"
    nsExec::Exec `"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "try { Get-ScheduledTask -TaskName '${PRODUCTNAME}','ComputeQuiet' -ErrorAction SilentlyContinue | ForEach-Object { Set-ScheduledTask -TaskName $$_.TaskName -TaskPath $$_.TaskPath -Action (New-ScheduledTaskAction -Execute '$INSTDIR\${MAINBINARYNAME}.exe' -Argument '--hidden') -ErrorAction Stop | Out-Null }; exit 0 } catch { exit 1 }"`
    Pop $0
    ${If} $0 != 0
      DetailPrint "The sign-in task could not be pointed at this copy ($0)."
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
