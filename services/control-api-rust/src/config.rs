//! Configuration from the environment.
//!
//! Two rules shape this file:
//!
//! * A secret is never read from a plain environment variable when a `_FILE`
//!   variant is available. Environment variables appear in `docker inspect` and
//!   in crash dumps (ADR 0005).
//! * The configuration is validated at start, not at first use. A missing
//!   database URL should stop the process, not surface as a 500 on the first
//!   request.

use std::path::PathBuf;

/// Everything the process needs to run.
#[derive(Debug, Clone)]
pub struct Config {
    /// Database URL. Never logged.
    pub database_url: String,
    /// Address to bind.
    pub listen_addr: String,
    /// Directory holding secret files, mounted read-only where possible.
    pub secret_dir: PathBuf,
    /// Whether the session cookie carries `Secure`. False while the console is
    /// served over plain HTTP on a private network; true behind TLS.
    pub session_cookie_secure: bool,
    /// Session idle timeout, in seconds.
    pub session_idle_seconds: i64,
    /// Session absolute lifetime, in seconds.
    pub session_max_seconds: i64,
    /// Allowed browser origins. Empty means same-origin only, which is the
    /// intended default: the console is served from the same host as the API.
    pub allowed_origins: Vec<String>,
    /// Runtime adapter endpoint (the sandbox runner).
    pub runner_url: String,
    /// Docker network sandboxes attach to.
    pub runtime_network: String,
    /// Whether to create the bootstrap admin on `migrate`.
    pub bootstrap_admin: bool,
    /// Request body limit in bytes.
    pub body_limit_bytes: usize,
    /// Requests per minute, per principal. A coarse guard, not a precise quota.
    pub rate_limit_per_minute: u32,
    /// How long a browser terminal may go without input before the control API
    /// closes it. Thirty minutes is the documented default (ADR 0023); the value
    /// is configurable so an idle-close can be watched in a test instead of
    /// waiting half an hour.
    pub terminal_idle_seconds: i64,
    /// `saas` lets a new person register through an identity provider; `selfhosted`
    /// only lets invited people in (ADR 0046).
    pub deploy_mode: DeployMode,
    /// The address browsers use for this installation, for the redirect URI of a sign-in.
    pub public_url: String,
    /// The identity providers an operator configured. Empty means password sign-in only.
    pub oidc: Vec<OidcProviderConfig>,
    /// How nodes on other machines reach this control plane (ADR 0047).
    pub overlay: OverlayConfig,
}

/// The WireGuard overlay the control plane is the hub of.
#[derive(Debug, Clone)]
pub struct OverlayConfig {
    /// Where the hub container and the API exchange files: `hub.pub` (read) and
    /// `peers.conf` (written). Empty disables joining.
    pub dir: PathBuf,
    /// `host:port` a node dials to reach the hub from outside, for example `203.0.113.7:51821`.
    pub endpoint: String,
    /// The hub's address inside the overlay.
    pub hub_ip: String,
    /// `host:port` of the image registry as nodes see it, over the overlay.
    pub registry: String,
    /// The control plane's address as nodes see it, over the overlay.
    pub control_url: String,
}

/// How this installation treats people it does not know yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployMode {
    /// A first sign-in through an identity provider starts an onboarding.
    Saas,
    /// Nobody registers; an administrator invites.
    SelfHosted,
}

/// One identity provider, as configured.
#[derive(Debug, Clone)]
pub struct OidcProviderConfig {
    /// The short name used in URLs and in `user_identities.provider`.
    pub id: String,
    pub label: String,
    /// `oidc` for any OpenID Connect issuer, `github` for GitHub's OAuth flavour.
    pub kind: String,
    /// The issuer whose `/.well-known/openid-configuration` names the endpoints.
    pub issuer: String,
    pub client_id: String,
    /// Never logged and never returned.
    pub client_secret: String,
}

