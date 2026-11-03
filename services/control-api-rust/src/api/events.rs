//! Server-Sent Events.
//!
//! The stream is the reason the console feels live. It is also a place where a
//! tenant filter is easy to forget, so the filter is applied here, at the last
//! point before the socket, regardless of what the publisher believed.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::Sse;
use axum::response::sse::{Event, KeepAlive};
use futures::stream::Stream;
use tokio::sync::broadcast::error::RecvError;

use crate::api::principal::Auth;
use crate::domain::telemetry::LiveEvent;
use crate::error::ApiError;
use crate::events::bus::is_visible_to;
use crate::state::AppState;

/// Whether a stream should carry telemetry rows or only state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamFilter {
    Telemetry,
    State,
    /// The three chat events. Kept apart from `State`, so a console that draws
    /// the agent map does not receive a token delta per model request.
    Chats,
}

/// The telemetry stream: network, DNS, and inference rows.
pub async fn telemetry_stream(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    crate::services::require(
        &auth.principal,
        crate::domain::auth::Permission::TelemetryRead,
    )?;
    Ok(build_stream(
        state.bus().clone(),
        state.services.clone(),
        auth.principal.environment_id,
        StreamFilter::Telemetry,
        Visibility::for_principal(&state, &auth.principal),
    ))
}

/// Live chat events: turns starting, answers growing, turns stored.
///
/// A delta is a partial answer, so who may see it is decided per event by the
/// chat service: an administrator with `admin_chat_view` off sees the event
/// without its text, and a user sees only their own sandboxes.
pub async fn chats_stream(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    crate::services::require(
        &auth.principal,
        crate::domain::auth::Permission::SandboxRead,
    )?;
    Ok(build_stream(
        state.bus().clone(),
        state.services.clone(),
        auth.principal.environment_id,
        StreamFilter::Chats,
        Visibility::for_principal(&state, &auth.principal),
    ))
}

/// The state stream: sandbox, gateway, job, and rollout changes.
pub async fn state_stream(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    crate::services::require(
        &auth.principal,
        crate::domain::auth::Permission::SandboxRead,
    )?;
    Ok(build_stream(
        state.bus().clone(),
        state.services.clone(),
        auth.principal.environment_id,
        StreamFilter::State,
        Visibility::for_principal(&state, &auth.principal),
    ))
}

/// Which sandboxes a stream may talk about.
///
/// An admin sees all of them. Anyone else sees events of their own sandboxes and the shared
/// ones, the same rule as everywhere else (row-level security), decided here from the
/// database because the bus carries every event of the environment. The set is read when a
/// stream opens and again whenever an event names a sandbox it does not know, at most every
/// two seconds, so a sandbox created after the stream opened still shows up.
struct Visibility {
    restricted: Option<(sqlx::PgPool, crate::domain::auth::Principal)>,
    ids: std::collections::HashSet<uuid::Uuid>,
    loaded: Option<std::time::Instant>,
}

impl Visibility {
    fn for_principal(state: &AppState, principal: &crate::domain::auth::Principal) -> Self {
        let restricted = (principal.role != crate::domain::auth::Role::Admin)
            .then(|| (state.services.pool.clone(), principal.clone()));
        Self { restricted, ids: Default::default(), loaded: None }
    }

    async fn refresh(&mut self) {
        let Some((pool, principal)) = &self.restricted else { return };
        if let Some(ids) = crate::services::sandbox_service::visible_ids(pool, principal).await {
            self.ids = ids;
        }
        self.loaded = Some(std::time::Instant::now());
    }

    async fn allows(&mut self, sandbox_id: Option<uuid::Uuid>) -> bool {
        if self.restricted.is_none() {
            return true;
        }
        // An event about no sandbox (a gateway, a build) is for admins.
        let Some(id) = sandbox_id else { return false };
        if self.loaded.is_none() {
            self.refresh().await;
        }
        if !self.ids.contains(&id)
            && self.loaded.is_some_and(|at| at.elapsed() > std::time::Duration::from_secs(2))
        {
            self.refresh().await;
        }
        self.ids.contains(&id)
    }
}

fn event_sandbox(event: &LiveEvent) -> Option<uuid::Uuid> {
    match event {
        LiveEvent::Telemetry(inner) => inner.sandbox_id(),
        LiveEvent::SandboxStateChanged { sandbox_id, .. } => Some(*sandbox_id),
        LiveEvent::SandboxControlChanged { sandbox_id, .. } => *sandbox_id,
        LiveEvent::ModelsChanged { sandbox_id, .. } => *sandbox_id,
        LiveEvent::Alert { sandbox_id, .. } => *sandbox_id,
        LiveEvent::ChatStarted { .. }
        | LiveEvent::ChatDelta { .. }
        | LiveEvent::ChatDone { .. } => event.chat_sandbox(),
        _ => None,
    }
}

