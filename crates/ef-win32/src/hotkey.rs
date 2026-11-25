use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Win32Error;

/// Identifies which global hotkey action is bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyKind {
    Capture = 1,
    FocusNudge = 2,
}

impl HotkeyKind {
    #[must_use]
    pub const fn win32_id(self) -> i32 {
        self as i32
    }

    #[must_use]
    pub const fn from_win32_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Capture),
            2 => Some(Self::FocusNudge),
            _ => None,
        }
    }
}

/// Portable representation of a global hotkey binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Virtual-key code (`VK_*`), e.g. `0x20` for Space or `0x4E` for `N`.
    pub vk_code: u32,
}

impl HotkeyBinding {
    /// Default F1 capture shortcut: `Ctrl+Shift+Space`.
    #[must_use]
    pub const fn default_capture() -> Self {
        Self {
            ctrl: true,
            alt: false,
            shift: true,
            win: false,
            vk_code: 0x20,
        }
    }

    /// Default F7 nudge focus-unlock shortcut: `Ctrl+Alt+Shift+N`.
    #[must_use]
    pub const fn default_focus_nudge() -> Self {
        Self {
            ctrl: true,
            alt: true,
            shift: true,
            win: false,
            vk_code: 0x4E,
        }
    }

    #[must_use]
    pub const fn has_modifier(&self) -> bool {
        self.ctrl || self.alt || self.shift || self.win
    }

    pub fn validate(&self) -> Result<(), Win32Error> {
        if !self.has_modifier() {
            return Err(Win32Error::InvalidArgument(
                "global hotkey requires at least one modifier key (Ctrl, Alt, Shift, or Win)"
                    .to_string(),
            ));
        }
        if vk_to_label(self.vk_code).is_none() {
            return Err(Win32Error::InvalidArgument(format!(
                "unsupported virtual key code 0x{:02X}",
                self.vk_code
            )));
        }
        Ok(())
    }
}

impl fmt::Display for HotkeyBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.win {
            parts.push("Win".to_string());
        }
        if let Some(key) = vk_to_label(self.vk_code) {
            parts.push(key.to_string());
        } else {
            parts.push(format!("0x{:02X}", self.vk_code));
        }
        f.write_str(&parts.join("+"))
    }
}

impl FromStr for HotkeyBinding {
    type Err = Win32Error;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let mut binding = Self {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            vk_code: 0,
        };
        let mut key_count = 0usize;
        for token in raw.split('+').map(str::trim).filter(|t| !t.is_empty()) {
            let upper = token.to_ascii_uppercase();
            match upper.as_str() {
                "CTRL" | "CONTROL" => binding.ctrl = true,
                "ALT" => binding.alt = true,
                "SHIFT" => binding.shift = true,
                "WIN" | "SUPER" => binding.win = true,
                other => {
                    key_count += 1;
                    binding.vk_code = label_to_vk(other).ok_or_else(|| {
                        Win32Error::InvalidArgument(format!("unknown key token `{token}`"))
                    })?;
                }
            }
        }
        if key_count != 1 {
            return Err(Win32Error::InvalidArgument(format!(
                "hotkey `{raw}` must specify exactly one primary key"
            )));
        }
        binding.validate()?;
        Ok(binding)
    }
}

fn vk_to_label(vk: u32) -> Option<&'static str> {
    match vk {
        0x20 => Some("Space"),
        0x41 => Some("A"),
        0x42 => Some("B"),
        0x43 => Some("C"),
        0x44 => Some("D"),
        0x45 => Some("E"),
        0x46 => Some("F"),
        0x47 => Some("G"),
        0x48 => Some("H"),
        0x49 => Some("I"),
        0x4A => Some("J"),
        0x4B => Some("K"),
        0x4C => Some("L"),
        0x4D => Some("M"),
        0x4E => Some("N"),
        0x4F => Some("O"),
        0x50 => Some("P"),
        0x51 => Some("Q"),
        0x52 => Some("R"),
        0x53 => Some("S"),
        0x54 => Some("T"),
        0x55 => Some("U"),
        0x56 => Some("V"),
        0x57 => Some("W"),
        0x58 => Some("X"),
        0x59 => Some("Y"),
        0x5A => Some("Z"),
        0x30 => Some("0"),
        0x31 => Some("1"),
        0x32 => Some("2"),
        0x33 => Some("3"),
        0x34 => Some("4"),
        0x35 => Some("5"),
        0x36 => Some("6"),
        0x37 => Some("7"),
        0x38 => Some("8"),
        0x39 => Some("9"),
        0x70 => Some("F1"),
        0x71 => Some("F2"),
        0x72 => Some("F3"),
        0x73 => Some("F4"),
        0x74 => Some("F5"),
        0x75 => Some("F6"),
        0x76 => Some("F7"),
        0x77 => Some("F8"),
        0x78 => Some("F9"),
        0x79 => Some("F10"),
        0x7A => Some("F11"),
        0x7B => Some("F12"),
        _ => None,
    }
}

