//! Global search: what a hit looks like and which kinds exist (contract C4).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a hit can be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Sandbox,
    Node,
    Model,
    Policy,
    User,
    Site,
    DataCenter,
    Cluster,
    Job,
}

impl SearchKind {
    /// Every kind, in the order the console groups them.
    pub const ALL: [SearchKind; 9] = [
        Self::Sandbox,
        Self::Node,
        Self::Model,
        Self::Policy,
        Self::User,
        Self::Site,
        Self::DataCenter,
        Self::Cluster,
        Self::Job,
    ];

    /// The query-string spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Node => "node",
            Self::Model => "model",
            Self::Policy => "policy",
            Self::User => "user",
            Self::Site => "site",
            Self::DataCenter => "data_center",
            Self::Cluster => "cluster",
            Self::Job => "job",
        }
    }

    /// Parse the query-string spelling.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// One search result.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchHit {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub status: Option<String>,
    pub environment_id: Option<uuid::Uuid>,
    /// Console path from contract C1.
    pub path: String,
    pub score: f64,
}

/// The parsed query of `GET /search`.
#[derive(Debug, Clone, Default)]
pub struct SearchParams {
    pub q: String,
    pub kinds: Vec<SearchKind>,
    pub environment: Option<uuid::Uuid>,
    pub status: Option<String>,
    pub limit: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_string() {
        for kind in SearchKind::ALL {
            assert_eq!(SearchKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(SearchKind::parse("nonsense"), None);
    }
}
