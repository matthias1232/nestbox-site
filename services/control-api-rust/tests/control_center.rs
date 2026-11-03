//! The Control Center services together against a real PostgreSQL (package B13, contracts C3-C13).
//!
//! One test per feature: tree, search, bulk actions, clusters, node summary, alerts, mounts,
//! policy preview, quick recording and permissions. The services are called directly (not over
//! HTTP). The suite owns `nestbox_cc_all`:
//!
//!   createdb -h /tmp/opencode/pgtest -p 55432 nestbox_cc_all
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_cc_all?host=/tmp/opencode/pgtest'
//!   cargo test --test control_center -- --test-threads=1
//!
//! Every test creates its own rows under unique names, so the tests do not depend on order.
//! Without `TEST_DATABASE_URL` each test prints a line and passes.

use std::os::unix::fs::PermissionsExt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::inventory::{TreeNode, TreeView};
use nestbox_control_api::domain::mounts::MountSpec;
use nestbox_control_api::domain::permissions::ScopeType;
use nestbox_control_api::domain::search::{SearchKind, SearchParams};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::telemetry_service::{self, ProxyBatch};
use nestbox_control_api::services::{
    alert_service, bulk_service, inventory_tree_service, model_cluster_service, mount_service,
    node_ops_service, permission_service, policy_preview_service, policy_service,
    quick_control_service, search_service, ServiceContext,
};

/// The database this suite owns.
const DATABASE: &str = "nestbox_cc_all";

/// The seeded administrator of the default data center.
const ADMIN: &str = "00000000-0000-0000-0000-000000000005";

/// The seeded platform organisation.
const PLATFORM_ORG: &str = "00000000-0000-0000-0000-000000000001";

static READY: OnceCell<bool> = OnceCell::const_new();

/// Create the suite's database once per process, then migrate it.
async fn ready() -> bool {
    *READY
        .get_or_init(|| async {
            let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
                return false;
            };
            let Ok(options) = PgConnectOptions::from_str(&url) else {
                return false;
            };
            let Ok(maintenance) = PgPool::connect_with(options.clone().database("postgres")).await
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
                let pool = PgPool::connect_with(options.database(DATABASE)).await?;
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

/// A pool as the database owner (seeding, assertions).
async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?;
    PgPool::connect_with(options.database(DATABASE)).await.ok()
}

/// A pool that runs as the application role, so row-level security applies.
async fn app_pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?.database(DATABASE);
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE nestbox_app").execute(connection).await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .ok()
}

fn principal(user_id: Uuid, environment_id: Uuid, role: Role) -> Principal {
    Principal {
        user_id,
        environment_id,
        role,
        display_name: "Test".into(),
        via: CredentialSource::Token,
    }
}

/// The seeded administrator acting in `environment_id`.
fn admin(environment_id: Uuid) -> Principal {
    principal(Uuid::parse_str(ADMIN).unwrap_or_default(), environment_id, Role::Admin)
}

fn context(pool: &PgPool, secret_dir: std::path::PathBuf) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        Arc::new(FakeRuntime::new()),
        Arc::new(nestbox_control_api::events::bus::LocalBus::new()),
        secret_dir,
    )
}

/// A unique, short, slug-safe tag.
fn tag() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_string()
}

async fn default_environment(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .unwrap_or_default()
}

async fn site_of(pool: &PgPool, environment_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM sites WHERE environment_id = $1 ORDER BY created_at LIMIT 1")
        .bind(environment_id)
        .fetch_one(pool)
        .await
        .unwrap_or_default()
}

async fn cluster_of(pool: &PgPool, environment_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM clusters WHERE environment_id = $1 ORDER BY id LIMIT 1")
        .bind(environment_id)
        .fetch_one(pool)
        .await
        .unwrap_or_default()
}

/// A node in the data center, placed on its first site.
async fn node(pool: &PgPool, environment_id: Uuid, name: &str) -> Uuid {
    let site = site_of(pool, environment_id).await;
    sqlx::query_scalar(
        "INSERT INTO nodes (name, display_name, environment_id, site_id)
         VALUES ($1, $1, $2, $3) RETURNING id",
    )
    .bind(name)
    .bind(environment_id)
    .bind(site)
    .fetch_one(pool)
    .await
    .unwrap_or_default()
}

