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

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Classes\LinhURL"
  DeleteRegKey HKCU "Software\Linh\Capabilities"
  DeleteRegKey /ifempty HKCU "Software\Linh"
  DeleteRegValue HKCU "Software\RegisteredApplications" "Linh"
!macroend

; An upgrade whose shortcuts already target the current MAINBINARYNAME.exe
; (i.e. every upgrade, since the exe name hasn't changed) never reaches
; installer.nsi's SetLnkAppUserModelId call: CreateOrUpdateStartMenuShortcut /
; CreateOrUpdateDesktopShortcut only set the AUMID when creating a shortcut or
; migrating one from an old binary name, and return early otherwise (see
; tauri-bundler 2.10.4's installer.nsi:959-1010). So a shortcut created under
; the old dev.tkhieu.linh identifier keeps that stale AUMID after upgrading to
; dev.hieutk.linh, and Windows toast notifications (keyed by the current
; BUNDLEID) won't show or will be misattributed for it. Re-apply the AUMID
; unconditionally here, mirroring installer.nsi's own shortcut paths
; (STARTMENUFOLDER branch at :991-993, desktop at :1021) and its
; SetLnkAppUserModelId macro (utils.nsh:78). Idempotent: re-setting the AUMID
; on an already-correct shortcut is a no-op.
!macro NSIS_HOOK_POSTINSTALL
  !if "${STARTMENUFOLDER}" != ""
    IfFileExists "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" 0 win_hieutk_skip_start
      !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
    win_hieutk_skip_start:
  !else
    IfFileExists "$SMPROGRAMS\${PRODUCTNAME}.lnk" 0 win_hieutk_skip_start
      !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    win_hieutk_skip_start:
  !endif

  IfFileExists "$DESKTOP\${PRODUCTNAME}.lnk" 0 win_hieutk_skip_desktop
    !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
  win_hieutk_skip_desktop:
!macroend
