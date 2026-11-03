//! The tenant tree: customers, data centers, sites, sandbox grants and invitations
//! (ADR 0044, 0045).
//!
//! A provider (platform or partner administrator) creates customers; an administrator of a
//! customer creates data centers and sites; an administrator hands a sandbox to a person or
//! invites someone to it. Every write is audited, and every rule that decides who may do
//! it is here, not in the handler.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal, Role};
use crate::domain::tenancy::{self, OrganizationKind};
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::security::session;
use crate::store::TenantScope;

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from(crate::store::map_sqlx_error(error))
}

async fn audit(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    kind: &str,
    subject_type: &str,
    subject: Option<Uuid>,
    detail: serde_json::Value,
) -> Result<(), ApiError> {
    crate::store::postgres::audit::record(
        scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind,
            subject_type: Some(subject_type),
            subject_id: subject,
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrganizationView {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub parent_id: Option<Uuid>,
    pub environments: Vec<DataCenterView>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DataCenterView {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub sites: i64,
    pub nodes: i64,
    pub sandboxes: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SiteView {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub provider: Option<String>,
    pub region: Option<String>,
    pub overlay_cidr: Option<String>,
    pub nodes: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GrantView {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub granted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InvitationView {
    pub id: Uuid,
    pub role: String,
    pub sandbox_id: Option<Uuid>,
    pub email: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
}

/// What a person sees before following an invitation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InvitationPreview {
    pub organization: String,
    pub data_center: String,
    pub role: String,
    pub sandbox: Option<String>,
    pub email: Option<String>,
}

/// A freshly created invitation. The token is shown once; only its hash is kept.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreatedInvitation {
    pub invitation: InvitationView,
    pub token: String,
}

/// The caller's own organisation and what is below it, with the data centers of each.
///
/// A provider sees its customers; a customer sees only itself. The reach is the same rule
/// as `nb_user_environments`, so the tree never shows something the caller could not open.
pub async fn organizations(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<OrganizationView>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let reachable: Vec<Uuid> = sqlx::query_scalar("SELECT environment_id FROM nb_user_environments($1)")
        .bind(principal.user_id)
        .fetch_all(&ctx.pool)
        .await
        .map_err(db)?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, String, Option<Uuid>)>(
        "SELECT DISTINCT o.id, o.name, o.slug, o.kind, o.parent_id
           FROM organizations o JOIN environments e ON e.organization_id = o.id
          WHERE e.id = ANY($1)
          ORDER BY o.name",
    )
    .bind(&reachable)
    .fetch_all(&ctx.pool)
    .await
    .map_err(db)?;
    let mut out = Vec::new();
    for (id, name, slug, kind, parent_id) in rows {
        let envs = sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT e.id, e.name, e.slug FROM environments e
              WHERE e.organization_id = $1 AND e.id = ANY($2) ORDER BY e.name",
        )
        .bind(id)
        .bind(&reachable)
        .fetch_all(&ctx.pool)
        .await
        .map_err(db)?;
        let mut environments = Vec::new();
        for (env_id, env_name, env_slug) in envs {
            // The counts read tables that row-level security guards, so each data center is
            // counted inside its own scope. No acting user is set: this is a total, not a view
            // of anyone's sandboxes, and the caller already reaches the data center.
            let mut scope = TenantScope::begin(&ctx.pool, env_id).await.map_err(ApiError::from)?;
            let (sites, nodes, sandboxes): (i64, i64, i64) = sqlx::query_as(
                "SELECT (SELECT count(*) FROM sites WHERE environment_id = $1),
                        (SELECT count(*) FROM nodes WHERE environment_id = $1),
                        (SELECT count(*) FROM sandboxes WHERE environment_id = $1)",
            )
            .bind(env_id)
            .fetch_one(scope.connection())
            .await
            .map_err(db)?;
            scope.commit().await.map_err(ApiError::from)?;
            environments.push(DataCenterView { id: env_id, name: env_name, slug: env_slug, sites, nodes, sandboxes });
        }
        out.push(OrganizationView { id, name, slug, kind, parent_id, environments });
    }
    Ok(out)
}

/// The organisation a user belongs to, with its kind.
async fn organization_of_user(ctx: &ServiceContext, user_id: Uuid) -> Result<(Uuid, OrganizationKind, Role), ApiError> {
    let (id, kind, role): (Uuid, String, String) = sqlx::query_as(
        "SELECT o.id, o.kind, u.role FROM users u JOIN organizations o ON o.id = u.organization_id WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&ctx.pool)
    .await
    .map_err(db)?;
    Ok((
        id,
        OrganizationKind::parse(&kind).unwrap_or(OrganizationKind::Customer),
        Role::parse(&role).unwrap_or(Role::Viewer),
    ))
}

/// Create a customer below the caller's organisation, with its first data center and site.
///
/// Only an administrator of a platform or partner organisation may; a customer cannot
/// create customers. The creator is not made a member of the customer: their reach comes
/// from the tree, so removing the customer from the tree removes the access with it.
pub async fn create_customer(
    ctx: &ServiceContext,
    principal: &Principal,
    name: &str,
) -> Result<OrganizationView, ApiError> {
    require(principal, Permission::UserManage)?;
    let (parent, kind, role) = organization_of_user(ctx, principal.user_id).await?;
    if !kind.may_have_children() || role != Role::Admin {
        return Err(ApiError::forbidden("organization.create_customer"));
    }
    let (org_id, environment_id) = onboard(&ctx.pool, name, Some(parent), OrganizationKind::Customer, None).await?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    audit(&mut scope, principal, "organization.create", "organization", Some(org_id), serde_json::json!({ "name": name })).await?;
    scope.commit().await.map_err(ApiError::from)?;
    organizations(ctx, principal)
        .await?
        .into_iter()
        .find(|org| org.id == org_id)
        .ok_or_else(|| ApiError::internal(format!("customer {org_id} / {environment_id} was not found after creation")))
}

/// Create an organisation, its first data center ("Main") and its first site ("Main site"),
/// and optionally a first administrator. One transaction: a half-made tenant is worse than
/// none.
///
/// Used by a provider creating a customer and by a self-service sign-up.
pub async fn onboard(
    pool: &sqlx::PgPool,
    name: &str,
    parent: Option<Uuid>,
    kind: OrganizationKind,
    first_admin: Option<Uuid>,
) -> Result<(Uuid, Uuid), ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::validation("A name is 1 to 80 characters."));
    }
    let base = tenancy::slugify(name).ok_or_else(|| ApiError::validation("Use letters or digits in the name."))?;
    let slug = format!("{}-{}", &base[..base.len().min(52)], &Uuid::new_v4().simple().to_string()[..8]);

    let mut tx = pool.begin().await.map_err(db)?;
    let org_id: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (name, slug, parent_id, kind) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(name)
    .bind(&slug)
    .bind(parent)
    .bind(kind.as_str())
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    let environment_id: Uuid = sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug) VALUES ($1, 'Main', 'main') RETURNING id",
    )
    .bind(org_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    sqlx::query("SELECT set_config('app.environment_id', $1, true)")
        .bind(environment_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO sites (environment_id, name, slug, kind) VALUES ($1, 'Main site', 'main', 'home')")
        .bind(environment_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO clusters (environment_id, name, slug) VALUES ($1, 'main', 'main')")
        .bind(environment_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    if let Some(user_id) = first_admin {
        sqlx::query("UPDATE users SET organization_id = $2, role = 'admin' WHERE id = $1")
            .bind(user_id)
            .bind(org_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
            .bind(user_id)
            .bind(environment_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    Ok((org_id, environment_id))
}

/// Create another data center in the caller's organisation (or, for a provider, in a
/// customer it reaches). The caller becomes its administrator.
pub async fn create_data_center(
    ctx: &ServiceContext,
    principal: &Principal,
    name: &str,
) -> Result<DataCenterView, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::validation("A name is 1 to 80 characters."));
    }
    let slug = tenancy::slugify(name).ok_or_else(|| ApiError::validation("Use letters or digits in the name."))?;
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM environments WHERE id = $1")
        .bind(principal.environment_id)
        .fetch_one(&ctx.pool)
        .await
        .map_err(db)?;
    let mut tx = ctx.pool.begin().await.map_err(db)?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(org)
    .bind(name)
    .bind(&slug)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match crate::store::map_sqlx_error(e) {
        StoreError::UniqueViolation(_) => ApiError::conflict(ErrorCode::SANDBOX_NAME_TAKEN, "A data center with this name exists."),
        other => ApiError::from(other),
    })?;
    sqlx::query("SELECT set_config('app.environment_id', $1, true)")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO sites (environment_id, name, slug, kind) VALUES ($1, 'Main site', 'main', 'home')")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query("INSERT INTO clusters (environment_id, name, slug) VALUES ($1, 'main', 'main')")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    sqlx::query(
        "INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')
         ON CONFLICT (user_id, environment_id) DO UPDATE SET role = 'admin'",
    )
    .bind(principal.user_id)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(DataCenterView { id, name: name.to_string(), slug, sites: 1, nodes: 0, sandboxes: 0 })
}

fn site_kind(kind: &str) -> Result<&str, ApiError> {
    tenancy::SITE_KINDS
        .iter()
        .find(|k| **k == kind)
        .copied()
        .ok_or_else(|| ApiError::validation("A site is home, cloud or edge."))
}

/// The sites of this data center, with the number of nodes at each.
pub async fn list_sites(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<SiteView>, ApiError> {
    require(principal, Permission::NodeRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, String, Option<String>, Option<String>, Option<String>, i64)>(
        "SELECT s.id, s.name, s.slug, s.kind, s.provider, s.region, s.overlay_cidr,
                (SELECT count(*) FROM nodes n WHERE n.site_id = s.id)
           FROM sites s WHERE s.environment_id = $1 ORDER BY s.name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|(id, name, slug, kind, provider, region, overlay_cidr, nodes)| SiteView { id, name, slug, kind, provider, region, overlay_cidr, nodes })
        .collect())
}

/// Add a site to this data center.
pub async fn create_site(
    ctx: &ServiceContext,
    principal: &Principal,
    name: &str,
    kind: &str,
    provider: Option<&str>,
    region: Option<&str>,
) -> Result<SiteView, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ApiError::validation("A name is 1 to 80 characters."));
    }
    let kind = site_kind(kind)?;
    let slug = tenancy::slugify(name).ok_or_else(|| ApiError::validation("Use letters or digits in the name."))?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO sites (environment_id, name, slug, kind, provider, region)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(name)
    .bind(&slug)
    .bind(kind)
    .bind(provider.map(str::trim).filter(|v| !v.is_empty()))
    .bind(region.map(str::trim).filter(|v| !v.is_empty()))
    .fetch_one(scope.connection())
    .await
    .map_err(|e| match crate::store::map_sqlx_error(e) {
        StoreError::UniqueViolation(_) => ApiError::conflict(ErrorCode::SANDBOX_NAME_TAKEN, "A site with this name exists."),
        other => ApiError::from(other),
    })?;
    audit(&mut scope, principal, "site.create", "site", Some(id), serde_json::json!({ "name": name, "kind": kind })).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(SiteView {
        id,
        name: name.to_string(),
        slug,
        kind: kind.to_string(),
        provider: provider.map(str::to_string),
        region: region.map(str::to_string),
        overlay_cidr: None,
        nodes: 0,
    })
}

