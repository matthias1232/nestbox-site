//! Model catalog, hardware, and install endpoints.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::api::principal::Auth;
use crate::domain::model::HardwareProfile;
use crate::domain::node::Node;
use crate::error::ApiError;
use crate::services::model_service;
use crate::services::model_service::CatalogQuery as CatalogFilter;
use crate::state::AppState;

/// Query parameters of the catalog.
///
/// Every filter narrows; omitting one keeps everything. `fits` is the verdict
/// switch the console's "fits this machine" toggle sends, and it selects from
/// the verdicts the server already produced — the client never re-classifies.
#[derive(Debug, Deserialize)]
pub struct CatalogQuery {
    /// Force a fresh hardware probe instead of using the cached result.
    #[serde(default)]
    pub refresh_hardware: bool,
    /// The node the fit is for; the local node when omitted.
    pub node: Option<String>,
    /// Only this family (exact, case-sensitive, as stored).
    pub family: Option<String>,
    /// Only this publisher.
    pub publisher: Option<String>,
    /// `cuda`, `cpu`, or `hybrid`.
    pub runtime: Option<String>,
    /// Only models that run without a GPU.
    #[serde(default)]
    pub cpu_only: bool,
    /// Only models that need a unified-memory pool.
    #[serde(default)]
    pub unified_only: bool,
    /// `true` keeps what fits this machine, `false` keeps the rest.
    pub fits: Option<bool>,
}

impl CatalogQuery {
    fn filter(&self) -> CatalogFilter {
        CatalogFilter {
            node: self.node.clone(),
            family: self.family.clone(),
            publisher: self.publisher.clone(),
            runtime: self.runtime.clone(),
            cpu_only: self.cpu_only,
            unified_only: self.unified_only,
            fits: self.fits,
        }
    }
}

/// The machine as JSON, in one shape for the catalog envelope, the hardware
/// endpoint, and a node probe, so the three cannot drift apart.
///
/// `container_gpu` sits next to the GPUs on purpose: a card the node has but a
/// container cannot be given is not capacity, and a reader should not have to
/// join two responses to see that.
pub(crate) fn hardware_json(profile: &HardwareProfile, node: &Node) -> serde_json::Value {
    serde_json::json!({
        "node": node.name,
        "node_id": node.id,
        "node_status": node.status,
        "container_gpu": node.container_gpu,
        "hostname": profile.hostname,
        "cpu_model": profile.cpu_model,
        "cpu_cores": profile.cpu_cores,
        "cpu_threads": profile.cpu_threads,
        "system_ram_bytes": profile.system_ram_bytes,
        "gpus": profile.gpus,
        "total_vram_bytes": profile.total_vram_bytes,
        "unified_memory_bytes": profile.unified_memory_bytes,
        "unified_memory_source": profile.unified_memory_source,
        "is_unified_memory": profile.is_unified(),
        "addressable_memory_bytes": profile.addressable_memory_bytes(),
        "addressable_ram_bytes": profile.addressable_ram_bytes(),
        "gpu_count": profile.gpu_count(),
        "has_gpu": profile.has_gpu(),
        "cuda_version": profile.cuda_version,
        "driver_version": profile.driver_version,
        "detected_at": profile.detected_at,
        "note": profile.detection_note,
    })
}

/// The catalog, classified against what one node has left.
pub async fn catalog(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let view = model_service::catalog(
        &state.services,
        &auth.principal,
        query.refresh_hardware,
        &query.filter(),
    )
    .await?;
    Ok(Json(serde_json::json!({
        "items": view.items,
        "hardware": hardware_json(&view.hardware, &view.node),
        "budget": view.budget,
        "node": {
            "id": view.node.id,
            "name": view.node.name,
            "is_local": view.node.is_local,
            "container_gpu": view.node.container_gpu,
        },
    })))
}

/// Only the installed models.
pub async fn installed(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let models = model_service::installed(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": models })))
}

/// The detected hardware of one node.
pub async fn hardware(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<CatalogQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let measured = model_service::hardware(
        &state.services,
        &auth.principal,
        query.refresh_hardware,
        query.node.as_deref(),
    )
    .await?;
    Ok(Json(hardware_json(&measured.profile, &measured.node)))
}

/// Counts for the filter chips.
pub async fn facets(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let facets = model_service::facets(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::to_value(facets).unwrap_or_else(
        |_| serde_json::json!({}),
    )))
}

#[derive(Debug, Deserialize)]
pub struct RequirementsQuery {
    /// The catalog key of the model.
    pub key: String,
}

