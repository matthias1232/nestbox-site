//! Moving a sandbox, with its persistent data, to another node and back (ADR 0049).
//!
//! The move is a sequence of small, named steps, each written to `sandbox_moves.log`, so the
//! console can show where it is and an operator can see where a failed one stopped:
//!
//! 1. **check** - the target is another node this data center may use, and it answers;
//!    the sandbox's roots belong to this sandbox alone;
//! 2. **image** - the target holds the sandbox's image (pulled from the registry if not);
//! 3. **snapshot** - every root is snapshotted into the data center's restic repository while
//!    the sandbox still runs, so the stop that follows has little left to send;
//! 4. **stop** - the sandbox stops on the source, through the normal path, so the reconcile loop
//!    does not start it again;
//! 5. **delta** - a second snapshot of each root, now at rest;
//! 6. **restore** - the roots are put at the same paths on the target (what was there is set
//!    aside, never deleted);
//! 7. **cutover** - roots and sandbox now belong to the target, the source container goes;
//! 8. **recreate** - the sandbox is created on the target with the same identity, policies and
//!    mounts, and started if it was running;
//! 9. **verify** - the target reports it running.
//!
//! The source's files stay where they are as a cold copy, and the source's container is removed
//! only after the cutover is committed. A failure before the cutover puts the sandbox back on the
//! source; a failure after it takes the row back to the source and creates the container there
//! again, so a move either ends on the target or ends where it started. Because a root is owned by
//! exactly one node row, there is never a moment in which two nodes both claim to run it.
//!
//! Nothing stops until `preflight` has said the move can work: the same checks that start a move
//! answer the console first, as a list of blockers rather than as the first error.

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::builder_service;
use super::sandbox_service::{self, LifecycleAction};
use super::{require, secret_service, ServiceContext};
use crate::config::Config;
use crate::domain::auth::{Permission, Principal};
use crate::domain::node::{Node, NodeBudget, NodeStatus};
use crate::domain::sandbox::{Sandbox, SandboxStatus};
use crate::domain::telemetry::LiveEvent;
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::runtime::router::RuntimeRouter;
use crate::store::TenantScope;
use crate::store::postgres::{model_installs, models, nodes as node_store, sandboxes as sandbox_store};

/// The image prefix of a sandbox the builder made. A recipe image's entrypoint is generated at
/// build time, so its container is created from the recipe, not by a pack's recreate.
const RECIPE_IMAGE_PREFIX: &str = "nestbox-recipe/";

/// What a move carries over besides the folders themselves, in the words the console shows.
const CARRIES: [&str; 8] = [
    "persistent folders",
    "agent logins (.agent-state)",
    "egress mode and policies",
    "agent sign-in modes",
    "resource limits, reservation, CPU weight, GPUs",
    "SSH keys and port forwards",
    "MCP attachments",
    "terminal tab names",
];

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from(crate::store::map_sqlx_error(error))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MoveView {
    pub id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub sandbox_name: String,
    pub from_node: String,
    pub to_node: String,
    /// `running`, `done` or `failed`.
    pub state: String,
    pub step: String,
    pub log: serde_json::Value,
    pub bytes_moved: i64,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// The restic repository of a data center, as the nodes reach it over the overlay.
pub fn repository_url(config: &Config, environment_id: Uuid) -> String {
    format!("rest:http://{}:8500/{}", config.overlay.hub_ip, environment_id)
}

/// A root can only move with a sandbox that is its sole user.
pub fn movable(sharing: &[(Uuid, i64)]) -> Result<(), String> {
    for (root, others) in sharing {
        if *others > 0 {
            return Err(format!(
                "The folder {root} is also mounted in another sandbox. Move them together or unmount it first."
            ));
        }
    }
    Ok(())
}

/// One reason a move cannot work, in a stable code and a sentence.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Blocker {
    /// `same_node`, `target_unknown`, `target_offline`, `shared_root`, `wrong_state`,
    /// `gpu_missing`, `source_unknown`, `recover_source_online` or `no_snapshot`.
    pub code: String,
    /// What a person reads.
    pub message: String,
}

impl Blocker {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into() }
    }

    /// The same blocker as the error a start returns.
    ///
    /// `begin` and `preflight` collect the same blockers, so the answer the console gets before
    /// anything stops and the refusal a start returns cannot drift apart.
    fn as_error(&self) -> ApiError {
        match self.code.as_str() {
            "same_node" | "shared_root" | "recipe_not_movable" => ApiError::validation(self.message.clone()),
            "target_unknown" | "source_unknown" => {
                ApiError::not_found(ErrorCode::NODE_NOT_FOUND, self.message.clone())
            }
            "target_offline" => {
                ApiError::conflict(ErrorCode::NODE_RUNTIME_UNAVAILABLE, self.message.clone())
            }
            "gpu_missing" => {
                ApiError::unprocessable(ErrorCode::NODE_INSUFFICIENT_RESOURCES, self.message.clone())
            }
            _ => ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, self.message.clone()),
        }
    }
}

