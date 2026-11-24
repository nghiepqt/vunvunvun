# Phase 00 - Foundation and de-risking

Read `../CONTEXT.md` first.

## Goal

Stand up the Rust Cargo workspace, the MSIX packaging pipeline, and the Store onboarding path, and prove out the four assumptions that could invalidate the whole plan. Nothing in later phases should discover a blocker that this phase could have found in a week.

Estimated two to three weeks. No product features are built here.

## Prerequisites

None.

## Scope

### 1. Cargo workspace skeleton

Create the workspace with these crates and directories. Empty/minimal is fine; the point is that the dependency DAG and compile-time isolation guardrails are enforced from the start.

```text
Cargo.toml                  Workspace root (Edition 2024, resolver = "3")
clippy.toml                 Disallows reqwest::Client outside ef-ai-transport / ef-gateway
.cargo/config.toml          Uses rust-lld on x86_64/aarch64 Windows MSVC targets
crates/
  ef-core/                  Domain types, UserProfile, HLC, UserAcceptedProof (zero internal deps)
  ef-protocol/              Shared wire DTOs, FeatureId, AiTier, StopReason, LlmUsageRecord
  ef-oplog/                 SyncMutation + OpLog::apply(); owns private SyncWriteConn
  ef-storage/               Device-local SQLite/SQLCipher tables, migrations, FTS5/vec store
  ef-prompts/               Versioned prompt files, schemars output schemas, golden tests
  ef-ai-transport/          Only client crate allowed to hold reqwest::Client
  ef-ai/                    Router, 7-stage Validator, response cache
  ef-ml/                    Pure-Rust GBDT quantile evaluator (M1/M2), LinUCB (M3), embeddings
  ef-sidecar/               llama-server supervisor, Foundry Local probe, heartbeat, backoff
  ef-win32/                 ONLY crate permitted to use unsafe: Win32 hooks, UIA, native overlays, Job Object
  ef-app/                   Composition root (binary: vun), tray, ephemeral WebView2 bridge
gateway/
  ef-gateway/               Separate axum cloud proxy service sharing ef-protocol
ui/
  tokens/                   palette.json shared across WebView2 and Win32 overlays
  workspace/                Ephemeral WebView2 frontend (HTML/CSS/TS)
i18n/                       Single localization catalog (en.json, vi.json)
packaging/msix/             AppxManifest.xml and Store Assets/
native/llama-server/        Bundled sidecar binary (acquired out of band)
resources/onnx/             Model/tree/tokenizer artifacts (acquired out of band)
docs/adr/                   Architecture decision records
```

Dependency and safety rules enforced from day one:
- `ef-core` depends on no internal workspace crate.
- Nothing depends on `ef-app` or `ef-gateway`.
- `#![forbid(unsafe_code)]` at the top of every crate except `ef-win32` (which enforces `#![deny(unsafe_op_in_unsafe_fn)]`).
- Workspace `[workspace.lints.rust]` denies all warnings (`warnings = "deny"`), and `clippy.toml` forbids `reqwest::Client` outside `ef-ai-transport` and `ef-gateway`.

### 2. MSIX packaging and standalone fallback

- Package the compiled Rust executable (`vun.exe`) into an installable MSIX via `build/Pack-Msix.ps1` (`MakeAppx.exe`) for both `x64` (`x86_64-pc-windows-msvc`) and `ARM64` (`aarch64-pc-windows-msvc`).
- Declare `<rescap:Capability Name="runFullTrust" />` and `EntryPoint="Windows.FullTrustApplication"` in `packaging/msix/AppxManifest.xml`.
- Because Rust compiles to native code with no .NET or Windows App SDK runtime prerequisite, the standalone `vun.exe` (with `app.manifest` declaring `PerMonitorV2` DPI awareness) also serves as the unpackaged direct-download binary.

### 3. Store onboarding

Start account setup in parallel—never let Store registration block coding (`cargo run` / `cargo test` and local self-signed MSIX sideloading work immediately):

- Register a free individual developer account at `storedeveloper.microsoft.com` ($0 fee, covers ~200 markets; requires government ID and selfie verification).
- Reserve the app name (`ThanhNghiepQuan.Vun` / `Vun`).
- Note on avoiding Store Policy 10.1 rejection: do **not** submit an empty "Hello World" window, as Store reviewers frequently reject placeholder apps. Instead, as soon as the minimal functional Capture-to-Inbox shell lands (Phase 03 or a minimal functional capture build), submit an MSIX package under **Hidden / Private Audience** visibility with a clear `runFullTrust` justification in *Submission options* (explaining desktop global hotkey, foreground window capture return, and local sidecar execution).
- Verify a Store-signed MSIX produces no SmartScreen warning on a clean machine.
- Separately, submit the release binary to Microsoft Defender and two or three major antivirus vendors to clear false positives (a tray app using `SetWinEventHook`, `RegisterHotKey`, and child processes resembles keylogger heuristics).
- Price the direct-download fallback: an organization-validated (OV) certificate at roughly $150–300/year plus SmartScreen reputation ramp. Extended-validation (EV) certificates no longer bypass SmartScreen.

### 4. Spike W1 - Win32 activity detection & Job Object inside MSIX

Prove that `GetForegroundWindow`, `GetWindowThreadProcessId`, `QueryFullProcessImageNameW`, `GetLastInputInfo`, `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`, and `CreateJobObjectW` (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) all behave identically inside a packaged MSIX (`runFullTrust`) as they do unpackaged.

