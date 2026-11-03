//! Policy preview against a real PostgreSQL (package B10, contract C11).
//!
//!   TEST_DATABASE_URL=postgres://… cargo test --test policy_preview -- --test-threads=1

use std::str::FromStr;
use std::sync::Arc;

use serde_json::json;
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

use nestbox_control_api::domain::auth::{CredentialSource, Principal, Role};
use nestbox_control_api::runtime::fake::FakeRuntime;
use nestbox_control_api::services::{policy_preview_service, ServiceContext};

/// The database this suite owns.
const DATABASE: &str = "nestbox_cc_b10";

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

#[tokio::test]
async fn a_network_and_a_filesystem_section_split_into_hot_and_cold() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal(default_environment(&pool).await);

    let spec = json!({
        "version": 1,
        "filesystem_policy": { "read_only": ["/etc"], "read_write": ["/home/nestbox"] },
        "network_policies": {
            "github": { "name": "github", "endpoints": [{ "host": "github.com", "port": 443, "protocol": "rest" }] }
        }
    });
    let answer = policy_preview_service::preview(&ctx, &principal, json!({ "spec": spec, "sandbox_id": null }))
        .await
        .expect("preview");
    assert_eq!(answer["valid"], true);
    assert_eq!(answer["hot"], json!(["network_policies"]));
    assert_eq!(answer["needs_recreate"], json!(["filesystem_policy"]));
    assert!(!answer["compiled_yaml"].as_str().unwrap_or_default().is_empty());
}

#[tokio::test]
async fn an_invalid_spec_reports_errors_and_no_document() {
    let Some(pool) = pool().await else {
        eprintln!("skipping: TEST_DATABASE_URL is not set");
        return;
    };
    let runtime = Arc::new(FakeRuntime::new());
    let ctx = context(&pool, &runtime);
    let principal = principal(default_environment(&pool).await);

    let answer = policy_preview_service::preview(&ctx, &principal, json!({ "spec": { "nothing": {} } }))
        .await
        .expect("preview");
    assert_eq!(answer["valid"], false);
    assert!(!answer["errors"].as_array().map(Vec::is_empty).unwrap_or(true));
    assert_eq!(answer["compiled_yaml"], "");
}
