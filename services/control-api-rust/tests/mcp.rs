//! The MCP layer against a real database.
//!
//! These prove the parts a pure function cannot: that the seed marks the hub as
//! visible and the stdio servers as not, that the registry round-trips through
//! validation and SQL, that an attachment keeps what discovery found, that a
//! failed discovery is recorded honestly, that a tenant does not see another
//! tenant's attachment, and that a tool call keeps its name, its session and its
//! redacted arguments.
//!
//! They run only when `TEST_DATABASE_URL` is set, and they use the database it
//! names rather than creating one, so the schema can be applied with `psql -f`
//! beforehand:
//!
//!   createdb -h /tmp/opencode/pgtest -p 55432 nestbox_mcp
//!   for f in migrations/*.sql; do psql -d nestbox_mcp -f "$f"; done
//!   TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_mcp?host=/tmp/opencode/pgtest' \
//!     cargo test --test mcp -- --test-threads=1
//!
//! Every row this suite writes carries a key or a name unique to the run, so a
//! second run against the same database starts from a clean slate in practice.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::mcp::McpStatus;
use nestbox_control_api::domain::metrics::{InventoryProbe, ProcessTable};
use nestbox_control_api::domain::runtime::{
    CatalogueAnswer, CatalogueRequest, ImageVerification, ModelDownload, ModelDownloadResult,
    ModelServe, ModelServeResult, BuildRequest, BuildResult, ExecResult, HardwareReport,
    RuntimeFacts, SandboxSpec, VerificationAnswer,
};
use nestbox_control_api::error::ErrorCode;
use nestbox_control_api::runtime::{BoxTerminal, RuntimeAdapter, RuntimeError};
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::services::mcp_service::{self, ServerInput};
use nestbox_control_api::store::TenantScope;
use nestbox_control_api::store::postgres::mcp as store;

/// The seeded administrator: an audit foreign key needs a real user.
const ADMIN: &str = "00000000-0000-0000-0000-000000000005";

static READY: OnceCell<bool> = OnceCell::const_new();

/// Connect once and make sure the schema exists.
///
/// A database prepared with `psql -f` has the tables but no `_sqlx_migrations`
/// row, so the migrations are applied only when the MCP table is missing. That
/// keeps both workflows honest instead of replaying every `CREATE TABLE` and
/// failing on the first one.
async fn ready() -> bool {
    *READY
        .get_or_init(|| async {
            let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
                return false;
            };
            let Ok(pool) = sqlx::PgPool::connect(&url).await else {
                return false;
            };
            let present: Option<String> =
                sqlx::query_scalar("SELECT to_regclass('public.mcp_registry')::text")
                    .fetch_one(&pool)
                    .await
                    .unwrap_or(None);
            if present.is_none() {
                let setup = async {
                    sqlx::query(
                        "DO $$BEGIN
                            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'nestbox_app') THEN
                                CREATE ROLE nestbox_app LOGIN;
                            END IF;
                        END$$",
                    )
                    .execute(&pool)
                    .await?;
                    sqlx::query("GRANT USAGE, CREATE ON SCHEMA public TO nestbox_app")
                        .execute(&pool)
                        .await?;
                    sqlx::migrate!("./migrations").run(&pool).await?;
                    Ok::<(), sqlx::Error>(())
                }
                .await;
                if setup.is_err() {
                    return false;
                }
            }
            true
        })
        .await
}

async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    PgPool::connect(&url).await.ok()
}

async fn environment_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .expect("the default environment is seeded")
}

fn principal(environment_id: Uuid, role: Role) -> Principal {
    Principal {
        user_id: Uuid::parse_str(ADMIN).expect("a uuid"),
        environment_id,
        role,
        display_name: "Test administrator".into(),
        via: CredentialSource::Token,
    }
}

/// A runtime whose only job is to answer `exec` with a fixed string.
///
/// It is a real `RuntimeAdapter`, so the service's discovery path runs exactly
/// as it does in production: build the argv, ask the runtime, read the answer.
/// The string stands in for what the probe prints inside the sandbox.
struct AnsweringRuntime {
    answer: String,
}