/// A running sandbox on a node, optionally owned by a user.
async fn sandbox(
    pool: &PgPool,
    environment_id: Uuid,
    node_id: Uuid,
    name: &str,
    owner: Option<Uuid>,
) -> Uuid {
    let cluster = cluster_of(pool, environment_id).await;
    sqlx::query_scalar(
        "INSERT INTO sandboxes
            (environment_id, cluster_id, node_id, name, pack_key, image, spec, status, owner_user_id)
         VALUES ($1, $2, $3, $4, 'ubuntu-dev', 'ghcr.io/nestbox/ubuntu-dev@sha256:abc', '{}', 'running', $5)
         RETURNING id",
    )
    .bind(environment_id)
    .bind(cluster)
    .bind(node_id)
    .bind(name)
    .bind(owner)
    .fetch_one(pool)
    .await
    .unwrap_or_default()
}

/// A user with a role in one data center.
async fn user_with_scope(pool: &PgPool, environment_id: Uuid, role: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (organization_id, email, display_name, password_hash, role)
         VALUES ($1::uuid, $2, 'Test user', 'x', $3) RETURNING id",
    )
    .bind(PLATFORM_ORG)
    .bind(format!("cc-{}@example.test", Uuid::new_v4()))
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap_or_default();
    let _ = sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(environment_id)
        .bind(role)
        .execute(pool)
        .await;
    id
}

/// Find a node of the tree by kind and id, depth first.
fn find<'a>(node: &'a TreeNode, kind: &str, id: &str) -> Option<&'a TreeNode> {
    if node.kind == kind && node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, kind, id))
}

/// Whether `parent` has a direct child of `kind` with `id`.
fn has_child(parent: &TreeNode, kind: &str, id: &str) -> bool {
    parent.children.iter().any(|child| child.kind == kind && child.id == id)
}

/// A node with a token file the proxy can authenticate with.
struct ProxyNode {
    /// Keeps the secret directory alive.
    dir: tempfile::TempDir,
    name: String,
    token: String,
}

async fn proxy_node(pool: &PgPool) -> ProxyNode {
    let dir = tempfile::tempdir().expect("a temporary secret directory");
    let token = format!("node-token-{}", Uuid::new_v4().simple());
    let path = dir.path().join("node.token");
    std::fs::write(&path, &token).expect("write the token file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let name = format!("cc-proxy-{}", tag());
    sqlx::query("INSERT INTO nodes (name, display_name, node_token_ref) VALUES ($1, $1, $2)")
        .bind(&name)
        .bind(format!("file://{}", path.display()))
        .execute(pool)
        .await
        .expect("create the proxy node");
    ProxyNode { dir, name, token }
}

/// Send one proxy traffic line for a sandbox through the real ingest path.
async fn ingest(
    ctx: &ServiceContext,
    proxy: &ProxyNode,
    environment_id: Uuid,
    sandbox_id: Uuid,
    host: &str,
    decision: &str,
) {
    let batch: ProxyBatch = serde_json::from_value(json!({
        "environment_id": environment_id,
        "traffic": [{
            "ts": chrono::Utc::now().to_rfc3339(),
            "sandbox_id": sandbox_id, "host": host, "port": 443,
            "method": "GET", "path": "/", "decision": decision
        }],
        "dns": []
    }))
    .expect("a batch");
    telemetry_service::ingest_batch(ctx, &proxy.name, &proxy.token, batch)
        .await
        .expect("the batch is accepted");
}

/// An installed model on a node with a reachable provider profile.
async fn install(pool: &PgPool, environment_id: Uuid, key: &str, node_id: Uuid, endpoint: &str) -> Uuid {
    sqlx::query(
        "INSERT INTO models (key, name, version, parameters, state)
         VALUES ($1, $1, '1', '1B', 'not_installed') ON CONFLICT (key) DO NOTHING",
    )
    .bind(key)
    .execute(pool)
    .await
    .expect("insert model");
    let install: Uuid = sqlx::query_scalar(
        "INSERT INTO model_installs (environment_id, model_key, node_id, state)
         VALUES ($1, $2, $3, 'installed') RETURNING id",
    )
    .bind(environment_id)
    .bind(key)
    .bind(node_id)
    .fetch_one(pool)
    .await
    .expect("insert install");
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO provider_profiles
            (environment_id, name, kind, endpoint, model, status, node_id, model_install_id)
         VALUES ($1, $2, 'local_openai_compatible', $3, $4, 'reachable', $5, $6) RETURNING id",
    )
    .bind(environment_id)
    .bind(format!("local/{key}@{node_id}"))
    .bind(endpoint)
    .bind(key)
    .bind(node_id)
    .bind(install)
    .fetch_one(pool)
    .await
    .expect("insert profile");
    sqlx::query("UPDATE model_installs SET provider_profile_id = $2 WHERE id = $1")
        .bind(install)
        .bind(profile)
        .execute(pool)
        .await
        .expect("link profile");
    install
}

