//! The builder: a recipe becomes an image, an image becomes a sandbox.
//!
//! Three rules shape this file:
//!
//! * **A recipe is data; the image is derived.** The Dockerfile is generated
//!   from the catalogs on every build and never stored, so a corrected pack or
//!   tool row changes the next build without a migration.
//! * **The image is addressed by the recipe's content hash.** The same recipe
//!   yields the same tag, so a second build is a reuse rather than a repeat, and
//!   an edit always yields a different tag.
//! * **A build is a job, not a request.** The endpoint records the job and
//!   returns; the build runs in the background and writes its outcome on the
//!   recipe, so a slow build cannot time out a browser.
//!
//! Everything that reaches a Dockerfile is validated first and quoted after
//! (ADR: never interpolate unvalidated text). A catalog row is data an operator
//! curates, not code we trust to be shell-safe.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::builder::{
    AgentLaunch, AgentSpec, HomeMode, ProjectMount, Recipe, RecipeName, RecipeState,
    ResourceLimits,
};
use crate::domain::job::{JobKind, TargetState};
use crate::domain::model::HardwareProfile;
use crate::domain::node::{InsufficientResources, Node, NodeBudget};
use crate::domain::pack::{AgentPack, InstallKind, PackageManager, Tool};
use crate::domain::persist::MountSpec;
use crate::domain::runtime::{normalize_memory_limit, BuildRequest, SandboxSpec};
use crate::domain::sandbox::{Sandbox, SandboxHealth, SandboxName, SandboxStatus};
use crate::domain::telemetry::LiveEvent;
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::TenantScope;
use crate::store::postgres::{
    agent_packs as catalog, environments as env_store, gateways, jobs as job_store,
        mcp as mcp_store, model_installs, models, nodes as node_store, persistent_roots, recipes as store,
    sandboxes as sandbox_store,
};

/// The generator version written into the content hash.
///
/// Bump this when the Dockerfile template changes in a way that must produce a
/// new image for an unchanged recipe. Without it a code deploy would keep
/// reusing the previous template's image because no recipe field moved.
const GENERATOR_VERSION: u32 = 6;

/// The supervisor program shipped inside every image the builder produces.
///
/// It has to be part of the content hash: a fix in this file changes what the
/// image runs, and an image tag that did not move would keep handing out the
/// old program to every recipe that did not change.
const SUPERVISOR_PROGRAM: &str = include_str!("../../../tool-images/supervisor/supervisor.py");

/// What a create or edit carries.
///
/// One struct for both verbs, because a recipe is replaced as a whole: a PATCH
/// that names only one field would make "what did the operator see" depend on
/// what they left out.
#[derive(Debug, Clone)]
pub struct RecipeInput {
    pub name: String,
    pub baseline_key: String,
    pub tools: Vec<String>,
    pub extra_packages: Vec<String>,
    pub repositories: Vec<crate::domain::builder::RepositorySpec>,
    pub mcp_server_keys: Vec<String>,
    pub agents: Vec<AgentSpec>,
    pub resources: ResourceLimits,
    pub project_mounts: Vec<ProjectMount>,
    /// The node to build and run on, by name. Empty means the local node.
    pub node: Option<String>,
}

/// How a pack is installed into the image.
///
/// The ref is resolved and validated once, here, so the generator only ever
/// sees values it can quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackInstall {
    /// The pack has nothing to install; it is a runtime or a bare shell.
    None,
    /// An installer script fetched over HTTPS.
    Script(String),
    /// A global npm package.
    Npm(String),
    /// A pip package.
    Pip(String),
    /// A single executable downloaded to `/usr/local/bin`.
    Binary { url: String, name: String },
    /// A git repository cloned to `/opt/<pack>`.
    Git { url: String, directory: String },
}

/// One agent as the image will contain it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAgent {
    pub instance: String,
    pub pack_key: String,
    pub install: PackInstall,
    pub home_directories: Vec<String>,
    /// The pack's command, so the image can start the agent itself.
    pub command: Vec<String>,
    /// The environment slots the pack reads at run time.
    pub env_slots: Vec<String>,
    /// Where the agent's home is, and therefore where it starts.
    pub home_mode: HomeMode,
    /// The port the agent listens on, when it has one.
    pub port: Option<u16>,
}

/// Everything the Dockerfile is generated from, already resolved.
///
/// No keys, no ids, no requests: a plan is what the catalogs and the recipe
/// together mean, so generating the same plan twice yields the same file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlan {
    pub name: String,
    pub baseline_key: String,
    pub base_image: String,
    pub manager: PackageManager,
    /// Package names for the baseline's manager, in install order.
    pub packages: Vec<String>,
    pub repositories: Vec<crate::domain::builder::RepositorySpec>,
    pub agents: Vec<PlannedAgent>,
    /// Directories created in the image; a bind mount replaces them at run time.
    pub home_directories: Vec<String>,
}

/// A validated recipe plus the image it would produce.
struct Prepared {
    name: String,
    baseline_key: String,
    tools: Vec<String>,
    extra_packages: Vec<String>,
    repositories: Vec<crate::domain::builder::RepositorySpec>,
    mcp_server_keys: Vec<String>,
    agents: Vec<AgentSpec>,
    resources: ResourceLimits,
    mounts: Vec<ProjectMount>,
    node: Node,
    plan: ImagePlan,
}

/// What a build request did.
pub struct BuildQueued {
    pub recipe: Recipe,
    /// Present when a job was queued; absent when an existing image was reused.
    pub job_id: Option<Uuid>,
    pub reused: bool,
}

// ---------------------------------------------------------------------------
// Reading and writing recipes
// ---------------------------------------------------------------------------

