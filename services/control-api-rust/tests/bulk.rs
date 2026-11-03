//! Bulk sandbox actions against a real database (package B03, compiled here, run in Wave 2).
//!
//!   createdb -h /tmp/opencode/pgtest -p 55432 nestbox_cc_b03
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_cc_b03?host=/tmp/opencode/pgtest'
//!   cargo test --test bulk -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use serde_json::json;
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::bulk_service;
use nestbox_control_api::store::postgres::jobs as job_store;
use nestbox_control_api::store::TenantScope;
use nestbox_control_api::services::ServiceContext;

/// The database this suite owns.
const DATABASE: &str = "nestbox_cc_b03";

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

/// A second tenant, so isolation is proven against real rows.
async fn other_environment(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug)
         VALUES ('00000000-0000-0000-0000-000000000001', $1, $1)
         RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("insert environment")
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

#[tokio::test]
async fn a_bulk_stop_is_one_job_with_a_result_per_target() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment_id = default_environment(&pool).await;
    let cluster_id = cluster(&pool, environment_id, "bulk-cluster").await;
    let node_id = node(&pool, "bulk-node").await;
    let first = sandbox(&pool, environment_id, cluster_id, node_id, "bulk-one").await;
    let second = sandbox(&pool, environment_id, cluster_id, node_id, "bulk-two").await;
    let principal = principal(environment_id);
    // SAFETY: the test sets the variable before it starts any thread that reads it.
    unsafe { std::env::set_var("DATABASE_URL", "postgres://unused") };
    let config = Arc::new(nestbox_control_api::config::Config::from_env().expect("a config"));

    let answer = bulk_service::sandboxes(
        &ctx,
        config,
        &principal,
        vec![first, second],
        "stop",
        json!({}),
    )
    .await
    .expect("the job starts");
    assert_eq!(answer["total"], 2);
    let job_id: Uuid = serde_json::from_value(answer["job_id"].clone()).expect("a job id");

    let mut completed = 0;
    for _ in 0..50 {
        let mut scope = TenantScope::begin(&pool, environment_id).await.expect("scope");
        let job = job_store::get(&mut scope, job_id).await.expect("the job exists");
        scope.commit().await.expect("commit");
        if job.finished_at.is_some() {
            completed = job.completed;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(completed, 2);
}
