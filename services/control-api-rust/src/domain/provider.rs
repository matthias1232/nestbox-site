//! Provider profiles: the shape that makes local and cloud models the same
//! thing.
//!
//! A profile is a `kind`, an `endpoint`, a `model`, and a `secret_ref`. A local
//! model is the same row with a loopback endpoint. That is why the policy does
//! not change between them: only the profile does.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Whether a provider answers probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    /// Created for an install whose server is not up yet.
    Pending,
    /// The last probe succeeded.
    Reachable,
    /// The last probe failed.
    Unreachable,
}

impl ProviderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Reachable => "reachable",
            Self::Unreachable => "unreachable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "reachable" => Self::Reachable,
            "unreachable" => Self::Unreachable,
            _ => return None,
        })
    }
}

/// What family a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// NVIDIA cloud inference endpoints.
    Nvidia,
    /// Its own API shape.
    Openai,
    Anthropic,
    /// An OpenAI-compatible server on the overlay: vLLM, llama.cpp, NIM local.
    OpenaiCompatible,
    /// An OpenAI-compatible server on this machine.
    LocalVllm,
    /// The server an installed model runs as, on its node: one container per
    /// install, `http://model-<key>:8000/v1` on the overlay.
    LocalOpenaiCompatible,
    /// The OmniRoute gateway used by the appliance itself.
    Omniroute,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nvidia => "nvidia",
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::OpenaiCompatible => "openai_compatible",
            Self::LocalVllm => "local_vllm",
            Self::LocalOpenaiCompatible => "local_openai_compatible",
            Self::Omniroute => "omniroute",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "nvidia" => Self::Nvidia,
            "openai" => Self::Openai,
            "anthropic" => Self::Anthropic,
            "openai_compatible" => Self::OpenaiCompatible,
            "local_vllm" => Self::LocalVllm,
            "local_openai_compatible" => Self::LocalOpenaiCompatible,
            "omniroute" => Self::Omniroute,
            _ => return None,
        })
    }

    /// Whether the endpoint is on this machine.
    ///
    /// This is what the console uses to label a model "local" rather than
    /// "cloud", and it is derived from the profile rather than stored twice.
    pub fn is_local(self) -> bool {
        matches!(self, Self::LocalVllm | Self::LocalOpenaiCompatible)
    }
}

/// A provider profile as stored and returned.
///
/// Note what is absent: the credential. `secret_ref` is a locator, and the
/// response reports `credential_configured` instead of a value (ADR 0005).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderProfile {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub name: String,
    pub kind: ProviderKind,
    pub endpoint: String,
    pub model: Option<String>,
    /// A locator such as `file:///etc/nestbox/secrets/nvidia`.
    pub secret_ref: Option<String>,
    /// Whether a credential is configured. Never the credential itself.
    pub credential_configured: bool,
    pub credential_expiry: Option<DateTime<Utc>>,
    pub labels: serde_json::Value,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: ProviderStatus,
    pub node_id: Option<Uuid>,
    pub model_install_id: Option<Uuid>,
    pub last_probed_at: Option<DateTime<Utc>>,
    /// The exit this provider is called through, when one is chosen. `None`
    /// is the node's own route.
    pub proxy_id: Option<Uuid>,
}

impl ProviderProfile {
    /// Whether the credential is close enough to expiry that the refresh job
    /// should act. Fifteen minutes matches the refresh schedule.
    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        match self.credential_expiry {
            Some(expiry) => expiry <= now + chrono::Duration::minutes(15),
            None => false,
        }
    }

    /// Whether this profile can be used to serve requests right now.
    pub fn is_usable(&self) -> bool {
        self.credential_configured || self.kind.is_local()
    }
}

/// A credential locator, as stored. Never a value.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SecretReference {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub name: String,
    /// A URI whose scheme selects the resolver: `file://`, `vault://`, `k8s://`.
    pub locator: String,
    pub kind: SecretKind,
    pub rotated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    ApiKey,
    TlsKey,
    Password,
    Token,
    CertificateAuthority,
}

