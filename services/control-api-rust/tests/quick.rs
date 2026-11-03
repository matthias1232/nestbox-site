//! Quick controls against a real database (package B11). Compiled by the worker,
//! run by the coordinator: `recording off` stores nothing, `recording full` stores again.
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_cc_b11?host=/tmp/opencode/pgtest'
//!   cargo test --test quick -- --test-threads=1

use std::os::unix::fs::PermissionsExt;
use std::str::FromStr;
use std::sync::Arc;

use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::services::telemetry_service::{self, ProxyBatch};

/// The database this suite owns.
const DATABASE: &str = "nestbox_cc_b11";

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
            let Ok(admin) = PgPool::connect_with(options.clone().database("postgres")).await else {
                return false;
            };
            let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {DATABASE} WITH (FORCE)"))
                .execute(&admin)
                .await;
            if sqlx::query(&format!("CREATE DATABASE {DATABASE}"))
                .execute(&admin)
                .await
                .is_err()
            {
                return false;
            }
            let Ok(pool) = PgPool::connect_with(options.database(DATABASE)).await else {
                return false;
            };
            sqlx::migrate!("./migrations").run(&pool).await.is_ok()
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
    PgPool::connect_with(options.database(DATABASE)).await.ok()
}

#[tokio::test]
async fn recording_off_stores_nothing_and_full_stores_again() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = tempfile::tempdir().expect("a temporary secret directory");
    let token = format!("node-token-{}", Uuid::new_v4().simple());
    let path = dir.path().join("node.token");
    std::fs::write(&path, &token).expect("write the token file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let node = format!("quick-node-{}", &Uuid::new_v4().simple().to_string()[..8]);
    sqlx::query("INSERT INTO nodes (name, display_name, node_token_ref) VALUES ($1, $1, $2)")
        .bind(&node)
        .bind(format!("file://{}", path.display()))
        .execute(&pool)
        .await
        .expect("create the node");
    let environment_id: Uuid = sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(&pool)
        .await
        .expect("the default environment is seeded");
    let cluster_id: Uuid = sqlx::query_scalar("SELECT id FROM clusters WHERE environment_id = $1 ORDER BY id LIMIT 1")
        .bind(environment_id)
        .fetch_one(&pool)
        .await
        .expect("a cluster");
    let node_id: Uuid = sqlx::query_scalar("SELECT id FROM nodes WHERE is_local ORDER BY id LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("the local node");
    let sandbox_id: Uuid = sqlx::query_scalar(
        "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, image)
         VALUES ($1, $2, $3, $4, 'ubuntu:24.04') RETURNING id",
    )
    .bind(environment_id)
    .bind(cluster_id)
    .bind(node_id)
    .bind(format!("quick-{}", &Uuid::new_v4().simple().to_string()[..8]))
    .fetch_one(&pool)
    .await
    .expect("create the sandbox");

    let ctx = ServiceContext::new(
        pool.clone(),
        Arc::new(FakeRuntime::new()),
        Arc::new(nestbox_control_api::events::bus::LocalBus::new()),
        dir.path().to_path_buf(),
    );
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM telemetry_events WHERE sandbox_id = $1")
            .bind(sandbox_id)
            .fetch_one(&pool)
            .await
            .expect("count")
    };
    let batch = |tag: &str| -> ProxyBatch {
        serde_json::from_value(serde_json::json!({
            "environment_id": environment_id,
            "traffic": [{
                "ts": "2026-11-03T10:00:00Z", "sandbox_id": sandbox_id, "host": format!("{tag}.example.net"),
                "port": 443, "method": "GET", "path": "/", "decision": "allow"
            }],
            "dns": []
        }))
        .expect("a batch")
    };

    sqlx::query("UPDATE sandboxes SET telemetry_mute_until = now() + interval '1 hour' WHERE id = $1")
        .bind(sandbox_id)
        .execute(&pool)
        .await
        .expect("mute");
    telemetry_service::ingest_batch(&ctx, &node, &token, batch("muted"))
        .await
        .expect("a muted batch is accepted");
    assert_eq!(count().await, 0, "recording off stores nothing");

    sqlx::query("UPDATE sandboxes SET telemetry_mute_until = NULL, telemetry_mute_bodies_until = NULL WHERE id = $1")
        .bind(sandbox_id)
        .execute(&pool)
        .await
        .expect("unmute");
    telemetry_service::ingest_batch(&ctx, &node, &token, batch("open"))
        .await
        .expect("an open batch is accepted");
    assert_eq!(count().await, 1, "recording full stores the event");
}
