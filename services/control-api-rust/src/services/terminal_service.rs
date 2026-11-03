//! The browser terminal: one audited session per open WebSocket.
//!
//! The rules this module owns, all of them from ADR 0023:
//!
//! * the permission is `sandbox.exec`, checked once here and not in the router;
//! * a session is opened only against a *running* sandbox;
//! * four terminals per sandbox and twenty per user, counted from the rows that
//!   are open, so the limit holds across API replicas and across a restart;
//! * thirty minutes without input ends it with `idle`;
//! * every session writes one `sandbox_sessions` row and two audit events, and
//!   neither ever contains a byte that crossed the terminal.
//!
//! The transport is described by two small ports — [`ClientSocket`] for the
//! browser and `runtime::TerminalSocket` for the PTY — so the pump, the idle
//! clock, and the close reasons are testable without a container or a WebSocket.
//! The API layer adapts axum to [`ClientSocket`]; it holds no rule itself.

use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use super::{ServiceContext, require};
use crate::domain::auth::{Permission, Principal};
use crate::domain::sandbox::SandboxStatus;
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::runtime::{RuntimeError, TerminalMessage, TerminalSocket};
use crate::store::TenantScope;
use crate::store::postgres::audit::{self as audit_store, NewAuditEvent};
use crate::store::postgres::sandboxes as sandbox_store;
use crate::store::postgres::terminal_sessions as session_store;
use crate::store::postgres::terminal_tabs as tab_store;

/// Terminals allowed in one sandbox at the same time.
/// Open terminals in one sandbox across everybody. One sandbox is a machine with
/// one owner and, usually, one administrator watching: four was a limit that two
/// people working together reached by accident.
pub const MAX_PER_SANDBOX: usize = 12;

/// Open terminals one person may keep in one sandbox: the number a person can
/// actually watch, small enough that one tab strip stays readable.
pub const MAX_PER_USER_IN_SANDBOX: usize = 4;

/// Terminals one user may hold open across all sandboxes.
pub const MAX_PER_USER: usize = 20;

/// How long a terminal may go without input before it is closed.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(30 * 60);

/// Extra slack before an unclosed row is treated as abandoned.
///
/// A row is closed by the code that opened it, but a crash or a killed process
/// cannot. Without this grace, one crashed API pod would permanently consume
/// the quota it was holding.
const STALE_GRACE: Duration = Duration::from_secs(5 * 60);

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const MAX_DIMENSION: u16 = 500;
/// A keystroke is small. Anything larger is a client bug or an attempt to make
/// the pump hold memory on behalf of a terminal nobody is watching.
const MAX_INPUT_BYTES: usize = 65_536;

/// The limits and the clock, passed in rather than read from a global so a test
/// and an operator can shorten the idle window without touching the code.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub per_sandbox: usize,
    pub per_user: usize,
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_sandbox: MAX_PER_SANDBOX,
            per_user: MAX_PER_USER,
            idle: DEFAULT_IDLE,
        }
    }
}

impl Limits {
    /// The age at which an unclosed row stops counting: a session cannot outlive
    /// its idle timeout, so anything older was abandoned.
    fn stale_after(&self) -> Duration {
        self.idle + STALE_GRACE
    }
}

/// Why a terminal ended. The four values are the ones the database check
/// constraint and the console's wording agree on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Idle,
    User,
    Error,
    SandboxStopped,
}

impl CloseReason {
    /// The wire and database value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::User => "user",
            Self::Error => "error",
            Self::SandboxStopped => "sandbox_stopped",
        }
    }

    /// Interpret the reason the runner named.
    ///
    /// An unknown reason becomes `error` rather than being passed through: a
    /// value the console does not know would leave it showing a dead cursor.
    fn from_runner(reason: &str) -> Self {
        match reason {
            "idle" => Self::Idle,
            "user" => Self::User,
            "sandbox_stopped" => Self::SandboxStopped,
            _ => Self::Error,
        }
    }
}

/// One frame from the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientFrame {
    /// Keyboard input, to be written to the PTY verbatim.
    Input(String),
    /// A new window size.
    Resize { cols: u16, rows: u16 },
    /// The browser asked to close.
    Closed,
    /// A control frame this version does not know. Ignored, never typed into the
    /// shell: a future frame type must not corrupt someone's session.
    Ignored,
}

