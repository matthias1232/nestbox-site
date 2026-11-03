//! Models: hardware detection, fit classification, and install jobs.
//!
//! The classifier lives in `domain::model` because it is a pure rule. This
//! module supplies the facts it needs — a stored detection result or a fresh
//! probe of the node — and turns an install request into a job.
//!
//! The probe runs on the node, not here. A container sees only the devices it
//! was given, so measuring from the control plane would describe the control
//! plane's container and call it the machine. The runner ships raw text and the
//! parsers below turn it into numbers; the result is stored per node, so a node
//! that is currently unreachable can still be described by its last
//! measurement.

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::job::JobKind;
use crate::domain::model::{
    classify, Fit, FitReport, GpuDevice, HardwareProfile, Model, ModelState,
};
use crate::domain::model_install::{
    reservation_for, Artifact, FreeBudget, InstallState, ModelInstall,
};
use crate::domain::node::{Node, NodeBudget, NodeStatus};
use crate::domain::runtime::{HardwareReport, ModelDownload, ModelServe};
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::runtime::RuntimeError;
use crate::store::TenantScope;
use crate::store::postgres::jobs as job_store;
use crate::store::postgres::model_installs as install_store;
use crate::store::postgres::models as store;
use crate::store::postgres::nodes;

/// One catalog entry with its verdict.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ModelView {
    #[serde(flatten)]
    pub model: Model,
    pub fit: FitReport,
}

/// A node's hardware next to the node facts that qualify it.
///
/// The profile is the measurement; the node carries what is not a measurement —
/// its status after the probe and whether containers on it may be given a GPU.
/// A GPU model needs both, because a card that exists on the host is not a card
/// a container may use.
#[derive(Debug, Clone)]
pub struct NodeHardware {
    pub profile: HardwareProfile,
    pub node: Node,
}

/// What the caller wants to see in the catalog.
///
/// Every field is a narrowing; an empty filter returns everything. The fit
/// filter is a verdict switch, not a second classifier: it keeps or drops rows
/// by the verdict `classify` already produced.
#[derive(Debug, Clone, Default)]
pub struct CatalogQuery {
    /// The node the verdict is for. `None` is the local node.
    pub node: Option<String>,
    pub family: Option<String>,
    pub publisher: Option<String>,
    pub runtime: Option<String>,
    pub cpu_only: bool,
    pub unified_only: bool,
    /// `Some(true)` keeps only what fits this machine, `Some(false)` only what
    /// does not, `None` keeps both.
    pub fits: Option<bool>,
}

impl CatalogQuery {
    fn store_filter(&self) -> store::CatalogFilter {
        store::CatalogFilter {
            family: self.family.clone(),
            publisher: self.publisher.clone(),
            runtime: self.runtime.clone(),
            cpu_only: self.cpu_only,
            unified_only: self.unified_only,
        }
    }
}

/// The catalog for one node, with the budget the verdict ran against.
#[derive(Debug, Clone)]
pub struct CatalogView {
    pub items: Vec<ModelView>,
    pub hardware: HardwareProfile,
    pub budget: NodeBudget,
    pub node: Node,
}

/// The catalog, classified against what one node has left.
pub async fn catalog(
    ctx: &ServiceContext,
    principal: &Principal,
    refresh_hardware: bool,
    query: &CatalogQuery,
) -> Result<CatalogView, ApiError> {
    require(principal, Permission::ModelRead)?;

    // The node and its hardware come first, each in its own short scope: a probe
    // goes through the node's runner and that call must not happen with a
    // transaction open.
    let resolved = resolve_node_for(ctx, principal, query.node.as_deref()).await?;
    let measured =
        load_hardware(ctx, principal.environment_id, refresh_hardware, &resolved).await?;
    // The node as the probe left it: online/offline and `container_gpu` are part
    // of what the fit and the response depend on.
    let node = measured.node.clone();

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let installs = install_store::list(&mut scope, Some(node.id))
        .await
        .map_err(ApiError::from)?;
    let mut models = store::list_catalog(&mut scope, &query.store_filter())
        .await
        .map_err(ApiError::from)?;
    let sandbox_counts = nodes::sandbox_counts(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let hardware = usable_hardware(&measured.profile, &node);
    let free = free_budget(&hardware, &installs);
    for model in &mut models {
        apply_install_state(model, &installs);
    }

    let budget = budget_of(
        &hardware,
        &free,
        &installs,
        sandbox_counts.get(&node.id).copied().unwrap_or(0),
    );
    let items = models
        .into_iter()
        .map(|model| ModelView {
            fit: classify(&model, &hardware, &free),
            model,
        })
        .filter(|view| query.fits.is_none_or(|wanted| view.fit.fit.fits() == wanted))
        .collect();

    Ok(CatalogView {
        items,
        hardware,
        budget,
        node,
    })
}

/// One model with its verdict, for the detail view.
pub async fn requirements(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
) -> Result<ModelView, ApiError> {
    require(principal, Permission::ModelRead)?;

    let resolved = resolve_node_for(ctx, principal, None).await?;
    let measured = load_hardware(ctx, principal.environment_id, false, &resolved).await?;
    let node = measured.node.clone();

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let installs = install_store::list(&mut scope, Some(node.id))
        .await
        .map_err(ApiError::from)?;
    let mut model = store::get_by_key(&mut scope, key)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(
                ErrorCode::MODEL_NOT_FOUND,
                format!("No model with key '{key}'."),
            ),
            other => ApiError::from(other),
        })?;
    scope.commit().await.map_err(ApiError::from)?;

    let hardware = usable_hardware(&measured.profile, &node);
    let free = free_budget(&hardware, &installs);
    apply_install_state(&mut model, &installs);
    Ok(ModelView {
        fit: classify(&model, &hardware, &free),
        model,
    })
}

/// Counts for the filter chips: what the catalog contains and how much of it
/// fits this machine.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct Facets {
    pub total: i64,
    pub fits: i64,
    pub families: std::collections::BTreeMap<String, i64>,
    pub publishers: std::collections::BTreeMap<String, i64>,
    pub runtimes: std::collections::BTreeMap<String, i64>,
    /// How many entries carry each verdict, so a badge count is never guessed.
    pub fits_by_verdict: std::collections::BTreeMap<String, i64>,
    pub cpu_only: i64,
    pub unified_only: i64,
}