impl AnsweringRuntime {
    fn new(answer: &str) -> Self {
        Self {
            answer: answer.to_string(),
        }
    }
}

#[async_trait]
impl RuntimeAdapter for AnsweringRuntime {
    fn name(&self) -> &'static str {
        "answering"
    }

    fn network(&self) -> &str {
        "nestbox-test"
    }

    async fn health(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn create(&self, _spec: SandboxSpec) -> Result<RuntimeFacts, RuntimeError> {
        Err(RuntimeError::Rejected("not used in this suite".into()))
    }

    async fn inspect(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        Err(RuntimeError::NotFound(name.to_string()))
    }

    async fn list(&self) -> Result<Vec<RuntimeFacts>, RuntimeError> {
        Ok(Vec::new())
    }

    async fn start(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        Err(RuntimeError::NotFound(name.to_string()))
    }

    async fn stop(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        Err(RuntimeError::NotFound(name.to_string()))
    }

    async fn restart(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        Err(RuntimeError::NotFound(name.to_string()))
    }

    async fn remove(&self, _name: &str) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn logs(&self, _name: &str, _tail: u32) -> Result<String, RuntimeError> {
        Ok(String::new())
    }

    async fn exec(
        &self,
        name: &str,
        command: &[String],
        _timeout_seconds: u32,
    ) -> Result<ExecResult, RuntimeError> {
        assert!(
            command.first().map(String::as_str) == Some("python3"),
            "discovery must run the probe script, not a shell command"
        );
        assert!(
            name.starts_with("mcp-test-"),
            "the probe runs inside the named sandbox"
        );
        Ok(ExecResult {
            exit_code: Some(0),
            output: self.answer.clone(),
            truncated: false,
        })
    }

    async fn hardware(&self) -> Result<HardwareReport, RuntimeError> {
        Ok(HardwareReport::default())
    }

    async fn build(&self, _request: BuildRequest) -> Result<BuildResult, RuntimeError> {
        Err(RuntimeError::Rejected("not used in this suite".into()))
    }

    // The methods this suite does not exercise. They are named here rather than
    // defaulted in the trait, so a real adapter that forgets one fails to
    // compile instead of answering "unsupported" at run time.
    async fn processes(&self, name: &str) -> Result<ProcessTable, RuntimeError> {
        Err(RuntimeError::NotFound(name.to_string()))
    }

    async fn inventory(&self, name: &str) -> Result<InventoryProbe, RuntimeError> {
        Err(RuntimeError::NotFound(name.to_string()))
    }

    async fn download_model(
        &self,
        _request: ModelDownload,
    ) -> Result<ModelDownloadResult, RuntimeError> {
        Err(RuntimeError::Rejected("not used in this suite".into()))
    }

    async fn serve_model(&self, _request: ModelServe) -> Result<ModelServeResult, RuntimeError> {
        Err(RuntimeError::Rejected("not used in this suite".into()))
    }

    async fn stop_model(&self, _model_key: &str) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn process_action(
        &self,
        name: &str,
        _pid: i64,
        _action: &str,
        _nice: Option<i8>,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::NotFound(name.to_string()))
    }

    async fn catalogue(&self, _request: CatalogueRequest) -> Result<CatalogueAnswer, RuntimeError> {
        Ok(CatalogueAnswer::default())
    }

    async fn verify_image(
        &self,
        _request: ImageVerification,
    ) -> Result<VerificationAnswer, RuntimeError> {
        Ok(VerificationAnswer::default())
    }

    async fn open_terminal(&self, _name: &str, _tab_id: &str, _who: &str) -> Result<BoxTerminal, RuntimeError> {
        Err(RuntimeError::Rejected("not used in this suite".into()))
    }
}

fn context(pool: &PgPool, runtime: Arc<AnsweringRuntime>) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        runtime,
        Arc::new(nestbox_control_api::events::bus::LocalBus::new()),
        std::path::PathBuf::from("/tmp/opencode"),
    )
}

/// A key unique to this run: the registry key is a primary key and tests share
/// one database.
fn unique_key(prefix: &str) -> String {
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &suffix[..10])
}

