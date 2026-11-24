#![deny(unsafe_op_in_unsafe_fn)]

use std::path::PathBuf;

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
}