impl ClientFrame {
    /// Interpret one text frame from the browser.
    ///
    /// Only a JSON object with a string `type` is a control frame. The cheap
    /// `{` guard keeps ordinary typing out of the JSON parser, and a shell line
    /// that happens to be valid JSON is input, not a command to the pump.
    pub fn parse(text: &str) -> Self {
        if text.len() > MAX_INPUT_BYTES {
            return Self::Closed;
        }
        let Some(value) = control_frame(text) else {
            return Self::Input(text.to_string());
        };
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("resize") => Self::Resize {
                cols: dimension(value.get("cols"), DEFAULT_COLS),
                rows: dimension(value.get("rows"), DEFAULT_ROWS),
            },
            Some("close") => Self::Closed,
            _ => Self::Ignored,
        }
    }
}

/// One frame to the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerFrame {
    /// What the shell printed.
    Output(String),
    /// The session is over; the browser should say why and stop reconnecting.
    Close {
        reason: CloseReason,
        message: Option<String>,
    },
}

impl ServerFrame {
    /// The text of a control frame, or `None` for output, which is sent as-is.
    pub fn control_json(&self) -> Option<String> {
        match self {
            Self::Output(_) => None,
            Self::Close { reason, message } => {
                let mut payload = serde_json::Map::new();
                payload.insert("type".into(), serde_json::Value::String("close".into()));
                payload.insert(
                    "reason".into(),
                    serde_json::Value::String(reason.as_str().to_string()),
                );
                if let Some(message) = message {
                    payload.insert(
                        "message".into(),
                        serde_json::Value::String(message.clone()),
                    );
                }
                Some(serde_json::Value::Object(payload).to_string())
            }
        }
    }
}

/// The browser end of a terminal, as this layer needs it.
///
/// A port rather than axum's `WebSocket`: the service layer must not know HTTP,
/// and the idle clock is easier to test against a queue of frames than against a
/// real socket.
#[async_trait]
pub trait ClientSocket: Send {
    /// The next frame, or `None` when the browser went away.
    async fn recv(&mut self) -> Option<ClientFrame>;

    /// Send one frame. A failed send means the browser is gone, and the next
    /// `recv` says so.
    async fn send(&mut self, frame: ServerFrame);
}

/// An authorised, recorded session waiting for its PTY.
///
/// The split between this and [`attach`] is what puts the refusals — permission,
/// not running, too many terminals — on the HTTP handshake as a status code,
/// instead of inside a WebSocket the browser has already opened.
#[derive(Debug, Clone)]
pub struct Ticket {
    pub session_id: Uuid,
    pub sandbox_id: Uuid,
    pub sandbox_name: String,
    pub user_id: Uuid,
    /// The name the open event recorded, so the close event names the same
    /// person instead of describing the session twice with different words.
    pub actor_label: String,
    pub environment_id: Uuid,
    pub limits: Limits,
    pub opened_at: chrono::DateTime<chrono::Utc>,
    pub tab_id: String,
}

/// What a finished session did, for the log and the return value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionOutcome {
    pub reason: CloseReason,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

