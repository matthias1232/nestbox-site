//! The tenant tree, the member role, sandbox grants, invitations and sign-in through an
//! identity provider, against a real PostgreSQL as the application role.
//!
//!   TEST_DATABASE_URL=postgres://… cargo test --test multitenant -- --test-threads=1

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use uuid::Uuid;

use nestbox_control_api::config::{Config, DeployMode, OidcProviderConfig};
use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::pagination::ListParams;
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::oidc_service::{self, Profile};
use nestbox_control_api::services::sandbox_service::{self, CreateSandbox};
use nestbox_control_api::services::{auth_service, tenancy_service, user_service, ServiceContext};

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
    let _ = sqlx::query("GRANT USAGE, CREATE ON SCHEMA public TO nestbox_app").execute(&pool).await;
    sqlx::migrate!("./migrations").run(&pool).await.ok()?;
    Some(pool)
}

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

fn context(pool: &sqlx::PgPool) -> ServiceContext {
    ServiceContext::new(
        pool.clone(),
        Arc::new(FakeRuntime::new()),
        Arc::new(LocalBus::new()),
        PathBuf::from(std::env::temp_dir()),
    )
}

fn config(mode: DeployMode) -> Config {
    Config {
        database_url: "postgres://unused".into(),
        listen_addr: "127.0.0.1:0".into(),
        secret_dir: std::env::temp_dir(),
        session_cookie_secure: false,
        session_idle_seconds: 3600,
        session_max_seconds: 7200,
        allowed_origins: Vec::new(),
        runner_url: "http://sandbox-runner:9000".into(),
        runtime_network: "nestbox-runtime".into(),
        bootstrap_admin: false,
        body_limit_bytes: 1_048_576,
        rate_limit_per_minute: 600,
        terminal_idle_seconds: 1800,
        deploy_mode: mode,
        public_url: "http://nestbox.test".into(),
        oidc: vec![OidcProviderConfig {
            id: "google".into(),
            label: "Google".into(),
            kind: "oidc".into(),
            issuer: "http://127.0.0.1:1".into(),
            client_id: "client".into(),
            client_secret: "secret".into(),
        }],
        overlay: nestbox_control_api::config::OverlayConfig {
            dir: Default::default(),
            endpoint: String::new(),
            hub_ip: "10.77.0.1".into(),
            registry: "10.77.0.1:5000".into(),
            control_url: "http://10.77.0.1:8080".into(),
        },
    }
}

async fn admin_user(pool: &sqlx::PgPool, org: &str, email: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, email, display_name, password_hash, role)
         VALUES ($1::uuid, $2, 'Admin', 'x', 'admin') RETURNING id",
    )
    .bind(org)
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("user")
}

fn principal(user_id: Uuid, environment_id: Uuid, role: Role) -> Principal {
    Principal { user_id, environment_id, role, display_name: "test".into(), via: CredentialSource::Token }
}