/// Which move a log line and a live event belong to.
#[derive(Debug, Clone, Copy)]
struct Trace {
    move_id: Uuid,
    environment_id: Uuid,
    sandbox_id: Uuid,
}

/// Add the keys of `extra` to `into`, when both are objects.
fn merge(into: &mut serde_json::Value, extra: &serde_json::Value) {
    let (serde_json::Value::Object(target), serde_json::Value::Object(source)) = (into, extra) else {
        return;
    };
    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

/// Append one line to a move's log and publish the same step, so the stored record and the
/// console's live view say the same thing.
async fn record(ctx: &ServiceContext, trace: Trace, step: &str, detail: &str, state: &str, extra: serde_json::Value) {
    let mut entry = serde_json::json!([{ "step": step, "at": Utc::now(), "detail": detail }]);
    if let Some(item) = entry.get_mut(0) {
        merge(item, &extra);
    }
    let _ = sqlx::query("UPDATE sandbox_moves SET step = $2, log = log || $3::jsonb WHERE id = $1")
        .bind(trace.move_id)
        .bind(step)
        .bind(entry)
        .execute(&ctx.pool)
        .await;
    let mut announced = serde_json::json!({ "step": step, "state": state, "message": detail });
    merge(&mut announced, &extra);
    ctx.bus.publish(LiveEvent::SandboxControlChanged {
        sandbox_id: Some(trace.sandbox_id),
        environment_id: trace.environment_id,
        what: "move".to_string(),
        detail: announced,
    });
}

/// A step that is running.
async fn note(ctx: &ServiceContext, trace: Trace, step: &str, detail: &str) {
    record(ctx, trace, step, detail, "running", serde_json::Value::Null).await;
}

/// The end of a failed move: the error on the row, and the step in the log.
async fn fail(ctx: &ServiceContext, trace: Trace, step: &str, error: &ApiError, rolled_back: bool) {
    let _ = sqlx::query("UPDATE sandbox_moves SET state = 'failed', error = $2, finished_at = now() WHERE id = $1")
        .bind(trace.move_id)
        .bind(format!("{step}: {}", error.message))
        .execute(&ctx.pool)
        .await;
    let extra = if rolled_back {
        serde_json::json!({ "rolled_back": true })
    } else {
        serde_json::Value::Null
    };
    record(ctx, trace, "failed", &format!("{step}: {}", error.message), "failed", extra).await;
}

/// The password of the data center's repository, made on first use and kept in the vault.
pub(crate) async fn repository_password(ctx: &ServiceContext, principal: &Principal) -> Result<String, ApiError> {
    let name = "restic-repository";
    if let Ok(existing) = secret_service::reveal(ctx, principal.environment_id, name).await {
        return Ok(existing);
    }
    let fresh = crate::security::session::random_token();
    secret_service::put_system(ctx, principal.environment_id, principal.user_id, name, "password", &fresh).await?;
    Ok(fresh)
}

/// Start moving a sandbox. Returns at once; the move runs in the background and is followed
/// through `get` and `list`.
pub async fn start(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    sandbox_id: Uuid,
    target: &str,
) -> Result<MoveView, ApiError> {
    begin(ctx, config, principal, sandbox_id, target, false).await
}

/// Bring a sandbox back on another node after the node it ran on is gone (ADR 0049).
///
/// Only for a source node that is not online: the folders are restored from their newest scheduled
/// snapshot, so the work since that snapshot (at most the snapshot interval) is lost, and the log
/// says which snapshot was used. If the source node is online, use a normal move: it loses nothing.
pub async fn recover(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    sandbox_id: Uuid,
    target: &str,
) -> Result<MoveView, ApiError> {
    begin(ctx, config, principal, sandbox_id, target, true).await
}

/// What a move works with once nothing blocks it.
struct Plan {
    from: Node,
    to: Node,
    roots: Vec<(Uuid, String)>,
    latest: Vec<(String, String)>,
}

/// Everything a move needs, and everything that stands in its way.
struct Survey {
    sandbox: Sandbox,
    /// The target, when a node of that name exists.
    to: Option<Node>,
    /// What the target has free, when it has been measured.
    budget: Option<NodeBudget>,
    /// The pieces of a move that can run; `None` when a blocker makes it pointless.
    plan: Option<Plan>,
    blockers: Vec<Blocker>,
}

/// Gather what a move needs and collect every reason it cannot work.
///
/// `begin` turns the first blocker into its refusal and `preflight` hands all of them to the
/// console, so the two can never disagree about what a move requires. Only a real failure (a
/// database that does not answer) is an error here; everything a person can fix is a blocker.
async fn survey(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    target: &str,
    recovering: bool,
) -> Result<Survey, ApiError> {
    let sandbox = sandbox_store::get(scope, sandbox_id).await.map_err(ApiError::from)?;
    let mut blockers: Vec<Blocker> = Vec::new();

    let to = match node_store::get_by_name(scope, target.trim()).await {
        Ok(node) => Some(node),
        Err(StoreError::NotFound) => {
            blockers.push(Blocker::new("target_unknown", format!("No node named '{}'.", target.trim())));
            None
        }
        Err(other) => return Err(ApiError::from(other)),
    };
    if let Some(node) = &to {
        if node.id == sandbox.node_id {
            blockers.push(Blocker::new("same_node", "The sandbox is already on that node."));
        }
        if node.status != NodeStatus::Online {
            blockers.push(Blocker::new("target_offline", format!("Node '{}' is not online.", node.name)));
        }
        let gpus = gpu_count(&sandbox);
        if gpus > 0 && !node.container_gpu {
            blockers.push(Blocker::new(
                "gpu_missing",
                format!("The sandbox asks for {gpus} GPU(s); node '{}' has no GPU runtime.", node.name),
            ));
        }
    }

    let from = match node_store::get(scope, sandbox.node_id).await {
        Ok(node) => Some(node),
        Err(StoreError::NotFound) => {
            blockers.push(Blocker::new(
                "source_unknown",
                format!("The node the sandbox runs on ({}) no longer exists.", sandbox.node_id),
            ));
            None
        }
        Err(other) => return Err(ApiError::from(other)),
    };

    let (roots, sharing) = move_roots(scope, &sandbox).await?;
    if let Err(message) = movable(&sharing) {
        blockers.push(Blocker::new("shared_root", message));
    }

    let mut latest: Vec<(String, String)> = Vec::new();
    if recovering {
        if let Some(node) = &from {
            if node.status == NodeStatus::Online {
                blockers.push(Blocker::new(
                    "recover_source_online",
                    format!("Node '{}' is online. Move the sandbox instead: a move loses nothing, a recovery loses the work since the last snapshot.", node.name),
                ));
            }
        }
        latest = newest_snapshots(scope, &roots, &mut blockers).await?;
    } else if ![SandboxStatus::Running, SandboxStatus::Stopped].contains(&sandbox.status) {
        blockers.push(Blocker::new(
            "wrong_state",
            "Only a running or stopped sandbox can be moved.",
        ));
    }

    let budget = match &to {
        Some(node) => node_budget(scope, node.id).await?,
        None => None,
    };
    let plan = match (from, to.clone()) {
        (Some(from), Some(to)) if blockers.is_empty() => Some(Plan { from, to, roots, latest }),
        _ => None,
    };
    Ok(Survey { sandbox, to, budget, plan, blockers })
}

/// The folders a move carries: their ids and host paths, and how many other sandboxes mount each.
///
/// The environment filter is explicit even though mounts are row-level secured: "shared" has to
/// mean shared with *this* data center's sandboxes, never with a stranger's.
async fn move_roots(
    scope: &mut TenantScope<'_>,
    sandbox: &Sandbox,
) -> Result<(Vec<(Uuid, String)>, Vec<(Uuid, i64)>), ApiError> {
    let roots: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT r.id, r.host_path FROM persistent_roots r JOIN mounts m ON m.root_id = r.id
          WHERE m.sandbox_id = $1 AND m.status = 'mounted' AND r.archived_at IS NULL
            AND r.environment_id = $2
          ORDER BY r.host_path",
    )
    .bind(sandbox.id)
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    let mut sharing = Vec::new();
    for (root, _) in &roots {
        let others: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM mounts m JOIN persistent_roots r ON r.id = m.root_id
              WHERE m.root_id = $1 AND m.sandbox_id <> $2 AND m.status = 'mounted'
                AND r.environment_id = $3",
        )
        .bind(root)
        .bind(sandbox.id)
        .bind(scope.environment_id())
        .fetch_one(scope.connection())
        .await
        .map_err(db)?;
        sharing.push((*root, others));
    }
    Ok((roots, sharing))
}

