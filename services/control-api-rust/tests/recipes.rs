//! The builder against a real database.
//!
//! These prove the parts a pure function cannot: that a draft is stored as a
//! draft, that an edit bumps the revision and keeps the old image, that a build
//! records the tag its hash names, that an unbuilt recipe refuses to
//! instantiate, and that a sandbox is placed with the recipe's mounts and
//! resources.
//!
//! They run only when `TEST_DATABASE_URL` is set and own a database of their
//! own, so they can run in parallel with `tests/database.rs` without either
//! suite dropping the other's schema:
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_test?host=/tmp/opencode/pgtest'
//!   cargo test --test recipes

use std::str::FromStr;
use std::sync::Arc;

use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::builder::{
    AgentSpec, HomeMode, Persistence, ProjectMount, ResourceLimits,
};
use nestbox_control_api::domain::gateway::{SupervisorReport, SupervisorStatus};
use nestbox_control_api::error::{ApiError, ErrorCode};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::services::builder_service::{self, RecipeInput};
use nestbox_control_api::services::mcp_service;
use nestbox_control_api::services::gateway_service;
use nestbox_control_api::store::postgres::recipes as recipe_store;
use nestbox_control_api::store::TenantScope;

/// The database this suite owns. Fixed so a previous failed run is replaced
/// rather than accumulated.
const DATABASE: &str = "nestbox_recipes_test";

/// The seeded administrator. The audit trail has a foreign key to `users`, so a
/// made-up actor id would turn a successful write into a 500.
const ADMIN: &str = "00000000-0000-0000-0000-000000000005";

static READY: OnceCell<bool> = OnceCell::const_new();

/// Create the suite's database once per process, then migrate it.
///
/// The pool is not cached: each `#[tokio::test]` gets its own runtime, and a
/// connection created in a finished runtime cannot be used in the next one.
async fn ready() -> bool {
    *READY
        .get_or_init(|| async {
            let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
                return false;
            };
            let Ok(options) = PgConnectOptions::from_str(&url) else {
                return false;
            };

            // A database of our own: `tests/database.rs` drops and recreates
            // the public schema of its database, and two suites doing that at
            // once is a race, not a test.
            let Ok(maintenance) =
                sqlx::PgPool::connect_with(options.clone().database("postgres")).await
            else {
                return false;
            };
            let setup = async {
                sqlx::query(
                    "DO $$BEGIN
                        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'nestbox_app') THEN
                            CREATE ROLE nestbox_app LOGIN;
                        END IF;
                    END$$",
                )
                .execute(&maintenance)
                .await?;
                sqlx::query(&format!("DROP DATABASE IF EXISTS {DATABASE} WITH (FORCE)"))
                    .execute(&maintenance)
                    .await?;
                sqlx::query(&format!("CREATE DATABASE {DATABASE}"))
                    .execute(&maintenance)
                    .await?;

                let pool = sqlx::PgPool::connect_with(options.database(DATABASE)).await?;
                sqlx::query("GRANT USAGE, CREATE ON SCHEMA public TO nestbox_app")
                    .execute(&pool)
                    .await?;
                sqlx::migrate!("./migrations").run(&pool).await?;
                Ok::<(), sqlx::Error>(())
            }
            .await;
            setup.is_ok()
        })
        .await
}

/// A pool for one test.
async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?;
    sqlx::PgPool::connect_with(options.database(DATABASE)).await.ok()
}

async fn environment_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .expect("the default environment is seeded")
}

fn principal(environment_id: Uuid) -> Principal {
    Principal {
        user_id: Uuid::parse_str(ADMIN).expect("a uuid"),
        environment_id,
        role: Role::Admin,
        display_name: "Test administrator".into(),
        via: CredentialSource::Token,
    }
}

/// A context with a database and the in-memory runtime: no Docker involved.
fn context(pool: &PgPool, runtime: &Arc<FakeRuntime>) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        runtime.clone(),
        Arc::new(nestbox_control_api::events::bus::LocalBus::new()),
        std::path::PathBuf::from("/tmp/opencode"),
    )
}

/// A name unique to this test, so tests can share one database.
fn unique(prefix: &str) -> String {
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &suffix[..8])
}

