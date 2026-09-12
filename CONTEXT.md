# Voxa

Voxa is a local-first desktop dictation app: the user records speech with a hold-to-talk hotkey or a hands-free toggle, it is transcribed locally, run through a cleanup pass, and the result is inserted into the active application — all from a floating always-on-top *pill*.

## Language

### Activation

**Pill**:
The floating recorder bar that surfaces Voxa's state. Hidden at idle, shows level bars while listening, a spinner while processing, and a dimmed grey/red look when something fails. It carries no text and no controls.
_Avoid_: rec bar, overlay, widget

**Hold-to-talk**:
Activation mode where the clip is recorded only while the hotkey is held; releasing the hotkey starts processing.
_Avoid_: push-to-talk, tap-to-talk

**Hands-free**:
Activation mode where the first hotkey tap starts continuous recording and a second tap stops it and starts processing. Everything spoken between the taps is one clip.
_Avoid_: dictation mode, continuous mode

**Dictation session**:
One activation of Voxa — a single hold-and-release, or the whole period between hands-free toggle on and toggle off. A session always yields exactly one clip.
_Avoid_: note, entry, call

**Clip**:
The single audio recording of one dictation session — everything heard from activation start to activation end. It is the unit that flows through transcribe, cleanup, and insertion.
_Avoid_: segment, utterance, take, audio file

### Text

**Raw transcript**:
The verbatim text produced by local transcription, exactly as recognised.
_Avoid_: raw text, unedited text

**Clean text**:
The result after a cleanup pass; equals the raw transcript when cleanup was skipped or unavailable.
_Avoid_: polished text, final text

**Cleanup**:
The transformation of a raw transcript into clean text — punctuation, casing, and light polish — performed by an external service when available and skipped (falling back to the raw transcript) when it is not.
_Avoid_: polishing, formatting, "the LLM pass"

**Cleanup provider**:
The external service performing cleanup, reachable only through the user's API key.
_Avoid_: API, backend, service

**Tone preset**:
A named cleanup style selectable by the user; every preset is a different way of producing clean text from the same raw transcript.
_Avoid_: style, personality, voice

**Model**:
The local transcription model (Whisper, e.g. `small.en`) that turns a clip into a raw transcript. Distinct from any cleanup provider.
_Avoid_: engine, backend

### Insertion

**Insertion**:
Placing clean text into the active application. Voxa's own word for the outcome; the clipboard is only the mechanism, never the vocabulary.
_Avoid_: paste, pasted

**Insertion target**:
The application that receives clean text — always the application focused at insert time, even if focus changed during the clip.
_Avoid_: active app, foreground app

### Recovery & storage

**Result**:
Clean text plus its provenance (insertion target, timestamp) — produced once per session and appended to the result history.
_Avoid_: entry, transcript, output

**Result history**:
The volatile, in-memory list of every result since Voxa last started; it is cleared when the app (or the computer) shuts down. The newest entry is the last result.
_Avoid_: history, archive, log

**Last result**:
The most recent entry of the result history — the recovery handle used when an insertion fails.
_Avoid_: previous text, resend

**Recovery**:
Re-inserting the last result after an insertion failed. Recovery is always manual and user-initiated; Voxa never retries on its own.
_Avoid_: retry, resume

**API key**:
The cleanup provider credential, stored only in the OS keychain and never in Voxa's settings file.
_Avoid_: token, secret

**Error**:
A failed step (mic, transcription, cleanup, insertion) surfaced through the pill's grey/red dimming with no inline text; detail lives outside the pill.
_Avoid_: alert, message