/// The newest snapshot of each folder, which is all a recovery has to restore from.
///
/// The snapshot table is not row-level secured (the scheduler writes it for every data center), so
/// this read filters by environment itself. A folder without a snapshot is a blocker, not a silent
/// hole in the restore.
async fn newest_snapshots(
    scope: &mut TenantScope<'_>,
    roots: &[(Uuid, String)],
    blockers: &mut Vec<Blocker>,
) -> Result<Vec<(String, String)>, ApiError> {
    let mut latest: Vec<(String, String)> = Vec::new();
    for (root, path) in roots {
        let newest: Option<String> = sqlx::query_scalar(
            "SELECT snapshot FROM root_snapshots
              WHERE root_id = $1 AND environment_id = $2
              ORDER BY taken_at DESC LIMIT 1",
        )
        .bind(root)
        .bind(scope.environment_id())
        .fetch_optional(scope.connection())
        .await
        .map_err(db)?;
        let Some(snapshot) = newest else {
            blockers.push(Blocker::new(
                "no_snapshot",
                format!("The folder {path} was never snapshotted, so there is nothing to recover from."),
            ));
            continue;
        };
        latest.push((path.clone(), snapshot));
    }
    Ok(latest)
}

/// How many GPUs the sandbox asked for; the spec is where every placement decision reads it.
fn gpu_count(sandbox: &Sandbox) -> i64 {
    sandbox.spec.get("gpu_count").and_then(|value| value.as_i64()).unwrap_or(0)
}

