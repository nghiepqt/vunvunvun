# Phase 00 - Foundation and de-risking

Read `../CONTEXT.md` first.

## Goal

Stand up the repository, the packaging pipeline, and the Store account, and prove out the four assumptions that could invalidate the whole plan. Nothing in later phases should discover a blocker that this phase could have found in a week.

Estimated two to three weeks. No product features are built here.

## Prerequisites

None.

## Scope

### 1. Solution skeleton

Create the solution with these projects. Empty is fine; the point is that the dependency graph is right from the start.

```
src/
  EfAssistant.App/          WinUI 3, the only project containing XAML
  EfAssistant.Core/         domain types, UserProfile, no I/O
  EfAssistant.Data/         Sqlite, migrations, repositories, op-log
  EfAssistant.Ai/           router, providers, prompts, validators
  EfAssistant.Ml/           OnnxRuntime sessions
  EfAssistant.Activity/     Win32 hook thread, UIA, idle
  EfAssistant.Sync/         Hlc, ops
  EfAssistant.Gateway/      ASP.NET Core cloud proxy
tests/
  EfAssistant.Data.Tests/
  EfAssistant.Ai.Tests/
  EfAssistant.Sync.Tests/
native/llama-server/        bundled sidecar binary
resources/onnx/             model artifacts
docs/adr/
```

Dependency rules to enforce from day one: `Core` depends on nothing. `App` depends on everything but is depended on by nothing. `Data`, `Ai`, `Ml`, `Activity`, and `Sync` may depend on `Core` and on each other only where the architecture requires it. No project references `App`.

Enable nullable reference types and warnings as errors across the solution.

### 2. MSIX packaging

Get a packaged build producing an installable MSIX on a clean machine. Declare `runFullTrust` in the package manifest.

Also produce a self-contained unpackaged build for the direct-download channel, using `WindowsAppSDKSelfContained`. It costs installer size and buys an install with no runtime prerequisite.

### 3. Store onboarding

Start this on day one. It has the longest lead time and no code depends on it.

- Register a free individual developer account at `storedeveloper.microsoft.com`. Registration is free and covers roughly two hundred markets, but requires government ID and selfie verification. Confirm the target market is covered before planning around it.
- Submit a skeleton MSIX early: a hello-world carrying the real package identity and the `runFullTrust` declaration. `runFullTrust` is a restricted capability requiring Microsoft approval. Find out now whether it is granted and how long it takes.
- Verify a Store-signed package produces no SmartScreen warning on a clean machine.
- Separately, submit the build to Microsoft Defender and two or three major antivirus vendors and record turnaround. A tray application that hooks foreground events, registers a global hotkey, and spawns a child process matches the behavioral profile of a keylogger and will be flagged.
- Price the direct-download fallback: an organization-validated certificate at roughly 150 to 300 US dollars a year, plus a reputation ramp of weeks. Extended-validation certificates no longer bypass SmartScreen and are not worth the premium.

### 4. Spike W1 - Win32 activity detection inside a package

Prove that `GetForegroundWindow`, `GetWindowThreadProcessId`, `QueryFullProcessImageNameW`, `GetLastInputInfo`, and `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` all behave identically inside an MSIX package with `runFullTrust` as they do unpackaged.

This must be run packaged, not unpackaged. An earlier plan for macOS died because the App Sandbox silently disables the equivalent API even when the user grants permission. A packaged Windows desktop app runs at medium integrity level and not in an AppContainer, so these should work, but assume nothing and prove it.

Implementation notes for the hook:

- It must live on a thread that owns a `GetMessageW` loop. Create a dedicated background thread, create a message-only window with `HWND_MESSAGE` as parent, install the hook, and run the pump.
- It must not go on the WinUI dispatcher thread. A blocking callback there stalls the UI.
- Forward events to the application over a `System.Threading.Channels.Channel<T>`.

Gate: all five APIs work packaged, and the message-pump thread survives a twenty-four hour soak without leaking handles or dropping events.

### 5. Spike W2 - Overlay behavior

