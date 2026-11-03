//! The interactive terminal: a WebSocket from the browser to a PTY in the
//! sandbox, proxied through the control API.
//!
//! The route is a WebSocket, not a request/response pair, so it is documented
//! with `x-stream: ws`. The control API is the only place the session is
//! recorded: who opened it, when, for how long, and how many bytes crossed —
//! never what was typed or printed (ADR 0023).
//!
//! This file holds no rule. It does four things and nothing else:
//!
//! * refuse a handshake from an origin that is not the console, because a
//!   WebSocket is not covered by the same-origin policy and a session cookie is;
//! * hand the socket to the service, which checks the permission and the limits
//!   *before* the upgrade, so a refusal is a status code and not a socket that
//!   dies mysteriously;
//! * adapt axum's `WebSocket` to the service's `ClientSocket` port;
//! * say why the session ended in the close frame the browser shows.

use std::time::Duration;

use async_trait::async_trait;
use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use uuid::Uuid;
use serde::Deserialize;

use crate::api::principal::Auth;
use crate::domain::auth::CredentialSource;
use crate::error::{ApiError, ErrorCode};
use crate::services::terminal_service::{self, ClientFrame, ClientSocket, Limits, ServerFrame};
use crate::state::AppState;

/// The subprotocol the console speaks. Named in the handshake so a proxy that
/// inspects the upgrade sees a protocol rather than a bare socket.
const SUBPROTOCOL: &str = "nestbox-terminal";

#[derive(Debug, Deserialize)]
pub struct TerminalQuery { pub tab: Option<String> }

/// The tab-list read: `all=1` shows every user's tabs of the sandbox.
#[derive(Debug, serde::Deserialize)]
pub struct TabsQuery {
    /// Any value means "everybody's tabs".
    #[serde(default)]
    pub all: Option<u8>,
}

/// One tab in a PUT body.
#[derive(Debug, Deserialize)]
pub struct TabSpec {
    pub tab_id: String,
    pub title: String,
}

/// The complete tab set the console is showing.
#[derive(Debug, Deserialize)]
pub struct TabSet {
    #[serde(default)]
    pub tabs: Vec<TabSpec>,
}

/// The tabs one user keeps in this sandbox, and whether their shells are alive.
///
/// The list lives on the server so a reload reopens the same terminals on any
/// machine. `durable` is the sandbox's answer about tmux: without it a tab cannot
/// survive a reload, and the console says so rather than showing a fresh shell
/// where an application used to be.
pub async fn tabs(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<TabsQuery>,
) -> Result<Json<terminal_service::TerminalTabs>, ApiError> {
    let tabs = terminal_service::tabs(&state.services, &auth.principal, id, query.all.is_some()).await?;
    Ok(Json(tabs))
}

/// Replace the tab set. A tab that is absent is closed, so a reload cannot
/// resurrect one the operator removed.
pub async fn save_tabs(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<TabSet>,
) -> Result<Json<terminal_service::TerminalTabs>, ApiError> {
    let items = body
        .tabs
        .into_iter()
        .map(|tab| (tab.tab_id, tab.title))
        .collect();
    let tabs = terminal_service::save_tabs(&state.services, &auth.principal, id, items).await?;
    Ok(Json(tabs))
}

/// Which terminal tabs of this sandbox are open, and who is in each.
pub async fn sessions(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(terminal_service::attached(&state.services, &auth.principal, id).await?))
}

/// Attach a terminal to a sandbox.
pub async fn terminal(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<TerminalQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    refuse_foreign_origin(&state, &auth, &headers)?;

    let limits = Limits {
        idle: Duration::from_secs(state.config.terminal_idle_seconds.max(5) as u64),
        ..Limits::default()
    };

    // Everything that can refuse — permission, a sandbox that is not running, a
    // quota that is full — happens here, while an HTTP status code is still
    // possible. A WebSocket that upgrades and then closes is a worse answer than
    // a 429 with a message.
    let tab_id = query.tab.unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    if tab_id.len() > 64 || !tab_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
        return Err(ApiError::validation("Terminal tab ids contain only letters, digits, dashes, and underscores."));
    }
    let ticket = terminal_service::open(&state.services, &auth.principal, id, limits, tab_id).await?;

    let services = state.services.clone();
    let upgrade = upgrade
        .protocols([SUBPROTOCOL])
        .on_upgrade(move |socket| async move {
            let mut client = BrowserSocket::new(socket);
            if let Err(error) = terminal_service::attach(&services, &ticket, &mut client).await {
                // The session is already over by the time this can fail; what
                // matters is that the reason is in the log, not on the wire.
                tracing::warn!(
                    sandbox = %ticket.sandbox_name,
                    session = %ticket.session_id,
                    error = %error.message,
                    "terminal session ended with an error"
                );
            }
            client.finish().await;
        });

    Ok(upgrade)
}

