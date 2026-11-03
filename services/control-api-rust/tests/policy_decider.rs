//! The egress decider against a real PostgreSQL.
//!
//! These tests are the proof that `POST /api/v1/internal/policy/check` decides,
//! records, and refuses in the way the plan promises — not that the pure
//! `decide` function does, which `domain::egress` already proves without a
//! database. Each test gets a fresh schema, so the migration path is exercised
//! too.
//!
//! They skip when `TEST_DATABASE_URL` is unset, so `cargo test` works without a
//! database:
//!
//!   export TEST_DATABASE_URL="postgres://core@localhost:55432/nestbox_policy?host=/tmp/opencode/pgtest"
//!   cargo test --test policy_decider -- --test-threads=1

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

use nestbox_control_api::config::Config;
use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::egress::{CheckRequest, Decision, DecisionReason};
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::services::policy_service::{self, CheckInput, CheckOutcome};
use nestbox_control_api::state::AppState;

/// The environment and the local node every migration seeds.
const ENVIRONMENT: &str = "00000000-0000-0000-0000-000000000002";
const CLUSTER: &str = "00000000-0000-0000-0000-000000000004";
const LOCAL_NODE: &str = "00000000-0000-0000-0000-000000000003";

fn environment_id() -> Uuid {
    Uuid::from_str(ENVIRONMENT).unwrap_or_else(|_| Uuid::nil())
}

fn local_node() -> Uuid {
    Uuid::from_str(LOCAL_NODE).unwrap_or_else(|_| Uuid::nil())
}

/// A fresh schema with the migrations applied.
async fn connect() -> Option<sqlx::PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let pool = sqlx::PgPool::connect(&url).await.ok()?;
    let _ = sqlx::query("DROP SCHEMA IF EXISTS public CASCADE")
        .execute(&pool)
        .await;
    let _ = sqlx::query("CREATE SCHEMA public").execute(&pool).await;
    let _ = sqlx::query(
        "DO $$BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'nestbox_app') THEN
                CREATE ROLE nestbox_app LOGIN;
            END IF;
        END$$",
    )
    .execute(&pool)
    .await;
    let _ = sqlx::query("GRANT USAGE, CREATE ON SCHEMA public TO nestbox_app")
        .execute(&pool)
        .await;
    sqlx::migrate!("./migrations").run(&pool).await.ok()?;
    Some(pool)
}

fn secret_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nestbox-policy-test-{}", Uuid::new_v4()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn context(pool: &sqlx::PgPool, secret_dir: &Path) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        Arc::new(FakeRuntime::new()),
        Arc::new(LocalBus::new()),
        secret_dir.to_path_buf(),
    )
}

fn config(secret_dir: PathBuf) -> Config {
    Config {
        database_url: "postgres://unused".into(),
        listen_addr: "127.0.0.1:0".into(),
        secret_dir,
        session_cookie_secure: false,
        session_idle_seconds: 3600,
        session_max_seconds: 7200,
        allowed_origins: Vec::new(),
        runner_url: "http://sandbox-runner:9000".into(),
        runtime_network: "nestbox-runtime".into(),
        bootstrap_admin: false,
        body_limit_bytes: 1_048_576,
        rate_limit_per_minute: 600,
        terminal_idle_seconds: 30 * 60,
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

fn principal() -> Principal {
    Principal {
        user_id: Uuid::nil(),
        environment_id: environment_id(),
        role: Role::Admin,
        display_name: "test".into(),
        via: CredentialSource::Token,
    }
}

/// A sandbox on the local node, as a create would leave it.
async fn sandbox(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, image, status)
         VALUES ($1, $2, $3, 'policy-test', 'ghcr.io/nestbox/base@sha256:0000', 'running')
         RETURNING id",
    )
    .bind(environment_id())
    .bind(Uuid::from_str(CLUSTER).unwrap_or_else(|_| Uuid::nil()))
    .bind(local_node())
    .fetch_one(pool)
    .await
    .expect("insert sandbox")
}

