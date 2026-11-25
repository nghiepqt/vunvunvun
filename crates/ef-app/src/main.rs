#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod controller;
mod i18n;
mod ipc;
mod workspace_host;

use std::fs::OpenOptions;
use std::io::Write as _;
use std::time::{Duration, Instant};

use ef_ai::Router;
use ef_core::UserProfile;
use ef_ml::EMBEDDING_DIM;
use ef_oplog::EntityType;
use ef_prompts::STRUCTURE_PROMPT_V1;
use ef_protocol::FeatureId;
use ef_sidecar::SidecarSupervisor;
use ef_storage::Database;
use ef_win32::{
    ACTIVITY_EVENT_CHANNEL_CAPACITY, ActivityMonitor, DEFAULT_SINGLE_INSTANCE_MUTEX,
    DEFAULT_WAKEUP_MESSAGE_NAME, DEFAULT_WAKEUP_WINDOW_CLASS, HotkeyBinding, JobObject,
    NativeShell, NativeShellConfig, SHELL_EVENT_CHANNEL_CAPACITY, SingleInstanceGuard,
    SingleInstanceOutcome, ThemeId, current_unix_ms, get_or_create_db_key, is_system_dark_mode,
    process_diagnostics, resolve_app_paths,
};
use serde::Serialize;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::controller::ShellController;
use crate::i18n::{I18nCatalog, Locale};
use crate::ipc::WorkspaceRoute;
use crate::workspace_host::{WORKSPACE_EVENT_CHANNEL_CAPACITY, WorkspaceHost};

const DB_KEY_TARGET: &str = "ThanhNghiepQuan.Vun.SqlCipherKey";
const ACTIVITY_SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliOptions {
    open_workspace_on_start: bool,
    spike_w1_soak: bool,
    soak_duration_secs: u64,
    soak_report_interval_secs: u64,
    spike_w2_probe: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            open_workspace_on_start: false,
            spike_w1_soak: false,
            soak_duration_secs: 86_400,
            soak_report_interval_secs: 300,
            spike_w2_probe: false,
        }
    }
}

fn parse_cli_args() -> CliOptions {
    let mut opts = CliOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open-workspace" => opts.open_workspace_on_start = true,
            "--minimized" => opts.open_workspace_on_start = false,
            "--spike-w1-soak" => opts.spike_w1_soak = true,
            "--spike-w2-probe" => {
                opts.spike_w2_probe = true;
                opts.open_workspace_on_start = true;
            }
            "--duration-secs" => {
                if let Some(val) = args.next()
                    && let Ok(secs) = val.parse::<u64>()
                {
                    opts.soak_duration_secs = secs;
                }
            }
            "--report-interval-secs" => {
                if let Some(val) = args.next()
                    && let Ok(secs) = val.parse::<u64>()
                {
                    opts.soak_report_interval_secs = secs.max(1);
                }
            }
            _ => {}
        }
    }
    opts
}

#[derive(Serialize)]
struct SoakReportLine {
    timestamp_ms: i64,
    elapsed_secs: u64,
    handle_count: u32,
    working_set_bytes: usize,
    events_received: u64,
    events_dropped: u64,
    job_object_ok: bool,
    idle_sec: i64,
}

async fn run_spike_w1_soak(opts: &CliOptions) -> Result<(), String> {
    let paths = resolve_app_paths();
    std::fs::create_dir_all(&paths.logs_dir).map_err(|e| e.to_string())?;
    let report_path = paths.logs_dir.join("spike-w1-soak.jsonl");

    let job_object_ok = JobObject::new(Some(512 * 1024 * 1024)).is_ok();
    let (act_tx, mut act_rx) = mpsc::channel(ACTIVITY_EVENT_CHANNEL_CAPACITY);
    let monitor = ActivityMonitor::start(act_tx).map_err(|e| e.to_string())?;

    let started = Instant::now();
    let deadline = Duration::from_secs(opts.soak_duration_secs);
    let mut report_tick =
        tokio::time::interval(Duration::from_secs(opts.soak_report_interval_secs));

    tracing::info!(
        duration_secs = opts.soak_duration_secs,
        report_interval_secs = opts.soak_report_interval_secs,
        report_path = %report_path.display(),
        job_object_ok,
        "Starting Spike W1 message-pump soak test"
    );

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Spike W1 soak interrupted via Ctrl+C");
                break;
            }
            Some(ev) = act_rx.recv() => {
                if let ef_win32::ActivityEvent::ForegroundChanged { hwnd_raw, process_name, .. } = ev {
                    let _ = monitor.resolve_process_with_uia_fallback(hwnd_raw, process_name).await;
                }
            }
            _ = report_tick.tick() => {
                let elapsed_secs = started.elapsed().as_secs();
                let stats = monitor.stats();
                let diag = process_diagnostics();
                let idle_sec = ef_win32::current_idle_seconds();
                let line = SoakReportLine {
                    timestamp_ms: current_unix_ms(),
                    elapsed_secs,
                    handle_count: diag.handle_count,
                    working_set_bytes: diag.working_set_bytes,
                    events_received: stats.events_received,
                    events_dropped: stats.events_dropped,
                    job_object_ok,
                    idle_sec,
                };
                if let Ok(json) = serde_json::to_string(&line)
                    && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&report_path)
                {
                    let _ = writeln!(file, "{json}");
                }
                tracing::info!(
                    elapsed_secs,
                    handles = diag.handle_count,
                    working_set_mb = format!("{:.2}", diag.working_set_mb()),
                    events_received = stats.events_received,
                    events_dropped = stats.events_dropped,
                    idle_sec,
                    "Spike W1 soak checkpoint"
                );
                if started.elapsed() >= deadline {
                    break;
                }
            }
        }
    }

    monitor.shutdown();
    Ok(())
}

