use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::ipc::{IpcRequest, IpcResponse, WorkspaceRoute};

pub const WORKSPACE_EVENT_CHANNEL_CAPACITY: usize = 64;

#[cfg(windows)]
#[derive(Debug, Clone)]
pub enum WorkspaceCommand {
    Open(WorkspaceRoute),
    PushResponse(Box<IpcResponse>),
    Shutdown,
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceEvent {
    Opened,
    Closed,
    Ipc(IpcRequest),
    IpcParseError(String),
    HostError(String),
}

/// Ephemeral WebView2 host (`PHASE-02` §6).
///
/// The underlying `tao` + `wry` thread is spawned lazily only when the user opens
/// Workspace or Settings, and closing the window explicitly drops `WebView` and `Window`
/// so all `msedgewebview2.exe` processes exit and resident idle RAM stays below 40 MB.
pub struct WorkspaceHost {
    #[cfg(windows)]
    webview2_data_dir: PathBuf,
    #[cfg(windows)]
    event_tx: mpsc::Sender<WorkspaceEvent>,
    #[cfg(windows)]
    proxy: Option<tao::event_loop::EventLoopProxy<WorkspaceCommand>>,
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(not(windows))]
    is_open: bool,
    #[cfg(not(windows))]
    _webview2_data_dir: PathBuf,
    #[cfg(not(windows))]
    event_tx: mpsc::Sender<WorkspaceEvent>,
}

impl WorkspaceHost {
    #[must_use]
    pub fn new(webview2_data_dir: PathBuf, event_tx: mpsc::Sender<WorkspaceEvent>) -> Self {
        Self {
            #[cfg(windows)]
            webview2_data_dir,
            #[cfg(windows)]
            event_tx,
            #[cfg(windows)]
            proxy: None,
            #[cfg(windows)]
            thread: None,
            #[cfg(not(windows))]
            is_open: false,
            #[cfg(not(windows))]
            _webview2_data_dir: webview2_data_dir,
            #[cfg(not(windows))]
            event_tx,
        }
    }

    pub fn open(&mut self, route: WorkspaceRoute) {
        #[cfg(windows)]
        {
            if self.proxy.is_none() {
                match windows_impl::spawn_ui_thread(
                    self.webview2_data_dir.clone(),
                    self.event_tx.clone(),
                ) {
                    Ok((proxy, thread)) => {
                        self.proxy = Some(proxy);
                        self.thread = Some(thread);
                    }
                    Err(err) => {
                        let _ = self.event_tx.try_send(WorkspaceEvent::HostError(err));
                        return;
                    }
                }
            }
            if let Some(proxy) = &self.proxy {
                let _ = proxy.send_event(WorkspaceCommand::Open(route));
            }
        }
        #[cfg(not(windows))]
        {
            let _ = route;
            if !self.is_open {
                self.is_open = true;
                let _ = self.event_tx.try_send(WorkspaceEvent::Opened);
            }
        }
    }

    pub fn push_response(&self, resp: IpcResponse) {
        #[cfg(windows)]
        if let Some(proxy) = &self.proxy {
            let _ = proxy.send_event(WorkspaceCommand::PushResponse(Box::new(resp)));
        }
        #[cfg(not(windows))]
        {
            let _ = resp;
        }
    }

    pub fn shutdown(mut self) {
        self.stop_thread();
    }

    fn stop_thread(&mut self) {
        #[cfg(windows)]
        {
            if let Some(proxy) = self.proxy.take() {
                let _ = proxy.send_event(WorkspaceCommand::Shutdown);
            }
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
        }
        #[cfg(not(windows))]
        {
            self.is_open = false;
        }
    }
}

