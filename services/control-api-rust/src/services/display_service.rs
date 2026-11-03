//! The display layout of a desktop sandbox, read and applied through
//! `nestbox-display` inside it (U09/U10a).
//!
//! The control plane is not the owner of the screen: the tool inside the sandbox
//! is. This service only decides who may ask, checks the layout against the
//! limits, hands it to the tool as one argument, and records what happened. That
//! keeps one implementation of the rules — the tool's — and one place where the
//! same rules are refused early (the domain's `validate`).
//!
//! Two things follow from "the sandbox is the owner":
//!
//! * the current layout is read by executing the tool, never cached here, so a
//!   layout an agent changed through the desktop-control MCP server is what the
//!   console sees;
//! * a write carries `expected_applied_at`. If the tool reports a different
//!   stamp, somebody else changed the screen in between, and overwriting it
//!   silently would undo their work — that is a `409 display.changed` with the
//!   layout that is really there.

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::display::{self, AppliedDisplay, DisplayLayout};
use crate::domain::sandbox::{Sandbox, SandboxStatus};
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::TenantScope;
use crate::store::postgres::audit;
use crate::store::postgres::sandboxes as sandbox_store;

/// How long one tool call may take. `set` has to finish in under a second on the
/// box; the rest is the runtime's own latency, and a stuck X server must not hold
/// the request open.
const TOOL_TIMEOUT_SECONDS: u32 = 10;

/// The exit code a shell answers when the program does not exist.
///
/// A sandbox without the desktop pack has no `nestbox-display`; that is the one
/// case the console turns into "this sandbox has no desktop".
const NOT_FOUND_EXIT: i64 = 127;

/// The layout that is on the sandbox's screen now.
///
/// `SandboxRead` is enough: a viewer may already watch the same screen through
/// the view-only desktop stream, so hiding the geometry from them would protect
/// nothing. Changing it is a different matter and needs `SandboxExec`.
pub async fn get(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<AppliedDisplay, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let sandbox = load_sandbox(ctx, principal, sandbox_id).await?;
    run_get(ctx, &sandbox).await
}

/// Apply a whole layout, then answer with what the sandbox now has.
///
/// `expected_applied_at` is the stamp the caller last read. When it is given and
/// the sandbox reports another one, the answer is `409 display.changed` with the
/// current layout rather than a silent overwrite.
pub async fn set(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    layout: &DisplayLayout,
    expected_applied_at: Option<&str>,
) -> Result<AppliedDisplay, ApiError> {
    // Driving the screen is the same power as typing into the desktop, so the
    // same permission decides it.
    require(principal, Permission::SandboxExec)?;
    if let Err(errors) = display::validate(layout) {
        let message = errors.join("; ");
        let details = serde_json::json!({ "errors": errors });
        return Err(ApiError::validation(message).with_details(details));
    }
    let sandbox = load_sandbox(ctx, principal, sandbox_id).await?;

    if let Some(expected) = expected_applied_at.filter(|value| !value.trim().is_empty()) {
        let current = run_get(ctx, &sandbox).await?;
        let current_stamp = current.applied_at.clone().unwrap_or_default();
        if current_stamp != expected {
            let details = serde_json::json!({
                "layout": &current.layout,
                "framebuffer": current.framebuffer,
                "applied_at": current.applied_at,
                "applied_by": current.applied_by,
            });
            let error = ApiError::conflict(
                ErrorCode::DISPLAY_CHANGED,
                "The display layout changed since this one was read.",
            );
            return Err(error.with_details(details));
        }
    }

    let body = serde_json::to_string(layout).map_err(|error| {
        ApiError::internal("The layout could not be sent to the sandbox.")
            .with_internal(error.to_string())
    })?;
    let command = vec![
        "env".to_string(),
        format!("NESTBOX_DISPLAY_ACTOR={}", principal.display_name),
        "nestbox-display".to_string(),
        "set".to_string(),
        body,
    ];
    // `set` answers with the layout it read back from X, which is the new `get`;
    // a second call would only repeat the same question.
    let applied = run_tool(ctx, &sandbox.name, command).await?;

    let detail = serde_json::json!({
        "mode": &applied.layout.mode,
        "monitors": &applied.layout.monitors,
        "applied_at": applied.applied_at,
        "applied_by": applied.applied_by,
    });
    {
        let mut scope = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        audit::record(
            &mut scope,
            audit::NewAuditEvent {
                actor_id: Some(principal.user_id),
                actor_label: &principal.display_name,
                kind: "sandbox.display",
                subject_type: Some("sandbox"),
                subject_id: Some(sandbox_id),
                detail: &detail,
                request_id: None,
            },
        )
        .await
        .map_err(ApiError::from)?;
        scope.commit().await.map_err(ApiError::from)?;
    }

    // The console redraws its monitor tabs from this, and an agent's own change
    // reaches an open desktop view the same way.
    ctx.bus
        .publish(crate::domain::telemetry::LiveEvent::SandboxControlChanged {
            sandbox_id: Some(sandbox_id),
            environment_id: sandbox.environment_id,
            what: "display".to_string(),
            detail,
        });
    Ok(applied)
}

