use ef_storage::{ActivitySampleRepository, Database, current_unix_ms};
use ef_win32::{
    ActivityEvent, ActivityMonitor, AppPaths, HotkeyBinding, HotkeyKind, HotkeyRegistrationResult,
    LaunchAtLoginState, NativeShell, SamplingPolicy, ShellCommand, ShellEvent, ThemeId,
    ToastResult, current_foreground_process_name, current_idle_seconds, get_launch_at_login,
    process_diagnostics, set_launch_at_login, show_toast_best_effort,
};

use crate::i18n::{I18nCatalog, Locale};
use crate::ipc::{
    ActivitySampleSummary, HotkeyConflictNotice, IpcRequest, IpcResponse, ShellStateSnapshot,
    WorkspaceRoute,
};
use crate::workspace_host::{WorkspaceEvent, WorkspaceHost};

pub struct ShellController {
    db: Database,
    activity_repo: ActivitySampleRepository,
    paths: AppPaths,
    catalog: I18nCatalog,
    shell: NativeShell,
    workspace: WorkspaceHost,
    activity: ActivityMonitor,
    route: WorkspaceRoute,
    locale: Locale,
    theme: ThemeId,
    activity_paused: bool,
    session_locked: bool,
    launch_at_login: LaunchAtLoginState,
    capture_hotkey: HotkeyBinding,
    nudge_hotkey: HotkeyBinding,
    hotkey_conflicts: Vec<HotkeyConflictNotice>,
    stub_capture_count: u64,
    last_capture_preview: Option<String>,
    last_wake_latency_ms: Option<u64>,
    last_foreground_process: String,
    status_message: Option<String>,
    workspace_open: bool,
    spike_w2_probe: bool,
}