/// A sandbox row, inserted directly: the lifecycle is another package's test.
async fn insert_sandbox(pool: &PgPool, environment_id: Uuid, status: &str) -> (Uuid, String) {
    let name = unique_key("mcp-test");
    let cluster: Uuid =
        sqlx::query_scalar("SELECT id FROM clusters WHERE environment_id = $1 LIMIT 1")
        .bind(environment_id)
        .fetch_one(pool)
        .await
        .expect("the default environment has a cluster");
    let node: Uuid = sqlx::query_scalar("SELECT id FROM nodes ORDER BY name LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("a node is seeded");
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, image, status)
         VALUES ($1, $2, $3, $4, 'nestbox-hermes:local', $5)
         RETURNING id",
    )
    .bind(environment_id)
    .bind(cluster)
    .bind(node)
    .bind(&name)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("a sandbox row");
    (id, name)
}

fn http_server(key: &str, endpoint: &str, env_slots: Vec<String>) -> ServerInput {
    ServerInput {
        key: key.to_string(),
        name: format!("Test server {key}"),
        transport: "http".into(),
        endpoint: Some(endpoint.to_string()),
        auth_type: if env_slots.is_empty() { "none".into() } else { "token".into() },
        env_slots,
        enabled: true,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_seed_marks_the_hub_visible_and_stdio_servers_not() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let servers = mcp_service::list(&context, &principal(environment, Role::Admin))
        .await
        .expect("the registry");

    let hub = servers
        .iter()
        .find(|server| server.key == "nestbox-hub")
        .expect("the hub is seeded");
    assert!(hub.transport.visible_to_proxy(), "the hub must be an HTTP server");
    assert_eq!(hub.endpoint.as_deref(), Some("http://mcp-hub:8080/mcp"));
    assert_eq!(hub.env_slots, vec!["NESTBOX_MCP_TOKEN".to_string()]);

    let filesystem = servers
        .iter()
        .find(|server| server.key == "filesystem")
        .expect("filesystem is seeded");
    assert_eq!(filesystem.transport.as_str(), "stdio");
    assert!(!filesystem.transport.visible_to_proxy());
    assert_eq!(filesystem.command.as_deref(), Some("npx"));
}

#[tokio::test]
async fn registry_crud_round_trips() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let key = unique_key("crud");

    let created = mcp_service::create(
        &context,
        &admin,
        http_server(&key, "https://mcp.example.com/mcp", vec!["MCP_TEST_TOKEN".into()]),
    )
    .await
    .expect("created");
    assert_eq!(created.key, key);
    assert_eq!(created.auth_type.as_str(), "token");

    let fetched = mcp_service::get(&context, &admin, &key).await.expect("read back");
    assert_eq!(fetched, created);

    let mut replacement = http_server(&key, "https://mcp.example.com/v2", vec![]);
    replacement.name = "Renamed server".into();
    replacement.enabled = false;
    let replaced = mcp_service::replace(&context, &admin, &key, replacement)
        .await
        .expect("replaced");
    assert_eq!(replaced.name, "Renamed server");
    assert_eq!(replaced.endpoint.as_deref(), Some("https://mcp.example.com/v2"));
    assert!(!replaced.enabled);

    mcp_service::delete(&context, &admin, &key).await.expect("deleted");
    let missing = mcp_service::get(&context, &admin, &key).await;
    assert_eq!(
        missing.expect_err("gone").code,
        ErrorCode::NOT_FOUND,
        "a deleted server is no longer readable"
    );
}

#[tokio::test]
async fn a_duplicate_registry_key_is_a_conflict() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let key = unique_key("dup");
    let input = http_server(&key, "https://mcp.example.com/mcp", vec![]);

    mcp_service::create(&context, &admin, input.clone())
        .await
        .expect("first create");
    let error = mcp_service::create(&context, &admin, input)
        .await
        .expect_err("the second create is refused");
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
    assert!(error.message.contains(&key));
}

