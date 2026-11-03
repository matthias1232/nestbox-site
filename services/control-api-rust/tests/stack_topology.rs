//! The stack diagram, automatic stacking and port publication, against a real PostgreSQL.
//!
//! The connect view is one read of the node's address and hostname, the
//! sandbox's published ports, and the key state. These tests prove it says why
//! nothing can be dialled, dials the hostname once one is set, and that setting
//! the endpoint is validated and needs the node permission.
//!
//!   export TEST_DATABASE_URL="postgres://core@localhost/nestbox_connect?host=/tmp/opencode&port=55432"
//!   cargo test --test stack_topology -- --test-threads=1

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::runtime::RuntimeAdapter;
use nestbox_control_api::services::sandbox_service::{self, CreateSandbox};
use nestbox_control_api::services::{access_service, stack_topology_service, ServiceContext};

const ENVIRONMENT: &str = "00000000-0000-0000-0000-000000000002";
const LOCAL_NODE: &str = "00000000-0000-0000-0000-000000000003";

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

#[tokio::test]
async fn a_tool_created_without_a_lead_joins_the_only_agent_and_not_when_there_are_two() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);

    // No agent yet: nothing to join.
    let alone = sandbox_service::create(&ctx, &principal, request("git-alone", "git")).await.expect("create");
    assert_eq!(alone.parent_id, None);

    let lead = sandbox_service::create(&ctx, &principal, request("hermes-01", "hermes")).await.expect("lead");
    assert_eq!(lead.parent_id, None, "an agent never joins anything");

    let helper = sandbox_service::create(&ctx, &principal, request("git-01", "git")).await.expect("helper");
    assert_eq!(helper.parent_id, Some(lead.id), "the only agent on the node is the lead");

    // A second agent makes the choice ambiguous, so the next helper stays alone.
    sandbox_service::create(&ctx, &principal, request("hermes-02", "hermes")).await.expect("second agent");
    let ambiguous = sandbox_service::create(&ctx, &principal, request("git-02", "git")).await.expect("ambiguous");
    assert_eq!(ambiguous.parent_id, None);
}

#[tokio::test]
async fn provisioning_opens_the_ports_on_the_runner_and_the_panel_tells_the_truth() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);
    sqlx::query("UPDATE nodes SET public_address = '192.168.55.50' WHERE id = $1")
        .bind(uuid(LOCAL_NODE))
        .execute(&pool)
        .await
        .expect("address");

    let created = sandbox_service::create(&ctx, &principal, request("oc-01", "openchamber")).await.expect("create");
    access_service::provision(&ctx, &principal, &created).await.expect("provision");

    // The runner was handed the complete set and the sandbox's public key.
    let (forwards, keys) = runtime.forwards_for("oc-01").expect("forwards were pushed");
    let purposes: Vec<&str> = forwards.iter().map(|forward| forward.purpose.as_str()).collect();
    assert_eq!(purposes.len(), 2);
    assert!(purposes.contains(&"ssh") && purposes.contains(&"agent"));
    assert_eq!(keys.len(), 1);
    assert!(keys[0].starts_with("ssh-ed25519 "));
    assert!(forwards.iter().all(|forward| (22200..=22299).contains(&forward.port)), "ports come from the new range");

    // What the runner opened is written back, so the row is no longer "pending".
    let states: Vec<String> = sqlx::query_scalar("SELECT state FROM forwards WHERE sandbox_id = $1")
        .bind(created.id)
        .fetch_all(&pool)
        .await
        .expect("states");
    assert!(states.iter().all(|state| state == "active"), "{states:?}");

    // The panel says what the port is for, and that nothing answers behind it yet.
    let info = access_service::connect(&ctx, &principal, created.id).await.expect("connect");
    assert_eq!(info.forwarding, "live");
    let web = info.ports.iter().find(|port| port.kind == "web").expect("the OpenChamber web port");
    assert_eq!(web.label.as_deref(), Some("OpenChamber web UI"));
    assert_eq!(web.listening, Some(true));
    assert_eq!(web.target_up, Some(false));
    assert!(web.url.as_deref().unwrap_or_default().starts_with("http://192.168.55.50:"));
    assert!(info.notes.iter().any(|note| note.contains("scp")));

    // A rotated key must reach the runner, or the new key is refused.
    access_service::rotate(&ctx, &principal, created.id).await.expect("rotate");
    let (_, rotated) = runtime.forwards_for("oc-01").expect("still pushed");
    assert_ne!(rotated, keys, "the runner has the new public key");

    // A runner that lost its state is healed by the next read.
    runtime.clear_forwards("oc-01").await.expect("clear");
    assert!(runtime.forwards_for("oc-01").is_none());
    access_service::connect(&ctx, &principal, created.id).await.expect("connect again");
    assert!(runtime.forwards_for("oc-01").is_some(), "the read pushed the set again");
}

