//! Shared Desktop Control IPC types and Unix domain socket client for Project M3.
//!
//! Enables `m3 app ...` CLI subcommands and `m3 mcp` tools to inspect and control
//! the running desktop app (`apps/m3-nsh`) over a local Unix domain socket.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::app_config::M3Paths;

/// Default IPC client timeout when none is specified by the caller.
pub const DEFAULT_APP_CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum response line size accepted from the desktop app control socket (4 MiB).
#[cfg(unix)]
const MAX_RESPONSE_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Resolves the Unix domain socket path used for desktop app control IPC.
///
/// Uses `M3_CONTROL_SOCKET` if set and non-empty; otherwise defaults to
/// `<M3Paths::resolve()?.config_dir>/control.sock`.
pub fn control_socket_path() -> Result<PathBuf> {
    if let Ok(override_path) = std::env::var("M3_CONTROL_SOCKET") {
        let trimmed = override_path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    let paths = M3Paths::resolve().context("resolve M3 host paths for control socket")?;
    Ok(paths.config_dir.join("control.sock"))
}

/// Request sent to the running M3 desktop application over the control socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppControlRequest {
    Ping,
    GetState,
    Navigate {
        #[serde(default)]
        tab: Option<String>,
        #[serde(default)]
        ticker: Option<String>,
        #[serde(default)]
        industry: Option<String>,
    },
    SetAlan {
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        tab: Option<String>,
        #[serde(default)]
        scope: Option<String>,
        #[serde(default)]
        pin: Option<bool>,
        #[serde(default)]
        anchor_label: Option<String>,
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        submit: bool,
    },
    TriggerAction {
        action: String,
        #[serde(default)]
        ticker: Option<String>,
    },
    Screenshot {
        output_path: String,
    },
}

/// Snapshot of the running M3 desktop application UI & Alan stage state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppControlState {
    pub active_tab: String,
    pub company_open: bool,
    pub selected_ticker: String,
    pub selected_industry_slug: Option<String>,
    pub sidebar_collapsed: bool,
    pub palette_open: bool,
    pub alan_stage_mode: String,
    pub alan_canvas_tab: String,
    pub alan_scope: String,
    pub alan_context_pinned: bool,
    pub alan_streaming: bool,
    pub alan_message_count: usize,
    pub last_alan_preview: Option<String>,
    pub workspace_root: String,
}

/// Response returned by the M3 desktop application control server.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AppControlResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<AppControlState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Sends an [`AppControlRequest`] to the running M3 desktop app at [`control_socket_path`].
pub fn send_app_control_request(
    req: &AppControlRequest,
    timeout: Duration,
) -> Result<AppControlResponse> {
    let socket_path = control_socket_path()?;
    send_app_control_request_at(&socket_path, req, timeout)
}

/// Sends an [`AppControlRequest`] to an explicit Unix domain socket path.
#[cfg(unix)]
pub fn send_app_control_request_at(
    socket_path: &Path,
    req: &AppControlRequest,
    timeout: Duration,
) -> Result<AppControlResponse> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixStream;

    let effective_timeout = if timeout.is_zero() {
        DEFAULT_APP_CONTROL_TIMEOUT
    } else {
        timeout
    };

    let mut stream = UnixStream::connect(socket_path).with_context(|| {
        format!(
            "connect to M3 desktop control socket `{}` (is the M3 desktop app running?)",
            socket_path.display()
        )
    })?;
    stream
        .set_read_timeout(Some(effective_timeout))
        .context("set control socket read timeout")?;
    stream
        .set_write_timeout(Some(effective_timeout))
        .context("set control socket write timeout")?;

    let mut payload = serde_json::to_string(req).context("serialize AppControlRequest payload")?;
    payload.push('\n');
    stream
        .write_all(payload.as_bytes())
        .context("write AppControlRequest to control socket")?;
    stream
        .flush()
        .context("flush AppControlRequest to control socket")?;

    let mut reader = BufReader::new(stream.take(MAX_RESPONSE_LINE_BYTES as u64));
    let mut line = String::new();
    let bytes_read = reader
        .read_line(&mut line)
        .context("read AppControlResponse from control socket")?;
    anyhow::ensure!(
        bytes_read > 0,
        "M3 desktop control socket `{}` closed without a response",
        socket_path.display()
    );
    let trimmed = line.trim();
    anyhow::ensure!(
        !trimmed.is_empty(),
        "M3 desktop control socket `{}` returned an empty response",
        socket_path.display()
    );

    serde_json::from_str::<AppControlResponse>(trimmed).with_context(|| {
        format!(
            "decode AppControlResponse JSON from `{}`",
            socket_path.display()
        )
    })
}

