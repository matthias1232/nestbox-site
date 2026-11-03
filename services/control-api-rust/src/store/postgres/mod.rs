//! The PostgreSQL repositories, one module per aggregate.

use crate::error::StoreError;

pub mod accounting;
pub mod agent_activity;
pub mod agent_auth;
pub mod agent_packs;
pub mod alerts;
pub mod access;
pub mod audit;
pub mod chat_turns;
pub mod auth;
pub mod control;
pub mod cudax;
pub mod egress;
pub mod egress_live;
pub mod environments;
pub mod gateways;
pub mod geography;
pub mod image_templates;
pub mod inventory;
pub mod jobs;
pub mod mcp;
pub mod media;
pub mod metrics;
pub mod login_rules;
pub mod model_clusters;
pub mod model_grants;
pub mod model_installs;
pub mod model_metrics;
pub mod model_readiness;
pub mod models;
pub mod node_gateway;
pub mod nodes;
pub mod permissions;
pub mod persistent_roots;
pub mod policies;
pub mod privacy_exceptions;
pub mod presence;
pub mod privacy;
pub mod backups;
pub mod providers;
pub mod proxies;
pub mod vault;
pub mod recipes;
pub mod sandboxes;
pub mod search;
pub mod telemetry;
pub mod terminal_sessions;
pub mod terminal_tabs;
pub mod tours;
pub mod traffic;
pub mod traffic_bodies;

/// Open the connection pool.
///
/// The pool is created with a bounded size so a burst cannot exhaust the
/// database's own connection limit, and with a statement timeout so a runaway
/// query is cancelled rather than holding a connection forever.
pub async fn connect(database_url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;

    PgPoolOptions::new()
        .max_connections(16)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Duration::from_secs(600))
        .connect(database_url)
        .await
}

/// Whether the database refused a statement because the function it names does
/// not exist.
///
/// A schema one migration behind answers SQLSTATE `42883` (`undefined_function`)
/// instead of rows. Callers that keep a fallback for such an installation check
/// this and nothing else: a broken statement, a permission error or an
/// unreachable database must surface as the 500 it is, not as a validation error
/// or an empty answer.
pub fn is_missing_function(error: &StoreError) -> bool {
    match error {
        StoreError::Database(sqlx::Error::Database(database)) => {
            database.code().as_deref() == Some("42883")
        }
        _ => false,
    }
}

/// Whether the database answers at all.
///
/// Used by the readiness probe. It deliberately does a trivial round trip rather
/// than inspecting a table: readiness means "this process can reach its store",
/// not "the schema is complete", which the migrations already guarantee.
pub async fn is_ready(pool: &sqlx::PgPool) -> bool {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
        .is_ok()
}
