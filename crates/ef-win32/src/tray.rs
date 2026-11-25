/// Localized strings supplied from `i18n/*.json` so `ef-win32` never embeds hardcoded user copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCopy {
    pub app_name: String,
    pub tray_tooltip: String,
    pub tray_open_workspace: String,
    pub tray_capture: String,
    pub tray_pause_activity: String,
    pub tray_resume_activity: String,
    pub tray_launch_at_login: String,
    pub tray_settings: String,
    pub tray_exit: String,
    pub capture_input_label: String,
    pub capture_hint: String,
    pub nudge_title: String,
    pub nudge_short_break: String,
    pub nudge_shrink_step: String,
    pub nudge_not_now: String,
    pub nudge_focus_hint: String,
}

impl Default for ShellCopy {
    fn default() -> Self {
        Self {
            app_name: "Vun".to_string(),
            tray_tooltip: "Vun".to_string(),
            tray_open_workspace: "Open workspace".to_string(),
            tray_capture: "Quick capture".to_string(),
            tray_pause_activity: "Pause activity sensing".to_string(),
            tray_resume_activity: "Resume activity sensing".to_string(),
            tray_launch_at_login: "Launch at login".to_string(),
            tray_settings: "Settings".to_string(),
            tray_exit: "Exit".to_string(),
            capture_input_label: "Capture thought".to_string(),
            capture_hint: "Enter saves. Shift+Enter new line. Escape closes.".to_string(),
            nudge_title: "A gentle check-in from Vun".to_string(),
            nudge_short_break: "A short break might help.".to_string(),
            nudge_shrink_step: "Break this step down further".to_string(),
            nudge_not_now: "Not now".to_string(),
            nudge_focus_hint: "Tab moves between actions, Escape closes.".to_string(),
        }
    }
}

/// Tray menu item identifiers.
#[cfg(windows)]
pub(crate) mod menu_ids {
    pub const OPEN_WORKSPACE: u32 = 1001;
    pub const QUICK_CAPTURE: u32 = 1002;
    pub const TOGGLE_ACTIVITY: u32 = 1003;
    pub const TOGGLE_LAUNCH_AT_LOGIN: u32 = 1004;
    pub const OPEN_SETTINGS: u32 = 1005;
    pub const EXIT_APP: u32 = 1006;
}