/// Refuse a handshake whose `Origin` is not this console.
///
/// Three cases, and each is deliberate:
///
/// * an `Origin` on the configured allowlist is accepted;
/// * an `Origin` on the console's own host is accepted — the console is served
///   from the same host as the API, and nginx forwards the host without a port,
///   so the port is compared only when the forwarded host carries one;
/// * no `Origin` at all is accepted only for a bearer token. A browser always
///   sends one; a token client is not subject to the same-origin policy and has
///   no ambient credential for a foreign page to ride on. A session request
///   without an `Origin` is the case where a non-browser is trying to use the
///   browser's cookie, and it is refused.
pub(crate) fn refuse_foreign_origin(
    state: &AppState,
    auth: &Auth,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok()) else {
        return match auth.principal.via {
            CredentialSource::Token => Ok(()),
            CredentialSource::Session => Err(refused(None)),
        };
    };

    if state
        .config
        .allowed_origins
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(origin))
    {
        return Ok(());
    }
    if same_host(origin, headers) {
        return Ok(());
    }
    Err(refused(Some(origin)))
}

/// The standard refusal, with the offending origin in the details.
fn refused(origin: Option<&str>) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        ErrorCode::SANDBOX_TERMINAL_ORIGIN,
        "A terminal can only be opened from the console.",
    )
    .with_details(match origin {
        Some(origin) => serde_json::json!({ "origin": origin }),
        None => serde_json::json!({ "origin": null }),
    })
}

/// Whether an origin names the host this request was sent to.
///
/// The comparison is on the host and, when both sides name one, the port. It
/// deliberately does not compare schemes: the console is served over HTTP on a
/// private network today and over HTTPS behind a proxy later, and the Host
/// header already identifies the machine.
fn same_host(origin: &str, headers: &HeaderMap) -> bool {
    let Some((origin_host, origin_port)) = authority(origin) else {
        return false;
    };
    let host_header = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let (host, host_port) = authority_of_host(host_header);
    if host.is_empty() || !host.eq_ignore_ascii_case(origin_host) {
        return false;
    }
    match (origin_port, host_port) {
        (Some(origin_port), Some(host_port)) => origin_port == host_port,
        // nginx forwards `$host`, which carries no port. The host matched, and a
        // page on another port of the same machine is the same machine.
        (Some(_), None) | (None, _) => true,
    }
}

/// Split `scheme://host:port/path` into its host and port.
fn authority(origin: &str) -> Option<(&str, Option<&str>)> {
    let rest = origin.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    // A userinfo section is not expected in an Origin header, but stripping it
    // keeps `https://user@host` from comparing as `user@host`.
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if authority.is_empty() {
        return None;
    }
    Some(authority_of_host(authority))
}

/// Split `host` or `host:port` into its two halves.
fn authority_of_host(value: &str) -> (&str, Option<&str>) {
    match value.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => {
            (host, Some(port))
        }
        _ => (value, None),
    }
}

/// The browser end of the session, as the service needs it.
struct BrowserSocket {
    socket: WebSocket,
}

impl BrowserSocket {
    fn new(socket: WebSocket) -> Self {
        Self { socket }
    }

    /// Close the WebSocket after the service has said why.
    async fn finish(&mut self) {
        let _ = self.socket.send(Message::Close(None)).await;
    }
}

#[async_trait]
impl ClientSocket for BrowserSocket {
    async fn recv(&mut self) -> Option<ClientFrame> {
        loop {
            match self.socket.recv().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return None,
                Some(Ok(Message::Text(text))) => return Some(ClientFrame::parse(text.as_str())),
                // xterm.js sends text. A client that pastes raw bytes is still
                // input, and refusing it would be a puzzle rather than a policy.
                Some(Ok(Message::Binary(bytes))) => {
                    return Some(ClientFrame::Input(
                        String::from_utf8_lossy(&bytes).to_string(),
                    ));
                }
                // axum answers pings itself; they are not frames the shell sees.
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
            }
        }
    }

    async fn send(&mut self, frame: ServerFrame) {
        let message = match &frame {
            ServerFrame::Output(text) => Message::Text(text.clone().into()),
            ServerFrame::Close { .. } => match frame.control_json() {
                Some(json) => Message::Text(json.into()),
                None => return,
            },
        };
        let _ = self.socket.send(message).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(host: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = host.parse() {
            headers.insert(HOST, value);
        }
        headers
    }

    #[test]
    fn the_console_host_is_the_same_origin() {
        assert!(same_host(
            "http://192.168.55.50:8080",
            &headers("192.168.55.50:8080")
        ));
        assert!(same_host("http://nestbox.local", &headers("nestbox.local")));
        // nginx forwards `$host`, which has no port.
        assert!(same_host(
            "http://192.168.55.50:8080",
            &headers("192.168.55.50")
        ));
    }

    #[test]
    fn another_host_is_not_the_same_origin() {
        assert!(!same_host(
            "http://evil.example",
            &headers("192.168.55.50:8080")
        ));
        // When both sides name a port, it must be the same one: the console is
        // served from one origin, and another port is another origin.
        assert!(!same_host(
            "http://192.168.55.50:9999",
            &headers("192.168.55.50:8080")
        ));
        assert!(!same_host("null", &headers("192.168.55.50:8080")));
        assert!(!same_host("http://192.168.55.50", &HeaderMap::new()));
    }

    #[test]
    fn the_origin_is_parsed_into_host_and_port() {
        assert_eq!(
            authority("https://nestbox:8443/api"),
            Some(("nestbox", Some("8443")))
        );
        assert_eq!(authority("http://nestbox"), Some(("nestbox", None)));
        assert_eq!(authority("nostream"), None);
    }
}
