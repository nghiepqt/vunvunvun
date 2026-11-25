# Spike W1 & Spike W2 Verification Checklist (Phase 02)

**Branch:** `feat/phase-02-native-shell`  
**References:** `agent-docs/CONTEXT.md`, `agent-docs/phases/PHASE-00-foundation.md` (Spikes W1 & W2), `agent-docs/phases/PHASE-02-native-shell.md`

---

## 1. Automated Verification (CI & Local)

### Cross-platform Unit & Integration Tests (Linux + Windows CI)
- `ef-win32`:
  - `activity::tests::compute_idle_seconds_is_never_negative_across_wrap_or_lock_unlock`: verifies 32-bit `GetTickCount()` wraparound and non-monotonic lock/unlock clamping to `>= 0`.
  - `activity::tests::process_basename_strips_directories_and_lowercases`: verifies only executable basename is kept (never directories or window titles).
  - `activity::tests::sampling_policy_suppresses_when_paused_or_locked`: verifies immediate suppression on tray pause and session lock.
  - `dpi::tests::mixed_dpi_scaling_and_negative_monitor_coords`: verifies 100%/150%/200% scaling and negative multi-monitor coordinates.
  - `hotkey::tests::default_hotkeys_format_and_roundtrip` & `hotkey_without_modifier_or_primary_key_is_rejected`.
  - `palette::tests::all_four_themes_load_from_palette_json`: verifies all 4 WCAG 2.1 AA themes load into Win32 `COLORREF`.
  - `toast::tests::toast_xml_escapes_special_chars_and_vietnamese`.
  - `single_instance::tests::single_instance_prevents_second_acquire_until_dropped`.
  - `startup::tests::launch_at_login_defaults_disabled_and_toggles_cleanly`.
- `ef-app`:
  - `i18n::tests::embedded_en_and_vi_catalogs_deserialize_completely`.
  - `ipc::tests::ipc_requests_roundtrip_json`.
  - `controller::tests::activity_tick_respects_pause_and_session_lock_and_records_non_negative_idle`: verifies end-to-end persistence into `ActivitySampleRepository` with SQLCipher.
  - `controller::tests::hotkey_conflict_surfaces_in_state_snapshot_without_failing_silently`.

### Windows-Only CI Tests (`windows-latest`, x64 & arm64)
- `ef_win32::job_object::tests::dropping_job_object_kills_assigned_child_process`: spawns a benign child, assigns to `JobObject` (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), drops `JobObject`, and asserts child terminates within 3 seconds.
- `ef_win32::dpi::tests::monitor_for_cursor_returns_sane_dimensions`.
- `ef_win32::activity::tests::activity_monitor_starts_resolves_fallback_and_shuts_down`: verifies `HWND_MESSAGE` pump, `SetWinEventHook`, `WTSRegisterSessionNotification`, and dedicated MTA `IUIAutomation` thread start and shut down cleanly.
- `Package MSIX (x64)` & `Package MSIX (arm64)`: verifies `AppxManifest.xml` includes `runFullTrust` and `windows.startupTask` (`VunStartupTask`).

---

## 2. Built-in Diagnostic Probes on Real Windows Hardware

Both probes are built into `vun.exe` (MSIX sideloaded and standalone `vun.exe`) so verification runs identically inside the `runFullTrust` AppContainer boundary and unpackaged:

1. **Spike W1 24-hour Soak & Handle/Event Probe**:
   ```powershell
   .\vun.exe --spike-w1-soak --duration-secs 86400 --report-interval-secs 300
   ```
   - Writes JSONL checkpoints to `%LOCALAPPDATA%\EfAssistant-Dev\logs\spike-w1-soak.jsonl` (or MSIX `LocalFolder\logs\spike-w1-soak.jsonl`).
   - Records `handle_count`, `working_set_bytes`, `events_received`, `events_dropped`, `job_object_ok`, and `idle_sec`.
   - Pass criteria: `events_dropped == 0`, `job_object_ok == true`, `idle_sec >= 0`, and `handle_count` shows zero monotonic leak over 24 hours.

