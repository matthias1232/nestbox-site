//! Making the models a sandbox was granted ready before it is used (ADR 0057).
//!
//! A local model on a node is asked one tiny question (one token) before the
//! sandbox is reported ready, so the first real request does not wait for a model
//! load on an expensive machine. Starting a stopped model stays an administrator's
//! action: this module reports what it found and never starts a job behind the
//! operator's back.
//!
//! Filled by the coordinator after package B22's worker ran out of model credits.

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::error::{ApiError, ErrorCode};
use crate::store::TenantScope;

/// How long a model may take to answer its warm-up question.
const WARM_TIMEOUT_SECONDS: u64 = 120;

/// How long a starting install is waited for, in five-second steps.
const READY_POLL_STEPS: u32 = 60;

/// The request that makes a model load without generating anything worth paying for.
fn ping_body(model: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1,
        "stream": false,
    })
}

/// Ask one model one question; returns the milliseconds it took.
async fn ping(endpoint: &str, model: &str) -> Result<u64, String> {
    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(WARM_TIMEOUT_SECONDS))
        .build()
        .map_err(|error| error.to_string())?;
    let started = std::time::Instant::now();
    let response = client
        .post(&url)
        .json(&ping_body(model))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status().is_success() {
        Ok(started.elapsed().as_millis() as u64)
    } else {
        Err(format!("{url} answered {}", response.status()))
    }
}

fn announce(ctx: &ServiceContext, environment_id: Uuid, sandbox_id: Option<Uuid>, model: &str, state: &str, detail: serde_json::Value) {
    ctx.bus.publish(crate::domain::telemetry::LiveEvent::ModelsChanged {
        sandbox_id,
        environment_id,
        detail: serde_json::json!({ "model": model, "state": state, "detail": detail }),
    });
}

/// Start and warm the granted local models of a sandbox when it starts.
pub async fn ensure_ready_for_sandbox(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
) -> Result<(), ApiError> {
    let targets = {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        let targets = crate::store::postgres::model_readiness::warm_targets(&mut scope, sandbox_id)
            .await
            .map_err(ApiError::from)?;
        scope.commit().await.map_err(ApiError::from)?;
        targets
    };
    for target in targets.into_iter().filter(|target| target.kind == "local") {
        announce(ctx, environment_id, Some(sandbox_id), &target.upstream_model, "warming", serde_json::json!({}));
        let mut waited = 0;
        while target.install_state.as_deref() != Some("serving") && waited < READY_POLL_STEPS {
            // The install settles on its own (the model service's reconcile pass);
            // this loop only waits for it, it never starts one.
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            waited += 1;
        }
        match ping(&target.endpoint, &target.upstream_model).await {
            Ok(ms) => announce(
                ctx,
                environment_id,
                Some(sandbox_id),
                &target.upstream_model,
                "ready",
                serde_json::json!({ "ms": ms }),
            ),
            Err(error) => {
                tracing::warn!(model = %target.upstream_model, %error, "model not warm");
                announce(
                    ctx,
                    environment_id,
                    Some(sandbox_id),
                    &target.upstream_model,
                    "failed",
                    serde_json::json!({ "error": error }),
                );
            }
        }
    }
    Ok(())
}

/// Warm the granted local models of a sandbox on the node it moves to.
pub async fn prewarm_on_node(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
    node_id: Uuid,
) -> Result<(), ApiError> {
    let targets = {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        let targets = crate::store::postgres::model_readiness::warm_targets(&mut scope, sandbox_id)
            .await
            .map_err(ApiError::from)?;
        scope.commit().await.map_err(ApiError::from)?;
        targets
    };
    for target in targets.into_iter().filter(|target| target.kind == "local") {
        match target.node_id {
            Some(on) if on == node_id => {
                if let Err(error) = ping(&target.endpoint, &target.upstream_model).await {
                    tracing::warn!(model = %target.upstream_model, %error, "prewarm failed");
                }
            }
            Some(_) => announce(
                ctx,
                environment_id,
                Some(sandbox_id),
                &target.upstream_model,
                "missing",
                serde_json::json!({ "reason": "the model is installed on another node" }),
            ),
            None => {}
        }
    }
    Ok(())
}

/// Warm one model on demand.
pub async fn warm(
    ctx: &ServiceContext,
    principal: &Principal,
    model_key: &str,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let target = crate::store::postgres::model_readiness::target_by_model(&mut scope, model_key)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::not_found(ErrorCode::MODEL_NOT_FOUND, "No such model in this environment."))?;
    scope.commit().await.map_err(ApiError::from)?;
    if target.kind == "local" && target.install_state.as_deref() != Some("serving") {
        return Ok(serde_json::json!({
            "model_key": model_key,
            "state": "stopped",
            "hint": "The model is not serving. Start it on the Models page, then warm it again.",
        }));
    }
    match ping(&target.endpoint, &target.upstream_model).await {
        Ok(ms) => Ok(serde_json::json!({ "model_key": model_key, "state": "ready", "ms": ms })),
        Err(error) => Ok(serde_json::json!({ "model_key": model_key, "state": "failed", "error": error })),
    }
}

/// Keep the models marked `keep_warm` answering.
pub async fn keep_warm_tick(ctx: &ServiceContext) -> Result<(), ApiError> {
    let environments: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments")
        .fetch_all(&ctx.pool)
        .await
        .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
    for environment_id in environments {
        let targets = {
            let mut scope = TenantScope::begin(&ctx.pool, environment_id)
                .await
                .map_err(ApiError::from)?;
            let targets = crate::store::postgres::model_readiness::keep_warm(&mut scope)
                .await
                .map_err(ApiError::from)?;
            scope.commit().await.map_err(ApiError::from)?;
            targets
        };
        for target in targets.into_iter().filter(|target| target.kind == "local") {
            if let Err(error) = ping(&target.endpoint, &target.upstream_model).await {
                tracing::debug!(model = %target.upstream_model, %error, "keep-warm ping failed");
            }
        }
    }
    Ok(())
}