/// The facet counts, computed from the same classification the list uses.
pub async fn facets(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Facets, ApiError> {
    require(principal, Permission::ModelRead)?;

    let resolved = resolve_node_for(ctx, principal, None).await?;
    let measured = load_hardware(ctx, principal.environment_id, false, &resolved).await?;
    let node = measured.node.clone();

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let installs = install_store::list(&mut scope, Some(node.id))
        .await
        .map_err(ApiError::from)?;
    let stored = store::catalog_facets(&mut scope).await.map_err(ApiError::from)?;
    let models = store::list_catalog(&mut scope, &store::CatalogFilter::default())
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let hardware = usable_hardware(&measured.profile, &node);
    let free = free_budget(&hardware, &installs);
    let mut facets = Facets {
        total: stored.total,
        fits: 0,
        families: stored.families,
        publishers: stored.publishers,
        runtimes: stored.runtimes,
        fits_by_verdict: std::collections::BTreeMap::new(),
        cpu_only: stored.cpu_only,
        unified_only: stored.unified_only,
    };
    for mut model in models {
        apply_install_state(&mut model, &installs);
        let report = classify(&model, &hardware, &free);
        if report.fit.fits() {
            facets.fits += 1;
        }
        *facets
            .fits_by_verdict
            .entry(report.fit.as_str().to_string())
            .or_insert(0) += 1;
    }
    Ok(facets)
}

/// Only the models installed on the local node.
///
/// Install state lives in `model_installs` (ADR 0018), so this reads the
/// catalog for names and numbers and the install rows for what is present.
pub async fn installed(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<Model>, ApiError> {
    require(principal, Permission::ModelRead)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let node = resolve_node(&mut scope, None).await?;
    let installs = install_store::list(&mut scope, Some(node.id))
        .await
        .map_err(ApiError::from)?;
    let models = store::list_catalog(&mut scope, &store::CatalogFilter::default())
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(models
        .into_iter()
        .filter_map(|mut model| {
            apply_install_state(&mut model, &installs);
            matches!(model.state, ModelState::Installed | ModelState::Updating)
                .then_some(model)
        })
        .collect())
}

/// The hardware of one node, as last measured or freshly probed.
///
/// `node` is a node name (`local`, `gpu-box-01`) or an id; `None` means the
/// local node. The node is named because the control plane is not the machine:
/// the numbers come from that node's own runner.
pub async fn hardware(
    ctx: &ServiceContext,
    principal: &Principal,
    refresh: bool,
    node_name: Option<&str>,
) -> Result<NodeHardware, ApiError> {
    require(principal, Permission::NodeRead)?;
    let node = resolve_node_for(ctx, principal, node_name).await?;
    load_hardware(ctx, principal.environment_id, refresh, &node).await
}

/// Probe one node's hardware through its own runner, now.
///
/// Always measures, and needs `node.manage`: this reaches a machine that may be
/// remote, so it is an operator action rather than a read.
pub async fn probe_node(
    ctx: &ServiceContext,
    principal: &Principal,
    node_id: Uuid,
) -> Result<NodeHardware, ApiError> {
    require(principal, Permission::NodeManage)?;

    let node = resolve_node_for(ctx, principal, Some(&node_id.to_string())).await?;
    probe_and_store(ctx, principal.environment_id, node).await
}

/// Measure a node on the control plane's own initiative, for the loop that watches joined
/// nodes. No caller to authorise: it only asks the node's own runner what it has.
pub async fn measure_node(
    ctx: &ServiceContext,
    environment_id: Uuid,
    node_id: Uuid,
) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let node = nodes::get(&mut scope, node_id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    probe_and_store(ctx, environment_id, node).await.map(|_| ())
}

/// One install as `GET /models/installs` returns it.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct InstallView {
    pub id: Uuid,
    pub model_key: String,
    pub model_name: String,
    pub node_id: Uuid,
    pub node_name: String,
    pub state: InstallState,
    pub installed_version: Option<String>,
    pub reserved_ram_bytes: i64,
    pub reserved_vram_bytes: i64,
    /// Whether the server runs; off frees the reservation.
    pub enabled: bool,
    /// The least RAM the model needs to load at all; the slider's floor.
    pub min_ram_bytes: i64,
    /// What the catalogue recommends, for the slider's marker.
    pub recommended_ram_bytes: Option<i64>,
    /// The most this install could hold: the node's free RAM plus its own share.
    pub max_ram_bytes: i64,
    pub provider_profile_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Every install in the environment, optionally narrowed to one node.
pub async fn installs(
    ctx: &ServiceContext,
    principal: &Principal,
    node_name: Option<&str>,
) -> Result<Vec<InstallView>, ApiError> {
    require(principal, Permission::ModelRead)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let node_filter = match node_name {
        Some(_) => Some(resolve_node(&mut scope, node_name).await?),
        None => None,
    };
    let installs = install_store::list(&mut scope, node_filter.as_ref().map(|node| node.id))
        .await
        .map_err(ApiError::from)?;
    let models = store::list_catalog(&mut scope, &store::CatalogFilter::default())
        .await
        .map_err(ApiError::from)?;
    let nodes = nodes::list(&mut scope).await.map_err(ApiError::from)?;
    let mut free_ram: std::collections::HashMap<Uuid, i64> = std::collections::HashMap::new();
    for install in &installs {
        if !free_ram.contains_key(&install.node_id) {
            let budget = super::node_service::free_budget(&mut scope, install.node_id).await?;
            free_ram.insert(install.node_id, budget.ram_free_bytes);
        }
    }
    scope.commit().await.map_err(ApiError::from)?;
    let ram_floor: std::collections::HashMap<&str, (i64, Option<i64>)> = models
        .iter()
        .map(|model| {
            (
                model.key.as_str(),
                (model.min_ram_bytes.unwrap_or(MIN_RESERVED_RAM), model.recommended_ram_bytes),
            )
        })
        .collect();

    let model_names: std::collections::HashMap<&str, &str> = models
        .iter()
        .map(|model| (model.key.as_str(), model.name.as_str()))
        .collect();
    let node_names: std::collections::HashMap<Uuid, &str> = nodes
        .iter()
        .map(|node| (node.id, node.name.as_str()))
        .collect();

    Ok(installs
        .into_iter()
        .map(|install| {
            let floor = ram_floor.get(install.model_key.as_str()).copied();
            let free = free_ram.get(&install.node_id).copied().unwrap_or(0);
            InstallView {
            model_name: model_names
                .get(install.model_key.as_str())
                .map(|name| (*name).to_string())
                .unwrap_or_else(|| install.model_key.clone()),
            node_name: node_names
                .get(&install.node_id)
                .map(|name| (*name).to_string())
                .unwrap_or_else(|| install.node_id.to_string()),
            id: install.id,
            model_key: install.model_key,
            node_id: install.node_id,
            state: install.state,
            installed_version: install.installed_version,
            reserved_ram_bytes: install.reserved_ram_bytes,
            reserved_vram_bytes: install.reserved_vram_bytes,
            enabled: install.enabled,
            min_ram_bytes: floor.map_or(MIN_RESERVED_RAM, |(min, _)| min),
            recommended_ram_bytes: floor.and_then(|(_, recommended)| recommended),
            max_ram_bytes: free + if install.enabled { install.reserved_ram_bytes } else { 0 },
            provider_profile_id: install.provider_profile_id,
            job_id: install.job_id,
            error: install.error,
            created_at: install.created_at,
            updated_at: install.updated_at,
            }
        })
        .collect())
}

/// Install a model on a node.
///
/// The install reserves memory on the node, creates the provider profile in
/// state `pending`, and enqueues the job. It does **not** download weights or
/// start a model server: that is phase 2, and the response says so.
///
/// Refused when the node's free budget cannot hold the reservation
/// (`node.insufficient_resources`), when the model cannot run on the node at all
/// (`model.not_installable`), or when it is already installed there
/// (`model.already_installed`).
pub async fn install(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
    node_name: Option<&str>,
    force: bool,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelInstall)?;

    // The node and the model first, each in its own short scope: a bad key is
    // answered before a probe, and the probe itself runs with no transaction
    // open.
    let resolved = resolve_node_for(ctx, principal, node_name).await?;
    let model = model_by_key(ctx, principal, key).await?;

    // A model without an artefact this platform can fetch cannot become a
    // server, and reserving memory for it would be a promise nobody keeps. The
    // refusal comes before the probe because it does not depend on the node.
    let artifact = model
        .artifact_ref
        .as_deref()
        .and_then(Artifact::parse)
        .ok_or_else(|| {
            ApiError::unprocessable(
                ErrorCode::MODEL_NOT_SERVABLE,
                format!(
                    "{} has no artefact this platform can fetch, so it cannot be installed \
                     as a local model server.",
                    model.name
                ),
            )
        })?;
    let serve_runtime = model.serve_runtime.clone().ok_or_else(|| {
        ApiError::unprocessable(
            ErrorCode::MODEL_NOT_SERVABLE,
            format!("{} has no server runtime registered.", model.name),
        )
    })?;

    let measured = load_hardware(ctx, principal.environment_id, false, &resolved).await?;
    // The node as the probe left it: install decides on the fresh `container_gpu`.
    let node = measured.node.clone();

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    // One install at a time per node. Two requests that read the same free
    // budget and both insert would reserve the memory twice, and the node would
    // be oversubscribed by exactly the second model. The row lock serialises
    // them; the second transaction then sees the first reservation and is
    // refused with the numbers instead of quietly stealing the space.
    sqlx::query("SELECT id FROM nodes WHERE id = $1 FOR UPDATE")
        .bind(node.id)
        .execute(scope.connection())
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;

    if let Some(existing) = install_store::get(&mut scope, key, node.id)
        .await
        .map_err(ApiError::from)?
    {
        if existing.state == InstallState::Failed {
            // A failed attempt holds nothing; removing its row is what lets the
            // operator try again without a second install record. Its profile
            // goes too, unless a sandbox has selected it in the meantime.
            if let Some(profile_id) = existing.provider_profile_id {
                let sandboxes = install_store::sandboxes_using_profile(&mut scope, profile_id)
                    .await
                    .map_err(ApiError::from)?;
                if !sandboxes.is_empty() {
                    return Err(ApiError::conflict(
                        ErrorCode::PROVIDER_IN_USE,
                        format!(
                            "The profile of the failed install is selected by sandbox {}; \
                             remove it there before installing again.",
                            sandboxes.join(", ")
                        ),
                    ));
                }
                super::provider_service::delete_for_install(&mut scope, profile_id)
                    .await
                    .map_err(ApiError::from)?;
            }
            install_store::delete(&mut scope, existing.id)
                .await
                .map_err(ApiError::from)?;
        } else {
            return Err(ApiError::conflict(
                ErrorCode::MODEL_ALREADY_INSTALLED,
                format!(
                    "{} is already {} on node '{}'. Uninstall it first.",
                    model.name,
                    existing.state.as_str(),
                    node.name
                ),
            ));
        }
    }

    let hardware = usable_hardware(&measured.profile, &node);
    let installs = install_store::list(&mut scope, Some(node.id))
        .await
        .map_err(ApiError::from)?;
    let free = free_budget(&hardware, &installs);
    let report = classify(&model, &hardware, &free);

    // The verdict is advice, not a gate. Without `force` the operator gets the
    // exact reason and the way forward; with `force` the install proceeds and
    // the same reason is recorded as a warning on the install and in the audit
    // trail, so nothing later pretends the node was believed to fit.
    let mut warnings: Vec<String> = Vec::new();
    if !report.fit.allows_install() {
        if force {
            warnings.push(report.reason.clone());
        } else {
            return Err(match report.fit {
                Fit::InsufficientRam | Fit::InsufficientVram | Fit::NoGpu => ApiError::unprocessable(
                    ErrorCode::NODE_INSUFFICIENT_RESOURCES,
                    format!("{} Retry with force to install anyway.", report.reason),
                ),
                _ => ApiError::conflict(
                    ErrorCode::MODEL_NOT_INSTALLABLE,
                    format!("{} Retry with force to install anyway.", report.reason),
                ),
            });
        }
    }

    let (reserved_ram_bytes, reserved_vram_bytes) =
        reservation_for(&model, report.cpu_path);
    if let Err(short) = free.check(reserved_ram_bytes, reserved_vram_bytes) {
        if force {
            warnings.push(short.message());
        } else {
            return Err(ApiError::unprocessable(
                ErrorCode::NODE_INSUFFICIENT_RESOURCES,
                format!("{} Retry with force to install anyway.", short.message()),
            ));
        }
    }

    // The registry row is the image the node will be told to start. Without it
    // the reservation would be for a model nobody can serve.
    if store::server_image(&mut scope, &serve_runtime).await.is_err() {
        return Err(ApiError::unprocessable(
            ErrorCode::MODEL_NOT_SERVABLE,
            format!("No server image is registered for runtime '{serve_runtime}'."),
        ));
    }

    let install = install_store::insert(
        &mut scope,
        install_store::NewInstall {
            model_key: &model.key,
            node_id: node.id,
            reserved_ram_bytes,
            reserved_vram_bytes,
            job_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;

    let profile = super::provider_service::create_for_install(&mut scope, &install, &node)
        .await
        .map_err(ApiError::from)?;
    install_store::set_provider(&mut scope, install.id, Some(profile.id))
        .await
        .map_err(ApiError::from)?;

    let job = crate::store::postgres::jobs::create(
        &mut scope,
        JobKind::ModelInstall,
        &[model.id],
        &serde_json::json!({
            "model_key": model.key,
            "version": model.version,
            "node": node.name,
            "reserved_ram_bytes": reserved_ram_bytes,
            "reserved_vram_bytes": reserved_vram_bytes,
            "artifact": format!("{}:{}", artifact.repo, artifact.file),
            "serve_runtime": serve_runtime,
        }),
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    install_store::set_job(&mut scope, install.id, job.id)
        .await
        .map_err(ApiError::from)?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "model.install",
            subject_type: Some("model_install"),
            subject_id: Some(install.id),
            detail: &serde_json::json!({
                "model_key": model.key,
                "node": node.name,
                "reserved_ram_bytes": reserved_ram_bytes,
                "reserved_vram_bytes": reserved_vram_bytes,
                "provider_profile_id": profile.id,
                "job_id": job.id,
                "force": force,
                "warnings": warnings,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    // Detached, like a recipe build: the request answers as soon as the
    // reservation is committed, and the outcome lands on the install row. A
    // restart settles what the task leaves behind (see `settle_serving`).
    let worker = ctx.clone();
    let environment_id = principal.environment_id;
    let install_id = install.id;
    let job_id = job.id;
    tokio::spawn(async move {
        if let Err(error) = run_install(&worker, environment_id, install_id, job_id).await {
            tracing::error!(
                code = error.code.0,
                install = %install_id,
                job = %job_id,
                "model install could not record its outcome"
            );
        }
    });

    Ok(serde_json::json!({
        "job_id": job.id,
        "install_id": install.id,
        "model_key": model.key,
        "node": node.name,
        "state": InstallState::Installing.as_str(),
        "reserved_ram_bytes": reserved_ram_bytes,
        "reserved_vram_bytes": reserved_vram_bytes,
        "provider_profile_id": profile.id,
        "fit": report.fit,
        "force": force,
        "warnings": warnings,
        "artifact": format!("huggingface:{}/{}", artifact.repo, artifact.file),
        "serve_runtime": serve_runtime,
        "note": "Memory is reserved. The node is fetching the artefact and will start the \
                 server; the state moves through downloading and starting to installed.",
    }))
}

/// Remove a model from a node and release what it reserved.
///
/// A profile that a sandbox selects is not deleted silently: the uninstall is
/// refused with `provider.in_use` and the sandbox names, because the workload
/// would otherwise keep an endpoint that no longer resolves.
pub async fn uninstall(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
    node_name: Option<&str>,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelInstall)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let node = resolve_node(&mut scope, node_name).await?;
    let install = install_store::get(&mut scope, key, node.id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(
                ErrorCode::MODEL_INSTALL_NOT_FOUND,
                format!("'{key}' is not installed on node '{}'.", node.name),
            )
        })?;

    // An install that is still fetching or starting owns a container and a
    // row the pipeline is writing to. Removing the row underneath it would make
    // the worker fail on a missing row and could leave a server running, so the
    // request is refused with the state it is waiting for.
    if matches!(
        install.state,
        InstallState::Installing | InstallState::Downloading | InstallState::Starting
    ) {
        return Err(ApiError::conflict(
            ErrorCode::MODEL_INSTALL_IN_PROGRESS,
            format!(
                "'{}' is {} on node '{}'. Wait for it to finish or fail, then uninstall.",
                install.model_key,
                install.state.as_str(),
                node.name
            ),
        ));
    }

    if let Some(profile_id) = install.provider_profile_id {
        let sandboxes = install_store::sandboxes_using_profile(&mut scope, profile_id)
            .await
            .map_err(ApiError::from)?;
        if !sandboxes.is_empty() {
            return Err(ApiError::conflict(
                ErrorCode::PROVIDER_IN_USE,
                format!(
                    "The provider profile of '{}' is selected by sandbox {}; remove it there first.",
                    install.model_key,
                    sandboxes.join(", ")
                ),
            ));
        }
    }

    // The server goes before the rows do. If the node cannot stop it, the
    // row stays: an install that is gone from the database while its container
    // still holds memory is worse than an uninstall that has to be retried.
    // The artefact stays on disk, so installing the same model again does not
    // download it a second time.
    scope.commit().await.map_err(ApiError::from)?;
    if let Err(error) = ctx.runtime.stop_model(&install.model_key).await {
        return Err(ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!(
                "The model server for '{}' could not be stopped: {error}. The install was kept \
                 so it can be retried.",
                install.model_key
            ),
        ));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    // The profile and the row are removed together, and only now that the
    // container is gone: an uninstall that failed at the stop leaves nothing
    // half-deleted for the retry to trip over.
    if let Some(profile_id) = install.provider_profile_id {
        super::provider_service::delete_for_install(&mut scope, profile_id)
            .await
            .map_err(ApiError::from)?;
    }
    install_store::delete(&mut scope, install.id)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "model.uninstall",
            subject_type: Some("model_install"),
            subject_id: Some(install.id),
            detail: &serde_json::json!({
                "model_key": install.model_key,
                "node": node.name,
                "released_ram_bytes": install.reserved_ram_bytes,
                "released_vram_bytes": install.reserved_vram_bytes,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(serde_json::json!({
        "model_key": install.model_key,
        "node": node.name,
        "released_ram_bytes": install.reserved_ram_bytes,
        "released_vram_bytes": install.reserved_vram_bytes,
        "server_stopped": true,
        "note": "The artefact stays on the node; installing this model again reuses it.",
    }))
}

/// The least reservation any model server gets, when the catalogue names no minimum.
const MIN_RESERVED_RAM: i64 = 64 * 1024 * 1024;

/// What an operator may change on an installed model.
///
/// Every field is optional: a request that only flips `enabled` keeps the
/// figures, and one that only sets the RAM keeps the switch.
#[derive(Debug, Clone, Default)]
pub struct InstallChange {
    /// The memory the model server may use, in bytes. Any value the node can
    /// hold: the catalogue's figure is the default, not a limit.
    pub reserved_ram_bytes: Option<i64>,
    /// The VRAM it reserves, in bytes.
    pub reserved_vram_bytes: Option<i64>,
    /// `false` stops the server and frees the memory; `true` starts it again.
    pub enabled: Option<bool>,
    /// The runtime flags to use from now on. A change to a running model
    /// restarts it, because the flags are the command the container starts
    /// with; a change to a stopped one is just saved.
    pub settings: Option<crate::domain::model_install::ModelTuning>,
}

/// Edit an installed model: its reserved memory and whether it runs.
///
/// Several models can stay installed while only the ones in use are on. Off
/// stops the server and releases the reservation (the artefact stays, so
/// switching on does not download it again); on checks that the node has the
/// memory left, then starts the server with the new limit. A change to the
/// figures of a running model restarts it, because the limit is set when the
/// container starts.
pub async fn update_install(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
    node_name: Option<&str>,
    change: InstallChange,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelInstall)?;

    if change.reserved_ram_bytes.is_some_and(|ram| ram < MIN_RESERVED_RAM) {
        return Err(ApiError::validation("Reserved RAM must be at least 64 MiB."));
    }
    if change.reserved_vram_bytes.is_some_and(|vram| vram < 0) {
        return Err(ApiError::validation("Reserved VRAM cannot be negative."));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let node = resolve_node(&mut scope, node_name).await?;

    // The same lock an install takes: two edits that read the same free budget
    // must not both claim it.
    sqlx::query("SELECT id FROM nodes WHERE id = $1 FOR UPDATE")
        .bind(node.id)
        .execute(scope.connection())
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;

    let install = install_store::get(&mut scope, key, node.id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(
                ErrorCode::MODEL_INSTALL_NOT_FOUND,
                format!("'{key}' is not installed on node '{}'.", node.name),
            )
        })?;
    if install.state != InstallState::Installed {
        return Err(ApiError::conflict(
            ErrorCode::MODEL_INSTALL_IN_PROGRESS,
            format!(
                "'{}' is {} on node '{}'. Only an installed model can be edited.",
                install.model_key,
                install.state.as_str(),
                node.name
            ),
        ));
    }

    let ram = change.reserved_ram_bytes.unwrap_or(install.reserved_ram_bytes);
    // The catalogue's minimum is what the model needs to load at all, so it is
    // a floor and not a warning: a server given less would be killed at start.
    let model = store::get_by_key(&mut scope, &install.model_key)
        .await
        .map_err(ApiError::from)?;
    if let Some(min) = model.min_ram_bytes.filter(|min| ram < *min) {
        return Err(ApiError::validation(format!(
            "{} needs at least {:.1} GiB of RAM to load; {:.1} GiB was asked for.",
            model.name,
            min as f64 / 1_073_741_824.0,
            ram as f64 / 1_073_741_824.0,
        )));
    }
    let vram = change.reserved_vram_bytes.unwrap_or(install.reserved_vram_bytes);
    let enabled = change.enabled.unwrap_or(install.enabled);
    let figures_changed =
        ram != install.reserved_ram_bytes || vram != install.reserved_vram_bytes;
    // Tuning is validated here, not in the store: an impossible value is
    // answered with the field that is wrong, and a merely expensive one is
    // returned as a warning while the save proceeds.
    let mut tuning_warnings: Vec<String> = Vec::new();
    let settings_changed = match change.settings.as_ref() {
        Some(settings) => {
            tuning_warnings = settings
                .validate()
                .map_err(ApiError::validation)?;
            settings != &install.settings
        }
        None => false,
    };
    if let Some(settings) = change.settings.as_ref() {
        install_store::set_settings(&mut scope, install.id, settings)
            .await
            .map_err(ApiError::from)?;
    }

    // Switching off: stop first, then release. A server that could not be
    // stopped keeps its reservation, because freeing memory a container still
    // holds is how a node is oversubscribed.
    if install.enabled && !enabled {
        if let Some(profile_id) = install.provider_profile_id {
            let sandboxes = install_store::sandboxes_using_profile(&mut scope, profile_id)
                .await
                .map_err(ApiError::from)?;
            if !sandboxes.is_empty() {
                return Err(ApiError::conflict(
                    ErrorCode::PROVIDER_IN_USE,
                    format!(
                        "The provider profile of '{}' is selected by sandbox {}; remove it there \
                         before switching the model off.",
                        install.model_key,
                        sandboxes.join(", ")
                    ),
                ));
            }
        }
        scope.commit().await.map_err(ApiError::from)?;
        ctx.runtime.stop_model(&install.model_key).await.map_err(|error| {
            ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::NODE_RUNTIME_UNAVAILABLE,
                format!(
                    "The model server for '{}' could not be stopped: {error}. Nothing was changed.",
                    install.model_key
                ),
            )
        })?;
        let mut scope = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        if figures_changed {
            install_store::set_reservation(&mut scope, install.id, ram, vram)
                .await
                .map_err(ApiError::from)?;
        }
        let off = install_store::set_enabled(&mut scope, install.id, false)
            .await
            .map_err(ApiError::from)?;
        if let Some(profile_id) = off.provider_profile_id {
            crate::store::postgres::providers::set_status(
                &mut scope,
                profile_id,
                crate::domain::provider::ProviderStatus::Unreachable,
            )
            .await
            .map_err(ApiError::from)?;
        }
        update_audit(&mut scope, principal, &off, &node.name, "model.disable").await?;
        scope.commit().await.map_err(ApiError::from)?;
        return Ok(serde_json::json!({
            "model_key": off.model_key,
            "node": node.name,
            "enabled": false,
            "state": off.state.as_str(),
            "reserved_ram_bytes": off.reserved_ram_bytes,
            "released_ram_bytes": install.reserved_ram_bytes,
            "released_vram_bytes": install.reserved_vram_bytes,
            "note": "The server is stopped and its memory is free. The artefact stays on the node.",
        }));
    }

    // Staying off: only the setting changes.
    if !install.enabled && !enabled {
        let saved = install_store::set_reservation(&mut scope, install.id, ram, vram)
            .await
            .map_err(ApiError::from)?;
        update_audit(
            &mut scope,
            principal,
            &saved,
            &node.name,
            if settings_changed { "model.tune" } else { "model.reserve" },
        )
        .await?;
        scope.commit().await.map_err(ApiError::from)?;
        return Ok(serde_json::json!({
            "model_key": saved.model_key,
            "node": node.name,
            "enabled": false,
            "state": saved.state.as_str(),
            "reserved_ram_bytes": saved.reserved_ram_bytes,
            "reserved_vram_bytes": saved.reserved_vram_bytes,
            "settings": saved.settings,
            "warnings": tuning_warnings,
            "note": "Saved. The model is off, so the figure counts against the node once it is switched on.",
        }));
    }

    // On, either newly or with other figures: the node has to hold them. A
    // model that is running already holds its old figures, so those are added
    // back before the check.
    if install.enabled && !figures_changed && !settings_changed {
        return Ok(serde_json::json!({
            "model_key": install.model_key,
            "node": node.name,
            "enabled": true,
            "state": install.state.as_str(),
            "reserved_ram_bytes": install.reserved_ram_bytes,
            "reserved_vram_bytes": install.reserved_vram_bytes,
            "settings": install.settings,
            "warnings": tuning_warnings,
            "note": "Nothing to change.",
        }));
    }
    let budget = super::node_service::free_budget(&mut scope, node.id).await?;
    let (own_ram, own_vram) = if install.enabled {
        (install.reserved_ram_bytes, install.reserved_vram_bytes)
    } else {
        (0, 0)
    };
    let free_ram = budget.ram_free_bytes + own_ram;
    let free_vram = budget.vram_free_bytes + own_vram;
    if ram > free_ram || vram > free_vram {
        return Err(ApiError::unprocessable(
            ErrorCode::NODE_INSUFFICIENT_RESOURCES,
            format!(
                "Node '{}' has {:.1} GiB of RAM and {:.1} GiB of VRAM left for this model; \
                 {:.1} GiB and {:.1} GiB were asked for. Switch another model off or lower the figure.",
                node.name,
                free_ram as f64 / 1_073_741_824.0,
                free_vram as f64 / 1_073_741_824.0,
                ram as f64 / 1_073_741_824.0,
                vram as f64 / 1_073_741_824.0,
            ),
        ));
    }

    let model = store::get_by_key(&mut scope, &install.model_key)
        .await
        .map_err(ApiError::from)?;
    install_store::set_reservation(&mut scope, install.id, ram, vram)
        .await
        .map_err(ApiError::from)?;
    install_store::set_enabled(&mut scope, install.id, true)
        .await
        .map_err(ApiError::from)?;
    let starting = install_store::set_state(
        &mut scope,
        install.id,
        InstallState::Installing,
        None,
        None,
    )
    .await
    .map_err(ApiError::from)?;
    let job = crate::store::postgres::jobs::create(
        &mut scope,
        JobKind::ModelInstall,
        &[model.id],
        &serde_json::json!({
            "model_key": model.key,
            "node": node.name,
            "reserved_ram_bytes": ram,
            "reserved_vram_bytes": vram,
            "restart": install.enabled,
        }),
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    install_store::set_job(&mut scope, install.id, job.id)
        .await
        .map_err(ApiError::from)?;
    update_audit(
        &mut scope,
        principal,
        &starting,
        &node.name,
        if settings_changed {
            "model.tune"
        } else if install.enabled {
            "model.reserve"
        } else {
            "model.enable"
        },
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;

    // A running container keeps its old limit until it is replaced.
    if install.enabled {
        if let Err(error) = ctx.runtime.stop_model(&install.model_key).await {
            let message = format!("The old server could not be stopped for the restart: {error}");
            fail_install(ctx, principal.environment_id, install.id, job.id, &message).await?;
            return Err(ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::NODE_RUNTIME_UNAVAILABLE,
                message,
            ));
        }
    }

    let worker = ctx.clone();
    let environment_id = principal.environment_id;
    let install_id = install.id;
    let job_id = job.id;
    tokio::spawn(async move {
        if let Err(error) = run_install(&worker, environment_id, install_id, job_id).await {
            tracing::error!(
                code = error.code.0,
                install = %install_id,
                job = %job_id,
                "model start could not record its outcome"
            );
        }
    });

    Ok(serde_json::json!({
        "model_key": model.key,
        "node": node.name,
        "enabled": true,
        "state": InstallState::Installing.as_str(),
        "reserved_ram_bytes": ram,
        "reserved_vram_bytes": vram,
        "settings": starting.settings,
        "warnings": tuning_warnings,
        "job_id": job.id,
        "note": "The server is starting with the new figures; the state moves to installed once it answers.",
    }))
}

/// Record an edit of an install in the audit trail.
async fn update_audit(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    install: &ModelInstall,
    node: &str,
    kind: &str,
) -> Result<(), ApiError> {
    crate::store::postgres::audit::record(
        scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind,
            subject_type: Some("model_install"),
            subject_id: Some(install.id),
            detail: &serde_json::json!({
                "model_key": install.model_key,
                "node": node,
                "enabled": install.enabled,
                "reserved_ram_bytes": install.reserved_ram_bytes,
                "reserved_vram_bytes": install.reserved_vram_bytes,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)
}

/// The container name the runner gives a model server.
///
/// Kept identical to the runner's transform so a log or metrics request names
/// the container that actually exists.
fn model_container_name(key: &str) -> String {
    format!(
        "model-{}",
        key.replace(['.', ':', '/'], "-").to_lowercase()
    )
}

/// The last `tail` log lines of a served model.
pub async fn logs(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
    node_name: Option<&str>,
    tail: u32,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelRead)?;
    let tail = tail.clamp(1, 2_000);
    let node = resolve_node_for(ctx, principal, node_name).await?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let install = install_store::get(&mut scope, key, node.id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(
                ErrorCode::MODEL_INSTALL_NOT_FOUND,
                format!("'{key}' is not installed on node '{}'.", node.name),
            )
        })?;
    scope.commit().await.map_err(ApiError::from)?;

    let container = model_container_name(key);
    let text = ctx.runtime.model_logs(key, tail).await.map_err(|error| {
        ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!("The logs of '{key}' could not be read: {error}"),
        )
    })?;
    Ok(serde_json::json!({
        "model_key": install.model_key,
        "node": node.name,
        "container": container,
        "tail": tail,
        "lines": text.lines().collect::<Vec<_>>(),
    }))
}

/// Sample a served model's metrics, keep the sample, and return the series.
///
/// The series is what an operator sees after a spike; the sample is written
/// even when nobody is looking at the page, because a graph that only exists
/// while the tab is open cannot answer "what happened at 03:00".
pub async fn metrics(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
    node_name: Option<&str>,
    limit: i64,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelRead)?;
    let limit = limit.clamp(1, 1_000);
    let node = resolve_node_for(ctx, principal, node_name).await?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let install = install_store::get(&mut scope, key, node.id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(
                ErrorCode::MODEL_INSTALL_NOT_FOUND,
                format!("'{key}' is not installed on node '{}'.", node.name),
            )
        })?;
    let consumers = match install.provider_profile_id {
        Some(profile_id) => install_store::sandboxes_using_profile(&mut scope, profile_id)
            .await
            .map_err(ApiError::from)?,
        None => Vec::new(),
    };
    scope.commit().await.map_err(ApiError::from)?;

    let sampled = ctx.runtime.model_metrics(key).await;
    let current = match sampled {
        Ok(value) => {
            let metrics = value
                .get("metrics")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let mut scope = TenantScope::begin_for(&ctx.pool, principal)
                .await
                .map_err(ApiError::from)?;
            crate::store::postgres::model_metrics::insert(
                &mut scope,
                &install.model_key,
                node.id,
                &metrics,
            )
            .await
            .map_err(ApiError::from)?;
            scope.commit().await.map_err(ApiError::from)?;
            Some(metrics)
        }
        Err(error) => {
            return Ok(serde_json::json!({
                "model_key": install.model_key,
                "node": node.name,
                "available": false,
                "reason": error.to_string(),
                "series": [],
                "consumers": consumers,
            }));
        }
    };

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let series = crate::store::postgres::model_metrics::list(
        &mut scope,
        &install.model_key,
        limit,
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(serde_json::json!({
        "model_key": install.model_key,
        "node": node.name,
        "available": true,
        "current": current,
        "series": series
            .into_iter()
            .map(|(at, metrics)| serde_json::json!({ "at": at, "metrics": metrics }))
            .collect::<Vec<_>>(),
        "consumers": consumers,
    }))
}

/// The `model_install` job body: fetch, start, probe, record.
///
/// Every network call happens outside a transaction, so a slow download does
/// not hold a connection or a row lock. Each step writes the state an operator
/// can see, which is what makes a stuck install diagnosable instead of just
/// "installing" forever: `downloading` means the artefact, `starting` means the
/// server, and a failure names which of the two it was.
///
/// Public so a test can drive it without an HTTP request and without the
/// detached task.
pub async fn run_install(
    ctx: &ServiceContext,
    environment_id: Uuid,
    install_id: Uuid,
    job_id: Uuid,
) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let started = job_store::start(&mut scope, job_id).await;

    let install = match install_store::get_by_id(&mut scope, install_id).await {
        Ok(install) => install,
        Err(error) => {
            let message = "The install row no longer exists.";
            let _ = job_store::log(&mut scope, job_id, "error", message).await;
            let _ = job_store::finish(&mut scope, job_id, None).await;
            let _ = scope.commit().await;
            return Err(ApiError::from(error));
        }
    };
    let model = store::get_by_key(&mut scope, &install.model_key).await?;
    let node = nodes::get(&mut scope, install.node_id).await?;

    let runtime_name = model.serve_runtime.clone().unwrap_or_default();
    let image = store::server_image(&mut scope, &runtime_name).await?;
    let artifact = Artifact::parse(model.artifact_ref.as_deref().unwrap_or_default());

    let plan = match (started, artifact) {
        (Ok(()), Some(artifact)) => Ok((model, node, image, artifact)),
        (Err(error), _) => Err(ApiError::from(error)),
        (_, None) => Err(ApiError::unprocessable(
            ErrorCode::MODEL_NOT_SERVABLE,
            "The catalogue row has no readable artefact reference.",
        )),
    };
    scope.commit().await.map_err(ApiError::from)?;

    let (model, node, image, artifact) = match plan {
        Ok(plan) => plan,
        Err(error) => {
            return fail_install(ctx, environment_id, install_id, job_id, &error.message).await;
        }
    };

    // 1. Fetch. The node owns the disk and the file names; it refuses anything
    //    outside its model base and anything larger than the row advertised.
    set_phase(
        ctx,
        environment_id,
        install_id,
        job_id,
        InstallState::Downloading,
        &format!("Fetching {} from {}.", artifact.file, artifact.repo),
    )
    .await?;

    // The catalogue's `weights_bytes` is the figure the fit gate uses, and a
    // quantised file can be larger than it (Q8_0 of a 1.7B model is about
    // 1.8 GiB). The bound is therefore generous on purpose, with a floor: it
    // exists to stop a mistyped repository from filling the disk, not to
    // second-guess the row.
    const GIB: i64 = 1024 * 1024 * 1024;
    let expected = model
        .weights_bytes
        .filter(|bytes| *bytes > 0)
        .map(|bytes| bytes.saturating_mul(2).saturating_add(GIB / 4))
        .unwrap_or(8 * GIB)
        .max(4 * GIB);
    // Every step below happens on the node the install belongs to. A joined
    // node is a different machine with its own disk and its own Docker, so the
    // call has to name it; the appliance's own node is the runner the control
    // plane already talks to, which is what `None` means.
    let target_node = if node.is_local { None } else { Some(node.id) };
    let download = ctx
        .runtime
        .download_model(ModelDownload {
            model_key: model.key.clone(),
            repo: artifact.repo.clone(),
            file: artifact.file.clone(),
            max_bytes: expected,
            node_id: target_node,
        })
        .await;
    let download = match download {
        Ok(download) => download,
        Err(error) => {
            return fail_install(
                ctx,
                environment_id,
                install_id,
                job_id,
                &format!("The node could not fetch the artefact: {error}"),
            )
            .await;
        }
    };
    log_install(
        ctx,
        environment_id,
        job_id,
        &format!(
            "Artefact {} at {} ({} bytes, {}).",
            artifact.file,
            download.path,
            download.bytes,
            if download.cached { "already present" } else { "downloaded" }
        ),
    )
    .await;

    // 2. Start. The container gets the reservation as its memory limit, the
    //    node's cores as threads, and nothing else: no privileges, no host
    //    mounts beyond the read-only model directory.
    let threads = node_budget_cores(ctx, environment_id, node.id).await;
    let model_path = format!("/models/{}", artifact.file);
    // The image's own command is the default; the operator's tuning replaces
    // the flags it controls instead of appending a second copy of them.
    let mut command = image.command_for(&model_path, threads);
    install.settings.apply(&mut command, threads);
    let memory_limit = install.reserved_ram_bytes.max(512 * 1024 * 1024);

    set_phase(
        ctx,
        environment_id,
        install_id,
        job_id,
        InstallState::Starting,
        &format!("Starting {} on port {}.", image.image, image.port),
    )
    .await?;

    let served = ctx
        .runtime
        .serve_model(ModelServe {
            model_key: model.key.clone(),
            image: image.digest.clone(),
            command,
            memory_limit_bytes: memory_limit,
            cpu_cores: threads as f64,
            node_id: target_node,
            port: image.port,
        })
        .await;
    let served = match served {
        Ok(served) => served,
        Err(error) => {
            let message = format!("The node could not start the model server: {error}");
            // A container that failed to start may still exist half-created;
            // removing it keeps the next attempt from finding a stale name.
            let _ = ctx.runtime.stop_model(&model.key).await;
            return fail_install(ctx, environment_id, install_id, job_id, &message).await;
        }
    };
    log_install(
        ctx,
        environment_id,
        job_id,
        &format!("Server container {} is {}.", served.name, served.status),
    )
    .await;

    // A joined node publishes the server on its overlay address, and that
    // address exists only once the runner has picked the port. The profile was
    // written before the container started, so it is corrected here, before the
    // probe asks it: otherwise every sandbox pointed at this install would get
    // an endpoint nobody answers on.
    if let (Some(address), Some(profile_id)) = (served.published.as_deref(), install.provider_profile_id) {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        crate::store::postgres::providers::set_endpoint(
            &mut scope,
            profile_id,
            &format!("http://{address}/v1"),
        )
        .await
        .map_err(ApiError::from)?;
        scope.commit().await.map_err(ApiError::from)?;
        log_install(
            ctx,
            environment_id,
            job_id,
            &format!("Published on {address}."),
        )
        .await;
    }

    // 3. Probe. A server that is up is not a server that has loaded the model:
    //    llama.cpp answers only after the weights are mapped. The profile the
    //    install created is the one sandboxes will use, so it is the thing that
    //    has to answer.
    let ready = wait_until_answering(ctx, environment_id, install.provider_profile_id).await;
    let profile_id = install.provider_profile_id;
    match ready {
        Ok(detail) => {
            let mut scope = TenantScope::begin(&ctx.pool, environment_id)
                .await
                .map_err(ApiError::from)?;
            install_store::set_state(
                &mut scope,
                install_id,
                InstallState::Installed,
                Some(&model.version),
                None,
            )
            .await
            .map_err(ApiError::from)?;
            if let Some(profile_id) = profile_id {
                crate::store::postgres::providers::set_status(
                    &mut scope,
                    profile_id,
                    crate::domain::provider::ProviderStatus::Reachable,
                )
                .await
                .map_err(ApiError::from)?;
            }
            job_store::log(&mut scope, job_id, "info", &format!("Ready: {detail}."))
                .await
                .map_err(ApiError::from)?;
            job_store::finish(&mut scope, job_id, Some(&serde_json::json!({
                "model_key": model.key,
                "node": node.name,
                "container": served.name,
                "bytes": download.bytes,
            })))
            .await
            .map_err(ApiError::from)?;
            scope.commit().await.map_err(ApiError::from)?;
            Ok(())
        }
        Err(detail) => {
            let _ = ctx.runtime.stop_model(&model.key).await;
            let message = format!("The model server never answered: {detail}");
            fail_install(ctx, environment_id, install_id, job_id, &message).await
        }
    }
}

/// Settle interrupted installs in every environment.
///
/// A boot routine is cross-tenant maintenance like the builder's: it lists the
/// environments unscoped (`environments` has no row-level policy, ADR 0004) and
/// writes only inside each tenant scope.
pub async fn settle_all(ctx: &ServiceContext) -> Result<u64, ApiError> {
    let environments: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments")
        .fetch_all(&ctx.pool)
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;
    let mut settled = 0;
    for environment_id in environments {
        settled += settle_serving(ctx, environment_id).await?;
    }
    Ok(settled)
}

/// Settle the installs a restart interrupted.
///
/// The pipeline runs detached. When the process dies between the reservation
/// and the answer, the row says `downloading` or `starting` and the job says
/// `running`, and neither will ever move. The boot stops whatever container the
/// node may have started and records the failure, so the operator can retry
/// without first finding a stale server.
pub async fn settle_serving(ctx: &ServiceContext, environment_id: Uuid) -> Result<u64, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let stale = sqlx::query_scalar::<_, String>(
        "SELECT model_key FROM model_installs
          WHERE environment_id = $1 AND state IN ('downloading', 'starting')",
    )
    .bind(environment_id)
    .fetch_all(scope.connection())
    .await
    .map_err(|error| ApiError::from(StoreError::Database(error)))?;
    scope.commit().await.map_err(ApiError::from)?;

    for key in &stale {
        // Best effort: a node that is gone has nothing to stop, and the row
        // still has to be settled.
        let _ = ctx.runtime.stop_model(key).await;
    }

    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let message = "The control plane restarted before the install finished. Install the model again.";
    // The profile goes first: once the install rows are `failed`, the set of
    // profiles this boot is responsible for is no longer readable from them,
    // and an old failure would be flipped on every restart.
    sqlx::query(
        "UPDATE provider_profiles
            SET status = 'unreachable', updated_at = now()
          WHERE environment_id = $1
            AND id IN (SELECT provider_profile_id FROM model_installs
                        WHERE environment_id = $1
                          AND state IN ('downloading', 'starting')
                          AND provider_profile_id IS NOT NULL)",
    )
    .bind(environment_id)
    .execute(scope.connection())
    .await
    .map_err(|error| ApiError::from(StoreError::Database(error)))?;

    let settled = sqlx::query(
        "UPDATE model_installs
            SET state = 'failed', error = $2, updated_at = now()
          WHERE environment_id = $1 AND state IN ('downloading', 'starting')",
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
          WHERE environment_id = $1 AND kind = 'model_install' AND status = 'running'",
    )
    .bind(environment_id)
    .bind(message)
    .execute(scope.connection())
    .await
    .map_err(|error| ApiError::from(StoreError::Database(error)))?;

    scope.commit().await.map_err(ApiError::from)?;
    Ok(settled)
}

/// Move an install to a phase and say so in the job log.
async fn set_phase(
    ctx: &ServiceContext,
    environment_id: Uuid,
    install_id: Uuid,
    job_id: Uuid,
    state: InstallState,
    message: &str,
) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    install_store::set_state(&mut scope, install_id, state, None, None)
        .await
        .map_err(ApiError::from)?;
    job_store::log(&mut scope, job_id, "info", message)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Append one line to the install's job log, ignoring a missing job.
async fn log_install(ctx: &ServiceContext, environment_id: Uuid, job_id: Uuid, message: &str) {
    if let Ok(mut scope) = TenantScope::begin(&ctx.pool, environment_id).await {
        let _ = job_store::log(&mut scope, job_id, "info", message).await;
        let _ = scope.commit().await;
    }
}

/// Record a failed install: state, error, profile status, job.
async fn fail_install(
    ctx: &ServiceContext,
    environment_id: Uuid,
    install_id: Uuid,
    job_id: Uuid,
    message: &str,
) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let install = install_store::set_state(
        &mut scope,
        install_id,
        InstallState::Failed,
        None,
        Some(message),
    )
    .await
    .map_err(ApiError::from)?;
    if let Some(profile_id) = install.provider_profile_id {
        crate::store::postgres::providers::set_status(
            &mut scope,
            profile_id,
            crate::domain::provider::ProviderStatus::Unreachable,
        )
        .await
        .map_err(ApiError::from)?;
    }
    job_store::log(&mut scope, job_id, "error", message)
        .await
        .map_err(ApiError::from)?;
    job_store::finish(&mut scope, job_id, None)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Wait until the install's provider profile answers, or give up.
///
/// llama.cpp loads the GGUF before it serves, which on four cores takes tens of
/// seconds for a small model and longer for a large one. The bound is generous
/// but finite, because "still starting" forever is the failure mode this whole
/// state machine exists to avoid.
async fn wait_until_answering(
    ctx: &ServiceContext,
    environment_id: Uuid,
    profile_id: Option<Uuid>,
) -> Result<String, String> {
    let profile_id = profile_id.ok_or_else(|| "the install has no provider profile".to_string())?;
    let mut last = "no attempt was made".to_string();
    for attempt in 0..40 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
        let profile = {
            let mut scope = match TenantScope::begin(&ctx.pool, environment_id).await {
                Ok(scope) => scope,
                Err(error) => return Err(format!("could not read the profile: {error}")),
            };
            let profile = crate::store::postgres::providers::get(&mut scope, profile_id).await;
            let _ = scope.commit().await;
            match profile {
                Ok(profile) => profile,
                Err(error) => return Err(format!("could not read the profile: {error}")),
            }
        };
        let probe = super::provider_service::measure(ctx, &profile).await;
        // A 200, not merely an answer. llama.cpp replies 503 while it maps the
        // weights, and a provider probe calls any HTTP answer "reachable"
        // because for a remote endpoint a 401 already proves the server is
        // there. Here the question is different: has this install finished
        // loading? Only the served model list answers that.
        if probe.status == Some(200) {
            return Ok(format!(
                "{} answered 200 with {} model(s)",
                profile.endpoint,
                probe.model_count.unwrap_or(0)
            ));
        }
        let status = probe
            .status
            .map(|code| format!("HTTP {code}"))
            .unwrap_or_else(|| "no answer".to_string());
        last = if probe.detail.is_empty() {
            status
        } else {
            format!("{status}: {}", probe.detail)
        };
    }
    Err(last)
}

