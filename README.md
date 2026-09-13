# Voxa

A lightweight cross-platform desktop dictation app in the image of Wispr Flow — but local.

Hold a hotkey, speak, release: Voxa transcribes with on-device Whisper, polishes the text with a Groq-powered cleanup pass, and inserts it into whatever app has your cursor. A floating pill shows honest mic-level metering while you talk.

**Status: in build.** The v1 build is underway against the [buildable spec](https://github.com/AmanatAliPanhwer/Voxa/issues/16); a Tauri 2 skeleton with the `session` actor, the pill projection, and stubbed capture/transcribe/insert stages lands first on `main`, then real hotkey arbitration, mic capture, transcription (embedded whisper-rs), the insertion loop, Groq cleanup, Settings, and onboarding. See the issue tracker for the ticket graph.

- Vocabulary: `CONTEXT.md`. Architecture and event contracts: `docs/adr/`.
- Build: `cd src-tauri && cargo build` (needs the Tauri toolchain; MSVC + WebView2 on Windows).