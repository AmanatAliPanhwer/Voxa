# Mic capture & honest level metering (Rust)

Resolved by subagent research (R3). Design Voxa can spec against.

## Decision

- **Capture**: `cpal` 0.18 alone (enable Cargo features `pipewire` + `pulseaudio` on Linux so runtime host order is PipeWire > PulseAudio > ALSA). No `rodio` (playback-centric), no system-audio loopback plugin.
- **Format**: open the stream at `device.default_input_config()` (native rate/channels — typically 48 kHz, 2ch), convert to f32 in the callback, mix to mono, **resample to 16 kHz mono with `rubato`**. Do NOT request 16 kHz directly (WASAPI shared-mode capture must match the engine mix format; a 16 kHz request fails on typical devices).
- **Threading**: cpal callback runs on a real-time audio thread → do nothing heavy there. Copy f32 chunks into a small lock-free ring buffer (`rtrb`/`ringbuf`); a ~100 Hz worker-thread drain loop consolidates → mixdown+resample → (a) appends to the Whisper PCM buffer, (b) computes RMS and emits level events.
- **Level stream**: one ordered `tauri::ipc::Channel` carrying tagged events (`level { mic_rms: f32 }`, plus a `failure { category }` variant). Throttle to **10 Hz**, dedupe below a 0.005 norm delta; **emit one explicit `0.0` on gate-close, then zero IPC while silent** → the pill is literally still.

## The honest meter (Q6 requirement)

- RMS window ~**50 ms** (16 kHz → 800 samples); convert to dBFS (`20·log10(rms)`, epsilon floor); map on `[-60, 0]` → `n = (dB+60)/60` clamped.
- **Ballistics**: one-pole time-based smoothing — attack ≈ 5–10 ms, release ≈ 150–300 ms (or constant ~30 dB/s). Fast attack for rising speech, slow release so transient dips don't jitter.
- **Mandatory gate** (raw mic never reads 0 — ADC self-noise ≈ −40…−60 dBFS): gate open above ≈ **−55 dBFS**, close below ≈ **−70 dBFS**, with hysteresis; applied after smoothing; on gate-close snap to `0.0`.

## Zero-samples = permission error (critical)

- macOS denied mic: stream builds and delivers **all-zero buffers** (no error). Windows: mic privacy toggle silently feeds zero-filled frames (preflight `CapabilityAccessManager\ConsentStore\microphone` registry key — "Deny" ⇒ block). Detect "stream open but RMS == 0 for N ms at start of a session" ⇒ surface the permission error + Settings deep-link, not a silent blank transcript.
- macOS array mics can expose 3 channels — handle mixdown generically (cpal #1143).

## Per-OS notes

- **Windows**: cpal 0.18 default-input streams auto-reroute on system default-device change and surface `ErrorKind::DeviceChanged` (ActivateAudioInterfaceAsync ?? DEVINTERFACE_AUDIO_CAPTURE + IMMNotificationClient). Specific-device streams die with `DeviceNotAvailable` → rebuild, fall back to default input. Known 24H2 "Communications" regression: `default_input_config()` reports 16 kHz mono but delivers all-zero — capture at native mix format avoids it. Skip 16 kHz request (AUTOCONVERTPCM is output-only).
- **macOS**: `NSMicrophoneUsageDescription` mandatory (else app termination). `AVCaptureDevice.requestAccess` for permission; detect `.denied` explicitly.
- **Linux**: native PipeWire/PulseAudio backends in 0.18 cover current distros; raw ALSA fails with `DeviceBusy` while a sound server holds `default`. Resampling at 16 kHz ≈ negligible CPU (<1% in a similar 10 Hz levels mode).

## Sources

- cpal 0.18 (PW/PA backends, WASAPI rerouting, runtime sample formats): https://docs.rs/cpal/latest/cpal/ · https://github.com/RustAudio/cpal/releases/tag/v0.18.0 · rerouting PR https://github.com/RustAudio/cpal/pull/1183 · 24H2 silent-capture: https://github.com/RustAudio/cpal/issues/1200 · DeviceId persistence https://github.com/RustAudio/cpal/pull/1014
- MS automatic stream routing / device formats: https://learn.microsoft.com/en-us/windows/win32/coreaudio/automatic-stream-routing · https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-formats
- rubato (SRC; real-time safe via worker-thread pattern): https://docs.rs/rubato/latest/rubato/
- Reference implementation to imitate (constants, drain loop, dedupe, Windows preflight): `tauri-plugin-system-audio` — https://docs.rs/crate/tauri-plugin-system-audio/latest/source/src/capture.rs and `src/resampler.rs` (129-tap anti-alias on 48k→16k downsample)
- Meter ballistics / dBFS mapping: https://github.com/mixxxdj/mixxx/pull/16608 · https://dev.to/tooleroid/...-audio-level-meter (Web Audio) · https://developer.mozilla.org/en-US/docs/Web/API/AnalyserNode/smoothingTimeConstant
- Apple mic permission: https://developer.apple.com/documentation/bundleresources/requesting-authorization-for-media-capture-on-macos · cpal macOS helpers https://github.com/RustAudio/cpal/pull/1124 · https://github.com/RustAudio/cpal/issues/1143
- PipeWire: https://pipewire.pages.freedesktop.org/pipewire/page_overview.html