/// What a node has free, when it has been measured. `None` until the probe has run, because a
/// number the probe never saw would be invented.
async fn node_budget(scope: &mut TenantScope<'_>, node_id: Uuid) -> Result<Option<NodeBudget>, ApiError> {
    let Some(profile) = models::latest_hardware(scope, node_id).await.map_err(ApiError::from)? else {
        return Ok(None);
    };
    let reserved = model_installs::reserved_on(scope, node_id).await.map_err(ApiError::from)?;
    let counts = node_store::sandbox_counts(scope).await.map_err(ApiError::from)?;
    let sandboxes = counts.get(&node_id).copied().unwrap_or(0) as i32;
    Ok(Some(builder_service::budget_from(&profile, reserved, sandboxes)))
}

async fn begin(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    sandbox_id: Uuid,
    target: &str,
    recovering: bool,
) -> Result<MoveView, ApiError> {
    require(principal, Permission::SandboxWrite)?;
    let Some(router) = ctx.router.clone() else {
        return Err(ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, "This installation has no node dispatcher."));
    };
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let survey = survey(&mut scope, sandbox_id, target, recovering).await?;
    if let Some(blocker) = survey.blockers.first() {
        return Err(blocker.as_error());
    }
    let Some(plan) = survey.plan else {
        return Err(ApiError::internal("A move without a plan."));
    };
    let sandbox = survey.sandbox;
    let from_id = plan.from.id;
    let to_id = plan.to.id;
    let from_name = plan.from.name.clone();
    let to_name = plan.to.name.clone();
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO sandbox_moves (environment_id, sandbox_id, sandbox_name, from_node_id, to_node_id, from_node_name, to_node_name, started_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(sandbox.id)
    .bind(&sandbox.name)
    .bind(from_id)
    .bind(to_id)
    .bind(&from_name)
    .bind(&to_name)
    .bind(principal.user_id)
    .fetch_one(scope.connection())
    .await
    .map_err(|e| match crate::store::map_sqlx_error(e) {
        crate::error::StoreError::UniqueViolation(_) => ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, "This sandbox is already being moved."),
        other => ApiError::from(other),
    })?;
    scope.commit().await.map_err(ApiError::from)?;

    let job = Job {
        environment_id: principal.environment_id,
        id,
        sandbox_id: sandbox.id,
        name: sandbox.name.clone(),
        image: sandbox.image.clone(),
        was_running: recovering || sandbox.status == SandboxStatus::Running,
        recover: recovering,
        latest: plan.latest,
        from: from_id,
        to: to_id,
        roots: plan.roots,
        repo: repository_url(config, principal.environment_id),
    };
    let background = ctx.clone();
    let acting = principal.clone();
    tokio::spawn(async move {
        run(&background, &router, &acting, job).await;
    });
    get(ctx, principal, id).await
}

/// Everything the background task needs, decided before it starts.
struct Job {
    id: Uuid,
    environment_id: Uuid,
    sandbox_id: Uuid,
    name: String,
    image: String,
    was_running: bool,
    /// Restoring from the newest scheduled snapshots because the source is gone.
    recover: bool,
    latest: Vec<(String, String)>,
    from: Uuid,
    to: Uuid,
    roots: Vec<(Uuid, String)>,
    repo: String,
}

