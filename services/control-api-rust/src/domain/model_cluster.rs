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
}