/// The threads to give the server: the node's measured cores, at least one.
async fn node_budget_cores(ctx: &ServiceContext, environment_id: Uuid, node_id: Uuid) -> i64 {
    let measured = match TenantScope::begin(&ctx.pool, environment_id).await {
        Ok(mut scope) => {
            let found = store::latest_hardware(&mut scope, node_id).await;
            let _ = scope.commit().await;
            found.ok().flatten()
        }
        Err(_) => None,
    };
    measured
        .and_then(|profile| profile.cpu_cores)
        .map(i64::from)
        .unwrap_or(1)
        .max(1)
}

/// The node a request names, resolved in a transaction of its own.
///
/// Hardware may be measured through the node's runner, and that call must not
/// happen with a transaction open: it can take as long as the runner's timeout,
/// and a runner that never answers would hold locks for all of it. So the node
/// and its measurement are resolved first, each in a short scope, and the scope
/// that reads installs and the catalog begins afterwards.
async fn resolve_node_for(
    ctx: &ServiceContext,
    principal: &Principal,
    node_ref: Option<&str>,
) -> Result<Node, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let node = resolve_node(&mut scope, node_ref).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(node)
}

/// The node a request names, or the local node when it names none.
///
/// A name is the stable identifier in URLs (`local`, `gpu-box-01`); an id is
/// accepted as well, because that is what a client holds after listing. The name
/// is tried first, so a machine that is literally named like a uuid still wins.
async fn resolve_node(
    scope: &mut TenantScope<'_>,
    node_ref: Option<&str>,
) -> Result<Node, ApiError> {
    let Some(value) = node_ref.map(str::trim).filter(|value| !value.is_empty()) else {
        return nodes::local(scope)
            .await
            .map_err(|error| node_missing(error, "local"));
    };

    match nodes::get_by_name(scope, value).await {
        Ok(node) => Ok(node),
        Err(StoreError::NotFound) => match Uuid::parse_str(value) {
            Ok(id) => nodes::get(scope, id)
                .await
                .map_err(|error| node_missing(error, value)),
            Err(_) => Err(node_missing(StoreError::NotFound, value)),
        },
        Err(other) => Err(ApiError::from(other)),
    }
}

