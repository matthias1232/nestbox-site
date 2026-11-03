//! Model clusters: create, change, delete, membership (contract C7).
//!
//! A cluster is installs of one model behind one provider profile. The profile's
//! endpoint is the accounting proxy's cluster path, so a sandbox sees one model
//! and the proxy spreads the requests.

use serde::Deserialize;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::model_cluster::{validate_name, ClusterStrategy, ModelCluster};
use crate::domain::provider::ProviderKind;
use crate::domain::telemetry::LiveEvent;
use crate::error::{ApiError, ErrorCode};
use crate::store::TenantScope;
use crate::store::postgres::audit::{self, NewAuditEvent};
use crate::store::postgres::model_clusters as store;
use crate::store::postgres::providers as provider_store;

/// The create body.
#[derive(Debug, Deserialize)]
struct CreateBody {
    name: String,
    model_key: String,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default)]
    install_ids: Vec<Uuid>,
}

/// The update body: name and strategy only.
#[derive(Debug, Deserialize)]
struct UpdateBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
}

/// Where sandboxes reach a cluster: the proxy base plus `/c/<id>`.
fn endpoint_for(id: Uuid) -> String {
    let base = super::provider_service::accounting_proxy_url()
        .unwrap_or_else(|| "http://accounting-proxy:18092".to_string());
    format!("{}/c/{id}", base.trim_end_matches('/'))
}

/// Serialise a cluster with its endpoint filled in.
fn view(mut cluster: ModelCluster) -> Result<serde_json::Value, ApiError> {
    cluster.endpoint = endpoint_for(cluster.id);
    serde_json::to_value(cluster).map_err(|_| ApiError::internal("Could not render the cluster."))
}

fn parse_strategy(value: &str) -> Result<ClusterStrategy, ApiError> {
    ClusterStrategy::parse(value).ok_or_else(|| {
        ApiError::validation("The strategy must be round_robin, least_loaded or failover.")
    })
}

fn not_found() -> ApiError {
    ApiError::not_found(ErrorCode::NOT_FOUND, "No such model cluster.")
}

/// Refuse an install that is not of the cluster's model or not in this data center.
async fn check_install(
    scope: &mut TenantScope<'_>,
    model_key: &str,
    install: Uuid,
) -> Result<(), ApiError> {
    match store::install_model_key(scope, install).await.map_err(ApiError::from)? {
        Some(key) if key == model_key => Ok(()),
        Some(_) => Err(ApiError::validation(
            "Every member must be an install of the cluster's model.",
        )),
        None => Err(ApiError::validation(
            "An install in the member list does not exist in this data center.",
        )),
    }
}

async fn record(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    kind: &str,
    id: Uuid,
    detail: serde_json::Value,
) -> Result<(), ApiError> {
    audit::record(
        scope,
        NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind,
            subject_type: Some("model_cluster"),
            subject_id: Some(id),
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)
}

fn announce(ctx: &ServiceContext, principal: &Principal, what: &str, id: Uuid) {
    ctx.bus.publish(LiveEvent::ModelsChanged {
        sandbox_id: None,
        environment_id: principal.environment_id,
        detail: serde_json::json!({ "cluster_id": id, "what": what }),
    });
}

/// Every cluster of the caller's data center.
pub async fn list(ctx: &ServiceContext, principal: &Principal) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let clusters = store::list(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    let items = clusters.into_iter().map(view).collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({ "items": items }))
}

/// One cluster with its members.
pub async fn get(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let cluster = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    view(cluster.ok_or_else(not_found)?)
}

/// Create a cluster and its provider profile.
///
/// Profile, cluster and the initial members are written in one transaction, so a
/// refused member leaves neither a half cluster nor an orphan profile.
pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelInstall)?;
    let body: CreateBody = serde_json::from_value(body)
        .map_err(|_| ApiError::validation("The body needs name and model_key."))?;
    let name = body.name.trim().to_string();
    validate_name(&name).map_err(ApiError::validation)?;
    let strategy = match body.strategy.as_deref() {
        Some(value) => parse_strategy(value)?,
        None => ClusterStrategy::RoundRobin,
    };

    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    if !store::model_exists(&mut scope, &body.model_key).await.map_err(ApiError::from)? {
        return Err(ApiError::not_found(ErrorCode::MODEL_NOT_FOUND, "No such model."));
    }
    for install in &body.install_ids {
        check_install(&mut scope, &body.model_key, *install).await?;
    }

    let id = Uuid::new_v4();
    let endpoint = endpoint_for(id);
    let profile = provider_store::insert(
        &mut scope,
        provider_store::NewProviderProfile {
            name: &name,
            kind: ProviderKind::LocalOpenaiCompatible,
            endpoint: &endpoint,
            model: Some(&body.model_key),
            secret_ref: None,
            labels: &serde_json::json!({}),
        },
    )
    .await
    .map_err(ApiError::from)?;
    store::create(
        &mut scope,
        id,
        &name,
        &body.model_key,
        strategy,
        Some(profile.id),
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    for install in &body.install_ids {
        store::add_member(&mut scope, id, *install, 1).await.map_err(ApiError::from)?;
    }
    record(
        &mut scope,
        principal,
        "model_cluster.created",
        id,
        serde_json::json!({ "name": name, "model_key": body.model_key, "members": body.install_ids.len() }),
    )
    .await?;
    let cluster = store::get(&mut scope, id).await.map_err(ApiError::from)?.ok_or_else(not_found)?;
    scope.commit().await.map_err(ApiError::from)?;
    announce(ctx, principal, "created", id);
    view(cluster)
}

