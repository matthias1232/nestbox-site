//! Permissions per scope against a real PostgreSQL, as the application role.
//!
//!   TEST_DATABASE_URL=postgres://… cargo test --test permissions -- --test-threads=1

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::events::bus::LocalBus;
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::domain::permissions::ScopeType;
use nestbox_control_api::services::permission_service;
use nestbox_control_api::services::{user_service, ServiceContext};

/// Own database name for this suite.
const DATABASE: &str = "nestbox_cc_b12";
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


fn as_role(user_id: Uuid, role: Role) -> Principal {
    Principal { role, ..principal_of(user_id) }
}

#[tokio::test]
async fn a_viewer_added_to_the_environment_is_listed_as_direct() {
    let Some(owner_pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set ({DATABASE})");
        return;
    };
    let _ = owner_pool;
    let pool = app_pool().await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let admin = as_role(user(&pool).await, Role::Admin);
    let env = uuid(ENVIRONMENT);
    let viewer = user_service::create(&ctx, &admin, "view@example.test", "View", "viewer-password-1", "viewer")
        .await
        .expect("viewer");

    permission_service::set(&ctx, &admin, viewer.id, ScopeType::Environment, env, "viewer")
        .await
        .expect("set");
    let listed = permission_service::list(&ctx, &admin, ScopeType::Environment, env).await.expect("list");
    let items = listed["items"].as_array().expect("items");
    let row = items
        .iter()
        .find(|item| item["user_id"] == serde_json::json!(viewer.id))
        .expect("viewer listed");
    assert_eq!(row["role"], "viewer");
    assert_eq!(row["via"], "direct");

    let member = as_role(viewer.id, Role::Member);
    assert!(permission_service::list(&ctx, &member, ScopeType::Environment, env).await.is_err());
}

#[tokio::test]
async fn the_only_admin_cannot_be_removed() {
    let Some(owner_pool) = connect().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set ({DATABASE})");
        return;
    };
    let _ = owner_pool;
    let pool = app_pool().await;
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let env = uuid(ENVIRONMENT);
    let admin_id = user(&pool).await;
    let admin = as_role(admin_id, Role::Admin);
    // The seeded admin of the environment is the only direct admin: remove the others first.
    sqlx::query("DELETE FROM user_scopes WHERE environment_id = $1 AND role = 'admin'")
        .bind(env)
        .execute(&pool)
        .await
        .ok();
    permission_service::set(&ctx, &admin, admin_id, ScopeType::Environment, env, "admin")
        .await
        .expect("make admin");

    let removed = permission_service::remove(&ctx, &admin, admin_id, ScopeType::Environment, env).await;
    let error = removed.expect_err("last admin refused");
    assert_eq!(error.status, axum::http::StatusCode::CONFLICT);
    let demoted = permission_service::set(&ctx, &admin, admin_id, ScopeType::Environment, env, "viewer").await;
    assert!(demoted.is_err());
}
