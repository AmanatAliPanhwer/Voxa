# Insertion reliability — cross-platform clipboard paste + synthetic keystrokes

Research findings · September 2026

Scope: Voxa's insertion flow is assumed to be *set clipboard to plain text → simulate a paste keystroke (Cmd+V / Ctrl+V) into the frontmost app → restore the previous clipboard*. These notes answer four reliability questions with primary sources. No product code was written.

---

## Fact 1 — macOS Accessibility TCC: simulating keystrokes requires an explicit user grant

**Bottom line:** On macOS there is **no permission-free way** to deliver a synthetic paste keystroke to another app. Posting keyboard events with Core Graphics (`CGEventPost` / `CGEventTapCreate`) requires the app to be granted **Accessibility** access (System Settings → Privacy & Security → Accessibility). Writing to and reading from the pasteboard (`NSPasteboard`) needs **no** permission, so the clipboard half of the flow is safe; the keystroke half is the gated part.

Evidence:

- **Permission is mandatory since 10.14:** an Apple engineer on the Developer Forums confirmed that posting events via `CGEventPost`/CGEvent taps requires explicit user permission, and the system will silently drop events when it is missing. (Apple Developer Forums, thread 103992; also `CGEventTapCreate` docs: event taps "may only receive key up and down events if access for assistive devices is enabled".)
- **Sandboxed apps are doubly blocked:** Apple's App Sandbox Design Guide states that "you cannot sandbox an app that controls another app", and that posting keyboard/mouse events with `CGEventPost` "is therefore not allowed from a sandboxed app". A Tauri/macOS **app store build that is sandboxed cannot insert via keystrokes at all**; a non-sandboxed (developer-distributed) build can, after the user grants Accessibility.
- **Grant location:** Apple Support, "Allow accessibility apps to access your Mac" — System Settings → Privacy & Security → Accessibility.
- **Pasteboard has no TCC:** `NSPasteboard` read/write of the general pasteboard requires no Accessibility (or other) consent — Apple's framed the risk of clipboard access as a privacy reader notice (macOS 13+ "app would like to paste from other apps"), which is raised on *read* when the app is not frontmost, not on write. It is not a permission gate like Accessibility.
- **Third-party confirmation:** enigo's platform notes document that the macOS backend needs the app to be granted accessibility permissions before synthetic input works.

Targeting notes: `CGEventPost` to `kCGHIDEventTap` injects into the system event stream (reaches the current foreground app); `CGEventPostToPid` (macOS 10.11+) targets a specific process PID. Both require Accessibility trust.

Sources
- Apple Developer Forums — CGEvent posting requires permission: https://developer.apple.com/forums/thread/103992
- Apple — `CGEventTapCreate` documentation/CGEvent.h (assistive-devices note): https://developer.apple.com/documentation/coregraphics/cgeventtapcreate(_:place:options:eventsofinterest:callback:userinfo:)
- Apple — App Sandbox Design Guide (cannot sandbox an app that controls another app): https://developer.apple.com/library/archive/documentation/security/conceptual/AppSandboxDesignGuide/AppSandboxInDepth/AppSandboxInDepth.html
- Apple Support — Allow accessibility apps to access your Mac (mh43185): https://support.apple.com/en-us/accessibility-mac
- Apple Support — Paste items from other apps (macOS 13+ pasteboard prompt): https://support.apple.com/en-us/102349
- enigo — permission requirements by platform: https://github.com/enigo-rs/enigo/blob/main/README.md#permissions-per-platform
- Apple — `CGEventPostToPid` (target a specific process): https://developer.apple.com/documentation/coregraphics/cgeventposttopid(_:_:)

---

## Fact 2 — Tauri v2 `clipboard-manager` plugin: what it can and cannot do (2.3.3)

**Bottom line:** The plugin (`@tauri-apps/plugin-clipboard-manager` 2.3.3, Rust crate `tauri-plugin-clipboard-manager`, backed by `arboard` 3) exposes exactly: `write_text`, `read_text`, `write_image`, `read_image`, `write_html`, `clear`. It has **no clipboard snapshot/restore API** — only per-format access plus clear. Writing text **replaces the entire clipboard on every platform**, destroying whatever else was there (images, file lists, rich text). `read_text` returns `` (empty string) when the clipboard holds no text, which you cannot distinguish from "clipboard really contains an empty string".

Key verified behaviours:

