# Dictation session state machine

Every clip flows through exactly one lifecycle — `idle → listening → processing → inserting → done`, plus `error` — and every later seam reads these same states: the pill (#8), the architecture (#12), the spec (#13).

Status: accepted.

## Decisions

- **One clip per activation.** A *dictation session* is a hold-to-talk press-and-release or a hands-free toggle-on/off period; it yields exactly one clip. Hands-free does **not** insert mid-session: nothing is transcribed or pasted until the user toggles off, then the whole clip is processed once ("process at toggle-off"). Chosen over Wispr-style per-utterance continuous inserts so the behaviour is predictable and record-then-transcribe holds (no live partials, no VAD segmentation in v1).
- **No cancel.** A session ends only by ending — releasing the hotkey or toggling off. Saying nothing is harmless: a blank clip is a silent no-op back to `idle` (no insert, no result, no error).
- **Model-loading is an edge, not a user step.** If the model is missing when processing starts, transcribe waits for an automatic download (spinner on the pill; progress detail lives in Settings) and then proceeds.
- **Cleanup never blocks insertion.** Missing key, quota (429), or network failure falls back to inserting the raw transcript; the pill just skips straight through cleaning to inserting.
- **Insertion targets the frontmost app at insert time**, even when focus changed during the clip. Apps that block paste (terminals, secure fields) are probed separately (#10); the contract here is the default target.
- **Insert failures are not retried.** The clean/raw text is kept as the newest entry of the volatile result history, the pill shows the error state, and the user recovers manually (recovery = insert last result). Voxa never retries on its own.
- **A lost mic discards the clip** and goes to `error`; no attempt to transcribe the partial.
- **Result history is volatile**: every result since Voxa last started, wiped on shutdown/restart; a History browsing UI stays out of v1 scope.

## Consequences

- States map to the pill (#8): `listening` = level bars (flat & still on silence), `processing`/`inserting` = spinner, `error` = grey/red dim, `done` = brief flash.
- No streaming, partial, or cancel states exist in v1, by design.

Considered and rejected: per-utterance continuous inserts (user picked end-of-session batch); an explicit cancel hotkey (no); a persistent, browsable history (no — History replay excluded at charting).