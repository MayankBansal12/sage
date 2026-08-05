# Sage

Fast, offline Linux voice-to-text with target-aware delivery:

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
sage model list
sage model use parakeet-unified-en-0.6b
sage doctor
```

`model use` downloads, verifies, and selects the model in one command. Use
`--language <code>` for an explicit language when a model supports more than
one, for example `sage model use canary-180m-flash --language de`.

## Speech-to-text models

Sage ships an offline catalog rather than downloading a live model list. Model
files come from pinned Hugging Face revisions and are verified with SHA-256
before they become active.

| Model | Best for | Download |
| --- | --- | ---: |
| `parakeet-unified-en-0.6b` | Fast, accurate English (recommended) | 697 MiB |
| `canary-180m-flash` | Fast English, German, Spanish, or French | 208 MiB |
| `whisper-medium` | Reliable automatic detection across 99 languages | 793 MiB |
| `moonshine-streaming-tiny` | Smallest and fastest English option | 48 MiB |
| `qwen3-asr-0.6b` | Accurate automatic detection across 30 languages | 811 MiB |

```bash
sage model list
sage model info qwen3-asr-0.6b
sage model use qwen3-asr-0.6b
sage model current
sage model pull whisper-medium   # download without selecting
```

The first release uses the portable CPU backend. Models advertising streaming
support still run in Sage's record-then-transcribe flow; live preview and GPU
acceleration are intentionally deferred. See [MODEL_LICENSES.md](MODEL_LICENSES.md)
for model licenses and attribution.

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
sage config set language auto
sage config set stt_threads 0
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
