# EF Assistant - Project Context

Read this file before writing any code. It is the contract. Phase files under `phases/` describe what to build in what order; this file describes what must be true in every phase.

## 1. What this is

A Windows desktop assistant ("Vun") that lowers the cost of starting, sequencing, and resuming work for neurodivergent adults (autistic, ADHD, or both). It is cognitive scaffolding, not a task manager.

The core loop: the user captures a thought with almost no friction, the system structures it without asking them to fill a form, it breaks the work down until the first step is trivially startable, it notices when they stall, and it reduces the demand rather than increasing the pressure.

What it is not, and must never become:

- A medical or therapeutic product. It does not treat, diagnose, or claim clinical effect.
- A productivity-maximization or accountability tool. No streaks, no scores, no guilt.
- An autonomous agent. The AI proposes; the user disposes. Nothing is written to user data without an explicit accept.
- A general chatbot. Open-ended conversation is itself an executive-function demand.

## 2. Users

Three overlapping profiles with genuinely conflicting needs. There is no default persona.

ADHD-leaning users need novelty; repetition kills engagement. Autistic-leaning users need predictability; change is costly. Users with both experience these needs at the same time and may swing between them daily. Autistic-leaning users typically prefer literal, complete, unambiguous instructions with no idioms. Many users across all three profiles have strong demand avoidance, where being told to do something is itself the thing that blocks them.

The consequence for implementation: every behavioral parameter is a stored user preference with a safe, low-demand default. Never infer a preference from behavior when the user can state it.

## 3. Functional requirements (MVP scope)

The MVP is five capabilities. Everything else is post-MVP and belongs in a later phase document.

**F1 Capture.** A global hotkey opens a borderless input that accepts text, voice, or a pasted image. One keystroke dismisses it. No title, project, due date, or priority is ever required. Capture appends to an inbox and must never block on a model or the network.

**F2 Structuring.** An unstructured dump becomes reviewable candidates: tasks, notes, events, questions. The user accepts, edits, or discards each one. Nothing is auto-committed.

**F3 Decomposition.** A task becomes ordered steps, with a granularity slider from 1 to 5. Level 1 gives three to five coarse steps. Level 3 gives six to ten steps of ten to thirty minutes. Level 5 gives twelve to twenty-five steps that each take between ten seconds and two minutes and require no decision-making, because for a stalled user the barrier is the first physical movement, not the task.

**F8 Time estimation.** Every task and step gets a predicted duration shown as a range, never a point. Countdowns are off by default because a visible countdown is a demand signal.

**F11 Workspace.** A home surface of spatially stable blocks: Now (one task), Today, Inbox, Parked. Fixed positions, because spatial memory is load-bearing for these users. Blocks are user-arrangeable but never rearrange themselves.

Post-MVP, documented here only so you do not design them out: retrieval over past notes, energy-based task routing, stall detection with low-demand interventions, obstacle dialogue, meeting notes, weekly review, and multi-device sync.

## 4. Non-functional requirements

### 4.1 Accessibility

This is not a polish item. It is a Definition of Done line on every UI ticket, and an app for disabled users that is itself inaccessible is a failed product.

- Every interactive element is reachable and operable by keyboard, with a visible focus indicator and a logical tab order.
- Every interactive element has an accessible name. Test with NVDA and Narrator, not just an automated checker.
- Text contrast at least 4.5:1, UI components at least 3:1. Automate this in CI.
- Honor the operating system reduced-motion setting. The app's own motion setting may only be stricter, never looser.
- No information conveyed by color alone.
- Text resizes to 200 percent without loss of function.

### 4.2 Performance budgets

These are requirements, not aspirations. A phase is not done if it misses its budget.

- Capture overlay visible within 150 ms of the hotkey, measured cold with the app idle in the tray.
- First streamed token from any model call within 800 ms, with optimistic UI before that.
- Task ranking computed in under 10 ms, fully offline.
- Retrieval p95 under 250 ms at fifty thousand fragments.
- Sustained 60 fps while scrolling five hundred items.
- Idle memory footprint under 150 MB with no model loaded.

### 4.3 Reliability

Degrade, never block. The app must remain fully usable with the network off, the inference sidecar dead, and no local model installed. Capture works, the database works, estimation works, the UI works. Only model-dependent features degrade, and they say so in plain language rather than failing silently.

Inference runs in a separate process so that a crash or out-of-memory condition there cannot take down the UI. A supervisor restarts it with exponential backoff and gives up after three attempts, disabling the local tier for the session.

### 4.4 Privacy

Local-first by default. The following data never leaves the machine under any circumstance, including telemetry and crash reports: foreground application samples, idle and activity signals, nudge outcomes, machine learning model state, and response caches.

Cloud model calls require explicit, granular consent, separate from any terms of service acceptance. The first-run default is to ask each time. A background job cannot prompt for consent; it parks the work and asks on next app focus. Local-only mode must be a genuinely usable product, not a crippled one.

Telemetry is off by default and opt-in.

### 4.5 Correctness of model output

Detailed in section 6. The single most important rule: the model never produces a number that matters.

### 4.6 Cost