/// Map a missing node row to the published `node.not_found` code.
fn node_missing(error: StoreError, what: &str) -> ApiError {
    match error {
        StoreError::NotFound => ApiError::not_found(
            ErrorCode::NODE_NOT_FOUND,
            format!("No node named '{what}'."),
        ),
        other => ApiError::from(other),
    }
}

/// One catalog row by key, read in a transaction of its own.
async fn model_by_key(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
) -> Result<Model, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let model = store::get_by_key(&mut scope, key)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(
                ErrorCode::MODEL_NOT_FOUND,
                format!("No model with key '{key}'."),
            ),
            other => ApiError::from(other),
        })?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(model)
}

/// The last measurement of a node, or a fresh probe of the local node.
///
/// Returns the node as well as the profile, because a probe changes the node:
/// it marks it online and stores the `container_gpu` the runner reported. Using
/// the node resolved before the probe would classify the first request against a
/// stale `container_gpu` and report a stale status.
///
/// A remote node is not probed from the control plane: that would describe this
/// container, not the machine. Without an adapter for it, an unmeasured remote
/// node gets zeroes and a note, never a guessed number.
///
/// The probe is the one thing here that may take seconds, so it runs with no
/// transaction open. A runner that does not answer writes no profile at all and
/// leaves the node marked offline or unknown; the previous measurement, if any,
/// is what comes back.
async fn load_hardware(
    ctx: &ServiceContext,
    environment_id: Uuid,
    refresh: bool,
    node: &Node,
) -> Result<NodeHardware, ApiError> {
    if !refresh {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        let stored = store::latest_hardware(&mut scope, node.id)
            .await
            .map_err(ApiError::from)?;
        scope.commit().await.map_err(ApiError::from)?;
        if let Some(profile) = stored {
            return Ok(NodeHardware {
                profile,
                node: node.clone(),
            });
        }
    }

    if !node.is_local {
        return Ok(NodeHardware {
            profile: unmeasured_hardware(environment_id, node.id),
            node: node.clone(),
        });
    }

    probe_and_store(ctx, environment_id, node.clone()).await
}

