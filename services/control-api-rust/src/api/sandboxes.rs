//! Sandbox endpoints.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::pagination::ListParams;
use crate::error::ApiError;
use crate::services::access_service;
use crate::services::node_service;
use crate::services::sandbox_service::{self, CreateSandbox, LifecycleAction};
use crate::state::AppState;

/// Query parameters for the sandbox list.
///
/// Filters use the `filter[field]=value` shape so a new filter does not need a
/// new query parameter.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    pub sort: Option<String>,
    pub direction: Option<String>,
    pub search: Option<String>,
    /// Only sandboxes on this node, by name or id. A node that does not exist
    /// is a 404 rather than an empty page: an empty page means "none there".
    pub node: Option<String>,
    /// `filter[status]=running` and friends.
    #[serde(flatten)]
    pub filters: std::collections::BTreeMap<String, String>,
}

/// List sandboxes with cursor paging and facets.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ListQuery>,
) -> Result<Json<crate::domain::pagination::Page<crate::domain::sandbox::SandboxSummary>>, ApiError> {
    // Only `filter[...]` keys become filters; anything else is a typo and is
    // ignored rather than silently broadening the query.
    let mut filters: std::collections::BTreeMap<String, String> = query
        .filters
        .iter()
        .filter_map(|(key, value)| {
            key.strip_prefix("filter[")
                .and_then(|rest| rest.strip_suffix(']'))
                .map(|field| (field.to_string(), value.clone()))
        })
        .collect();

    // `node_id` is the key the store reads, and it is set below from a node the
    // service resolved. A client-supplied one never reaches SQL.
    filters.remove("node_id");
    // `?node=` names a node; an unknown one is a 404, not an empty page.
    if let Some(reference) = query
        .node
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let node = node_service::resolve(&state.services, &auth.principal, reference).await?;
        filters.insert("node_id".to_string(), node.id.to_string());
    }

    let params = ListParams::from_query(
        query.cursor,
        query.limit,
        query.sort,
        query.direction,
        query.search,
        filters,
    );

    let page = sandbox_service::list(&state.services, &auth.principal, &params).await?;
    Ok(Json(page))
}

/// Read one sandbox, merging what the runtime reports.
pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::domain::sandbox::SandboxDetail>, ApiError> {
    let detail = sandbox_service::get(&state.services, &auth.principal, id).await?;
    Ok(Json(detail))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRequest {
    /// The lead of the stack this sandbox joins; absent for a standalone one.
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub pack_key: String,
    /// Optional: empty means the pack's default image.
    #[serde(default)]
    pub image: String,
    pub image_digest: Option<String>,
    /// Optional: empty means the pack's default command.
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default = "default_memory")]
    pub memory_limit: String,
    #[serde(default = "default_cpu")]
    pub cpu_limit: f64,
    #[serde(default = "default_pids")]
    pub pids_limit: i64,
    /// GPUs to hand to the container. Omitted means none; a node that cannot hand
    /// them out refuses the request.
    #[serde(default)]
    pub gpu_count: Option<u32>,
    /// The node to place the sandbox on, by name. Omitted means the local node.
    #[serde(default)]
    pub node: Option<String>,
    /// `gateway` (default) or `own_account`: whether the agent uses the appliance's model
    /// key or the user signs in themselves inside the sandbox.
    #[serde(default)]
    pub login: Option<String>,
    /// The egress mode this sandbox is created for; it decides the runtime.
    /// `transparent` and `hybrid` run on Docker, `forward_proxy` on the gateway.
    #[serde(default)]
    pub egress_mode: Option<String>,
}

fn default_memory() -> String {
    "2Gi".to_string()
}
fn default_cpu() -> f64 {
    1.0
}
fn default_pids() -> i64 {
    512
}