/// A plain Ubuntu shell recipe: one agent, no tools, one GiB of RAM.
fn shell_recipe(name: &str) -> RecipeInput {
    RecipeInput {
        name: name.to_string(),
        baseline_key: "ubuntu-24.04".into(),
        tools: vec!["git".into()],
        extra_packages: vec![],
        repositories: vec![],
        mcp_server_keys: vec![],
        agents: vec![AgentSpec {
            pack_key: "hermes".into(),
            instance: "hermes".into(),
            home_mode: HomeMode::Shared,
            persistence: Persistence::Persistent,
            home_paths: vec![],
            port: None,
        }],
        resources: ResourceLimits {
            memory_limit: "1Gi".into(),
            cpu_limit: 1.0,
            gpu_count: 0,
            pids_limit: 256,
        },
        project_mounts: vec![],
        node: None,
    }
}

/// Wait for the detached build task to write its outcome.
async fn wait_for_build(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> nestbox_control_api::domain::builder::Recipe {
    for _ in 0..200 {
        let recipe = builder_service::get(ctx, principal, id)
            .await
            .expect("the recipe is readable");
        if matches!(
            recipe.state,
            nestbox_control_api::domain::builder::RecipeState::Built
                | nestbox_control_api::domain::builder::RecipeState::Failed
        ) {
            return recipe;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the build did not settle within ten seconds");
}

#[tokio::test]
async fn a_draft_is_stored_as_a_draft_and_a_duplicate_name_is_a_conflict() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment);

    let name = unique("draft");
    let created = builder_service::create(&ctx, &who, shell_recipe(&name))
        .await
        .expect("a valid draft is created");
    assert_eq!(created.name, name);
    assert_eq!(
        created.state,
        nestbox_control_api::domain::builder::RecipeState::Draft
    );
    assert_eq!(created.revision, 1);
    assert!(created.image_ref.is_none());
    assert_eq!(created.owner_user_id, Some(who.user_id));
    assert_eq!(created.tools, vec!["git".to_string()]);

    let duplicate = builder_service::create(&ctx, &who, shell_recipe(&name)).await;
    let error = duplicate.expect_err("the same name twice is refused");
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
    assert_eq!(error.code, ErrorCode::CONFLICT);

    builder_service::delete(&ctx, &who, created.id)
        .await
        .expect("deletable");
}

#[tokio::test]
async fn an_edit_bumps_the_revision_resets_the_state_and_keeps_the_old_image() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment);

    let created = builder_service::create(&ctx, &who, shell_recipe(&unique("edited")))
        .await
        .expect("created");

    // Pretend a build succeeded: the state and the image a sandbox would use.
    let old_tag = "nestbox-recipe/edited:0000000000000000";
    let mut scope = TenantScope::begin(pool, environment).await.expect("scope");
    recipe_store::set_build(
        &mut scope,
        created.id,
        nestbox_control_api::domain::builder::RecipeState::Built,
        Some(old_tag),
        Some("0000000000000000"),
        None,
        None,
    )
    .await
    .expect("state recorded");
    scope.commit().await.expect("commit");

    let mut edit = shell_recipe(&created.name);
    edit.resources.cpu_limit = 2.0;
    let updated = builder_service::update(&ctx, &who, created.id, edit)
        .await
        .expect("updated");

    assert_eq!(updated.revision, created.revision + 1);
    assert_eq!(
        updated.state,
        nestbox_control_api::domain::builder::RecipeState::Draft
    );
    assert_eq!(
        updated.image_ref.as_deref(),
        Some(old_tag),
        "the old image stays until a new build replaces it"
    );
}