#[tokio::test]
async fn tree_shows_a_sandbox_under_its_node() {
    let (Some(pool), Some(app)) = (pool().await, app_pool().await) else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let environment = default_environment(&pool).await;
    let site = site_of(&pool, environment).await;
    let suffix = tag();
    let node_id = node(&pool, environment, &format!("cc-tree-{suffix}")).await;
    // An owned sandbox: one without an owner is shared and visible to every member by design.
    let owner_id = user_with_scope(&pool, environment, "member").await;
    let sandbox_id =
        sandbox(&pool, environment, node_id, &format!("cc-tree-box-{suffix}"), Some(owner_id)).await;

    let tree = inventory_tree_service::tree(&ctx, &admin(environment), TreeView::Infrastructure)
        .await
        .expect("the tree");
    let dc = find(&tree.root, "data_center", &environment.to_string()).expect("the data center");
    let site_node = find(dc, "site", &site.to_string()).expect("the site");
    assert!(has_child(dc, "site", &site.to_string()));
    let node_node = find(site_node, "node", &node_id.to_string()).expect("the node");
    assert!(has_child(site_node, "node", &node_id.to_string()));
    assert!(has_child(node_node, "sandbox", &sandbox_id.to_string()), "the sandbox sits under its node");

    // A member who owns nothing sees the node, without a budget, and no sandbox.
    let member_id = user_with_scope(&pool, environment, "member").await;
    let member_ctx = context(&app, std::env::temp_dir());
    let member = principal(member_id, environment, Role::Member);
    let seen = inventory_tree_service::tree(&member_ctx, &member, TreeView::Infrastructure)
        .await
        .expect("the member's tree");
    let seen_node = find(&seen.root, "node", &node_id.to_string()).expect("the member sees the node");
    assert!(seen_node.children.is_empty(), "no sandbox of someone else");
    assert!(find(&seen.root, "sandbox", &sandbox_id.to_string()).is_none());
    assert!(seen_node.extra.get("budget").map_or(true, |budget| budget.is_null()));
}

#[tokio::test]
async fn search_finds_a_sandbox_by_prefix() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let environment = default_environment(&pool).await;
    let suffix = tag();
    let node_id = node(&pool, environment, &format!("cc-search-node-{suffix}")).await;
    let name = format!("ccfind-{suffix}-box");
    let sandbox_id = sandbox(&pool, environment, node_id, &name, None).await;

    let hits = search_service::search(
        &ctx,
        &admin(environment),
        &SearchParams {
            q: format!("ccfind-{suffix}"),
            kinds: vec![SearchKind::Sandbox],
            environment: None,
            status: None,
            limit: 20,
        },
    )
    .await
    .expect("search");
    let hit = hits.iter().find(|hit| hit.id == sandbox_id.to_string()).expect("the sandbox is found");
    assert_eq!(hit.kind, "sandbox");
    assert!(hit.path.starts_with("/sandboxes/"));
}

