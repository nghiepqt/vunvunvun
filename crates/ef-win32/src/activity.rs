use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot};

use crate::Win32Error;

pub const UIA_TIMEOUT: Duration = Duration::from_millis(250);
pub const ACTIVITY_EVENT_CHANNEL_CAPACITY: usize = 256;

/// Events emitted from the dedicated Win32 `SetWinEventHook` + WTS thread.
///
/// Privacy invariant (`CONTEXT.md` §4.4, `PHASE-02` §8): never carries window titles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityEvent {
    ForegroundChanged {
        hwnd_raw: isize,
        pid: u32,
        process_name: String,
        timestamp_ms: i64,
    },
    SessionLockChanged {
        locked: bool,
        timestamp_ms: i64,
    },
}

/// Pure policy determining whether a 15-second tick records an `activity_sample` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SamplingPolicy {
    pub paused: bool,
    pub session_locked: bool,
}

impl SamplingPolicy {
    #[must_use]
    pub const fn should_record(self) -> bool {
        !self.paused && !self.session_locked
    }
}

/// Diagnostic counters for the 24-hour Spike W1 message-pump soak test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActivityStatsSnapshot {
    pub events_received: u64,
    pub events_dropped: u64,
}

/// Process resource counters for Spike W1 & W2 verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcessDiagnostics {
    pub handle_count: u32,
    pub working_set_bytes: usize,
}

impl ProcessDiagnostics {
    #[must_use]
    pub fn working_set_mb(self) -> f64 {
        (self.working_set_bytes as f64) / (1024.0 * 1024.0)
    }
}

#[derive(Default)]
struct ActivityStats {
    events_received: AtomicU64,
    events_dropped: AtomicU64,
}

impl ActivityStats {
    fn snapshot(&self) -> ActivityStatsSnapshot {
        ActivityStatsSnapshot {
            events_received: self.events_received.load(Ordering::Relaxed),
            events_dropped: self.events_dropped.load(Ordering::Relaxed),
        }
    }
}

/// Computes non-negative idle seconds from 32-bit `GetTickCount()` values.
///
/// Uses wrapping unsigned subtraction to survive 49.7-day rollover, and clamps
/// apparent negative intervals (`delta > i32::MAX`) from session lock/unlock or
/// sleep transitions to `0` (`PHASE-02` §8).
#[must_use]
pub const fn compute_idle_seconds(now_tick_ms: u32, last_input_tick_ms: u32) -> i64 {
    let delta_ms = now_tick_ms.wrapping_sub(last_input_tick_ms);
    if delta_ms > i32::MAX as u32 {
        0
    } else {
        (delta_ms / 1000) as i64
    }
}

/// Extracts only the lowercase executable basename (e.g. `"code.exe"`), discarding
/// directories and any user-identifying path segments.
#[must_use]
pub fn extract_process_basename(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "unknown".to_string();
    }
    let name = trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed).trim();
    if name.is_empty() {
        "unknown".to_string()
    } else {
        name.to_ascii_lowercase()
    }
}

