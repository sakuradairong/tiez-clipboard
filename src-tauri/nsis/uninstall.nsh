!include "LogicLib.nsh"
!define TIEZ_CLIPBOARD_RESTORE_SCRIPT "${__FILEDIR__}\restore-clipboard-settings.ps1"

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Stopping TieZ before uninstall..."

  # 应用关闭主窗口时会缩到托盘，卸载器不能依赖普通的关闭请求。
  # 这里在开始删除文件前强制结束已安装的进程，避免 exe 被占用。
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /F /T /IM TieZ.exe'
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /F /T /IM tiez-app.exe'
  Sleep 1200

  # 更新仍会使用卸载钩子；保留自启动设置供新版本继续使用。
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "TieZ"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "tie-z"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "tiez-app"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    # Restore only values this app changed, while they still match its last write.
    # Missing snapshots and subsequent user/policy changes are left untouched.
    DetailPrint "Restoring TieZ-owned Windows Clipboard settings..."
    InitPluginsDir
    File /oname=$PLUGINSDIR\restore-clipboard-settings.ps1 "${TIEZ_CLIPBOARD_RESTORE_SCRIPT}"
    nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "$PLUGINSDIR\restore-clipboard-settings.ps1"'

    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "TieZ"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "tie-z"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "tiez-app"
    DeleteRegKey HKCU "Software\tiez\TieZ"
    DeleteRegKey /ifempty HKCU "Software\tiez\ClipboardSettingsBackup"
    DeleteRegKey /ifempty HKCU "Software\tiez"

    # NSIS removes installed files. Preserve portable data and all other user files.
  ${EndIf}
!macroend