/// Run the steps in order; any error stops the move, records where, and undoes what can be undone.
async fn run(ctx: &ServiceContext, router: &RuntimeRouter, principal: &Principal, job: Job) {
    let trace = Trace { move_id: job.id, environment_id: job.environment_id, sandbox_id: job.sandbox_id };
    let source = router.for_node(Some(job.from));
    let target = router.for_node(Some(job.to));
    let mut cut_over = false;
    let mut stopped = false;
    let outcome: Result<(), (String, ApiError)> = async {
        let step = |name: &str| name.to_string();
        let password = repository_password(ctx, principal).await.map_err(|e| (step("check"), e))?;
        note(ctx, trace, "check", &format!("{} folder(s) travel with the sandbox", job.roots.len())).await;

        // The image: the target must hold the same bytes. A local target has no registry to pull
        // from and already has what the appliance built.
        note(ctx, trace, "image", &job.image).await;
        if router.is_remote(job.to) {
            let held = target.images().await.map(|list| list.iter().any(|i| i.image == job.image)).unwrap_or(false);
            if !held {
                target.pull_image(&job.image).await.map_err(|e| (step("image"), ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, format!("The target could not get the image {}: {e}", job.image))))?;
            }
        }
        // Loading a model takes longer than copying the folders, so the warm-up starts now and is
        // not waited for: the target is ready to answer when the sandbox gets there.
        prewarm(ctx, &job);

        let mut bytes = 0i64;
        let mut snapshots = Vec::new();
        let tags = vec![format!("sandbox:{}", job.name), format!("move:{}", job.id)];
        if job.recover {
            for (path, snapshot) in &job.latest {
                note(ctx, trace, "recover", &format!("{path} from snapshot {snapshot}")).await;
                snapshots.push((path.clone(), snapshot.clone()));
            }
        }
        for (_, path) in job.roots.iter().filter(|_| !job.recover) {
            note(ctx, trace, "snapshot", path).await;
            let fact = source.snapshot_backup(&job.repo, &password, path, &tags).await.map_err(|e| (step("snapshot"), ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, e.to_string())))?;
            bytes += fact.bytes_added;
        }

        if job.was_running && !job.recover {
            note(ctx, trace, "stop", "stopping on the source").await;
            sandbox_service::act(ctx, principal, job.sandbox_id, LifecycleAction::Stop).await.map_err(|e| (step("stop"), e))?;
            stopped = true;
        }
        for (_, path) in job.roots.iter().filter(|_| !job.recover) {
            note(ctx, trace, "delta", path).await;
            let fact = source.snapshot_backup(&job.repo, &password, path, &tags).await.map_err(|e| (step("delta"), ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, e.to_string())))?;
            bytes += fact.bytes_added;
            snapshots.push((path.clone(), fact.snapshot));
        }
        let _ = sqlx::query("UPDATE sandbox_moves SET bytes_moved = $2, snapshots = $3 WHERE id = $1")
            .bind(job.id)
            .bind(bytes)
            .bind(serde_json::json!(snapshots))
            .execute(&ctx.pool)
            .await;

        for (path, snapshot) in &snapshots {
            note(ctx, trace, "restore", path).await;
            target.snapshot_restore(&job.repo, &password, snapshot, path, path).await.map_err(|e| (step("restore"), ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, e.to_string())))?;
        }

        note(ctx, trace, "cutover", "the folders and the sandbox now belong to the target").await;
        let roots: Vec<Uuid> = job.roots.iter().map(|root| root.0).collect();
        place_rows(ctx, principal, job.sandbox_id, &roots, job.to).await.map_err(|e| (step("cutover"), e))?;
        // The source's container goes only now: while the row still names the source, there is a
        // sandbox to go back to. Its files stay where they are as a cold copy either way.
        let _ = source.remove(&job.name).await;
        router.place(&job.name, Some(job.to));
        cut_over = true;

        note(ctx, trace, "recreate", "creating the sandbox on the target").await;
        recreate(ctx, principal, job.sandbox_id).await.map_err(|e| (step("recreate"), e))?;
        carry(ctx, principal, &job).await;
        if !job.was_running {
            let _ = sandbox_service::act(ctx, principal, job.sandbox_id, LifecycleAction::Stop).await;
        }

        if job.was_running {
            note(ctx, trace, "verify", "waiting for the target to report it running").await;
            let mut running = false;
            for _ in 0..30 {
                if let Ok(facts) = target.inspect(&job.name).await {
                    if facts.status == "running" {
                        running = true;
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            if !running {
                return Err((step("verify"), ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, "The target did not report the sandbox running within a minute.")));
            }
        }
        Ok(())
    }
    .await;

    match outcome {
        Ok(()) => {
            let _ = sqlx::query("UPDATE sandbox_moves SET state = 'done', step = 'done', finished_at = now() WHERE id = $1")
                .bind(job.id)
                .execute(&ctx.pool)
                .await;
            record(ctx, trace, "done", "the source keeps its files as a cold copy", "done", serde_json::Value::Null).await;
        }
        Err((step, error)) => {
            tracing::warn!(move_id = %job.id, step = %step, code = error.code.0, "a sandbox move failed");
            let rolled_back = if cut_over {
                // The row already names the target, so the way back is the other way round: the row
                // goes back to the source and the container is created there again.
                return_to_source(ctx, router, principal, &job, trace).await
            } else {
                // Before the cutover the source is still the sandbox's home: start it again there.
                if stopped {
                    let _ = sandbox_service::act(ctx, principal, job.sandbox_id, LifecycleAction::Start).await;
                    note(ctx, trace, "rollback", "started again on the source").await;
                }
                false
            };
            fail(ctx, trace, &step, &error, rolled_back).await;
        }
    }
}

