# Phase 03 - Capture

Read `../CONTEXT.md` first.

## Goal

Make F1 real end to end. A thought becomes a stored inbox item in under five seconds with no decisions required, working offline, working with no model, working with a screen reader.

This is the feature the product lives or dies on. If capture has friction, nothing downstream matters, because nothing gets captured.

Estimated one and a half weeks.

## Prerequisites

Phase 01 for the inbox tables and the op-log. Phase 02 for the overlay and hotkey.

## Design intent

The user is mid-thought, possibly mid-panic, and every field you ask them to fill is a place to abandon the interaction. So: no title, no project, no due date, no priority, no tags, no category. One input. They type or speak or paste, they hit Enter, the overlay disappears, and they are back in whatever they were doing.

Structuring happens later and asynchronously. Capture never waits on a model, a network call, or a disk flush the user can perceive.

## Scope

### 1. Text capture

A single multi-line input. Enter commits, Shift+Enter inserts a newline, Escape discards.

On commit: write the inbox item through the op-log, hide the overlay, restore foreground to the saved window handle. Order matters here; restore focus before hiding, as established in phase 02.

Give a confirmation that respects the user's affect preference. At the minimal setting this is a brief border flash and nothing else: no toast, no sound, no text. At warmer settings a short acknowledgement line is acceptable. Never a modal, never something requiring dismissal.

`raw_text` stores exactly what the user typed. It is immutable forever; nothing downstream rewrites it.

### 2. Voice capture

For many users this is the primary modality, because speaking is lower-friction than typing when the thought is still forming.

Recording starts immediately when the overlay opens in voice mode, with a visible recording indicator. There is no separate Record button to press; the press of the hotkey is the intent.

Transcription runs on-device. Two candidate paths:

- Windows AI Speech Recognition. It runs on the CPU, works on machines without a neural processing unit, and keeps audio local. It requires MSIX package identity, Windows 11 24H2, and a current Windows App SDK. The model is preinstalled on some hardware and downloaded on demand on others.
- whisper.cpp running in the sidecar process, as the fallback if the packaging requirements above cannot be met.

Verify Vietnamese transcription quality before committing to either. Small Whisper models are weak at Vietnamese, and if the product targets Vietnamese-speaking users a poor transcription is worse than no voice input, because it silently corrupts the captured thought.

Audio is written to a temp file, transcribed, then deleted unless the user has explicitly chosen to keep audio. The path lives in the device-local `inbox_audio` table, never in the syncable inbox row.

If microphone permission is denied, show the text input instantly. Do not show an error dialog; an error dialog at the moment of capture is exactly the friction this feature exists to remove.

### 3. Image capture

Accept a pasted image from the clipboard, and a dropped image file.

Store the image locally and create an inbox item whose `source` is `image`. Optical character recognition is deliberately not in this phase; structuring in phase 04 handles image content when a multimodal path exists. For now the item exists and is visible, which preserves the user's trust that nothing was lost.

### 4. Background structuring queue

A hosted background service drains pending inbox items and hands them to the structuring pipeline built in phase 04. In this phase, build the queue and leave the handler as a no-op that marks items processed.

One constraint that must be designed in now rather than discovered later: if the user's cloud preference is set to ask each time, and no local model is available, this background job cannot ask. Nobody is there. Park the item with status `pending_consent` and batch-ask on next application focus. Never block, and never silently send.

### 5. Inbox view

The simplest possible list, newest first, showing raw text and capture time. Delete and restore. This is not the workspace; it is proof that captures land and a way to see them before structuring exists.

## Accessibility requirements

- The overlay is announced when it appears.
- The input has an accessible name and a described-by hint.
- Recording state is announced, not only shown as a colored dot.
- The whole flow is completable without a mouse.
- Test with NVDA and Narrator, both.

## Out of scope

No structuring, no model calls, no task creation. No optical character recognition. No editing of captured text beyond delete; editing belongs with structuring.

## Definition of done

- Hotkey to visible overlay under 150 ms cold.
- Median time from hotkey press to stored item under five seconds for a short text capture, measured with a real person, not a script.
- Capture succeeds with the network disconnected and no model installed.
- Focus returns to the previously active application and it receives keystrokes immediately.
- Voice capture produces usable transcription in the target languages, or the fallback path is chosen with a written reason.
- Denied microphone permission degrades to text input with no error dialog.
- All inbox writes go through the op-log, verified by test.
- `raw_text` is never modified after insert, verified by test.
- The full flow works with NVDA.