#[tokio::test]
async fn bulk_stop_records_one_result_per_target() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let environment = default_environment(&pool).await;
    let suffix = tag();
    let node_id = node(&pool, environment, &format!("cc-bulk-node-{suffix}")).await;
    let first = sandbox(&pool, environment, node_id, &format!("cc-bulk-a-{suffix}"), None).await;
    let second = sandbox(&pool, environment, node_id, &format!("cc-bulk-b-{suffix}"), None).await;

    // SAFETY: the test sets the variable before it starts any thread that reads it.
    unsafe { std::env::set_var("DATABASE_URL", "postgres://unused") };
    let config = std::sync::Arc::new(nestbox_control_api::config::Config::from_env().expect("a config"));
    let answer = bulk_service::sandboxes(&ctx, config, &admin(environment), vec![first, second], "stop", json!({}))
        .await
        .expect("a bulk job");
    assert_eq!(answer["total"], 2);
    let job_id: Uuid = serde_json::from_value(answer["job_id"].clone()).expect("a job id");

    let mut finished = false;
    for _ in 0..40 {
        let done: Option<chrono::DateTime<chrono::Utc>> =
            sqlx::query_scalar("SELECT finished_at FROM jobs WHERE id = $1")
                .bind(job_id)
                .fetch_one(&pool)
                .await
                .expect("the job row");
        if done.is_some() {
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(finished, "the job finishes");

    let (kind, completed, failed): (String, i32, i32) =
        sqlx::query_as("SELECT kind, completed, failed FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("the job row");
    assert_eq!(kind, "sandbox_bulk_action");
    assert_eq!(completed + failed, 2, "every target is counted once");
    for target in [first, second] {
        let lines: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM job_logs
              WHERE job_id = $1 AND (message || coalesce(detail::text, '')) LIKE '%' || $2 || '%'",
        )
        .bind(job_id)
        .bind(target.to_string())
        .fetch_one(&pool)
        .await
        .expect("count log lines");
        assert!(lines >= 1, "a result line names the sandbox {target}");
    }
}

#[tokio::test]
async fn cluster_round_trip() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let environment = default_environment(&pool).await;
    let who = admin(environment);
    let suffix = tag();
    let key = format!("cc-model-{suffix}");
    let first = node(&pool, environment, &format!("cc-gpu-{suffix}-1")).await;
    let second = node(&pool, environment, &format!("cc-gpu-{suffix}-2")).await;
    let one = install(&pool, environment, &key, first, "http://10.77.3.4:8080").await;
    let two = install(&pool, environment, &key, second, "http://10.77.3.5:8080").await;

    let created = model_cluster_service::create(
        &ctx,
        &who,
        json!({ "name": format!("cc-cluster-{suffix}"), "model_key": key, "install_ids": [one, two] }),
    )
    .await
    .expect("create the cluster");
    let id = Uuid::parse_str(created["id"].as_str().unwrap_or_default()).expect("a cluster id");
    assert_eq!(created["members"].as_array().map(Vec::len), Some(2));

    let profile_id =
        Uuid::parse_str(created["provider_profile_id"].as_str().unwrap_or_default()).expect("a profile id");
    let (endpoint, kind): (String, String) =
        sqlx::query_as("SELECT endpoint, kind FROM provider_profiles WHERE id = $1")
            .bind(profile_id)
            .fetch_one(&pool)
            .await
            .expect("the provider profile exists");
    assert!(endpoint.ends_with(&format!("/c/{id}")));
    assert_eq!(kind, "local_openai_compatible");

    model_cluster_service::delete(&ctx, &who, id).await.expect("delete the cluster");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_profiles WHERE id = $1")
        .bind(profile_id)
        .fetch_one(&pool)
        .await
        .expect("count profiles");
    assert_eq!(left, 0, "deleting the cluster deletes its provider profile");
}

#[tokio::test]
async fn node_summary_counts_sandboxes() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let environment = default_environment(&pool).await;
    let suffix = tag();
    let node_id = node(&pool, environment, &format!("cc-summary-{suffix}")).await;
    sandbox(&pool, environment, node_id, &format!("cc-summary-box-{suffix}"), None).await;

    let summary = node_ops_service::summary(&ctx, &admin(environment), node_id)
        .await
        .expect("a summary");
    assert_eq!(summary["sandboxes"]["total"], 1);
    assert_eq!(summary["sandboxes"]["running"], 1);
    assert_eq!(summary["empty"], false);
}

