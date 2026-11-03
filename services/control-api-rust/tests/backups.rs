//! Backup and restore of persistent roots against a real PostgreSQL.
//!
//!   TEST_DATABASE_URL=postgres://… cargo test --test backups -- --test-threads=1

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::sandbox_service::{self, CreateSandbox};
use nestbox_control_api::services::persist_service::{self, MountRequest};
use nestbox_control_api::services::{backup_service, ServiceContext};

const ENVIRONMENT: &str = "00000000-0000-0000-0000-000000000002";

fn uuid(value: &str) -> Uuid {
    Uuid::from_str(value).unwrap_or_else(|_| Uuid::nil())
}

async fn connect() -> Option<sqlx::PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let pool = sqlx::PgPool::connect(&url).await.ok()?;
    let _ = sqlx::query("DROP SCHEMA IF EXISTS public CASCADE").execute(&pool).await;
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

fn context(pool: &sqlx::PgPool, runtime: &Arc<FakeRuntime>) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        runtime.clone(),
        Arc::new(LocalBus::new()),
        PathBuf::from(std::env::temp_dir()),
    )
}

/// The signed-in operator. The audit trail references a real user, so the
/// principal needs a row.
async fn user(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, email, display_name, password_hash, role)
         VALUES ('00000000-0000-0000-0000-000000000001', $1, 'Test', 'x', 'admin')
         RETURNING id",
    )
    .bind(format!("stack-{}@example.test", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert user")
}

fn principal_of(user_id: Uuid) -> Principal {
    Principal {
        user_id,
        environment_id: uuid(ENVIRONMENT),
        role: Role::Admin,
        display_name: "test".into(),
        via: CredentialSource::Token,
    }
}

fn request(name: &str, pack: &str) -> CreateSandbox {
    CreateSandbox {
        name: name.to_string(),
        pack_key: pack.to_string(),
        image: String::new(),
        image_digest: None,
        command: Vec::new(),
        memory_limit: "512Mi".to_string(),
        cpu_limit: 0.5,
        pids_limit: 256,
        gpu_count: None,
        node: None,
        parent_id: None,
        login: None,
        egress_mode: None,
    }
}


async fn a_root(ctx: &ServiceContext, principal: &Principal, slug: &str) -> nestbox_control_api::domain::persist::PersistentRoot {
    persist_service::create(
        ctx,
        principal,
        persist_service::NewRoot { kind: "project".into(), slug: slug.into(), node: None },
    )
    .await
    .expect("root")
}

#[tokio::test]
async fn a_root_is_backed_up_listed_restored_and_deleted() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);

    let root = a_root(&ctx, &principal, "backup-me").await;
    let backup = backup_service::create(&ctx, &principal, root.id, Some("before the upgrade")).await.expect("backup");
    assert_eq!(backup.root_slug, "backup-me");
    assert_eq!(backup.bytes, 4096);
    assert_eq!(backup.note.as_deref(), Some("before the upgrade"));
    assert_eq!(runtime.backup_calls()[0].0, "create");
    assert_eq!(runtime.backup_calls()[0].2, root.host_path);

    assert_eq!(backup_service::list(&ctx, &principal, Some(root.id)).await.expect("list").len(), 1);
    assert_eq!(backup_service::list(&ctx, &principal, None).await.expect("all").len(), 1);
    assert!(backup_service::create(&ctx, &principal, root.id, Some(&"x".repeat(201))).await.is_err());

    // Restore into the same root; with replace, the node is told to move content aside.
    backup_service::restore(&ctx, &principal, backup.id, None, true, false).await.expect("restore");
    assert_eq!(runtime.backup_calls().last().map(|c| c.0.as_str()), Some("restore-replace"));

    // Delete removes the archive first, then the row.
    backup_service::delete(&ctx, &principal, backup.id).await.expect("delete");
    assert_eq!(runtime.backup_calls().last().map(|c| c.0.as_str()), Some("delete"));
    assert!(backup_service::list(&ctx, &principal, None).await.expect("empty").is_empty());
    assert!(backup_service::restore(&ctx, &principal, backup.id, None, false, false).await.is_err());
}

