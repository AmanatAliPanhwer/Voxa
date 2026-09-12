# Wispr Flow — desktop "Flow Bar" pill: ground truth

Sub-action of prototype ticket #8 ("the pill"). Question: what exactly does the Wispr Flow pill look like, so the chosen Voxa pill can match it.

## Sources

- Wispr Flow Help Center — "Navigating the Wispr Flow App" (docs.wisprflow.ai/articles/5096240724), "Move and Dock the Flow Bar on Desktop" (docs.wisprflow.ai/articles/1790396454), "Troubleshooting the Flow Bar" (docs.wisprflow.ai/articles/5002934560), "Keyboard and Screen Reader Accessibility in Wispr Flow" (docs.wisprflow.ai/articles/3941699399)
- Community field guide (github.com/vkorost/wispr-flow-field-guide), chapters "The Dictation Model" and "macOS", cites Wispr docs

## Shape & placement

- The Flow Bar is described as "the **small floating bubble** that shows your dictation status — **resting, recording, or processing**" — a floating pill/capsule.
- Lives at the **bottom of the screen**; "click the center bubble ... to start dictating". On new installs it is **hidden by default** (Settings → System → Show Flow Bar at all times).
- Draggable; snaps to **pill-shaped drop zones** on the bottom/left/right edges; **reorients vertically** when side-docked; position remembered across launches; Escape cancels a drag.

## Color & metering behavior

- When listening, the bar shows an **animated white waveform**. When voice is detected, **the bar turns blue** — two-state feedback: white bars moving but never blue = mic picking up silence; no bars at all = hotkey did not register.
- **"The waveform now goes flat after a short period of silence"** (Wispr troubleshooting log) — genuine silence = still waveform, exactly the behavior Voxa's gate encodes.
- During processing there is a **progress indicator**; **Cancel appears** shortly into processing.
- **Offline, the pill turns grey.**
- Audible **ping when recording starts**, a **paste sound on submit** (toggle under Sound Effects).

## Interaction model (activation, not the pill look)

- Push-to-talk: hold hotkey → speak → release → transcribe → insert. Clicking the bar while push-to-talk ends the session.
- Hands-free: hotkey + Space; clicking the bar does **not** stop it; use stop/cancel or the hotkey again.
- Click-through: buttons respond to clicks; **transparent/empty areas pass clicks to the app underneath**.
- Right-click opens the Flow Menu (hide for 1 hour, Settings, Microphone, languages, transcript history, Paste last transcript).

## Caveat

Pixel-exact parity cannot be independently measured without the commercial app; this documents its documented look and behavior. The Voxa final pill encodes: dark capsule; white waveform that turns blue on voice; flat-still on silence; thin progress line while processing; grey/dim on error/offline; nothing else on the capsule (no dot, no hint, no label).