/// Screen coordinates in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ScreenRect {
    #[must_use]
    pub const fn width(self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    #[must_use]
    pub const fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }
}

/// Work area and effective DPI for a target monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorInfo {
    pub work_area: ScreenRect,
    pub dpi: u32,
}

impl Default for MonitorInfo {
    fn default() -> Self {
        Self {
            work_area: ScreenRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            dpi: 96,
        }
    }
}

pub const CAPTURE_WIDTH_DIP: i32 = 640;
pub const CAPTURE_HEIGHT_DIP: i32 = 136;
pub const NUDGE_WIDTH_DIP: i32 = 380;
pub const NUDGE_HEIGHT_DIP: i32 = 140;
pub const NUDGE_MARGIN_DIP: i32 = 20;

/// Scales a logical DIP value to physical pixels at `dpi` (minimum 1 px).
#[must_use]
pub fn scale_dip(dip: i32, dpi: u32) -> i32 {
    let safe_dpi = i64::from(dpi.max(96));
    let scaled = (i64::from(dip) * safe_dpi + 48) / 96;
    i32::try_from(scaled).unwrap_or(dip).max(1)
}

/// Computes the physical pixel rectangle for the F1 Capture overlay,
/// centered horizontally and placed in the upper quarter of `monitor.work_area`.
#[must_use]
pub fn compute_capture_rect(monitor: MonitorInfo) -> ScreenRect {
    let work = monitor.work_area;
    let margin = scale_dip(16, monitor.dpi);
    let max_w = (work.width() - margin * 2).max(200);
    let max_h = (work.height() - margin * 2).max(96);
    let width = scale_dip(CAPTURE_WIDTH_DIP, monitor.dpi).min(max_w);
    let height = scale_dip(CAPTURE_HEIGHT_DIP, monitor.dpi).min(max_h);

    let left = work.left + (work.width() - width) / 2;
    let top_offset = (work.height() / 4).max(margin);
    let max_top = (work.bottom - height - margin).max(work.top);
    let top = (work.top + top_offset).min(max_top).max(work.top);

    ScreenRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

/// Computes the physical pixel rectangle for the F7 Nudge overlay in the
/// bottom-right corner of `monitor.work_area`, clear of the taskbar.
#[must_use]
pub fn compute_nudge_rect(monitor: MonitorInfo) -> ScreenRect {
    let work = monitor.work_area;
    let margin = scale_dip(NUDGE_MARGIN_DIP, monitor.dpi);
    let max_w = (work.width() - margin * 2).max(200);
    let max_h = (work.height() - margin * 2).max(96);
    let width = scale_dip(NUDGE_WIDTH_DIP, monitor.dpi).min(max_w);
    let height = scale_dip(NUDGE_HEIGHT_DIP, monitor.dpi).min(max_h);

    let right = (work.right - margin).max(work.left + width);
    let bottom = (work.bottom - margin).max(work.top + height);
    let left = right - width;
    let top = bottom - height;

    ScreenRect {
        left,
        top,
        right,
        bottom,
    }
}

/// Queries the monitor containing the current cursor position and its effective DPI.
#[must_use]
pub fn monitor_for_cursor() -> MonitorInfo {
    #[cfg(windows)]
    {
        windows_impl::monitor_for_cursor()
    }
    #[cfg(not(windows))]
    {
        MonitorInfo::default()
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    use super::{MonitorInfo, ScreenRect};

    pub(super) fn monitor_for_cursor() -> MonitorInfo {
        let mut pt = POINT::default();
        // SAFETY: `pt` is a valid mutable `POINT`.
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return MonitorInfo::default();
        }
        // SAFETY: `MonitorFromPoint` with `MONITOR_DEFAULTTONEAREST` always returns a valid monitor or null.
        let hmon = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
        if hmon.is_invalid() {
            return MonitorInfo::default();
        }

        let mut mi = MONITORINFO {
            cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(40),
            ..Default::default()
        };
        // SAFETY: `hmon` is valid and `mi.cbSize` is initialized.
        if !unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
            return MonitorInfo::default();
        }

        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        // SAFETY: `hmon` is valid and output pointers are valid mutable `u32`s.
        if unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_err() {
            dpi_x = 96;
        }

        MonitorInfo {
            work_area: ScreenRect {
                left: mi.rcWork.left,
                top: mi.rcWork.top,
                right: mi.rcWork.right,
                bottom: mi.rcWork.bottom,
            },
            dpi: dpi_x.max(96),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_dpi_scaling_and_negative_monitor_coords() {
        assert_eq!(scale_dip(100, 96), 100);
        assert_eq!(scale_dip(100, 144), 150);
        assert_eq!(scale_dip(100, 192), 200);

        // Secondary 150% DPI monitor positioned to the left of primary (negative X).
        let secondary = MonitorInfo {
            work_area: ScreenRect {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1040,
            },
            dpi: 144,
        };
        let cap = compute_capture_rect(secondary);
        assert!(cap.left >= secondary.work_area.left);
        assert!(cap.right <= secondary.work_area.right);
        assert_eq!(cap.width(), 960);

        let nudge = compute_nudge_rect(secondary);
        assert!(nudge.left >= secondary.work_area.left);
        assert_eq!(nudge.right, -30);
        assert_eq!(nudge.bottom, 1010);
        assert_eq!(nudge.width(), 570);
    }

    #[test]
    fn monitor_for_cursor_returns_sane_dimensions() {
        let info = monitor_for_cursor();
        assert!(info.dpi >= 96);
        assert!(info.work_area.width() > 0);
        assert!(info.work_area.height() > 0);
    }
}