impl Drop for WorkspaceHost {
    fn drop(&mut self) {
        self.stop_thread();
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::borrow::Cow;
    use std::path::PathBuf;
    use std::thread::JoinHandle;

    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
    use tao::platform::run_return::EventLoopExtRunReturn as _;
    use tao::platform::windows::EventLoopBuilderExtWindows as _;
    use tao::window::{Window, WindowBuilder};
    use tokio::sync::mpsc;
    use wry::http::{Response, header::CONTENT_TYPE};
    use wry::{WebContext, WebView, WebViewBuilder};

    use super::{WorkspaceCommand, WorkspaceEvent};
    use crate::ipc::{IpcRequest, WorkspaceRoute};

    const INDEX_HTML: &str = include_str!("../../../ui/workspace/index.html");
    const TOKENS_CSS: &str = include_str!("../../../ui/workspace/src/styles/tokens.css");
    const APP_JS: &str = include_str!("../../../ui/workspace/src/app.js");

    struct ActiveWorkspace {
        webview: WebView,
        window: Window,
    }

    fn serve_asset(path: &str) -> Response<Cow<'static, [u8]>> {
        let normalized = path.trim_start_matches('/');
        let (mime, body): (&str, &'static [u8]) = match normalized {
            "" | "index.html" => ("text/html; charset=utf-8", INDEX_HTML.as_bytes()),
            "src/styles/tokens.css" => ("text/css; charset=utf-8", TOKENS_CSS.as_bytes()),
            "src/app.js" => ("application/javascript; charset=utf-8", APP_JS.as_bytes()),
            _ => {
                return Response::builder()
                    .status(404)
                    .header(CONTENT_TYPE, "text/plain; charset=utf-8")
                    .body(Cow::Borrowed(b"Not found".as_slice()))
                    .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"".as_slice())));
            }
        };

        Response::builder()
            .status(200)
            .header(CONTENT_TYPE, mime)
            .body(Cow::Borrowed(body))
            .unwrap_or_else(|_| Response::new(Cow::Borrowed(body)))
    }

    fn create_workspace_window(
        target: &EventLoopWindowTarget<WorkspaceCommand>,
        web_context: &mut WebContext,
        route: WorkspaceRoute,
        event_tx: mpsc::Sender<WorkspaceEvent>,
    ) -> Result<ActiveWorkspace, String> {
        let title = match route {
            WorkspaceRoute::Workspace => "Vun Workspace",
            WorkspaceRoute::Settings => "Vun Settings",
        };
        let window = WindowBuilder::new()
            .with_title(title)
            .with_inner_size(LogicalSize::new(980.0, 740.0))
            .with_min_inner_size(LogicalSize::new(420.0, 420.0))
            .build(target)
            .map_err(|e| format!("tao WindowBuilder failed: {e}"))?;

        let tx_ipc = event_tx;
        let webview = WebViewBuilder::with_web_context(web_context)
            .with_devtools(cfg!(debug_assertions))
            .with_custom_protocol("vun".to_string(), move |_id, request| {
                serve_asset(request.uri().path())
            })
            .with_ipc_handler(move |req| {
                let raw = req.body();
                match serde_json::from_str::<IpcRequest>(raw) {
                    Ok(parsed) => {
                        let _ = tx_ipc.try_send(WorkspaceEvent::Ipc(parsed));
                    }
                    Err(err) => {
                        let _ = tx_ipc.try_send(WorkspaceEvent::IpcParseError(err.to_string()));
                    }
                }
            })
            .with_url("http://vun.localhost/index.html")
            .build(&window)
            .map_err(|e| format!("wry WebViewBuilder failed: {e}"))?;

        Ok(ActiveWorkspace { webview, window })
    }

    pub(super) fn spawn_ui_thread(
        webview2_data_dir: PathBuf,
        event_tx: mpsc::Sender<WorkspaceEvent>,
    ) -> Result<(EventLoopProxy<WorkspaceCommand>, JoinHandle<()>), String> {
        let (ready_tx, ready_rx) =
            std::sync::mpsc::sync_channel::<Result<EventLoopProxy<WorkspaceCommand>, String>>(1);

        let handle = std::thread::Builder::new()
            .name("ef-workspace-ui".to_string())
            .spawn(move || {
                let mut event_loop = EventLoopBuilder::<WorkspaceCommand>::with_user_event()
                    .with_any_thread(true)
                    .with_dpi_aware(false)
                    .build();

                let proxy = event_loop.create_proxy();
                if ready_tx.send(Ok(proxy)).is_err() {
                    return;
                }

                let mut web_context = WebContext::new(Some(webview2_data_dir));
                let mut active: Option<ActiveWorkspace> = None;

                event_loop.run_return(move |event, target, control_flow| {
                    *control_flow = ControlFlow::Wait;
                    match event {
                        Event::UserEvent(cmd) => match cmd {
                            WorkspaceCommand::Open(route) => {
                                if let Some(current) = &active {
                                    current.window.set_focus();
                                    let _ = event_tx.try_send(WorkspaceEvent::Opened);
                                    return;
                                }
                                match create_workspace_window(
                                    target,
                                    &mut web_context,
                                    route,
                                    event_tx.clone(),
                                ) {
                                    Ok(ws) => {
                                        active = Some(ws);
                                        let _ = event_tx.try_send(WorkspaceEvent::Opened);
                                    }
                                    Err(err) => {
                                        let _ = event_tx.try_send(WorkspaceEvent::HostError(err));
                                    }
                                }
                            }
                            WorkspaceCommand::PushResponse(resp) => {
                                if let Some(current) = &active
                                    && let Ok(json) = serde_json::to_string(&*resp)
                                {
                                    let script = format!("window.__vunOnIpcMessage({json});");
                                    let _ = current.webview.evaluate_script(&script);
                                }
                            }
                            WorkspaceCommand::Shutdown => {
                                if let Some(ActiveWorkspace { webview, window }) = active.take() {
                                    // Drop WebView2 controller first so msedgewebview2.exe exits cleanly.
                                    drop(webview);
                                    drop(window);
                                }
                                *control_flow = ControlFlow::Exit;
                            }
                        },
                        Event::WindowEvent {
                            event: WindowEvent::CloseRequested,
                            ..
                        } => {
                            if let Some(ActiveWorkspace { webview, window }) = active.take() {
                                // Explicitly destroy ephemeral WebView2 on close to tray (`PHASE-02` §6).
                                drop(webview);
                                drop(window);
                                let _ = event_tx.try_send(WorkspaceEvent::Closed);
                            }
                        }
                        _ => {}
                    }
                });
            })
            .map_err(|e| format!("failed to spawn ef-workspace-ui thread: {e}"))?;

        let proxy = ready_rx
            .recv()
            .map_err(|_| "ef-workspace-ui thread exited before initialization".to_string())??;
        Ok((proxy, handle))
    }
}
