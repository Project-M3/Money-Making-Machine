//! Local Unix domain socket IPC server for inspecting and controlling the running M3 desktop app.
//!
//! Binds `m3_core::app_control::control_socket_path()` with `0600` permissions on a background
//! OS thread and dispatches line-delimited [`AppControlRequest`] payloads onto the NSH engine
//! thread via [`crate::stores::async_bridge::spawn_detached`].

use std::path::Path;

use crate::components::animate_shell_spring;
use crate::stores::{
    AlanCanvasTab, AlanScope, AlanStageMode, AppStore, CompanyViewTab, WorkspaceTab,
};
use m3_core::app_control::{AppControlRequest, AppControlResponse, AppControlState};

#[cfg(unix)]
use std::sync::OnceLock;

/// Maximum incoming request line size accepted on the control socket (1 MiB).
#[cfg(unix)]
const MAX_REQUEST_LINE_BYTES: usize = 1024 * 1024;

#[cfg(unix)]
struct ControlEnvelope {
    request: AppControlRequest,
    reply_tx: std::sync::mpsc::SyncSender<AppControlResponse>,
}

/// Starts the desktop app control IPC server once per process on Unix platforms.
///
/// Must be called from the NSH engine thread (for example inside `app::render_app_root`).
#[cfg(unix)]
pub fn start_control_server(store: AppStore) {
    use tokio::sync::mpsc;

    static CONTROL_SERVER_STARTED: OnceLock<()> = OnceLock::new();
    if CONTROL_SERVER_STARTED.set(()).is_err() {
        return;
    }

    let socket_path = match m3_core::app_control::control_socket_path() {
        Ok(path) => path,
        Err(err) => {
            log::warn!("m3-nsh failed to resolve app control socket path: {err:#}");
            return;
        }
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<ControlEnvelope>();
    crate::stores::async_bridge::spawn_detached(async move {
        while let Some(env) = rx.recv().await {
            let response = dispatch_app_control_request(store, env.request);
            let _ = env.reply_tx.send(response);
        }
    });

    if let Err(err) = std::thread::Builder::new()
        .name("m3-control-ipc".into())
        .spawn(move || run_control_listener_thread(&socket_path, tx))
    {
        log::warn!("m3-nsh failed to spawn app control IPC thread: {err}");
    }
}

/// Non-Unix no-op stub.
#[cfg(not(unix))]
pub fn start_control_server(_store: AppStore) {}

#[cfg(unix)]
fn run_control_listener_thread(
    socket_path: &Path,
    tx: tokio::sync::mpsc::UnboundedSender<ControlEnvelope>,
) {
    let listener = match bind_control_listener(socket_path) {
        Ok(l) => l,
        Err(err) => {
            log::warn!(
                "m3-nsh app control IPC disabled at `{}`: {err:#}",
                socket_path.display()
            );
            return;
        }
    };

    for stream_res in listener.incoming() {
        match stream_res {
            Ok(stream) => handle_control_connection(stream, &tx),
            Err(err) => {
                log::warn!("m3-nsh app control accept error: {err}");
            }
        }
    }
}

/// Binds a Unix domain socket at `socket_path` with `0600` permissions, removing a stale
/// unconnected socket file safely while refusing to clobber an active listener or non-socket.
#[cfg(unix)]
pub(crate) fn bind_control_listener(
    socket_path: &Path,
) -> anyhow::Result<std::os::unix::net::UnixListener> {
    use anyhow::Context as _;
    use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
    use std::os::unix::net::{UnixListener, UnixStream};

    match std::fs::symlink_metadata(socket_path) {
        Ok(meta) => {
            if !meta.file_type().is_socket() {
                anyhow::bail!(
                    "refusing to replace non-socket path `{}`",
                    socket_path.display()
                );
            }
            match UnixStream::connect(socket_path) {
                Ok(_active) => {
                    anyhow::bail!(
                        "control socket `{}` is already owned by a live M3 instance",
                        socket_path.display()
                    );
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                    ) =>
                {
                    let _ = std::fs::remove_file(socket_path);
                }
                Err(err) => {
                    anyhow::bail!(
                        "control socket `{}` exists and liveness probe failed: {err}",
                        socket_path.display()
                    );
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!("inspect control socket path `{}`", socket_path.display())
            });
        }
    }

    if let Some(parent) = socket_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "create control socket parent directory `{}`",
                parent.display()
            )
        })?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("bind control socket `{}`", socket_path.display()))?;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600)).with_context(
        || {
            format!(
                "set 0600 permissions on control socket `{}`",
                socket_path.display()
            )
        },
    )?;
    Ok(listener)
}

