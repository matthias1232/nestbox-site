//! Node summary and drain against a real database (package B07).
//!
//! The suite owns `nestbox_cc_b07`, so it can run next to the other suites:
//!
//!   createdb -h /tmp/opencode/pgtest -p 55432 nestbox_cc_b07
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_cc_b07?host=/tmp/opencode/pgtest'
//!   cargo test --test node_ops -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::config::{Config, DeployMode};
use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::node_ops_service;
use nestbox_control_api::services::ServiceContext;

/// The database this suite owns.
const DATABASE: &str = "nestbox_cc_b07";

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

/// A configuration with no overlay: drain never reaches the parts that need one in these tests.
fn config() -> Config {
    Config {
        database_url: "postgres://unused".into(),
        listen_addr: "127.0.0.1:0".into(),
        secret_dir: std::env::temp_dir(),
        session_cookie_secure: false,
        session_idle_seconds: 3600,
        session_max_seconds: 7200,
        allowed_origins: Vec::new(),
        runner_url: "http://sandbox-runner:9000".into(),
        runtime_network: "nestbox-runtime".into(),
        bootstrap_admin: false,
        body_limit_bytes: 1_048_576,
        rate_limit_per_minute: 600,
        terminal_idle_seconds: 1800,
        deploy_mode: DeployMode::SelfHosted,
        public_url: "http://nestbox.test".into(),
        oidc: Vec::new(),
        overlay: nestbox_control_api::config::OverlayConfig {
            dir: Default::default(),
            endpoint: String::new(),
            hub_ip: "10.77.0.1".into(),
            registry: "10.77.0.1:5000".into(),
            control_url: "http://10.77.0.1:8080".into(),
        },
    }
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

async fn local_node(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM nodes WHERE is_local ORDER BY created_at LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("the local node is seeded")
}

#[tokio::test]
async fn the_summary_of_an_empty_local_node_says_so() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let node = local_node(&pool).await;

    let summary = node_ops_service::summary(&ctx, &principal(environment), node)
        .await
        .expect("a summary");
    assert_eq!(summary["sandboxes"]["total"], 0);
    assert_eq!(summary["empty"], true);
    assert_eq!(summary["images"]["present"], 0);
    // No router in this context, so the load is unknown rather than invented.
    assert!(summary["load"].is_null());
}

#[tokio::test]
async fn draining_an_empty_node_finishes_with_nothing_to_do() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let node = local_node(&pool).await;
    let config = config();

    let answer = node_ops_service::drain(
        &ctx,
        &config,
        &principal(environment),
        node,
        json!({ "to_node": null, "stop_only": true }),
    )
    .await
    .expect("a drain job");
    assert_eq!(answer["total"], 0);
    let job_id: Uuid = serde_json::from_value(answer["job_id"].clone()).expect("a job id");

    let mut status = String::new();
    for _ in 0..40 {
        status = sqlx::query_scalar("SELECT status FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_one(&pool)
            .await
            .expect("the job row");
        if status == "succeeded" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert_eq!(status, "succeeded");
    let total: i32 = sqlx::query_scalar("SELECT total FROM jobs WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await
        .expect("the job row");
    assert_eq!(total, 0);
}

#[tokio::test]
async fn a_drain_onto_the_same_node_is_refused() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let node = local_node(&pool).await;
    let name: String = sqlx::query_scalar("SELECT name FROM nodes WHERE id = $1")
        .bind(node)
        .fetch_one(&pool)
        .await
        .expect("the node name");
    let config = config();

    let refused = node_ops_service::drain(
        &ctx,
        &config,
        &principal(environment),
        node,
        json!({ "to_node": name, "stop_only": false }),
    )
    .await;
    assert!(refused.is_err());
}