/// What one probe produced: the profile plus the container-GPU answer.
///
/// The flag belongs to the node rather than to the profile, because it is a
/// property of the Docker daemon, not of the cards, and it changes without the
/// hardware changing.
struct Measurement {
    profile: HardwareProfile,
    container_gpu: bool,
}

/// Measure a node and record both the numbers and that the node answered.
async fn probe_and_store(
    ctx: &ServiceContext,
    environment_id: Uuid,
    node: Node,
) -> Result<NodeHardware, ApiError> {
    match probe_hardware(ctx, environment_id, &node).await {
        Ok(measured) => store_probe(ctx, node, measured).await,
        Err(error) => node_offline(ctx, environment_id, node, error).await,
    }
}

/// Persist a measurement and mark the node online with what it reported.
async fn store_probe(
    ctx: &ServiceContext,
    node: Node,
    measured: Measurement,
) -> Result<NodeHardware, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, measured.profile.environment_id)
        .await
        .map_err(ApiError::from)?;
    store::insert_hardware(&mut scope, &measured.profile)
        .await
        .map_err(ApiError::from)?;
    nodes::mark_seen(&mut scope, node.id, NodeStatus::Online, measured.container_gpu)
        .await
        .map_err(ApiError::from)?;
    let updated = nodes::get(&mut scope, node.id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(NodeHardware {
        profile: measured.profile,
        node: updated,
    })
}