async fn insert_rule(
    pool: &sqlx::PgPool,
    sandbox_id: Uuid,
    host: &str,
    port: Option<i32>,
    action: &str,
    enforcement: &str,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO network_rules
            (environment_id, sandbox_id, name, action, host, port, protocol, enforcement, priority)
         VALUES ($1, $2, $3, $4, $5, $6, 'rest', $7, 10)
         RETURNING id",
    )
    .bind(environment_id())
    .bind(sandbox_id)
    .bind(format!("{action} {host}"))
    .bind(action)
    .bind(host)
    .bind(port)
    .bind(enforcement)
    .fetch_one(pool)
    .await
    .expect("insert rule")
}

/// Insert a rule in matcher form, the shape the console sends.
///
/// The flat columns are filled from the matchers by the insert, so this also
/// proves the derived host is written and the decider reads the matchers.
async fn insert_matcher_rule(
    pool: &sqlx::PgPool,
    sandbox_id: Uuid,
    kind: &str,
    value: &str,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO network_rules
            (environment_id, sandbox_id, name, action, host, port, protocol,
             enforcement, priority, matchers)
         VALUES ($1, $2, $3, 'allow', $4, NULL, 'tcp', 'enforced', 10, $5::jsonb)
         RETURNING id",
    )
    .bind(environment_id())
    .bind(sandbox_id)
    .bind(format!("{kind} {value}"))
    .bind("*")
    .bind(serde_json::json!([{ "kind": kind, "value": value }]).to_string())
    .fetch_one(pool)
    .await
    .expect("insert matcher rule")
}

/// A suffix matcher in the database decides, not the flat column.
///
/// The rule says `.example.com`; the flat host is `*`, which the legacy reader
/// would read as "anything". The decision proves the matchers are what the
/// proxy's requests are judged against.
#[tokio::test]
async fn a_suffix_matcher_allows_the_subdomain_and_denies_the_lookalike() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;
    insert_matcher_rule(&pool, sandbox_id, "host_suffix", ".example.com").await;

    let allowed = check(&ctx, sandbox_id, "api.example.com", 443).await;
    assert_eq!(allowed.response.decision, Decision::Allow);

    let refused = check(&ctx, sandbox_id, "notexample.com", 443).await;
    assert_eq!(refused.response.decision, Decision::Deny);
    assert_eq!(refused.response.reason, DecisionReason::DefaultDeny);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A legacy rule written into the flat columns keeps meaning what it meant.
///
/// The row carries no matchers; the reader derives them from the flat fields,
/// which is the same translation migration 0032 did once.
#[tokio::test]
async fn a_flat_rule_without_matchers_still_decides_the_same_way() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;
    insert_rule(&pool, sandbox_id, "example.com", Some(443), "allow", "enforced").await;

    let allowed = check(&ctx, sandbox_id, "example.com", 443).await;
    assert_eq!(allowed.response.decision, Decision::Allow);
    let refused = check(&ctx, sandbox_id, "other.com", 443).await;
    assert_eq!(refused.response.decision, Decision::Deny);
    let _ = std::fs::remove_dir_all(&dir);
}

async fn open_window(pool: &sqlx::PgPool, sandbox_id: Uuid, minutes: i64) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO traffic_windows
            (environment_id, sandbox_id, scope, reason, expires_at)
         VALUES ($1, $2, 'sandbox', 'debugging', now() + make_interval(mins => $3))
         RETURNING id",
    )
    .bind(environment_id())
    .bind(sandbox_id)
    .bind(minutes as i32)
    .fetch_one(pool)
    .await
    .expect("insert window")
}

fn request(sandbox_id: Uuid, host: &str, port: i32) -> CheckRequest {
    CheckRequest {
        sandbox_id: Some(sandbox_id),
        node_id: Some(local_node()),
        host: host.into(),
        port,
        protocol: Some("https".into()),
        method: Some("GET".into()),
        path: Some("/".into()),
        user_agent: Some("hermes-agent/0.22.5".into()),
        mcp_tool: None,
        mcp_session_id: None,
    }
}