fn build_stream(
    bus: std::sync::Arc<dyn crate::events::EventBus>,
    services: crate::services::ServiceContext,
    environment_id: uuid::Uuid,
    filter: StreamFilter,
    visibility: Visibility,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let receiver = bus.subscribe();

    // `unfold` keeps the receiver as the stream's state, so each poll resumes
    // where the last one stopped. A lagged subscriber skips ahead rather than
    // being disconnected, because a browser that missed a burst is better served
    // by the next event than by a reconnect loop.
    let stream = futures::stream::unfold((receiver, visibility, services), move |(mut receiver, mut visibility, services)| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if !is_visible_to(&event, environment_id) {
                        continue;
                    }
                    if !matches_filter(&event, filter) {
                        continue;
                    }
                    if !visibility.allows(event_sandbox(&event)).await {
                        continue;
                    }
                    // A chat event may carry text an administrator is not
                    // allowed to read. The chat service decides; `None` drops it.
                    let event = match (&event, &visibility.restricted) {
                        (event, Some((_, principal))) if event.is_chat() => {
                            match crate::services::chat_service::redact_live_for(
                                &services,
                                principal,
                                event.clone(),
                            )
                            .await
                            {
                                Some(redacted) => redacted,
                                None => continue,
                            }
                        }
                        _ => event,
                    };
                    let name = event_name(&event);
                    match serde_json::to_string(&event) {
                        Ok(data) => {
                            return Some((Ok(Event::default().event(name).data(data)), (receiver, visibility, services)));
                        }
                        Err(_) => continue,
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "sse subscriber lagged; skipping ahead");
                    continue;
                }
                Err(RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(
        // A comment frame every 15 s keeps an idle connection alive through
        // proxies that drop silent sockets.
        KeepAlive::new().interval(std::time::Duration::from_secs(15)),
    )
}

/// The event name, which is part of the published contract (ADR 0015).
fn event_name(event: &LiveEvent) -> &'static str {
    match event {
        LiveEvent::Telemetry(inner) => inner.kind().stream_name(),
        LiveEvent::SandboxStateChanged { .. } => "sandbox",
        LiveEvent::GatewayHealthChanged { .. } => "gateway",
        LiveEvent::JobProgress { .. } => "job",
        LiveEvent::PolicyRollout { .. } => "policy_rollout",
        LiveEvent::SandboxControlChanged { .. } => "control",
        LiveEvent::ModelsChanged { .. } => "models",
        LiveEvent::NodeTunnelChanged { .. } => "node",
        LiveEvent::Alert { .. } => "alert",
        LiveEvent::ChatStarted { .. } => "chat_started",
        LiveEvent::ChatDelta { .. } => "chat_delta",
        LiveEvent::ChatDone { .. } => "chat_done",
    }
}

fn matches_filter(event: &LiveEvent, filter: StreamFilter) -> bool {
    match filter {
        StreamFilter::Telemetry => matches!(event, LiveEvent::Telemetry(_)),
        StreamFilter::State => !matches!(event, LiveEvent::Telemetry(_)) && !event.is_chat(),
        StreamFilter::Chats => event.is_chat(),
    }
}

/// A helper for the console: read one batch of events and close.
///
/// Useful behind a proxy that buffers, and for a smoke test that must prove the
/// stream produces anything at all.
pub async fn drain_once(
    state: &AppState,
    environment_id: uuid::Uuid,
    limit: usize,
) -> Vec<LiveEvent> {
    let mut receiver = state.bus().subscribe();
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while out.len() < limit {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, receiver.recv()).await {
            Ok(Ok(event)) if is_visible_to(&event, environment_id) => out.push(event),
            Ok(Ok(_)) => continue,
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn state_event(environment_id: Uuid) -> LiveEvent {
        LiveEvent::SandboxStateChanged {
            sandbox_id: Uuid::new_v4(),
            environment_id,
            status: "running".into(),
            version: 2,
        }
    }

    #[test]
    fn the_state_filter_excludes_telemetry() {
        let event = state_event(Uuid::nil());
        assert!(matches_filter(&event, StreamFilter::State));
        assert!(!matches_filter(&event, StreamFilter::Telemetry));
    }

    #[test]
    fn event_names_match_the_published_contract() {
        assert_eq!(event_name(&state_event(Uuid::nil())), "sandbox");
        let gateway = LiveEvent::GatewayHealthChanged {
            gateway_id: Uuid::nil(),
            environment_id: Uuid::nil(),
            status: "healthy".into(),
            drifted: false,
        };
        assert_eq!(event_name(&gateway), "gateway");
    }
}