#[tokio::test]
async fn the_stack_diagram_covers_the_lead_and_its_members_and_what_is_missing() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);

    let lead = sandbox_service::create(&ctx, &principal, request("hermes-01", "hermes")).await.expect("lead");
    access_service::provision(&ctx, &principal, &lead).await.expect("provision");
    let helper = sandbox_service::create(&ctx, &principal, request("git-01", "git")).await.expect("helper");
    assert_eq!(helper.parent_id, Some(lead.id));

    // Opening either member draws the same stack.
    let from_member = stack_topology_service::stack(&ctx, &principal, helper.id, None).await.expect("stack");
    let from_lead = stack_topology_service::stack(&ctx, &principal, lead.id, None).await.expect("stack");
    assert_eq!(from_member.lead, lead.id);
    assert_eq!(from_member.nodes.len(), from_lead.nodes.len());

    let kinds: Vec<&str> = from_lead.nodes.iter().map(|node| node.kind.as_str()).collect();
    for kind in ["operator", "port", "sandbox", "control_api", "runner", "egress_proxy", "internet"] {
        assert!(kinds.contains(&kind), "missing a {kind} card");
    }
    assert_eq!(kinds.iter().filter(|kind| **kind == "sandbox").count(), 2);
    assert!(from_lead.edges.iter().any(|edge| edge.kind == "stack"));
    // Every sandbox gets its supervisor when it is created, so the agent has one
    // and the diagram has nothing to flag as missing.
    assert!(from_lead.nodes.iter().any(|node| node.kind == "supervisor"));
    assert!(!from_lead.nodes.iter().any(|node| node.kind == "absent"));
    // Every component card names a log the console can open.
    let proxy = from_lead.nodes.iter().find(|node| node.kind == "egress_proxy").expect("proxy");
    assert_eq!(proxy.status, "ok");
    assert_eq!(proxy.logs.as_ref().map(|log| log.id.as_str()), Some("egress-proxy"));

    let logs = stack_topology_service::component_logs(&ctx, &principal, "egress-proxy", 50).await.expect("log");
    assert!(logs.contains("egress-proxy"));
    assert!(stack_topology_service::component_logs(&ctx, &principal, "../etc", 50).await.is_err());
}

#[tokio::test]
async fn a_sandbox_from_before_ports_existed_gets_a_way_in_when_its_panel_is_read() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);
    let legacy: Uuid = sqlx::query_scalar(
        "INSERT INTO sandboxes (environment_id, cluster_id, node_id, name, pack_key, image, status)
         VALUES ($1, '00000000-0000-0000-0000-000000000004', $2, 'legacy-01', 'hermes',
                 'nestbox-hermes:local', 'running')
         RETURNING id",
    )
    .bind(uuid(ENVIRONMENT))
    .bind(uuid(LOCAL_NODE))
    .fetch_one(&pool)
    .await
    .expect("legacy sandbox");

    let info = access_service::connect(&ctx, &principal, legacy).await.expect("connect");
    assert!(info.ports.iter().any(|port| port.kind == "ssh"), "an SSH port was claimed");
    assert!(info.key.is_some(), "a key pair was made");
    assert!(runtime.forwards_for("legacy-01").is_some(), "and the runner was told");
    assert_eq!(info.forwarding, "live");

    // Reading again claims nothing more.
    let again = access_service::connect(&ctx, &principal, legacy).await.expect("connect");
    assert_eq!(again.ports.len(), info.ports.len());
}