/// One model as a requirement profile, with the verdict for this machine.
pub async fn requirements(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<RequirementsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let view = model_service::requirements(&state.services, &auth.principal, &query.key).await?;
    Ok(Json(serde_json::to_value(view).unwrap_or_else(
        |_| serde_json::json!({}),
    )))
}

/// Which installs to list.
#[derive(Debug, Deserialize)]
pub struct InstallsQuery {
    /// Only installs on this node; every node when omitted.
    pub node: Option<String>,
}

/// Installs per node with their reservation and provider profile.
pub async fn installs(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<InstallsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let items = model_service::installs(
        &state.services,
        &auth.principal,
        query.node.as_deref(),
    )
    .await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// Refresh the catalog from the public sources as a job.
///
/// The job is created first so the run has an id, then executed in the request:
/// the operator gets the per-source counts instead of a job id that nothing in
/// this process would pick up. The same function is what a scheduled run calls
/// with the job its schedule created.
pub async fn sync(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let summary = crate::services::catalog_sync::start(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::to_value(summary).unwrap_or_else(
        |_| serde_json::json!({}),
    )))
}

/// The body of an install or uninstall: which node, or the local one.
#[derive(Debug, Default, Deserialize)]
pub struct NodeRequest {
    pub node: Option<String>,
    /// Install even when the hardware verdict or the free budget says the node
    /// cannot hold it. The refusal reason is returned and recorded as a warning.
    #[serde(default)]
    pub force: bool,
}

/// Install a model on a node.
pub async fn install(
    State(state): State<AppState>,
    auth: Auth,
    Path(key): Path<String>,
    body: Option<Json<NodeRequest>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (node, force) = body
        .map(|Json(request)| (request.node, request.force))
        .unwrap_or((None, false));
    Ok(Json(
        model_service::install(&state.services, &auth.principal, &key, node.as_deref(), force)
            .await?,
    ))
}

/// Install a model on many nodes as one job (package B04).
pub async fn install_bulk(
    State(state): State<AppState>,
    auth: Auth,
    Path(key): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    let value = crate::services::model_bulk_service::install_bulk(&state.services, &auth.principal, &key, body).await?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(value)))
}

/// Uninstall a model from a node.
pub async fn uninstall(
    State(state): State<AppState>,
    auth: Auth,
    Path(key): Path<String>,
    body: Option<Json<NodeRequest>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let node = body.and_then(|Json(request)| request.node);
    Ok(Json(
        model_service::uninstall(&state.services, &auth.principal, &key, node.as_deref()).await?,
    ))
}

/// The body of an install edit.
#[derive(Debug, Default, Deserialize)]
pub struct UpdateInstallRequest {
    pub node: Option<String>,
    /// New reserved RAM in bytes.
    pub reserved_ram_bytes: Option<i64>,
    /// New reserved VRAM in bytes.
    pub reserved_vram_bytes: Option<i64>,
    /// Switch the model on or off.
    pub enabled: Option<bool>,
    /// Runtime flags: context, GPU layers, batching, KV cache, threads, ...
    /// A change to a running model restarts it with the new command.
    pub settings: Option<crate::domain::model_install::ModelTuning>,
}

/// Query of the log view.
#[derive(Debug, Deserialize)]
pub struct LogsQuery {
    pub node: Option<String>,
    /// How many trailing lines; the server clamps to 1..2000.
    pub tail: Option<u32>,
}

/// The last lines a served model wrote.
pub async fn logs(
    State(state): State<AppState>,
    auth: Auth,
    Path(key): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        model_service::logs(
            &state.services,
            &auth.principal,
            &key,
            query.node.as_deref(),
            query.tail.unwrap_or(200),
        )
        .await?,
    ))
}

/// Query of the metrics view.
#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    pub node: Option<String>,
    /// How many samples of the series; the server clamps to 1..1000.
    pub limit: Option<i64>,
}

/// Sample the served model now and return the stored series.
pub async fn metrics(
    State(state): State<AppState>,
    auth: Auth,
    Path(key): Path<String>,
    Query(query): Query<MetricsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        model_service::metrics(
            &state.services,
            &auth.principal,
            &key,
            query.node.as_deref(),
            query.limit.unwrap_or(120),
        )
        .await?,
    ))
}

/// Edit an installed model: reserved memory and the on/off switch.
pub async fn update_install(
    State(state): State<AppState>,
    auth: Auth,
    Path(key): Path<String>,
    Json(body): Json<UpdateInstallRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        model_service::update_install(
            &state.services,
            &auth.principal,
            &key,
            body.node.as_deref(),
            model_service::InstallChange {
                reserved_ram_bytes: body.reserved_ram_bytes,
                reserved_vram_bytes: body.reserved_vram_bytes,
                enabled: body.enabled,
                settings: body.settings,
            },
        )
        .await?,
    ))
}