/// Start warming the sandbox's models on the target, without waiting.
///
/// A model that is not loaded yet answers slowly, and the sandbox is on its way; the warm-up runs
/// beside the snapshot and a failure is a warning, not a failed move.
fn prewarm(ctx: &ServiceContext, job: &Job) {
    let background = ctx.clone();
    let environment_id = job.environment_id;
    let sandbox_id = job.sandbox_id;
    let node_id = job.to;
    tokio::spawn(async move {
        if let Err(error) = crate::services::model_readiness_service::prewarm_on_node(&background, environment_id, sandbox_id, node_id).await {
            tracing::warn!(sandbox_id = %sandbox_id, code = error.code.0, "the sandbox's models could not be warmed on the target before the move");
        }
    });
}

/// Say that this sandbox's row and its roots belong to `node`.
///
/// The cutover and the rollback are the same two statements with the two nodes swapped; keeping
/// them in one place is what makes "back on the source" the exact reverse of "now on the target".
async fn place_rows(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    roots: &[Uuid],
    node: Uuid,
) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    sqlx::query("UPDATE persistent_roots SET node_id = $2, updated_at = now() WHERE id = ANY($1)")
        .bind(roots)
        .bind(node)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    sqlx::query("UPDATE sandboxes SET node_id = $2, updated_at = now() WHERE id = $1")
        .bind(sandbox_id)
        .bind(node)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)
}

/// The sandbox as stored now, for the helpers that need more than its id.
async fn sandbox_row(ctx: &ServiceContext, principal: &Principal, sandbox_id: Uuid) -> Result<Sandbox, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(sandbox)
}

/// Create the sandbox's container again on the node its row names.
///
/// A recipe sandbox's entrypoint is generated at build time and `sandbox_service::recreate` rightly
/// refuses it, so `builder_service::recreate_in_place` builds the same container from the recipe.
/// The image tag decides, because that is the one thing both paths agree on.
async fn recreate(ctx: &ServiceContext, principal: &Principal, sandbox_id: Uuid) -> Result<(), ApiError> {
    let sandbox = sandbox_row(ctx, principal, sandbox_id).await?;
    if sandbox.image.starts_with(RECIPE_IMAGE_PREFIX) {
        builder_service::recreate_in_place(ctx, principal, sandbox_id).await?;
    } else {
        sandbox_service::recreate(ctx, principal, sandbox_id).await?;
    }
    Ok(())
}

/// What the sandbox needs on the target besides its folders: the key and port forwards it is
/// reachable with, and the agent files, the MCP attachments among them.
///
/// Best effort: a container that runs but cannot be reached is a move worth finishing, and the log
/// says what did not arrive.
async fn carry(ctx: &ServiceContext, principal: &Principal, job: &Job) {
    match sandbox_row(ctx, principal, job.sandbox_id).await {
        Ok(sandbox) => crate::services::access_service::push_forwards_logged(ctx, principal, &sandbox).await,
        Err(error) => tracing::warn!(sandbox_id = %job.sandbox_id, code = error.code.0, "the port forwards could not be pushed after the move"),
    }
    if let Err(error) = sandbox_service::write_agent_files(ctx, job.environment_id, job.sandbox_id).await {
        tracing::warn!(sandbox_id = %job.sandbox_id, code = error.code.0, "the agent files could not be written on the target");
    }
}

/// Put the sandbox back on the source node after the cutover was committed and the target failed.
///
/// The roots on the source were never deleted, so its data is the state of the last delta snapshot.
/// The row names the source before the container is created, so the same recreate path builds it
/// there. Returns whether the sandbox is back on the source; `false` means only the row could not
/// be moved, and the caller must not claim a rollback.
async fn return_to_source(
    ctx: &ServiceContext,
    router: &RuntimeRouter,
    principal: &Principal,
    job: &Job,
    trace: Trace,
) -> bool {
    let roots: Vec<Uuid> = job.roots.iter().map(|root| root.0).collect();
    if let Err(error) = place_rows(ctx, principal, job.sandbox_id, &roots, job.from).await {
        tracing::error!(move_id = %job.id, code = error.code.0, "the folders could not be given back to the source node");
        note(ctx, trace, "rollback", "the folders could not be moved back to the source").await;
        return false;
    }
    router.place(&job.name, Some(job.from));
    match recreate(ctx, principal, job.sandbox_id).await {
        Ok(()) => note(ctx, trace, "rollback", "back on the source").await,
        Err(error) => {
            tracing::warn!(move_id = %job.id, code = error.code.0, "the sandbox is back on the source but its container did not start");
            note(ctx, trace, "rollback", "back on the source; its container did not start").await;
        }
    }
    true
}

