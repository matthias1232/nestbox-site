//! Who has which role in which scope (contract C13).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::auth::Role;

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
    /// The person.
    pub user_id: Uuid,
    /// Their email.
    pub email: String,
    /// Their display name.
    pub display_name: String,
    /// `admin`, `operator`, `member` or `viewer`.
    pub role: String,
    /// `direct`, `organization`, `grant` or `owner`.
    pub via: String,
    /// When the role was given, if known.
    pub since: Option<DateTime<Utc>>,
}

/// Whether `role` may be written in `scope`: a sandbox grant is always `member`, a data center
/// takes any role, and the organisation tree is read-only. The error is a message for the caller.
pub fn allowed_role_for(scope: ScopeType, role: Role) -> Result<(), &'static str> {
    match scope {
        ScopeType::Organization => Err("The organisation tree is read-only here."),
        ScopeType::Environment => Ok(()),
        ScopeType::Sandbox if role == Role::Member => Ok(()),
        ScopeType::Sandbox => Err("A sandbox grant always gives the member role."),
    }
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

    #[test]
    fn a_sandbox_grant_is_always_member() {
        assert!(allowed_role_for(ScopeType::Sandbox, Role::Member).is_ok());
        assert!(allowed_role_for(ScopeType::Sandbox, Role::Admin).is_err());
        assert!(allowed_role_for(ScopeType::Sandbox, Role::Viewer).is_err());
    }

    #[test]
    fn a_data_center_takes_any_role() {
        for role in [Role::Viewer, Role::Member, Role::Operator, Role::Admin] {
            assert!(allowed_role_for(ScopeType::Environment, role).is_ok());
        }
    }

    #[test]
    fn the_organisation_tree_is_read_only() {
        assert!(allowed_role_for(ScopeType::Organization, Role::Admin).is_err());
    }
}