#[cfg(windows)]
pub(crate) mod windows_tray {
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, WPARAM};
    use windows::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, IDI_APPLICATION, LoadIconW,
        MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, PostMessageW, SetForegroundWindow,
        TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_NULL,
    };
    use windows::core::PCWSTR;

    use super::{ShellCopy, menu_ids};
    use crate::hotkey::HotkeyBinding;

    pub(crate) const WM_TRAY_CALLBACK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 10;
    const TRAY_ICON_ID: u32 = 1;

    fn copy_wide_trunc(dst: &mut [u16], text: &str) {
        dst.fill(0);
        let max_chars = dst.len().saturating_sub(1);
        for (slot, unit) in dst.iter_mut().take(max_chars).zip(text.encode_utf16()) {
            *slot = unit;
        }
    }

    fn load_app_icon() -> HICON {
        // SAFETY: `LoadIconW(None, IDI_APPLICATION)` loads the default system icon fallback;
        // resource ID 1 is embedded by `winresource` when building `vun.exe`.
        let hmodule = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }.ok();
        if let Some(hmod) = hmodule {
            // Resource icon ID 1 (`MAKEINTRESOURCEW(1)`).
            let res_icon = unsafe {
                LoadIconW(
                    HINSTANCE(hmod.0),
                    PCWSTR(std::ptr::without_provenance::<u16>(1)),
                )
            };
            if let Ok(icon) = res_icon
                && !icon.is_invalid()
            {
                return icon;
            }
        }
        unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default()
    }

    pub(crate) struct TrayIconHandle {
        hwnd: HWND,
        icon: HICON,
    }

    impl TrayIconHandle {
        pub(crate) fn install(hwnd: HWND, copy: &ShellCopy) -> Self {
            let icon = load_app_icon();
            let this = Self { hwnd, icon };
            this.add_or_reinstall(copy);
            this
        }

        pub(crate) fn add_or_reinstall(&self, copy: &ShellCopy) {
            let mut nid = self.base_nid(copy);
            // SAFETY: `nid.cbSize` is initialized and `self.hwnd` is valid.
            let _ = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            // SAFETY: Set `NOTIFYICON_VERSION_4` so `NIN_SELECT`/`NIN_KEYSELECT`/`WM_CONTEXTMENU` fire.
            let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &nid) };
        }

        pub(crate) fn update_tooltip(&self, copy: &ShellCopy) {
            let nid = self.base_nid(copy);
            // SAFETY: `nid` holds updated `szTip`.
            let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
        }

        fn base_nid(&self, copy: &ShellCopy) -> NOTIFYICONDATAW {
            let mut nid = NOTIFYICONDATAW {
                cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(976),
                hWnd: self.hwnd,
                uID: TRAY_ICON_ID,
                uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
                uCallbackMessage: WM_TRAY_CALLBACK,
                hIcon: self.icon,
                ..Default::default()
            };
            copy_wide_trunc(&mut nid.szTip, &copy.tray_tooltip);
            nid
        }

        pub(crate) fn show_context_menu(
            &self,
            copy: &ShellCopy,
            activity_paused: bool,
            launch_at_login: bool,
            capture_hotkey: &HotkeyBinding,
        ) -> Option<u32> {
            // SAFETY: Creating a transient Win32 popup menu.
            let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
                return None;
            };

            let append_str = |id: u32, label: &str, checked: bool| {
                let mut flags = MF_STRING;
                if checked {
                    flags |= MF_CHECKED;
                } else {
                    flags |= MF_UNCHECKED;
                }
                let wide: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                // SAFETY: `menu` is valid and `wide` is NUL-terminated UTF-16.
                let _ = unsafe {
                    AppendMenuW(menu, flags, id as usize, PCWSTR::from_raw(wide.as_ptr()))
                };
            };

            append_str(menu_ids::OPEN_WORKSPACE, &copy.tray_open_workspace, false);
            let capture_label = format!("{} ({capture_hotkey})", copy.tray_capture);
            append_str(menu_ids::QUICK_CAPTURE, &capture_label, false);

            let activity_label = if activity_paused {
                &copy.tray_resume_activity
            } else {
                &copy.tray_pause_activity
            };
            append_str(menu_ids::TOGGLE_ACTIVITY, activity_label, activity_paused);
            append_str(
                menu_ids::TOGGLE_LAUNCH_AT_LOGIN,
                &copy.tray_launch_at_login,
                launch_at_login,
            );
            append_str(menu_ids::OPEN_SETTINGS, &copy.tray_settings, false);

            // SAFETY: Appending separator before Exit.
            let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };
            append_str(menu_ids::EXIT_APP, &copy.tray_exit, false);

            let mut pt = POINT::default();
            // SAFETY: Querying cursor screen position for menu anchor.
            let _ = unsafe { GetCursorPos(&mut pt) };
            // SAFETY: Win32 tray menu requirement so clicking outside dismisses the popup menu.
            let _ = unsafe { SetForegroundWindow(self.hwnd) };
            // SAFETY: Blocking popup menu returning the chosen command ID via `TPM_RETURNCMD`.
            let cmd = unsafe {
                TrackPopupMenuEx(
                    menu,
                    (TPM_RIGHTBUTTON | TPM_RETURNCMD).0,
                    pt.x,
                    pt.y,
                    self.hwnd,
                    None,
                )
            };
            // SAFETY: Required after `TrackPopupMenuEx` per MSDN tray guidelines.
            let _ = unsafe { PostMessageW(self.hwnd, WM_NULL, WPARAM(0), LPARAM(0)) };
            // SAFETY: Always destroy transient `HMENU`.
            let _ = unsafe { DestroyMenu(menu) };

            if cmd.0 > 0 {
                u32::try_from(cmd.0).ok()
            } else {
                None
            }
        }
    }

    impl Drop for TrayIconHandle {
        fn drop(&mut self) {
            let nid = NOTIFYICONDATAW {
                cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(976),
                hWnd: self.hwnd,
                uID: TRAY_ICON_ID,
                ..Default::default()
            };
            // SAFETY: Remove tray icon cleanly on daemon exit.
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
        }
    }
}
