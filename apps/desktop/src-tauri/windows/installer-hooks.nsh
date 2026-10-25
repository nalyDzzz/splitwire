; Registers the background service on install and removes it on uninstall.
; See https://v2.tauri.app/distribute/windows-installer/#installer-hooks

!macro NSIS_HOOK_PREINSTALL
  ; Stop a running service from a previous install so its files can be replaced.
  IfFileExists "$INSTDIR\engine\splitwire-service.exe" 0 +2
    nsExec::Exec '"$INSTDIR\engine\splitwire-service.exe" stop'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToLog '"$INSTDIR\engine\splitwire-service.exe" install'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog '"$INSTDIR\engine\splitwire-service.exe" uninstall'
!macroend
