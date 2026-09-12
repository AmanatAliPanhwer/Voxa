# Whisper model distribution (GGML)

Resolved by subagent research (R4). Spec the first-run downloader against this.

## Model catalog (fp16 `ggml-*.bin`; all MIT)

| File | Disk | Peak RAM (CPU, whisper.cpp) |
|---|---|---|
| `ggml-tiny.en.bin` | ~74 MiB | ~273 MB |
| `ggml-base.en.bin` | ~141 MiB | ~388 MB |
| `ggml-small.en.bin` | ~465 MiB | ~852 MB |
| `ggml-medium.en.bin` | ~1.43 GiB | ~2.1 GB |
| `ggml-large-v3.bin` | ~2.88 GiB | ~3.9 GB |
| `ggml-large-v3-turbo.bin` | ~1.51 GiB | ~2.2 GB (est.) |

Quantized variants also exist (`.en` use `q5_1`/`q8_0`): `base.en-q8_0` ~78 MiB, `small.en-q8_0` ~252 MiB, `small.en-q5_1` ~181 MiB. Q8_0 is effectively lossless for EN dictation (WER equal to fp16); Q5 drops ~0.3 WER on large. **English-only dictation: prefer `.en` models.**

Default pick per Q12: **`small.en`** (~465 MB / ~852 MB RAM); `tiny.en`/`base.en` for low-RAM/low-disk; `large-v3` behind a "≥3 GB disk, ~4 GB RAM" warning. Installer must NOT bundle a model.

## Sources & license

- Canonical mirror: HuggingFace repo `ggerganov/whisper.cpp`, stable scheme `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-<name>.bin` (302 → HF CDN; follow redirects).
- Machine-readable catalog incl. **LFS SHA-256**: `https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/main` blocks.
- License: OpenAI Whisper code **and weights** MIT; ggml conversions MIT. Redistribution allowed incl. commercial; if mirroring, keep an attribution note ("Whisper © OpenAI, MIT; ggml conversion by whisper.cpp, MIT") in an About/Licenses screen, and note GitHub Releases caps single assets at 2 GiB (large-v3 won't fit; small.en and below will).

## Download UX mechanics (spec)

- **Downloader**: Rust `reqwest` streaming command; emit progress (`bytes_downloaded/total` + speed/eta) via `tauri::Emitter`; pill shows progress during first-run model download.
- **Resume**: stream to `<model>.bin.part`, `Range: bytes=<offset>-` after the redirect, atomic rename on checksum match. On checksum mismatch: delete `.part` and restart from 0 (never load a failed-verification model).
- **Checksums**: pin SHA-256 in a small in-repo `models.json` catalog (name, display size, RAM, resolve URL, sha256); optionally pin the HF commit SHA in the URL (files are static since 2024-10).
- **Cache dirs** (Tauri v2 path API, keyed on `bundleIdentifier` e.g. `com.voxa.app`):
  - Windows: `%APPDATA%\com.voxa.app\models\`
  - macOS: `~/Library/Application Support/com.voxa.app/models/`
  - Linux: `$XDG_DATA_HOME` or `~/.local/share/com.voxa.app/models/`
- **Failure behavior**: app launches fine without a model; dictation commands return a typed "No model installed" error; Settings shows a Retry button; never block app start on download.
- **Gotchas**: HF Resolver has the highest quota but can 429 (exponential backoff + respect `RateLimit-Reset`); CDN redirects may be blocked by some networks → surface a clear "your network blocks HuggingFace's CDN" message; deleting a model must refuse while it's loaded and clean up `.part`.

## Sources

- whisper.cpp models README (hashes, download script, HF mirror): https://raw.githubusercontent.com/ggerganov/whisper.cpp/master/models/README.md
- Memory usage table: https://raw.githubusercontent.com/ggerganov/whisper.cpp/master/README.md#memory-usage
- OpenAI Whisper license (MIT, weights): https://raw.githubusercontent.com/openai/whisper/main/README.md
- HF repo metadata + tree API: https://huggingface.co/api/models/ggerganov/whisper.cpp
- HF download/resolve endpoints + rate limits: https://huggingface.co/docs/hub/en/models-downloading · https://huggingface.co/docs/hub/rate-limits
- Tauri path API: https://v2.tauri.app/reference/javascript/api/namespacepath/
- Quantization vs WER: https://gist.github.com/in0vik/8cd17a3bc51d88f740b8a7c190ea154f · https://arxiv.org/abs/2503.09905
- GitHub Releases asset limit (2 GiB): GitHub docs "About releases"