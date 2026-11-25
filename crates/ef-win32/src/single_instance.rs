use crate::Win32Error;

pub const DEFAULT_SINGLE_INSTANCE_MUTEX: &str = "Local\\ThanhNghiepQuan.Vun.SingleInstance";
pub const DEFAULT_WAKEUP_WINDOW_CLASS: &str = "VunShellMessageWindow";
pub const DEFAULT_WAKEUP_MESSAGE_NAME: &str = "ThanhNghiepQuan.Vun.Wakeup";

/// Result of attempting to claim the single-instance lock.
#[derive(Debug)]
pub enum SingleInstanceOutcome {
    Primary(SingleInstanceGuard),
    AlreadyRunning { wakeup_sent: bool },
}

/// Owned single-instance mutex guard released on drop.
#[derive(Debug)]
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
    #[cfg(not(windows))]
    mutex_name: String,
}

// SAFETY: A Win32 Mutex handle can be owned and closed on shutdown.
unsafe impl Send for SingleInstanceGuard {}
// SAFETY: `SingleInstanceGuard` only closes its handle on drop.
unsafe impl Sync for SingleInstanceGuard {}

impl SingleInstanceGuard {
    /// Claims `mutex_name`. If another instance owns it, sends `wakeup_msg_name`
    /// to `wakeup_window_class` and returns `AlreadyRunning`.
    pub fn acquire(
        mutex_name: &str,
        wakeup_window_class: &str,
        wakeup_msg_name: &str,
    ) -> Result<SingleInstanceOutcome, Win32Error> {
        #[cfg(windows)]
        {
            windows_impl::acquire(mutex_name, wakeup_window_class, wakeup_msg_name)
        }
        #[cfg(not(windows))]
        {
            let _ = (wakeup_window_class, wakeup_msg_name);
            unix_impl::acquire(mutex_name)
        }
    }
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            // SAFETY: `self.handle` was created by `CreateMutexW` and is owned exclusively.
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.handle) };
        }
    }
}

#[cfg(not(windows))]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unix_impl::release(&self.mutex_name);
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, LPARAM, WPARAM,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, PostMessageW, RegisterWindowMessageW,
    };
    use windows::core::PCWSTR;

    use super::{SingleInstanceGuard, SingleInstanceOutcome};
    use crate::Win32Error;

    fn to_wide_null(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn acquire(
        mutex_name: &str,
        wakeup_window_class: &str,
        wakeup_msg_name: &str,
    ) -> Result<SingleInstanceOutcome, Win32Error> {
        let mutex_w = to_wide_null(mutex_name);
        // SAFETY: `mutex_w` is NUL-terminated UTF-16.
        let handle = unsafe { CreateMutexW(None, false, PCWSTR::from_raw(mutex_w.as_ptr())) }
            .map_err(|e| Win32Error::api("CreateMutexW", e.to_string()))?;

        // SAFETY: `GetLastError` reads the thread-local Win32 error set by `CreateMutexW`.
        let last_err = unsafe { GetLastError() };
        if last_err == ERROR_ALREADY_EXISTS {
            // SAFETY: Do not retain a duplicate handle when another primary instance exists.
            let _ = unsafe { CloseHandle(handle) };
            let wakeup_sent = send_wakeup(wakeup_window_class, wakeup_msg_name);
            return Ok(SingleInstanceOutcome::AlreadyRunning { wakeup_sent });
        }

        Ok(SingleInstanceOutcome::Primary(SingleInstanceGuard {
            handle,
        }))
    }

    fn send_wakeup(window_class: &str, msg_name: &str) -> bool {
        let class_w = to_wide_null(window_class);
        let msg_w = to_wide_null(msg_name);
        // SAFETY: Both strings are valid NUL-terminated UTF-16 buffers.
        let msg_id = unsafe { RegisterWindowMessageW(PCWSTR::from_raw(msg_w.as_ptr())) };
        if msg_id == 0 {
            return false;
        }
        // SAFETY: `class_w` is valid NUL-terminated UTF-16; `FindWindowW` returns `Err` when absent.
        let Ok(hwnd) = (unsafe { FindWindowW(PCWSTR::from_raw(class_w.as_ptr()), PCWSTR::null()) })
        else {
            return false;
        };
        if hwnd.is_invalid() {
            return false;
        }
        // SAFETY: Posting custom registered message to the existing instance's message window.
        unsafe { PostMessageW(hwnd, msg_id, WPARAM(0), LPARAM(0)) }.is_ok()
    }
}

#[cfg(not(windows))]
mod unix_impl {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    use super::{SingleInstanceGuard, SingleInstanceOutcome};
    use crate::Win32Error;

    static HELD_LOCKS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

    fn registry() -> &'static Mutex<HashSet<String>> {
        HELD_LOCKS.get_or_init(|| Mutex::new(HashSet::new()))
    }

    pub(super) fn acquire(mutex_name: &str) -> Result<SingleInstanceOutcome, Win32Error> {
        let mut guard = registry()
            .lock()
            .map_err(|_| Win32Error::api("unix_mutex_lock", "poisoned"))?;
        if guard.contains(mutex_name) {
            return Ok(SingleInstanceOutcome::AlreadyRunning { wakeup_sent: false });
        }
        guard.insert(mutex_name.to_string());
        Ok(SingleInstanceOutcome::Primary(SingleInstanceGuard {
            mutex_name: mutex_name.to_string(),
        }))
    }

    pub(super) fn release(mutex_name: &str) {
        if let Ok(mut guard) = registry().lock() {
            guard.remove(mutex_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_instance_prevents_second_acquire_until_dropped() {
        let name = format!("Local\\VunTestMutex-{}", std::process::id());
        let first = SingleInstanceGuard::acquire(
            &name,
            DEFAULT_WAKEUP_WINDOW_CLASS,
            DEFAULT_WAKEUP_MESSAGE_NAME,
        )
        .expect("first acquire");
        let guard = match first {
            SingleInstanceOutcome::Primary(g) => g,
            SingleInstanceOutcome::AlreadyRunning { .. } => {
                panic!("expected Primary on first acquire")
            }
        };

        let second = SingleInstanceGuard::acquire(
            &name,
            DEFAULT_WAKEUP_WINDOW_CLASS,
            DEFAULT_WAKEUP_MESSAGE_NAME,
        )
        .expect("second acquire");
        assert!(matches!(
            second,
            SingleInstanceOutcome::AlreadyRunning { .. }
        ));

        drop(guard);

        let third = SingleInstanceGuard::acquire(
            &name,
            DEFAULT_WAKEUP_WINDOW_CLASS,
            DEFAULT_WAKEUP_MESSAGE_NAME,
        )
        .expect("third acquire after drop");
        assert!(matches!(third, SingleInstanceOutcome::Primary(_)));
    }
}