impl SecretKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::TlsKey => "tls_key",
            Self::Password => "password",
            Self::Token => "token",
            Self::CertificateAuthority => "ca",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "api_key" => Self::ApiKey,
            "tls_key" => Self::TlsKey,
            "password" => Self::Password,
            "token" => Self::Token,
            "ca" => Self::CertificateAuthority,
            _ => return None,
        })
    }
}

/// Validate a locator.
///
/// A locator is a URI with a scheme the resolver understands. Validating here
/// means an unreadable reference is rejected at write time rather than failing
/// silently at use time, when the failure looks like "the model is down".
pub fn validate_locator(locator: &str) -> Result<(), String> {
    let Some((scheme, rest)) = locator.split_once("://") else {
        return Err("locator must be a URI such as file:///path or vault://path".into());
    };
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '+' || c == '-' || c == '.') {
        return Err("locator scheme is invalid".into());
    }
    if rest.is_empty() {
        return Err("locator must have a path".into());
    }
    // The schemes this build can resolve. A new backend adds a scheme here and
    // a resolver, with no schema change (ADR 0005).
    match scheme {
        "file" | "vault" | "k8s" | "env" => Ok(()),
        other => Err(format!(
            "unsupported locator scheme '{other}'; supported: file, vault, k8s, env"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_locator_is_valid() {
        assert!(validate_locator("file:///etc/nestbox/secrets/nvidia").is_ok());
    }

    #[test]
    fn a_bare_path_is_not_a_locator() {
        // Storing a bare path would make the resolver guess the scheme.
        assert!(validate_locator("/etc/nestbox/secrets/nvidia").is_err());
    }

    #[test]
    fn an_unknown_scheme_is_rejected_with_a_useful_message() {
        let err = validate_locator("ftp://host/x").unwrap_err();
        assert!(err.contains("unsupported locator scheme"));
    }

    #[test]
    fn a_scheme_without_a_path_is_rejected() {
        assert!(validate_locator("file://").is_err());
    }

    #[test]
    fn a_local_provider_is_usable_without_a_credential() {
        let profile = ProviderProfile {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            name: "local".into(),
            kind: ProviderKind::LocalVllm,
            endpoint: "http://10.244.0.5:8000/v1".into(),
            model: Some("nvidia/Nemotron".into()),
            secret_ref: None,
            credential_configured: false,
            credential_expiry: None,
            labels: serde_json::json!({}),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            status: ProviderStatus::Reachable,
            node_id: None,
            model_install_id: None,
            last_probed_at: None,
        proxy_id: None,
        };
        assert!(profile.is_usable());
    }

    #[test]
    fn a_cloud_provider_without_a_credential_is_not_usable() {
        let profile = ProviderProfile {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            name: "cloud".into(),
            kind: ProviderKind::Nvidia,
            endpoint: "https://integrate.api.nvidia.com/v1".into(),
            model: None,
            secret_ref: Some("file:///etc/nestbox/secrets/nvidia".into()),
            credential_configured: false,
            credential_expiry: None,
            labels: serde_json::json!({}),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            status: ProviderStatus::Reachable,
            node_id: None,
            model_install_id: None,
            last_probed_at: None,
        proxy_id: None,
        };
        assert!(!profile.is_usable());
    }

    #[test]
    fn an_expiring_credential_needs_refresh() {
        let now = Utc::now();
        let mut profile = ProviderProfile {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            name: "c".into(),
            kind: ProviderKind::Nvidia,
            endpoint: "https://x/v1".into(),
            model: None,
            secret_ref: None,
            credential_configured: true,
            credential_expiry: Some(now + chrono::Duration::minutes(5)),
            labels: serde_json::json!({}),
            version: 1,
            created_at: now,
            updated_at: now,
            status: ProviderStatus::Reachable,
            node_id: None,
            model_install_id: None,
            last_probed_at: None,
        proxy_id: None,
        };
        assert!(profile.needs_refresh(now));
        profile.credential_expiry = Some(now + chrono::Duration::hours(2));
        assert!(!profile.needs_refresh(now));
    }
}
