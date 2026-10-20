; NSIS hooks for the Tauri installer (bundle.windows.nsis.installerHooks).
;
; What the stock template already does, and this app relies on:
; - install: Start menu shortcut $SMPROGRAMS\feathertune.lnk carrying the AppUserModelID
;   dev.feathertune.desktop (the bundle identifier, same as smtc.rs APP_ID), so the media flyout
;   shows the app name;
; - uninstall: removes that shortcut and the "Start with Windows" entry (Run key "feathertune");
; - uninstall: a "delete the application data" checkbox, which only clears the folders named after
;   the bundle identifier. The profile lives in %LOCALAPPDATA%\feathertune (main.rs profile_dir),
;   so it is cleared here, and only when the user ticked the checkbox.

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    ; WebView2 profile (sign-in), config.toml, Last.fm queue.
    RmDir /r "$LOCALAPPDATA\feathertune"
    ; Last.fm session key: Credential Manager entry "<user>.<service>" (lastfm.rs KEYRING_*).
    ; Harmless if it does not exist.
    nsExec::Exec 'cmdkey /delete:lastfm-session.feathertune'
    Pop $0
  ${EndIf}
!macroend