#[tokio::test]
async fn alerts_list_a_refused_destination_and_hide_an_answered_one() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let proxy = proxy_node(&pool).await;
    let ctx = context(&pool, proxy.dir.path().to_path_buf());
    let environment = default_environment(&pool).await;
    let who = admin(environment);
    let suffix = tag();
    let node_id = node(&pool, environment, &format!("cc-alert-node-{suffix}")).await;
    let sandbox_id = sandbox(&pool, environment, node_id, &format!("cc-alert-box-{suffix}"), None).await;
    let host = format!("blocked-{suffix}.example.net");

    ingest(&ctx, &proxy, environment, sandbox_id, &host, "deny").await;
    let open = alert_service::list(&ctx, &who, 15, Some(sandbox_id)).await.expect("alerts");
    assert_eq!(open.len(), 1, "one refused destination is one alert");
    assert_eq!(open[0].host, host);
    assert_eq!(open[0].sandbox_id, sandbox_id);
    assert_eq!(open[0].count, 1);

    // The tree's badge counts the same alerts as the list, not every refused event.
    let tree = inventory_tree_service::tree(&ctx, &who, TreeView::Infrastructure)
        .await
        .expect("the tree");
    let box_node = find(&tree.root, "sandbox", &sandbox_id.to_string()).expect("the sandbox in the tree");
    assert_eq!(
        box_node.extra.get("alerts").and_then(serde_json::Value::as_i64),
        Some(1),
        "the tree counts the open alert"
    );

    // The window is created after the last refusal, which is what answers the alert.
    tokio::time::sleep(Duration::from_millis(150)).await;
    policy_service::open_window(&ctx, &who, Some(sandbox_id), Some(15), "bypass", "all", "alert test")
        .await
        .expect("open a window");
    let answered = alert_service::list(&ctx, &who, 15, Some(sandbox_id)).await.expect("alerts");
    assert!(answered.is_empty(), "an answered alert is not listed");
    let answered_tree = inventory_tree_service::tree(&ctx, &who, TreeView::Infrastructure)
        .await
        .expect("the tree");
    let answered_box =
        find(&answered_tree.root, "sandbox", &sandbox_id.to_string()).expect("the sandbox in the tree");
    assert_eq!(
        answered_box.extra.get("alerts").and_then(serde_json::Value::as_i64),
        Some(0),
        "an answered alert is not counted in the tree"
    );
}

#[tokio::test]
async fn mounts_set_and_read_back() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let environment = default_environment(&pool).await;
    let who = admin(environment);
    let suffix = tag();
    let node_id = node(&pool, environment, &format!("cc-mount-node-{suffix}")).await;
    let sandbox_id = sandbox(&pool, environment, node_id, &format!("cc-mount-box-{suffix}"), None).await;
    let root_id: Uuid = sqlx::query_scalar(
        "INSERT INTO persistent_roots (environment_id, node_id, kind, slug, host_path)
         VALUES ($1, $2, 'project', $3, $4) RETURNING id",
    )
    .bind(environment)
    .bind(node_id)
    .bind(format!("cc-{suffix}"))
    .bind(format!("/var/lib/nestbox/persistent/test/project--cc-{suffix}"))
    .fetch_one(&pool)
    .await
    .expect("insert root");

    let before = mount_service::effective(&ctx, &who, sandbox_id).await.expect("effective");
    assert!(before.roots.is_empty());
    assert_eq!(before.available_roots.len(), 1, "the root is offered");

    let spec = MountSpec { root_id, mount_path: "/work/portal".into(), read_only: false };
    let mounted = mount_service::set(&ctx, &who, sandbox_id, vec![spec], "later")
        .await
        .expect("mount");
    assert_eq!(mounted.roots.len(), 1);
    assert_eq!(mounted.roots[0]["mount_path"], json!("/work/portal"));

    let again = mount_service::effective(&ctx, &who, sandbox_id).await.expect("read back");
    assert_eq!(again.roots.len(), 1);
    assert_eq!(again.roots[0]["mount_path"], json!("/work/portal"));
    assert_eq!(again.roots[0]["read_only"], json!(false));
}

