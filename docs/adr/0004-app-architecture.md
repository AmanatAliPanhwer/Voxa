# App architecture: module boundaries, threading, and the event contract

Voxa is a single-crate Tauri/Rust app with one webview window for the pill and one for Settings. The module map below gives the build phase a navigable, testable repo; a single `session` actor owns the ADR-0001 state machine and every other module is a consumer or a worker stage. Communication is event-driven: the state machine broadcasts, stages come back with results.

Status: accepted.

## Decisions

### Module map

One crate (no cargo workspace), eight modules. Deep modules — a narrow public mouth, hidden internals — are `hotkeys`, `capture`, `transcribe`, `insert`, `cleanup`. Shallow leaves are `config`, `store`, `frontend`. `session` sits at the centre: the only module that mutates the lifecycle.

| Module | Kind | Public mouth |
|---|---|---|
| `session` | Core | Owns the ADR-0001 state machine (single writer). Consumes `activate:*` events; drives the stage modules; broadcasts lifecycle events. |
| `hotkeys` | Deep | The gesture arbiter — sole owner of the tap-vs-hold and double-tap timing machine. Exposes only the four committed `activate:*` events (ADR-0003). Per-OS registration (Win `RegisterHotKey`, macOS Carbon, X11 `XGrabKey`, Wayland: no registration) plus tray controls. |
| `capture` | Deep | `start(device) / stop()` + a levels+state sink. Hides cpal, ring buffer, rubato resample to 16 kHz mono f32, RMS/gate. |
| `transcribe` | Deep | The `Transcriber` trait seam (whisper-rs embedded today, sidecar later); a job queue; a single blocking worker with a warm model. |
| `insert` | Deep | The OS-native snapshot → write → paste → restore primitive (ADR-0002) + the clipboard-only tier + recovery (`insert_last_result`). |
| `cleanup` | Deep | Async Groq client (OpenAI-compatible chat API, `/models` probe, tone presets); golden rule: every failure returns the raw transcript. |
| `config` | Leaf | Plain `appDataDir/config.json` + Groq key in the OS keychain only; owns load/save and emits `config:changed`. |
| `store` | Leaf | Volatile result history (in-memory, newest = last result). A small struct, not a crate. |
| `frontend` | Leaf | Tauri bridge: inbound commands, outbound broadcasts, current-state snapshot for a window joining late. |

The spec reproduces this layout verbatim.

### State ownership: the session actor

`session` is an async task (Tokio mailbox) and the **single writer** of the lifecycle. It receives the four `activate:*` events from the arbiter, advances the ADR-0001 machine, and dispatches stage work: clip → transcribe worker → raw transcript → cleanup → clean text (or raw) → insert. Workers reply with results; `session` performs the transitions and broadcasts them. Nothing else mutates session state, giving the machine a serialized, unit-testable shape.

### Threading map