#[cfg(unix)]
fn handle_control_connection(
    stream: std::os::unix::net::UnixStream,
    tx: &tokio::sync::mpsc::UnboundedSender<ControlEnvelope>,
) {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::sync::mpsc::{RecvTimeoutError, sync_channel};
    use std::time::Duration;

    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let Ok(mut write_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_LINE_BYTES as u64));
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = match reader.read_line(&mut line) {
            Ok(n) => n,
            Err(_) => break,
        };
        if bytes_read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<AppControlRequest>(trimmed) {
            Ok(request) => {
                let (reply_tx, reply_rx) = sync_channel(1);
                if tx.send(ControlEnvelope { request, reply_tx }).is_err() {
                    error_response(None, "NSH engine thread is unavailable")
                } else {
                    match reply_rx.recv_timeout(Duration::from_secs(15)) {
                        Ok(resp) => resp,
                        Err(RecvTimeoutError::Timeout) => {
                            error_response(None, "timed out waiting for NSH engine thread")
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            error_response(None, "NSH engine thread dropped control request")
                        }
                    }
                }
            }
            Err(err) => error_response(None, format!("invalid AppControlRequest JSON: {err}")),
        };

        let Ok(mut out) = serde_json::to_string(&response) else {
            break;
        };
        out.push('\n');
        if write_stream.write_all(out.as_bytes()).is_err() || write_stream.flush().is_err() {
            break;
        }
    }
}

/// Captures a snapshot of the current reactive [`AppStore`] state as [`AppControlState`].
pub fn snapshot_app_control_state(store: AppStore) -> AppControlState {
    let active_tab = if store.settings_open.get_untracked() {
        "settings".to_owned()
    } else {
        workspace_tab_str(store.active_tab.get_untracked()).to_owned()
    };

    let alan_stage_mode = match store.alan_stage_mode.get_untracked() {
        AlanStageMode::Closed => "closed",
        AlanStageMode::Split => "split",
        AlanStageMode::Immersive => "immersive",
    }
    .to_owned();

    let alan_canvas_tab = match store.alan_canvas_tab.get_untracked() {
        AlanCanvasTab::Interrogate => "interrogate",
        AlanCanvasTab::Feynman => "feynman",
    }
    .to_owned();

    let alan_scope = format_alan_scope(&store.alan_scope.get_untracked());
    let alan_message_count = store.agent_messages.with_untracked(|msgs| msgs.len());
    let last_alan_preview = store.agent_messages.with_untracked(|msgs| {
        msgs.iter()
            .rev()
            .find(|m| !m.text.trim().is_empty())
            .map(|m| truncate_chars(m.text.trim(), 240))
    });

    AppControlState {
        active_tab,
        company_open: store.company_open.get_untracked(),
        selected_ticker: store.selected_ticker.get_untracked(),
        selected_industry_slug: store.selected_industry_slug.get_untracked(),
        sidebar_collapsed: store.sidebar_collapsed.get_untracked(),
        palette_open: store.palette_open.get_untracked(),
        alan_stage_mode,
        alan_canvas_tab,
        alan_scope,
        alan_context_pinned: store.alan_context_pinned.get_untracked(),
        alan_streaming: store.agent_streaming.get_untracked(),
        alan_message_count,
        last_alan_preview,
        workspace_root: store.paths.get_untracked().repo_root.display().to_string(),
    }
}

/// Dispatches an [`AppControlRequest`] synchronously on the NSH engine thread against `store`.
pub fn dispatch_app_control_request(store: AppStore, req: AppControlRequest) -> AppControlResponse {
    match req {
        AppControlRequest::Ping => AppControlResponse {
            ok: true,
            state: Some(snapshot_app_control_state(store)),
            message: Some("pong".to_owned()),
            screenshot_path: None,
            error: None,
        },
        AppControlRequest::GetState => AppControlResponse {
            ok: true,
            state: Some(snapshot_app_control_state(store)),
            message: None,
            screenshot_path: None,
            error: None,
        },
        AppControlRequest::Navigate {
            tab,
            ticker,
            industry,
        } => handle_navigate(store, tab, ticker, industry),
        AppControlRequest::SetAlan {
            mode,
            tab,
            scope,
            pin,
            anchor_label,
            prompt,
            submit,
        } => handle_set_alan(store, mode, tab, scope, pin, anchor_label, prompt, submit),
        AppControlRequest::TriggerAction { action, ticker } => {
            handle_trigger_action(store, &action, ticker)
        }
        AppControlRequest::Screenshot { output_path } => handle_screenshot(store, &output_path),
    }
}

