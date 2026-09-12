# Whisper embedding: whisper-rs embedded vs whisper.cpp sidecar

Resolved by subagent research (R1, uses primary sources). Full conversation summary + sources below.

## Decision

**Embed `whisper-rs`** in the Rust core. Deliver the transcriber behind a small `Transcriber` trait seam (Tauri command) so a sidecar can slot in later at near-zero cost.

Fallback triggers (switch to a `whisper-cli` sidecar pulled from whisper.cpp prebuilt release tarballs):
1. Recurring Windows bindgen/libclang CI failures despite `WHISPER_DONT_GENERATE_BINDINGS=1`.
2. Need to hotfix/pin whisper.cpp version or swap engines (faster-whisper, Parakeet) without re-releasing the core.
3. Future batch-transcription feature where per-clip cold start is irrelevant.

Both paths consume identical GGML `.bin` files and 16 kHz input, so the seam is cheap to build now.

## Key facts

- **Build**: whisper-rs vendors whisper.cpp and builds it with CMake in a build script (`whisper-rs-sys`). Static link of whisper/ggml → single binary (one code-signing surface). Per-OS: Windows/MSVC needs VS C++ + CMake + LLVM/clang for bindgen (escape hatch: bundled `bindings.rs` + `WHISPER_DONT_GENERATE_BINDINGS`); macOS needs Xcode CLT + cmake (Accelerate linked automatically); Linux "just works".
- **Portability — critical**: ggml defaults to `GGML_NATIVE=ON` (→ `-march=native`), so production/CI builds MUST set `GGML_NATIVE=OFF` for portable binaries (AVX2/FMA etc. still enabled, runtime-dispatched). whisper-rs-sys passes any `GGML_*`/`WHISPER_*` env var through to CMake.
- **Metal**: forced OFF unless the `metal` feature is opted in; default CPU-only builds are fine.
- **Inference perf** (community benchmarks, whisper.cpp #89 + blog reports):
  - `base.en`: RTF ~0.10–0.16 ≈ 1–2 s per 10 s clip on 2020-era mid laptops; instant on Apple Silicon.
  - `small.en`: RTF ~0.30–0.70 ≈ 3–7 s per 10 s clip on mid-tier x86 laptops; ~0.15 RTF on M1/M2 CPU.
  - Ultra-low-power 4-core (Intel N97) fails to hit real-time even on base — the failure hardware class.
- **Memory** (fp16 files, whisper.cpp table): tiny 75 MiB/~273 MB · base 142 MiB/~388 MB · small 466 MiB/~852 MB · medium 1.5 GiB/~2.1 GB.
- **Threading**: >8 threads doesn't help (memory-bound); 4→8 ≈ 1.5–2×.
- **Input**: whisper-rs takes f32 samples at 16 kHz mono directly (Vec<f32>). Sidecar would need 16-bit WAV files on disk → extra encode step. Embedded wins on ergonomics.
- **Callbacks**: `set_progress_callback_safe`, `set_segment_callback_safe`, `set_abort_callback_safe` — progress/segments/cancel from Rust; sidecar would have to scrape CLI stderr.
- **Warm context**: keep the `WhisperContext` alive across clips — model load dominates CLI wall time on short clips (~0.2–0.6 s tax per clip avoided).
- **macOS signing**: embedded = single signed binary; sidecars risk the "resources-copy destroys pre-existing signature" trap and add one extra binary per target triple to sign.
- **Precedent**: shipped Tauri STT apps embed whisper-rs — spiel (push-to-talk plugin, closest shape), echo, Taurscribe, WhisperDesk; audiox uses a sidecar.

## Sources

- whisper-rs repo + BUILDING.md + sys build.rs (codeberg tazz4843/whisper-rs): https://github.com/tazz4843/whisper-rs
- whisper.cpp ggml CMakeLists `GGML_NATIVE` default: https://github.com/ggml-org/whisper.cpp/blob/master/ggml/CMakeLists.txt
- whisper.cpp README (memory usage table, 16 kHz convention): https://github.com/ggml-org/whisper.cpp
- Whisper CPU benchmark collection: https://github.com/ggerganov/whisper.cpp/issues/89
- M2 CPU/Metal benchmark: https://getspeakup.app/blog/whisper-cpp-benchmark-mac/
- Small on i5-1240P: https://snailtext.app/blog/how-whisper-cpp-works/
- Model-load-dominates-short-clips: https://allenkuo.medium.com/choosing-a-real-time-whisper-engine-c4eeb5885e22
- Prebuilt whisper.cpp binaries (for sidecar alternative): https://github.com/ggml-org/whisper.cpp/releases/tag/b5130
- Tauri v2 sidecar: https://v2.tauri.app/develop/sidecar/ · macOS signing: https://v2.tauri.app/distribute/sign/macos/
- Precedent apps: https://github.laiyagushi.com/robertdevore/spiel · https://github.com/c3ulnta0rk/echo · https://github.com/stormlightlabs/audiox