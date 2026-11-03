//! Quick controls against a real database (package B11). Compiled by the worker,
//! run by the coordinator: `recording off` stores no flow, still stores a refused
//! network or DNS event without its bodies, and `recording full` stores again.
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_cc_b11?host=/tmp/opencode/pgtest'
//!   cargo test --test quick -- --test-threads=1

use std::os::unix::fs::PermissionsExt;
use std::str::FromStr;
use std::sync::Arc;

use sqlx::postgres::PgConnectOptions;
use sqlx::{PgPool, Row};
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

/// One node with a token file, one sandbox, and a context to ingest through.
///
/// The secret directory lives here, so the token file outlives the context that
/// reads it; the tests keep the whole fixture alive for that reason.
struct Fixture {
    pool: PgPool,
    ctx: ServiceContext,
    node: String,
    token: String,
    environment_id: Uuid,
    sandbox_id: Uuid,
    _secret_dir: tempfile::TempDir,
}

/// Build the rows a proxy batch needs.
async fn fixture() -> Option<Fixture> {
    let pool = pool().await?;
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

    Some(Fixture {
        pool,
        ctx,
        node,
        token,
        environment_id,
        sandbox_id,
        _secret_dir: dir,
    })
}

/// Mute both recording columns for an hour, or clear them.
async fn set_mute(pool: &PgPool, sandbox_id: Uuid, muted: bool) {
    let statement = if muted {
        "UPDATE sandboxes SET telemetry_mute_until = now() + interval '1 hour',
                              telemetry_mute_bodies_until = now() + interval '1 hour'
          WHERE id = $1"
    } else {
        "UPDATE sandboxes SET telemetry_mute_until = NULL,
                              telemetry_mute_bodies_until = NULL
          WHERE id = $1"
    };
    sqlx::query(statement)
        .bind(sandbox_id)
        .execute(pool)
        .await
        .expect("set the recording mutes");
}

/// How many telemetry rows one sandbox has.
async fn event_count(pool: &PgPool, sandbox_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM telemetry_events WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await
        .expect("count the events")
}

/// How many captured body rows one sandbox has.
async fn body_count(pool: &PgPool, sandbox_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM traffic_bodies WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await
        .expect("count the bodies")
}

/// How many MCP tool-call rows one sandbox has.
async fn call_count(pool: &PgPool, sandbox_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM mcp_tool_calls WHERE sandbox_id = $1")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await
        .expect("count the tool calls")
}

#[tokio::test]
async fn recording_off_stores_nothing_and_full_stores_again() {
    let Some(fx) = fixture().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let batch = |tag: &str| -> ProxyBatch {
        serde_json::from_value(serde_json::json!({
            "environment_id": fx.environment_id,
            "traffic": [{
                "ts": "2026-11-03T10:00:00Z", "sandbox_id": fx.sandbox_id,
                "host": format!("{tag}.example.net"),
                "port": 443, "method": "GET", "path": "/", "decision": "allow"
            }],
            "dns": []
        }))
        .expect("a batch")
    };

    set_mute(&fx.pool, fx.sandbox_id, true).await;
    telemetry_service::ingest_batch(&fx.ctx, &fx.node, &fx.token, batch("muted"))
        .await
        .expect("a muted batch is accepted");
    assert_eq!(
        event_count(&fx.pool, fx.sandbox_id).await,
        0,
        "recording off stores no allowed flow"
    );

    set_mute(&fx.pool, fx.sandbox_id, false).await;
    telemetry_service::ingest_batch(&fx.ctx, &fx.node, &fx.token, batch("open"))
        .await
        .expect("an open batch is accepted");
    assert_eq!(
        event_count(&fx.pool, fx.sandbox_id).await,
        1,
        "recording full stores the event"
    );
}

