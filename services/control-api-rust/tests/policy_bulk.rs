//! Cloning policies, assigning one to several sandboxes, copying rules: against a real
//! PostgreSQL.
//!
//!   TEST_DATABASE_URL=postgres://… cargo test --test policy_bulk -- --test-threads=1

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::sandbox_service::{self, CreateSandbox};
use nestbox_control_api::domain::policy::{Enforcement, NetworkRule, RuleAction, RuleProtocol};
use nestbox_control_api::services::{policy_service, ServiceContext};

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


fn rule(name: &str, host: &str, sandbox_id: Option<Uuid>) -> NetworkRule {
    NetworkRule {
        id: Uuid::nil(),
        environment_id: uuid(ENVIRONMENT),
        policy_id: None,
        sandbox_id,
        name: name.to_string(),
        action: RuleAction::Allow,
        host: host.to_string(),
        port: Some(443),
        protocol: RuleProtocol::Rest,
        methods: vec![],
        binaries: vec![],
        matchers: vec![],
        enforcement: Enforcement::Enforced,
        priority: 100,
        enabled: true,
        version: 1,
    }
}

#[tokio::test]
async fn a_policy_is_cloned_assigned_to_several_sandboxes_and_rules_are_copied() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);

    let a = sandbox_service::create(&ctx, &principal, request("bulk-a", "ubuntu-dev")).await.expect("a");
    let b = sandbox_service::create(&ctx, &principal, request("bulk-b", "ubuntu-dev")).await.expect("b");
    let c = sandbox_service::create(&ctx, &principal, request("bulk-c", "ubuntu-dev")).await.expect("c");

    // Clone: same document, a new name, never global.
    let spec = serde_json::json!({ "version": 1, "network_policies": {} });
    let original = policy_service::create(&ctx, &principal, "original", nestbox_control_api::domain::policy::PolicyScope::Sandbox, &spec)
        .await
        .expect("create");
    let copy = policy_service::clone_policy(&ctx, &principal, original.policy.id, "the copy").await.expect("clone");
    assert_ne!(copy.policy.id, original.policy.id);
    assert_eq!(copy.policy.name, "the copy");
    assert_eq!(copy.policy.spec, original.policy.spec);
    assert!(policy_service::clone_policy(&ctx, &principal, original.policy.id, "  ").await.is_err());

    // Assign to several: a good one, and a missing one that is reported, not hidden.
    let done = policy_service::assign_policy_to_many(&ctx, &principal, copy.policy.id, &[a.id, b.id, Uuid::new_v4()])
        .await
        .expect("assign");
    assert_eq!(done.done, 2);
    assert_eq!(done.failed.len(), 1);
    assert!(policy_service::assign_policy_to_many(&ctx, &principal, copy.policy.id, &[]).await.is_err());
    // A policy in use is refused with a reason, and deletes once nothing uses it.
    let in_use = policy_service::delete(&ctx, &principal, copy.policy.id).await.expect_err("in use");
    assert!(in_use.message.contains("bulk-a"), "{}", in_use.message);
    policy_service::assign_policy_to_many(&ctx, &principal, original.policy.id, &[a.id, b.id]).await.expect("move");
    policy_service::delete(&ctx, &principal, copy.policy.id).await.expect("delete unused");

    // Copy one rule to two sandboxes, then every rule of a sandbox, then to all.
    let source = policy_service::create_rule(&ctx, &principal, rule("api", "api.example.test", Some(a.id))).await.expect("rule");
    let result = policy_service::copy_rules(&ctx, &principal, &[source.id], None, &[b.id, c.id], false).await.expect("copy");
    assert_eq!(result.done, 2);
    let mine = |sandbox: Uuid, rules: &[NetworkRule]| rules.iter().filter(|r| r.sandbox_id == Some(sandbox) && r.host == "api.example.test").count();
    let all = policy_service::list_rules(&ctx, &principal).await.expect("list");
    assert_eq!((mine(a.id, &all), mine(b.id, &all), mine(c.id, &all)), (1, 1, 1));

    // Copying onto the sandbox the rule already belongs to is skipped.
    let again = policy_service::copy_rules(&ctx, &principal, &[source.id], None, &[a.id], false).await.expect("skip");
    assert_eq!(again.done, 0);

    // To all: every sandbox that exists gets a copy (a rule always has an owner).
    let to_all = policy_service::copy_rules(&ctx, &principal, &[], Some(a.id), &[], true).await.expect("all");
    assert!(to_all.done >= 2, "failed: {:?}", to_all.failed);
    let all = policy_service::list_rules(&ctx, &principal).await.expect("list");
    assert!(all.iter().all(|r| r.sandbox_id.is_some() || r.policy_id.is_some()));

    // Nothing to copy, or no target, is a plain refusal.
    assert!(policy_service::copy_rules(&ctx, &principal, &[], None, &[b.id], false).await.is_err());
    assert!(policy_service::copy_rules(&ctx, &principal, &[source.id], None, &[], false).await.is_err());
}
