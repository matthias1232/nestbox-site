//! The move preflight against a real database.
//!
//! These prove what a pure function cannot: that the answer before anything stops is the same
//! survey that would start the move, and that it names the reason instead of failing silently. The
//! two cases that need no runner are covered here: the sandbox's own node, and a node that does not
//! exist. Everything else in a move needs a node that copies folders.
//!
//! The suite owns `nestbox_wb_b21`, so it can run next to the other suites:
//!
//!   createdb -h /tmp/opencode/pgtest -p 55432 nestbox_wb_b21
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_wb_b21?host=/tmp/opencode/pgtest'
//!   cargo test --test moves_preflight -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::services::move_service;

/// The database this suite owns.
const DATABASE: &str = "nestbox_wb_b21";

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

/// The blocker codes of a preflight answer.
fn codes(answer: &serde_json::Value) -> Vec<String> {
    answer["blockers"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|blocker| blocker["code"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_move_to_the_sandboxs_own_node_is_blocked() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let who = principal(environment);
    let cluster = cluster(&pool, environment, "b21-same").await;
    let node_id = node(&pool, "b21-same-node").await;
    let sandbox_id = sandbox(&pool, environment, cluster, node_id, "b21-same-box").await;

    let answer = move_service::preflight(&ctx, &who, sandbox_id, "b21-same-node")
        .await
        .expect("the preflight answers");

    assert_eq!(answer["ok"], serde_json::json!(false));
    assert!(
        codes(&answer).contains(&"same_node".to_string()),
        "the own node is named as the blocker: {answer}"
    );
    // What the move carries is part of the answer, so nothing has to be discovered mid-move.
    assert_eq!(answer["carries"].as_array().map(Vec::len), Some(8));
    assert_eq!(answer["target"]["name"], serde_json::json!("b21-same-node"));
    // A running sandbox loses its running sessions, and that is said before anything stops.
    let warnings = answer["warnings"].as_array().cloned().unwrap_or_default();
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .is_some_and(|text| text.contains("--continue"))),
        "the stopped sessions are announced: {answer}"
    );
}

#[tokio::test]
async fn a_move_to_an_unknown_node_names_it() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let who = principal(environment);
    let cluster = cluster(&pool, environment, "b21-unknown").await;
    let node_id = node(&pool, "b21-unknown-home").await;
    let sandbox_id = sandbox(&pool, environment, cluster, node_id, "b21-unknown-box").await;

    let answer = move_service::preflight(&ctx, &who, sandbox_id, "b21-nowhere")
        .await
        .expect("the preflight answers");

    assert_eq!(answer["ok"], serde_json::json!(false));
    assert!(
        codes(&answer).contains(&"target_unknown".to_string()),
        "an unknown node is named: {answer}"
    );
    // Nothing is known about a node that does not exist, and the answer does not invent it.
    assert_eq!(answer["target"]["online"], serde_json::json!(false));
    assert_eq!(answer["target"]["free_memory_bytes"], serde_json::Value::Null);
}