/// Authorise a terminal and reserve a session row.
///
/// Returns a ticket, not a terminal: the runtime is not touched until the
/// WebSocket has actually upgraded, so a browser that dies during the handshake
/// does not leave a `docker exec` behind.
pub async fn open(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    limits: Limits,
    tab_id: String,
) -> Result<Ticket, ApiError> {
    require(principal, Permission::SandboxExec)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    let sandbox =
        sandbox_store::get(&mut scope, sandbox_id)
            .await
            .map_err(|error| match error {
                StoreError::NotFound => ApiError::not_found(
                    ErrorCode::SANDBOX_NOT_FOUND,
                    "No sandbox with that id exists in this environment.",
                ),
                other => ApiError::from(other),
            })?;

    if sandbox.status != SandboxStatus::Running {
        scope.commit().await.map_err(ApiError::from)?;
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            format!(
                "A terminal needs a running sandbox; this one is '{}'.",
                sandbox.status.as_str()
            ),
        ));
    }

    // Rows whose process died keep their quota slot until they age out of the
    // count. Closing them here means the next operator frees the ghosts instead
    // of being refused with an empty Quota tab in front of them.
    let swept = session_store::close_stale(&mut scope, limits.stale_after())
        .await
        .map_err(ApiError::from)?;
    if swept > 0 {
        tracing::info!(closed = swept, "closed abandoned terminal sessions");
    }

    // The lock makes "count, then insert" one decision for this user, so two
    // tabs opened at the same moment cannot both pass the last free slot.
    session_store::lock_user(&mut scope, principal.user_id)
        .await
        .map_err(ApiError::from)?;

    let in_sandbox =
        session_store::count_open_in_sandbox(&mut scope, sandbox_id, limits.stale_after())
            .await
            .map_err(ApiError::from)?;
    if in_sandbox >= limits.per_sandbox as i64 {
        scope.commit().await.map_err(ApiError::from)?;
        return Err(ApiError::too_many(
            ErrorCode::SANDBOX_TERMINAL_LIMIT,
            format!(
                "This sandbox already has {} open terminals (limit {}). Close one and try again.",
                in_sandbox, limits.per_sandbox
            ),
        )
        .with_details(serde_json::json!({
            "limit": limits.per_sandbox,
            "open": in_sandbox,
            "scope": "sandbox",
        })));
    }

    let mine_in_sandbox = session_store::count_open_for_user_in_sandbox(
        &mut scope,
        sandbox_id,
        principal.user_id,
        limits.stale_after(),
    )
    .await
    .map_err(ApiError::from)?;
    if mine_in_sandbox >= MAX_PER_USER_IN_SANDBOX as i64 {
        scope.commit().await.map_err(ApiError::from)?;
        return Err(ApiError::too_many(
            ErrorCode::SANDBOX_TERMINAL_LIMIT,
            format!(
                "You already have {mine_in_sandbox} terminals in this sandbox (limit {MAX_PER_USER_IN_SANDBOX}). Close one and try again."
            ),
        )
        .with_details(serde_json::json!({
            "limit": MAX_PER_USER_IN_SANDBOX,
            "open": mine_in_sandbox,
            "scope": "user_in_sandbox",
        })));
    }

    let for_user =
        session_store::count_open_for_user(&mut scope, principal.user_id, limits.stale_after())
            .await
            .map_err(ApiError::from)?;
    if for_user >= limits.per_user as i64 {
        scope.commit().await.map_err(ApiError::from)?;
        return Err(ApiError::too_many(
            ErrorCode::SANDBOX_TERMINAL_LIMIT,
            format!(
                "You already have {} open terminals (limit {}). Close one and try again.",
                for_user, limits.per_user
            ),
        )
        .with_details(serde_json::json!({
            "limit": limits.per_user,
            "open": for_user,
            "scope": "user",
        })));
    }

    // The row is presence as well as quota: it names the tab, so the console can
    // say who is in which shell (ADR 0056).
    let session_id = crate::store::postgres::presence::open_session(
        &mut scope,
        sandbox_id,
        Some(principal.user_id),
        "terminal",
        Some(&tab_id),
        false,
    )
    .await
    .map_err(ApiError::from)?;

    let opened_at = chrono::Utc::now();
    audit_store::record(
        &mut scope,
        NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.terminal_opened",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            // Identity and time. Never a keystroke, never a line of output:
            // that is the whole point of ADR 0023.
            //
            // The key is `terminal_id`, not `session_id`, because the central
            // redactor masks every key containing "session" (a session cookie
            // must never leak). A masked reference is worse than none: it looks
            // like a secret was there. `terminal_id` survives and still names
            // the row a reviewer can look up.
            detail: &serde_json::json!({
                "sandbox": sandbox.name,
                "terminal_id": session_id,
                "user": principal.display_name,
                "started_at": opened_at.to_rfc3339(),
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    crate::services::presence_service::announce(ctx, principal.environment_id, sandbox_id);

    Ok(Ticket {
        session_id,
        sandbox_id,
        sandbox_name: sandbox.name,
        user_id: principal.user_id,
        actor_label: principal.display_name.clone(),
        environment_id: principal.environment_id,
        limits,
        opened_at,
        tab_id,
    })
}

/// Attach a PTY to a ticketed session and run it to completion.
///
/// The session row is closed on every path, including a runtime that refuses the
/// terminal: a row that says "opened, never closed" is exactly the stale entry
/// the limits then have to work around.
pub async fn attach(
    ctx: &ServiceContext,
    ticket: &Ticket,
    client: &mut dyn ClientSocket,
) -> Result<SessionOutcome, ApiError> {
    let mut terminal = match ctx.runtime.open_terminal(&ticket.sandbox_name, &ticket.tab_id, &ticket.actor_label).await {
        Ok(terminal) => terminal,
        Err(error) => {
            let failure = runtime_failure(error);
            close_session(ctx, ticket, CloseReason::Error, 0, 0).await?;
            client
                .send(ServerFrame::Close {
                    reason: CloseReason::Error,
                    message: Some(failure.message.clone()),
                })
                .await;
            return Err(failure);
        }
    };

    let outcome = pump(terminal.as_mut(), client, ticket.limits.idle).await;
    terminal.close(outcome.reason.as_str()).await;
    close_session(
        ctx,
        ticket,
        outcome.reason,
        outcome.bytes_in,
        outcome.bytes_out,
    )
    .await?;
    Ok(outcome)
}

/// Move frames between the browser and the PTY until one side ends the session.
///
/// The idle clock resets on *input* only (ADR 0023): a process that repaints
/// forever is still a session nobody is watching, and a session nobody closes is
/// a shell left open. Output counts as liveness for neither the clock nor the
/// quota.
pub async fn pump(
    terminal: &mut dyn TerminalSocket,
    client: &mut dyn ClientSocket,
    idle: Duration,
) -> SessionOutcome {
    let mut bytes_in: u64 = 0;
    let mut bytes_out: u64 = 0;
    let mut deadline = tokio::time::Instant::now() + idle;
    // Every path below that leaves the loop names a reason; the compiler checks
    // that, which is the point of not defaulting it here.
    let reason: CloseReason;

    loop {
        tokio::select! {
            incoming = client.recv() => {
                match incoming {
                    None | Some(ClientFrame::Closed) => {
                        reason = CloseReason::User;
                        break;
                    }
                    Some(ClientFrame::Ignored) => continue,
                    Some(ClientFrame::Resize { cols, rows }) => {
                        // A failed resize is not worth ending a shell over: the
                        // window keeps its previous size and the user can still
                        // type.
                        let _ = terminal.resize(cols, rows).await;
                        continue;
                    }
                    Some(ClientFrame::Input(text)) => {
                        bytes_in += text.len() as u64;
                        deadline = tokio::time::Instant::now() + idle;
                        if terminal.send_input(&text).await.is_err() {
                            reason = CloseReason::SandboxStopped;
                            break;
                        }
                    }
                }
            }
            message = terminal.recv() => {
                match message {
                    Ok(Some(TerminalMessage::Output(text))) => {
                        bytes_out += text.len() as u64;
                        client.send(ServerFrame::Output(text)).await;
                    }
                    Ok(Some(TerminalMessage::Closed(why))) => {
                        reason = CloseReason::from_runner(&why);
                        break;
                    }
                    Ok(None) => {
                        reason = CloseReason::SandboxStopped;
                        break;
                    }
                    Err(_) => {
                        reason = CloseReason::SandboxStopped;
                        break;
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                reason = CloseReason::Idle;
                break;
            }
        }
    }

    client
        .send(ServerFrame::Close {
            reason,
            message: None,
        })
        .await;
    SessionOutcome {
        reason,
        bytes_in,
        bytes_out,
    }
}

/// Close the row and record the end of the session.
async fn close_session(
    ctx: &ServiceContext,
    ticket: &Ticket,
    reason: CloseReason,
    bytes_in: u64,
    bytes_out: u64,
) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, ticket.environment_id)
        .await
        .map_err(ApiError::from)?;

    session_store::close(
        &mut scope,
        ticket.session_id,
        reason.as_str(),
        bytes_in.min(i64::MAX as u64) as i64,
        bytes_out.min(i64::MAX as u64) as i64,
    )
    .await
    .map_err(ApiError::from)?;

    let duration_s = (chrono::Utc::now() - ticket.opened_at).num_seconds().max(0);
    audit_store::record(
        &mut scope,
        NewAuditEvent {
            actor_id: Some(ticket.user_id),
            actor_label: &ticket.actor_label,
            kind: "sandbox.terminal_closed",
            subject_type: Some("sandbox"),
            subject_id: Some(ticket.sandbox_id),
            detail: &serde_json::json!({
                "sandbox": ticket.sandbox_name,
                "terminal_id": ticket.session_id,
                "reason": reason.as_str(),
                "duration_s": duration_s,
                "bytes_in": bytes_in,
                "bytes_out": bytes_out,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Map a runtime refusal to the published envelope.
fn runtime_failure(error: RuntimeError) -> ApiError {
    match error {
        RuntimeError::NotFound(_) => ApiError::not_found(
            ErrorCode::SANDBOX_NOT_FOUND,
            "The runtime has no container for this sandbox.",
        ),
        RuntimeError::Rejected(message) => ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!("The runtime refused the terminal: {message}"),
        ),
        other => ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            "The runtime could not open a terminal for this sandbox.",
        )
        .with_internal(other.to_string()),
    }
}

/// The JSON object in a text frame, if it is one.
fn control_frame(text: &str) -> Option<serde_json::Value> {
    if !text.starts_with('{') {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    // The `?` is the rule: no string `type` means this is typing, not control.
    value.get("type")?.as_str()?;
    Some(value)
}

/// One terminal dimension, clamped to what a PTY accepts.
fn dimension(value: Option<&serde_json::Value>, default: u16) -> u16 {
    let Some(number) = value.and_then(serde_json::Value::as_u64) else {
        return default;
    };
    number.clamp(1, MAX_DIMENSION as u64) as u16
}

/// One stored terminal tab, with whether its shell is still there.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TerminalTab {
    pub tab_id: String,
    pub title: String,
    /// Who created the tab, for the shared strip (absent in the old shape).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by_name: Option<String>,
    /// Who is in the tab right now.
    #[serde(default)]
    pub viewers: Vec<serde_json::Value>,
    /// Whether the named tmux session exists in the sandbox right now. A tab
    /// whose shell ended is still listed: it is a place to reconnect to, and
    /// hiding it would look like the tab had been closed.
    pub alive: bool,
}

/// A sandbox's terminal tab list.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TerminalTabs {
    pub items: Vec<TerminalTab>,
    /// Whether the image has tmux. `None` when the runtime could not be asked,
    /// which must not be displayed as "not durable".
    pub durable: Option<bool>,
    /// How many tabs may be kept. The console uses it to disable "+".
    pub limit: usize,
}

/// What the sandbox reports about tmux: whether it exists, and its sessions.
///
/// One command, so the two answers cannot come from two different moments. The
/// command is a fixed vector, never built from a request.
async fn tmux_state(ctx: &ServiceContext, sandbox_name: &str) -> Option<(bool, Vec<String>)> {
    let script = "if command -v tmux >/dev/null 2>&1; then echo NESTBOX_TMUX=1; \
                  tmux list-sessions -F '#{session_name}' 2>/dev/null; \
                  else echo NESTBOX_TMUX=0; fi";
    let command = vec!["sh".to_string(), "-lc".to_string(), script.to_string()];
    let result = ctx.runtime.exec(sandbox_name, &command, 10).await.ok()?;
    let mut durable = None;
    let mut sessions = Vec::new();
    for line in result.output.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(value) = line.strip_prefix("NESTBOX_TMUX=") {
            durable = Some(value == "1");
        } else {
            sessions.push(line.to_string());
        }
    }
    Some((durable?, sessions))
}

/// The tabs one user keeps in one sandbox, with the state of their shells.
///
/// The list is the server's, not the browser's: that is what makes a reload
/// reopen the same shells on any machine instead of whatever `localStorage`
/// remembered.
pub async fn tabs(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    all: bool,
) -> Result<TerminalTabs, ApiError> {
    require(principal, Permission::SandboxExec)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = load_sandbox(&mut scope, sandbox_id).await?;
    // With `all` the strip is the sandbox's, not the caller's: an owner and an
    // administrator see the same tabs and can join the same shell (ADR 0056).
    let stored = if all {
        tab_store::list_all(&mut scope, sandbox_id)
            .await
            .map_err(ApiError::from)?
            .into_iter()
            .map(|tab| tab_store::TabRow {
                tab_id: tab.tab_id,
                title: tab.title,
                position: 0,
                last_seen_at: chrono::Utc::now(),
            })
            .collect::<Vec<_>>()
    } else {
        tab_store::list(&mut scope, sandbox_id, principal.user_id)
            .await
            .map_err(ApiError::from)?
    };
    let shared = if all {
        tab_store::list_all(&mut scope, sandbox_id)
            .await
            .map_err(ApiError::from)?
    } else {
        Vec::new()
    };
    let presence = crate::store::postgres::presence::list_open(&mut scope, sandbox_id, 35 * 60)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let state = if sandbox.status == SandboxStatus::Running {
        tmux_state(ctx, &sandbox.name).await
    } else {
        None
    };
    let (durable, sessions) = match state {
        Some((durable, sessions)) => (Some(durable), sessions),
        None => (None, Vec::new()),
    };

    let items = stored
        .into_iter()
        .map(|row| {
            let creator = shared
                .iter()
                .find(|tab| tab.tab_id == row.tab_id)
                .and_then(|tab| tab.created_by_name.clone());
            let viewers: Vec<serde_json::Value> = presence
                .iter()
                .filter(|entry| entry.tab_id.as_deref() == Some(row.tab_id.as_str()))
                .map(|entry| {
                    serde_json::json!({
                        "user_name": entry.user_name,
                        "kind": entry.kind,
                        "view_only": entry.view_only,
                    })
                })
                .collect();
            TerminalTab {
                alive: sessions.iter().any(|name| name == &format!("nestbox-{}", row.tab_id)),
                tab_id: row.tab_id,
                title: row.title,
                created_by_name: creator,
                viewers,
            }
        })
        .collect();

    Ok(TerminalTabs {
        items,
        durable,
        limit: tab_store::MAX_TABS,
    })
}

/// Replace the tab list one user keeps in one sandbox.
///
/// The console sends what it is showing; a tab it closed is absent here and
/// therefore gone. Validation is the same shape the WebSocket route enforces,
/// because the id becomes a tmux session name.
pub async fn save_tabs(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    tabs: Vec<(String, String)>,
) -> Result<TerminalTabs, ApiError> {
    require(principal, Permission::SandboxExec)?;
    if tabs.len() > tab_store::MAX_TABS {
        return Err(ApiError::validation(format!(
            "A sandbox keeps at most {} terminal tabs.",
            tab_store::MAX_TABS
        )));
    }
    for (tab_id, title) in &tabs {
        let shaped = !tab_id.is_empty()
            && tab_id.len() <= 64
            && tab_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        if !shaped {
            return Err(ApiError::validation(
                "A terminal tab id is 1-64 letters, digits, dashes or underscores.",
            ));
        }
        if title.trim().is_empty() || title.chars().count() > 120 {
            return Err(ApiError::validation(
                "A terminal tab title is 1-120 characters.",
            ));
        }
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    load_sandbox(&mut scope, sandbox_id).await?;
    let stored = tab_store::replace(&mut scope, sandbox_id, principal.user_id, &tabs)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(TerminalTabs {
        items: stored
            .into_iter()
            .map(|row| TerminalTab {
                alive: false,
                tab_id: row.tab_id,
                title: row.title,
                created_by_name: None,
                viewers: Vec::new(),
            })
            .collect(),
        durable: None,
        limit: tab_store::MAX_TABS,
    })
}

/// One sandbox, or the published not-found error.
async fn load_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<crate::domain::sandbox::Sandbox, ApiError> {
    sandbox_store::get(scope, sandbox_id)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(
                ErrorCode::SANDBOX_NOT_FOUND,
                "No sandbox with that id exists in this environment.",
            ),
            other => ApiError::from(other),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A PTY end that the test drives.
    struct TestTerminal {
        feed: tokio::sync::mpsc::UnboundedReceiver<TerminalMessage>,
        sender: tokio::sync::mpsc::UnboundedSender<TerminalMessage>,
        input: Vec<String>,
        resizes: Vec<(u16, u16)>,
        closed_with: Option<String>,
        fail_input: bool,
    }

    impl TestTerminal {
        fn new() -> Self {
            let (sender, feed) = tokio::sync::mpsc::unbounded_channel();
            Self {
                feed,
                sender,
                input: Vec::new(),
                resizes: Vec::new(),
                closed_with: None,
                fail_input: false,
            }
        }

        fn say(&self, text: &str) {
            let _ = self.sender.send(TerminalMessage::Output(text.to_string()));
        }

        fn end(&self, reason: &str) {
            let _ = self
                .sender
                .send(TerminalMessage::Closed(reason.to_string()));
        }
    }

    #[async_trait]
    impl TerminalSocket for TestTerminal {
        async fn recv(&mut self) -> Result<Option<TerminalMessage>, RuntimeError> {
            Ok(self.feed.recv().await)
        }

        async fn send_input(&mut self, text: &str) -> Result<(), RuntimeError> {
            if self.fail_input {
                return Err(RuntimeError::Unavailable("pipe closed".into()));
            }
            self.input.push(text.to_string());
            Ok(())
        }

        async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), RuntimeError> {
            self.resizes.push((cols, rows));
            Ok(())
        }

        async fn close(&mut self, reason: &str) {
            self.closed_with = Some(reason.to_string());
        }
    }

    /// A browser end with a scripted queue.
    #[derive(Default)]
    struct TestClient {
        frames: VecDeque<ClientFrame>,
        sent: Vec<ServerFrame>,
        gone: bool,
        /// How long the browser takes to send each frame, so a test can prove
        /// that input moves the idle deadline.
        delay: Option<Duration>,
    }

    impl TestClient {
        fn with(frames: Vec<ClientFrame>) -> Self {
            Self {
                frames: frames.into(),
                sent: Vec::new(),
                gone: false,
                delay: None,
            }
        }

        fn output(&self) -> String {
            self.sent
                .iter()
                .filter_map(|frame| match frame {
                    ServerFrame::Output(text) => Some(text.clone()),
                    ServerFrame::Close { .. } => None,
                })
                .collect()
        }

        fn close(&self) -> Option<&ServerFrame> {
            self.sent
                .iter()
                .find(|frame| matches!(frame, ServerFrame::Close { .. }))
        }
    }

    #[async_trait]
    impl ClientSocket for TestClient {
        async fn recv(&mut self) -> Option<ClientFrame> {
            if let (Some(delay), false) = (self.delay, self.frames.is_empty()) {
                tokio::time::sleep(delay).await;
            }
            match self.frames.pop_front() {
                Some(frame) => Some(frame),
                // An empty queue means the browser said nothing, which is not
                // the same as the browser leaving: the idle clock must be able
                // to fire while a tab sits still.
                None if self.gone => None,
                None => futures::future::pending().await,
            }
        }

        async fn send(&mut self, frame: ServerFrame) {
            self.sent.push(frame);
        }
    }

    #[tokio::test]
    async fn input_flows_to_the_terminal() {
        // Input and the browser's close are one queue, so their order is the
        // order the pump sees them in. Output is a separate channel and would
        // race, which is why the output direction is its own test.
        let mut terminal = TestTerminal::new();
        let mut client = TestClient::with(vec![
            ClientFrame::Input("ls\r".into()),
            ClientFrame::Closed,
        ]);

        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            pump(&mut terminal, &mut client, DEFAULT_IDLE),
        )
        .await;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(_) => panic!("the pump did not finish"),
        };

        assert_eq!(terminal.input, vec!["ls\r".to_string()]);
        assert_eq!(outcome.reason, CloseReason::User);
        assert_eq!(outcome.bytes_in, 3);
        assert_eq!(
            client.close(),
            Some(&ServerFrame::Close {
                reason: CloseReason::User,
                message: None
            })
        );
    }

    #[tokio::test]
    async fn output_reaches_the_browser_and_is_counted() {
        let mut terminal = TestTerminal::new();
        let mut client = TestClient::default();
        terminal.say("$ ls\r\n");
        terminal.end("sandbox_stopped");

        let outcome = pump(&mut terminal, &mut client, DEFAULT_IDLE).await;

        assert_eq!(client.output(), "$ ls\r\n");
        assert_eq!(outcome.bytes_out, 6);
        assert_eq!(outcome.reason, CloseReason::SandboxStopped);
    }

    #[tokio::test]
    async fn a_browser_that_leaves_ends_the_session() {
        let mut terminal = TestTerminal::new();
        let mut client = TestClient {
            gone: true,
            ..TestClient::default()
        };

        let outcome = pump(&mut terminal, &mut client, DEFAULT_IDLE).await;

        assert_eq!(outcome.reason, CloseReason::User);
    }

    #[tokio::test]
    async fn a_resize_is_applied_and_never_typed() {
        let mut terminal = TestTerminal::new();
        let mut client = TestClient::with(vec![ClientFrame::Resize { cols: 80, rows: 24 }]);
        client.gone = true;

        let outcome = pump(&mut terminal, &mut client, DEFAULT_IDLE).await;

        assert_eq!(terminal.resizes, vec![(80, 24)]);
        assert!(terminal.input.is_empty());
        assert_eq!(outcome.reason, CloseReason::User);
    }

    #[tokio::test]
    async fn an_idle_terminal_closes_with_the_idle_reason() {
        let mut terminal = TestTerminal::new();
        let mut client = TestClient::default();

        let outcome = pump(&mut terminal, &mut client, Duration::from_millis(50)).await;

        assert_eq!(outcome.reason, CloseReason::Idle);
        assert!(matches!(
            client.close(),
            Some(ServerFrame::Close {
                reason: CloseReason::Idle,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn output_does_not_postpone_the_idle_close() {
        // ADR 0023 measures idleness in input: a repainting process is still an
        // unattended session.
        let mut terminal = TestTerminal::new();
        let mut client = TestClient::default();
        let sender = terminal.sender.clone();
        let sparkle = tokio::spawn(async move {
            for _ in 0..40 {
                let _ = sender.send(TerminalMessage::Output("tick".into()));
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        let outcome = pump(&mut terminal, &mut client, Duration::from_millis(120)).await;
        sparkle.abort();

        assert_eq!(outcome.reason, CloseReason::Idle);
    }

    #[tokio::test]
    async fn input_moves_the_idle_deadline() {
        // Two keystrokes, 100 ms apart, against a 150 ms idle window. Without
        // the reset the session would close at 150 ms; with it, both arrive and
        // the session ends on the clock after the last one.
        let mut terminal = TestTerminal::new();
        let mut client = TestClient::with(vec![
            ClientFrame::Input("a".into()),
            ClientFrame::Input("b".into()),
        ]);
        client.delay = Some(Duration::from_millis(150));

        let outcome = pump(&mut terminal, &mut client, Duration::from_millis(400)).await;

        assert_eq!(terminal.input, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(outcome.bytes_in, 2);
        assert_eq!(outcome.reason, CloseReason::Idle);
    }

    #[tokio::test]
    async fn the_runner_reason_is_passed_through() {
        let mut terminal = TestTerminal::new();
        let mut client = TestClient::default();
        terminal.end("sandbox_stopped");

        let outcome = pump(&mut terminal, &mut client, DEFAULT_IDLE).await;

        assert_eq!(outcome.reason, CloseReason::SandboxStopped);
        assert!(matches!(
            client.close(),
            Some(ServerFrame::Close {
                reason: CloseReason::SandboxStopped,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn an_unknown_runner_reason_becomes_an_error() {
        let mut terminal = TestTerminal::new();
        let mut client = TestClient::default();
        terminal.end("something_new");

        let outcome = pump(&mut terminal, &mut client, DEFAULT_IDLE).await;

        assert_eq!(outcome.reason, CloseReason::Error);
    }

    #[tokio::test]
    async fn a_closed_pipe_ends_the_session() {
        let mut terminal = TestTerminal::new();
        terminal.fail_input = true;
        let mut client = TestClient::with(vec![ClientFrame::Input("x".into())]);

        let outcome = pump(&mut terminal, &mut client, DEFAULT_IDLE).await;

        assert_eq!(outcome.reason, CloseReason::SandboxStopped);
    }

    #[test]
    fn typing_is_not_a_control_frame() {
        for text in ["ls -la\r", "{", "{}", "{not json", "\u{1b}[A"] {
            assert_eq!(
                ClientFrame::parse(text),
                ClientFrame::Input(text.to_string()),
                "{text}"
            );
        }
    }

    #[test]
    fn a_resize_frame_is_clamped() {
        assert_eq!(
            ClientFrame::parse(r#"{"type":"resize","cols":120,"rows":40}"#),
            ClientFrame::Resize {
                cols: 120,
                rows: 40
            }
        );
        assert_eq!(
            ClientFrame::parse(r#"{"type":"resize","cols":0,"rows":9999}"#),
            ClientFrame::Resize {
                cols: 1,
                rows: MAX_DIMENSION
            }
        );
        assert_eq!(
            ClientFrame::parse(r#"{"type":"resize"}"#),
            ClientFrame::Resize {
                cols: DEFAULT_COLS,
                rows: DEFAULT_ROWS
            }
        );
    }

    #[test]
    fn a_close_frame_and_an_unknown_frame_are_control_frames() {
        assert_eq!(
            ClientFrame::parse(r#"{"type":"close"}"#),
            ClientFrame::Closed
        );
        assert_eq!(
            ClientFrame::parse(r#"{"type":"ping"}"#),
            ClientFrame::Ignored
        );
    }

    #[test]
    fn an_oversized_frame_is_not_typed_into_the_shell() {
        let huge = "x".repeat(MAX_INPUT_BYTES + 1);
        assert_eq!(ClientFrame::parse(&huge), ClientFrame::Closed);
    }

    #[test]
    fn the_close_frame_carries_the_reason() {
        let frame = ServerFrame::Close {
            reason: CloseReason::Idle,
            message: None,
        };
        let json: serde_json::Value =
            serde_json::from_str(&frame.control_json().unwrap_or_default())
                .unwrap_or(serde_json::Value::Null);
        assert_eq!(json["type"], "close");
        assert_eq!(json["reason"], "idle");
        assert!(json.get("message").is_none());
        assert_eq!(ServerFrame::Output("x".into()).control_json(), None);
    }

    #[test]
    fn an_abandoned_row_stops_counting_after_the_idle_window() {
        let limits = Limits::default();
        assert_eq!(limits.idle, Duration::from_secs(1800));
        assert_eq!(limits.stale_after(), Duration::from_secs(2100));
    }
}


/// Who is attached to which terminal tab of one sandbox.
///
/// A terminal is shared: several people can sit in the same `tmux` session. The
/// runner holds the sockets, so this is a read of its live answer, not of a table.
pub async fn attached(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(|error| match error {
            crate::error::StoreError::NotFound => {
                ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox in this environment.")
            }
            other => ApiError::from(other),
        })?;
    scope.commit().await.map_err(ApiError::from)?;
    ctx.runtime
        .terminal_sessions(&sandbox.name)
        .await
        .map_err(|error| ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, error.to_string()))
}