#[tokio::test]
async fn a_build_records_the_tag_its_hash_names_and_a_second_build_reuses_it() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment);

    let name = unique("built");
    let created = builder_service::create(&ctx, &who, shell_recipe(&name))
        .await
        .expect("created");

    let queued = builder_service::build(&ctx, &who, created.id)
        .await
        .expect("the build is queued");
    assert!(!queued.reused);
    assert!(queued.job_id.is_some());
    assert_eq!(
        queued.recipe.state,
        nestbox_control_api::domain::builder::RecipeState::Building
    );

    let built = wait_for_build(&ctx, &who, created.id).await;
    assert_eq!(
        built.state,
        nestbox_control_api::domain::builder::RecipeState::Built,
        "build error: {:?}",
        built.build_error
    );
    let hash = built.content_hash.clone().expect("a content hash");
    assert_eq!(hash.len(), 64);
    assert_eq!(
        built.image_ref,
        Some(builder_service::image_tag(&name, &hash))
    );
    assert!(built
        .image_ref
        .as_deref()
        .is_some_and(|tag| tag.starts_with(&format!("nestbox-recipe/{name}:"))));

    // Nothing changed, so the same image answers the second request.
    let again = builder_service::build(&ctx, &who, created.id)
        .await
        .expect("the second request is answered");
    assert!(again.reused);
    assert!(again.job_id.is_none());
    assert_eq!(again.recipe.image_ref, built.image_ref);

    // An edit moves the content, so the next build produces a different tag.
    let mut edit = shell_recipe(&name);
    edit.resources.pids_limit = 512;
    builder_service::update(&ctx, &who, created.id, edit)
        .await
        .expect("updated");
    builder_service::build(&ctx, &who, created.id)
        .await
        .expect("queued");
    let rebuilt = wait_for_build(&ctx, &who, created.id).await;
    assert_eq!(
        rebuilt.state,
        nestbox_control_api::domain::builder::RecipeState::Built,
        "build error: {:?}",
        rebuilt.build_error
    );
    assert_ne!(rebuilt.content_hash, built.content_hash);
    assert_ne!(rebuilt.image_ref, built.image_ref);
}

#[tokio::test]
async fn a_recipe_without_an_image_refuses_to_instantiate() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment);

    let created = builder_service::create(&ctx, &who, shell_recipe(&unique("unbuilt")))
        .await
        .expect("created");
    let error = builder_service::instantiate(&ctx, &who, created.id, &unique("sandbox"), None)
        .await
        .expect_err("an unbuilt recipe has no image");
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
    assert_eq!(error.code, ErrorCode::RECIPE_NOT_BUILT);
}

#[tokio::test]
async fn an_incompatible_pack_and_an_unknown_tool_are_refused_before_the_row_exists() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment);

    // hermes supports Ubuntu and Debian, not Rocky.
    let mut rocky = shell_recipe(&unique("rocky"));
    rocky.baseline_key = "rocky-9".into();
    let error = builder_service::create(&ctx, &who, rocky)
        .await
        .expect_err("hermes on Rocky is refused");
    assert_eq!(error.code, ErrorCode::PACK_INCOMPATIBLE);
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);

    // build-essential has no dnf name in the catalog: the tool resolves for apt
    // and must be an error for dnf rather than a silently missing package.
    let mut rocky_tool = shell_recipe(&unique("rocky-tool"));
    rocky_tool.baseline_key = "rocky-9".into();
    rocky_tool.agents = vec![];
    rocky_tool.tools = vec!["build-essential".into()];
    let error = builder_service::create(&ctx, &who, rocky_tool)
        .await
        .expect_err("a tool without an rpm name is refused");
    assert_eq!(error.code, ErrorCode::RECIPE_INVALID);
    assert!(error.message.contains("dnf"), "{}", error.message);

    let mut unknown = shell_recipe(&unique("unknown-tool"));
    unknown.tools = vec!["not-a-tool".into()];
    let error = builder_service::create(&ctx, &who, unknown)
        .await
        .expect_err("an unknown tool is refused");
    assert_eq!(error.code, ErrorCode::RECIPE_INVALID);
}