- `desktop.rs` is a thin wrapper over `arboard::Clipboard` — `set_text`/`get_text`/`set_image`/`get_image`/`set_html`/`clear`.
- **Windows:** `set_text` → `clipboard_win::set_string` → `EmptyClipboard()` then `SetClipboardData(CF_UNICODETEXT)`. `EmptyClipboard` wipes every format — previous clipboard contents are gone the moment Voxa writes.
- **macOS:** `Set::text` calls `NSPasteboard.clearContents()` then `writeObjects` — again, replaces the whole pasteboard.
- **X11:** setting CLIPBOARD claims ownership; the previous owner is dropped (SelectionClear). Wayland support is feature-gated (`wayland-data-control`) and depends on compositor support of `wlr-data-control`/`ext-data-control-v1` — not universal.
- **Restore limitation that matters for Voxa:** the only pre-insert snapshot Voxa can capture through this plugin is `read_text` (and separately image/html at most one each). A clipboard that also held a file list, PDF, rich text, or multiple mixed formats cannot be faithfully restored. If "restore previous clipboard" must lose nothing, the plugin alone is insufficient on all three desktop OSes.

Sources
- npm — `@tauri-apps/plugin-clipboard-manager` (2.3.3): https://www.npmjs.com/package/@tauri-apps/plugin-clipboard-manager
- crates.io — `tauri-plugin-clipboard-manager` (uses `arboard` with `wayland-data-control`): https://crates.io/crates/tauri-plugin-clipboard-manager
- Plugin desktop backend (thin arboard wrapper): https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/clipboard-manager/src/desktop.rs
- Plugin JS guest API: https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/clipboard-manager/guest-js/index.ts
- arboard — macOS `Set::text` → `clearContents()` + `writeObjects`: https://github.com/1Password/arboard/blob/master/src/platform/osx.rs
- arboard — Windows `Set::text` → `clipboard_win::set_string`: https://github.com/1Password/arboard/blob/master/src/platform/windows.rs
- clipboard-win — `set_string` → `EmptyClipboard()` + `SetClipboardData(CF_UNICODETEXT)`: https://github.com/DoumanAsh/clipboard-win/blob/master/src/raw.rs

---

## Fact 3 — Paste key recipe: the "Ctrl+V/Cmd+V" shortcut is not universal

**Bottom line:** Cmd+V is effectively universal on macOS (Terminal.app, iTerm2, browsers, Office, JetBrains all bind it). Ctrl+V is near-universal in ordinary desktop apps on Windows/Linux (browsers, Office, code editors, JetBrains). **Terminals are the exception**, and they differ per app and per platform. On Linux, plain Ctrl+V in most terminals does not paste — it sends the literal control character ^V to the shell (readline "quoted-insert"); the paste binding is Ctrl+Shift+V. On X11 there is an extra trap: middle-click / Shift+Insert in many terminal emulators paste the **PRIMARY** selection, not **CLIPBOARD** — so a snippet written to CLIPBOARD will not come out.

Matrix (paste binding, frontmost app):

| Target | Windows | macOS | Linux |
|---|---|---|---|
| Windows Terminal 1.x | Ctrl+V (`paste`) **and** Ctrl+Shift+V | – | – |
| cmd.exe / PowerShell (conhost) | classic: right-click paste (QuickEdit); Ctrl+Shift+V with the "Enable Ctrl key shortcuts" console option; newer Win10+ builds also accept Ctrl+V | – | – |
| ConEmu | Ctrl+V (default) | – | – |
| Terminal.app | – | Cmd+V | – |
| iTerm2 | – | Cmd+V (Ctrl+V = "Paste or send ^V", customizable) | – |
| GNOME Terminal / Konsole | – | – | Ctrl+Shift+V (Ctrl+V passes ^V to the shell) |
| xterm | – | – | Shift+Insert / middle-click paste (PRIMARY); Ctrl+V = ^V, not bound to paste |
| VS Code | Ctrl+V (editor); terminal: Ctrl+Shift+V | Cmd+V (editor + terminal) | Ctrl+V (editor); terminal: Ctrl+Shift+V; Shift+Insert pastes selection |
| JetBrains IDEs | Ctrl+V | Cmd+V | Ctrl+V |
| Chrome / Edge / Firefox | Ctrl+V; Ctrl+Shift+V = paste as plain text | Cmd+V; Cmd+Shift+V = plain text | same as Windows |
| Word / Excel / PowerPoint | Ctrl+V; Ctrl+Alt+V = Paste Special; Ctrl+Shift+V = paste text only (Word/Excel) | Cmd+V | – |
| Generic text edit (Notepad, etc.) | Ctrl+V / Shift+Insert | Cmd+V | Ctrl+V (GTK/Qt apps); Shift+Insert in many |

