//! Media settings per environment and per sandbox, and the render record.
//!
//! Three tables live here (`0046_media_mcp.sql`). The settings reads are one
//! row each, so there is no pagination and no list shape to keep in sync with a
//! filter; `media_renders` is the only growing table and its list is bounded by
//! the caller's limit.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::media::{MediaKind, MediaSettings, MediaTarget, SandboxMediaSettings, UserMediaGrant};
use crate::error::StoreError;

/// One render as the API reports it: the prompt and where the asset is, never
/// a provider credential.
#[derive(Debug, Clone)]
pub struct RenderRow {
    pub id: Uuid,
    pub sandbox_id: Uuid,
    pub kind: MediaKind,
    pub provider_name: Option<String>,
    pub model: String,
    pub prompt: String,
    pub status: String,
    pub asset_name: Option<String>,
    pub mime_type: Option<String>,
    pub bytes: Option<i64>,
    pub error: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

const RENDER_COLUMNS: &str = "id, sandbox_id, kind, provider_name, model, prompt, status, asset_name, \
     mime_type, bytes, error, duration_ms, created_at, finished_at";

fn target_from(row: &sqlx::postgres::PgRow, prefix: &str) -> Result<Option<MediaTarget>, StoreError> {
    let provider: Option<Uuid> = row.try_get(format!("{prefix}_provider_id").as_str())?;
    let model: Option<String> = row.try_get(format!("{prefix}_model").as_str())?;
    Ok(match (provider, model) {
        (Some(provider_id), Some(model)) if !model.is_empty() => Some(MediaTarget { provider_id, model }),
        // The CHECK constraint makes a half pair impossible; a NULL pair is a
        // deliberate "inherit".
        _ => None,
    })
}

/// The environment default, or an all-empty default when the row does not exist
/// yet. Rendering is enabled by default so a fresh appliance does not look
/// broken before an admin saves anything; the absence of a model is what makes
/// a request fail, and it says so by name.
pub async fn settings(scope: &mut TenantScope<'_>) -> Result<MediaSettings, StoreError> {
    let row = sqlx::query(
        "SELECT enabled, image_provider_id, image_model, video_provider_id, video_model, \
                music_provider_id, music_model, allowed_provider_ids \
         FROM media_settings WHERE environment_id = $1",
    )
    .bind(scope.environment_id())
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else {
        return Ok(MediaSettings { enabled: true, ..Default::default() });
    };
    Ok(MediaSettings {
        enabled: row.try_get("enabled")?,
        image: target_from(&row, "image")?,
        video: target_from(&row, "video")?,
        music: target_from(&row, "music")?,
        allowed_provider_ids: row.try_get("allowed_provider_ids")?,
    })
}

/// Write the environment default. Idempotent by environment, so a second save
/// replaces rather than stacking rows.
pub async fn save_settings(
    scope: &mut TenantScope<'_>,
    settings: &MediaSettings,
    actor: Option<Uuid>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO media_settings (environment_id, enabled, image_provider_id, image_model, \
            video_provider_id, video_model, music_provider_id, music_model, \
            allowed_provider_ids, updated_by, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now()) \
         ON CONFLICT (environment_id) DO UPDATE SET \
            enabled = EXCLUDED.enabled, \
            image_provider_id = EXCLUDED.image_provider_id, image_model = EXCLUDED.image_model, \
            video_provider_id = EXCLUDED.video_provider_id, video_model = EXCLUDED.video_model, \
            music_provider_id = EXCLUDED.music_provider_id, music_model = EXCLUDED.music_model, \
            allowed_provider_ids = EXCLUDED.allowed_provider_ids, \
            updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(scope.environment_id())
    .bind(settings.enabled)
    .bind(settings.image.as_ref().map(|item| item.provider_id))
    .bind(settings.image.as_ref().map(|item| item.model.as_str()))
    .bind(settings.video.as_ref().map(|item| item.provider_id))
    .bind(settings.video.as_ref().map(|item| item.model.as_str()))
    .bind(settings.music.as_ref().map(|item| item.provider_id))
    .bind(settings.music.as_ref().map(|item| item.model.as_str()))
    .bind(&settings.allowed_provider_ids)
    .bind(actor)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// A sandbox's own choice, or `None` when it inherits everything.
pub async fn sandbox_settings(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<SandboxMediaSettings>, StoreError> {
    let row = sqlx::query(
        "SELECT image_provider_id, image_model, video_provider_id, video_model, \
                music_provider_id, music_model \
         FROM sandbox_media_settings WHERE sandbox_id = $1 AND environment_id = $2",
    )
    .bind(sandbox_id)
    .bind(scope.environment_id())
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(SandboxMediaSettings {
        image: target_from(&row, "image")?,
        video: target_from(&row, "video")?,
        music: target_from(&row, "music")?,
    }))
}

/// Replace a sandbox's choice. All three kinds are written together, so a
/// cleared kind means "inherit" rather than "keep the previous value".
pub async fn save_sandbox_settings(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    settings: &SandboxMediaSettings,
    actor: Option<Uuid>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO sandbox_media_settings (sandbox_id, environment_id, image_provider_id, image_model, \
            video_provider_id, video_model, music_provider_id, music_model, updated_by, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now()) \
         ON CONFLICT (sandbox_id) DO UPDATE SET \
            image_provider_id = EXCLUDED.image_provider_id, image_model = EXCLUDED.image_model, \
            video_provider_id = EXCLUDED.video_provider_id, video_model = EXCLUDED.video_model, \
            music_provider_id = EXCLUDED.music_provider_id, music_model = EXCLUDED.music_model, \
            updated_by = EXCLUDED.updated_by, updated_at = now()",
    )
    .bind(sandbox_id)
    .bind(scope.environment_id())
    .bind(settings.image.as_ref().map(|item| item.provider_id))
    .bind(settings.image.as_ref().map(|item| item.model.as_str()))
    .bind(settings.video.as_ref().map(|item| item.provider_id))
    .bind(settings.video.as_ref().map(|item| item.model.as_str()))
    .bind(settings.music.as_ref().map(|item| item.provider_id))
    .bind(settings.music.as_ref().map(|item| item.model.as_str()))
    .bind(actor)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Record a render before the provider is called, so a timeout leaves a row
/// that says `running` rather than no evidence at all.
#[allow(clippy::too_many_arguments)]
pub async fn insert_render(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    kind: MediaKind,
    provider_id: Uuid,
    provider_name: &str,
    model: &str,
    prompt: &str,
    actor: Option<Uuid>,
) -> Result<Uuid, StoreError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO media_renders (environment_id, sandbox_id, kind, provider_id, provider_name, \
            model, prompt, status, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'running', $8) RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(kind.as_str())
    .bind(provider_id)
    .bind(provider_name)
    .bind(model)
    .bind(prompt)
    .bind(actor)
    .fetch_one(scope.connection())
    .await?;
    Ok(id)
}

/// Store the outcome. The asset name is a locator under the media directory,
/// never an absolute path.
pub async fn finish_render(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    asset: Option<(&str, &str, i64)>,
    error: Option<&str>,
    duration_ms: i32,
) -> Result<(), StoreError> {
    let (asset_name, mime_type, bytes) = match asset {
        Some((name, mime, size)) => (Some(name), Some(mime), Some(size)),
        None => (None, None, None),
    };
    sqlx::query(
        "UPDATE media_renders SET status = $2, asset_name = $3, mime_type = $4, bytes = $5, \
            error = $6, duration_ms = $7, finished_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(if asset_name.is_some() { "done" } else { "error" })
    .bind(asset_name)
    .bind(mime_type)
    .bind(bytes)
    .bind(error)
    .bind(duration_ms)
    .execute(scope.connection())
    .await?;
    Ok(())
}

fn row_to_render(row: &sqlx::postgres::PgRow) -> Result<RenderRow, StoreError> {
    let kind_raw: String = row.try_get("kind")?;
    Ok(RenderRow {
        id: row.try_get("id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        kind: MediaKind::parse(&kind_raw).unwrap_or(MediaKind::Image),
        provider_name: row.try_get("provider_name")?,
        model: row.try_get("model")?,
        prompt: row.try_get("prompt")?,
        status: row.try_get("status")?,
        asset_name: row.try_get("asset_name")?,
        mime_type: row.try_get("mime_type")?,
        bytes: row.try_get("bytes")?,
        error: row.try_get("error")?,
        duration_ms: row.try_get("duration_ms")?,
        created_at: row.try_get("created_at")?,
        finished_at: row.try_get("finished_at")?,
    })
}

/// The recent renders for one sandbox, newest first.
pub async fn list_renders(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    limit: i64,
) -> Result<Vec<RenderRow>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {RENDER_COLUMNS} FROM media_renders \
         WHERE sandbox_id = $1 AND environment_id = $2 ORDER BY created_at DESC LIMIT $3"
    ))
    .bind(sandbox_id)
    .bind(scope.environment_id())
    .bind(limit.clamp(1, 200))
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(row_to_render).collect()
}

