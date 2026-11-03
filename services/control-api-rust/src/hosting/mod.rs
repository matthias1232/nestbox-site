//! Provider drivers (ADR 0050). See `AGENTS.md`.

pub mod digitalocean;
pub mod hetzner;
pub mod proxmox;
pub mod vultr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

/// Why a driver could not do what it was asked.
#[derive(Debug, Error)]
pub enum HostError {
    #[error("the provider refused the request: {0}")]
    Rejected(String),
    #[error("the provider cannot be reached: {0}")]
    Unavailable(String),
    #[error("{0}")]
    Invalid(String),
}

/// A kind of machine a provider offers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HostType {
    pub name: String,
    pub cores: u32,
    pub memory_mb: u64,
    pub disk_gb: u64,
    /// Thousandths of a currency unit per hour (a price of 0.0072 is 7), `None` if unpriced.
    pub hourly_milli: Option<u64>,
    /// Where it can be created, if the provider has places.
    pub locations: Vec<String>,
}

/// What to create.
#[derive(Debug, Clone)]
pub struct HostRequest {
    pub name: String,
    pub host_type: String,
    pub location: Option<String>,
    /// The cloud-init document that runs the join command. Drivers that cannot hand user data to
    /// the machine run `join_command` another way.
    pub user_data: String,
    pub join_command: String,
}

/// A machine as the provider reports it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HostState {
    pub external_id: String,
    /// `creating`, `running` or `gone`.
    pub state: String,
    pub address: Option<String>,
}

#[async_trait]
pub trait HostDriver: Send + Sync {
    fn kind(&self) -> &'static str;
    async fn types(&self) -> Result<Vec<HostType>, HostError>;
    /// Create the machine. Returns when the provider has accepted it; the machine joins on its own.
    async fn create(&self, request: &HostRequest) -> Result<HostState, HostError>;
    async fn status(&self, external_id: &str) -> Result<HostState, HostError>;
    /// Run the join command inside a machine that has no user data of its own. A provider that
    /// hands user data to the machine (Hetzner) has nothing to do here. Safe to repeat: a spent
    /// token makes the second run stop at once.
    async fn run_join(&self, _external_id: &str, _join_command: &str) -> Result<(), HostError> {
        Ok(())
    }
    /// Delete the machine. Deleting one that is already gone is success.
    async fn destroy(&self, external_id: &str) -> Result<(), HostError>;
}

/// The cloud-init document for a machine that only has to run the join command.
///
/// `#cloud-config` with one `runcmd`. The command is a single quoted shell word: it comes from the
/// control plane (a URL with a random token), but quoting it keeps a future change honest.
pub fn cloud_init(join_command: &str) -> String {
    let quoted = join_command.replace('\'', "'\\''");
    format!(
        "#cloud-config\npackage_update: false\nruncmd:\n  - [ bash, -c, '{quoted}' ]\n"
    )
}

/// Build the driver for an account. `config` holds the non-secret settings, `secret` the token.
pub fn driver_for(kind: &str, config: &serde_json::Value, secret: &str) -> Result<Box<dyn HostDriver>, HostError> {
    match kind {
        "hetzner" => Ok(Box::new(hetzner::Hetzner::new(
            config.get("api_url").and_then(|v| v.as_str()).unwrap_or("https://api.hetzner.cloud/v1"),
            secret,
        ))),
        "proxmox" => Ok(Box::new(proxmox::Proxmox::from_config(config, secret)?)),
        "digitalocean" => Ok(Box::new(digitalocean::DigitalOcean::new(
            config.get("api_url").and_then(|v| v.as_str()).unwrap_or("https://api.digitalocean.com/v2"),
            secret,
        ))),
        "vultr" => Ok(Box::new(vultr::Vultr::new(
            config.get("api_url").and_then(|v| v.as_str()).unwrap_or("https://api.vultr.com/v2"),
            secret,
            config.get("os_id").and_then(|v| v.as_u64()).unwrap_or(2284),
        ))),
        other => Err(HostError::Invalid(format!("There is no driver for '{other}'. Supported: hetzner, digitalocean, vultr, proxmox."))),
    }
}

/// Providers a console can offer.
pub const KINDS: [&str; 4] = ["hetzner", "digitalocean", "vultr", "proxmox"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cloud_init_document_runs_exactly_the_join_command() {
        let doc = cloud_init("curl -fsSL \"http://cp/api/v1/join/abc/script\" | sudo bash");
        assert!(doc.starts_with("#cloud-config\n"));
        assert!(doc.contains("runcmd"));
        assert!(doc.contains("/api/v1/join/abc/script"));
    }

    #[test]
    fn a_quote_in_the_command_cannot_end_the_word() {
        let doc = cloud_init("echo 'x'; touch /tmp/pwned");
        assert!(doc.contains(r"'echo '\''x'\''; touch /tmp/pwned'"));
    }

    #[test]
    fn an_unknown_kind_has_no_driver() {
        assert!(driver_for("nonsense", &serde_json::json!({}), "t").is_err());
    }
}
