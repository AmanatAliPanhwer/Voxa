# Voxa

A lightweight cross-platform desktop dictation app in the image of Wispr Flow — but local.

Hold a hotkey, speak, release: Voxa transcribes with on-device Whisper, polishes the text with a Groq-powered cleanup pass, and inserts it into whatever app has your cursor. A floating pill shows honest mic-level metering while you talk.

**Status: v1 Complete (Hand-off Ready).** The v1 build is complete against the [buildable spec](https://github.com/AmanatAliPanhwer/Voxa/issues/16) and verified against the [Definition-of-Done Acceptance Matrix](docs/ACCEPTANCE.md) ([#27](https://github.com/AmanatAliPanhwer/Voxa/issues/27)). All architectural contracts (ADR-0001 through ADR-0004) and platform slices (hotkeys, capture, transcription, insertion loop, Groq cleanup, Settings, and onboarding wizard) are delivered and tested.

- Vocabulary: `CONTEXT.md`. Architecture and event contracts: `docs/adr/`.
- Acceptance & Verification Matrix: `docs/ACCEPTANCE.md`.
- Build: `cd src-tauri && cargo build` (needs the Tauri toolchain; MSVC + WebView2 on Windows).

## Download & install

Releases are published as **unsigned** draft builds — engines are built fresh on CI, and no code-signing certificate is in place yet. Download the installer that matches your OS from the latest release, then handle the platform warning once:

- **Windows** (`*-setup.exe`, NSIS, per-user install): "Windows protected your PC" → *More info* → *Run anyway*. Very new, zero-reputation binaries can also trip Defender — report via the Microsoft WDSI submission form.
- **macOS** (`*.dmg`, one per architecture — Apple Silicon and Intel): right-click (control-click) the app → *Open* → *Open Anyway*, or System Settings → *Privacy & Security* → *Open Anyway*. Pick the `aarch64` DMG on Apple Silicon, `x64` on Intel.
- **Linux** (`*.deb` for Debian/Ubuntu, `*.AppImage` for everything else): no friction. AppImage needs FUSE, or use the extract-and-run fallback if your distro ships `fuse3` only.

`cargo install tauri-cli && cargo tauri build` also produces local installers from source.