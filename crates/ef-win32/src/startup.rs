use serde::{Deserialize, Serialize};

/// Current OS launch-at-login status (`PHASE-02` §1: off by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchAtLoginState {
    Disabled,
    Enabled,
    DisabledByUser,
    DisabledByPolicy,
    Unsupported,
}

impl LaunchAtLoginState {
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// Reads the current launch-at-login state (MSIX `StartupTask` when packaged,
/// `HKCU\...\Run` when running standalone).
#[must_use]
pub fn get_launch_at_login(is_packaged: bool) -> LaunchAtLoginState {
    #[cfg(windows)]
    {
        windows_impl::get_state(is_packaged)
    }
    #[cfg(not(windows))]
    {
        let _ = is_packaged;
        unix_impl::get_state()
    }
}

/// Enables or disables launch-at-login (`--minimized` in standalone Run key).
pub fn set_launch_at_login(
    is_packaged: bool,
    enabled: bool,
) -> Result<LaunchAtLoginState, crate::Win32Error> {
    #[cfg(windows)]
    {
        windows_impl::set_state(is_packaged, enabled)
    }
    #[cfg(not(windows))]
    {
        let _ = is_packaged;
        Ok(unix_impl::set_state(enabled))
    }
}

#[cfg(not(windows))]
mod unix_impl {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::LaunchAtLoginState;

    static DEV_STARTUP: AtomicBool = AtomicBool::new(false);

    pub(super) fn get_state() -> LaunchAtLoginState {
        if DEV_STARTUP.load(Ordering::Relaxed) {
            LaunchAtLoginState::Enabled
        } else {
            LaunchAtLoginState::Disabled
        }
    }

    pub(super) fn set_state(enabled: bool) -> LaunchAtLoginState {
        DEV_STARTUP.store(enabled, Ordering::Relaxed);
        get_state()
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::ApplicationModel::{StartupTask, StartupTaskState};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, REG_VALUE_TYPE, RegCloseKey,
        RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows::core::{HSTRING, PCWSTR};

    use super::LaunchAtLoginState;
    use crate::Win32Error;

    const STARTUP_TASK_ID: &str = "VunStartupTask";
    const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run\0";
    const RUN_VALUE_NAME: &str = "Vun\0";

    fn map_task_state(state: StartupTaskState) -> LaunchAtLoginState {
        match state {
            StartupTaskState::Disabled => LaunchAtLoginState::Disabled,
            StartupTaskState::DisabledByUser => LaunchAtLoginState::DisabledByUser,
            StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy => {
                LaunchAtLoginState::Enabled
            }
            StartupTaskState::DisabledByPolicy => LaunchAtLoginState::DisabledByPolicy,
            _ => LaunchAtLoginState::Unsupported,
        }
    }

    pub(super) fn get_state(is_packaged: bool) -> LaunchAtLoginState {
        if is_packaged {
            let Ok(op) = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID)) else {
                return LaunchAtLoginState::Unsupported;
            };
            let Ok(task) = op.get() else {
                return LaunchAtLoginState::Unsupported;
            };
            return task
                .State()
                .map(map_task_state)
                .unwrap_or(LaunchAtLoginState::Unsupported);
        }
        get_run_key_state()
    }

