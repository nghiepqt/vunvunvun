use std::thread::JoinHandle;

use tokio::sync::mpsc;

use crate::Win32Error;
use crate::hotkey::{HotkeyBinding, HotkeyKind, HotkeyRegistrationResult};
use crate::palette::ThemeId;
use crate::tray::ShellCopy;

pub const SHELL_EVENT_CHANNEL_CAPACITY: usize = 128;

/// Commands sent from the `ef-app` Tokio coordinator to the dedicated `ef-win32-shell` thread.
#[derive(Debug, Clone)]
pub enum ShellCommand {
    ShowCapture,
    ShowNudge {
        message: Option<String>,
    },
    UnlockNudgeFocus,
    SetActivityPaused(bool),
    SetLaunchAtLogin(bool),
    SetTheme(ThemeId),
    UpdateCopy(Box<ShellCopy>),
    RebindHotkey {
        kind: HotkeyKind,
        binding: HotkeyBinding,
    },
    Shutdown,
}

/// Events emitted from `ef-win32-shell` (tray, hotkeys, native overlays, wakeup signal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    WakeupFromSecondInstance,
    TrayOpenWorkspace,
    TrayOpenSettings,
    TrayQuickCapture,
    TrayToggleActivityMonitoring,
    TrayToggleLaunchAtLogin,
    TrayExit,
    HotkeyPressed(HotkeyKind),
    HotkeyRegistration {
        kind: HotkeyKind,
        result: HotkeyRegistrationResult,
    },
    CaptureShown {
        wake_latency_ms: u64,
    },
    CaptureSubmitted {
        text: String,
    },
    CaptureDismissed,
    NudgeShown,
    NudgeFocusUnlocked,
    NudgeDismissed {
        action: String,
    },
}

/// Configuration for starting the resident Win32 shell thread.
#[derive(Debug, Clone)]
pub struct NativeShellConfig {
    pub theme: ThemeId,
    pub copy: ShellCopy,
    pub capture_hotkey: HotkeyBinding,
    pub nudge_hotkey: HotkeyBinding,
    pub activity_paused: bool,
    pub launch_at_login: bool,
    pub wakeup_window_class: String,
    pub wakeup_message_name: String,
}

/// Handle to the resident Win32 shell thread (`tray` + `hotkeys` + pre-created F1/F7 overlays).
pub struct NativeShell {
    #[cfg(windows)]
    cmd_tx: std::sync::mpsc::Sender<ShellCommand>,
    #[cfg(windows)]
    hwnd_raw: isize,
    #[cfg(not(windows))]
    stub_tx: mpsc::Sender<ShellEvent>,
    thread: Option<JoinHandle<()>>,
}

impl NativeShell {
    pub fn start(
        config: NativeShellConfig,
        event_tx: mpsc::Sender<ShellEvent>,
    ) -> Result<Self, Win32Error> {
        #[cfg(windows)]
        {
            windows_impl::start(config, event_tx)
        }
        #[cfg(not(windows))]
        {
            let _ = config;
            Ok(Self {
                stub_tx: event_tx,
                thread: None,
            })
        }
    }

    pub fn send(&self, cmd: ShellCommand) -> Result<(), Win32Error> {
        #[cfg(windows)]
        {
            self.cmd_tx
                .send(cmd)
                .map_err(|_| Win32Error::ChannelClosed)?;
            windows_impl::wake_pump(self.hwnd_raw);
            Ok(())
        }
        #[cfg(not(windows))]
        {
            match cmd {
                ShellCommand::ShowCapture => {
                    let _ = self
                        .stub_tx
                        .try_send(ShellEvent::CaptureShown { wake_latency_ms: 0 });
                }
                ShellCommand::ShowNudge { .. } => {
                    let _ = self.stub_tx.try_send(ShellEvent::NudgeShown);
                }
                ShellCommand::UnlockNudgeFocus => {
                    let _ = self.stub_tx.try_send(ShellEvent::NudgeFocusUnlocked);
                }
                ShellCommand::Shutdown => {
                    let _ = self.stub_tx.try_send(ShellEvent::TrayExit);
                }
                _ => {}
            }
            Ok(())
        }
    }

