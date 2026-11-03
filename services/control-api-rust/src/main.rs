//! Process start: configuration, pool, migrations, runtime adapter, serve.
//!
//! This file contains no rules. If it grows an `if` that decides something about
//! a sandbox, that decision belongs in `services/`.

use std::sync::Arc;

use nestbox_control_api::config::Config;
use nestbox_control_api::runtime::RuntimeAdapter;
use nestbox_control_api::runtime::http_runner::HttpRunner;
use nestbox_control_api::security::password;
use nestbox_control_api::state::AppState;
use nestbox_control_api::{app, store};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=warn".into()),
        )
        .init();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(message) => {
            // A configuration error must name the missing variable, not fail
            // later with a 500 on the first request.
            tracing::error!("configuration error: {message}");
            std::process::exit(2);
        }
    };
    tracing::info!(summary = %config.summary(), "starting nestbox control api");

    let pool = match store::postgres::connect(&config.database_url).await {
        Ok(pool) => pool,
        Err(error) => {
            tracing::error!(error = %error, "cannot connect to the database");
            std::process::exit(2);
        }
    };

    // `migrate` applies the schema and exits, so the API never races a migration.
    if std::env::args().any(|arg| arg == "migrate") {
        if let Err(error) = migrate(&pool, &config).await {
            tracing::error!(error = %error, "migration failed");
            std::process::exit(1);
        }
        tracing::info!("migrations complete");
        return;
    }

    // The local runner is the default; nodes that joined over the overlay are added by the
    // loop below, and the router sends each call to the machine the sandbox lives on.
    let local: Arc<dyn RuntimeAdapter> = Arc::new(HttpRunner::new(
        config.runner_url.clone(),
        config.runtime_network.clone(),
    ));
    let router = Arc::new(nestbox_control_api::runtime::router::RuntimeRouter::new(local, Some(pool.clone())));
    let runtime: Arc<dyn RuntimeAdapter> = router.clone();

    let mut state = AppState::new(config.clone(), runtime, pool);
    state.services = state.services.clone().with_router(router.clone());

    match nestbox_control_api::services::move_service::settle_interrupted(&state.services).await {
        Ok(0) => {}
        Ok(settled) => tracing::info!(settled, "marked moves interrupted by a restart as failed"),
        Err(error) => tracing::warn!(code = error.code.0, "could not settle interrupted moves"),
    }

    // Rented machines: deleted when their lifetime is over or they sit empty (ADR 0050).
    {
        let services = state.services.clone();
        let config = state.config.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(45)).await;
            loop {
                match nestbox_control_api::services::provision_service::reap(&services, &config).await {
                    Ok(0) => {}
                    Ok(count) => tracing::info!(count, "rented machines were handed back"),
                    Err(error) => tracing::debug!(code = error.code.0, "reaper pass skipped"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        });
    }

    // Snapshots of folders that work away from home, every NESTBOX_SNAPSHOT_MINUTES (default 15),
    // (pruning old ones is an administrator action: POST /api/v1/snapshots/prune, ADR 0049).
    {
        let services = state.services.clone();
        let config = state.config.clone();
        tokio::spawn(async move {
            let minutes: i64 = std::env::var("NESTBOX_SNAPSHOT_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(15).max(1);
            tokio::time::sleep(std::time::Duration::from_secs(90)).await;
            loop {
                match nestbox_control_api::services::snapshot_service::tick(&services, &config, minutes).await {
                    Ok(0) => {}
                    Ok(taken) => tracing::info!(taken, "scheduled snapshots were taken"),
                    Err(error) => tracing::debug!(code = error.code.0, "snapshot pass skipped"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        });
    }

    // Images: each joined node holds what it was assigned (ADR 0048).
    {
        let services = state.services.clone();
        let router = router.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            loop {
                match nestbox_control_api::services::image_sync_service::reconcile(&services, &router).await {
                    Ok(0) => {}
                    Ok(pulled) => tracing::info!(pulled, "node images brought in step with the registry"),
                    Err(error) => tracing::debug!(code = error.code.0, "image sync pass skipped"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    }

    // Joined nodes: load their runners into the router and look at each one.
    {
        let services = state.services.clone();
        let router = router.clone();
        let config = state.config.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) = nestbox_control_api::services::node_join_service::sync(&services, &router).await {
                    tracing::debug!(code = error.code.0, "node sync pass skipped");
                }
                if !config.overlay.dir.as_os_str().is_empty() {
                    let _ = nestbox_control_api::services::node_join_service::write_peers(&services.pool, &config.overlay.dir).await;
                }
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
        });
    }

    // A build that was running when the process died leaves its recipe
    // `building` and its job `running`. Neither will ever finish, so they are
    // settled before the first request can see them.
    match nestbox_control_api::services::builder_service::settle_interrupted(&state.services).await {
        Ok(0) => {}
        Ok(settled) => tracing::info!(settled, "settled builds interrupted by a restart"),
        Err(error) => tracing::warn!(code = error.code.0, "could not settle interrupted builds"),
    }

    // The same for an install caught between "reserved" and "answering": the
    // container it may have started is stopped and the row is marked failed,
    // instead of showing `downloading` forever.
    match nestbox_control_api::services::model_service::settle_all(&state.services).await {
        Ok(0) => {}
        Ok(settled) => tracing::info!(settled, "settled model installs interrupted by a restart"),
        Err(error) => tracing::warn!(code = error.code.0, "could not settle interrupted installs"),
    }

    // And the same for the Control Center's own jobs (bulk action, bulk model
    // install, drain, quick deploy): they run in a detached task too, so a restart
    // leaves them `running` and the dock keeps naming one that will never move.
    match nestbox_control_api::services::job_service::settle_interrupted(&state.services).await {
        Ok(0) => {}
        Ok(settled) => tracing::info!(
            settled,
            "settled Control Center jobs interrupted by a restart"
        ),
        Err(error) => tracing::warn!(code = error.code.0, "could not settle interrupted jobs"),
    }

    // The hub's registry entry follows the deployment: its address differs when the
    // sandboxes run on an OpenShell gateway. Idempotent, and only when it differs.
    let hub_url = nestbox_control_api::services::provider_service::mcp_hub_url();
    match sqlx::query("UPDATE mcp_registry SET endpoint = $1, updated_at = now() WHERE key = 'nestbox-hub' AND endpoint IS DISTINCT FROM $1")
        .bind(&hub_url)
        .execute(&state.services.pool)
        .await
    {
        Ok(done) if done.rows_affected() > 0 => tracing::info!(url = %hub_url, "pointed the MCP hub entry at this deployment"),
        Ok(_) => {}
        Err(error) => tracing::warn!(error = %error, "could not update the MCP hub entry"),
    }

    // Bring back what a reboot took down, then keep checking: the stored state is
    // the desired state, and the runtime is allowed to disagree only briefly.
    {
        let services = state.services.clone();
        tokio::spawn(async move {
            loop {
                match nestbox_control_api::services::sandbox_service::reconcile_all(&services).await {
                    Ok(0) => {}
                    Ok(changed) => tracing::info!(changed, "reconciled sandboxes with the runtime"),
                    Err(error) => tracing::debug!(code = error.code.0, "reconcile pass skipped"),
                }
                if let Err(error) =
                    nestbox_control_api::services::quick_allow::expire_due(&services).await
                {
                    tracing::warn!(code = error.code.0, message = %error.message, "rule expiry failed");
                }
                if let Err(error) =
                    nestbox_control_api::services::telemetry_service::pull_openshell_events(&services).await
                {
                    tracing::warn!(code = error.code.0, message = %error.message, "OpenShell event pull failed");
                }
                if let Err(error) =
                    nestbox_control_api::services::mcp_service::check_attached(&services).await
                {
                    tracing::debug!(code = error.code.0, "mcp health check skipped");
                }
                if let Err(error) =
                    nestbox_control_api::services::gateway_service::ensure_supervisors(&services).await
                {
                    tracing::debug!(code = error.code.0, "supervisor sync skipped");
                }
                if let Err(error) =
                    nestbox_control_api::services::sandbox_service::heal_crashed(&services).await
                {
                    tracing::debug!(code = error.code.0, "crash healing pass skipped");
                }
                if let Err(error) =
                    nestbox_control_api::services::policy_service::sync_openshell_policies(&services).await
                {
                    tracing::debug!(code = error.code.0, "OpenShell policy sync skipped");
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    }

    // The workbox's slow loops: finish an agent restart that waited for an idle
    // session, honour an egress mode's revert timer, keep warm models answering,
    // and follow each node's WireGuard tunnel.
    {
        let services = state.services.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) =
                    nestbox_control_api::services::agent_auth_service::run_pending_restarts(&services).await
                {
                    tracing::debug!(code = error.code.0, "pending agent restarts skipped");
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
        let services = state.services.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) =
                    nestbox_control_api::services::egress_live_service::revert_due(&services).await
                {
                    tracing::debug!(code = error.code.0, "egress mode revert skipped");
                }
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
        });
        let services = state.services.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) =
                    nestbox_control_api::services::model_readiness_service::keep_warm_tick(&services).await
                {
                    tracing::debug!(code = error.code.0, "keep-warm pass skipped");
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        });
        let services = state.services.clone();
        let config = state.config.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) =
                    nestbox_control_api::services::node_connect_service::refresh_tunnels(&services, &config).await
                {
                    tracing::debug!(code = error.code.0, "tunnel refresh skipped");
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    let listener = match tokio::net::TcpListener::bind(&config.listen_addr).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(address = %config.listen_addr, error = %error, "cannot bind");
            std::process::exit(2);
        }
    };
    tracing::info!(address = %config.listen_addr, "listening");

    if let Err(error) = axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        tracing::error!(error = %error, "server stopped");
    }
}

