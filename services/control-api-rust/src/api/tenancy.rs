//! Customers, data centers, sites, sandbox grants and invitations (ADR 0044, 0045).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::tenancy_service::{self, CreatedInvitation, DataCenterView, InvitationPreview, OrganizationView, SiteView};
use crate::state::AppState;

/// The organisation tree the caller reaches, with data centers.
pub async fn organizations(State(state): State<AppState>, auth: Auth) -> Result<Json<serde_json::Value>, ApiError> {
    let items = tenancy_service::organizations(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NameRequest {
    pub name: String,
}

/// Create a customer below the caller's organisation (providers only).
pub async fn create_customer(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<NameRequest>,
) -> Result<(StatusCode, Json<OrganizationView>), ApiError> {
    let org = tenancy_service::create_customer(&state.services, &auth.principal, &body.name).await?;
    Ok((StatusCode::CREATED, Json(org)))
}

/// Create a data center in the caller's organisation.
pub async fn create_data_center(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<NameRequest>,
) -> Result<(StatusCode, Json<DataCenterView>), ApiError> {
    let view = tenancy_service::create_data_center(&state.services, &auth.principal, &body.name).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

/// The sites of the current data center.
pub async fn list_sites(State(state): State<AppState>, auth: Auth) -> Result<Json<serde_json::Value>, ApiError> {
    let items = tenancy_service::list_sites(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSiteRequest {
    pub name: String,
    /// `home`, `cloud` or `edge`.
    pub kind: String,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
}

/// Add a site.
pub async fn create_site(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<CreateSiteRequest>,
) -> Result<(StatusCode, Json<SiteView>), ApiError> {
    let site = tenancy_service::create_site(
        &state.services,
        &auth.principal,
        &body.name,
        &body.kind,
        body.provider.as_deref(),
        body.region.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(site)))
}

/// Remove an empty site.
pub async fn delete_site(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    tenancy_service::delete_site(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Who holds a grant on a sandbox.
pub async fn list_grants(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<Json<serde_json::Value>, ApiError> {
    let items = tenancy_service::list_grants(&state.services, &auth.principal, id).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// Hand a sandbox to a person.
pub async fn grant(State(state): State<AppState>, auth: Auth, Path((id, user)): Path<(Uuid, Uuid)>) -> Result<StatusCode, ApiError> {
    tenancy_service::grant(&state.services, &auth.principal, id, user).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Take a sandbox back from a person.
pub async fn revoke(State(state): State<AppState>, auth: Auth, Path((id, user)): Path<(Uuid, Uuid)>) -> Result<StatusCode, ApiError> {
    tenancy_service::revoke(&state.services, &auth.principal, id, user).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The invitations of the current data center.
pub async fn list_invitations(State(state): State<AppState>, auth: Auth) -> Result<Json<serde_json::Value>, ApiError> {
    let items = tenancy_service::list_invitations(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct InviteRequest {
    /// `admin`, `operator`, `member` or `viewer`. Ignored for a sandbox invitation, which is
    /// always a member's.
    pub role: String,
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub valid_days: Option<i64>,
}

/// Invite a person. The token in the answer is shown once.
pub async fn invite(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<InviteRequest>,
) -> Result<(StatusCode, Json<CreatedInvitation>), ApiError> {
    let created = tenancy_service::invite(
        &state.services,
        &auth.principal,
        &body.role,
        body.sandbox_id,
        body.email.as_deref(),
        body.valid_days.unwrap_or(7),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Withdraw an unused invitation.
pub async fn revoke_invitation(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    tenancy_service::revoke_invitation(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What an invitation link offers. Public: the person has no account yet.
pub async fn preview_invitation(State(state): State<AppState>, Path(token): Path<String>) -> Result<Json<InvitationPreview>, ApiError> {
    Ok(Json(tenancy_service::preview(&state.services, &token).await?))
}