/// Every recipe in the environment.
pub async fn list(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<Recipe>, ApiError> {
    require(principal, Permission::RecipeRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let recipes = store::list(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(recipes)
}

/// One recipe.
pub async fn get(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<Recipe, ApiError> {
    require(principal, Permission::RecipeRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let recipe = store::get(&mut scope, id).await.map_err(recipe_error)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(recipe)
}

/// Create a draft recipe.
///
/// Everything is validated before the row exists, so a draft is always a
/// recipe that could be built; the draft state means "not built yet", not
/// "half checked".
pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    input: RecipeInput,
) -> Result<Recipe, ApiError> {
    require(principal, Permission::RecipeWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let prepared = prepare(&mut scope, &input, None).await?;

    let recipe = store::insert(
        &mut scope,
        store::NewRecipe {
            owner_user_id: Some(principal.user_id),
            name: &prepared.name,
            baseline_key: &prepared.baseline_key,
            tool_profile_id: None,
            tools: &prepared.tools,
            extra_packages: &prepared.extra_packages,
            repositories: &prepared.repositories,
            mcp_server_keys: &prepared.mcp_server_keys,
            agents: &prepared.agents,
            resources: &prepared.resources,
            project_mounts: &prepared.mounts,
            node_id: prepared.node.id,
        },
    )
    .await
    .map_err(recipe_write_error)?;
    audit(
        &mut scope,
        principal,
        "recipe.create",
        recipe.id,
        serde_json::json!({ "name": recipe.name, "node": prepared.node.name }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(recipe)
}

/// Replace a recipe's editable fields.
///
/// The revision increases and the state returns to draft, because the image no
/// longer matches the recipe. The old `image_ref` stays until a new build
/// replaces it: a sandbox may still be running it.
pub async fn update(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    input: RecipeInput,
) -> Result<Recipe, ApiError> {
    require(principal, Permission::RecipeWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    // The existing node is the default: a PATCH that does not name a node must
    // not silently move the recipe to the local node.
    let existing = store::get(&mut scope, id).await.map_err(recipe_error)?;
    let prepared = prepare(&mut scope, &input, Some(existing.node_id)).await?;
    let recipe = store::update(
        &mut scope,
        id,
        store::NewRecipe {
            owner_user_id: None,
            name: &prepared.name,
            baseline_key: &prepared.baseline_key,
            tool_profile_id: None,
            tools: &prepared.tools,
            extra_packages: &prepared.extra_packages,
            repositories: &prepared.repositories,
            mcp_server_keys: &prepared.mcp_server_keys,
            agents: &prepared.agents,
            resources: &prepared.resources,
            project_mounts: &prepared.mounts,
            node_id: prepared.node.id,
        },
    )
    .await
    .map_err(recipe_write_error)?;
    audit(
        &mut scope,
        principal,
        "recipe.update",
        recipe.id,
        serde_json::json!({ "name": recipe.name, "revision": recipe.revision }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(recipe)
}

/// Delete a recipe. Sandboxes created from it keep running.
pub async fn delete(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    require(principal, Permission::RecipeWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let recipe = store::get(&mut scope, id).await.map_err(recipe_error)?;
    store::delete(&mut scope, id).await.map_err(recipe_error)?;
    audit(
        &mut scope,
        principal,
        "recipe.delete",
        id,
        serde_json::json!({ "name": recipe.name }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------

/// Queue (or skip) the build of a recipe's image.
///
/// A built recipe whose content hash still matches is answered from the
/// existing image: rebuilding identical content burns minutes and produces the
/// same tag, so the honest result is "reused".
pub async fn build(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<BuildQueued, ApiError> {
    require(principal, Permission::RecipeWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let recipe = store::get(&mut scope, id).await.map_err(recipe_error)?;

    let prepared = prepare(&mut scope, &input_from(&recipe), Some(recipe.node_id)).await?;
    let hash = content_hash(
        &prepared.plan,
        &prepared.resources,
        &prepared.mounts,
        SUPERVISOR_PROGRAM,
    );
    let tag = image_tag(&prepared.name, &hash);

    // A build already in flight is the answer to a second request: queueing a
    // second one would race the first for the same row and the same tag.
    if recipe.state == RecipeState::Building {
        scope.commit().await.map_err(ApiError::from)?;
        return Ok(BuildQueued {
            job_id: recipe.build_job_id,
            recipe,
            reused: false,
        });
    }

    // The same content has an image already: a PATCH set the state to draft,
    // but the tag still names exactly this recipe.
    if recipe.image_ref.is_some() && recipe.content_hash.as_deref() == Some(hash.as_str()) {
        let built = store::set_build(
            &mut scope,
            id,
            RecipeState::Built,
            None,
            Some(hash.as_str()),
            None,
            None,
        )
        .await
        .map_err(recipe_error)?;
        scope.commit().await.map_err(ApiError::from)?;
        return Ok(BuildQueued {
            recipe: built,
            job_id: None,
            reused: true,
        });
    }

    let job = job_store::create(
        &mut scope,
        JobKind::RecipeBuild,
        &[recipe.id],
        &serde_json::json!({
            "recipe_id": recipe.id,
            "name": recipe.name,
            "tag": tag,
            "content_hash": hash,
        }),
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;

    let queued = store::set_build(
        &mut scope,
        id,
        RecipeState::Building,
        None,
        None,
        Some(job.id),
        None,
    )
    .await
    .map_err(recipe_error)?;
    audit(
        &mut scope,
        principal,
        "recipe.build",
        id,
        serde_json::json!({ "name": recipe.name, "job_id": job.id, "tag": tag }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;

    // The job runs detached: the request returns as soon as it is recorded, and
    // the outcome lands on the recipe row. A worker process would be a second
    // way to start the same work; this is the one way until there is a queue.
    let ctx = ctx.clone();
    let environment_id = principal.environment_id;
    tokio::spawn(async move {
        if let Err(error) = run_build(&ctx, environment_id, id, job.id).await {
            tracing::error!(
                code = error.code.0,
                recipe = %id,
                job = %job.id,
                "recipe build could not record its outcome"
            );
        }
    });

    Ok(BuildQueued {
        recipe: queued,
        job_id: Some(job.id),
        reused: false,
    })
}

/// The `recipe_build` job body: render, build, record.
///
/// Public so the job can be run from a test without going through an HTTP
/// request and a detached task.
/// Settle the builds a restart interrupted.
///
/// A build runs in a detached task, so a control-plane restart kills it while
/// the recipe still says `building` and the job says `running`. Nothing will
/// ever finish either, and the console would show a progress that never moves.
/// The boot calls this, records the reason on both rows, and lets the operator
/// queue the build again — a build is idempotent, it reuses an image whose
/// content hash already exists.
pub async fn settle_interrupted(ctx: &ServiceContext) -> Result<u64, ApiError> {
    // `environments` has no row-level policy by design (ADR 0004): the login path
    // must list them before a scope exists. A boot routine is the same kind of
    // cross-tenant maintenance, and it writes only within each scope below.
    let environments: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments")
        .fetch_all(&ctx.pool)
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;

    let message = "The control plane restarted before the build finished. Build the recipe again.";
    let mut settled = 0;

    for environment_id in environments {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;

        let recipes = sqlx::query(
            "UPDATE recipes
                SET state = 'failed', build_error = $2, updated_at = now()
              WHERE environment_id = $1 AND state = 'building'",
        )
        .bind(environment_id)
        .bind(message)
        .execute(scope.connection())
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?
        .rows_affected();

        sqlx::query(
            "UPDATE jobs
                SET status = 'failed', error = $2, finished_at = now(), updated_at = now()
              WHERE environment_id = $1 AND kind = 'recipe_build' AND status = 'running'",
        )
        .bind(environment_id)
        .bind(message)
        .execute(scope.connection())
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;

        scope.commit().await.map_err(ApiError::from)?;
        settled += recipes;
    }

    Ok(settled)
}

pub async fn run_build(
    ctx: &ServiceContext,
    environment_id: Uuid,
    recipe_id: Uuid,
    job_id: Uuid,
) -> Result<Recipe, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    job_store::start(&mut scope, job_id)
        .await
        .map_err(ApiError::from)?;
    tracing::info!(job = %job_id, "build: job started");
    let recipe = match store::get(&mut scope, recipe_id).await {
        Ok(recipe) => recipe,
        Err(error) => {
            // The recipe was deleted between queueing and starting: the job
            // still has to end, or the console shows "running" forever.
            let message = "The recipe no longer exists.";
            let _ = job_store::record_target(
                &mut scope,
                job_id,
                recipe_id,
                TargetState::Failed,
                Some(message),
            )
            .await;
            let _ = job_store::log(&mut scope, job_id, "error", message).await;
            let _ = job_store::finish(&mut scope, job_id, None).await;
            let _ = scope.commit().await;
            return Err(recipe_error(error));
        }
    };
    scope.commit().await.map_err(ApiError::from)?;

    // Resolve and render in their own transaction: nothing holds a connection
    // while the daemon works.
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    tracing::info!(job = %job_id, "build: preparing");
    let prepared = match prepare(&mut scope, &input_from(&recipe), Some(recipe.node_id)).await {
        Ok(prepared) => prepared,
        Err(error) => {
            scope.commit().await.map_err(ApiError::from)?;
            return record_failure(ctx, environment_id, recipe_id, job_id, &error.message).await;
        }
    };
    let hash = content_hash(
        &prepared.plan,
        &prepared.resources,
        &prepared.mounts,
        SUPERVISOR_PROGRAM,
    );
    let tag = image_tag(&prepared.name, &hash);
    let request = match render(&prepared.plan, &tag) {
        Ok(request) => request,
        Err(message) => {
            scope.commit().await.map_err(ApiError::from)?;
            return record_failure(ctx, environment_id, recipe_id, job_id, &message).await;
        }
    };
    scope.commit().await.map_err(ApiError::from)?;
    tracing::info!(job = %job_id, tag = %tag, "build: calling the runner");

    let outcome = ctx.runtime.build(request).await;
    tracing::info!(job = %job_id, "build: runner answered");
    match outcome {
        Ok(result) if result.success => {
            // A Dockerfile that ran is not a verified image: a package that the
            // index offered but the build skipped would surface later as a
            // missing tool. The image's own package database answers, and a
            // mismatch fails the build instead of being reported as success.
            let verification = ctx
                .runtime
                .verify_image(crate::domain::runtime::ImageVerification {
                    tag: tag.clone(),
                    package_manager: prepared.plan.manager.as_str().to_string(),
                    packages: prepared.plan.packages.clone(),
                })
                .await;
            match verification {
                Ok(answer) if answer.missing.is_empty() => {
                    record_success(ctx, environment_id, recipe_id, job_id, &tag, &hash, &result.log)
                        .await
                }
                Ok(answer) => {
                    let detail = format!(
                        "The image was built but {} of {} packages are missing: {}. {}",
                        answer.missing.len(),
                        prepared.plan.packages.len(),
                        answer.missing.join(", "),
                        answer.note.unwrap_or_default()
                    );
                    record_failure(ctx, environment_id, recipe_id, job_id, &detail).await
                }
                Err(error) => {
                    let detail = format!(
                        "The image was built but its packages could not be verified: {error}"
                    );
                    record_failure(ctx, environment_id, recipe_id, job_id, &detail).await
                }
            }
        }
        Ok(result) => {
            let detail = format!("The build failed: {}", tail(&result.log));
            record_failure(ctx, environment_id, recipe_id, job_id, &detail).await
        }
        Err(error) => {
            let detail = format!("The runtime refused the build: {error}");
            record_failure(ctx, environment_id, recipe_id, job_id, &detail).await
        }
    }
}

/// Write a successful build onto the recipe and its job.
async fn record_success(
    ctx: &ServiceContext,
    environment_id: Uuid,
    recipe_id: Uuid,
    job_id: Uuid,
    tag: &str,
    hash: &str,
    log: &str,
) -> Result<Recipe, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    job_store::log(&mut scope, job_id, "info", &format!("Built {tag}."))
        .await
        .map_err(ApiError::from)?;
    job_store::log(&mut scope, job_id, "info", &tail(log))
        .await
        .map_err(ApiError::from)?;
    job_store::record_target(&mut scope, job_id, recipe_id, TargetState::Succeeded, None)
        .await
        .map_err(ApiError::from)?;
    job_store::finish(
        &mut scope,
        job_id,
        Some(&serde_json::json!({ "tag": tag, "content_hash": hash })),
    )
    .await
    .map_err(ApiError::from)?;
    let recipe = store::set_build(
        &mut scope,
        recipe_id,
        RecipeState::Built,
        Some(tag),
        Some(hash),
        None,
        None,
    )
    .await
    .map_err(recipe_error)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(recipe)
}

/// Write a failed build onto the recipe and its job.
///
/// The message is redacted and truncated before it is stored: a build log can
/// quote whatever the installer printed, and a credential in a failed log is
/// still a credential.
async fn record_failure(
    ctx: &ServiceContext,
    environment_id: Uuid,
    recipe_id: Uuid,
    job_id: Uuid,
    message: &str,
) -> Result<Recipe, ApiError> {
    let message = tail(message);
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    job_store::log(&mut scope, job_id, "error", &message)
        .await
        .map_err(ApiError::from)?;
    job_store::record_target(
        &mut scope,
        job_id,
        recipe_id,
        TargetState::Failed,
        Some(&message),
    )
    .await
    .map_err(ApiError::from)?;
    job_store::finish(&mut scope, job_id, None)
        .await
        .map_err(ApiError::from)?;
    let recipe = store::set_build(
        &mut scope,
        recipe_id,
        RecipeState::Failed,
        None,
        None,
        None,
        Some(&message),
    )
    .await
    .map_err(recipe_error)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(recipe)
}

// ---------------------------------------------------------------------------
// Instantiating
// ---------------------------------------------------------------------------

/// Create a sandbox from a built recipe.
///
/// The sandbox name comes from the request because a recipe is a template: two
/// sandboxes from the same recipe must be distinguishable. Everything else —
/// image, resources, mounts, agents — comes from the recipe, so a request can
/// not widen what the operator built.
pub async fn instantiate(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    name: &str,
) -> Result<Sandbox, ApiError> {
    require(principal, Permission::RecipeWrite)?;
    require(principal, Permission::SandboxWrite)?;
    let sandbox_name = SandboxName::new(name).map_err(ApiError::validation)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let recipe = store::get(&mut scope, id).await.map_err(recipe_error)?;
    let Some(image) = recipe.image_ref.clone() else {
        return Err(ApiError::conflict(
            ErrorCode::RECIPE_NOT_BUILT,
            format!(
                "Recipe '{}' has no image yet. Build it before creating a sandbox.",
                recipe.name
            ),
        ));
    };

    let node = node_store::get(&mut scope, recipe.node_id)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(
                ErrorCode::NODE_NOT_FOUND,
                "The recipe's node no longer exists.",
            ),
            other => ApiError::from(other),
        })?;
    if !node.kind.is_operable() {
        return Err(ApiError::conflict(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!("Node '{}' has no runtime adapter yet.", node.name),
        ));
    }

    let memory_limit = normalize_memory_limit(&recipe.resources.memory_limit)
        .map_err(|message| ApiError::unprocessable(ErrorCode::RECIPE_INVALID, message))?;
    let ram_bytes = bytes_of(&memory_limit)
        .ok_or_else(|| ApiError::internal("A normalised memory limit is not readable."))?;
    let _budget = check_capacity(
        &mut scope,
        &node,
        ram_bytes,
        recipe.resources.cpu_limit,
        recipe.resources.gpu_count,
    )
    .await?;

    let mut runtime_mounts = Vec::with_capacity(recipe.project_mounts.len());
    let mut mount_rows = Vec::with_capacity(recipe.project_mounts.len());
    for mount in &recipe.project_mounts {
        let root = persistent_roots::get_root(&mut scope, mount.root_id)
            .await
            .map_err(|error| match error {
                StoreError::NotFound => ApiError::not_found(
                    ErrorCode::ROOT_NOT_FOUND,
                    format!("No persistent root {}.", mount.root_id),
                ),
                other => ApiError::from(other),
            })?;
        if root.node_id != node.id {
            return Err(ApiError::unprocessable(
                ErrorCode::RECIPE_INVALID,
                format!(
                    "Root '{}' lives on another node; a sandbox can only mount a root of its own node.",
                    root.slug
                ),
            ));
        }
        let target = crate::domain::persist::validate_mount_path(&mount.mount_path)
            .map_err(|message| ApiError::unprocessable(ErrorCode::RECIPE_INVALID, message))?;
        runtime_mounts.push(MountSpec {
            source: root.host_path.clone(),
            target: target.clone(),
            read_only: mount.read_only,
        });
        mount_rows.push((root.id, target, mount.read_only));
    }

    let cluster_id = env_store::default_cluster(&mut scope)
        .await
        .map_err(ApiError::from)?;
    let gateway_id = gateways::list(&mut scope)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .find(|gateway| gateway.accepts_placement())
        .map(|gateway| gateway.id);

    let pack_key = recipe
        .agents
        .first()
        .map(|agent| agent.pack_key.clone())
        .unwrap_or_else(|| "ubuntu-dev".to_string());
    let spec = serde_json::json!({
        "recipe_id": recipe.id,
        "recipe_name": recipe.name,
        "recipe_revision": recipe.revision,
        "image_content_hash": recipe.content_hash,
        "pack_key": pack_key,
        "memory_limit": memory_limit,
        "cpu_limit": recipe.resources.cpu_limit,
        "pids_limit": recipe.resources.pids_limit,
        "gpu_count": recipe.resources.gpu_count,
        "node": node.name,
        "agents": recipe.agents,
        "project_mounts": recipe.project_mounts,
    });

    // The row is written here rather than through `sandbox_service::create`
    // because a recipe carries a mount set, an agent set, and a content-addressed
    // image tag that `CreateSandbox` cannot express. The pieces that matter are
    // shared: the same name validation, placement rules, image, and status
    // settlement, so the runtime sees the same shape from both paths.
    let sandbox = sandbox_store::insert(
        &mut scope,
        sandbox_store::NewSandbox {
            cluster_id,
            gateway_id,
            node_id: node.id,
            name: sandbox_name.as_str(),
            pack_key: &pack_key,
            image: &image,
            // The tag is `nestbox-recipe/<name>:<content hash>`: pinned by
            // construction, so there is no floating tag to refuse. The hash is
            // recorded in `spec.image_content_hash` for the audit trail.
            image_digest: None,
            spec: &spec,
            parent_id: None,
        },
    )
    .await
    .map_err(sandbox_write_error)?;

    // Every sandbox gets its supervisor in the same transaction as the row, so
    // there is no window in which a running sandbox has no one watching it.
    let supervisor_token = match gateway_id {
        Some(gateway_id) => Some(
            crate::services::gateway_service::create_for_sandbox(
                &mut scope,
                gateway_id,
                sandbox.id,
                &sandbox.name,
            )
            .await
            .map_err(ApiError::from)?,
        ),
        None => None,
    };

    store::attach_sandbox(&mut scope, recipe.id, sandbox.id)
        .await
        .map_err(recipe_error)?;
    for key in &recipe.mcp_server_keys {
        let server = mcp_store::get_server(&mut scope, key).await.map_err(|_| {
            recipe_invalid(format!("MCP server '{key}' is not in the registry."))
        })?;
        if !server.enabled {
            return Err(recipe_invalid(format!("MCP server '{key}' is disabled.")));
        }
        mcp_store::insert_attachment(&mut scope, sandbox.id, key, true)
            .await
            .map_err(ApiError::from)?;
    }

    for (root_id, target, read_only) in &mount_rows {
        persistent_roots::insert_mount(&mut scope, *root_id, sandbox.id, target, *read_only)
            .await
            .map_err(|error| match error {
                StoreError::UniqueViolation(_) => ApiError::conflict(
                    ErrorCode::MOUNT_CONFLICT,
                    format!(
                        "Root {root_id} is already mounted read-write in another sandbox."
                    ),
                ),
                other => ApiError::from(other),
            })?;
    }
    for agent in &recipe.agents {
        persistent_roots::insert_agent(
            &mut scope,
            sandbox.id,
            &agent.pack_key,
            &agent.instance,
            agent.home_mode.as_str(),
            agent.persistence.as_str(),
            None,
            agent.port.map(i32::from),
        )
        .await
        .map_err(ApiError::from)?;
    }

    audit(
        &mut scope,
        principal,
        "recipe.instantiate",
        sandbox.id,
        serde_json::json!({
            "recipe": recipe.name,
            "sandbox": sandbox.name,
            "node": node.name,
        }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;

    // Same default as a plain sandbox: a recipe sandbox also starts on the
    // best-practice policy, and the operator replaces it the same way. A recipe
    // that shipped its own policy would be the caller naming one; today it does
    // not, so the default applies.
    crate::services::policy_service::apply_safe_defaults(
        ctx,
        principal.environment_id,
        sandbox.id,
    )
    .await;

    // A pack that talks to a model needs the endpoint and the credential in its
    // environment; the credential is resolved at use and never stored.
    let mut environment = if crate::services::sandbox_service::pack_needs_inference(&sandbox.pack_key)
    {
        match crate::services::provider_service::environment_for_pack(
            ctx,
            principal.environment_id,
            &sandbox.pack_key,
        )
        .await
        {
            Ok(values) => values.into_iter().collect(),
            Err(error) => {
                tracing::warn!(
                    code = error.code.0,
                    "provider environment unavailable; creating without it"
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    // Same rule as `sandbox_service::create`: a recipe sandbox also has no route
    // out, so the proxy is the only address it can use. The two paths must send
    // the same shape to the runtime or a recipe sandbox is a silent special case.
    environment.extend(crate::services::sandbox_service::egress_environment(
        &crate::services::sandbox_service::egress_proxy_url(),
    ));
    // The model on this node is on the runtime network; proxying it would turn
    // a local call into a denied egress request.
    crate::services::sandbox_service::add_inference_bypass_vec(&mut environment);

    // The supervisor's identity, not the node's: the token opens exactly one
    // endpoint and one row, and the node token never enters a sandbox.
    if let Some((supervisor, token)) = &supervisor_token {
        environment.push((
            "NESTBOX_SUPERVISOR_STATE".to_string(),
            crate::domain::builder::supervisor_state_directory(),
        ));
        environment.push((
            "NESTBOX_SUPERVISOR_ID".to_string(),
            supervisor.id.to_string(),
        ));
        environment.push(("NESTBOX_SUPERVISOR_TOKEN".to_string(), token.clone()));
        environment.push((
            "NESTBOX_CONTROL_URL".to_string(),
            std::env::var("NESTBOX_CONTROL_URL")
                .unwrap_or_else(|_| "http://control-api:8000".to_string()),
        ));
        // The report goes to the control plane, not to the internet. Without
        // this the supervisor's own reports are denied by the egress policy it
        // is supposed to observe.
        crate::services::sandbox_service::add_env_url_bypass_vec(
            &mut environment,
            "NESTBOX_CONTROL_URL",
        );
        // The recipe's own tool list, so "are the tools there" is measured
        // against what was asked for and not against a default invented in the
        // supervisor.
        environment.push(("NESTBOX_TOOLS".to_string(), recipe.tools.join(",")));
    }

    let runtime_spec = SandboxSpec {
        name: sandbox.name.clone(),
        image,
        // The image's own entrypoint starts the agent; a command here would
        // replace it and silently run nothing the recipe installed.
        command: Vec::new(),
        environment,
        pack_key: sandbox.pack_key.clone(),
        tool_profile: String::new(),
        memory_limit,
        cpu_limit: recipe.resources.cpu_limit,
        pids_limit: recipe.resources.pids_limit,
        mounts: runtime_mounts,
        gpu_count: recipe.resources.gpu_count,
    };

    match ctx.runtime.create(runtime_spec).await {
        Ok(facts) => {
            let health = crate::services::sandbox_service::health_from_runtime(&facts);
            settle(ctx, sandbox, SandboxStatus::Running, health).await
        }
        Err(error) => {
            tracing::error!(sandbox = %sandbox.name, error = %error, "runtime create failed");
            settle(
                ctx,
                sandbox,
                SandboxStatus::Failed,
                SandboxHealth::Unhealthy,
            )
            .await
        }
    }
}

/// Move a freshly created sandbox to its settled status and announce it.
async fn settle(
    ctx: &ServiceContext,
    sandbox: Sandbox,
    status: SandboxStatus,
    health: SandboxHealth,
) -> Result<Sandbox, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, sandbox.environment_id)
        .await
        .map_err(ApiError::from)?;
    let updated = sandbox_store::update_status(&mut scope, sandbox.id, sandbox.version, status, health)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    ctx.bus.publish(LiveEvent::SandboxStateChanged {
        sandbox_id: updated.id,
        environment_id: updated.environment_id,
        status: updated.status.as_str().to_string(),
        version: updated.version,
    });
    Ok(updated)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Turn a request into a validated recipe and the image it would produce.
///
/// Every lookup against the catalogs happens here, once, so create, update,
/// build, and instantiate agree about what a recipe means.
async fn prepare(
    scope: &mut TenantScope<'_>,
    input: &RecipeInput,
    node_id: Option<Uuid>,
) -> Result<Prepared, ApiError> {
    let name = RecipeName::new(&input.name).map_err(recipe_invalid)?;
    let node = resolve_node(scope, input.node.as_deref(), node_id).await?;
    let baseline = catalog::get_baseline(scope, &input.baseline_key)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(
                ErrorCode::BASELINE_NOT_FOUND,
                format!("No baseline '{}'.", input.baseline_key),
            ),
            other => ApiError::from(other),
        })?;

    if input.agents.len() > 32 {
        return Err(recipe_invalid("A recipe holds at most 32 agents."));
    }

    let mut seen_instances = BTreeSet::new();
    let mut planned = Vec::with_capacity(input.agents.len());
    let mut home_directories = BTreeSet::new();
    // The default home always exists, so `HOME` in the image points somewhere.
    home_directories.insert("/home/nestbox".to_string());
    let mut kinds = Vec::with_capacity(input.agents.len());

    for agent in &input.agents {
        validate_instance(&agent.instance).map_err(recipe_invalid)?;
        if !seen_instances.insert(agent.instance.clone()) {
            return Err(recipe_invalid(format!(
                "Agent instance '{}' is used more than once.",
                agent.instance
            )));
        }
        let pack = catalog::get_pack(scope, &agent.pack_key)
            .await
            .map_err(|error| match error {
                StoreError::NotFound => ApiError::not_found(
                    ErrorCode::PACK_NOT_FOUND,
                    format!("No agent pack '{}'.", agent.pack_key),
                ),
                other => ApiError::from(other),
            })?;
        if !pack.enabled {
            return Err(ApiError::not_found(
                ErrorCode::PACK_NOT_FOUND,
                format!("Agent pack '{}' is disabled.", pack.key),
            ));
        }
        if !pack.supports(&baseline.key) {
            return Err(ApiError::conflict(
                ErrorCode::PACK_INCOMPATIBLE,
                format!(
                    "Agent pack '{}' does not support baseline '{}'; it supports: {}.",
                    pack.key,
                    baseline.key,
                    pack.base_os.join(", ")
                ),
            ));
        }
        if pack.requires_gpu && !node.container_gpu {
            return Err(ApiError::unprocessable(
                ErrorCode::NODE_INSUFFICIENT_RESOURCES,
                format!(
                    "Agent pack '{}' needs a GPU; node '{}' has no GPU runtime.",
                    pack.key, node.name
                ),
            ));
        }
        if agent.port == Some(0) {
            return Err(recipe_invalid("A port is between 1 and 65535."));
        }

        let install = pack_install(&pack).map_err(recipe_invalid)?;
        let directories = agent_directories(agent, &pack).map_err(recipe_invalid)?;
        for directory in &directories {
            home_directories.insert(directory.clone());
        }
        kinds.push(pack.install_kind);
        planned.push(PlannedAgent {
            instance: agent.instance.clone(),
            pack_key: pack.key.clone(),
            install,
            home_directories: directories,
            command: pack.default_command.clone(),
            env_slots: pack.env_slots.clone(),
            home_mode: agent.home_mode,
            port: agent.port,
        });
    }

    // Tools the install kinds need, added after the operator's choices so the
    // Dockerfile reads in the order the recipe asked for.
    let mut keys = input.tools.clone();
    for key in prerequisites(&kinds) {
        if !keys.iter().any(|chosen| chosen == key) {
            keys.push(key.to_string());
        }
    }
    // The supervisor is a Python program shipped in every image and every image
    // runs one, so the interpreter is needed whether or not the recipe named an
    // agent.
    if !keys.iter().any(|chosen| chosen == "python3") {
        keys.push("python3".to_string());
    }
    let resolved = catalog::tools_by_key(scope, &keys)
        .await
        .map_err(ApiError::from)?;
    let by_key: BTreeMap<&str, &Tool> = resolved
        .iter()
        .map(|tool| (tool.key.as_str(), tool))
        .collect();
    let mut packages: Vec<String> = Vec::with_capacity(keys.len());
    for key in &keys {
        let tool = by_key.get(key.as_str()).ok_or_else(|| {
            recipe_invalid(format!("The tool catalog has no tool '{key}'."))
        })?;
        let package = tool.package_for(baseline.package_manager).ok_or_else(|| {
            recipe_invalid(format!(
                "Tool '{}' is not packaged for '{}'; choose another tool or baseline.",
                tool.name,
                baseline.package_manager.as_str()
            ))
        })?;
        let package = validate_package(package).map_err(recipe_invalid)?;
        if !packages.contains(&package) {
            packages.push(package);
        }
    }
    for package in super::selection::packages(&input.extra_packages).map_err(recipe_invalid)? {
        if !packages.contains(&package) {
            packages.push(package);
        }
    }
    let repositories = input
        .repositories
        .iter()
        .map(super::selection::repository)
        .collect::<Result<Vec<_>, _>>()
        .map_err(recipe_invalid)?;
    // An MCP assignment is checked when the recipe is saved, not when the
    // sandbox starts: a key that the registry does not have is an operator
    // typo, and finding it after a five-minute build is too late.
    for key in &input.mcp_server_keys {
        let server = mcp_store::get_server(scope, key).await.map_err(|error| match error {
            StoreError::NotFound => recipe_invalid(format!(
                "MCP server '{key}' is not in the registry."
            )),
            other => ApiError::from(other),
        })?;
        if !server.enabled {
            return Err(recipe_invalid(format!("MCP server '{key}' is disabled.")));
        }
    }

    let memory_limit = normalize_memory_limit(&input.resources.memory_limit)
        .map_err(|message| ApiError::unprocessable(ErrorCode::RECIPE_INVALID, message))?;
    if !input.resources.cpu_limit.is_finite()
        || input.resources.cpu_limit <= 0.0
        || input.resources.cpu_limit > 256.0
    {
        return Err(recipe_invalid("A CPU limit is a number above 0 and at most 256."));
    }
    if input.resources.pids_limit < 1 || input.resources.pids_limit > 65_536 {
        return Err(recipe_invalid("A PID limit is between 1 and 65536."));
    }
    if input.resources.gpu_count > 0 && !node.container_gpu {
        return Err(ApiError::unprocessable(
            ErrorCode::NODE_INSUFFICIENT_RESOURCES,
            format!(
                "The recipe asks for {} GPU(s); node '{}' has no GPU runtime.",
                input.resources.gpu_count, node.name
            ),
        ));
    }
    let resources = ResourceLimits {
        memory_limit,
        ..input.resources.clone()
    };

    let mut mounts = Vec::with_capacity(input.project_mounts.len());
    let mut paths = BTreeSet::new();
    let mut writable_roots = BTreeSet::new();
    for mount in &input.project_mounts {
        let path = crate::domain::persist::validate_mount_path(&mount.mount_path)
            .map_err(recipe_invalid)?;
        if !paths.insert(path.clone()) {
            return Err(recipe_invalid(format!(
                "Two project roots are mounted at '{path}'."
            )));
        }
        let root = persistent_roots::get_root(scope, mount.root_id)
            .await
            .map_err(|error| match error {
                StoreError::NotFound => ApiError::not_found(
                    ErrorCode::ROOT_NOT_FOUND,
                    format!("No persistent root {}.", mount.root_id),
                ),
                other => ApiError::from(other),
            })?;
        if root.archived_at.is_some() {
            return Err(recipe_invalid(format!(
                "Root '{}' is archived; a recipe mounts an active root.",
                root.slug
            )));
        }
        if root.node_id != node.id {
            return Err(recipe_invalid(format!(
                "Root '{}' lives on another node than the recipe's node.",
                root.slug
            )));
        }
        if !mount.read_only && !writable_roots.insert(mount.root_id) {
            return Err(recipe_invalid(format!(
                "Root '{}' is mounted read-write twice; one writer per root.",
                root.slug
            )));
        }
        mounts.push(ProjectMount {
            root_id: mount.root_id,
            mount_path: path,
            read_only: mount.read_only,
        });
    }

    let plan = ImagePlan {
        name: name.as_str().to_string(),
        baseline_key: baseline.key.clone(),
        base_image: baseline.base_image.clone(),
        manager: baseline.package_manager,
        packages,
        repositories,
        agents: planned,
        home_directories: home_directories.into_iter().collect(),
    };

    Ok(Prepared {
        name: name.as_str().to_string(),
        baseline_key: baseline.key,
        tools: input.tools.clone(),
        extra_packages: input.extra_packages.clone(),
        repositories: input.repositories.clone(),
        mcp_server_keys: input.mcp_server_keys.clone(),
        agents: input.agents.clone(),
        resources,
        mounts,
        node,
        plan,
    })
}

/// The recipe's stored fields in the shape `prepare` expects.
fn input_from(recipe: &Recipe) -> RecipeInput {
    RecipeInput {
        name: recipe.name.clone(),
        baseline_key: recipe.baseline_key.clone(),
        tools: recipe.tools.clone(),
        extra_packages: recipe.extra_packages.clone(),
        repositories: recipe.repositories.clone(),
        mcp_server_keys: recipe.mcp_server_keys.clone(),
        agents: recipe.agents.clone(),
        resources: recipe.resources.clone(),
        project_mounts: recipe.project_mounts.clone(),
        node: None,
    }
}

/// The node a recipe builds on: named by the request, or by id when it is
/// already stored.
async fn resolve_node(
    scope: &mut TenantScope<'_>,
    requested: Option<&str>,
    node_id: Option<Uuid>,
) -> Result<Node, ApiError> {
    // A named node wins over the stored one: that is how a recipe is moved on
    // purpose. Omitting the name keeps the node it already had.
    let lookup = match requested.filter(|name| !name.is_empty()) {
        Some(name) => node_store::get_by_name(scope, name).await,
        None => match node_id {
            Some(id) => node_store::get(scope, id).await,
            None => node_store::local(scope).await,
        },
    };
    lookup.map_err(|error| match error {
        StoreError::NotFound => ApiError::not_found(
            ErrorCode::NODE_NOT_FOUND,
            match requested {
                Some(name) if !name.is_empty() => format!("No node named '{name}'."),
                _ => "The recipe's node no longer exists.".to_string(),
            },
        ),
        other => ApiError::from(other),
    })
}

/// The material a baseline and an agent pack need before their install step.
fn prerequisites(kinds: &[InstallKind]) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = Vec::new();
    for kind in kinds {
        let needed: &[&'static str] = match kind {
            // A script installer fetches its payload over HTTPS and, in the
            // common shape, clones a repository before it runs. Hermes is one
            // of those: without `git` it stops at "git is required" after the
            // download already looked like success.
            InstallKind::Script => &["curl", "ca-certificates", "git"],
            InstallKind::Npm => &["nodejs", "npm"],
            InstallKind::Pip => &["python3", "python3-pip"],
            InstallKind::Binary => &["curl", "ca-certificates"],
            InstallKind::Git => &["git"],
            InstallKind::Image | InstallKind::None => &[],
        };
        for key in needed {
            if !keys.contains(key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// Validate and resolve a pack's install step.
///
/// A pack that is its own image is refused: a recipe composes an image from a
/// baseline, and there is no honest way to merge two base images. Such a pack
/// runs as a sandbox from its own image instead.
fn pack_install(pack: &AgentPack) -> Result<PackInstall, String> {
    let reference = pack
        .install_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let missing = || {
        format!(
            "Agent pack '{}' has no install reference for its '{}' install.",
            pack.key,
            pack.install_kind.as_str()
        )
    };
    Ok(match pack.install_kind {
        InstallKind::None => PackInstall::None,
        InstallKind::Image => {
            return Err(format!(
                "Agent pack '{}' is a prebuilt image and cannot be installed into a baseline; \
                 run it as a sandbox from its own image instead.",
                pack.key
            ))
        }
        InstallKind::Script => PackInstall::Script(validate_url(reference.ok_or_else(missing)?)?),
        InstallKind::Npm => PackInstall::Npm(validate_spec(reference.ok_or_else(missing)?)?),
        InstallKind::Pip => PackInstall::Pip(validate_spec(reference.ok_or_else(missing)?)?),
        InstallKind::Git => PackInstall::Git {
            url: validate_url(reference.ok_or_else(missing)?)?,
            directory: validate_slug(&pack.key)?,
        },
        InstallKind::Binary => {
            let url = validate_url(reference.ok_or_else(missing)?)?;
            if [".tar.gz", ".tgz", ".tar.xz", ".zip", ".gz"]
                .iter()
                .any(|suffix| url.to_ascii_lowercase().ends_with(suffix))
            {
                return Err(format!(
                    "Agent pack '{}' downloads an archive; a binary install expects a single executable.",
                    pack.key
                ));
            }
            PackInstall::Binary {
                url,
                name: validate_slug(&pack.key)?,
            }
        }
    })
}

/// The directories an agent's home needs, inside the container.
fn agent_directories(agent: &AgentSpec, pack: &AgentPack) -> Result<Vec<String>, String> {
    let home = validate_directory(&agent.home_mode.home_path(&agent.instance))?;
    let paths = if agent.home_paths.is_empty() {
        &pack.home_paths
    } else {
        &agent.home_paths
    };
    let mut directories = vec![home.clone()];
    for path in paths {
        let relative = path.trim().trim_matches('/');
        if relative.is_empty() {
            continue;
        }
        if relative.split('/').any(|part| part == ".." || part.is_empty()) {
            return Err(format!("'{path}' is not a path inside the agent's home."));
        }
        directories.push(validate_directory(&format!("{home}/{relative}"))?);
    }
    directories.sort();
    directories.dedup();
    Ok(directories)
}

// ---------------------------------------------------------------------------
// The Dockerfile generator
// ---------------------------------------------------------------------------

/// Render a plan into the request the runtime's build endpoint takes.
pub fn render(plan: &ImagePlan, tag: &str) -> Result<BuildRequest, String> {
    Ok(BuildRequest {
        tag: validate_tag(tag)?,
        dockerfile: dockerfile(plan)?,
        files: vec![
            (
                "nestbox-entrypoint.sh".to_string(),
                entrypoint_script(plan)?,
            ),
            (
                "nestbox-supervisor.sh".to_string(),
                supervisor_script(plan)?,
            ),
            (
                "nestbox-agent-supervisor".to_string(),
                SUPERVISOR_PROGRAM.to_string(),
            ),
        ],
    })
}

/// Encode a multi-line payload for a one-line Dockerfile instruction.
///
/// A signing key spans many lines, and a Dockerfile instruction ends at the
/// newline; pasting it verbatim turns the rest of the key into "unknown
/// instruction". Base64 keeps the instruction one line and the payload exact,
/// and every base image this builder supports has `base64 -d`.
fn dockerfile_payload(text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
}

/// Generate the Dockerfile for a plan.
///
/// Pure and deterministic: the same plan produces the same file, which is what
/// makes the content hash meaningful. Every value is validated before it is
/// written and quoted after, so a crafted catalog row cannot become a second
/// command.
pub fn dockerfile(plan: &ImagePlan) -> Result<String, String> {
    let base = validate_image(&plan.base_image)?;
    let manager = plan.manager;
    let mut out = String::new();

    out.push_str(&format!(
        "# Generated by Nestbox from recipe '{}'.\n",
        validate_slug(&plan.name)?
    ));
    out.push_str("# Do not edit: the next build regenerates this file from the recipe.\n");
    out.push_str(&format!("FROM {base}\n"));

    // A repository is reached over HTTPS, and a minimal base image has no trust
    // store: without ca-certificates `apt-get update` answers "No system
    // certificates available" before the repository is ever used.
    if !plan.repositories.is_empty() {
        match manager {
            PackageManager::Apt => out.push_str(
                "RUN apt-get update \\\n && apt-get install -y --no-install-recommends ca-certificates \\\n && rm -rf /var/lib/apt/lists/*\n",
            ),
            PackageManager::Dnf => out.push_str(
                "RUN dnf install -y ca-certificates \\\n && dnf clean all\n",
            ),
            PackageManager::Apk => out.push_str("RUN apk add --no-cache ca-certificates\n"),
        }
    }

    for repository in &plan.repositories {
        let name = super::selection::repository_name(&repository.name)?;
        let url = super::selection::repository_url(&repository.url)?;
        match manager {
            PackageManager::Apt => {
                let suite = repository.suite.as_deref().unwrap_or("stable");
                let components = repository.components.join(" ");
                let line = if repository.signing_key.is_some() {
                    format!("deb [signed-by=/etc/apt/keyrings/nestbox-{name}.asc] {url} {suite} {components}\n")
                } else {
                    format!("deb {url} {suite} {components}\n")
                };
                if let Some(key) = &repository.signing_key {
                    out.push_str(&format!(
                        "RUN mkdir -p /etc/apt/keyrings && echo {} | base64 -d > /etc/apt/keyrings/nestbox-{name}.asc\n",
                        dockerfile_payload(key)
                    ));
                }
                out.push_str(&format!(
                    "RUN echo {} | base64 -d > /etc/apt/sources.list.d/nestbox-{name}.list\n",
                    dockerfile_payload(&line)
                ));
            }
            PackageManager::Dnf => {
                let mut block = format!("[nestbox-{name}]\nname=Nestbox {name}\nbaseurl={url}\nenabled=1\n");
                if let Some(key) = &repository.signing_key {
                    block.push_str(&format!("gpgcheck=1\ngpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-nestbox-{name}\n"));
                    out.push_str(&format!(
                        "RUN mkdir -p /etc/pki/rpm-gpg && echo {} | base64 -d > /etc/pki/rpm-gpg/RPM-GPG-KEY-nestbox-{name}\n",
                        dockerfile_payload(key)
                    ));
                } else {
                    block.push_str("gpgcheck=0\n");
                }
                out.push_str(&format!(
                    "RUN echo {} | base64 -d > /etc/yum.repos.d/nestbox-{name}.repo\n",
                    dockerfile_payload(&block)
                ));
            }
            PackageManager::Apk => {
                // Alpine keeps every repository in one file; the line is the
                // repository URL and its optional tag, nothing else.
                out.push_str(&format!(
                    "RUN echo {} | base64 -d >> /etc/apk/repositories\n",
                    dockerfile_payload(&format!("{url}\n"))
                ));
                if let Some(key) = &repository.signing_key {
                    out.push_str(&format!(
                        "RUN echo {} | base64 -d > /etc/apk/keys/nestbox-{name}.rsa.pub\n",
                        dockerfile_payload(key)
                    ));
                }
            }
        }
    }

    // The homes and the search path come first: an installer that writes to
    // `~/.local/bin` (the common shape for an agent CLI) needs both to exist
    // before it runs, and the supervisor later needs the same PATH.
    let directories: Vec<String> = plan
        .home_directories
        .iter()
        .map(|directory| validate_directory(directory))
        .collect::<Result<_, _>>()?;
    let home = directories
        .first()
        .cloned()
        .unwrap_or_else(|| "/home/nestbox".to_string());
    out.push_str("\n# Homes and the persisted paths inside them.\n");
    out.push_str("# A bind mount replaces a home at run time; the copy here keeps\n");
    out.push_str("# an unmounted container usable.\n");
    out.push_str(&format!(
        "RUN mkdir -p {}\n",
        directories
            .iter()
            .map(|directory| shell_quote(directory))
            .collect::<Vec<_>>()
            .join(" ")
    ));
    // No quotes here: ENV and WORKDIR are Dockerfile syntax, not a shell, so a
    // quote would become part of the value. The path is validated above.
    out.push_str(&format!("ENV HOME={home}\n"));
    out.push_str(&format!("ENV PATH={home}/.local/bin:$PATH\n"));

    if !plan.packages.is_empty() {
        let packages = plan.packages.join(" ");
        match manager {
            PackageManager::Apt => {
                out.push_str("\n# Baseline tools and the tools the recipe chose.\n");
                out.push_str("ENV DEBIAN_FRONTEND=noninteractive\n");
                out.push_str(&format!(
                    "RUN apt-get update \\\n && apt-get install -y --no-install-recommends {packages} \\\n && rm -rf /var/lib/apt/lists/*\n"
                ));
            }
            PackageManager::Dnf => {
                out.push_str("\n# Baseline tools and the tools the recipe chose.\n");
                out.push_str(&format!(
                    "RUN dnf install -y {packages} \\\n && dnf clean all\n"
                ));
            }
            PackageManager::Apk => {
                out.push_str("\n# Baseline tools and the tools the recipe chose.\n");
                out.push_str(&format!("RUN apk add --no-cache {packages}\n"));
            }
        }
    }

    for agent in &plan.agents {
        let instance = validate_slug(&agent.instance)?;
        let pack = validate_slug(&agent.pack_key)?;
        out.push_str(&format!("\n# Agent '{instance}' from pack '{pack}'.\n"));
        out.push_str(&install_step(&agent.install)?);
    }

    out.push_str("\nCOPY nestbox-entrypoint.sh /usr/local/bin/nestbox-entrypoint\n");
    out.push_str("COPY nestbox-supervisor.sh /usr/local/bin/nestbox-supervisor\n");
    out.push_str("COPY nestbox-agent-supervisor /usr/local/bin/nestbox-agent-supervisor\n");
    out.push_str("RUN chmod 0755 /usr/local/bin/nestbox-entrypoint /usr/local/bin/nestbox-supervisor /usr/local/bin/nestbox-agent-supervisor\n");
    out.push_str(&format!("WORKDIR {home}\n"));
    out.push_str("ENTRYPOINT [\"/usr/local/bin/nestbox-entrypoint\"]\n");
    out.push_str("CMD [\"sleep\", \"infinity\"]\n");
    Ok(out)
}

/// The entrypoint script written next to the Dockerfile.
///
/// It exists because a mounted home may be empty: the container prepares its
/// directories, then runs the image's command or waits.
pub fn entrypoint_script(plan: &ImagePlan) -> Result<String, String> {
    let mut script = String::from(
        "#!/bin/sh\n# Generated by Nestbox: prepare the agent homes, then run the command.\nset -eu\n",
    );
    for directory in &plan.home_directories {
        let directory = validate_directory(directory)?;
        script.push_str(&format!(
            "mkdir -p {} 2>/dev/null || true\n",
            shell_quote(&directory)
        ));
    }
    script.push_str(
        "# The supervisor starts every agent the recipe named and ends the\n\
         # container when one of them stops.\n\
         exec /usr/local/bin/nestbox-supervisor\n",
    );
    Ok(script)
}

/// The supervisor script: one process per agent, one `wait`.
///
/// It is the same generator the agent API answers with, so the image and the
/// API cannot disagree about what a sandbox is running. The image writes it to
/// `/usr/local/bin/nestbox-supervisor`; the entrypoint execs it.
pub fn supervisor_script(plan: &ImagePlan) -> Result<String, String> {
    let mut launches: Vec<AgentLaunch> = plan
        .agents
        .iter()
        .map(|agent| AgentLaunch {
            instance: agent.instance.clone(),
            home_mode: agent.home_mode,
            command: agent.command.clone(),
            env_slots: agent.env_slots.clone(),
            port: agent.port,
        })
        .collect();
    // The supervisor runs beside the agents, in its own home, and reports to the
    // control plane. It is not a pack: it is part of every image this builder
    // produces, which is what makes "every sandbox has a supervisor" true
    // without an operator adding one.
    launches.push(AgentLaunch {
        instance: "supervisor".to_string(),
        home_mode: HomeMode::Isolated,
        command: vec![
            "python3".to_string(),
            "/usr/local/bin/nestbox-agent-supervisor".to_string(),
        ],
        env_slots: vec![
            "NESTBOX_SUPERVISOR_ID".to_string(),
            "NESTBOX_SUPERVISOR_TOKEN".to_string(),
            "NESTBOX_CONTROL_URL".to_string(),
            "NESTBOX_SUPERVISOR_INTERVAL".to_string(),
            "NESTBOX_SUPERVISOR_STATE".to_string(),
            "NESTBOX_TOOLS".to_string(),
        ],
        port: None,
    });
    let command = crate::domain::builder::entrypoint_command(&launches);
    match command.as_slice() {
        // The shebang is not decoration: without it the kernel hands the file
        // to /bin/sh, dash has no `wait -n`, and the sandbox dies at start with
        // "wait: Illegal option -n".
        [shell, flag, script] if shell == "/bin/bash" && flag == "-c" => {
            Ok(format!("#!/bin/bash\n{script}"))
        }
        _ => Err("The supervisor generator did not produce a bash script.".to_string()),
    }
}

/// One `RUN` line per install kind.
fn install_step(install: &PackInstall) -> Result<String, String> {
    Ok(match install {
        PackInstall::None => {
            "# No install step: this pack is the runtime itself.\n".to_string()
        }
        PackInstall::Script(url) => {
            let url = validate_url(url)?;
            // Downloaded, made executable, then run: the script's own shebang
            // decides its interpreter, which piping into `sh` would override.
            format!(
                "RUN curl -fsSL --proto '=https' --tlsv1.2 -o /tmp/nestbox-install.sh {} \\\n && chmod 0755 /tmp/nestbox-install.sh \\\n && /tmp/nestbox-install.sh \\\n && rm -f /tmp/nestbox-install.sh\n",
                shell_quote(&url)
            )
        }
        PackInstall::Npm(spec) => {
            let spec = validate_spec(spec)?;
            format!(
                "RUN npm install --global {} \\\n && npm cache clean --force\n",
                shell_quote(&spec)
            )
        }
        PackInstall::Pip(spec) => {
            let spec = validate_spec(spec)?;
            format!(
                "RUN python3 -m pip install --no-cache-dir --break-system-packages {}\n",
                shell_quote(&spec)
            )
        }
        PackInstall::Git { url, directory } => {
            let url = validate_url(url)?;
            let directory = validate_slug(directory)?;
            format!(
                "RUN git clone --depth 1 {} {}\n",
                shell_quote(&url),
                shell_quote(&format!("/opt/{directory}"))
            )
        }
        PackInstall::Binary { url, name } => {
            let url = validate_url(url)?;
            let name = validate_slug(name)?;
            let target = shell_quote(&format!("/usr/local/bin/{name}"));
            format!(
                "RUN curl -fsSL --proto '=https' --tlsv1.2 -o {target} {} \\\n && chmod 0755 {target}\n",
                shell_quote(&url)
            )
        }
    })
}

/// The image tag for a recipe name and content hash.
pub fn image_tag(name: &str, hash: &str) -> String {
    format!("nestbox-recipe/{name}:{hash}")
}

/// The content hash of a recipe: what the image is addressed by.
///
/// Deliberately excludes the id, the revision, the node, and the build state:
/// they describe the row, not the image. It includes everything that changes
/// the Dockerfile, plus the template version, so a rebuild after a generator
/// change does not silently reuse the old template's image.
pub fn content_hash(
    plan: &ImagePlan,
    resources: &ResourceLimits,
    mounts: &[ProjectMount],
    supervisor_program: &str,
) -> String {
    let agents: Vec<serde_json::Value> = plan
        .agents
        .iter()
        .map(|agent| {
            serde_json::json!({
                "instance": agent.instance,
                "pack_key": agent.pack_key,
                "install": match &agent.install {
                    PackInstall::None => serde_json::json!({ "kind": "none" }),
                    PackInstall::Script(url) => serde_json::json!({ "kind": "script", "url": url }),
                    PackInstall::Npm(spec) => serde_json::json!({ "kind": "npm", "spec": spec }),
                    PackInstall::Pip(spec) => serde_json::json!({ "kind": "pip", "spec": spec }),
                    PackInstall::Binary { url, name } => {
                        serde_json::json!({ "kind": "binary", "url": url, "name": name })
                    }
                    PackInstall::Git { url, directory } => {
                        serde_json::json!({ "kind": "git", "url": url, "directory": directory })
                    }
                },
                "homes": agent.home_directories,
            })
        })
        .collect();
    let mounts: Vec<serde_json::Value> = mounts
        .iter()
        .map(|mount| {
            serde_json::json!({
                "root_id": mount.root_id,
                "mount_path": mount.mount_path,
                "read_only": mount.read_only,
            })
        })
        .collect();

    let mut program_hasher = Sha256::new();
    program_hasher.update(supervisor_program.as_bytes());
    let program_digest: String = program_hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();

    let canonical = serde_json::json!({
        "generator": GENERATOR_VERSION,
        "supervisor_program": program_digest,
        "name": plan.name,
        "baseline": plan.baseline_key,
        "base_image": plan.base_image,
        "package_manager": plan.manager.as_str(),
        "packages": plan.packages,
        "repositories": plan.repositories,
        "agents": agents,
        "homes": plan.home_directories,
        "resources": {
            "memory_limit": resources.memory_limit,
            "cpu_limit": resources.cpu_limit,
            "pids_limit": resources.pids_limit,
            "gpu_count": resources.gpu_count,
        },
        "mounts": mounts,
    });

    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string().as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Node capacity
// ---------------------------------------------------------------------------

/// What a node has left for a sandbox, from its last measurement.
///
/// Pure so the arithmetic is testable without a database: total from the
/// hardware profile, reservations from the model installs, never below zero.
pub fn budget_from(
    profile: &HardwareProfile,
    reserved: (i64, i64, i64),
    sandboxes: i32,
) -> NodeBudget {
    let ram_total = profile.addressable_ram_bytes();
    let vram_total = profile.addressable_memory_bytes();
    NodeBudget {
        ram_total_bytes: ram_total,
        ram_reserved_bytes: reserved.0,
        ram_free_bytes: (ram_total - reserved.0).max(0),
        vram_total_bytes: vram_total,
        vram_reserved_bytes: reserved.1,
        vram_free_bytes: (vram_total - reserved.1).max(0),
        cpu_cores: profile.cores(),
        gpu_count: profile.gpu_count(),
        installs: reserved.2 as i32,
        sandboxes,
    }
}

/// Refuse a sandbox the node cannot host, with the numbers that decided it.
async fn check_capacity(
    scope: &mut TenantScope<'_>,
    node: &Node,
    ram_bytes: i64,
    cores: f64,
    gpus: u32,
) -> Result<Option<NodeBudget>, ApiError> {
    if gpus > 0 && !node.container_gpu {
        return Err(ApiError::unprocessable(
            ErrorCode::NODE_INSUFFICIENT_RESOURCES,
            format!(
                "The sandbox asks for {gpus} GPU(s); node '{}' has no GPU runtime.",
                node.name
            ),
        ));
    }
    let profile = models::latest_hardware(scope, node.id)
        .await
        .map_err(ApiError::from)?;
    let Some(profile) = profile else {
        // No measurement yet is not a refusal: the node has not been probed, so
        // a number here would be invented. The probe fills it in.
        tracing::warn!(
            node = %node.name,
            "no hardware profile for this node; placing without a capacity check"
        );
        return Ok(None);
    };
    let reserved = model_installs::reserved_on(scope, node.id)
        .await
        .map_err(ApiError::from)?;
    let counts = node_store::sandbox_counts(scope)
        .await
        .map_err(ApiError::from)?;
    let sandboxes = counts.get(&node.id).copied().unwrap_or(0) as i32;
    let budget = budget_from(&profile, reserved, sandboxes);

    budget
        .can_host(ram_bytes, cores)
        .map_err(|why: InsufficientResources| {
            ApiError::unprocessable(
                ErrorCode::NODE_INSUFFICIENT_RESOURCES,
                format!("Node '{}' cannot host this sandbox: {}", node.name, why.message()),
            )
            .with_details(serde_json::json!({
                "node": node.name,
                "reason": why.message(),
                "requested_ram_bytes": ram_bytes,
                "free_ram_bytes": budget.ram_free_bytes,
                "reserved_ram_bytes": budget.ram_reserved_bytes,
                "reserved_vram_bytes": budget.vram_reserved_bytes,
                "cpu_cores": budget.cpu_cores,
            }))
        })?;
    Ok(Some(budget))
}

/// Bytes from the normalised `<n>b` form.
fn bytes_of(normalised: &str) -> Option<i64> {
    normalised.strip_suffix('b')?.parse::<i64>().ok()
}

// ---------------------------------------------------------------------------
// Value validation
// ---------------------------------------------------------------------------

/// A task or pack name that becomes part of a path or comment.
fn validate_slug(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 63 {
        return Err("A build name is 1 to 63 characters.".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || value.starts_with('-')
    {
        return Err(format!(
            "'{value}' must be lowercase letters, digits, and hyphens."
        ));
    }
    Ok(value.to_string())
}

/// An agent instance name: the same shape the database accepts.
fn validate_instance(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 32 {
        return Err("An agent instance is 1 to 32 characters.".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || value.starts_with('-')
    {
        return Err(format!(
            "Agent instance '{value}' uses lowercase letters, digits, and hyphens."
        ));
    }
    Ok(())
}

/// A base image reference: no scheme, no shell metacharacters, no spaces.
fn validate_image(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 255 {
        return Err("A base image reference is 1 to 255 characters.".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-/:@".contains(c))
    {
        return Err(format!(
            "Base image '{value}' contains characters that are not allowed in an image reference."
        ));
    }
    Ok(value.to_string())
}

/// A package name for apt or dnf.
fn validate_package(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return Err("A package name is 1 to 128 characters.".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "+._-".contains(c))
    {
        return Err(format!("'{value}' is not a package name."));
    }
    Ok(value.to_string())
}

/// A package spec for npm or pip: a name, optionally scoped, optionally versioned.
fn validate_spec(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 {
        return Err("A package spec is 1 to 256 characters.".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "@/._+~^=<>!-".contains(c))
    {
        return Err(format!("'{value}' is not a package specification."));
    }
    Ok(value.to_string())
}

/// An install URL. HTTPS only, no characters that could end the quoting.
fn validate_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 1024 {
        return Err("An install URL is 1 to 1024 characters.".to_string());
    }
    if !value.starts_with("https://") {
        return Err(format!("'{value}' is not an HTTPS URL."));
    }
    if value.chars().any(|c| {
        c.is_whitespace() || c.is_control() || matches!(c, '\'' | '"' | '`' | '\\' | '$' | '|' | ';')
    }) {
        return Err(format!(
            "'{value}' contains characters that are not allowed in an install URL."
        ));
    }
    Ok(value.to_string())
}

/// An absolute directory inside the image.
fn validate_directory(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() || !value.starts_with('/') || value.len() > 512 {
        return Err(format!("'{value}' is not an absolute directory."));
    }
    if value.split('/').any(|part| part == "..") {
        return Err(format!("'{value}' contains '..'."));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
    {
        return Err(format!(
            "'{value}' contains characters that are not allowed in a path."
        ));
    }
    Ok(value.to_string())
}

/// A value that reaches a build tag.
fn validate_tag(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 255 {
        return Err("A build tag is 1 to 255 characters.".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-/:".contains(c))
    {
        return Err(format!("'{value}' is not a valid build tag."));
    }
    Ok(value.to_string())
}

/// Wrap a validated value in single quotes, escaping any quote that got through.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The last few KiB of a build log, so a noisy installer cannot fill a column.
fn tail(value: &str) -> String {
    const LIMIT: usize = 4_000;
    let redacted = crate::security::redaction::redact(value);
    if redacted.len() <= LIMIT {
        return redacted;
    }
    // Cut on a character boundary: a multi-byte character split in half would
    // panic in `String::from_utf8_lossy` country, and the boundary is cheap.
    let mut start = redacted.len() - LIMIT;
    while !redacted.is_char_boundary(start) {
        start += 1;
    }
    redacted[start..].to_string()
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// A recipe-specific refusal: well-formed but not buildable.
fn recipe_invalid(message: impl Into<String>) -> ApiError {
    ApiError::unprocessable(ErrorCode::RECIPE_INVALID, message)
}

/// Map a recipe read or write: a missing row is the recipe's own code.
fn recipe_error(error: StoreError) -> ApiError {
    match error {
        StoreError::NotFound => ApiError::not_found(
            ErrorCode::RECIPE_NOT_FOUND,
            "No such recipe.",
        ),
        other => ApiError::from(other),
    }
}

/// Map a recipe write, where a duplicate name is a conflict rather than a bug.
fn recipe_write_error(error: StoreError) -> ApiError {
    match error {
        StoreError::UniqueViolation(_) => ApiError::conflict(
            ErrorCode::CONFLICT,
            "A recipe with that name already exists in this environment.",
        ),
        other => recipe_error(other),
    }
}

/// Map a sandbox insert, where a duplicate name is the sandbox's code.
fn sandbox_write_error(error: StoreError) -> ApiError {
    match error {
        StoreError::UniqueViolation(_) => ApiError::conflict(
            ErrorCode::SANDBOX_NAME_TAKEN,
            "A sandbox with that name already exists in this environment.",
        ),
        other => ApiError::from(other),
    }
}

/// Record one builder action in the audit trail.
async fn audit(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    kind: &str,
    subject_id: Uuid,
    detail: serde_json::Value,
) -> Result<(), ApiError> {
    crate::store::postgres::audit::record(
        scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind,
            subject_type: Some("recipe"),
            subject_id: Some(subject_id),
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::builder::HomeMode;

    fn plan(manager: PackageManager, packages: &[&str]) -> ImagePlan {
        ImagePlan {
            name: "dev-box".into(),
            baseline_key: "ubuntu-24.04".into(),
            base_image: "ubuntu:24.04".into(),
            manager,
            packages: packages.iter().map(|p| p.to_string()).collect(),
            repositories: vec![],
            agents: vec![PlannedAgent {
                instance: "hermes".into(),
                pack_key: "hermes".into(),
                install: PackInstall::Script("https://example.test/install.sh".into()),
                home_directories: vec!["/home/nestbox".into(), "/home/nestbox/.hermes".into()],
                command: vec!["sleep".into(), "infinity".into()],
                env_slots: vec!["OPENAI_API_KEY".into()],
                home_mode: HomeMode::Shared,
                port: None,
            }],
            home_directories: vec!["/home/nestbox".into(), "/home/nestbox/.hermes".into()],
        }
    }

    fn resources() -> ResourceLimits {
        ResourceLimits {
            memory_limit: "2147483648b".into(),
            cpu_limit: 2.0,
            gpu_count: 0,
            pids_limit: 512,
        }
    }

    #[test]
    fn an_apt_recipe_installs_the_apt_names() {
        let text = dockerfile(&plan(PackageManager::Apt, &["vim", "build-essential"]))
            .expect("dockerfile");
        assert!(text.starts_with("# Generated by Nestbox"));
        assert!(text.contains("FROM ubuntu:24.04\n"));
        assert!(text.contains("apt-get install -y --no-install-recommends vim build-essential"));
        assert!(text.contains("rm -rf /var/lib/apt/lists/*"));
    }

    #[test]
    fn a_dnf_recipe_installs_the_rpm_names() {
        let mut rocky = plan(PackageManager::Dnf, &["vim-enhanced", "openssh-clients"]);
        rocky.baseline_key = "rocky-9".into();
        rocky.base_image = "rockylinux:9".into();
        let text = dockerfile(&rocky).expect("dockerfile");
        assert!(text.contains("FROM rockylinux:9\n"));
        assert!(text.contains("dnf install -y vim-enhanced openssh-clients"));
        assert!(text.contains("dnf clean all"));
        assert!(!text.contains("apt-get"));
    }

    #[test]
    fn a_tool_without_a_name_for_the_manager_is_an_error_not_a_skip() {
        // build-essential has no rpm name in the seed, so composing it for dnf
        // must fail rather than quietly build an image without the compiler.
        let tool = Tool {
            key: "build-essential".into(),
            name: "Build tools".into(),
            group: "build".into(),
            description: None,
            apt_name: Some("build-essential".into()),
            rpm_name: None,
            apk_name: Some("build-base".into()),
        };
        let package = tool.package_for(PackageManager::Dnf);
        assert_eq!(package, None);
        // The resolver in `prepare` turns that into a refusal; here the point
        // is that it never becomes a silent empty package list.
        assert!(validate_package("build-essential").is_ok());
    }

    #[test]
    fn every_install_kind_renders_a_step() {
        let cases = [
            (PackInstall::None, "No install step"),
            (PackInstall::Script("https://example.test/i.sh".into()), "nestbox-install.sh"),
            (PackInstall::Npm("@scope/pkg@1.2.3".into()), "npm install --global"),
            (PackInstall::Pip("pkg==1.0".into()), "python3 -m pip install"),
            (
                PackInstall::Binary {
                    url: "https://example.test/tool".into(),
                    name: "tool".into(),
                },
                "/usr/local/bin/tool",
            ),
            (
                PackInstall::Git {
                    url: "https://example.test/repo.git".into(),
                    directory: "tool".into(),
                },
                "git clone --depth 1",
            ),
        ];
        for (install, expected) in cases {
            let step = install_step(&install).expect("step");
            assert!(step.contains(expected), "{install:?} produced {step}");
        }
    }

    #[test]
    fn an_image_pack_is_refused_instead_of_guessed() {
        let pack = pack_of(InstallKind::Image, Some("nestbox-openclaw:local"));
        let error = pack_install(&pack).expect_err("an image pack cannot compose");
        assert!(error.contains("prebuilt image"));
    }

    fn pack_of(kind: InstallKind, reference: Option<&str>) -> AgentPack {
        AgentPack {
            key: "openclaw".into(),
            name: "OpenClaw".into(),
            vendor: None,
            description: None,
            base_os: vec![],
            install_kind: kind,
            install_ref: reference.map(str::to_string),
            home_paths: vec![".openclaw".into()],
            requires_gpu: false,
            needs_inference: false,
            default_image: "nestbox-openclaw:local".into(),
            default_command: vec![],
            ports: vec![],
            env_slots: vec![],
            enabled: true,
        }
    }

    #[test]
    fn a_hostile_value_never_reaches_the_dockerfile() {
        assert!(validate_package("vim; rm -rf /").is_err());
        assert!(validate_package("$(reboot)").is_err());
        assert!(validate_url("https://example.test/x; reboot").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_directory("/home/../../etc").is_err());
        assert!(validate_image("ubuntu:24.04 && reboot").is_err());
        assert!(validate_spec("@scope/pkg' ; reboot").is_err());

        // A quote is escaped, not trusted, even if validation were loosened.
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn the_persisted_home_directories_are_created_and_the_supervisor_started() {
        let request = render(&plan(PackageManager::Apt, &["vim"]), "nestbox-recipe/dev-box:abc")
            .expect("request");
        assert!(request.dockerfile.contains(
            "RUN mkdir -p '/home/nestbox' '/home/nestbox/.hermes'"
        ));
        assert!(request
            .dockerfile
            .contains("ENTRYPOINT [\"/usr/local/bin/nestbox-entrypoint\"]"));
        assert!(request.dockerfile.contains("ENV HOME=/home/nestbox"));
        assert!(request.dockerfile.contains("WORKDIR /home/nestbox"));
        // Every script ships, or the entrypoint execs a file that does not exist.
        let names: Vec<&str> = request.files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "nestbox-entrypoint.sh",
                "nestbox-supervisor.sh",
                "nestbox-agent-supervisor",
            ]
        );

        // The supervisor program is a real Python file, not an empty placeholder:
        // a truncated include would still build and only fail at run time.
        let (_, program) = &request.files[2];
        assert!(program.starts_with("#!/usr/bin/env python3"), "the program needs its shebang");
        assert!(program.contains("NESTBOX_SUPERVISOR_ID"));

        let (_, entrypoint) = &request.files[0];
        assert!(entrypoint.starts_with("#!/bin/sh"));
        assert!(entrypoint.contains("exec /usr/local/bin/nestbox-supervisor"));

        // The supervisor is the same generator the agents API answers with, so
        // the image and the API cannot disagree about what is running.
        let (_, supervisor) = &request.files[1];
        assert!(supervisor.starts_with("#!/bin/bash\n"), "the kernel needs the shebang");
        assert!(supervisor.contains("mkdir -p /home/nestbox"));
        assert!(supervisor.contains("exec sleep infinity"));
        assert!(supervisor.contains("wait -n"));
        assert!(supervisor.contains("export OPENAI_API_KEY="));
    }

    #[test]
    fn a_signed_repository_is_registered_before_the_packages_are_installed() {
        let mut plan = plan(PackageManager::Apt, &["nginx"]);
        plan.repositories = vec![crate::domain::builder::RepositorySpec {
            name: "artgate".into(),
            url: "https://packages.example.test/apt".into(),
            suite: Some("stable".into()),
            components: vec!["main".into()],
            signing_key: Some("-----BEGIN PGP PUBLIC KEY BLOCK-----\nabc\n-----END-----\n".into()),
            credential_ref: None,
        }];
        let text = dockerfile(&plan).expect("a valid repository renders");
        let trust = text.find("install -y --no-install-recommends ca-certificates")
            .expect("the trust store is installed before the repository");
        let keyring = text.find("/etc/apt/keyrings/nestbox-artgate.asc").expect("keyring written");
        let list = text.find("/etc/apt/sources.list.d/nestbox-artgate.list").expect("repo written");
        let install = text
            .find("apt-get install -y --no-install-recommends nginx")
            .expect("install step");
        assert!(
            trust < keyring && keyring < install && list < install,
            "the trust store, then the repository, then the install"
        );
        let encoded = dockerfile_payload(
            "deb [signed-by=/etc/apt/keyrings/nestbox-artgate.asc] https://packages.example.test/apt stable main\n",
        );
        assert!(text.contains(&encoded), "the source line is carried as one line");
    }

    #[test]
    fn an_alpine_recipe_uses_apk() {
        let mut plan = plan(PackageManager::Apk, &["nginx"]);
        plan.base_image = "alpine:3.21".into();
        let text = dockerfile(&plan).expect("a valid plan renders");
        assert!(text.contains("FROM alpine:3.21"));
        assert!(text.contains("apk add --no-cache nginx"));
        assert!(!text.contains("apt-get"));
    }

    #[test]
    fn the_content_hash_changes_when_a_repository_is_added() {
        let plain = plan(PackageManager::Apt, &["nginx"]);
        let mut with_repo = plan(PackageManager::Apt, &["nginx"]);
        with_repo.repositories = vec![crate::domain::builder::RepositorySpec {
            name: "artgate".into(),
            url: "https://packages.example.test/apt".into(),
            suite: None,
            components: vec![],
            signing_key: None,
            credential_ref: None,
        }];
        let a = content_hash(&plain, &resources(), &[], SUPERVISOR_PROGRAM);
        let b = content_hash(&with_repo, &resources(), &[], SUPERVISOR_PROGRAM);
        assert_ne!(a, b, "a new repository must produce a new image tag");
    }

    #[test]
    fn the_content_hash_is_stable_for_the_same_recipe() {
        let a = content_hash(&plan(PackageManager::Apt, &["vim"]), &resources(), &[], SUPERVISOR_PROGRAM);
        let b = content_hash(&plan(PackageManager::Apt, &["vim"]), &resources(), &[], SUPERVISOR_PROGRAM);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// A fix inside the supervisor program must produce a different image.
    ///
    /// The program is embedded in the binary and copied into every image, so a
    /// change to it is a change to what the sandbox runs — even when the recipe
    /// itself did not move.
    #[test]
    fn a_changed_supervisor_program_changes_the_hash() {
        let plan = plan(PackageManager::Apt, &["vim"]);
        let before = content_hash(&plan, &resources(), &[], "print('one')");
        let after = content_hash(&plan, &resources(), &[], "print('two')");
        assert_ne!(before, after);
    }

    #[test]
    fn one_changed_field_changes_the_hash() {
        let base = content_hash(&plan(PackageManager::Apt, &["vim"]), &resources(), &[], SUPERVISOR_PROGRAM);

        let more_packages =
            content_hash(&plan(PackageManager::Apt, &["vim", "git"]), &resources(), &[], SUPERVISOR_PROGRAM);
        assert_ne!(base, more_packages);

        let mut other_resources = resources();
        other_resources.cpu_limit = 3.0;
        assert_ne!(
            base,
            content_hash(&plan(PackageManager::Apt, &["vim"]), &other_resources, &[], SUPERVISOR_PROGRAM)
        );

        let mount = ProjectMount {
            root_id: Uuid::nil(),
            mount_path: "/workspace".into(),
            read_only: false,
        };
        assert_ne!(
            base,
            content_hash(&plan(PackageManager::Apt, &["vim"]), &resources(), &[mount], SUPERVISOR_PROGRAM)
        );

        let mut another_agent = plan(PackageManager::Apt, &["vim"]);
        another_agent.agents[0].instance = "other".into();
        assert_ne!(base, content_hash(&another_agent, &resources(), &[], SUPERVISOR_PROGRAM));
    }

    #[test]
    fn the_image_tag_carries_the_name_and_the_hash() {
        let hash = content_hash(&plan(PackageManager::Apt, &["vim"]), &resources(), &[], SUPERVISOR_PROGRAM);
        let tag = image_tag("dev-box", &hash);
        assert!(tag.starts_with("nestbox-recipe/dev-box:"));
        assert_eq!(tag.len(), "nestbox-recipe/dev-box:".len() + 64);
    }

    #[test]
    fn the_install_kinds_bring_what_they_need() {
        assert_eq!(
            prerequisites(&[InstallKind::Npm]),
            vec!["nodejs", "npm"]
        );
        // A script installer alone still needs `git`: the payload is usually a
        // repository, and Hermes stops with "git is required" without it.
        assert_eq!(
            prerequisites(&[InstallKind::Script]),
            vec!["curl", "ca-certificates", "git"]
        );
        assert_eq!(
            prerequisites(&[InstallKind::Script, InstallKind::Git]),
            vec!["curl", "ca-certificates", "git"]
        );
        assert!(prerequisites(&[InstallKind::None, InstallKind::Image]).is_empty());
    }

    #[test]
    fn an_agents_directories_follow_the_home_mode() {
        let agent = AgentSpec {
            pack_key: "hermes".into(),
            instance: "hermes".into(),
            home_mode: HomeMode::Isolated,
            persistence: crate::domain::builder::Persistence::Persistent,
            home_paths: vec![".hermes".into()],
            port: None,
        };
        let pack = pack_of(InstallKind::Script, Some("https://example.test/i.sh"));
        let directories = agent_directories(&agent, &pack).expect("directories");
        assert_eq!(
            directories,
            vec![
                "/home/agents/hermes".to_string(),
                "/home/agents/hermes/.hermes".to_string()
            ]
        );
    }

    #[test]
    fn a_node_budget_subtracts_what_installs_reserved() {
        const GIB: i64 = 1024 * 1024 * 1024;
        let profile = HardwareProfile {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            node_id: Uuid::nil(),
            detected_at: chrono::Utc::now(),
            hostname: None,
            gpus: vec![],
            total_vram_bytes: 0,
            unified_memory_bytes: None,
            unified_memory_source: None,
            cuda_version: None,
            driver_version: None,
            detection_note: None,
            cpu_model: None,
            cpu_cores: Some(4),
            cpu_threads: Some(8),
            system_ram_bytes: Some(8 * GIB),
        };
        let budget = budget_from(&profile, (4 * GIB, 0, 1), 2);
        assert_eq!(budget.ram_total_bytes, 8 * GIB);
        assert_eq!(budget.ram_free_bytes, 4 * GIB);
        assert_eq!(budget.cpu_cores, 4);
        assert_eq!(budget.installs, 1);
        assert_eq!(budget.sandboxes, 2);

        // A 6 GiB sandbox no longer fits, and the refusal names the numbers.
        let why = budget.can_host(6 * GIB, 1.0).expect_err("refused");
        assert!(why.message().contains("4.0 GiB"));
        assert!(why.message().contains("6.0 GiB"));
    }

    #[test]
    fn a_normalised_limit_reads_back_as_bytes() {
        assert_eq!(bytes_of("2147483648b"), Some(2_147_483_648));
        assert_eq!(bytes_of("1g"), None);
    }
}
