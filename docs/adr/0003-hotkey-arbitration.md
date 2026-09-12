# Hotkey arbitration: one key, hold-to-talk vs double-tap hands-free

A single global hotkey serves two activation modes (hold-to-talk and hands-free) via gesture disambiguation, mimicking Wispr Flow's interaction model. The arbiter is the sole owner of the timing state machine; the domain model (ADR-0001) sees only committed activation events.

Status: accepted.

## Decisions

### Gesture thresholds

- **Tap vs hold threshold: 150 ms.** A press whose duration is ≤150 ms is a *tap*; above it, a *hold-to-talk* that starts recording. On Windows, `GetAsyncKeyState` quantizes to 50 ms poll ticks (0/50/100 ms), so real-world taps sit ≤100 ms with room to spare; deliberate slow presses clear the threshold and become holds, never hanging taps waiting for a second press.

- **Double-tap window: 300 ms from first release to second press.** A tap followed by a second press arriving within 300 ms is a candidate double-tap. Both presses must themselves be taps (released within the 150 ms threshold). The whole double-tap completes within ~600 ms worst case (150 + 300 + 150). This aligns with Wispr Flow's observed behaviour: the second press crossing the hold threshold becomes push-to-talk, not a toggle.

- **Lone tap (window expires, no second press): silent no-op.** No clip is started, no state is changed, no error surfaced. Consistent with the "no cancel" rule (ADR-0001): a stray tap is a quiet nothing.

### Microphone open timing

- **Capture starts at key-down**, not at hold-declaration, so holds never lose the first syllable of speech. An event that becomes a tap discards the sub-threshold buffer (a few hundred ms of silence or stray noise; never a clip). The pill may briefly flicker bars on a stray tap.

- **Orphaned tap dies when a slow press arrives inside the window:** the tap is abandoned and the hold starts its own clip. No combined state is created.

### Hands-free lock-in gesture

- **Convert a live hold to hands-free.** While actively dictating on a hold (a live push-to-talk session), a quick release-and-repress of the same key within the double-tap window *locks* the ongoing session into hands-free mode, so the user can release the key and keep dictating. On lock-in, the arbiter emits `activate:hold-released` followed immediately by `activate:toggle-on`; the domain model transitions from `listening` back to `listening`, and the session continues without the key. This is Wispr Flow's documented "lock into hands-free from push-to-talk" gesture, adopted for behaviour parity.

- **Hold during hands-free: no-op.** Only the double-tap ends a hands-free session. Holding the key while hands-free is armed neither records separately nor ends the session; the key is ignored. This keeps "one clip per period, no mid-session inserts" airtight.

### Toggle-off symmetry

- A double-tap while hands-free is armed emits `activate:toggle-off`; the whole hands-free period is one clip processed at that point (ADR-0001). A lone tap during hands-free is a no-op, same as from idle.

### Wayland fallback

- **No global key on Wayland** (Tauri `global-shortcut` is X11-only; registration hangs silently). The fallback is the **tray menu** (start/stop hold-to-talk, toggle hands-free) plus an **app-local key** that works only while a Voxa window is focused. The spec's Activation chapter states this explicitly so v1 has a documented path for Wayland users.

## Activation event contract

The arbiter exposes four events to the domain model. The naming and fire-points are committed for the architecture (ADR-0004, pending) and reproduced verbatim in the v1 spec.

| Event | Fires when | Domain transition |
|---|---|---|
| `activate:hold-began` | Key press crosses the 150 ms hold threshold (not at key-down) | idle → listening |
| `activate:hold-released` | Key-up of a recognised hold | listening → processing |
| `activate:toggle-on` | Second tap's release completes a double-tap from idle, *or* on the re-press of a lock-in gesture from within a live hold | idle → listening (or hold → hands-free listening on lock-in) |
| `activate:toggle-off` | Second tap's release while hands-free is armed | listening → processing |

- **Taps emit nothing.** Threshold timing and window waiting are a pure arbiter concern; the domain model is never consulted during a tap-in-progress.
- **Lock-in** emits `activate:hold-released` + `activate:toggle-on` back-to-back; the domain model processes the hold-released (brief transition to processing) then immediately re-enters listening via toggle-on. The clip is **not** split — the lock-in is seamless: audio capture never stops; the processing stage is skipped because the session hasn't ended; the domain remains in `listening` after the back-to-back events.
- **Pill visibility**: on `toggle-on`, the pill and its companion Stop bubble appear and persist for the full hands-free period. On `hold-began`, the pill appears without companion bubbles (the user's hand is on the key). On `hold-released` or `toggle-off`, the pill transitions to processing (spinner) then hides on `done`.

## Considered and rejected

- **Immediate arm on key-down (no tap/hold delay).** A hold starts recording instantly on press. Rejected: no disambiguation window exists, so a double-tap is impossible — every first tap would start a clip before the second tap can arrive. The 150 ms dead zone is the minimum cost of gesture disambiguation on a shared key.
- **Single-tap-to-record mode (Wispr iOS pattern).** Rejected as out of v1 scope; the map settled one key + double-tap at charting (Q5).
- **Esc / dedicated cancel key.** Rejected at G1 (ADR-0001): a blank clip is a silent no-op, never an error. Wispr's Cancel is deliberately diverged from.
- **Optional dedicated hands-free binding.** Wispr Flow offers one; the charted scheme is a single key only. Rejected for v1 simplicity.

See: ADR-0001 (dictation-session state machine), ADR-0002 (insertion loop); wayfinder ticket #14.
