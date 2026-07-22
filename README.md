# Sage

Linux voice-to-text:

```text
sage start → speak → sage stop → text (+ clipboard)
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
```

## Daemon

```bash
sage daemon          # stdin: start|stop|cancel|status|quit
sage daemon-status
```

Point compositor hotkeys at `sage start` / `sage stop`, or pipe commands into the daemon.

## Package

```bash
./scripts/package.sh     # dist/sage-*-linux-x86_64.tar.gz
```

## License

MIT