2. **Spike W2 Cold-Wake, Focus & WebView2 Memory Probe**:
   ```powershell
   .\vun.exe --spike-w2-probe
   ```
   - Logs `wake_latency_ms` on every F1 trigger (`within_budget = wake_latency_ms < 150`) and process `working_set_mb` / `handle_count` before opening WebView2, while WebView2 is open, and immediately after closing WebView2 back to tray.

---

## 3. Manual Windows 10 22H2 & Windows 11 Hardware Checklist

Run this checklist on both **MSIX sideloaded** (`Vun_x64.msix` / `Vun_arm64.msix`) and **standalone `vun.exe`**:

### Lifecycle, Tray & Global Hotkeys
- [ ] Launch `vun.exe`: starts minimized in the system tray with no console window; Task Manager shows ~15–25 MB RAM and zero `msedgewebview2.exe` processes.
- [ ] Launch `vun.exe` a second time: second process exits cleanly (`0`) and wakes the Workspace window of the running instance.
- [ ] Toggle **Launch at login** on/off in Settings and tray menu; verify default is **off** on fresh install (`StartupTask` in Task Manager Startup tab when packaged, `HKCU\...\Run\Vun` when standalone).
- [ ] Hold `Ctrl+Shift+Space` in another test app (or register conflict externally) before starting Vun: confirm Vun surfaces a non-blocking banner in Workspace + best-effort Windows Toast and allows rebinding in Settings.
- [ ] Click **Pause activity sensing** in tray: confirm `activity_sample` inserts stop immediately; click **Resume** to restart 15s sampling.

### Spike W1 (MSIX `runFullTrust` Activity & Lock/Unlock)
- [ ] Switch between Win32 apps (Notepad, Explorer) and UWP/packaged apps (Settings, Calculator): confirm `QueryFullProcessImageNameW` + 250 ms MTA UIA fallback resolves process basenames cleanly without storing any window titles in SQLite.
- [ ] Lock desktop (`Win+L`), wait 30 seconds, unlock: confirm zero samples recorded while locked and `idle_sec >= 0` immediately on unlock.
- [ ] Run `--spike-w1-soak` for 24 hours inside MSIX: verify zero dropped events and flat handle count.

### Spike W2 (F1 Capture, F7 Nudge, Mixed-DPI, IME, Screen Readers, Ephemeral WebView2)
- [ ] **F1 Cold Wake (< 150 ms)**: Leave Vun idle in tray for > 10 minutes; press `Ctrl+Shift+Space` and verify `wake_latency_ms < 150` in `vun.log`.
- [ ] **F1 Focus Return**: Focus Notepad/VS Code, press `Ctrl+Shift+Space`, type `"hello"`, press `Enter` or `Esc`, and immediately type `"world"` without clicking — `"world"` must go directly into Notepad/VS Code.
- [ ] **Vietnamese IME**: Test **Unikey**, **EVKey**, and **Windows Telex** inside F1 Capture (`"Tiếng Việt có dấu"`, `Shift+Enter` newline, `Enter` submit).
- [ ] **F1 Accessibility**: With **NVDA** and **Windows Narrator**, verify the RichEdit input name (`"Capture thought"`) and typed text are read via native UIA `ITextProvider`.
- [ ] **F7 Nudge Non-Stealing Focus**: Trigger **Preview gentle check-in** while typing in an editor and while playing fullscreen YouTube/VLC video — active app must retain focus and fullscreen playback must stay uninterrupted.
- [ ] **F7 Accessibility Second Route**: On F7 appearance, confirm NVDA/Narrator announces the polite live region (`IAccPropServices` + `EVENT_OBJECT_LIVEREGIONCHANGED`). Press `Ctrl+Alt+Shift+N` to unlock focus, then navigate buttons with `Tab`, `Enter`, and `Esc`.
- [ ] **Mixed-DPI Two-Monitor Setup (100% + 150%)**: Move cursor to the 150% monitor and trigger F1 & F7; move to the 100% monitor and trigger again — both overlays center/dock cleanly inside work areas without clipping or taskbar overlap.
- [ ] **Ephemeral WebView2 Teardown**: Open Workspace, interact with Settings, then close the window to tray (repeat 5 times). Verify in Task Manager / Process Explorer that all `msedgewebview2.exe` child processes terminate and `vun.exe` resident RAM returns below **40 MB** (budget < 150 MB).
