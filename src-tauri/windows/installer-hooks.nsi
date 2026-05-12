; Custom NSIS hooks invoked by the Tauri-generated installer.
;
; The only thing we add on top of Tauri's default flow is cleaning
; up wintun.dll that gets staged next to v2rayV.exe at runtime.
;
; Background: xray-core's wintun backend calls LoadLibrary("wintun.dll"),
; and Windows' loader searches the calling exe's directory first, so the
; DLL must sit right next to v2rayV.exe / xray.exe. The app stages it
; there on first launch via `ensure_wintun_next_to_exe` in src/lib.rs by
; copying from `<install_dir>\resources\binaries\wintun.dll`. NSIS only
; tracks files it installed itself, so the staged copy survives a normal
; uninstall and leaves a stray DLL in $INSTDIR. These hooks delete it
; explicitly before and after the standard uninstall pass, and also
; remove the install directory if it ends up empty.

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\wintun.dll"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Belt-and-braces: if for any reason the pre-uninstall delete left
  ; the file behind (e.g. it was in use), try once more after the
  ; rest of the app has been removed.
  Delete "$INSTDIR\wintun.dll"
  RMDir "$INSTDIR"
!macroend
