//! Agent sign-in modes against a real database.
//!
//! These prove the parts a pure function cannot: that a sandbox's old
//! `login: own_account` choice is the default mode of every agent its pack
//! carries, that a switch is stored with or without a waiting restart, and that
//! the per-sandbox firewall rules that open an agent's identity hosts appear and
//! disappear with the mode instead of piling up.
//!
//! The suite owns `nestbox_wb_b02`, so it can run next to the other integration
//! suites without any of them dropping another's schema:
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/postgres?host=/tmp/opencode/pgtest'
//!   cargo test --test agent_auth

use std::str::FromStr;
use std::sync::Arc;

use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::agent_auth::{AgentAuthMode, AgentKind, RestartPolicy};
use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::runtime::SandboxSpec;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::runtime::RuntimeAdapter;
use nestbox_control_api::services::agent_auth_service;
use nestbox_control_api::services::ServiceContext;

/// The database this suite owns.
const DATABASE: &str = "nestbox_wb_b02";

/// The seeded administrator; the audit trail has a foreign key to `users`.
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

/// A pool for one test.
async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?;
    sqlx::PgPool::connect_with(options.database(DATABASE))
        .await
        .ok()
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

/// A name unique to this test, so tests can share one database.
fn unique(prefix: &str) -> String {
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &suffix[..8])
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

/// A sandbox that exists both in the database and in the fake runner.
///
/// Both halves matter after the merge: the store resolves the sandbox row, and
/// the launcher and the activity reader talk to the runtime, which refuses a
/// name it never created.
async fn sandbox(
    pool: &PgPool,
    runtime: &FakeRuntime,
    environment_id: Uuid,
    cluster_id: Uuid,
    node_id: Uuid,
    pack_key: &str,
    spec: serde_json::Value,
) -> Uuid {
    let name = unique(pack_key);
    runtime
        .create(SandboxSpec {
            name: name.clone(),
            image: format!("ghcr.io/nestbox/{pack_key}@sha256:0123456789abcdef"),
            command: vec!["sleep".into(), "infinity".into()],
            environment: Vec::new(),
            pack_key: pack_key.to_string(),
            tool_profile: "base".into(),
            memory_limit: "1Gi".into(),
            cpu_limit: 1.0,
            pids_limit: 256,
            mounts: Vec::new(),
            gpu_count: 0,
            egress_mode: "forward_proxy".into(),
            node_id: None,
            memory_reservation: None,
            cpu_shares: None,
        })
        .await
        .expect("the fake runner creates a sandbox");
    sqlx::query_scalar(
        "INSERT INTO sandboxes
            (environment_id, cluster_id, node_id, name, pack_key, image, spec, status)
         VALUES ($1, $2, $3, $4, $5, 'ghcr.io/nestbox/pack@sha256:0123456789abcdef', $6, 'running')
         RETURNING id",
    )
    .bind(environment_id)
    .bind(cluster_id)
    .bind(node_id)
    .bind(&name)
    .bind(pack_key)
    .bind(spec)
    .fetch_one(pool)
    .await
    .expect("insert sandbox")
}

/// The sign-in rules of one sandbox, by name.
async fn sign_in_rules(pool: &PgPool, sandbox_id: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT name FROM network_rules
          WHERE sandbox_id = $1 AND name LIKE 'Sign-in (%' ORDER BY name",
    )
    .bind(sandbox_id)
    .fetch_all(pool)
    .await
    .expect("read the sign-in rules")
}

/// A claude-code sandbox keeps the old own-account choice for both of its agents,
/// while an opencode sandbox has nothing but its gateway login.
#[tokio::test]
async fn the_old_login_choice_is_the_default_mode_of_each_agent() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let who = principal(environment);
    let cluster_id = cluster(&pool, environment, &unique("cluster")).await;
    let node_id = node(&pool, &unique("node")).await;

    let claude_code = sandbox(
        &pool,
        &runtime,
        environment,
        cluster_id,
        node_id,
        "claude-code",
        json!({ "login": "own_account" }),
    )
    .await;
    let agents = agent_auth_service::list(&ctx, &who, claude_code)
        .await
        .expect("the agents are listed");
    let modes: Vec<(AgentKind, AgentAuthMode)> =
        agents.iter().map(|agent| (agent.agent, agent.mode)).collect();
    assert!(modes.contains(&(AgentKind::Claude, AgentAuthMode::Subscription)));
    assert!(modes.contains(&(AgentKind::Codex, AgentAuthMode::Subscription)));
    assert!(!modes.iter().any(|(agent, _)| *agent == AgentKind::Opencode));

    let opencode = sandbox(
        &pool,
        &runtime,
        environment,
        cluster_id,
        node_id,
        "opencode",
        json!({}),
    )
    .await;
    let agents = agent_auth_service::list(&ctx, &who, opencode)
        .await
        .expect("the agents are listed");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent, AgentKind::Opencode);
    assert_eq!(agents[0].mode, AgentAuthMode::Gateway);
    assert!(!agents[0].pending_restart);
}