fn request(name: &str) -> CreateSandbox {
    CreateSandbox {
        name: name.to_string(),
        pack_key: "ubuntu-dev".to_string(),
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

async fn names(ctx: &ServiceContext, who: &Principal) -> Vec<String> {
    let page = sandbox_service::list(ctx, who, &ListParams::default()).await.expect("list");
    let mut names: Vec<String> = page.items.into_iter().map(|item| item.sandbox.name).collect();
    names.sort();
    names
}

fn profile(subject: &str, email: &str, verified: bool) -> Profile {
    Profile { subject: subject.into(), email: email.into(), email_verified: verified, name: "Ada".into() }
}

#[tokio::test]
async fn a_member_sees_and_works_in_only_the_sandbox_they_were_granted() {
    let Some(owner) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let ctx = context(&pool);
    let env = uuid(ENVIRONMENT);
    let admin_id = admin_user(&owner, "00000000-0000-0000-0000-000000000001", "root@example.test").await;
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
        .bind(admin_id).bind(env).execute(&owner).await.expect("scope");
    let admin = principal(admin_id, env, Role::Admin);
    let made = user_service::create(&ctx, &admin, "eve@example.test", "Eve", "eves-password-12", "member").await.expect("member");
    let eve = principal(made.id, env, Role::Member);

    let one = sandbox_service::create(&ctx, &admin, request("alpha")).await.expect("alpha");
    let two = sandbox_service::create(&ctx, &admin, request("beta")).await.expect("beta");

    assert!(names(&ctx, &eve).await.is_empty(), "a member sees nothing until a sandbox is handed over");
    tenancy_service::grant(&ctx, &admin, one.id, eve.user_id).await.expect("grant");
    assert_eq!(names(&ctx, &eve).await, vec!["alpha"]);

    // Everything inside the granted sandbox works; the other one does not exist for her.
    assert!(sandbox_service::get(&ctx, &eve, one.id).await.is_ok());
    assert!(sandbox_service::act(&ctx, &eve, one.id, sandbox_service::LifecycleAction::Stop).await.is_ok());
    assert!(sandbox_service::get(&ctx, &eve, two.id).await.is_err());
    assert!(sandbox_service::delete(&ctx, &eve, two.id).await.is_err());

    // Shared things stay out of reach: she cannot hand the sandbox on, invite, or list users.
    assert!(tenancy_service::grant(&ctx, &eve, one.id, admin_id).await.is_err());
    assert!(tenancy_service::invite(&ctx, &eve, "admin", None, None, 7).await.is_err());
    assert!(user_service::list(&ctx, &eve).await.is_err());

    // Her own sandbox's rules and windows are hers; a foreign sandbox and shared policy are not.
    use nestbox_control_api::services::policy_service;
    assert!(policy_service::allow_destination(&ctx, &eve, one.id, "example.com", Some(443), None).await.is_ok());
    assert!(policy_service::allow_destination(&ctx, &eve, two.id, "example.com", Some(443), None).await.is_err());
    assert!(policy_service::open_window(&ctx, &eve, Some(one.id), Some(5), "audit", "all", "testing").await.is_ok());
    assert!(policy_service::open_window(&ctx, &eve, Some(two.id), Some(5), "audit", "all", "testing").await.is_err());
    assert!(policy_service::open_window(&ctx, &eve, None, Some(5), "audit", "all", "everyone").await.is_err(), "the environment-wide switch is not hers");

    tenancy_service::revoke(&ctx, &admin, one.id, eve.user_id).await.expect("revoke");
    assert!(names(&ctx, &eve).await.is_empty(), "taking the sandbox back closes the door");
    assert!(sandbox_service::get(&ctx, &eve, one.id).await.is_err());

    // A viewer cannot be given a sandbox to work in.
    let viewer = user_service::create(&ctx, &admin, "vic@example.test", "Vic", "vics-password-12", "viewer").await.expect("viewer");
    assert!(tenancy_service::grant(&ctx, &admin, one.id, viewer.id).await.is_err());
}

/// A member sees what they own and what was handed to them — never the boxes of the data center
/// that happen to have no owner.
///
/// A removed user leaves their sandboxes behind with `owner_user_id` NULL (ADR 0038), and that
/// "shared" row used to be visible to every non-admin, so a member granted one sandbox saw all of
/// them in "My sandboxes" (U06-05). The rule lives in the database, so it is proved as the
/// application role: the owner pool bypasses row-level security and would prove nothing.
#[tokio::test]
async fn a_member_sees_only_the_sandboxes_they_own_or_were_granted() {
    let Some(owner) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let ctx = context(&pool);
    let env = uuid(ENVIRONMENT);
    let admin_id = admin_user(&owner, "00000000-0000-0000-0000-000000000001", "root@example.test").await;
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
        .bind(admin_id).bind(env).execute(&owner).await.expect("scope");
    let admin = principal(admin_id, env, Role::Admin);
    let operator = principal(admin_id, env, Role::Operator);
    let made = user_service::create(&ctx, &admin, "eve@example.test", "Eve", "eves-password-12", "member").await.expect("member");
    let eve = principal(made.id, env, Role::Member);

    // Her own sandbox, created as the member: the row carries its owner from the insert on,
    // because the policy lets a member write only what is theirs (migration 0090).
    let own = sandbox_service::create(&ctx, &eve, request("eves-box")).await.expect("a member makes their own box");
    let own_owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_user_id FROM sandboxes WHERE id = $1")
        .bind(own.id)
        .fetch_one(&owner)
        .await
        .expect("owner");
    assert_eq!(own_owner, Some(eve.user_id), "a member's new sandbox is theirs, not shared");
    let handed = sandbox_service::create(&ctx, &admin, request("handed-over")).await.expect("admin box");
    tenancy_service::grant(&ctx, &admin, handed.id, eve.user_id).await.expect("grant");
    let shared = sandbox_service::create(&ctx, &admin, request("left-behind")).await.expect("admin box");
    sqlx::query("UPDATE sandboxes SET owner_user_id = NULL WHERE id = $1")
        .bind(shared.id)
        .execute(&owner)
        .await
        .expect("a removed user's sandbox");

    assert_eq!(
        names(&ctx, &eve).await,
        vec!["eves-box", "handed-over"],
        "a member sees what they own and what was handed to them, and not the shared box"
    );
    let page = sandbox_service::list(&ctx, &eve, &ListParams::default()).await.expect("list");
    assert_eq!(page.total_estimate, 2, "the count beside the rows follows the same rule");
    assert!(sandbox_service::get(&ctx, &eve, shared.id).await.is_err(), "a shared box is not readable by a member");
    assert!(sandbox_service::get(&ctx, &eve, handed.id).await.is_ok(), "the granted box still opens");

    // The people who run the data center see the shared box as before.
    assert_eq!(names(&ctx, &operator).await, vec!["eves-box", "handed-over", "left-behind"]);
    assert_eq!(names(&ctx, &admin).await, vec!["eves-box", "handed-over", "left-behind"]);
    assert!(sandbox_service::get(&ctx, &admin, shared.id).await.is_ok());

    // A viewer sees no shared box either.
    let viewer = user_service::create(&ctx, &admin, "vic@example.test", "Vic", "vics-password-12", "viewer").await.expect("viewer");
    let vic = principal(viewer.id, env, Role::Viewer);
    assert!(names(&ctx, &vic).await.is_empty(), "a viewer sees nothing that was not handed over");
}


#[tokio::test]
async fn a_provider_reaches_its_customers_and_a_customer_reaches_only_itself() {
    let Some(owner) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let ctx = context(&pool);
    let env = uuid(ENVIRONMENT);
    let provider_id = admin_user(&owner, "00000000-0000-0000-0000-000000000001", "provider@example.test").await;
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
        .bind(provider_id).bind(env).execute(&owner).await.expect("scope");
    let provider = principal(provider_id, env, Role::Admin);

    let acme = tenancy_service::create_customer(&ctx, &provider, "Acme GmbH").await.expect("acme");
    let globex = tenancy_service::create_customer(&ctx, &provider, "Globex").await.expect("globex");
    assert_eq!(acme.kind, "customer");
    assert_eq!(acme.environments.len(), 1, "a customer starts with one data center");

    // The provider reaches both customers, as admin.
    let reach: Vec<(Uuid, String)> = sqlx::query_as("SELECT environment_id, role FROM nb_user_environments($1)")
        .bind(provider_id).fetch_all(&owner).await.expect("reach");
    let acme_env = acme.environments[0].id;
    let globex_env = globex.environments[0].id;
    assert!(reach.iter().any(|(e, r)| *e == acme_env && r == "admin"));
    assert!(reach.iter().any(|(e, r)| *e == globex_env && r == "admin"));

    // Each customer's own admin reaches only its own data center.
    let acme_admin = admin_user(&owner, &acme.id.to_string(), "boss@acme.test").await;
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
        .bind(acme_admin).bind(acme_env).execute(&owner).await.expect("scope");
    let own: Vec<Uuid> = sqlx::query_scalar("SELECT environment_id FROM nb_user_environments($1)")
        .bind(acme_admin).fetch_all(&owner).await.expect("own");
    assert_eq!(own, vec![acme_env]);
    assert!(auth_service::principal_for_environment(&owner, acme_admin, "boss", globex_env, CredentialSource::Session).await.is_err());
    assert!(auth_service::principal_for_environment(&owner, acme_admin, "boss", env, CredentialSource::Session).await.is_err());
    assert!(auth_service::principal_for_environment(&owner, provider_id, "provider", globex_env, CredentialSource::Session).await.is_ok());

    // A customer cannot create customers; a customer's data centers are its own business.
    let acme_principal = principal(acme_admin, acme_env, Role::Admin);
    assert!(tenancy_service::create_customer(&ctx, &acme_principal, "Sub").await.is_err());
    let second = tenancy_service::create_data_center(&ctx, &acme_principal, "Frankfurt").await.expect("data center");
    let own: Vec<Uuid> = sqlx::query_scalar("SELECT environment_id FROM nb_user_environments($1)")
        .bind(acme_admin).fetch_all(&owner).await.expect("own");
    assert!(own.contains(&second.id) && own.len() == 2);

    // The tree the console shows is the same reach.
    let seen = tenancy_service::organizations(&ctx, &acme_principal).await.expect("tree");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].environments.len(), 2);

    // Sites: one is made with the data center; an empty one can go, the last name stays unique.
    let site = tenancy_service::create_site(&ctx, &acme_principal, "Hetzner Falkenstein", "cloud", Some("hetzner"), Some("fsn1")).await.expect("site");
    assert!(tenancy_service::create_site(&ctx, &acme_principal, "Hetzner Falkenstein", "cloud", None, None).await.is_err());
    assert!(tenancy_service::create_site(&ctx, &acme_principal, "Odd", "moon", None, None).await.is_err());
    assert_eq!(tenancy_service::list_sites(&ctx, &acme_principal).await.expect("sites").len(), 2);
    tenancy_service::delete_site(&ctx, &acme_principal, site.id).await.expect("delete");
}

