//! The bulk model install: the plan, the job and the refusals.
//!
//! The suite owns `nestbox_cc_b04`, so it can run next to the other suites:
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_cc_b04?host=/tmp/opencode/pgtest'
//!   cargo test --test model_bulk -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::model_bulk_service;
use nestbox_control_api::services::ServiceContext;

/// The database this suite owns.
const DATABASE: &str = "nestbox_cc_b04";
const ENVIRONMENT: &str = "00000000-0000-0000-0000-000000000002";
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

fn principal() -> Principal {
    Principal {
        user_id: Uuid::parse_str(ADMIN).unwrap_or_else(|_| Uuid::nil()),
        environment_id: Uuid::parse_str(ENVIRONMENT).unwrap_or_else(|_| Uuid::nil()),
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

async fn seed(pool: &PgPool, key: &str, nodes: &[&str]) {
    sqlx::query(
        "INSERT INTO models (key, name, version, parameters, state, artifact_ref, serve_runtime)
         VALUES ($1, $1, '1', '1B', 'not_installed', 'huggingface:acme/tiny/tiny.gguf', 'llama-cpp')
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(key)
    .execute(pool)
    .await
    .expect("insert model");
    for name in nodes {
        sqlx::query(
            "INSERT INTO nodes (name, display_name, environment_id, status, container_gpu)
             VALUES ($1, $1, $2, 'online', true) ON CONFLICT (name) DO NOTHING",
        )
        .bind(name)
        .bind(Uuid::parse_str(ENVIRONMENT).unwrap_or_else(|_| Uuid::nil()))
        .execute(pool)
        .await
        .expect("insert node");
    }
}

#[tokio::test]
async fn two_nodes_make_two_targets_and_one_job() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    seed(&pool, "bulk-a", &["bulk-n1", "bulk-n2"]).await;

    let answer = model_bulk_service::install_bulk(
        &ctx,
        &principal(),
        "bulk-a",
        serde_json::json!({ "nodes": ["bulk-n1", "bulk-n2"] }),
    )
    .await
    .expect("the plan is accepted");

    let targets = answer["targets"].as_array().cloned().unwrap_or_default();
    assert_eq!(targets.len(), 2);
    let planned = targets.iter().filter(|target| target["skipped"].is_null()).count() as i32;
    let job_id: Uuid = answer["job_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .expect("a job id");
    let (kind, total): (String, i32) =
        sqlx::query_as("SELECT kind, total FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("the job row");
    assert_eq!(kind, "model_bulk_install");
    assert_eq!(total, planned, "total counts the targets that are not skipped");
}

#[tokio::test]
async fn a_request_without_exactly_one_selector_is_refused() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let ctx = context(&pool, &Arc::new(FakeRuntime::new()));
    seed(&pool, "bulk-b", &["bulk-n3"]).await;

    let none = model_bulk_service::install_bulk(&ctx, &principal(), "bulk-b", serde_json::json!({})).await;
    assert!(none.is_err());
    let two = model_bulk_service::install_bulk(
        &ctx,
        &principal(),
        "bulk-b",
        serde_json::json!({ "nodes": ["bulk-n3"], "environment_wide": true }),
    )
    .await;
    assert!(two.is_err());
}