fn handle_navigate(
    store: AppStore,
    tab: Option<String>,
    ticker: Option<String>,
    industry: Option<String>,
) -> AppControlResponse {
    let parsed_tab = match tab.as_deref() {
        Some(raw) => match parse_workspace_tab(raw) {
            Ok(t) => Some(t),
            Err(err) => return error_response(Some(snapshot_app_control_state(store)), err),
        },
        None => None,
    };

    if let Some(t) = parsed_tab {
        store.open_tab(t);
    }

    if let Some(slug) = industry {
        let trimmed = slug.trim();
        if parsed_tab.is_none() && !trimmed.is_empty() {
            store.open_tab(WorkspaceTab::Industry);
        }
        if trimmed.is_empty() {
            store.selected_industry_slug.set(None);
            store.reload_selected_industry_context();
        } else {
            store.select_industry(trimmed);
        }
    }

    if let Some(raw_ticker) = ticker {
        let trimmed = raw_ticker.trim();
        if trimmed.is_empty() {
            store.close_company_overlay();
        } else {
            let canonical = canonical_ticker(store, trimmed);
            store.open_company(&canonical, CompanyViewTab::FinancialsMemo);
        }
    }

    animate_shell_spring();
    AppControlResponse {
        ok: true,
        state: Some(snapshot_app_control_state(store)),
        message: Some("navigated".to_owned()),
        screenshot_path: None,
        error: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_set_alan(
    store: AppStore,
    mode: Option<String>,
    tab: Option<String>,
    scope: Option<String>,
    pin: Option<bool>,
    anchor_label: Option<String>,
    prompt: Option<String>,
    submit: bool,
) -> AppControlResponse {
    let parsed_mode = match mode.as_deref() {
        Some(raw) => match parse_alan_stage_mode(raw) {
            Ok(m) => Some(m),
            Err(err) => return error_response(Some(snapshot_app_control_state(store)), err),
        },
        None => None,
    };
    let parsed_tab = match tab.as_deref() {
        Some(raw) => match parse_alan_canvas_tab(raw) {
            Ok(t) => Some(t),
            Err(err) => return error_response(Some(snapshot_app_control_state(store)), err),
        },
        None => None,
    };
    let parsed_scope = match scope.as_deref() {
        Some(raw) => match parse_alan_scope(raw) {
            Ok(s) => Some(s),
            Err(err) => return error_response(Some(snapshot_app_control_state(store)), err),
        },
        None => None,
    };

    let explicit_scope = parsed_scope.is_some();

    if parsed_mode == Some(AlanStageMode::Closed) {
        if let Some(sc) = parsed_scope {
            set_alan_scope_preserving_closed(store, &sc);
        }
        if let Some(t) = parsed_tab {
            store.set_alan_canvas_tab(t);
        }
        if let Some(p) = pin {
            store.alan_context_pinned.set(p);
        }
        store.set_alan_stage_mode(AlanStageMode::Closed);
    } else {
        if let Some(sc) = parsed_scope {
            let eff_tab = parsed_tab.unwrap_or_else(|| store.alan_canvas_tab.get_untracked());
            store.open_alan_for_scope(sc, eff_tab);
            if let Some(m) = parsed_mode {
                store.set_alan_stage_mode(m);
            }
        } else {
            if let Some(m) = parsed_mode {
                store.set_alan_stage_mode(m);
            }
            if let Some(t) = parsed_tab {
                store.set_alan_canvas_tab(t);
            }
        }

        if let Some(p) = pin {
            if p {
                store.alan_context_pinned.set(true);
            } else if explicit_scope {
                store.alan_context_pinned.set(false);
            } else if store.alan_context_pinned.get_untracked() {
                store.toggle_alan_pin();
            } else {
                store.alan_context_pinned.set(false);
            }
        }
    }

    if let Some(label) = anchor_label {
        let clean = label.trim();
        store
            .alan_anchor_label
            .set((!clean.is_empty()).then(|| clean.to_owned()));
    }

    if submit {
        let question = match prompt {
            Some(ref p) if !p.trim().is_empty() => p.trim().to_owned(),
            _ => store.agent_input_draft.get_untracked().trim().to_owned(),
        };
        if question.is_empty() {
            animate_shell_spring();
            return error_response(
                Some(snapshot_app_control_state(store)),
                "cannot submit empty Alan prompt",
            );
        }
        if store.alan_stage_mode.get_untracked() == AlanStageMode::Closed
            && parsed_mode != Some(AlanStageMode::Closed)
        {
            store.set_alan_stage_mode(AlanStageMode::Split);
        }
        store.submit_scoped_alan_question(question);
    } else if let Some(p) = prompt {
        store.agent_input_draft.set(p);
    }

    animate_shell_spring();
    AppControlResponse {
        ok: true,
        state: Some(snapshot_app_control_state(store)),
        message: Some("alan updated".to_owned()),
        screenshot_path: None,
        error: None,
    }
}

fn set_alan_scope_preserving_closed(store: AppStore, next_scope: &AlanScope) {
    let norm = next_scope.normalized();
    if store.alan_scope.get_untracked() == norm {
        return;
    }
    store.save_alan_session();
    store.alan_scope.set(norm.clone());
    store.agent_expanded_evidence.set(None);
    store.agent_expanded_tape.set(None);
    store.alan_anchor_label.set(None);
    store.agent_tape.set(Vec::new());
    store.agent_streaming.set(false);
    store.agent_status_hint.set(None);
    store.agent_session_ticker.set(match &norm {
        AlanScope::Company(t) => Some(t.clone()),
        _ => None,
    });
    store.load_alan_session(&norm);
}

fn handle_trigger_action(
    store: AppStore,
    action: &str,
    ticker: Option<String>,
) -> AppControlResponse {
    let norm = action.trim().to_ascii_lowercase();
    match norm.as_str() {
        "toggle_sidebar" => {
            animate_shell_spring();
            store.toggle_sidebar();
        }
        "open_palette" => {
            animate_shell_spring();
            store.open_palette();
        }
        "close_palette" => {
            animate_shell_spring();
            store.close_palette();
        }
        "close_company" => {
            animate_shell_spring();
            store.close_company_overlay();
        }
        "sync" => {
            store.start_sync();
        }
        "refresh_briefing" => {
            let period = store.briefing_period.get_untracked();
            store.generate_briefing_now(period, true);
        }
        "reset_alan" => {
            store.reset_alan_session();
        }
        "deep_research" => {
            let target = ticker
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| canonical_ticker(store, s))
                .or_else(|| {
                    let cur = store.selected_ticker.get_untracked();
                    let trimmed = cur.trim();
                    (!trimmed.is_empty()).then(|| canonical_ticker(store, trimmed))
                });
            let Some(resolved) = target else {
                return error_response(
                    Some(snapshot_app_control_state(store)),
                    "deep_research requires a `ticker` or an already-selected company",
                );
            };
            store.open_company(&resolved, CompanyViewTab::FinancialsMemo);
            store.start_deep_research(&resolved, None, "control_ipc");
        }
        _ => {
            return error_response(
                Some(snapshot_app_control_state(store)),
                format!("unknown control action `{action}`"),
            );
        }
    }

    AppControlResponse {
        ok: true,
        state: Some(snapshot_app_control_state(store)),
        message: Some(format!("triggered {norm}")),
        screenshot_path: None,
        error: None,
    }
}

fn handle_screenshot(store: AppStore, raw_output_path: &str) -> AppControlResponse {
    let trimmed = raw_output_path.trim();
    if trimmed.is_empty() {
        return error_response(
            Some(snapshot_app_control_state(store)),
            "screenshot output_path must not be empty",
        );
    }

    let out_path = Path::new(trimmed);
    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        return error_response(
            Some(snapshot_app_control_state(store)),
            format!(
                "failed to create screenshot directory `{}`: {err}",
                parent.display()
            ),
        );
    }

    match capture_process_window_to_path(out_path) {
        Ok(wid) => AppControlResponse {
            ok: true,
            state: Some(snapshot_app_control_state(store)),
            message: Some(format!("captured window {wid}")),
            screenshot_path: Some(trimmed.to_owned()),
            error: None,
        },
        Err(err) => error_response(Some(snapshot_app_control_state(store)), err),
    }
}

