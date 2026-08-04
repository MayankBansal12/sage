# Sage phases

| Phase | Status |
|-------|--------|
| 0 Skeleton | ✅ |
| 1 Recording | ✅ |
| 2 Transcription | ✅ |
| 3 History persistence | ✅ |
| 4 Clipboard | ✅ |
| 5 Single-owner daemon + Unix socket | ✅ |
| 6 Auto-paste (best-effort) | ✅ |
| 7 Cancel, sounds, discard, doctor | ✅ |
| 8 Modular ASR + TOML config | ✅ |
| 9 Settings via `sage config` (CLI UI) | ✅ |
| 10 Package tarball + install.sh | ✅ |
| 11 SQLite job queue + restart recovery | ✅ |
| 12 Streaming recording | ✅ |
| 13 Wayland Global Shortcuts portal | ✅ |
| 14 AT-SPI target capture + paste-on-focus | ✅ |

## Notes

- **Hotkeys:** GNOME Wayland uses the Global Shortcuts portal. Other desktops
  can bind `sage toggle` as a compositor/desktop shortcut.
- **Delivery:** AT-SPI editable controls receive pending transcripts when the
  captured target regains focus. Unsupported targets fall back to pending/manual
  clipboard delivery.
- **GUI:** full GTK tray deferred; config CLI is the settings surface.
- **AppImage:** tarball primary; AppImage when `appimagetool` available.