/// Non-Unix fallback returning an explicit unsupported-platform error.
#[cfg(not(unix))]
pub fn send_app_control_request_at(
    _socket_path: &Path,
    _req: &AppControlRequest,
    _timeout: Duration,
) -> Result<AppControlResponse> {
    anyhow::bail!(
        "Desktop app control IPC over Unix domain sockets is only supported on Unix platforms"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn control_socket_path_respects_env_and_fallback() {
        let fallback = control_socket_path().unwrap();
        assert!(fallback.ends_with("control.sock"));
    }

    #[test]
    fn app_control_request_and_response_serde_roundtrip() {
        let requests = [
            AppControlRequest::Ping,
            AppControlRequest::GetState,
            AppControlRequest::Navigate {
                tab: Some("universe".to_owned()),
                ticker: Some("AAPL".to_owned()),
                industry: None,
            },
            AppControlRequest::SetAlan {
                mode: Some("split".to_owned()),
                tab: Some("interrogate".to_owned()),
                scope: Some("company:AAPL".to_owned()),
                pin: Some(true),
                anchor_label: Some("ROIC".to_owned()),
                prompt: Some("Check moat durability".to_owned()),
                submit: true,
            },
            AppControlRequest::TriggerAction {
                action: "toggle_sidebar".to_owned(),
                ticker: None,
            },
            AppControlRequest::Screenshot {
                output_path: "/tmp/m3-shot.png".to_owned(),
            },
        ];

        for req in requests {
            let raw = serde_json::to_string(&req).unwrap();
            let back: AppControlRequest = serde_json::from_str(&raw).unwrap();
            assert_eq!(back, req);
        }

        let resp = AppControlResponse {
            ok: true,
            state: Some(AppControlState {
                active_tab: "inbox".to_owned(),
                company_open: true,
                selected_ticker: "AAPL".to_owned(),
                selected_industry_slug: None,
                sidebar_collapsed: false,
                palette_open: false,
                alan_stage_mode: "split".to_owned(),
                alan_canvas_tab: "interrogate".to_owned(),
                alan_scope: "company:AAPL".to_owned(),
                alan_context_pinned: true,
                alan_streaming: false,
                alan_message_count: 2,
                last_alan_preview: Some("Summary".to_owned()),
                workspace_root: "/tmp/ws".to_owned(),
            }),
            message: Some("pong".to_owned()),
            screenshot_path: None,
            error: None,
        };
        let raw = serde_json::to_string(&resp).unwrap();
        let back: AppControlResponse = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, resp);
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_roundtrip_handles_requests_and_missing_socket() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;

        let seq = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("m3-app-ctrl-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sock_path = dir.join("control.sock");

        // Missing socket returns clear error.
        let missing = send_app_control_request_at(
            &sock_path,
            &AppControlRequest::Ping,
            Duration::from_secs(1),
        );
        assert!(missing.is_err());

        let listener = UnixListener::bind(&sock_path).unwrap();
        let server = std::thread::spawn(move || {
            for stream_res in listener.incoming().take(2) {
                let mut stream = stream_res.unwrap();
                let mut line = String::new();
                {
                    let mut reader = BufReader::new(&stream);
                    reader.read_line(&mut line).unwrap();
                }
                let req: AppControlRequest = serde_json::from_str(line.trim()).unwrap();
                let resp = match req {
                    AppControlRequest::Ping => AppControlResponse {
                        ok: true,
                        state: Some(AppControlState {
                            active_tab: "inbox".to_owned(),
                            alan_stage_mode: "closed".to_owned(),
                            ..Default::default()
                        }),
                        message: Some("pong".to_owned()),
                        screenshot_path: None,
                        error: None,
                    },
                    AppControlRequest::Navigate { tab, ticker, .. } => AppControlResponse {
                        ok: true,
                        state: Some(AppControlState {
                            active_tab: tab.unwrap_or_else(|| "inbox".to_owned()),
                            company_open: ticker.is_some(),
                            selected_ticker: ticker.unwrap_or_default(),
                            ..Default::default()
                        }),
                        message: Some("navigated".to_owned()),
                        screenshot_path: None,
                        error: None,
                    },
                    other => AppControlResponse {
                        ok: false,
                        state: None,
                        message: None,
                        screenshot_path: None,
                        error: Some(format!("unexpected {other:?}")),
                    },
                };
                let mut out = serde_json::to_string(&resp).unwrap();
                out.push('\n');
                stream.write_all(out.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
        });

        let ping_resp = send_app_control_request_at(
            &sock_path,
            &AppControlRequest::Ping,
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(ping_resp.ok);
        assert_eq!(ping_resp.message.as_deref(), Some("pong"));
        assert_eq!(
            ping_resp.state.as_ref().map(|s| s.active_tab.as_str()),
            Some("inbox")
        );

        let nav_resp = send_app_control_request_at(
            &sock_path,
            &AppControlRequest::Navigate {
                tab: Some("baskets".to_owned()),
                ticker: Some("COST".to_owned()),
                industry: None,
            },
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(nav_resp.ok);
        let nav_state = nav_resp.state.unwrap();
        assert_eq!(nav_state.active_tab, "baskets");
        assert!(nav_state.company_open);
        assert_eq!(nav_state.selected_ticker, "COST");

        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
