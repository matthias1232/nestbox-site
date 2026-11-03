//! Identity, roles, and permissions.
//!
//! Two separate questions, two separate answers (ADR 0004):
//!
//! * Who is asking — a session cookie or a bearer token.
//! * What may they do — a role, evaluated per action, inside the service.
//!
//! The permission check lives in the service rather than the router because a
//! new route that forgets the check is the common failure. A router-level check
//! is the second line, not the first.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The roles of one environment. Coarse on purpose: finer permissions would need a table,
/// and these cover the real cases without weakening the model.
///
/// `Member` (ADR 0045) is the employee role: everything inside the sandboxes the person
/// owns or was granted, nothing outside them. The "nothing outside" is row-level security on
/// `sandboxes`, not a check here, so a forgotten check cannot widen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Read-only, including telemetry.
    Viewer,
    /// Works in the sandboxes it owns or was granted: lifecycle, shell, its own rules.
    Member,
    /// Lifecycle, policy edits, exec, model install.
    Operator,
    /// Everything operator can do, plus gateway settings and global policy.
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Member => "member",
            Self::Operator => "operator",
            Self::Admin => "admin",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "viewer" => Some(Self::Viewer),
            "member" => Some(Self::Member),
            "operator" => Some(Self::Operator),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Whether this role includes at least the privilege of `other`.
    ///
    /// Ordering is the whole model: admin ⊇ operator ⊇ member ⊇ viewer. A permission set
    /// per role would be more flexible and considerably harder to reason about
    /// at three roles.
    pub fn includes(self, other: Role) -> bool {
        self >= other
    }
}

/// A specific action. Each maps to a minimum role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    SandboxRead,
    SandboxWrite,
    /// Rules, drafts and traffic windows of one sandbox. A member holds this; the policies
    /// that every sandbox shares stay `PolicyWrite`.
    SandboxPolicy,
    /// Shell access into a sandbox. Separate from write because it is the most
    /// powerful action in the product, and a deployment may want to grant
    /// lifecycle without granting a shell.
    SandboxExec,
    PolicyRead,
    PolicyWrite,
    /// The gateway-global override blocks all sandbox-level edits, so it is an
    /// admin action even though it is "just a policy".
    PolicyGlobal,
    GatewayRead,
    /// Changing a gateway's operator configuration.
    GatewayWrite,
    TelemetryRead,
    ModelRead,
    ModelInstall,
    ProviderRead,
    ProviderWrite,
    /// Reading or managing credentials by reference. Never returns a value.
    SecretManage,
    JobRead,
    JobWrite,
    AuditRead,
    UserManage,
    EnvironmentManage,
    NodeRead,
    /// Renaming a node or probing its hardware through its runner.
    NodeManage,
    RecipeRead,
    RecipeWrite,
    /// Persistent roots and mounts touch the node's filesystem.
    PersistWrite,
    /// Which media model a sandbox used, and what it rendered.
    MediaRead,
    /// The environment default, the allow-list, and enabling the capability.
    /// Admin because every sandbox inherits it and it decides what may be
    /// billed; a sandbox's own choice is `SandboxWrite`, not this.
    MediaConfigure,
}

impl Permission {
    /// The minimum role required.
    pub fn required_role(self) -> Role {
        match self {
            Self::SandboxRead
            | Self::PolicyRead
            | Self::GatewayRead
            | Self::TelemetryRead
            | Self::ModelRead
            | Self::ProviderRead
            | Self::JobRead
            | Self::NodeRead
            | Self::RecipeRead
            | Self::MediaRead => Role::Viewer,

            Self::SandboxWrite
            | Self::SandboxExec
            | Self::SandboxPolicy
            | Self::PersistWrite
            | Self::JobWrite => Role::Member,

            Self::PolicyWrite
            | Self::ModelInstall
            | Self::RecipeWrite
            | Self::NodeManage => Role::Operator,

            Self::MediaConfigure => Role::Admin,

            // Providers and their keys are shared by every user of the environment, so an
            // admin sets them; a user picks from what the admin made available.
            // The audit trail names every user's actions.
            Self::AuditRead
            | Self::ProviderWrite
            | Self::PolicyGlobal
            | Self::GatewayWrite
            | Self::SecretManage
            | Self::UserManage
            | Self::EnvironmentManage => Role::Admin,
        }
    }

