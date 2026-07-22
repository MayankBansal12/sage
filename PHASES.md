# Sage phases

| Phase | Status |
|-------|--------|
| 0 Skeleton | ✅ |
| 1 Recording | ✅ |
| 2 Transcription | ✅ |
| 3 History persistence | ✅ |
| 4 Clipboard | ✅ |
| 5 Hotkey processor + daemon | ✅ |
| 6 Auto-paste (best-effort) | ✅ |
| 7 Cancel, sounds, discard, doctor | ✅ |
| 8 Modular ASR + TOML config | ✅ |
| 9 Settings via `sage config` (CLI UI) | ✅ |
| 10 Package tarball + install.sh | ✅ |

## Notes

- **Hotkeys:** pure state machine + daemon stdin control; OS-level global grab is env-specific (use compositor binds → `sage start/stop` or daemon stdin).
- **GUI:** full GTK tray deferred; config CLI is the settings surface.
- **AppImage:** tarball primary; AppImage when `appimagetool` available.
