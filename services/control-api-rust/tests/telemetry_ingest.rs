//! The egress proxy's telemetry batch, against a real database.
//!
//! These prove the parts a pure function cannot: that a node token is checked
//! fail-closed, that a batch lands as `kind=network` and `kind=dns`, that a body
//! is redacted before it is stored, that an MCP POST fills `mcp_tool_calls`, that
//! the batch cap is enforced before anything is written, and that a node cannot
//! attribute traffic to another tenant's sandbox.
//!
//! They run only when `TEST_DATABASE_URL` is set and create their own database
//! from it, so they can run beside `tests/database.rs` and `tests/recipes.rs`
//! without any of them dropping the other's schema:
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_telemetry?host=/tmp/opencode/pgtest'
//!   cargo test --test telemetry_ingest -- --test-threads=1
//!
//! The application role `nestbox_app` is created here when it is missing, which
//! is what the appliance's Postgres init script does; on a cluster where the
//! connecting role may not create one, this suite skips rather than fails. The
//! row-level-security checks live in `tests/database.rs`; this suite does not
//! repeat them.

use std::os::unix::fs::PermissionsExt;
use std::str::FromStr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tokio::sync::OnceCell;
use tower::ServiceExt;
use uuid::Uuid;

use nestbox_control_api::config::Config;
use nestbox_control_api::domain::telemetry::LiveEvent;
use nestbox_control_api::error::ErrorCode;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::services::telemetry_service::{self, MAX_EVENTS_PER_BATCH, ProxyBatch};
use nestbox_control_api::state::AppState;
use nestbox_control_api::store::postgres::traffic_bodies;

/// The database this suite owns. Fixed so a previous failed run is replaced
/// rather than accumulated.
const DATABASE: &str = "nestbox_proxy_telemetry_test";

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

/// A pool for one test.
async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?;
    sqlx::PgPool::connect_with(options.database(DATABASE)).await.ok()
}

/// A token directory and the node row that points at it.
struct TestNode {
    name: String,
    token: String,
    _dir: tempfile::TempDir,
}

/// Register a node whose token is a 0600 file, as on the appliance.
async fn node(pool: &PgPool) -> TestNode {
    let dir = tempfile::tempdir().expect("a temporary secret directory");
    let token = format!("node-token-{}", Uuid::new_v4().simple());
    let path = dir.path().join("node.token");
    std::fs::write(&path, &token).expect("write the token file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("restrict the token file");

    let name = format!("telemetry-node-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let locator = format!("file://{}", path.display());
    sqlx::query("INSERT INTO nodes (name, display_name, node_token_ref) VALUES ($1, $1, $2)")
        .bind(&name)
        .bind(&locator)
        .execute(pool)
        .await
        .expect("create the node");

    TestNode {
        name,
        token,
        _dir: dir,
    }
}

/// The seeded default environment.
async fn environment_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .expect("the default environment is seeded")
}

/// A sandbox in an environment, with the seeded cluster and the local node.
async fn sandbox(pool: &PgPool, environment_id: Uuid, label: &str) -> Uuid {
    let cluster_id: Uuid =
        sqlx::query_scalar("SELECT id FROM clusters WHERE environment_id = $1 ORDER BY id LIMIT 1")
            .bind(environment_id)
            .fetch_one(pool)
            .await
            .expect("the environment has a cluster");
    let node_id: Uuid =
        sqlx::query_scalar("SELECT id FROM nodes WHERE is_local ORDER BY id LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("the local node is seeded");
    let name = format!("{label}-{}", &Uuid::new_v4().simple().to_string()[..8]);
    sqlx::query_scalar(
        "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, image)
         VALUES ($1, $2, $3, $4, 'ubuntu:24.04')
         RETURNING id",
    )
    .bind(environment_id)
    .bind(cluster_id)
    .bind(node_id)
    .bind(&name)
    .fetch_one(pool)
    .await
    .expect("create the sandbox")
}

/// A service context whose bus is the one the SSE handler reads.
fn context(pool: &PgPool, secret_dir: &str) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        Arc::new(FakeRuntime::new()),
        Arc::new(nestbox_control_api::events::bus::LocalBus::new()),
        std::path::PathBuf::from(secret_dir),
    )
}