#[must_use]
pub fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Queries the current session idle duration in seconds (always `>= 0`).
#[must_use]
pub fn current_idle_seconds() -> i64 {
    #[cfg(windows)]
    {
        windows_impl::current_idle_seconds()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Returns current process handle count and working set RAM bytes.
#[must_use]
pub fn process_diagnostics() -> ProcessDiagnostics {
    #[cfg(windows)]
    {
        windows_impl::process_diagnostics()
    }
    #[cfg(not(windows))]
    {
        ProcessDiagnostics::default()
    }
}

/// Queries the current foreground process basename synchronously (used as fallback or for initial sample).
#[must_use]
pub fn current_foreground_process_name() -> Option<(isize, u32, String)> {
    #[cfg(windows)]
    {
        windows_impl::current_foreground_process()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

struct UiaProbeRequest {
    hwnd_raw: isize,
    reply: oneshot::Sender<Option<String>>,
}

/// Resident background activity monitor owning:
/// 1. `ef-win32-activity`: dedicated `std::thread` (`HWND_MESSAGE` + `GetMessageW` + `SetWinEventHook` + WTS).
/// 2. `ef-win32-uia`: dedicated MTA `std::thread` (`IUIAutomation`) bounded by a 250 ms timeout.
pub struct ActivityMonitor {
    stats: Arc<ActivityStats>,
    uia_tx: std::sync::mpsc::SyncSender<UiaProbeRequest>,
    #[cfg(windows)]
    pump_hwnd_raw: isize,
    pump_thread: Option<JoinHandle<()>>,
    uia_thread: Option<JoinHandle<()>>,
}

impl ActivityMonitor {
    pub fn start(event_tx: mpsc::Sender<ActivityEvent>) -> Result<Self, Win32Error> {
        let stats = Arc::new(ActivityStats::default());
        let (uia_tx, uia_rx) = std::sync::mpsc::sync_channel::<UiaProbeRequest>(16);

        let uia_thread = std::thread::Builder::new()
            .name("ef-win32-uia".to_string())
            .spawn(move || {
                #[cfg(windows)]
                windows_impl::run_uia_mta_loop(uia_rx);
                #[cfg(not(windows))]
                {
                    while let Ok(req) = uia_rx.recv() {
                        let _ = req.hwnd_raw;
                        let _ = req.reply.send(None);
                    }
                    tracing::debug!("ef-win32-uia stub thread stopped");
                }
            })
            .map_err(Win32Error::Io)?;

        #[cfg(windows)]
        {
            let (ready_tx, ready_rx) =
                std::sync::mpsc::sync_channel::<Result<isize, Win32Error>>(1);
            let stats_clone = Arc::clone(&stats);
            let pump_thread = std::thread::Builder::new()
                .name("ef-win32-activity".to_string())
                .spawn(move || {
                    windows_impl::run_activity_pump(event_tx, stats_clone, ready_tx);
                })
                .map_err(Win32Error::Io)?;

            let pump_hwnd_raw = ready_rx.recv().map_err(|_| Win32Error::ChannelClosed)??;

            Ok(Self {
                stats,
                uia_tx,
                pump_hwnd_raw,
                pump_thread: Some(pump_thread),
                uia_thread: Some(uia_thread),
            })
        }

        #[cfg(not(windows))]
        {
            let _ = event_tx;
            Ok(Self {
                stats,
                uia_tx,
                pump_thread: None,
                uia_thread: Some(uia_thread),
            })
        }
    }

    #[must_use]
    pub fn stats(&self) -> ActivityStatsSnapshot {
        self.stats.snapshot()
    }

    /// Resolves UWP / hosted child process name via the dedicated MTA UIA thread,
    /// enforcing a hard 250 ms `tokio::time::timeout` and falling back to `fallback_name`.
    pub async fn resolve_process_with_uia_fallback(
        &self,
        hwnd_raw: isize,
        fallback_name: String,
    ) -> String {
        if hwnd_raw == 0 {
            return fallback_name;
        }
        let (tx, rx) = oneshot::channel();
        if self
            .uia_tx
            .try_send(UiaProbeRequest {
                hwnd_raw,
                reply: tx,
            })
            .is_err()
        {
            return fallback_name;
        }
        match tokio::time::timeout(UIA_TIMEOUT, rx).await {
            Ok(Ok(Some(resolved))) if !resolved.is_empty() => resolved,
            _ => fallback_name,
        }
    }

    pub fn shutdown(mut self) {
        self.stop_threads();
    }

    fn stop_threads(&mut self) {
        #[cfg(windows)]
        {
            if self.pump_hwnd_raw != 0 {
                windows_impl::post_close(self.pump_hwnd_raw);
                self.pump_hwnd_raw = 0;
            }
        }
        if let Some(handle) = self.pump_thread.take() {
            let _ = handle.join();
        }
        // Dropping `uia_tx` (when `self` drops) closes the UIA loop, or if called explicitly
        // via `shutdown(self)` we replace with a disconnected sender so `join()` finishes.
        let (dummy_tx, _) = std::sync::mpsc::sync_channel(1);
        let old_tx = std::mem::replace(&mut self.uia_tx, dummy_tx);
        drop(old_tx);
        if let Some(handle) = self.uia_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ActivityMonitor {
    fn drop(&mut self) {
        self.stop_threads();
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use tokio::sync::mpsc;
    use windows::Win32::Foundation::{CloseHandle, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
    };
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessHandleCount, OpenProcess, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, HWINEVENTHOOK, IUIAutomation, SetWinEventHook, UnhookWinEvent,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
        GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HWND_MESSAGE, MSG,
        PostMessageW, PostQuitMessage, RegisterClassExW, TranslateMessage, WINDOW_EX_STYLE,
        WINDOW_STYLE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CLOSE, WM_DESTROY,
        WM_WTSSESSION_CHANGE, WNDCLASSEXW, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
    };
    use windows::core::{PCWSTR, PWSTR};

    use super::{
        ActivityEvent, ActivityStats, ProcessDiagnostics, UiaProbeRequest, compute_idle_seconds,
        current_unix_ms, extract_process_basename,
    };
    use crate::Win32Error;

    struct HookContext {
        tx: mpsc::Sender<ActivityEvent>,
        stats: Arc<ActivityStats>,
    }

    thread_local! {
        static HOOK_CTX: RefCell<Option<HookContext>> = const { RefCell::new(None) };
    }

    pub(super) fn post_close(hwnd_raw: isize) {
        let hwnd = HWND(hwnd_raw as *mut c_void);
        // SAFETY: Posting `WM_CLOSE` to our owned `HWND_MESSAGE` window to break `GetMessageW`.
        let _ = unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }

    pub(super) fn current_idle_seconds() -> i64 {
        let mut lii = LASTINPUTINFO {
            cbSize: u32::try_from(std::mem::size_of::<LASTINPUTINFO>()).unwrap_or(8),
            dwTime: 0,
        };
        // SAFETY: `lii.cbSize` is initialized and `GetLastInputInfo` fills `dwTime`.
        let ok = unsafe { GetLastInputInfo(&mut lii) }.as_bool();
        if !ok {
            return 0;
        }
        // SAFETY: `GetTickCount` takes no pointers and returns monotonic ms mod 2^32.
        let now_tick = unsafe { GetTickCount() };
        compute_idle_seconds(now_tick, lii.dwTime)
    }

    pub(super) fn process_diagnostics() -> ProcessDiagnostics {
        // SAFETY: `GetCurrentProcess` returns a pseudo-handle that requires no closing.
        let proc_handle = unsafe { GetCurrentProcess() };
        let mut handle_count: u32 = 0;
        // SAFETY: `proc_handle` is valid and `handle_count` is a valid mutable `u32`.
        let _ = unsafe { GetProcessHandleCount(proc_handle, &mut handle_count) };

        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(72),
            ..Default::default()
        };
        // SAFETY: `counters.cb` is initialized and receives memory usage counters.
        let _ = unsafe { GetProcessMemoryInfo(proc_handle, &mut counters, counters.cb) };

        ProcessDiagnostics {
            handle_count,
            working_set_bytes: counters.WorkingSetSize,
        }
    }

    pub(super) fn current_foreground_process() -> Option<(isize, u32, String)> {
        // SAFETY: `GetForegroundWindow` returns the current foreground HWND or null.
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid: u32 = 0;
        // SAFETY: `hwnd` is non-null and `pid` receives the owning process ID.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == 0 {
            return None;
        }
        let name = query_process_basename(pid).unwrap_or_else(|| "unknown".to_string());
        Some((hwnd.0 as isize, pid, name))
    }

    fn query_process_basename(pid: u32) -> Option<String> {
        if pid == 0 {
            return None;
        }
        // SAFETY: Opening process with `PROCESS_QUERY_LIMITED_INFORMATION` only.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buf = [0u16; 512];
        let mut len = u32::try_from(buf.len()).unwrap_or(512);
        // SAFETY: `buf` is a mutable UTF-16 buffer with capacity `len`.
        let res = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR::from_raw(buf.as_mut_ptr()),
                &mut len,
            )
        };
        // SAFETY: Always close the opened process handle.
        let _ = unsafe { CloseHandle(handle) };
        res.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Some(extract_process_basename(&path))
    }

    fn emit_event(event: ActivityEvent) {
        HOOK_CTX.with(|slot| {
            if let Some(ctx) = slot.borrow().as_ref() {
                ctx.stats.events_received.fetch_add(1, Ordering::Relaxed);
                if ctx.tx.try_send(event).is_err() {
                    ctx.stats.events_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
    }

    unsafe extern "system" fn win_event_proc(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        _id_object: i32,
        _id_child: i32,
        _event_thread: u32,
        _event_time: u32,
    ) {
        if event != EVENT_SYSTEM_FOREGROUND || hwnd.is_invalid() {
            return;
        }
        let mut pid: u32 = 0;
        // SAFETY: `hwnd` is the foreground window passed by `SetWinEventHook`.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let process_name = query_process_basename(pid).unwrap_or_else(|| "unknown".to_string());
        emit_event(ActivityEvent::ForegroundChanged {
            hwnd_raw: hwnd.0 as isize,
            pid,
            process_name,
            timestamp_ms: current_unix_ms(),
        });
    }

    unsafe extern "system" fn activity_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_WTSSESSION_CHANGE => {
                let code = u32::try_from(wparam.0).unwrap_or(0);
                if code == WTS_SESSION_LOCK {
                    emit_event(ActivityEvent::SessionLockChanged {
                        locked: true,
                        timestamp_ms: current_unix_ms(),
                    });
                } else if code == WTS_SESSION_UNLOCK {
                    emit_event(ActivityEvent::SessionLockChanged {
                        locked: false,
                        timestamp_ms: current_unix_ms(),
                    });
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                // SAFETY: Closing our owned message-only window triggers `WM_DESTROY`.
                let _ = unsafe { DestroyWindow(hwnd) };
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: Exit the dedicated `GetMessageW` pump loop cleanly.
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: Delegate unhandled messages to `DefWindowProcW`.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    pub(super) fn run_activity_pump(
        event_tx: mpsc::Sender<ActivityEvent>,
        stats: Arc<ActivityStats>,
        ready_tx: std::sync::mpsc::SyncSender<Result<isize, Win32Error>>,
    ) {
        HOOK_CTX.with(|slot| {
            *slot.borrow_mut() = Some(HookContext {
                tx: event_tx,
                stats,
            });
        });

        // SAFETY: Querying current module HINSTANCE for window class registration.
        let hmod = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
            Ok(h) => h,
            Err(err) => {
                let _ = ready_tx.send(Err(Win32Error::api("GetModuleHandleW", err.to_string())));
                return;
            }
        };
        let hinstance = HINSTANCE(hmod.0);

        let class_name: Vec<u16> = "VunActivityMessageWindow\0".encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(80),
            lpfnWndProc: Some(activity_wnd_proc),
            hInstance: hinstance,
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        // SAFETY: `wc` is fully initialized with a valid NUL-terminated class name.
        let _ = unsafe { RegisterClassExW(&wc) };

        // SAFETY: Creating a message-only `HWND_MESSAGE` window on this dedicated thread.
        let hwnd = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR::from_raw(class_name.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                None,
                hinstance,
                None,
            )
        } {
            Ok(h) => h,
            Err(err) => {
                let _ = ready_tx.send(Err(Win32Error::api("CreateWindowExW", err.to_string())));
                return;
            }
        };

        // SAFETY: Registering for lock/unlock notifications on this thread's `HWND_MESSAGE`.
        let wts_ok =
            unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }.is_ok();
        if !wts_ok {
            tracing::warn!(
                "WTSRegisterSessionNotification failed; lock/unlock suppression degraded"
            );
        }

        // SAFETY: Installing out-of-context foreground hook on this dedicated Win32 pump thread.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if hook.is_invalid() {
            tracing::warn!("SetWinEventHook(EVENT_SYSTEM_FOREGROUND) returned invalid hook");
        }

        let _ = ready_tx.send(Ok(hwnd.0 as isize));

        let mut msg = MSG::default();
        loop {
            // SAFETY: Blocking `GetMessageW` on the dedicated `ef-win32-activity` thread.
            let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if ret.0 <= 0 {
                break;
            }
            // SAFETY: Standard message dispatch for `HWND_MESSAGE`.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if !hook.is_invalid() {
            // SAFETY: Unhook `SetWinEventHook` on the same thread that installed it.
            let _ = unsafe { UnhookWinEvent(hook) };
        }
        if wts_ok {
            // SAFETY: Unregister session notifications before thread exit.
            let _ = unsafe { WTSUnRegisterSessionNotification(hwnd) };
        }
        HOOK_CTX.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }

    pub(super) fn run_uia_mta_loop(rx: std::sync::mpsc::Receiver<UiaProbeRequest>) {
        // SAFETY: Initializing a dedicated MTA COM apartment for `IUIAutomation` (`CONTEXT.md` §7).
        let init_hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let com_ok = init_hr.is_ok();
        let automation: Option<IUIAutomation> = if com_ok {
            // SAFETY: Instantiating `CUIAutomation` inside the MTA apartment.
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL) }.ok()
        } else {
            None
        };

        while let Ok(req) = rx.recv() {
            let resolved = automation.as_ref().and_then(|uia| {
                let hwnd = HWND(req.hwnd_raw as *mut c_void);
                // SAFETY: Querying foreground element and its owning process ID on MTA thread.
                let elem = unsafe { uia.ElementFromHandle(hwnd) }.ok()?;
                // SAFETY: Reading `CurrentProcessId` from valid `IUIAutomationElement`.
                let pid_i32 = unsafe { elem.CurrentProcessId() }.ok()?;
                let pid = u32::try_from(pid_i32).ok()?;
                query_process_basename(pid)
            });
            let _ = req.reply.send(resolved);
        }

        drop(automation);
        if com_ok {
            // SAFETY: Balanced `CoUninitialize` for `CoInitializeEx` on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_idle_seconds_is_never_negative_across_wrap_or_lock_unlock() {
        assert_eq!(compute_idle_seconds(10_500, 5_000), 5);
        // 32-bit GetTickCount wraparound (~49.7 days)
        assert_eq!(compute_idle_seconds(2_000, u32::MAX - 2_999), 5);
        // Non-monotonic jump where last_input > now (e.g. lock/unlock clock quirk) -> clamped to 0
        assert_eq!(compute_idle_seconds(5_000, 10_000), 0);
        assert_eq!(compute_idle_seconds(0, 1), 0);
    }

    #[test]
    fn process_basename_strips_directories_and_lowercases() {
        assert_eq!(
            extract_process_basename(r"C:\Program Files\Microsoft VS Code\Code.exe"),
            "code.exe"
        );
        assert_eq!(extract_process_basename("/usr/bin/Firefox"), "firefox");
        assert_eq!(extract_process_basename(""), "unknown");
        assert_eq!(extract_process_basename("   "), "unknown");
    }

    #[test]
    fn sampling_policy_suppresses_when_paused_or_locked() {
        assert!(
            SamplingPolicy {
                paused: false,
                session_locked: false
            }
            .should_record()
        );
        assert!(
            !SamplingPolicy {
                paused: true,
                session_locked: false
            }
            .should_record()
        );
        assert!(
            !SamplingPolicy {
                paused: false,
                session_locked: true
            }
            .should_record()
        );
    }

    #[tokio::test]
    async fn activity_monitor_starts_resolves_fallback_and_shuts_down() {
        let (tx, _rx) = mpsc::channel(16);
        let monitor = ActivityMonitor::start(tx).expect("start activity monitor");
        let resolved = monitor
            .resolve_process_with_uia_fallback(0, "notepad.exe".to_string())
            .await;
        assert_eq!(resolved, "notepad.exe");
        assert!(current_idle_seconds() >= 0);
        monitor.shutdown();
    }
}
