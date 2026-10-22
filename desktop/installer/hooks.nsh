; Game Repo Server in both Windows installers (docs/PLAN.md, "Phase 9a design", D89):
; Game Repo's, with the server beside the app, and Game Repo Server's own. Each adds
; Windows Firewall rules for its copy of the server, for private networks only: inbound
; TCP to the program (its port can be changed in Settings) and UDP 5353 (DNS-SD, how
; devices find it). Adding a rule needs an administrator, so Windows asks once; when
; that's declined the install goes on, and Windows asks itself when the server first
; listens. Installing or removing stops that copy of the server first, and removing takes
; its rules and its Start with Windows away. (Included before the installer defines
; PRODUCTNAME, so the names are defined when a hook is used.)

!macro GR_NAMES
  !ifndef GR_RULE
    !if "${PRODUCTNAME}" == "Game Repo Server"
      !define GR_RULE "Game Repo Server"
    !else
      !define GR_RULE "Game Repo Server (with ${PRODUCTNAME})"
    !endif
    !define GR_SERVER "$INSTDIR\gamerepo-server.exe"
    !define GR_RUN "Software\Microsoft\Windows\CurrentVersion\Run"
  !endif
!macroend

; Windows keeps the icons of shortcuts and taskbar pins in a cache: asked to read them again,
; a pin made before the program's icon changed shows the new one (D143). Game Repo Go's
; installer has the same lines (go-hooks.nsh). ie4uinit isn't waited for: the installer
; needn't wait while Windows reads its icons again.
!macro GR_REFRESH_ICONS
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  Exec `"$SYSDIR\ie4uinit.exe" -show`
!macroend

; this folder's server, stopped so its file can be replaced or removed
!macro GR_STOP_SERVER
  Push $0
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "Get-Process gamerepo-server -ErrorAction SilentlyContinue | Where-Object { $$_.Path -eq '${GR_SERVER}' } | Stop-Process -Force"`
  Pop $0
  Pop $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro GR_NAMES
  !insertmacro GR_STOP_SERVER
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro GR_NAMES
  Push $0
  nsExec::Exec `netsh advfirewall firewall show rule name="${GR_RULE}"`
  Pop $0
  ${If} $0 != 0
    ExecShell "runas" "$SYSDIR\cmd.exe" `/c netsh advfirewall firewall add rule name="${GR_RULE}" dir=in action=allow program="${GR_SERVER}" protocol=TCP profile=private enable=yes & netsh advfirewall firewall add rule name="${GR_RULE}" dir=in action=allow program="${GR_SERVER}" protocol=UDP localport=5353 profile=private enable=yes` SW_HIDE
  ${EndIf}
  Pop $0
  !insertmacro GR_REFRESH_ICONS
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro GR_NAMES
  !insertmacro GR_STOP_SERVER
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro GR_NAMES
  ; (an update keeps them)
  ${If} $UpdateMode <> 1
    Push $0
    ReadRegStr $0 HKCU "${GR_RUN}" "Game Repo Server"
    ${If} $0 == `"${GR_SERVER}"`
      DeleteRegValue HKCU "${GR_RUN}" "Game Repo Server"
    ${EndIf}
    nsExec::Exec `netsh advfirewall firewall show rule name="${GR_RULE}"`
    Pop $0
    ${If} $0 == 0
      ExecShell "runas" "$SYSDIR\cmd.exe" `/c netsh advfirewall firewall delete rule name="${GR_RULE}"` SW_HIDE
    ${EndIf}
    Pop $0
  ${EndIf}
!macroend