/// A config for the router-level test. Only the secret directory matters here.
fn config(secret_dir: &str) -> Config {
    Config {
        database_url: std::env::var("TEST_DATABASE_URL").unwrap_or_default(),
        listen_addr: "127.0.0.1:0".into(),
        secret_dir: std::path::PathBuf::from(secret_dir),
        session_cookie_secure: false,
        session_idle_seconds: 3600,
        session_max_seconds: 86_400,
        allowed_origins: Vec::new(),
        runner_url: "http://runner:9000".into(),
        runtime_network: "nestbox-runtime".into(),
        bootstrap_admin: false,
        body_limit_bytes: 1_048_576,
        rate_limit_per_minute: 600,
        terminal_idle_seconds: 1800,
        deploy_mode: nestbox_control_api::config::DeployMode::SelfHosted,
        public_url: String::new(),
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

/// A batch as the proxy ships it: access-log fields plus base64 bodies.
fn batch_json(
    environment_id: Uuid,
    sandbox_id: Uuid,
    tools_request: &[u8],
    tools_response: &[u8],
) -> serde_json::Value {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    serde_json::json!({
        "environment_id": environment_id,
        "traffic": [
            {
                "ts": "2026-10-27T10:00:00Z",
                "sandbox_id": sandbox_id,
                "host": "mcp.example.com",
                "port": 443,
                "protocol": "HTTP/2",
                "method": "POST",
                "path": "/mcp",
                "status": 200,
                "duration_ms": 37,
                "bytes_in": 120,
                "bytes_out": 340,
                "client": "10.244.0.2",
                "user_agent": "hermes/0.22.5",
                "request_id": "req-1",
                "decision": "allow",
                "reason": "rule",
                "mcp_session_id": "sess-1",
                "mcp_server": "nestbox-hub",
                "request_body": engine.encode(tools_request),
                "response_body": engine.encode(tools_response)
            },
            {
                "ts": "2026-10-27T10:00:01Z",
                "sandbox_id": sandbox_id,
                "host": "api.anthropic.com",
                "port": 443,
                "protocol": "HTTP/1.1",
                "method": "POST",
                "path": "/v1/messages",
                "status": 500,
                "duration_ms": 1200,
                "bytes_in": 800,
                "bytes_out": 90,
                "client": "10.244.0.2",
                "request_id": "req-2",
                "decision": "deny",
                "reason": "default_deny"
            }
        ],
        "dns": [
            {
                "ts": "2026-10-27T10:00:02Z",
                "sandbox_id": sandbox_id,
                "hostname": "mcp.example.com",
                "qtype": "A",
                "answer": "93.184.216.34",
                "ttl": 60,
                "decision": "allow",
                "resolver": "coredns",
                "request_id": "dns-1"
            }
        ]
    })
}

/// The MCP request the agent sends: a tool call with a key and a pasted token.
fn mcp_request() -> Vec<u8> {
    br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"/etc/hostname","api_key":"sk-abcdefghijklmnopqrstuvwxyz012345","note":"here is sk-proj-ZYXWVUTSRQPONMLKJIHGFEDCBA9876543210"}}}"#.to_vec()
}

/// A successful MCP result.
fn mcp_response() -> Vec<u8> {
    br#"{"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"hermes-01"}],"isError":false}}"#.to_vec()
}

/// The secret material that must never reach the database.
const SECRET: &str = "sk-abcdefghijklmnopqrstuvwxyz012345";
const PASTED_SECRET: &str = "sk-proj-ZYXWVUTSRQPONMLKJIHGFEDCBA9876543210";

