//! Model grants against a real database.
//!
//! These prove what a pure function cannot: that a grant written by an
//! administrator reaches a sandbox the caller only owns, that a member cannot
//! widen their own models, that the narrower scope wins, that the payload the
//! accounting proxy routes with names the upstream and its base, and that the
//! proxy's sandbox name resolves to that sandbox's own data center — with a 409
//! when the name exists in two, and a 404 when it exists in none (review
//! finding F0).
//!
//! The suite owns `nestbox_wb_b16`, so it can run next to the other suites:
//!
//!   createdb -h /tmp/opencode/pgtest -p 55432 nestbox_wb_b16
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_wb_b16?host=/tmp/opencode/pgtest'
//!   cargo test --test model_grants -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use serde_json::json;
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::domain::model_grant::NewModelGrant;
use nestbox_control_api::error::ErrorCode;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::model_grant_service;
use nestbox_control_api::services::ServiceContext;

/// The database this suite owns.
const DATABASE: &str = "nestbox_wb_b16";

/// The seeded administrator, which every audit trail foreign key accepts.
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

async fn pool() -> Option<PgPool> {
    if !ready().await {
        return None;
    }
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let options = PgConnectOptions::from_str(&url).ok()?;
    sqlx::PgPool::connect_with(options.database(DATABASE)).await.ok()
}

fn principal(environment_id: Uuid, user_id: Uuid, role: Role) -> Principal {
    Principal {
        user_id,
        environment_id,
        role,
        display_name: format!("Test {}", role.as_str()),
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

async fn default_environment(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .expect("the default environment is seeded")
}

/// A member of the environment, distinct from the seeded administrator.
async fn member(pool: &PgPool, environment_id: Uuid, email: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (organization_id, email, display_name, password_hash, role, status)
         VALUES ('00000000-0000-0000-0000-000000000001', $1, $1, '!bootstrap', 'member', 'active')
         ON CONFLICT (organization_id, email) DO UPDATE SET display_name = EXCLUDED.display_name
         RETURNING id",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("insert member");
    // The membership row the session would normally imply; the permission checks
    // in the service read the role from the principal, not from here.
    sqlx::query(
        "INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, 'member')
         ON CONFLICT (user_id, environment_id) DO NOTHING",
    )
    .bind(id)
    .bind(environment_id)
    .execute(pool)
    .await
    .expect("scope the member to the environment");
    id
}

/// A running sandbox owned by `owner_id`, on its own cluster and node.
async fn sandbox(pool: &PgPool, environment_id: Uuid, name: &str, owner_id: Uuid) -> Uuid {
    sandbox_row(pool, environment_id, name, name, owner_id).await
}

/// A running sandbox whose cluster and node are labelled `label`.
///
/// A node's name is unique across the appliance, so the same sandbox name in a
/// second data center needs a second node: the label is what keeps the two rows
/// apart while the sandbox keeps the name the test asks about.
async fn sandbox_row(
    pool: &PgPool,
    environment_id: Uuid,
    name: &str,
    label: &str,
    owner_id: Uuid,
) -> Uuid {
    let cluster: Uuid = sqlx::query_scalar(
        "INSERT INTO clusters (environment_id, name, slug) VALUES ($1, $2, $2) RETURNING id",
    )
    .bind(environment_id)
    .bind(format!("c-{label}"))
    .fetch_one(pool)
    .await
    .expect("insert cluster");
    let node: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(format!("n-{label}"))
    .fetch_one(pool)
    .await
    .expect("insert node");
    sqlx::query_scalar(
        "INSERT INTO sandboxes
            (environment_id, cluster_id, node_id, name, pack_key, image, spec, status, owner_user_id)
         VALUES ($1, $2, $3, $4, 'opencode', 'ghcr.io/nestbox/opencode@sha256:abc', '{}', 'running', $5)
         RETURNING id",
    )
    .bind(environment_id)
    .bind(cluster)
    .bind(node)
    .bind(name)
    .bind(owner_id)
    .fetch_one(pool)
    .await
    .expect("insert sandbox")
}

/// A second data center, so a sandbox name can resolve somewhere other than
/// `default` — the state the appliance is in since "Berlin" exists.
async fn second_environment(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug)
         VALUES ('00000000-0000-0000-0000-000000000001', $1, $1)
         RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("insert environment")
}

/// A provider profile the grants can point at.
async fn profile(pool: &PgPool, environment_id: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO provider_profiles (environment_id, name, kind, endpoint, labels)
         VALUES ($1, $2, 'openai_compatible', 'http://model-b16:8000/v1', $3)
         RETURNING id",
    )
    .bind(environment_id)
    .bind(name)
    .bind(json!({}))
    .fetch_one(pool)
    .await
    .expect("insert provider profile")
}