#[tokio::test]
async fn sign_in_by_identity_provider_registers_invites_and_refuses_strangers() {
    let Some(owner) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let ctx = context(&pool);
    let saas = config(DeployMode::Saas);
    let hosted = config(DeployMode::SelfHosted);

    // A stranger on a self-hosted installation is turned away; one with an unverified email too.
    assert!(oidc_service::resolve(&ctx, &hosted, "google", &profile("s1", "new@example.test", true), None).await.is_err());
    assert!(oidc_service::resolve(&ctx, &saas, "google", &profile("s2", "new@example.test", false), None).await.is_err());

    // In saas mode the first sign-in starts an onboarding: organisation, data center, site.
    let (user_id, _) = oidc_service::resolve(&ctx, &saas, "google", &profile("s3", "new@example.test", true), None).await.expect("register");
    let reach: Vec<(Uuid, String)> = sqlx::query_as("SELECT environment_id, role FROM nb_user_environments($1)")
        .bind(user_id).fetch_all(&owner).await.expect("reach");
    assert_eq!(reach.len(), 1);
    assert_eq!(reach[0].1, "admin");
    let sites: i64 = sqlx::query_scalar("SELECT count(*) FROM sites WHERE environment_id = $1")
        .bind(reach[0].0).fetch_one(&owner).await.expect("sites");
    assert_eq!(sites, 1);
    let kind: String = sqlx::query_scalar("SELECT o.kind FROM users u JOIN organizations o ON o.id = u.organization_id WHERE u.id = $1")
        .bind(user_id).fetch_one(&owner).await.expect("kind");
    assert_eq!(kind, "customer");
    // The same identity signs in again as the same person; a password login is not possible.
    let (again, _) = oidc_service::resolve(&ctx, &saas, "google", &profile("s3", "new@example.test", true), None).await.expect("again");
    assert_eq!(again, user_id);
    assert!(auth_service::login(&owner, "new@example.test", "anything-at-all", 60, 60).await.is_err());
    // An address that already has an account is not taken over by a new identity.
    assert!(oidc_service::resolve(&ctx, &saas, "google", &profile("s4", "new@example.test", true), None).await.is_err());

    // An invitation to one sandbox: the person becomes a member and sees exactly that sandbox.
    let env = reach[0].0;
    let owner_principal = principal(user_id, env, Role::Admin);
    // A new workspace has no node (it brings its own); give it one so it can hold a sandbox.
    sqlx::query("INSERT INTO nodes (name, display_name, environment_id, kind, runner_url) VALUES ('signup-node', 'signup-node', $1, 'vm', 'http://10.77.0.9:9000')")
        .bind(env).execute(&owner).await.expect("node");
    let kept = sandbox_service::create(&ctx, &owner_principal, request("kept")).await.expect("kept");
    sandbox_service::create(&ctx, &owner_principal, request("private")).await.expect("private");
    let invitation = tenancy_service::invite(&ctx, &owner_principal, "admin", Some(kept.id), Some("guest@example.test"), 7).await.expect("invite");
    assert_eq!(invitation.invitation.role, "member", "a sandbox invitation is always a member's");
    let preview = tenancy_service::preview(&ctx, &invitation.token).await.expect("preview");
    assert_eq!(preview.sandbox.as_deref(), Some("kept"));
    assert!(tenancy_service::preview(&ctx, "not-a-token").await.is_err());

    let id: Uuid = invitation.invitation.id;
    // Another address cannot use an invitation made for one.
    assert!(oidc_service::resolve(&ctx, &hosted, "google", &profile("g1", "thief@example.test", true), Some(id)).await.is_err());
    let (guest, _) = oidc_service::resolve(&ctx, &hosted, "google", &profile("g2", "guest@example.test", true), Some(id)).await.expect("accept");
    let role = principal(guest, env, Role::Member);
    assert_eq!(names(&ctx, &role).await, vec!["kept"]);
    // Used once.
    assert!(tenancy_service::preview(&ctx, &invitation.token).await.is_err());
    assert!(oidc_service::resolve(&ctx, &hosted, "google", &profile("g3", "other@example.test", true), Some(id)).await.is_err());
}

