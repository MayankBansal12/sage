# Sage

Queued Linux voice-to-text with target-aware delivery:

```text
focus a text field → hotkey → speak → hotkey → transcript returns to that field
```

## Build

```bash
sudo apt-get install -y pkg-config libasound2-dev cmake clang libclang-dev
source "$HOME/.cargo/env"
cd sage
cargo build --release
./scripts/install.sh          # optional: ~/.local/bin/sage
```

## Setup

```bash
sage model pull
sage doctor
```

## Core usage

```bash
sage start
# speak…
sage stop          # transcribe + clipboard (default)
sage toggle        # recommended global-shortcut command
sage last
sage copy          # re-copy last transcript
sage cancel        # abort recording
```

Offline file:

```bash
sage transcribe /path/to/audio.wav
```

## Config & history

```bash
sage config show
sage config set copy_to_clipboard true
sage config set auto_paste false
sage shortcut set ctrl+shift+r
sage history list
sage pending
sage paste-next
```

## Daemon

```bash
sage daemon          # foreground daemon
sage daemon-status
sage daemon-stop
```

On GNOME Wayland, Sage requests the configured shortcut through the Global
Shortcuts portal. If the desktop does not expose that portal, bind a desktop or
compositor shortcut to `sage toggle`; the command talks to the same daemon.

Sage captures the focused AT-SPI editable component when recording begins.
Transcription runs in a durable background queue. When that component regains
focus, ready transcripts are inserted in recording order. Ambiguous,
unsupported, closed, or expired targets remain available through `sage pending`
and `sage paste-next`.

Native GTK/Qt controls are the primary v1 target. Browser, Electron, terminal,
KDE Wayland, and X11 behavior is best-effort until dedicated adapters are added.

## Package

```bash
./scripts/package.sh     # dist/sage-*-linux-x86_64.tar.gz
```

## License

MIT