Consequences for Voxa:

1. macOS: always send **Cmd+V**; add Cmd+Shift+V only if plain-text paste is desired (it is the default in Safari/Chrome/Edge word processors; most editing surfaces handle plain text fine with Cmd+V).
2. Windows: **Ctrl+V** covers almost everything; be aware the frontmost app may be a console (conhost) where Ctrl+V only works on newer builds with the option enabled — and Windows Terminal (which binds Ctrl+V) is the more common target nowadays.
3. Linux: plain **Ctrl+V is the wrong key for terminals**; the terminal-generic binding is **Ctrl+Shift+V**. A text editor target wants Ctrl+V. Voxa either needs per-target detection or a user setting for the Linux paste chord. Additionally, on X11 the clipboard must be populated as the CLIPBOARD selection (which the plugin does) rather than PRIMARY; middle-click/Shift+Insert paste of PRIMARY is a separate channel and won't deliver Voxa text.

Sources
- Windows Terminal tips & tricks (default Ctrl+C/Ctrl+V, fallback Ctrl+Shift+C/V): https://devblogs.microsoft.com/commandline/windows-terminal-tips-and-tricks
- Microsoft — Keyboard shortcuts in Windows (Ctrl+V / Shift+Insert paste; command-prompt paste): https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec
- Microsoft Q&A (Andy Pennell, MSFT) — right-click paste in CMD; newer Win10 builds accept Ctrl+V: https://learn.microsoft.com/en-us/answers/questions/510/when-can-the-windows-command-line-tool-directly-co
- ConEmu keyboard shortcuts (Ctrl+V paste): https://conemu.github.io/en/KeyboardShortcuts.html
- Apple — keyboard shortcuts on Mac (Command-V = Paste): https://support.apple.com/en-us/102650
- iTerm2 — keyboard shortcuts (⌘V paste; Ctrl+V "Paste or send ^V"): https://iterm2.com/documentation-keybindings.html
- GNOME Terminal — copy and paste (Ctrl+Shift+C/V): https://help.gnome.org/users/gnome-terminal/stable/gnome-terminal-click-and-current-file.html
- Konsole — copy and paste (Ctrl+Shift+C/V, Ctrl+Shift+Insert): https://docs.kde.org/stable5/en/applications/konsole/index.html
- xterm man page (paste via Shift+Insert / mouse; no Ctrl+V binding): https://www.x.org/releases/current/doc/man/man1/xterm.1.xhtml
- VS Code — integrated terminal keybindings & editor basics: https://code.visualstudio.com/docs/terminal/basics and https://code.visualstudio.com/docs/getstarted/keybindings
- JetBrains — default keymap (Paste Ctrl+V): https://www.jetbrains.com/help/idea/reference-keymap-win-default.html
- Mozilla — Firefox keyboard shortcuts (Paste Ctrl+V/Cmd+V; Paste as plain text Ctrl+Shift+V): https://support.mozilla.org/en-US/kb/keyboard-shortcuts-perform-firefox-tasks-quickly
- Microsoft Edge — keyboard shortcuts (Ctrl+Shift+V = paste without formatting): https://support.microsoft.com/en-us/edge/keyboard-shortcuts-in-microsoft-edge
- Microsoft Excel — keyboard shortcuts (Paste selection Ctrl+V; Paste Special Ctrl+Alt+V): https://support.microsoft.com/en-us/accessibility/excel/keyboard-shortcuts-in-excel
- Microsoft Word — keyboard shortcuts (paste text only Ctrl+Shift+V): https://support.microsoft.com/en-us/accessibility/word/keyboard-shortcuts-in-word
- Raymond Chen, The Old New Thing — Ctrl+Shift+V plain paste in Edge/Chrome/Firefox, Ctrl+Alt+V in Office: https://devblogs.microsoft.com/oldnewthing/20220906-00?p=107124
- freedesktop.org — X selections: PRIMARY (middle-click) vs CLIPBOARD (Ctrl+C/Ctrl+V): https://www.freedesktop.org/wiki/Specifications/XSelections