/// One move, with node names.

pub async fn get(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<MoveView, ApiError> {
    require(principal, Permission::SandboxRead)?;
    list_where(ctx, principal, Some(id)).await?.into_iter().next().ok_or_else(|| ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "move"))
}

/// The recent moves of this data center that the caller may see.
pub async fn list(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<MoveView>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    list_where(ctx, principal, None).await
}

async fn list_where(ctx: &ServiceContext, principal: &Principal, only: Option<Uuid>) -> Result<Vec<MoveView>, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    // A member sees the moves of sandboxes they can see: the join to `sandboxes` runs under their
    // row-level security. A move whose sandbox is gone is for administrators.
    let rows = sqlx::query_as::<_, (Uuid, Option<Uuid>, String, String, String, String, String, serde_json::Value, i64, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)>(
        "SELECT m.id, m.sandbox_id, m.sandbox_name, coalesce(f.name, m.from_node_name, '?'), coalesce(t.name, m.to_node_name, '?'), m.state, m.step, m.log, m.bytes_moved, m.error, m.started_at, m.finished_at
           FROM sandbox_moves m
           LEFT JOIN nodes f ON f.id = m.from_node_id LEFT JOIN nodes t ON t.id = m.to_node_id
          WHERE m.environment_id = $1 AND ($2::uuid IS NULL OR m.id = $2)
            AND ($3 OR EXISTS (SELECT 1 FROM sandboxes s WHERE s.id = m.sandbox_id))
          ORDER BY m.started_at DESC LIMIT 100",
    )
    .bind(scope.environment_id())
    .bind(only)
    .bind(principal.role == crate::domain::auth::Role::Admin)
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|(id, sandbox_id, sandbox_name, from_node, to_node, state, step, log, bytes_moved, error, started_at, finished_at)| MoveView {
            id, sandbox_id, sandbox_name, from_node, to_node, state, step, log, bytes_moved, error, started_at, finished_at,
        })
        .collect())
}

/// A move that was running when the process died will never finish: say so. The sandbox is where
/// the last completed step left it, which the log shows.
pub async fn settle_interrupted(ctx: &ServiceContext) -> Result<u64, ApiError> {
    let done = sqlx::query(
        "UPDATE sandbox_moves SET state = 'failed', finished_at = now(),
                error = 'The control plane restarted while this move ran. Check where the sandbox is and move it again.'
          WHERE state = 'running'",
    )
    .execute(&ctx.pool)
    .await
    .map_err(db)?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_shared_with_another_sandbox_does_not_move() {
        assert!(movable(&[(Uuid::nil(), 0)]).is_ok());
        assert!(movable(&[(Uuid::nil(), 0), (Uuid::nil(), 2)]).is_err());
        assert!(movable(&[]).is_ok());
    }

    #[test]
    fn the_repository_of_a_data_center_is_its_own_path_on_the_hub() {
        let mut config = crate::config::Config {
            database_url: String::new(),
            listen_addr: String::new(),
            secret_dir: Default::default(),
            session_cookie_secure: false,
            session_idle_seconds: 1,
            session_max_seconds: 1,
            allowed_origins: Vec::new(),
            runner_url: String::new(),
            runtime_network: String::new(),
            bootstrap_admin: false,
            body_limit_bytes: 1,
            rate_limit_per_minute: 1,
            terminal_idle_seconds: 1,
            deploy_mode: crate::config::DeployMode::SelfHosted,
            public_url: String::new(),
            oidc: Vec::new(),
            overlay: crate::config::OverlayConfig {
                dir: Default::default(),
                endpoint: String::new(),
                hub_ip: "10.77.0.1".into(),
                registry: String::new(),
                control_url: String::new(),
            },
        };
        config.overlay.hub_ip = "10.77.0.1".into();
        let env = Uuid::nil();
        assert_eq!(repository_url(&config, env), format!("rest:http://10.77.0.1:8500/{env}"));
    }
}