fn request(scope: &str, model_name: &str, provider_profile_id: Uuid) -> NewModelGrant {
    NewModelGrant {
        scope: scope.to_string(),
        user_id: None,
        sandbox_id: None,
        model_name: model_name.to_string(),
        provider_profile_id,
        upstream_model: None,
        is_default: false,
        keep_warm: false,
    }
}

#[tokio::test]
async fn an_environment_grant_reaches_a_sandbox_its_member_owns() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let admin = principal(environment, Uuid::parse_str(ADMIN).expect("a uuid"), Role::Admin);
    let owner_id = member(&pool, environment, "b16-owner@nestbox.local").await;
    let sandbox_id = sandbox(&pool, environment, "b16-env-grant", owner_id).await;
    let profile_id = profile(&pool, environment, "b16-profile-env").await;

    let mut body = request("environment", "b16-fast", profile_id);
    body.upstream_model = Some("qwen3:8b".to_string());
    body.is_default = true;
    let grant = model_grant_service::create(&ctx, &admin, body)
        .await
        .expect("an admin grants a model to the environment");
    assert_eq!(grant.scope, "environment");
    assert!(grant.user_id.is_none() && grant.sandbox_id.is_none());

    let owner = principal(environment, owner_id, Role::Member);
    let models = model_grant_service::effective_for_sandbox(&ctx, &owner, sandbox_id)
        .await
        .expect("the owner sees the sandbox's models");
    let model = models
        .iter()
        .find(|model| model.model_name == "b16-fast")
        .expect("the environment grant applies");
    assert_eq!(model.upstream_model, "qwen3:8b");
    assert_eq!(model.source, "environment");
    assert!(model.is_default, "the only model is the default");
}

#[tokio::test]
async fn a_member_cannot_grant_to_the_environment() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let member_id = member(&pool, environment, "b16-refused@nestbox.local").await;
    let profile_id = profile(&pool, environment, "b16-profile-refused").await;
    let caller = principal(environment, member_id, Role::Member);

    let environment_grant = model_grant_service::create(
        &ctx,
        &caller,
        request("environment", "b16-forbidden", profile_id),
    )
    .await
    .expect_err("a member cannot grant to the whole environment");
    assert_eq!(environment_grant.status, axum::http::StatusCode::FORBIDDEN);

    let mut user_body = request("user", "b16-forbidden", profile_id);
    user_body.user_id = Some(member_id);
    let user_grant = model_grant_service::create(&ctx, &caller, user_body)
        .await
        .expect_err("a member cannot grant to a user either");
    assert_eq!(user_grant.status, axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_sandbox_grant_beats_the_environment_grant_of_the_same_name() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let admin = principal(environment, Uuid::parse_str(ADMIN).expect("a uuid"), Role::Admin);
    let owner_id = member(&pool, environment, "b16-owner-narrow@nestbox.local").await;
    let sandbox_id = sandbox(&pool, environment, "b16-narrow", owner_id).await;
    let environment_profile = profile(&pool, environment, "b16-profile-wide").await;
    let sandbox_profile = profile(&pool, environment, "b16-profile-narrow").await;

    model_grant_service::create(
        &ctx,
        &admin,
        request("environment", "b16-shared", environment_profile),
    )
    .await
    .expect("the environment grant");

    // The owner may narrow their own sandbox: write access to it is the rule.
    let owner = principal(environment, owner_id, Role::Member);
    let mut body = request("sandbox", "b16-shared", sandbox_profile);
    body.sandbox_id = Some(sandbox_id);
    body.is_default = true;
    model_grant_service::create(&ctx, &owner, body)
        .await
        .expect("the owner grants to their own sandbox");

    let models = model_grant_service::effective_for_sandbox(&ctx, &owner, sandbox_id)
        .await
        .expect("the effective list");
    let shared: Vec<_> = models
        .iter()
        .filter(|model| model.model_name == "b16-shared")
        .collect();
    assert_eq!(shared.len(), 1, "one row per model name");
    assert_eq!(shared[0].source, "sandbox");
    assert_eq!(shared[0].provider_profile_id, sandbox_profile);
    assert!(shared[0].is_default, "the narrower grant asked for the default");
}