#[tokio::test]
async fn instantiate_places_the_sandbox_with_the_recipes_mount_and_refuses_an_oversized_one() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment);

    // The node as VM200 reports it: 8 GiB of RAM, no GPU.
    let node: Uuid = sqlx::query_scalar("SELECT id FROM nodes WHERE is_local")
        .fetch_one(pool)
        .await
        .expect("the local node is seeded");
    sqlx::query(
        "INSERT INTO hardware_profiles
             (environment_id, node_id, hostname, gpus, total_vram_bytes, cpu_cores, system_ram_bytes)
         VALUES ($1, $2, 'test-node', '[]'::jsonb, 0, 4, $3)",
    )
    .bind(environment)
    .bind(node)
    .bind(8i64 * 1024 * 1024 * 1024)
    .execute(pool)
    .await
    .expect("a measurement");

    let slug = unique("project");
    let root: Uuid = sqlx::query_scalar(
        "INSERT INTO persistent_roots (environment_id, node_id, kind, owner_user_id, slug, host_path)
         VALUES ($1, $2, 'project', $3, $4, $5) RETURNING id",
    )
    .bind(environment)
    .bind(node)
    .bind(who.user_id)
    .bind(&slug)
    .bind(format!("/var/lib/nestbox/persistent/{slug}"))
    .fetch_one(pool)
    .await
    .expect("a persistent root");

    let name = unique("mounted");
    let mut input = shell_recipe(&name);
    input.project_mounts = vec![ProjectMount {
        root_id: root,
        mount_path: "/workspace".into(),
        read_only: false,
    }];
    let recipe = builder_service::create(&ctx, &who, input.clone())
        .await
        .expect("created");

    // Mark it built so instantiation is about placement, not about building.
    let hash = "a".repeat(64);
    let tag = builder_service::image_tag(&name, &hash);
    let mut scope = TenantScope::begin(pool, environment).await.expect("scope");
    recipe_store::set_build(
        &mut scope,
        recipe.id,
        nestbox_control_api::domain::builder::RecipeState::Built,
        Some(tag.as_str()),
        Some(hash.as_str()),
        None,
        None,
    )
    .await
    .expect("state recorded");
    scope.commit().await.expect("commit");

    let sandbox_name = unique("sandbox");
    let sandbox = builder_service::instantiate(&ctx, &who, recipe.id, &sandbox_name, None)
        .await
        .expect("the sandbox is placed");
    assert_eq!(sandbox.name, sandbox_name);
    assert_eq!(sandbox.image, tag);
    assert_eq!(sandbox.node_id, node);

    let recipe_id: Option<Uuid> =
        sqlx::query_scalar("SELECT recipe_id FROM sandboxes WHERE id = $1")
            .bind(sandbox.id)
            .fetch_one(pool)
            .await
            .expect("the link is stored");
    assert_eq!(recipe_id, Some(recipe.id));

    let mounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mounts WHERE sandbox_id = $1")
        .bind(sandbox.id)
        .fetch_one(pool)
        .await
        .expect("count");
    assert_eq!(mounts, 1, "the project root is mounted");

    let agents: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sandbox_agents WHERE sandbox_id = $1 AND instance = 'hermes'",
    )
    .bind(sandbox.id)
    .fetch_one(pool)
    .await
    .expect("count");
    assert_eq!(agents, 1, "the recipe's agent is recorded");

    // The runtime was asked for the same mounts and the same memory: the
    // sandbox is not just a row.
    let specs = runtime.created_specs();
    let spec = specs.last().expect("the runtime was called");
    assert_eq!(spec.image, tag);
    assert_eq!(spec.mounts.len(), 1);
    assert_eq!(spec.mounts[0].target, "/workspace");
    assert!(spec.mounts[0].source.ends_with(&format!("/var/lib/nestbox/persistent/{slug}")));
    assert_eq!(spec.memory_limit, "1073741824b");

    // 32 GiB on a node measured at 8 GiB is refused with the numbers.
    let mut too_big = shell_recipe(&unique("too-big"));
    too_big.resources.memory_limit = "32Gi".into();
    let too_big = builder_service::create(&ctx, &who, too_big)
        .await
        .expect("a draft may ask for anything");
    let big_tag = builder_service::image_tag(&too_big.name, &hash);
    let mut scope = TenantScope::begin(pool, environment).await.expect("scope");
    recipe_store::set_build(
        &mut scope,
        too_big.id,
        nestbox_control_api::domain::builder::RecipeState::Built,
        Some(big_tag.as_str()),
        Some(hash.as_str()),
        None,
        None,
    )
    .await
    .expect("state recorded");
    scope.commit().await.expect("commit");

    let error = builder_service::instantiate(&ctx, &who, too_big.id, &unique("sandbox"), None)
        .await
        .expect_err("32 GiB do not fit an 8 GiB node");
    assert_eq!(error.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error.code, ErrorCode::NODE_INSUFFICIENT_RESOURCES);
    assert!(
        error.details["free_ram_bytes"].is_number(),
        "the refusal carries the numbers: {}",
        error.details
    );
    assert!(error.message.contains("cannot host"), "{}", error.message);
}