/// A mute must not also hide a refusal: that is the governance hole ADR 0066
/// closes. The flow and the name stay, without a single byte of the body.
#[tokio::test]
async fn recording_off_keeps_the_refusals_without_their_bodies() {
    let Some(fx) = fixture().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment_id = fx.environment_id;
    let sandbox_id = fx.sandbox_id;

    // The request is recognisable as an MCP call, so a leaked tool-call row
    // would show up in the assertions below.
    let body = base64_of(b"token=secret-value");
    let batch = |body: &str| -> ProxyBatch {
        serde_json::from_value(serde_json::json!({
            "environment_id": environment_id,
            "traffic": [
                {
                    "ts": "2026-11-03T11:00:00Z", "sandbox_id": sandbox_id,
                    "host": "allowed.example.net", "port": 443, "method": "GET",
                    "path": "/", "decision": "allow"
                },
                {
                    "ts": "2026-11-03T11:00:01Z", "sandbox_id": sandbox_id,
                    "host": "blocked.example.net", "port": 443, "method": "POST",
                    "path": "/v1/messages", "decision": "deny",
                    "request_body": body,
                    "mcp_session_id": "sess-1", "mcp_method": "tools/call",
                    "mcp_tool": "read_file"
                }
            ],
            "dns": [{
                "ts": "2026-11-03T11:00:02Z", "sandbox_id": sandbox_id,
                "hostname": "blocked.example.net", "qtype": "A", "decision": "deny"
            }]
        }))
        .expect("a batch")
    };

    set_mute(&fx.pool, sandbox_id, true).await;
    let outcome = telemetry_service::ingest_batch(&fx.ctx, &fx.node, &fx.token, batch(&body))
        .await
        .expect("a muted batch is accepted");
    assert_eq!(outcome.accepted, 3, "every event is accepted");
    assert_eq!(
        outcome.stored, 2,
        "only the refused flow and the denied name are stored"
    );

    let rows = sqlx::query(
        "SELECT kind,
                coalesce(detail->>'destination', detail->>'hostname') AS name,
                detail->>'decision' AS decision,
                detail->>'has_body' AS has_body
           FROM telemetry_events
          WHERE sandbox_id = $1
          ORDER BY kind",
    )
    .bind(sandbox_id)
    .fetch_all(&fx.pool)
    .await
    .expect("read the stored events");
    let stored: Vec<(String, String, String, Option<String>)> = rows
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("kind"),
                row.get::<String, _>("name"),
                row.get::<String, _>("decision"),
                row.get::<Option<String>, _>("has_body"),
            )
        })
        .collect();
    assert_eq!(stored.len(), 2, "the allowed flow is not stored");
    assert!(
        stored
            .iter()
            .all(|(_, name, decision, _)| name == "blocked.example.net" && decision == "deny"),
        "only the refusals survive the mute: {stored:?}"
    );
    let flow = stored
        .iter()
        .find(|(kind, ..)| kind == "network")
        .expect("the refused flow");
    assert_eq!(
        flow.3.as_deref(),
        Some("false"),
        "a kept refusal must not claim the body it lost"
    );

    assert_eq!(
        body_count(&fx.pool, sandbox_id).await,
        0,
        "a mute drops the body even for a kept refusal"
    );
    assert_eq!(
        call_count(&fx.pool, sandbox_id).await,
        0,
        "a tool call carries content, so a mute drops it too"
    );

    // The mute is what dropped these rows, not a defect elsewhere: the same
    // batch stores its body and its call once recording is on again.
    set_mute(&fx.pool, sandbox_id, false).await;
    let outcome = telemetry_service::ingest_batch(&fx.ctx, &fx.node, &fx.token, batch(&body))
        .await
        .expect("an open batch is accepted");
    assert_eq!(outcome.stored, 3, "recording full stores every event");
    assert_eq!(
        event_count(&fx.pool, sandbox_id).await,
        5,
        "two rows survived the mute, three were stored after it"
    );
    assert_eq!(
        body_count(&fx.pool, sandbox_id).await,
        1,
        "recording full stores the body again"
    );
    assert_eq!(
        call_count(&fx.pool, sandbox_id).await,
        1,
        "recording full stores the tool call again"
    );
}

/// The body the proxy ships, encoded the way Envoy's JSON log carries it.
fn base64_of(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