/// One render, for the asset handler and for an ownership check.
pub async fn get_render(scope: &mut TenantScope<'_>, id: Uuid) -> Result<Option<RenderRow>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {RENDER_COLUMNS} FROM media_renders WHERE id = $1 AND environment_id = $2"
    ))
    .bind(id)
    .bind(scope.environment_id())
    .fetch_optional(scope.connection())
    .await?;
    row.as_ref().map(row_to_render).transpose()
}

/// Resolve a sandbox by name inside one environment.
///
/// The bridge names a sandbox, not an environment; this is the lookup that
/// turns the name into the id every other statement uses.
pub async fn sandbox_id_by_name(
    scope: &mut TenantScope<'_>,
    name: &str,
) -> Result<Option<Uuid>, StoreError> {
    let id = sqlx::query_scalar("SELECT id FROM sandboxes WHERE name = $1 AND environment_id = $2")
        .bind(name)
        .bind(scope.environment_id())
        .fetch_optional(scope.connection())
        .await?;
    Ok(id)
}

/// The data centers a sandbox name appears in, answered across tenants.
///
/// The accounting proxy carries no principal, so there is no scope to read
/// `sandboxes` under, and "which environment exists?" is not the question: the
/// sandbox is. `nb_sandbox_environment(name)` (migration 0088) returns the
/// environment of every sandbox with that name and nothing else.
///
/// `None` means the installation's schema predates the function, so the caller
/// keeps its single-data-center fallback; `Some(vec![])` means no such sandbox;
/// two entries mean the name exists in two data centers, which the service
/// answers as a 409 rather than picking one.
pub async fn sandbox_environments_by_name(
    pool: &sqlx::PgPool,
    name: &str,
) -> Result<Option<Vec<Uuid>>, StoreError> {
    let found = sqlx::query_scalar::<_, Uuid>(
        "SELECT DISTINCT environment_id \
           FROM nb_sandbox_environment($1::text) AS environments(environment_id)",
    )
    .bind(name)
    .fetch_all(pool)
    .await;
    match found {
        Ok(environments) => Ok(Some(environments)),
        Err(error) => {
            let error = StoreError::from(error);
            if super::is_missing_function(&error) {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}

/// Whether the media MCP server is attached and enabled for this sandbox.
///
/// The attachment is the grant: a tool name the agent learned somewhere else is
/// not enough to render, and removing the tool removes the ability with it.
pub async fn media_attached(scope: &mut TenantScope<'_>, sandbox_id: Uuid) -> Result<bool, StoreError> {
    let attached: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sandbox_mcp_servers \
         WHERE sandbox_id = $1 AND environment_id = $2 AND server_key = 'nestbox-media' AND enabled)",
    )
    .bind(sandbox_id)
    .bind(scope.environment_id())
    .fetch_one(scope.connection())
    .await?;
    Ok(attached)
}

/// The environments this appliance holds.
///
/// The media bridge is one appliance-side caller without a browser session, so
/// it cannot name a tenant the way a principal does. It is allowed to proceed
/// only when the answer is unambiguous; a multi-tenant deployment must set
/// `NESTBOX_MEDIA_ENVIRONMENT_ID` instead of guessing.
pub async fn environment_ids(pool: &sqlx::PgPool) -> Result<Vec<Uuid>, StoreError> {
    let ids = sqlx::query_scalar("SELECT id FROM environments ORDER BY created_at")
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

// --- per-user grants -------------------------------------------------------

/// What one person may see and choose in this environment, if a row exists.
pub async fn find_user_grant(
    scope: &mut TenantScope<'_>,
    user_id: Uuid,
) -> Result<Option<UserMediaGrant>, StoreError> {
    let row = sqlx::query(
        "SELECT provider_ids, can_configure FROM user_media_grants WHERE user_id = $1 AND environment_id = $2",
    )
    .bind(user_id)
    .bind(scope.environment_id())
    .fetch_optional(scope.connection())
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(UserMediaGrant {
        provider_ids: row.try_get("provider_ids")?,
        can_configure: row.try_get("can_configure")?,
    }))
}

/// Write (or clear) a person's grant.
///
/// A row that grants everything and restricts nothing is deleted instead: an
/// absent row means "inherit", and storing a no-op row would freeze the person
/// against a later change of the environment default.
pub async fn upsert_user_grant(
    scope: &mut TenantScope<'_>,
    user_id: Uuid,
    grant: &UserMediaGrant,
    updated_by: Uuid,
) -> Result<(), StoreError> {
    if grant.provider_ids.is_empty() && grant.can_configure {
        sqlx::query("DELETE FROM user_media_grants WHERE user_id = $1 AND environment_id = $2")
            .bind(user_id)
            .bind(scope.environment_id())
            .execute(scope.connection())
            .await?;
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO user_media_grants
             (user_id, environment_id, provider_ids, can_configure, updated_by, updated_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (user_id, environment_id) DO UPDATE
            SET provider_ids = EXCLUDED.provider_ids,
                can_configure = EXCLUDED.can_configure,
                updated_by = EXCLUDED.updated_by,
                updated_at = now()",
    )
    .bind(user_id)
    .bind(scope.environment_id())
    .bind(&grant.provider_ids)
    .bind(grant.can_configure)
    .bind(updated_by)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Every explicit grant in this environment, for the admin screen.
pub async fn list_user_grants(
    scope: &mut TenantScope<'_>,
) -> Result<Vec<(Uuid, UserMediaGrant)>, StoreError> {
    let rows = sqlx::query(
        "SELECT user_id, provider_ids, can_configure FROM user_media_grants
          WHERE environment_id = $1 ORDER BY user_id",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get("user_id")?,
                UserMediaGrant {
                    provider_ids: row.try_get("provider_ids")?,
                    can_configure: row.try_get("can_configure")?,
                },
            ))
        })
        .collect()
}