#[tokio::test]
async fn nodes_belong_to_a_tenant_and_a_sandbox_name_is_unique_on_the_installation() {
    let Some(owner) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let ctx = context(&pool);
    let env = uuid(ENVIRONMENT);
    let provider_id = admin_user(&owner, "00000000-0000-0000-0000-000000000001", "p@example.test").await;
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
        .bind(provider_id).bind(env).execute(&owner).await.expect("scope");
    let provider = principal(provider_id, env, Role::Admin);
    let acme = tenancy_service::create_customer(&ctx, &provider, "Acme").await.expect("acme");
    let globex = tenancy_service::create_customer(&ctx, &provider, "Globex").await.expect("globex");
    let acme_env = acme.environments[0].id;
    let globex_env = globex.environments[0].id;
    let acme_admin = principal(admin_user(&owner, &acme.id.to_string(), "a@acme.test").await, acme_env, Role::Admin);
    let globex_admin = principal(admin_user(&owner, &globex.id.to_string(), "g@globex.test").await, globex_env, Role::Admin);

    // A new customer has no node: the provider's local node is not theirs to use.
    let refused = sandbox_service::create(&ctx, &acme_admin, request("first")).await.unwrap_err();
    assert!(refused.message.contains("Add a node"), "{}", refused.message);

    // Each customer brings a node of their own.
    for (name, environment) in [("acme-node", acme_env), ("globex-node", globex_env)] {
        sqlx::query(
            "INSERT INTO nodes (name, display_name, environment_id, kind, runner_url) VALUES ($1, $1, $2, 'vm', 'http://10.77.0.2:9000')",
        )
        .bind(name).bind(environment).execute(&owner).await.expect("node");
    }
    let made = sandbox_service::create(&ctx, &acme_admin, request("shared-name")).await.expect("acme sandbox");
    assert_eq!(made.node_name.as_deref(), Some("acme-node"), "a lone own node is the default");
    // Naming another tenant's node is a not-found, not a refusal that confirms it exists.
    let mut elsewhere = request("probe");
    elsewhere.node = Some("globex-node".into());
    assert!(sandbox_service::create(&ctx, &acme_admin, elsewhere).await.is_err());
    // The same name in another data center is refused, on any node.
    let clash = sandbox_service::create(&ctx, &globex_admin, request("shared-name")).await.unwrap_err();
    assert!(clash.message.contains("another data center"), "{}", clash.message);
    assert!(sandbox_service::create(&ctx, &globex_admin, request("other-name")).await.is_ok());
    // Neither sees the other's sandboxes.
    assert_eq!(names(&ctx, &acme_admin).await, vec!["shared-name"]);
    assert_eq!(names(&ctx, &globex_admin).await, vec!["other-name"]);
}

