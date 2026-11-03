//! Agents inside a sandbox: one pack instance each, with its own or a shared home.
//!
//! A sandbox started with a single pack is a container; a sandbox with agents is
//! a small fleet inside one container. The rules that make that work:
//!
//! * **The home is the unit of persistence.** A shared home is one directory for
//!   every agent (`/home/nestbox`), an isolated home is one per instance
//!   (`/home/agents/<instance>`). A persistent home is a root on the node, so it
//!   outlives the container; an ephemeral one dies with it.
//! * **One supervisor, not N containers.** The entrypoint generator renders one
//!   script that starts every pack's command in its own home and ends the
//!   sandbox when the first agent stops: a sandbox with a dead agent is not a
//!   healthy sandbox, and reporting it as one would hide the failure.
//! * **A recipe constrains what may join.** When a sandbox was instantiated from
//!   a recipe, only packs the recipe's baseline supports are accepted, so a
//!   sandbox never gains an agent the image cannot run.
//! * **Removing an agent does not silently destroy data.** `keep_persistent`
//!   decides, and it defaults to keeping.

use std::collections::BTreeMap;

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::builder::{entrypoint_command, AgentLaunch, HomeMode, Persistence};
use crate::domain::pack::AgentPack;
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::TenantScope;
use crate::store::postgres::agent_packs as pack_store;
use crate::store::postgres::audit::{self as audit_store, NewAuditEvent};
use crate::store::postgres::persistent_roots as store;
use crate::store::postgres::sandboxes as sandbox_store;

/// One agent as the API returns it.
///
/// The pack key is kept beside the pack name so a client can link to the pack
/// even after the pack is renamed; the name is what an operator recognises.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AgentView {
    pub pack_key: String,
    pub pack_name: String,
    pub instance: String,
    pub home_mode: String,
    /// Where inside the container this agent's home is.
    pub home_path: String,
    pub persistence: String,
    /// The root that holds the home, when it is persistent.
    pub root_id: Option<Uuid>,
    pub port: Option<i32>,
    pub status: String,
}

/// Everything a sandbox runs, and how it is started.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct SandboxAgents {
    pub sandbox_id: Uuid,
    /// Ordered by instance, which is also the supervisor's start order.
    pub agents: Vec<AgentView>,
    /// The container command: `/bin/bash -c <supervisor script>`.
    pub entrypoint: Vec<String>,
}

/// What a request to add an agent carries.
pub struct AddAgentRequest {
    pub pack_key: String,
    pub instance: String,
    pub home_mode: HomeMode,
    pub persistence: Persistence,
    pub port: Option<u16>,
}

/// List the agents of a sandbox together with the command that starts them.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<SandboxAgents, ApiError> {
    require(principal, Permission::SandboxRead)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(sandbox_not_found)?;

    let rows = store::agents_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let mut packs: BTreeMap<String, AgentPack> = pack_store::list_packs(&mut scope)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .map(|pack| (pack.key.clone(), pack))
        .collect();

    let mut views = Vec::with_capacity(rows.len());
    let mut launches = Vec::with_capacity(rows.len());
    for row in &rows {
        let pack = match packs.remove(&row.pack_key) {
            Some(pack) => Some(pack),
            None => match pack_store::get_pack(&mut scope, &row.pack_key).await {
                Ok(pack) => Some(pack),
                Err(StoreError::NotFound) => None,
                Err(error) => return Err(ApiError::from(error)),
            },
        };
        let home_mode = HomeMode::parse(&row.home_mode).unwrap_or(HomeMode::Shared);
        let persistence = Persistence::parse(&row.persistence).unwrap_or(Persistence::Ephemeral);
        let port = row.port.and_then(|value| u16::try_from(value).ok());

        views.push(AgentView {
            pack_key: row.pack_key.clone(),
            pack_name: pack
                .as_ref()
                .map(|pack| pack.name.clone())
                .unwrap_or_else(|| row.pack_key.clone()),
            instance: row.instance.clone(),
            home_mode: home_mode.as_str().to_string(),
            home_path: home_mode.home_path(&row.instance),
            persistence: persistence.as_str().to_string(),
            root_id: row.root_id,
            port: row.port,
            status: row.status.clone(),
        });

        launches.push(AgentLaunch {
            instance: row.instance.clone(),
            home_mode,
            command: pack
                .as_ref()
                .map(|pack| pack.default_command.clone())
                .unwrap_or_default(),
            env_slots: pack
                .as_ref()
                .map(|pack| pack.env_slots.clone())
                .unwrap_or_default(),
            port,
        });
    }
    scope.commit().await.map_err(ApiError::from)?;

    let entrypoint = entrypoint_command(&launches);
    Ok(SandboxAgents {
        sandbox_id,
        agents: views,
        entrypoint,
    })
}

