use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Win32Error;

const PALETTE_JSON: &str = include_str!("../../../ui/tokens/palette.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeId {
    StandardLight,
    StandardDark,
    MutedLight,
    MutedDark,
}

impl ThemeId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StandardLight => "standard-light",
            Self::StandardDark => "standard-dark",
            Self::MutedLight => "muted-light",
            Self::MutedDark => "muted-dark",
        }
    }

    #[must_use]
    pub const fn is_dark(self) -> bool {
        matches!(self, Self::StandardDark | Self::MutedDark)
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "standard-light" => Some(Self::StandardLight),
            "standard-dark" => Some(Self::StandardDark),
            "muted-light" => Some(Self::MutedLight),
            "muted-dark" => Some(Self::MutedDark),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    /// Win32 `COLORREF` (`0x00BBGGRR`).
    #[must_use]
    pub const fn to_colorref(self) -> u32 {
        (self.r as u32) | ((self.g as u32) << 8) | ((self.b as u32) << 16)
    }

    pub fn from_hex(raw: &str) -> Result<Self, Win32Error> {
        let trimmed = raw.trim().trim_start_matches('#');
        let hex = if trimmed.len() == 8 {
            &trimmed[2..]
        } else {
            trimmed
        };
        if hex.len() != 6 {
            return Err(Win32Error::InvalidArgument(format!(
                "invalid hex color `{raw}`"
            )));
        }
        let r = u8::from_str_radix(&hex[0..2], 16)
            .map_err(|e| Win32Error::InvalidArgument(e.to_string()))?;
        let g = u8::from_str_radix(&hex[2..4], 16)
            .map_err(|e| Win32Error::InvalidArgument(e.to_string()))?;
        let b = u8::from_str_radix(&hex[4..6], 16)
            .map_err(|e| Win32Error::InvalidArgument(e.to_string()))?;
        Ok(Self { r, g, b })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayColors {
    pub bg_canvas: RgbColor,
    pub bg_surface: RgbColor,
    pub text_primary: RgbColor,
    pub text_muted: RgbColor,
    pub accent_primary: RgbColor,
    pub text_on_accent: RgbColor,
    pub border_subtle: RgbColor,
}

#[derive(Deserialize)]
struct RawPaletteFile {
    themes: BTreeMap<String, BTreeMap<String, String>>,
}

/// Loads color tokens for `theme` from the embedded `ui/tokens/palette.json`.
pub fn load_theme_colors(theme: ThemeId) -> Result<OverlayColors, Win32Error> {
    let parsed: RawPaletteFile = serde_json::from_str(PALETTE_JSON)
        .map_err(|e| Win32Error::InvalidArgument(format!("invalid palette.json: {e}")))?;
    let map = parsed.themes.get(theme.as_str()).ok_or_else(|| {
        Win32Error::InvalidArgument(format!("missing theme `{}`", theme.as_str()))
    })?;
    let pick = |key: &str| -> Result<RgbColor, Win32Error> {
        let val = map.get(key).ok_or_else(|| {
            Win32Error::InvalidArgument(format!("missing token `{key}` in `{}`", theme.as_str()))
        })?;
        RgbColor::from_hex(val)
    };
    Ok(OverlayColors {
        bg_canvas: pick("bg_canvas")?,
        bg_surface: pick("bg_surface")?,
        text_primary: pick("text_primary")?,
        text_muted: pick("text_muted")?,
        accent_primary: pick("accent_primary")?,
        text_on_accent: pick("text_on_accent")?,
        border_subtle: pick("border_subtle")?,
    })
}

/// Returns `true` when Windows High Contrast mode is active.
#[must_use]
pub fn is_high_contrast_enabled() -> bool {
    #[cfg(windows)]
    {
        windows_impl::is_high_contrast()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Best-effort query of Windows system dark mode preference.
#[must_use]
pub fn is_system_dark_mode() -> bool {
    #[cfg(windows)]
    {
        windows_impl::is_dark_mode()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
pub(crate) mod windows_impl {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
        RegQueryValueExW,
    };
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    use windows::core::PCWSTR;

    pub(super) fn is_high_contrast() -> bool {
        let mut hc = HIGHCONTRASTW {
            cbSize: u32::try_from(std::mem::size_of::<HIGHCONTRASTW>()).unwrap_or(16),
            ..Default::default()
        };
        // SAFETY: `hc` has valid `cbSize` and receives `HIGHCONTRASTW` flags.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                hc.cbSize,
                Some((&raw mut hc).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        ok.is_ok() && (hc.dwFlags & HCF_HIGHCONTRASTON) == HCF_HIGHCONTRASTON
    }

    pub(super) fn is_dark_mode() -> bool {
        let subkey: Vec<u16> =
            "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
                .encode_utf16()
                .collect();
        let value_name: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
        let mut hkey = HKEY::default();
        // SAFETY: Querying standard HKCU Personalize key.
        let open_err = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR::from_raw(subkey.as_ptr()),
                0,
                KEY_READ,
                &mut hkey,
            )
        };
        if open_err.0 != 0 {
            return false;
        }
        let mut data: u32 = 1;
        let mut size: u32 = 4;
        let mut val_type = REG_VALUE_TYPE::default();
        // SAFETY: `hkey` is open and `data` holds a 4-byte `REG_DWORD`.
        let q_err = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR::from_raw(value_name.as_ptr()),
                None,
                Some(&mut val_type),
                Some((&raw mut data).cast()),
                Some(&mut size),
            )
        };
        // SAFETY: Close opened registry handle.
        let _ = unsafe { RegCloseKey(hkey) };
        q_err.0 == 0 && data == 0
    }

    /// Applies rounded corners and optional Windows 11 Acrylic backdrop best-effort.
    /// Gracefully ignores unsupported attributes on Windows 10 22H2.
    pub(crate) fn apply_dwm_backdrop(hwnd: HWND, dark: bool, transient_acrylic: bool) {
        let dark_val: i32 = i32::from(dark);
        // SAFETY: `hwnd` is a valid top-level window and `dark_val` is a 4-byte BOOL.
        let _ = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&raw const dark_val).cast(),
                4,
            )
        };
        let corner = DWMWCP_ROUND;
        // SAFETY: `corner` is a valid `DWM_WINDOW_CORNER_PREFERENCE`.
        let _ = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&raw const corner).cast(),
                4,
            )
        };
        if transient_acrylic {
            let backdrop = DWMSBT_TRANSIENTWINDOW;
            // SAFETY: `backdrop` is a valid `DWM_SYSTEMBACKDROP_TYPE`; Win10 safely returns error which we ignore.
            let _ = unsafe {
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_SYSTEMBACKDROP_TYPE,
                    (&raw const backdrop).cast(),
                    4,
                )
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_themes_load_from_palette_json() {
        for theme in [
            ThemeId::StandardLight,
            ThemeId::StandardDark,
            ThemeId::MutedLight,
            ThemeId::MutedDark,
        ] {
            let colors = load_theme_colors(theme).expect("load theme");
            assert_ne!(colors.bg_surface, colors.text_primary);
        }
    }
}