Implementation notes for the hook:
- Run on a dedicated `std::thread` owning a message-only window (`HWND_MESSAGE`) and a synchronous `GetMessageW` loop—never on a `tokio` worker thread or UI thread.
- Forward events over a bounded `tokio::sync::mpsc` channel (`try_send`, never blocking the OS hook callback).

Gate: all APIs work packaged in MSIX, and the message-pump thread survives a twenty-four hour soak without leaking handles or dropping events.

### 5. Spike W2 - Split-Surface UI overlays and WebView2 lifecycle

Prove both native Win32 overlays and the ephemeral WebView2 teardown:

- **F1 Capture Overlay (native Win32 + `MSFTEDIT_CLASS` RichEdit + DWM backdrop)**:
  - Set `WS_EX_TOOLWINDOW` only (must take focus).
  - Save `GetForegroundWindow()` before showing; on dismiss, call `SetForegroundWindow` on the saved handle *while the overlay is still foreground*, then hide.
  - Verify cold hotkey-to-visible latency is well below 150 ms (target ~5–15 ms), Vietnamese IME (Unikey / EVKey / Windows Telex) composes cleanly, and NVDA/Narrator reads the input via native UIA `ITextProvider`.
- **F7 Nudge Overlay (native Win32)**:
  - Set `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` after window creation so it never steals focus.
  - Because `WS_EX_NOACTIVATE` blocks keyboard focus and screen reader announcement, implement a dedicated global shortcut that clears `WS_EX_NOACTIVATE` and focuses the window, plus a UIA live-region announcement on show.
- **F11 Ephemeral WebView2 (`wry` + `tao`)**:
  - Open the workspace window, close it completely (`close()` / drop webview), and confirm `msedgewebview2.exe` child processes terminate and resident tray RAM drops back below 40 MB (well under the 150 MB budget).

Gate: both overlays behave correctly on a mixed-DPI (100%/150%) two-monitor setup and over fullscreen video, Vietnamese IME + NVDA pass on F1, F7 is keyboard-operable and NVDA-announced, and WebView2 releases memory on close.

### 6. Spike W3 - Sidecar and structured output

Bundle `llama-server.exe` in the MSIX payload and spawn it inside a Win32 Job Object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`). Point it at model files via `ef_win32::resolve_app_paths()` (`ApplicationData::Current()?.LocalFolder()` when packaged, `%LOCALAPPDATA%\EfAssistant-Dev` when unpackaged).

Benchmark two permissive-license models on decomposition at granularity levels 1–3 using `schemars`-generated JSON schemas (`json_schema` / GBNF constrained decoding):
- Phi-4-mini 3.8B (MIT, default).
- Qwen 3.5 4B (Apache 2.0, alternative; has small sibling for speculative decoding).

Flags: `--sleep-idle-seconds -1` and `--slots`. (Do not use `--keep-alive`; that belongs to Ollama.)

Gate: at least one model produces acceptable granularity-3 output in under 3.5 seconds p95 on a mid-range machine.

### 7. Spike W4 - Static `sqlite-vec` + SQLCipher inside MSIX

Verify inside an MSIX package (x64 and ARM64) that `rusqlite` (`bundled-sqlcipher-vendored-openssl`, FTS5 enabled) statically links and auto-registers `sqlite-vec` via `sqlite3_auto_extension` in `build.rs`. Confirm the primary key mapping used with `vec0` (e.g., integer `rowid` join table mapping UUIDv7 `fragment.id` to `vec0` rowid if `vec0` requires integer primary keys).

Gate: written verification of static compilation, FTS5 `unicode61 remove_diacritics 0`, and `vec0` primary-key strategy.

### 8. Continuous integration

- Policy checks (`ubuntu-latest`): `python3 ./build/check_policy.py`, `./build/Test-BannedVocabulary.ps1`, and `./build/Test-Contrast.ps1`.
- Workspace build & test (`windows-latest`): `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
- MSIX packaging (`windows-latest`, `x64` & `arm64`): verify `.msix` creation and `runFullTrust` presence in the manifest.
- Standalone binary build (`windows-latest`, `x86_64-pc-windows-msvc`).

### 9. Architecture decision records

- `docs/adr/ADR-001-winui3-and-dotnet.md`: Marked superseded by ADR-004.
- `docs/adr/ADR-002-sync-conflict-resolution.md`: Sync conflict shape and HLC.
- `docs/adr/ADR-003-prompt-caching-strategy.md`: Prompt caching strategy (stubbed for Phase 04).
- `docs/adr/ADR-004-rust-and-split-surface-ui.md`: Rust + Split-Surface Hybrid UI (Option A).

## Out of scope

No product features. No UI beyond what spikes need. No AI integration beyond the sidecar benchmark. No full data model implementation; that is Phase 01.

## Definition of done

- A packaged MSIX installs cleanly on a fresh Windows 11 machine and on a Windows 10 22H2 machine.
- Partner Center developer registration is underway/complete and Store submission plan for `runFullTrust` is ready (never blocking local spike work).
- All four spikes have written outcomes, including any failures.
- CI (`policy-checks`, `build-and-test`, `package-msix`, `package-unpackaged`) is green on every push.
- ADRs 001–004 are present and consistent.