/// Add an agent to a sandbox.
///
/// The row is written first and the persistent home second, so a name collision
/// is refused before a directory is claimed on the node. Everything runs in one
/// transaction: a refused mount leaves no agent behind.
pub async fn add(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    request: AddAgentRequest,
) -> Result<AgentView, ApiError> {
    require(principal, Permission::SandboxWrite)?;
    let instance = validate_instance(&request.instance)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(sandbox_not_found)?;

    let pack = pack_store::get_pack(&mut scope, &request.pack_key)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(
                ErrorCode::PACK_NOT_FOUND,
                format!("No agent pack '{}'.", request.pack_key),
            ),
            other => ApiError::from(other),
        })?;
    if !pack.enabled {
        return Err(ApiError::not_found(
            ErrorCode::PACK_NOT_FOUND,
            format!("The agent pack '{}' is disabled.", pack.key),
        ));
    }

    let baseline = store::recipe_baseline_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    ensure_pack_supports(&pack, baseline.as_deref())?;

    let home_path = request.home_mode.home_path(&instance);
    let inserted = store::insert_agent(
        &mut scope,
        sandbox_id,
        &pack.key,
        &instance,
        request.home_mode.as_str(),
        request.persistence.as_str(),
        None,
        request.port.map(i32::from),
    )
    .await
    .map_err(ApiError::from)?;
    let Some(row) = inserted else {
        return Err(ApiError::conflict(
            ErrorCode::AGENT_EXISTS,
            format!("An agent named '{instance}' already runs in this sandbox."),
        ));
    };

    let root_id = if request.persistence == Persistence::Persistent {
        let root = store::attach_agent_home(
            &mut scope,
            sandbox_id,
            &instance,
            request.home_mode,
            Some(principal.user_id),
        )
        .await
        .map_err(home_error)?;
        Some(root.id)
    } else {
        None
    };

    audit_store::record(
        &mut scope,
        NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "agent.add",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            detail: &serde_json::json!({
                "instance": instance,
                "pack_key": pack.key,
                "persistence": request.persistence.as_str(),
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(AgentView {
        pack_key: pack.key,
        pack_name: pack.name,
        instance,
        home_mode: request.home_mode.as_str().to_string(),
        home_path,
        persistence: request.persistence.as_str().to_string(),
        root_id,
        port: row.port,
        status: row.status,
    })
}

/// Remove an agent from a sandbox.
///
/// `keep_persistent` is the whole decision of this function: `true` leaves the
/// home and its data on the node for a later re-add, `false` deletes the root
/// when no other sandbox and no other agent in this one is using it. The default
/// at the API is `true`, because a destructive default is a mistake waiting to
/// happen.
///
/// The host directory itself is not deleted here. This layer can remove the
/// record that says the directory exists; deleting bytes on a node is the
/// runner's job, and a control plane that writes to arbitrary host paths is the
/// thing ADR 0021 exists to prevent.
pub async fn remove(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    instance: &str,
    keep_persistent: bool,
) -> Result<(), ApiError> {
    require(principal, Permission::SandboxWrite)?;
    if !is_slug(instance) {
        return Err(agent_not_found(instance));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(sandbox_not_found)?;

    let row = store::remove_agent(&mut scope, sandbox_id, instance)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => agent_not_found(instance),
            other => ApiError::from(other),
        })?;

    if let Some(root_id) = row.root_id {
        if work_still_attached(&mut scope, root_id, sandbox_id).await? {
            return Err(ApiError::conflict(
                ErrorCode::ROOT_IN_USE,
                "This agent's home is mounted in another sandbox; detach it there first.",
            ));
        }
        if !keep_persistent
            && shared_with_another_agent(&mut scope, root_id, sandbox_id, instance).await?
        {
            return Err(ApiError::conflict(
                ErrorCode::ROOT_IN_USE,
                "Another agent in this sandbox still lives in this home; it cannot be deleted yet.",
            ));
        }
        // The mount belonged to this agent, so it goes with it in both
        // directions. Keeping the root keeps the data, not the binding.
        match store::dismount(&mut scope, root_id, sandbox_id).await {
            Ok(()) | Err(StoreError::NotFound) => {}
            Err(error) => return Err(ApiError::from(error)),
        }
        if !keep_persistent {
            store::remove_root(&mut scope, root_id, false)
                .await
                .map_err(ApiError::from)?;
        }
    }

    audit_store::record(
        &mut scope,
        NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "agent.remove",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            detail: &serde_json::json!({
                "instance": instance,
                "keep_persistent": keep_persistent,
                "root_id": row.root_id,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// The instance name: a slug, so it can name a directory.
///
/// Deliberately the same rule as the container's home path and the column's
/// `CHECK`: one validation, three users. An invalid name cannot reach a path.
fn validate_instance(value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if is_slug(trimmed) {
        Ok(trimmed.to_string())
    } else {
        Err(ApiError::validation(
            "An agent instance is a slug: lowercase letters, digits, and hyphens, 1 to 31 characters.",
        ))
    }
}

/// Whether a value is a valid agent instance.
///
/// One to 31 characters, lowercase alphanumerics and hyphens, starting with an
/// alphanumeric. Kept as a function because `validate_instance` produces the
/// message and the removal path only needs the answer.
fn is_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 31 {
        return false;
    }
    bytes.first().is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// Refuse a pack the sandbox's recipe baseline cannot run.
///
/// A sandbox without a recipe has no baseline to violate: it is its own image,
/// chosen when it was created. With a recipe, the image was built for one
/// baseline and adding a pack that needs another one fails at run time rather
/// than here, which is why it is refused with `pack.incompatible`.
fn ensure_pack_supports(pack: &AgentPack, baseline: Option<&str>) -> Result<(), ApiError> {
    match baseline {
        Some(baseline) if !pack.supports(baseline) => Err(ApiError::conflict(
            ErrorCode::PACK_INCOMPATIBLE,
            format!(
                "The pack '{}' does not support the recipe's baseline '{baseline}'.",
                pack.key
            ),
        )),
        _ => Ok(()),
    }
}

/// The error for a sandbox that is not in this environment.
fn sandbox_not_found(error: StoreError) -> ApiError {
    match error {
        StoreError::NotFound => ApiError::not_found(
            ErrorCode::SANDBOX_NOT_FOUND,
            "No such sandbox in this environment.",
        ),
        other => ApiError::from(other),
    }
}

/// The error for an agent instance that is not configured.
fn agent_not_found(instance: &str) -> ApiError {
    ApiError::not_found(
        ErrorCode::AGENT_NOT_FOUND,
        format!("No agent '{instance}' in this sandbox."),
    )
}

/// Map a failure to create an agent's home to the published codes.
///
/// The partial unique index on `mounts` is the conflict an agent home can hit:
/// the same root already mounted read-write in another sandbox. Anything else,
/// including the root's own uniqueness, is a real error and keeps its code.
fn home_error(error: StoreError) -> ApiError {
    match error {
        StoreError::UniqueViolation(detail) if detail.contains("mounts_one_rw_per_root") => {
            ApiError::conflict(
                ErrorCode::MOUNT_CONFLICT,
                "This agent's home is already mounted read-write in another sandbox.",
            )
            .with_internal(detail)
        }
        other => ApiError::from(other),
    }
}

/// Whether another agent in this sandbox still writes to the same home.
async fn shared_with_another_agent(
    scope: &mut TenantScope<'_>,
    root_id: Uuid,
    sandbox_id: Uuid,
    instance: &str,
) -> Result<bool, ApiError> {
    store::root_used_by_other_agent(scope, root_id, sandbox_id, instance)
        .await
        .map_err(ApiError::from)
}

/// Whether a root is still mounted in a sandbox other than this one.
async fn work_still_attached(
    scope: &mut TenantScope<'_>,
    root_id: Uuid,
    sandbox_id: Uuid,
) -> Result<bool, ApiError> {
    store::root_mounted_elsewhere(scope, root_id, sandbox_id)
        .await
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::builder::AgentSpec;

    #[test]
    fn an_instance_is_a_slug() {
        assert!(validate_instance("hermes").is_ok());
        assert!(validate_instance("claude-code-2").is_ok());
        assert!(validate_instance(" Hermes ").is_err());
        assert!(validate_instance("her mes").is_err());
        assert!(validate_instance("-hermes").is_err());
        assert!(validate_instance("hermes/../x").is_err());
        assert!(validate_instance("").is_err());
        assert!(validate_instance(&"a".repeat(32)).is_err());
        assert!(is_slug(&"a".repeat(31)));
    }

    #[test]
    fn a_pack_must_match_the_recipes_baseline() {
        let pack = AgentPack {
            key: "hermes".into(),
            name: "Hermes".into(),
            vendor: None,
            description: None,
            base_os: vec!["ubuntu-24.04".into(), "debian-12".into()],
            install_kind: crate::domain::pack::InstallKind::Script,
            install_ref: None,
            home_paths: Vec::new(),
            requires_gpu: false,
            needs_inference: true,
            default_image: "nestbox-hermes:local".into(),
            default_command: vec!["sleep".into(), "infinity".into()],
            ports: Vec::new(),
            env_slots: Vec::new(),
            enabled: true,
        };

        // No recipe, no baseline: anything the operator picks is allowed.
        assert!(ensure_pack_supports(&pack, None).is_ok());
        assert!(ensure_pack_supports(&pack, Some("ubuntu-24.04")).is_ok());
        let refused = ensure_pack_supports(&pack, Some("rocky-9"))
            .err()
            .unwrap_or_else(|| panic!("rocky-9 must be refused"));
        assert_eq!(refused.code, ErrorCode::PACK_INCOMPATIBLE);
        assert_eq!(refused.status, axum::http::StatusCode::CONFLICT);
    }

    // -----------------------------------------------------------------------
    // Database checks
    //
    // These follow the conventions of `tests/database.rs`: they need
    // `TEST_DATABASE_URL` and skip without it. They live here because the
    // behaviour under test — which root survives a removal — is one function in
    // this module, and a copy of it in a second file would be a second place to
    // forget.
    // -----------------------------------------------------------------------

    async fn test_pool() -> Option<sqlx::PgPool> {
        let url = std::env::var("TEST_DATABASE_URL").ok()?;
        let pool = sqlx::PgPool::connect(&url).await.ok()?;
        sqlx::migrate!("./migrations").run(&pool).await.ok()?;
        Some(pool)
    }

    async fn test_principal(pool: &sqlx::PgPool) -> Principal {
        let row: (Uuid, Uuid) = sqlx::query_as(
            "SELECT e.id, u.id
               FROM environments e CROSS JOIN users u
              WHERE e.slug = 'default'
              ORDER BY u.created_at LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .expect("the seed has an environment and an admin");
        Principal {
            user_id: row.1,
            environment_id: row.0,
            role: crate::domain::auth::Role::Admin,
            display_name: "test".into(),
            via: crate::domain::auth::CredentialSource::Token,
        }
    }

    async fn test_context(pool: sqlx::PgPool) -> ServiceContext {
        let runtime: std::sync::Arc<dyn crate::runtime::RuntimeAdapter> =
            std::sync::Arc::new(crate::runtime::fake::FakeRuntime::new());
        let bus: std::sync::Arc<dyn crate::events::bus::EventBus> =
            std::sync::Arc::new(crate::events::bus::LocalBus::new());
        ServiceContext::new(pool, runtime, bus, std::env::temp_dir())
    }

    /// The sandbox the checks below add agents to.
    async fn test_sandbox(pool: &sqlx::PgPool, principal: &Principal) -> Uuid {
        let mut scope = TenantScope::begin(pool, principal.environment_id)
            .await
            .expect("scope");
        let cluster_id = crate::store::postgres::environments::default_cluster(&mut scope)
            .await
            .expect("cluster");
        let node = crate::store::postgres::nodes::local(&mut scope)
            .await
            .expect("node");
        let name = format!("agents-{}", Uuid::new_v4().simple());
        let sandbox = sandbox_store::insert(
            &mut scope,
            sandbox_store::NewSandbox {
                cluster_id,
                gateway_id: None,
                node_id: node.id,
                name: &name,
                pack_key: "ubuntu-dev",
                image: "nestbox-ubuntu-dev:local",
                image_digest: None,
                spec: &serde_json::json!({}),
                parent_id: None,
                owner_user_id: None,
            },
        )
        .await
        .expect("sandbox");
        scope.commit().await.expect("commit");
        sandbox.id
    }

    /// Configure the agents a recipe names, the way instantiate does.
    async fn add_from_spec(
        pool: &sqlx::PgPool,
        principal: &Principal,
        sandbox_id: Uuid,
        specs: &[AgentSpec],
    ) -> usize {
        let mut scope = TenantScope::begin(pool, principal.environment_id)
            .await
            .expect("scope");
        let rows = store::add_agents_from_spec(&mut scope, sandbox_id, specs)
            .await
            .expect("specs");
        scope.commit().await.expect("commit");
        rows.len()
    }

    /// Drop what the check created: the sandbox (its agents and mounts
    /// cascade) and the roots the agent homes live in.
    async fn cleanup(pool: &sqlx::PgPool, sandbox_id: Uuid, roots: &[Uuid]) {
        let _ = sqlx::query("DELETE FROM sandboxes WHERE id = $1")
            .bind(sandbox_id)
            .execute(pool)
            .await;
        for root in roots {
            let _ = sqlx::query("DELETE FROM persistent_roots WHERE id = $1")
                .bind(root)
                .execute(pool)
                .await;
        }
    }

    #[tokio::test]
    async fn removing_an_agent_decides_what_survives() {
        let Some(pool) = test_pool().await else {
            eprintln!("skipping: TEST_DATABASE_URL is not set");
            return;
        };
        let principal = test_principal(&pool).await;
        let ctx = test_context(pool.clone()).await;
        let sandbox_id = test_sandbox(&pool, &principal).await;

        let hermes = add(
            &ctx,
            &principal,
            sandbox_id,
            AddAgentRequest {
                pack_key: "hermes".into(),
                instance: "hermes".into(),
                home_mode: HomeMode::Shared,
                persistence: Persistence::Persistent,
                port: None,
            },
        )
        .await
        .expect("hermes");
        let git = add(
            &ctx,
            &principal,
            sandbox_id,
            AddAgentRequest {
                pack_key: "git".into(),
                instance: "git".into(),
                home_mode: HomeMode::Isolated,
                persistence: Persistence::Persistent,
                port: None,
            },
        )
        .await
        .expect("git");

        let hermes_root = hermes.root_id.expect("hermes has a root");
        let git_root = git.root_id.expect("git has a root");
        assert_ne!(hermes_root, git_root);

        // A second agent with the same instance is refused.
        let duplicate = add(
            &ctx,
            &principal,
            sandbox_id,
            AddAgentRequest {
                pack_key: "git".into(),
                instance: "hermes".into(),
                home_mode: HomeMode::Shared,
                persistence: Persistence::Ephemeral,
                port: None,
            },
        )
        .await
        .err()
        .expect("a duplicate instance is refused");
        assert_eq!(duplicate.code, ErrorCode::AGENT_EXISTS);

        // A shared home is one directory: the second shared agent joins it
        // instead of claiming a second root at the same path.
        let opencode = add(
            &ctx,
            &principal,
            sandbox_id,
            AddAgentRequest {
                pack_key: "opencode".into(),
                instance: "opencode".into(),
                home_mode: HomeMode::Shared,
                persistence: Persistence::Persistent,
                port: None,
            },
        )
        .await
        .expect("opencode");
        assert_eq!(
            opencode.root_id,
            Some(hermes_root),
            "a shared home reuses the root"
        );

        // ... so dropping it with `keep_persistent = false` would take the
        // other agent's data with it, and is refused.
        let shared_in_use = remove(&ctx, &principal, sandbox_id, "opencode", false)
            .await
            .err()
            .expect("a shared home cannot be deleted under another agent");
        assert_eq!(shared_in_use.code, ErrorCode::ROOT_IN_USE);
        remove(&ctx, &principal, sandbox_id, "opencode", true)
            .await
            .expect("opencode goes, the home stays");

        // keep_persistent = false takes that agent's root with it.
        remove(&ctx, &principal, sandbox_id, "hermes", false)
            .await
            .expect("remove hermes");
        let gone: Option<Uuid> = sqlx::query_scalar("SELECT id FROM persistent_roots WHERE id = $1")
            .bind(hermes_root)
            .fetch_optional(&pool)
            .await
            .expect("query");
        assert!(gone.is_none(), "hermes's root must be gone");

        // ... and keep_persistent = true leaves the other one alone.
        remove(&ctx, &principal, sandbox_id, "git", true)
            .await
            .expect("remove git");
        let kept: Option<Uuid> = sqlx::query_scalar("SELECT id FROM persistent_roots WHERE id = $1")
            .bind(git_root)
            .fetch_optional(&pool)
            .await
            .expect("query");
        assert_eq!(kept, Some(git_root), "git's root must survive");

        // The instances are free again: a removed agent can come back with its
        // kept data instead of colliding with its own tombstone.
        let again = add(
            &ctx,
            &principal,
            sandbox_id,
            AddAgentRequest {
                pack_key: "git".into(),
                instance: "git".into(),
                home_mode: HomeMode::Isolated,
                persistence: Persistence::Persistent,
                port: None,
            },
        )
        .await
        .expect("git comes back");
        assert_eq!(again.root_id, Some(git_root), "the kept home is reused");

        let unknown = remove(&ctx, &principal, sandbox_id, "nobody", true)
            .await
            .err()
            .expect("unknown agent");
        assert_eq!(unknown.code, ErrorCode::AGENT_NOT_FOUND);

        let listed = list(&ctx, &principal, sandbox_id).await.expect("list");
        assert_eq!(listed.agents.len(), 1);
        assert_eq!(listed.agents[0].instance, "git");
        assert_eq!(listed.agents[0].home_path, "/home/agents/git");
        assert_eq!(listed.entrypoint[0], "/bin/bash");

        // The recipe path: the same specs twice configure once, so a retried
        // instantiate does not grow a second copy of every agent.
        let specs = vec![AgentSpec {
            pack_key: "hermes".into(),
            instance: "recipe-hermes".into(),
            home_mode: HomeMode::Isolated,
            persistence: Persistence::Persistent,
            home_paths: Vec::new(),
            port: None,
        }];
        assert_eq!(
            add_from_spec(&pool, &principal, sandbox_id, &specs).await,
            2,
            "the recipe agent joins the existing one"
        );
        assert_eq!(
            add_from_spec(&pool, &principal, sandbox_id, &specs).await,
            2,
            "a re-run must not duplicate rows"
        );
        let recipe_root: Option<Uuid> = sqlx::query_scalar(
            "SELECT root_id FROM sandbox_agents WHERE sandbox_id = $1 AND instance = 'recipe-hermes'",
        )
        .bind(sandbox_id)
        .fetch_one(&pool)
        .await
        .expect("query");
        assert!(recipe_root.is_some(), "a persistent spec gets a home");

        cleanup(
            &pool,
            sandbox_id,
            &[git_root, recipe_root.unwrap_or_else(Uuid::nil)],
        )
        .await;
    }
}
