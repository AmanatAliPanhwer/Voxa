# Global hotkeys & OS permissions — per-OS reality

Resolved by subagent research (R2). Version/DE-dependent items flagged as of Sept 2026.

## Decision matrix (spec-ready)

| OS | Global-hotkey mechanism (Tauri v2) | Permission prompts | On denial/failure | v1 fallback |
|---|---|---|---|---|
| Windows | `RegisterHotKey` via hidden no-activate window; Released via `GetAsyncKeyState` poll (~50 ms) | **None** | Chord already taken → error; won't fire when an elevated/UAC window has focus or on the lock/secure desktop | Tray/pill click-to-talk; "pick another key" UI |
| macOS | Carbon `RegisterEventHotKey` (Pressed + Released) | **Hotkey: none** (no Input Monitoring/Accessibility). **Microphone** TCC prompt at first capture (`NSMicrophoneUsageDescription` is mandatory or the app dies). **Accessibility only if we synthesize keys** — we clipboard-paste instead, so NOT needed | Reserved chord (⌘Space etc.) → non-zero return; mic denied → zero-samples capture | Menu-bar/pill + Cmd+V paste |
| Linux X11 | `XGrabKey` on root (global-hotkey x11) | None | `BadAccess` if grabbed elsewhere | Tray/pill fallback |
| Linux Wayland | **Not supported** — registration is a hang/silent no-op; gate on `XDG_SESSION_TYPE` | only if adopting the GlobalShortcuts portal ourselves (DE-mediated) | no-op/hang → do NOT call register | **App-local key + tray/pill click to start/stop; clipboard paste.** The portal path (KDE 5.26+, GNOME 48+) is a later iteration |

## Key facts

- **Wayland detail**: Tauri `global-shortcut` pulls only `global-hotkey`, which is X11-only with its own message "Other window systems on Linux are not supported". Under Wayland the X11 connect fails silently, then `register()` blocks forever (issue open since 2022; live GNOME 48 bug #3267). The xdg-desktop-portal `GlobalShortcuts` interface exists but is user-mediated and DE-stored (app gets descriptions, not the key), shipped on KDE 5.26+ and GNOME **48+ only** (2025-03-19).
- **macOS**: Carbon hotkey needs no TCC permission (this is why several production apps use it); users only ever see the Microphone prompt. Detect mic denial via `AVCaptureDevice` authorization status; denied ⇒ prompt to Settings → `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone`.
- **Windows**: hotkeys never reach the lock/secure desktop or elevated windows; recommending a chord like `Ctrl+Win+B` requires confirming it isn't OS-reserved (reserved: Ctrl+Win+D/F4/←/→); mic privacy toggle is advisory for desktop apps — instead detect "no device / no samples".
- **Linux**: no OS gate for native mic apps (only Flatpak sandboxes enforce); detect "no audio input nodes" and guide the user.
- **Permission UX must be scripted in onboarding** (R2 note): prompt order = mic first, hotkey fallback silently, clear Settings deep-links per OS.

## Sources

- Tauri v2 global-shortcut plugin (Cargo, plugin repo): https://github.com/tauri-apps/plugins-workspace/tree/v2/plugins/global-shortcut
- global-hotkey crate (Win/macOS/x11 backends, Wayland-out-of-scope message): https://github.com/tauri-apps/global-hotkey · Linux issue: https://github.com/tauri-apps/global-hotkey/issues/28 · Wayland hang bug: https://github.com/tauri-apps/plugins-workspace/issues/3267
- MS RegisterHotKey + reservations: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey
- UAC/UIPI elevated-window behavior: MS docs (User Account Control overview)
- Apple `NSMicrophoneUsageDescription` (mandatory): https://developer.apple.com/documentation/bundleresources/information-property-list/nsmicrophoneusagedescription
- Accessibility via `AXIsProcessTrusted`: https://developer.apple.com/documentation/applicationservices/1460720-axisprocesstrusted (only needed if we synthesize keys — we don't)
- xdg-desktop-portal GlobalShortcuts: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html · GNOME backend in GNOME 48: https://release.gnome.org/48/ · KDE backend since Plasma 5.26
- Windows/Apple privacy-policy guidance for desktop apps (advisory mic toggle): MS support docs; PipeWire access model: https://docs.pipewire.org/page_access.html