#[tokio::test]
async fn a_proxy_batch_lands_as_network_and_dns_with_redacted_bodies() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment_id = environment_id(&pool).await;
    let sandbox_id = sandbox(&pool, environment_id, "telemetry").await;
    let node = node(&pool).await;

    let state = AppState::new(
        config(&node._dir.path().display().to_string()),
        Arc::new(FakeRuntime::new()),
        pool.clone(),
    );

    // An SSE client subscribes to this bus; `drain_once` is the same read path
    // the stream uses, including the environment filter.
    let collector = tokio::spawn({
        let state = state.clone();
        async move {
            nestbox_control_api::api::events::drain_once(&state, environment_id, 3).await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let body = batch_json(environment_id, sandbox_id, &mcp_request(), &mcp_response());
    let response = nestbox_control_api::app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/internal/telemetry/batch")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", node.token))
                .header("x-nestbox-node", &node.name)
                .body(Body::from(body.to_string()))
                .expect("a request"),
        )
        .await
        .expect("the router answers");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    let answer: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(answer["accepted"], 3, "{answer}");
    assert_eq!(answer["stored"], 3, "{answer}");
    assert_eq!(answer["bodies"], 1, "{answer}");
    assert_eq!(answer["mcp_calls"], 1, "{answer}");

    // 1. The batch landed as two kinds.
    let kinds: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM telemetry_events WHERE sandbox_id = $1 ORDER BY ts",
    )
    .bind(sandbox_id)
    .fetch_all(&pool)
    .await
    .expect("query kinds");
    assert_eq!(kinds, vec!["network", "network", "dns"], "{kinds:?}");

    let destination: String = sqlx::query_scalar(
        "SELECT detail->>'destination' FROM telemetry_events
          WHERE sandbox_id = $1 AND kind = 'network' AND ts = '2026-10-27T10:00:00Z'",
    )
    .bind(sandbox_id)
    .fetch_one(&pool)
    .await
    .expect("the first network row");
    assert_eq!(destination, "mcp.example.com");

    let dns_answer: String = sqlx::query_scalar(
        "SELECT detail->>'answer' FROM telemetry_events
          WHERE sandbox_id = $1 AND kind = 'dns'",
    )
    .bind(sandbox_id)
    .fetch_one(&pool)
    .await
    .expect("the dns row");
    assert_eq!(dns_answer, "93.184.216.34");

    // 2. The body is stored redacted: the keyed value and the pasted token.
    let request_body: String = sqlx::query_scalar(
        "SELECT request_body FROM traffic_bodies WHERE sandbox_id = $1",
    )
    .bind(sandbox_id)
    .fetch_one(&pool)
    .await
    .expect("the stored body");
    assert!(request_body.contains("[redacted]"), "{request_body}");
    assert!(!request_body.contains(SECRET), "{request_body}");
    assert!(!request_body.contains(PASTED_SECRET), "{request_body}");
    // The readable part survives, so the body is useful evidence.
    assert!(request_body.contains("/etc/hostname"), "{request_body}");
    assert!(request_body.contains("read_file"), "{request_body}");

    // 3. The MCP POST filled the tool-call row.
    let (method, tool, session, is_error, duration): (String, String, String, bool, i32) =
        sqlx::query_as(
            "SELECT method, tool_name, session_id, is_error, duration_ms
               FROM mcp_tool_calls WHERE sandbox_id = $1",
        )
        .bind(sandbox_id)
        .fetch_one(&pool)
        .await
        .expect("the mcp row");
    assert_eq!(method, "tools/call");
    assert_eq!(tool, "read_file");
    assert_eq!(session, "sess-1");
    assert!(!is_error);
    assert_eq!(duration, 37);

    let arguments: serde_json::Value = sqlx::query_scalar(
        "SELECT arguments FROM mcp_tool_calls WHERE sandbox_id = $1",
    )
    .bind(sandbox_id)
    .fetch_one(&pool)
    .await
    .expect("the arguments");
    assert_eq!(arguments["api_key"], "[redacted]");
    assert_eq!(arguments["path"], "/etc/hostname");

    // 4. The stream saw the events, live, on the bus the SSE handler reads.
    let events = collector.await.expect("the collector task");
    assert_eq!(events.len(), 3, "expected three live events");
    let names: Vec<String> = events
        .iter()
        .filter_map(|event| match event {
            LiveEvent::Telemetry(inner) => Some(inner.kind().stream_name().to_string()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"network".to_string()), "{names:?}");
    assert!(names.contains(&"dns".to_string()), "{names:?}");
    assert!(events
        .iter()
        .all(|event| event.environment_id() == environment_id));
}