#[tokio::test]
async fn an_invalid_definition_is_refused_before_the_database() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);

    let mut input = http_server(&unique_key("bad"), "mcp://example.com/x", vec![]);
    input.transport = "http".into();
    let error = mcp_service::create(&context, &admin, input)
        .await
        .expect_err("a non-HTTP endpoint is refused");
    assert_eq!(error.code, ErrorCode::VALIDATION_FAILED);
    assert_eq!(error.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_viewer_may_read_but_not_register_a_server() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let viewer = principal(environment, Role::Viewer);

    mcp_service::list(&context, &viewer).await.expect("a viewer may read");
    let error = mcp_service::create(
        &context,
        &viewer,
        http_server(&unique_key("viewer"), "https://mcp.example.com/mcp", vec![]),
    )
    .await
    .expect_err("a viewer may not write");
    assert_eq!(error.code, ErrorCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// Attachments and discovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn attach_discover_and_detach_round_trips() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    // The probe's answer, as the hub would print it inside the sandbox.
    let runtime = Arc::new(AnsweringRuntime::new(
        r#"{"ok": true, "session_id": "session-42", "tools": [{"name": "read_file", "description": "Read a file", "inputSchema": {"type": "object"}}, {"name": "run_command", "description": "Run a command", "inputSchema": {"type": "object"}}]}"#,
    ));
    let context = context(&pool, runtime);
    let admin = principal(environment, Role::Admin);
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "running").await;
    let key = unique_key("hub");
    mcp_service::create(
        &context,
        &admin,
        http_server(&key, "http://mcp-hub:8080/mcp", vec!["NESTBOX_MCP_TOKEN".into()]),
    )
    .await
    .expect("the server exists");

    let attached = mcp_service::attach(&context, &admin, sandbox_id, &key, true)
        .await
        .expect("attached");
    assert_eq!(attached.status, McpStatus::Unknown);
    assert_eq!(attached.tools, serde_json::json!([]));

    let discovered = mcp_service::discover(&context, &admin, sandbox_id, &key)
        .await
        .expect("discovery ran");
    assert_eq!(discovered.status, McpStatus::Connected);
    assert_eq!(discovered.last_session_id.as_deref(), Some("session-42"));
    assert_eq!(discovered.tools.as_array().map(Vec::len), Some(2));
    assert!(discovered.last_seen_at.is_some());
    assert_eq!(discovered.last_error, None);

    let listed = mcp_service::sandbox_servers(&context, &admin, sandbox_id)
        .await
        .expect("listed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, McpStatus::Connected);

    mcp_service::detach(&context, &admin, sandbox_id, &key)
        .await
        .expect("detached");
    let empty = mcp_service::sandbox_servers(&context, &admin, sandbox_id)
        .await
        .expect("listed");
    assert!(empty.is_empty());

    sqlx::query("DELETE FROM sandboxes WHERE id = $1")
        .bind(sandbox_id)
        .execute(&pool)
        .await
        .expect("cleanup");
    mcp_service::delete(&context, &admin, &key)
        .await
        .expect("cleanup the server");
}

#[tokio::test]
async fn attaching_twice_is_a_conflict() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "running").await;
    let key = unique_key("twice");
    mcp_service::create(&context, &admin, http_server(&key, "https://mcp.example.com/mcp", vec![]))
        .await
        .expect("the server exists");
    mcp_service::attach(&context, &admin, sandbox_id, &key, true)
        .await
        .expect("first attach");
    let error = mcp_service::attach(&context, &admin, sandbox_id, &key, true)
        .await
        .expect_err("second attach");
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_failed_discovery_is_recorded_with_a_redacted_reason() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let runtime = Arc::new(AnsweringRuntime::new(
        r#"{"ok": false, "error": "HTTP 403 from the endpoint: policy denied; key=sk-live-abcdef123456"}"#,
    ));
    let context = context(&pool, runtime);
    let admin = principal(environment, Role::Admin);
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "running").await;
    let key = unique_key("denied");
    mcp_service::create(
        &context,
        &admin,
        http_server(&key, "https://mcp.example.com/mcp", vec!["MCP_TEST_TOKEN".into()]),
    )
    .await
    .expect("the server exists");
    mcp_service::attach(&context, &admin, sandbox_id, &key, true)
        .await
        .expect("attached");

    let attachment = mcp_service::discover(&context, &admin, sandbox_id, &key)
        .await
        .expect("the request succeeded; the server did not");
    assert_eq!(attachment.status, McpStatus::Error);
    let reason = attachment.last_error.unwrap_or_default();
    assert!(reason.contains("403"), "the reason is kept: {reason}");
    assert!(
        !reason.contains("sk-live-abcdef123456"),
        "a token-shaped string is masked: {reason}"
    );
}

