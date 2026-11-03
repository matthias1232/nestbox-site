//! Access against a real database: SSH keys, the port pool, sessions.
//!
//! These prove what a pure function cannot: that the private key exists only as
//! a 0600 file and a `file://` locator, that the one-time download is refused a
//! second time, that rotation replaces the file, that a port is claimed by the
//! unique constraint rather than by looking first, and that a withdrawn forward
//! gives its port back.
//!
//! They run only when `TEST_DATABASE_URL` is set and own a database of their own,
//! so they can run next to `tests/database.rs` without either suite dropping the
//! other's schema:
//!
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_access?host=/tmp/opencode/pgtest'
//!   cargo test --test access -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use axum::http::StatusCode;
use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::access::{
    ForwardPurpose, PublishedPort, SessionKind,
};
use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::sandbox::Sandbox;
use nestbox_control_api::error::{ApiError, ErrorCode, StoreError};
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::ServiceContext;
use nestbox_control_api::services::access_service;
use nestbox_control_api::store::TenantScope;
use nestbox_control_api::store::postgres::access as store;
use nestbox_control_api::store::postgres::environments as env_store;
use nestbox_control_api::store::postgres::nodes as node_store;
use nestbox_control_api::store::postgres::sandboxes as sandbox_store;

/// The database this suite owns. Fixed so a previous failed run is replaced
/// rather than accumulated.
const DATABASE: &str = "nestbox_access";

/// The seeded administrator. The audit trail has a foreign key to `users`, so a
/// made-up actor id would turn a successful write into a 500.
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
    sqlx::PgPool::connect_with(options.database(DATABASE)).await.ok()
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

/// A context with a database, the in-memory runtime, and a secret directory of
/// its own. The directory is returned so a test can look at the files.
fn context(pool: &PgPool) -> (ServiceContext, std::path::PathBuf) {
    let secret_dir = std::env::temp_dir().join(format!("nestbox-access-{}", Uuid::new_v4()));
    let ctx = ServiceContext::new(
        pool.clone(),
        Arc::new(FakeRuntime::new()),
        Arc::new(LocalBus::new()),
        secret_dir.clone(),
    );
    (ctx, secret_dir)
}

/// A name unique to this test, so tests can share one database.
fn unique(prefix: &str) -> String {
    let suffix = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &suffix[..8])
}

/// Insert a sandbox row directly: these tests are about access, not about the
/// create path or the image policy, and a container would prove neither.
async fn sandbox(ctx: &ServiceContext, environment_id: Uuid, pack_key: &str) -> Sandbox {
    let name = unique("access");
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .expect("scope");
    let cluster_id = env_store::default_cluster(&mut scope).await.expect("cluster");
    let node = node_store::local(&mut scope).await.expect("local node");
    let inserted = sandbox_store::insert(
        &mut scope,
        sandbox_store::NewSandbox {
            cluster_id,
            gateway_id: None,
            node_id: node.id,
            name: &name,
            pack_key,
            image: "nestbox/base@sha256:0000000000000000000000000000000000000000000000000000000000000000",
            image_digest: None,
            spec: &serde_json::json!({ "pack_key": pack_key }),
            parent_id: None,
            owner_user_id: None,
        },
    )
    .await
    .expect("insert sandbox");
    scope.commit().await.expect("commit");
    inserted
}

/// The node's public address is what the Access table prints; set it so the
/// assertion is about our join and not about a null column.
async fn name_the_node(ctx: &ServiceContext, node_id: Uuid) {
    sqlx::query("UPDATE nodes SET public_address = '192.168.55.50' WHERE id = $1")
        .bind(node_id)
        .execute(&ctx.pool)
        .await
        .expect("node address");
}

fn key_error(error: &ApiError, expected: ErrorCode) {
    assert_eq!(error.code, expected, "unexpected error: {}", error.message);
}

fn only_ssh(ports: &[PublishedPort]) -> &PublishedPort {
    ports
        .iter()
        .find(|port| port.purpose == ForwardPurpose::Ssh)
        .expect("an ssh forward")
}

/// The whole row of a key, as text, so a test can search it for material.
async fn key_row_text(pool: &PgPool, sandbox_id: Uuid) -> String {
    let row: (String, String, String) = sqlx::query_as(
        "SELECT public_key, private_key_ref, fingerprint FROM sandbox_ssh_keys WHERE sandbox_id = $1",
    )
    .bind(sandbox_id)
    .fetch_one(pool)
    .await
    .expect("the key row");
    format!("{}{}{}", row.0, row.1, row.2)
}

