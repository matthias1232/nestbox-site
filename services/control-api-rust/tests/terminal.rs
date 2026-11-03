//! The terminal sessions against a real database.
//!
//! What a pure function cannot prove: a session row is written and closed with
//! a reason and byte counts and no content, the two quotas are counted from the
//! open rows, an abandoned row is swept instead of blocking a slot forever, and
//! the tenant policy from `0025` keeps another environment's sessions invisible.
//!
//! They run only when `TEST_DATABASE_URL` is set and own a database of their
//! own, so they can run next to the other suites:
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_terminal_test?host=/tmp/opencode/pgtest'
//!   cargo test --test terminal
//!
//! The tests share that database and one of them fills the user quota, so they
//! take a lock in turn rather than trusting `--test-threads=1` from outside.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::{Mutex, OnceCell};
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::runtime::SandboxSpec;
use nestbox_control_api::error::ErrorCode;
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::RuntimeAdapter;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::services::terminal_service::{
    self, ClientFrame, ClientSocket, Limits, ServerFrame,
};

/// The database this suite owns. Fixed so a failed run is replaced rather than
/// accumulated.
const DATABASE: &str = "nestbox_terminal_test";

/// The seeded administrator. The audit trail has a foreign key to `users`, so a
/// made-up actor id would turn a correct write into a 500.
const ADMIN: &str = "00000000-0000-0000-0000-000000000005";

/// The seeded node. Nodes are not environment-scoped: one machine serves every
/// environment, and a sandbox row needs one.
const NODE: &str = "00000000-0000-0000-0000-000000000003";

static READY: OnceCell<bool> = OnceCell::const_new();
static SERIAL: OnceCell<Mutex<()>> = OnceCell::const_new();

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

/// A pool for one test, or `None` when no database was configured.
async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?;
    sqlx::PgPool::connect_with(options.database(DATABASE)).await.ok()
}

/// One test at a time, on a clean slate.
///
/// The user quota is global for the seeded administrator and the tests share
/// one database, so a leftover open row from the previous test would decide the
/// next one. Sessions are deleted rather than the schema dropped: the audits
/// stay readable while a failure is being diagnosed.
async fn serial(pool: &PgPool) -> tokio::sync::MutexGuard<'static, ()> {
    let guard = SERIAL
        .get_or_init(|| async { Mutex::new(()) })
        .await
        .lock()
        .await;
    sqlx::query("DELETE FROM sandbox_sessions")
        .execute(pool)
        .await
        .expect("a clean slate");
    guard
}

async fn environment_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .expect("the default environment is seeded")
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

/// A context with a database and the in-memory runtime: no Docker involved.
fn context(pool: &PgPool, runtime: &Arc<FakeRuntime>) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        runtime.clone(),
        Arc::new(LocalBus::new()),
        std::path::PathBuf::from("/tmp/opencode"),
    )
}

/// A name unique to this test, so tests can share one database.
fn unique(prefix: &str) -> String {
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &suffix[..8])
}

/// A running sandbox row, which is what a terminal is allowed to attach to.
async fn running_sandbox(pool: &PgPool, environment_id: Uuid, name: &str) -> Uuid {
    // The node is global (one machine), the cluster is the environment's own.
    sqlx::query_scalar(
        "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, pack_key, image, status)
         VALUES ($1, (SELECT id FROM clusters WHERE environment_id = $1 LIMIT 1), $2, $3,
                 'ubuntu-dev', 'ubuntu:24.04', 'running')
         RETURNING id",
    )
    .bind(environment_id)
    .bind(Uuid::parse_str(NODE).expect("a uuid"))
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("a sandbox row")
}

fn spec(name: &str) -> SandboxSpec {
    SandboxSpec {
        name: name.to_string(),
        image: "ubuntu:24.04".into(),
        command: vec!["sleep".into(), "infinity".into()],
        environment: Vec::new(),
        pack_key: "ubuntu-dev".into(),
        tool_profile: "default".into(),
        memory_limit: "1Gi".into(),
        cpu_limit: 1.0,
        pids_limit: 256,
        mounts: Vec::new(),
        gpu_count: 0,
        egress_mode: "forward_proxy".into(),
        node_id: None,
        memory_reservation: None,
        cpu_shares: None,
    }
}

/// A browser that says nothing and remembers what it was told.
#[derive(Default)]
struct QuietBrowser {
    sent: Vec<ServerFrame>,
}

