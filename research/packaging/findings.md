# Per-OS packaging & release (unsigned, OSS, whisper-rs backend)

Resolved by subagent research (R6). Spec distribution against this.

## Decision (v1)

- **Windows**: **NSIS only**, `installMode: currentUser` (no admin/UAC, avoids VBSCRIPT+WiX fragility). MSI only when enterprise/GPO demand appears.
- **macOS**: **two separate DMGs** — `aarch64-apple-darwin` and `x86_64-apple-darwin` as two matrix jobs. No universal (lipo) binary in v1: Tauri v2 removed the v1 convenience; manual lipo doubles the C transcribe build for no launch-day benefit. Notarization deferred (needs paid Apple Developer Program).
- **Linux**: **deb + AppImage** (+ rpm optional; rpm = RHEL/Fedora). AppImage is the distro-agnostic "download and run" option — note FUSE runtime variance (Ubuntu 22.04+ needs `libfuse2`; AppImage has an extract-and-run fallback).
- **Auto-update**: Tauri's minisign updater works unsigned (its own keypair), but every update re-triggers Gatekeeper/SmartScreen per-release → point users at Releases for v1; "Check for Updates" behind a flag until certs land.
- **Icons**: `pnpm tauri icon <1024x1024-source.png>` generates `.ico`/`.icns`/PNG set for `src-tauri/icons/`; swap Tauri's placeholder before first release.

## Unsigned user-facing remediation (spec copy-paste)

- **Windows**: "Windows protected your PC" → More info → Run anyway. Zero-reputation apps can also catch Defender/AV false positives; report via Microsoft WDSI submission (a known issue for fresh Rust-built binaries).
- **macOS**: Gatekeeper: control-click → Open → Open Anyway, or System Settings → Privacy & Security → Open Anyway. (Community `xattr -dr com.apple.quarantine` is a power-user path; Apple documents the two GUI routes.)
- **Linux**: no friction.

## GitHub Actions shape

- Matrix: `ubuntu-latest` (or `ubuntu-24.04`), `windows-latest`, `macos-15` + optional `--target aarch64-apple-darwin` entry. **`macos-latest` now resolves to macOS 26** — pin explicit labels.
- `dtolnay/rust-toolchain@stable` + `swatinem/rust-cache@v2` (workspaces: `./src-tauri -> target`), `actions/setup-node@v4` node 20 (or the project's pnpm).
- Linux system deps: `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf`.
- The C/C++ (whisper-rs/ggml) must build natively on each runner — no cross-compile; extra macOS arch needs `rustup target add <arch>`.
- `tauri-apps/tauri-action@v0` with `GITHUB_TOKEN` and **`permissions: contents: write`**, `tagName: v__VERSION__`, `releaseDraft: true`. Outputs `releaseId`, `releaseHtmlUrl`, `artifactPaths`.
- whisper-rs CI note (R1): set `GGML_NATIVE=OFF` so CI-built engines aren't `-march=native`-locked.

## cert/signing wait-conditions (deferred, recorded for v1.x)

| Capability | Blocks on |
|---|---|
| Windows auto-update UX clean | Authenticode cert (OV/EV) |
| macOS auto-update UX clean | Developer ID + paid Apple Program + notarization (`notarytool`, stapled) |
| Windows AV false-positive suppression | OV/EV cert + WDSI submissions |
| macOS gatekeeper-clean single-file run | Developer ID + notarization |

Secrets to wire later (`gh secret set`): `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)`, `APPLE_CERTIFICATE(_PASSWORD)`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, `WINDOWS_CERTIFICATE(_PASSWORD)`.

## Sources

- Tauri v2 bundle config (targets = all ⇒ deb/rpm/appimage/nsis/msi or app/dmg; staticVCRuntime default): https://v2.tauri.app/reference/config
- tauri-action: https://github.com/tauri-apps/tauri-action · GH pipelines guide: https://v2.tauri.app/distribute/pipelines/github
- Windows installer (NSIS default, installMode, MSI/VBSCRIPT): https://v2.tauri.app/distribute/windows-installer
- macOS signing/notarization (notarytool, stapling): https://v2.tauri.app/distribute/sign/macos · https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution.md
- Gatekeeper remediation: https://support.apple.com/guide/mac-help/mh40616/mac
- AppImage + FUSE: https://v2.tauri.app/distribute/appimage · https://docs.appimage.org/user-guide/troubleshooting/fuse.html
- deb: https://v2.tauri.app/distribute/debian · rpm: https://v2.tauri.app/distribute/rpm
- Updater plugin (minisign keys, latest.json): https://v2.tauri.app/plugin/updater
- Icons: https://v2.tauri.app/develop/icons · Universal removed: https://github.com/tauri-apps/tauri/issues/3317
- macos-latest → macOS 26: https://github.com/actions/runner-images/issues/14167
- Unsigned Windows SmartScreen/AV evidence: https://github.com/codebureau/barebill/issues/46