- **RT audio callback** (`cpal`): memcpy only, into a lock-free ring buffer.
- **Capture drain worker** (~100 Hz): mixdown + resample (rubato) → append to the Whisper PCM buffer, compute RMS, gate, emit level events.
- **Transcribe worker** (blocking thread, warm `WhisperContext`, per-clip queue): f32 16 kHz mono in, `GGML_NATIVE=OFF` for portable binaries.
- **Cleanup**: on the Tokio runtime (async reqwest, ≤~8 in-flight, batch ≤1 req/s).
- **Insert**: serialized on the main thread (Windows clipboard open/close must be same-thread).
- **Level stream**: 10 Hz throttled, deduped below 0.005 RMS norm delta, one explicit `0.0` on gate-close then zero IPC while silent (pill renders literally still). (R3's number, not the charting sketch of ~30 Hz.)

### Event & command contract

Outbound, via Tauri **Emitter** broadcast (two windows must both receive; no polling, no per-invoke Channel):

| Event | Payload |
|---|---|
| `session:state` | Canonical domain state (below). |
| `session:levels` | `f32` RMS, gated → `0.0` then silence = zero IPC. |
| `session:progress` | Model download / cleanup progress, with phase. |
| `session:error` | `Err` (taxonomy below). |
| `session:clip` | Result produced: `inserted \| failed`, with target + timestamp (for Settings/diagnostics; no History UI). |

Inbound, via commands: `session:snapshot` (initial state replay for a window), `session:insert_last_result`, `settings:get`, `settings:apply`, `model:list`, `model:download`, `model:delete`, `cleanup:test_key`, `app:open_settings`, `app:quit`.

The four `activate:*` events remain the OS → session inbound contract, committed verbatim in ADR-0003. The recovery hotkey and tray actions are inbound events into `session` on the same bus.

### Canonical state vocabulary

The domain machine keeps ADR-0001's six states — `idle`, `listening`, `processing`, `inserting`, `done`, `error` — unchanged; they are the only machine states. The pill renders a **projection** of them, never its own vocabulary: `done` renders as the 0.45 s flash then hidden; `error` renders grey/red dim. Model download rides inside `processing` (spinner) with `session:progress` carrying the detail — **no** `transcribing`, `cleaning`, `downloading`, or `offline` states exist in the machine. "Offline" is a pill-only look modifier applied on error, not a state.

### Error taxonomy

One enum feeds the pill and the diagnostics log:

```
Err { kind: Activation | Capture | Transcribe | Cleanup | Insert | Config,
      recoverable: bool,
      deep_link: Option<SettingsPane>,     // e.g. microphone, model, cleanup
      detail }                              // log-only, never on the pill
```

The pill renders a look derived from `kind` + `recoverable` (grey dim, faint red cast); text never appears on the pill. Detail lives only in the rotating diagnostics log (`appDataDir/logs`), surfaced via Settings "Reveal logs". Hotkey-conflict/reserved-chord is an `Activation` error — its own kind, distinct from capture errors, non-fatal (tray fallback).

### Startup & laziness

Nothing blocks app start. Sequence: config load → keychain read (async) → hotkey register (arbiter; failure = typed error + tray fallback, never fatal) → tray up. Lazy on first need: mic opens at first `activate` (permission prompt, Windows preflight); whisper model loads on first clip (missing → download edge inside `processing`, progress via `session:progress`); `/models` probe fires in background at startup, never blocks, never errors the session. "No model" and "cleanup offline" both degrade to working-but-raw, not to a dead app.

### Settings reactivity

`config` owns load/save and emits `config:changed`; consumers resubscribe, never poll. Hotkey arbitration re-registers on chord change, capture re-opens/reroutes on `mic.device` change, `transcribe` swaps on `model.id` change (next clip), cleanup re-probes on key save. No setting requires an app restart.

## Considered and rejected

- **Multi-crate workspace.** Rejected for v1: the deep/shallow boundary works as Rust module boundaries; crate seams would add build/cross-compile surface without a payoff yet. Revisit only if a stage (e.g. transcriber) needs to become a reusable library.
- **Pill owns its own state vocabulary.** Rejected: three vocabularies already coexist (ADR-0001, pill-final, first pill.html); the machine's six states are the single contract, and the pill renders a projection, so the pill never drifts from the machine.
- **Per-invoke IPC `Channel` for levels.** Rejected for the multi-window app: a Channel is bound to one invoke; Emitter broadcast delivers to both windows and needs no reconnect logic. (R3's "one ordered Channel" was the capture module's chunk-level view; the app-level contract is broadcast.)
- **UI-driven polling of state.** Rejected: everything the UI renders is already pushed; snapshot exists only for a window joining late.
- **Pause/resume or per-stage substates in the machine.** Rejected at G1/ADR-0001; keeping the machine to its six states preserves the "no cancel, no partial" contract.

See: ADR-0001 (dictation-session state machine), ADR-0002 (insertion reliability loop), ADR-0003 (hotkey arbitration); wayfinder tickets #12 (this decision), with facts from #2, #3, #6, #8, #9, #11, #14.