#[cfg(target_os = "macos")]
fn capture_process_window_to_path(out_path: &Path) -> Result<u32, String> {
    let pid = std::process::id();
    let wid = macos_window::find_onscreen_window_id(pid)?;

    let out_str = out_path.to_string_lossy().into_owned();
    let output = std::process::Command::new("screencapture")
        .args(["-l", &wid.to_string(), "-o", "-x", &out_str])
        .output()
        .map_err(|err| format!("failed to run `screencapture`: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!(
                "`screencapture -l {wid} -o -x` exited with status {}",
                output.status
            )
        } else {
            format!("`screencapture` failed: {detail}")
        });
    }

    let meta = std::fs::metadata(out_path).map_err(|err| {
        format!(
            "screenshot file `{}` missing after capture: {err}",
            out_path.display()
        )
    })?;
    if !meta.is_file() || meta.len() == 0 {
        return Err(format!(
            "screenshot file `{}` is empty after capture",
            out_path.display()
        ));
    }

    Ok(wid)
}

#[cfg(not(target_os = "macos"))]
fn capture_process_window_to_path(_out_path: &Path) -> Result<u32, String> {
    Err("window screenshot capture is currently supported only on macOS".to_owned())
}

#[cfg(target_os = "macos")]
mod macos_window {
    use std::ffi::c_void;