/// Apply migrations, then bootstrap the administrator if the sentinel is still
/// in place.
async fn migrate(pool: &sqlx::PgPool, config: &Config) -> Result<(), String> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|error| error.to_string())?;

    if config.bootstrap_admin {
        bootstrap_admin(pool).await.map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Replace the `!bootstrap` sentinel with a real password, printed once.
///
/// Printing a credential is normally wrong. It is acceptable exactly here: the
/// value is generated now, shown once on the operator's own terminal, and never
/// retrievable again. The alternative — a default password in a file — is worse.
async fn bootstrap_admin(pool: &sqlx::PgPool) -> Result<(), String> {
    const EMAIL: &str = "admin@nestbox.local";

    let generated = generate_password();
    let hash = password::hash_password(generated.clone()).await?;

    let changed = store::postgres::auth::set_password_if_bootstrap(pool, EMAIL, &hash)
        .await
        .map_err(|error| error.to_string())?;

    if changed {
        tracing::warn!("======================================================");
        tracing::warn!("bootstrap administrator created");
        tracing::warn!("email:    {EMAIL}");
        tracing::warn!("password: {generated}");
        tracing::warn!("this password is shown once and is not recoverable");
        tracing::warn!("======================================================");
    }
    Ok(())
}

/// A readable random password.
///
/// Hex rather than base64 so it survives a copy-paste and a phone keyboard
/// without ambiguity, at the cost of four extra characters for the same entropy.
fn generate_password() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 18];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Wait for a termination signal so a restart does not drop in-flight requests.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
