//! The sandbox desktop: a WebSocket from the browser (noVNC) to the VNC server
//! inside the sandbox, carried through the control API and the runner.
//!
//! Like the terminal route it checks the origin, then the permission and the
//! sandbox state *before* the upgrade, and then only moves frames. It reads no
//! pixel and records no screen; the audit line says who attached to which
//! sandbox and for how long.
//!
//! A caller who may only read the sandbox gets a view-only stream: the request
//! says so, and the runner drops keyboard, pointer, clipboard and resize
//! messages on their way to the desktop. The permission for driving and the
//! filter in the runner are decided in two places on purpose — neither the
//! console nor the caller can widen the other.

use axum::extract::ws::{Message as ClientMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::Message as RunnerMessage;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::api::sandboxes_terminal::refuse_foreign_origin;
use crate::domain::auth::Principal;
use crate::error::ApiError;
use crate::services::{ServiceContext, metrics_service, presence_service};
use crate::state::AppState;
use crate::store::TenantScope;
use crate::store::postgres::presence;

/// noVNC offers `binary`; answering with it keeps the handshake legal.
const SUBPROTOCOL: &str = "binary";

/// The query the console sends when it attaches to a desktop.
#[derive(Debug, Deserialize)]
pub struct DesktopQuery {
    /// `1` asks for a stream that shows the desktop but cannot drive it.
    #[serde(default)]
    pub view: Option<u8>,
}

/// Attach the browser to the sandbox's desktop.
pub async fn desktop(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<DesktopQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    refuse_foreign_origin(&state, &auth, &headers)?;
    // The console may ask for a view-only stream; a caller without `SandboxExec`
    // gets one whether they asked or not, which `desktop_target` decides.
    let requested_view_only = query.view == Some(1);
    let (name, view_only) = metrics_service::desktop_target(
        &state.services,
        &auth.principal,
        id,
        requested_view_only,
    )
    .await?;
    let runner = state
        .services
        .runtime
        .open_vnc(&name, view_only)
        .await
        .map_err(crate::services::metrics_service::runner_error)?;
    let who = auth.principal.display_name.clone();
    let principal = auth.principal.clone();
    let services = state.services.clone();
    Ok(upgrade.protocols([SUBPROTOCOL]).on_upgrade(move |socket| async move {
        tracing::info!(sandbox = %name, user = %who, view_only, "desktop session opened");
        let started = std::time::Instant::now();
        let session = open_presence(&services, &principal, id, view_only).await;
        pump(socket, runner).await;
        close_presence(&services, &principal, id, session).await;
        tracing::info!(sandbox = %name, user = %who, seconds = started.elapsed().as_secs(), "desktop session closed");
    }))
}

/// Record that somebody is watching this desktop, and say so on the bus.
///
/// Presence is bookkeeping around the stream, not part of it: a failure here is
/// logged and the desktop keeps working, because a database hiccup must not
/// take the picture away from someone who is allowed to see it. The returned
/// id is what the close path needs; `None` means there is nothing to close.
async fn open_presence(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    view_only: bool,
) -> Option<Uuid> {
    let mut scope = match TenantScope::begin_for(&ctx.pool, principal).await {
        Ok(scope) => scope,
        Err(error) => {
            tracing::warn!(sandbox = %sandbox_id, error = %error, "desktop presence: could not open a scope");
            return None;
        }
    };
    let session = match presence::open_session(
        &mut scope,
        sandbox_id,
        Some(principal.user_id),
        "desktop",
        None,
        view_only,
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(sandbox = %sandbox_id, error = %error, "desktop presence: could not record the session");
            return None;
        }
    };
    if let Err(error) = scope.commit().await {
        tracing::warn!(sandbox = %sandbox_id, error = %error, "desktop presence: could not commit the session");
        return None;
    }
    presence_service::announce(ctx, principal.environment_id, sandbox_id);
    Some(session)
}

/// Close the presence row of a desktop that ended, and announce it.
///
/// Same rule as opening: a failure is logged and changes nothing else.
async fn close_presence(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    session: Option<Uuid>,
) {
    let Some(session) = session else {
        return;
    };
    let mut scope = match TenantScope::begin_for(&ctx.pool, principal).await {
        Ok(scope) => scope,
        Err(error) => {
            tracing::warn!(sandbox = %sandbox_id, error = %error, "desktop presence: could not open a scope to close");
            return;
        }
    };
    if let Err(error) = presence::close_session(&mut scope, session, "closed").await {
        tracing::warn!(sandbox = %sandbox_id, error = %error, "desktop presence: could not close the session");
        return;
    }
    if let Err(error) = scope.commit().await {
        tracing::warn!(sandbox = %sandbox_id, error = %error, "desktop presence: could not commit the close");
        return;
    }
    presence_service::announce(ctx, principal.environment_id, sandbox_id);
}

/// Move frames both ways until either side ends.
async fn pump(browser: WebSocket, runner: crate::runtime::VncStream) {
    let (mut to_browser, mut from_browser) = browser.split();
    let (mut to_runner, mut from_runner) = runner.split();
    let down = async {
        while let Some(Ok(message)) = from_runner.next().await {
            let forwarded = match message {
                RunnerMessage::Binary(bytes) => ClientMessage::Binary(bytes.to_vec().into()),
                RunnerMessage::Close(_) => break,
                _ => continue,
            };
            if to_browser.send(forwarded).await.is_err() {
                break;
            }
        }
    };
    let up = async {
        while let Some(Ok(message)) = from_browser.next().await {
            let forwarded = match message {
                ClientMessage::Binary(bytes) => RunnerMessage::Binary(bytes.to_vec().into()),
                ClientMessage::Close(_) => break,
                _ => continue,
            };
            if to_runner.send(forwarded).await.is_err() {
                break;
            }
        }
    };
    tokio::select! { _ = down => {}, _ = up => {} }
}