    type CFArrayRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFNumberRef = *const c_void;
    type CFStringRef = *const c_void;

    const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
    const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const K_CG_NULL_WINDOW_ID: u32 = 0;
    const K_CF_NUMBER_SINT32_TYPE: isize = 3;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        static kCGWindowOwnerPID: CFStringRef;
        static kCGWindowNumber: CFStringRef;
        static kCGWindowLayer: CFStringRef;

        fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(the_array: CFArrayRef) -> isize;
        fn CFArrayGetValueAtIndex(the_array: CFArrayRef, idx: isize) -> *const c_void;
        fn CFDictionaryGetValue(the_dict: CFDictionaryRef, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: CFNumberRef, the_type: isize, value_ptr: *mut c_void) -> u8;
        fn CFRelease(cf: *const c_void);
    }

    struct CfArrayGuard(CFArrayRef);

    impl Drop for CfArrayGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` was retained by `CGWindowListCopyWindowInfo`.
                unsafe { CFRelease(self.0) };
            }
        }
    }

    fn dict_i32(dict: CFDictionaryRef, key: CFStringRef) -> Option<i32> {
        if dict.is_null() || key.is_null() {
            return None;
        }
        // SAFETY: `dict` and `key` are valid CoreFoundation references from the window list.
        let num = unsafe { CFDictionaryGetValue(dict, key) };
        if num.is_null() {
            return None;
        }
        let mut out: i32 = 0;
        // SAFETY: `num` is a CFNumber entry in the window info dictionary and `out` is valid.
        let ok = unsafe {
            CFNumberGetValue(
                num,
                K_CF_NUMBER_SINT32_TYPE,
                (&raw mut out).cast::<c_void>(),
            )
        };
        (ok != 0).then_some(out)
    }

    /// Finds the frontmost normal-layer onscreen window ID owned by `pid`.
    pub(super) fn find_onscreen_window_id(pid: u32) -> Result<u32, String> {
        let target_pid = i32::try_from(pid).map_err(|_| format!("invalid PID {pid}"))?;
        let opts =
            K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
        // SAFETY: CoreGraphics returns a newly retained CFArray or NULL.
        let list = unsafe { CGWindowListCopyWindowInfo(opts, K_CG_NULL_WINDOW_ID) };
        if list.is_null() {
            return Err("CoreGraphics returned no window list".to_owned());
        }
        let _guard = CfArrayGuard(list);

        // SAFETY: `list` is a valid non-null CFArrayRef.
        let count = unsafe { CFArrayGetCount(list) };
        for idx in 0..count {
            // SAFETY: `idx` is within `0..count` on `list`.
            let dict = unsafe { CFArrayGetValueAtIndex(list, idx) };
            if dict.is_null() {
                continue;
            }
            // SAFETY: `kCGWindowOwnerPID`, `kCGWindowLayer`, `kCGWindowNumber` are CoreGraphics globals.
            let (owner_pid, layer, wid) = unsafe {
                (
                    dict_i32(dict, kCGWindowOwnerPID),
                    dict_i32(dict, kCGWindowLayer),
                    dict_i32(dict, kCGWindowNumber),
                )
            };
            if owner_pid == Some(target_pid)
                && layer == Some(0)
                && let Some(raw_wid) = wid
                && raw_wid > 0
            {
                return Ok(raw_wid as u32);
            }
        }

        Err(format!("no onscreen M3 window found for PID {pid}"))
    }
}

fn error_response(
    state: Option<AppControlState>,
    message: impl Into<String>,
) -> AppControlResponse {
    AppControlResponse {
        ok: false,
        state,
        message: None,
        screenshot_path: None,
        error: Some(message.into()),
    }
}

fn canonical_ticker(store: AppStore, raw: &str) -> String {
    let trimmed = raw.trim();
    store.reports.with_untracked(|reps| {
        reps.iter()
            .find(|r| r.ticker.eq_ignore_ascii_case(trimmed))
            .map(|r| r.ticker.clone())
            .unwrap_or_else(|| trimmed.to_ascii_uppercase())
    })
}

fn workspace_tab_str(tab: WorkspaceTab) -> &'static str {
    match tab {
        WorkspaceTab::Inbox => "inbox",
        WorkspaceTab::Universe => "universe",
        WorkspaceTab::Industry => "industry",
        WorkspaceTab::Baskets => "baskets",
        WorkspaceTab::Journal => "journal",
        WorkspaceTab::Settings => "settings",
    }
}

fn parse_workspace_tab(raw: &str) -> Result<WorkspaceTab, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "inbox" => Ok(WorkspaceTab::Inbox),
        "universe" => Ok(WorkspaceTab::Universe),
        "industry" => Ok(WorkspaceTab::Industry),
        "baskets" | "circle" => Ok(WorkspaceTab::Baskets),
        "journal" => Ok(WorkspaceTab::Journal),
        "settings" => Ok(WorkspaceTab::Settings),
        _ => Err(format!(
            "unknown workspace tab `{raw}` (expected inbox|universe|industry|baskets|journal|settings)"
        )),
    }
}

fn parse_alan_stage_mode(raw: &str) -> Result<AlanStageMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "closed" => Ok(AlanStageMode::Closed),
        "split" => Ok(AlanStageMode::Split),
        "immersive" => Ok(AlanStageMode::Immersive),
        _ => Err(format!(
            "unknown Alan stage mode `{raw}` (expected closed|split|immersive)"
        )),
    }
}

fn parse_alan_canvas_tab(raw: &str) -> Result<AlanCanvasTab, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "interrogate" => Ok(AlanCanvasTab::Interrogate),
        "feynman" => Ok(AlanCanvasTab::Feynman),
        _ => Err(format!(
            "unknown Alan canvas tab `{raw}` (expected interrogate|feynman)"
        )),
    }
}

fn parse_alan_scope(raw: &str) -> Result<AlanScope, String> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower == "portfolio" {
        return Ok(AlanScope::Portfolio);
    }
    if lower == "briefing" {
        return Ok(AlanScope::Briefing("latest".to_owned()));
    }
    if let Some((kind, value)) = trimmed.split_once(':') {
        let kind_lower = kind.trim().to_ascii_lowercase();
        let val = value.trim();
        return match kind_lower.as_str() {
            "company" if !val.is_empty() => Ok(AlanScope::Company(val.to_ascii_uppercase())),
            "industry" if !val.is_empty() => Ok(AlanScope::Industry(val.to_owned())),
            "briefing" => Ok(AlanScope::Briefing(if val.is_empty() {
                "latest".to_owned()
            } else {
                val.to_owned()
            })),
            _ => Err(format!(
                "invalid Alan scope `{raw}` (expected company:TICKER|industry:SLUG|briefing:latest|portfolio)"
            )),
        };
    }
    Err(format!(
        "invalid Alan scope `{raw}` (expected company:TICKER|industry:SLUG|briefing:latest|portfolio)"
    ))
}

fn format_alan_scope(scope: &AlanScope) -> String {
    match scope {
        AlanScope::Company(t) => format!("company:{}", t.trim().to_ascii_uppercase()),
        AlanScope::Industry(s) => format!("industry:{}", s.trim()),
        AlanScope::Briefing(id) => {
            let trimmed = id.trim();
            format!(
                "briefing:{}",
                if trimmed.is_empty() {
                    "latest"
                } else {
                    trimmed
                }
            )
        }
        AlanScope::Portfolio => "portfolio".to_owned(),
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}
