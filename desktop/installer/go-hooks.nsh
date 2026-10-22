; Game Repo Go's Windows installer (docs/PLAN.md, D143): Windows keeps the icons of
; shortcuts and taskbar pins in a cache, so after installing it's asked to read them again,
; and a pin made before the program's icon changed shows the new one. (Game Repo's and Game
; Repo Server's installers do the same in hooks.nsh.)

!macro NSIS_HOOK_POSTINSTALL
  ; Windows' own program, not waited for and not the installer's own call (SHChangeNotify),
  ; as either could hang the installer where no one is signed in (an automated install)
  Exec `"$SYSDIR\ie4uinit.exe" -show`
!macroend