/// An instantiated recipe carries a supervisor whose token reports for its row.
///
/// The sandbox is handed an id and a token, not the node token; the token opens
/// exactly one row. A report with the right token moves that row to `running`,
/// and a report with a wrong token is refused without touching it.
#[tokio::test]
async fn an_instantiated_sandbox_reports_through_its_supervisor_token() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment);

    // The node as VM200 reports it: 8 GiB of RAM, no GPU. The supervisor is
    // created on the same path, so the sandbox must have a supervisor row by
    // the time `instantiate` returns.
    let node: Uuid = sqlx::query_scalar("SELECT id FROM nodes WHERE is_local")
        .fetch_one(pool)
        .await
        .expect("the local node is seeded");
    sqlx::query(
        "INSERT INTO hardware_profiles
             (environment_id, node_id, hostname, gpus, total_vram_bytes, cpu_cores, system_ram_bytes)
         VALUES ($1, $2, 'test-node', '[]'::jsonb, 0, 4, $3)",
    )
    .bind(environment)
    .bind(node)
    .bind(8i64 * 1024 * 1024 * 1024)
    .execute(pool)
    .await
    .expect("a measurement");

    let name = unique("supervised");
    let recipe = builder_service::create(&ctx, &who, shell_recipe(&name))
        .await
        .expect("created");
    // Mark it built so instantiation is about placement, not about building.
    let hash = "b".repeat(64);
    let tag = builder_service::image_tag(&name, &hash);
    let mut scope = TenantScope::begin(pool, environment).await.expect("scope");
    recipe_store::set_build(
        &mut scope,
        recipe.id,
        nestbox_control_api::domain::builder::RecipeState::Built,
        Some(tag.as_str()),
        Some(hash.as_str()),
        None,
        None,
    )
    .await
    .expect("build recorded");
    scope.commit().await.expect("commit");

    let sandbox_name = unique("sandbox");
    let sandbox = builder_service::instantiate(&ctx, &who, recipe.id, &sandbox_name, None)
        .await
        .expect("the sandbox is placed");

    // 1. The row exists and belongs to this sandbox.
    let row: (Uuid, Option<Uuid>, String) = sqlx::query_as(
        "SELECT id, sandbox_id, name FROM supervisors WHERE sandbox_id = $1",
    )
    .bind(sandbox.id)
    .fetch_one(pool)
    .await
    .expect("the sandbox has a supervisor");
    assert_eq!(row.1, Some(sandbox.id));
    assert!(row.2.contains(&sandbox_name));

    // 2. The token travels in the environment, the id too, and neither is the
    //    node token.
    let specs = runtime.created_specs();
    let spec = specs.last().expect("the runtime was called");
    let value = |key: &str| {
        spec.environment
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(value("NESTBOX_SUPERVISOR_ID"), Some(row.0.to_string()));
    let token = value("NESTBOX_SUPERVISOR_TOKEN").expect("the token is in the environment");
    assert!(!token.is_empty());

    // 3. The token reports for its own row, and only that row.
    let report = SupervisorReport {
        status: SupervisorStatus::Running,
        version: Some("0.1.0".into()),
        config_dirtiness: 2,
        last_policy_applied_at: None,
        checks: Vec::new(),
        facts: serde_json::json!({"started": true}),
    };
    let updated = gateway_service::record_report(&ctx, &token, row.0, report.clone())
        .await
        .expect("the token reports");
    assert_eq!(updated.status, SupervisorStatus::Running);
    assert_eq!(updated.version.as_deref(), Some("0.1.0"));
    assert_eq!(updated.config_dirtiness, 2);
    assert!(updated.last_seen.is_some());

    let refused = gateway_service::record_report(&ctx, "not-the-token", row.0, report)
        .await
        .expect_err("a wrong token is refused");
    assert_eq!(refused.status, axum::http::StatusCode::UNAUTHORIZED);

    let still: String = sqlx::query_scalar("SELECT status FROM supervisors WHERE id = $1")
        .bind(row.0)
        .fetch_one(pool)
        .await
        .expect("read back");
    assert_eq!(
        still,
        SupervisorStatus::Running.as_str(),
        "the refusal wrote nothing"
    );

    // 4. Settings are stored per supervisor, and a running sandbox receives them.
    let stored = gateway_service::update_sandbox_settings(
        &ctx,
        &who,
        sandbox.id,
        &serde_json::json!({ "self_healing": true, "tools": ["python3"] }),
    )
    .await
    .expect("settings are accepted");
    assert_eq!(stored["settings"]["self_healing"], serde_json::json!(true));
    assert_eq!(stored["delivered"], serde_json::json!(true));

    let read_back = gateway_service::sandbox_settings(&ctx, &who, sandbox.id)
        .await
        .expect("settings are readable");
    assert_eq!(read_back["self_healing"], serde_json::json!(true));

    // 5. An action is queued through the runtime, and the whitelist is enforced.
    let queued = gateway_service::queue_supervisor_action(
        &ctx,
        &who,
        sandbox.id,
        "run_check",
        &serde_json::json!({}),
    )
    .await
    .expect("an action is queued");
    assert_eq!(queued["queued"], serde_json::json!(true));
    assert_eq!(queued["action"], serde_json::json!("run_check"));

    let refused = gateway_service::queue_supervisor_action(
        &ctx,
        &who,
        sandbox.id,
        "rm -rf /",
        &serde_json::json!({}),
    )
    .await
    .expect_err("a free command is not on the whitelist");
    assert_eq!(refused.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);

    // 6. A deleted sandbox takes its supervisor with it: a row that can never
    //    report again is noise in the operator's table.
    nestbox_control_api::services::sandbox_service::delete(&ctx, &who, sandbox.id)
        .await
        .expect("the sandbox is deleted");
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM supervisors WHERE id = $1")
        .bind(row.0)
        .fetch_one(pool)
        .await
        .expect("count");
    assert_eq!(remaining, 0, "the supervisor is removed with its sandbox");
}