#[tokio::test]
async fn a_sandbox_gets_a_private_key_file_a_locator_and_a_port_per_service() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let (ctx, secret_dir) = context(&pool);
    let who = principal(environment);
    let created = sandbox(&ctx, environment, "hermes").await;
    name_the_node(&ctx, created.node_id).await;

    let provisioned = access_service::provision(&ctx, &who, &created)
        .await
        .expect("provisioning works");

    // 1. The key: a real Ed25519 pair, a locator in the database, a 0600 file on
    //    the node, and no material in any column.
    assert!(provisioned.key.public_key.starts_with("ssh-ed25519 "));
    assert!(provisioned.key.fingerprint.starts_with("SHA256:"));
    assert!(
        provisioned.key.private_key_ref.starts_with("file://"),
        "the database holds a locator, not a key"
    );
    assert!(provisioned.key.downloadable());

    let path = secret_dir.join(format!("ssh/{}.key", created.id));
    let material = std::fs::read_to_string(&path).expect("the private key file exists");
    assert!(material.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the private key must be 0600");
    }
    let row = key_row_text(&pool, created.id).await;
    assert!(
        !row.contains("PRIVATE KEY"),
        "the database must not hold key material"
    );
    assert!(!row.contains(material.trim()));

    // 2. The ports: SSH plus one per pack port, all from the node's range.
    assert_eq!(provisioned.ports.len(), 2, "ssh + hermes 8640");
    let ssh = only_ssh(&provisioned.ports);
    assert_eq!(ssh.target_port, 22);
    assert!((22000..=22999).contains(&ssh.public_port));
    assert_eq!(ssh.dial().as_deref(), Some(&*format!("192.168.55.50:{}", ssh.public_port)));

    let hermes = provisioned
        .ports
        .iter()
        .find(|port| port.purpose == ForwardPurpose::Agent)
        .expect("the pack port");
    assert_eq!(hermes.target_port, 8640);
    assert_eq!(hermes.label.as_deref(), Some("hermes"));
    assert_ne!(ssh.public_port, hermes.public_port);

    // 3. Provisioning again keeps the key and the ports instead of taking more.
    let again = access_service::provision(&ctx, &who, &created)
        .await
        .expect("provisioning is idempotent");
    assert_eq!(again.key.fingerprint, provisioned.key.fingerprint);
    assert_eq!(again.ports.len(), provisioned.ports.len());

    // 4. What the endpoint lists is what the store holds, with the node's address.
    let listed = access_service::ports(&ctx, &who, created.id)
        .await
        .expect("ports are readable");
    assert_eq!(listed.len(), provisioned.ports.len());
    assert_eq!(listed[0].public_address.as_deref(), Some("192.168.55.50"));

    let _ = std::fs::remove_dir_all(&secret_dir);
}

#[tokio::test]
async fn the_key_downloads_once_and_rotation_writes_a_new_file() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let (ctx, secret_dir) = context(&pool);
    let who = principal(environment);
    let created = sandbox(&ctx, environment, "hermes").await;
    access_service::provision(&ctx, &who, &created)
        .await
        .expect("provisioning works");

    let first = access_service::download(&ctx, &who, created.id)
        .await
        .expect("the first download works");
    assert!(first.private_key_pem.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    assert!(first.fingerprint.starts_with("SHA256:"));
    assert_eq!(first.sandbox_id, created.id);
    assert_eq!(first.filename, format!("nestbox-{}.key", created.name));

    let second = access_service::download(&ctx, &who, created.id)
        .await
        .expect_err("the second download must be refused");
    assert_eq!(second.status, StatusCode::GONE);
    key_error(&second, ErrorCode::SSH_KEY_ALREADY_DOWNLOADED);
    assert!(
        !second.message.contains("BEGIN"),
        "a refusal must not repeat material"
    );

    // Wrong role: a viewer may read the record, not the key.
    let mut viewer = who.clone();
    viewer.role = Role::Viewer;
    let refused = access_service::download(&ctx, &viewer, created.id)
        .await
        .expect_err("a viewer cannot download a key");
    assert_eq!(refused.status, StatusCode::FORBIDDEN);

    // Rotation: a new pair, the old file is gone, and the download works again.
    let rotated = access_service::rotate(&ctx, &who, created.id)
        .await
        .expect("rotation works");
    assert_ne!(rotated.fingerprint, first.fingerprint);
    assert!(rotated.downloadable(), "rotation resets the download mark");
    assert!(rotated.rotated_at.is_some());

    let path = secret_dir.join(format!("ssh/{}.key", created.id));
    let after = std::fs::read_to_string(&path).expect("the new key file exists");
    assert_ne!(after, first.private_key_pem, "the old private key is gone");
    assert!(!after.contains(first.private_key_pem.trim()));

    let third = access_service::download(&ctx, &who, created.id)
        .await
        .expect("the rotated key downloads once");
    assert_eq!(third.fingerprint, rotated.fingerprint);
    // Byte for byte, including the final newline: a trimmed PEM is a key
    // `ssh-keygen` refuses to load (`error in libcrypto`).
    assert_eq!(third.private_key_pem, after);
    assert!(third.private_key_pem.ends_with('\n'));

    // The downloaded material really is the pair behind the recorded public
    // half, not merely a file that looks like a key.
    let parsed = ssh_key::PrivateKey::from_openssh(&third.private_key_pem).expect("a usable key");
    assert_eq!(
        parsed.public_key().to_openssh().expect("public half"),
        rotated.public_key
    );

    // The record endpoint never carries material.
    let record = access_service::key(&ctx, &who, created.id)
        .await
        .expect("the record is readable");
    let json = serde_json::to_string(&record).expect("serialises");
    assert!(!json.contains("PRIVATE KEY"));
    assert!(!json.contains("private_key_pem"));

    let _ = std::fs::remove_dir_all(&secret_dir);
}