Target under two US dollars per active user per month during the cloud-only MVP, falling under twenty-five cents once local inference carries most calls. Track cost per call and alert when a feature exceeds its budget.

## 5. Tech stack

Fixed. Do not substitute without changing this document and adding an ADR first (see `docs/adr/ADR-004-rust-and-split-surface-ui.md`).

- Language & build: **Rust (Edition 2024, stable toolchain)**, workspace organized under `crates/*` and `gateway/ef-gateway`, linked with `rust-lld` on Windows MSVC.
- UI (Split-Surface Hybrid, Option A):
  - **Resident overlays & tray (`ef-win32`)**: Native Win32 via Microsoft's `windows` crate (`MSFTEDIT_CLASS` RichEdit for F1 capture, native `HWND` with `WS_EX_TOOLWINDOW`/`WS_EX_NOACTIVATE` for F7 nudge, DWM Mica/Acrylic backdrop, `tray-icon` / `Shell_NotifyIconW`). Chosen for <15 ms cold wake, ~15–25 MB idle RAM, native Windows UIA (`ITextProvider`), and full Vietnamese IME compatibility (Unikey/EVKey/Windows Telex).
  - **Main workspace & settings (`ui/workspace` via `wry` + `tao`)**: Ephemeral WebView2 window created on demand when opened and **explicitly destroyed on close to tray** (never kept resident hidden in tray). Chosen for Chromium's full UIA accessibility tree, `aria-live` support, 200% zoom, `prefers-reduced-motion`, and automated `axe-core` checks.
- Packaging: **MSIX** (`MakeAppx.exe`, declaring `runFullTrust`), distributed primarily through the Microsoft Store. The Store re-signs the MSIX package for free ($0 individual developer account), removing the commercial code-signing certificate requirement and SmartScreen reputation problem. Direct-download fallback builds standalone `vun.exe`.
- Database: `rusqlite` with `bundled-sqlcipher-vendored-openssl` (includes FTS5 and at-rest encryption for GDPR Art. 9).
- Vector search: `sqlite-vec` statically linked into the Rust binary via `sqlite3_auto_extension` (`cc` in `build.rs`). Self-declared pre-1.0, brute-force KNN; pin an exact version.
- Win32 & WinRT interop: Official `windows` crate (never hand-write raw ABI layouts when `windows` bindings exist).
- Tier-0 ML & embeddings:
  - M1 (Duration estimator) and M2 (Stall detector): Pure-Rust GBDT quantile evaluator over exported LightGBM trees (<1 ms in-process, zero C++ runtime).
  - M3 (Energy-aware ranker): Pure-Rust deterministic LinUCB (`nalgebra` or fixed matrix ops).
  - Embeddings (`multilingual-e5-small`, 384-dim): HuggingFace `tokenizers` (native Rust) + `candle` (CPU SIMD) or optional `ort` (DirectML GPU).
- Local LLM (T1): `llama.cpp` `llama-server` binary bundled as a sidecar process under a Win32 Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + process memory limit), spoken to over localhost HTTP.
- Voice transcription (F1): On-device `whisper.cpp` sidecar or quantized Whisper/PhoWhisper via `candle`/`ort`, verified on Vietnamese across Windows 10 22H2 and Windows 11.
- Serialization & schemas: `serde` + `serde_json` + `schemars` (compile-time, zero reflection; `schemars` generates JSON Schemas for constrained decoding).
- Async runtime & logging: `tokio` multi-thread runtime + `tracing` with rotating file appender (`tracing-appender`).
- Cloud proxy: Separate `axum` service (`gateway/ef-gateway`) sharing wire/protocol types via `ef-protocol`. API keys never ship to clients.

## 6. Architecture invariants

Break any of these and the review fails. They exist because each one was expensive to discover.

**Every model call goes through the router.** `ef_ai::Router` is the only place a model or tier is chosen. `clippy.toml` (`disallowed-types`) forbids `reqwest::Client` outside `ef-ai-transport` and `ef-gateway`, and `ef-ai-transport` requires a `RouterDispatchToken` issued only by `ef_ai::Router`.

**The model never emits a number that matters.** No dates, times, durations, deadlines, or numeric priorities in any model output schema. The model copies the user's temporal phrase verbatim into a dedicated field and a deterministic parser resolves it against the user's time zone. All durations come from the estimation model. A hallucinated deadline is the fastest way to destroy trust in a tool whose whole promise is that you can stop holding things in your head. Enforce this with a unit test inspecting the `schemars` JSON schemas.

**Always check `stop_reason` before trusting a structured response.** Constrained decoding makes schema violations rare, not impossible: the guarantee is void when generation hits the token cap or the model refuses. Truncation and refusal route straight to the deterministic fallback. They do not count as validation failures and do not consume the retry.

**Two strikes, then fall back.** A failed validation gets exactly one retry with the error appended. A second failure returns the deterministic fallback. Never a third attempt.

**Every write to a synced table goes through the op-log.** A raw `INSERT` or `UPDATE` on `task`, `note`, `inbox_item`, `user_profile`, `transcript`, or `project` outside `ef_oplog::OpLog::apply` is a defect. `ef-oplog` encapsulates `SyncWriteConn` privately so other crates cannot obtain a raw write handle for synced tables. Device-local tables are written normally through `ef-storage`.

