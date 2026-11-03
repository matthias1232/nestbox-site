//! Model clusters: installs of one model behind one provider profile (contract C7).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// How the accounting proxy picks a member per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClusterStrategy {
    /// Weighted rotation.
    RoundRobin,
    /// Fewest in-flight requests.
    LeastLoaded,
    /// First healthy member in order.
    Failover,
}

impl ClusterStrategy {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::LeastLoaded => "least_loaded",
            Self::Failover => "failover",
        }
    }

    /// Parse the stored spelling.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "round_robin" => Self::RoundRobin,
            "least_loaded" => Self::LeastLoaded,
            "failover" => Self::Failover,
            _ => return None,
        })
    }
}

/// Check a cluster name: 2 to 64 characters, lowercase letters, digits and hyphens.
///
/// The name becomes the provider profile name and part of what an operator types
/// into a model picker, so it stays short and plain.
pub fn validate_name(name: &str) -> Result<(), String> {
    let length = name.chars().count();
    if !(2..=64).contains(&length) {
        return Err("A cluster name must be between 2 and 64 characters.".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("A cluster name may only contain a-z, 0-9 and hyphens.".into());
    }
    Ok(())
}

/// Count the members the proxy would send traffic to.
///
/// Healthy means the install is `installed` and its provider profile answered
/// the last probe; the same rule the proxy view applies.
pub fn healthy_count(members: &[ClusterMember]) -> i32 {
    members
        .iter()
        .filter(|m| m.state == "installed" && m.provider_status == "reachable")
        .count() as i32
}

/// One install that belongs to a cluster.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClusterMember {
    pub install_id: Uuid,
    pub node_id: Uuid,
    pub node: String,
    pub state: String,
    pub provider_status: String,
    pub endpoint: String,
    pub weight: i32,
}

/// A cluster as the API returns it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelCluster {
    pub id: Uuid,
    pub name: String,
    pub model_key: String,
    pub strategy: ClusterStrategy,
    pub provider_profile_id: Option<Uuid>,
    pub endpoint: String,
    pub healthy_members: i32,
    pub members: Vec<ClusterMember>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strategy_round_trips_through_its_string() {
        for strategy in [
            ClusterStrategy::RoundRobin,
            ClusterStrategy::LeastLoaded,
            ClusterStrategy::Failover,
        ] {
            assert_eq!(ClusterStrategy::parse(strategy.as_str()), Some(strategy));
        }
        assert_eq!(ClusterStrategy::parse("random"), None);
    }

    #[test]
    fn names_are_short_lowercase_labels() {
        assert!(validate_name("deepseek-x4").is_ok());
        assert!(validate_name("a").is_err());
        assert!(validate_name("Upper").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
        assert!(validate_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn only_installed_and_reachable_members_are_healthy() {
        let member = |state: &str, status: &str| ClusterMember {
            install_id: Uuid::nil(),
            node_id: Uuid::nil(),
            node: "n".into(),
            state: state.into(),
            provider_status: status.into(),
            endpoint: String::new(),
            weight: 1,
        };
        let members = vec![
            member("installed", "reachable"),
            member("installed", "pending"),
            member("failed", "reachable"),
        ];
        assert_eq!(healthy_count(&members), 1);
    }
}