#[tokio::test]
async fn the_node_token_path_fails_closed() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment_id = environment_id(&pool).await;
    let sandbox_id = sandbox(&pool, environment_id, "token").await;
    let node = node(&pool).await;

    // A node whose row has no token locator cannot ingest, even with a guess.
    let untokened = format!("telemetry-node-{}", &Uuid::new_v4().simple().to_string()[..8]);
    sqlx::query("INSERT INTO nodes (name, display_name) VALUES ($1, $1)")
        .bind(&untokened)
        .execute(&pool)
        .await
        .expect("create the untokened node");

    let ctx = context(&pool, &node._dir.path().display().to_string());
    let batch = || -> ProxyBatch {
        serde_json::from_value(batch_json(
            environment_id,
            sandbox_id,
            &mcp_request(),
            &mcp_response(),
        ))
        .expect("a valid batch")
    };

    let cases = [
        ("unknown node", "no-such-node-here", node.token.as_str()),
        ("node without a token", untokened.as_str(), node.token.as_str()),
        ("wrong token", node.name.as_str(), "not-the-token"),
        ("no token", node.name.as_str(), ""),
        ("no node", "", node.token.as_str()),
    ];
    for (label, reference, token) in cases {
        let error = telemetry_service::ingest_batch(&ctx, reference, token, batch())
            .await
            .expect_err(label);
        assert_eq!(error.code, ErrorCode::UNAUTHENTICATED, "{label}");
        assert_eq!(error.status, StatusCode::UNAUTHORIZED, "{label}");
    }

    // Nothing from a refused batch was written.
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM telemetry_events WHERE sandbox_id = $1",
    )
    .bind(sandbox_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(rows, 0);

    // And the correct token is accepted.
    let outcome = telemetry_service::ingest_batch(&ctx, &node.name, &node.token, batch())
        .await
        .expect("the correct token is accepted");
    assert_eq!(outcome.accepted, 3);
}

#[tokio::test]
async fn a_batch_beyond_the_cap_is_refused_before_anything_is_written() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment_id = environment_id(&pool).await;
    let sandbox_id = sandbox(&pool, environment_id, "flood").await;
    let node = node(&pool).await;
    let ctx = context(&pool, &node._dir.path().display().to_string());

    let entry = |index: usize| {
        serde_json::json!({
            "sandbox_id": sandbox_id,
            "host": format!("host-{index}.example.com"),
            "port": 443,
            "decision": "allow"
        })
    };
    let traffic: Vec<serde_json::Value> = (0..=MAX_EVENTS_PER_BATCH).map(entry).collect();
    let batch: ProxyBatch = serde_json::from_value(serde_json::json!({
        "environment_id": environment_id,
        "traffic": traffic
    }))
    .expect("a valid batch");

    let error = telemetry_service::ingest_batch(&ctx, &node.name, &node.token, batch)
        .await
        .expect_err("a batch beyond the cap is refused");
    assert_eq!(error.code, ErrorCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM telemetry_events WHERE sandbox_id = $1",
    )
    .bind(sandbox_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(rows, 0, "a refused batch writes nothing");
}

#[tokio::test]
async fn a_batch_cannot_name_another_tenants_sandbox() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment_id = environment_id(&pool).await;
    let other_environment: Uuid = sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug, timezone, retention_days)
         VALUES ('00000000-0000-0000-0000-000000000001', 'Other', $1, 'UTC', 30)
         RETURNING id",
    )
    .bind(format!("other-{}", &Uuid::new_v4().simple().to_string()[..8]))
    .fetch_one(&pool)
    .await
    .expect("create a second environment");
    // A sandbox needs a cluster in its own environment.
    sqlx::query(
        "INSERT INTO clusters (environment_id, name, slug, placement_policy)
         VALUES ($1, 'other', 'other', 'least_loaded')",
    )
    .bind(other_environment)
    .execute(&pool)
    .await
    .expect("create a cluster for the second environment");
    let foreign_sandbox = sandbox(&pool, other_environment, "foreign").await;
    let node = node(&pool).await;
    let ctx = context(&pool, &node._dir.path().display().to_string());

    let batch: ProxyBatch = serde_json::from_value(serde_json::json!({
        "environment_id": environment_id,
        "traffic": [{
            "sandbox_id": foreign_sandbox,
            "host": "mcp.example.com",
            "port": 443,
            "decision": "allow"
        }]
    }))
    .expect("a valid batch");

    let error = telemetry_service::ingest_batch(&ctx, &node.name, &node.token, batch)
        .await
        .expect_err("a foreign sandbox is refused");
    assert_eq!(error.status, StatusCode::FORBIDDEN);

    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM telemetry_events WHERE sandbox_id = $1",
    )
    .bind(foreign_sandbox)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(rows, 0, "a refused batch writes nothing");
}

