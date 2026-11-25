use ef_win32::ShellCopy;
use serde::{Deserialize, Serialize};

const EN_JSON: &str = include_str!("../../../i18n/en.json");
const VI_JSON: &str = include_str!("../../../i18n/vi.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Locale {
    En,
    Vi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppCopy {
    pub name: String,
    pub tagline: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrayCopy {
    pub tooltip: String,
    pub open_workspace: String,
    pub capture: String,
    pub pause_activity: String,
    pub resume_activity: String,
    pub launch_at_login: String,
    pub settings: String,
    pub exit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureCopy {
    pub input_label: String,
    pub hint: String,
    pub saved_minimal: String,
    pub voice_recording: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NudgeCopy {
    pub title: String,
    pub short_break: String,
    pub not_now: String,
    pub shrink_step: String,
    pub focus_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyCopy {
    pub capture_label: String,
    pub nudge_label: String,
    pub conflict_banner: String,
    pub rebind_action: String,
    pub rebind_success: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlocksCopy {
    pub now: String,
    pub today: String,
    pub inbox: String,
    pub parked: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCopy {
    pub greeting: String,
    pub blocks: BlocksCopy,
    pub settings_title: String,
    pub activity_status_running: String,
    pub activity_status_paused: String,
    pub activity_status_locked: String,
    pub launch_at_login_label: String,
    pub launch_at_login_note: String,
    pub theme_label: String,
    pub locale_label: String,
    pub trigger_capture: String,
    pub test_nudge: String,
    pub recent_activity_label: String,
    pub degraded_local_only: String,
    pub disclaimer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsCopy {
    pub countdowns_note: String,
    pub elapsed_time_note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToastCopy {
    pub hotkey_conflict_title: String,
    pub hotkey_conflict_body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct I18nStrings {
    pub app: AppCopy,
    pub tray: TrayCopy,
    pub capture: CaptureCopy,
    pub nudge: NudgeCopy,
    pub hotkey: HotkeyCopy,
    pub workspace: WorkspaceCopy,
    pub settings: SettingsCopy,
    pub toast: ToastCopy,
}

impl I18nStrings {
    #[must_use]
    pub fn to_shell_copy(&self) -> ShellCopy {
        ShellCopy {
            app_name: self.app.name.clone(),
            tray_tooltip: self.tray.tooltip.clone(),
            tray_open_workspace: self.tray.open_workspace.clone(),
            tray_capture: self.tray.capture.clone(),
            tray_pause_activity: self.tray.pause_activity.clone(),
            tray_resume_activity: self.tray.resume_activity.clone(),
            tray_launch_at_login: self.tray.launch_at_login.clone(),
            tray_settings: self.tray.settings.clone(),
            tray_exit: self.tray.exit.clone(),
            capture_input_label: self.capture.input_label.clone(),
            capture_hint: self.capture.hint.clone(),
            nudge_title: self.nudge.title.clone(),
            nudge_short_break: self.nudge.short_break.clone(),
            nudge_shrink_step: self.nudge.shrink_step.clone(),
            nudge_not_now: self.nudge.not_now.clone(),
            nudge_focus_hint: self.nudge.focus_hint.clone(),
        }
    }
}

/// Embedded localization catalog loaded from `i18n/en.json` and `i18n/vi.json`.
#[derive(Debug, Clone)]
pub struct I18nCatalog {
    pub en: I18nStrings,
    pub vi: I18nStrings,
}

impl I18nCatalog {
    pub fn load_embedded() -> Result<Self, serde_json::Error> {
        let en: I18nStrings = serde_json::from_str(EN_JSON)?;
        let vi: I18nStrings = serde_json::from_str(VI_JSON)?;
        Ok(Self { en, vi })
    }

    #[must_use]
    pub fn for_locale(&self, locale: Locale) -> &I18nStrings {
        match locale {
            Locale::En => &self.en,
            Locale::Vi => &self.vi,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_en_and_vi_catalogs_deserialize_completely() {
        let cat = I18nCatalog::load_embedded().expect("load embedded i18n");
        assert_eq!(cat.en.app.name, "Vun");
        assert_eq!(cat.vi.app.name, "Vun");
        assert!(!cat.en.hotkey.conflict_banner.is_empty());
        assert!(!cat.vi.hotkey.conflict_banner.is_empty());
    }
}