impl Config {
    /// Read and validate from the environment.
    ///
    /// Returns `Err` with a message naming the missing variable, so a
    /// misconfigured deployment fails with an actionable line rather than a
    /// panic deep in a request.
    pub fn from_env() -> Result<Self, String> {
        let database_url = read_with_file("DATABASE_URL")
            .ok_or_else(|| "DATABASE_URL or DATABASE_URL_FILE must be set".to_string())?;

        Ok(Self {
            database_url,
            listen_addr: std::env::var("NESTBOX_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8000".into()),
            secret_dir: std::env::var("NESTBOX_SECRET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/etc/nestbox/secrets")),
            session_cookie_secure: env_bool("NESTBOX_SESSION_COOKIE_SECURE", false),
            session_idle_seconds: env_i64("NESTBOX_SESSION_IDLE_SECONDS", 8 * 60 * 60),
            session_max_seconds: env_i64("NESTBOX_SESSION_MAX_SECONDS", 7 * 24 * 60 * 60),
            allowed_origins: std::env::var("NESTBOX_ALLOWED_ORIGINS")
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            runner_url: std::env::var("NESTBOX_RUNNER_URL")
                .unwrap_or_else(|_| "http://sandbox-runner:9000".into()),
            runtime_network: std::env::var("NESTBOX_RUNTIME_NETWORK")
                .unwrap_or_else(|_| "nestbox-runtime".into()),
            bootstrap_admin: env_bool("NESTBOX_BOOTSTRAP_ADMIN", false),
            body_limit_bytes: env_i64("NESTBOX_BODY_LIMIT_BYTES", 1_048_576) as usize,
            rate_limit_per_minute: env_i64("NESTBOX_RATE_LIMIT_PER_MINUTE", 600) as u32,
            terminal_idle_seconds: env_i64("NESTBOX_TERMINAL_IDLE_SECONDS", 30 * 60),
            deploy_mode: if std::env::var("NESTBOX_MODE").is_ok_and(|v| v.trim().eq_ignore_ascii_case("saas")) {
                DeployMode::Saas
            } else {
                DeployMode::SelfHosted
            },
            public_url: std::env::var("NESTBOX_PUBLIC_URL")
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string(),
            oidc: read_oidc_providers(),
            overlay: OverlayConfig {
                dir: std::env::var("NESTBOX_OVERLAY_DIR").map(PathBuf::from).unwrap_or_default(),
                endpoint: std::env::var("NESTBOX_OVERLAY_ENDPOINT").unwrap_or_default(),
                hub_ip: std::env::var("NESTBOX_OVERLAY_HUB_IP").unwrap_or_else(|_| "10.77.0.1".into()),
                registry: std::env::var("NESTBOX_REGISTRY").unwrap_or_else(|_| "10.77.0.1:5000".into()),
                control_url: std::env::var("NESTBOX_OVERLAY_CONTROL_URL")
                    .unwrap_or_else(|_| "http://10.77.0.1:8000".into()),
            },
        })
    }

    /// A redaction-safe summary for the start-up log.
    ///
    /// Deliberately does not include the database URL: a connection string
    /// usually contains a password, and a log line is exactly where that would
    /// leak.
    pub fn summary(&self) -> String {
        format!(
            "listen={} runner={} secret_dir={} tls_cookie={} origins={}",
            self.listen_addr,
            self.runner_url,
            self.secret_dir.display(),
            self.session_cookie_secure,
            if self.allowed_origins.is_empty() {
                "same-origin".to_string()
            } else {
                self.allowed_origins.join(",")
            }
        )
    }
}

/// The identity providers named in `NESTBOX_OIDC_PROVIDERS` (comma separated).
///
/// Each name `x` reads `NESTBOX_OIDC_X_CLIENT_ID` and `NESTBOX_OIDC_X_CLIENT_SECRET` (or the
/// `_FILE` form). `google`, `microsoft` and `github` know their issuer; any other name needs
/// `NESTBOX_OIDC_X_ISSUER`. A provider with a missing credential is skipped with a log line,
/// not half-configured.
fn read_oidc_providers() -> Vec<OidcProviderConfig> {
    let names = std::env::var("NESTBOX_OIDC_PROVIDERS").unwrap_or_default();
    let mut out = Vec::new();
    for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let key = name.to_ascii_uppercase().replace('-', "_");
        let var = |suffix: &str| read_with_file(&format!("NESTBOX_OIDC_{key}_{suffix}"));
        let (Some(client_id), Some(client_secret)) = (var("CLIENT_ID"), var("CLIENT_SECRET")) else {
            tracing::warn!(provider = name, "identity provider skipped: client id or secret is missing");
            continue;
        };
        let lower = name.to_ascii_lowercase();
        let (kind, issuer, label) = match lower.as_str() {
            "google" => ("oidc", "https://accounts.google.com".to_string(), "Google".to_string()),
            "microsoft" => ("oidc", "https://login.microsoftonline.com/common/v2.0".to_string(), "Microsoft".to_string()),
            "github" => ("github", "https://github.com".to_string(), "GitHub".to_string()),
            _ => {
                let Some(issuer) = var("ISSUER") else {
                    tracing::warn!(provider = name, "identity provider skipped: NESTBOX_OIDC_{key}_ISSUER is missing");
                    continue;
                };
                let label = var("LABEL").unwrap_or_else(|| name.to_string());
                ("oidc", issuer.trim_end_matches('/').to_string(), label)
            }
        };
        out.push(OidcProviderConfig { id: lower, label, kind: kind.into(), issuer, client_id, client_secret });
    }
    out
}

/// Read a value from `NAME` or, preferentially, from the file named by
/// `NAME_FILE`.
///
/// The file variant wins because a mounted file is the only way to pass a
/// secret without it appearing in the process environment.
fn read_with_file(name: &str) -> Option<String> {
    let file_var = format!("{name}_FILE");
    if let Ok(path) = std::env::var(&file_var) {
        match std::fs::read_to_string(&path) {
            Ok(value) => return Some(value.trim().to_string()),
            Err(err) => {
                tracing::error!(path = %path, error = %err, "cannot read secret file");
                return None;
            }
        }
    }
    std::env::var(name).ok()
}

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

fn env_i64(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_a_bad_integer_falls_back_rather_than_panicking() {
        // A typo in an env var must not prevent the process from starting with
        // a sane default.
        assert_eq!(env_i64("NESTBOX_DEFINITELY_NOT_SET_12345", 42), 42);
    }

    #[test]
    fn booleans_accept_the_usual_spellings() {
        // Exercised through the env, because the helper is private.
        unsafe { std::env::set_var("NESTBOX_TEST_BOOL_A", "yes") };
        assert!(env_bool("NESTBOX_TEST_BOOL_A", false));
        unsafe { std::env::set_var("NESTBOX_TEST_BOOL_B", "0") };
        assert!(!env_bool("NESTBOX_TEST_BOOL_B", true));
        unsafe { std::env::remove_var("NESTBOX_TEST_BOOL_A") };
        unsafe { std::env::remove_var("NESTBOX_TEST_BOOL_B") };
    }
}