/// Remove an empty site. A site with nodes is refused: its nodes would lose their place.
pub async fn delete_site(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let nodes: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes WHERE site_id = $1")
        .bind(id)
        .fetch_one(scope.connection())
        .await
        .map_err(db)?;
    if nodes > 0 {
        return Err(ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, "This site still has nodes. Move or remove them first."));
    }
    let result = sqlx::query("DELETE FROM sites WHERE id = $1 AND environment_id = $2")
        .bind(id)
        .bind(scope.environment_id())
        .execute(scope.connection())
        .await
        .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "site"));
    }
    audit(&mut scope, principal, "site.delete", "site", Some(id), serde_json::json!({})).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Sandbox grants
// ---------------------------------------------------------------------------

/// Load a sandbox the caller may see, as its name. Under row-level security a sandbox the
/// caller may not see is simply absent, so a guessed id is a plain not-found.
async fn visible_sandbox(scope: &mut TenantScope<'_>, id: Uuid) -> Result<String, ApiError> {
    sqlx::query_scalar::<_, String>("SELECT name FROM sandboxes WHERE id = $1")
        .bind(id)
        .fetch_optional(scope.connection())
        .await
        .map_err(db)?
        .ok_or_else(|| ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "sandbox"))
}

/// Who holds a grant on this sandbox.
pub async fn list_grants(ctx: &ServiceContext, principal: &Principal, sandbox_id: Uuid) -> Result<Vec<GrantView>, ApiError> {
    require(principal, Permission::UserManage)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    visible_sandbox(&mut scope, sandbox_id).await?;
    let rows = sqlx::query_as::<_, (Uuid, String, String, DateTime<Utc>)>(
        "SELECT u.id, u.email, u.display_name, g.created_at
           FROM sandbox_grants g JOIN users u ON u.id = g.user_id
          WHERE g.sandbox_id = $1 ORDER BY u.email",
    )
    .bind(sandbox_id)
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|(user_id, email, display_name, granted_at)| GrantView { user_id, email, display_name, granted_at })
        .collect())
}