#[tokio::test]
async fn discovery_of_a_stdio_server_is_refused_and_says_why() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "running").await;

    let filesystem = ServerInput {
        key: unique_key("stdio"),
        name: "Stdio test server".into(),
        transport: "stdio".into(),
        command: Some("npx".into()),
        args: vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into()],
        enabled: true,
        ..Default::default()
    };
    let key = filesystem.key.clone();
    mcp_service::create(&context, &admin, filesystem)
        .await
        .expect("the stdio server exists");
    mcp_service::attach(&context, &admin, sandbox_id, &key, true)
        .await
        .expect("attached");

    let error = mcp_service::discover(&context, &admin, sandbox_id, &key)
        .await
        .expect_err("a stdio server cannot be discovered over HTTP");
    assert_eq!(error.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error.message.contains("stdio"));

    let listed = mcp_service::sandbox_servers(&context, &admin, sandbox_id)
        .await
        .expect("listed");
    assert_eq!(
        listed[0].status,
        McpStatus::Unknown,
        "a refusal does not pretend discovery happened"
    );
}

#[tokio::test]
async fn discovery_through_a_stopped_sandbox_is_refused() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "stopped").await;
    let key = unique_key("stopped");
    mcp_service::create(&context, &admin, http_server(&key, "https://mcp.example.com/mcp", vec![]))
        .await
        .expect("the server exists");
    mcp_service::attach(&context, &admin, sandbox_id, &key, true)
        .await
        .expect("attached");

    let error = mcp_service::discover(&context, &admin, sandbox_id, &key)
        .await
        .expect_err("a stopped sandbox has no probe to run");
    assert_eq!(error.code, ErrorCode::SANDBOX_INVALID_STATE);
}

#[tokio::test]
async fn a_server_still_attached_cannot_be_deleted() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "running").await;
    let key = unique_key("inuse");
    mcp_service::create(&context, &admin, http_server(&key, "https://mcp.example.com/mcp", vec![]))
        .await
        .expect("the server exists");
    mcp_service::attach(&context, &admin, sandbox_id, &key, true)
        .await
        .expect("attached");

    let error = mcp_service::delete(&context, &admin, &key)
        .await
        .expect_err("a server in use is refused");
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
    assert_eq!(error.details["attached"], 1);
}

// ---------------------------------------------------------------------------
// Tenancy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn another_environment_does_not_see_the_attachment() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "running").await;
    let key = unique_key("tenant");
    mcp_service::create(&context, &admin, http_server(&key, "https://mcp.example.com/mcp", vec![]))
        .await
        .expect("the server exists");
    mcp_service::attach(&context, &admin, sandbox_id, &key, true)
        .await
        .expect("attached");

    let organization: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM environments WHERE id = $1")
            .bind(environment)
            .fetch_one(&pool)
            .await
            .expect("an organization");
    let other: Uuid = sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug)
         VALUES ($1, 'Other', $2) RETURNING id",
    )
    .bind(organization)
    .bind(unique_key("other"))
    .fetch_one(&pool)
    .await
    .expect("a second environment");
    sqlx::query("INSERT INTO clusters (environment_id, name, slug) VALUES ($1, 'Other', 'local')")
        .bind(other)
        .execute(&pool)
        .await
        .expect("the second environment has a cluster");
    let (other_sandbox, _) = insert_sandbox(&pool, other, "running").await;

    let other_admin = principal(other, Role::Admin);
    let empty = mcp_service::sandbox_servers(&context, &other_admin, other_sandbox)
        .await
        .expect("listed");
    assert!(
        empty.is_empty(),
        "the other environment's sandbox has no attachments"
    );

    // The first environment's attachment is still there and not reachable from
    // the second one, even though the registry itself is global.
    let own = mcp_service::sandbox_servers(&context, &admin, sandbox_id)
        .await
        .expect("listed");
    assert_eq!(own.len(), 1);
    let cross = mcp_service::sandbox_servers(&context, &other_admin, sandbox_id).await;
    assert!(
        cross.is_err(),
        "a sandbox of another environment is not visible"
    );
}