    /// A stable code for the audit record and the refusal message.
    pub fn code(self) -> &'static str {
        match self {
            Self::SandboxRead => "sandbox.read",
            Self::SandboxWrite => "sandbox.write",
            Self::SandboxPolicy => "sandbox.policy",
            Self::SandboxExec => "sandbox.exec",
            Self::PolicyRead => "policy.read",
            Self::PolicyWrite => "policy.write",
            Self::PolicyGlobal => "policy.global",
            Self::GatewayRead => "gateway.read",
            Self::GatewayWrite => "gateway.write",
            Self::TelemetryRead => "telemetry.read",
            Self::ModelRead => "model.read",
            Self::ModelInstall => "model.install",
            Self::ProviderRead => "provider.read",
            Self::ProviderWrite => "provider.write",
            Self::SecretManage => "secret.manage",
            Self::JobRead => "job.read",
            Self::JobWrite => "job.write",
            Self::AuditRead => "audit.read",
            Self::UserManage => "user.manage",
            Self::EnvironmentManage => "environment.manage",
            Self::NodeRead => "node.read",
            Self::NodeManage => "node.manage",
            Self::RecipeRead => "recipe.read",
            Self::RecipeWrite => "recipe.write",
            Self::PersistWrite => "persist.write",
            Self::MediaRead => "media.read",
            Self::MediaConfigure => "media.configure",
        }
    }
}

/// The authenticated caller, as the service layer sees it.
#[derive(Debug, Clone)]
pub struct Principal {
    pub user_id: Uuid,
    /// The environment this request is scoped to. Chosen at login or per
    /// request, and always verified against membership before use.
    pub environment_id: Uuid,
    /// The role in that environment, from `user_scopes`.
    pub role: Role,
    pub display_name: String,
    /// Whether the caller used a session (browser) or a token (automation).
    /// A session request must carry a CSRF token; a token request must not.
    pub via: CredentialSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    Session,
    Token,
}

impl Principal {
    /// Whether the caller may perform an action.
    pub fn can(&self, permission: Permission) -> bool {
        self.role.includes(permission.required_role())
    }

    /// The same check as a `Result`, with the code in the error so the refusal
    /// names the missing permission rather than saying "forbidden".
    pub fn require(&self, permission: Permission) -> Result<(), ()> {
        if self.can(permission) {
            Ok(())
        } else {
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(role: Role) -> Principal {
        Principal {
            user_id: Uuid::nil(),
            environment_id: Uuid::nil(),
            role,
            display_name: "t".into(),
            via: CredentialSource::Session,
        }
    }

    #[test]
    fn roles_are_ordered_admin_operator_viewer() {
        assert!(Role::Admin.includes(Role::Operator));
        assert!(Role::Operator.includes(Role::Viewer));
        assert!(!Role::Viewer.includes(Role::Operator));
        assert!(Role::Admin.includes(Role::Admin));
    }

    #[test]
    fn a_viewer_cannot_write_or_exec() {
        let viewer = principal(Role::Viewer);
        assert!(viewer.can(Permission::SandboxRead));
        assert!(!viewer.can(Permission::SandboxWrite));
        assert!(!viewer.can(Permission::SandboxExec));
    }

    #[test]
    fn a_member_works_in_a_sandbox_but_changes_nothing_shared() {
        let member = principal(Role::Member);
        assert!(member.can(Permission::SandboxWrite));
        assert!(member.can(Permission::SandboxExec));
        assert!(member.can(Permission::SandboxPolicy));
        assert!(member.can(Permission::TelemetryRead));
        assert!(!member.can(Permission::PolicyWrite));
        assert!(!member.can(Permission::ModelInstall));
        assert!(!member.can(Permission::NodeManage));
        assert!(!member.can(Permission::ProviderWrite));
        assert!(!member.can(Permission::SecretManage));
        assert!(!member.can(Permission::UserManage));
        assert!(Role::Operator.includes(Role::Member));
        assert!(Role::Member.includes(Role::Viewer));
    }

    #[test]
    fn an_operator_can_exec_but_not_manage_the_gateway() {
        let operator = principal(Role::Operator);
        assert!(operator.can(Permission::SandboxExec));
        assert!(operator.can(Permission::PolicyWrite));
        assert!(!operator.can(Permission::GatewayWrite));
        assert!(!operator.can(Permission::PolicyGlobal));
        assert!(!operator.can(Permission::SecretManage));
    }

    #[test]
    fn only_an_admin_can_set_the_global_override() {
        assert!(principal(Role::Admin).can(Permission::PolicyGlobal));
        assert!(!principal(Role::Operator).can(Permission::PolicyGlobal));
    }

    #[test]
    fn every_permission_has_a_stable_code() {
        let all = [
            Permission::SandboxRead,
            Permission::SandboxExec,
            Permission::PolicyGlobal,
            Permission::SecretManage,
        ];
        for permission in all {
            let code = permission.code();
            assert!(code.contains('.'), "{code} should be namespaced");
            assert!(code.chars().all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'));
        }
    }
}