#[tokio::main]
async fn main() {
    let opts = parse_cli_args();
    let paths = resolve_app_paths();
    if let Err(err) = std::fs::create_dir_all(&paths.logs_dir) {
        eprintln!("failed to create logs dir: {err}");
    }

    let file_appender = tracing_appender::rolling::daily(&paths.logs_dir, "vun.log");
    let (non_blocking, _log_guard) = tracing_appender::non_blocking(file_appender);
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
        .init();

    if opts.spike_w1_soak {
        if let Err(err) = run_spike_w1_soak(&opts).await {
            tracing::error!(error = %err, "Spike W1 soak failed");
            std::process::exit(1);
        }
        return;
    }

    let single_instance = match SingleInstanceGuard::acquire(
        DEFAULT_SINGLE_INSTANCE_MUTEX,
        DEFAULT_WAKEUP_WINDOW_CLASS,
        DEFAULT_WAKEUP_MESSAGE_NAME,
    ) {
        Ok(SingleInstanceOutcome::Primary(guard)) => guard,
        Ok(SingleInstanceOutcome::AlreadyRunning { wakeup_sent }) => {
            tracing::info!(wakeup_sent, "Another Vun instance is active; exiting");
            return;
        }
        Err(err) => {
            tracing::error!(error = %err, "Failed to acquire single-instance mutex");
            std::process::exit(1);
        }
    };

    let db_key = match get_or_create_db_key(DB_KEY_TARGET) {
        Ok(k) => k,
        Err(err) => {
            tracing::error!(error = %err, "Failed to initialize SQLCipher key");
            std::process::exit(1);
        }
    };
    let db_path = paths.local_data_dir.join("vun.db");
    let db = match Database::open_encrypted(&db_path, &db_key) {
        Ok(db) => db,
        Err(err) => {
            tracing::error!(error = %err, db_path = %db_path.display(), "Failed to open SQLCipher database");
            std::process::exit(1);
        }
    };

    let catalog = match I18nCatalog::load_embedded() {
        Ok(c) => c,
        Err(err) => {
            tracing::error!(error = %err, "Failed to parse embedded i18n catalog");
            std::process::exit(1);
        }
    };

    let locale = Locale::En;
    let theme = if is_system_dark_mode() {
        ThemeId::StandardDark
    } else {
        ThemeId::StandardLight
    };

    let (shell_tx, mut shell_rx) = mpsc::channel(SHELL_EVENT_CHANNEL_CAPACITY);
    let shell_cfg = NativeShellConfig {
        theme,
        copy: catalog.for_locale(locale).to_shell_copy(),
        capture_hotkey: HotkeyBinding::default_capture(),
        nudge_hotkey: HotkeyBinding::default_focus_nudge(),
        activity_paused: false,
        launch_at_login: ef_win32::get_launch_at_login(paths.is_packaged).is_enabled(),
        wakeup_window_class: DEFAULT_WAKEUP_WINDOW_CLASS.to_string(),
        wakeup_message_name: DEFAULT_WAKEUP_MESSAGE_NAME.to_string(),
    };
    let shell = match NativeShell::start(shell_cfg, shell_tx) {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(error = %err, "Failed to start native Win32 shell");
            std::process::exit(1);
        }
    };

    let (act_tx, mut act_rx) = mpsc::channel(ACTIVITY_EVENT_CHANNEL_CAPACITY);
    let activity = match ActivityMonitor::start(act_tx) {
        Ok(m) => m,
        Err(err) => {
            tracing::error!(error = %err, "Failed to start Win32 activity monitor");
            shell.shutdown();
            std::process::exit(1);
        }
    };

    let (ws_tx, mut ws_rx) = mpsc::channel(WORKSPACE_EVENT_CHANNEL_CAPACITY);
    let workspace = WorkspaceHost::new(paths.local_data_dir.join("webview2"), ws_tx);

    let mut controller = ShellController::new(
        db,
        paths.clone(),
        catalog,
        shell,
        workspace,
        activity,
        locale,
        theme,
        opts.spike_w2_probe,
    );

    let profile = UserProfile::default();
    let router = Router;
    let supervisor = SidecarSupervisor::new();
    let prompt_ver = router
        .prompt_version(FeatureId::BraindumpStructure)
        .unwrap_or(STRUCTURE_PROMPT_V1);

    tracing::info!(
        packaged = paths.is_packaged,
        local_dir = %paths.local_data_dir.display(),
        cluster_id = %profile.cluster_id(),
        embedding_dim = EMBEDDING_DIM,
        root_entity = EntityType::InboxItem.as_str(),
        ?prompt_ver,
        sidecar_state = ?supervisor.state(),
        "Vun native shell started minimized to tray"
    );

    if opts.open_workspace_on_start {
        controller.open_workspace(WorkspaceRoute::Workspace);
    }

    let mut sample_interval = tokio::time::interval(ACTIVITY_SAMPLE_INTERVAL);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown requested via Ctrl+C");
                break;
            }
            Some(ev) = shell_rx.recv() => {
                if controller.handle_shell_event(ev).await {
                    tracing::info!("Shutdown requested from tray menu");
                    break;
                }
            }
            Some(ev) = act_rx.recv() => {
                controller.handle_activity_event(ev).await;
            }
            Some(ev) = ws_rx.recv() => {
                controller.handle_workspace_event(ev).await;
            }
            _ = sample_interval.tick() => {
                controller.record_activity_tick().await;
            }
        }
    }

    controller.shutdown().await;
    drop(single_instance);
    tracing::info!("Vun exited cleanly");
}