#[tokio::test]
async fn a_backup_outlives_its_root_and_a_running_writer_blocks_a_restore() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);

    let root = a_root(&ctx, &principal, "busy-root").await;
    let backup = backup_service::create(&ctx, &principal, root.id, None).await.expect("backup");

    let sandbox = sandbox_service::create(&ctx, &principal, request("writer", "ubuntu-dev")).await.expect("sandbox");
    persist_service::mount(
        &ctx,
        &principal,
        root.id,
        MountRequest { sandbox_id: sandbox.id, mount_path: "/work".into(), read_only: false },
    )
    .await
    .expect("mount");

    // The sandbox is running with the root mounted read-write: refused, naming it.
    let refused = backup_service::restore(&ctx, &principal, backup.id, None, false, false).await.expect_err("busy");
    assert!(refused.message.contains("writer"), "{}", refused.message);
    // With force it goes through.
    backup_service::restore(&ctx, &principal, backup.id, None, false, true).await.expect("forced");

    // Remove the root's record: the backup stays, and needs a named target to restore.
    persist_service::dismount(&ctx, &principal, root.id, sandbox.id).await.expect("dismount");
    persist_service::delete(&ctx, &principal, root.id, true).await.expect("purge");
    let kept = backup_service::list(&ctx, &principal, None).await.expect("list");
    assert_eq!(kept.len(), 1);
    assert!(kept[0].root_id.is_none());
    assert!(backup_service::restore(&ctx, &principal, backup.id, None, false, false).await.is_err());
}

#[tokio::test]
async fn a_claude_code_sandbox_gets_a_persistent_home_and_a_recreated_one_gets_it_back() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);

    let first = sandbox_service::create(&ctx, &principal, request("claude-home", "claude-code")).await.expect("create");
    let spec = runtime.created_specs().pop().expect("spec");
    let home = spec
        .mounts
        .iter()
        .find(|mount| mount.target == "/home/nestbox/.agent-state")
        .expect("the state directory is mounted");
    assert!(!home.read_only);
    assert!(home.source.contains("agent-home--home-claude-home") || home.source.contains("home-claude-home"));
    assert!(spec.environment.iter().any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == "/home/nestbox/.agent-state/claude"));

    // Delete and create again under the same name: the same directory comes back.
    sandbox_service::delete(&ctx, &principal, first.id).await.expect("delete");
    sandbox_service::create(&ctx, &principal, request("claude-home", "claude-code")).await.expect("again");
    let again = runtime.created_specs().pop().expect("spec");
    let source = |spec: &nestbox_control_api::domain::runtime::SandboxSpec| {
        spec.mounts.iter().find(|m| m.target == "/home/nestbox/.agent-state").map(|m| m.source.clone())
    };
    assert_eq!(source(&again), Some(home.source.clone()));

    // Another pack gets no such mount.
    sandbox_service::create(&ctx, &principal, request("plain-box", "ubuntu-dev")).await.expect("plain");
    let plain = runtime.created_specs().pop().expect("spec");
    assert!(source(&plain).is_none());
}

#[tokio::test]
async fn an_own_account_sandbox_carries_no_gateway_key_and_a_bad_choice_is_refused() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);

    let mut own = request("own-login", "claude-code");
    own.login = Some("own_account".into());
    let created = sandbox_service::create(&ctx, &principal, own).await.expect("own account");
    assert_eq!(created.spec.get("login").and_then(|v| v.as_str()), Some("own_account"));
    let spec = runtime.created_specs().pop().expect("spec");
    assert!(!spec.environment.iter().any(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN" || k == "ANTHROPIC_BASE_URL"));
    // The login lives on the persistent home, so that is still mounted and configured.
    assert!(spec.mounts.iter().any(|m| m.target == "/home/nestbox/.agent-state"));
    assert!(spec.environment.iter().any(|(k, _)| k == "CLAUDE_CONFIG_DIR"));

    // The sign-in hosts are allowed as ordinary rules of this sandbox, and only those.
    let rules = nestbox_control_api::services::policy_service::rules_for_sandbox(&ctx, &principal, created.id).await.expect("rules");
    let own: Vec<&str> = rules.iter().filter(|r| r.name.starts_with("Sign-in:")).map(|r| r.host.as_str()).collect();
    assert!(own.contains(&"claude.ai") && own.contains(&"console.anthropic.com"));
    assert!(!rules.iter().any(|r| r.host == "example.com"));

    let mut bad = request("bad-login", "claude-code");
    bad.login = Some("magic".into());
    assert!(sandbox_service::create(&ctx, &principal, bad).await.is_err());
    let mut hermes = request("hermes-own", "hermes");
    hermes.login = Some("own_account".into());
    assert!(sandbox_service::create(&ctx, &principal, hermes).await.is_err());

    let default = sandbox_service::create(&ctx, &principal, request("gateway-login", "ubuntu-dev")).await.expect("default");
    assert_eq!(default.spec.get("login").and_then(|v| v.as_str()), Some("gateway"));
}
