# Sage phases

## Phase 0 — Skeleton ✅

| Task | Done |
|------|------|
| 0.1 Cargo binary `sage` | ✅ |
| 0.2 CLI surface | ✅ |
| 0.3 Modules | ✅ |
| 0.4 Paths | ✅ |

## Phase 1 — Recording ✅

| Task | Done |
|------|------|
| 1.1 Mic capture | ✅ |
| 1.2 WAV on stop | ✅ |
| 1.3 Idle \| Recording | ✅ |
| 1.4 status | ✅ |

## Phase 2 — Transcription ✅

| Task | Done |
|------|------|
| 2.1 Default model path (`models/ggml-tiny.bin`) | ✅ |
| 2.2 Load via whisper-rs; clear missing-model error | ✅ |
| 2.3 `transcribe(path) -> String` | ✅ |
| 2.4 `stop` → transcribe → store + print | ✅ |
| 2.5 `last` prints transcript | ✅ |
| 2.6 `model pull` / `model path` / `transcribe <wav>` | ✅ |

## Next

- Phase 3: richer history persistence  
- Phase 4: clipboard  
- Phase 5: global hotkey + daemon  