/// Record that a node did not answer, and fall back to what was known.
///
/// Nothing is invented. No profile is written, the node is marked offline when
/// its runner was unreachable and unknown when the runner answered with an
/// error, and the previous measurement — if there is one — is returned
/// unchanged: the last thing that was measured is still the best knowledge, and
/// a fresh `detected_at` would make it look newer than it is. Without an earlier
/// measurement there is nothing honest to return, so the answer is `503` and the
/// runner's own message goes to the log rather than to the client.
async fn node_offline(
    ctx: &ServiceContext,
    environment_id: Uuid,
    node: Node,
    error: RuntimeError,
) -> Result<NodeHardware, ApiError> {
    let status = match &error {
        RuntimeError::Unavailable(_) => NodeStatus::Offline,
        RuntimeError::NotFound(_) | RuntimeError::Rejected(_) => NodeStatus::Unknown,
    };

    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    nodes::mark_seen(&mut scope, node.id, status, node.container_gpu)
        .await
        .map_err(ApiError::from)?;
    let previous = store::latest_hardware(&mut scope, node.id)
        .await
        .map_err(ApiError::from)?;
    let updated = nodes::get(&mut scope, node.id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let Some(profile) = previous else {
        return Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!(
                "The runner of node '{}' did not answer and the node has never been measured.",
                node.name
            ),
        )
        .with_internal(error.to_string()));
    };

    Ok(NodeHardware {
        profile,
        node: updated,
    })
}

/// Ask a node's runner what the node has.
///
/// The runner ships files, not conclusions: `/proc/cpuinfo`, `/proc/meminfo`,
/// and the two `nvidia-smi` outputs. It is asked rather than measured here
/// because the control plane may run in a container that sees none of the
/// node's devices, and a container that reports its own limits as the machine's
/// would be wrong in the direction that matters — it would say "no GPU" about a
/// GPU box.
///
/// Phase 1 has one adapter, so only the local node is measured this way; a
/// remote node is `unmeasured_hardware` until it has an adapter of its own.
async fn probe_hardware(
    ctx: &ServiceContext,
    environment_id: Uuid,
    node: &Node,
) -> Result<Measurement, RuntimeError> {
    let report = ctx.runtime_for(node.id).hardware().await?;
    Ok(Measurement {
        profile: profile_from_report(environment_id, node.id, &report),
        container_gpu: report.container_gpu,
    })
}

