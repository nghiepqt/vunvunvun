# Phase 02 - Native shell

Read `../CONTEXT.md` first.

## Goal

Build everything that makes this feel like a resident Windows application rather than a window: the tray presence, the global hotkey, both overlay window types, and the background activity monitor. No product features, no model calls, no database beyond what already exists.

This phase is where the performance budget for capture is won or lost.

Estimated two weeks.

## Prerequisites

Phase 00, specifically spikes W1 and W2. This phase productionizes what those proved.

## Scope

### 1. Application lifetime

Single-instance. A second launch signals the running instance and exits.

Start minimized to tray. Closing the main window hides it rather than exiting; only an explicit Exit from the tray menu terminates.

Launch at login, user-toggleable, off by default.

Host everything under `Microsoft.Extensions.Hosting` so background services have a managed lifetime and dependency injection is available everywhere.

### 2. Tray icon

WinUI 3 has no built-in tray icon. Use `H.NotifyIcon.WinUI`.

Menu: Open, Capture, Pause activity monitoring, Settings, Exit. Keep it short; a long tray menu is itself a decision cost.

The icon must have a tooltip and must be reachable by keyboard through the notification area, which Windows handles if the control is used correctly.

### 3. Global hotkey

Register through CsWin32-generated `RegisterHotKey`. Default is Ctrl+Shift+Space, user-rebindable.

Registration can fail if another application already owns the combination. Surface that failure in the UI with a clear message and a prompt to pick another. Do not swallow it; a silently dead hotkey is the worst possible failure for the core interaction.

Unregister on exit.

### 4. Capture overlay window

Borderless, always on top, centered on the monitor containing the cursor, sized for a single input plus a hint line.

Focus semantics, which are the opposite of the nudge overlay:

- Before showing, record `GetForegroundWindow()`.
- Show and take focus. The user is about to type; a window that refuses focus would send their keystrokes to whatever was behind it.
- Set `WS_EX_TOOLWINDOW` on the raw handle so it stays out of Alt-Tab and the taskbar. Do not set `WS_EX_NOACTIVATE` here.
- On dismiss, call `SetForegroundWindow` on the saved handle while this overlay is still the foreground window, then hide. Hiding first makes the call fail silently, because Windows only honors it from the foreground process or the process that received the last input event.

Performance requirement: visible within 150 ms of the hotkey, measured cold with the application idle in the tray. Creating a window at hotkey time will not meet this. Create it hidden at startup and only show it on hotkey. Note that Windows trims a background process's working set, so measure a genuinely cold press rather than a warm one.

Escape dismisses without saving. Enter commits. Both must work with a screen reader active.

### 5. Nudge overlay window

Not used by any MVP feature, but built now because its constraints shape the window infrastructure and it is far cheaper to get right once than to retrofit.

Appears unprompted, so it must not steal focus. Set `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` on the raw handle after the window exists.

A `WS_EX_NOACTIVATE` window never becomes active, so by construction it cannot take keyboard focus and screen readers will not announce it. Mouse-only dismissal is not acceptable. Implement the second route:

- A dedicated global shortcut that clears `WS_EX_NOACTIVATE`, focuses the window, and lets the user act on it by keyboard.
- An automation live-region announcement when the overlay appears, so a screen reader user knows it is there at all.
- Restore `WS_EX_NOACTIVATE` when it is dismissed.

Never a red alert, never a countdown, never a sound by default.

### 6. Multi-monitor and DPI

Per-monitor DPI awareness version 2. Re-layout both overlays on DPI change. Test on a mixed 100 percent and 150 percent two-monitor setup; this is where overlay code breaks, consistently.

Place overlays on the monitor containing the cursor, not the primary monitor.

### 7. Activity monitor

A hosted background service in `EfAssistant.Activity`.

Foreground tracking runs on a dedicated background thread that owns a message-only window and a `GetMessageW` loop, with `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` installed on it. It must not run on the dispatcher thread; a blocking callback there stalls the UI. Forward events over a `Channel<T>`.

Idle time comes from `GetLastInputInfo`. Two documented traps that will produce wrong behavior if ignored:

- It is session-local, not system-wide. Fine for a per-user desktop application, but do not describe or treat it as global.
- Its tick count is not guaranteed to increase monotonically. In some cases it returns a value lower than a previous reading. Clamp negatives to zero and guard against a nonsense drop, or the stall detector built in a later phase will occasionally fire for no reason.

On the lock screen, idle keeps accumulating. Register for session change notifications and suppress activity sampling while the session is locked, or every lunch break registers as a stall.

Window titles are read with `GetWindowTextW` when needed. It reads the cached caption without sending a window message, so it cannot be hung by an unresponsive target and it works against elevated same-user applications. Do not store titles; the sample table has no column for them.

UI Automation, if used at all, goes on its own thread with a hard 250 ms timeout, because it can block indefinitely on some applications, notably Office and Teams. On timeout, fall back to foreground window plus process name and record the timeout. Never call it from the dispatcher thread or from a thread-pool thread whose blocking matters.

Sample every fifteen seconds into `activity_sample`: timestamp, process name, idle seconds, foreground switch count in the last sixty seconds, and the active task identifier once tasks exist.

A user-facing pause toggle in the tray menu stops sampling entirely. Respect it immediately, not on the next tick.

### 8. Notifications

Toast notifications through the Windows App SDK. Register a Start Menu shortcut carrying the application user model identifier, or toast actions will not work.

Do Not Disturb and Focus sessions suppress toasts silently and there is no supported way around it. Treat a toast as best-effort delivery. Any user-visible signal that matters must also have an in-application path.

## Out of scope

No capture logic; the overlay is an empty shell that raises an event. No database writes. No models. No stall detection; the samples are collected and stored, nothing reads them yet.

## Definition of done

- Single instance, tray, launch at login, and clean exit all work from a packaged build.
- Hotkey press to overlay visible measures under 150 ms cold, on a machine that has been idle for at least ten minutes.
- Dismissing the capture overlay returns focus to the previously active application, verified by typing into it immediately afterward.
- The nudge overlay demonstrably does not steal focus, is operable by keyboard through its dedicated shortcut, and is announced by NVDA.
- Both overlays render correctly on a mixed-DPI two-monitor setup and over a fullscreen video.
- The message-pump thread survives twenty-four hours without leaking handles or missing foreground events.
- Idle detection returns sane values across a screen lock and unlock cycle, and never a negative interval.
- The pause toggle stops sampling within one tick.