fn label_to_vk(upper: &str) -> Option<u32> {
    match upper {
        "SPACE" => Some(0x20),
        "A" => Some(0x41),
        "B" => Some(0x42),
        "C" => Some(0x43),
        "D" => Some(0x44),
        "E" => Some(0x45),
        "F" => Some(0x46),
        "G" => Some(0x47),
        "H" => Some(0x48),
        "I" => Some(0x49),
        "J" => Some(0x4A),
        "K" => Some(0x4B),
        "L" => Some(0x4C),
        "M" => Some(0x4D),
        "N" => Some(0x4E),
        "O" => Some(0x4F),
        "P" => Some(0x50),
        "Q" => Some(0x51),
        "R" => Some(0x52),
        "S" => Some(0x53),
        "T" => Some(0x54),
        "U" => Some(0x55),
        "V" => Some(0x56),
        "W" => Some(0x57),
        "X" => Some(0x58),
        "Y" => Some(0x59),
        "Z" => Some(0x5A),
        "0" => Some(0x30),
        "1" => Some(0x31),
        "2" => Some(0x32),
        "3" => Some(0x33),
        "4" => Some(0x34),
        "5" => Some(0x35),
        "6" => Some(0x36),
        "7" => Some(0x37),
        "8" => Some(0x38),
        "9" => Some(0x39),
        "F1" => Some(0x70),
        "F2" => Some(0x71),
        "F3" => Some(0x72),
        "F4" => Some(0x73),
        "F5" => Some(0x74),
        "F6" => Some(0x75),
        "F7" => Some(0x76),
        "F8" => Some(0x77),
        "F9" => Some(0x78),
        "F10" => Some(0x79),
        "F11" => Some(0x7A),
        "F12" => Some(0x7B),
        _ => None,
    }
}

/// Explicit outcome of registering a system-wide hotkey (`PHASE-02` §3: never fail silently).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyRegistrationResult {
    Registered(HotkeyBinding),
    Conflict {
        binding: HotkeyBinding,
        os_error: String,
    },
    Failed {
        binding: HotkeyBinding,
        error: String,
    },
}

#[cfg(windows)]
pub(crate) mod windows_hotkey {
    use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
        UnregisterHotKey,
    };

    use super::{HotkeyBinding, HotkeyKind, HotkeyRegistrationResult};

    fn to_modifiers(binding: &HotkeyBinding) -> HOT_KEY_MODIFIERS {
        let mut mods = MOD_NOREPEAT;
        if binding.ctrl {
            mods |= MOD_CONTROL;
        }
        if binding.alt {
            mods |= MOD_ALT;
        }
        if binding.shift {
            mods |= MOD_SHIFT;
        }
        if binding.win {
            mods |= MOD_WIN;
        }
        mods
    }

    pub(crate) fn register(
        hwnd: HWND,
        kind: HotkeyKind,
        binding: &HotkeyBinding,
    ) -> HotkeyRegistrationResult {
        if let Err(err) = binding.validate() {
            return HotkeyRegistrationResult::Failed {
                binding: binding.clone(),
                error: err.to_string(),
            };
        }
        // SAFETY: `hwnd` is a valid message window owned by the shell thread.
        match unsafe {
            RegisterHotKey(
                hwnd,
                kind.win32_id(),
                to_modifiers(binding),
                binding.vk_code,
            )
        } {
            Ok(()) => HotkeyRegistrationResult::Registered(binding.clone()),
            Err(err) if err.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() => {
                HotkeyRegistrationResult::Conflict {
                    binding: binding.clone(),
                    os_error: err.to_string(),
                }
            }
            Err(err) => HotkeyRegistrationResult::Failed {
                binding: binding.clone(),
                error: err.to_string(),
            },
        }
    }

    pub(crate) fn unregister(hwnd: HWND, kind: HotkeyKind) {
        // SAFETY: Best-effort unregistration on rebind or shutdown.
        let _ = unsafe { UnregisterHotKey(hwnd, kind.win32_id()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkeys_format_and_roundtrip() {
        let cap = HotkeyBinding::default_capture();
        assert_eq!(cap.to_string(), "Ctrl+Shift+Space");
        assert_eq!(
            "ctrl + shift + space"
                .parse::<HotkeyBinding>()
                .expect("parse capture"),
            cap
        );

        let nudge = HotkeyBinding::default_focus_nudge();
        assert_eq!(nudge.to_string(), "Ctrl+Alt+Shift+N");
        assert_eq!(
            "Ctrl+Alt+Shift+N"
                .parse::<HotkeyBinding>()
                .expect("parse nudge"),
            nudge
        );
    }

    #[test]
    fn hotkey_without_modifier_or_primary_key_is_rejected() {
        assert!("Space".parse::<HotkeyBinding>().is_err());
        assert!("Ctrl+Shift".parse::<HotkeyBinding>().is_err());
        assert!("Ctrl+UnknownKey".parse::<HotkeyBinding>().is_err());
    }
}
