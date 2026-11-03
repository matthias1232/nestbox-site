//! Users and ownership against a real PostgreSQL, as the application role.
//!
//!   TEST_DATABASE_URL=postgres://… cargo test --test tenancy -- --test-threads=1

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::sandbox_service::{self, CreateSandbox};
use nestbox_control_api::domain::pagination::ListParams;
use nestbox_control_api::services::{backup_service, persist_service, user_service, ServiceContext};

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

/// The services run as `nestbox_app`, the role the real application connects as. The suite's
/// own connection is the schema owner and would bypass row-level security, which is exactly
/// what these tests are about.
async fn app_pool() -> sqlx::PgPool {
    let url = std::env::var("TEST_DATABASE_URL").expect("url");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE nestbox_app").execute(connection).await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .expect("app pool")
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


fn as_role(user_id: Uuid, role: Role) -> Principal {
    Principal { role, ..principal_of(user_id) }
}

#[tokio::test]
async fn a_user_sees_their_own_sandboxes_an_admin_sees_all_and_shared_ones_are_open_to_everyone() {
    let Some(owner_pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _ = owner_pool;
    let pool = app_pool().await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);

    let admin = as_role(user(&pool).await, Role::Admin);
    let alice = user_service::create(&ctx, &admin, "alice@example.test", "Alice", "alice-password-1", "operator").await.expect("alice");
    let bob = user_service::create(&ctx, &admin, "bob@example.test", "Bob", "bobs-password-12", "operator").await.expect("bob");
    let alice = as_role(alice.id, Role::Operator);
    let bob = as_role(bob.id, Role::Operator);

    sandbox_service::create(&ctx, &alice, request("alice-box", "ubuntu-dev")).await.expect("alice's");
    let bobs = sandbox_service::create(&ctx, &bob, request("bob-box", "ubuntu-dev")).await.expect("bob's");
    sandbox_service::create(&ctx, &admin, request("admin-box", "ubuntu-dev")).await.expect("admin's");

    let names = |principal: &Principal| {
        let ctx = &ctx;
        let principal = principal.clone();
        async move {
            let page = sandbox_service::list(ctx, &principal, &ListParams::default()).await.expect("list");
            let mut names: Vec<String> = page.items.into_iter().map(|item| item.sandbox.name).collect();
            names.sort();
            names
        }
    };
    assert_eq!(names(&alice).await, vec!["alice-box"], "a user sees only their own");
    assert_eq!(names(&bob).await, vec!["bob-box"]);
    assert_eq!(names(&admin).await, vec!["admin-box", "alice-box", "bob-box"], "an admin sees every sandbox");

    // Reaching someone else's sandbox by id is a plain not-found, for every kind of access.
    assert!(sandbox_service::get(&ctx, &alice, bobs.id).await.is_err());
    assert!(sandbox_service::act(&ctx, &alice, bobs.id, sandbox_service::LifecycleAction::Stop).await.is_err());
    assert!(sandbox_service::delete(&ctx, &alice, bobs.id).await.is_err());
    // …and the owner and an admin can.
    assert!(sandbox_service::get(&ctx, &bob, bobs.id).await.is_ok());
    assert!(sandbox_service::get(&ctx, &admin, bobs.id).await.is_ok());

    // A sandbox nobody owns (every sandbox from before ownership) is shared.
    sqlx::query("UPDATE sandboxes SET owner_user_id = NULL WHERE name = 'admin-box'")
        .execute(&owner_pool)
        .await
        .expect("make shared");
    assert_eq!(names(&alice).await, vec!["admin-box", "alice-box"]);
    assert_eq!(names(&bob).await, vec!["admin-box", "bob-box"]);
}

#[tokio::test]
async fn only_an_admin_manages_users_and_the_guards_keep_an_admin_in_place() {
    let Some(owner_pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _ = owner_pool;
    let pool = app_pool().await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);

    let admin_id = user(&pool).await;
    // A real user always has a role in the environment.
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
        .bind(admin_id)
        .bind(uuid(ENVIRONMENT))
        .execute(&owner_pool)
        .await
        .expect("scope");
    let admin = as_role(admin_id, Role::Admin);
    let made = user_service::create(&ctx, &admin, "carol@example.test", "Carol", "carols-password-1", "operator").await.expect("carol");
    let carol = as_role(made.id, Role::Operator);

    // An operator cannot list, create or change users.
    assert!(user_service::list(&ctx, &carol).await.is_err());
    assert!(user_service::create(&ctx, &carol, "x@example.test", "X", "long-enough-pass-1", "viewer").await.is_err());
    assert!(user_service::update(&ctx, &carol, admin.user_id, None, Some("viewer"), None).await.is_err());

    // Validation: a short password, a bad address, a duplicate, a bad role.
    assert!(user_service::create(&ctx, &admin, "dave@example.test", "Dave", "short", "viewer").await.is_err());
    assert!(user_service::create(&ctx, &admin, "not-an-address", "Dave", "long-enough-pass-1", "viewer").await.is_err());
    assert!(user_service::create(&ctx, &admin, "carol@example.test", "Carol 2", "long-enough-pass-1", "viewer").await.is_err());
    assert!(user_service::create(&ctx, &admin, "eve@example.test", "Eve", "long-enough-pass-1", "root").await.is_err());

    // The listing carries role and owned sandboxes, and marks the caller.
    sandbox_service::create(&ctx, &carol, request("carol-box", "ubuntu-dev")).await.expect("carol's");
    let listed = user_service::list(&ctx, &admin).await.expect("list");
    let carol_row = listed.iter().find(|u| u.email == "carol@example.test").expect("carol listed");
    assert_eq!((carol_row.role.as_str(), carol_row.sandboxes), ("operator", 1));
    assert!(listed.iter().any(|u| u.is_you && u.role == "admin"));

    // The guards: an admin cannot demote, disable or remove themselves, and the last one stays.
    assert!(user_service::update(&ctx, &admin, admin.user_id, None, Some("operator"), None).await.is_err());
    assert!(user_service::update(&ctx, &admin, admin.user_id, None, None, Some("disabled")).await.is_err());
    assert!(user_service::remove(&ctx, &admin, admin.user_id).await.is_err());

    // Changing a role works, and a disabled user is out; removing one hands their sandboxes to nobody.
    user_service::update(&ctx, &admin, made.id, Some("Carol C."), Some("viewer"), None).await.expect("demote");
    user_service::set_password(&ctx, &admin, made.id, "a-new-password-12").await.expect("password");
    user_service::remove(&ctx, &admin, made.id).await.expect("remove");
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_user_id FROM sandboxes WHERE name = 'carol-box'")
        .fetch_one(&owner_pool)
        .await
        .expect("owner");
    assert!(owner.is_none(), "a removed user's sandboxes become shared");
    assert!(user_service::list(&ctx, &admin).await.expect("list").iter().all(|u| u.email != "carol@example.test"));
}

#[tokio::test]
async fn persistent_data_and_its_backups_belong_to_their_owner() {
    let Some(owner_pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _ = owner_pool;
    let pool = app_pool().await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);

    let admin = as_role(user(&pool).await, Role::Admin);
    let a = user_service::create(&ctx, &admin, "ann@example.test", "Ann", "ann-password-123", "operator").await.expect("ann");
    let b = user_service::create(&ctx, &admin, "ben@example.test", "Ben", "ben-password-123", "operator").await.expect("ben");
    let (ann, ben) = (as_role(a.id, Role::Operator), as_role(b.id, Role::Operator));

    let root = persist_service::create(
        &ctx,
        &ann,
        persist_service::NewRoot { kind: "project".into(), slug: "ann-data".into(), node: None },
    )
    .await
    .expect("ann's root");
    let backup = backup_service::create(&ctx, &ann, root.id, None).await.expect("ann's backup");

    let slugs = |principal: &Principal| {
        let (ctx, principal) = (&ctx, principal.clone());
        async move { persist_service::list(ctx, &principal).await.expect("list").into_iter().map(|r| r.slug).collect::<Vec<_>>() }
    };
    assert_eq!(slugs(&ann).await, vec!["ann-data"]);
    assert!(slugs(&ben).await.is_empty(), "another user's data is not listed");
    assert_eq!(slugs(&admin).await, vec!["ann-data"], "an admin sees it");

    // Ben cannot read, back up, list the backups of, restore or delete Ann's data.
    assert!(persist_service::get(&ctx, &ben, root.id).await.is_err());
    assert!(backup_service::create(&ctx, &ben, root.id, None).await.is_err());
    assert!(backup_service::list(&ctx, &ben, None).await.expect("list").is_empty());
    assert!(backup_service::restore(&ctx, &ben, backup.id, None, false, true).await.is_err());
    assert!(backup_service::delete(&ctx, &ben, backup.id).await.is_err());
    // Ann can.
    assert_eq!(backup_service::list(&ctx, &ann, None).await.expect("list").len(), 1);
}

#[tokio::test]
async fn telemetry_rules_and_supervisors_follow_the_sandbox_they_belong_to() {
    let Some(owner_pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);

    let admin = as_role(user(&pool).await, Role::Admin);
    let a = user_service::create(&ctx, &admin, "uma@example.test", "Uma", "uma-password-123", "operator").await.expect("uma");
    let b = user_service::create(&ctx, &admin, "vic@example.test", "Vic", "vic-password-123", "operator").await.expect("vic");
    let (uma, vic) = (as_role(a.id, Role::Operator), as_role(b.id, Role::Operator));
    let uma_box = sandbox_service::create(&ctx, &uma, request("uma-box", "ubuntu-dev")).await.expect("uma's");

    // One telemetry row and one sandbox rule of Uma's sandbox, written by the control plane.
    sqlx::query(
        "INSERT INTO telemetry_events (id, environment_id, sandbox_id, kind, ts, detail)
         VALUES (gen_random_uuid(), $1, $2, 'network', now(), '{}'::jsonb),
                (gen_random_uuid(), $1, NULL, 'network', now(), '{}'::jsonb)",
    )
    .bind(uuid(ENVIRONMENT))
    .bind(uma_box.id)
    .execute(&owner_pool)
    .await
    .expect("telemetry");
    sqlx::query(
        "INSERT INTO network_rules (environment_id, sandbox_id, name, action, host, protocol, enforcement, priority)
         VALUES ($1, $2, 'uma only', 'allow', 'private.example.test', 'rest', 'enforced', 100)",
    )
    .bind(uuid(ENVIRONMENT))
    .bind(uma_box.id)
    .execute(&owner_pool)
    .await
    .expect("rule");

    let count = |principal: &Principal, sql: &'static str| {
        let (pool, principal) = (pool.clone(), principal.clone());
        async move {
            let mut scope = nestbox_control_api::store::TenantScope::begin_for(&pool, &principal).await.expect("scope");
            let n: i64 = sqlx::query_scalar(sql).fetch_one(scope.connection()).await.expect("count");
            n
        }
    };
    let events = "SELECT count(*) FROM telemetry_events";
    let rules = "SELECT count(*) FROM network_rules WHERE name = 'uma only'";
    let supervisors = "SELECT count(*) FROM supervisors WHERE name = 'sup-uma-box'";

    assert_eq!(count(&uma, events).await, 1, "Uma sees what her sandbox did, not the row about no sandbox");
    assert_eq!(count(&vic, events).await, 0, "Vic sees none of it");
    assert_eq!(count(&admin, events).await, 2, "an admin sees both");
    assert_eq!((count(&uma, rules).await, count(&vic, rules).await, count(&admin, rules).await), (1, 0, 1));
    assert_eq!((count(&uma, supervisors).await, count(&vic, supervisors).await, count(&admin, supervisors).await), (1, 0, 1));

    // The audit trail and the key list are for admins.
    assert!(nestbox_control_api::services::audit_service::list(&ctx, &uma, 10).await.is_err());
    assert!(nestbox_control_api::services::audit_service::list(&ctx, &admin, 10).await.is_ok());
    assert!(nestbox_control_api::services::secret_service::list(&ctx, &uma).await.is_err());
}