/// The person who owns a sandbox, if it has one.
///
/// A grant is per person, so a render has to know whose grant applies. A shared
/// sandbox (no owner) has none, and the environment allow-list is the only bound.
pub async fn sandbox_owner(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<Uuid>, StoreError> {
    let owner = sqlx::query_scalar("SELECT owner_user_id FROM sandboxes WHERE id = $1")
        .bind(sandbox_id)
        .fetch_optional(scope.connection())
        .await?;
    Ok(owner.flatten())
}

/// Delete one render and return its asset name, so the caller can remove the
/// file. The row goes first: an orphaned file is invisible, an orphaned row with
/// no file is a broken link the console would show.
pub async fn delete_render(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<Option<String>, StoreError> {
    let asset: Option<Option<String>> = sqlx::query_scalar(
        "DELETE FROM media_renders WHERE id = $1 AND environment_id = $2 RETURNING asset_name",
    )
    .bind(id)
    .bind(scope.environment_id())
    .fetch_optional(scope.connection())
    .await?;
    Ok(asset.flatten())
}

/// Delete every render older than `before` in this environment, returning the
/// asset names that are now unreferenced.
pub async fn delete_renders_before(
    scope: &mut TenantScope<'_>,
    before: DateTime<Utc>,
) -> Result<Vec<String>, StoreError> {
    let rows = sqlx::query_scalar::<_, Option<String>>(
        "DELETE FROM media_renders
          WHERE environment_id = $1 AND created_at < $2
          RETURNING asset_name",
    )
    .bind(scope.environment_id())
    .bind(before)
    .fetch_all(scope.connection())
    .await?;
    Ok(rows.into_iter().flatten().collect())
}
