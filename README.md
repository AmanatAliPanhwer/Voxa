# Voxa

A lightweight cross-platform desktop dictation app in the image of Wispr Flow — but local.

Hold a hotkey, speak, release: Voxa transcribes with on-device Whisper, polishes the text with a Groq-powered cleanup pass, and inserts it into whatever app has your cursor. A floating pill shows honest mic-level metering while you talk.

**Status: in build.** The v1 build is underway against the [buildable spec](https://github.com/AmanatAliPanhwer/Voxa/issues/16); a Tauri 2 skeleton with the `session` actor, the pill projection, and stubbed capture/transcribe/insert stages lands first on `main`, then real hotkey arbitration, mic capture, transcription (embedded whisper-rs), the insertion loop, Groq cleanup, Settings, and onboarding. See the issue tracker for the ticket graph.

- Vocabulary: `CONTEXT.md`. Architecture and event contracts: `docs/adr/`.
- Build: `cd src-tauri && cargo build` (needs the Tauri toolchain; MSVC + WebView2 on Windows).

## Download & install

Releases are published as **unsigned** draft builds — engines are built fresh on CI, and no code-signing certificate is in place yet. Download the installer that matches your OS from the latest release, then handle the platform warning once:

- **Windows** (`*-setup.exe`, NSIS, per-user install): "Windows protected your PC" → *More info* → *Run anyway*. Very new, zero-reputation binaries can also trip Defender — report via the Microsoft WDSI submission form.
- **macOS** (`*.dmg`, one per architecture — Apple Silicon and Intel): right-click (control-click) the app → *Open* → *Open Anyway*, or System Settings → *Privacy & Security* → *Open Anyway*. Pick the `aarch64` DMG on Apple Silicon, `x64` on Intel.
- **Linux** (`*.deb` for Debian/Ubuntu, `*.AppImage` for everything else): no friction. AppImage needs FUSE, or use the extract-and-run fallback if your distro ships `fuse3` only.

`cargo install tauri-cli && cargo tauri build` also produces local installers from source.