/// Turn a runner's raw report into a profile.
///
/// Parsing lives here so the numbers are unit-tested Rust rather than a second
/// implementation in Python; the runner stays a transport. A machine without a
/// GPU is not an error: it is a fact, and the RAM and core count are what decide
/// whether a CPU model fits. The detection note explains why the GPU list is
/// what it is, so the console never has to guess.
///
/// Whether containers on the node can *use* a card is `usable_hardware`'s job;
/// this turns the report into numbers and leaves the gate to the caller.
fn profile_from_report(
    environment_id: Uuid,
    node_id: Uuid,
    report: &HardwareReport,
) -> HardwareProfile {
    let cpu = parse_cpuinfo(&report.cpuinfo);
    let system_ram_bytes = parse_meminfo_total(&report.meminfo);
    let gpus = report
        .nvidia_smi_csv
        .as_deref()
        .map(parse_nvidia_smi)
        .unwrap_or_default();
    let (driver_version, cuda_version) =
        parse_nvidia_smi_header(report.nvidia_smi_header.as_deref().unwrap_or_default());

    // A unified-memory machine (DGX Spark / GB10) has no discrete VRAM: the
    // model and the CPU draw from one pool. That pool is deliberately *not*
    // inferred from "no discrete card but a lot of RAM", because every CPU-only
    // VM would then look like a Spark and wrongly pass GPU gates. It is only
    // reported when the hardware identifies itself as GB10/Grace.
    let unified = detect_unified_pool(cpu.model.as_deref(), &gpus, system_ram_bytes);

    let notes = match unified.source.clone() {
        Some(source) => vec![source],
        None => gpu_notes(report, &gpus),
    };
    let note = (!notes.is_empty()).then(|| notes.join(" "));
    let total_vram: i64 = gpus.iter().map(|gpu| gpu.memory_bytes).sum();

    HardwareProfile {
        id: Uuid::new_v4(),
        environment_id,
        node_id,
        detected_at: chrono::Utc::now(),
        hostname: report.hostname.clone(),
        gpus,
        total_vram_bytes: total_vram,
        unified_memory_bytes: unified.bytes,
        unified_memory_source: unified.source,
        cuda_version,
        driver_version,
        detection_note: note,
        cpu_model: cpu.model,
        cpu_cores: cpu.cores,
        cpu_threads: cpu.threads,
        system_ram_bytes,
    }
}

/// Why the GPU list is what it is.
///
/// The runner's own sentence wins over ours where it has one: it stands next to
/// the node and may name the actual reason. What a container may use is a
/// separate question, answered by `usable_hardware` from the node's
/// `container_gpu`.
fn gpu_notes(report: &HardwareReport, gpus: &[GpuDevice]) -> Vec<String> {
    let runner_note = report
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty());

    if report.nvidia_smi_csv.is_none() {
        return vec![runner_note.map(str::to_string).unwrap_or_else(|| {
            "The runner found no nvidia-smi on this node. GPU models need a GPU host; CPU models \
             are gated by RAM and cores only."
                .to_string()
        })];
    }

    let mut notes = Vec::new();
    if gpus.is_empty() {
        notes.push(
            "nvidia-smi ran on the node but reported no usable GPU. GPU models cannot be \
             installed here; CPU models are gated by RAM and cores only."
                .to_string(),
        );
    }
    if let Some(note) = runner_note {
        notes.push(format!("Runner: {note}"));
    }
    notes
}

/// A profile for a node that has not reported: zeroes and the reason.
fn unmeasured_hardware(environment_id: Uuid, node_id: Uuid) -> HardwareProfile {
    HardwareProfile {
        id: Uuid::new_v4(),
        environment_id,
        node_id,
        detected_at: chrono::Utc::now(),
        hostname: None,
        gpus: Vec::new(),
        total_vram_bytes: 0,
        unified_memory_bytes: None,
        unified_memory_source: None,
        cuda_version: None,
        driver_version: None,
        detection_note: Some(
            "This node has not been probed, so its capacity is unknown. No install can be \
             judged against it until its runner reports its hardware."
                .to_string(),
        ),
        cpu_model: None,
        cpu_cores: None,
        cpu_threads: None,
        system_ram_bytes: None,
    }
}

/// The measured hardware as a model may use it on this node.
///
/// A node whose runner reports no NVIDIA container runtime cannot give a
/// container a GPU, so its GPU fields are cleared: every GPU model is `no_gpu`
/// there whatever the host sees (ADR 0018).
fn usable_hardware(profile: &HardwareProfile, node: &Node) -> HardwareProfile {
    if node.container_gpu {
        return profile.clone();
    }

    let mut usable = profile.clone();
    usable.gpus.clear();
    usable.total_vram_bytes = 0;
    usable.unified_memory_bytes = None;
    usable.unified_memory_source = None;
    let runtime_note = "This node's runner reports no NVIDIA container runtime, so GPU models \
                        are no_gpu here.";
    usable.detection_note = Some(match profile.detection_note.as_deref() {
        Some(note) => format!("{note} {runtime_note}"),
        None => runtime_note.to_string(),
    });
    usable
}

/// What this node has left, given its installs.
fn free_budget(hardware: &HardwareProfile, installs: &[ModelInstall]) -> FreeBudget {
    FreeBudget::from_installs(
        hardware.addressable_ram_bytes(),
        hardware.addressable_memory_bytes(),
        installs,
    )
}

/// The budget the console shows next to the catalog.
fn budget_of(
    hardware: &HardwareProfile,
    free: &FreeBudget,
    installs: &[ModelInstall],
    sandboxes: i64,
) -> NodeBudget {
    NodeBudget {
        ram_total_bytes: free.ram_total_bytes,
        ram_reserved_bytes: free.ram_reserved_bytes,
        ram_free_bytes: free.ram_free_bytes,
        vram_total_bytes: free.vram_total_bytes,
        vram_reserved_bytes: free.vram_reserved_bytes,
        vram_free_bytes: free.vram_free_bytes,
        cpu_cores: hardware.cores(),
        gpu_count: hardware.gpu_count(),
        installs: installs.iter().filter(|i| i.state.reserves()).count() as i32,
        sandboxes: sandboxes as i32,
    }
}

/// Substitute the install state for one node into the catalog row.
fn apply_install_state(model: &mut Model, installs: &[ModelInstall]) {
    if let Some(install) = installs.iter().find(|install| install.model_key == model.key) {
        model.state = install.state.model_state();
        model.installed_version = install.installed_version.clone();
    }
}

/// The CPU facts the classifier needs, as `/proc/cpuinfo` describes them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CpuFacts {
    model: Option<String>,
    /// Distinct physical cores across all sockets.
    cores: Option<i32>,
    /// Logical processors, which is what a thread pool can use.
    threads: Option<i32>,
}

/// Parse model name, physical cores, and logical processors.
///
/// `/proc/cpuinfo` is a sequence of `key\t: value` blocks separated by blank
/// lines. The physical core count needs the `(physical id, core id)` pairs; on
/// an ARM host (GB10) those keys are absent, so the logical count is the honest
/// answer there and is reported as both.
fn parse_cpuinfo(text: &str) -> CpuFacts {
    let mut model: Option<String> = None;
    let mut threads = 0i32;
    let mut pairs: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    let mut physical: Option<String> = None;
    let mut core: Option<String> = None;
    let mut cores_field: Option<i32> = None;

    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            // A blank line ends a processor block.
            if line.trim().is_empty() {
                physical = None;
                core = None;
            }
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "processor" => threads += 1,
            "model name" | "Model" if model.is_none() => model = Some(value.to_string()),
            "physical id" => physical = Some(value.to_string()),
            "core id" => core = Some(value.to_string()),
            "cpu cores" => cores_field = value.parse::<i32>().ok().or(cores_field),
            _ => {}
        }
        if let (Some(p), Some(c)) = (physical.as_ref(), core.as_ref()) {
            pairs.insert((p.clone(), c.clone()));
        }
    }

    let cores = if !pairs.is_empty() {
        Some(pairs.len() as i32)
    } else {
        cores_field.or(if threads > 0 { Some(threads) } else { None })
    };

    CpuFacts {
        model,
        cores,
        threads: (threads > 0).then_some(threads),
    }
}

/// `MemTotal` from `/proc/meminfo`, in bytes.
///
/// The value is in kB. It is what the kernel says it can give to user space,
/// which is the right figure for "will this model load" — not the installed
/// DIMM size.
fn parse_meminfo_total(text: &str) -> Option<i64> {
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("MemTotal:") else {
            continue;
        };
        let value = rest.trim().trim_end_matches(" kB").trim();
        return value.parse::<i64>().ok().map(|kb| kb * 1024);
    }
    None
}

/// The unified pool, when the hardware says it is a GB10-class machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct UnifiedPool {
    bytes: Option<i64>,
    source: Option<String>,
}

/// Detect a unified memory pool without guessing.
///
/// Only two things count as evidence: `nvidia-smi` naming a GB10 device, or
/// `/proc/cpuinfo` naming NVIDIA's Grace/GB10 part. Both are reported together
/// with the pool size, so a reader can see the number was measured from
/// `MemTotal` on a machine that is known to share it.
fn detect_unified_pool(
    cpu_model: Option<&str>,
    gpus: &[GpuDevice],
    system_ram_bytes: Option<i64>,
) -> UnifiedPool {
    let names_gb10 = |value: &str| {
        let lowered = value.to_ascii_lowercase();
        lowered.contains("gb10") || lowered.contains("grace")
    };

    let from_gpu = gpus.iter().any(|gpu| names_gb10(&gpu.name));
    let from_cpu = cpu_model.is_some_and(names_gb10);
    if !from_gpu && !from_cpu {
        return UnifiedPool::default();
    }

    let Some(ram) = system_ram_bytes else {
        return UnifiedPool {
            bytes: None,
            source: Some(
                "Unified-memory machine (GB10/Grace) recognised, but MemTotal could not be \
                 read, so the pool size is unknown rather than guessed."
                    .to_string(),
            ),
        };
    };

    let evidence = if from_gpu {
        "nvidia-smi reports a GB10 device"
    } else {
        "the CPU identifies as NVIDIA GB10/Grace"
    };
    UnifiedPool {
        bytes: Some(ram),
        source: Some(format!(
            "Unified memory: {evidence}; pool = MemTotal from /proc/meminfo ({} GiB), \
             shared by CPU and GPU.",
            ram / (1024 * 1024 * 1024)
        )),
    }
}