async fn check(ctx: &ServiceContext, sandbox_id: Uuid, host: &str, port: i32) -> CheckOutcome {
    policy_service::check(
        ctx,
        CheckInput {
            node_id: local_node(),
            request: request(sandbox_id, host, port),
            cache_hit: false,
            reported_hash: None,
        },
    )
    .await
    .expect("check")
}

/// One recorded decision, as the console would read it later.
struct Recorded {
    decision: String,
    reason: String,
    rule_id: Option<Uuid>,
    window_id: Option<Uuid>,
    cache_hit: bool,
}

async fn recorded(pool: &sqlx::PgPool, sandbox_id: Uuid) -> Option<Recorded> {
    let row = sqlx::query(
        "SELECT decision, reason, rule_id, window_id, cache_hit
           FROM policy_checks WHERE sandbox_id = $1 ORDER BY ts DESC, id DESC LIMIT 1",
    )
    .bind(sandbox_id)
    .fetch_optional(pool)
    .await
    .ok()??;

    use sqlx::Row;
    Some(Recorded {
        decision: row.try_get("decision").ok()?,
        reason: row.try_get("reason").ok()?,
        rule_id: row.try_get("rule_id").ok()?,
        window_id: row.try_get("window_id").ok()?,
        cache_hit: row.try_get("cache_hit").ok()?,
    })
}

