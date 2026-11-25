#![deny(unsafe_op_in_unsafe_fn)]

pub mod activity;
pub mod dpi;
pub mod error;
pub mod hotkey;
pub mod job_object;
pub mod overlays;
pub mod palette;
pub mod secrets;
pub mod shell;
pub mod single_instance;
pub mod sqlite_vec;
pub mod startup;
pub mod toast;
pub mod tray;

use std::path::PathBuf;

pub use activity::{
    ACTIVITY_EVENT_CHANNEL_CAPACITY, ActivityEvent, ActivityMonitor, ActivityStatsSnapshot,
    ProcessDiagnostics, SamplingPolicy, UIA_TIMEOUT, compute_idle_seconds,
    current_foreground_process_name, current_idle_seconds, current_unix_ms,
    extract_process_basename, process_diagnostics,
};
pub use dpi::{
    MonitorInfo, ScreenRect, compute_capture_rect, compute_nudge_rect, monitor_for_cursor,
    scale_dip,
};
pub use error::Win32Error;
pub use hotkey::{HotkeyBinding, HotkeyKind, HotkeyRegistrationResult};
pub use job_object::JobObject;
pub use palette::{
    OverlayColors, RgbColor, ThemeId, is_high_contrast_enabled, is_system_dark_mode,
    load_theme_colors,
};
pub use secrets::{DbKey, KeyStoreError, get_or_create_db_key};
pub use shell::{
    NativeShell, NativeShellConfig, SHELL_EVENT_CHANNEL_CAPACITY, ShellCommand, ShellEvent,
};
pub use single_instance::{
    DEFAULT_SINGLE_INSTANCE_MUTEX, DEFAULT_WAKEUP_MESSAGE_NAME, DEFAULT_WAKEUP_WINDOW_CLASS,
    SingleInstanceGuard, SingleInstanceOutcome,
};
pub use sqlite_vec::register_sqlite_vec;
pub use startup::{LaunchAtLoginState, get_launch_at_login, set_launch_at_login};
pub use toast::{ToastResult, build_toast_xml, escape_xml, show_toast_best_effort};
pub use tray::ShellCopy;

/// Resolved application directories (`CONTEXT.md` Section 6).
///
/// Always routes user data, logs, attachments, and downloaded GGUF models outside the
/// read-only MSIX install directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub is_packaged: bool,
    pub local_data_dir: PathBuf,
    pub models_dir: PathBuf,
    pub attachments_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl AppPaths {
    fn from_root(local_data_dir: PathBuf, is_packaged: bool) -> Self {
        let models_dir = local_data_dir.join("models");
        let attachments_dir = local_data_dir.join("attachments");
        let logs_dir = local_data_dir.join("logs");
        Self {
            is_packaged,
            local_data_dir,
            models_dir,
            attachments_dir,
            logs_dir,
        }
    }
}

/// Overlay focus semantics (`CONTEXT.md`, `PHASE-02`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    /// User-initiated F1 capture: must take focus (`WS_EX_TOOLWINDOW` only) and restore saved HWND before hiding.
    Capture,
    /// Unprompted F7 nudge: must never steal focus (`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`), with keyboard fallback route.
    Nudge,
}

impl OverlayKind {
    #[must_use]
    pub const fn takes_focus_on_show(self) -> bool {
        matches!(self, Self::Capture)
    }

    #[must_use]
    pub const fn restores_foreground_before_hide(self) -> bool {
        matches!(self, Self::Capture)
    }
}

/// Resolves writable local storage paths cleanly for both MSIX packaged and unpackaged dev runs.
#[must_use]
pub fn resolve_app_paths() -> AppPaths {
    #[cfg(windows)]
    {
        if let Some(packaged) = windows_impl::try_msix_local_folder() {
            return AppPaths::from_root(packaged, true);
        }
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        AppPaths::from_root(base.join("EfAssistant-Dev"), false)
    }

    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("ef-assistant-dev"));
        AppPaths::from_root(base.join("Vun"), false)
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::path::PathBuf;
    use windows::Storage::ApplicationData;

    pub(super) fn try_msix_local_folder() -> Option<PathBuf> {
        let app_data = ApplicationData::Current().ok()?;
        let local_folder = app_data.LocalFolder().ok()?;
        let hstring_path = local_folder.Path().ok()?;
        Some(PathBuf::from(hstring_path.to_os_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_focus_contracts_are_opposite() {
        assert!(OverlayKind::Capture.takes_focus_on_show());
        assert!(OverlayKind::Capture.restores_foreground_before_hide());
        assert!(!OverlayKind::Nudge.takes_focus_on_show());
        assert!(!OverlayKind::Nudge.restores_foreground_before_hide());
    }

    #[test]
    fn resolves_non_empty_local_data_dir() {
        let paths = resolve_app_paths();
        assert!(!paths.local_data_dir.as_os_str().is_empty());
        assert!(paths.models_dir.starts_with(&paths.local_data_dir));
    }

    #[test]
    fn db_key_redacts_debug_and_formats_sqlcipher_hex() {
        let key = DbKey::from_bytes([0xab; 32]);
        assert_eq!(format!("{key:?}"), "DbKey([REDACTED])");
        let hex = key.to_sqlcipher_pragma_hex();
        assert_eq!(hex.len(), 67);
        assert!(hex.starts_with("x'abab"));
        assert!(hex.ends_with('\''));
    }

    #[test]
    fn sqlite_vec_registration_is_idempotent() {
        register_sqlite_vec().expect("first sqlite-vec registration");
        register_sqlite_vec().expect("second sqlite-vec registration");
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        let version: String = conn
            .query_row("SELECT vec_version()", [], |row| row.get(0))
            .expect("query vec_version()");
        assert!(
            version.starts_with('v'),
            "unexpected vec_version: {version}"
        );
    }
}