/// Hand a sandbox to a person who is a member of this data center. A viewer cannot be
/// given a sandbox to work in: raise their role first, which is an explicit decision.
pub async fn grant(ctx: &ServiceContext, principal: &Principal, sandbox_id: Uuid, user_id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::UserManage)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let name = visible_sandbox(&mut scope, sandbox_id).await?;
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM user_scopes WHERE user_id = $1 AND environment_id = $2")
        .bind(user_id)
        .bind(scope.environment_id())
        .fetch_optional(scope.connection())
        .await
        .map_err(db)?;
    let role = role
        .and_then(|r| Role::parse(&r))
        .ok_or_else(|| ApiError::validation("This person is not a member of the data center."))?;
    if !tenancy::grantable(role) {
        return Err(ApiError::validation("A viewer cannot work in a sandbox. Make them a member first."));
    }
    sqlx::query(
        "INSERT INTO sandbox_grants (sandbox_id, user_id, environment_id, granted_by)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(sandbox_id)
    .bind(user_id)
    .bind(scope.environment_id())
    .bind(principal.user_id)
    .execute(scope.connection())
    .await
    .map_err(db)?;
    audit(&mut scope, principal, "sandbox.grant", "sandbox", Some(sandbox_id), serde_json::json!({ "user_id": user_id, "sandbox": name })).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Take a sandbox back from a person.
pub async fn revoke(ctx: &ServiceContext, principal: &Principal, sandbox_id: Uuid, user_id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::UserManage)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    visible_sandbox(&mut scope, sandbox_id).await?;
    sqlx::query("DELETE FROM sandbox_grants WHERE sandbox_id = $1 AND user_id = $2")
        .bind(sandbox_id)
        .bind(user_id)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    audit(&mut scope, principal, "sandbox.revoke", "sandbox", Some(sandbox_id), serde_json::json!({ "user_id": user_id })).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Invitations
// ---------------------------------------------------------------------------

/// Invite a person. With a `sandbox_id` the invitation hands over that sandbox only and the
/// person becomes a member; without one it carries the role asked for.
pub async fn invite(
    ctx: &ServiceContext,
    principal: &Principal,
    role: &str,
    sandbox_id: Option<Uuid>,
    email: Option<&str>,
    valid_days: i64,
) -> Result<CreatedInvitation, ApiError> {
    require(principal, Permission::UserManage)?;
    let role = tenancy::invitation_role(role, sandbox_id.is_some())
        .ok_or_else(|| ApiError::validation("A role is admin, operator, member or viewer."))?;
    let email = email.map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty());
    if let Some(email) = &email {
        if !email.contains('@') || email.len() > 254 {
            return Err(ApiError::validation("Enter an email address, for example name@example.com."));
        }
    }
    let days = valid_days.clamp(1, 30);
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    if let Some(id) = sandbox_id {
        visible_sandbox(&mut scope, id).await?;
    }
    let org: Uuid = sqlx::query_scalar("SELECT organization_id FROM environments WHERE id = $1")
        .bind(scope.environment_id())
        .fetch_one(scope.connection())
        .await
        .map_err(db)?;
    let token = session::random_token();
    let expires_at = Utc::now() + Duration::days(days);
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO invitations (token_hash, organization_id, environment_id, role, sandbox_id, email, created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(session::hash_token(&token))
    .bind(org)
    .bind(scope.environment_id())
    .bind(role.as_str())
    .bind(sandbox_id)
    .bind(&email)
    .bind(principal.user_id)
    .bind(expires_at)
    .fetch_one(scope.connection())
    .await
    .map_err(db)?;
    audit(&mut scope, principal, "invitation.create", "invitation", Some(id), serde_json::json!({ "role": role.as_str(), "sandbox_id": sandbox_id })).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(CreatedInvitation {
        invitation: InvitationView { id, role: role.as_str().into(), sandbox_id, email, expires_at, accepted_at: None },
        token,
    })
}

/// The open and recent invitations of this data center.
pub async fn list_invitations(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<InvitationView>, ApiError> {
    require(principal, Permission::UserManage)?;
    let rows = sqlx::query_as::<_, (Uuid, String, Option<Uuid>, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)>(
        "SELECT id, role, sandbox_id, email, expires_at, accepted_at FROM invitations
          WHERE environment_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(principal.environment_id)
    .fetch_all(&ctx.pool)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|(id, role, sandbox_id, email, expires_at, accepted_at)| InvitationView { id, role, sandbox_id, email, expires_at, accepted_at })
        .collect())
}

/// Withdraw an invitation that has not been used.
pub async fn revoke_invitation(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::UserManage)?;
    let result = sqlx::query("DELETE FROM invitations WHERE id = $1 AND environment_id = $2 AND accepted_at IS NULL")
        .bind(id)
        .bind(principal.environment_id)
        .execute(&ctx.pool)
        .await
        .map_err(db)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "invitation"));
    }
    Ok(())
}