/// Whether a move to `target` can work, said before anything stops.
///
/// The answer is the same survey that starts a move: every reason it cannot work as a blocker (not
/// only the first), the warnings a person should read before agreeing, and what the move carries
/// over. The target's numbers come from its last hardware measurement, and stay `null` when it has
/// never been measured: an invented figure would make the answer look more certain than it is.
pub async fn preflight(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    target: &str,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let survey = survey(&mut scope, sandbox_id, target, false).await?;
    scope.commit().await.map_err(ApiError::from)?;

    let mut warnings: Vec<&str> = Vec::new();
    if survey.sandbox.status == SandboxStatus::Running {
        warnings.push("Running agent sessions will be stopped; they continue with `--continue` after the move.");
    }
    warnings.push("Packages installed outside persistent folders are lost.");

    let target_view = match &survey.to {
        Some(node) => serde_json::json!({
            "name": node.name,
            "online": node.status == NodeStatus::Online,
            "container_gpu": node.container_gpu,
            "gpu_count": survey.budget.as_ref().map(|budget| budget.gpu_count),
            "free_memory_bytes": survey.budget.as_ref().map(|budget| budget.ram_free_bytes),
        }),
        None => serde_json::json!({
            "name": target.trim(),
            "online": false,
            "container_gpu": false,
            "gpu_count": serde_json::Value::Null,
            "free_memory_bytes": serde_json::Value::Null,
        }),
    };
    Ok(serde_json::json!({
        "ok": survey.blockers.is_empty(),
        "blockers": survey.blockers,
        "warnings": warnings,
        "carries": CARRIES,
        "target": target_view,
    }))
}

#[cfg(test)]
mod preflight_tests {
    use super::*;

    /// A blocker code becomes the same error a start would have returned.
    ///
    /// The preflight exists so nothing stops before the answer is known; that only holds while the
    /// blocker and the refusal are the same thing said twice.
    #[test]
    fn a_blocker_becomes_the_error_a_start_would_return() {
        assert_eq!(
            Blocker::new("same_node", "The sandbox is already on that node.").as_error().code,
            ErrorCode::VALIDATION_FAILED
        );
        assert_eq!(
            Blocker::new("target_unknown", "No node named 'x'.").as_error().code,
            ErrorCode::NODE_NOT_FOUND
        );
        assert_eq!(
            Blocker::new("target_offline", "Node 'x' is not online.").as_error().code,
            ErrorCode::NODE_RUNTIME_UNAVAILABLE
        );
        assert_eq!(
            Blocker::new("wrong_state", "Only a running or stopped sandbox can be moved.").as_error().code,
            ErrorCode::SANDBOX_INVALID_STATE
        );
        // A node without a GPU runtime is an unprocessable request, not a state conflict: the
        // console tells the operator to choose another node, not to retry.
        let gpus = Blocker::new("gpu_missing", "The sandbox asks for 1 GPU(s); node 'x' has no GPU runtime.").as_error();
        assert_eq!(gpus.code, ErrorCode::NODE_INSUFFICIENT_RESOURCES);
        assert_eq!(gpus.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// A sandbox that asked for GPUs cannot be moved to a node without a GPU runtime.
    #[test]
    fn gpus_are_read_from_what_the_sandbox_asked_for() {
        let spec = serde_json::json!({ "gpu_count": 2 });
        assert_eq!(gpu_count(&Sandbox { spec, ..sample_sandbox() }), 2);
        let plain = serde_json::json!({});
        assert_eq!(gpu_count(&Sandbox { spec: plain, ..sample_sandbox() }), 0);
    }

    /// The step an operator reads and the event the console shows carry the same words.
    #[test]
    fn an_extra_detail_is_merged_into_the_step_it_belongs_to() {
        let mut entry = serde_json::json!({ "step": "rollback", "detail": "back on the source" });
        merge(&mut entry, &serde_json::json!({ "rolled_back": true }));
        assert_eq!(entry["step"], serde_json::json!("rollback"));
        assert_eq!(entry["rolled_back"], serde_json::json!(true));
        // Nothing to merge is not an error and changes nothing.
        merge(&mut entry, &serde_json::Value::Null);
        assert_eq!(entry["detail"], serde_json::json!("back on the source"));
    }

    /// A sandbox with nothing to move, as a test fixture built from the type itself.
    fn sample_sandbox() -> Sandbox {
        Sandbox {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            cluster_id: Uuid::nil(),
            gateway_id: None,
            gateway_name: None,
            supervisor_id: None,
            workspace_id: None,
            parent_id: None,
            node_id: Uuid::nil(),
            node_name: None,
            name: "sample".into(),
            pack_key: "ubuntu-dev".into(),
            image: "ghcr.io/nestbox/ubuntu-dev@sha256:abc".into(),
            image_digest: None,
            spec: serde_json::json!({}),
            status: SandboxStatus::Running,
            health: crate::domain::sandbox::SandboxHealth::Unknown,
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            spec_detail: None,
        }
    }
}
