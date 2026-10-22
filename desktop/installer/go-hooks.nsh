; Game Repo Go's Windows installer (docs/PLAN.md, D143): Windows keeps the icons of
; shortcuts and taskbar pins in a cache, so after installing it's asked to read them again,
; and a pin made before the program's icon changed shows the new one. (Game Repo's and Game
; Repo Server's installers do the same in hooks.nsh.)

!macro NSIS_HOOK_POSTINSTALL
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ; not waited for: the installer needn't wait while Windows reads its icons again
  Exec `"$SYSDIR\ie4uinit.exe" -show`
!macroend
