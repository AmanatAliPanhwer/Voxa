# Insertion loop: snapshot → paste chord → restore, with clipboard-only tier and manual recovery

Voxa places clean text into the frontmost app. Charting settled "clipboard paste + save/restore" (Q10), but the reliability edges were unproven; a HITL prototype (wayfinder #10) pushed them through terminals, clipboard-only tiers, guards, and undetectable failures. This ADR pins the loop the spec commits to.

Status: accepted.

## Decisions

- **The loop** is four steps: **snapshot** the user's clipboard → **write** clean text → **fire one paste chord** → **restore** the snapshot. It runs on resolution of the clip's clean text, in the domain's `inserting` state; multi-step sub-mechanics are not user-visible.
- **Guards fail fast, before the clipboard is touched**: no foreground app, or the frontmost app is Voxa itself (e.g. its Settings window) → insertion is an error: no clipboard write, notification, recovery offered. Inserting into yourself is impossible by construction.
- **Snapshot/restore is OS-native, not the Tauri clipboard plugin.** The plugin (`arboard`) calls `EmptyClipboard`/`clearContents` on write — it destroys the user's prior clipboard (files, images, rich text) before we can get it back, and has no snapshot API. The insert module implements real snapshot/restore: Windows `EnumClipboardFormats`/format read-back; macOS `NSPasteboard` readableTypes + data copy; X11 cache the CLIPBOARD selection and re-offer it as owner on restore. Full fidelity.
- **Paste chord, chosen per OS and per target class, sent via enigo**: Windows `Ctrl+V` (conhost/cmd → `Shift+Insert`); Linux X11 `Ctrl+V` for editors, **`Ctrl+Shift+V` for terminals** (plain `Ctrl+V` sends a literal `^V` into a shell; a small frontmost-window allowlist picks the chord, with a per-app override in Settings); macOS `⌘V`.
- **macOS auto-insert requires the Accessibility grant** (`CGEventPost` is TCC-gated; events are silently dropped without it). R2's "no Accessibility since we clipboard-paste" was wrong *for the paste keystroke* — the Carbon hotkey needs no permission, but synthesizing ⌘V does. If the grant is missing, macOS falls into the clipboard-only tier.
- **Clipboard-only tier** (macOS without Accessibility, and native Wayland targets — XTEST never reaches Wayland-native windows): no synthetic paste is possible. Clean text is written to the clipboard, a system notification says "press ⌘V/⌃V", and the user pastes. **Restore is honestly dropped**: the parked snapshot is abandoned the moment the clipboard moves on — restoring would clobber the user's new copy. In this tier the prior clipboard is not returned.
- **Blind paste, no post-hoc verification.** There is no cheap cross-platform way to know the paste landed (clipboard monitors see our own write; AX/UIA/AT-SPI read-back is flaky and per-platform). Success is optimistic; silent misses (elevated windows, fields that swallow paste) land on the recovery net. Write failures and guards *are* detectable and error loudly.
- **Recovery stays manual, and it's a second hotkey + tray item**: insert-last-result is a dedicated global hotkey (sketch `⌘⌥V` / `Ctrl+Alt+V`) and a tray menu item, re-copying the newest result from the in-memory last-result buffer and re-running the loop. The pill is a click-through visualizer (pill prototype, #8) so it can never take the click.
- **Failure surfacing is outside the pill**: the pill dims grey (its `error` look); the OS system notification carries the recovery instruction. No inline pill text ever (CONTEXT: pill carries no text).

## Consequences

- The insert module carries a real OS-native clipboard snapshot/restore primitive — a contained, testable seam (feeding architecture #12). The Tauri clipboard plugin is not used for insertion.
- macOS gains one permission beyond the microphone (Accessibility). The R2 record (research/hotkeys-permissions, map gist for #4) is amended: *hotkey* needs no permission; *auto-paste* needs Accessibility.
- Linux/Wayland and macOS-without-Accessibility users get a two-step insert (clipboard + own paste) — the honest cost of "no synthetic keys".
- Elevated/admin windows and password/read-only fields are undetectable failure classes; recovery is the answer, never a silent retry.
- Timing budget for the spec: snapshot (asynchronous, fast) → write → ~80 ms settle → chord → ~300 ms paste window → restore.

## Considered and rejected

- **Text-only restore via the plugin** — destroys non-text prior clipboard; rejected for full OS-native fidelity.
- **Verify paste by accessibility read-back** (AX on macOS / UIA on Windows / AT-SPI on Linux) — flaky, expensive, platform-balkanised; the recovery net is the honest safety net. v1.x refinement at most.
- **Leave clean text on the clipboard after a successful insert** — convenient, but silently stands on the user's prior clipboard and muddies custody; the last-result buffer is the single home of clean text.
- **Leave clean text on the clipboard in the auto tier** — same custody concern; restore always.
- **Pill-click recovery** — the pill is a click-through visualizer by design; it takes no input.

See: `prototype/insertion/insertion-loop.html` and `research/insertion-reliability/findings.md` (branch `prototype/insertion`); wayfinder ticket #10.