#[async_trait]
impl ClientSocket for QuietBrowser {
    async fn recv(&mut self) -> Option<ClientFrame> {
        // No input, no disconnect: only the idle clock can end this session.
        futures::future::pending().await
    }

    async fn send(&mut self, frame: ServerFrame) {
        self.sent.push(frame);
    }
}

#[tokio::test]
async fn a_session_is_recorded_and_closed_with_a_reason_and_no_content() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _guard = serial(&pool).await;
    let environment = environment_id(&pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let name = unique("terminal-record");
    let sandbox_id = running_sandbox(&pool, environment, &name).await;
    runtime.create(spec(&name)).await.expect("the fake sandbox");
    let ctx = context(&pool, &runtime);

    let ticket = terminal_service::open(
        &ctx,
        &principal(environment),
        sandbox_id,
        Limits {
            idle: Duration::from_millis(150),
            ..Limits::default()
        },
        String::new(),
    )
    .await
    .expect("a running sandbox accepts a terminal");

    // Something the shell printed, so the byte count is not trivially zero.
    // It has to arrive after `attach` has taken the channel, and before the idle
    // clock ends the session.
    let mut browser = QuietBrowser::default();
    let session = terminal_service::attach(&ctx, &ticket, &mut browser);
    let words = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        runtime.terminal_say("$ ls\r\n");
    };
    let (outcome, ()) = tokio::join!(session, words);
    let outcome = outcome.expect("the session runs to its end");
    assert_eq!(outcome.reason.as_str(), "idle");
    assert_eq!(outcome.bytes_out, 6);
    assert_eq!(outcome.bytes_in, 0);

    let row: (String, Option<String>, Option<chrono::DateTime<chrono::Utc>>, i64) = sqlx::query_as(
        "SELECT kind, close_reason, closed_at, bytes_out
           FROM sandbox_sessions WHERE id = $1",
    )
    .bind(ticket.session_id)
    .fetch_one(&pool)
    .await
    .expect("the session row");
    assert_eq!(row.0, "terminal");
    assert_eq!(row.1.as_deref(), Some("idle"));
    assert!(row.2.is_some(), "the row is closed");
    assert_eq!(row.3, 6);

    // Two events, identity and counts only. A keystroke here would undo ADR 0023.
    let details: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_events
          WHERE subject_id = $1 AND kind LIKE 'sandbox.terminal%'
          ORDER BY kind",
    )
    .bind(sandbox_id)
    .fetch_all(&pool)
    .await
    .expect("the audit events");
    assert_eq!(details.len(), 2, "one open, one close");
    for detail in &details {
        let text = detail.to_string();
        assert!(
            !text.contains("$ ls"),
            "the audit must not carry output: {text}"
        );
        assert!(
            detail.get("terminal_id").is_some(),
            "the session is referenced without a key the redactor masks: {detail}"
        );
    }
}

#[tokio::test]
async fn the_fifth_terminal_of_one_user_in_one_sandbox_is_refused() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _guard = serial(&pool).await;
    let environment = environment_id(&pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let name = unique("terminal-quota");
    let sandbox_id = running_sandbox(&pool, environment, &name).await;
    let ctx = context(&pool, &runtime);
    let principal = principal(environment);

    // One person may keep four shells here; the sandbox itself has room for more,
    // because an administrator joins the same machine (ADR 0056).
    for slot in 0..terminal_service::MAX_PER_USER_IN_SANDBOX {
        terminal_service::open(&ctx, &principal, sandbox_id, Limits::default(), String::new())
            .await
            .unwrap_or_else(|error| panic!("slot {slot} should be open: {}", error.message));
    }

    let refusal = terminal_service::open(&ctx, &principal, sandbox_id, Limits::default(), String::new())
        .await
        .expect_err("the fifth terminal of one user must be refused");
    assert_eq!(refusal.code, ErrorCode::SANDBOX_TERMINAL_LIMIT);
    assert_eq!(refusal.status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert!(
        refusal.message.contains("limit 4"),
        "the message names the limit: {}",
        refusal.message
    );
    assert_eq!(refusal.details["scope"], "user_in_sandbox");
    assert_eq!(refusal.details["open"], 4);
}

#[tokio::test]
async fn the_twenty_first_terminal_for_one_user_is_refused() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _guard = serial(&pool).await;
    let environment = environment_id(&pool).await;
    let user = Uuid::parse_str(ADMIN).expect("a uuid");

    // Twenty open rows spread over five sandboxes, four each: no sandbox is
    // full, so only the per-user limit can refuse the next one.
    for round in 0..5 {
        let sandbox_id = running_sandbox(&pool, environment, &unique("terminal-user")).await;
        for slot in 0..4 {
            sqlx::query(
                "INSERT INTO sandbox_sessions (environment_id, sandbox_id, user_id, kind, opened_at)
                 VALUES ($1, $2, $3, 'terminal', now())",
            )
            .bind(environment)
            .bind(sandbox_id)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap_or_else(|error| panic!("row {round}.{slot}: {error}"));
        }
    }

    let runtime = Arc::new(FakeRuntime::new());
    let target = running_sandbox(&pool, environment, &unique("terminal-target")).await;
    let ctx = context(&pool, &runtime);
    let refusal = terminal_service::open(&ctx, &principal(environment), target, Limits::default(), String::new())
        .await
        .expect_err("the twenty-first terminal must be refused");
    assert_eq!(refusal.code, ErrorCode::SANDBOX_TERMINAL_LIMIT);
    assert_eq!(refusal.details["scope"], "user");
    assert_eq!(refusal.details["limit"], 20);
}