#[tokio::test]
async fn a_machine_joins_with_a_one_time_token_and_gets_an_address_and_a_place_on_the_hub() {
    use nestbox_control_api::services::node_join_service;
    let Some(owner) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("vault_key"), "ab".repeat(32)).expect("vault key");
    std::fs::write(dir.path().join("hub.pub"), "HUBKEYHUBKEYHUBKEYHUBKEYHUBKEYHUBKEYHUBKEY1=\n").expect("hub key");
    let ctx = ServiceContext::new(
        pool.clone(),
        Arc::new(FakeRuntime::new()),
        Arc::new(LocalBus::new()),
        dir.path().to_path_buf(),
    );
    let mut cfg = config(DeployMode::SelfHosted);
    cfg.overlay.dir = dir.path().to_path_buf();
    cfg.overlay.endpoint = "203.0.113.7:51821".into();

    let env = uuid(ENVIRONMENT);
    let admin_id = admin_user(&owner, "00000000-0000-0000-0000-000000000001", "root@example.test").await;
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
        .bind(admin_id).bind(env).execute(&owner).await.expect("scope");
    let admin = principal(admin_id, env, Role::Admin);
    let site = tenancy_service::list_sites(&ctx, &admin).await.expect("sites")[0].id;

    // Only an administrator asks, and the name is checked.
    let member = principal(admin_id, env, Role::Member);
    assert!(node_join_service::create_token(&ctx, &cfg, &member, "gpu-1", site).await.is_err());
    assert!(node_join_service::create_token(&ctx, &cfg, &admin, "Not A Name", site).await.is_err());
    let made = node_join_service::create_token(&ctx, &cfg, &admin, "gpu-1", site).await.expect("token");
    assert!(made.command.contains(&made.token) && made.command.contains("| sudo bash"));

    let script = node_join_service::script(&ctx, &cfg, &made.token).await.expect("script");
    assert!(script.contains(&made.token) && !script.contains("__TOKEN__") && !script.contains("__COMPOSE__"));
    assert!(node_join_service::script(&ctx, &cfg, "wrong").await.is_err());

    let key = "8GVx1N0wZ2tJ5p3m8cEoQ0Yg3ZtWk1bq6sT9eXr+pQk=";
    assert!(node_join_service::join(&ctx, &cfg, &made.token, "not-a-key", "h").await.is_err());
    let joined = node_join_service::join(&ctx, &cfg, &made.token, key, "gpu-box").await.expect("join");
    assert_eq!(joined.node_name, "gpu-1");
    assert!(joined.overlay_ip.starts_with("10.77.1."), "the first site network is 10.77.1.0/24: {}", joined.overlay_ip);
    assert_eq!(joined.hub_endpoint, "203.0.113.7:51821");

    // The token is spent.
    assert!(node_join_service::join(&ctx, &cfg, &made.token, key, "again").await.is_err());
    assert!(node_join_service::script(&ctx, &cfg, &made.token).await.is_err());

    // The node row, its place in the site, and what the control plane keeps of the token.
    let (url, site_id, locator, runner_ref): (String, Uuid, String, String) = sqlx::query_as(
        "SELECT runner_url, site_id, node_token_ref, runner_token_ref FROM nodes WHERE name = 'gpu-1'",
    )
    .fetch_one(&owner).await.expect("node");
    assert_eq!(url, format!("http://{}:9000", joined.overlay_ip));
    assert_eq!(site_id, site);
    assert!(locator.starts_with("digest://") && !locator.contains(&joined.node_token), "only a hash of the token is kept as a locator");
    assert_eq!(runner_ref, "db://node-gpu-1");
    assert!(nestbox_control_api::security::node_token::verify(&locator, dir.path(), &joined.node_token).is_ok());
    assert!(nestbox_control_api::security::node_token::verify(&locator, dir.path(), "another-token").is_err());

    // The vault returns the clear token for the runner, and the hub is told about the key.
    use nestbox_control_api::services::secret_service;
    assert_eq!(secret_service::credential(&ctx, env, &runner_ref).await.expect("vault"), joined.node_token);
    let peers = std::fs::read_to_string(dir.path().join("peers.conf")).expect("peers");
    assert!(peers.contains(key) && peers.contains(&format!("{}/32", joined.overlay_ip)));

    // A second node in the same site gets the next address; a name taken is refused.
    assert!(node_join_service::create_token(&ctx, &cfg, &admin, "gpu-1", site).await.is_err());
    let second = node_join_service::create_token(&ctx, &cfg, &admin, "gpu-2", site).await.expect("second token");
    let other = node_join_service::join(&ctx, &cfg, &second.token, "Zk1bq6sT9eXr+pQk8GVx1N0wZ2tJ5p3m8cEoQ0Yg3Zt=", "b").await.expect("second");
    assert_eq!(other.overlay_ip, joined.overlay_ip.replace(".2", ".3"));

    // The node belongs to this data center only, and can be removed while empty.
    let customer = tenancy_service::create_customer(&ctx, &admin, "Other Co").await.expect("customer");
    let other_env = customer.environments[0].id;
    let other_admin = principal(admin_user(&owner, &customer.id.to_string(), "x@other.test").await, other_env, Role::Admin);
    assert!(node_join_service::remove(&ctx, &cfg, &other_admin, joined.node_id).await.is_err(), "not another data center's node");
    node_join_service::remove(&ctx, &cfg, &admin, joined.node_id).await.expect("remove");
    let peers = std::fs::read_to_string(dir.path().join("peers.conf")).expect("peers");
    assert!(!peers.contains(key), "a removed node loses its place on the hub");
}