impl ShellController {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Database,
        paths: AppPaths,
        catalog: I18nCatalog,
        shell: NativeShell,
        workspace: WorkspaceHost,
        activity: ActivityMonitor,
        locale: Locale,
        theme: ThemeId,
        spike_w2_probe: bool,
    ) -> Self {
        let activity_repo = ActivitySampleRepository::new(db.clone());
        let launch_at_login = get_launch_at_login(paths.is_packaged);
        let initial_proc = current_foreground_process_name()
            .map(|(_, _, name)| name)
            .unwrap_or_else(|| "unknown".to_string());

        Self {
            db,
            activity_repo,
            paths,
            catalog,
            shell,
            workspace,
            activity,
            route: WorkspaceRoute::Workspace,
            locale,
            theme,
            activity_paused: false,
            session_locked: false,
            launch_at_login,
            capture_hotkey: HotkeyBinding::default_capture(),
            nudge_hotkey: HotkeyBinding::default_focus_nudge(),
            hotkey_conflicts: Vec::new(),
            stub_capture_count: 0,
            last_capture_preview: None,
            last_wake_latency_ms: None,
            last_foreground_process: initial_proc,
            status_message: None,
            workspace_open: false,
            spike_w2_probe,
        }
    }

    #[must_use]
    pub const fn sampling_policy(&self) -> SamplingPolicy {
        SamplingPolicy {
            paused: self.activity_paused,
            session_locked: self.session_locked,
        }
    }

    pub fn open_workspace(&mut self, route: WorkspaceRoute) {
        self.route = route;
        self.workspace_open = true;
        if self.spike_w2_probe {
            let diag = process_diagnostics();
            tracing::info!(
                handles = diag.handle_count,
                working_set_mb = format!("{:.2}", diag.working_set_mb()),
                "Spike W2 probe: before opening ephemeral WebView2"
            );
        }
        self.workspace.open(route);
    }

    pub async fn handle_shell_event(&mut self, ev: ShellEvent) -> bool {
        match ev {
            ShellEvent::WakeupFromSecondInstance | ShellEvent::TrayOpenWorkspace => {
                self.open_workspace(WorkspaceRoute::Workspace);
                self.push_state().await;
            }
            ShellEvent::TrayOpenSettings => {
                self.open_workspace(WorkspaceRoute::Settings);
                self.push_state().await;
            }
            ShellEvent::TrayQuickCapture => {
                self.status_message = None;
            }
            ShellEvent::TrayToggleActivityMonitoring => {
                self.toggle_activity_monitoring();
                self.push_state().await;
            }
            ShellEvent::TrayToggleLaunchAtLogin => {
                let target = !self.launch_at_login.is_enabled();
                self.apply_launch_at_login(target);
                self.push_state().await;
            }
            ShellEvent::TrayExit => {
                return true;
            }
            ShellEvent::HotkeyPressed(kind) => {
                tracing::debug!(?kind, "Global hotkey pressed");
            }
            ShellEvent::HotkeyRegistration { kind, result } => {
                self.handle_hotkey_registration(kind, result);
                self.push_state().await;
            }
            ShellEvent::CaptureShown { wake_latency_ms } => {
                self.last_wake_latency_ms = Some(wake_latency_ms);
                if self.spike_w2_probe {
                    tracing::info!(
                        wake_latency_ms,
                        within_budget = wake_latency_ms < 150,
                        "Spike W2 probe: F1 capture wake latency"
                    );
                }
            }
            ShellEvent::CaptureSubmitted { text } => {
                self.stub_capture_count += 1;
                let preview: String = text.chars().take(120).collect();
                tracing::info!(
                    stub_capture_count = self.stub_capture_count,
                    char_len = text.chars().count(),
                    "Phase 02 capture stub event received (not persisted to inbox yet)"
                );
                self.last_capture_preview = Some(preview);
                self.status_message = Some(
                    self.catalog
                        .for_locale(self.locale)
                        .capture
                        .saved_minimal
                        .clone(),
                );
                self.push_state().await;
            }
            ShellEvent::CaptureDismissed => {}
            ShellEvent::NudgeShown => {
                if self.spike_w2_probe {
                    tracing::info!(
                        active_process = %self.last_foreground_process,
                        "Spike W2 probe: F7 nudge shown without stealing focus"
                    );
                }
            }
            ShellEvent::NudgeFocusUnlocked => {
                tracing::debug!("F7 nudge focus unlocked via accessibility shortcut");
            }
            ShellEvent::NudgeDismissed { action } => {
                tracing::debug!(%action, "F7 nudge dismissed");
            }
        }
        false
    }

    pub async fn handle_activity_event(&mut self, ev: ActivityEvent) {
        match ev {
            ActivityEvent::ForegroundChanged {
                hwnd_raw,
                pid: _,
                process_name,
                timestamp_ms: _,
            } => {
                let resolved = self
                    .activity
                    .resolve_process_with_uia_fallback(hwnd_raw, process_name)
                    .await;
                self.last_foreground_process = resolved;
            }
            ActivityEvent::SessionLockChanged {
                locked,
                timestamp_ms: _,
            } => {
                self.session_locked = locked;
                tracing::info!(locked, "Session lock state changed");
                self.push_state().await;
            }
        }
    }

    pub async fn handle_workspace_event(&mut self, ev: WorkspaceEvent) {
        match ev {
            WorkspaceEvent::Opened => {
                self.workspace_open = true;
                self.push_state().await;
                if self.spike_w2_probe {
                    let diag = process_diagnostics();
                    tracing::info!(
                        handles = diag.handle_count,
                        working_set_mb = format!("{:.2}", diag.working_set_mb()),
                        "Spike W2 probe: WebView2 opened"
                    );
                }
            }
            WorkspaceEvent::Closed => {
                self.workspace_open = false;
                if self.spike_w2_probe {
                    let diag = process_diagnostics();
                    tracing::info!(
                        handles = diag.handle_count,
                        working_set_mb = format!("{:.2}", diag.working_set_mb()),
                        "Spike W2 probe: WebView2 closed & destroyed"
                    );
                }
            }
            WorkspaceEvent::Ipc(req) => {
                self.handle_ipc_request(req).await;
            }
            WorkspaceEvent::IpcParseError(msg) => {
                self.workspace
                    .push_response(IpcResponse::Error { message: msg });
            }
            WorkspaceEvent::HostError(msg) => {
                tracing::error!(error = %msg, "Workspace host error");
                self.status_message = Some(msg);
            }
        }
    }

    pub async fn handle_ipc_request(&mut self, req: IpcRequest) {
        match req {
            IpcRequest::GetShellState => {
                self.push_state().await;
            }
            IpcRequest::TriggerCapture => {
                let _ = self.shell.send(ShellCommand::ShowCapture);
            }
            IpcRequest::ShowTestNudge => {
                let msg = self
                    .catalog
                    .for_locale(self.locale)
                    .nudge
                    .short_break
                    .clone();
                let _ = self
                    .shell
                    .send(ShellCommand::ShowNudge { message: Some(msg) });
            }
            IpcRequest::ToggleActivityMonitoring => {
                self.toggle_activity_monitoring();
                self.push_state().await;
            }
            IpcRequest::SetLaunchAtLogin { enabled } => {
                self.apply_launch_at_login(enabled);
                self.push_state().await;
            }
            IpcRequest::RebindHotkey { kind, binding } => match binding.parse::<HotkeyBinding>() {
                Ok(parsed) => {
                    let _ = self.shell.send(ShellCommand::RebindHotkey {
                        kind,
                        binding: parsed,
                    });
                }
                Err(err) => {
                    self.workspace.push_response(IpcResponse::Error {
                        message: err.to_string(),
                    });
                }
            },
            IpcRequest::SetTheme { theme } => {
                self.theme = theme;
                let _ = self.shell.send(ShellCommand::SetTheme(theme));
                self.push_state().await;
            }
            IpcRequest::SetLocale { locale } => {
                self.locale = locale;
                let copy = self.catalog.for_locale(locale).to_shell_copy();
                let _ = self.shell.send(ShellCommand::UpdateCopy(Box::new(copy)));
                self.push_state().await;
            }
            IpcRequest::Navigate { route } => {
                self.route = route;
                self.push_state().await;
            }
        }
    }

    /// Samples foreground activity every 15 seconds into `activity_sample`,
    /// suppressing writes immediately when paused or session-locked (`PHASE-02` §8).
    pub async fn record_activity_tick(&self) {
        if !self.sampling_policy().should_record() {
            return;
        }
        let idle_sec = current_idle_seconds().max(0);
        let now_ms = current_unix_ms();
        let proc_name = self.last_foreground_process.clone();
        if let Err(err) = self
            .activity_repo
            .record(now_ms, proc_name, idle_sec, false)
            .await
        {
            tracing::warn!(error = %err, "Failed to record activity sample");
        }
    }

    pub async fn build_snapshot(&self) -> ShellStateSnapshot {
        let recent_samples = self
            .activity_repo
            .list_recent(5)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|s| ActivitySampleSummary {
                sampled_at: s.sampled_at,
                process_name: s.process_name,
                idle_sec: s.idle_sec,
            })
            .collect();
        let diag = process_diagnostics();
        let working_set_mb_tenths = (diag.working_set_mb() * 10.0).round().max(0.0) as u64;

        ShellStateSnapshot {
            route: self.route,
            locale: self.locale,
            theme: self.theme,
            is_packaged: self.paths.is_packaged,
            activity_paused: self.activity_paused,
            session_locked: self.session_locked,
            launch_at_login: self.launch_at_login,
            capture_hotkey: self.capture_hotkey.to_string(),
            nudge_hotkey: self.nudge_hotkey.to_string(),
            hotkey_conflicts: self.hotkey_conflicts.clone(),
            stub_capture_count: self.stub_capture_count,
            last_capture_preview: self.last_capture_preview.clone(),
            last_wake_latency_ms: self.last_wake_latency_ms,
            recent_samples,
            working_set_mb_tenths,
            status_message: self.status_message.clone(),
            strings: self.catalog.for_locale(self.locale).clone(),
        }
    }

    pub async fn shutdown(self) {
        self.workspace.shutdown();
        self.shell.shutdown();
        self.activity.shutdown();
        drop(self.db);
    }

    fn toggle_activity_monitoring(&mut self) {
        self.activity_paused = !self.activity_paused;
        let _ = self
            .shell
            .send(ShellCommand::SetActivityPaused(self.activity_paused));
        tracing::info!(paused = self.activity_paused, "Activity monitoring toggled");
    }

    fn apply_launch_at_login(&mut self, enabled: bool) {
        match set_launch_at_login(self.paths.is_packaged, enabled) {
            Ok(state) => {
                self.launch_at_login = state;
                let _ = self
                    .shell
                    .send(ShellCommand::SetLaunchAtLogin(state.is_enabled()));
            }
            Err(err) => {
                self.status_message = Some(err.to_string());
            }
        }
    }

    fn handle_hotkey_registration(&mut self, kind: HotkeyKind, result: HotkeyRegistrationResult) {
        self.hotkey_conflicts.retain(|c| c.kind != kind);
        match result {
            HotkeyRegistrationResult::Registered(binding) => {
                match kind {
                    HotkeyKind::Capture => self.capture_hotkey = binding,
                    HotkeyKind::FocusNudge => self.nudge_hotkey = binding,
                }
                self.status_message = Some(
                    self.catalog
                        .for_locale(self.locale)
                        .hotkey
                        .rebind_success
                        .clone(),
                );
            }
            HotkeyRegistrationResult::Conflict { binding, os_error } => {
                let binding_str = binding.to_string();
                tracing::warn!(
                    ?kind,
                    binding = %binding_str,
                    %os_error,
                    "Global hotkey already owned by another application"
                );
                let strings = self.catalog.for_locale(self.locale);
                let toast_body = strings
                    .toast
                    .hotkey_conflict_body
                    .replace("{binding}", &binding_str);
                let toast_res = show_toast_best_effort(
                    &strings.toast.hotkey_conflict_title,
                    &toast_body,
                    self.paths.is_packaged,
                );
                if let ToastResult::Unavailable(reason) = toast_res {
                    tracing::debug!(%reason, "Toast notification skipped; surfacing in-app banner");
                }
                self.hotkey_conflicts.push(HotkeyConflictNotice {
                    kind,
                    binding: binding_str,
                    os_error,
                });
            }
            HotkeyRegistrationResult::Failed { binding, error } => {
                self.hotkey_conflicts.push(HotkeyConflictNotice {
                    kind,
                    binding: binding.to_string(),
                    os_error: error,
                });
            }
        }
    }

    async fn push_state(&self) {
        if !self.workspace_open {
            return;
        }
        let snapshot = self.build_snapshot().await;
        self.workspace.push_response(IpcResponse::State {
            snapshot: Box::new(snapshot),
        });
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::workspace_host::WORKSPACE_EVENT_CHANNEL_CAPACITY;
    use ef_win32::{
        ACTIVITY_EVENT_CHANNEL_CAPACITY, NativeShellConfig, SHELL_EVENT_CHANNEL_CAPACITY,
        resolve_app_paths,
    };

    async fn make_test_controller() -> ShellController {
        let db = Database::open_in_memory().expect("open in-memory db");
        let paths = resolve_app_paths();
        let catalog = I18nCatalog::load_embedded().expect("load embedded i18n");
        let copy = catalog.en.to_shell_copy();
        let (shell_tx, _shell_rx) = mpsc::channel(SHELL_EVENT_CHANNEL_CAPACITY);
        let shell = NativeShell::start(
            NativeShellConfig {
                theme: ThemeId::StandardLight,
                copy,
                capture_hotkey: HotkeyBinding::default_capture(),
                nudge_hotkey: HotkeyBinding::default_focus_nudge(),
                activity_paused: false,
                launch_at_login: false,
                wakeup_window_class: "VunTestShell".to_string(),
                wakeup_message_name: "VunTestWakeup".to_string(),
            },
            shell_tx,
        )
        .expect("start shell");
        let (ws_tx, _ws_rx) = mpsc::channel(WORKSPACE_EVENT_CHANNEL_CAPACITY);
        let workspace = WorkspaceHost::new(paths.local_data_dir.join("webview2-test"), ws_tx);
        let (act_tx, _act_rx) = mpsc::channel(ACTIVITY_EVENT_CHANNEL_CAPACITY);
        let activity = ActivityMonitor::start(act_tx).expect("start activity");

        ShellController::new(
            db,
            paths,
            catalog,
            shell,
            workspace,
            activity,
            Locale::En,
            ThemeId::StandardLight,
            false,
        )
    }

    #[tokio::test]
    async fn activity_tick_respects_pause_and_session_lock_and_records_non_negative_idle() {
        let mut ctrl = make_test_controller().await;
        ctrl.handle_activity_event(ActivityEvent::ForegroundChanged {
            hwnd_raw: 0,
            pid: 42,
            process_name: "code.exe".to_string(),
            timestamp_ms: 1000,
        })
        .await;

        ctrl.record_activity_tick().await;
        let first = ctrl.activity_repo.list_recent(10).await.expect("list");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].process_name, "code.exe");
        assert!(first[0].idle_sec >= 0);
        assert!(!first[0].session_locked);

        // Pause from tray halts sampling immediately.
        ctrl.handle_shell_event(ShellEvent::TrayToggleActivityMonitoring)
            .await;
        ctrl.record_activity_tick().await;
        assert_eq!(
            ctrl.activity_repo
                .list_recent(10)
                .await
                .expect("list")
                .len(),
            1
        );

        // Resume, then lock session -> still suppressed while locked.
        ctrl.handle_shell_event(ShellEvent::TrayToggleActivityMonitoring)
            .await;
        ctrl.handle_activity_event(ActivityEvent::SessionLockChanged {
            locked: true,
            timestamp_ms: 2000,
        })
        .await;
        ctrl.record_activity_tick().await;
        assert_eq!(
            ctrl.activity_repo
                .list_recent(10)
                .await
                .expect("list")
                .len(),
            1
        );

        ctrl.shutdown().await;
    }

    #[tokio::test]
    async fn hotkey_conflict_surfaces_in_state_snapshot_without_failing_silently() {
        let mut ctrl = make_test_controller().await;
        ctrl.handle_shell_event(ShellEvent::HotkeyRegistration {
            kind: HotkeyKind::Capture,
            result: HotkeyRegistrationResult::Conflict {
                binding: HotkeyBinding::default_capture(),
                os_error: "0x80070581".to_string(),
            },
        })
        .await;

        let snap = ctrl.build_snapshot().await;
        assert_eq!(snap.hotkey_conflicts.len(), 1);
        assert_eq!(snap.hotkey_conflicts[0].binding, "Ctrl+Shift+Space");

        ctrl.handle_shell_event(ShellEvent::CaptureSubmitted {
            text: "buy milk".to_string(),
        })
        .await;
        let snap2 = ctrl.build_snapshot().await;
        assert_eq!(snap2.stub_capture_count, 1);
        assert_eq!(snap2.last_capture_preview.as_deref(), Some("buy milk"));

        ctrl.shutdown().await;
    }

    #[tokio::test]
    async fn workspace_ipc_events_update_theme_locale_and_close_cleanly() {
        let mut ctrl = make_test_controller().await;
        ctrl.handle_workspace_event(WorkspaceEvent::Opened).await;
        ctrl.handle_workspace_event(WorkspaceEvent::Ipc(IpcRequest::SetTheme {
            theme: ThemeId::MutedDark,
        }))
        .await;
        ctrl.handle_workspace_event(WorkspaceEvent::Ipc(IpcRequest::SetLocale {
            locale: Locale::Vi,
        }))
        .await;
        ctrl.handle_workspace_event(WorkspaceEvent::IpcParseError("bad json".to_string()))
            .await;
        ctrl.handle_workspace_event(WorkspaceEvent::HostError("transient".to_string()))
            .await;
        ctrl.handle_workspace_event(WorkspaceEvent::Closed).await;

        let snap = ctrl.build_snapshot().await;
        assert_eq!(snap.theme, ThemeId::MutedDark);
        assert_eq!(snap.locale, Locale::Vi);
        assert_eq!(snap.status_message.as_deref(), Some("transient"));

        ctrl.shutdown().await;
    }
}