/// The sandbox row, after checking that the caller may see it at all.
///
/// A stopped sandbox has no X server, so there is no layout to read or apply;
/// saying so is more useful than a runtime error from the runner.
async fn load_sandbox(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<Sandbox, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => {
                ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox.")
            }
            other => ApiError::from(other),
        })?;
    scope.commit().await.map_err(ApiError::from)?;
    if sandbox.status != SandboxStatus::Running {
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            format!(
                "A desktop needs a running sandbox; this one is '{}'.",
                sandbox.status.as_str()
            ),
        ));
    }
    Ok(sandbox)
}

/// Read the current layout with `nestbox-display get`.
async fn run_get(ctx: &ServiceContext, sandbox: &Sandbox) -> Result<AppliedDisplay, ApiError> {
    run_tool(
        ctx,
        &sandbox.name,
        vec!["nestbox-display".to_string(), "get".to_string()],
    )
    .await
}

/// Run one `nestbox-display` command and read its one JSON line.
///
/// A missing tool is a `409 sandbox.no_desktop` because the caller's next step is
/// to use a sandbox with the desktop pack, not to retry. Any other refusal is a
/// `422` carrying the tool's own sentence: the tool writes those sentences for a
/// person, and the control plane cannot say it better.
async fn run_tool(
    ctx: &ServiceContext,
    sandbox_name: &str,
    command: Vec<String>,
) -> Result<AppliedDisplay, ApiError> {
    let result = ctx
        .runtime
        .exec(sandbox_name, &command, TOOL_TIMEOUT_SECONDS)
        .await
        .map_err(|error| {
            ApiError::unavailable(
                ErrorCode::NODE_RUNTIME_UNAVAILABLE,
                "The sandbox's display tool could not be reached.",
            )
            .with_internal(error.to_string())
        })?;

    match result.exit_code {
        Some(0) => parse_answer(&result.output),
        Some(NOT_FOUND_EXIT) => Err(ApiError::conflict(
            ErrorCode::SANDBOX_NO_DESKTOP,
            "This sandbox has no desktop: its image does not carry nestbox-display.",
        )),
        Some(code) => Err(ApiError::validation(tool_message(&result.output))
            .with_internal(format!("nestbox-display exited with {code}"))),
        None => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            "The sandbox's display tool ended without an exit code.",
        )),
    }
}

/// The one JSON line the tool printed, as an applied layout.
///
/// The runtime merges stderr into stdout, so the line is looked for from the end
/// and a line that is not JSON is skipped rather than guessed at.
fn parse_answer(output: &str) -> Result<AppliedDisplay, ApiError> {
    for line in output.lines().rev() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if value.get("error").is_some() {
            return Err(ApiError::validation(tool_message(output)));
        }
        if let Ok(applied) = serde_json::from_value::<AppliedDisplay>(value) {
            // A layout without a monitor is not a layout; an answer that lost its
            // monitors would otherwise look like an empty desktop.
            if !applied.layout.monitors.is_empty() {
                return Ok(applied);
            }
        }
    }
    Err(ApiError::unavailable(
        ErrorCode::NODE_RUNTIME_UNAVAILABLE,
        "The sandbox's display tool answered with something that is not a layout.",
    ))
}

/// The tool's own error sentence, or its output when it printed none.
fn tool_message(output: &str) -> String {
    for line in output.lines().rev() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(message) = value.get("error").and_then(|error| error.as_str()) {
                return format!("The display tool refused the layout: {message}");
            }
        }
    }
    let text = output.trim();
    if text.is_empty() {
        "The display tool refused the layout without saying why.".to_string()
    } else {
        format!("The display tool refused the layout: {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_answer_that_is_a_layout_is_read() {
        let output = r#"{"mode":"separate","monitors":[{"id":"m1","name":"Monitor 1","width":1440,"height":900,"x":0,"y":0,"primary":true}],"framebuffer":[1440,900],"applied_at":"2026-11-03T10:00:00Z","applied_by":"admin"}"#;
        let applied = parse_answer(output).expect("a layout is an answer");
        assert_eq!(applied.layout.monitors.len(), 1);
        assert_eq!(applied.applied_at.as_deref(), Some("2026-11-03T10:00:00Z"));
    }

    #[test]
    fn the_json_line_is_found_after_noise_on_stderr() {
        let output = "Warning: something\n{\"mode\":\"stretched\",\"monitors\":[{\"id\":\"m1\",\"width\":800,\"height\":600,\"x\":0,\"y\":0}],\"framebuffer\":[800,600]}\n";
        let applied = parse_answer(output).expect("the last JSON line wins");
        assert_eq!(applied.layout.mode, display::MODE_STRETCHED);
    }

    #[test]
    fn an_answer_without_a_monitor_is_not_a_layout() {
        let error = parse_answer("{\"mode\":\"separate\",\"monitors\":[]}")
            .expect_err("an empty layout is not an answer");
        assert_eq!(error.status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn a_tool_error_is_a_validation_error_with_the_tools_sentence() {
        let error = parse_answer("{\"error\": \"mode must be separate or stretched\"}")
            .expect_err("an error line is not a layout");
        assert_eq!(error.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error.message.contains("mode must be separate or stretched"));
    }

    #[test]
    fn a_refusal_without_a_json_line_keeps_the_plain_output() {
        assert!(tool_message("xrandr: cannot find mode").contains("xrandr"));
        assert!(tool_message("   ").contains("without saying why"));
    }
}