Two overlay types with opposite focus semantics. Prove both.

The nudge overlay appears unprompted and must not steal focus. Set `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` on the raw window handle, obtained with `WinRT.Interop.WindowNative.GetWindowHandle`, after the window exists.

The capture overlay is opened deliberately by the user and must take focus, because they are about to type. Set `WS_EX_TOOLWINDOW` only. Before showing it, save the current `GetForegroundWindow()`. On dismiss, call `SetForegroundWindow` on the saved handle while your overlay is still the foreground window, then hide it. Hide first and Windows silently refuses the call, because only the foreground process or the process that received the last input event may set the foreground window.

Critical accessibility problem to solve in this spike, not later: a `WS_EX_NOACTIVATE` window never becomes active, so by construction it cannot take keyboard focus and screen readers will not announce it. The non-functional requirements make keyboard operability mandatory. The nudge overlay therefore needs a second route: a dedicated global shortcut that clears `WS_EX_NOACTIVATE` and focuses the window, plus a live-region announcement when it appears. Mouse-only dismissal is not acceptable.

Gate: both overlays behave correctly on a mixed-DPI two-monitor setup and with a fullscreen video playing, and the nudge overlay is fully operable by keyboard and announced by NVDA.

### 6. Spike W3 - Sidecar and structured output

Bundle `llama-server.exe` in the MSIX payload and launch it with `Process.Start`. Point it at a model file in `ApplicationData.Current.LocalFolder`; the install directory is read-only under MSIX.

Benchmark two models on decomposition at granularity levels 1 through 3, which is the only range that will ever be routed locally:

- Phi-4-mini 3.8B, MIT licensed, the default.
- Qwen 3.5 4B, Apache 2.0, the alternative and the only one with a small sibling if speculative decoding is ever wanted.

Measure JSON validity under a supplied schema, hand-rated step quality, p95 latency, peak working set, and cold start time.

Also measure granularity 5 as information only. It routes to the cloud by design, but the token count feeds the output cap decision in the AI phase.

Useful flags: `--sleep-idle-seconds -1` disables the server's own idle sleep so the supervisor owns the lifetime, and `--slots` exposes the endpoint used later to erase a slot's prompt cache under memory pressure. There is no `--keep-alive` flag; that belongs to a different tool.

Gate: at least one model produces acceptable granularity-3 output in under 3.5 seconds on a mid-range machine. If neither does, local decomposition is descoped and the product stays cloud-first for that feature.

### 7. Spike W4 - Database and extensions

Verify inside an MSIX package that `Microsoft.Data.Sqlite` with the chosen `SQLitePCLRaw` bundle can enable and load the `sqlite-vec` extension, and that FTS5 is available. Confirm the primary key type the schema intends to use is actually accepted by `vec0`; every official example uses an integer primary key, and a text primary key may not be supported.

Gate: a written answer on both, because the data-layer phase depends on it.

### 8. Continuous integration

- Build both packaged and unpackaged configurations.
- Run all test projects.
- Accessibility contrast check.
- Banned-vocabulary check against the localization catalog, word-boundary regex, with an allowlist file. Scope it to user-facing strings only; if it runs over identifiers it will fail on the project's own domain vocabulary.

### 9. Architecture decision records

Write three, even briefly:

- ADR-001: WinUI 3 and .NET. Already decided, recorded so it is not reopened.
- ADR-002: sync conflict resolution shape, specifically whether per-field clocks live in their own table or are derived from the op-log.
- ADR-003: prompt caching strategy, resolved during the AI phase but stubbed now.

## Out of scope

No product features. No UI beyond what a spike needs. No AI integration beyond the sidecar benchmark. No data model implementation; that is phase 01.

## Definition of done

- A packaged MSIX installs cleanly on a fresh Windows 11 machine and on a Windows 10 22H2 machine.
- The Store account exists and a skeleton submission has been made, with a known answer on `runFullTrust`.
- All four spikes have written outcomes, including the ones that failed.
- CI is green and runs on every push.
- The three ADRs exist.