/// Change a cluster's name or strategy.
pub async fn update(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelInstall)?;
    let body: UpdateBody = serde_json::from_value(body)
        .map_err(|_| ApiError::validation("The body may carry name and strategy."))?;
    let name = body.name.as_deref().map(str::trim);
    if let Some(name) = name {
        validate_name(name).map_err(ApiError::validation)?;
    }
    let strategy = body.strategy.as_deref().map(parse_strategy).transpose()?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let cluster = store::update(&mut scope, id, name, strategy)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(not_found)?;
    record(
        &mut scope,
        principal,
        "model_cluster.updated",
        id,
        serde_json::json!({ "name": cluster.name, "strategy": cluster.strategy.as_str() }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;
    announce(ctx, principal, "updated", id);
    view(cluster)
}

/// Delete a cluster and its provider profile.
///
/// A profile that sandboxes still use refuses with 409 `provider.in_use`; the
/// transaction then rolls back and the cluster stays.
pub async fn delete(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::ModelInstall)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let cluster = store::get(&mut scope, id).await.map_err(ApiError::from)?.ok_or_else(not_found)?;
    store::delete(&mut scope, id).await.map_err(ApiError::from)?;
    if let Some(profile) = cluster.provider_profile_id {
        super::provider_service::delete_for_install(&mut scope, profile).await?;
    }
    record(
        &mut scope,
        principal,
        "model_cluster.deleted",
        id,
        serde_json::json!({ "name": cluster.name }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;
    announce(ctx, principal, "deleted", id);
    Ok(())
}

/// Add an install to a cluster.
pub async fn add_member(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    install_id: Uuid,
    weight: Option<i32>,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelInstall)?;
    let weight = weight.unwrap_or(1);
    if !(1..=100).contains(&weight) {
        return Err(ApiError::validation("The weight must be between 1 and 100."));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let cluster = store::get(&mut scope, id).await.map_err(ApiError::from)?.ok_or_else(not_found)?;
    check_install(&mut scope, &cluster.model_key, install_id).await?;
    store::add_member(&mut scope, id, install_id, weight).await.map_err(ApiError::from)?;
    record(
        &mut scope,
        principal,
        "model_cluster.member_added",
        id,
        serde_json::json!({ "install_id": install_id, "weight": weight }),
    )
    .await?;
    let cluster = store::get(&mut scope, id).await.map_err(ApiError::from)?.ok_or_else(not_found)?;
    scope.commit().await.map_err(ApiError::from)?;
    announce(ctx, principal, "member_added", id);
    view(cluster)
}

/// Remove an install from a cluster.
pub async fn remove_member(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    install_id: Uuid,
) -> Result<(), ApiError> {
    require(principal, Permission::ModelInstall)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    if !store::remove_member(&mut scope, id, install_id).await.map_err(ApiError::from)? {
        return Err(not_found());
    }
    record(
        &mut scope,
        principal,
        "model_cluster.member_removed",
        id,
        serde_json::json!({ "install_id": install_id }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;
    announce(ctx, principal, "member_removed", id);
    Ok(())
}

/// The accounting proxy's view of a cluster (bridge token, no principal).
///
/// The tenant comes from the appliance's own data center, as for the other
/// internal accounting routes, never from the request.
pub async fn proxy_view(ctx: &ServiceContext, id: Uuid) -> Result<serde_json::Value, ApiError> {
    let environment = super::media_service::bridge_environment(ctx).await?;
    let mut scope = TenantScope::begin(&ctx.pool, environment).await.map_err(ApiError::from)?;
    let value = store::proxy_view(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    value.ok_or_else(not_found)
}