#[tokio::test]
async fn an_image_is_assigned_once_per_scope_and_the_matrix_lists_it() {
    use nestbox_control_api::services::image_sync_service;
    let Some(owner) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let ctx = context(&pool);
    let env = uuid(ENVIRONMENT);
    let admin_id = admin_user(&owner, "00000000-0000-0000-0000-000000000001", "img@example.test").await;
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'admin')")
        .bind(admin_id).bind(env).execute(&owner).await.expect("scope");
    let admin = principal(admin_id, env, Role::Admin);
    let member = principal(admin_id, env, Role::Member);
    let site = tenancy_service::list_sites(&ctx, &admin).await.expect("sites")[0].id;

    let first = image_sync_service::assign(&ctx, &admin, "nestbox-claude-code:local", "environment", None).await.expect("assign");
    let again = image_sync_service::assign(&ctx, &admin, "nestbox-claude-code:local", "environment", None).await.expect("again");
    assert_eq!(first.id, again.id, "the same assignment twice is one assignment");
    image_sync_service::assign(&ctx, &admin, "nestbox-claude-code:local", "site", Some(site)).await.expect("site scope");
    assert!(image_sync_service::assign(&ctx, &admin, "http://evil/x", "environment", None).await.is_err());
    assert!(image_sync_service::assign(&ctx, &admin, "x:1", "site", Some(Uuid::new_v4())).await.is_err(), "a site of another data center is not found");
    assert!(image_sync_service::assign(&ctx, &member, "x:1", "environment", None).await.is_err(), "only an administrator assigns");
    let long = format!("a{}", "b".repeat(255));
    assert!(image_sync_service::assign(&ctx, &admin, &long, "environment", None).await.is_err());

    let matrix = image_sync_service::matrix(&ctx, &admin).await.expect("matrix");
    assert_eq!(matrix.images, vec!["nestbox-claude-code:local"]);
    assert_eq!(matrix.assignments.len(), 2);
    image_sync_service::unassign(&ctx, &admin, first.id).await.expect("unassign");
    assert_eq!(image_sync_service::matrix(&ctx, &admin).await.expect("matrix").assignments.len(), 1);
}