#[tokio::test]
async fn an_abandoned_row_is_swept_instead_of_blocking_a_slot() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _guard = serial(&pool).await;
    let environment = environment_id(&pool).await;
    let runtime = Arc::new(FakeRuntime::new());
    let name = unique("terminal-stale");
    let sandbox_id = running_sandbox(&pool, environment, &name).await;
    let ctx = context(&pool, &runtime);

    // A row from a process that died: older than any session can live.
    let abandoned: Uuid = sqlx::query_scalar(
        "INSERT INTO sandbox_sessions (environment_id, sandbox_id, user_id, kind, opened_at)
         VALUES ($1, $2, $3, 'terminal', now() - interval '2 hours')
         RETURNING id",
    )
    .bind(environment)
    .bind(sandbox_id)
    .bind(Uuid::parse_str(ADMIN).expect("a uuid"))
    .fetch_one(&pool)
    .await
    .expect("an abandoned row");

    // The next terminal sweeps it and is accepted, rather than being refused by
    // a ghost that would otherwise hold a slot for good.
    terminal_service::open(&ctx, &principal(environment), sandbox_id, Limits::default(), String::new())
        .await
        .expect("the abandoned row must not block the slot");

    let swept: Option<String> =
        sqlx::query_scalar("SELECT close_reason FROM sandbox_sessions WHERE id = $1")
            .bind(abandoned)
            .fetch_one(&pool)
            .await
            .expect("the swept row");
    assert_eq!(swept.as_deref(), Some("error"));
}

#[tokio::test]
async fn another_environment_cannot_read_the_sessions() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _guard = serial(&pool).await;
    let environment = environment_id(&pool).await;

    // A second tenant with a session of its own, written as the owner.
    let other: Uuid = sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug)
         VALUES ('00000000-0000-0000-0000-000000000001', 'other', $1)
         RETURNING id",
    )
    .bind(unique("other-env"))
    .fetch_one(&pool)
    .await
    .expect("a second environment");
    sqlx::query(
        "INSERT INTO clusters (environment_id, name, slug, placement_policy)
         VALUES ($1, 'other', 'other', 'least_loaded')",
    )
    .bind(other)
    .execute(&pool)
    .await
    .expect("a second cluster");
    let foreign = running_sandbox(&pool, other, &unique("terminal-foreign")).await;
    sqlx::query(
        "INSERT INTO sandbox_sessions (environment_id, sandbox_id, user_id, kind)
         VALUES ($1, $2, $3, 'terminal')",
    )
    .bind(other)
    .bind(foreign)
    .bind(Uuid::parse_str(ADMIN).expect("a uuid"))
    .execute(&pool)
    .await
    .expect("a foreign session");

    // The application role does not own the table, so the policy applies to it.
    let mut tx = pool.begin().await.expect("tx");
    sqlx::query("SELECT set_config('app.environment_id', $1, true)")
        .bind(environment.to_string())
        .execute(&mut *tx)
        .await
        .expect("scope the transaction");
    sqlx::query("SET LOCAL ROLE nestbox_app")
        .execute(&mut *tx)
        .await
        .expect("switch to the application role");
    let visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sandbox_sessions WHERE sandbox_id = $1")
            .bind(foreign)
            .fetch_one(&mut *tx)
            .await
            .expect("count in scope");
    assert_eq!(
        visible, 0,
        "another environment's session must be invisible to the application role"
    );
}
