//! Who has which role in which scope (contract C13).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Where a role applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScopeType {
    /// Read-only: the organisation tree.
    Organization,
    /// A data center (`user_scopes`).
    Environment,
    /// One sandbox (`sandbox_grants`).
    Sandbox,
}

impl ScopeType {
    /// The wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Environment => "environment",
            Self::Sandbox => "sandbox",
        }
    }

    /// Parse the wire spelling.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "organization" => Self::Organization,
            "environment" => Self::Environment,
            "sandbox" => Self::Sandbox,
            _ => return None,
        })
    }
}

/// One person's role in one scope.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PermissionRow {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    /// `direct`, `organization` or `grant`.
    pub via: String,
    pub since: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_round_trips_through_its_string() {
        for scope in [ScopeType::Organization, ScopeType::Environment, ScopeType::Sandbox] {
            assert_eq!(ScopeType::parse(scope.as_str()), Some(scope));
        }
        assert_eq!(ScopeType::parse("galaxy"), None);
    }
}