/// The driver and CUDA versions from the plain `nvidia-smi` header.
///
/// Both are printed in the same table line:
/// `| NVIDIA-SMI 535.104.05   Driver Version: 535.104.05   CUDA Version: 12.2 |`.
/// Neither is a query field that `nvidia-smi` offers per device, so the line is
/// read from the text the runner sends. A field that is not there stays `None`
/// rather than becoming a guess.
fn parse_nvidia_smi_header(text: &str) -> (Option<String>, Option<String>) {
    fn field(text: &str, name: &str) -> Option<String> {
        for line in text.lines() {
            let Some(index) = line.find(name) else {
                continue;
            };
            let value = line[index + name.len()..]
                .trim()
                .split_whitespace()
                .next()?
                .trim_end_matches('|')
                .trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
        None
    }
    (
        field(text, "Driver Version:"),
        field(text, "CUDA Version:"),
    )
}

/// Parse `index, name, total MiB, free MiB` lines.
fn parse_nvidia_smi(text: &str) -> Vec<GpuDevice> {
    const MIB: i64 = 1024 * 1024;
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split(',').map(str::trim);
            let index = parts.next()?.parse::<u32>().ok()?;
            let name = parts.next()?.to_string();
            let total = parts.next()?.parse::<i64>().ok()?;
            let free = parts.next().and_then(|v| v.parse::<i64>().ok());
            Some(GpuDevice {
                index,
                name,
                memory_bytes: total * MIB,
                free_bytes: free.map(|value| value * MIB),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeAdapter;

    #[test]
    fn nvidia_smi_output_is_parsed() {
        let text = "0, NVIDIA H100 80GB HBM3, 81559, 81000\n1, NVIDIA H100 80GB HBM3, 81559, 0\n";
        let gpus = parse_nvidia_smi(text);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].index, 0);
        assert_eq!(gpus[0].name, "NVIDIA H100 80GB HBM3");
        assert_eq!(gpus[0].memory_bytes, 81559 * 1024 * 1024);
        assert_eq!(gpus[1].free_bytes, Some(0));
    }

    #[test]
    fn garbage_output_yields_no_gpus_rather_than_a_panic() {
        assert!(parse_nvidia_smi("no gpu here").is_empty());
        assert!(parse_nvidia_smi("").is_empty());
        assert!(parse_nvidia_smi("0, , , ").is_empty());
    }

    /// A report from a plain x86 node with no NVIDIA tooling, as the runner
    /// sends it: raw files, one sentence about the missing tool, no GPU flag.
    fn report_without_nvidia_tooling() -> HardwareReport {
        HardwareReport {
            hostname: Some("appliance".into()),
            cpuinfo: "\
processor\t: 0
model name\t: Intel(R) Core(TM) i7-2600 CPU @ 3.40GHz
physical id\t: 0
core id\t\t: 0

processor\t: 1
model name\t: Intel(R) Core(TM) i7-2600 CPU @ 3.40GHz
physical id\t: 0
core id\t\t: 0
"
            .into(),
            meminfo: "MemTotal:        8176000 kB\n".into(),
            nvidia_smi_csv: None,
            nvidia_smi_header: None,
            container_gpu: false,
            note: Some(
                "nvidia-smi is not available on this node; GPU models need a GPU host.".into(),
            ),
        }
    }

    #[tokio::test]
    async fn the_fake_runners_report_parses_to_the_machines_numbers() {
        // The fake adapter is what every service test runs against, so its
        // report has to mean the same thing a real node's report means.
        let report = crate::runtime::fake::FakeRuntime::new()
            .hardware()
            .await
            .expect("the fake adapter always answers");

        assert!(
            !report.container_gpu,
            "the fake node cannot give a container a GPU, so no GPU model may be \
             called installable there"
        );

        let profile = profile_from_report(uuid::Uuid::nil(), uuid::Uuid::nil(), &report);

        assert_eq!(profile.cpu_model.as_deref(), Some("Fake CPU"));
        assert_eq!(profile.cpu_cores, Some(2));
        assert_eq!(profile.cpu_threads, Some(2));
        assert_eq!(profile.system_ram_bytes, Some(8 * 1024 * 1024 * 1024));
        assert_eq!(profile.gpu_count(), 0);
        assert_eq!(profile.total_vram_bytes, 0);
        assert_eq!(profile.node_id, uuid::Uuid::nil());
        assert_eq!(
            profile.detection_note.as_deref(),
            Some("fake runtime: no GPU"),
            "the runner's own sentence is what an operator reads"
        );
    }

    #[test]
    fn a_node_without_nvidia_tooling_yields_no_gpu_and_the_runners_reason() {
        let profile = profile_from_report(
            uuid::Uuid::nil(),
            uuid::Uuid::nil(),
            &report_without_nvidia_tooling(),
        );

        assert!(profile.gpus.is_empty());
        assert_eq!(profile.cpu_cores, Some(1));
        assert!(!profile.is_unified(), "a plain x86 node is not a Spark");
        assert_eq!(
            profile.detection_note.as_deref(),
            Some("nvidia-smi is not available on this node; GPU models need a GPU host.")
        );
    }

    #[test]
    fn the_nvidia_smi_header_yields_driver_and_cuda_versions() {
        let header = "\
+-----------------------------------------------------------------------------+
| NVIDIA-SMI 535.104.05   Driver Version: 535.104.05   CUDA Version: 12.2     |
|-------------------------------+----------------------+----------------------+
";
        assert_eq!(
            parse_nvidia_smi_header(header),
            (Some("535.104.05".into()), Some("12.2".into()))
        );
        assert_eq!(parse_nvidia_smi_header(""), (None, None));
        assert_eq!(
            parse_nvidia_smi_header("no versions in here"),
            (None, None)
        );
    }

    #[test]
    fn cpuinfo_yields_model_cores_and_threads() {
        let text = "\
processor\t: 0
model name\t: AMD EPYC 7B13 64-Core Processor
physical id\t: 0
core id\t\t: 0
cpu cores\t: 2

processor\t: 1
model name\t: AMD EPYC 7B13 64-Core Processor
physical id\t: 0
core id\t\t: 1
cpu cores\t: 2

processor\t: 2
model name\t: AMD EPYC 7B13 64-Core Processor
physical id\t: 0
core id\t\t: 0
cpu cores\t: 2
";
        let facts = parse_cpuinfo(text);
        assert_eq!(facts.model.as_deref(), Some("AMD EPYC 7B13 64-Core Processor"));
        assert_eq!(facts.cores, Some(2));
        assert_eq!(facts.threads, Some(3));
    }

    #[test]
    fn an_arm_cpuinfo_without_core_ids_falls_back_to_the_logical_count() {
        // GB10 reports neither `physical id` nor `core id`.
        let text = "\
processor\t: 0
model name\t: NVIDIA GB10
processor\t: 1
model name\t: NVIDIA GB10
";
        let facts = parse_cpuinfo(text);
        assert_eq!(facts.model.as_deref(), Some("NVIDIA GB10"));
        assert_eq!(facts.cores, Some(2));
        assert_eq!(facts.threads, Some(2));
    }

    #[test]
    fn empty_cpuinfo_yields_no_claims() {
        let facts = parse_cpuinfo("");
        assert_eq!(facts, CpuFacts::default());
    }

    #[test]
    fn memtotal_is_read_in_bytes() {
        let text = "MemTotal:       16324516 kB\nMemFree:         1234567 kB\n";
        assert_eq!(parse_meminfo_total(text), Some(16324516 * 1024));
        assert_eq!(parse_meminfo_total("MemFree: 1 kB\n"), None);
    }

    #[test]
    fn a_plain_x86_machine_is_never_treated_as_unified_memory() {
        // The dangerous mistake would be "no discrete card but plenty of RAM,
        // so it must be a Spark": that would let GPU gates pass on a CPU VM.
        let pool = detect_unified_pool(Some("AMD EPYC 7B13"), &[], Some(128 * 1024 * 1024 * 1024));
        assert_eq!(pool, UnifiedPool::default());
    }

    #[test]
    fn a_gb10_machine_reports_the_pool_with_its_source() {
        let gpus = vec![GpuDevice {
            index: 0,
            name: "NVIDIA GB10".into(),
            memory_bytes: 0,
            free_bytes: None,
        }];
        let pool = detect_unified_pool(Some("NVIDIA GB10"), &gpus, Some(128 * 1024 * 1024 * 1024));
        assert_eq!(pool.bytes, Some(128 * 1024 * 1024 * 1024));
        let source = pool.source.expect("source note");
        assert!(source.contains("MemTotal"));
    }

    #[test]
    fn a_recognised_unified_machine_without_meminfo_reports_unknown_rather_than_a_number() {
        let pool = detect_unified_pool(Some("NVIDIA GB10"), &[], None);
        assert_eq!(pool.bytes, None);
        assert!(pool.source.expect("note").contains("unknown"));
    }

    fn node(container_gpu: bool) -> Node {
        Node {
            id: uuid::Uuid::nil(),
            name: "local".into(),
            display_name: "Local".into(),
            kind: crate::domain::node::NodeKind::Local,
            status: crate::domain::node::NodeStatus::Online,
            is_local: true,
            runner_url: String::new(),
            container_gpu,
            zone: String::new(),
            region: String::new(),
            description: None,
            last_seen: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn measured_gpu() -> HardwareProfile {
        HardwareProfile {
            id: uuid::Uuid::nil(),
            environment_id: uuid::Uuid::nil(),
            node_id: uuid::Uuid::nil(),
            detected_at: chrono::Utc::now(),
            hostname: Some("host".into()),
            gpus: vec![GpuDevice {
                index: 0,
                name: "NVIDIA H100".into(),
                memory_bytes: 80 * 1024 * 1024 * 1024,
                free_bytes: None,
            }],
            total_vram_bytes: 80 * 1024 * 1024 * 1024,
            unified_memory_bytes: None,
            unified_memory_source: None,
            cuda_version: None,
            driver_version: None,
            detection_note: None,
            cpu_model: None,
            cpu_cores: Some(8),
            cpu_threads: None,
            system_ram_bytes: Some(64 * 1024 * 1024 * 1024),
        }
    }

    #[test]
    fn a_node_without_container_gpu_offers_no_gpu_whatever_the_host_reports() {
        // ADR 0018: the runner's `container_gpu` is the gate, not the host's
        // `nvidia-smi`. Without an NVIDIA runtime in Docker, a GPU model is
        // `no_gpu` even though the machine has a card.
        let usable = usable_hardware(&measured_gpu(), &node(false));
        assert!(!usable.has_gpu());
        assert_eq!(usable.addressable_memory_bytes(), 0);
        assert!(usable
            .detection_note
            .unwrap_or_default()
            .contains("no NVIDIA container runtime"));
    }

    #[test]
    fn a_node_with_container_gpu_keeps_what_was_measured() {
        let measured = measured_gpu();
        let usable = usable_hardware(&measured, &node(true));
        assert_eq!(usable.gpus.len(), 1);
        assert_eq!(usable.addressable_memory_bytes(), measured.addressable_memory_bytes());
    }
}
