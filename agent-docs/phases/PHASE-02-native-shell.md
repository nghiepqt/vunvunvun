# Phase 02 - Native shell

Read `../CONTEXT.md` first.

## Goal

Build everything that makes Vun feel like a fast, reliable resident Windows application rather than a heavy browser window: single-instance lifecycle, system tray, global hotkey, both native Win32 overlay windows (F1 & F7), the ephemeral WebView2 workspace host, and the background Win32 activity monitor.

This phase is where the <150 ms F1 wake and <150 MB idle RAM budgets are won or lost.

Estimated two weeks.

## Prerequisites

Phase 00, specifically Spikes W1 and W2.

## Scope

### 1. Application lifetime (`ef-app` + `ef-win32`)

- Single-instance enforced via a named Win32 mutex (`CreateMutexW`) and wakeup signal to bring up the existing instance on second launch.
- Starts minimized to system tray. Closing the main Workspace window **destroys** the ephemeral WebView2 instance (`close()` / drop) rather than hiding it in RAM; only explicit Exit from the tray menu terminates the resident daemon.
- Launch at login: user-toggleable (`StartupTask` / Run key), off by default.
- Structured lifecycle supervised under `tokio` + dedicated OS threads where Win32 message pumps or COM apartments require thread affinity.

### 2. Tray icon

- Implemented via `tray-icon` or direct `Shell_NotifyIconW` inside `ef-win32` (zero XAML/WebView dependency).
- Short menu: Open Workspace, Capture, Pause activity monitoring, Settings, Exit.
- Tooltip and keyboard accessibility through the Windows notification area.

### 3. Global hotkey

- Registered via `RegisterHotKey` (`Ctrl+Shift+Space` default, user-rebindable).
- If another app already owns the key combination, surface a clear non-blocking warning and allow rebinding—never fail silently.
- Unregister cleanly on exit.

### 4. F1 Capture overlay window (Native Win32)

- Borderless, always-on-top, centered on the monitor containing the cursor, hosting a native `MSFTEDIT_CLASS` RichEdit control + hint line + optional DWM Mica/Acrylic backdrop.
- Focus semantics (opposite of F7):
  - Before showing, record `GetForegroundWindow()`.
  - Show and take focus (`WS_EX_TOOLWINDOW` only; never `WS_EX_NOACTIVATE`).
  - On dismiss (Enter or Escape), call `SetForegroundWindow(saved_hwnd)` *while the overlay is still foreground*, then hide it.
- Pre-created hidden at startup so cold wake is ~5–15 ms (well below the 150 ms budget, even after Windows trims background working set).
- Supports Enter to commit, Shift+Enter for newline, Escape to dismiss, Vietnamese IME (Unikey/EVKey/Windows Telex), and NVDA/Narrator via native RichEdit `ITextProvider`.

### 5. F7 Nudge overlay window (Native Win32)

- Built now so `WS_EX_NOACTIVATE` + accessibility constraints are solved once in `ef-win32`.
- Appears unprompted with `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` set on the `HWND` so it never steals focus.
- Accessibility second route (mandatory):
  - A dedicated global shortcut clears `WS_EX_NOACTIVATE`, focuses the window, and allows full keyboard navigation/dismissal.
  - Triggers a UIA live-region announcement on appear so screen reader users know it is present.
  - Restores `WS_EX_NOACTIVATE` upon dismissal.
- Never red alert, never countdown, never sound by default.

### 6. Ephemeral Workspace shell (`wry` + `tao`)

- Opening "Open Workspace" or "Settings" constructs the `wry` WebView2 window on the UI thread, binds the typed IPC channel to Rust Core, and navigates to the local bundled `ui/workspace` assets.
- Closing the window explicitly destroys the WebView2 controller so `msedgewebview2.exe` processes exit and resident idle memory returns below 40 MB.

### 7. Multi-monitor and DPI

- `PerMonitorV2` declared in `crates/ef-app/app.manifest`.
- Handle `WM_DPICHANGED` to re-layout both native overlays cleanly on mixed 100%/150% monitor setups.
- Position overlays on the monitor containing the cursor (`MonitorFromPoint` + `GetMonitorInfoW`).

### 8. Activity monitor (`ef-win32`)

- Dedicated `std::thread` owning an `HWND_MESSAGE` window and `GetMessageW` loop with `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)`. Never run on the UI loop or `tokio` worker pool. Forward via bounded `tokio::sync::mpsc::Sender` (`try_send`).
- Idle seconds from `GetLastInputInfo`: session-local only; guard against non-monotonic tick wrap/drop using saturating arithmetic (`saturating_sub`, clamp negatives to 0).
- Register `WTSRegisterSessionNotification` and suppress sampling while the session is locked.
- Read cached window titles only via `GetWindowTextW` if needed (never store titles in `activity_sample`).
- UIA fallback (`IUIAutomation`) lives on its own dedicated MTA `std::thread` wrapped by `tokio::time::timeout(Duration::from_millis(250), ...)`. On timeout, fall back to foreground `HWND` + `QueryFullProcessImageNameW`.
- Sample every 15 seconds into `activity_sample`. Tray pause toggle halts sampling immediately.

### 9. Notifications

- Best-effort Windows Toast notifications via `windows::UI::Notifications::ToastNotificationManager` (with registered Start Menu AUMID shortcut). Always provide an in-app path because Do Not Disturb / Focus sessions suppress toasts silently.

## Out of scope

No capture storage logic beyond emitting stub events, no AI model calls, no stall detection.

## Definition of done

- Single instance, tray, launch at login, and clean exit work from MSIX and standalone `vun.exe`.
- Cold hotkey-to-visible F1 overlay measures under 150 ms after 10+ minutes idle.
- Dismissing F1 restores focus and delivers subsequent keystrokes to the prior app immediately.
- F7 never steals focus, is operable by keyboard via its dedicated shortcut, and is announced by NVDA.
- Opening and closing the WebView2 Workspace cleanly destroys `msedgewebview2.exe` and returns idle RAM below 150 MB (target ~15–30 MB).
- Both overlays render cleanly on mixed-DPI two-monitor setups and over fullscreen video.
- Message-pump thread passes a 24-hour handle/event soak test; idle detection survives lock/unlock with zero negative intervals.