/// What an invitation link offers, for the page a person lands on. Reveals nothing about a
/// link that is unknown, used or expired beyond "not valid", so links cannot be probed.
pub async fn preview(ctx: &ServiceContext, token: &str) -> Result<InvitationPreview, ApiError> {
    let row = sqlx::query_as::<_, (String, String, String, Option<Uuid>, Option<String>, Uuid)>(
        "SELECT o.name, e.name, i.role, i.sandbox_id, i.email, i.environment_id
           FROM invitations i
           JOIN organizations o ON o.id = i.organization_id
           JOIN environments e ON e.id = i.environment_id
          WHERE i.token_hash = $1 AND i.accepted_at IS NULL AND i.expires_at > now()",
    )
    .bind(session::hash_token(token))
    .fetch_optional(&ctx.pool)
    .await
    .map_err(db)?;
    let Some((organization, data_center, role, sandbox_id, email, environment_id)) = row else {
        return Err(ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "invitation"));
    };
    let sandbox = match sandbox_id {
        Some(id) => {
            let mut scope = TenantScope::begin(&ctx.pool, environment_id).await.map_err(ApiError::from)?;
            let name = sqlx::query_scalar::<_, String>("SELECT name FROM sandboxes WHERE id = $1")
                .bind(id)
                .fetch_optional(scope.connection())
                .await
                .map_err(db)?;
            scope.commit().await.map_err(ApiError::from)?;
            name
        }
        None => None,
    };
    Ok(InvitationPreview { organization, data_center, role, sandbox, email })
}