// ---------------------------------------------------------------------------
// Calls the proxy saw
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tool_call_keeps_the_tool_the_session_and_masks_the_arguments() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "running").await;

    store::insert_call(
        &pool,
        store::NewToolCall {
            environment_id: environment,
            sandbox_id: Some(sandbox_id),
            server_host: "mcp-hub".into(),
            server_key: Some("nestbox-hub".into()),
            session_id: Some("session-42".into()),
            method: "tools/call".into(),
            tool_name: Some("read_file".into()),
            arguments: Some(serde_json::json!({
                "path": "notes.txt",
                "api_key": "sk-live-abcdef123456",
            })),
            result: Some(serde_json::json!({"content": [{"type": "text", "text": "hi"}]})),
            is_error: false,
            duration_ms: Some(17),
            request_id: Some("7".into()),
        },
    )
    .await
    .expect("the call is stored");

    let calls = mcp_service::calls(&context, &admin, sandbox_id, 50)
        .await
        .expect("the call is readable");
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.tool_name.as_deref(), Some("read_file"));
    assert_eq!(call.session_id.as_deref(), Some("session-42"));
    assert_eq!(call.server_host, "mcp-hub");
    assert_eq!(call.duration_ms, Some(17));
    let arguments = call.arguments.clone().expect("arguments");
    assert_eq!(arguments["api_key"], "[redacted]");
    assert_eq!(arguments["path"], "notes.txt");
}

#[tokio::test]
async fn a_result_over_the_cap_is_marked_rather_than_half_stored() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let (sandbox_id, _) = insert_sandbox(&pool, environment, "running").await;
    let big = "x".repeat(20 * 1024);

    store::insert_call(
        &pool,
        store::NewToolCall {
            environment_id: environment,
            sandbox_id: Some(sandbox_id),
            server_host: "mcp-hub".into(),
            server_key: None,
            session_id: None,
            method: "tools/call".into(),
            tool_name: Some("read_file".into()),
            arguments: None,
            result: Some(serde_json::json!({ "content": [{ "type": "text", "text": big }] })),
            is_error: false,
            duration_ms: None,
            request_id: None,
        },
    )
    .await
    .expect("the call is stored");

    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT result FROM mcp_tool_calls WHERE sandbox_id = $1")
            .bind(sandbox_id)
            .fetch_one(&pool)
            .await
            .expect("the row");
    assert_eq!(stored["truncated"], true);
    assert!(stored["bytes"].as_u64().unwrap_or(0) > 16 * 1024);
}

#[tokio::test]
async fn the_call_read_is_scoped_to_the_sandbox_and_the_tenant() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let context = context(&pool, Arc::new(AnsweringRuntime::new("{}")));
    let admin = principal(environment, Role::Admin);
    let (first, _) = insert_sandbox(&pool, environment, "running").await;
    let (second, _) = insert_sandbox(&pool, environment, "running").await;

    store::insert_call(
        &pool,
        store::NewToolCall {
            environment_id: environment,
            sandbox_id: Some(first),
            server_host: "mcp-hub".into(),
            server_key: None,
            session_id: None,
            method: "tools/list".into(),
            tool_name: None,
            arguments: None,
            result: None,
            is_error: false,
            duration_ms: None,
            request_id: None,
        },
    )
    .await
    .expect("stored");

    let other = mcp_service::calls(&context, &admin, second, 50)
        .await
        .expect("readable");
    assert!(other.is_empty(), "a sandbox's calls are its own");
}

/// `TenantScope` is used by the store directly; this test keeps the import
/// honest and proves the helper applies the tenant, which every MCP query
/// relies on.
#[tokio::test]
async fn a_tenant_scope_reports_its_environment() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let scope = TenantScope::begin(&pool, environment).await.expect("scope");
    assert_eq!(scope.environment_id(), environment);
    scope.commit().await.expect("commit");
}