async fn compiled_hash(pool: &sqlx::PgPool) -> Option<String> {
    sqlx::query_scalar("SELECT hash FROM policy_compilations WHERE environment_id = $1")
        .bind(environment_id())
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// A rule that matches allows, and the decision is written with its rule id.
#[tokio::test]
async fn an_allow_rule_allows_and_the_decision_is_recorded() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;
    let rule_id = insert_rule(
        &pool,
        sandbox_id,
        "example.com",
        Some(443),
        "allow",
        "enforced",
    )
    .await;

    let outcome = check(&ctx, sandbox_id, "example.com", 443).await;

    assert_eq!(outcome.response.decision, Decision::Allow);
    assert_eq!(outcome.response.reason, DecisionReason::Rule);
    assert_eq!(outcome.response.rule_id, Some(rule_id));
    assert!(!outcome.response.policy_hash.is_empty());

    let row = recorded(&pool, sandbox_id).await.expect("a row");
    assert_eq!(row.decision, "allow");
    assert_eq!(row.reason, "rule");
    assert_eq!(row.rule_id, Some(rule_id));
    assert_eq!(row.window_id, None);
    assert!(!row.cache_hit);

    assert_eq!(
        compiled_hash(&pool).await.as_deref(),
        Some(outcome.response.policy_hash.as_str()),
        "the compiled hash is the one the decision carried"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Nothing matching is a deny, and the row says so.
#[tokio::test]
async fn without_a_matching_rule_the_answer_is_default_deny() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;

    let outcome = check(&ctx, sandbox_id, "example.com", 443).await;

    assert_eq!(outcome.response.decision, Decision::Deny);
    assert_eq!(outcome.response.reason, DecisionReason::DefaultDeny);
    assert!(outcome.response.explanation.contains("example.com:443"));

    let row = recorded(&pool, sandbox_id).await.expect("a row");
    assert_eq!(row.decision, "deny");
    assert_eq!(row.reason, "default_deny");
    assert_eq!(row.rule_id, None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An open window beats the rule set, and its id is part of the evidence.
#[tokio::test]
async fn an_open_window_allows_and_records_the_window() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;
    let window_id = open_window(&pool, sandbox_id, 15).await;

    let outcome = check(&ctx, sandbox_id, "anything.example.net", 8443).await;

    assert_eq!(outcome.response.decision, Decision::Allow);
    assert_eq!(outcome.response.reason, DecisionReason::Window);
    assert_eq!(outcome.response.window_id, Some(window_id));
    assert!(outcome.response.cache_ttl_seconds <= 5);

    let row = recorded(&pool, sandbox_id).await.expect("a row");
    assert_eq!(row.decision, "allow");
    assert_eq!(row.reason, "window");
    assert_eq!(row.window_id, Some(window_id));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An audit-only deny lets the connection through but marks it.
#[tokio::test]
async fn an_audit_rule_passes_but_is_recorded_as_audit() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;
    let rule_id = insert_rule(&pool, sandbox_id, "*", None, "deny", "audit").await;

    let outcome = check(&ctx, sandbox_id, "blocked.example", 443).await;

    assert_eq!(outcome.response.decision, Decision::Audit);
    assert!(outcome.response.decision.passes());
    assert_eq!(outcome.response.rule_id, Some(rule_id));

    let row = recorded(&pool, sandbox_id).await.expect("a row");
    assert_eq!(row.decision, "audit");
    assert_eq!(row.reason, "rule");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every rule-set change changes the hash, and the row in `policy_compilations`
/// follows it.
#[tokio::test]
async fn the_compiled_hash_changes_with_the_rule_set() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;
    // A provider another test (or a real probe of the seeded endpoint) marked
    // reachable contributes an allow rule; this test counts only its own.
    sqlx::query("UPDATE provider_profiles SET status = 'unreachable'")
        .execute(&pool)
        .await
        .expect("reset provider status");

    let first = check(&ctx, sandbox_id, "example.com", 443).await;
    assert_eq!(
        compiled_hash(&pool).await.as_deref(),
        Some(first.response.policy_hash.as_str())
    );

    insert_rule(
        &pool,
        sandbox_id,
        "example.com",
        Some(443),
        "allow",
        "enforced",
    )
    .await;
    let second = check(&ctx, sandbox_id, "example.com", 443).await;

    assert_ne!(
        first.response.policy_hash, second.response.policy_hash,
        "a new rule must change the hash, or the deployed badge would lie"
    );
    assert_eq!(
        compiled_hash(&pool).await.as_deref(),
        Some(second.response.policy_hash.as_str())
    );

    let rule_count: i32 =
        sqlx::query_scalar("SELECT rule_count FROM policy_compilations WHERE environment_id = $1")
            .bind(environment_id())
            .fetch_one(&pool)
            .await
            .expect("rule count");
    assert_eq!(rule_count, 1);

    // A check that changes nothing must not move `compiled_at`: it means "when
    // the rule set last changed", not "when this request happened".
    let compiled_at: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT compiled_at FROM policy_compilations WHERE environment_id = $1")
            .bind(environment_id())
            .fetch_one(&pool)
            .await
            .expect("compiled_at");

    open_window(&pool, sandbox_id, 10).await;
    let third = check(&ctx, sandbox_id, "elsewhere.example", 443).await;
    assert_eq!(third.response.decision, Decision::Allow);
    assert_eq!(third.response.policy_hash, second.response.policy_hash);

    let after = sqlx::query(
        "SELECT compiled_at, window_count FROM policy_compilations WHERE environment_id = $1",
    )
    .bind(environment_id())
    .fetch_one(&pool)
    .await
    .expect("compilation row");
    use sqlx::Row;
    let after_compiled_at: chrono::DateTime<chrono::Utc> =
        after.try_get("compiled_at").expect("compiled_at");
    assert_eq!(
        after_compiled_at, compiled_at,
        "a window is not a rule-set change"
    );
    assert_eq!(after.try_get::<i32, _>("window_count").expect("count"), 1);

    let _ = std::fs::remove_dir_all(&dir);
}

/// A node that reports another hash is visibly "not deployed"; one that reports
/// the compiled hash is deployed.
#[tokio::test]
async fn a_node_that_reports_another_hash_is_not_deployed() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;
    insert_rule(
        &pool,
        sandbox_id,
        "example.com",
        Some(443),
        "allow",
        "enforced",
    )
    .await;

    let first = check(&ctx, sandbox_id, "example.com", 443).await;
    let compiled = first.response.policy_hash.clone();
    let stale_hash = "0".repeat(64);

    // The proxy reports an older rule set: the badge must say "not deployed".
    let stale = policy_service::check(
        &ctx,
        CheckInput {
            node_id: local_node(),
            request: request(sandbox_id, "example.com", 443),
            cache_hit: true,
            reported_hash: Some(stale_hash.clone()),
        },
    )
    .await
    .expect("check");
    assert!(stale.cache_hit);
    assert!(
        recorded(&pool, sandbox_id).await.expect("a row").cache_hit,
        "a revalidated answer is recorded as a cache hit"
    );

    let deployment = policy_service::deployment(&ctx, &principal())
        .await
        .expect("deployment");
    assert_eq!(deployment.compiled_hash.as_deref(), Some(compiled.as_str()));
    let node = deployment
        .nodes
        .iter()
        .find(|node| node.node_id == local_node())
        .expect("the local node runs the sandbox");
    assert!(!node.deployed);
    assert_eq!(node.reported_hash.as_deref(), Some(stale_hash.as_str()));

    // Now it reports what is compiled: deployed.
    policy_service::check(
        &ctx,
        CheckInput {
            node_id: local_node(),
            request: request(sandbox_id, "example.com", 443),
            cache_hit: false,
            reported_hash: Some(compiled.clone()),
        },
    )
    .await
    .expect("check");

    let deployment = policy_service::deployment(&ctx, &principal())
        .await
        .expect("deployment");
    let node = deployment
        .nodes
        .iter()
        .find(|node| node.node_id == local_node())
        .expect("the local node runs the sandbox");
    assert!(node.deployed);
    assert_eq!(node.reported_hash.as_deref(), Some(compiled.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A node cannot get a decision about a sandbox on another node.
#[tokio::test]
async fn a_check_from_a_node_that_does_not_run_the_sandbox_is_refused() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let ctx = context(&pool, &dir);
    let sandbox_id = sandbox(&pool).await;
    insert_rule(
        &pool,
        sandbox_id,
        "example.com",
        Some(443),
        "allow",
        "enforced",
    )
    .await;

    let other_node: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name, display_name, kind, status)
         VALUES ('other', 'Other', 'vm', 'unknown')
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert node");

    let outcome = policy_service::check(
        &ctx,
        CheckInput {
            node_id: other_node,
            request: request(sandbox_id, "example.com", 443),
            cache_hit: false,
            reported_hash: None,
        },
    )
    .await
    .expect("check");

    assert_eq!(outcome.response.decision, Decision::Deny);
    assert_eq!(outcome.response.reason, DecisionReason::DefaultDeny);
    assert!(
        recorded(&pool, sandbox_id).await.is_none(),
        "a refused caller gets no evidence row in another tenant's name"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The lookup that has to happen before a scope exists still works for the
/// least-privilege application role, with row-level security in force.
///
/// This is the property the production deployment depends on: `nestbox_app` is
/// not the table owner, so a read that ignored the tenant scope would return no
/// row and every decision would be a deny.
#[tokio::test]
async fn the_sandbox_lookup_works_for_the_application_role() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let sandbox_id = sandbox(&pool).await;
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL is set when connect succeeded");

    let options = match sqlx::postgres::PgConnectOptions::from_str(&url) {
        Ok(options) => options.username("nestbox_app"),
        Err(error) => {
            eprintln!("skipping: cannot parse TEST_DATABASE_URL: {error}");
            return;
        }
    };
    let app_pool = match sqlx::PgPool::connect_with(options).await {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!("skipping: the application role cannot connect here: {error}");
            return;
        }
    };

    let resolved =
        nestbox_control_api::store::postgres::egress::sandbox_scope(&app_pool, sandbox_id)
            .await
            .expect("scope lookup");
    let resolved = resolved.expect("the sandbox is visible through the scoped lookup");
    assert_eq!(resolved.environment_id, environment_id());
    assert_eq!(resolved.node_id, local_node());

    // And the same role cannot see the row without the scope, which is what
    // makes the scoped walk necessary rather than decorative.
    let unscoped: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sandboxes WHERE id = $1")
        .bind(sandbox_id)
        .fetch_one(&app_pool)
        .await
        .expect("count");
    assert_eq!(unscoped, 0, "row-level security must still hide the row");
}

/// The HTTP handler reads the facts from headers, refuses a bad token, and puts
/// the decision in the response headers Envoy reads.
#[tokio::test]
async fn the_handler_answers_from_headers_and_refuses_a_wrong_token() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let dir = secret_dir();
    let sandbox_id = sandbox(&pool).await;
    insert_rule(
        &pool,
        sandbox_id,
        "example.com",
        Some(443),
        "allow",
        "enforced",
    )
    .await;

    // The node's token: a 0600 file, referenced by a locator in the database.
    let token_file = dir.join("node-token");
    std::fs::write(&token_file, "node-secret-token\n").expect("write token");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_file, std::fs::Permissions::from_mode(0o600))
            .expect("chmod");
    }
    sqlx::query("UPDATE nodes SET node_token_ref = $1 WHERE id = $2")
        .bind(format!("file://{}", token_file.display()))
        .bind(local_node())
        .execute(&pool)
        .await
        .expect("set locator");

    let state = AppState::new(
        config(dir.clone()),
        Arc::new(FakeRuntime::new()),
        pool.clone(),
    );

    let request = |token: &str| {
        Request::builder()
            .method("GET")
            .uri("/api/v1/internal/policy/check/v1/chat/completions")
            .header("x-nestbox-node", LOCAL_NODE)
            .header("x-nestbox-sandbox", sandbox_id.to_string())
            .header("x-nestbox-host", "example.com:443")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(""))
            .expect("request")
    };

    // A wrong token is a 401, and nothing is recorded.
    let response = nestbox_control_api::app(state.clone())
        .oneshot(request("wrong-token"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(recorded(&pool, sandbox_id).await.is_none());

    // The right token gets an allow, with the path taken from the route suffix.
    let response = nestbox_control_api::app(state.clone())
        .oneshot(request("node-secret-token"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-nestbox-decision")
            .and_then(|value| value.to_str().ok()),
        Some("allow")
    );
    assert_eq!(
        response
            .headers()
            .get("x-nestbox-reason")
            .and_then(|value| value.to_str().ok()),
        Some("rule")
    );
    assert!(
        response.headers().get("x-nestbox-rule-id").is_some(),
        "the proxy logs which rule decided"
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(body["decision"], "allow");

    let path: String =
        sqlx::query_scalar("SELECT path FROM policy_checks WHERE sandbox_id = $1 LIMIT 1")
            .bind(sandbox_id)
            .fetch_one(&pool)
            .await
            .expect("row");
    assert_eq!(path, "/v1/chat/completions");

    // A host with no rule is a 403, which is how the ext_authz filter sees the
    // deny; the headers still say why.
    let denied = Request::builder()
        .method("POST")
        .uri("/api/v1/internal/policy/check/v1/messages")
        .header("x-nestbox-node", LOCAL_NODE)
        .header("x-nestbox-sandbox", sandbox_id.to_string())
        .header("x-nestbox-host", "elsewhere.example")
        .header("authorization", "Bearer node-secret-token")
        .body(Body::from(""))
        .expect("request");
    let response = nestbox_control_api::app(state)
        .oneshot(denied)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response
            .headers()
            .get("x-nestbox-decision")
            .and_then(|value| value.to_str().ok()),
        Some("deny")
    );
    assert_eq!(
        response
            .headers()
            .get("x-nestbox-reason")
            .and_then(|value| value.to_str().ok()),
        Some("default_deny")
    );

    let _ = std::fs::remove_dir_all(&dir);
}
