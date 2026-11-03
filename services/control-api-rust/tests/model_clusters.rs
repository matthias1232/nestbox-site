//! Model clusters against a real PostgreSQL (package B05).
//!
//! The suite owns `nestbox_cc_b05`:
//!
//!   createdb -h /tmp/opencode/pgtest -p 55432 nestbox_cc_b05
//!   export TEST_DATABASE_URL='postgres://core@localhost:55432/nestbox_cc_b05?host=/tmp/opencode/pgtest'
//!   cargo test --test model_clusters -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use serde_json::json;
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::model_cluster_service;
use nestbox_control_api::services::ServiceContext;

/// The database this suite owns.
const DATABASE: &str = "nestbox_cc_b05";

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

async fn default_environment(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM environments WHERE slug = 'default'")
        .fetch_one(pool)
        .await
        .expect("the default environment is seeded")
}

async fn node(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO nodes (name, display_name) VALUES ($1, $1) RETURNING id")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("insert node")
}

/// An installed model on a node, with a reachable provider profile.
async fn install(pool: &PgPool, environment_id: Uuid, key: &str, node_id: Uuid, endpoint: &str) -> Uuid {
    sqlx::query(
        "INSERT INTO models (key, name, version, parameters, state)
         VALUES ($1, $1, '1', '1B', 'not_installed') ON CONFLICT (key) DO NOTHING",
    )
    .bind(key)
    .execute(pool)
    .await
    .expect("insert model");
    let install: Uuid = sqlx::query_scalar(
        "INSERT INTO model_installs (environment_id, model_key, node_id, state)
         VALUES ($1, $2, $3, 'installed') RETURNING id",
    )
    .bind(environment_id)
    .bind(key)
    .bind(node_id)
    .fetch_one(pool)
    .await
    .expect("insert install");
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO provider_profiles
            (environment_id, name, kind, endpoint, model, status, node_id, model_install_id)
         VALUES ($1, $2, 'local_openai_compatible', $3, $4, 'reachable', $5, $6) RETURNING id",
    )
    .bind(environment_id)
    .bind(format!("local/{key}@{node_id}"))
    .bind(endpoint)
    .bind(key)
    .bind(node_id)
    .bind(install)
    .fetch_one(pool)
    .await
    .expect("insert profile");
    sqlx::query("UPDATE model_installs SET provider_profile_id = $2 WHERE id = $1")
        .bind(install)
        .bind(profile)
        .execute(pool)
        .await
        .expect("link profile");
    install
}

#[tokio::test]
async fn a_cluster_is_installs_of_one_model_behind_one_provider_profile() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let environment = default_environment(&pool).await;
    let admin = principal(environment);

    let first = node(&pool, "gpu-b05-1").await;
    let second = node(&pool, "gpu-b05-2").await;
    let one = install(&pool, environment, "b05-model", first, "http://10.77.3.4:8080").await;
    let two = install(&pool, environment, "b05-model", second, "http://10.77.3.5:8080").await;

    let created = model_cluster_service::create(
        &ctx,
        &admin,
        json!({ "name": "b05-cluster", "model_key": "b05-model", "install_ids": [one, two] }),
    )
    .await
    .expect("create the cluster");
    let id = Uuid::parse_str(created["id"].as_str().expect("an id")).expect("a uuid");
    assert_eq!(created["healthy_members"], 2);
    assert_eq!(created["members"].as_array().map(Vec::len), Some(2));
    assert!(created["endpoint"].as_str().unwrap_or("").ends_with(&format!("/c/{id}")));

    let profile_id = Uuid::parse_str(created["provider_profile_id"].as_str().expect("a profile")).expect("a uuid");
    let (endpoint, kind): (String, String) =
        sqlx::query_as("SELECT endpoint, kind FROM provider_profiles WHERE id = $1")
            .bind(profile_id)
            .fetch_one(&pool)
            .await
            .expect("the profile exists");
    assert!(endpoint.ends_with(&format!("/c/{id}")));
    assert_eq!(kind, "local_openai_compatible");

    let view = model_cluster_service::proxy_view(&ctx, id).await.expect("the proxy view");
    assert_eq!(view["members"].as_array().map(Vec::len), Some(2));
    assert_eq!(view["members"][0]["healthy"], true);

    // An install of another model is refused.
    let other = install(&pool, environment, "b05-other", first, "http://10.77.3.4:9090").await;
    let refused = model_cluster_service::add_member(&ctx, &admin, id, other, None).await;
    assert!(refused.is_err());

    model_cluster_service::delete(&ctx, &admin, id).await.expect("delete the cluster");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_profiles WHERE id = $1")
        .bind(profile_id)
        .fetch_one(&pool)
        .await
        .expect("count profiles");
    assert_eq!(left, 0);
}

#[tokio::test]
async fn the_proxy_finds_a_cluster_in_a_second_data_center() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);

    // A second data center breaks every "the appliance has one" default, which
    // is why the proxy view reads the cluster's own environment (X06).
    let organization: Uuid = sqlx::query_scalar(
        "INSERT INTO organizations (name, slug, kind)
         VALUES ('b05 second organization', 'b05-second-org', 'customer') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert the organization");
    let environment: Uuid = sqlx::query_scalar(
        "INSERT INTO environments (organization_id, name, slug)
         VALUES ($1, 'b05 second data center', 'second') RETURNING id",
    )
    .bind(organization)
    .fetch_one(&pool)
    .await
    .expect("insert the second data center");
    let admin = principal(environment);

    let node_id = node(&pool, "gpu-b05-3").await;
    let one = install(&pool, environment, "b05-second-model", node_id, "http://10.77.3.6:8080").await;
    let created = model_cluster_service::create(
        &ctx,
        &admin,
        json!({ "name": "b05-second-cluster", "model_key": "b05-second-model", "install_ids": [one] }),
    )
    .await
    .expect("create the cluster in the second data center");
    let id = Uuid::parse_str(created["id"].as_str().expect("an id")).expect("a uuid");

    // No principal and two data centers: the answer can only come from the row.
    let view = model_cluster_service::proxy_view(&ctx, id)
        .await
        .expect("the proxy view in a second data center");
    assert_eq!(view["model_key"], "b05-second-model");
    assert_eq!(view["members"].as_array().map(Vec::len), Some(1));

    // An unknown cluster is a 404, never a guess at one data center.
    assert!(model_cluster_service::proxy_view(&ctx, Uuid::new_v4()).await.is_err());

    model_cluster_service::delete(&ctx, &admin, id).await.expect("delete the cluster");
    sqlx::query("DELETE FROM environments WHERE id = $1")
        .bind(environment)
        .execute(&pool)
        .await
        .expect("remove the second data center");
    sqlx::query("DELETE FROM organizations WHERE id = $1")
        .bind(organization)
        .execute(&pool)
        .await
        .expect("remove the organization");
}