#[tokio::test]
async fn a_live_session_gets_a_new_csrf_token_and_the_old_one_stops_working() {
    let Some(owner_pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    use nestbox_control_api::security::session;
    use nestbox_control_api::services::auth_service;
    let admin_id = user(&owner_pool).await;
    let issued = session::issue();
    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, csrf_hash, expires_at, idle_deadline)
         VALUES ($1, $2, $3, now() + interval '1 hour', now() + interval '1 hour')",
    )
    .bind(admin_id)
    .bind(&issued.token_hash)
    .bind(&issued.csrf_hash)
    .execute(&owner_pool)
    .await
    .expect("session");

    let fresh = auth_service::rotate_csrf(&owner_pool, &issued.token).await.expect("rotate");
    assert_ne!(fresh, issued.csrf_token);
    let stored: String = sqlx::query_scalar("SELECT csrf_hash FROM sessions WHERE token_hash = $1")
        .bind(&issued.token_hash)
        .fetch_one(&owner_pool)
        .await
        .expect("hash");
    assert!(auth_service::verify_csrf(&fresh, &stored).is_ok(), "the new token is accepted");
    assert!(auth_service::verify_csrf(&issued.csrf_token, &stored).is_err(), "the old one is not");
    assert!(auth_service::rotate_csrf(&owner_pool, "no-such-session-token").await.is_err());
}

#[tokio::test]
async fn only_an_admin_defines_mcp_servers_because_attaching_one_opens_its_endpoint() {
    let Some(owner_pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let _ = owner_pool;
    let pool = app_pool().await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let admin = as_role(user(&pool).await, Role::Admin);
    let made = user_service::create(&ctx, &admin, "ola@example.test", "Ola", "ola-password-123", "operator").await.expect("ola");
    let ola = as_role(made.id, Role::Operator);
    let input = || nestbox_control_api::services::mcp_service::ServerInput {
        key: "sneaky".into(), name: "Sneaky".into(), description: None, publisher: None, homepage: None,
        transport: "http".into(), endpoint: Some("https://anywhere.example.test/mcp".into()), command: None,
        args: vec![], auth_type: "none".into(), env_slots: vec![], packs: vec![], enabled: true,
    };
    assert!(nestbox_control_api::services::mcp_service::create(&ctx, &ola, input()).await.is_err(), "an operator cannot define a server");
    assert!(nestbox_control_api::services::mcp_service::create(&ctx, &admin, input()).await.is_ok(), "an admin can");
    assert!(nestbox_control_api::services::mcp_service::delete(&ctx, &ola, "sneaky").await.is_err());
}