#[tokio::test]
async fn preview_splits_hot_and_cold() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let who = admin(default_environment(&pool).await);
    let spec = json!({
        "version": 1,
        "filesystem_policy": { "read_only": ["/etc"], "read_write": ["/home/nestbox"] },
        "network_policies": {
            "github": { "name": "github", "endpoints": [{ "host": "github.com", "port": 443, "protocol": "rest" }] }
        }
    });
    let answer = policy_preview_service::preview(&ctx, &who, json!({ "spec": spec, "sandbox_id": null }))
        .await
        .expect("preview");
    assert_eq!(answer["valid"], true);
    assert_eq!(answer["hot"], json!(["network_policies"]));
    assert_eq!(answer["needs_recreate"], json!(["filesystem_policy"]));
    assert!(!answer["compiled_yaml"].as_str().unwrap_or_default().is_empty());
}

#[tokio::test]
async fn quick_recording_off_stores_nothing() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let proxy = proxy_node(&pool).await;
    let ctx = context(&pool, proxy.dir.path().to_path_buf());
    let environment = default_environment(&pool).await;
    let who = admin(environment);
    let suffix = tag();
    let node_id = node(&pool, environment, &format!("cc-quick-node-{suffix}")).await;
    let sandbox_id = sandbox(&pool, environment, node_id, &format!("cc-quick-box-{suffix}"), None).await;
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM telemetry_events WHERE sandbox_id = $1")
            .bind(sandbox_id)
            .fetch_one(&pool)
            .await
            .expect("count")
    };

    quick_control_service::set(&ctx, &who, sandbox_id, "recording", json!("off"), Some(60))
        .await
        .expect("recording off");
    ingest(&ctx, &proxy, environment, sandbox_id, "muted.example.net", "allow").await;
    assert_eq!(count().await, 0, "recording off stores nothing");

    quick_control_service::set(&ctx, &who, sandbox_id, "recording", json!("full"), None)
        .await
        .expect("recording full");
    ingest(&ctx, &proxy, environment, sandbox_id, "open.example.net", "allow").await;
    assert_eq!(count().await, 1, "recording full stores the event again");
}

#[tokio::test]
async fn permissions_list_direct_and_org_reach() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let default = default_environment(&pool).await;
    let who = admin(default);
    let suffix = tag();

    // A customer below the platform organisation, with its own data center.
    let organization: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (name, slug, parent_id, kind)
         VALUES ($1, $2, $3::uuid, 'customer') RETURNING id",
    )
    .bind(format!("Customer {suffix}"))
    .bind(format!("cc-customer-{suffix}"))
    .bind(PLATFORM_ORG)
    .fetch_one(&pool)
    .await
    .expect("insert the customer");
    let customer_env: Uuid = sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug) VALUES ($1, $2, 'main') RETURNING id",
    )
    .bind(organization)
    .bind(format!("Customer {suffix} DC"))
    .fetch_one(&pool)
    .await
    .expect("insert the customer's data center");
    let viewer = user_with_scope(&pool, customer_env, "viewer").await;

    let listed = permission_service::list(&ctx, &who, ScopeType::Environment, customer_env)
        .await
        .expect("list");
    let items = listed["items"].as_array().expect("items");
    let platform_admin = items
        .iter()
        .find(|item| item["user_id"] == json!(ADMIN))
        .expect("the platform administrator reaches the customer's data center");
    assert_eq!(platform_admin["via"], "organization");
    let direct = items
        .iter()
        .find(|item| item["user_id"] == json!(viewer))
        .expect("the viewer is listed");
    assert_eq!(direct["via"], "direct");
    assert_eq!(direct["role"], "viewer");
}

#[tokio::test]
async fn tree_keeps_a_node_without_a_site() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, std::env::temp_dir());
    let environment = default_environment(&pool).await;
    let suffix = tag();
    let node_id = node(&pool, environment, &format!("cc-nosite-{suffix}")).await;
    sqlx::query("UPDATE nodes SET site_id = NULL WHERE id = $1")
        .bind(node_id)
        .execute(&pool)
        .await
        .expect("detach the node from its site");
    let tree = inventory_tree_service::tree(&ctx, &admin(environment), TreeView::Infrastructure)
        .await
        .expect("the tree");
    assert!(
        find(&tree.root, "node", &node_id.to_string()).is_some(),
        "a node without a site still shows in the tree"
    );
}
