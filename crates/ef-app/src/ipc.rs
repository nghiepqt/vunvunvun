use ef_win32::{HotkeyKind, LaunchAtLoginState, ThemeId};
use serde::{Deserialize, Serialize};

use crate::i18n::{I18nStrings, Locale};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRoute {
    Workspace,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyConflictNotice {
    pub kind: HotkeyKind,
    pub binding: String,
    pub os_error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivitySampleSummary {
    pub sampled_at: i64,
    pub process_name: String,
    pub idle_sec: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellStateSnapshot {
    pub route: WorkspaceRoute,
    pub locale: Locale,
    pub theme: ThemeId,
    pub is_packaged: bool,
    pub activity_paused: bool,
    pub session_locked: bool,
    pub launch_at_login: LaunchAtLoginState,
    pub capture_hotkey: String,
    pub nudge_hotkey: String,
    pub hotkey_conflicts: Vec<HotkeyConflictNotice>,
    pub stub_capture_count: u64,
    pub last_capture_preview: Option<String>,
    pub last_wake_latency_ms: Option<u64>,
    pub recent_samples: Vec<ActivitySampleSummary>,
    pub working_set_mb_tenths: u64,
    pub status_message: Option<String>,
    pub strings: I18nStrings,
}

/// Typed IPC requests sent from `ui/workspace` inside WebView2 to Rust Core (`ef-app`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcRequest {
    GetShellState,
    TriggerCapture,
    ShowTestNudge,
    ToggleActivityMonitoring,
    SetLaunchAtLogin { enabled: bool },
    RebindHotkey { kind: HotkeyKind, binding: String },
    SetTheme { theme: ThemeId },
    SetLocale { locale: Locale },
    Navigate { route: WorkspaceRoute },
}

/// Typed IPC response pushed to `ui/workspace`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcResponse {
    State { snapshot: Box<ShellStateSnapshot> },
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_requests_roundtrip_json() {
        let req = IpcRequest::RebindHotkey {
            kind: HotkeyKind::Capture,
            binding: "Ctrl+Alt+Space".to_string(),
        };
        let raw = serde_json::to_string(&req).expect("serialize");
        let parsed: IpcRequest = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(parsed, req);
    }
}