#[tokio::test]
async fn the_accounting_router_is_told_which_upstream_serves_a_model() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let admin = principal(environment, Uuid::parse_str(ADMIN).expect("a uuid"), Role::Admin);
    let owner_id = member(&pool, environment, "b16-owner-router@nestbox.local").await;
    let sandbox_name = "b16-router";
    sandbox(&pool, environment, sandbox_name, owner_id).await;
    let profile_id = profile(&pool, environment, "b16-profile-router").await;

    let mut body = request("environment", "b16-routed", profile_id);
    body.upstream_model = Some("qwen3:8b".to_string());
    body.is_default = true;
    model_grant_service::create(&ctx, &admin, body)
        .await
        .expect("the environment grant");

    let routes = model_grant_service::routes_for_accounting(&ctx, sandbox_name)
        .await
        .expect("the router payload");
    assert_eq!(routes["default"], json!("b16-routed"));
    let models = routes["models"].as_array().expect("a model list");
    let routed = models
        .iter()
        .find(|model| model["model_name"] == json!("b16-routed"))
        .expect("the granted model");
    assert_eq!(routed["upstream_base"], json!("http://model-b16:8000"));
    assert_eq!(routed["upstream_model"], json!("qwen3:8b"));
    assert!(
        routed["authorization"].is_null(),
        "a profile without a credential sends none"
    );
}

/// The proxy names a sandbox while the appliance holds two data centers.
///
/// The sandbox's own row answers which data center it is, so its granted models
/// route and its usage records — both paths answered 422 as soon as a second data
/// center existed (review finding F0).
#[tokio::test]
async fn a_sandbox_name_answers_the_data_center_it_lives_in() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let elsewhere = second_environment(&pool, "b16-elsewhere").await;
    let admin = principal(elsewhere, Uuid::parse_str(ADMIN).expect("a uuid"), Role::Admin);
    let owner_id = member(&pool, elsewhere, "b16-owner-elsewhere@nestbox.local").await;
    let sandbox_name = "b16-far";
    sandbox(&pool, elsewhere, sandbox_name, owner_id).await;
    let profile_id = profile(&pool, elsewhere, "b16-profile-elsewhere").await;

    let mut body = request("environment", "b16-far-model", profile_id);
    body.upstream_model = Some("qwen3:8b".to_string());
    body.is_default = true;
    model_grant_service::create(&ctx, &admin, body)
        .await
        .expect("the grant in the second data center");

    let data_centers: i64 = sqlx::query_scalar("SELECT count(*) FROM environments")
        .fetch_one(&pool)
        .await
        .expect("count the data centers");
    assert!(
        data_centers >= 2,
        "the appliance holds more than one data center, which is the case this covers"
    );

    let routes = model_grant_service::routes_for_accounting(&ctx, sandbox_name)
        .await
        .expect("the sandbox's own data center answers");
    assert_eq!(routes["default"], json!("b16-far-model"));
}

/// A name that exists in two data centers is refused, not matched to the one
/// that sorts first: routing another tenant's requests is exactly the failure a
/// guess makes silent.
#[tokio::test]
async fn a_name_in_two_data_centers_is_refused_rather_than_guessed() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let here = default_environment(&pool).await;
    let there = second_environment(&pool, "b16-twin").await;
    let owner_here = member(&pool, here, "b16-owner-twin-here@nestbox.local").await;
    let owner_there = member(&pool, there, "b16-owner-twin-there@nestbox.local").await;
    sandbox_row(&pool, here, "b16-twin", "b16-twin-here", owner_here).await;
    sandbox_row(&pool, there, "b16-twin", "b16-twin-there", owner_there).await;

    let refused = model_grant_service::routes_for_accounting(&ctx, "b16-twin")
        .await
        .expect_err("an ambiguous name is not routed");
    assert_eq!(refused.status, axum::http::StatusCode::CONFLICT);
    assert_eq!(refused.code, ErrorCode::CONFLICT);
}

/// A name no data center holds is a 404, not the first data center's empty list.
#[tokio::test]
async fn an_unknown_sandbox_name_is_not_found() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);

    let missing = model_grant_service::routes_for_accounting(&ctx, "b16-nobody")
        .await
        .expect_err("a name no data center holds has no routes");
    assert_eq!(missing.status, axum::http::StatusCode::NOT_FOUND);
    assert_eq!(missing.code, ErrorCode::SANDBOX_NOT_FOUND);
}
