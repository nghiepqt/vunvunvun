# Phase 03 - Capture

Read `../CONTEXT.md` first.

## Goal

Make F1 real end to end. A thought becomes a stored `inbox_item` through `ef_oplog::OpLog::apply` in under five seconds with no decisions required, working offline, working with no LLM installed, working with Vietnamese IME and a screen reader.

Estimated one and a half weeks.

## Prerequisites

Phase 01 (`inbox_item`, `inbox_audio`, `OpLog::apply`) and Phase 02 (native Win32 F1 overlay and hotkey).

## Design intent

The user is mid-thought, possibly mid-panic, and every field you ask them to fill is a place to abandon the interaction. No title, no project, no due date, no priority, no tags, no category. One input. They type, speak, or paste; hit Enter; the overlay restores focus and disappears.

## Scope

### 1. Text capture

- Native Win32 `MSFTEDIT_CLASS` RichEdit input: Enter commits, Shift+Enter inserts newline, Escape discards.
- On commit: write `inbox_item` via `ef_oplog::OpLog::apply`, call `SetForegroundWindow(saved_hwnd)` before hiding the overlay, and return focus.
- Confirmation respects `UserProfile.affect`: minimal setting shows a brief border pulse only (no toast, no sound, no modal).
- `raw_text` is stored verbatim and immutable forever.

### 2. Voice capture

- Recording starts immediately when opened in voice mode (via `wasapi` / `cpal` into a temp WAV in `resolve_app_paths().temp_dir()`), with an accessible recording indicator.
- On-device transcription without Windows App SDK lock-in:
  - Primary path: `whisper.cpp` sidecar or quantized Whisper/PhoWhisper via `candle`/`ort` running locally on CPU/GPU, compatible with both Windows 10 22H2 and Windows 11.
  - Verify Vietnamese transcription quality before committing to model size; small Whisper models are weak at Vietnamese, so benchmark PhoWhisper/large-v3-turbo-q4 or allow opting into a Vietnamese-capable model.
- Store temporary audio path in device-local `inbox_audio` only; delete audio after transcription unless user opted to retain it.
- If microphone permission is denied or no mic exists, degrade straight to text input with zero error dialog.

### 3. Image capture

- Accept clipboard images (`CF_DIBV5` / `CF_PNG` / `CF_HDROP`) and dropped image files in the F1 Win32 overlay.
- Save image bytes under `resolve_app_paths().attachments_dir()` and create an `inbox_item` with `source = image`. OCR/multimodal structuring waits for Phase 04.

### 4. Background structuring queue

- `tokio` background worker drains `pending` inbox items.
- When `CloudAiMode::AskEachTime` is active and no local T1 model is available, park the item with `status = pending_consent` instead of prompting from the background—ask in batch on next Workspace focus.

### 5. Minimal Inbox view (first ephemeral WebView2 view)

- Newest-first list showing `raw_text` and capture timestamp, with delete/restore (tombstone via `OpLog::apply`).

## Out of scope

No structuring, no LLM calls, no task creation, no OCR.

## Definition of done

- Cold hotkey to visible overlay under 150 ms.
- Median hotkey-to-stored-item under 5 seconds with a real user.
- Capture succeeds with network off and zero models installed.
- Focus returns cleanly to the prior app; Vietnamese IME (Unikey/EVKey) works without dropped diacritics.
- Denied mic degrades silently to text mode.
- Every inbox write goes through `OpLog::apply`, and `raw_text` immutability is verified by test.
- End-to-end flow passes with NVDA and Narrator.