    pub fn shutdown(mut self) {
        let _ = self.send(ShellCommand::Shutdown);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for NativeShell {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.send(ShellCommand::Shutdown);
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::cell::RefCell;
    use std::ffi::c_void;

    use tokio::sync::mpsc;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::NIN_SELECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
        PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, TranslateMessage,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_HOTKEY,
        WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSEXW,
    };
    use windows::core::PCWSTR;

    use super::{NativeShell, NativeShellConfig, ShellCommand, ShellEvent};
    use crate::Win32Error;
    use crate::hotkey::{HotkeyBinding, HotkeyKind, HotkeyRegistrationResult, windows_hotkey};
    use crate::overlays::windows_overlays::OverlayHost;
    use crate::tray::windows_tray::{TrayIconHandle, WM_TRAY_CALLBACK};
    use crate::tray::{ShellCopy, menu_ids};

    const WM_SHELL_WAKE: u32 = WM_APP + 20;
    const NIN_KEYSELECT: u32 = 0x0401;

    struct ShellThreadState {
        hwnd: HWND,
        cmd_rx: std::sync::mpsc::Receiver<ShellCommand>,
        event_tx: mpsc::Sender<ShellEvent>,
        copy: ShellCopy,
        activity_paused: bool,
        launch_at_login: bool,
        capture_hotkey: HotkeyBinding,
        nudge_hotkey: HotkeyBinding,
        wakeup_msg: u32,
        taskbar_created_msg: u32,
        tray: Option<TrayIconHandle>,
        overlays: OverlayHost,
    }

    thread_local! {
        static SHELL_STATE: RefCell<Option<ShellThreadState>> = const { RefCell::new(None) };
    }

    pub(super) fn wake_pump(hwnd_raw: isize) {
        if hwnd_raw != 0 {
            let hwnd = HWND(hwnd_raw as *mut c_void);
            // SAFETY: Waking the `GetMessageW` loop on `VunShellMessageWindow` to drain `cmd_rx`.
            let _ = unsafe { PostMessageW(hwnd, WM_SHELL_WAKE, WPARAM(0), LPARAM(0)) };
        }
    }

    pub(super) fn start(
        config: NativeShellConfig,
        event_tx: mpsc::Sender<ShellEvent>,
    ) -> Result<NativeShell, Win32Error> {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<ShellCommand>();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<isize, Win32Error>>(1);

        let thread = std::thread::Builder::new()
            .name("ef-win32-shell".to_string())
            .spawn(move || {
                run_shell_thread(config, cmd_rx, event_tx, ready_tx);
            })
            .map_err(Win32Error::Io)?;

        let hwnd_raw = ready_rx.recv().map_err(|_| Win32Error::ChannelClosed)??;
        Ok(NativeShell {
            cmd_tx,
            hwnd_raw,
            thread: Some(thread),
        })
    }

    fn drain_commands() {
        let mut should_close = false;
        let mut target_hwnd = HWND::default();

        SHELL_STATE.with(|slot| {
            let mut borrow = slot.borrow_mut();
            let Some(state) = borrow.as_mut() else {
                return;
            };
            target_hwnd = state.hwnd;
            while let Ok(cmd) = state.cmd_rx.try_recv() {
                match cmd {
                    ShellCommand::ShowCapture => state.overlays.show_capture(),
                    ShellCommand::ShowNudge { message } => {
                        state.overlays.show_nudge(message.as_deref());
                    }
                    ShellCommand::UnlockNudgeFocus => state.overlays.unlock_nudge_focus(),
                    ShellCommand::SetActivityPaused(paused) => {
                        state.activity_paused = paused;
                    }
                    ShellCommand::SetLaunchAtLogin(enabled) => {
                        state.launch_at_login = enabled;
                    }
                    ShellCommand::SetTheme(theme) => {
                        state.overlays.apply_theme(theme);
                    }
                    ShellCommand::UpdateCopy(copy) => {
                        state.copy = *copy;
                        state.overlays.update_copy(&state.copy);
                        if let Some(tray) = &state.tray {
                            tray.update_tooltip(&state.copy);
                        }
                    }
                    ShellCommand::RebindHotkey { kind, binding } => {
                        let prev = match kind {
                            HotkeyKind::Capture => state.capture_hotkey.clone(),
                            HotkeyKind::FocusNudge => state.nudge_hotkey.clone(),
                        };
                        windows_hotkey::unregister(state.hwnd, kind);
                        let res = windows_hotkey::register(state.hwnd, kind, &binding);
                        match &res {
                            HotkeyRegistrationResult::Registered(accepted) => match kind {
                                HotkeyKind::Capture => state.capture_hotkey = accepted.clone(),
                                HotkeyKind::FocusNudge => state.nudge_hotkey = accepted.clone(),
                            },
                            _ => {
                                // Restore previous binding if rebind failed or conflicted.
                                let _ = windows_hotkey::register(state.hwnd, kind, &prev);
                            }
                        }
                        let _ = state
                            .event_tx
                            .try_send(ShellEvent::HotkeyRegistration { kind, result: res });
                    }
                    ShellCommand::Shutdown => {
                        should_close = true;
                    }
                }
            }
        });

        if should_close && !target_hwnd.is_invalid() {
            // SAFETY: Trigger clean shutdown of `VunShellMessageWindow` and overlays.
            let _ = unsafe { PostMessageW(target_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
    }

    unsafe extern "system" fn shell_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_SHELL_WAKE {
            drain_commands();
            return LRESULT(0);
        }

        if msg == WM_HOTKEY {
            let id = i32::try_from(wparam.0).unwrap_or(0);
            if let Some(kind) = HotkeyKind::from_win32_id(id) {
                SHELL_STATE.with(|slot| {
                    if let Some(state) = slot.borrow().as_ref() {
                        match kind {
                            HotkeyKind::Capture => state.overlays.show_capture(),
                            HotkeyKind::FocusNudge => state.overlays.unlock_nudge_focus(),
                        }
                        let _ = state.event_tx.try_send(ShellEvent::HotkeyPressed(kind));
                    }
                });
            }
            return LRESULT(0);
        }

        if msg == WM_TRAY_CALLBACK {
            let event_code = u32::try_from(lparam.0 & 0xFFFF).unwrap_or(0);
            match event_code {
                c if c == NIN_SELECT || c == NIN_KEYSELECT || c == WM_LBUTTONUP => {
                    SHELL_STATE.with(|slot| {
                        if let Some(state) = slot.borrow().as_ref() {
                            let _ = state.event_tx.try_send(ShellEvent::TrayOpenWorkspace);
                        }
                    });
                }
                c if c == WM_CONTEXTMENU || c == WM_RBUTTONUP => {
                    let chosen = SHELL_STATE.with(|slot| {
                        let borrow = slot.borrow();
                        let state = borrow.as_ref()?;
                        let tray = state.tray.as_ref()?;
                        let id = tray.show_context_menu(
                            &state.copy,
                            state.activity_paused,
                            state.launch_at_login,
                            &state.capture_hotkey,
                        )?;
                        Some((id, state.event_tx.clone()))
                    });
                    if let Some((cmd_id, tx)) = chosen {
                        match cmd_id {
                            menu_ids::OPEN_WORKSPACE => {
                                let _ = tx.try_send(ShellEvent::TrayOpenWorkspace);
                            }
                            menu_ids::QUICK_CAPTURE => {
                                SHELL_STATE.with(|slot| {
                                    if let Some(state) = slot.borrow().as_ref() {
                                        state.overlays.show_capture();
                                    }
                                });
                                let _ = tx.try_send(ShellEvent::TrayQuickCapture);
                            }
                            menu_ids::TOGGLE_ACTIVITY => {
                                let _ = tx.try_send(ShellEvent::TrayToggleActivityMonitoring);
                            }
                            menu_ids::TOGGLE_LAUNCH_AT_LOGIN => {
                                let _ = tx.try_send(ShellEvent::TrayToggleLaunchAtLogin);
                            }
                            menu_ids::OPEN_SETTINGS => {
                                let _ = tx.try_send(ShellEvent::TrayOpenSettings);
                            }
                            menu_ids::EXIT_APP => {
                                let _ = tx.try_send(ShellEvent::TrayExit);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            return LRESULT(0);
        }

        let handled_custom = SHELL_STATE.with(|slot| {
            let mut borrow = slot.borrow_mut();
            let Some(state) = borrow.as_mut() else {
                return false;
            };
            if state.wakeup_msg != 0 && msg == state.wakeup_msg {
                let _ = state
                    .event_tx
                    .try_send(ShellEvent::WakeupFromSecondInstance);
                return true;
            }
            if state.taskbar_created_msg != 0 && msg == state.taskbar_created_msg {
                if let Some(tray) = &state.tray {
                    tray.add_or_reinstall(&state.copy);
                }
                return true;
            }
            false
        });
        if handled_custom {
            return LRESULT(0);
        }

        match msg {
            WM_CLOSE => {
                SHELL_STATE.with(|slot| {
                    if let Some(mut state) = slot.borrow_mut().take() {
                        windows_hotkey::unregister(state.hwnd, HotkeyKind::Capture);
                        windows_hotkey::unregister(state.hwnd, HotkeyKind::FocusNudge);
                        state.tray.take();
                        state.overlays.destroy();
                    }
                });
                // SAFETY: Destroy `VunShellMessageWindow`.
                let _ = unsafe { DestroyWindow(hwnd) };
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: Terminate `ef-win32-shell` pump loop.
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: Default message processing.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    fn run_shell_thread(
        config: NativeShellConfig,
        cmd_rx: std::sync::mpsc::Receiver<ShellCommand>,
        event_tx: mpsc::Sender<ShellEvent>,
        ready_tx: std::sync::mpsc::SyncSender<Result<isize, Win32Error>>,
    ) {
        // SAFETY: STA COM apartment for Shell_NotifyIconW, RichEdit, and IAccPropServices.
        let com_ok = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();

        let hmod = match unsafe { GetModuleHandleW(PCWSTR::null()) } {
            Ok(h) => h,
            Err(err) => {
                let _ = ready_tx.send(Err(Win32Error::api("GetModuleHandleW", err.to_string())));
                return;
            }
        };
        let hinstance = HINSTANCE(hmod.0);

        let class_w: Vec<u16> = config
            .wakeup_window_class
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let wc = WNDCLASSEXW {
            cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(80),
            lpfnWndProc: Some(shell_wnd_proc),
            hInstance: hinstance,
            lpszClassName: PCWSTR::from_raw(class_w.as_ptr()),
            ..Default::default()
        };
        // SAFETY: Register top-level hidden shell window class (must be findable via `FindWindowW`).
        let _ = unsafe { RegisterClassExW(&wc) };

        let hwnd = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR::from_raw(class_w.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                hinstance,
                None,
            )
        } {
            Ok(h) => h,
            Err(err) => {
                let _ = ready_tx.send(Err(Win32Error::api(
                    "CreateWindowExW(Shell)",
                    err.to_string(),
                )));
                if com_ok {
                    unsafe { CoUninitialize() };
                }
                return;
            }
        };

        let overlays = match OverlayHost::create(config.theme, &config.copy, event_tx.clone()) {
            Ok(o) => o,
            Err(err) => {
                let _ = unsafe { DestroyWindow(hwnd) };
                let _ = ready_tx.send(Err(err));
                if com_ok {
                    unsafe { CoUninitialize() };
                }
                return;
            }
        };

        let tray = TrayIconHandle::install(hwnd, &config.copy);

        let cap_reg = windows_hotkey::register(hwnd, HotkeyKind::Capture, &config.capture_hotkey);
        let _ = event_tx.try_send(ShellEvent::HotkeyRegistration {
            kind: HotkeyKind::Capture,
            result: cap_reg,
        });
        let nudge_reg =
            windows_hotkey::register(hwnd, HotkeyKind::FocusNudge, &config.nudge_hotkey);
        let _ = event_tx.try_send(ShellEvent::HotkeyRegistration {
            kind: HotkeyKind::FocusNudge,
            result: nudge_reg,
        });

        let wakeup_w: Vec<u16> = config
            .wakeup_message_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let taskbar_w: Vec<u16> = "TaskbarCreated\0".encode_utf16().collect();
        // SAFETY: Register global custom window messages.
        let wakeup_msg = unsafe { RegisterWindowMessageW(PCWSTR::from_raw(wakeup_w.as_ptr())) };
        let taskbar_created_msg =
            unsafe { RegisterWindowMessageW(PCWSTR::from_raw(taskbar_w.as_ptr())) };

        SHELL_STATE.with(|slot| {
            *slot.borrow_mut() = Some(ShellThreadState {
                hwnd,
                cmd_rx,
                event_tx,
                copy: config.copy,
                activity_paused: config.activity_paused,
                launch_at_login: config.launch_at_login,
                capture_hotkey: config.capture_hotkey,
                nudge_hotkey: config.nudge_hotkey,
                wakeup_msg,
                taskbar_created_msg,
                tray: Some(tray),
                overlays,
            });
        });

        let _ = ready_tx.send(Ok(hwnd.0 as isize));

        let mut msg = MSG::default();
        loop {
            // SAFETY: Synchronous Win32 `GetMessageW` pump for tray, hotkeys, F1 & F7 overlays.
            let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
            if ret.0 <= 0 {
                break;
            }
            // SAFETY: Standard Win32 message translation and dispatch.
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        SHELL_STATE.with(|slot| {
            *slot.borrow_mut() = None;
        });
        if com_ok {
            // SAFETY: Balanced `CoUninitialize`.
            unsafe { CoUninitialize() };
        }
    }
}
