use std::fmt;

use thiserror::Error;

/// 256-bit SQLCipher database encryption key with redacted `Debug` and zeroize-on-drop.
#[derive(Clone, PartialEq, Eq)]
pub struct DbKey([u8; 32]);

impl DbKey {
    /// Generates a fresh 256-bit cryptographically random key.
    pub fn generate() -> Result<Self, KeyStoreError> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|e| KeyStoreError::Entropy(e.to_string()))?;
        Ok(Self(bytes))
    }

    /// Wraps an existing 32-byte key.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Formats the key as a SQLCipher raw hex key literal (`x'<64 lowercase hex chars>'`).
    #[must_use]
    pub fn to_sqlcipher_pragma_hex(&self) -> String {
        let mut out = String::with_capacity(2 + 64 + 1);
        out.push_str("x'");
        for byte in self.0 {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
        out.push('\'');
        out
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for DbKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DbKey([REDACTED])")
    }
}

impl Drop for DbKey {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            // SAFETY: `byte` is a valid, aligned mutable reference to an element of `self.0`.
            // Using `write_volatile` prevents the compiler from eliding key zeroization on drop.
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

#[derive(Debug, Error)]
pub enum KeyStoreError {
    #[error("failed to generate random key material: {0}")]
    Entropy(String),
    #[error("stored credential key has invalid length {0}; expected 32 bytes")]
    InvalidKeyLength(usize),
    #[error("credential store I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Windows Credential Manager error: {0}")]
    WindowsCred(String),
}

/// Loads the 256-bit SQLCipher key for `target_name` from the OS credential store,
/// or generates and persists a new key on first run.
pub fn get_or_create_db_key(target_name: &str) -> Result<DbKey, KeyStoreError> {
    #[cfg(windows)]
    {
        windows_cred::get_or_create(target_name)
    }

    #[cfg(not(windows))]
    {
        unix_dev_key::get_or_create(target_name)
    }
}

#[cfg(windows)]
mod windows_cred {
    use std::ffi::c_void;

    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{
        CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredFree,
        CredReadW, CredWriteW,
    };
    use windows::core::{PCWSTR, PWSTR};

    use super::{DbKey, KeyStoreError};

    fn to_wide_null(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn get_or_create(target_name: &str) -> Result<DbKey, KeyStoreError> {
        let mut target_w = to_wide_null(target_name);
        let mut cred_ptr: *mut CREDENTIALW = std::ptr::null_mut();

        // SAFETY: `target_w` is NUL-terminated UTF-16 and `cred_ptr` receives an OS-allocated
        // `CREDENTIALW` buffer freed via `CredFree` below.
        let read_res = unsafe {
            CredReadW(
                PCWSTR::from_raw(target_w.as_ptr()),
                CRED_TYPE_GENERIC,
                None,
                &mut cred_ptr,
            )
        };

        match read_res {
            Ok(()) => {
                // SAFETY: `CredReadW` succeeded so `cred_ptr` is non-null and valid until `CredFree`.
                let key_result = unsafe {
                    let cred = &*cred_ptr;
                    let len = cred.CredentialBlobSize as usize;
                    let result = if len == 32 && !cred.CredentialBlob.is_null() {
                        let slice = std::slice::from_raw_parts(cred.CredentialBlob, 32);
                        let mut buf = [0u8; 32];
                        buf.copy_from_slice(slice);
                        Ok(DbKey::from_bytes(buf))
                    } else {
                        Err(KeyStoreError::InvalidKeyLength(len))
                    };
                    CredFree(cred_ptr.cast::<c_void>());
                    result
                };
                key_result
            }
            Err(err) if err.code() == ERROR_NOT_FOUND.to_hresult() => {
                let new_key = DbKey::generate()?;
                let mut blob = *new_key.as_bytes();
                let mut user_w = to_wide_null("VunLocalVault");
                let cred = CREDENTIALW {
                    Flags: CRED_FLAGS(0),
                    Type: CRED_TYPE_GENERIC,
                    TargetName: PWSTR::from_raw(target_w.as_mut_ptr()),
                    Comment: PWSTR::null(),
                    LastWritten: Default::default(),
                    CredentialBlobSize: 32,
                    CredentialBlob: blob.as_mut_ptr(),
                    Persist: CRED_PERSIST_LOCAL_MACHINE,
                    AttributeCount: 0,
                    Attributes: std::ptr::null_mut(),
                    TargetAlias: PWSTR::null(),
                    UserName: PWSTR::from_raw(user_w.as_mut_ptr()),
                };

                // SAFETY: `cred` references live, pinned local buffers for the duration of `CredWriteW`.
                let write_res = unsafe { CredWriteW(&cred, 0) };
                for byte in &mut blob {
                    // SAFETY: `byte` is a valid mutable reference in `blob`.
                    unsafe {
                        std::ptr::write_volatile(byte, 0);
                    }
                }
                write_res.map_err(|e| KeyStoreError::WindowsCred(e.to_string()))?;
                Ok(new_key)
            }
            Err(err) => Err(KeyStoreError::WindowsCred(err.to_string())),
        }
    }
}

#[cfg(not(windows))]
mod unix_dev_key {
    use std::fs::{self, OpenOptions};
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    use super::{DbKey, KeyStoreError};
    use crate::resolve_app_paths;

    pub(super) fn get_or_create(target_name: &str) -> Result<DbKey, KeyStoreError> {
        let sanitized: String = target_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dir = resolve_app_paths().local_data_dir;
        fs::create_dir_all(&dir)?;
        let key_path = dir.join(format!("{sanitized}.key"));
        if key_path.exists() {
            let bytes = fs::read(&key_path)?;
            if bytes.len() != 32 {
                return Err(KeyStoreError::InvalidKeyLength(bytes.len()));
            }
            let mut buf = [0u8; 32];
            buf.copy_from_slice(&bytes);
            return Ok(DbKey::from_bytes(buf));
        }

        let new_key = DbKey::generate()?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&key_path)?;
        file.write_all(new_key.as_bytes())?;
        file.sync_all()?;
        Ok(new_key)
    }
}