#[tokio::test]
async fn an_operator_key_and_a_temporary_login_reach_the_runner_and_can_be_taken_back() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);
    let created = sandbox_service::create(&ctx, &principal, request("ssh-01", "claude-code")).await.expect("create");
    access_service::provision(&ctx, &principal, &created).await.expect("provision");

    // A private key pasted by mistake is refused, a real public key is kept.
    assert!(access_service::add_user_key(&ctx, &principal, created.id, "oops", "-----BEGIN OPENSSH PRIVATE KEY-----\nx\n-----END OPENSSH PRIVATE KEY-----").await.is_err());
    let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIECZIlOh/xPXL7cP+DXG0WkxDRpM3r9xm/gaOay5jTiY laptop";
    let key = access_service::add_user_key(&ctx, &principal, created.id, "laptop", line).await.expect("add key");
    let (_, keys) = runtime.forwards_for("ssh-01").expect("pushed");
    assert_eq!(keys.len(), 2, "the generated key and the operator's");
    assert!(access_service::add_user_key(&ctx, &principal, created.id, "again", line).await.is_err(), "the same key twice is a conflict");

    // A temporary login: the password is shown once, only a hash goes anywhere.
    let issued = access_service::issue_credential(&ctx, &principal, created.id, Some(30)).await.expect("issue");
    assert!(issued.username.starts_with("tmp-") && issued.password.len() >= 16);
    let sent = runtime.credentials_for("ssh-01");
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].hash,
        nestbox_control_api::domain::ssh_access::hash_password(&sent[0].salt, &issued.password)
    );
    assert_ne!(sent[0].hash, issued.password);
    let listed = access_service::ssh_access(&ctx, &principal, created.id).await.expect("list");
    assert_eq!(listed.credentials[0].state, "active");
    let serialised = serde_json::to_string(&listed).expect("json");
    assert!(!serialised.contains(&issued.password), "a listing never carries the password");

    access_service::revoke_credential(&ctx, &principal, created.id, issued.id).await.expect("revoke");
    assert!(runtime.credentials_for("ssh-01").is_empty(), "a revoked login is no longer sent to the runner");
    access_service::remove_user_key(&ctx, &principal, created.id, key.id).await.expect("remove");
    assert_eq!(runtime.forwards_for("ssh-01").expect("pushed").1.len(), 1);
}

#[tokio::test]
async fn resources_change_live_are_remembered_and_checked() {
    let Some(pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal_of(user(&pool).await);
    let created = sandbox_service::create(&ctx, &principal, request("res-01", "claude-code")).await.expect("create");

    let answer = sandbox_service::set_resources(
        &ctx,
        &principal,
        created.id,
        sandbox_service::ResourceRequest { memory_limit: Some("1Gi".into()), cpu_limit: Some(1.5), pids_limit: Some(300), ..Default::default() },
    )
    .await
    .expect("change");
    assert_eq!(answer["applied"], "applied");
    let changes = runtime.resource_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].1, Some(1_073_741_824));
    // (name, memory, reservation, cpus, shares, pids): no reservation was named.
    assert_eq!(changes[0].2, None);
    assert_eq!(changes[0].3, Some(1.5));

    // The desired state is stored, so a restart keeps it.
    let stored = sandbox_service::get(&ctx, &principal, created.id).await.expect("get");
    assert_eq!(stored.sandbox.spec["memory_limit"], "1073741824b");
    assert_eq!(stored.sandbox.spec["cpu_limit"], 1.5);

    assert!(sandbox_service::set_resources(&ctx, &principal, created.id, sandbox_service::ResourceRequest::default()).await.is_err());
    assert!(sandbox_service::set_resources(&ctx, &principal, created.id, sandbox_service::ResourceRequest { cpu_limit: Some(500.0), ..Default::default() }).await.is_err());
    assert!(sandbox_service::set_resources(&ctx, &principal, created.id, sandbox_service::ResourceRequest { memory_limit: Some("lots".into()), ..Default::default() }).await.is_err());
}