---

## Fact 4 — Cross-platform synthetic keystroke costs (per-platform mechanics and hazards)

**Bottom line:** Doing this with a library on all three platforms means **enigo**, whose backends map directly to the platform mechanisms:

| Platform | Mechanic | Effort / hazards |
|---|---|---|
| Windows | Win32 `SendInput` (via `windows` crate) | Free (no permission). Subject to **UIPI**: input injected at a lower integrity level is dropped by elevated (admin) apps, and the **UAC secure desktop** (consent prompt) neither accepts injected keystrokes nor clipboard paste (clipboard paste on the secure desktop has been disabled since Windows Server 2019). |
| macOS | Core Graphics `CGEventPost(CGHIDEventTap)` via `CGEventCreateKeyboardEvent` | Requires **Accessibility trust** (`AXIsProcessTrusted`); events are dropped without it. Sandboxed builds cannot post global events at all. `CGEventPostToPid` allows targeting a specific process. |
| Linux/X11 | **XTEST** fake key events to the X server (via `x11rb`) | Works without special privileges and is delivered to any foreground X client. But XTEST only affects **X clients** — under **Wayland** it reaches XWayland windows only; native Wayland compositors do not see it (needs the experimental `wayland` backend or the libei backend). |
| Linux/Wayland | experimental `wayland` feature / **libei** backend | Compositor cooperation required (libei portal); not universally available; marked experimental in enigo. |

Additional verified facts:

- **Windows UIPI is real:** SendInput's docs state it "is subject to UIPI" — an equal-or-lower-integrity caller may only inject into equal-or-lower integrity targets, so an elevated (admin, "Run as administrator") frontmost app silently drops the paste keystroke. This is not configurable by Voxa.
- **UAC secure desktop:** the elevation prompt runs on a separate secure desktop; only Windows and trusted processes exist there, and clipboard pasting onto it is blocked (since Windows Server 2019). While the consent prompt is up, the insert simply cannot happen — Voxa's pill dimming should cover this and recovery should be offered.
- **enigo version/backends:** enigo 0.6.x (latest 0.6.1, Aug 2025) — Windows `SendInput`, macOS `CGEvent`, Linux default X11/XTEST, experimental Wayland + libei features; on macOS it explicitly checks/requests Accessibility permission and `CGEvent::post` to `HIDEventTap` (verified in `src/macos/macos_impl.rs`).
- **macOS layout caveat:** encoding the clean text as per-key events (as opposed to clipboard+paste) must map Unicode → keycodes and is layout-sensitive; `CGEventKeyboardSetUnicodeString` is the reliable path. Prefer clipboard+paste on every platform; reserve key-by-key typing for clipboard-hostile targets.

Recommendation implied by the four facts: on all three platforms the cheapest *correct* first attempt is **write plain text to clipboard, then one paste chord** — Cmd+V on macOS, Ctrl+V on Windows, Ctrl+Shift+V on Linux (configurable) — because every alternative (key-by-key typing, per-app DPIs) costs more and adds failure modes. The remaining gaps are (a) macOS Accessibility consent, (b) elevated/secure-desktop Windows targets, (c) Wayland-only Linux targets, (d) restoring a non-text clipboard that the plugin cannot snapshot.

Sources
- enigo — Cargo.toml (backends: x11rb/XTEST default, experimental `wayland` and `libei`) and README/platform notes: https://github.com/enigo-rs/enigo/blob/main/Cargo.toml and https://github.com/enigo-rs/enigo/blob/main/README.md
- enigo — macOS impl uses `CGEvent::post(CGEventTapLocation::HIDEventTap, …)` and `AXIsProcessTrustedWithOptions`: https://github.com/enigo-rs/enigo/blob/main/src/macos/macos_impl.rs
- Microsoft — SendInput (UIPI note; injection only into equal-or-lesser integrity targets): https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput
- Microsoft — UAC secure desktop (elevation prompts isolated from normal desktop): https://learn.microsoft.com/en-us/windows/security/application-security/application-control/user-account-control/how-it-works
- X.Org — XTEST extension (synthesizes core input events, delivered to X clients): https://www.x.org/releases/current/doc/libXtst/Xtst-lib.html
- Wayland — ext-data-control / wlr-data-control (compositor-dependent clipboard + input support): https://wayland.app/protocols/wlr-data-control-unstable-v1