/// A recipe write without the write permission is refused before any query.
#[tokio::test]
async fn a_viewer_cannot_write_a_recipe() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let mut who = principal(environment);
    who.role = Role::Viewer;

    let error: ApiError = builder_service::create(&ctx, &who, shell_recipe(&unique("denied")))
        .await
        .expect_err("a viewer cannot create a recipe");
    assert_eq!(error.status, axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_mcp_assignment_is_attached_when_the_recipe_becomes_a_sandbox() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = &pool;
    let environment = environment_id(pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(pool, &runtime);
    let who = principal(environment);

    let node: Uuid = sqlx::query_scalar("SELECT id FROM nodes WHERE is_local")
        .fetch_one(pool)
        .await
        .expect("the local node is seeded");
    sqlx::query(
        "INSERT INTO hardware_profiles
             (environment_id, node_id, hostname, gpus, total_vram_bytes, cpu_cores, system_ram_bytes)
         VALUES ($1, $2, 'test-node', '[]'::jsonb, 0, 4, $3)",
    )
    .bind(environment)
    .bind(node)
    .bind(8i64 * 1024 * 1024 * 1024)
    .execute(pool)
    .await
    .expect("a measurement");

    let name = unique("mcp-assigned");
    let mut input = shell_recipe(&name);
    input.mcp_server_keys = vec!["nestbox-hub".into()];
    let recipe = builder_service::create(&ctx, &who, input)
        .await
        .expect("a recipe may name a registry server");

    // An unknown key is an operator typo, refused when the recipe is saved.
    let mut unknown = shell_recipe(&unique("mcp-unknown"));
    unknown.mcp_server_keys = vec!["not-a-server".into()];
    let error = builder_service::create(&ctx, &who, unknown)
        .await
        .expect_err("an unknown MCP server is refused");
    assert_eq!(error.code, ErrorCode::RECIPE_INVALID);

    // Mark it built so instantiation is about the assignment, not the build.
    let hash = "b".repeat(64);
    let tag = builder_service::image_tag(&name, &hash);
    let mut scope = TenantScope::begin(pool, environment).await.expect("scope");
    recipe_store::set_build(
        &mut scope,
        recipe.id,
        nestbox_control_api::domain::builder::RecipeState::Built,
        Some(tag.as_str()),
        Some(hash.as_str()),
        None,
        None,
    )
    .await
    .expect("state recorded");
    scope.commit().await.expect("commit");

    let sandbox = builder_service::instantiate(&ctx, &who, recipe.id, &unique("mcp-sandbox"), None)
        .await
        .expect("the sandbox starts");
    let attached = mcp_service::sandbox_servers(&ctx, &who, sandbox.id)
        .await
        .expect("the attachments are readable");
    assert!(
        attached.iter().any(|server| server.server_key == "nestbox-hub"),
        "the recipe's MCP server is attached without an extra step"
    );
}