/// A tiny identity provider: discovery, a token endpoint that checks PKCE, and a userinfo endpoint.
async fn mock_provider(expected_challenge: std::sync::Arc<std::sync::Mutex<String>>) -> String {
    use axum::routing::{get, post};
    use axum::{Form, Json, Router};
    use std::collections::HashMap;
    use sha2::{Digest, Sha256};
    use base64::Engine;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let discovery_base = base.clone();
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let base = discovery_base.clone();
                async move {
                    Json(serde_json::json!({
                        "authorization_endpoint": format!("{base}/authorize"),
                        "token_endpoint": format!("{base}/token"),
                        "userinfo_endpoint": format!("{base}/userinfo"),
                    }))
                }
            }),
        )
        .route(
            "/token",
            post(move |Form(form): Form<HashMap<String, String>>| {
                let expected = expected_challenge.clone();
                async move {
                    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
                    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
                    let ok = form.get("grant_type").map(String::as_str) == Some("authorization_code")
                        && form.get("code").map(String::as_str) == Some("good-code")
                        && form.get("client_secret").map(String::as_str) == Some("secret")
                        && challenge == *expected.lock().expect("lock");
                    if ok {
                        (axum::http::StatusCode::OK, Json(serde_json::json!({"access_token": "at-1", "token_type": "Bearer"})))
                    } else {
                        (axum::http::StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "invalid_grant"})))
                    }
                }
            }),
        )
        .route(
            "/userinfo",
            get(|headers: axum::http::HeaderMap| async move {
                if headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer at-1") {
                    (axum::http::StatusCode::OK, Json(serde_json::json!({"sub": "mock-42", "email": "Mock.User@Example.test", "email_verified": true, "name": "Mock User"})))
                } else {
                    (axum::http::StatusCode::UNAUTHORIZED, Json(serde_json::json!({})))
                }
            }),
        );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    base
}