**Never auto-set `accepted_at`.** It is set only by an explicit user action requiring `ef_core::UserAcceptedProof` (a non-deserializable sealed token). Search for `accepted_at` before touching anything near it.

**Never write to the install directory.** MSIX makes it read-only. All paths must resolve through `ef_win32::resolve_app_paths()`, which checks `GetCurrentPackageFullName` and uses `ApplicationData::Current()?.LocalFolder()` when packaged or `%LOCALAPPDATA%\EfAssistant-Dev` when unpackaged in dev/tests.

**Energy and emotional state are self-reported only.** Never inferred from typing cadence, voice, camera, calendar density, or activity patterns. The database enforces this with a check constraint; do not relax it.

**Never increase demand in response to failure.** A missed task reduces scope. It never escalates urgency, and it never produces guilt copy.

**Banned vocabulary in user-facing copy.** These words must not appear in app strings, store listings, or marketing: treat, treatment, therapy, therapeutic, clinical, diagnose, diagnosis, symptom, patient, intervention, disorder, dysregulation, cure, remedy. Say "a short break" not "an intervention", "helps you get started" not "treats executive dysfunction". Checked in CI with a word-boundary regex scoped to `i18n/*.json` and `packaging/msix/AppxManifest.xml` (plus store metadata). Code identifiers and comments are exempt.

## 7. Coding standards

- Workspace lints deny all compiler warnings (`warnings = "deny"`) and `unsafe_op_in_unsafe_fn = "deny"`.
- `#![forbid(unsafe_code)]` is required in every crate except `ef-win32`. All Win32 FFI, hooks, and raw `HWND` manipulation live inside `ef-win32`.
- Never block the UI thread or `tokio` worker threads: Win32 `SetWinEventHook` runs on a dedicated `std::thread` with `HWND_MESSAGE` + `GetMessageW`; UIA runs on a dedicated MTA `std::thread` with a hard 250 ms `tokio::time::timeout`; SQLite writes serialize through a dedicated writer thread in WAL mode.
- Validate every deserialized model response before it touches the database. Model output is untrusted input.
- Prompts are versioned files under `crates/ef-prompts/`, never inline strings. Every AI-produced row records `origin`, `origin_model`, and `origin_prompt_ver`.
- All user-facing copy lives in `i18n/*.json` so the banned-vocabulary check can scan it from one place.
- All app-defined UI color tokens live in `ui/tokens/palette.json` and every foreground/background pairing is declared in `build/contrast-pairs.json`.
- Feature flags for every AI feature, so it can be disabled per user without a release.
- Architecture decision records under `docs/adr/` for anything that changes the stack, the model routing, or the data layer.
- When this document and the code disagree, stop and ask. Do not silently pick one.

## 8. Testing requirements

- Prompt golden tests per version. A change that alters golden output requires a version bump.
- A schema test asserting no date, time, duration, deadline, or numeric-estimate field exists in any `schemars` model output schema. A confidence score is explicitly allowed.
- Unit tests for every output validator, including adversarial inputs.
- Accessibility checks (`axe-core` + contrast checks) in CI, plus a manual NVDA and Narrator pass per release.
- A degradation suite that runs with the network off, the sidecar killed, and no model present. All must pass.
- A twenty-four hour soak on the Win32 message-pump thread.
- Convergence property tests (`proptest`) for the sync layer, described in the data-layer phase.

## 9. Metrics

Track: time from hotkey to captured item, decomposition acceptance rate split by accepted clean, accepted with edit, and discarded, estimation calibration, cost per active user, share of calls served locally, schema validation failure rate, and sidecar crash rate.

Never track and never display: streaks, completion percentage as a score, comparison to other users, or any productivity index. These are precisely the mechanics this product exists to avoid.

## 10. Phase index

Build in this order. Each phase file states its own prerequisites, scope, explicit non-scope, and definition of done.

- `phases/PHASE-00-foundation.md` - Cargo workspace, MSIX packaging, Store onboarding, CI, and the de-risking spikes that can invalidate the plan.
- `phases/PHASE-01-data-layer.md` - SQLite/SQLCipher schema, migrations, `ef-oplog`, hybrid logical clock, repositories.
- `phases/PHASE-02-native-shell.md` - tray, global hotkey, native Win32 F1/F7 overlays, ephemeral WebView2 lifecycle, Win32 activity monitor.
- `phases/PHASE-03-capture.md` - F1 end to end: text, voice, and image into the inbox.
- `phases/PHASE-04-ai-layer.md` - the tier router, `axum` cloud gateway, prompt contracts, output validators, and F2 structuring.
- `phases/PHASE-05-decompose-ui.md` - F3 decomposition, F11 workspace, settings surface in ephemeral WebView2.
- `phases/PHASE-06-estimation-alpha.md` - F8 pure-Rust estimation model, time display, alpha packaging and ship.

Phases 00 through 02 have no dependency on any model. Do not let AI work block the foundation.