#[tokio::test]
async fn a_port_claim_is_settled_by_the_constraint_not_by_looking_first() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let (ctx, secret_dir) = context(&pool);
    let first = sandbox(&ctx, environment, "hermes").await;
    let second = sandbox(&ctx, environment, "hermes").await;

    // A port in the range that nothing has claimed.
    let port = 22951;

    // Two claims of the same port at the same time: exactly one wins, and the
    // loser is refused by the primary key rather than by a check that raced.
    let claim = |sandbox_id: Uuid| {
        let pool = pool.clone();
        async move {
            let mut scope = TenantScope::begin(&pool, environment)
                .await
                .expect("scope");
            let won = store::claim_port(
                &mut scope,
                first.node_id,
                port,
                sandbox_id,
                ForwardPurpose::Custom,
                4096,
            )
            .await
            .expect("claim runs");
            scope.commit().await.expect("commit");
            won
        }
    };
    let (left, right) = tokio::join!(claim(first.id), claim(second.id));
    assert_eq!(
        usize::from(left) + usize::from(right),
        1,
        "one claim wins, the other is refused"
    );

    // The full create path allocates disjoint ports for two sandboxes.
    let left = access_service::provision(&ctx, &principal(environment), &first)
        .await
        .expect("provisioning works");
    let right = access_service::provision(&ctx, &principal(environment), &second)
        .await
        .expect("provisioning works");
    let mut ports: Vec<i32> = left
        .ports
        .iter()
        .chain(right.ports.iter())
        .map(|port| port.public_port)
        .collect();
    let total = ports.len();
    ports.sort_unstable();
    ports.dedup();
    assert_eq!(ports.len(), total, "two sandboxes never share a port");

    let _ = std::fs::remove_dir_all(&secret_dir);
}

#[tokio::test]
async fn withdrawing_a_forward_returns_its_port_to_the_pool() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let (ctx, secret_dir) = context(&pool);
    let who = principal(environment);
    let created = sandbox(&ctx, environment, "hermes").await;
    let provisioned = access_service::provision(&ctx, &who, &created)
        .await
        .expect("provisioning works");
    let ssh = only_ssh(&provisioned.ports).clone();

    access_service::withdraw_port(&ctx, &who, created.id, ssh.forward_id)
        .await
        .expect("withdrawing works");

    let left = access_service::ports(&ctx, &who, created.id)
        .await
        .expect("ports are readable");
    assert!(
        left.iter().all(|port| port.forward_id != ssh.forward_id),
        "the withdrawn forward is gone"
    );

    // The port is free again: the claim table no longer holds it.
    let mut scope = TenantScope::begin(&ctx.pool, environment)
        .await
        .expect("scope");
    let claimed = store::claimed_ports(&mut scope, created.node_id)
        .await
        .expect("claims are readable");
    assert!(
        !claimed.contains(&ssh.public_port),
        "the port went back to the pool"
    );
    let reclaimed = store::claim_port(
        &mut scope,
        created.node_id,
        ssh.public_port,
        created.id,
        ForwardPurpose::Ssh,
        22,
    )
    .await
    .expect("claim runs");
    assert!(reclaimed, "a released port can be claimed again");
    scope.commit().await.expect("commit");

    // A foreign forward id is a 404, not a silent success.
    let missing = access_service::withdraw_port(&ctx, &who, created.id, Uuid::new_v4())
        .await
        .expect_err("an unknown forward is refused");
    key_error(&missing, ErrorCode::FORWARD_NOT_FOUND);

    // Publishing another port from the pool works and is visible afterwards.
    let added = access_service::publish_port(&ctx, &who, created.id, 6000, Some("web".into()))
        .await
        .expect("publishing works");
    assert_eq!(added.target_port, 6000);
    assert_eq!(added.label.as_deref(), Some("web"));
    assert!((22000..=22999).contains(&added.public_port));
    let duplicate = access_service::publish_port(&ctx, &who, created.id, 6000, None)
        .await
        .expect_err("the same target port is refused");
    assert_eq!(duplicate.status, StatusCode::CONFLICT);

    let _ = std::fs::remove_dir_all(&secret_dir);
}

