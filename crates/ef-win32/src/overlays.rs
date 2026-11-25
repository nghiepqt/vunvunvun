#[cfg(windows)]
pub(crate) mod windows_overlays {
    use std::ffi::c_void;
    use std::time::Instant;

    use tokio::sync::mpsc;
    use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
        DEFAULT_PITCH, DeleteObject, FF_SWISS, FillRect, HBRUSH, HDC, HFONT, HGDIOBJ,
        InvalidateRect, OUT_DEFAULT_PRECIS, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
    use windows::Win32::UI::Accessibility::{
        CLSID_AccPropServices, IAccPropServices, LiveSetting_Property_GUID, NotifyWinEvent,
    };
    use windows::Win32::UI::Controls::RichEdit::{EM_SETBKGNDCOLOR, ENM_NONE, MSFTEDIT_CLASS};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, SetFocus, VK_ESCAPE, VK_RETURN, VK_SHIFT,
    };
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        BN_CLICKED, BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, ES_AUTOVSCROLL,
        ES_MULTILINE, ES_WANTRETURN, EVENT_OBJECT_LIVEREGIONCHANGED, GWL_EXSTYLE, GWLP_USERDATA,
        GetClientRect, GetForegroundWindow, GetWindowLongPtrW, HWND_TOPMOST, IDC_ARROW, IsWindow,
        LoadCursorW, OBJID_CLIENT, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW, SetForegroundWindow,
        SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
        WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
        WM_GETTEXT, WM_GETTEXTLENGTH, WM_KEYDOWN, WM_SETFONT, WM_SETTEXT, WNDCLASSEXW, WS_CHILD,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_TABSTOP, WS_VISIBLE,
    };
    use windows::core::PCWSTR;

    use crate::Win32Error;
    use crate::dpi::{
        ScreenRect, compute_capture_rect, compute_nudge_rect, monitor_for_cursor, scale_dip,
    };
    use crate::palette::{OverlayColors, ThemeId, is_high_contrast_enabled, load_theme_colors};
    use crate::shell::ShellEvent;
    use crate::tray::ShellCopy;

    const CAPTURE_CLASS: &str = "VunCaptureOverlayWindow\0";
    const NUDGE_CLASS: &str = "VunNudgeOverlayWindow\0";
    const RICHEDIT_SUBCLASS_ID: usize = 1;
    const BTN_SHRINK_ID: usize = 201;
    const BTN_NOT_NOW_ID: usize = 202;

    fn to_wide_null(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn create_ui_font(dpi: u32, height_dip: i32) -> HFONT {
        let px = -scale_dip(height_dip, dpi);
        let face = to_wide_null("Segoe UI");
        // SAFETY: Creating a ClearType Segoe UI font scaled to `dpi`.
        unsafe {
            CreateFontW(
                px,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                u32::from(DEFAULT_CHARSET.0),
                u32::from(OUT_DEFAULT_PRECIS.0),
                u32::from(CLIP_DEFAULT_PRECIS.0),
                u32::from(CLEARTYPE_QUALITY.0),
                u32::from(DEFAULT_PITCH.0 | FF_SWISS.0),
                PCWSTR::from_raw(face.as_ptr()),
            )
        }
    }

    fn apply_font(hwnd: HWND, font: HFONT) {
        if !hwnd.is_invalid() && !font.is_invalid() {
            // SAFETY: Standard `WM_SETFONT` sent to child control.
            unsafe { SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1)) };
        }
    }

    fn set_hwnd_text(hwnd: HWND, text: &str) {
        let wide = to_wide_null(text);
        // SAFETY: `wide` is NUL-terminated UTF-16.
        let _ = unsafe { SetWindowTextW(hwnd, PCWSTR::from_raw(wide.as_ptr())) };
    }

    fn read_hwnd_text(hwnd: HWND) -> String {
        // SAFETY: Querying UTF-16 character length of RichEdit control.
        let len_res = unsafe { SendMessageW(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)) };
        let len = usize::try_from(len_res.0).unwrap_or(0);
        if len == 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len + 1];
        // SAFETY: `buf` has capacity `len + 1` UTF-16 code units.
        let copied = unsafe {
            SendMessageW(
                hwnd,
                WM_GETTEXT,
                WPARAM(buf.len()),
                LPARAM(buf.as_mut_ptr() as isize),
            )
        };
        let actual = usize::try_from(copied.0).unwrap_or(0).min(len);
        String::from_utf16_lossy(&buf[..actual])
    }

    struct CaptureState {
        hwnd: HWND,
        edit_hwnd: HWND,
        hint_hwnd: HWND,
        saved_foreground: Option<HWND>,
        visible: bool,
        dpi: u32,
        colors: OverlayColors,
        bg_brush: HBRUSH,
        surface_brush: HBRUSH,
        font_edit: HFONT,
        font_hint: HFONT,
        event_tx: mpsc::Sender<ShellEvent>,
    }

    impl CaptureState {
        fn apply_theme(&mut self, theme: ThemeId) {
            if let Ok(colors) = load_theme_colors(theme) {
                self.colors = colors;
                // SAFETY: Rebuilding owned GDI brushes cleanly.
                unsafe {
                    if !self.bg_brush.is_invalid() {
                        let _ = DeleteObject(HGDIOBJ(self.bg_brush.0));
                    }
                    if !self.surface_brush.is_invalid() {
                        let _ = DeleteObject(HGDIOBJ(self.surface_brush.0));
                    }
                    self.bg_brush = CreateSolidBrush(COLORREF(colors.bg_canvas.to_colorref()));
                    self.surface_brush =
                        CreateSolidBrush(COLORREF(colors.bg_surface.to_colorref()));
                    if !self.edit_hwnd.is_invalid() {
                        SendMessageW(
                            self.edit_hwnd,
                            EM_SETBKGNDCOLOR,
                            WPARAM(0),
                            LPARAM(colors.bg_surface.to_colorref() as isize),
                        );
                    }
                    crate::palette::windows_impl::apply_dwm_backdrop(
                        self.hwnd,
                        theme.is_dark(),
                        true,
                    );
                    let _ = InvalidateRect(self.hwnd, None, true);
                }
            }
        }

        fn layout(&mut self, rect: ScreenRect, dpi: u32) {
            self.dpi = dpi.max(96);
            // SAFETY: Replacing DPI-scaled GDI fonts and positioning child controls.
            unsafe {
                if !self.font_edit.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.font_edit.0));
                }
                if !self.font_hint.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.font_hint.0));
                }
                self.font_edit = create_ui_font(self.dpi, 16);
                self.font_hint = create_ui_font(self.dpi, 12);
                apply_font(self.edit_hwnd, self.font_edit);
                apply_font(self.hint_hwnd, self.font_hint);

                let pad = scale_dip(14, self.dpi);
                let hint_h = scale_dip(22, self.dpi);
                let w = rect.width().max(200);
                let h = rect.height().max(96);
                let edit_h = (h - pad * 3 - hint_h).max(36);
                let inner_w = (w - pad * 2).max(120);

                let _ = SetWindowPos(
                    self.edit_hwnd,
                    None,
                    pad,
                    pad,
                    inner_w,
                    edit_h,
                    Default::default(),
                );
                let _ = SetWindowPos(
                    self.hint_hwnd,
                    None,
                    pad,
                    pad * 2 + edit_h,
                    inner_w,
                    hint_h,
                    Default::default(),
                );
            }
        }

        fn show(&mut self) {
            let started = Instant::now();
            // SAFETY: Record prior foreground window BEFORE showing F1 (`PHASE-02` §4).
            let prior = unsafe { GetForegroundWindow() };
            if !prior.is_invalid() && prior != self.hwnd && prior != self.edit_hwnd {
                self.saved_foreground = Some(prior);
            }
            let monitor = monitor_for_cursor();
            let rect = compute_capture_rect(monitor);
            self.layout(rect, monitor.dpi);

            // SAFETY: Position, show, and explicitly take keyboard focus (`WS_EX_TOOLWINDOW` only).
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    rect.left,
                    rect.top,
                    rect.width(),
                    rect.height(),
                    SWP_SHOWWINDOW,
                );
                let _ = ShowWindow(self.hwnd, SW_SHOW);
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(self.edit_hwnd);
            }
            self.visible = true;
            let wake_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            tracing::info!(
                wake_latency_ms = wake_ms,
                dpi = monitor.dpi,
                "F1 capture overlay shown"
            );
            let _ = self.event_tx.try_send(ShellEvent::CaptureShown {
                wake_latency_ms: wake_ms,
            });
        }

        fn dismiss(&mut self, submit: bool) {
            if !self.visible {
                return;
            }
            let text = if submit {
                read_hwnd_text(self.edit_hwnd).trim().to_string()
            } else {
                String::new()
            };

            // Clear editor before hiding so next cold wake starts clean.
            let empty = to_wide_null("");
            // SAFETY: Clearing RichEdit text buffer.
            unsafe {
                SendMessageW(
                    self.edit_hwnd,
                    WM_SETTEXT,
                    WPARAM(0),
                    LPARAM(empty.as_ptr() as isize),
                );
            }

            // CRITICAL INVARIANT (`PHASE-02` §4): Call `SetForegroundWindow(saved_hwnd)`
            // *while the overlay is still foreground*, then hide!
            if let Some(saved) = self.saved_foreground.take()
                && !saved.is_invalid()
                && unsafe { IsWindow(saved) }.as_bool()
            {
                // SAFETY: Restoring foreground focus to the user's prior window before hiding.
                let _ = unsafe { SetForegroundWindow(saved) };
            }
            // SAFETY: Hide pre-created F1 overlay back into standby.
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.visible = false;

            if submit && !text.is_empty() {
                let _ = self
                    .event_tx
                    .try_send(ShellEvent::CaptureSubmitted { text });
            } else {
                let _ = self.event_tx.try_send(ShellEvent::CaptureDismissed);
            }
        }
    }

    impl Drop for CaptureState {
        fn drop(&mut self) {
            // SAFETY: Free owned GDI brushes/fonts and remove subclass.
            unsafe {
                if !self.edit_hwnd.is_invalid() {
                    let _ = RemoveWindowSubclass(
                        self.edit_hwnd,
                        Some(richedit_subclass_proc),
                        RICHEDIT_SUBCLASS_ID,
                    );
                }
                if !self.bg_brush.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.bg_brush.0));
                }
                if !self.surface_brush.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.surface_brush.0));
                }
                if !self.font_edit.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.font_edit.0));
                }
                if !self.font_hint.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.font_hint.0));
                }
            }
        }
    }

    unsafe extern "system" fn richedit_subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _uid_subclass: usize,
        dw_ref_data: usize,
    ) -> LRESULT {
        if msg == WM_KEYDOWN {
            let vk = u16::try_from(wparam.0).unwrap_or(0);
            if vk == VK_ESCAPE.0 {
                if dw_ref_data != 0 {
                    // SAFETY: `dw_ref_data` points to live `CaptureState` owned by parent `HWND`.
                    let state = unsafe { &mut *(dw_ref_data as *mut CaptureState) };
                    state.dismiss(false);
                }
                return LRESULT(0);
            }
            if vk == VK_RETURN.0 {
                // SAFETY: Check Shift state without interfering with IME `WM_CHAR` / `WM_IME_*`.
                let shift_down =
                    (unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } as u16 & 0x8000) != 0;
                if !shift_down {
                    if dw_ref_data != 0 {
                        // SAFETY: `dw_ref_data` points to live `CaptureState`.
                        let state = unsafe { &mut *(dw_ref_data as *mut CaptureState) };
                        state.dismiss(true);
                    }
                    return LRESULT(0);
                }
            }
        }
        // SAFETY: Forward all other messages (including IME composition) to RichEdit untouched.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    unsafe extern "system" fn capture_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: Retrieve `CaptureState` pointer stored in `GWLP_USERDATA`.
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut CaptureState;
        match msg {
            WM_ERASEBKGND => {
                if !ptr.is_null() {
                    // SAFETY: `ptr` is valid while `hwnd` lives.
                    let state = unsafe { &*ptr };
                    let hdc = HDC(wparam.0 as *mut c_void);
                    let mut rc = RECT::default();
                    // SAFETY: Paint canvas background using palette token brush.
                    unsafe {
                        let _ = GetClientRect(hwnd, &mut rc);
                        FillRect(hdc, &rc, state.bg_brush);
                    }
                    return LRESULT(1);
                }
            }
            WM_CTLCOLORSTATIC => {
                if !ptr.is_null() && !is_high_contrast_enabled() {
                    // SAFETY: `ptr` is valid; configure hint text color & background.
                    let state = unsafe { &*ptr };
                    let hdc = HDC(wparam.0 as *mut c_void);
                    unsafe {
                        SetTextColor(hdc, COLORREF(state.colors.text_muted.to_colorref()));
                        SetBkColor(hdc, COLORREF(state.colors.bg_canvas.to_colorref()));
                        SetBkMode(hdc, TRANSPARENT);
                    }
                    return LRESULT(state.bg_brush.0 as isize);
                }
            }
            WM_DPICHANGED => {
                if !ptr.is_null() {
                    // SAFETY: `lparam` points to the suggested `RECT` for the new monitor DPI.
                    let suggested = unsafe { &*(lparam.0 as *const RECT) };
                    let new_dpi = u32::try_from((wparam.0 >> 16) & 0xFFFF).unwrap_or(96);
                    let state = unsafe { &mut *ptr };
                    let rect = ScreenRect {
                        left: suggested.left,
                        top: suggested.top,
                        right: suggested.right,
                        bottom: suggested.bottom,
                    };
                    // SAFETY: Resize/reposition window to OS-suggested DPI rect.
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            rect.left,
                            rect.top,
                            rect.width(),
                            rect.height(),
                            SWP_NOACTIVATE,
                        );
                    }
                    state.layout(rect, new_dpi);
                    return LRESULT(0);
                }
            }
            WM_DESTROY => {
                if !ptr.is_null() {
                    // SAFETY: Clear pointer and free heap-allocated `CaptureState`.
                    unsafe {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                        drop(Box::from_raw(ptr));
                    }
                }
                return LRESULT(0);
            }
            _ => {}
        }
        // SAFETY: Forward unhandled messages to default Win32 procedure.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    struct NudgeState {
        hwnd: HWND,
        label_hwnd: HWND,
        shrink_btn_hwnd: HWND,
        dismiss_btn_hwnd: HWND,
        visible: bool,
        focus_unlocked: bool,
        dpi: u32,
        colors: OverlayColors,
        bg_brush: HBRUSH,
        font_body: HFONT,
        font_btn: HFONT,
        event_tx: mpsc::Sender<ShellEvent>,
    }

    impl NudgeState {
        fn apply_theme(&mut self, theme: ThemeId) {
            if let Ok(colors) = load_theme_colors(theme) {
                self.colors = colors;
                // SAFETY: Rebuild surface brush and DWM backdrop.
                unsafe {
                    if !self.bg_brush.is_invalid() {
                        let _ = DeleteObject(HGDIOBJ(self.bg_brush.0));
                    }
                    self.bg_brush = CreateSolidBrush(COLORREF(colors.bg_surface.to_colorref()));
                    crate::palette::windows_impl::apply_dwm_backdrop(
                        self.hwnd,
                        theme.is_dark(),
                        false,
                    );
                    let _ = InvalidateRect(self.hwnd, None, true);
                }
            }
        }

        fn layout(&mut self, rect: ScreenRect, dpi: u32) {
            self.dpi = dpi.max(96);
            // SAFETY: Rebuild DPI-scaled fonts and position nudge text & buttons.
            unsafe {
                if !self.font_body.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.font_body.0));
                }
                if !self.font_btn.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.font_btn.0));
                }
                self.font_body = create_ui_font(self.dpi, 14);
                self.font_btn = create_ui_font(self.dpi, 13);
                apply_font(self.label_hwnd, self.font_body);
                apply_font(self.shrink_btn_hwnd, self.font_btn);
                apply_font(self.dismiss_btn_hwnd, self.font_btn);

                let pad = scale_dip(14, self.dpi);
                let btn_h = scale_dip(32, self.dpi);
                let gap = scale_dip(10, self.dpi);
                let w = rect.width().max(220);
                let h = rect.height().max(96);
                let inner_w = (w - pad * 2).max(160);
                let label_h = (h - pad * 3 - btn_h).max(36);
                let btn_w = ((inner_w - gap) / 2).max(70);
                let btn_y = pad * 2 + label_h;

                let _ = SetWindowPos(
                    self.label_hwnd,
                    None,
                    pad,
                    pad,
                    inner_w,
                    label_h,
                    SWP_NOACTIVATE,
                );
                let _ = SetWindowPos(
                    self.shrink_btn_hwnd,
                    None,
                    pad,
                    btn_y,
                    btn_w,
                    btn_h,
                    SWP_NOACTIVATE,
                );
                let _ = SetWindowPos(
                    self.dismiss_btn_hwnd,
                    None,
                    pad + btn_w + gap,
                    btn_y,
                    btn_w,
                    btn_h,
                    SWP_NOACTIVATE,
                );
            }
        }

        fn show(&mut self, message: Option<&str>) {
            if let Some(msg_text) = message {
                set_hwnd_text(self.label_hwnd, msg_text);
            }
            self.ensure_noactivate_flag(true);
            let monitor = monitor_for_cursor();
            let rect = compute_nudge_rect(monitor);
            self.layout(rect, monitor.dpi);

            // SAFETY: Show with `SWP_NOACTIVATE` so focus NEVER moves from user's active window.
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    rect.left,
                    rect.top,
                    rect.width(),
                    rect.height(),
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
                NotifyWinEvent(
                    EVENT_OBJECT_LIVEREGIONCHANGED,
                    self.label_hwnd,
                    OBJID_CLIENT.0,
                    0,
                );
            }
            self.visible = true;
            let _ = self.event_tx.try_send(ShellEvent::NudgeShown);
        }

        fn unlock_focus(&mut self) {
            if !self.visible {
                return;
            }
            self.ensure_noactivate_flag(false);
            // SAFETY: Explicitly focus the first action button via the accessibility shortcut (`PHASE-02` §5).
            unsafe {
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetFocus(self.shrink_btn_hwnd);
            }
            let _ = self.event_tx.try_send(ShellEvent::NudgeFocusUnlocked);
        }

        fn dismiss(&mut self, action: &str) {
            if !self.visible {
                return;
            }
            self.ensure_noactivate_flag(true);
            // SAFETY: Hide F7 back into standby.
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.visible = false;
            let _ = self.event_tx.try_send(ShellEvent::NudgeDismissed {
                action: action.to_string(),
            });
        }

        fn ensure_noactivate_flag(&mut self, noactivate: bool) {
            // SAFETY: Updating `GWL_EXSTYLE` on owned F7 window.
            unsafe {
                let current = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as usize;
                let updated = if noactivate {
                    current | (WS_EX_NOACTIVATE.0 as usize)
                } else {
                    current & !(WS_EX_NOACTIVATE.0 as usize)
                };
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, updated as isize);
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            self.focus_unlocked = !noactivate;
        }
    }

    impl Drop for NudgeState {
        fn drop(&mut self) {
            // SAFETY: Free owned GDI resources.
            unsafe {
                if !self.bg_brush.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.bg_brush.0));
                }
                if !self.font_body.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.font_body.0));
                }
                if !self.font_btn.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.font_btn.0));
                }
            }
        }
    }

    unsafe extern "system" fn nudge_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: Retrieve `NudgeState` pointer stored in `GWLP_USERDATA`.
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut NudgeState;
        match msg {
            WM_ERASEBKGND => {
                if !ptr.is_null() {
                    // SAFETY: `ptr` is valid while `hwnd` lives.
                    let state = unsafe { &*ptr };
                    let hdc = HDC(wparam.0 as *mut c_void);
                    let mut rc = RECT::default();
                    unsafe {
                        let _ = GetClientRect(hwnd, &mut rc);
                        FillRect(hdc, &rc, state.bg_brush);
                    }
                    return LRESULT(1);
                }
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                if !ptr.is_null() && !is_high_contrast_enabled() {
                    // SAFETY: `ptr` is valid; style nudge label calmly using palette tokens.
                    let state = unsafe { &*ptr };
                    let hdc = HDC(wparam.0 as *mut c_void);
                    unsafe {
                        SetTextColor(hdc, COLORREF(state.colors.text_primary.to_colorref()));
                        SetBkColor(hdc, COLORREF(state.colors.bg_surface.to_colorref()));
                        SetBkMode(hdc, TRANSPARENT);
                    }
                    return LRESULT(state.bg_brush.0 as isize);
                }
            }
            WM_COMMAND => {
                if !ptr.is_null() {
                    let id = wparam.0 & 0xFFFF;
                    let code = u32::try_from((wparam.0 >> 16) & 0xFFFF).unwrap_or(0);
                    if code == BN_CLICKED {
                        // SAFETY: `ptr` is valid while `hwnd` lives.
                        let state = unsafe { &mut *ptr };
                        if id == BTN_SHRINK_ID {
                            state.dismiss("shrink_step");
                            return LRESULT(0);
                        }
                        if id == BTN_NOT_NOW_ID {
                            state.dismiss("not_now");
                            return LRESULT(0);
                        }
                    }
                }
            }
            WM_KEYDOWN => {
                if !ptr.is_null() && u16::try_from(wparam.0).unwrap_or(0) == VK_ESCAPE.0 {
                    // SAFETY: `ptr` is valid while `hwnd` lives.
                    let state = unsafe { &mut *ptr };
                    state.dismiss("escape");
                    return LRESULT(0);
                }
            }
            WM_DPICHANGED => {
                if !ptr.is_null() {
                    // SAFETY: `lparam` points to suggested `RECT`.
                    let suggested = unsafe { &*(lparam.0 as *const RECT) };
                    let new_dpi = u32::try_from((wparam.0 >> 16) & 0xFFFF).unwrap_or(96);
                    let state = unsafe { &mut *ptr };
                    let rect = ScreenRect {
                        left: suggested.left,
                        top: suggested.top,
                        right: suggested.right,
                        bottom: suggested.bottom,
                    };
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            rect.left,
                            rect.top,
                            rect.width(),
                            rect.height(),
                            SWP_NOACTIVATE,
                        );
                    }
                    state.layout(rect, new_dpi);
                    return LRESULT(0);
                }
            }
            WM_DESTROY => {
                if !ptr.is_null() {
                    // SAFETY: Free heap-allocated `NudgeState`.
                    unsafe {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                        drop(Box::from_raw(ptr));
                    }
                }
                return LRESULT(0);
            }
            _ => {}
        }
        // SAFETY: Forward unhandled messages to `DefWindowProcW`.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub(crate) struct OverlayHost {
        capture_hwnd: HWND,
        nudge_hwnd: HWND,
        nudge_label_hwnd: HWND,
        nudge_shrink_hwnd: HWND,
        nudge_not_now_hwnd: HWND,
        capture_hint_hwnd: HWND,
        capture_edit_hwnd: HWND,
    }

    impl OverlayHost {
        pub(crate) fn create(
            theme: ThemeId,
            copy: &ShellCopy,
            event_tx: mpsc::Sender<ShellEvent>,
        ) -> Result<Self, Win32Error> {
            // SAFETY: Query module instance and load `msftedit.dll` for `MSFTEDIT_CLASS`.
            let hmod = unsafe { GetModuleHandleW(PCWSTR::null()) }
                .map_err(|e| Win32Error::api("GetModuleHandleW", e.to_string()))?;
            let hinstance = HINSTANCE(hmod.0);
            let msftedit = to_wide_null("msftedit.dll");
            let _ = unsafe { LoadLibraryW(PCWSTR::from_raw(msftedit.as_ptr())) }
                .map_err(|e| Win32Error::api("LoadLibraryW(msftedit.dll)", e.to_string()))?;
            let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();

            let cap_class = to_wide_null(CAPTURE_CLASS.trim_end_matches('\0'));
            let cap_wc = WNDCLASSEXW {
                cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(80),
                lpfnWndProc: Some(capture_wnd_proc),
                hInstance: hinstance,
                hCursor: cursor,
                lpszClassName: PCWSTR::from_raw(cap_class.as_ptr()),
                ..Default::default()
            };
            // SAFETY: Register F1 overlay window class.
            let _ = unsafe { RegisterClassExW(&cap_wc) };

            let nudge_class = to_wide_null(NUDGE_CLASS.trim_end_matches('\0'));
            let nudge_wc = WNDCLASSEXW {
                cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(80),
                lpfnWndProc: Some(nudge_wnd_proc),
                hInstance: hinstance,
                hCursor: cursor,
                lpszClassName: PCWSTR::from_raw(nudge_class.as_ptr()),
                ..Default::default()
            };
            // SAFETY: Register F7 overlay window class.
            let _ = unsafe { RegisterClassExW(&nudge_wc) };

            let colors = load_theme_colors(theme)?;
            let monitor = monitor_for_cursor();
            let cap_rect = compute_capture_rect(monitor);
            let cap_title = to_wide_null(&copy.capture_input_label);

            // SAFETY: Pre-create hidden F1 Capture overlay (`WS_EX_TOOLWINDOW | WS_EX_TOPMOST`).
            let capture_hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                    PCWSTR::from_raw(cap_class.as_ptr()),
                    PCWSTR::from_raw(cap_title.as_ptr()),
                    WS_POPUP,
                    cap_rect.left,
                    cap_rect.top,
                    cap_rect.width(),
                    cap_rect.height(),
                    None,
                    None,
                    hinstance,
                    None,
                )
            }
            .map_err(|e| Win32Error::api("CreateWindowExW(F1)", e.to_string()))?;

            let static_cls = to_wide_null("STATIC");
            let button_cls = to_wide_null("BUTTON");
            let hint_w = to_wide_null(&copy.capture_hint);
            // SAFETY: Child `MSFTEDIT_CLASS` RichEdit control inside F1 overlay.
            let capture_edit_hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    MSFTEDIT_CLASS,
                    PCWSTR::from_raw(cap_title.as_ptr()),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_TABSTOP
                        | WINDOW_STYLE(
                            (ES_MULTILINE as u32)
                                | (ES_AUTOVSCROLL as u32)
                                | (ES_WANTRETURN as u32),
                        ),
                    0,
                    0,
                    100,
                    50,
                    capture_hwnd,
                    None,
                    hinstance,
                    None,
                )
            }
            .map_err(|e| Win32Error::api("CreateWindowExW(RichEdit)", e.to_string()))?;

            let capture_hint_hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR::from_raw(static_cls.as_ptr()),
                    PCWSTR::from_raw(hint_w.as_ptr()),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    100,
                    24,
                    capture_hwnd,
                    None,
                    hinstance,
                    None,
                )
            }
            .map_err(|e| Win32Error::api("CreateWindowExW(Hint)", e.to_string()))?;

            let mut cap_state = Box::new(CaptureState {
                hwnd: capture_hwnd,
                edit_hwnd: capture_edit_hwnd,
                hint_hwnd: capture_hint_hwnd,
                saved_foreground: None,
                visible: false,
                dpi: monitor.dpi,
                colors,
                bg_brush: unsafe { CreateSolidBrush(COLORREF(colors.bg_canvas.to_colorref())) },
                surface_brush: unsafe {
                    CreateSolidBrush(COLORREF(colors.bg_surface.to_colorref()))
                },
                font_edit: HFONT::default(),
                font_hint: HFONT::default(),
                event_tx: event_tx.clone(),
            });
            cap_state.apply_theme(theme);
            cap_state.layout(cap_rect, monitor.dpi);
            let cap_ptr = Box::into_raw(cap_state);
            // SAFETY: Attach `CaptureState` to `capture_hwnd` and subclass RichEdit for Enter/Esc.
            unsafe {
                SetWindowLongPtrW(capture_hwnd, GWLP_USERDATA, cap_ptr as isize);
                let _ = SetWindowSubclass(
                    capture_edit_hwnd,
                    Some(richedit_subclass_proc),
                    RICHEDIT_SUBCLASS_ID,
                    cap_ptr as usize,
                );
                let _ = SendMessageW(
                    capture_edit_hwnd,
                    windows::Win32::UI::Controls::RichEdit::EM_SETEVENTMASK,
                    WPARAM(0),
                    LPARAM(ENM_NONE as isize),
                );
            }

            // Pre-create hidden F7 Nudge overlay (`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST`).
            let nudge_rect = compute_nudge_rect(monitor);
            let nudge_title = to_wide_null(&copy.nudge_title);
            let nudge_body = to_wide_null(&copy.nudge_short_break);
            let shrink_label = to_wide_null(&copy.nudge_shrink_step);
            let not_now_label = to_wide_null(&copy.nudge_not_now);

            let nudge_hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                    PCWSTR::from_raw(nudge_class.as_ptr()),
                    PCWSTR::from_raw(nudge_title.as_ptr()),
                    WS_POPUP,
                    nudge_rect.left,
                    nudge_rect.top,
                    nudge_rect.width(),
                    nudge_rect.height(),
                    None,
                    None,
                    hinstance,
                    None,
                )
            }
            .map_err(|e| Win32Error::api("CreateWindowExW(F7)", e.to_string()))?;

            let nudge_label_hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR::from_raw(static_cls.as_ptr()),
                    PCWSTR::from_raw(nudge_body.as_ptr()),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    100,
                    40,
                    nudge_hwnd,
                    None,
                    hinstance,
                    None,
                )
            }
            .map_err(|e| Win32Error::api("CreateWindowExW(NudgeLabel)", e.to_string()))?;

            let nudge_shrink_hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR::from_raw(button_cls.as_ptr()),
                    PCWSTR::from_raw(shrink_label.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    0,
                    0,
                    100,
                    32,
                    nudge_hwnd,
                    windows::Win32::UI::WindowsAndMessaging::HMENU(BTN_SHRINK_ID as *mut c_void),
                    hinstance,
                    None,
                )
            }
            .map_err(|e| Win32Error::api("CreateWindowExW(NudgeShrink)", e.to_string()))?;

            let nudge_not_now_hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    PCWSTR::from_raw(button_cls.as_ptr()),
                    PCWSTR::from_raw(not_now_label.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                    0,
                    0,
                    100,
                    32,
                    nudge_hwnd,
                    windows::Win32::UI::WindowsAndMessaging::HMENU(BTN_NOT_NOW_ID as *mut c_void),
                    hinstance,
                    None,
                )
            }
            .map_err(|e| Win32Error::api("CreateWindowExW(NudgeNotNow)", e.to_string()))?;

            // Configure UIA LiveSetting = Polite (1) via `IAccPropServices` on `nudge_label_hwnd`.
            // SAFETY: Best-effort COM property registration for screen reader polite announcement.
            if let Ok(acc_props) = unsafe {
                CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_ALL)
            } {
                let polite = windows::core::VARIANT::from(1i32);
                let _ = unsafe {
                    acc_props.SetHwndProp(
                        nudge_label_hwnd,
                        OBJID_CLIENT.0 as u32,
                        0,
                        LiveSetting_Property_GUID,
                        &polite,
                    )
                };
            }

            let mut nudge_state = Box::new(NudgeState {
                hwnd: nudge_hwnd,
                label_hwnd: nudge_label_hwnd,
                shrink_btn_hwnd: nudge_shrink_hwnd,
                dismiss_btn_hwnd: nudge_not_now_hwnd,
                visible: false,
                focus_unlocked: false,
                dpi: monitor.dpi,
                colors,
                bg_brush: unsafe { CreateSolidBrush(COLORREF(colors.bg_surface.to_colorref())) },
                font_body: HFONT::default(),
                font_btn: HFONT::default(),
                event_tx,
            });
            nudge_state.apply_theme(theme);
            nudge_state.layout(nudge_rect, monitor.dpi);
            let nudge_ptr = Box::into_raw(nudge_state);
            // SAFETY: Attach `NudgeState` to `nudge_hwnd`.
            unsafe { SetWindowLongPtrW(nudge_hwnd, GWLP_USERDATA, nudge_ptr as isize) };

            Ok(Self {
                capture_hwnd,
                nudge_hwnd,
                nudge_label_hwnd,
                nudge_shrink_hwnd,
                nudge_not_now_hwnd,
                capture_hint_hwnd,
                capture_edit_hwnd,
            })
        }

        pub(crate) fn show_capture(&self) {
            // SAFETY: Called on `ef-win32-shell` thread owning `capture_hwnd`.
            let ptr =
                unsafe { GetWindowLongPtrW(self.capture_hwnd, GWLP_USERDATA) } as *mut CaptureState;
            if !ptr.is_null() {
                unsafe { (&mut *ptr).show() };
            }
        }

        pub(crate) fn show_nudge(&self, message: Option<&str>) {
            // SAFETY: Called on `ef-win32-shell` thread owning `nudge_hwnd`.
            let ptr =
                unsafe { GetWindowLongPtrW(self.nudge_hwnd, GWLP_USERDATA) } as *mut NudgeState;
            if !ptr.is_null() {
                unsafe { (&mut *ptr).show(message) };
            }
        }

        pub(crate) fn unlock_nudge_focus(&self) {
            let ptr =
                unsafe { GetWindowLongPtrW(self.nudge_hwnd, GWLP_USERDATA) } as *mut NudgeState;
            if !ptr.is_null() {
                unsafe { (&mut *ptr).unlock_focus() };
            }
        }

        pub(crate) fn apply_theme(&self, theme: ThemeId) {
            let cap_ptr =
                unsafe { GetWindowLongPtrW(self.capture_hwnd, GWLP_USERDATA) } as *mut CaptureState;
            if !cap_ptr.is_null() {
                unsafe { (&mut *cap_ptr).apply_theme(theme) };
            }
            let nudge_ptr =
                unsafe { GetWindowLongPtrW(self.nudge_hwnd, GWLP_USERDATA) } as *mut NudgeState;
            if !nudge_ptr.is_null() {
                unsafe { (&mut *nudge_ptr).apply_theme(theme) };
            }
        }

        pub(crate) fn update_copy(&self, copy: &ShellCopy) {
            set_hwnd_text(self.capture_hwnd, &copy.capture_input_label);
            set_hwnd_text(self.capture_edit_hwnd, &copy.capture_input_label);
            set_hwnd_text(self.capture_hint_hwnd, &copy.capture_hint);
            set_hwnd_text(self.nudge_hwnd, &copy.nudge_title);
            set_hwnd_text(self.nudge_label_hwnd, &copy.nudge_short_break);
            set_hwnd_text(self.nudge_shrink_hwnd, &copy.nudge_shrink_step);
            set_hwnd_text(self.nudge_not_now_hwnd, &copy.nudge_not_now);
        }

        pub(crate) fn destroy(&self) {
            // SAFETY: Destroy both pre-created overlay windows on the shell thread.
            unsafe {
                let _ = DestroyWindow(self.capture_hwnd);
                let _ = DestroyWindow(self.nudge_hwnd);
            }
        }
    }
}
