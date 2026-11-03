//! Effective mounts against a real database (package B09, contract C10).
//!
//! A root on the sandbox's node is mounted, switched to read-only and removed through
//! `mount_service::set`; `effective` must show each state. The suite owns `nestbox_cc_b09`:
//!
//!   createdb -h /tmp/opencode/pgtest -p 55432 nestbox_cc_b09
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_cc_b09?host=/tmp/opencode/pgtest'
//!   cargo test --test mounts -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::domain::mounts::MountSpec;
use nestbox_control_api::services::mount_service;

/// The database this suite owns.
const DATABASE: &str = "nestbox_cc_b09";

/// The seeded administrator, which every audit trail foreign key accepts.
const ADMIN: &str = "00000000-0000-0000-0000-000000000005";

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

async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?;
    sqlx::PgPool::connect_with(options.database(DATABASE)).await.ok()
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

fn context(pool: &PgPool, runtime: &Arc<FakeRuntime>) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        runtime.clone(),
        Arc::new(nestbox_control_api::events::bus::LocalBus::new()),
        std::path::PathBuf::from("/tmp/opencode"),
    )
}

async fn default_environment(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .expect("the default environment is seeded")
}

async fn cluster(pool: &PgPool, environment_id: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO clusters (environment_id, name, slug) VALUES ($1, $2, $2) RETURNING id",
    )
    .bind(environment_id)
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("insert cluster")
}

async fn node(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO nodes (name, display_name) VALUES ($1, $1) RETURNING id")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("insert node")
}

async fn sandbox(
    pool: &PgPool,
    environment_id: Uuid,
    cluster_id: Uuid,
    node_id: Uuid,
    name: &str,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO sandboxes
            (environment_id, cluster_id, node_id, name, pack_key, image, spec, status)
         VALUES ($1, $2, $3, $4, 'ubuntu-dev', 'ghcr.io/nestbox/ubuntu-dev@sha256:abc', '{}', 'running')
         RETURNING id",
    )
    .bind(environment_id)
    .bind(cluster_id)
    .bind(node_id)
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("insert sandbox")
}

async fn root(pool: &PgPool, environment_id: Uuid, node_id: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO persistent_roots (environment_id, node_id, kind, slug, host_path)
         VALUES ($1, $2, 'project', $3, $4) RETURNING id",
    )
    .bind(environment_id)
    .bind(node_id)
    .bind(slug)
    .bind(format!("/var/lib/nestbox/persistent/test/project--{slug}"))
    .fetch_one(pool)
    .await
    .expect("insert root")
}

fn spec(root_id: Uuid, path: &str, read_only: bool) -> MountSpec {
    MountSpec { root_id, mount_path: path.into(), read_only }
}

#[tokio::test]
async fn a_root_is_mounted_changed_and_removed() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let who = principal(environment);
    let cluster = cluster(&pool, environment, "b09-cluster").await;
    let node_id = node(&pool, "b09-node").await;
    let sandbox_id = sandbox(&pool, environment, cluster, node_id, "b09-box").await;
    let root_id = root(&pool, environment, node_id, "b09-portal").await;

    let before = mount_service::effective(&ctx, &who, sandbox_id).await.expect("effective");
    assert!(before.roots.is_empty());
    assert_eq!(before.available_roots.len(), 1, "the root is offered");
    assert_eq!(before.filesystem["source"], serde_json::json!("runner_default"));

    let mounted = mount_service::set(&ctx, &who, sandbox_id, vec![spec(root_id, "/work/portal", false)], "later")
        .await
        .expect("mount");
    assert_eq!(mounted.roots.len(), 1);
    assert_eq!(mounted.roots[0]["mount_path"], serde_json::json!("/work/portal"));
    assert_eq!(mounted.roots[0]["read_only"], serde_json::json!(false));
    assert!(mounted.available_roots.is_empty());

    let read_only = mount_service::set(&ctx, &who, sandbox_id, vec![spec(root_id, "/work/portal", true)], "later")
        .await
        .expect("change");
    assert_eq!(read_only.roots[0]["read_only"], serde_json::json!(true));

    let gone = mount_service::set(&ctx, &who, sandbox_id, vec![], "later").await.expect("remove");
    assert!(gone.roots.is_empty());
}

#[tokio::test]
async fn a_root_on_another_node_is_refused() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let who = principal(environment);
    let cluster = cluster(&pool, environment, "b09-wrong").await;
    let home = node(&pool, "b09-home").await;
    let other = node(&pool, "b09-other").await;
    let sandbox_id = sandbox(&pool, environment, cluster, home, "b09-wrong-box").await;
    let root_id = root(&pool, environment, other, "b09-far").await;

    let error = mount_service::set(&ctx, &who, sandbox_id, vec![spec(root_id, "/work/far", false)], "later")
        .await
        .expect_err("the root is on another node");
    assert_eq!(error.code.0, "mount.wrong_node");
}