#[tokio::test]
async fn the_batch_insert_satisfies_row_level_security_as_the_application_role() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment_id = environment_id(&pool).await;
    let sandbox_id = sandbox(&pool, environment_id, "rls").await;
    let node = node(&pool).await;

    // The API connects as `nestbox_app`, a non-owner role, which is the only
    // configuration in which row-level security applies. `role` is a startup
    // parameter, so the session is authenticated as the test user and then runs
    // as the application role — no password and no second role to create.
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let Ok(options) = PgConnectOptions::from_str(&url) else {
        eprintln!("skipping: TEST_DATABASE_URL is not a connection string");
        return;
    };
    let app_pool = match PgPool::connect_with(
        options
            .database(DATABASE)
            .options([("role", "nestbox_app")]),
    )
    .await
    {
        Ok(pool) => pool,
        Err(error) => {
            // A cluster where the application role cannot be assumed is a known
            // gap, not a failure of this code; say so instead of bending the
            // test into something weaker.
            eprintln!("skipping: the application role cannot be assumed: {error}");
            return;
        }
    };

    // Without a tenant scope, the same insert is refused by the policy. This is
    // the control the scope exists to satisfy, proven rather than assumed.
    let unscoped = sqlx::query(
        "INSERT INTO telemetry_events (environment_id, sandbox_id, kind, ts, detail)
         VALUES ($1, $2, 'network', now(), '{}'::jsonb)",
    )
    .bind(environment_id)
    .bind(sandbox_id)
    .execute(&app_pool)
    .await;
    assert!(
        unscoped.is_err(),
        "row-level security must refuse an unscoped insert"
    );

    let ctx = context(&app_pool, &node._dir.path().display().to_string());
    let batch: ProxyBatch = serde_json::from_value(batch_json(
        environment_id,
        sandbox_id,
        &mcp_request(),
        &mcp_response(),
    ))
    .expect("a valid batch");
    let outcome = telemetry_service::ingest_batch(&ctx, &node.name, &node.token, batch)
        .await
        .expect("the application role can store a scoped batch");
    assert_eq!(outcome.stored, 3);
    assert_eq!(outcome.bodies, 1);
    assert_eq!(outcome.mcp_calls, 1);

    let kinds: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM telemetry_events WHERE sandbox_id = $1 ORDER BY ts",
    )
    .bind(sandbox_id)
    .fetch_all(&pool)
    .await
    .expect("query kinds");
    assert_eq!(kinds, vec!["network", "network", "dns"], "{kinds:?}");

    // The single-event path shares the scope, so `POST /telemetry/ingest` and
    // the demo seed also store while RLS applies.
    let single = telemetry_service::sample_event(environment_id, Some(sandbox_id), 0);
    let stored = telemetry_service::ingest(&ctx, single)
        .await
        .expect("a single event is stored as the application role");
    let single_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM telemetry_events WHERE id = $1")
            .bind(stored)
            .fetch_one(&pool)
            .await
            .expect("count the single event");
    assert_eq!(single_rows, 1);
}

#[tokio::test]
async fn bodies_older_than_the_retention_window_are_purged() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment_id = environment_id(&pool).await;
    let sandbox_id = sandbox(&pool, environment_id, "retention").await;

    let fresh = Uuid::new_v4();
    let stale = Uuid::new_v4();
    let now = chrono::Utc::now();
    let bodies = vec![
        traffic_bodies::NewBody {
            event_id: fresh,
            sandbox_id: Some(sandbox_id),
            ts: now,
            request_body: Some("fresh"),
            response_body: None,
            request_bytes: 5,
            response_bytes: 0,
            truncated: false,
        },
        traffic_bodies::NewBody {
            event_id: stale,
            sandbox_id: Some(sandbox_id),
            ts: now - chrono::Duration::days(2),
            request_body: Some("stale"),
            response_body: None,
            request_bytes: 5,
            response_bytes: 0,
            truncated: false,
        },
    ];
    // The suite's database keeps rows between runs (only the schema is
    // dropped), so an earlier run's stale body would be purged here as well and
    // the count would say 2. Clear first, then measure this run's work.
    traffic_bodies::purge_expired(&pool, now - chrono::Duration::days(1))
        .await
        .expect("a clean slate");

    let written = traffic_bodies::insert_batch(&pool, environment_id, &bodies)
        .await
        .expect("insert bodies");
    assert_eq!(written, 2);

    let purged = traffic_bodies::purge_expired(&pool, now - chrono::Duration::days(1))
        .await
        .expect("purge");
    assert_eq!(purged, 1, "only this run's stale body is purged");

    let kept: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM traffic_bodies WHERE event_id = $1",
    )
    .bind(fresh)
    .fetch_one(&pool)
    .await
    .expect("count fresh");
    assert_eq!(kept, 1);

    let dropped: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM traffic_bodies WHERE event_id = $1",
    )
    .bind(stale)
    .fetch_one(&pool)
    .await
    .expect("count stale");
    assert_eq!(dropped, 0);
}
