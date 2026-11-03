//! Recipes: the builder.
//!
//! A handler parses, the service decides. The only thing that lives here is
//! the wire shape, because that is what the console sees and what the OpenAPI
//! document describes.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::builder::{AgentSpec, ProjectMount, Recipe, RepositorySpec, ResourceLimits};
use crate::error::ApiError;
use crate::services::builder_service::{self, RecipeInput};
use crate::services::template_service;
use crate::state::AppState;

/// A create or edit body.
///
/// The whole recipe is sent, not a patch of fields: "what the operator saw" is
/// then the recipe, and a forgotten field cannot be mistaken for "keep it".
#[derive(Debug, Deserialize, ToSchema)]
pub struct RecipeRequest {
    pub name: String,
    /// The baseline key, e.g. `ubuntu-24.04`.
    pub baseline_key: String,
    /// Tools by `tool_catalog` key.
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub extra_packages: Vec<String>,
    #[serde(default)]
    pub repositories: Vec<RepositorySpec>,
    #[serde(default)]
    pub mcp_server_keys: Vec<String>,
    #[serde(default)]
    pub agents: Vec<AgentSpec>,
    #[serde(default)]
    pub resources: ResourceLimits,
    #[serde(default)]
    pub project_mounts: Vec<ProjectMount>,
    /// The node to build and run on, by name. Omitted means the local node.
    #[serde(default)]
    pub node: Option<String>,
}

impl RecipeRequest {
    fn into_input(self) -> RecipeInput {
        RecipeInput {
            name: self.name,
            baseline_key: self.baseline_key,
            tools: self.tools,
            extra_packages: self.extra_packages,
            repositories: self.repositories,
            mcp_server_keys: self.mcp_server_keys,
            agents: self.agents,
            resources: self.resources,
            project_mounts: self.project_mounts,
            node: self.node,
        }
    }
}

/// The body of an apply-template request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ApplyTemplateRequest {
    /// The template whose selection becomes the recipe's.
    pub template_id: Uuid,
}

/// The body of an instantiate request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct InstantiateRequest {
    /// The sandbox to create. A recipe is a template, so the name is per
    /// sandbox and not stored on the recipe.
    pub name: String,
    /// How much of the sandbox's traffic the proxy may read: `direct`,
    /// `forward_proxy`, `transparent` or `hybrid`. Fixed when the container is
    /// created, because a recipe sandbox cannot be recreated later. Omitted:
    /// `transparent` for a recipe with a desktop, `forward_proxy` otherwise.
    #[serde(default)]
    pub egress_mode: Option<String>,
}

/// List recipes.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let recipes = builder_service::list(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": recipes })))
}

/// Create a draft.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<RecipeRequest>,
) -> Result<(StatusCode, Json<Recipe>), ApiError> {
    let recipe =
        builder_service::create(&state.services, &auth.principal, body.into_input()).await?;
    Ok((StatusCode::CREATED, Json(recipe)))
}

/// Read a recipe.
pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Recipe>, ApiError> {
    Ok(Json(
        builder_service::get(&state.services, &auth.principal, id).await?,
    ))
}

/// Edit a recipe; bumps the revision and returns it to draft.
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<RecipeRequest>,
) -> Result<Json<Recipe>, ApiError> {
    Ok(Json(
        builder_service::update(&state.services, &auth.principal, id, body.into_input()).await?,
    ))
}

/// Copy a template's selection onto a recipe and return it as a new draft.
pub async fn apply_template(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<ApplyTemplateRequest>,
) -> Result<Json<Recipe>, ApiError> {
    Ok(Json(
        template_service::apply_to_recipe(
            &state.services,
            &auth.principal,
            body.template_id,
            id,
        )
        .await?,
    ))
}

/// Delete a recipe.
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    builder_service::delete(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Build the image: a job when something is built, a reuse when nothing changed.
pub async fn build(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let outcome = builder_service::build(&state.services, &auth.principal, id).await?;
    let status = if outcome.job_id.is_some() {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(serde_json::json!({
            "job_id": outcome.job_id,
            "reused": outcome.reused,
            "recipe": outcome.recipe,
        })),
    ))
}

/// Create a sandbox from a built recipe.
pub async fn instantiate(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<InstantiateRequest>,
) -> Result<(StatusCode, Json<crate::domain::sandbox::Sandbox>), ApiError> {
    let sandbox = builder_service::instantiate(
        &state.services,
        &auth.principal,
        id,
        &body.name,
        body.egress_mode.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(sandbox)))
}
