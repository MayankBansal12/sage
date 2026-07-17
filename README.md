# Sage

Linux voice-to-text CLI:

```text
sage start → speak → sage stop → transcript
```

**Through Phase 2:** recording + local Whisper transcription.

## Build deps (Debian/Ubuntu)

```bash
sudo apt-get install -y pkg-config libasound2-dev cmake clang libclang-dev
```

## Build

```bash
source "$HOME/.cargo/env"   # if needed
cd /path/to/sage
cargo build --release
```

## Setup (once)

```bash
./target/release/sage model pull    # downloads ggml-tiny.bin (~77MB)
./target/release/sage model path
```

## Run

```bash
./target/release/sage start     # needs a microphone
# speak…
./target/release/sage stop      # saves WAV + prints transcript
./target/release/sage last      # print last transcript
./target/release/sage status
```

Transcribe an existing WAV:

```bash
./target/release/sage transcribe /path/to/file.wav
```

## Test

```bash
cargo test
```

## Paths

| Path | Purpose |
|------|---------|
| `~/.local/share/sage/models/ggml-tiny.bin` | Default model |
| `~/.local/share/sage/recordings/` | WAV files |
| `~/.local/share/sage/run/state.json` | Session + last transcript |

Overrides: `SAGE_DATA_DIR`, `SAGE_MODEL_PATH`, `SAGE_CONFIG_DIR`, `SAGE_TEMP_DIR`.

## CLI

| Command | Description |
|---------|-------------|
| `start` / `stop` | Record then transcribe |
| `last` | Last transcript |
| `status` | Session state |
| `transcribe <wav>` | Transcribe file only |
| `model pull` / `model path` | Download / locate model |
| `paths` | Print all dirs |

## License

MIT