#[tokio::test]
async fn sessions_are_listed_without_content_and_never_leave_the_environment() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let (ctx, secret_dir) = context(&pool);
    let who = principal(environment);
    let created = sandbox(&ctx, environment, "hermes").await;

    // A terminal session as the terminal package would record it: who, when,
    // and how long. Nothing about what was typed.
    let mut scope = TenantScope::begin(&ctx.pool, environment)
        .await
        .expect("scope");
    let session_id = store::open_session(
        &mut scope,
        store::NewSession {
            sandbox_id: created.id,
            user_id: Some(who.user_id),
            kind: SessionKind::Terminal,
            close_reason: None,
        },
    )
    .await
    .expect("session opens");
    store::close_session(&mut scope, session_id, "idle", 12, 4096)
        .await
        .expect("session closes");
    scope.commit().await.expect("commit");

    let sessions = access_service::sessions(&ctx, &who, created.id)
        .await
        .expect("sessions are readable");
    assert_eq!(sessions.len(), 1);
    let session = &sessions[0];
    assert_eq!(session.kind, SessionKind::Terminal);
    assert_eq!(session.close_reason.as_deref(), Some("idle"));
    assert_eq!(session.bytes_in, 12);
    assert_eq!(session.bytes_out, 4096);
    assert!(session.closed_at.is_some());

    // The wire shape has no field a keystroke could travel in.
    let json = serde_json::to_value(session).expect("serialises");
    let object = json.as_object().expect("an object");
    for forbidden in ["content", "bytes", "output", "input", "log"] {
        assert!(
            !object.contains_key(forbidden),
            "a session must not carry '{forbidden}'"
        );
    }

    // A sandbox in another environment is not reachable through this tenant.
    let other = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO environments (organization_id, name, slug)
         SELECT organization_id, 'Other', 'other' FROM environments WHERE slug = 'default'
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("a second environment");
    let other_who = principal(other);
    let hidden = access_service::sessions(&ctx, &other_who, created.id)
        .await
        .expect_err("a foreign sandbox is a 404");
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    key_error(&hidden, ErrorCode::SANDBOX_NOT_FOUND);
    let hidden_key = access_service::key(&ctx, &other_who, created.id)
        .await
        .expect_err("a foreign key is a 404");
    assert_eq!(hidden_key.status, StatusCode::NOT_FOUND);

    let _ = std::fs::remove_dir_all(&secret_dir);
}

#[tokio::test]
async fn a_sandbox_without_a_key_says_so_instead_of_inventing_one() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let environment = environment_id(&pool).await;
    let (ctx, secret_dir) = context(&pool);
    let who = principal(environment);
    let created = sandbox(&ctx, environment, "hermes").await;

    let missing = access_service::key(&ctx, &who, created.id)
        .await
        .expect_err("no key exists yet");
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    key_error(&missing, ErrorCode::SSH_KEY_NOT_FOUND);

    // A missing sandbox is a sandbox 404, not a key 404: the two are different
    // answers for the console.
    let unknown = access_service::key(&ctx, &who, Uuid::new_v4())
        .await
        .expect_err("an unknown sandbox");
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    key_error(&unknown, ErrorCode::SANDBOX_NOT_FOUND);

    // Downloads and rotation follow the same rule.
    assert!(matches!(
        access_service::download(&ctx, &who, unknown_sandbox(&ctx, environment).await).await,
        Err(error) if error.code == ErrorCode::SSH_KEY_NOT_FOUND
    ));

    let _ = std::fs::remove_dir_all(&secret_dir);
}

/// A sandbox row that exists but has never been provisioned.
async fn unknown_sandbox(ctx: &ServiceContext, environment: Uuid) -> Uuid {
    let created = sandbox(ctx, environment, "hermes").await;
    let mut scope = TenantScope::begin(&ctx.pool, environment)
        .await
        .expect("scope");
    let absent = store::get_key(&mut scope, created.id).await;
    scope.commit().await.expect("commit");
    assert!(matches!(absent, Err(StoreError::NotFound)));
    created.id
}