#[tokio::test]
async fn the_whole_sign_in_flow_runs_against_a_provider_with_pkce_and_a_single_use_state() {
    let Some(owner) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let pool = app_pool().await;
    let ctx = context(&pool);
    let challenge = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let issuer = mock_provider(challenge.clone()).await;
    let mut cfg = config(DeployMode::Saas);
    cfg.oidc[0].issuer = issuer;

    // start: the browser is sent to the authorise address with state and a S256 challenge.
    let url = oidc_service::start(&ctx, &cfg, "google", None).await.expect("start");
    let parsed = reqwest::Url::parse(&url).expect("url");
    let query: std::collections::HashMap<String, String> = parsed.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    assert_eq!(query["response_type"], "code");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["redirect_uri"], "http://nestbox.test/api/v1/auth/oidc/google/callback");
    assert_eq!(query["client_id"], "client");
    *challenge.lock().expect("lock") = query["code_challenge"].clone();
    let state = query["state"].clone();

    // A wrong state, a wrong code and an unknown provider are refused.
    assert!(oidc_service::complete(&ctx, &cfg, "google", "good-code", "not-the-state").await.is_err());
    assert!(oidc_service::start(&ctx, &cfg, "nonexistent", None).await.is_err());
    // A bad code consumes the state (it is single use) and fails.
    let bad = oidc_service::start(&ctx, &cfg, "google", None).await.expect("second start");
    let bad_state = reqwest::Url::parse(&bad).expect("url").query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.to_string()).expect("state");
    assert!(oidc_service::complete(&ctx, &cfg, "google", "bad-code", &bad_state).await.is_err());
    assert!(oidc_service::complete(&ctx, &cfg, "google", "good-code", &bad_state).await.is_err(), "a used state does not work twice");

    // The real one: code exchanged with the verifier, profile read, a workspace created.
    let (user_id, name) = oidc_service::complete(&ctx, &cfg, "google", "good-code", &state).await.expect("complete");
    assert_eq!(name, "Mock User");
    let email: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1").bind(user_id).fetch_one(&owner).await.expect("email");
    assert_eq!(email, "mock.user@example.test");
    let reach: i64 = sqlx::query_scalar("SELECT count(*) FROM nb_user_environments($1)").bind(user_id).fetch_one(&owner).await.expect("reach");
    assert_eq!(reach, 1);
    // The state is gone: replaying the callback finds nothing.
    assert!(oidc_service::complete(&ctx, &cfg, "google", "good-code", &state).await.is_err());

    // And a session can be opened for the person.
    let session = auth_service::start_session(&owner, user_id, name, 3600, 7200).await.expect("session");
    assert_eq!(session.role.as_str(), "admin");

    // Self-hosted: the same provider does not register a second stranger.
    let hosted = {
        let mut c = config(DeployMode::SelfHosted);
        c.oidc[0].issuer = cfg.oidc[0].issuer.clone();
        c
    };
    let again = oidc_service::start(&ctx, &hosted, "google", None).await.expect("start");
    let st = reqwest::Url::parse(&again).expect("url").query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.to_string()).expect("state");
    let ch = reqwest::Url::parse(&again).expect("url").query_pairs().find(|(k, _)| k == "code_challenge").map(|(_, v)| v.to_string()).expect("challenge");
    *challenge.lock().expect("lock") = ch;
    // The identity is known already, so this one is the same person signing in again.
    let (same, _) = oidc_service::complete(&ctx, &hosted, "google", "good-code", &st).await.expect("known identity");
    assert_eq!(same, user_id);
}