/// Create a sandbox.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<CreateRequest>,
) -> Result<(StatusCode, Json<crate::domain::sandbox::Sandbox>), ApiError> {
    let sandbox = sandbox_service::create(
        &state.services,
        &auth.principal,
        CreateSandbox {
            name: body.name,
            pack_key: body.pack_key,
            image: body.image,
            image_digest: body.image_digest,
            command: body.command,
            memory_limit: body.memory_limit,
            cpu_limit: body.cpu_limit,
            pids_limit: body.pids_limit,
            gpu_count: body.gpu_count,
            node: body.node,
            parent_id: body.parent_id,
            login: body.login,
            egress_mode: body.egress_mode,
        },
    )
    .await?;

    // Give the new sandbox a way in: an Ed25519 key pair and a port per
    // published service. A failure here leaves a sandbox without SSH rather than
    // failing a create that already happened, so it is logged and the sandbox is
    // still returned — the Access tab can then show what is missing.
    if let Err(error) = access_service::provision(&state.services, &auth.principal, &sandbox).await {
        tracing::error!(
            sandbox = %sandbox.name,
            code = error.code.0,
            "access provisioning failed; this sandbox has no SSH key or ports"
        );
    }

    Ok((StatusCode::CREATED, Json(sandbox)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ActionRequest {
    pub action: String,
}

/// Run a lifecycle action.
pub async fn action(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<ActionRequest>,
) -> Result<Json<crate::domain::sandbox::Sandbox>, ApiError> {
    let action = LifecycleAction::parse(&body.action).ok_or_else(|| {
        ApiError::validation("Unknown action; expected start, stop or restart.")
    })?;
    let sandbox = sandbox_service::act(&state.services, &auth.principal, id, action).await?;
    Ok(Json(sandbox))
}

/// Delete a sandbox.
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    sandbox_service::delete(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    #[serde(default = "default_tail")]
    pub tail: u32,
}

fn default_tail() -> u32 {
    200
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct LogResponse {
    pub sandbox_id: Uuid,
    pub content: String,
}

/// Read a sandbox's logs.
pub async fn logs(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<LogQuery>,
) -> Result<Json<LogResponse>, ApiError> {
    let content = sandbox_service::logs(
        &state.services,
        &auth.principal,
        id,
        query.tail.clamp(1, 5000),
    )
    .await?;
    Ok(Json(LogResponse {
        sandbox_id: id,
        content,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ExecRequest {
    /// An argument vector, e.g. `["bash", "-lc", "hermes --version"]`.
    pub command: Vec<String>,
    #[serde(default = "default_exec_timeout")]
    pub timeout_seconds: u32,
}

fn default_exec_timeout() -> u32 {
    60
}

/// Run one command inside a running sandbox and return its output.
pub async fn exec(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<ExecRequest>,
) -> Result<Json<crate::domain::runtime::ExecResult>, ApiError> {
    let result = sandbox_service::exec(
        &state.services,
        &auth.principal,
        id,
        body.command,
        body.timeout_seconds,
    )
    .await?;
    Ok(Json(result))
}

/// The body of a stack change.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StackRequest {
    /// The lead to join, or `null` to leave the stack.
    pub parent_id: Option<Uuid>,
}

/// Put a sandbox into a stack, or take it out.
pub async fn put_stack(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<StackRequest>,
) -> Result<Json<crate::domain::sandbox::Sandbox>, ApiError> {
    let sandbox =
        sandbox_service::set_stack(&state.services, &auth.principal, id, body.parent_id).await?;
    Ok(Json(sandbox))
}


#[derive(Debug, Deserialize, ToSchema)]
pub struct ResourcesRequest {
    /// `512Mi`, `2g`, ...
    #[serde(default)]
    pub memory_limit: Option<String>,
    /// The soft target below the hard limit.
    #[serde(default)]
    pub memory_reservation: Option<String>,
    #[serde(default)]
    pub cpu_limit: Option<f64>,
    #[serde(default)]
    pub cpu_shares: Option<i64>,
    #[serde(default)]
    pub pids_limit: Option<i64>,
}

/// Change a sandbox's memory, CPU and process limits; live when it runs.
pub async fn put_resources(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<ResourcesRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = sandbox_service::set_resources(
        &state.services,
        &auth.principal,
        id,
        sandbox_service::ResourceRequest {
            memory_limit: body.memory_limit,
            memory_reservation: body.memory_reservation,
            cpu_limit: body.cpu_limit,
            cpu_shares: body.cpu_shares,
            pids_limit: body.pids_limit,
        },
    )
    .await?;
    Ok(Json(result))
}

/// The host's own load.
pub async fn host_load(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(sandbox_service::host_load(&state.services, &auth.principal).await?))
}


/// Write the agent's configuration files (model endpoint, proxy) into a running sandbox.
pub async fn reconfigure(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(sandbox_service::reconfigure_agent(&state.services, &auth.principal, id).await?))
}


/// Replace the container with one that has the current mounts (see ADR 0021).
pub async fn recreate(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(sandbox_service::recreate(&state.services, &auth.principal, id).await?))
}