/// A switch that is not allowed to restart yet is stored as waiting, so the
/// background loop picks it up once the agent is idle.
#[tokio::test]
async fn a_later_switch_is_stored_as_waiting() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let who = principal(environment);
    let cluster_id = cluster(&pool, environment, &unique("cluster")).await;
    let node_id = node(&pool, &unique("node")).await;
    let sandbox_id = sandbox(
        &pool,
        &runtime,
        environment,
        cluster_id,
        node_id,
        "claude-code",
        json!({ "login": "own_account" }),
    )
    .await;

    let agents = agent_auth_service::set(
        &ctx,
        &who,
        sandbox_id,
        AgentKind::Claude,
        AgentAuthMode::Gateway,
        RestartPolicy::Later,
    )
    .await
    .expect("the mode is switched");
    let claude = agents
        .iter()
        .find(|agent| agent.agent == AgentKind::Claude)
        .expect("claude is listed");
    assert_eq!(claude.mode, AgentAuthMode::Gateway);
    assert!(claude.pending_restart);

    let stored: (String, bool) = sqlx::query_as(
        "SELECT mode, pending_restart FROM sandbox_agent_auth
          WHERE sandbox_id = $1 AND agent = 'claude'",
    )
    .bind(sandbox_id)
    .fetch_one(&pool)
    .await
    .expect("the choice is stored");
    assert_eq!(stored.0, "gateway");
    assert!(stored.1);
}

/// A restart never changes the mode; it only marks the agent as waiting when the
/// operator asked for it later.
#[tokio::test]
async fn a_restart_leaves_the_mode_alone() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let who = principal(environment);
    let cluster_id = cluster(&pool, environment, &unique("cluster")).await;
    let node_id = node(&pool, &unique("node")).await;
    let sandbox_id = sandbox(
        &pool,
        &runtime,
        environment,
        cluster_id,
        node_id,
        "opencode",
        json!({}),
    )
    .await;

    let agents = agent_auth_service::restart(
        &ctx,
        &who,
        sandbox_id,
        AgentKind::Opencode,
        RestartPolicy::Later,
    )
    .await
    .expect("the restart is accepted");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].mode, AgentAuthMode::Gateway);
    assert!(agents[0].pending_restart);
}

/// The identity hosts of an agent are allowed while it signs in with its own
/// account and taken away again when it goes back to the gateway.
#[tokio::test]
async fn the_sign_in_rules_follow_the_mode() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let who = principal(environment);
    let cluster_id = cluster(&pool, environment, &unique("cluster")).await;
    let node_id = node(&pool, &unique("node")).await;
    let sandbox_id = sandbox(
        &pool,
        &runtime,
        environment,
        cluster_id,
        node_id,
        "claude-code",
        json!({ "login": "own_account" }),
    )
    .await;

    agent_auth_service::set(
        &ctx,
        &who,
        sandbox_id,
        AgentKind::Claude,
        AgentAuthMode::Subscription,
        RestartPolicy::Later,
    )
    .await
    .expect("the mode is switched to the subscription");
    let rules = sign_in_rules(&pool, sandbox_id).await;
    assert!(
        rules.iter().any(|name| name == "Sign-in (claude): claude.ai"),
        "the claude identity host is allowed: {rules:?}"
    );
    assert!(
        rules.iter().any(|name| name == "Sign-in (claude): statsig.anthropic.com"),
        "every claude host is allowed: {rules:?}"
    );

    // The rule is a real rule, not an empty shell: host, port and the matcher
    // the decider reads are all filled.
    let (host, port, matchers): (String, Option<i32>, serde_json::Value) = sqlx::query_as(
        "SELECT host, port, matchers FROM network_rules
          WHERE sandbox_id = $1 AND name = 'Sign-in (claude): claude.ai'",
    )
    .bind(sandbox_id)
    .fetch_one(&pool)
    .await
    .expect("the rule is stored");
    assert_eq!(host, "claude.ai");
    assert_eq!(port, Some(443));
    assert!(matchers.to_string().contains("claude.ai"), "{matchers}");

    agent_auth_service::set(
        &ctx,
        &who,
        sandbox_id,
        AgentKind::Claude,
        AgentAuthMode::Gateway,
        RestartPolicy::Later,
    )
    .await
    .expect("the mode is switched back to the gateway");
    let rules = sign_in_rules(&pool, sandbox_id).await;
    assert!(
        !rules.iter().any(|name| name.starts_with("Sign-in (claude)")),
        "no claude host is allowed any more: {rules:?}"
    );
    assert!(
        rules.iter().any(|name| name.starts_with("Sign-in (codex)")),
        "codex still signs in with the own account: {rules:?}"
    );
}
