!macro NSIS_HOOK_PREINSTALL
  ReadRegDWORD $0 HKLM "SOFTWARE\Microsoft\Windows NT\CurrentVersion" "CurrentMajorVersionNumber"
  IntCmp $0 10 check_build unsupported check_build
  check_build:
  ReadRegStr $1 HKLM "SOFTWARE\Microsoft\Windows NT\CurrentVersion" "CurrentBuildNumber"
  IntCmp $1 17763 supported unsupported supported
  unsupported:
    MessageBox MB_ICONSTOP|MB_OK "Linh requires Windows 10 version 1809 or later."
    Abort
  supported:
!macroend