    pub(super) fn set_state(
        is_packaged: bool,
        enabled: bool,
    ) -> Result<LaunchAtLoginState, Win32Error> {
        if is_packaged {
            let task = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK_ID))
                .map_err(|e| Win32Error::api("StartupTask::GetAsync", e.to_string()))?
                .get()
                .map_err(|e| Win32Error::api("StartupTask::GetAsync.get", e.to_string()))?;
            if enabled {
                let new_state = task
                    .RequestEnableAsync()
                    .map_err(|e| Win32Error::api("StartupTask::RequestEnableAsync", e.to_string()))?
                    .get()
                    .map_err(|e| {
                        Win32Error::api("StartupTask::RequestEnableAsync.get", e.to_string())
                    })?;
                return Ok(map_task_state(new_state));
            }
            task.Disable()
                .map_err(|e| Win32Error::api("StartupTask::Disable", e.to_string()))?;
            let state = task
                .State()
                .map(map_task_state)
                .unwrap_or(LaunchAtLoginState::Disabled);
            return Ok(state);
        }
        set_run_key_state(enabled)
    }

    fn get_run_key_state() -> LaunchAtLoginState {
        let subkey_w: Vec<u16> = RUN_SUBKEY.encode_utf16().collect();
        let value_w: Vec<u16> = RUN_VALUE_NAME.encode_utf16().collect();
        let mut hkey = HKEY::default();
        // SAFETY: Opening HKCU Run key read-only.
        let open_err = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR::from_raw(subkey_w.as_ptr()),
                0,
                KEY_READ,
                &mut hkey,
            )
        };
        if open_err.0 != 0 {
            return LaunchAtLoginState::Disabled;
        }
        let mut val_type = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        // SAFETY: Querying value presence/length in HKCU Run.
        let q_err = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR::from_raw(value_w.as_ptr()),
                None,
                Some(&mut val_type),
                None,
                Some(&mut size),
            )
        };
        // SAFETY: Close opened handle.
        let _ = unsafe { RegCloseKey(hkey) };
        if q_err.0 == 0 && size > 0 {
            LaunchAtLoginState::Enabled
        } else {
            LaunchAtLoginState::Disabled
        }
    }

    fn set_run_key_state(enabled: bool) -> Result<LaunchAtLoginState, Win32Error> {
        let subkey_w: Vec<u16> = RUN_SUBKEY.encode_utf16().collect();
        let value_w: Vec<u16> = RUN_VALUE_NAME.encode_utf16().collect();
        let mut hkey = HKEY::default();
        // SAFETY: Opening HKCU Run key with write access.
        let open_err = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR::from_raw(subkey_w.as_ptr()),
                0,
                KEY_WRITE,
                &mut hkey,
            )
        };
        if open_err.0 != 0 {
            return Err(Win32Error::api(
                "RegOpenKeyExW",
                format!("Win32 error {}", open_err.0),
            ));
        }

        let result = if enabled {
            let exe = std::env::current_exe()?;
            let cmd = format!("\"{}\" --minimized", exe.display());
            let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes: Vec<u8> = wide.iter().flat_map(|u| u.to_le_bytes()).collect();
            // SAFETY: `hkey` is open and `bytes` contains NUL-terminated UTF-16 `REG_SZ`.
            let err = unsafe {
                RegSetValueExW(
                    hkey,
                    PCWSTR::from_raw(value_w.as_ptr()),
                    0,
                    REG_SZ,
                    Some(&bytes),
                )
            };
            if err.0 == 0 {
                Ok(LaunchAtLoginState::Enabled)
            } else {
                Err(Win32Error::api(
                    "RegSetValueExW",
                    format!("Win32 error {}", err.0),
                ))
            }
        } else {
            // SAFETY: `hkey` is open; `ERROR_FILE_NOT_FOUND` means already disabled.
            let err = unsafe { RegDeleteValueW(hkey, PCWSTR::from_raw(value_w.as_ptr())) };
            if err.0 == 0 || err == ERROR_FILE_NOT_FOUND {
                Ok(LaunchAtLoginState::Disabled)
            } else {
                Err(Win32Error::api(
                    "RegDeleteValueW",
                    format!("Win32 error {}", err.0),
                ))
            }
        };

        // SAFETY: Always close opened registry handle.
        let _ = unsafe { RegCloseKey(hkey) };
        result
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn launch_at_login_defaults_disabled_and_toggles_cleanly() {
        assert_eq!(get_launch_at_login(false), LaunchAtLoginState::Disabled);
        let enabled = set_launch_at_login(false, true).expect("enable");
        assert_eq!(enabled, LaunchAtLoginState::Enabled);
        let disabled = set_launch_at_login(false, false).expect("disable");
        assert_eq!(disabled, LaunchAtLoginState::Disabled);
    }
}
